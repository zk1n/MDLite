//! Workspace filesystem boundaries for the native editor.
//!
//! Opening a workspace only resolves an existing directory. Creating MDLite
//! state is a separate, explicit operation.

use std::{
    collections::HashSet,
    ffi::{OsStr, OsString},
    fmt, fs,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
};

const STATE_IGNORE: &str = "/.mdlite/.state/";
const CACHE_IGNORE: &str = "/.mdlite/.cache/";

#[derive(Debug)]
pub enum WorkspaceError {
    Io(io::Error),
    InvalidRoot(PathBuf),
    InvalidPath(PathBuf),
    OutsideWorkspace(PathBuf),
    ReparsePoint(PathBuf),
    ProtectedPath(PathBuf),
    InvalidName(String),
    InvalidGitignore(PathBuf),
    UnsupportedEntry(PathBuf),
    Collision(PathBuf),
    SamePath(PathBuf),
    SelfChild {
        source: PathBuf,
        destination: PathBuf,
    },
    OpenDocument(PathBuf),
    OverlappingSources(PathBuf),
}

impl fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::InvalidRoot(path) => write!(
                f,
                "workspace is not an existing directory: {}",
                path.display()
            ),
            Self::InvalidPath(path) => {
                write!(f, "invalid workspace-relative path: {}", path.display())
            }
            Self::OutsideWorkspace(path) => {
                write!(f, "path is outside the workspace: {}", path.display())
            }
            Self::ReparsePoint(path) => {
                write!(f, "reparse points are not allowed here: {}", path.display())
            }
            Self::ProtectedPath(path) => {
                write!(f, "path is reserved for workspace data: {}", path.display())
            }
            Self::InvalidName(name) => write!(f, "invalid file or folder name: {name}"),
            Self::InvalidGitignore(path) => {
                write!(f, "cannot safely update .gitignore: {}", path.display())
            }
            Self::UnsupportedEntry(path) => {
                write!(f, "unsupported filesystem entry: {}", path.display())
            }
            Self::Collision(path) => write!(f, "destination already exists: {}", path.display()),
            Self::SamePath(path) => write!(
                f,
                "source and destination resolve to the same path: {}",
                path.display()
            ),
            Self::SelfChild {
                source,
                destination,
            } => write!(
                f,
                "cannot place a directory inside itself ({} -> {})",
                source.display(),
                destination.display()
            ),
            Self::OpenDocument(path) => write!(
                f,
                "cannot move an open document or one of its ancestor folders: {}",
                path.display()
            ),
            Self::OverlappingSources(path) => {
                write!(f, "selected paths overlap: {}", path.display())
            }
        }
    }
}

impl std::error::Error for WorkspaceError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for WorkspaceError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug)]
pub struct WorkspaceIdentity {
    canonical_root: PathBuf,
}

