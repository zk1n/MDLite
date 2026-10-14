//! Query-scoped workspace walking. No persistent index and no external process.
use crate::{
    search::{self, Query, Results, SearchState},
    storage::FileSnapshot,
    workspace::Workspace,
};
use ignore::{
    gitignore::{Gitignore, GitignoreBuilder},
    overrides::{Override, OverrideBuilder},
};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
const MAX_FILE: u64 = 8 * 1024 * 1024;
const MAX_TOTAL: u64 = 512 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;
const MAX_RESULTS: usize = 8 * 1024 * 1024;
#[derive(Clone, Debug)]
pub struct Options {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub respect_ignore: bool,
    pub hidden: bool,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
            respect_ignore: true,
            hidden: false,
        }
    }
}
#[derive(Clone)]
pub struct OpenSource {
    pub path: PathBuf,
    pub document_id: u64,
    pub revision: u64,
    pub source: Arc<str>,
}
pub struct FileResults {
    pub path: PathBuf,
    pub matches: Results,
    pub snapshot: Option<FileSnapshot>,
}
pub struct WorkspaceResults {
    pub root: PathBuf,
    pub generation: u64,
    pub files: Vec<FileResults>,
    pub issues: Vec<ScanIssue>,
    pub complete: bool,
    pub scanned_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct ScanIssue {
    pub relative: PathBuf,
    pub message: String,
}
struct Walk {
    files: Vec<PathBuf>,
    issues: Vec<ScanIssue>,
    complete: bool,
}
fn issue(issues: &mut Vec<ScanIssue>, root: &Path, path: &Path, message: impl ToString) {
    // Cap retained diagnostics; incomplete is tracked independently.
    if issues.len() < 1000 {
        issues.push(ScanIssue {
            relative: path.strip_prefix(root).unwrap_or(Path::new(".")).to_owned(),
            message: message.to_string(),
        });
    }
}
fn overrides(root: &Path, patterns: &[String]) -> Result<Override, String> {
    let mut builder = OverrideBuilder::new(root);
    builder
        .case_insensitive(cfg!(windows))
        .map_err(|e| e.to_string())?;
    for pattern in patterns {
        if pattern.trim().is_empty() {
            continue;
        }
        if pattern.starts_with('!') {
            return Err("include/exclude patterns must not begin with !".into());
        }
        builder.add(pattern).map_err(|e| e.to_string())?;
    }
    builder.build().map_err(|e| e.to_string())
}
fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    metadata.file_type().is_symlink()
}
fn hidden(path: &Path, metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 2 != 0 {
            return true;
        }
    }
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
}
fn protected(path: &Path) -> bool {
    path.file_name().is_some_and(|n| {
        matches!(
            n.to_string_lossy().to_ascii_lowercase().as_str(),
            ".git"
                | ".mdlite"
                | ".svn"
                | ".hg"
                | ".codex"
                | ".codex-runtime"
                | "node_modules"
                | "target"
        )
    })
}
fn walk(root: &Path, options: &Options, cancel: &AtomicBool) -> Result<Walk, String> {
    let include = overrides(root, &options.include)?;
    let exclude = overrides(root, &options.exclude)?;
    let has_include = options.include.iter().any(|p| !p.trim().is_empty());
    let mut pending = vec![(root.to_owned(), Vec::<Arc<Gitignore>>::new())];
    let mut result = Walk {
        files: Vec::new(),
        issues: Vec::new(),
        complete: true,
    };
    let mut entries = 0;
    let mut ignore_bytes = 0usize;
    let mut ignore_rules = 0usize;
    while let Some((directory, mut rules)) = pending.pop() {
        if cancel.load(Ordering::Relaxed) {
            result.complete = false;
            issue(&mut result.issues, root, &directory, "cancelled");
            break;
        }
        if options.respect_ignore {
            let ignore_path = directory.join(".gitignore");
            match fs::symlink_metadata(&ignore_path) {
                Ok(metadata) => {
                    let load = || -> Result<(Gitignore, usize, usize), String> {
                        if reparse(&metadata) || !metadata.is_file() {
                            return Err("ignore file is not a safe regular file".into());
                        }
                        let mut bytes = Vec::new();
                        fs::File::open(&ignore_path)
                            .map_err(|e| e.to_string())?
                            .take(64 * 1024 + 1)
                            .read_to_end(&mut bytes)
                            .map_err(|e| e.to_string())?;
                        if bytes.len() > 64 * 1024 || ignore_bytes + bytes.len() > 4 * 1024 * 1024 {
                            return Err("ignore byte limit reached".into());
                        }
                        let text = std::str::from_utf8(&bytes)
                            .map_err(|_| "ignore file is not UTF-8")?
                            .trim_start_matches('\u{feff}');
                        let count = text
                            .lines()
                            .filter(|line| !line.trim_end().is_empty() && !line.starts_with('#'))
                            .count();
                        if ignore_rules + count > 10_000 {
                            return Err("ignore rule limit reached".into());
                        }
                        let mut builder = GitignoreBuilder::new(&directory);
                        builder
                            .case_insensitive(cfg!(windows))
                            .map_err(|e| e.to_string())?;
                        for line in text.lines() {
                            builder
                                .add_line(Some(ignore_path.clone()), line)
                                .map_err(|e| e.to_string())?;
                        }
                        Ok((
                            builder.build().map_err(|e| e.to_string())?,
                            bytes.len(),
                            count,
                        ))
                    };
                    match load() {
                        Ok((rule, bytes, count)) => {
                            rules.push(Arc::new(rule));
                            ignore_bytes += bytes;
                            ignore_rules += count;
                        }
                        Err(e) => {
                            result.complete = false;
                            issue(&mut result.issues, root, &ignore_path, e);
                            continue;
                        }
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => {
                    result.complete = false;
                    issue(&mut result.issues, root, &ignore_path, e);
                    continue;
                }
            }
        }
        let iterator = match fs::read_dir(&directory) {
            Ok(i) => i,
            Err(e) => {
                result.complete = false;
                issue(&mut result.issues, root, &directory, e);
                continue;
            }
        };
        for entry in iterator {
            entries += 1;
            if entries > MAX_ENTRIES {
                result.complete = false;
                issue(&mut result.issues, root, &directory, "entry limit reached");
                return Ok(result);
            }
            if cancel.load(Ordering::Relaxed) {
                result.complete = false;
                issue(&mut result.issues, root, &directory, "cancelled");
                return Ok(result);
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    result.complete = false;
                    issue(&mut result.issues, root, &directory, e);
                    continue;
                }
            };
            let path = entry.path();
            if protected(&path) {
                continue;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => {
                    result.complete = false;
                    issue(&mut result.issues, root, &path, e);
                    continue;
                }
            };
            if reparse(&metadata) || (!options.hidden && hidden(&path, &metadata)) {
                continue;
            }
            let is_dir = metadata.is_dir();
            let mut ignored = false;
            for rule in rules.iter().rev() {
                let matched = rule.matched(&path, is_dir);
                if matched.is_ignore() {
                    ignored = true;
                    break;
                }
                if matched.is_whitelist() {
                    break;
                }
            }
            if ignored || exclude.matched(&path, is_dir).is_whitelist() {
                continue;
            }
            if is_dir {
                pending.push((path, rules.clone()));
            } else if metadata.is_file()
                && (!has_include || include.matched(&path, false).is_whitelist())
            {
                result.files.push(path);
            }
        }
    }
    result.files.sort();
    Ok(result)
}
pub fn search_workspace(
    workspace: &Workspace,
    query: Query,
    options: &Options,
    open: &[OpenSource],
    generation: u64,
    cancel: &AtomicBool,
) -> Result<WorkspaceResults, String> {
    let root = workspace.root();
    let walked = walk(root, options, cancel)?;
    let mut result = WorkspaceResults {
        root: root.to_owned(),
        generation,
        files: Vec::new(),
        issues: walked.issues,
        complete: walked.complete,
        scanned_bytes: 0,
    };
    let open_paths: HashMap<PathBuf, &OpenSource> = open
        .iter()
        .filter_map(|source| {
            fs::canonicalize(&source.path)
                .ok()
                .map(|path| (path, source))
        })
        .collect();
    let open_identities: HashMap<same_file::Handle, &OpenSource> = open
        .iter()
        .filter_map(|source| {
            same_file::Handle::from_path(&source.path)
                .ok()
                .map(|identity| (identity, source))
        })
        .collect();
    let mut retained = 0usize;
    let mut hits = 0usize;
    let mut matched_identities = HashSet::new();
    for path in walked.files {
        if cancel.load(Ordering::Relaxed) {
            result.complete = false;
            issue(&mut result.issues, root, &path, "cancelled");
            break;
        }
        let physical = same_file::Handle::from_path(&path).ok();
        let by_identity = physical
            .as_ref()
            .and_then(|identity| open_identities.get(identity).copied());
        if physical
            .as_ref()
            .is_some_and(|identity| matched_identities.contains(identity))
        {
            continue;
        }
        let (id, revision, source, snapshot) =
            if let Some(open) = open_paths.get(&path).copied().or(by_identity) {
                (
                    open.document_id,
                    open.revision,
                    Arc::clone(&open.source),
                    None,
                )
            } else {
                match FileSnapshot::open_bounded(&path, MAX_FILE) {
                    Ok(loaded) => {
                        if loaded.source.contains('\0') {
                            continue;
                        }
                        (0, 0, Arc::from(loaded.source), Some(loaded.snapshot))
                    }
                    Err(e) => {
                        result.complete = false;
                        issue(&mut result.issues, root, &path, e);
                        continue;
                    }
                }
            };
        let bytes = snapshot
            .as_ref()
            .map_or(source.len(), FileSnapshot::baseline_byte_len) as u64;
        if bytes > MAX_FILE {
            result.complete = false;
            issue(&mut result.issues, root, &path, "file byte limit reached");
            continue;
        }
        if result.scanned_bytes.saturating_add(bytes) > MAX_TOTAL {
            result.complete = false;
            issue(&mut result.issues, root, &path, "scan byte limit reached");
            break;
        }
        result.scanned_bytes += bytes;
        let matches = search::search_snapshot(
            id,
            revision,
            source,
            query.clone(),
            generation,
            cancel,
            Duration::from_millis(200),
        )
        .map_err(|e| e.to_string())?;
        if matches.state != SearchState::Complete {
            result.complete = false;
            issue(
                &mut result.issues,
                root,
                &path,
                format!("{:?}", matches.state),
            );
        }
        if matches.hits.is_empty() {
            continue;
        }
        // Source snapshots retained for replacement are charged to the result
        // budget too; no hidden half-gigabyte collection of matched bodies.
        let retained_next = retained
            + matches.source.len()
            + snapshot.as_ref().map_or(0, FileSnapshot::baseline_byte_len)
            + matches
                .hits
                .iter()
                .map(|hit| {
                    std::mem::size_of::<search::Hit>()
                        + hit.captures.len() * std::mem::size_of::<Option<std::ops::Range<usize>>>()
                })
                .sum::<usize>();
        if hits + matches.hits.len() > 20_000 || retained_next > MAX_RESULTS {
            result.complete = false;
            issue(
                &mut result.issues,
                root,
                &path,
                "result byte or hit limit reached",
            );
            break;
        }
        retained = retained_next;
        hits += matches.hits.len();
        if let Some(physical) = physical {
            matched_identities.insert(physical);
        }
        result.files.push(FileResults {
            path,
            matches,
            snapshot,
        });
    }
    Ok(result)
}
pub struct QuickOpen {
    pub paths: Vec<PathBuf>,
    pub complete: bool,
    pub more: bool,
    pub issues: Vec<ScanIssue>,
}
pub fn quick_open(
    workspace: &Workspace,
    query: &str,
    recent: &[PathBuf],
    cancel: &AtomicBool,
) -> Result<QuickOpen, String> {
    let walked = walk(workspace.root(), &Options::default(), cancel)?;
    let needle = query.to_lowercase().replace('\\', "/");
    let mut paths: Vec<_> = walked
        .files
        .into_iter()
        .filter(|p| {
            p.strip_prefix(workspace.root()).is_ok_and(|r| {
                r.to_string_lossy()
                    .replace('\\', "/")
                    .to_lowercase()
                    .contains(&needle)
            })
        })
        .collect();
    let recent: HashMap<_, _> = recent.iter().enumerate().map(|(i, p)| (p, i)).collect();
    paths.sort_by_key(|p| (recent.get(p).copied().unwrap_or(usize::MAX), p.clone()));
    let more = paths.len() > 200;
    paths.truncate(200);
    Ok(QuickOpen {
        paths,
        complete: walked.complete,
        more,
        issues: walked.issues,
    })
}
/// Guard used by both journal preview and its final application.
pub fn validate_replace_plan(
    result: &WorkspaceResults,
    generation: u64,
    workspace: &Workspace,
) -> Result<(), String> {
    if !result.complete || result.generation != generation || result.root != workspace.root() {
        return Err("incomplete or stale replacement preview".into());
    }
    let mut seen = HashSet::new();
    for file in &result.files {
        let canonical = fs::canonicalize(&file.path).map_err(|e| e.to_string())?;
        if !canonical.starts_with(workspace.root()) || !seen.insert(canonical) {
            return Err("replacement path escaped workspace or repeated".into());
        }
        if let Some(snapshot) = &file.snapshot {
            let current =
                FileSnapshot::open_bounded(&file.path, MAX_FILE).map_err(|e| e.to_string())?;
            if current.snapshot.path() != snapshot.path()
                || current.snapshot.baseline_sha256() != snapshot.baseline_sha256()
            {
                return Err("closed file changed since preview".into());
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_alias_uses_one_dirty_source_and_one_replacement_target() {
        let f = Fixture::new();
        let file = f.0.join("a.md");
        let alias = f.0.join("b.md");
        fs::write(&file, "disk").unwrap();
        fs::hard_link(&file, &alias).unwrap();
        let open = OpenSource {
            path: file,
            document_id: 17,
            revision: 9,
            source: Arc::from("dirty hit"),
        };
        let result = search_workspace(
            &f.workspace(),
            Query {
                text: "hit".into(),
                ..Default::default()
            },
            &Options::default(),
            &[open],
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].matches.document_id, 17);
        assert!(result.files[0].snapshot.is_none());
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mdlite-scan-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn workspace(&self) -> Workspace {
            Workspace::open(&self.0).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn ignore_globs_dirty_sources_binary_and_late_quick_open_target() {
        let f = Fixture::new();
        fs::create_dir(f.0.join("sub")).unwrap();
        fs::write(f.0.join(".gitignore"), "ignored.md\nsub/drop*\n").unwrap();
        fs::write(f.0.join("sub/.gitignore"), "!drop-keep.md\n").unwrap();
        for name in [
            "a.md",
            "ignored.md",
            "sub/drop-no.md",
            "sub/drop-keep.md",
            "binary.md",
        ] {
            fs::write(
                f.0.join(name),
                if name == "binary.md" {
                    "hit\0binary"
                } else {
                    "disk hit"
                },
            )
            .unwrap();
        }
        for i in 0..250 {
            fs::write(f.0.join(format!("note-{i:03}.md")), "empty").unwrap();
        }
        fs::write(f.0.join("zzz-unique.md"), "empty").unwrap();
        let open = OpenSource {
            path: f.0.join("a.md"),
            document_id: 9,
            revision: 7,
            source: Arc::from("dirty hit hit"),
        };
        let result = search_workspace(
            &f.workspace(),
            Query {
                text: "hit".into(),
                ..Default::default()
            },
            &Options {
                include: vec!["**/*.md".into()],
                ..Default::default()
            },
            &[open],
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(result.complete);
        assert_eq!(result.files.len(), 2);
        let dirty = result
            .files
            .iter()
            .find(|x| x.matches.document_id == 9)
            .unwrap();
        assert_eq!(dirty.matches.hits.len(), 2);
        assert!(dirty.snapshot.is_none());
        let quick = quick_open(&f.workspace(), "zzz-unique", &[], &AtomicBool::new(false)).unwrap();
        assert_eq!(quick.paths.len(), 1);
        assert!(quick.complete);
        assert!(!quick.more);
        fs::write(f.0.join("sub/drop-keep.md"), "peer").unwrap();
        assert!(validate_replace_plan(&result, 1, &f.workspace()).is_err());
    }
    #[test]
    fn invalid_ignore_and_cancel_are_incomplete_not_empty_success() {
        let f = Fixture::new();
        fs::write(f.0.join(".gitignore"), [0xff]).unwrap();
        fs::write(f.0.join("a.md"), "hit").unwrap();
        let result = search_workspace(
            &f.workspace(),
            Query {
                text: "hit".into(),
                ..Default::default()
            },
            &Options::default(),
            &[],
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!result.complete);
        assert!(!result.issues.is_empty());
        let quick = quick_open(&f.workspace(), "a", &[], &AtomicBool::new(true)).unwrap();
        assert!(!quick.complete);
    }
}
