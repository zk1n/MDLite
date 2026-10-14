//! Explicit external image upload. Run `UploadPlan::run` on a worker thread.
//! This module never writes document/asset bytes, fetches a URL, or retries an upload.
use crate::{
    document::{Document, DocumentId},
    process::{self, Cancellation, ProcessOutcome, ProcessRequest, ProcessResult},
    trust::{TrustStore, TrustToken, WorkspaceIdentity},
};
use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    time::Duration,
};

const CONFIG_LIMIT: u64 = 64 * 1024;
const OUTPUT_LIMIT: usize = 64 * 1024;
const URI_LIMIT: usize = 8192;
const ASSET_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct StorageAdapter {
    executable: PathBuf,
    arguments: Vec<String>,
    timeout: Duration,
    config: PathBuf,
    config_hash: [u8; 32],
}
impl StorageAdapter {
    /// Legacy storage.toml dialect: executable, repeated argument, optional timeout_ms.
    /// Strings support double-quoted escapes or literal single-quoted Windows paths.
    pub fn load(config: &Path) -> Result<Self, String> {
        let config = fs::canonicalize(config).map_err(|e| e.to_string())?;
        let bytes = read_config(&config)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| "storage.toml must be UTF-8")?
            .trim_start_matches('\u{feff}');
        let mut executable = None;
        let mut arguments = Vec::new();
        let mut timeout = None;
        for (index, line) in text.lines().enumerate() {
            let line = config_line(line)?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("Invalid storage setting on line {}", index + 1))?;
            match key.trim() {
                "executable" if executable.is_none() => executable = Some(quoted(value.trim())?),
                "argument" if arguments.len() < 256 => arguments.push(quoted(value.trim())?),
                "timeout_ms" if timeout.is_none() => {
                    let value = value.trim();
                    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                        return Err("timeout_ms must be a finite positive integer".into());
                    }
                    let ms = value.parse::<u64>().map_err(|_| "Invalid timeout_ms")?;
                    if !(1..=3_600_000).contains(&ms) {
                        return Err(
                            "timeout_ms must be 1..3600000; INFINITE is not supported".into()
                        );
                    }
                    timeout = Some(Duration::from_millis(ms));
                }
                _ => {
                    return Err(format!(
                        "Unknown, duplicate, or excessive storage setting on line {}",
                        index + 1
                    ));
                }
            }
        }
        let executable = executable.ok_or("storage executable is required")?;
        if executable.is_empty()
            || executable
                .chars()
                .any(|c| c.is_control() || matches!(c, '{' | '}'))
        {
            return Err("Invalid storage executable".into());
        }
        let executable = resolve_executable(
            &executable,
            config.parent().ok_or("Missing config directory")?,
        )?;
        let mut has_file = false;
        for argument in &arguments {
            if argument.chars().any(char::is_control) {
                return Err("Control characters are not allowed in adapter arguments".into());
            }
            let mut tail = argument.as_str();
            while let Some(start) = tail.find(['{', '}']) {
                tail = &tail[start..];
                if let Some(rest) = tail.strip_prefix("{file}") {
                    has_file = true;
                    tail = rest;
                } else if let Some(rest) = tail.strip_prefix("{revision}") {
                    tail = rest;
                } else {
                    return Err("Only {file} and {revision} placeholders are supported".into());
                }
            }
        }
        if !has_file {
            return Err("At least one argument must include {file}".into());
        }
        if arguments.iter().map(String::len).sum::<usize>() > 16 * 1024 {
            return Err("Adapter arguments exceed the supported budget".into());
        }
        Ok(Self {
            executable,
            arguments,
            timeout: timeout.unwrap_or(Duration::from_secs(120)),
            config,
            config_hash: Sha256::digest(&bytes).into(),
        })
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
    pub fn argument_templates(&self) -> &[String] {
        &self.arguments
    }
    fn current(&self) -> Result<(), String> {
        if <[u8; 32]>::from(Sha256::digest(read_config(&self.config)?)) != self.config_hash {
            return Err("Storage configuration changed; prepare and confirm again".into());
        }
        Ok(())
    }
    fn arguments(&self, target: &UploadTarget) -> Vec<OsString> {
        self.arguments
            .iter()
            .map(|argument| {
                // Expand once: braces inside the actual filename are literal.
                let mut output = OsString::new();
                let mut tail = argument.as_str();
                while let Some(start) = tail.find('{') {
                    output.push(&tail[..start]);
                    tail = &tail[start..];
                    if let Some(rest) = tail.strip_prefix("{file}") {
                        output.push(&target.asset);
                        tail = rest;
                    } else if let Some(rest) = tail.strip_prefix("{revision}") {
                        output.push(target.revision.to_string());
                        tail = rest;
                    } else {
                        unreachable!("validated template");
                    }
                }
                output.push(tail);
                output
            })
            .collect()
    }
}
fn read_config(path: &Path) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    file.take(CONFIG_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > CONFIG_LIMIT {
        return Err("storage.toml exceeds 64 KiB".into());
    }
    Ok(bytes)
}
fn config_line(line: &str) -> Result<&str, String> {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            escaped = true;
            continue;
        }
        if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if quote.is_none() && ch == '#' {
            return Ok(&line[..index]);
        }
    }
    if quote.is_some() {
        return Err("Unterminated storage string".into());
    }
    Ok(line)
}
fn quoted(value: &str) -> Result<String, String> {
    if value.starts_with('"') {
        return serde_json::from_str::<String>(value)
            .map_err(|_| "Invalid quoted storage string".into());
    }
    if let Some(value) = value.strip_prefix('\'').and_then(|s| s.strip_suffix('\''))
        && !value.contains('\'')
    {
        return Ok(value.into());
    }
    Err("Storage values must be quoted strings".into())
}
fn resolve_executable(value: &str, config_dir: &Path) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if !path
        .extension()
        .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
    {
        return Err(
            "Storage adapter must be an .exe; batch/script files are not launched implicitly"
                .into(),
        );
    }
    if path
        .file_name()
        .is_some_and(|s| s.eq_ignore_ascii_case("cmd.exe"))
    {
        return Err("Shell command strings are not supported".into());
    }
    let resolved = if path.is_absolute() {
        fs::canonicalize(path)
    } else if path.components().count() > 1 {
        fs::canonicalize(config_dir.join(path))
    } else {
        let local = config_dir.join(path);
        if local.is_file() {
            fs::canonicalize(local)
        } else {
            std::env::var_os("PATH")
                .and_then(|paths| {
                    std::env::split_paths(&paths)
                        .map(|dir| dir.join(path))
                        .find(|p| p.is_file())
                })
                .ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Storage executable not found",
                    )
                })
                .and_then(fs::canonicalize)
        }
    }
    .map_err(|e| e.to_string())?;
    if !resolved.is_file() {
        return Err("Storage executable must be a file".into());
    }
    Ok(resolved)
}