impl WorkspaceIdentity {
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub fn same_workspace(&self, other: &Self) -> bool {
        path_equal(&self.canonical_root, &other.canonical_root)
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    root: PathBuf,
    identity: WorkspaceIdentity,
}

impl Workspace {
    /// Open an existing directory without creating workspace state.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WorkspaceError> {
        let requested = path.as_ref();
        let canonical_root = fs::canonicalize(requested).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                WorkspaceError::InvalidRoot(requested.to_path_buf())
            } else {
                WorkspaceError::Io(error)
            }
        })?;
        if !fs::metadata(&canonical_root)?.is_dir() {
            return Err(WorkspaceError::InvalidRoot(requested.to_path_buf()));
        }
        let identity = WorkspaceIdentity {
            canonical_root: canonical_root.clone(),
        };
        Ok(Self {
            root: canonical_root,
            identity,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }

    /// Create missing workspace support directories and append only the two
    /// required ignore rules. Existing files are never replaced.
    pub fn initialize(&self) -> Result<InitializationReport, WorkspaceError> {
        let state_root = self.root.join(".mdlite");
        let state = state_root.join(".state");
        let cache = state_root.join(".cache");
        let gitignore = self.root.join(".gitignore");

        for (path, expected_directory) in [
            (&state_root, true),
            (&state, true),
            (&cache, true),
            (&gitignore, false),
        ] {
            match fs::symlink_metadata(path) {
                Ok(metadata) => {
                    if is_reparse_point(&metadata) {
                        return Err(WorkspaceError::ReparsePoint(path.clone()));
                    }
                    if expected_directory != metadata.is_dir()
                        || (!expected_directory && !metadata.is_file())
                    {
                        return Err(if path == &gitignore {
                            WorkspaceError::InvalidGitignore(path.clone())
                        } else {
                            WorkspaceError::UnsupportedEntry(path.clone())
                        });
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(WorkspaceError::Io(error)),
            }
        }

        let original_gitignore = match fs::read(&gitignore) {
            Ok(bytes) => Some(
                String::from_utf8(bytes)
                    .map_err(|_| WorkspaceError::InvalidGitignore(gitignore.clone()))?,
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(WorkspaceError::Io(error)),
        };
        let mut missing_rules = Vec::new();
        if !has_effective_rule(original_gitignore.as_deref().unwrap_or(""), STATE_IGNORE) {
            missing_rules.push(STATE_IGNORE);
        }
        if !has_effective_rule(original_gitignore.as_deref().unwrap_or(""), CACHE_IGNORE) {
            missing_rules.push(CACHE_IGNORE);
        }

        let mut report = InitializationReport::default();
        for directory in [&state_root, &state, &cache] {
            if path_exists(directory) {
                continue;
            }
            match fs::create_dir(directory) {
                Ok(()) => report.created_directories.push(directory.clone()),
                Err(error) => {
                    report.failures.push(InitializationFailure {
                        path: directory.clone(),
                        message: error.to_string(),
                    });
                    break;
                }
            }
        }

        if !missing_rules.is_empty() {
            let eol = original_gitignore
                .as_deref()
                .and_then(detect_line_ending)
                .unwrap_or("\n");
            let mut addition = String::new();
            if let Some(contents) = original_gitignore.as_deref()
                && !contents.is_empty()
                && !contents.ends_with('\n')
            {
                addition.push_str(eol);
            }
            for rule in &missing_rules {
                addition.push_str(rule);
                addition.push_str(eol);
            }

            let write_result = if original_gitignore.is_some() {
                OpenOptions::new()
                    .append(true)
                    .open(&gitignore)
                    .and_then(|mut file| file.write_all(addition.as_bytes()))
            } else {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&gitignore)
                    .and_then(|mut file| file.write_all(addition.as_bytes()))
            };
            match write_result {
                Ok(()) => {
                    report.added_ignore_rules =
                        missing_rules.into_iter().map(str::to_owned).collect()
                }
                Err(error) => report.failures.push(InitializationFailure {
                    path: gitignore,
                    message: error.to_string(),
                }),
            }
        }
        Ok(report)
    }

    /// List one directory in stable folder-first order. Protected paths and
    /// reparse points are omitted; read and metadata errors remain visible.
    pub fn list_directory(
        &self,
        directory: impl AsRef<Path>,
    ) -> Result<ExplorerListing, WorkspaceError> {
        let directory = self.resolve_existing(directory.as_ref())?;
        if !fs::metadata(&directory)?.is_dir() {
            return Err(WorkspaceError::UnsupportedEntry(directory));
        }

        let mut listing = ExplorerListing {
            directory: directory.clone(),
            entries: Vec::new(),
            issues: Vec::new(),
        };
        let read_dir = fs::read_dir(&directory)?;
        let mut raw_entries = Vec::new();
        for item in read_dir {
            match item {
                Ok(item) => raw_entries.push(item),
                Err(error) => listing.issues.push(ExplorerIssue {
                    path: directory.clone(),
                    kind: ExplorerIssueKind::Metadata,
                    message: error.to_string(),
                }),
            }
        }
        raw_entries.sort_by(|left, right| compare_names(&left.file_name(), &right.file_name()));

        for item in raw_entries {
            let path = item.path();
            if self.is_protected(&path)? {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    listing.issues.push(ExplorerIssue {
                        path,
                        kind: ExplorerIssueKind::Metadata,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if is_reparse_point(&metadata) {
                listing.issues.push(ExplorerIssue {
                    path,
                    kind: ExplorerIssueKind::ReparsePoint,
                    message: "entry skipped".to_owned(),
                });
                continue;
            }
            let kind = if metadata.is_dir() {
                ExplorerEntryKind::Folder
            } else if metadata.is_file() {
                ExplorerEntryKind::File
            } else {
                listing.issues.push(ExplorerIssue {
                    path,
                    kind: ExplorerIssueKind::Unsupported,
                    message: "entry skipped".to_owned(),
                });
                continue;
            };
            listing.entries.push(ExplorerEntry {
                name: item.file_name(),
                path,
                kind,
            });
        }
        listing.entries.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| compare_names(&left.name, &right.name))
        });
        Ok(listing)
    }

    /// Create an empty file using create-new semantics.
    pub fn create_file(
        &self,
        parent: impl AsRef<Path>,
        name: impl AsRef<OsStr>,
    ) -> Result<PathBuf, WorkspaceError> {
        let parent = self.resolve_directory(parent.as_ref())?;
        let name = name.as_ref();
        validate_name(name)?;
        let target = parent.join(Path::new(name));
        self.ensure_user_path(&target)?;
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(_) => Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(WorkspaceError::Collision(target))
            }
            Err(error) => Err(WorkspaceError::Io(error)),
        }
    }

    /// Create a folder without replacing an existing entry.
    pub fn create_folder(
        &self,
        parent: impl AsRef<Path>,
        name: impl AsRef<OsStr>,
    ) -> Result<PathBuf, WorkspaceError> {
        let parent = self.resolve_directory(parent.as_ref())?;
        let name = name.as_ref();
        validate_name(name)?;
        let target = parent.join(Path::new(name));
        self.ensure_user_path(&target)?;
        match fs::create_dir(&target) {
            Ok(()) => Ok(target),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                Err(WorkspaceError::Collision(target))
            }
            Err(error) => Err(WorkspaceError::Io(error)),
        }
    }

    /// Copy files and folders into an existing workspace directory. All
    /// selected trees and root destinations are checked before the first write.
    pub fn copy_items(
        &self,
        sources: &[PathBuf],
        destination: impl AsRef<Path>,
    ) -> Result<OperationReport, WorkspaceError> {
        let destination = self.resolve_directory(destination.as_ref())?;
        let plans = self.plan_items(sources, &destination)?;
        let mut report = OperationReport::default();
        for plan in plans {
            let mut created_paths = Vec::new();
            match copy_planned(&plan, &mut created_paths) {
                Ok(()) => report.completed.push(plan.destination),
                Err(error) => report.failures.push(OperationFailure {
                    source: plan.source,
                    destination: plan.destination,
                    message: error.to_string(),
                    created_paths,
                }),
            }
        }
        Ok(report)
    }

    /// Move files and folders into an existing workspace directory. Open files
    /// and any directory containing an open file are refused, even when clean.
    pub fn move_items(
        &self,
        sources: &[PathBuf],
        destination: impl AsRef<Path>,
        open_documents: &[PathBuf],
    ) -> Result<OperationReport, WorkspaceError> {
        let destination = self.resolve_directory(destination.as_ref())?;
        let resolved_sources = self.resolve_sources(sources)?;
        let resolved_open = open_documents
            .iter()
            .map(|path| self.resolve_existing(path))
            .collect::<Result<Vec<_>, _>>()?;
        let mut targets = HashSet::new();
        let mut planned = Vec::new();
        for source in &resolved_sources {
            if source == &self.root {
                return Err(WorkspaceError::ProtectedPath(source.clone()));
            }
            if resolved_open
                .iter()
                .any(|opened| path_is_ancestor_or_same(source, opened))
            {
                return Err(WorkspaceError::OpenDocument(source.clone()));
            }
            let name = source
                .file_name()
                .ok_or_else(|| WorkspaceError::InvalidPath(source.clone()))?;
            validate_name(name)?;
            let target = destination.join(name);
            self.ensure_user_path(&target)?;
            if path_equal(source, &target) {
                return Err(WorkspaceError::SamePath(source.clone()));
            }
            ensure_missing(&target)?;
            if fs::metadata(source)?.is_dir() && path_is_ancestor_or_same(source, &destination) {
                return Err(WorkspaceError::SelfChild {
                    source: source.clone(),
                    destination: destination.clone(),
                });
            }
            let key = path_key(&target);
            if !targets.insert(key) {
                return Err(WorkspaceError::Collision(target));
            }
            planned.push((source.clone(), target));
        }

        let mut report = OperationReport::default();
        for (source, target) in planned {
            match rename_without_replace(&source, &target) {
                Ok(()) => report.completed.push(target),
                Err(error) => report.failures.push(OperationFailure {
                    source,
                    destination: target,
                    message: error.to_string(),
                    created_paths: Vec::new(),
                }),
            }
        }
        Ok(report)
    }

    fn resolve_directory(&self, path: &Path) -> Result<PathBuf, WorkspaceError> {
        let resolved = self.resolve_existing(path)?;
        if !fs::metadata(&resolved)?.is_dir() {
            return Err(WorkspaceError::UnsupportedEntry(resolved));
        }
        Ok(resolved)
    }

    fn resolve_sources(&self, sources: &[PathBuf]) -> Result<Vec<PathBuf>, WorkspaceError> {
        if sources.is_empty() {
            return Err(WorkspaceError::InvalidPath(PathBuf::new()));
        }
        let mut resolved: Vec<PathBuf> = Vec::with_capacity(sources.len());
        for source in sources {
            let path = self.resolve_existing(source)?;
            if path == self.root {
                return Err(WorkspaceError::ProtectedPath(path));
            }
            if resolved.iter().any(|previous| {
                path_is_ancestor_or_same(previous, &path)
                    || path_is_ancestor_or_same(&path, previous)
            }) {
                return Err(WorkspaceError::OverlappingSources(path));
            }
            resolved.push(path);
        }
        Ok(resolved)
    }

    fn plan_items(
        &self,
        sources: &[PathBuf],
        destination: &Path,
    ) -> Result<Vec<CopyPlan>, WorkspaceError> {
        let sources = self.resolve_sources(sources)?;
        let mut target_keys = HashSet::new();
        let mut plans = Vec::with_capacity(sources.len());
        for source in sources {
            let name = source
                .file_name()
                .ok_or_else(|| WorkspaceError::InvalidPath(source.clone()))?;
            validate_name(name)?;
            let target = destination.join(name);
            self.ensure_user_path(&target)?;
            if path_equal(&source, &target) {
                return Err(WorkspaceError::SamePath(source));
            }
            if fs::metadata(&source)?.is_dir() && path_is_ancestor_or_same(&source, destination) {
                return Err(WorkspaceError::SelfChild {
                    source,
                    destination: destination.to_path_buf(),
                });
            }
            ensure_missing(&target)?;
            plans.push(self.plan_copy_node(source, target, &mut target_keys)?);
        }
        Ok(plans)
    }

    fn plan_copy_node(
        &self,
        source: PathBuf,
        destination: PathBuf,
        target_keys: &mut HashSet<String>,
    ) -> Result<CopyPlan, WorkspaceError> {
        let relative = self.relative_components(&source)?;
        if is_protected_relative(&relative) {
            return Err(WorkspaceError::ProtectedPath(source));
        }
        let metadata = fs::symlink_metadata(&source)?;
        if is_reparse_point(&metadata) {
            return Err(WorkspaceError::ReparsePoint(source));
        }
        let kind = if metadata.is_dir() {
            ExplorerEntryKind::Folder
        } else if metadata.is_file() {
            ExplorerEntryKind::File
        } else {
            return Err(WorkspaceError::UnsupportedEntry(source));
        };
        if !target_keys.insert(path_key(&destination)) {
            return Err(WorkspaceError::Collision(destination));
        }
        let mut plan = CopyPlan {
            source: source.clone(),
            destination,
            kind,
            children: Vec::new(),
        };
        if kind == ExplorerEntryKind::Folder {
            let mut entries = fs::read_dir(&source)?.collect::<Result<Vec<_>, _>>()?;
            entries.sort_by(|left, right| compare_names(&left.file_name(), &right.file_name()));
            for entry in entries {
                let child_source = self.resolve_existing(&entry.path())?;
                let name = child_source
                    .file_name()
                    .ok_or_else(|| WorkspaceError::InvalidPath(child_source.clone()))?;
                validate_name(name)?;
                let child_destination = plan.destination.join(name);
                self.ensure_user_path(&child_destination)?;
                plan.children.push(self.plan_copy_node(
                    child_source,
                    child_destination,
                    target_keys,
                )?);
            }
        }
        Ok(plan)
    }

    fn resolve_existing(&self, path: &Path) -> Result<PathBuf, WorkspaceError> {
        let components = self.relative_components(path)?;
        let mut candidate = self.root.clone();
        for component in &components {
            candidate.push(component);
            let metadata = fs::symlink_metadata(&candidate)?;
            if is_reparse_point(&metadata) {
                return Err(WorkspaceError::ReparsePoint(candidate));
            }
        }
        let canonical = fs::canonicalize(&candidate)?;
        if !path_is_ancestor_or_same(&self.root, &canonical) {
            return Err(WorkspaceError::OutsideWorkspace(path.to_path_buf()));
        }
        if is_protected_relative(&components) {
            return Err(WorkspaceError::ProtectedPath(canonical));
        }
        Ok(canonical)
    }

    fn relative_components(&self, path: &Path) -> Result<Vec<OsString>, WorkspaceError> {
        let root_components: Vec<_> = self.root.components().collect();
        if !path.is_absolute() {
            return path
                .components()
                .try_fold(Vec::new(), |mut relative, component| {
                    match component {
                        Component::Normal(value) => relative.push(value.to_os_string()),
                        Component::CurDir => {}
                        _ => return Err(WorkspaceError::InvalidPath(path.to_path_buf())),
                    }
                    Ok(relative)
                });
        }
        let mut absolute = path.to_path_buf();
        let mut path_components: Vec<_> = absolute.components().collect();
        let direct_match = path_components.len() >= root_components.len()
            && root_components
                .iter()
                .zip(path_components.iter())
                .all(|(root, input)| components_equal(root, input));
        if !direct_match {
            absolute = fs::canonicalize(path)
                .map_err(|_| WorkspaceError::OutsideWorkspace(path.to_path_buf()))?;
            path_components = absolute.components().collect();
            if path_components.len() < root_components.len()
                || !root_components
                    .iter()
                    .zip(path_components.iter())
                    .all(|(root, input)| components_equal(root, input))
            {
                return Err(WorkspaceError::OutsideWorkspace(path.to_path_buf()));
            }
        }
        path_components.iter().skip(root_components.len()).try_fold(
            Vec::new(),
            |mut relative, component| {
                match component {
                    Component::Normal(value) => relative.push(value.to_os_string()),
                    Component::CurDir => {}
                    _ => return Err(WorkspaceError::InvalidPath(path.to_path_buf())),
                }
                Ok(relative)
            },
        )
    }

    fn ensure_user_path(&self, path: &Path) -> Result<(), WorkspaceError> {
        let relative = self.relative_components(path)?;
        if is_protected_relative(&relative) {
            return Err(WorkspaceError::ProtectedPath(path.to_path_buf()));
        }
        Ok(())
    }

    fn is_protected(&self, path: &Path) -> Result<bool, WorkspaceError> {
        Ok(is_protected_relative(&self.relative_components(path)?))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExplorerEntryKind {
    Folder,
    File,
}

#[derive(Clone, Debug)]
pub struct ExplorerEntry {
    pub name: OsString,
    pub path: PathBuf,
    pub kind: ExplorerEntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExplorerIssueKind {
    Metadata,
    ReparsePoint,
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct ExplorerIssue {
    pub path: PathBuf,
    pub kind: ExplorerIssueKind,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct ExplorerListing {
    pub directory: PathBuf,
    pub entries: Vec<ExplorerEntry>,
    pub issues: Vec<ExplorerIssue>,
}

impl ExplorerListing {
    pub fn is_complete(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Default)]
pub struct InitializationReport {
    pub created_directories: Vec<PathBuf>,
    pub added_ignore_rules: Vec<String>,
    pub failures: Vec<InitializationFailure>,
}

impl InitializationReport {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug)]
pub struct InitializationFailure {
    pub path: PathBuf,
    pub message: String,
}

#[derive(Debug, Default)]
pub struct OperationReport {
    pub completed: Vec<PathBuf>,
    pub failures: Vec<OperationFailure>,
}

impl OperationReport {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

#[derive(Debug)]
pub struct OperationFailure {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub message: String,
    /// Entries created by this copy before it stopped. No rollback is implied.
    pub created_paths: Vec<PathBuf>,
}

#[derive(Debug)]
struct CopyPlan {
    source: PathBuf,
    destination: PathBuf,
    kind: ExplorerEntryKind,
    children: Vec<CopyPlan>,
}

fn copy_planned(plan: &CopyPlan, created: &mut Vec<PathBuf>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(&plan.source)?;
    if is_reparse_point(&metadata) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "source became a reparse point",
        ));
    }
    match plan.kind {
        ExplorerEntryKind::Folder => {
            if !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source folder changed type",
                ));
            }
            fs::create_dir(&plan.destination)?;
            created.push(plan.destination.clone());
            for child in &plan.children {
                copy_planned(child, created)?;
            }
            Ok(())
        }
        ExplorerEntryKind::File => {
            if !metadata.is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "source file changed type",
                ));
            }
            let mut input = File::open(&plan.source)?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&plan.destination)?;
            created.push(plan.destination.clone());
            io::copy(&mut input, &mut output)?;
            output.flush()
        }
    }
}

