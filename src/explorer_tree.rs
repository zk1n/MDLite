//! A caller-expanded projection of the workspace Explorer tree.
//!
//! This module owns no expansion or selection state. It reads only the root
//! directory and folders whose absolute workspace paths are in `expanded`.

use crate::workspace::{
    ExplorerEntry, ExplorerEntryKind, ExplorerIssue, ExplorerIssueKind, Workspace, WorkspaceError,
};
use std::{collections::BTreeSet, path::Path};

pub const DEFAULT_TREE_LIMIT: usize = 20_000;
const MAX_TREE_DEPTH: usize = 64;

#[derive(Clone, Debug)]
pub struct TreeRow {
    pub entry: ExplorerEntry,
    /// Direct children of `root` have depth zero.
    pub depth: usize,
    /// True when this folder is in the caller-owned expansion set.
    pub expanded: bool,
}

#[derive(Clone, Debug)]
pub struct TreeListing {
    pub rows: Vec<TreeRow>,
    pub issues: Vec<ExplorerIssue>,
    /// False when a directory had issues or a depth/row bound stopped traversal.
    pub complete: bool,
}

/// List visible rows below `root`, reading only folders in `expanded`.
///
/// Expansion keys are the absolute `ExplorerEntry::path` values returned by
/// this function. The root itself is implicit and is always listed once.
pub fn list_expanded(
    workspace: &Workspace,
    root: &Path,
    expanded: &BTreeSet<std::path::PathBuf>,
    limit: usize,
) -> Result<TreeListing, WorkspaceError> {
    let listing = workspace.list_directory(root)?;
    let complete = listing.is_complete();
    let entries = listing.entries;
    let issues = listing.issues;
    let mut result = TreeListing {
        rows: Vec::new(),
        issues,
        complete,
    };
    append_rows(workspace, entries, 0, expanded, limit, &mut result);
    Ok(result)
}

fn append_rows(
    workspace: &Workspace,
    entries: Vec<ExplorerEntry>,
    depth: usize,
    expanded: &BTreeSet<std::path::PathBuf>,
    limit: usize,
    result: &mut TreeListing,
) {
    for entry in entries {
        if result.rows.len() >= limit {
            result.complete = false;
            return;
        }

        let is_folder = entry.kind == ExplorerEntryKind::Folder;
        let is_expanded = is_folder && expanded.contains(&entry.path);
        let path = entry.path.clone();
        result.rows.push(TreeRow {
            entry,
            depth,
            expanded: is_expanded,
        });

        if !is_expanded {
            continue;
        }
        if result.rows.len() >= limit {
            result.complete = false;
            continue;
        }
        if depth >= MAX_TREE_DEPTH {
            result.complete = false;
            continue;
        }

        match workspace.list_directory(&path) {
            Ok(listing) => {
                result.complete &= listing.is_complete();
                result.issues.extend(listing.issues);
                append_rows(
                    workspace,
                    listing.entries,
                    depth + 1,
                    expanded,
                    limit,
                    result,
                );
            }
            Err(error) => {
                result.complete = false;
                result.issues.push(issue_for_expansion(&path, &error));
            }
        }
    }
}