#[derive(Clone, Debug)]
pub struct UploadTarget {
    document_id: DocumentId,
    revision: u64,
    destination: Range<usize>,
    original: String,
    asset: PathBuf,
    asset_reference: PathBuf,
    semantic_destination: String,
    asset_hash: [u8; 32],
    kind: DestinationKind,
}
#[derive(Clone, Debug)]
enum DestinationKind {
    Markdown { angle: bool },
    Html,
    Reference { alt_prefix: String, title: String },
}
impl UploadTarget {
    /// `reference` is the selected image's original UTF-8 source range.
    pub fn capture(
        document: &Document,
        reference: Range<usize>,
        document_dir: &Path,
    ) -> Result<Self, String> {
        Self::capture_snapshot(
            document.id(),
            document.revision(),
            document.source(),
            reference,
            document_dir,
        )
    }
    /// Immutable UI capture for preparation and hashing on an owned worker thread.
    pub fn capture_snapshot(
        document_id: DocumentId,
        revision: u64,
        source: &str,
        reference: Range<usize>,
        document_dir: &Path,
    ) -> Result<Self, String> {
        let snippet = source
            .get(reference.clone())
            .filter(|s| s.len() <= 16 * 1024)
            .ok_or("Invalid or excessive image source range")?;
        let (relative, semantic, kind) = if let Some(image) =
            crate::image_reference::parse_legacy(snippet)
        {
            (
                image.destination_range,
                image.destination,
                DestinationKind::Html,
            )
        } else {
            let images: Vec<_> = Parser::new_ext(source, crate::markdown::parser_options())
                .into_offset_iter()
                .filter_map(|(event, range)| {
                    if let Event::Start(Tag::Image {
                        link_type,
                        dest_url,
                        title,
                        ..
                    }) = event
                        && range.start >= reference.start
                        && range.end <= reference.end
                    {
                        Some((range, link_type, dest_url.to_string(), title.to_string()))
                    } else {
                        None
                    }
                })
                .collect();
            if images.len() != 1 {
                return Err("Select one parsed image".into());
            }
            let (image, link_type, semantic, title) = &images[0];
            if *link_type == LinkType::Inline {
                let (range, destination) = markdown_destination(snippet)?;
                let angle =
                    range.start > 0 && snippet.as_bytes().get(range.start - 1) == Some(&b'<');
                (range, destination, DestinationKind::Markdown { angle })
            } else if matches!(
                link_type,
                LinkType::Reference | LinkType::Collapsed | LinkType::Shortcut
            ) {
                let mut image = image.clone();
                // pulldown's collapsed-image offset omits the final empty label.
                if *link_type == LinkType::Collapsed
                    && source.get(image.end..image.end + 2) == Some("[]")
                {
                    image.end += 2;
                }
                let raw = &source[image.clone()];
                let end = parsed_alt_end(source, image.clone())? - image.start;
                let alt_prefix = raw[..end].to_owned();
                let probe = format!("{alt_prefix}(<mdlite-storage-probe.png>)");
                let valid = Parser::new_ext(&probe, crate::markdown::parser_options()).into_offset_iter().any(|(event,range)| range == (0..probe.len()) && matches!(event,Event::Start(Tag::Image { dest_url,.. }) if dest_url.as_ref() == "mdlite-storage-probe.png"));
                if !valid {
                    return Err("Reference image cannot be converted safely".into());
                }
                (
                    image.start - reference.start..image.end - reference.start,
                    semantic.clone(),
                    DestinationKind::Reference {
                        alt_prefix,
                        title: title.clone(),
                    },
                )
            } else {
                return Err("Unresolved image reference".into());
            }
        };
        let destination = relative.start + reference.start..relative.end + reference.start;
        let original = source
            .get(destination.clone())
            .ok_or("Invalid destination range")?
            .to_owned();
        let asset =
            crate::assets::local_image_path(document_dir, &semantic).map_err(|e| e.to_string())?;
        let asset_reference = asset;
        let asset = fs::canonicalize(&asset_reference).map_err(|e| e.to_string())?;
        let asset_hash = hash_asset(&asset)?;
        Ok(Self {
            document_id,
            revision,
            destination,
            original,
            asset,
            asset_reference,
            semantic_destination: semantic,
            asset_hash,
            kind,
        })
    }
    pub fn asset(&self) -> &Path {
        &self.asset
    }
    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn destination_range(&self) -> Range<usize> {
        self.destination.clone()
    }
    pub fn reference_becomes_inline(&self) -> bool {
        matches!(self.kind, DestinationKind::Reference { .. })
    }
    fn current_hash_matches(&self) -> bool {
        fs::canonicalize(&self.asset_reference).is_ok_and(|path| path == self.asset)
            && hash_asset(&self.asset).is_ok_and(|hash| hash == self.asset_hash)
    }
}
fn parsed_alt_end(source: &str, image: Range<usize>) -> Result<usize, String> {
    let mut inside = false;
    let mut depth = 0usize;
    let mut end = image.start + 2;
    for (event, range) in
        Parser::new_ext(source, crate::markdown::parser_options()).into_offset_iter()
    {
        if !inside {
            if matches!(event, Event::Start(Tag::Image { .. })) && range.start == image.start {
                inside = true;
                depth = 1;
            }
            continue;
        }
        match event {
            Event::End(TagEnd::Image) => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            Event::Start(Tag::Image { .. }) => depth += 1,
            _ => {}
        }
        end = end.max(range.end);
    }
    if inside && source.as_bytes().get(end) == Some(&b']') {
        Ok(end + 1)
    } else {
        Err("Image alt source range unavailable".into())
    }
}
fn hash_asset(path: &Path) -> Result<[u8; 32], String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > ASSET_LIMIT {
        return Err("Local asset must be a regular file of at most 512 MiB".into());
    }
    let mut hash = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0; 16384];
    let mut reader = file.take(ASSET_LIMIT + 1);
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > ASSET_LIMIT {
            return Err("Local asset exceeded the size budget".into());
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
fn markdown_destination(source: &str) -> Result<(Range<usize>, String), String> {
    let images: Vec<_> = Parser::new_ext(source, crate::markdown::parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            if let Event::Start(Tag::Image {
                link_type: LinkType::Inline,
                dest_url,
                ..
            }) = event
            {
                Some((range, dest_url.to_string()))
            } else {
                None
            }
        })
        .collect();
    if images.len() != 1 {
        return Err("Select one inline Markdown or supported legacy image".into());
    }
    let (image, semantic) = &images[0];
    let raw = &source[image.clone()];
    let mut cursor = parsed_alt_end(source, image.clone())? - image.start;
    let bytes = raw.as_bytes();
    if bytes.get(cursor) != Some(&b'(') {
        return Err("Image destination range unavailable".into());
    }
    cursor += 1;
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    let (start, end) = if bytes.get(cursor) == Some(&b'<') {
        let start = cursor + 1;
        let end = raw[start..]
            .find('>')
            .map(|i| i + start)
            .ok_or("Unclosed image destination")?;
        (start, end)
    } else {
        let start = cursor;
        let mut nesting = 0usize;
        while cursor < bytes.len() {
            match bytes[cursor] {
                b'\\' => cursor += 2,
                b'(' => {
                    nesting += 1;
                    cursor += 1;
                }
                b')' if nesting != 0 => {
                    nesting -= 1;
                    cursor += 1;
                }
                b')' => break,
                c if c.is_ascii_whitespace() => break,
                _ => cursor += 1,
            }
        }
        (start, cursor)
    };
    if start == end || end > raw.len() || !raw.is_char_boundary(start) || !raw.is_char_boundary(end)
    {
        return Err("Image destination range unavailable".into());
    }
    // Validate our source locator through the canonical parser rather than silently
    // trusting bracket spellings inside complex alt text.
    let mut probe = raw.to_owned();
    probe.replace_range(start..end, "mdlite-storage-probe.png");
    let valid = Parser::new_ext(&probe, crate::markdown::parser_options()).any(|event| matches!(event, Event::Start(Tag::Image { dest_url, .. }) if dest_url.as_ref() == "mdlite-storage-probe.png"));
    if !valid {
        return Err("Image destination cannot be edited safely".into());
    }
    Ok((image.start + start..image.start + end, semantic.clone()))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UploadConsent {
    #[default]
    Declined,
    Confirmed,
}
pub struct UploadPlan {
    adapter: StorageAdapter,
    target: UploadTarget,
    workspace: PathBuf,
    token: TrustToken,
    generation: u64,
}
pub struct ConfirmationDetails<'a> {
    pub executable: &'a Path,
    pub arguments: Vec<OsString>,
    pub workspace: &'a Path,
    pub asset: &'a Path,
    pub document_id: DocumentId,
    pub revision: u64,
    /// The dialog must disclose this selected-image conversion; shared definitions stay unchanged.
    pub reference_becomes_inline: bool,
}
impl UploadPlan {
    pub fn prepare(
        adapter: StorageAdapter,
        target: UploadTarget,
        workspace: &Path,
        trust: &TrustStore,
        generation: u64,
    ) -> Result<Self, String> {
        let token = trust.token(workspace)?;
        Ok(Self {
            adapter,
            target,
            workspace: token.identity().canonical.clone(),
            token,
            generation,
        })
    }
    /// Display these exact targets and arguments in a default-No dialog before confirming.
    pub fn confirmation(&self) -> ConfirmationDetails<'_> {
        ConfirmationDetails {
            executable: &self.adapter.executable,
            arguments: self.adapter.arguments(&self.target),
            workspace: &self.workspace,
            asset: &self.target.asset,
            document_id: self.target.document_id,
            revision: self.target.revision,
            reference_becomes_inline: self.target.reference_becomes_inline(),
        }
    }
    pub fn run(
        &self,
        trust: &TrustStore,
        consent: UploadConsent,
        cancellation: &Cancellation,
    ) -> UploadRun {
        let rejected = |reason: String| UploadRun {
            outcome: UploadOutcome::Rejected(reason),
            process: None,
        };
        if consent != UploadConsent::Confirmed {
            return UploadRun {
                outcome: UploadOutcome::Declined,
                process: None,
            };
        }
        if cancellation.is_cancelled() {
            return UploadRun {
                outcome: UploadOutcome::Cancelled,
                process: None,
            };
        }
        let _lease = match trust.begin_mutation(&self.token, &self.workspace) {
            Ok(v) => v,
            Err(e) => return rejected(e),
        };
        if let Err(e) = self.adapter.current() {
            return rejected(e);
        }
        if !self.target.current_hash_matches() {
            return rejected("Local asset changed or became unreadable before upload".into());
        }
        if let Err(e) = trust.check(&self.token, &self.workspace) {
            return rejected(e);
        }
        let result = process::run(&ProcessRequest {
            executable: self.adapter.executable.clone(),
            arguments: self.adapter.arguments(&self.target),
            cwd: self
                .target
                .asset
                .parent()
                .expect("canonical file parent")
                .to_owned(),
            timeout: self.adapter.timeout,
            output_limit: OUTPUT_LIMIT,
            generation: self.generation,
            cancellation: cancellation.clone(),
            environment: vec![],
            remove_environment: vec![],
        });
        let outcome = match result.outcome {
            ProcessOutcome::Cancelled => UploadOutcome::Cancelled,
            ProcessOutcome::TimedOut => UploadOutcome::TimedOut,
            ProcessOutcome::Exited(0) if !result.truncated => {
                match first_https_line(&result.stdout) {
                    Ok(reference) => {
                        if !self.target.current_hash_matches() {
                            UploadOutcome::RemoteSucceededButNotApplied(ApplyFailure {
                                reference,
                                reason: ApplyRejection::LocalAssetChanged,
                            })
                        } else {
                            UploadOutcome::RemoteSucceeded(Box::new(PendingUpload {
                                target: self.target.clone(),
                                reference,
                                cancellation: cancellation.clone(),
                                generation: self.generation,
                                workspace: self.token.identity().clone(),
                            }))
                        }
                    }
                    Err(e) => UploadOutcome::InvalidResponse(e),
                }
            }
            ProcessOutcome::Exited(0) => {
                UploadOutcome::InvalidResponse("Adapter output was truncated".into())
            }
            _ => UploadOutcome::Failed,
        };
        UploadRun {
            outcome,
            process: Some(result),
        }
    }
}
pub struct UploadRun {
    pub outcome: UploadOutcome,
    pub process: Option<ProcessResult>,
}
pub enum UploadOutcome {
    Declined,
    Rejected(String),
    Cancelled,
    TimedOut,
    Failed,
    InvalidResponse(String),
    RemoteSucceeded(Box<PendingUpload>),
    RemoteSucceededButNotApplied(ApplyFailure),
}
/// Retain this receipt if native application fails; remote success must not trigger an automatic retry.
pub struct PendingUpload {
    target: UploadTarget,
    reference: String,
    cancellation: Cancellation,
    generation: u64,
    workspace: WorkspaceIdentity,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyRejection {
    CancelledAfterRemoteSuccess,
    StaleDocument,
    StaleWorkspace,
    DestinationChanged,
    LocalAssetChanged,
}
#[derive(Debug)]
pub struct ApplyFailure {
    pub reference: String,
    pub reason: ApplyRejection,
}
#[derive(Debug)]
pub struct DestinationEdit {
    pub document_id: DocumentId,
    pub revision: u64,
    pub range: Range<usize>,
    pub expected: String,
    pub replacement: String,
}
impl PendingUpload {
    pub fn reference(&self) -> &str {
        &self.reference
    }
    /// Call immediately before the native single-Undo edit. No document is changed here.
    pub fn edit(
        &self,
        document: &Document,
        generation: u64,
        workspace: &Path,
        document_dir: &Path,
    ) -> Result<DestinationEdit, ApplyFailure> {
        self.edit_snapshot(
            document.id(),
            document.revision(),
            document.source(),
            generation,
            workspace,
            document_dir,
        )
    }
    /// Validate a current immutable UI capture on a worker; UI must then recheck
    /// returned id/revision/expected source before its single-Undo transaction.
    pub fn edit_snapshot(
        &self,
        document_id: DocumentId,
        revision: u64,
        source: &str,
        generation: u64,
        workspace: &Path,
        document_dir: &Path,
    ) -> Result<DestinationEdit, ApplyFailure> {
        let failure = |reason| ApplyFailure {
            reference: self.reference.clone(),
            reason,
        };
        if self.cancellation.is_cancelled() {
            return Err(failure(ApplyRejection::CancelledAfterRemoteSuccess));
        }
        if generation != self.generation
            || !WorkspaceIdentity::read(workspace).is_ok_and(|i| i == self.workspace)
        {
            return Err(failure(ApplyRejection::StaleWorkspace));
        }
        if document_id != self.target.document_id || revision != self.target.revision {
            return Err(failure(ApplyRejection::StaleDocument));
        }
        if source.get(self.target.destination.clone()) != Some(self.target.original.as_str()) {
            return Err(failure(ApplyRejection::DestinationChanged));
        }
        let current_asset =
            crate::assets::local_image_path(document_dir, &self.target.semantic_destination)
                .and_then(fs::canonicalize);
        if !current_asset.is_ok_and(|path| path == self.target.asset)
            || !self.target.current_hash_matches()
        {
            return Err(failure(ApplyRejection::LocalAssetChanged));
        }
        let mut replacement = self.reference.replace('&', "&amp;");
        match &self.target.kind {
            DestinationKind::Html => replacement = replacement.replace('\'', "&apos;"),
            DestinationKind::Markdown { angle: false } => {
                replacement = replacement.replace('(', "\\(").replace(')', "\\)")
            }
            DestinationKind::Markdown { angle: true } => {}
            DestinationKind::Reference { alt_prefix, title } => {
                let title = if title.is_empty() {
                    String::new()
                } else {
                    format!(
                        " \"{}\"",
                        title
                            .replace('&', "&amp;")
                            .replace('"', "&quot;")
                            .replace('\\', "&#92;")
                            .replace('<', "&lt;")
                            .replace('>', "&gt;")
                            .replace('\n', "&#10;")
                            .replace('\r', "&#13;")
                    )
                };
                replacement = format!("{alt_prefix}(<{replacement}>{title})");
            }
        }
        Ok(DestinationEdit {
            document_id: self.target.document_id,
            revision: self.target.revision,
            range: self.target.destination.clone(),
            expected: self.target.original.clone(),
            replacement,
        })
    }
}

fn first_https_line(bytes: &[u8]) -> Result<String, String> {
    let line = bytes.split(|b| *b == b'\n').next().unwrap_or_default();
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    if line.is_empty() || line.len() > URI_LIMIT {
        return Err("First stdout line must be an HTTPS URI of at most 8192 bytes".into());
    }
    let reference = std::str::from_utf8(line).map_err(|_| "Adapter URI must be UTF-8")?;
    if !valid_https_uri(reference) {
        return Err(
            "Adapter must return a valid absolute HTTPS URI on the first stdout line".into(),
        );
    }
    Ok(reference.to_owned())
}
/// RFC 3986 ASCII URI syntax, with HTTPS authority and no user information.
pub fn valid_https_uri(uri: &str) -> bool {
    if uri.len() > URI_LIMIT
        || !uri.is_ascii()
        || !uri
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
    {
        return false;
    }
    let authority_end = uri[8..].find(['/', '?', '#']).map_or(uri.len(), |i| i + 8);
    let authority = &uri[8..authority_end];
    if authority.is_empty() || authority.contains('@') {
        return false;
    }
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let Some(end) = rest.find(']') else {
            return false;
        };
        let host = &rest[..end];
        if !valid_ip_literal(host) {
            return false;
        }
        let suffix = &rest[end + 1..];
        if suffix.is_empty() {
            (host, None)
        } else if let Some(port) = suffix.strip_prefix(':') {
            (host, Some(port))
        } else {
            return false;
        }
    } else {
        let (host, port) = authority
            .split_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)));
        if host.is_empty() || !uri_chars(host, false) {
            return false;
        }
        (host, port)
    };
    if host.is_empty()
        || port.is_some_and(|p| {
            p.is_empty()
                || !p.bytes().all(|b| b.is_ascii_digit())
                || p.parse::<u16>().map_or(true, |n| n == 0)
        })
    {
        return false;
    }
    let remainder = &uri[authority_end..];
    remainder.bytes().filter(|b| *b == b'#').count() <= 1 && uri_chars(remainder, true)
}
fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)
}
fn uri_chars(value: &str, remainder: bool) -> bool {
    let bytes = value.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len()
                || !bytes[i + 1].is_ascii_hexdigit()
                || !bytes[i + 2].is_ascii_hexdigit()
            {
                return false;
            }
            i += 3;
        } else if unreserved(bytes[i])
            || b"!$&'()*+,;=".contains(&bytes[i])
            || (remainder && b"/:@?#".contains(&bytes[i]))
        {
            i += 1;
        } else {
            return false;
        }
    }
    true
}
fn valid_ip_literal(host: &str) -> bool {
    if host.starts_with(['v', 'V']) {
        let Some((version, name)) = host[1..].split_once('.') else {
            return false;
        };
        return !version.is_empty()
            && version.bytes().all(|b| b.is_ascii_hexdigit())
            && !name.is_empty()
            && name
                .bytes()
                .all(|b| unreserved(b) || b"!$&'()*+,;=:".contains(&b));
    }
    let address = if let Some((address, zone)) = host.split_once("%25") {
        if zone.is_empty() || !uri_chars(zone, false) {
            return false;
        }
        address
    } else {
        host
    };
    address.parse::<std::net::Ipv6Addr>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        asset: PathBuf,
        config: PathBuf,
        marker: PathBuf,
        executable: PathBuf,
        trust: TrustStore,
    }
    impl Fixture {
        fn new(compile_mock: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "mdlite-storage-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&root).unwrap();
            let asset = root.join("画像 {revision} asset.png");
            fs::write(&asset, b"unchanged local asset").unwrap();
            let executable = if compile_mock {
                let code = root.join("mock.rs");
                let executable = root.join("mock-adapter.exe");
                fs::write(&code, r#"use std::{io::Write,time::Duration};
fn main(){let a:Vec<_>=std::env::args_os().collect();std::fs::write(&a[4],format!("{}\n{}",a[1].to_string_lossy(),a[2].to_string_lossy())).unwrap();
match a[3].to_str().unwrap(){
"fail"=>{eprintln!("fixture failure");std::process::exit(9)},
"sleep"=>std::thread::sleep(Duration::from_secs(60)),
"bad"=>println!("https:///missing-host"),
"huge"=>{println!("https://example.invalid/asset");std::io::stderr().write_all(&vec![b'e';131072]).unwrap();},
"long"=>println!("https://example.invalid/{}","a".repeat(9000)),
"hash"=>{std::fs::write(&a[1],b"adapter changed local bytes").unwrap();println!("https://example.invalid/asset");},
_=>println!("https://example.invalid/a(b).png?x=1&y=2"),}}
"#).unwrap();
                let output = std::process::Command::new("rustc.exe")
                    .args(["--edition", "2024", "--crate-name", "mdlite_storage_mock"])
                    .arg(&code)
                    .arg("-o")
                    .arg(&executable)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
                executable
            } else {
                std::env::current_exe().unwrap()
            };
            let config = root.join("storage.toml");
            let marker = root.join("adapter-ran.txt");
            let trust = TrustStore::at(root.join("registry"));
            trust.set_trusted(&root, true).unwrap();
            let fixture = Self {
                root,
                asset,
                config,
                marker,
                executable,
                trust,
            };
            fixture.adapter("success", 3000);
            fixture
        }
        fn adapter(&self, mode: &str, timeout: u64) -> StorageAdapter {
            let q = |s: &str| serde_json::to_string(s).unwrap();
            let text = format!(
                "executable = {}\nargument = \"{{file}}\"\nargument = \"{{revision}}\"\nargument = {}\nargument = {}\ntimeout_ms = {timeout}\n",
                q(self.executable.to_str().unwrap()),
                q(mode),
                q(self.marker.to_str().unwrap())
            );
            fs::write(&self.config, text).unwrap();
            StorageAdapter::load(&self.config).unwrap()
        }
        fn source(&self) -> String {
            format!(
                "![alt](<{}> \"keep title\") <!-- mdlite-width:480 -->",
                self.asset.file_name().unwrap().to_str().unwrap()
            )
        }
        fn plan(&self, adapter: StorageAdapter, document: &Document) -> UploadPlan {
            let target =
                UploadTarget::capture(document, 0..document.source().len(), &self.root).unwrap();
            UploadPlan::prepare(adapter, target, &self.root, &self.trust, 41).unwrap()
        }
        fn clear_marker(&self) {
            if self.marker.exists() {
                fs::remove_file(&self.marker).unwrap();
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn configuration_snapshot_validation_and_literal_expansion() {
        let f = Fixture::new(false);
        let adapter = f.adapter("success", 120000);
        let source = f.source();
        let document = Document::new(7, source);
        let plan = f.plan(adapter, &document);
        let details = plan.confirmation();
        assert_eq!(
            details.arguments[0],
            fs::canonicalize(&f.asset).unwrap().as_os_str()
        );
        assert_eq!(details.arguments[1], "0");
        assert_eq!(details.document_id, 7);
        assert_eq!(details.asset, fs::canonicalize(&f.asset).unwrap());
        assert_eq!(UploadConsent::default(), UploadConsent::Declined);
        let original = fs::read_to_string(&f.config).unwrap();
        for text in [
            original.replace("120000", "4294967295"),
            original.replace("120000", "0"),
            original.replace("120000", "-1"),
            original.replace("120000", "never"),
            original.replace("{file}", "{files}"),
            original.replace("{file}", "fixed.png"),
            format!("{original}executable = \"other.exe\"\n"),
            format!("{original}unknown = true\n"),
            original.replace("executable =", "executable_extra ="),
            "executable = 'script.cmd'\nargument = '{file}'".into(),
            "executable = 'cmd.exe'\nargument = '{file}'".into(),
        ] {
            fs::write(&f.config, text).unwrap();
            assert!(StorageAdapter::load(&f.config).is_err());
        }
        fs::write(&f.config, &original).unwrap();
        fs::write(&f.config, format!("\u{feff}# comment\n{original}# after\n")).unwrap();
        assert!(StorageAdapter::load(&f.config).is_ok());
        fs::write(&f.config, vec![b'x'; CONFIG_LIMIT as usize + 1]).unwrap();
        assert!(StorageAdapter::load(&f.config).is_err());
    }
    #[test]
    fn uri_validation_rejects_unsafe_and_accepts_encoded_https() {
        for bad in [
            "",
            "http://example.invalid/a",
            "https:///a",
            "https://user@host/a",
            "https://host:0/a",
            "https://host:65536/a",
            "https://host:abc/a",
            "https://[garbage]/a",
            "https://host[name/a",
            "https://host/a b",
            "https://host/a%2G",
            "https://host/a{b}",
            "https://host/\n",
            "https://host/a#b#c",
        ] {
            assert!(!valid_https_uri(bad), "{bad:?}");
        }
        for good in [
            "https://example.invalid/a%20b.png?x=1&y=2",
            "HTTPS://[2001:db8::1]:443/a",
            "https://[fe80::1%25eth0]/a",
            "https://[v1.host]:443/a",
            "https://example.invalid/a(b).png",
        ] {
            assert!(valid_https_uri(good), "{good}");
        }
        assert!(first_https_line(b"https://example.invalid/a\r\nextra\n").is_ok());
        assert!(first_https_line(b"\nhttps://example.invalid/a").is_err());
    }
    #[test]
    fn destination_edits_preserve_markdown_and_legacy_attributes_one_undo() {
        let f = Fixture::new(false);
        let name = f.asset.file_name().unwrap().to_str().unwrap();
        for source in [
            f.source(),
            format!("<img src='{name}' alt='a &amp; b' title=\"keep\" width='480'>"),
        ] {
            let mut document = Document::new(22, source.clone());
            let target = UploadTarget::capture(&document, 0..source.len(), &f.root).unwrap();
            let snapshot_target = UploadTarget::capture_snapshot(
                document.id(),
                document.revision(),
                document.source(),
                0..source.len(),
                &f.root,
            )
            .unwrap();
            assert_eq!(target.destination, snapshot_target.destination);
            assert_eq!(target.asset_hash, snapshot_target.asset_hash);
            assert_eq!(target.original, snapshot_target.original);
            let pending = PendingUpload {
                target,
                reference: "https://example.invalid/a(b)'s.png?x=1&y=2".into(),
                cancellation: Cancellation::default(),
                generation: 41,
                workspace: WorkspaceIdentity::read(&f.root).unwrap(),
            };
            let edit = pending.edit(&document, 41, &f.root, &f.root).unwrap();
            let snapshot_edit = pending
                .edit_snapshot(
                    document.id(),
                    document.revision(),
                    document.source(),
                    41,
                    &f.root,
                    &f.root,
                )
                .unwrap();
            assert_eq!(edit.range, snapshot_edit.range);
            assert_eq!(edit.replacement, snapshot_edit.replacement);
            assert_eq!(edit.expected, snapshot_edit.expected);
            document
                .replace_range(edit.range, &edit.replacement)
                .unwrap();
            assert_eq!(document.undo_count(), 1);
            if source.starts_with('<') {
                let parsed = crate::image_reference::parse_legacy(document.source()).unwrap();
                assert_eq!(parsed.alt, "a & b");
                assert_eq!(parsed.title.as_deref(), Some("keep"));
                assert_eq!(parsed.width, Some(480));
                assert_eq!(parsed.destination, pending.reference);
            } else {
                assert!(document.source().starts_with("![alt](<https://"));
                assert!(
                    document
                        .source()
                        .ends_with(" \"keep title\") <!-- mdlite-width:480 -->")
                );
                let image = Parser::new_ext(document.source(), crate::markdown::parser_options())
                    .find_map(|e| {
                        if let Event::Start(Tag::Image { dest_url, .. }) = e {
                            Some(dest_url.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap();
                assert_eq!(image, "https://example.invalid/a(b)'s.png?x=1&y=2");
            }
            assert!(document.undo());
            assert_eq!(document.source(), source);
            assert_eq!(fs::read(&f.asset).unwrap(), b"unchanged local asset");
        }
        let nested = "![a [nested] alt](a\\(1\\).png \"t\")";
        let (range, semantic) = markdown_destination(nested).unwrap();
        assert_eq!(&nested[range], "a\\(1\\).png");
        assert_eq!(semantic, "a(1).png");
        assert!(markdown_destination("![alt][id]\n\n[id]: local.png\n").is_err());
    }
    #[test]
    fn selected_reference_conversion_preserves_shared_definition_title_width_and_other_image() {
        let f = Fixture::new(false);
        let name = f.asset.file_name().unwrap().to_str().unwrap();
        for token in ["![raw *alt*][shared]", "![shared][]", "![shared]"] {
            let source = format!(
                "{token} <!-- mdlite-width:480 -->\n\n![other][shared]\n\n[shared]: <{name}> 'resolved &amp; title'\n"
            );
            let mut document = Document::new(51, source.clone());
            let target = UploadTarget::capture(&document, 0..token.len(), &f.root).unwrap();
            assert!(target.reference_becomes_inline());
            let plan = UploadPlan::prepare(
                f.adapter("success", 3000),
                target.clone(),
                &f.root,
                &f.trust,
                1,
            )
            .unwrap();
            assert!(plan.confirmation().reference_becomes_inline);
            let pending = PendingUpload {
                target,
                reference: "https://example.invalid/a(b).png?x=1&y=2".into(),
                cancellation: Cancellation::default(),
                generation: 41,
                workspace: WorkspaceIdentity::read(&f.root).unwrap(),
            };
            let edit = pending.edit(&document, 41, &f.root, &f.root).unwrap();
            assert_eq!(edit.range, 0..token.len());
            document
                .replace_range(edit.range, &edit.replacement)
                .unwrap();
            assert_eq!(document.undo_count(), 1);
            assert!(document.source().ends_with(&source[token.len()..]));
            let images: Vec<_> =
                Parser::new_ext(document.source(), crate::markdown::parser_options())
                    .filter_map(|event| {
                        if let Event::Start(Tag::Image {
                            dest_url, title, ..
                        }) = event
                        {
                            Some((dest_url.to_string(), title.to_string()))
                        } else {
                            None
                        }
                    })
                    .collect();
            assert_eq!(images.len(), 2);
            assert_eq!(images[0].0, pending.reference);
            assert_eq!(images[0].1, "resolved & title");
            assert_eq!(images[1].0, name);
            assert_eq!(images[1].1, "resolved & title");
            let expected_prefix = token.split("][").next().unwrap().trim_end_matches(']');
            assert!(document.source().starts_with(expected_prefix));
            assert!(document.undo());
            assert_eq!(document.source(), source);
        }
    }
    #[test]
    #[cfg(windows)]
    fn mock_executable_gates_failures_cancel_hash_and_stale_remote_success() {
        let f = Fixture::new(true);
        let mut document = Document::new(13, f.source());
        let cancellation = Cancellation::default();
        let plan = f.plan(f.adapter("success", 3000), &document);
        assert!(matches!(
            plan.run(&f.trust, UploadConsent::default(), &cancellation)
                .outcome,
            UploadOutcome::Declined
        ));
        assert!(!f.marker.exists());
        f.trust.set_trusted(&f.root, false).unwrap();
        assert!(
            UploadPlan::prepare(
                f.adapter("success", 3000),
                UploadTarget::capture(&document, 0..document.source().len(), &f.root).unwrap(),
                &f.root,
                &f.trust,
                41
            )
            .is_err()
        );
        assert!(matches!(
            plan.run(&f.trust, UploadConsent::Confirmed, &cancellation)
                .outcome,
            UploadOutcome::Rejected(_)
        ));
        assert!(!f.marker.exists());
        f.trust.set_trusted(&f.root, true).unwrap();
        let plan = f.plan(f.adapter("success", 3000), &document);
        fs::write(&f.config, "changed after confirmation").unwrap();
        assert!(matches!(
            plan.run(&f.trust, UploadConsent::Confirmed, &cancellation)
                .outcome,
            UploadOutcome::Rejected(_)
        ));
        assert!(!f.marker.exists());
        for mode in ["fail", "bad", "huge", "long"] {
            f.clear_marker();
            let plan = f.plan(f.adapter(mode, 3000), &document);
            let result = plan.run(&f.trust, UploadConsent::Confirmed, &cancellation);
            assert!(f.marker.exists());
            if mode == "fail" {
                assert!(matches!(result.outcome, UploadOutcome::Failed));
                assert_eq!(result.process.unwrap().outcome, ProcessOutcome::Exited(9));
            } else {
                assert!(matches!(result.outcome, UploadOutcome::InvalidResponse(_)));
                if mode == "huge" {
                    assert!(result.process.unwrap().truncated);
                }
            }
            assert_eq!(document.source(), f.source());
            assert_eq!(fs::read(&f.asset).unwrap(), b"unchanged local asset");
        }
        let result = f.plan(f.adapter("sleep", 100), &document).run(
            &f.trust,
            UploadConsent::Confirmed,
            &cancellation,
        );
        assert!(matches!(result.outcome, UploadOutcome::TimedOut));
        f.clear_marker();
        let plan = f.plan(f.adapter("sleep", 3000), &document);
        let cancellation = Cancellation::default();
        let trigger = cancellation.clone();
        let marker = f.marker.clone();
        let thread = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while !marker.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            trigger.cancel();
        });
        assert!(matches!(
            plan.run(&f.trust, UploadConsent::Confirmed, &cancellation)
                .outcome,
            UploadOutcome::Cancelled
        ));
        thread.join().unwrap();
        assert!(f.marker.exists());
        let cancellation = Cancellation::default();
        let plan = f.plan(f.adapter("hash", 3000), &document);
        let result = plan.run(&f.trust, UploadConsent::Confirmed, &cancellation);
        assert!(matches!(
            result.outcome,
            UploadOutcome::RemoteSucceededButNotApplied(ApplyFailure {
                reason: ApplyRejection::LocalAssetChanged,
                ..
            })
        ));
        assert_eq!(document.source(), f.source());
        fs::write(&f.asset, b"unchanged local asset").unwrap();
        let plan = f.plan(f.adapter("success", 3000), &document);
        let result = plan.run(&f.trust, UploadConsent::Confirmed, &cancellation);
        let UploadOutcome::RemoteSucceeded(pending) = result.outcome else {
            panic!("Mock success was rejected");
        };
        assert_eq!(result.process.unwrap().generation, 41);
        assert!(pending.edit(&document, 41, &f.root, &f.root).is_ok());
        assert_eq!(
            pending
                .edit(&document, 41, &f.root, &f.root.join("registry"))
                .unwrap_err()
                .reason,
            ApplyRejection::LocalAssetChanged
        );
        assert_eq!(
            pending
                .edit(&document, 42, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::StaleWorkspace
        );
        assert_eq!(
            pending
                .edit(&document, 41, &f.root.join("registry"), &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::StaleWorkspace
        );
        let wrong_owner = Document::new(14, document.source().into());
        assert_eq!(
            pending
                .edit(&wrong_owner, 41, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::StaleDocument
        );
        let changed_destination = Document::new(13, document.source().replace("画像", "変更"));
        assert_eq!(
            pending
                .edit(&changed_destination, 41, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::DestinationChanged
        );
        fs::write(&f.asset, b"changed after remote success").unwrap();
        assert_eq!(
            pending
                .edit(&document, 41, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::LocalAssetChanged
        );
        fs::write(&f.asset, b"unchanged local asset").unwrap();
        document.replace_range(0..0, "changed").unwrap();
        assert_eq!(
            pending
                .edit(&document, 41, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::StaleDocument
        );
        cancellation.cancel();
        assert_eq!(
            pending
                .edit(&document, 41, &f.root, &f.root)
                .unwrap_err()
                .reason,
            ApplyRejection::CancelledAfterRemoteSuccess
        );
    }
}
