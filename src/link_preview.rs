//! Bounded local-only previews for Markdown link hover.
//!
//! This module never launches a target or performs network access. A file read
//! is allowed only for a lexical path inside an already trusted workspace,
//! after every existing path component has been checked for reparse points.

use crate::{
    assets::{self, DecodedImage},
    document::{Document, DocumentSnapshot},
    navigation::{Outline, percent_decode},
    projection::DisplayProjection,
};
use std::{
    collections::VecDeque,
    fs::{self, File, Metadata},
    io::{self, Read},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::SystemTime,
};

const MAX_TEXT_FILE_BYTES: u64 = 512 * 1024;
const MAX_EXCERPT_CHARS: usize = 560;
const MAX_IMAGE_PIXELS: u64 = 8_000_000;
const THUMBNAIL_WIDTH: u32 = 440;
const THUMBNAIL_HEIGHT: u32 = 300;
const CACHE_MAX_ENTRIES: usize = 12;
const CACHE_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct OpenDocumentSnapshot {
    pub path: PathBuf,
    /// COW Rope snapshot: copying the open-tab list on WM_MOUSEMOVE does not
    /// materialize every tab's full source string on the UI thread.
    pub document: DocumentSnapshot,
}

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    /// Identity of the editor document whose link is currently hovered.
    pub source_document_id: u64,
    pub source_revision: u64,
    pub source_path: Option<PathBuf>,
    pub source: Arc<str>,
    pub workspace_root: Option<PathBuf>,
    pub destination: String,
    /// Open tabs are snapshots, so hover never activates a document or asks it
    /// to save. `source_shared()` makes these cheap to assemble on the UI thread.
    pub open_documents: Vec<OpenDocumentSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewFallback {
    ExternalLink,
    UnsupportedScheme,
    UnsafePath,
    OutsideWorkspace,
    MissingTarget,
    ReadDenied,
    UnsupportedType,
    TooLarge,
    InvalidText,
    InvalidImage,
    FragmentNotFound,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewContent {
    Text {
        label: String,
        title: String,
        excerpt: String,
        fragment: Option<String>,
    },
    Image {
        label: String,
        width: u32,
        height: u32,
        /// Premultiplied BGRA pixels, matching `assets::DecodedImage`.
        pixels: Vec<u8>,
    },
    Unavailable {
        label: String,
        reason: PreviewFallback,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewResult {
    pub generation: u64,
    pub source_document_id: u64,
    pub source_revision: u64,
    pub content: PreviewContent,
}

impl PreviewResult {
    pub fn is_current_for(&self, generation: u64, document_id: u64, revision: u64) -> bool {
        self.generation == generation
            && self.source_document_id == document_id
            && self.source_revision == revision
    }
}

#[derive(Clone, Debug)]
struct RequestEnvelope {
    generation: u64,
    request: PreviewRequest,
}

#[derive(Default)]
struct WorkQueue {
    latest: Option<RequestEnvelope>,
    stopped: bool,
}

/// One background resolver with a single replaceable pending request. Hover
/// churn cannot create a thread or an unbounded backlog per pointer move.
pub struct PreviewCoordinator {
    queue: Arc<(Mutex<WorkQueue>, Condvar)>,
    generation: Arc<AtomicU64>,
    results: Receiver<PreviewResult>,
}

impl PreviewCoordinator {
    pub fn new() -> io::Result<Self> {
        let queue = Arc::new((Mutex::new(WorkQueue::default()), Condvar::new()));
        let generation = Arc::new(AtomicU64::new(0));
        let (result_tx, results) = mpsc::channel();
        let worker_queue = Arc::clone(&queue);
        let worker_generation = Arc::clone(&generation);
        thread::Builder::new()
            .name("mdlite-link-preview".into())
            .spawn(move || preview_worker(worker_queue, worker_generation, result_tx))?;
        Ok(Self {
            queue,
            generation,
            results,
        })
    }

    /// Queue a hover request and return its generation. Results from earlier
    /// links are discarded even when their bounded local read finishes later.
    pub fn request(&self, request: PreviewRequest) -> u64 {
        let generation = next_generation(&self.generation);
        let (lock, wake) = &*self.queue;
        if let Ok(mut queue) = lock.lock() {
            queue.latest = Some(RequestEnvelope {
                generation,
                request,
            });
            wake.notify_one();
        }
        generation
    }

    /// Close the current preview and invalidate both queued and in-flight work.
    pub fn cancel(&self) -> u64 {
        let generation = next_generation(&self.generation);
        let (lock, wake) = &*self.queue;
        if let Ok(mut queue) = lock.lock() {
            queue.latest = None;
            wake.notify_one();
        }
        generation
    }

    pub fn current_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Drain stale completions until the latest generation is available.
    pub fn poll(&self) -> Option<PreviewResult> {
        loop {
            match self.results.try_recv() {
                Ok(result) if result.generation == self.current_generation() => {
                    return Some(result);
                }
                Ok(_) => continue,
                Err(_) => return None,
            }
        }
    }
}

impl Drop for PreviewCoordinator {
    fn drop(&mut self) {
        let (lock, wake) = &*self.queue;
        if let Ok(mut queue) = lock.lock() {
            queue.stopped = true;
            queue.latest = None;
            wake.notify_one();
        }
    }
}

fn next_generation(generation: &AtomicU64) -> u64 {
    generation.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
}

fn preview_worker(
    queue: Arc<(Mutex<WorkQueue>, Condvar)>,
    generation: Arc<AtomicU64>,
    results: Sender<PreviewResult>,
) {
    let (lock, wake) = &*queue;
    let mut cache = PreviewCache::default();
    loop {
        let envelope = {
            let mut state = match lock.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            while state.latest.is_none() && !state.stopped {
                state = match wake.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            if state.stopped {
                return;
            }
            state.latest.take()
        };
        let Some(envelope) = envelope else {
            continue;
        };
        let result = resolve_cached(envelope.generation, envelope.request, &mut cache);
        if result.generation == generation.load(Ordering::Acquire) {
            let _ = results.send(result);
        }
    }
}

/// Synchronous form used by focused tests and the single background worker.
/// UI integration should call `PreviewCoordinator::request` instead.
pub fn resolve_preview(request: PreviewRequest) -> PreviewResult {
    resolve_cached(0, request, &mut PreviewCache::default())
}

fn resolve_cached(
    generation: u64,
    request: PreviewRequest,
    cache: &mut PreviewCache,
) -> PreviewResult {
    let source_document_id = request.source_document_id;
    let source_revision = request.source_revision;
    let content = match prepare(&request) {
        Err((label, reason)) => PreviewContent::Unavailable { label, reason },
        Ok(prepared) => {
            if let Some(content) = cache.get(&prepared.key) {
                content.clone()
            } else {
                let content = match prepared.source {
                    PreviewSource::Text(source) => {
                        build_text_preview(&prepared.label, &source, prepared.fragment.as_deref())
                    }
                    PreviewSource::Image(path, metadata) => {
                        build_image_preview(&prepared.label, &path, &metadata)
                    }
                };
                if !matches!(content, PreviewContent::Unavailable { .. }) {
                    cache.insert(prepared.key, content.clone());
                }
                content
            }
        }
    };
    PreviewResult {
        generation,
        source_document_id,
        source_revision,
        content,
    }
}

struct PreparedPreview {
    label: String,
    fragment: Option<String>,
    key: CacheKey,
    source: PreviewSource,
}

enum PreviewSource {
    Text(Arc<str>),
    Image(PathBuf, Metadata),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CacheKey {
    Document {
        path: PathBuf,
        document_id: u64,
        revision: u64,
        fragment: Option<String>,
    },
    File {
        path: PathBuf,
        modified: SystemTime,
        bytes: u64,
        fragment: Option<String>,
    },
}

#[derive(Default)]
struct PreviewCache {
    entries: VecDeque<(CacheKey, PreviewContent, usize)>,
    bytes: usize,
}

impl PreviewCache {
    fn get(&mut self, key: &CacheKey) -> Option<&PreviewContent> {
        let index = self
            .entries
            .iter()
            .position(|(candidate, _, _)| candidate == key)?;
        let entry = self.entries.remove(index)?;
        self.entries.push_back(entry);
        self.entries.back().map(|(_, content, _)| content)
    }

    fn insert(&mut self, key: CacheKey, content: PreviewContent) {
        let size = content_size(&content);
        if size > CACHE_MAX_BYTES {
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|(candidate, _, _)| *candidate == key)
            && let Some((_, _, old_size)) = self.entries.remove(index)
        {
            self.bytes = self.bytes.saturating_sub(old_size);
        }
        while self.entries.len() >= CACHE_MAX_ENTRIES
            || self.bytes.saturating_add(size) > CACHE_MAX_BYTES
        {
            let Some((_, _, removed_size)) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed_size);
        }
        self.bytes = self.bytes.saturating_add(size);
        self.entries.push_back((key, content, size));
    }
}

fn content_size(content: &PreviewContent) -> usize {
    match content {
        PreviewContent::Text {
            label,
            title,
            excerpt,
            fragment,
        } => label.len() + title.len() + excerpt.len() + fragment.as_ref().map_or(0, String::len),
        PreviewContent::Image { label, pixels, .. } => label.len().saturating_add(pixels.len()),
        PreviewContent::Unavailable { label, .. } => label.len(),
    }
}

fn prepare(request: &PreviewRequest) -> Result<PreparedPreview, (String, PreviewFallback)> {
    let destination = request.destination.trim();
    let (candidate, fragment) = match parse_target(destination, request.source_path.as_deref()) {
        Ok(ParsedTarget::CurrentFragment(fragment)) => {
            let label = request
                .source_path
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Untitled".into());
            let key = CacheKey::Document {
                path: request
                    .source_path
                    .as_deref()
                    .and_then(normalize_internal_path)
                    .unwrap_or_default(),
                document_id: request.source_document_id,
                revision: request.source_revision,
                fragment: Some(fragment.clone()),
            };
            return Ok(PreparedPreview {
                label,
                fragment: Some(fragment),
                key,
                source: PreviewSource::Text(Arc::clone(&request.source)),
            });
        }
        Ok(ParsedTarget::Local { path, fragment }) => (path, fragment),
        Err(reason) => return Err((preview_label(destination), reason)),
    };

    if is_text_preview_path(&candidate) {
        if let Some(source_path) = request
            .source_path
            .as_deref()
            .and_then(normalize_internal_path)
            && same_path(&candidate, &source_path)
        {
            return Ok(PreparedPreview {
                label: file_label(&source_path),
                key: CacheKey::Document {
                    path: source_path,
                    document_id: request.source_document_id,
                    revision: request.source_revision,
                    fragment: fragment.clone(),
                },
                fragment,
                source: PreviewSource::Text(Arc::clone(&request.source)),
            });
        }
        if let Some(document) = open_document(request, &candidate) {
            let path =
                normalize_internal_path(&document.path).unwrap_or_else(|| document.path.clone());
            if document.document.rope().byte_len() as u64 > MAX_TEXT_FILE_BYTES {
                return Err((file_label(&path), PreviewFallback::TooLarge));
            }
            let source = Arc::<str>::from(document.document.rope().to_string());
            return Ok(PreparedPreview {
                label: file_label(&path),
                key: CacheKey::Document {
                    path,
                    document_id: document.document.id,
                    revision: document.document.revision,
                    fragment: fragment.clone(),
                },
                fragment,
                source: PreviewSource::Text(source),
            });
        }
    }

    let Some(workspace_root) = request.workspace_root.as_deref() else {
        return Err((file_label(&candidate), PreviewFallback::OutsideWorkspace));
    };
    let (path, metadata) = checked_workspace_file(workspace_root, &candidate)
        .map_err(|reason| (file_label(&candidate), reason))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "md" | "markdown" | "mdown" | "txt" => {
            if metadata.len() > MAX_TEXT_FILE_BYTES {
                return Err((file_label(&path), PreviewFallback::TooLarge));
            }
            let Some(modified) = metadata.modified().ok() else {
                return Err((file_label(&path), PreviewFallback::ReadDenied));
            };
            Ok(PreparedPreview {
                label: file_label(&path),
                key: CacheKey::File {
                    path: path.clone(),
                    modified,
                    bytes: metadata.len(),
                    fragment: fragment.clone(),
                },
                fragment,
                source: PreviewSource::Text(read_text_file(&path, metadata.len())?),
            })
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" => {
            let Some(modified) = metadata.modified().ok() else {
                return Err((file_label(&path), PreviewFallback::ReadDenied));
            };
            Ok(PreparedPreview {
                label: file_label(&path),
                key: CacheKey::File {
                    path: path.clone(),
                    modified,
                    bytes: metadata.len(),
                    fragment: None,
                },
                fragment: None,
                source: PreviewSource::Image(path, metadata),
            })
        }
        _ => Err((file_label(&path), PreviewFallback::UnsupportedType)),
    }
}

enum ParsedTarget {
    CurrentFragment(String),
    Local {
        path: PathBuf,
        fragment: Option<String>,
    },
}

fn parse_target(
    destination: &str,
    source_path: Option<&Path>,
) -> Result<ParsedTarget, PreviewFallback> {
    if destination.chars().any(char::is_control) {
        return Err(PreviewFallback::UnsafePath);
    }
    if destination.starts_with("\\\\")
        || destination.starts_with("//")
        || is_device_namespace(destination)
    {
        return Err(PreviewFallback::UnsafePath);
    }
    if let Some(fragment) = destination.strip_prefix('#') {
        percent_decode(fragment).map_err(|_| PreviewFallback::UnsafePath)?;
        return Ok(ParsedTarget::CurrentFragment(fragment.to_owned()));
    }
    if let Some((scheme, rest)) = destination.split_once(':') {
        if ["http", "https", "mailto"]
            .iter()
            .any(|known| scheme.eq_ignore_ascii_case(known))
        {
            return Err(PreviewFallback::ExternalLink);
        }
        if scheme.len() != 1 || !scheme.as_bytes()[0].is_ascii_alphabetic() {
            return Err(PreviewFallback::UnsupportedScheme);
        }
        if !rest.starts_with('/') && !rest.starts_with('\\') {
            return Err(PreviewFallback::UnsafePath); // Reject drive-relative paths.
        }
    }

    let (encoded_path, fragment) = destination
        .split_once('#')
        .map_or((destination, None), |(path, fragment)| {
            (path, Some(fragment.to_owned()))
        });
    if let Some(fragment) = fragment.as_deref() {
        percent_decode(fragment).map_err(|_| PreviewFallback::UnsafePath)?;
    }
    let decoded_path = percent_decode(encoded_path).map_err(|_| PreviewFallback::UnsafePath)?;
    if decoded_path.is_empty() {
        if let Some(fragment) = fragment {
            return Ok(ParsedTarget::CurrentFragment(fragment));
        }
        return Err(PreviewFallback::MissingTarget);
    }
    if decoded_path.contains("://")
        || decoded_path.starts_with("//")
        || decoded_path.starts_with("\\\\")
        || decoded_path.starts_with('/')
        || decoded_path.starts_with('\\')
        || is_device_namespace(&decoded_path)
    {
        return Err(PreviewFallback::UnsafePath);
    }
    let input_path = Path::new(&decoded_path);
    if input_path.is_absolute() {
        if is_network_path(input_path) {
            return Err(PreviewFallback::UnsafePath);
        }
        let path = normalize_lexically(input_path).ok_or(PreviewFallback::UnsafePath)?;
        return Ok(ParsedTarget::Local { path, fragment });
    }
    let Some(source_path) = source_path.and_then(normalize_internal_path) else {
        return Err(PreviewFallback::MissingTarget);
    };
    if is_network_path(&source_path) {
        return Err(PreviewFallback::UnsafePath);
    }
    let Some(parent) = source_path.parent() else {
        return Err(PreviewFallback::UnsafePath);
    };
    let path = normalize_lexically(&parent.join(input_path)).ok_or(PreviewFallback::UnsafePath)?;
    if is_network_path(&path) {
        return Err(PreviewFallback::UnsafePath);
    }
    Ok(ParsedTarget::Local { path, fragment })
}

fn open_document<'a>(
    request: &'a PreviewRequest,
    candidate: &Path,
) -> Option<&'a OpenDocumentSnapshot> {
    let candidate = normalize_lexically(candidate)?;
    request.open_documents.iter().find(|document| {
        normalize_internal_path(&document.path).is_some_and(|path| same_path(&candidate, &path))
    })
}

fn checked_workspace_file(
    root: &Path,
    candidate: &Path,
) -> Result<(PathBuf, Metadata), PreviewFallback> {
    let root = normalize_internal_path(root).ok_or(PreviewFallback::UnsafePath)?;
    if is_network_path(&root) || is_network_path(candidate) {
        return Err(PreviewFallback::UnsafePath);
    }
    if !path_is_within(&root, candidate) {
        return Err(PreviewFallback::OutsideWorkspace);
    }

    // Check from the volume root, not just below the workspace root: that
    // root or one of its ancestors may have become a reparse point since the
    // workspace was trusted. Canonical containment is checked again before
    // reading; a concurrent path swap after this preflight remains a normal
    // path-based TOCTOU limitation.
    let components = candidate.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    let mut final_metadata = None;
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::Prefix(_) | Component::RootDir => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => return Err(PreviewFallback::UnsafePath),
            Component::Normal(part) => {
                current.push(part);
                let metadata = fs::symlink_metadata(&current).map_err(map_io_error)?;
                if is_reparse_point(&metadata) {
                    return Err(PreviewFallback::UnsafePath);
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(PreviewFallback::MissingTarget);
                }
                final_metadata = Some(metadata);
            }
        }
    }
    let metadata = final_metadata.ok_or(PreviewFallback::MissingTarget)?;
    if !metadata.is_file() {
        return Err(PreviewFallback::UnsupportedType);
    }
    let canonical_root = fs::canonicalize(&root).map_err(map_io_error)?;
    let canonical_candidate = fs::canonicalize(&current).map_err(map_io_error)?;
    if !path_is_within(&canonical_root, &canonical_candidate) {
        return Err(PreviewFallback::OutsideWorkspace);
    }
    Ok((canonical_candidate, metadata))
}

