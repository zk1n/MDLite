//! Workspace-wide replacement plans and restart-safe journals for closed files.
//!
//! Open-document edits are returned to the native owner with their source,
//! revision, and generation guard. Closed files are saved only after a durable
//! journal exists and every original snapshot has been rechecked.

use crate::{
    document::{Document, Selection},
    search::{self, Query, SearchState},
    storage::{FileFormat, FileSnapshot, StorageError},
    workspace::Workspace,
    workspace_search::{self, WorkspaceResults},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REPLACEMENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_REPLACEMENTS: usize = 20_000;
const MAX_JOURNAL_BYTES: u64 = 128 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 128 * 1024;
const MAX_LEGACY_PATH_UNITS: usize = 32_767;
const V1_MAGIC: &[u8] = b"MDLITE_REPLACE_JOURNAL_V1";
const V2_MAGIC: &[u8] = b"MDLITE_REPLACE_JOURNAL_V2\r\n";
const V3_MAGIC: &[u8] = b"MDLITE_REPLACE_JOURNAL_V3\r\n";
static NEXT_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub struct ReplaceError(String);

impl ReplaceError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl std::fmt::Display for ReplaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ReplaceError {}

impl From<io::Error> for ReplaceError {
    fn from(value: io::Error) -> Self {
        Self(value.to_string())
    }
}

impl From<StorageError> for ReplaceError {
    fn from(value: StorageError) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug)]
pub struct OpenDocumentEdit {
    pub path: PathBuf,
    pub document_id: u64,
    pub revision: u64,
    pub generation: u64,
    pub expected_source: Arc<str>,
    pub after_source: String,
    pub selection_after: Selection,
    pub replacement_count: usize,
}

impl OpenDocumentEdit {
    /// The native owner must check this guard after confirmation and before its
    /// single `Document::replace_range_with_selection` transaction.
    pub fn matches_current(
        &self,
        document_id: u64,
        revision: u64,
        generation: u64,
        source: &str,
    ) -> bool {
        self.document_id == document_id
            && self.revision == revision
            && self.generation == generation
            && self.expected_source.as_ref() == source
    }
}

#[derive(Debug)]
pub struct ClosedFileEdit {
    pub path: PathBuf,
    pub before_source: String,
    pub after_source: String,
    pub format: FileFormat,
    pub before_fingerprint: String,
    pub replacement_count: usize,
}

#[derive(Debug)]
pub struct ReplacePlan {
    root: PathBuf,
    generation: u64,
    open: Vec<OpenDocumentEdit>,
    closed: Vec<ClosedFileEdit>,
    replacement_count: usize,
    output_bytes: usize,
}

impl ReplacePlan {
    pub fn root(&self) -> &Path {
        &self.root
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub fn open_edits(&self) -> &[OpenDocumentEdit] {
        &self.open
    }

    pub fn closed_edits(&self) -> &[ClosedFileEdit] {
        &self.closed
    }

    pub const fn replacement_count(&self) -> usize {
        self.replacement_count
    }

    pub const fn output_bytes(&self) -> usize {
        self.output_bytes
    }
}

/// Build immutable replacement output from the exact sources in search results.
/// The search is repeated against those immutable sources to use the canonical
/// `Results::replace` capture formatter without editing a live document.
pub fn preview_workspace_replace(
    results: &WorkspaceResults,
    query: &Query,
    replacement: &str,
    generation: u64,
    workspace: &Workspace,
    cancel: &AtomicBool,
) -> Result<ReplacePlan, ReplaceError> {
    if cancel.load(Ordering::Relaxed) {
        return Err(ReplaceError::new("replacement preview was cancelled"));
    }
    if !results.complete || results.generation != generation || results.root != workspace.root() {
        return Err(ReplaceError::new(
            "replacement search results are incomplete or stale",
        ));
    }
    workspace_search::validate_replace_plan(results, generation, workspace)
        .map_err(ReplaceError::new)?;
    if replacement.len() > MAX_REPLACEMENT_BYTES {
        return Err(ReplaceError::new(
            "replacement text exceeds the 64 MiB plan limit",
        ));
    }
    if replacement.contains('\0') {
        return Err(ReplaceError::new(
            "replacement text contains a NUL character",
        ));
    }

    let mut plan = ReplacePlan {
        root: workspace.root().to_path_buf(),
        generation,
        open: Vec::new(),
        closed: Vec::new(),
        replacement_count: 0,
        output_bytes: 0,
    };
    let mut seen_paths = std::collections::HashSet::new();
    let mut seen_physical = std::collections::HashSet::new();
    let mut seen_document_ids = std::collections::HashSet::new();
    let mut plan_bytes = 0usize;

    for file in &results.files {
        if cancel.load(Ordering::Relaxed) {
            return Err(ReplaceError::new("replacement preview was cancelled"));
        }
        let matches = &file.matches;
        if matches.generation != generation
            || matches.query != *query
            || matches.state != SearchState::Complete
            || matches.hits.is_empty()
        {
            return Err(ReplaceError::new(
                "a replacement result is incomplete or stale",
            ));
        }
        let path = safe_workspace_file(workspace, &file.path)?;
        if !seen_paths.insert(path_key(&path)) {
            return Err(ReplaceError::new(
                "replacement results contain a duplicate path",
            ));
        }
        let physical = same_file::Handle::from_path(&path)
            .map_err(|e| ReplaceError::new(format!("cannot identify replacement target: {e}")))?;
        if !seen_physical.insert(physical) {
            return Err(ReplaceError::new(
                "replacement results contain a duplicate physical file",
            ));
        }
        if file.snapshot.is_none() && !seen_document_ids.insert(matches.document_id) {
            return Err(ReplaceError::new(
                "replacement results repeat an open document id",
            ));
        }
        let Some(path_text) = path.to_str() else {
            return Err(ReplaceError::new(
                "replacement path cannot be represented in the journal format",
            ));
        };
        if path_text.len() > MAX_PATH_BYTES {
            return Err(ReplaceError::new(
                "replacement path exceeds the journal path limit",
            ));
        }
        if path_text.encode_utf16().count() > MAX_LEGACY_PATH_UNITS {
            return Err(ReplaceError::new(
                "replacement path exceeds the Windows path limit",
            ));
        }
        let (after_source, selection_after, count) = replacement_output(
            matches,
            query,
            replacement,
            generation,
            file.snapshot.is_some(),
            cancel,
        )?;
        if count != matches.hits.len() || count == 0 {
            return Err(ReplaceError::new(
                "replacement count differs from its search results",
            ));
        }
        plan.replacement_count = plan
            .replacement_count
            .checked_add(count)
            .filter(|total| *total <= MAX_REPLACEMENTS)
            .ok_or_else(|| ReplaceError::new("replacement count exceeds 20,000"))?;
        plan.output_bytes = plan
            .output_bytes
            .checked_add(after_source.len())
            .filter(|total| *total <= MAX_REPLACEMENT_BYTES)
            .ok_or_else(|| ReplaceError::new("replacement output exceeds 64 MiB"))?;
        let item_plan_bytes = path
            .to_string_lossy()
            .len()
            .checked_add(matches.source.len())
            .and_then(|total| total.checked_add(after_source.len()))
            .ok_or_else(|| ReplaceError::new("replacement plan size overflow"))?;
        plan_bytes = plan_bytes
            .checked_add(item_plan_bytes)
            .filter(|total| *total <= MAX_REPLACEMENT_BYTES)
            .ok_or_else(|| ReplaceError::new("replacement plan exceeds 64 MiB"))?;

        if let Some(snapshot) = &file.snapshot {
            if matches.document_id != 0 || matches.revision != 0 {
                return Err(ReplaceError::new(
                    "closed-file result has a live document identity",
                ));
            }
            let snapshot_path = safe_workspace_file(workspace, snapshot.path())?;
            if path_key(&snapshot_path) != path_key(&path) {
                return Err(ReplaceError::new(
                    "closed-file snapshot path differs from its result",
                ));
            }
            plan.closed.push(ClosedFileEdit {
                path,
                before_source: matches.source.to_string(),
                after_source,
                format: snapshot.format(),
                before_fingerprint: snapshot.baseline_sha256(),
                replacement_count: count,
            });
        } else {
            plan.open.push(OpenDocumentEdit {
                path,
                document_id: matches.document_id,
                revision: matches.revision,
                generation,
                expected_source: Arc::clone(&matches.source),
                after_source,
                selection_after,
                replacement_count: count,
            });
        }
    }

    Ok(plan)
}

/// Result for closed files. The loop stops at the first cancellation or save
/// conflict; it never claims rollback and never proceeds through remaining files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub journal: Option<PathBuf>,
    pub applied_files: usize,
    pub applied_replacements: usize,
    pub remaining_files: usize,
    pub remaining_replacements: usize,
    pub stopped_at: Option<PathBuf>,
    pub message: Option<String>,
    pub cancelled: bool,
    pub complete: bool,
}

