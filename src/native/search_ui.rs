//! Native workspace and document search controls.
//!
//! Search results are immutable snapshots. Workspace writes go through the
//! bounded replacement journal core; open documents are committed by the UI
//! thread as one guarded edit each.

use super::*;
use mdlite::{
    coordinates::BoundaryAffinity,
    document::{Document, Selection},
    panel::{PanelId, Rect},
    replace::{self, ApplyOutcome, JournalInfo, RecoveryOutcome, RecoveryPreview, ReplacePlan},
    search::{self, Query, Results, SearchState},
    storage::FileSnapshot,
    workspace::Workspace,
    workspace_search::{self, OpenSource, Options, WorkspaceResults},
};
use std::{
    cell::Cell,
    fs,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};
use windows::Win32::UI::Controls::*;

pub(super) const COMMANDS: &[(usize, &str, Option<&str>)] = &[
    (5000, "文書内を検索", Some("edit.find")),
    (5001, "Workspaceを検索", Some("edit.findWorkspace")),
    (5002, "次の検索結果", Some("edit.findNext")),
    (5003, "前の検索結果", Some("edit.findPrevious")),
    (5004, "現在の一致を置換", None),
    (5005, "文書内をすべて置換", None),
    (5006, "Workspaceを置換", None),
    (5007, "置換journalを開く", None),
    (5008, "選択したjournalを復旧", None),
    (5009, "検索を閉じる", None),
    (5010, "検索を中止", None),
];

const CMD_DOCUMENT_SEARCH: usize = 5000;
const CMD_WORKSPACE_SEARCH: usize = 5001;
const CMD_NEXT: usize = 5002;
const CMD_PREVIOUS: usize = 5003;
const CMD_REPLACE_ONE: usize = 5004;
const CMD_REPLACE_DOCUMENT_ALL: usize = 5005;
const CMD_REPLACE_WORKSPACE: usize = 5006;
const CMD_OPEN_JOURNALS: usize = 5007;
const CMD_RESTORE_JOURNAL: usize = 5008;
const CMD_CLOSE: usize = 5009;
const CMD_CANCEL_OPERATION: usize = 5010;

const CONTROL_SCOPE_DOCUMENT: usize = 5600;
const CONTROL_SCOPE_WORKSPACE: usize = 5601;
const CONTROL_QUERY: usize = 5602;
const CONTROL_REPLACEMENT: usize = 5603;
const CONTROL_CASE: usize = 5604;
const CONTROL_REGEX: usize = 5605;
const CONTROL_WORD: usize = 5606;
const CONTROL_DETAILS: usize = 5607;
const CONTROL_INCLUDE: usize = 5608;
const CONTROL_EXCLUDE: usize = 5609;
const CONTROL_IGNORE: usize = 5610;
const CONTROL_HIDDEN: usize = 5611;
const CONTROL_RESULTS: usize = 5612;
const CONTROL_STATUS: usize = 5613;

// EDIT limits are UTF-16 code units. This leaves enough room for every literal
// query within the core's 8 MiB UTF-8 cap; regex validation stays in build_query.
const QUERY_CONTROL_UNIT_LIMIT: usize = search::MAX_LITERAL_QUERY_BYTES;
const GLOB_LIMIT: usize = 64 * 1024;
const REPLACEMENT_LIMIT: usize = 64 * 1024 * 1024;
const DEBOUNCE: Duration = Duration::from_millis(160);
const FILE_READ_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    Document,
    Workspace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SearchKey {
    scope: Scope,
    query: Query,
    include: Vec<String>,
    exclude: Vec<String>,
    respect_ignore: bool,
    hidden: bool,
    workspace: Option<PathBuf>,
    active_document: Option<(u64, u64)>,
    open_documents: Vec<(PathBuf, u64, u64)>,
}

enum SearchBody {
    Document {
        document_id: u64,
        revision: u64,
        source: Arc<str>,
        query: Query,
    },
    Workspace {
        root: PathBuf,
        query: Query,
        options: Options,
        open: Vec<OpenSource>,
    },
}

struct SearchRequest {
    generation: u64,
    body: SearchBody,
}

struct PendingSearch {
    request: SearchRequest,
    not_before: Instant,
}

struct ActiveSearch {
    generation: u64,
    cancel: Arc<AtomicBool>,
}

enum SearchResults {
    Document(Results),
    Workspace(WorkspaceResults),
}