fn is_reparse_point(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn map_io_error(error: io::Error) -> PreviewFallback {
    if error.kind() == io::ErrorKind::NotFound {
        PreviewFallback::MissingTarget
    } else {
        PreviewFallback::ReadDenied
    }
}

fn read_text_file(path: &Path, bytes: u64) -> Result<Arc<str>, (String, PreviewFallback)> {
    if bytes > MAX_TEXT_FILE_BYTES {
        return Err((file_label(path), PreviewFallback::TooLarge));
    }
    let file = File::open(path).map_err(|error| (file_label(path), map_io_error(error)))?;
    let limit = MAX_TEXT_FILE_BYTES.saturating_add(1);
    let mut input = Vec::with_capacity(bytes as usize);
    file.take(limit)
        .read_to_end(&mut input)
        .map_err(|error| (file_label(path), map_io_error(error)))?;
    if input.len() as u64 > MAX_TEXT_FILE_BYTES {
        return Err((file_label(path), PreviewFallback::TooLarge));
    }
    String::from_utf8(input)
        .map(Arc::<str>::from)
        .map_err(|_| (file_label(path), PreviewFallback::InvalidText))
}

fn build_text_preview(label: &str, source: &str, fragment: Option<&str>) -> PreviewContent {
    if source.len() as u64 > MAX_TEXT_FILE_BYTES {
        return unavailable(label, PreviewFallback::TooLarge);
    }
    let id = 1;
    let document = Document::new_unsaved(id, source.to_owned());
    let projection = DisplayProjection::parse(document.source());
    let outline = Outline::build(&document, &projection);
    let selected = match fragment {
        Some(fragment) => match outline.anchor(fragment) {
            Ok(offset) => outline
                .headings
                .iter()
                .find(|heading| heading.source.start == offset),
            Err(_) => return unavailable(label, PreviewFallback::FragmentNotFound),
        },
        None => outline.headings.first(),
    };
    let (title, excerpt_range, fragment_label) = if let Some(heading) = selected {
        (
            heading.label.clone(),
            heading.section.clone(),
            Some(heading.label.clone()),
        )
    } else {
        (label.to_owned(), 0..source.len(), None)
    };
    let excerpt_source = source.get(excerpt_range).unwrap_or(source);
    let excerpt = compact_excerpt(excerpt_source);
    PreviewContent::Text {
        label: label.to_owned(),
        title,
        excerpt,
        fragment: fragment_label,
    }
}

fn compact_excerpt(source: &str) -> String {
    let mut excerpt = String::new();
    let mut lines = 0usize;
    let mut source_lines = source
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .peekable();
    let mut chars = 0usize;
    while let Some(line) = source_lines.next() {
        if lines > 0 {
            excerpt.push('\n');
        }
        for ch in line.chars() {
            if chars >= MAX_EXCERPT_CHARS {
                excerpt.push('…');
                return excerpt;
            }
            excerpt.push(ch);
            chars += 1;
        }
        lines += 1;
        if lines >= 7 {
            if source_lines.peek().is_some() {
                excerpt.push('\n');
                excerpt.push('…');
            }
            break;
        }
    }
    excerpt
}

fn build_image_preview(label: &str, path: &Path, metadata: &Metadata) -> PreviewContent {
    if metadata.len() > 128 * 1024 * 1024 {
        return unavailable(label, PreviewFallback::TooLarge);
    }
    let (width, height) = match assets::image_dimensions(path) {
        Ok(dimensions) => dimensions,
        Err(_) => return unavailable(label, PreviewFallback::InvalidImage),
    };
    let pixels = u64::from(width).saturating_mul(u64::from(height));
    if pixels == 0 || pixels > MAX_IMAGE_PIXELS {
        return unavailable(label, PreviewFallback::TooLarge);
    }
    let decoded = match assets::decode(path) {
        Ok(decoded) => decoded,
        Err(_) => return unavailable(label, PreviewFallback::InvalidImage),
    };
    match thumbnail(decoded) {
        Some(image) => PreviewContent::Image {
            label: label.to_owned(),
            width: image.width,
            height: image.height,
            pixels: image.pixels,
        },
        None => unavailable(label, PreviewFallback::InvalidImage),
    }
}

fn thumbnail(image: DecodedImage) -> Option<DecodedImage> {
    if image.width == 0
        || image.height == 0
        || image.pixels.len() != image.width as usize * image.height as usize * 4
    {
        return None;
    }
    let scale = (THUMBNAIL_WIDTH as f64 / f64::from(image.width))
        .min(THUMBNAIL_HEIGHT as f64 / f64::from(image.height))
        .min(1.0);
    let width = ((f64::from(image.width) * scale).round() as u32).max(1);
    let height = ((f64::from(image.height) * scale).round() as u32).max(1);
    if width == image.width && height == image.height {
        return Some(image);
    }
    let mut pixels = vec![0; width as usize * height as usize * 4];
    for y in 0..height {
        let source_y = ((u64::from(y) * u64::from(image.height)) / u64::from(height)) as usize;
        for x in 0..width {
            let source_x = ((u64::from(x) * u64::from(image.width)) / u64::from(width)) as usize;
            let source = (source_y * image.width as usize + source_x) * 4;
            let target = (y as usize * width as usize + x as usize) * 4;
            pixels[target..target + 4].copy_from_slice(&image.pixels[source..source + 4]);
        }
    }
    Some(DecodedImage {
        width,
        height,
        pixels,
    })
}

fn unavailable(label: &str, reason: PreviewFallback) -> PreviewContent {
    PreviewContent::Unavailable {
        label: label.to_owned(),
        reason,
    }
}

fn preview_label(value: &str) -> String {
    let value = value.chars().take(120).collect::<String>();
    if value.is_empty() {
        "Link".into()
    } else {
        value
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().chars().take(120).collect())
        .unwrap_or_else(|| "Local file".into())
}

fn is_text_preview_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            ["md", "markdown", "mdown", "txt"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
}

fn normalize_internal_path(path: &Path) -> Option<PathBuf> {
    let text = path.to_string_lossy();
    const EXTENDED_LOCAL_PREFIX: &str = "\\\\?\\";
    let local_extended = text.len() >= 7
        && text.starts_with(EXTENDED_LOCAL_PREFIX)
        && text.as_bytes().get(5) == Some(&b':')
        && matches!(text.as_bytes().get(6), Some(b'\\' | b'/'));
    if local_extended {
        return normalize_lexically(Path::new(text.strip_prefix(EXTENDED_LOCAL_PREFIX)?));
    }
    if is_network_path(path) || is_device_path(path) {
        return None;
    }
    normalize_lexically(path)
}

fn normalize_lexically(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(value) => normalized.push(value),
        }
    }
    normalized.is_absolute().then_some(normalized)
}