/// Journal and apply closed-file edits. `cancelled` is polled at preflight
/// boundaries and before each file save; cancellation stops the remaining files.
pub fn apply_closed_replacements<F>(
    workspace: &Workspace,
    plan: &ReplacePlan,
    open_paths: &[PathBuf],
    mut cancelled: F,
) -> Result<ApplyOutcome, ReplaceError>
where
    F: FnMut() -> bool,
{
    let changed: Vec<_> = plan
        .closed
        .iter()
        .filter(|edit| edit.before_source != edit.after_source)
        .collect();
    let already_equal = plan
        .closed
        .iter()
        .filter(|edit| edit.before_source == edit.after_source)
        .map(|edit| edit.replacement_count)
        .sum::<usize>();
    let mut outcome = ApplyOutcome {
        applied_replacements: already_equal,
        complete: true,
        ..ApplyOutcome::default()
    };
    if path_key(workspace.root()) != path_key(&plan.root) {
        return Err(ReplaceError::new(
            "replacement plan belongs to another Workspace",
        ));
    }
    if changed.is_empty() {
        return Ok(outcome);
    }
    if cancelled() {
        outcome.cancelled = true;
        outcome.complete = false;
        outcome.remaining_files = changed.len();
        outcome.remaining_replacements = changed.iter().map(|edit| edit.replacement_count).sum();
        return Ok(outcome);
    }

    let mut open_keys = std::collections::HashSet::new();
    let mut open_physical = std::collections::HashSet::new();
    for path in open_paths {
        open_keys.insert(path_key(path));
        if let Ok(identity) = same_file::Handle::from_path(path) {
            open_physical.insert(identity);
        }
    }
    for edit in &changed {
        let identity = same_file::Handle::from_path(&edit.path)
            .map_err(|e| ReplaceError::new(format!("cannot identify replacement target: {e}")))?;
        if open_keys.contains(&path_key(&edit.path)) || open_physical.contains(&identity) {
            return Err(ReplaceError::new(format!(
                "closed replacement target is now open: {}",
                edit.path.display()
            )));
        }
    }

    // First preflight: no journal or document writes until every searched disk
    // snapshot still matches the preview.
    let mut first_pass = Vec::with_capacity(changed.len());
    for edit in &changed {
        if cancelled() {
            outcome.cancelled = true;
            outcome.complete = false;
            outcome.remaining_files = changed.len();
            outcome.remaining_replacements =
                changed.iter().map(|item| item.replacement_count).sum();
            return Ok(outcome);
        }
        first_pass.push(open_expected_file(workspace, edit)?);
    }
    drop(first_pass);

    let journal_directory = ensure_journal_directory(workspace)?;
    let mut journal_entries = Vec::with_capacity(changed.len());
    for edit in &changed {
        if cancelled() {
            outcome.cancelled = true;
            outcome.complete = false;
            outcome.remaining_files = changed.len();
            outcome.remaining_replacements =
                changed.iter().map(|item| item.replacement_count).sum();
            return Ok(outcome);
        }
        let after_fingerprint =
            fingerprint_encoded_source(&journal_directory, &edit.after_source, edit.format)?;
        journal_entries.push(JournalEntry {
            path: edit.path.clone(),
            before: edit.before_source.clone(),
            after: edit.after_source.clone(),
            format: Some(edit.format),
            before_fingerprint: Some(parse_digest(&edit.before_fingerprint)?),
            after_fingerprint: Some(after_fingerprint),
            before_source_digest: Some(digest(edit.before_source.as_bytes())),
            after_source_digest: Some(digest(edit.after_source.as_bytes())),
            replacement_count: Some(edit.replacement_count),
        });
    }
    let journal = write_journal(&journal_directory, &journal_entries)?;
    outcome.journal = Some(journal.clone());

    // Second preflight happens after the flushed journal and before the first
    // mutation. These held FileSnapshots also guard each later atomic save.
    let mut guarded = Vec::with_capacity(changed.len());
    for edit in &changed {
        if cancelled() {
            let _ = fs::remove_file(&journal);
            outcome.journal = None;
            outcome.cancelled = true;
            outcome.complete = false;
            outcome.remaining_files = changed.len();
            outcome.remaining_replacements =
                changed.iter().map(|item| item.replacement_count).sum();
            return Ok(outcome);
        }
        match open_expected_file(workspace, edit) {
            Ok(snapshot) => guarded.push(snapshot),
            Err(error) => {
                let _ = fs::remove_file(&journal);
                return Err(error);
            }
        }
    }

    for (index, (edit, mut snapshot)) in changed.iter().zip(guarded).enumerate() {
        if cancelled() {
            outcome.cancelled = true;
            outcome.complete = false;
            outcome.remaining_files = changed.len() - index;
            outcome.remaining_replacements = changed[index..]
                .iter()
                .map(|item| item.replacement_count)
                .sum();
            break;
        }
        if let Err(error) = snapshot.save(&edit.after_source) {
            outcome.stopped_at = Some(edit.path.clone());
            outcome.message = Some(error.to_string());
            outcome.complete = false;
            outcome.remaining_files = changed.len() - index;
            outcome.remaining_replacements = changed[index..]
                .iter()
                .map(|item| item.replacement_count)
                .sum();
            break;
        }
        outcome.applied_files += 1;
        outcome.applied_replacements += edit.replacement_count;
    }
    if outcome.remaining_files == 0 {
        outcome.complete = true;
        outcome.journal = Some(journal);
    }
    Ok(outcome)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryFileState {
    AlreadyRestored,
    NeedsRestore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryFilePreview {
    pub path: PathBuf,
    pub state: RecoveryFileState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryPreview {
    pub journal: PathBuf,
    pub version: u32,
    pub files: Vec<RecoveryFilePreview>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalInfo {
    pub path: PathBuf,
    pub version: Option<u32>,
    pub file_count: Option<usize>,
    /// Parse/validation errors are shown to the user; the file is preserved.
    pub recovery_message: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoveryOutcome {
    pub restored_files: usize,
    pub already_restored_files: usize,
    pub remaining_files: usize,
    pub stopped_at: Option<PathBuf>,
    pub message: Option<String>,
    pub cancelled: bool,
    pub complete: bool,
}

/// Enumerate `.journal` files without deleting or rewriting any entry. Broken
/// or unknown files remain visible with a recovery message.
pub fn enumerate_journals(workspace: &Workspace) -> Result<Vec<JournalInfo>, ReplaceError> {
    let Some(directory) = existing_journal_directory(workspace)? else {
        return Ok(Vec::new());
    };
    let mut paths = Vec::new();
    for item in fs::read_dir(&directory)? {
        let entry = item?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("journal"))
        {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths
        .into_iter()
        .map(|path| match read_journal(workspace, &path) {
            Ok(journal) => JournalInfo {
                path,
                version: Some(journal.version),
                file_count: Some(journal.entries.len()),
                recovery_message: None,
            },
            Err(error) => JournalInfo {
                path,
                version: None,
                file_count: None,
                recovery_message: Some(error.to_string()),
            },
        })
        .collect())
}

/// Read-only validation of a journal and its current targets. Any target open
/// in the application is rejected, including clean tabs.
pub fn preview_recovery(
    workspace: &Workspace,
    journal_path: &Path,
    open_paths: &[PathBuf],
) -> Result<RecoveryPreview, ReplaceError> {
    let journal = read_journal(workspace, journal_path)?;
    preview_journal_entries(workspace, &journal, open_paths)
}

fn preview_journal_entries(
    workspace: &Workspace,
    journal: &Journal,
    open_paths: &[PathBuf],
) -> Result<RecoveryPreview, ReplaceError> {
    let mut open = std::collections::HashSet::new();
    let mut open_physical = std::collections::HashSet::new();
    for path in open_paths {
        open.insert(path_key(path));
        if let Ok(identity) = same_file::Handle::from_path(path) {
            open_physical.insert(identity);
        }
    }
    let mut preview = RecoveryPreview {
        journal: journal.path.clone(),
        version: journal.version,
        files: Vec::with_capacity(journal.entries.len()),
    };
    for entry in &journal.entries {
        let path = safe_workspace_file_allow_missing(workspace, &entry.path)?;
        let identity = same_file::Handle::from_path(&path).ok();
        if open.contains(&path_key(&path))
            || identity
                .as_ref()
                .is_some_and(|value| open_physical.contains(value))
        {
            return Err(ReplaceError::new(format!(
                "recovery is refused while the target is open: {}",
                path.display()
            )));
        }
        let loaded =
            FileSnapshot::open_bounded(&path, MAX_REPLACEMENT_BYTES as u64).map_err(|e| {
                ReplaceError::new(format!(
                    "cannot read recovery target {}: {e}",
                    path.display()
                ))
            })?;
        let state = recovery_state(entry, &loaded.snapshot, &loaded.source)?;
        preview.files.push(RecoveryFilePreview { path, state });
    }
    Ok(preview)
}

/// Restore only files that still match the journal's exact after-snapshot.
/// It preflights all targets before saving and stops at the first later conflict.
pub fn restore_journal<F>(
    workspace: &Workspace,
    journal_path: &Path,
    open_paths: &[PathBuf],
    mut cancelled: F,
) -> Result<RecoveryOutcome, ReplaceError>
where
    F: FnMut() -> bool,
{
    let journal = read_journal(workspace, journal_path)?;
    let preview = preview_journal_entries(workspace, &journal, open_paths)?;
    let mut outcome = RecoveryOutcome::default();
    let mut pending = Vec::new();
    for (entry, file) in journal.entries.iter().zip(&preview.files) {
        match file.state {
            RecoveryFileState::AlreadyRestored => outcome.already_restored_files += 1,
            RecoveryFileState::NeedsRestore => pending.push((entry, file.path.clone())),
        }
    }
    if pending.is_empty() {
        outcome.complete = true;
        return Ok(outcome);
    }
    if cancelled() {
        outcome.cancelled = true;
        outcome.remaining_files = pending.len();
        return Ok(outcome);
    }

    let mut guarded = Vec::with_capacity(pending.len());
    for (entry, path) in &pending {
        if cancelled() {
            outcome.cancelled = true;
            outcome.remaining_files = pending.len();
            return Ok(outcome);
        }
        let loaded =
            FileSnapshot::open_bounded(path, MAX_REPLACEMENT_BYTES as u64).map_err(|e| {
                ReplaceError::new(format!(
                    "cannot recheck recovery target {}: {e}",
                    path.display()
                ))
            })?;
        if recovery_state(entry, &loaded.snapshot, &loaded.source)?
            != RecoveryFileState::NeedsRestore
        {
            return Err(ReplaceError::new(format!(
                "recovery target changed during preflight: {}",
                path.display()
            )));
        }
        guarded.push(loaded.snapshot);
    }

    for (index, ((entry, path), mut snapshot)) in pending.iter().zip(guarded).enumerate() {
        if cancelled() {
            outcome.cancelled = true;
            outcome.remaining_files = pending.len() - index;
            break;
        }
        if let Err(error) = snapshot.save(&entry.before) {
            outcome.stopped_at = Some(path.clone());
            outcome.message = Some(error.to_string());
            outcome.remaining_files = pending.len() - index;
            break;
        }
        outcome.restored_files += 1;
    }
    outcome.complete =
        !outcome.cancelled && outcome.stopped_at.is_none() && outcome.remaining_files == 0;
    Ok(outcome)
}

struct Journal {
    path: PathBuf,
    version: u32,
    entries: Vec<JournalEntry>,
}

struct JournalEntry {
    path: PathBuf,
    before: String,
    after: String,
    format: Option<FileFormat>,
    before_fingerprint: Option<[u8; 32]>,
    after_fingerprint: Option<[u8; 32]>,
    before_source_digest: Option<[u8; 32]>,
    after_source_digest: Option<[u8; 32]>,
    replacement_count: Option<usize>,
}

fn replacement_output(
    source_results: &search::Results,
    query: &Query,
    replacement: &str,
    generation: u64,
    closed: bool,
    cancel: &AtomicBool,
) -> Result<(String, Selection, usize), ReplaceError> {
    let recomputed = if closed {
        if source_results.document_id != 0 || source_results.revision != 0 {
            return Err(ReplaceError::new(
                "closed-file result has a live document identity",
            ));
        }
        None
    } else {
        Some(
            search::search_snapshot(
                0,
                0,
                Arc::clone(&source_results.source),
                query.clone(),
                generation,
                cancel,
                std::time::Duration::from_millis(200),
            )
            .map_err(|e| ReplaceError::new(format!("cannot verify replacement hits: {e}")))?,
        )
    };
    let canonical = recomputed.as_ref().unwrap_or(source_results);
    if canonical.state != SearchState::Complete || !same_hits(canonical, source_results) {
        return Err(ReplaceError::new(
            "replacement hits changed while previewing",
        ));
    }
    let mut document = Document::new(0, source_results.source.to_string());
    let count = canonical
        .replace(&mut document, generation, None, replacement)
        .map_err(|e| ReplaceError::new(format!("cannot format replacement: {e}")))?;
    if document.byte_len() > MAX_REPLACEMENT_BYTES {
        return Err(ReplaceError::new("replacement output exceeds 64 MiB"));
    }
    Ok((document.source().to_owned(), document.selection(), count))
}

fn same_hits(left: &search::Results, right: &search::Results) -> bool {
    left.hits.len() == right.hits.len()
        && left
            .hits
            .iter()
            .zip(&right.hits)
            .all(|(a, b)| a.range == b.range && a.captures == b.captures)
}

fn open_expected_file(
    workspace: &Workspace,
    edit: &ClosedFileEdit,
) -> Result<FileSnapshot, ReplaceError> {
    let path = safe_workspace_file(workspace, &edit.path)?;
    let loaded = FileSnapshot::open_bounded(&path, MAX_FILE_BYTES)?;
    if loaded.source != edit.before_source
        || loaded.snapshot.format() != edit.format
        || loaded.snapshot.baseline_sha256() != edit.before_fingerprint
    {
        return Err(ReplaceError::new(format!(
            "closed file changed since preview: {}",
            path.display()
        )));
    }
    Ok(loaded.snapshot)
}

fn recovery_state(
    entry: &JournalEntry,
    snapshot: &FileSnapshot,
    source: &str,
) -> Result<RecoveryFileState, ReplaceError> {
    if let Some(format) = entry.format {
        if snapshot.format() != format {
            return Err(ReplaceError::new(format!(
                "recovery target encoding changed: {}",
                snapshot.path().display()
            )));
        }
        let fingerprint = parse_digest(&snapshot.baseline_sha256())?;
        if source == entry.before
            && entry.before_fingerprint == Some(fingerprint)
            && entry.before_source_digest == Some(digest(source.as_bytes()))
        {
            return Ok(RecoveryFileState::AlreadyRestored);
        }
        if source == entry.after
            && entry.after_fingerprint == Some(fingerprint)
            && entry.after_source_digest == Some(digest(source.as_bytes()))
        {
            return Ok(RecoveryFileState::NeedsRestore);
        }
    } else {
        if source == entry.before {
            return Ok(RecoveryFileState::AlreadyRestored);
        }
        if source == entry.after {
            return Ok(RecoveryFileState::NeedsRestore);
        }
    }
    Err(ReplaceError::new(format!(
        "recovery target no longer matches the journal after-snapshot: {}",
        snapshot.path().display()
    )))
}

fn ensure_journal_directory(workspace: &Workspace) -> Result<PathBuf, ReplaceError> {
    let mut current = workspace.root().to_path_buf();
    for component in [".mdlite", ".state", "replace"] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(ReplaceError::new(format!(
                        "journal path is not a physical directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error.into()),
                }
                let metadata = fs::symlink_metadata(&current)?;
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(ReplaceError::new(format!(
                        "journal directory was replaced by an unsafe entry: {}",
                        current.display()
                    )));
                }
            }
            Err(error) => return Err(error.into()),
        }
    }
    let physical = fs::canonicalize(current)?;
    if !path_within(workspace.root(), &physical) {
        return Err(ReplaceError::new(
            "replacement journal directory resolves outside the Workspace",
        ));
    }
    Ok(physical)
}

fn existing_journal_directory(workspace: &Workspace) -> Result<Option<PathBuf>, ReplaceError> {
    let mut current = workspace.root().to_path_buf();
    for component in [".mdlite", ".state", "replace"] {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) || !metadata.is_dir() {
                    return Err(ReplaceError::new(format!(
                        "journal directory is unsafe: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(current))
}

fn fingerprint_encoded_source(
    directory: &Path,
    source: &str,
    format: FileFormat,
) -> Result<[u8; 32], ReplaceError> {
    for _ in 0..10 {
        let path = unique_path(directory, "fingerprint", "tmp");
        match FileSnapshot::create_new(&path, source, format) {
            Ok(snapshot) => {
                let fingerprint = parse_digest(&snapshot.baseline_sha256())?;
                let saved_path = snapshot.path().to_path_buf();
                drop(snapshot);
                fs::remove_file(saved_path)?;
                return Ok(fingerprint);
            }
            Err(StorageError::DestinationExists) => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(ReplaceError::new(
        "cannot allocate a temporary encoding fingerprint file",
    ))
}

fn write_journal(directory: &Path, entries: &[JournalEntry]) -> Result<PathBuf, ReplaceError> {
    for _ in 0..10 {
        let path = unique_path(directory, "replace", "journal");
        let mut file = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        };
        let result = (|| -> io::Result<()> {
            file.write_all(V3_MAGIC)?;
            file.write_all(&(entries.len() as u64).to_le_bytes())?;
            for entry in entries {
                let format = entry.format.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing file format")
                })?;
                let before_fingerprint = entry.before_fingerprint.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing before fingerprint")
                })?;
                let after_fingerprint = entry.after_fingerprint.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing after fingerprint")
                })?;
                let before_digest = entry.before_source_digest.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing before source digest")
                })?;
                let after_digest = entry.after_source_digest.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidData, "missing after source digest")
                })?;
                let path = entry
                    .path
                    .to_str()
                    .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "path is not UTF-8"))?
                    .as_bytes();
                let before = entry.before.as_bytes();
                let after = entry.after.as_bytes();
                file.write_all(&[format_tag(format)])?;
                for length in [
                    path.len(),
                    before.len(),
                    after.len(),
                    entry.replacement_count.unwrap_or(0),
                ] {
                    file.write_all(&(length as u64).to_le_bytes())?;
                }
                file.write_all(&before_fingerprint)?;
                file.write_all(&after_fingerprint)?;
                file.write_all(&before_digest)?;
                file.write_all(&after_digest)?;
                file.write_all(path)?;
                file.write_all(before)?;
                file.write_all(after)?;
            }
            file.flush()?;
            file.sync_all()
        })();
        if let Err(error) = result {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(ReplaceError::new(format!(
                "replacement journal could not be flushed; no file changes were made: {error}"
            )));
        }
        return Ok(path);
    }
    Err(ReplaceError::new(
        "cannot allocate a unique replacement journal",
    ))
}