fn validate_name(name: &OsStr) -> Result<(), WorkspaceError> {
    let Some(name) = name.to_str() else {
        return Err(WorkspaceError::InvalidName(
            name.to_string_lossy().into_owned(),
        ));
    };
    let reserved = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let device_name = matches!(reserved.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (reserved.len() == 4
            && (reserved.starts_with("COM") || reserved.starts_with("LPT"))
            && matches!(reserved.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || matches!(name, "." | "..")
        || name.ends_with(' ')
        || name.ends_with('.')
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
        || device_name
    {
        return Err(WorkspaceError::InvalidName(name.to_owned()));
    }
    Ok(())
}

fn is_protected_relative(components: &[OsString]) -> bool {
    components.iter().enumerate().any(|(index, component)| {
        os_equal(component, OsStr::new(".git"))
            || (os_equal(component, OsStr::new(".mdlite"))
                && components.get(index + 1).is_some_and(|next| {
                    os_equal(next, OsStr::new(".state")) || os_equal(next, OsStr::new(".cache"))
                }))
    })
}

fn has_effective_rule(contents: &str, rule: &str) -> bool {
    let unignore = format!("!{rule}");
    let mut state = None;
    for line in contents.lines().map(|line| line.trim_end_matches('\r')) {
        let line = line.trim();
        if line == rule || line == rule.trim_start_matches('/') {
            state = Some(true);
        } else if line == unignore {
            state = Some(false);
        }
    }
    state == Some(true)
}

fn detect_line_ending(contents: &str) -> Option<&'static str> {
    contents.find('\n').map(|index| {
        if contents.as_bytes().get(index.saturating_sub(1)) == Some(&b'\r') {
            "\r\n"
        } else {
            "\n"
        }
    })
}

fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn ensure_missing(path: &Path) -> Result<(), WorkspaceError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(WorkspaceError::Collision(path.to_path_buf())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(WorkspaceError::Io(error)),
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

fn rename_without_replace(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
        // Deliberately omit MOVEFILE_REPLACE_EXISTING.
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (source, destination);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "safe no-replace move is unavailable on this platform",
        ))
    }
}