struct SearchCompletion {
    generation: u64,
    result: Result<SearchResults, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HitLocation {
    file: usize,
    hit: usize,
}

#[derive(Clone, Copy)]
enum ResultRow {
    Group(usize),
    Hit(HitLocation),
    Journal(usize),
}

struct FocusSnapshot {
    document_id: u64,
    revision: u64,
    selection: Selection,
    affinity: BoundaryAffinity,
    hwnd: HWND,
}

#[derive(Default)]
struct SearchControls {
    owner: HWND,
    scope_document: HWND,
    scope_workspace: HWND,
    query_label: HWND,
    query: HWND,
    replacement_label: HWND,
    replacement: HWND,
    case_sensitive: HWND,
    regex: HWND,
    word: HWND,
    details: HWND,
    include_label: HWND,
    include: HWND,
    exclude_label: HWND,
    exclude: HWND,
    respect_ignore: HWND,
    hidden: HWND,
    next: HWND,
    previous: HWND,
    replace_one: HWND,
    replace_document_all: HWND,
    replace_workspace: HWND,
    journals: HWND,
    restore: HWND,
    cancel: HWND,
    close: HWND,
    results: HWND,
    status: HWND,
}

enum OperationCompletion {
    WorkspacePreview {
        results: WorkspaceResults,
        plan: Result<ReplacePlan, String>,
    },
    Replace(Result<ApplyOutcome, String>),
    RecoveryPreview(Result<RecoveryPreview, String>),
    Restore(Result<RecoveryOutcome, String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationPhase {
    WorkspacePreview,
    WorkspaceApply,
    RecoveryPreview,
    RecoveryRestore,
}

struct Operation {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<OperationCompletion>,
    phase: OperationPhase,
    open_edits: Vec<OpenEdit>,
    replacement_count: usize,
    open_document_count: usize,
    journal_path: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
}

struct OpenEdit {
    path: PathBuf,
    document_id: u64,
    revision: u64,
    generation: u64,
    expected_source: Arc<str>,
    after_source: String,
    selection_after: Selection,
    replacement_count: usize,
}

pub(super) struct SearchPane {
    controls: SearchControls,
    query_composing: Option<Box<Cell<bool>>>,
    replacement_composing: Option<Box<Cell<bool>>>,
    include_composing: Option<Box<Cell<bool>>>,
    exclude_composing: Option<Box<Cell<bool>>>,
    initialized: bool,
    scope: Scope,
    details_open: bool,
    journal_mode: bool,
    journals: Vec<JournalInfo>,
    selected_journal: Option<usize>,
    result_rows: Vec<ResultRow>,
    results: Option<SearchResults>,
    current: Option<HitLocation>,
    focus_restore: Option<FocusSnapshot>,
    generation: u64,
    key: Option<SearchKey>,
    pending: Option<PendingSearch>,
    active: Option<ActiveSearch>,
    sender: Sender<SearchCompletion>,
    receiver: Receiver<SearchCompletion>,
    operation: Option<Operation>,
    status: String,
    inputs_dirty: bool,
}

impl Default for SearchPane {
    fn default() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            controls: SearchControls::default(),
            query_composing: None,
            replacement_composing: None,
            include_composing: None,
            exclude_composing: None,
            initialized: false,
            scope: Scope::Document,
            details_open: false,
            journal_mode: false,
            journals: Vec::new(),
            selected_journal: None,
            result_rows: Vec::new(),
            results: None,
            current: None,
            focus_restore: None,
            generation: 0,
            key: None,
            pending: None,
            active: None,
            sender,
            receiver,
            operation: None,
            status: String::new(),
            inputs_dirty: true,
        }
    }
}

impl Drop for SearchPane {
    fn drop(&mut self) {
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        if let Some(operation) = &self.operation {
            operation.cancel.store(true, Ordering::Relaxed);
        }
        unsafe {
            for hwnd in self.controls.all() {
                if hwnd != HWND::default() && IsWindow(Some(hwnd)).as_bool() {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }
}

impl SearchControls {
    fn all(&self) -> Vec<HWND> {
        [
            self.scope_document,
            self.scope_workspace,
            self.query_label,
            self.query,
            self.replacement_label,
            self.replacement,
            self.case_sensitive,
            self.regex,
            self.word,
            self.details,
            self.include_label,
            self.include,
            self.exclude_label,
            self.exclude,
            self.respect_ignore,
            self.hidden,
            self.next,
            self.previous,
            self.replace_one,
            self.replace_document_all,
            self.replace_workspace,
            self.journals,
            self.restore,
            self.cancel,
            self.close,
            self.results,
            self.status,
        ]
        .into_iter()
        .filter(|hwnd| *hwnd != HWND::default())
        .collect()
    }
}

impl SearchPane {
    fn diagnostics_summary(&self) -> String {
        let mut files = 0usize;
        let mut matches = 0usize;
        let state = if self.active.is_some() || self.pending.is_some() {
            "searching"
        } else if self.operation.is_some() {
            "operation"
        } else {
            match self.results.as_ref() {
                Some(SearchResults::Document(result)) => {
                    files = 1;
                    matches = result.hits.len();
                    if result.generation != self.generation {
                        "stale"
                    } else {
                        match &result.state {
                            SearchState::Complete => "complete",
                            SearchState::Cancelled => "cancelled",
                            SearchState::TimedOut => "timed out",
                            SearchState::ResourceLimit => "resource limit",
                            SearchState::HitLimit => "hit limit",
                        }
                    }
                }
                Some(SearchResults::Workspace(result)) => {
                    files = result.files.len();
                    matches = result.files.iter().fold(0usize, |total, file| {
                        total.saturating_add(file.matches.hits.len())
                    });
                    if result.generation != self.generation {
                        "stale"
                    } else if result.complete {
                        "complete"
                    } else {
                        "incomplete"
                    }
                }
                None => "idle",
            }
        };

        format!(
            "Search: state={state}; generation={}; files={files}; matches={matches}",
            self.generation
        )
    }

    #[cfg(test)]
    pub(super) fn pending_mutation_fixture(restoring: bool) -> Self {
        let mut pane = Self::default();
        let (_, receiver) = mpsc::channel();
        pane.operation = Some(Operation {
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
            phase: if restoring {
                OperationPhase::RecoveryRestore
            } else {
                OperationPhase::WorkspaceApply
            },
            open_edits: Vec::new(),
            replacement_count: 0,
            open_document_count: 0,
            journal_path: None,
            workspace_root: None,
        });
        pane
    }
    #[cfg(test)]
    pub(super) fn seed_diagnostics_privacy_test_state(&mut self, query: &str, path: &str) {
        self.generation = 42;
        self.status = format!("{query} {path}");
        self.key = Some(SearchKey {
            scope: Scope::Workspace,
            query: search::Query {
                text: query.to_owned(),
                ..search::Query::default()
            },
            include: Vec::new(),
            exclude: Vec::new(),
            respect_ignore: true,
            hidden: false,
            workspace: Some(PathBuf::from(path)),
            active_document: None,
            open_documents: Vec::new(),
        });
    }

    unsafe fn create_controls(&mut self, owner: HWND) {
        unsafe {
            self.controls.owner = owner;
            let multiline_style = WINDOW_STYLE(0x0004 | 0x0040 | 0x1000 | 0x0020_0000);
            let checkbox_style = WINDOW_STYLE(0x0003);
            let button =
                |label, id| shell::control(owner, w!("BUTTON"), label, id, WINDOW_STYLE(0));
            let label = |text, id| shell::control(owner, w!("STATIC"), text, id, WINDOW_STYLE(0));

            self.controls.scope_document = button("文書", CONTROL_SCOPE_DOCUMENT);
            self.controls.scope_workspace = button("Workspace", CONTROL_SCOPE_WORKSPACE);
            self.controls.query_label = label("検索", 5620);
            self.controls.query =
                shell::control(owner, w!("EDIT"), "", CONTROL_QUERY, multiline_style);
            self.query_composing = Some(shell::track_control_ime(self.controls.query));
            self.controls.replacement_label = label("置換", 5621);
            self.controls.replacement =
                shell::control(owner, w!("EDIT"), "", CONTROL_REPLACEMENT, multiline_style);
            self.replacement_composing = Some(shell::track_control_ime(self.controls.replacement));
            self.controls.case_sensitive = shell::control(
                owner,
                w!("BUTTON"),
                "大/小文字",
                CONTROL_CASE,
                checkbox_style,
            );
            self.controls.regex = shell::control(
                owner,
                w!("BUTTON"),
                "正規表現",
                CONTROL_REGEX,
                checkbox_style,
            );
            self.controls.word =
                shell::control(owner, w!("BUTTON"), "単語", CONTROL_WORD, checkbox_style);
            self.controls.details = button("対象詳細", CONTROL_DETAILS);
            self.controls.include_label = label("include globs", 5622);
            self.controls.include =
                shell::control(owner, w!("EDIT"), "", CONTROL_INCLUDE, multiline_style);
            self.include_composing = Some(shell::track_control_ime(self.controls.include));
            self.controls.exclude_label = label("exclude globs", 5623);
            self.controls.exclude =
                shell::control(owner, w!("EDIT"), "", CONTROL_EXCLUDE, multiline_style);
            self.exclude_composing = Some(shell::track_control_ime(self.controls.exclude));
            self.controls.respect_ignore = shell::control(
                owner,
                w!("BUTTON"),
                ".gitignoreを尊重",
                CONTROL_IGNORE,
                checkbox_style,
            );
            self.controls.hidden = shell::control(
                owner,
                w!("BUTTON"),
                "隠し項目",
                CONTROL_HIDDEN,
                checkbox_style,
            );
            self.controls.next = button("次", CMD_NEXT);
            self.controls.previous = button("前", CMD_PREVIOUS);
            self.controls.replace_one = button("一致を置換", CMD_REPLACE_ONE);
            self.controls.replace_document_all = button("文書すべて", CMD_REPLACE_DOCUMENT_ALL);
            self.controls.replace_workspace = button("Workspace置換", CMD_REPLACE_WORKSPACE);
            self.controls.journals = button("journal", CMD_OPEN_JOURNALS);
            self.controls.restore = button("復旧", CMD_RESTORE_JOURNAL);
            self.controls.cancel = button("中止", CMD_CANCEL_OPERATION);
            self.controls.close = button("×", CMD_CLOSE);
            self.controls.results = shell::control(
                owner,
                w!("LISTBOX"),
                "",
                CONTROL_RESULTS,
                WINDOW_STYLE(0x0001 | 0x0100 | 0x0020_0000),
            );
            self.controls.status = label("", CONTROL_STATUS);

            let _ = shell::send(
                self.controls.query,
                EM_LIMITTEXT,
                WPARAM(QUERY_CONTROL_UNIT_LIMIT),
                LPARAM(0),
            );
            let _ = shell::send(
                self.controls.replacement,
                EM_LIMITTEXT,
                WPARAM(REPLACEMENT_LIMIT),
                LPARAM(0),
            );
            let _ = shell::send(
                self.controls.include,
                EM_LIMITTEXT,
                WPARAM(GLOB_LIMIT),
                LPARAM(0),
            );
            let _ = shell::send(
                self.controls.exclude,
                EM_LIMITTEXT,
                WPARAM(GLOB_LIMIT),
                LPARAM(0),
            );
            let _ = shell::send(
                self.controls.respect_ignore,
                BM_SETCHECK,
                WPARAM(1),
                LPARAM(0),
            );
            for hwnd in self.controls.all() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
            self.initialized = true;
        }
    }

    fn is_composing(&self) -> bool {
        [
            &self.query_composing,
            &self.replacement_composing,
            &self.include_composing,
            &self.exclude_composing,
        ]
        .into_iter()
        .any(|flag| flag.as_ref().is_some_and(|flag| flag.get()))
    }

    fn position_controls(&self, app: &App) {
        unsafe {
            if !self.initialized {
                return;
            }
            let area = app
                .feature_panel_rect(PanelId::Explorer)
                .unwrap_or_default();
            let visible = app.search_pane_active() && area.width > 0.0 && area.height > 0.0;
            let dpi = app.dpi.max(0.1);
            let position = |hwnd: HWND, r: Rect| {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    (r.x * dpi) as i32,
                    (r.y * dpi) as i32,
                    (r.width.max(0.0) * dpi) as i32,
                    (r.height.max(0.0) * dpi) as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
            };
            let show = |hwnd: HWND, state: bool| {
                let _ = ShowWindow(hwnd, if state { SW_SHOW } else { SW_HIDE });
            };
            let left = area.x + 5.0;
            let width = (area.width - 10.0).max(0.0);
            let mut y = area.y + 3.0;
            let button_h = 23.0;
            let gap = 3.0;
            position(
                self.controls.close,
                Rect {
                    x: left + width - 28.0,
                    y,
                    width: 28.0,
                    height: button_h,
                },
            );
            show(self.controls.close, visible);
            y += button_h + gap;
            let scope_w = (width - gap) * 0.5;
            for (hwnd, rect) in [
                (
                    self.controls.scope_document,
                    Rect {
                        x: left,
                        y,
                        width: scope_w,
                        height: button_h,
                    },
                ),
                (
                    self.controls.scope_workspace,
                    Rect {
                        x: left + scope_w + gap,
                        y,
                        width: scope_w,
                        height: button_h,
                    },
                ),
            ] {
                position(hwnd, rect);
                show(hwnd, visible);
            }
            y += button_h + gap;

            let label_h = 16.0;
            position(
                self.controls.query_label,
                Rect {
                    x: left,
                    y,
                    width,
                    height: label_h,
                },
            );
            show(self.controls.query_label, visible);
            y += label_h;
            let query_h = (area.height * 0.13)
                .clamp(36.0, 58.0)
                .min((area.height * 0.28).max(1.0));
            position(
                self.controls.query,
                Rect {
                    x: left,
                    y,
                    width,
                    height: query_h,
                },
            );
            show(self.controls.query, visible);
            y += query_h + gap;

            position(
                self.controls.replacement_label,
                Rect {
                    x: left,
                    y,
                    width,
                    height: label_h,
                },
            );
            show(self.controls.replacement_label, visible);
            y += label_h;
            let replacement_h = (area.height * 0.10)
                .clamp(28.0, 48.0)
                .min((area.height * 0.20).max(1.0));
            position(
                self.controls.replacement,
                Rect {
                    x: left,
                    y,
                    width,
                    height: replacement_h,
                },
            );
            show(self.controls.replacement, visible);
            y += replacement_h + gap;

            let option_w = (width - 2.0 * gap) / 3.0;
            for (index, hwnd) in [
                self.controls.case_sensitive,
                self.controls.regex,
                self.controls.word,
            ]
            .into_iter()
            .enumerate()
            {
                position(
                    hwnd,
                    Rect {
                        x: left + index as f32 * (option_w + gap),
                        y,
                        width: option_w,
                        height: button_h,
                    },
                );
                show(hwnd, visible);
            }
            y += button_h + gap;
            let details_label = if self.details_open {
                "対象詳細 ▲"
            } else {
                "対象詳細 ▼"
            };
            let details_text = wide(details_label);
            let _ = SetWindowTextW(self.controls.details, PCWSTR(details_text.as_ptr()));
            position(
                self.controls.details,
                Rect {
                    x: left,
                    y,
                    width,
                    height: button_h,
                },
            );
            show(
                self.controls.details,
                visible && self.scope == Scope::Workspace,
            );
            y += button_h + gap;

            if self.details_open && self.scope == Scope::Workspace {
                let detail_h = (area.height * 0.08).clamp(22.0, 38.0);
                for (label_hwnd, edit_hwnd) in [
                    (self.controls.include_label, self.controls.include),
                    (self.controls.exclude_label, self.controls.exclude),
                ] {
                    position(
                        label_hwnd,
                        Rect {
                            x: left,
                            y,
                            width,
                            height: label_h,
                        },
                    );
                    show(label_hwnd, visible);
                    y += label_h;
                    position(
                        edit_hwnd,
                        Rect {
                            x: left,
                            y,
                            width,
                            height: detail_h,
                        },
                    );
                    show(edit_hwnd, visible);
                    y += detail_h + gap;
                }
                let check_w = (width - gap) * 0.5;
                position(
                    self.controls.respect_ignore,
                    Rect {
                        x: left,
                        y,
                        width: check_w,
                        height: button_h,
                    },
                );
                position(
                    self.controls.hidden,
                    Rect {
                        x: left + check_w + gap,
                        y,
                        width: check_w,
                        height: button_h,
                    },
                );
                show(self.controls.respect_ignore, visible);
                show(self.controls.hidden, visible);
                y += button_h + gap;
            } else {
                show(self.controls.include_label, false);
                show(self.controls.include, false);
                show(self.controls.exclude_label, false);
                show(self.controls.exclude, false);
                show(self.controls.respect_ignore, false);
                show(self.controls.hidden, false);
            }

            let action_w = (width - 3.0 * gap) / 4.0;
            for (index, hwnd) in [
                self.controls.next,
                self.controls.previous,
                self.controls.replace_one,
                self.controls.replace_document_all,
            ]
            .into_iter()
            .enumerate()
            {
                position(
                    hwnd,
                    Rect {
                        x: left + index as f32 * (action_w + gap),
                        y,
                        width: action_w,
                        height: button_h,
                    },
                );
                show(hwnd, visible && self.scope == Scope::Document);
            }
            y += button_h + gap;
            for (index, hwnd) in [
                self.controls.replace_workspace,
                self.controls.journals,
                self.controls.restore,
                self.controls.cancel,
            ]
            .into_iter()
            .enumerate()
            {
                position(
                    hwnd,
                    Rect {
                        x: left + index as f32 * (action_w + gap),
                        y,
                        width: action_w,
                        height: button_h,
                    },
                );
                let show_control = visible
                    && match index {
                        0 => self.scope == Scope::Workspace && !self.journal_mode,
                        1 | 2 => self.scope == Scope::Workspace,
                        _ => self.operation.is_some(),
                    };
                show(hwnd, show_control);
            }
            y += button_h + gap;
            let status_h = 38.0f32.min(area.height * 0.1);
            let list_y = y + gap;
            let list_h = (area.y + area.height - status_h - list_y).max(0.0);
            position(
                self.controls.results,
                Rect {
                    x: left,
                    y: list_y,
                    width,
                    height: list_h,
                },
            );
            position(
                self.controls.status,
                Rect {
                    x: left,
                    y: area.y + area.height - status_h,
                    width,
                    height: status_h,
                },
            );
            show(self.controls.results, visible);
            show(self.controls.status, visible);

            let operation_active = self.operation.is_some();
            for hwnd in [
                self.controls.query,
                self.controls.replacement,
                self.controls.case_sensitive,
                self.controls.regex,
                self.controls.word,
                self.controls.details,
                self.controls.include,
                self.controls.exclude,
                self.controls.respect_ignore,
                self.controls.hidden,
                self.controls.scope_document,
                self.controls.scope_workspace,
                self.controls.results,
            ] {
                let _ = EnableWindow(hwnd, !operation_active);
            }
            for (hwnd, command) in [
                (self.controls.next, CMD_NEXT),
                (self.controls.previous, CMD_PREVIOUS),
                (self.controls.replace_one, CMD_REPLACE_ONE),
                (self.controls.replace_document_all, CMD_REPLACE_DOCUMENT_ALL),
                (self.controls.replace_workspace, CMD_REPLACE_WORKSPACE),
                (self.controls.journals, CMD_OPEN_JOURNALS),
                (self.controls.restore, CMD_RESTORE_JOURNAL),
                (self.controls.cancel, CMD_CANCEL_OPERATION),
                (self.controls.close, CMD_CLOSE),
            ] {
                let _ = EnableWindow(hwnd, self.enabled(app, command));
            }
        }
    }

    fn set_status(&mut self, text: impl Into<String>) {
        let text = text.into();
        if self.status == text {
            return;
        }
        self.status = text;
        unsafe {
            let value = wide(&self.status);
            let _ = SetWindowTextW(self.controls.status, PCWSTR(value.as_ptr()));
        }
    }

    fn set_scope(&mut self, scope: Scope, app: &mut App) {
        if self.scope == scope {
            return;
        }
        self.scope = scope;
        self.journal_mode = false;
        self.current = None;
        self.results = None;
        self.result_rows.clear();
        self.inputs_dirty = true;
        self.position_controls(app);
        unsafe {
            let _ = SetFocus(Some(self.controls.query));
        }
    }

    fn query_from_controls(&self, app: &App) -> Result<Option<(SearchKey, SearchBody)>, String> {
        if app.feature_ime_active() || self.is_composing() {
            return Ok(None);
        }
        let text = shell::text_of(self.controls.query);
        if text.is_empty() {
            return Ok(None);
        }
        let query = build_query(
            text,
            checked(self.controls.case_sensitive),
            checked(self.controls.regex),
            checked(self.controls.word),
        )
        .map_err(str::to_owned)?;
        match self.scope {
            Scope::Document => {
                let document_id = app.doc().id();
                let revision = app.doc().revision();
                let source = app.doc().source_shared();
                let key = SearchKey {
                    scope: self.scope,
                    query: query.clone(),
                    include: Vec::new(),
                    exclude: Vec::new(),
                    respect_ignore: true,
                    hidden: false,
                    workspace: None,
                    active_document: Some((document_id, revision)),
                    open_documents: Vec::new(),
                };
                Ok(Some((
                    key,
                    SearchBody::Document {
                        document_id,
                        revision,
                        source,
                        query,
                    },
                )))
            }
            Scope::Workspace => {
                let include_text = shell::text_of(self.controls.include);
                let exclude_text = shell::text_of(self.controls.exclude);
                if include_text.len() > GLOB_LIMIT || exclude_text.len() > GLOB_LIMIT {
                    return Err(format!(
                        "include/excludeは{GLOB_LIMIT} byte以内にしてください"
                    ));
                }
                let root = app
                    .persistence
                    .workspace_root()
                    .ok_or_else(|| "Workspaceが開かれていません".to_owned())?;
                let root = fs::canonicalize(&root).unwrap_or(root);
                let open: Vec<_> = app
                    .tabs
                    .iter()
                    .filter_map(|tab| {
                        let path = tab.file.as_ref()?.path().to_path_buf();
                        Some(OpenSource {
                            path,
                            document_id: tab.doc.id(),
                            revision: tab.doc.revision(),
                            source: tab.doc.source_shared(),
                        })
                    })
                    .collect();
                let mut open_documents: Vec<_> = open
                    .iter()
                    .map(|item| (item.path.clone(), item.document_id, item.revision))
                    .collect();
                open_documents.sort();
                let include = glob_lines(&include_text);
                let exclude = glob_lines(&exclude_text);
                let respect_ignore = checked(self.controls.respect_ignore);
                let hidden = checked(self.controls.hidden);
                let options = Options {
                    include: include.clone(),
                    exclude: exclude.clone(),
                    respect_ignore,
                    hidden,
                };
                let key = SearchKey {
                    scope: self.scope,
                    query: query.clone(),
                    include,
                    exclude,
                    respect_ignore,
                    hidden,
                    workspace: Some(root.clone()),
                    active_document: None,
                    open_documents,
                };
                Ok(Some((
                    key,
                    SearchBody::Workspace {
                        root,
                        query,
                        options,
                        open,
                    },
                )))
            }
        }
    }

    fn reconcile_inputs(&mut self, app: &App, now: Instant) -> bool {
        if self.operation.is_some() || self.journal_mode {
            return false;
        }
        let captured = match self.query_from_controls(app) {
            Ok(captured) => captured,
            Err(error) => {
                let changed =
                    self.key.is_some() || self.results.is_some() || self.pending.is_some();
                if changed {
                    self.invalidate_search();
                }
                let status_changed = self.status != error;
                self.set_status(error);
                self.inputs_dirty = false;
                return changed || status_changed;
            }
        };
        let Some((key, body)) = captured else {
            let composing = app.feature_ime_active() || self.is_composing();
            let had_search = self.key.is_some() || self.results.is_some() || self.pending.is_some();
            if had_search {
                self.invalidate_search();
            }
            if composing {
                self.set_status("IME変換中 — 確定後に検索します");
            } else if shell::text_of(self.controls.query).is_empty() {
                self.set_status("検索語を入力してください");
            } else {
                self.set_status("検索を待機しています");
            }
            self.inputs_dirty = false;
            return had_search;
        };
        if self.key.as_ref() == Some(&key) && !self.inputs_dirty {
            return false;
        }
        if self.key.as_ref() == Some(&key) {
            self.inputs_dirty = false;
            return false;
        }
        self.key = Some(key);
        self.generation = self.generation.wrapping_add(1);
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.pending = Some(PendingSearch {
            request: SearchRequest {
                generation: self.generation,
                body,
            },
            not_before: now + DEBOUNCE,
        });
        self.results = None;
        self.result_rows.clear();
        self.current = None;
        self.render_rows();
        self.set_status("検索中…");
        self.inputs_dirty = false;
        true
    }

    fn invalidate_search(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.key = None;
        self.pending = None;
        self.results = None;
        self.current = None;
        self.result_rows.clear();
        self.render_rows();
    }

    fn start_search(&mut self, request: SearchRequest) {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let sender = self.sender.clone();
        let generation = request.generation;
        self.active = Some(ActiveSearch { generation, cancel });
        thread::spawn(move || {
            let result = match request.body {
                SearchBody::Document {
                    document_id,
                    revision,
                    source,
                    query,
                } => search::search_snapshot(
                    document_id,
                    revision,
                    source,
                    query,
                    generation,
                    &worker_cancel,
                    Duration::from_millis(200),
                )
                .map(SearchResults::Document)
                .map_err(|error| error.to_string()),
                SearchBody::Workspace {
                    root,
                    query,
                    options,
                    open,
                } => Workspace::open(root)
                    .map_err(|error| error.to_string())
                    .and_then(|workspace| {
                        workspace_search::search_workspace(
                            &workspace,
                            query,
                            &options,
                            &open,
                            generation,
                            &worker_cancel,
                        )
                    })
                    .map(SearchResults::Workspace),
            };
            let _ = sender.send(SearchCompletion { generation, result });
        });
    }

    fn poll_search(&mut self, now: Instant) -> bool {
        let mut changed = false;
        while let Ok(completion) = self.receiver.try_recv() {
            if self
                .active
                .as_ref()
                .is_some_and(|active| active.generation == completion.generation)
            {
                self.active = None;
            }
            if completion.generation != self.generation {
                continue;
            }
            match completion.result {
                Ok(results) => {
                    self.results = Some(results);
                    self.populate_result_rows();
                    self.update_result_status();
                }
                Err(error) => {
                    self.results = None;
                    self.result_rows.clear();
                    self.current = None;
                    self.render_rows();
                    self.set_status(format!("検索失敗: {error}"));
                }
            }
            changed = true;
        }
        if self.active.is_none()
            && self
                .pending
                .as_ref()
                .is_some_and(|pending| pending.not_before <= now)
        {
            let pending = self.pending.take().unwrap();
            if pending.request.generation == self.generation {
                self.start_search(pending.request);
                changed = true;
            }
        }
        changed
    }

    fn populate_result_rows(&mut self) {
        self.result_rows.clear();
        if self.journal_mode {
            self.result_rows
                .extend((0..self.journals.len()).map(ResultRow::Journal));
            self.render_rows();
            return;
        }
        match self.results.as_ref() {
            Some(SearchResults::Document(results)) => {
                self.result_rows.extend(
                    (0..results.hits.len()).map(|hit| ResultRow::Hit(HitLocation { file: 0, hit })),
                );
            }
            Some(SearchResults::Workspace(results)) => {
                for (file, item) in results.files.iter().enumerate() {
                    self.result_rows.push(ResultRow::Group(file));
                    self.result_rows.extend(
                        (0..item.matches.hits.len())
                            .map(|hit| ResultRow::Hit(HitLocation { file, hit })),
                    );
                }
            }
            None => {}
        }
        self.render_rows();
    }

    fn render_rows(&self) {
        unsafe {
            let _ = shell::send(self.controls.results, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for row in &self.result_rows {
                let text = match (row, self.results.as_ref()) {
                    (ResultRow::Group(file), Some(SearchResults::Workspace(result))) => {
                        let item = &result.files[*file];
                        let path = item
                            .path
                            .strip_prefix(&result.root)
                            .unwrap_or(&item.path)
                            .to_string_lossy();
                        format!("{path}  ({} matches)", item.matches.hits.len())
                    }
                    (ResultRow::Hit(location), Some(SearchResults::Document(result))) => {
                        hit_label(&result.source, &result.hits[location.hit].range, None)
                    }
                    (ResultRow::Hit(location), Some(SearchResults::Workspace(result))) => {
                        let item = &result.files[location.file];
                        hit_label(
                            &item.matches.source,
                            &item.matches.hits[location.hit].range,
                            None,
                        )
                    }
                    (ResultRow::Journal(index), _) => self
                        .journals
                        .get(*index)
                        .map(|journal| {
                            let version = journal
                                .version
                                .map_or_else(|| "unknown".to_owned(), |value| format!("V{value}"));
                            let count = journal
                                .file_count
                                .map_or_else(|| "?".to_owned(), |value| value.to_string());
                            format!("{version} | {count} files | {}", journal.path.display())
                        })
                        .unwrap_or_else(|| "journal".to_owned()),
                    _ => continue,
                };
                add_list_item(self.controls.results, &text);
            }
            if let Some(current) = self.current
                && let Some(index) = self
                    .result_rows
                    .iter()
                    .position(|row| matches!(row, ResultRow::Hit(location) if *location == current))
            {
                let _ = shell::send(
                    self.controls.results,
                    LB_SETCURSEL,
                    WPARAM(index),
                    LPARAM(0),
                );
            }
        }
    }

    fn update_result_status(&mut self) {
        match self.results.as_ref() {
            Some(SearchResults::Document(result)) => {
                let suffix = if result.state == SearchState::Complete {
                    "完了".to_owned()
                } else {
                    format!("不完全: {:?}", result.state)
                };
                self.set_status(format!("{}件 — {suffix}", result.hits.len()));
            }
            Some(SearchResults::Workspace(result)) => {
                let hits = result
                    .files
                    .iter()
                    .map(|file| file.matches.hits.len())
                    .sum::<usize>();
                let suffix = if result.complete {
                    "完了".to_owned()
                } else {
                    format!("不完全 / {} issues", result.issues.len())
                };
                self.set_status(format!(
                    "{} files / {hits}件 / {} bytes — {suffix}",
                    result.files.len(),
                    result.scanned_bytes
                ));
            }
            None => self.set_status("検索結果がありません"),
        }
    }

    fn navigate_relative(&mut self, app: &mut App, forward: bool) {
        let hits: Vec<_> = self
            .result_rows
            .iter()
            .filter_map(|row| match row {
                ResultRow::Hit(location) => Some(*location),
                _ => None,
            })
            .collect();
        if hits.is_empty() {
            self.set_status("一致する検索結果がありません");
            return;
        }
        let index = self
            .current
            .and_then(|current| hits.iter().position(|location| *location == current))
            .map_or_else(
                || if forward { 0 } else { hits.len() - 1 },
                |index| {
                    if forward {
                        (index + 1) % hits.len()
                    } else {
                        index.checked_sub(1).unwrap_or(hits.len() - 1)
                    }
                },
            );
        self.navigate_to(app, hits[index]);
    }

    fn navigate_to(&mut self, app: &mut App, location: HitLocation) {
        if app.feature_ime_active() || self.is_composing() {
            self.set_status("IME確定後に検索結果へ移動してください");
            return;
        }
        let target = match self.results.as_ref() {
            Some(SearchResults::Document(results)) => {
                if location.file != 0
                    || results.generation != self.generation
                    || !results.is_current(app.doc(), self.generation)
                {
                    return self.stale_result("文書の検索結果が古くなりました");
                }
                Some((
                    results.document_id,
                    results.revision,
                    results.source.clone(),
                    results.hits.get(location.hit).map(|hit| hit.range.clone()),
                    None,
                    None,
                ))
            }
            Some(SearchResults::Workspace(results)) => {
                if results.generation != self.generation {
                    return self.stale_result("Workspaceの検索結果が古くなりました");
                }
                let Some(file) = results.files.get(location.file) else {
                    return self.stale_result("検索結果のfile indexが不正です");
                };
                Some((
                    file.matches.document_id,
                    file.matches.revision,
                    Arc::clone(&file.matches.source),
                    file.matches
                        .hits
                        .get(location.hit)
                        .map(|hit| hit.range.clone()),
                    Some(file.path.clone()),
                    file.snapshot.as_ref().map(FileSnapshot::baseline_sha256),
                ))
            }
            None => None,
        };
        let Some((document_id, revision, source, range, path, expected_fingerprint)) = target
        else {
            self.set_status("検索結果がありません");
            return;
        };
        let Some(range) = range else {
            return self.stale_result("検索結果のhit indexが不正です");
        };
        if source.get(range.clone()).is_none() {
            return self.stale_result("検索結果の範囲が不正です");
        }

        let tab_index = if document_id != 0 {
            app.tabs.iter().position(|tab| tab.doc.id() == document_id)
        } else {
            let Some(path) = path.as_ref() else {
                return self.stale_result("closed fileのpathがありません");
            };
            if app.open_tab_for_path(path).is_some() {
                return self.stale_result("検索後にfileが開かれました — 再検索してください");
            }
            let Some(expected_fingerprint) = expected_fingerprint else {
                return self.stale_result("closed fileのsnapshotがありません");
            };
            match FileSnapshot::open_bounded(path, FILE_READ_LIMIT) {
                Ok(current)
                    if current.snapshot.baseline_sha256() == expected_fingerprint
                        && current.source == source.as_ref() =>
                {
                    app.open_path(path.clone());
                    app.open_tab_for_path(path)
                }
                Ok(_) => {
                    return self
                        .stale_result("closed fileが検索後に変更されました — 再検索してください");
                }
                Err(error) => {
                    return self.stale_result(format!("検索対象を再確認できません: {error}"));
                }
            }
        };
        let Some(tab_index) = tab_index else {
            return self.stale_result("検索対象の文書を開けませんでした");
        };
        if app.tabs[tab_index].doc.id() != document_id && document_id != 0 {
            return self.stale_result("検索結果のdocument idが変わりました");
        }
        if app.tabs[tab_index].doc.revision() != revision
            || app.tabs[tab_index].doc.source() != source.as_ref()
        {
            return self.stale_result("文書が検索後に変更されました — 再検索してください");
        }
        if tab_index != app.active {
            app.switch_tab(tab_index);
        }
        if app
            .doc_mut()
            .set_selection_with_affinity(range.start, range.end, BoundaryAffinity::After)
            .is_err()
        {
            return self.stale_result("検索結果の選択範囲を設定できませんでした");
        }
        self.current = Some(location);
        self.render_rows();
        self.set_status(format!(
            "{}:{}",
            line_column(&source, range.start).0,
            line_column(&source, range.start).1
        ));
        unsafe { app.after_input() };
    }

    fn stale_result(&mut self, message: impl Into<String>) {
        self.current = None;
        self.invalidate_search();
        self.set_status(message);
        self.inputs_dirty = true;
    }

    fn replace_document(&mut self, app: &mut App, selected_only: bool) {
        if app.feature_ime_active() || self.is_composing() {
            self.set_status("IME確定後に置換してください");
            return;
        }
        if self.scope != Scope::Document {
            self.set_status("文書内置換は文書検索scopeで実行してください");
            return;
        }
        let replacement = shell::text_of(self.controls.replacement);
        if replacement.len() > REPLACEMENT_LIMIT {
            self.set_status("置換文字列は64 MiB以内にしてください");
            return;
        }
        let Some(SearchResults::Document(results)) = self.results.as_ref() else {
            self.set_status("現在文書の検索結果がありません");
            return;
        };
        if !results.is_current(app.doc(), self.generation) {
            self.stale_result("文書が検索後に変更されました — 再検索してください");
            return;
        }
        let index = if selected_only {
            let Some(location) = self
                .current
                .or_else(|| (!results.hits.is_empty()).then_some(HitLocation { file: 0, hit: 0 }))
            else {
                self.set_status("置換する一致がありません");
                return;
            };
            if location.file != 0 || location.hit >= results.hits.len() {
                self.stale_result("現在の一致が古くなりました");
                return;
            }
            Some(location.hit)
        } else {
            None
        };
        let count = match results.replace(app.doc_mut(), self.generation, index, &replacement) {
            Ok(count) => count,
            Err(error) => {
                self.set_status(format!("置換できません: {error}"));
                return;
            }
        };
        if count == 0 {
            self.set_status("置換対象は変更されませんでした");
            return;
        }
        app.layout_key = None;
        app.projection_cache = None;
        app.parsed_tables.clear();
        self.invalidate_search();
        self.inputs_dirty = true;
        self.set_status(format!(
            "{count}件を文書の1回のUndoで置換しました。再検索中…"
        ));
        unsafe { app.after_input() };
    }

    fn open_scope(&mut self, app: &mut App, scope: Scope) {
        if app.feature_ime_active() || self.is_composing() {
            self.set_status("IME確定後に検索を開いてください");
            return;
        }
        if !app.search_pane_active() {
            let selection = app.doc().selection();
            self.focus_restore = Some(FocusSnapshot {
                document_id: app.doc().id(),
                revision: app.doc().revision(),
                selection,
                affinity: app.doc().selection_affinity(),
                hwnd: unsafe { GetFocus() },
            });
            let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
            if !range.is_empty()
                && let Some(selected) = app.doc().source().get(range)
                && selected.len() <= search::MAX_LITERAL_QUERY_BYTES
            {
                let text = wide(selected);
                unsafe {
                    let _ = SetWindowTextW(self.controls.query, PCWSTR(text.as_ptr()));
                }
            }
        }
        if self.scope != scope {
            self.scope = scope;
            self.results = None;
            self.result_rows.clear();
            self.current = None;
            self.key = None;
        }
        self.journal_mode = false;
        self.inputs_dirty = true;
        app.set_search_pane_active(true);
        unsafe {
            let _ = SetFocus(Some(self.controls.query));
            let length = shell::text_of(self.controls.query).encode_utf16().count();
            let _ = shell::send(
                self.controls.query,
                EM_SETSEL,
                WPARAM(length),
                LPARAM(length as isize),
            );
        }
        self.position_controls(app);
    }

    fn close_scope(&mut self, app: &mut App) {
        if self.is_composing() || app.feature_ime_active() {
            self.set_status("IME確定後に検索paneを閉じてください");
            return;
        }
        if let Some(active) = &self.active {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.pending = None;
        self.generation = self.generation.wrapping_add(1);
        self.key = None;
        self.journal_mode = false;
        app.set_search_pane_active(false);
        let focus = self.focus_restore.take();
        if let Some(snapshot) = focus {
            if app.doc().id() == snapshot.document_id
                && app.doc().revision() == snapshot.revision
                && self.current.is_none()
            {
                let _ = app.doc_mut().set_selection_with_affinity(
                    snapshot.selection.anchor,
                    snapshot.selection.head,
                    snapshot.affinity,
                );
            }
            unsafe {
                if snapshot.hwnd != HWND::default() && IsWindow(Some(snapshot.hwnd)).as_bool() {
                    let _ = SetFocus(Some(snapshot.hwnd));
                } else {
                    let _ = SetFocus(Some(app.editing_window()));
                }
            }
        }
        self.position_controls(app);
    }

    unsafe fn control(&mut self, app: &mut App, id: usize, notification: u16) -> bool {
        unsafe {
            if notification == EN_CHANGE as u16 {
                match id {
                    CONTROL_QUERY => {
                        self.journal_mode = false;
                        self.inputs_dirty = true;
                    }
                    CONTROL_INCLUDE | CONTROL_EXCLUDE => {
                        self.journal_mode = false;
                        self.inputs_dirty = true;
                    }
                    CONTROL_REPLACEMENT => {}
                    _ => return false,
                }
                return true;
            }
            if id == CONTROL_RESULTS {
                if notification == LBN_SELCHANGE as u16 || notification == LBN_DBLCLK as u16 {
                    let selected =
                        shell::send(self.controls.results, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    if selected < 0 {
                        return true;
                    }
                    match self.result_rows.get(selected as usize).copied() {
                        Some(ResultRow::Hit(location)) => self.navigate_to(app, location),
                        Some(ResultRow::Journal(index)) => {
                            self.selected_journal = Some(index);
                            let status = self
                                .journals
                                .get(index)
                                .and_then(|journal| journal.recovery_message.clone())
                                .unwrap_or_else(|| "journalを選択しました".to_owned());
                            self.set_status(status);
                            self.position_controls(app);
                        }
                        Some(ResultRow::Group(_)) | None => {}
                    }
                    return true;
                }
                return false;
            }
            if notification != BN_CLICKED as u16 {
                return false;
            }
            match id {
                CONTROL_SCOPE_DOCUMENT => self.set_scope(Scope::Document, app),
                CONTROL_SCOPE_WORKSPACE => self.set_scope(Scope::Workspace, app),
                CONTROL_CASE | CONTROL_REGEX | CONTROL_WORD | CONTROL_IGNORE | CONTROL_HIDDEN => {
                    self.journal_mode = false;
                    self.inputs_dirty = true;
                }
                CONTROL_DETAILS => {
                    self.details_open = !self.details_open;
                    self.position_controls(app);
                }
                CMD_DOCUMENT_SEARCH => self.open_scope(app, Scope::Document),
                CMD_WORKSPACE_SEARCH => self.open_scope(app, Scope::Workspace),
                CMD_NEXT
                | CMD_PREVIOUS
                | CMD_REPLACE_ONE
                | CMD_REPLACE_DOCUMENT_ALL
                | CMD_REPLACE_WORKSPACE
                | CMD_OPEN_JOURNALS
                | CMD_RESTORE_JOURNAL
                | CMD_CLOSE
                | CMD_CANCEL_OPERATION => {
                    self.command(app, id);
                }
                _ => return false,
            }
            true
        }
    }

    fn message(&mut self, app: &App, message: &MSG) -> bool {
        if message.message != WM_KEYDOWN || !app.search_pane_active() {
            return false;
        }
        let key = message.wParam.0 as u16;
        let focus = unsafe { GetFocus() };
        if self.is_composing() || app.feature_ime_active() {
            return false;
        }
        let edit_focused = [
            self.controls.query,
            self.controls.replacement,
            self.controls.include,
            self.controls.exclude,
        ]
        .contains(&focus);
        if key == VK_RETURN.0 && edit_focused {
            self.post_command(if unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0 {
                CMD_PREVIOUS
            } else {
                CMD_NEXT
            });
            return true;
        }
        if key == VK_ESCAPE.0 {
            if self.operation.is_some() {
                self.post_command(CMD_CANCEL_OPERATION);
                return true;
            }
            self.post_command(CMD_CLOSE);
            return true;
        }
        false
    }

    fn post_command(&self, command: usize) {
        unsafe {
            if self.controls.owner != HWND::default() {
                let _ = PostMessageW(
                    Some(self.controls.owner),
                    WM_COMMAND,
                    WPARAM(command),
                    LPARAM(0),
                );
            }
        }
    }

    fn command(&mut self, app: &mut App, id: usize) {
        if id != CMD_CANCEL_OPERATION && !self.enabled(app, id) {
            return;
        }
        if self.operation.is_some() && id != CMD_CANCEL_OPERATION {
            return;
        }
        match id {
            CMD_DOCUMENT_SEARCH => self.open_scope(app, Scope::Document),
            CMD_WORKSPACE_SEARCH => self.open_scope(app, Scope::Workspace),
            CMD_NEXT => self.navigate_relative(app, true),
            CMD_PREVIOUS => self.navigate_relative(app, false),
            CMD_REPLACE_ONE => self.replace_document(app, true),
            CMD_REPLACE_DOCUMENT_ALL => self.replace_document(app, false),
            CMD_REPLACE_WORKSPACE => self.start_workspace_preview(app),
            CMD_OPEN_JOURNALS => self.open_journals(app),
            CMD_RESTORE_JOURNAL => self.start_recovery_preview(app),
            CMD_CLOSE => self.close_scope(app),
            CMD_CANCEL_OPERATION => self.cancel_operation(),
            _ => {}
        }
    }

    fn enabled(&self, app: &App, id: usize) -> bool {
        let composed = app.feature_ime_active() || self.is_composing();
        match id {
            CMD_DOCUMENT_SEARCH | CMD_WORKSPACE_SEARCH => !composed && self.operation.is_none(),
            CMD_NEXT | CMD_PREVIOUS => {
                !composed
                    && app.search_pane_active()
                    && self.results.as_ref().is_some_and(|results| match results {
                        SearchResults::Document(result) => {
                            result.generation == self.generation
                                && result.is_current(app.doc(), self.generation)
                        }
                        SearchResults::Workspace(result) => result.generation == self.generation,
                    })
                    && self
                        .result_rows
                        .iter()
                        .any(|row| matches!(row, ResultRow::Hit(_)))
            }
            CMD_REPLACE_ONE | CMD_REPLACE_DOCUMENT_ALL => {
                !composed
                    && self.scope == Scope::Document
                    && self.operation.is_none()
                    && matches!(
                        self.results.as_ref(),
                        Some(SearchResults::Document(results))
                            if results.state == SearchState::Complete
                                && results.is_current(app.doc(), self.generation)
                                && !results.hits.is_empty()
                    )
            }
            CMD_REPLACE_WORKSPACE => {
                !composed
                    && self.scope == Scope::Workspace
                    && !self.journal_mode
                    && self.operation.is_none()
                    && matches!(
                        self.results.as_ref(),
                        Some(SearchResults::Workspace(results))
                            if results.complete
                                && results.generation == self.generation
                                && results.files.iter().any(|file| !file.matches.hits.is_empty())
                    )
            }
            CMD_OPEN_JOURNALS => {
                !composed && self.operation.is_none() && app.persistence.workspace_root().is_some()
            }
            CMD_RESTORE_JOURNAL => {
                !composed
                    && self.scope == Scope::Workspace
                    && self.journal_mode
                    && self.operation.is_none()
                    && app.persistence.workspace_root().is_some()
                    && self.selected_journal.is_some_and(|index| {
                        self.journals
                            .get(index)
                            .is_some_and(|journal| journal.recovery_message.is_none())
                    })
            }
            CMD_CLOSE => !composed && app.search_pane_active() && self.operation.is_none(),
            CMD_CANCEL_OPERATION => self.operation.is_some(),
            _ => true,
        }
    }

    fn disabled_reason(&self, app: &App, id: usize) -> &'static str {
        let composed = app.feature_ime_active() || self.is_composing();
        if composed {
            return "IME変換中のため実行できません";
        }
        if let Some(operation) = &self.operation {
            if id == CMD_CANCEL_OPERATION {
                return "中止可能点で停止しています";
            }
            return if matches!(
                operation.phase,
                OperationPhase::WorkspaceApply | OperationPhase::RecoveryRestore
            ) {
                "置換の適用中です — 中止または完了を待ってください"
            } else {
                "検索またはjournalのpreview中です — 完了を待ってください"
            };
        }
        match id {
            CMD_DOCUMENT_SEARCH | CMD_WORKSPACE_SEARCH => "検索を開始できません",
            CMD_NEXT | CMD_PREVIOUS => {
                if !app.search_pane_active() {
                    "検索paneを開いてください"
                } else if shell::text_of(self.controls.query).is_empty() {
                    "検索語を入力してください"
                } else {
                    match self.results.as_ref() {
                        Some(SearchResults::Document(result))
                            if result.generation != self.generation
                                || !result.is_current(app.doc(), self.generation) =>
                        {
                            "検索結果が古くなりました — 再検索してください"
                        }
                        Some(SearchResults::Workspace(result))
                            if result.generation != self.generation =>
                        {
                            "検索結果が古くなりました — 再検索してください"
                        }
                        Some(_) => "一致する検索結果がありません",
                        None => "検索結果を待っています",
                    }
                }
            }
            CMD_REPLACE_ONE | CMD_REPLACE_DOCUMENT_ALL => {
                if self.scope != Scope::Document {
                    "文書内置換には文書検索scopeが必要です"
                } else {
                    match self.results.as_ref() {
                        Some(SearchResults::Document(result))
                            if result.state != SearchState::Complete =>
                        {
                            "検索が不完全です — 完了した結果だけ置換できます"
                        }
                        Some(SearchResults::Document(result))
                            if !result.is_current(app.doc(), self.generation) =>
                        {
                            "文書が検索後に変更されました — 再検索してください"
                        }
                        Some(SearchResults::Document(_)) => "置換する一致がありません",
                        _ => "現在文書の検索結果を待っています",
                    }
                }
            }
            CMD_REPLACE_WORKSPACE => {
                if app.persistence.workspace_root().is_none() {
                    "Workspaceが開かれていません"
                } else if self.scope != Scope::Workspace || self.journal_mode {
                    "Workspace検索結果を表示してください"
                } else {
                    match self.results.as_ref() {
                        Some(SearchResults::Workspace(result)) if !result.complete => {
                            "Workspace検索が不完全です — 全件完了後に置換できます"
                        }
                        Some(SearchResults::Workspace(result))
                            if result.generation != self.generation =>
                        {
                            "Workspace検索結果が古くなりました — 再検索してください"
                        }
                        Some(SearchResults::Workspace(_)) => "置換する一致がありません",
                        _ => "Workspace検索結果を待っています",
                    }
                }
            }
            CMD_OPEN_JOURNALS if app.persistence.workspace_root().is_none() => {
                "置換journalを開くWorkspaceがありません"
            }
            CMD_OPEN_JOURNALS => "journal一覧は開けません",
            CMD_RESTORE_JOURNAL if app.persistence.workspace_root().is_none() => {
                "journalのWorkspaceがありません"
            }
            CMD_RESTORE_JOURNAL if !self.journal_mode => "journal一覧から対象を選択してください",
            CMD_RESTORE_JOURNAL if self.selected_journal.is_none() => {
                "復旧するjournalを選択してください"
            }
            CMD_RESTORE_JOURNAL => "journalが破損または非対応です。原本は保持されています",
            CMD_CLOSE if !app.search_pane_active() => "検索paneは開いていません",
            CMD_CLOSE => "検索paneを閉じられません",
            CMD_CANCEL_OPERATION => "中止できる処理はありません",
            _ => "この操作は現在実行できません",
        }
    }

    fn click(&mut self, app: &App, panel: PanelId, x: f32, y: f32) -> bool {
        if panel != PanelId::Explorer || !app.search_pane_active() {
            return false;
        }
        app.feature_panel_rect(PanelId::Explorer)
            .is_some_and(|area| area.contains(x, y))
    }

    fn tick(&mut self, app: &mut App, now: Instant) -> bool {
        let mut changed = self.poll_operation(app);
        if app.search_pane_active()
            && !self.operation.as_ref().is_some_and(|operation| {
                matches!(
                    operation.phase,
                    OperationPhase::WorkspacePreview | OperationPhase::RecoveryPreview
                )
            })
        {
            changed |= self.reconcile_inputs(app, now);
        }
        changed |= self.poll_search(now);
        if changed {
            self.position_controls(app);
        }
        changed
    }

    fn busy(&self) -> bool {
        self.operation.as_ref().is_some_and(|operation| {
            matches!(
                operation.phase,
                OperationPhase::WorkspaceApply | OperationPhase::RecoveryRestore
            )
        })
    }

    fn start_workspace_preview(&mut self, app: &mut App) {
        if app.feature_ime_active() || self.is_composing() || self.scope != Scope::Workspace {
            self.set_status("Workspace置換にはIME確定とWorkspace検索scopeが必要です");
            return;
        }
        let replacement = shell::text_of(self.controls.replacement);
        if replacement.len() > REPLACEMENT_LIMIT {
            self.set_status("置換文字列は64 MiB以内にしてください");
            return;
        }
        let Some(SearchResults::Workspace(results)) = self.results.take() else {
            self.set_status("Workspace検索結果がありません");
            return;
        };
        let Some(key) = self.key.as_ref() else {
            self.results = Some(SearchResults::Workspace(results));
            self.set_status("検索結果のquery generationがありません");
            return;
        };
        if !results.complete || results.generation != self.generation {
            self.results = Some(SearchResults::Workspace(results));
            self.set_status("Workspace検索結果が不完全または古いため置換できません");
            return;
        }
        let query = key.query.clone();
        let generation = self.generation;
        let root = results.root.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.operation = Some(Operation {
            cancel,
            receiver,
            phase: OperationPhase::WorkspacePreview,
            open_edits: Vec::new(),
            replacement_count: 0,
            open_document_count: 0,
            journal_path: None,
            workspace_root: Some(root.clone()),
        });
        self.set_status("Workspace置換previewを作成しています…");
        self.position_controls(app);
        thread::spawn(move || {
            let plan = Workspace::open(root)
                .map_err(|error| error.to_string())
                .and_then(|workspace| {
                    replace::preview_workspace_replace(
                        &results,
                        &query,
                        &replacement,
                        generation,
                        &workspace,
                        &worker_cancel,
                    )
                    .map_err(|error| error.to_string())
                });
            let _ = sender.send(OperationCompletion::WorkspacePreview { results, plan });
        });
    }

    fn open_journals(&mut self, app: &mut App) {
        if app.feature_ime_active() || self.is_composing() {
            self.set_status("IME確定後にjournalを開いてください");
            return;
        }
        if !app.search_pane_active() {
            self.open_scope(app, Scope::Workspace);
        }
        let Some(root) = app.persistence.workspace_root() else {
            self.set_status("journalを開くWorkspaceがありません");
            return;
        };
        let result = Workspace::open(root)
            .map_err(|error| error.to_string())
            .and_then(|workspace| {
                replace::enumerate_journals(&workspace).map_err(|e| e.to_string())
            });
        match result {
            Ok(journals) => {
                self.journals = journals;
                self.journal_mode = true;
                self.selected_journal = None;
                self.current = None;
                self.result_rows = (0..self.journals.len()).map(ResultRow::Journal).collect();
                self.render_rows();
                if self.journals.is_empty() {
                    self.set_status("Workspaceに置換journalはありません");
                } else {
                    self.set_status(format!(
                        "{}件のjournal — 復旧対象を選択してください",
                        self.journals.len()
                    ));
                }
            }
            Err(error) => self.set_status(format!("journal一覧を読めません: {error}")),
        }
        self.position_controls(app);
    }

    fn start_recovery_preview(&mut self, app: &mut App) {
        if app.feature_ime_active() || self.is_composing() {
            self.set_status("IME確定後にjournalを復旧してください");
            return;
        }
        let Some(index) = self.selected_journal else {
            self.set_status("復旧するjournalを選択してください");
            return;
        };
        let Some(journal) = self.journals.get(index) else {
            self.set_status("選択したjournalが見つかりません");
            return;
        };
        if journal.recovery_message.is_some() {
            self.set_status("journalは破損または非対応のため復旧できません。原本は保持しています");
            return;
        }
        let Some(root) = app.persistence.workspace_root() else {
            self.set_status("journalのWorkspaceがありません");
            return;
        };
        let journal_path = journal.path.clone();
        let open_paths = open_document_paths(app);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        self.operation = Some(Operation {
            cancel,
            receiver,
            phase: OperationPhase::RecoveryPreview,
            open_edits: Vec::new(),
            replacement_count: 0,
            open_document_count: 0,
            journal_path: Some(journal_path.clone()),
            workspace_root: Some(root.clone()),
        });
        self.set_status("journalと現在のfileを検証しています…");
        self.position_controls(app);
        thread::spawn(move || {
            let result = if worker_cancel.load(Ordering::Relaxed) {
                Err("journal preview was cancelled".to_owned())
            } else {
                Workspace::open(root)
                    .map_err(|error| error.to_string())
                    .and_then(|workspace| {
                        replace::preview_recovery(&workspace, &journal_path, &open_paths)
                            .map_err(|error| error.to_string())
                    })
            };
            let _ = sender.send(OperationCompletion::RecoveryPreview(result));
        });
    }

    fn cancel_operation(&mut self) {
        let Some(operation) = &self.operation else {
            return;
        };
        operation.cancel.store(true, Ordering::Relaxed);
        self.set_status("中止を要求しました — 安全な停止点まで待っています");
    }

    fn poll_operation(&mut self, app: &mut App) -> bool {
        let received = self
            .operation
            .as_mut()
            .map(|operation| operation.receiver.try_recv());
        match received {
            Some(Ok(completion)) => {
                self.handle_operation_completion(app, completion);
                true
            }
            Some(Err(TryRecvError::Disconnected)) => {
                self.operation = None;
                self.position_controls(app);
                self.set_status("background operation ended without a result");
                true
            }
            Some(Err(TryRecvError::Empty)) | None => false,
        }
    }

    fn handle_operation_completion(&mut self, app: &mut App, completion: OperationCompletion) {
        match completion {
            OperationCompletion::WorkspacePreview { results, plan } => {
                self.results = Some(SearchResults::Workspace(results));
                let cancelled = self
                    .operation
                    .as_ref()
                    .is_some_and(|operation| operation.cancel.load(Ordering::Relaxed));
                if cancelled {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status("Workspace置換previewを中止しました。fileは変更していません");
                    self.position_controls(app);
                    return;
                }
                let plan = match plan {
                    Ok(plan) => plan,
                    Err(error) => {
                        self.operation = None;
                        self.populate_result_rows();
                        self.set_status(format!("Workspace置換previewに失敗しました: {error}"));
                        self.position_controls(app);
                        return;
                    }
                };
                if !workspace_matches(app, plan.root()) {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status(
                        "Workspace changed during replacement preview — no file was changed",
                    );
                    self.position_controls(app);
                    return;
                }
                let open_edits = open_edits_from_plan(&plan);
                if !open_edits
                    .iter()
                    .all(|edit| open_edit_matches(app, edit, self.generation))
                {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status(
                        "open documentがpreview後に変更されました — fileは変更していません",
                    );
                    self.position_controls(app);
                    return;
                }
                if plan.replacement_count() == 0 {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status("置換対象がありません");
                    self.position_controls(app);
                    return;
                }
                let prompt = workspace_preview_message(&plan);
                if !confirm(self.controls.owner, "Workspace置換の確認", &prompt) {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status("Workspace置換を取り消しました。fileは変更していません");
                    self.position_controls(app);
                    return;
                }
                if app.feature_ime_active() || self.is_composing() {
                    self.operation = None;
                    self.populate_result_rows();
                    self.set_status("IMEが開始したため置換を中止しました。fileは変更していません");
                    self.position_controls(app);
                    return;
                }
                self.start_apply_worker(app, plan, open_edits);
            }
            OperationCompletion::Replace(result) => self.finish_workspace_replace(app, result),
            OperationCompletion::RecoveryPreview(result) => {
                let cancelled = self
                    .operation
                    .as_ref()
                    .is_some_and(|operation| operation.cancel.load(Ordering::Relaxed));
                if cancelled {
                    self.operation = None;
                    self.set_status("journal previewを中止しました。fileは変更していません");
                    self.position_controls(app);
                    return;
                }
                let preview = match result {
                    Ok(preview) => preview,
                    Err(error) => {
                        self.operation = None;
                        self.set_status(format!("journal復旧previewに失敗しました: {error}"));
                        self.position_controls(app);
                        return;
                    }
                };
                let expected_root = self
                    .operation
                    .as_ref()
                    .and_then(|operation| operation.workspace_root.as_deref());
                if !expected_root.is_some_and(|root| {
                    workspace_matches(app, root)
                        && self
                            .operation
                            .as_ref()
                            .and_then(|operation| operation.journal_path.as_ref())
                            == Some(&preview.journal)
                }) {
                    self.operation = None;
                    self.set_status(
                        "Workspace changed during journal preview — no file was changed",
                    );
                    self.position_controls(app);
                    return;
                }
                let needs_restore = preview
                    .files
                    .iter()
                    .filter(|file| file.state == replace::RecoveryFileState::NeedsRestore)
                    .count();
                let already_restored = preview.files.len().saturating_sub(needs_restore);
                if needs_restore == 0 {
                    self.operation = None;
                    self.set_status(format!(
                        "journalのfileはすべて復旧済みです ({already_restored})"
                    ));
                    self.position_controls(app);
                    return;
                }
                let paths = preview
                    .files
                    .iter()
                    .filter(|file| file.state == replace::RecoveryFileState::NeedsRestore)
                    .take(5)
                    .map(|file| file.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join("\n");
                let prompt = format!(
                    "Journal V{}\n{} file(s) will be restored; {} already restored.\n{}\n\nRestore the before-snapshots?",
                    preview.version, needs_restore, already_restored, paths,
                );
                if !confirm(self.controls.owner, "置換journalの復旧確認", &prompt) {
                    self.operation = None;
                    self.set_status("journal復旧を取り消しました。fileは変更していません");
                    self.position_controls(app);
                    return;
                }
                if app.feature_ime_active() || self.is_composing() {
                    self.operation = None;
                    self.set_status("IMEが開始したためjournal復旧を中止しました");
                    self.position_controls(app);
                    return;
                }
                self.start_restore_worker(app);
            }
            OperationCompletion::Restore(result) => self.finish_restore(app, result),
        }
    }

    fn start_apply_worker(&mut self, app: &mut App, plan: ReplacePlan, open_edits: Vec<OpenEdit>) {
        if !workspace_matches(app, plan.root()) {
            self.operation = None;
            self.populate_result_rows();
            self.set_status("replacement Workspace is no longer active — no file was changed");
            self.position_controls(app);
            return;
        }
        if !open_edits
            .iter()
            .all(|edit| open_edit_matches(app, edit, self.generation))
        {
            self.operation = None;
            self.populate_result_rows();
            self.set_status("open document preflight failed — no closed file was changed");
            self.position_controls(app);
            return;
        }
        let workspace = match Workspace::open(plan.root()) {
            Ok(workspace) => workspace,
            Err(error) => {
                self.operation = None;
                self.populate_result_rows();
                self.set_status(format!("Workspaceを再確認できません: {error}"));
                self.position_controls(app);
                return;
            }
        };
        let open_paths = open_document_paths(app);
        let replacement_count = plan.replacement_count();
        let open_document_count = plan.open_edits().len();
        let closed_file_count = plan.closed_edits().len();
        let Some(existing_operation) = self.operation.as_ref() else {
            self.set_status("replacement operation disappeared before apply");
            return;
        };
        if existing_operation.cancel.load(Ordering::Relaxed) {
            self.operation = None;
            self.populate_result_rows();
            self.set_status("Workspace置換を中止しました。fileは変更していません");
            self.position_controls(app);
            return;
        }
        let cancel = Arc::clone(&existing_operation.cancel);
        let (sender, receiver) = mpsc::channel();
        if let Some(operation) = self.operation.as_mut() {
            operation.receiver = receiver;
            operation.phase = OperationPhase::WorkspaceApply;
            operation.open_edits = open_edits;
            operation.replacement_count = replacement_count;
            operation.open_document_count = open_document_count;
        }
        self.set_status(format!(
            "置換中… {closed_file_count} closed files / {replacement_count} matches"
        ));
        unsafe {
            let _ = SetFocus(Some(self.controls.cancel));
        }
        self.position_controls(app);
        thread::spawn(move || {
            let result = replace::apply_closed_replacements(&workspace, &plan, &open_paths, || {
                cancel.load(Ordering::Relaxed)
            })
            .map_err(|error| error.to_string());
            let _ = sender.send(OperationCompletion::Replace(result));
        });
    }

    fn finish_workspace_replace(&mut self, app: &mut App, result: Result<ApplyOutcome, String>) {
        let (mut open_edits, total_count, open_count) = match self.operation.as_mut() {
            Some(operation) => (
                std::mem::take(&mut operation.open_edits),
                operation.replacement_count,
                operation.open_document_count,
            ),
            None => {
                self.set_status("replacement completion has no operation owner");
                return;
            }
        };
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                self.operation = None;
                self.invalidate_search();
                self.inputs_dirty = true;
                self.set_status(format!("Workspace置換失敗: {error} — 文書置換は未適用です"));
                self.position_controls(app);
                return;
            }
        };
        if !outcome.complete {
            let journal = outcome
                .journal
                .as_ref()
                .map_or_else(String::new, |path| format!("; journal {}", path.display()));
            let stopped = outcome
                .stopped_at
                .as_ref()
                .map_or_else(String::new, |path| {
                    format!("; stopped at {}", path.display())
                });
            let message = outcome.message.as_deref().unwrap_or("operation stopped");
            let stopped_label = if outcome.cancelled {
                "Workspace置換を中止しました"
            } else {
                "Workspace置換は部分停止しました"
            };
            self.operation = None;
            self.invalidate_search();
            self.inputs_dirty = true;
            self.set_status(format!(
                "{stopped_label}: closed {} files / {} replacements applied; {} files / {} replacements remain; open documents skipped; {}{}{}",
                outcome.applied_files,
                outcome.applied_replacements,
                outcome.remaining_files,
                outcome.remaining_replacements,
                message,
                stopped,
                journal,
            ));
            self.position_controls(app);
            return;
        }

        let active_id = app.doc().id();
        let active_revision = app.doc().revision();
        let mut applied_documents = 0usize;
        let mut applied_open_replacements = 0usize;
        let mut open_failure = app
            .composing
            .then(|| "IME composition started before open-document edits".to_owned());
        for edit in open_edits.drain(..) {
            if open_failure.is_some() {
                break;
            }
            if app.feature_ime_active() || self.is_composing() {
                open_failure =
                    Some("IME composition started before an open-document Undo".to_owned());
                break;
            }
            if !open_edit_matches(app, &edit, self.generation) {
                open_failure = Some(format!(
                    "open document {} changed before its Undo",
                    edit.path.display()
                ));
                break;
            }
            let Some(index) = app
                .tabs
                .iter()
                .position(|tab| tab.doc.id() == edit.document_id)
            else {
                open_failure = Some(format!(
                    "open document closed before its Undo: {}",
                    edit.path.display()
                ));
                break;
            };
            let source_len = edit.expected_source.len();
            if let Err(error) = app.tabs[index].doc.replace_range_with_selection(
                0..source_len,
                &edit.after_source,
                edit.selection_after,
            ) {
                open_failure = Some(format!("open document replacement failed: {error}"));
                break;
            }
            applied_documents += 1;
            applied_open_replacements += edit.replacement_count;
        }
        let journal = outcome.journal.as_ref().map_or_else(
            || "no closed-file journal".to_owned(),
            |path| format!("journal {}", path.display()),
        );
        self.operation = None;
        self.invalidate_search();
        self.inputs_dirty = true;
        if app.doc().id() == active_id && app.doc().revision() != active_revision {
            app.layout_key = None;
            app.projection_cache = None;
            app.parsed_tables.clear();
            unsafe { app.after_input() };
        }
        if let Some(failure) = open_failure {
            self.set_status(format!(
                "closed置換完了 ({}/{total_count})。open documentsは{applied_documents}/{open_count} documents / {applied_open_replacements}件まで適用し、残りで停止: {failure}; {journal}",
                outcome.applied_replacements,
            ));
        } else {
            self.set_status(format!(
                "Workspace置換完了: closed {} files / {} replacements, open {} documents / {} replacements; {journal}",
                outcome.applied_files,
                outcome.applied_replacements,
                applied_documents,
                applied_open_replacements,
            ));
        }
        self.position_controls(app);
    }

    fn start_restore_worker(&mut self, app: &mut App) {
        let Some(existing_operation) = self.operation.as_ref() else {
            return;
        };
        let Some(journal_path) = existing_operation.journal_path.clone() else {
            self.operation = None;
            self.set_status("復旧対象journalがありません");
            self.position_controls(app);
            return;
        };
        let Some(root) = app.persistence.workspace_root() else {
            self.operation = None;
            self.set_status("journalのWorkspaceがありません");
            self.position_controls(app);
            return;
        };
        if !existing_operation
            .workspace_root
            .as_deref()
            .is_some_and(|expected| workspace_matches(app, expected))
        {
            self.operation = None;
            self.set_status("journal Workspace changed before restore — no file was changed");
            self.position_controls(app);
            return;
        }
        if existing_operation.cancel.load(Ordering::Relaxed) {
            self.operation = None;
            self.set_status("journal復旧を中止しました。fileは変更していません");
            self.position_controls(app);
            return;
        }
        let open_paths = open_document_paths(app);
        let cancel = Arc::clone(&existing_operation.cancel);
        let (sender, receiver) = mpsc::channel();
        if let Some(operation) = self.operation.as_mut() {
            operation.receiver = receiver;
            operation.phase = OperationPhase::RecoveryRestore;
        }
        self.set_status("journal復旧中… 中止は安全なfile境界で反映します");
        unsafe {
            let _ = SetFocus(Some(self.controls.cancel));
        }
        self.position_controls(app);
        thread::spawn(move || {
            let result = Workspace::open(root)
                .map_err(|error| error.to_string())
                .and_then(|workspace| {
                    replace::restore_journal(&workspace, &journal_path, &open_paths, || {
                        cancel.load(Ordering::Relaxed)
                    })
                    .map_err(|error| error.to_string())
                });
            let _ = sender.send(OperationCompletion::Restore(result));
        });
    }

    fn finish_restore(&mut self, app: &mut App, result: Result<RecoveryOutcome, String>) {
        self.operation = None;
        self.invalidate_search();
        self.inputs_dirty = true;
        let status = match result {
            Ok(outcome) if outcome.complete => {
                format!(
                    "journal復旧完了: {} file(s) restored, {} already restored",
                    outcome.restored_files, outcome.already_restored_files
                )
            }
            Ok(outcome) => {
                let message = outcome.message.as_deref().unwrap_or("operation stopped");
                let stopped = if outcome.cancelled {
                    "journal復旧を中止しました"
                } else {
                    "journal復旧は部分停止しました"
                };
                format!(
                    "{stopped}: {} restored, {} remain; {}; {}",
                    outcome.restored_files,
                    outcome.remaining_files,
                    message,
                    outcome
                        .stopped_at
                        .as_ref()
                        .map_or_else(String::new, |path| path.display().to_string()),
                )
            }
            Err(error) => format!("journal復旧に失敗しました: {error}"),
        };
        if self.journal_mode {
            self.open_journals(app);
        }
        self.set_status(status);
        self.position_controls(app);
    }

    fn reveal(&self, app: &App) -> Option<Range<usize>> {
        if !app.search_pane_active() || app.tabs[app.active].raw {
            return None;
        }
        let (_, range) = self.current_hit_for_document(app.doc())?;
        let table = app
            .parsed_tables
            .iter()
            .find(|table| range.start >= table.range.start && range.end <= table.range.end)?;
        let fully_visible = app
            .views
            .iter()
            .any(|view| fully_visible_range(&view.map, &view.before, &range));
        (!fully_visible).then(|| table.range.clone())
    }

    unsafe fn highlight_view(
        &self,
        app: &App,
        target: &ID2D1HwndRenderTarget,
        view: &TextView,
        x: f32,
        y: f32,
    ) -> windows::core::Result<()> {
        unsafe {
            if !app.search_pane_active() || app.composing || app.tabs[app.active].raw {
                return Ok(());
            }
            let Some((current, file_index, hits)) = self.hits_for_document(app.doc()) else {
                return Ok(());
            };
            let mut ordinary_color = app.shell.tokens.accent;
            ordinary_color.a = 0.27;
            let mut current_color = app.shell.tokens.accent;
            current_color.a = 0.58;
            let ordinary = target.CreateSolidColorBrush(&ordinary_color, None)?;
            let selected = target.CreateSolidColorBrush(&current_color, None)?;
            let first_hit = hits.partition_point(|hit| hit.range.end < view.source.start);
            for (hit_index, hit) in hits.iter().enumerate().skip(first_hit) {
                if hit.range.start > view.source.end {
                    break;
                }
                let location = HitLocation {
                    file: file_index,
                    hit: hit_index,
                };
                let range = &hit.range;
                let start = view.map.partition_point(|&byte| byte < range.start);
                let end = view.map.partition_point(|&byte| byte < range.end);
                if end <= start {
                    continue;
                }
                let mut count = 0;
                let _ = view.layout.HitTestTextRange(
                    start as u32,
                    (end - start) as u32,
                    x,
                    y,
                    None,
                    &mut count,
                );
                if count == 0 {
                    continue;
                }
                let mut metrics = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
                let _ = view.layout.HitTestTextRange(
                    start as u32,
                    (end - start) as u32,
                    x,
                    y,
                    Some(&mut metrics),
                    &mut count,
                );
                let brush = if current == Some(location) {
                    &selected
                } else {
                    &ordinary
                };
                for item in metrics {
                    target
                        .FillRectangle(&rect(item.left, item.top, item.width, item.height), brush);
                }
            }
            Ok(())
        }
    }

    fn current_hit_for_document(&self, doc: &Document) -> Option<(HitLocation, Range<usize>)> {
        let location = self.current?;
        let (_, file_index, hits) = self.hits_for_document(doc)?;
        if location.file != file_index {
            return None;
        }
        Some((location, hits.get(location.hit)?.range.clone()))
    }

    fn hits_for_document<'a>(
        &'a self,
        doc: &Document,
    ) -> Option<(Option<HitLocation>, usize, &'a [search::Hit])> {
        match self.results.as_ref()? {
            SearchResults::Document(results) if results.is_current(doc, self.generation) => {
                Some((self.current, 0, &results.hits))
            }
            SearchResults::Workspace(results) if results.generation == self.generation => {
                let file = results.files.iter().enumerate().find(|(_, file)| {
                    file.matches.document_id == doc.id()
                        && file.matches.revision == doc.revision()
                        && file.matches.source.as_ref() == doc.source()
                })?;
                Some((self.current, file.0, &file.1.matches.hits))
            }
            _ => None,
        }
    }
}

fn checked(hwnd: HWND) -> bool {
    unsafe { shell::send(hwnd, BM_GETCHECK, WPARAM(0), LPARAM(0)).0 == 1 }
}

fn build_query(
    text: String,
    case_sensitive: bool,
    regex: bool,
    word: bool,
) -> Result<Query, &'static str> {
    if regex {
        if text.encode_utf16().count() > search::MAX_REGEX_QUERY_UTF16 {
            return Err("正規表現は4096 UTF-16単位以内にしてください");
        }
    } else if text.len() > search::MAX_LITERAL_QUERY_BYTES {
        return Err("通常検索語はUTF-8 8 MiB以内にしてください");
    }
    Ok(Query {
        text,
        case_sensitive,
        regex,
        word,
    })
}

fn glob_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect()
}

fn add_list_item(hwnd: HWND, text: &str) {
    unsafe {
        let encoded = wide(text);
        let _ = shell::send(
            hwnd,
            LB_ADDSTRING,
            WPARAM(0),
            LPARAM(encoded.as_ptr() as isize),
        );
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit('\n')
        .next()
        .unwrap_or_default()
        .trim_end_matches('\r')
        .chars()
        .count()
        + 1;
    (line, column)
}

fn fully_visible_range(map: &[usize], before: &[usize], range: &Range<usize>) -> bool {
    let start = map.partition_point(|&byte| byte < range.start);
    let end = map.partition_point(|&byte| byte < range.end);
    start < end
        && map.get(start).copied() == Some(range.start)
        && before.get(end).copied() == Some(range.end)
        && (start..end).all(|index| before.get(index + 1) == map.get(index + 1))
}

fn hit_label(source: &str, range: &Range<usize>, path: Option<&Path>) -> String {
    let (line, column) = line_column(source, range.start);
    let line_start = source[..range.start]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line_end = source[range.start..]
        .find('\n')
        .map_or(source.len(), |index| range.start + index);
    let snippet = source[line_start..line_end]
        .trim_end_matches('\r')
        .chars()
        .take(96)
        .collect::<String>();
    match path {
        Some(path) => format!("{}:{line}:{column}  {snippet}", path.display()),
        None => format!("{line}:{column}  {snippet}"),
    }
}

fn open_document_paths(app: &App) -> Vec<PathBuf> {
    app.tabs
        .iter()
        .filter_map(|tab| tab.file.as_ref().map(|file| file.path().to_path_buf()))
        .collect()
}

fn workspace_matches(app: &App, expected: &Path) -> bool {
    let Some(current) = app.persistence.workspace_root() else {
        return false;
    };
    let current = fs::canonicalize(&current).unwrap_or(current);
    let expected = fs::canonicalize(expected).unwrap_or_else(|_| expected.to_path_buf());
    current == expected
}

fn open_edits_from_plan(plan: &ReplacePlan) -> Vec<OpenEdit> {
    plan.open_edits()
        .iter()
        .map(|edit| OpenEdit {
            path: edit.path.clone(),
            document_id: edit.document_id,
            revision: edit.revision,
            generation: edit.generation,
            expected_source: Arc::clone(&edit.expected_source),
            after_source: edit.after_source.clone(),
            selection_after: edit.selection_after,
            replacement_count: edit.replacement_count,
        })
        .collect()
}

fn workspace_preview_message(plan: &ReplacePlan) -> String {
    let mut message = format!(
        "Workspace: {}\nClosed files: {}\nOpen documents: {}\nReplacements: {}\nOutput: {} bytes\n\n",
        plan.root().display(),
        plan.closed_edits().len(),
        plan.open_edits().len(),
        plan.replacement_count(),
        plan.output_bytes(),
    );
    for edit in plan.closed_edits().iter().take(8) {
        let path = edit
            .path
            .strip_prefix(plan.root())
            .unwrap_or(&edit.path)
            .display();
        message.push_str(&format!(
            "closed  {path}  ({} replacements)\n",
            edit.replacement_count
        ));
    }
    for edit in plan.open_edits().iter().take(8) {
        let path = edit
            .path
            .strip_prefix(plan.root())
            .unwrap_or(&edit.path)
            .display();
        message.push_str(&format!(
            "open    {path}  ({} replacements)\n",
            edit.replacement_count
        ));
    }
    let target_count = plan.closed_edits().len() + plan.open_edits().len();
    let shown = plan.closed_edits().len().min(8) + plan.open_edits().len().min(8);
    if target_count > shown {
        message.push_str(&format!(
            "… and {} more target files\n",
            target_count - shown
        ));
    }
    message.push_str("\nClosed files are journaled and rechecked before saving. Apply?");
    message
}

fn open_edit_matches(app: &App, edit: &OpenEdit, generation: u64) -> bool {
    edit.generation == generation
        && app.tabs.iter().any(|tab| {
            tab.doc.id() == edit.document_id
                && tab.doc.revision() == edit.revision
                && tab.doc.source() == edit.expected_source.as_ref()
                && tab
                    .file
                    .as_ref()
                    .is_some_and(|file| super::same_file_identity(file.path(), &edit.path))
        })
}

fn confirm(owner: HWND, title: &str, message: &str) -> bool {
    unsafe {
        let title = wide(title);
        let message = wide(message);
        MessageBoxW(
            Some(owner),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        ) == IDYES
    }
}

impl App {
    pub(super) fn search_status_for_diagnostics(&self) -> String {
        self.search_ui.diagnostics_summary()
    }

    pub(super) unsafe fn search_init(&mut self) {
        unsafe {
            if self.search_ui.initialized || !self.shell.enabled {
                return;
            }
            let owner = self.shell.main_window;
            let mut pane = std::mem::take(&mut self.search_ui);
            pane.create_controls(owner);
            pane.position_controls(self);
            self.search_ui = pane;
        }
    }

    pub(super) fn search_sync(&mut self) {
        let pane = std::mem::take(&mut self.search_ui);
        pane.position_controls(self);
        self.search_ui = pane;
    }

    pub(super) unsafe fn search_highlight_view(
        &self,
        target: &ID2D1HwndRenderTarget,
        view: &TextView,
        x: f32,
        y: f32,
    ) -> windows::core::Result<()> {
        unsafe { self.search_ui.highlight_view(self, target, view, x, y) }
    }

    pub(super) fn search_reveal(&self) -> Option<Range<usize>> {
        self.search_ui.reveal(self)
    }

    pub(super) fn search_control_composing(&self) -> bool {
        self.search_ui.is_composing()
    }

    pub(super) fn search_busy(&self) -> bool {
        self.search_ui.busy()
    }

    pub(super) fn search_tick(&mut self, now: Instant) -> bool {
        let mut pane = std::mem::take(&mut self.search_ui);
        let changed = pane.tick(self, now);
        self.search_ui = pane;
        changed
    }

    pub(super) unsafe fn search_control(&mut self, id: usize, notification: u16) -> bool {
        unsafe {
            let mut pane = std::mem::take(&mut self.search_ui);
            let handled = pane.control(self, id, notification);
            self.search_ui = pane;
            handled
        }
    }

    pub(super) fn search_message(&mut self, message: &MSG) -> bool {
        let mut pane = std::mem::take(&mut self.search_ui);
        let handled = pane.message(self, message);
        self.search_ui = pane;
        handled
    }

    pub(super) fn search_command(&mut self, id: usize) -> bool {
        if !COMMANDS.iter().any(|(command, _, _)| *command == id) {
            return false;
        }
        let mut pane = std::mem::take(&mut self.search_ui);
        pane.command(self, id);
        self.search_ui = pane;
        true
    }

    pub(super) fn search_enabled(&self, id: usize) -> bool {
        self.search_ui.enabled(self, id)
    }

    pub(super) fn search_disabled_reason(&self, id: usize) -> &'static str {
        self.search_ui.disabled_reason(self, id)
    }

    pub(super) fn search_click(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        let mut pane = std::mem::take(&mut self.search_ui);
        let handled = pane.click(self, panel, x, y);
        self.search_ui = pane;
        handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_glob_lines_remain_empty_and_nonempty_patterns_are_preserved() {
        assert!(glob_lines("\r\n  \n\t\r\n").is_empty());
        assert_eq!(
            glob_lines("**/*.md\r\n  docs/*.txt \r\n"),
            vec!["**/*.md".to_owned(), "  docs/*.txt ".to_owned()]
        );
    }

    #[test]
    fn line_column_counts_unicode_scalars_and_crlf() {
        let source = "a😀\r\n猫x";
        assert_eq!(line_column(source, "a😀".len()), (1, 3));
        assert_eq!(line_column(source, source.len()), (2, 3));
    }

    #[test]
    fn table_source_is_revealed_when_a_hit_crosses_a_hidden_delimiter() {
        let map = [4, 5, 9, 10];
        let before = [4, 5, 5, 10];
        assert!(fully_visible_range(&map, &before, &(4..5)));
        assert!(!fully_visible_range(&map, &before, &(5..10)));
        assert!(!fully_visible_range(&map, &before, &(6..9)));
    }

    #[test]
    fn query_edit_keeps_long_literal_text_when_regex_validation_rejects_it() {
        let owner = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                240,
                160,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let payload = "x".repeat(5000);
        let mut pane = SearchPane::default();
        unsafe {
            pane.create_controls(owner);
            let encoded = wide(&payload);
            SetWindowTextW(pane.controls.query, PCWSTR(encoded.as_ptr())).unwrap();
            let control_text = shell::text_of(pane.controls.query);
            assert_eq!(control_text, payload);
            let literal = build_query(
                control_text.clone(),
                checked(pane.controls.case_sensitive),
                checked(pane.controls.regex),
                checked(pane.controls.word),
            )
            .unwrap();
            assert_eq!(literal.text, payload);
            shell::send(pane.controls.regex, BM_SETCHECK, WPARAM(1), LPARAM(0));
            assert!(checked(pane.controls.regex));
            let regex_error = build_query(
                control_text,
                checked(pane.controls.case_sensitive),
                checked(pane.controls.regex),
                checked(pane.controls.word),
            )
            .unwrap_err();
            assert!(regex_error.contains("4096"));
            assert_eq!(shell::text_of(pane.controls.query), payload);
        }
        drop(pane);
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }
    #[test]
    fn document_query_refreshes_on_edit_and_tab_switch_without_query_change() {
        let owner = super::super::tests::hidden_test_window_size(600, 400);
        let mut app = super::super::tests::composing_test_app("first needle");
        app.composing = false;
        let mut pane = SearchPane::default();
        pane.scope = Scope::Document;
        unsafe {
            pane.create_controls(owner);
            SetWindowTextW(pane.controls.query, w!("needle")).unwrap();
        }
        let now = Instant::now();
        assert!(pane.reconcile_inputs(&app, now));
        let first = pane.generation;
        assert!(!pane.reconcile_inputs(&app, now));
        app.doc_mut().replace_range(0..0, "edit ").unwrap();
        assert!(pane.reconcile_inputs(&app, now));
        assert!(pane.generation > first);
        let edited = pane.generation;
        app.new_document();
        app.doc_mut().replace_range(0..0, "second needle").unwrap();
        assert!(pane.reconcile_inputs(&app, now));
        assert!(pane.generation > edited);
        let switched = pane.generation;
        app.active = 0;
        assert!(pane.reconcile_inputs(&app, now));
        assert!(pane.generation > switched);
        let body = &pane.pending.as_ref().unwrap().request.body;
        assert!(
            matches!(body,SearchBody::Document{document_id,revision,source,..} if *document_id==app.doc().id()&&*revision==app.doc().revision()&&source.as_ref()==app.doc().source())
        );
        drop(pane);
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }
    #[test]
    fn search_pane_does_not_restore_removed_f3_binding() {
        let owner = super::super::tests::hidden_test_window_size(600, 400);
        let mut app = super::super::tests::composing_test_app("needle");
        app.composing = false;
        app.hwnd = owner;
        app.set_search_pane_active(true);
        app.shell.enabled = true;
        let mut pane = SearchPane::default();
        unsafe {
            pane.create_controls(owner);
        }
        let mut message = MSG {
            hwnd: pane.controls.query,
            message: WM_KEYDOWN,
            wParam: WPARAM(0x72),
            ..Default::default()
        };
        app.shell
            .settings
            .keybindings
            .insert("edit.findNext".into(), "Ctrl+J".into());
        assert!(!app.shell_message(&message));
        assert!(!pane.message(&app, &message));
        app.shell.settings.keybindings.remove("edit.findNext");
        assert!(!app.shell_message(&message));
        assert!(!pane.message(&app, &message));
        app.shell
            .settings
            .keybindings
            .insert("edit.findNext".into(), "F3".into());
        assert!(app.shell_message(&message));
        message.wParam = WPARAM(0x71);
        assert!(!app.shell_message(&message));
        drop(pane);
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }
}