fn unique_path(directory: &Path, prefix: &str, extension: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = NEXT_FILE_ID.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        "{prefix}-{stamp}-{}-{sequence}.{extension}",
        std::process::id()
    ))
}

fn read_journal(workspace: &Workspace, path: &Path) -> Result<Journal, ReplaceError> {
    let path = safe_journal_path(workspace, path)?;
    let metadata = fs::symlink_metadata(&path)?;
    if is_reparse_point(&metadata) || !metadata.is_file() {
        return Err(ReplaceError::new("journal is not a safe regular file"));
    }
    let input = File::open(&path)?;
    let mut bytes = Vec::new();
    input.take(MAX_JOURNAL_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(ReplaceError::new("journal exceeds the 128 MiB read limit"));
    }
    let mut journal = if bytes.starts_with(V3_MAGIC) {
        parse_v3(&bytes[V3_MAGIC.len()..], path.clone())?
    } else if bytes.starts_with(V2_MAGIC) {
        parse_v2(&bytes[V2_MAGIC.len()..], path.clone())?
    } else {
        parse_v1(&bytes, path.clone())?
    };
    validate_journal_entries(workspace, &mut journal.entries)?;
    Ok(journal)
}

fn parse_v3(bytes: &[u8], path: PathBuf) -> Result<Journal, ReplaceError> {
    let mut cursor = Cursor::new(bytes);
    let count = cursor.u64()?;
    if count > MAX_REPLACEMENTS as u64 {
        return Err(ReplaceError::new("journal has too many files"));
    }
    let mut entries = Vec::with_capacity(count as usize);
    let mut total = 0usize;
    let mut total_replacements = 0usize;
    for _ in 0..count {
        let tag = cursor.byte()?;
        let file_format = parse_format(tag)?;
        let path_len = cursor.length(MAX_PATH_BYTES)?;
        let before_len = cursor.length(MAX_REPLACEMENT_BYTES)?;
        let after_len = cursor.length(MAX_REPLACEMENT_BYTES)?;
        let replacement_count = cursor.length(MAX_REPLACEMENTS)?;
        let before_fingerprint = cursor.array32()?;
        let after_fingerprint = cursor.array32()?;
        let before_source_digest = cursor.array32()?;
        let after_source_digest = cursor.array32()?;
        total = total
            .checked_add(path_len)
            .and_then(|value| value.checked_add(before_len))
            .and_then(|value| value.checked_add(after_len))
            .ok_or_else(|| ReplaceError::new("journal size overflow"))?;
        if total > MAX_REPLACEMENT_BYTES || replacement_count == 0 {
            return Err(ReplaceError::new("journal exceeds its plan or count limit"));
        }
        total_replacements = total_replacements
            .checked_add(replacement_count)
            .filter(|total| *total <= MAX_REPLACEMENTS)
            .ok_or_else(|| ReplaceError::new("journal replacement count exceeds 20,000"))?;
        let entry_path = std::str::from_utf8(cursor.take(path_len)?)
            .map_err(|_| ReplaceError::new("journal path is not UTF-8"))?;
        let before = std::str::from_utf8(cursor.take(before_len)?)
            .map_err(|_| ReplaceError::new("journal before-source is not UTF-8"))?
            .to_owned();
        let after = std::str::from_utf8(cursor.take(after_len)?)
            .map_err(|_| ReplaceError::new("journal after-source is not UTF-8"))?
            .to_owned();
        if entry_path.contains('\0')
            || before.contains('\0')
            || after.contains('\0')
            || digest(before.as_bytes()) != before_source_digest
            || digest(after.as_bytes()) != after_source_digest
        {
            return Err(ReplaceError::new(
                "journal path, source, or fingerprint is invalid",
            ));
        }
        entries.push(JournalEntry {
            path: PathBuf::from(entry_path),
            before,
            after,
            format: Some(file_format),
            before_fingerprint: Some(before_fingerprint),
            after_fingerprint: Some(after_fingerprint),
            before_source_digest: Some(before_source_digest),
            after_source_digest: Some(after_source_digest),
            replacement_count: Some(replacement_count),
        });
    }
    cursor.finish()?;
    Ok(Journal {
        path,
        version: 3,
        entries,
    })
}