fn components_equal(left: &Component<'_>, right: &Component<'_>) -> bool {
    match (left, right) {
        (Component::Prefix(left), Component::Prefix(right)) => {
            os_equal(left.as_os_str(), right.as_os_str())
        }
        (Component::RootDir, Component::RootDir) => true,
        (Component::Normal(left), Component::Normal(right)) => os_equal(left, right),
        _ => false,
    }
}

fn path_is_ancestor_or_same(ancestor: &Path, child: &Path) -> bool {
    let ancestor: Vec<_> = ancestor.components().collect();
    let child: Vec<_> = child.components().collect();
    ancestor.len() <= child.len()
        && ancestor
            .iter()
            .zip(child.iter())
            .all(|(left, right)| components_equal(left, right))
}

fn path_equal(left: &Path, right: &Path) -> bool {
    path_is_ancestor_or_same(left, right) && path_is_ancestor_or_same(right, left)
}

fn os_equal(left: &OsStr, right: &OsStr) -> bool {
    #[cfg(windows)]
    {
        left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
    }
    #[cfg(not(windows))]
    {
        left == right
    }
}

fn path_key(path: &Path) -> String {
    let key = path.to_string_lossy().replace('\\', "/");
    #[cfg(windows)]
    {
        key.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        key
    }
}

