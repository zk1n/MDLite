//! Explicit Git operations. Nothing runs when a workspace is constructed/opened.
//! UI callers run these blocking methods on a worker, then check `RequestIdentity`.
use crate::{
    process::{self, Cancellation, ProcessOutcome, ProcessRequest, ProcessResult},
    trust::{TrustStore, TrustToken, WorkspaceIdentity},
};
use std::{
    ffi::OsString,
    fs,
    ops::Range,
    path::{Component, Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestIdentity {
    pub generation: u64,
    pub workspace: WorkspaceIdentity,
}
impl RequestIdentity {
    pub fn is_current(&self, generation: u64, active: &Path, requested: &Path) -> bool {
        self.generation == generation
            && WorkspaceIdentity::read(active).is_ok_and(|i| i == self.workspace)
            && WorkspaceIdentity::read(requested).is_ok_and(|i| i == self.workspace)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitState {
    NoGit,
    NoRepository,
    NoRemote,
    Ready,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    pub path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub index: u8,
    pub worktree: u8,
    pub untracked: bool,
    pub conflicted: bool,
}
impl FileStatus {
    pub fn staged(&self) -> bool {
        !self.untracked && self.index != b'.'
    }
    pub fn unstaged(&self) -> bool {
        self.untracked || self.worktree != b'.'
    }
}
#[derive(Clone, Debug)]
pub struct GitStatus {
    pub state: GitState,
    /// Repository-wide scope may include files outside the trusted workspace subdirectory.
    pub repository_root: PathBuf,
    pub branch: String,
    pub unborn: bool,
    pub detached: bool,
    pub remotes: Vec<String>,
    pub files: Vec<FileStatus>,
}
impl Default for GitStatus {
    fn default() -> Self {
        Self {
            state: GitState::Ready,
            repository_root: PathBuf::new(),
            branch: String::new(),
            unborn: false,
            detached: false,
            remotes: vec![],
            files: vec![],
        }
    }
}
#[derive(Debug)]
pub enum GitError {
    NoWorkspace,
    Untrusted(String),
    Invalid(String),
    IncompleteOutput,
    Process(ProcessResult),
}

/// Parse complete `status --porcelain=v2 --branch -z`. Reject truncated/malformed records.
pub fn parse_status(bytes: &[u8]) -> Result<GitStatus, GitError> {
    if bytes.is_empty() || !bytes.ends_with(&[0]) {
        return Err(GitError::IncompleteOutput);
    }
    let mut status = GitStatus::default();
    let mut records = bytes[..bytes.len() - 1].split(|b| *b == 0);
    let mut oid = false;
    let mut head = false;
    while let Some(record) = records.next() {
        if record.starts_with(b"# ") {
            let text = std::str::from_utf8(record).map_err(|_| GitError::IncompleteOutput)?;
            if let Some(value) = text.strip_prefix("# branch.oid ") {
                oid = true;
                status.unborn = value == "(initial)";
                if !status.unborn
                    && (value.len() < 40 || !value.bytes().all(|b| b.is_ascii_hexdigit()))
                {
                    return Err(GitError::IncompleteOutput);
                }
            } else if let Some(value) = text.strip_prefix("# branch.head ") {
                head = !value.is_empty();
                status.detached = value == "(detached)";
                status.branch = value.to_owned();
            } else if !text.starts_with("# branch.upstream ") && !text.starts_with("# branch.ab ") {
                return Err(GitError::IncompleteOutput);
            }
            continue;
        }
        let (parts, path_index, conflict, untracked) = match record.first() {
            Some(b'1') => (
                record.splitn(9, |b| *b == b' ').collect::<Vec<_>>(),
                8,
                false,
                false,
            ),
            Some(b'2') => (record.splitn(10, |b| *b == b' ').collect(), 9, false, false),
            Some(b'u') => (record.splitn(11, |b| *b == b' ').collect(), 10, true, false),
            Some(b'?') => (record.splitn(2, |b| *b == b' ').collect(), 1, false, true),
            _ => return Err(GitError::IncompleteOutput),
        };
        if parts.len() != path_index + 1 {
            return Err(GitError::IncompleteOutput);
        }
        let path = parse_path(parts[path_index])?;
        let xy = if untracked {
            b"??".as_slice()
        } else {
            parts[1]
        };
        if xy.len() != 2 || !xy.iter().all(|b| b".MADRCUT?!".contains(b)) {
            return Err(GitError::IncompleteOutput);
        }
        let original_path = if record[0] == b'2' {
            Some(parse_path(
                records.next().ok_or(GitError::IncompleteOutput)?,
            )?)
        } else {
            None
        };
        status.files.push(FileStatus {
            path,
            original_path,
            index: xy[0],
            worktree: xy[1],
            untracked,
            conflicted: conflict,
        });
    }
    if !oid || !head {
        return Err(GitError::IncompleteOutput);
    }
    Ok(status)
}
fn parse_path(bytes: &[u8]) -> Result<PathBuf, GitError> {
    let text = std::str::from_utf8(bytes).map_err(|_| GitError::IncompleteOutput)?;
    if text.is_empty() {
        return Err(GitError::IncompleteOutput);
    }
    Ok(PathBuf::from(text))
}

pub struct GitClient {
    pub executable: PathBuf,
    pub workspace: PathBuf,
    pub trust: TrustStore,
}
impl GitClient {
    pub fn request_identity(&self, generation: u64) -> Result<RequestIdentity, GitError> {
        Ok(RequestIdentity {
            generation,
            workspace: WorkspaceIdentity::read(&self.workspace).map_err(GitError::Untrusted)?,
        })
    }
    fn run(
        &self,
        token: &TrustToken,
        cwd: &Path,
        args: Vec<OsString>,
        cancellation: &Cancellation,
        generation: u64,
    ) -> Result<ProcessResult, GitError> {
        // Recheck immediately before EVERY process, including status, diff and discovery.
        self.trust
            .check(token, &self.workspace)
            .map_err(GitError::Untrusted)?;
        let mut arguments = vec![
            "--no-pager".into(),
            "--literal-pathspecs".into(),
            "-c".into(),
            "core.fsmonitor=false".into(),
            "-c".into(),
            "core.untrackedCache=false".into(),
            "-c".into(),
            "core.quotepath=false".into(),
        ];
        arguments.extend(args);
        Ok(process::run(&ProcessRequest {
            executable: self.executable.clone(),
            arguments,
            cwd: cwd.to_owned(),
            timeout: Duration::from_secs(120),
            output_limit: 1024 * 1024,
            generation,
            cancellation: cancellation.clone(),
            environment: vec![
                ("GIT_TERMINAL_PROMPT".into(), "0".into()),
                ("GCM_INTERACTIVE".into(), "Never".into()),
                ("LC_ALL".into(), "C".into()),
                ("GIT_OPTIONAL_LOCKS".into(), "0".into()),
            ],
            remove_environment: strings(&[
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_INDEX_FILE",
                "GIT_COMMON_DIR",
                "GIT_OBJECT_DIRECTORY",
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                "GIT_CONFIG_COUNT",
                "GIT_CONFIG_PARAMETERS",
            ]),
        }))
    }
    fn complete(
        &self,
        token: &TrustToken,
        cwd: &Path,
        args: Vec<OsString>,
        cancel: &Cancellation,
        generation: u64,
    ) -> Result<ProcessResult, GitError> {
        let result = self.run(token, cwd, args, cancel, generation)?;
        if !result.succeeded() {
            return Err(GitError::Process(result));
        }
        if result.truncated {
            return Err(GitError::IncompleteOutput);
        }
        Ok(result)
    }
    pub fn refresh(
        &self,
        token: &TrustToken,
        cancel: &Cancellation,
        generation: u64,
    ) -> Result<GitStatus, GitError> {
        if !self.workspace.is_dir() {
            return Err(GitError::NoWorkspace);
        }
        let root = self.run(
            token,
            &self.workspace,
            strings(&["rev-parse", "--show-toplevel"]),
            cancel,
            generation,
        )?;
        if matches!(root.outcome, ProcessOutcome::SpawnFailed(_)) {
            let available = self.executable.is_file()
                || std::env::var_os("PATH").is_some_and(|paths| {
                    std::env::split_paths(&paths).any(|dir| dir.join(&self.executable).is_file())
                });
            if available {
                return Err(GitError::Process(root));
            }
            return Ok(GitStatus {
                state: GitState::NoGit,
                ..Default::default()
            });
        }
        if !root.succeeded() {
            if root.outcome == ProcessOutcome::Exited(128)
                && std::str::from_utf8(&root.stderr).is_ok_and(|text| {
                    text.trim_end()
                        == "fatal: not a git repository (or any of the parent directories): .git"
                })
            {
                return Ok(GitStatus {
                    state: GitState::NoRepository,
                    ..Default::default()
                });
            }
            return Err(GitError::Process(root));
        }
        if root.truncated {
            return Err(GitError::IncompleteOutput);
        }
        let root = std::str::from_utf8(&root.stdout)
            .map_err(|_| GitError::IncompleteOutput)?
            .strip_suffix('\n')
            .ok_or(GitError::IncompleteOutput)?
            .trim_end_matches('\r');
        let root = fs::canonicalize(root).map_err(|e| GitError::Invalid(e.to_string()))?;
        let output = self.complete(
            token,
            &root,
            strings(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ]),
            cancel,
            generation,
        )?;
        let mut status = parse_status(&output.stdout)?;
        let remote = self.complete(token, &root, strings(&["remote"]), cancel, generation)?;
        status.remotes = std::str::from_utf8(&remote.stdout)
            .map_err(|_| GitError::IncompleteOutput)?
            .lines()
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .collect();
        status.state = if status.remotes.is_empty() {
            GitState::NoRemote
        } else {
            GitState::Ready
        };
        status.repository_root = root;
        Ok(status)
    }
    /// Only an explicit Trust grant or explicit retry may call this method.
    /// Trust is already durable; initialization failure must never revoke it.
    pub fn discover_or_init(
        &self,
        token: &TrustToken,
        cancel: &Cancellation,
        generation: u64,
    ) -> Result<GitStatus, GitError> {
        let _lease = self
            .trust
            .begin_mutation(token, &self.workspace)
            .map_err(GitError::Untrusted)?;
        let status = self.refresh(token, cancel, generation)?;
        if status.state != GitState::NoRepository {
            return Ok(status);
        }
        // Rediscover immediately before init: a parent repository must win over
        // creating a nested one. Any discovery error propagates without init.
        let status = self.refresh(token, cancel, generation)?;
        if status.state != GitState::NoRepository {
            return Ok(status);
        }
        self.complete(
            token,
            &token.identity().canonical,
            strings(&["init"]),
            cancel,
            generation,
        )?;
        self.refresh(token, cancel, generation)
    }
    pub fn diff(
        &self,
        token: &TrustToken,
        path: &Path,
        cancel: &Cancellation,
        generation: u64,
    ) -> Result<GitDiff, GitError> {
        let status = self.refresh(token, cancel, generation)?;
        validate_paths(&status.repository_root, &[path.to_owned()])?;
        let file = status
            .files
            .iter()
            .find(|f| f.path == path)
            .ok_or_else(|| GitError::Invalid("File has no changes".into()))?;
        let mut parts = Vec::new();
        let commands = if file.untracked {
            vec![strings(&[
                "diff",
                "--no-index",
                "--no-ext-diff",
                "--no-textconv",
                "--",
                "/dev/null",
            ])]
        } else {
            vec![
                strings(&["diff", "--cached", "--no-ext-diff", "--no-textconv", "--"]),
                strings(&["diff", "--no-ext-diff", "--no-textconv", "--"]),
            ]
        };
        for mut args in commands {
            args.push(path.as_os_str().to_owned());
            let result = self.run(token, &status.repository_root, args, cancel, generation)?;
            if !result.succeeded()
                && !(file.untracked && result.outcome == ProcessOutcome::Exited(1))
            {
                return Err(GitError::Process(result));
            }
            parts.push(result);
        }
        Ok(GitDiff { parts })
    }
    /// Every operation is explicit. This API never saves documents or chains stage/commit/push.
    pub fn execute(
        &self,
        token: &TrustToken,
        operation: &GitOperation,
        guard: ExecutionGuard,
        cancel: &Cancellation,
        generation: u64,
    ) -> Result<ProcessResult, GitError> {
        if operation.needs_confirmation() && !guard.confirmed {
            return Err(GitError::Invalid("Explicit confirmation required".into()));
        }
        if operation.needs_presave() && !guard.presave_completed {
            return Err(GitError::Invalid(
                "Pre-save must complete before Git confirmation".into(),
            ));
        }
        let _lease = self
            .trust
            .begin_mutation(token, &self.workspace)
            .map_err(GitError::Untrusted)?;
        let status = self.refresh(token, cancel, generation)?;
        if matches!(status.state, GitState::NoGit | GitState::NoRepository) {
            return Err(GitError::Invalid("Git repository unavailable".into()));
        }
        let args = build_command(operation, &status)?;
        self.run(token, &status.repository_root, args, cancel, generation)
    }
}
pub struct GitDiff {
    /// Index and worktree sections are separate; truncation is visible per section.
    pub parts: Vec<ProcessResult>,
}
#[derive(Clone, Copy, Default)]
pub struct ExecutionGuard {
    pub confirmed: bool,
    pub presave_completed: bool,
}
pub enum GitOperation {
    Stage(Vec<PathBuf>),
    Unstage(Vec<PathBuf>),
    Commit {
        message: String,
    },
    CreateBranch(String),
    SwitchBranch(String),
    Merge(String),
    AbortMerge,
    Fetch {
        remote: String,
    },
    PullFastForward {
        remote: String,
        branch: String,
    },
    Push {
        remote: String,
        branch: String,
    },
    /// Same validation and command builder as ordinary Stage; UI saves and confirms first.
    StageResolved(PathBuf),
}
impl GitOperation {
    pub fn needs_presave(&self) -> bool {
        matches!(
            self,
            Self::CreateBranch(_)
                | Self::SwitchBranch(_)
                | Self::Merge(_)
                | Self::AbortMerge
                | Self::PullFastForward { .. }
                | Self::StageResolved(_)
        )
    }
    pub fn needs_confirmation(&self) -> bool {
        !matches!(
            self,
            Self::Stage(_) | Self::Unstage(_) | Self::Commit { .. }
        )
    }
}
fn strings(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn safe_ref(value: &str) -> Result<(), GitError> {
    if value.is_empty()
        || value.starts_with('-')
        || value.starts_with('/')
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value == "@"
        || value
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || "~^:?*[\\".contains(c))
        || value
            .split('/')
            .any(|c| c.is_empty() || c.starts_with('.') || c.ends_with(".lock"))
    {
        return Err(GitError::Invalid("Invalid branch or remote name".into()));
    }
    Ok(())
}
pub fn validate_paths(root: &Path, paths: &[PathBuf]) -> Result<(), GitError> {
    if paths.is_empty() || root.as_os_str().is_empty() {
        return Err(GitError::Invalid("Explicit selected files required".into()));
    }
    let root = fs::canonicalize(root).map_err(|e| GitError::Invalid(e.to_string()))?;
    for path in paths {
        if path.as_os_str().is_empty()
            || path
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || path
                .as_os_str()
                .to_string_lossy()
                .split(['/', '\\'])
                .any(|c| {
                    c.is_empty()
                        || c == "."
                        || c == ".."
                        || c.eq_ignore_ascii_case(".git")
                        || c.contains(':')
                })
        {
            return Err(GitError::Invalid(
                "Repository-relative file path required".into(),
            ));
        }
        let full = root.join(path);
        if fs::symlink_metadata(&full).is_ok_and(|m| m.is_dir()) {
            return Err(GitError::Invalid("Select files, not directories".into()));
        }
        let mut parent = full
            .parent()
            .ok_or_else(|| GitError::Invalid("Missing parent".into()))?;
        while !parent.exists() {
            parent = parent
                .parent()
                .ok_or_else(|| GitError::Invalid("Missing parent".into()))?;
        }
        if !fs::canonicalize(parent)
            .map_err(|e| GitError::Invalid(e.to_string()))?
            .starts_with(&root)
        {
            return Err(GitError::Invalid(
                "Path traverses outside repository".into(),
            ));
        }
    }
    Ok(())
}
fn build_command(operation: &GitOperation, status: &GitStatus) -> Result<Vec<OsString>, GitError> {
    let remote_args =
        |remote: &str, branch: Option<&str>, command: &[&str]| -> Result<Vec<OsString>, GitError> {
            safe_ref(remote)?;
            if !status.remotes.iter().any(|r| r == remote) {
                return Err(GitError::Invalid("Select an existing remote".into()));
            }
            let mut args = strings(command);
            args.push(remote.into());
            if let Some(branch) = branch {
                safe_ref(branch)?;
                args.push(branch.into());
            }
            Ok(args)
        };
    match operation {
        GitOperation::Stage(paths) | GitOperation::Unstage(paths) => {
            validate_paths(&status.repository_root, paths)?;
            let mut args = if matches!(operation, GitOperation::Stage(_)) {
                strings(&["add", "--"])
            } else if status.unborn {
                strings(&["rm", "--cached", "--force", "--ignore-unmatch", "--"])
            } else {
                strings(&["restore", "--staged", "--"])
            };
            args.extend(paths.iter().map(|p| p.as_os_str().to_owned()));
            Ok(args)
        }
        GitOperation::StageResolved(path) => {
            if !status.files.iter().any(|f| f.path == *path && f.conflicted) {
                return Err(GitError::Invalid("File is not Git-unmerged".into()));
            }
            build_command(&GitOperation::Stage(vec![path.clone()]), status)
        }
        GitOperation::Commit { message } => {
            if message.trim().is_empty() || message.contains('\0') {
                return Err(GitError::Invalid("Commit message required".into()));
            }
            Ok(vec!["commit".into(), "-m".into(), message.into()])
        }
        GitOperation::CreateBranch(name) => {
            safe_ref(name)?;
            Ok(vec![
                "switch".into(),
                "--no-guess".into(),
                "-c".into(),
                name.into(),
                "--".into(),
            ])
        }
        GitOperation::SwitchBranch(name) => {
            safe_ref(name)?;
            Ok(vec![
                "switch".into(),
                "--no-guess".into(),
                name.into(),
                "--".into(),
            ])
        }
        GitOperation::Merge(name) => {
            safe_ref(name)?;
            Ok(vec![
                "merge".into(),
                "--no-edit".into(),
                "--no-autostash".into(),
                "--".into(),
                name.into(),
            ])
        }
        GitOperation::AbortMerge => Ok(strings(&["merge", "--abort"])),
        GitOperation::Fetch { remote } => remote_args(remote, None, &["fetch", "--"]),
        GitOperation::PullFastForward { remote, branch } => remote_args(
            remote,
            Some(branch),
            &["pull", "--ff-only", "--no-rebase", "--no-autostash", "--"],
        ),
        GitOperation::Push { remote, branch } => remote_args(remote, Some(branch), &["push", "--"]),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictBlock {
    pub range: Range<usize>,
    pub current: Range<usize>,
    pub incoming: Range<usize>,
}
#[derive(Clone, Copy)]
pub enum ConflictChoice {
    Current,
    Incoming,
    Both,
}
pub struct ConflictEdit {
    pub range: Range<usize>,
    pub replacement: String,
}
/// Byte ranges into the original canonical UTF-8 source, for one native Undo transaction.
pub fn conflict_blocks(source: &str) -> Vec<ConflictBlock> {
    let mut lines = Vec::new();
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        lines.push((
            offset,
            offset + line.len(),
            line.trim_end_matches(['\r', '\n']),
        ));
        offset += line.len();
    }
    let marker = |text: &str, symbol: char| {
        let count = text.chars().take_while(|c| *c == symbol).count();
        count >= 7 && (text.len() == count || text.as_bytes().get(count) == Some(&b' '))
    };
    let mut result = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        if !marker(lines[index].2, '<') {
            index += 1;
            continue;
        }
        let start = index;
        let mut base = None;
        let mut separator = None;
        let mut end = None;
        index += 1;
        while index < lines.len() {
            let text = lines[index].2;
            if marker(text, '<') {
                break;
            }
            if marker(text, '|') && base.is_none() && separator.is_none() {
                base = Some(index);
            } else if text.len() >= 7 && text.bytes().all(|c| c == b'=') && separator.is_none() {
                separator = Some(index);
            } else if marker(text, '>') && separator.is_some() {
                end = Some(index);
                break;
            }
            index += 1;
        }
        if let (Some(separator), Some(end)) = (separator, end) {
            result.push(ConflictBlock {
                range: lines[start].0..lines[end].1,
                current: lines[start].1..lines[base.unwrap_or(separator)].0,
                incoming: lines[separator].1..lines[end].0,
            });
            index = end + 1;
        } else {
            index = start + 1;
        }
    }
    result
}
pub fn conflict_edit(
    source: &str,
    file: &FileStatus,
    index: usize,
    choice: ConflictChoice,
) -> Result<ConflictEdit, GitError> {
    if !file.conflicted {
        return Err(GitError::Invalid(
            "Markers alone do not establish a Git conflict".into(),
        ));
    }
    let block = conflict_blocks(source)
        .get(index)
        .cloned()
        .ok_or_else(|| GitError::Invalid("Conflict block unavailable".into()))?;
    let mut replacement = String::new();
    if matches!(choice, ConflictChoice::Current | ConflictChoice::Both) {
        replacement.push_str(&source[block.current]);
    }
    if matches!(choice, ConflictChoice::Incoming | ConflictChoice::Both) {
        replacement.push_str(&source[block.incoming]);
    }
    Ok(ConflictEdit {
        range: block.range,
        replacement,
    })
}
pub fn adjacent_conflict(source: &str, position: usize, previous: bool) -> Option<usize> {
    let blocks = conflict_blocks(source);
    if blocks.is_empty() {
        return None;
    }
    if previous {
        blocks
            .iter()
            .rposition(|b| b.range.start < position)
            .or(Some(blocks.len() - 1))
    } else {
        blocks
            .iter()
            .position(|b| b.range.start > position)
            .or(Some(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_porcelain_rename_conflict_and_malformed() {
        let bytes = b"# branch.oid (initial)\0# branch.head main\0? space name.md\x001 A. N... 000000 100644 100644 abc def staged.md\x002 R. N... 100644 100644 100644 abc def R100 new.md\0old.md\0u UU N... 100644 100644 100644 100644 a b c conflict.md\0";
        let status = parse_status(bytes).unwrap();
        assert!(status.unborn);
        assert_eq!(status.files.len(), 4);
        assert_eq!(status.files[2].original_path, Some("old.md".into()));
        assert!(status.files[3].conflicted);
        assert!(parse_status(&bytes[..bytes.len() - 1]).is_err());
        assert!(
            parse_status(
                b"# branch.oid (initial)\0# branch.head main\x002 R. N... 1 2 3 a b R100 path\0"
            )
            .is_err()
        );
        assert!(parse_status(b"? no-header\0").is_err());
        let detached = parse_status(
            b"# branch.oid 0123456789012345678901234567890123456789\0# branch.head (detached)\0",
        )
        .unwrap();
        assert!(detached.detached);
    }
    #[test]
    fn diff3_one_block_edit_requires_git_conflict() {
        let source = "前\r\n<<<<<<< HEAD\r\n現在\r\n||||||| base\r\n祖先\r\n=======\r\nincoming\r\n>>>>>>> branch\r\n後\r\n<<<<<<< HEAD\nsecond\n=======\nother\n>>>>>>> topic\n";
        let blocks = conflict_blocks(source);
        assert_eq!(blocks.len(), 2);
        let mut file = FileStatus {
            path: "conflict.md".into(),
            original_path: None,
            index: b'U',
            worktree: b'U',
            untracked: false,
            conflicted: false,
        };
        assert!(conflict_edit(source, &file, 0, ConflictChoice::Both).is_err());
        file.conflicted = true;
        let edit = conflict_edit(source, &file, 0, ConflictChoice::Both).unwrap();
        assert_eq!(edit.replacement, "現在\r\nincoming\r\n");
        let mut changed = source.to_owned();
        changed.replace_range(edit.range, &edit.replacement);
        assert!(changed.starts_with("前\r\n現在\r\nincoming\r\n後\r\n"));
        assert_eq!(conflict_blocks(&changed).len(), 1);
        assert_eq!(adjacent_conflict(source, 0, true), Some(1));
        assert_eq!(adjacent_conflict(source, source.len(), false), Some(0));
    }
    #[cfg(windows)]
    struct Fixture {
        root: PathBuf,
        repo: PathBuf,
        client: GitClient,
        token: TrustToken,
    }
    #[cfg(windows)]
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mdlite-git-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let repo = root.join("repo");
            fs::create_dir_all(&repo).unwrap();
            fixture_git(&repo, &["init", "-b", "main"]);
            fixture_git(&repo, &["config", "user.name", "Synthetic fixture"]);
            fixture_git(&repo, &["config", "user.email", "fixture@example.invalid"]);
            // Never inherit user hooks/signing/transport rewriting in synthetic fixture operations.
            fixture_git(
                &repo,
                &["config", "core.hooksPath", ".git/fixture-no-hooks"],
            );
            fixture_git(&repo, &["config", "commit.gpgsign", "false"]);
            fixture_git(&repo, &["config", "core.autocrlf", "false"]);
            let trust = TrustStore::at(root.join("registry"));
            trust.set_trusted(&repo, true).unwrap();
            let token = trust.token(&repo).unwrap();
            Self {
                client: GitClient {
                    executable: "git.exe".into(),
                    workspace: repo.clone(),
                    trust,
                },
                repo,
                root,
                token,
            }
        }
        fn execute(&self, op: GitOperation) -> ProcessResult {
            let r = self
                .client
                .execute(
                    &self.token,
                    &op,
                    ExecutionGuard {
                        confirmed: true,
                        presave_completed: true,
                    },
                    &Cancellation::default(),
                    7,
                )
                .unwrap();
            assert!(r.succeeded(), "{r:?}");
            r
        }
        fn status(&self) -> GitStatus {
            self.client
                .refresh(&self.token, &Cancellation::default(), 7)
                .unwrap()
        }
    }
    #[cfg(windows)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    #[cfg(windows)]
    fn fixture_git(repo: &Path, args: &[&str]) -> Vec<u8> {
        let config = repo.join(".mdlite-fixture-empty.gitconfig");
        fs::write(&config, "").unwrap();
        let output = std::process::Command::new("git.exe")
            .args(args)
            .current_dir(repo)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", &config)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .unwrap();
        fs::remove_file(config).unwrap();
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        output.stdout
    }
    #[test]
    #[cfg(windows)]
    fn synthetic_repo_literal_selected_unborn_index_commit_and_local_remote() {
        let f = Fixture::new();
        let cancel = Cancellation::default();
        for name in ["[ab].md", "a.md", "space 日本語.md", "-dash.md"] {
            fs::write(f.repo.join(name), name).unwrap();
        }
        let initial = f.status();
        assert_eq!(initial.state, GitState::NoRemote);
        assert!(initial.unborn);
        assert_eq!(initial.files.len(), 4);
        for invalid in [
            "",
            ".",
            "..",
            "../a.md",
            ".git/config",
            "sub/../../a.md",
            "C:/a.md",
            ":(glob)*",
        ] {
            assert!(
                validate_paths(&f.repo, &[invalid.into()]).is_err(),
                "{invalid}"
            );
        }
        f.execute(GitOperation::Stage(vec![
            "[ab].md".into(),
            "space 日本語.md".into(),
            "-dash.md".into(),
        ]));
        let staged = f.status();
        assert_eq!(staged.files.iter().filter(|s| s.staged()).count(), 3);
        assert!(
            staged
                .files
                .iter()
                .find(|s| s.path == Path::new("a.md"))
                .unwrap()
                .untracked
        );
        fs::write(
            f.repo.join("[ab].md"),
            "unstaged work must survive unborn unstage",
        )
        .unwrap();
        f.execute(GitOperation::Unstage(vec!["[ab].md".into()]));
        assert!(f.repo.join("[ab].md").is_file());
        assert_eq!(
            fs::read_to_string(f.repo.join("[ab].md")).unwrap(),
            "unstaged work must survive unborn unstage"
        );
        assert!(
            f.status()
                .files
                .iter()
                .find(|s| s.path == Path::new("[ab].md"))
                .unwrap()
                .untracked
        );
        f.execute(GitOperation::Commit {
            message: "index only fixture".into(),
        });
        assert!(!f.status().unborn);
        assert!(
            String::from_utf8(fixture_git(&f.repo, &["ls-tree", "--name-only", "HEAD"]))
                .unwrap()
                .contains("-dash.md")
        );
        assert!(
            !String::from_utf8(fixture_git(&f.repo, &["ls-tree", "--name-only", "HEAD"]))
                .unwrap()
                .contains("a.md")
        );
        let diff = f
            .client
            .diff(&f.token, Path::new("[ab].md"), &cancel, 7)
            .unwrap();
        assert_eq!(diff.parts[0].outcome, ProcessOutcome::Exited(1));
        assert!(!diff.parts[0].stdout.is_empty());
        f.execute(GitOperation::CreateBranch("topic".into()));
        f.execute(GitOperation::SwitchBranch("topic".into()));
        assert_eq!(f.status().branch, "topic");
        assert!(
            f.client
                .execute(
                    &f.token,
                    &GitOperation::SwitchBranch("main".into()),
                    ExecutionGuard::default(),
                    &cancel,
                    7
                )
                .is_err()
        );
        let bare = f.root.join("remote.git");
        fixture_git(&f.root, &["init", "--bare", bare.to_str().unwrap()]);
        fixture_git(
            &f.repo,
            &["remote", "add", "fixture", bare.to_str().unwrap()],
        );
        f.execute(GitOperation::Push {
            remote: "fixture".into(),
            branch: "topic".into(),
        });
        f.execute(GitOperation::Fetch {
            remote: "fixture".into(),
        });
        f.execute(GitOperation::PullFastForward {
            remote: "fixture".into(),
            branch: "topic".into(),
        });
        assert_eq!(f.status().state, GitState::Ready);
        let identity = f.client.request_identity(7).unwrap();
        assert!(identity.is_current(7, &f.repo, &f.repo));
        assert!(!identity.is_current(8, &f.repo, &f.repo));
        f.client.trust.set_trusted(&f.repo, false).unwrap();
        assert!(matches!(
            f.client.refresh(&f.token, &cancel, 7),
            Err(GitError::Untrusted(_))
        ));
    }
    #[test]
    #[cfg(windows)]
    fn synthetic_conflict_merge_abort_and_same_literal_stage_contract() {
        let f = Fixture::new();
        let path = PathBuf::from("[conflict].md");
        let full = f.repo.join(&path);
        fs::write(&full, "original\n").unwrap();
        f.execute(GitOperation::Stage(vec![path.clone()]));
        f.execute(GitOperation::Commit {
            message: "base".into(),
        });
        f.execute(GitOperation::CreateBranch("topic".into()));
        f.execute(GitOperation::SwitchBranch("topic".into()));
        fs::write(&full, "incoming\n").unwrap();
        f.execute(GitOperation::Stage(vec![path.clone()]));
        f.execute(GitOperation::Commit {
            message: "topic".into(),
        });
        f.execute(GitOperation::SwitchBranch("main".into()));
        fs::write(&full, "current\n").unwrap();
        f.execute(GitOperation::Stage(vec![path.clone()]));
        f.execute(GitOperation::Commit {
            message: "main".into(),
        });
        fixture_git(&f.repo, &["config", "merge.conflictStyle", "diff3"]);
        let merge = || {
            f.client
                .execute(
                    &f.token,
                    &GitOperation::Merge("topic".into()),
                    ExecutionGuard {
                        confirmed: true,
                        presave_completed: true,
                    },
                    &Cancellation::default(),
                    7,
                )
                .unwrap()
        };
        assert_eq!(merge().outcome, ProcessOutcome::Exited(1));
        assert!(
            f.status()
                .files
                .iter()
                .any(|s| s.path == path && s.conflicted)
        );
        f.execute(GitOperation::AbortMerge);
        assert!(!f.status().files.iter().any(|s| s.conflicted));
        assert_eq!(fs::read_to_string(&full).unwrap(), "current\n");
        assert_eq!(merge().outcome, ProcessOutcome::Exited(1));
        let status = f.status();
        let file = status.files.iter().find(|s| s.path == path).unwrap();
        let source = fs::read_to_string(&full).unwrap();
        assert!(source.contains("|||||||"));
        let edit = conflict_edit(&source, file, 0, ConflictChoice::Both).unwrap();
        let mut resolved = source;
        resolved.replace_range(edit.range, &edit.replacement);
        fs::write(&full, resolved).unwrap();
        fs::write(f.repo.join("c.md"), "literal sibling must remain untracked").unwrap();
        f.execute(GitOperation::StageResolved(path));
        let status = f.status();
        assert!(!status.files.iter().any(|s| s.conflicted));
        assert!(
            status
                .files
                .iter()
                .find(|s| s.path == Path::new("c.md"))
                .unwrap()
                .untracked
        );
        f.execute(GitOperation::Commit {
            message: "resolved".into(),
        });
        fixture_git(&f.repo, &["checkout", "--detach"]);
        assert!(f.status().detached);
    }
    #[test]
    #[cfg(windows)]
    fn no_repository_no_git_and_untrusted_read_states() {
        let f = Fixture::new();
        let trust = f.client.trust.clone();
        trust.set_trusted(&f.root, true).unwrap();
        let token = trust.token(&f.root).unwrap();
        let mut client = GitClient {
            executable: "git.exe".into(),
            workspace: f.root.clone(),
            trust,
        };
        assert_eq!(
            client
                .refresh(&token, &Cancellation::default(), 1)
                .unwrap()
                .state,
            GitState::NoRepository
        );
        client.executable = f.root.join("missing-fixture-git.exe");
        assert_eq!(
            client
                .refresh(&token, &Cancellation::default(), 1)
                .unwrap()
                .state,
            GitState::NoGit
        );
        client.trust.set_trusted(&f.root, false).unwrap();
        assert!(matches!(
            client.refresh(&token, &Cancellation::default(), 1),
            Err(GitError::Untrusted(_))
        ));
    }

    #[test]
    #[cfg(windows)]
    fn explicit_init_parent_discovery_unicode_and_cancellation() {
        let f = Fixture::new();
        let workspace = f.root.join("空白 workspace 日本語");
        fs::create_dir(&workspace).unwrap();
        fs::write(workspace.join("日本語 note.md"), "本文\r\n").unwrap();
        let client = GitClient {
            executable: f.client.executable.clone(),
            workspace: workspace.clone(),
            trust: f.client.trust.clone(),
        };
        client.trust.set_trusted(&workspace, true).unwrap();
        let token = client.trust.token(&workspace).unwrap();
        assert_eq!(
            client
                .refresh(&token, &Cancellation::default(), 1)
                .unwrap()
                .state,
            GitState::NoRepository
        );
        assert!(!workspace.join(".git").exists());
        let cancelled = Cancellation::default();
        cancelled.cancel();
        assert!(client.discover_or_init(&token, &cancelled, 1).is_err());
        assert!(!workspace.join(".git").exists());
        let status = client
            .discover_or_init(&token, &Cancellation::default(), 1)
            .unwrap();
        assert_eq!(status.state, GitState::NoRemote);
        assert!(status.unborn);
        assert!(!status.branch.is_empty());
        assert_eq!(
            status.repository_root,
            fs::canonicalize(&workspace).unwrap()
        );
        assert!(
            status
                .files
                .iter()
                .any(|p| p.path == Path::new("日本語 note.md") && p.untracked)
        );
        assert!(status.files.iter().all(|p| !p.staged()));
        assert_eq!(
            fs::read(workspace.join("日本語 note.md")).unwrap(),
            "本文\r\n".as_bytes()
        );

        let child = workspace.join("child");
        fs::create_dir(&child).unwrap();
        client.trust.set_trusted(&child, true).unwrap();
        let child_client = GitClient {
            workspace: child.clone(),
            ..client
        };
        let child_token = child_client.trust.token(&child).unwrap();
        let inherited = child_client
            .discover_or_init(&child_token, &Cancellation::default(), 2)
            .unwrap();
        assert_eq!(inherited.repository_root, status.repository_root);
        assert!(!child.join(".git").exists());
        assert!(inherited.remotes.is_empty());

        fixture_git(&f.repo, &["commit", "--allow-empty", "-m", "fixture"]);
        let worktree = f.root.join("linked-worktree");
        fixture_git(
            &f.repo,
            &[
                "worktree",
                "add",
                "-b",
                "linked",
                worktree.to_str().unwrap(),
            ],
        );
        f.client.trust.set_trusted(&worktree, true).unwrap();
        let linked = GitClient {
            executable: f.client.executable.clone(),
            workspace: worktree.clone(),
            trust: f.client.trust.clone(),
        };
        let token = linked.trust.token(&worktree).unwrap();
        let status = linked
            .discover_or_init(&token, &Cancellation::default(), 3)
            .unwrap();
        assert_eq!(status.repository_root, fs::canonicalize(&worktree).unwrap());
        assert_eq!(status.branch, "linked");
        assert!(worktree.join(".git").is_file());
    }

    #[test]
    #[cfg(windows)]
    fn explicit_init_failure_retry_and_discovery_error_command_contract() {
        let f = Fixture::new();
        let workspace = f.root.join("no-repo");
        fs::create_dir(&workspace).unwrap();
        let source = f.root.join("git_mock.rs");
        let executable = f.root.join("git_mock.exe");
        fs::write(&source, r##"
use std::{fs, io::Write, path::Path};
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    writeln!(fs::OpenOptions::new().create(true).append(true).open("commands").unwrap(), "{}", args.join(" ")).unwrap();
    let mode = fs::read_to_string("mode").unwrap_or_default();
    if args.iter().any(|a| a == "rev-parse") {
        if mode == "discovery-error" { eprintln!("fatal: not a git repository: malformed.git"); std::process::exit(128); }
        if !Path::new("initialized").exists() { eprintln!("fatal: not a git repository (or any of the parent directories): .git"); std::process::exit(128); }
        println!("{}", std::env::current_dir().unwrap().display());
    } else if args.iter().any(|a| a == "init") {
        if mode == "init-error" { eprintln!("fatal: permission denied 日本語"); std::process::exit(128); }
        fs::write("initialized", "").unwrap();
    } else if args.iter().any(|a| a == "status") {
        std::io::stdout().write_all(b"# branch.oid (initial)\0# branch.head fixture-default\0").unwrap();
    } else { assert!(args.iter().any(|a| a == "remote")); }
}
"##).unwrap();
        let compiled = std::process::Command::new("rustc.exe")
            .arg(&source)
            .args(["--edition", "2024", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let client = GitClient {
            executable,
            workspace: workspace.clone(),
            trust: f.client.trust.clone(),
        };
        client.trust.set_trusted(&workspace, true).unwrap();
        let token = client.trust.token(&workspace).unwrap();
        fs::write(workspace.join("mode"), "discovery-error").unwrap();
        assert!(
            client
                .discover_or_init(&token, &Cancellation::default(), 1)
                .is_err()
        );
        let log = fs::read_to_string(workspace.join("commands")).unwrap();
        assert_eq!(log.lines().count(), 1);
        assert!(!log.lines().any(|line| line.ends_with(" init")));
        fs::write(workspace.join("commands"), "").unwrap();
        fs::write(workspace.join("mode"), "init-error").unwrap();
        assert!(
            client
                .discover_or_init(&token, &Cancellation::default(), 1)
                .is_err()
        );
        assert!(client.trust.token(&workspace).is_ok());
        let log = fs::read_to_string(workspace.join("commands")).unwrap();
        assert_eq!(log.lines().count(), 3);
        assert!(log.lines().last().unwrap().ends_with(" init"));
        assert!(!workspace.join("initialized").exists());
        fs::write(workspace.join("mode"), "").unwrap();
        fs::write(workspace.join("commands"), "").unwrap();
        let status = client
            .discover_or_init(&token, &Cancellation::default(), 1)
            .unwrap();
        assert_eq!(status.state, GitState::NoRemote);
        assert_eq!(status.branch, "fixture-default");
        let log = fs::read_to_string(workspace.join("commands")).unwrap();
        assert_eq!(log.lines().count(), 6);
        for line in log.lines() {
            assert!(
                line.ends_with(" init")
                    || line.ends_with("rev-parse --show-toplevel")
                    || line.ends_with("status --porcelain=v2 --branch -z --untracked-files=all")
                    || line.ends_with(" remote"),
                "{line}"
            );
        }
        fs::write(workspace.join("commands"), "").unwrap();
        client.trust.set_trusted(&workspace, false).unwrap();
        assert!(matches!(
            client.discover_or_init(&token, &Cancellation::default(), 1),
            Err(GitError::Untrusted(_))
        ));
        assert!(fs::read(workspace.join("commands")).unwrap().is_empty());
    }
}