fn parse_v2(bytes: &[u8], path: PathBuf) -> Result<Journal, ReplaceError> {
    let mut cursor = Cursor::new(bytes);
    let count = cursor.u64()?;
    if count > MAX_REPLACEMENTS as u64 {
        return Err(ReplaceError::new("legacy journal has too many files"));
    }
    let mut entries = Vec::with_capacity(count as usize);
    let mut total = 0usize;
    for _ in 0..count {
        let path_units = cursor.length(32_767)?;
        let before_units = cursor.length(MAX_REPLACEMENT_BYTES / 2)?;
        let after_units = cursor.length(MAX_REPLACEMENT_BYTES / 2)?;
        let item_bytes = path_units
            .checked_add(before_units)
            .and_then(|value| value.checked_add(after_units))
            .and_then(|value| value.checked_mul(2))
            .ok_or_else(|| ReplaceError::new("legacy journal size overflow"))?;
        total = total
            .checked_add(item_bytes)
            .ok_or_else(|| ReplaceError::new("legacy journal size overflow"))?;
        if total > MAX_REPLACEMENT_BYTES {
            return Err(ReplaceError::new(
                "legacy journal exceeds its 64 MiB data limit",
            ));
        }
        let path_text = decode_utf16(cursor.take(path_units * 2)?)?;
        let before = decode_utf16(cursor.take(before_units * 2)?)?;
        let after = decode_utf16(cursor.take(after_units * 2)?)?;
        if path_text.contains('\0') || before.contains('\0') || after.contains('\0') {
            return Err(ReplaceError::new("legacy journal contains a NUL"));
        }
        entries.push(legacy_entry(PathBuf::from(path_text), before, after));
    }
    cursor.finish()?;
    Ok(Journal {
        path,
        version: 2,
        entries,
    })
}