fn compare_names(left: &OsStr, right: &OsStr) -> std::cmp::Ordering {
    left.to_string_lossy()
        .to_lowercase()
        .cmp(&right.to_string_lossy().to_lowercase())
        .then_with(|| left.cmp(right))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TestWorkspace(PathBuf);

    impl TestWorkspace {
        fn new() -> Self {
            loop {
                let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir()
                    .join(format!("mdlite-workspace-test-{}-{id}", std::process::id()));
                match fs::create_dir(&root) {
                    Ok(()) => return Self(root),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("failed to create fixture: {error}"),
                }
            }
        }

        fn workspace(&self) -> Workspace {
            Workspace::open(&self.0).unwrap()
        }
    }

    impl Drop for TestWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn open_does_not_initialize_and_identity_uses_canonical_root() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        assert!(!fixture.0.join(".mdlite").exists());
        assert!(workspace.identity().same_workspace(workspace.identity()));
        assert!(matches!(
            Workspace::open(fixture.0.join("missing")),
            Err(WorkspaceError::InvalidRoot(_))
        ));
    }

    #[test]
    fn initialize_preserves_existing_files_and_is_idempotent() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();
        fs::write(fixture.0.join(".mdlite/templates/keep.md"), "template").unwrap();
        fs::write(fixture.0.join(".gitignore"), "user-rule\n").unwrap();

        let first = workspace.initialize().unwrap();
        assert!(first.is_complete());
        assert_eq!(first.created_directories.len(), 2);
        assert_eq!(
            fs::read(fixture.0.join(".mdlite/templates/keep.md")).unwrap(),
            b"template"
        );
        let gitignore = fs::read_to_string(fixture.0.join(".gitignore")).unwrap();
        assert!(gitignore.starts_with("user-rule\n"));
        assert!(gitignore.contains(STATE_IGNORE));
        assert!(gitignore.contains(CACHE_IGNORE));

        let second = workspace.initialize().unwrap();
        assert!(second.is_complete());
        assert!(second.created_directories.is_empty());
        assert!(second.added_ignore_rules.is_empty());
        assert_eq!(
            fs::read_to_string(fixture.0.join(".gitignore")).unwrap(),
            gitignore
        );
    }

    #[test]
    fn explorer_is_folder_first_stable_and_hides_protected_paths() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        fs::create_dir(fixture.0.join("z-folder")).unwrap();
        fs::create_dir(fixture.0.join("A-folder")).unwrap();
        fs::write(fixture.0.join("B.md"), "").unwrap();
        fs::write(fixture.0.join("a.md"), "").unwrap();
        fs::create_dir_all(fixture.0.join(".git/objects")).unwrap();
        fs::create_dir_all(fixture.0.join(".mdlite/.state")).unwrap();
        fs::create_dir_all(fixture.0.join(".mdlite/.cache")).unwrap();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();

        let listing = workspace.list_directory(".").unwrap();
        assert!(listing.is_complete());
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, [".mdlite", "A-folder", "z-folder", "a.md", "B.md"]);
        assert_eq!(listing.entries[0].kind, ExplorerEntryKind::Folder);
        assert_eq!(listing.entries[3].kind, ExplorerEntryKind::File);
    }

    #[test]
    fn create_uses_validated_names_and_never_replaces() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        let file = workspace.create_file(".", "note.md").unwrap();
        fs::write(&file, "original").unwrap();
        assert!(matches!(
            workspace.create_file(".", "note.md"),
            Err(WorkspaceError::Collision(_))
        ));
        assert!(matches!(
            workspace.create_folder(".", "CON"),
            Err(WorkspaceError::InvalidName(_))
        ));
        assert_eq!(fs::read(file).unwrap(), b"original");
    }

    #[test]
    fn copy_preflights_all_targets_and_copies_folder_trees() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        let destination = fixture.0.join("destination");
        let folder = fixture.0.join("folder");
        fs::create_dir(&destination).unwrap();
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("nested.md"), "nested").unwrap();
        fs::write(fixture.0.join("first.md"), "first").unwrap();
        fs::write(fixture.0.join("second.md"), "second").unwrap();
        fs::write(destination.join("second.md"), "keep").unwrap();

        assert!(matches!(
            workspace.copy_items(
                &[fixture.0.join("first.md"), fixture.0.join("second.md")],
                &destination
            ),
            Err(WorkspaceError::Collision(_))
        ));
        assert!(!destination.join("first.md").exists());
        assert_eq!(fs::read(destination.join("second.md")).unwrap(), b"keep");

        let report = workspace
            .copy_items(std::slice::from_ref(&folder), &destination)
            .unwrap();
        assert!(report.is_complete());
        assert_eq!(
            fs::read(destination.join("folder/nested.md")).unwrap(),
            b"nested"
        );
    }

    #[cfg(windows)]
    #[test]
    fn move_refuses_open_files_and_ancestors_then_moves_without_overwrite() {
        let fixture = TestWorkspace::new();
        let workspace = fixture.workspace();
        let source = fixture.0.join("source");
        let destination = fixture.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        let open_file = source.join("open.md");
        fs::write(&open_file, "contents").unwrap();

        assert!(matches!(
            workspace.move_items(
                std::slice::from_ref(&source),
                &destination,
                std::slice::from_ref(&open_file)
            ),
            Err(WorkspaceError::OpenDocument(_))
        ));
        assert!(source.exists());
        let report = workspace
            .move_items(std::slice::from_ref(&source), &destination, &[])
            .unwrap();
        assert!(report.is_complete());
        assert!(!source.exists());
        assert_eq!(
            fs::read(destination.join("source/open.md")).unwrap(),
            b"contents"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected_and_listed_as_incomplete() {
        use std::os::unix::fs::symlink;

        let fixture = TestWorkspace::new();
        let outside = TestWorkspace::new();
        let workspace = fixture.workspace();
        fs::write(outside.0.join("secret.md"), "outside").unwrap();
        symlink(&outside.0, fixture.0.join("escape")).unwrap();

        let listing = workspace.list_directory(".").unwrap();
        assert!(!listing.is_complete());
        assert!(listing.entries.iter().all(|entry| entry.name != "escape"));
        assert!(matches!(
            workspace.copy_items(&[PathBuf::from("escape")], "."),
            Err(WorkspaceError::ReparsePoint(_))
        ));
    }
}