fn issue_for_expansion(path: &Path, error: &WorkspaceError) -> ExplorerIssue {
    let kind = match error {
        WorkspaceError::ReparsePoint(_) => ExplorerIssueKind::ReparsePoint,
        WorkspaceError::InvalidPath(_)
        | WorkspaceError::OutsideWorkspace(_)
        | WorkspaceError::ProtectedPath(_)
        | WorkspaceError::UnsupportedEntry(_) => ExplorerIssueKind::Unsupported,
        _ => ExplorerIssueKind::Metadata,
    };
    ExplorerIssue {
        path: path.to_path_buf(),
        kind,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            loop {
                let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("mdlite-explorer-tree-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("failed to create tree fixture: {error}"),
                }
            }
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
    fn expanded_rows_are_lazy_stable_and_leave_selection_to_the_caller() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join("alpha/deep")).unwrap();
        fs::create_dir(fixture.0.join("beta")).unwrap();
        fs::write(fixture.0.join("alpha/a.md"), "a").unwrap();
        fs::write(fixture.0.join("alpha/deep/b.md"), "b").unwrap();
        fs::write(fixture.0.join("root.md"), "root").unwrap();
        let workspace = fixture.workspace();
        let root = workspace.list_directory(".").unwrap();
        let alpha = root
            .entries
            .iter()
            .find(|entry| entry.name == "alpha")
            .unwrap();
        let deep = workspace.list_directory(&alpha.path).unwrap();
        let deep_path = deep
            .entries
            .iter()
            .find(|entry| entry.name == "deep")
            .unwrap()
            .path
            .clone();
        let selected_path = deep
            .entries
            .iter()
            .find(|entry| entry.name == "a.md")
            .unwrap()
            .path
            .clone();
        let selected_by_caller = BTreeSet::from([selected_path.clone()]);
        let root_order = |tree: &TreeListing| {
            tree.rows
                .iter()
                .filter(|row| row.depth == 0)
                .map(|row| row.entry.name.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };

        let mut expanded = BTreeSet::from([alpha.path.clone()]);
        let first = list_expanded(&workspace, Path::new("."), &expanded, 100).unwrap();
        assert!(first.complete);
        assert_eq!(first.rows[0].entry.name, "alpha");
        assert_eq!(first.rows[0].depth, 0);
        assert!(first.rows[0].expanded);
        assert!(
            first
                .rows
                .iter()
                .any(|row| row.entry.name == "deep" && row.depth == 1 && !row.expanded)
        );
        assert!(!first.rows.iter().any(|row| row.entry.name == "b.md"));
        assert_eq!(root_order(&first), ["alpha", "beta", "root.md"]);

        expanded.insert(deep_path);
        let second = list_expanded(&workspace, Path::new("."), &expanded, 100).unwrap();
        assert!(second.complete);
        assert!(
            second
                .rows
                .iter()
                .any(|row| row.entry.name == "b.md" && row.depth == 2)
        );
        assert!(
            second
                .rows
                .iter()
                .any(|row| selected_by_caller.contains(&row.entry.path))
        );
        assert_eq!(first.rows.last().unwrap().entry.name, "root.md");
        assert_eq!(second.rows.last().unwrap().entry.name, "root.md");
        assert_eq!(root_order(&second), root_order(&first));

        expanded.clear();
        let collapsed = list_expanded(&workspace, Path::new("."), &expanded, 100).unwrap();
        assert!(collapsed.complete);
        assert!(!collapsed.rows.iter().any(|row| row.entry.name == "deep"
            || row.entry.name == "a.md"
            || row.entry.name == "b.md"));
        assert!(selected_by_caller.contains(&selected_path));
    }

    #[test]
    fn protected_paths_invalid_roots_and_row_bounds_are_reported() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.0.join(".mdlite/.state")).unwrap();
        fs::write(fixture.0.join(".mdlite/.state/private.md"), "private").unwrap();
        fs::write(fixture.0.join("a.md"), "a").unwrap();
        fs::write(fixture.0.join("b.md"), "b").unwrap();
        let workspace = fixture.workspace();

        assert!(matches!(
            list_expanded(&workspace, Path::new("missing"), &BTreeSet::new(), 20),
            Err(WorkspaceError::Io(_))
        ));
        let limited = list_expanded(&workspace, Path::new("."), &BTreeSet::new(), 1).unwrap();
        assert_eq!(limited.rows.len(), 1);
        assert!(!limited.complete);
        assert!(
            !limited
                .rows
                .iter()
                .any(|row| row.entry.name == ".state" || row.entry.name == "private.md")
        );
    }

    #[cfg(windows)]
    #[test]
    fn reparse_directories_are_not_followed() {
        use std::os::windows::ffi::OsStrExt;

        let fixture = Fixture::new();
        let outside = Fixture::new();
        fs::write(outside.0.join("secret.md"), "outside").unwrap();
        let link = fixture.0.join("escape");
        let link_wide: Vec<u16> = link.as_os_str().encode_wide().chain(Some(0)).collect();
        let target_wide: Vec<u16> = outside.0.as_os_str().encode_wide().chain(Some(0)).collect();
        let created = unsafe {
            windows_sys::Win32::Storage::FileSystem::CreateSymbolicLinkW(
                link_wide.as_ptr(),
                target_wide.as_ptr(),
                windows_sys::Win32::Storage::FileSystem::SYMBOLIC_LINK_FLAG_DIRECTORY
                    | windows_sys::Win32::Storage::FileSystem::SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
            )
        };
        if !created {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(87 | 1314)) {
                eprintln!("skipping reparse fixture: symlink privilege is unavailable");
                return;
            }
            panic!("failed to create reparse fixture: {error}");
        }
        let workspace = fixture.workspace();

        let tree = list_expanded(&workspace, Path::new("."), &BTreeSet::new(), 100).unwrap();
        assert!(!tree.complete);
        assert!(tree.rows.iter().all(|row| row.entry.name != "escape"));
        assert!(
            tree.issues
                .iter()
                .any(|issue| issue.kind == ExplorerIssueKind::ReparsePoint)
        );
        assert!(tree.rows.iter().all(|row| row.entry.name != "secret.md"));
    }

    #[test]
    fn depth_bound_returns_an_incomplete_projection() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let mut expanded = BTreeSet::new();
        let mut current = workspace.root().to_path_buf();
        for depth in 0..=MAX_TREE_DEPTH + 1 {
            current.push(format!("d{depth}"));
            fs::create_dir(&current).unwrap();
            expanded.insert(current.clone());
        }

        let tree =
            list_expanded(&workspace, Path::new("."), &expanded, MAX_TREE_DEPTH + 10).unwrap();
        assert!(!tree.complete);
        assert_eq!(tree.rows.len(), MAX_TREE_DEPTH + 1);
        assert_eq!(tree.rows.last().unwrap().depth, MAX_TREE_DEPTH);
    }
}