fn parse_v1(bytes: &[u8], path: PathBuf) -> Result<Journal, ReplaceError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ReplaceError::new("legacy V1 journal is not UTF-8"))?;
    let mut lines = text.lines();
    if lines.next() != Some(std::str::from_utf8(V1_MAGIC).unwrap()) {
        return Err(ReplaceError::new(
            "unknown or malformed replacement journal version",
        ));
    }
    let mut entries = Vec::new();
    let mut total = 0usize;
    for line in lines {
        if entries.len() >= MAX_REPLACEMENTS {
            return Err(ReplaceError::new("legacy journal has too many files"));
        }
        let mut fields = line.split('\t');
        let path_text = unhex_utf8(fields.next().unwrap_or_default())?;
        let before = unhex_utf8(fields.next().unwrap_or_default())?;
        let after = unhex_utf8(fields.next().unwrap_or_default())?;
        if fields.next().is_some() || path_text.is_empty() {
            return Err(ReplaceError::new("legacy V1 journal line is malformed"));
        }
        if path_text.encode_utf16().count() > MAX_LEGACY_PATH_UNITS {
            return Err(ReplaceError::new("legacy V1 path exceeds its size limit"));
        }
        if path_text.contains('\0') || before.contains('\0') || after.contains('\0') {
            return Err(ReplaceError::new("legacy V1 journal contains a NUL"));
        }
        total = total
            .checked_add(path_text.len())
            .and_then(|value| value.checked_add(before.len()))
            .and_then(|value| value.checked_add(after.len()))
            .ok_or_else(|| ReplaceError::new("legacy journal size overflow"))?;
        if total > MAX_REPLACEMENT_BYTES {
            return Err(ReplaceError::new(
                "legacy journal exceeds its 64 MiB data limit",
            ));
        }
        entries.push(legacy_entry(PathBuf::from(path_text), before, after));
    }
    Ok(Journal {
        path,
        version: 1,
        entries,
    })
}