fn path_is_within(root: &Path, candidate: &Path) -> bool {
    let root_parts = root.components().collect::<Vec<_>>();
    let candidate_parts = candidate.components().collect::<Vec<_>>();
    candidate_parts.len() >= root_parts.len()
        && root_parts
            .iter()
            .zip(candidate_parts.iter())
            .all(|(left, right)| {
                left.as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
            })
}

fn same_path(left: &Path, right: &Path) -> bool {
    let left = normalize_lexically(left);
    let right = normalize_lexically(right);
    left.zip(right).is_some_and(|(left, right)| {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    })
}

fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    text.starts_with("\\\\") || text.starts_with("//")
}

fn is_device_namespace(text: &str) -> bool {
    text.starts_with("\\\\?\\")
        || text.starts_with("\\\\.\\")
        || text.starts_with("\\??\\")
        || text.starts_with("\\Device\\")
        || text.starts_with("\\GLOBALROOT\\")
}

fn is_device_path(path: &Path) -> bool {
    is_device_namespace(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(destination: &str, source_path: Option<PathBuf>, source: &str) -> PreviewRequest {
        PreviewRequest {
            source_document_id: 41,
            source_revision: 7,
            source_path,
            source: Arc::from(source),
            workspace_root: None,
            destination: destination.to_owned(),
            open_documents: Vec::new(),
        }
    }

    #[test]
    fn fragment_preview_uses_unsaved_source_and_selects_only_requested_section() {
        let result = resolve_preview(request(
            "#second-section",
            None,
            "# First section\nold body\n\n# Second section\ncurrent body\n",
        ));
        assert_eq!(result.source_document_id, 41);
        assert_eq!(result.source_revision, 7);
        assert_eq!(
            result.content,
            PreviewContent::Text {
                label: "Untitled".into(),
                title: "Second section".into(),
                excerpt: "# Second section\ncurrent body".into(),
                fragment: Some("Second section".into()),
            }
        );
    }

    #[test]
    fn unicode_windows_paths_are_safe_for_origin_root_open_tab_and_fragment() {
        let unicode_origin = PathBuf::from(r"C:\日本語\origin.md");
        let unicode_target = PathBuf::from(r"C:\日本語\draft.md");
        assert_eq!(
            normalize_internal_path(&unicode_origin),
            Some(unicode_origin.clone())
        );
        assert_eq!(
            normalize_internal_path(Path::new(r"\\?\C:\日本語\origin.md")),
            Some(unicode_origin.clone())
        );

        let fragment = resolve_preview(request(
            "#chosen",
            Some(unicode_origin.clone()),
            "# Chosen\nunsaved origin section\n",
        ));
        assert_eq!(
            fragment.content,
            PreviewContent::Text {
                label: "origin.md".into(),
                title: "Chosen".into(),
                excerpt: "# Chosen\nunsaved origin section".into(),
                fragment: Some("Chosen".into()),
            }
        );

        let mut open_target = request("draft.md#fresh", Some(unicode_origin), "# origin\n");
        open_target.open_documents.push(OpenDocumentSnapshot {
            path: unicode_target,
            document: Document::new_unsaved(88, "# Fresh\nunsaved target section\n".into())
                .snapshot(),
        });
        assert_eq!(
            resolve_preview(open_target).content,
            PreviewContent::Text {
                label: "draft.md".into(),
                title: "Fresh".into(),
                excerpt: "# Fresh\nunsaved target section".into(),
                fragment: Some("Fresh".into()),
            }
        );

        let mut unicode_root = request(
            r"C:\elsewhere\outside.md",
            Some(PathBuf::from(r"C:\日本語\origin.md")),
            "# origin\n",
        );
        unicode_root.workspace_root = Some(PathBuf::from(r"C:\日本語\workspace"));
        assert_eq!(
            resolve_preview(unicode_root).content,
            PreviewContent::Unavailable {
                label: "outside.md".into(),
                reason: PreviewFallback::OutsideWorkspace,
            }
        );
    }

    #[test]
    fn open_unsaved_target_precedes_disk_and_works_without_workspace_trust() {
        let base = test_directory();
        let origin = base.join("origin.md");
        let target = base.join("draft.md");
        let mut request = request("draft.md#fresh", Some(origin), "# origin\n");
        request.open_documents.push(OpenDocumentSnapshot {
            path: target,
            document: Document::new_unsaved(88, "# Fresh\nunsaved contents\n".into()).snapshot(),
        });
        let result = resolve_preview(request);
        assert_eq!(result.source_document_id, 41);
        assert_eq!(result.source_revision, 7);
        assert_eq!(
            result.content,
            PreviewContent::Text {
                label: "draft.md".into(),
                title: "Fresh".into(),
                excerpt: "# Fresh\nunsaved contents".into(),
                fragment: Some("Fresh".into()),
            }
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn external_unc_device_and_encoded_network_destinations_stop_before_io() {
        let network_root = PathBuf::from(r"\\preview-invalid\workspace");
        for (destination, expected) in [
            (
                "https://127.0.0.1:9/never-fetch",
                PreviewFallback::ExternalLink,
            ),
            (
                r"\\preview-invalid\share\note.md",
                PreviewFallback::UnsafePath,
            ),
            (r"\\?\C:\device\note.md", PreviewFallback::UnsafePath),
            (
                "%5C%5Cpreview-invalid%5Cshare%5Cnote.md",
                PreviewFallback::UnsafePath,
            ),
        ] {
            let mut input = request(
                destination,
                Some(PathBuf::from(r"C:\work\origin.md")),
                "# source\n",
            );
            input.workspace_root = Some(network_root.clone());
            assert!(
                matches!(
                    resolve_preview(input).content,
                    PreviewContent::Unavailable { reason, .. } if reason == expected
                ),
                "{destination:?}"
            );
        }
    }

    #[test]
    fn paths_outside_workspace_are_refused_before_target_read() {
        let root = std::env::temp_dir().join("mdlite-preview-trusted-workspace");
        let origin = root.join("docs").join("origin.md");
        let mut input = request("../../outside.md", Some(origin), "# source\n");
        input.workspace_root = Some(root);
        assert_eq!(
            resolve_preview(input).content,
            PreviewContent::Unavailable {
                label: "outside.md".into(),
                reason: PreviewFallback::OutsideWorkspace,
            }
        );
    }

    #[test]
    fn replaced_workspace_root_junction_is_refused_before_outside_read() {
        let base = test_directory();
        let workspace = base.join("workspace");
        let moved_workspace = base.join("workspace-original");
        let outside = base.join("outside");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(workspace.join("target.md"), "inside sentinel\n").unwrap();
        fs::write(outside.join("target.md"), "outside sentinel\n").unwrap();
        let mut input = request("target.md", Some(workspace.join("origin.md")), "# origin\n");
        input.workspace_root = Some(workspace.clone());
        assert!(matches!(
            resolve_preview(input.clone()).content,
            PreviewContent::Text { excerpt, .. } if excerpt == "inside sentinel"
        ));

        fs::rename(&workspace, &moved_workspace).unwrap();
        create_directory_link(&outside, &workspace).unwrap();
        assert_eq!(
            resolve_preview(input).content,
            PreviewContent::Unavailable {
                label: "target.md".into(),
                reason: PreviewFallback::UnsafePath,
            }
        );
        remove_directory_link(&workspace).unwrap();
        assert_eq!(
            fs::read_to_string(outside.join("target.md")).unwrap(),
            "outside sentinel\n"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn replaced_workspace_ancestor_junction_is_refused_before_outside_read() {
        let base = test_directory();
        let parent = base.join("parent");
        let moved_parent = base.join("parent-original");
        let workspace = parent.join("workspace");
        let outside = base.join("outside");
        let outside_workspace = outside.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        fs::create_dir_all(&outside_workspace).unwrap();
        fs::write(workspace.join("target.md"), "inside sentinel\n").unwrap();
        fs::write(outside_workspace.join("target.md"), "outside sentinel\n").unwrap();
        let mut input = request("target.md", Some(workspace.join("origin.md")), "# origin\n");
        input.workspace_root = Some(workspace.clone());
        assert!(matches!(
            resolve_preview(input.clone()).content,
            PreviewContent::Text { excerpt, .. } if excerpt == "inside sentinel"
        ));

        fs::rename(&parent, &moved_parent).unwrap();
        create_directory_link(&outside, &parent).unwrap();
        assert_eq!(
            resolve_preview(input).content,
            PreviewContent::Unavailable {
                label: "target.md".into(),
                reason: PreviewFallback::UnsafePath,
            }
        );
        remove_directory_link(&parent).unwrap();
        assert_eq!(
            fs::read_to_string(outside_workspace.join("target.md")).unwrap(),
            "outside sentinel\n"
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn local_workspace_markdown_loads_current_contents_and_fragment() {
        let root = test_directory();
        let docs = root.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let origin = docs.join("origin.md");
        let target = docs.join("target.md");
        fs::write(&target, "# Disk section\ncurrent disk body\n").unwrap();
        let mut input = request("target.md#disk-section", Some(origin), "# origin\n");
        input.workspace_root = Some(root.clone());
        assert_eq!(
            resolve_preview(input).content,
            PreviewContent::Text {
                label: "target.md".into(),
                title: "Disk section".into(),
                excerpt: "# Disk section\ncurrent disk body".into(),
                fragment: Some("Disk section".into()),
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn closed_disk_target_is_never_read_without_workspace_trust() {
        let root = test_directory();
        let origin = root.join("origin.md");
        let target = root.join("target.txt");
        fs::write(&target, "this is readable but not implicitly trusted").unwrap();
        let input = request("target.txt", Some(origin), "# origin\n");
        assert_eq!(
            resolve_preview(input).content,
            PreviewContent::Unavailable {
                label: "target.txt".into(),
                reason: PreviewFallback::OutsideWorkspace,
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn thumbnail_keeps_aspect_ratio_and_bounds_pixels() {
        let source = DecodedImage {
            width: 880,
            height: 600,
            pixels: vec![32; 880 * 600 * 4],
        };
        let thumb = thumbnail(source).unwrap();
        assert_eq!((thumb.width, thumb.height), (440, 300));
        assert_eq!(thumb.pixels.len(), 440 * 300 * 4);
        assert_eq!(&thumb.pixels[..4], &[32; 4]);
    }

    #[test]
    fn trusted_workspace_image_returns_decoded_thumbnail_pixels() {
        use image::ImageEncoder;

        let root = test_directory();
        let docs = root.join("docs");
        fs::create_dir_all(&docs).unwrap();
        let target = docs.join("thumbnail.png");
        let rgba = [255, 0, 0, 255, 0, 255, 0, 128];
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, 2, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        fs::write(&target, png).unwrap();

        let mut input = request("thumbnail.png", Some(docs.join("origin.md")), "# origin\n");
        input.workspace_root = Some(root.clone());
        let result = resolve_preview(input);
        assert_eq!(
            result.content,
            PreviewContent::Image {
                label: "thumbnail.png".into(),
                width: 2,
                height: 1,
                pixels: vec![0, 0, 255, 255, 0, 128, 0, 128],
            }
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn latest_generation_rejects_old_async_results_after_target_change() {
        let coordinator = PreviewCoordinator::new().unwrap();
        let first = coordinator.request(request("#first", None, "# First\n"));
        let second = coordinator.request(request("#second", None, "# Second\n"));
        assert_eq!(first, 1);
        assert_eq!(second, 2);
        assert_ne!(first, coordinator.current_generation());
        assert_eq!(second, coordinator.current_generation());
        let stale = PreviewResult {
            generation: first,
            source_document_id: 41,
            source_revision: 7,
            content: PreviewContent::Unavailable {
                label: "first".into(),
                reason: PreviewFallback::FragmentNotFound,
            },
        };
        let current = PreviewResult {
            generation: second,
            source_document_id: 41,
            source_revision: 7,
            content: PreviewContent::Text {
                label: "second.md".into(),
                title: "Second".into(),
                excerpt: "new target".into(),
                fragment: None,
            },
        };
        assert!(!stale.is_current_for(second, 41, 7));
        assert!(current.is_current_for(second, 41, 7));
        assert!(!current.is_current_for(second, 42, 7));
        assert!(!current.is_current_for(second, 41, 8));
    }

    #[test]
    fn cache_evicts_by_entry_count_and_keeps_document_revision_in_key() {
        let mut cache = PreviewCache::default();
        let content = PreviewContent::Text {
            label: "note.md".into(),
            title: "Title".into(),
            excerpt: "Excerpt".into(),
            fragment: None,
        };
        let key = |revision| CacheKey::Document {
            path: PathBuf::from(r"C:\notes\note.md"),
            document_id: 9,
            revision,
            fragment: None,
        };
        cache.insert(key(1), content.clone());
        assert!(cache.get(&key(1)).is_some());
        assert!(cache.get(&key(2)).is_none());
        for index in 0..CACHE_MAX_ENTRIES {
            cache.insert(key(100 + index as u64), content.clone());
        }
        assert!(cache.entries.len() <= CACHE_MAX_ENTRIES);
        assert!(cache.bytes <= CACHE_MAX_BYTES);
    }

    fn test_directory() -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mdlite-link-preview-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(windows)]
    fn create_directory_link(target: &Path, link: &Path) -> io::Result<()> {
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "New-Item -ItemType Junction -Path $env:MDLITE_PREVIEW_LINK -Target $env:MDLITE_PREVIEW_TARGET | Out-Null",
            ])
            .env("MDLITE_PREVIEW_LINK", link)
            .env("MDLITE_PREVIEW_TARGET", target)
            .output()?;
        if output.status.success() {
            Ok(())
        } else {
            Err(io::Error::other(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ))
        }
    }

    #[cfg(not(windows))]
    fn create_directory_link(target: &Path, link: &Path) -> io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn remove_directory_link(link: &Path) -> io::Result<()> {
        fs::remove_dir(link)
    }

    #[cfg(not(windows))]
    fn remove_directory_link(link: &Path) -> io::Result<()> {
        fs::remove_file(link)
    }
}