fn legacy_entry(path: PathBuf, before: String, after: String) -> JournalEntry {
    JournalEntry {
        path,
        before,
        after,
        format: None,
        before_fingerprint: None,
        after_fingerprint: None,
        before_source_digest: None,
        after_source_digest: None,
        replacement_count: None,
    }
}

fn validate_journal_entries(
    workspace: &Workspace,
    entries: &mut [JournalEntry],
) -> Result<(), ReplaceError> {
    let mut seen = std::collections::HashSet::new();
    let mut seen_physical = std::collections::HashSet::new();
    for entry in entries {
        if !entry.path.is_absolute() {
            return Err(ReplaceError::new(
                "journal contains a non-absolute target path",
            ));
        }
        let path = safe_workspace_file_allow_missing(workspace, &entry.path)?;
        if !seen.insert(path_key(&path)) {
            return Err(ReplaceError::new("journal contains duplicate target paths"));
        }
        if let Ok(identity) = same_file::Handle::from_path(&path)
            && !seen_physical.insert(identity)
        {
            return Err(ReplaceError::new(
                "journal contains duplicate physical target files",
            ));
        }
        entry.path = path;
    }
    Ok(())
}

fn safe_journal_path(workspace: &Workspace, path: &Path) -> Result<PathBuf, ReplaceError> {
    let relative = relative_components(workspace.root(), path)?;
    if relative.len() != 4
        || !component_eq(&relative[0], ".mdlite")
        || !component_eq(&relative[1], ".state")
        || !component_eq(&relative[2], "replace")
        || !Path::new(&relative[3])
            .extension()
            .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("journal"))
    {
        return Err(ReplaceError::new(
            "journal is outside this Workspace replace area",
        ));
    }
    let mut current = workspace.root().to_path_buf();
    for (index, component) in relative.iter().enumerate() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current)?;
        if is_reparse_point(&metadata)
            || (index < 3 && !metadata.is_dir())
            || (index == 3 && !metadata.is_file())
        {
            return Err(ReplaceError::new(
                "journal path contains a reparse point or wrong file type",
            ));
        }
    }
    Ok(fs::canonicalize(current)?)
}

fn safe_workspace_file(workspace: &Workspace, path: &Path) -> Result<PathBuf, ReplaceError> {
    safe_workspace_file_inner(workspace, path, false)
}

fn safe_workspace_file_allow_missing(
    workspace: &Workspace,
    path: &Path,
) -> Result<PathBuf, ReplaceError> {
    safe_workspace_file_inner(workspace, path, true)
}

fn safe_workspace_file_inner(
    workspace: &Workspace,
    path: &Path,
    allow_missing: bool,
) -> Result<PathBuf, ReplaceError> {
    let relative = relative_components(workspace.root(), path)?;
    if relative.is_empty() || relative.iter().any(|part| is_protected(part)) {
        return Err(ReplaceError::new(
            "replacement path is a protected Workspace path",
        ));
    }
    let mut current = workspace.root().to_path_buf();
    for (index, component) in relative.iter().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                let last = index + 1 == relative.len();
                if is_reparse_point(&metadata)
                    || (!last && !metadata.is_dir())
                    || (last && !metadata.is_file())
                {
                    return Err(ReplaceError::new(format!(
                        "replacement path is not a safe regular file: {}",
                        current.display()
                    )));
                }
            }
            Err(error)
                if allow_missing
                    && index + 1 == relative.len()
                    && error.kind() == io::ErrorKind::NotFound =>
            {
                return Ok(current);
            }
            Err(error) => return Err(error.into()),
        }
    }
    let canonical = fs::canonicalize(&current)?;
    if !path_within(workspace.root(), &canonical) {
        return Err(ReplaceError::new(
            "replacement path resolves outside the Workspace",
        ));
    }
    Ok(canonical)
}

fn relative_components(root: &Path, path: &Path) -> Result<Vec<std::ffi::OsString>, ReplaceError> {
    if !path.is_absolute() {
        return Err(ReplaceError::new("replacement path is not absolute"));
    }
    let root_components = normalized_components(root)?;
    let path_components = normalized_components(path)?;
    if path_components.len() <= root_components.len()
        || !root_components
            .iter()
            .zip(&path_components)
            .all(|(left, right)| component_matches(left, right))
    {
        return Err(ReplaceError::new(
            "replacement path is outside the Workspace",
        ));
    }
    let relative = path_components[root_components.len()..].to_vec();
    if relative.iter().any(|part| part == "." || part == "..") {
        return Err(ReplaceError::new(
            "replacement path contains traversal components",
        ));
    }
    Ok(relative)
}

fn normalized_components(path: &Path) -> Result<Vec<std::ffi::OsString>, ReplaceError> {
    let path = normalize_verbatim(path);
    let mut result = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => result.push(prefix.as_os_str().to_owned()),
            Component::RootDir => result.push(std::ffi::OsString::from(
                std::path::MAIN_SEPARATOR.to_string(),
            )),
            Component::Normal(value) => result.push(value.to_os_string()),
            Component::CurDir | Component::ParentDir => {
                return Err(ReplaceError::new(
                    "replacement path contains traversal components",
                ));
            }
        }
    }
    Ok(result)
}

#[cfg(windows)]
fn normalize_verbatim(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path.to_path_buf()
    }
}

#[cfg(not(windows))]
fn normalize_verbatim(path: &Path) -> PathBuf {
    path.to_path_buf()
}

fn component_matches(left: &std::ffi::OsStr, right: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn component_eq(value: &std::ffi::OsStr, expected: &str) -> bool {
    component_matches(value, std::ffi::OsStr::new(expected))
}

fn is_protected(component: &std::ffi::OsStr) -> bool {
    let name = component.to_string_lossy();
    [
        ".git",
        ".mdlite",
        ".cache",
        ".state",
        ".svn",
        ".hg",
        ".codex",
        ".codex-runtime",
        "node_modules",
        "target",
        "build",
        "out",
    ]
    .iter()
    .any(|protected| name.eq_ignore_ascii_case(protected))
}

fn path_within(root: &Path, path: &Path) -> bool {
    match (normalized_components(root), normalized_components(path)) {
        (Ok(root), Ok(path)) => {
            path.len() >= root.len() && root.iter().zip(&path).all(|(a, b)| component_matches(a, b))
        }
        _ => false,
    }
}

fn path_key(path: &Path) -> String {
    let text = normalize_verbatim(path)
        .to_string_lossy()
        .replace('/', "\\");
    #[cfg(windows)]
    {
        text.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        text
    }
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn format_tag(format: FileFormat) -> u8 {
    match format {
        FileFormat::Utf8 { bom: false } => 0,
        FileFormat::Utf8 { bom: true } => 1,
        FileFormat::Cp932 => 2,
    }
}

fn parse_format(tag: u8) -> Result<FileFormat, ReplaceError> {
    match tag {
        0 => Ok(FileFormat::Utf8 { bom: false }),
        1 => Ok(FileFormat::Utf8 { bom: true }),
        2 => Ok(FileFormat::Cp932),
        _ => Err(ReplaceError::new("journal encoding tag is unknown")),
    }
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn parse_digest(hex: &str) -> Result<[u8; 32], ReplaceError> {
    if hex.len() != 64 {
        return Err(ReplaceError::new(
            "SHA-256 fingerprint has the wrong length",
        ));
    }
    let mut result = [0u8; 32];
    for (index, output) in result.iter_mut().enumerate() {
        let pair = &hex[index * 2..index * 2 + 2];
        *output = u8::from_str_radix(pair, 16)
            .map_err(|_| ReplaceError::new("SHA-256 fingerprint is not hexadecimal"))?;
    }
    Ok(result)
}

fn decode_utf16(bytes: &[u8]) -> Result<String, ReplaceError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(ReplaceError::new(
            "legacy UTF-16 journal field has odd length",
        ));
    }
    let units = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect::<Vec<_>>();
    String::from_utf16(&units).map_err(|_| ReplaceError::new("legacy journal has invalid UTF-16"))
}

fn unhex_utf8(value: &str) -> Result<String, ReplaceError> {
    if !value.len().is_multiple_of(2) {
        return Err(ReplaceError::new("legacy V1 journal hex has odd length"));
    }
    let mut bytes = Vec::with_capacity(value.len() / 2);
    for pair in value.as_bytes().as_chunks::<2>().0 {
        let high = upper_hex(pair[0])
            .ok_or_else(|| ReplaceError::new("legacy V1 journal hex is invalid"))?;
        let low = upper_hex(pair[1])
            .ok_or_else(|| ReplaceError::new("legacy V1 journal hex is invalid"))?;
        bytes.push((high << 4) | low);
    }
    String::from_utf8(bytes).map_err(|_| ReplaceError::new("legacy V1 journal text is not UTF-8"))
}

fn upper_hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], ReplaceError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| ReplaceError::new("journal length overflow"))?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| ReplaceError::new("journal is truncated"))?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, ReplaceError> {
        Ok(self.take(1)?[0])
    }

    fn u64(&mut self) -> Result<u64, ReplaceError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn length(&mut self, limit: usize) -> Result<usize, ReplaceError> {
        let value = usize::try_from(self.u64()?)
            .map_err(|_| ReplaceError::new("journal length exceeds this platform"))?;
        if value > limit {
            return Err(ReplaceError::new("journal field exceeds its size limit"));
        }
        Ok(value)
    }

    fn array32(&mut self) -> Result<[u8; 32], ReplaceError> {
        Ok(self.take(32)?.try_into().unwrap())
    }

    fn finish(self) -> Result<(), ReplaceError> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err(ReplaceError::new("journal has trailing bytes"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_search::{OpenSource, Options};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mdlite-replace-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }

        fn workspace(&self) -> Workspace {
            Workspace::open(&self.0).unwrap()
        }

        fn journal_dir(&self) -> PathBuf {
            self.0.join(".mdlite").join(".state").join("replace")
        }

        fn search(&self, query: Query, generation: u64) -> WorkspaceResults {
            workspace_search::search_workspace(
                &self.workspace(),
                query,
                &Options::default(),
                &[],
                generation,
                &AtomicBool::new(false),
            )
            .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn plan(fixture: &Fixture, query: Query, replacement: &str, generation: u64) -> ReplacePlan {
        let results = fixture.search(query.clone(), generation);
        preview_workspace_replace(
            &results,
            &query,
            replacement,
            generation,
            &fixture.workspace(),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn replacement_uses_canonical_capture_formatter_and_restores_through_v3_journal() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("note.md"), b"\xef\xbb\xbfitem abc abc\r\n").unwrap();
        let query = Query {
            text: "(?<= )([a-z]+)".to_owned(),
            regex: true,
            ..Query::default()
        };
        let plan = plan(&fixture, query, "<$1>", 17);
        assert_eq!(plan.closed_edits().len(), 1);
        assert_eq!(plan.closed_edits()[0].after_source, "item <abc> <abc>\r\n");
        assert_eq!(plan.open_edits().len(), 0);

        let result = apply_closed_replacements(&fixture.workspace(), &plan, &[], || false).unwrap();
        assert!(result.complete);
        assert_eq!(result.applied_files, 1);
        assert_eq!(result.applied_replacements, 2);
        let after = fs::read(fixture.0.join("note.md")).unwrap();
        assert!(after.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(
            std::str::from_utf8(&after[3..]).unwrap(),
            "item <abc> <abc>\r\n"
        );
        let journal = result.journal.unwrap();
        let listed = enumerate_journals(&fixture.workspace()).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].version, Some(3));
        assert_eq!(listed[0].file_count, Some(1));

        let preview = preview_recovery(&fixture.workspace(), &journal, &[]).unwrap();
        assert_eq!(preview.files[0].state, RecoveryFileState::NeedsRestore);
        let restored = restore_journal(&fixture.workspace(), &journal, &[], || false).unwrap();
        assert!(restored.complete);
        assert_eq!(restored.restored_files, 1);
        let restored_bytes = fs::read(fixture.0.join("note.md")).unwrap();
        assert!(restored_bytes.starts_with(b"\xef\xbb\xbf"));
        assert_eq!(
            std::str::from_utf8(&restored_bytes[3..]).unwrap(),
            "item abc abc\r\n"
        );
    }

    #[test]
    fn stale_preflight_and_journal_directory_failure_leave_closed_files_unchanged() {
        let fixture = Fixture::new();
        let file = fixture.0.join("note.md");
        fs::write(&file, b"before").unwrap();
        let query = Query {
            text: "before".to_owned(),
            ..Query::default()
        };
        let stale_plan = plan(&fixture, query.clone(), "after", 1);
        fs::write(&file, b"peer edit").unwrap();
        assert!(
            apply_closed_replacements(&fixture.workspace(), &stale_plan, &[], || false).is_err()
        );
        assert_eq!(fs::read(&file).unwrap(), b"peer edit");
        assert!(!fixture.0.join(".mdlite").exists());

        fs::write(&file, b"before").unwrap();
        fs::write(fixture.0.join(".mdlite"), b"blocking file").unwrap();
        let blocked_plan = plan(&fixture, query, "after", 2);
        assert!(
            apply_closed_replacements(&fixture.workspace(), &blocked_plan, &[], || false).is_err()
        );
        assert_eq!(fs::read(&file).unwrap(), b"before");
        assert_eq!(
            fs::read(fixture.0.join(".mdlite")).unwrap(),
            b"blocking file"
        );
    }

    #[test]
    fn cancellation_stops_after_one_closed_file_and_reports_remaining_counts() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.md"), b"x x").unwrap();
        fs::write(fixture.0.join("b.md"), b"x x").unwrap();
        let query = Query {
            text: "x".to_owned(),
            ..Query::default()
        };
        let plan = plan(&fixture, query, "y", 3);
        assert_eq!(plan.closed_edits().len(), 2);
        let first = fixture.0.join("a.md");
        let result = apply_closed_replacements(&fixture.workspace(), &plan, &[], || {
            fs::read_to_string(&first).is_ok_and(|text| text == "y y")
        })
        .unwrap();
        assert!(result.cancelled);
        assert!(!result.complete);
        assert_eq!(result.applied_files, 1);
        assert_eq!(result.applied_replacements, 2);
        assert_eq!(result.remaining_files, 1);
        assert_eq!(result.remaining_replacements, 2);
        assert!(result.journal.is_some());
        let contents = [
            fs::read_to_string(fixture.0.join("a.md")).unwrap(),
            fs::read_to_string(fixture.0.join("b.md")).unwrap(),
        ];
        assert_eq!(
            contents
                .iter()
                .filter(|text| text.as_str() == "y y")
                .count(),
            1
        );
        assert_eq!(
            contents
                .iter()
                .filter(|text| text.as_str() == "x x")
                .count(),
            1
        );
    }

    #[test]
    fn recovery_refuses_open_tabs_and_any_changed_after_snapshot() {
        let fixture = Fixture::new();
        let file = fixture.0.join("note.md");
        fs::write(&file, b"before").unwrap();
        let query = Query {
            text: "before".to_owned(),
            ..Query::default()
        };
        let plan = plan(&fixture, query, "after", 4);
        let applied =
            apply_closed_replacements(&fixture.workspace(), &plan, &[], || false).unwrap();
        let journal = applied.journal.unwrap();
        let alias = fixture.0.join("open-alias.md");
        fs::hard_link(&file, &alias).unwrap();
        assert!(
            preview_recovery(&fixture.workspace(), &journal, std::slice::from_ref(&alias)).is_err()
        );
        fs::remove_file(alias).unwrap();

        // Same decoded text with a changed BOM is still a peer change.
        fs::write(&file, b"\xef\xbb\xbfafter").unwrap();
        assert!(preview_recovery(&fixture.workspace(), &journal, &[]).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"\xef\xbb\xbfafter");
    }

    #[test]
    fn legacy_v1_and_v2_journals_are_read_and_restored_without_rewriting_them() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.journal_dir()).unwrap();
        let v1_target = fixture.0.join("v1.md");
        let v2_target = fixture.0.join("v2.md");
        fs::write(&v1_target, b"v1 after").unwrap();
        fs::write(&v2_target, b"v2 after").unwrap();
        let v1_path = fixture.journal_dir().join("legacy-v1.journal");
        let v2_path = fixture.journal_dir().join("legacy-v2.journal");
        let v1 = format!(
            "MDLITE_REPLACE_JOURNAL_V1\n{}\t{}\t{}\n",
            hex(v1_target.to_string_lossy().as_bytes()),
            hex(b"v1 before"),
            hex(b"v1 after")
        );
        fs::write(&v1_path, v1).unwrap();
        fs::write(
            &v2_path,
            legacy_v2_bytes(&v2_target, "v2 before", "v2 after"),
        )
        .unwrap();

        let listed = enumerate_journals(&fixture.workspace()).unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|item| item.version == Some(1)));
        assert!(listed.iter().any(|item| item.version == Some(2)));
        for path in [&v1_path, &v2_path] {
            let result = restore_journal(&fixture.workspace(), path, &[], || false).unwrap();
            assert!(result.complete);
            assert_eq!(result.restored_files, 1);
        }
        assert_eq!(fs::read_to_string(v1_target).unwrap(), "v1 before");
        assert_eq!(fs::read_to_string(v2_target).unwrap(), "v2 before");
        assert!(v1_path.exists() && v2_path.exists());
    }

    #[test]
    fn broken_legacy_journal_is_listed_with_message_and_preserved() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.journal_dir()).unwrap();
        let path = fixture.journal_dir().join("broken.journal");
        fs::write(
            &path,
            b"MDLITE_REPLACE_JOURNAL_V99\nprivate input stays here\n",
        )
        .unwrap();
        let listed = enumerate_journals(&fixture.workspace()).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].recovery_message.is_some());
        assert_eq!(
            fs::read(&path).unwrap(),
            b"MDLITE_REPLACE_JOURNAL_V99\nprivate input stays here\n"
        );
    }

    #[test]
    fn open_document_plan_is_revision_and_source_guarded() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("open.md"), b"disk source").unwrap();
        let path = fs::canonicalize(fixture.0.join("open.md")).unwrap();
        let open = OpenSource {
            path: path.clone(),
            document_id: 42,
            revision: 7,
            source: Arc::from("dirty source"),
        };
        let query = Query {
            text: "dirty".to_owned(),
            ..Query::default()
        };
        let results = workspace_search::search_workspace(
            &fixture.workspace(),
            query.clone(),
            &Options::default(),
            &[open],
            9,
            &AtomicBool::new(false),
        )
        .unwrap();
        let plan = preview_workspace_replace(
            &results,
            &query,
            "unsaved",
            9,
            &fixture.workspace(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(plan.closed_edits().len(), 0);
        let edit = &plan.open_edits()[0];
        assert!(edit.matches_current(42, 7, 9, "dirty source"));
        assert!(!edit.matches_current(42, 8, 9, "dirty source"));
        assert_eq!(edit.after_source, "unsaved source");
    }

    #[test]
    fn replacement_plan_rejects_two_names_for_the_same_physical_file() {
        let fixture = Fixture::new();
        let first = fixture.0.join("first.md");
        let second = fixture.0.join("second.md");
        fs::write(&first, b"needle").unwrap();
        fs::hard_link(&first, &second).unwrap();
        let query = Query {
            text: "needle".to_owned(),
            ..Query::default()
        };
        let first_loaded = FileSnapshot::open_bounded(&first, MAX_FILE_BYTES).unwrap();
        let second_loaded = FileSnapshot::open_bounded(&second, MAX_FILE_BYTES).unwrap();
        let first_matches = search::search_snapshot(
            0,
            0,
            Arc::from(first_loaded.source),
            query.clone(),
            5,
            &AtomicBool::new(false),
            std::time::Duration::from_millis(200),
        )
        .unwrap();
        let second_matches = search::search_snapshot(
            0,
            0,
            Arc::from(second_loaded.source),
            query.clone(),
            5,
            &AtomicBool::new(false),
            std::time::Duration::from_millis(200),
        )
        .unwrap();
        let results = WorkspaceResults {
            root: fixture.workspace().root().to_path_buf(),
            generation: 5,
            files: vec![
                workspace_search::FileResults {
                    path: fs::canonicalize(&first).unwrap(),
                    matches: first_matches,
                    snapshot: Some(first_loaded.snapshot),
                },
                workspace_search::FileResults {
                    path: fs::canonicalize(&second).unwrap(),
                    matches: second_matches,
                    snapshot: Some(second_loaded.snapshot),
                },
            ],
            issues: Vec::new(),
            complete: true,
            scanned_bytes: 12,
        };
        assert!(
            preview_workspace_replace(
                &results,
                &query,
                "replacement",
                5,
                &fixture.workspace(),
                &AtomicBool::new(false),
            )
            .is_err()
        );
        assert_eq!(fs::read(&first).unwrap(), b"needle");
        assert_eq!(fs::read(&second).unwrap(), b"needle");
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02X}")).collect()
    }

    fn legacy_v2_bytes(path: &Path, before: &str, after: &str) -> Vec<u8> {
        let mut bytes = V2_MAGIC.to_vec();
        bytes.extend_from_slice(&1u64.to_le_bytes());
        let path: Vec<u16> = path.to_string_lossy().encode_utf16().collect();
        let before: Vec<u16> = before.encode_utf16().collect();
        let after: Vec<u16> = after.encode_utf16().collect();
        for length in [path.len(), before.len(), after.len()] {
            bytes.extend_from_slice(&(length as u64).to_le_bytes());
        }
        for units in [path, before, after] {
            for unit in units {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }
}
