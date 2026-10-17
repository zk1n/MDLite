//! Explicit Git and storage entry points. No worker starts from workspace synchronization.
use super::*;
use mdlite::{
    git::{
        self, ConflictChoice, ExecutionGuard, FileStatus, GitClient, GitDiff, GitError,
        GitOperation, GitState, GitStatus, RequestIdentity,
    },
    panel::{PanelId, Rect},
    process::{Cancellation, ProcessOutcome, ProcessResult},
    storage_adapter::{
        ApplyFailure, DestinationEdit, PendingUpload, StorageAdapter, UploadConsent, UploadOutcome,
        UploadPlan, UploadRun, UploadTarget,
    },
    trust::{TrustStore, WorkspaceIdentity},
};
use std::{
    fs,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
use windows::Win32::UI::Controls::*;

const REFRESH: usize = 6000;
const TRUST: usize = 6001;
const UNTRUST: usize = 6002;
const CANCEL: usize = 6003;
const DIFF: usize = 6004;
const STAGE: usize = 6005;
const UNSTAGE: usize = 6006;
const COMMIT: usize = 6008;
const CREATE: usize = 6009;
const SWITCH: usize = 6010;
const MERGE: usize = 6011;
const ABORT: usize = 6012;
const FETCH: usize = 6013;
const PULL: usize = 6014;
const PUSH: usize = 6015;
const PREVIOUS: usize = 6016;
const NEXT: usize = 6017;
const CURRENT: usize = 6018;
const INCOMING: usize = 6019;
const BOTH: usize = 6020;
const RESOLVED: usize = 6021;
const OPEN_CONFLICT: usize = 6022;
const UPLOAD: usize = 6023;
const STORAGE_CONFIG: usize = 6024;
const CONFLICTS: usize = 6025;
const GIT_PANE_HOST: usize = 6508;
const SUMMARY: usize = 6500;
const FILES: usize = 6501;
const DIFF_TEXT: usize = 6502;
const INPUT: usize = 6503;
const REMOTE: usize = 6504;
const INPUT_LABEL: usize = 6505;
const EXECUTE: usize = 6506;
const INPUT_CANCEL: usize = 6507;
pub(super) const COMMANDS: &[(usize, &str, Option<&str>)] = &[
    (REFRESH, "Git: 状態を更新", None),
    (TRUST, "Workspaceをこの端末で信頼…", None),
    (UNTRUST, "Workspaceの信頼を解除…", None),
    (CANCEL, "Git / uploadを取消", None),
    (DIFF, "Git: 選択ファイルのDiff", None),
    (STAGE, "Git: 選択だけStage…", None),
    (UNSTAGE, "Git: 選択だけUnstage…", None),
    (COMMIT, "Git: indexだけCommit…", None),
    (CREATE, "Git: branch作成・切替…", None),
    (SWITCH, "Git: branch切替…", None),
    (MERGE, "Git: Merge…", None),
    (ABORT, "Git: Mergeを中止…", None),
    (FETCH, "Git: Fetch…", None),
    (PULL, "Git: ff-only Pull…", None),
    (PUSH, "Git: Push…", None),
    (PREVIOUS, "Git: 前の競合", None),
    (NEXT, "Git: 次の競合", None),
    (CURRENT, "Git: Currentを採用…", None),
    (INCOMING, "Git: Incomingを採用…", None),
    (BOTH, "Git: Bothを採用…", None),
    (RESOLVED, "Git: 保存して解決済み一ファイルをStage…", None),
    (OPEN_CONFLICT, "Git: 選択した未解決ファイルを開く", None),
    (CONFLICTS, "Git: 未解決のみ / 全一覧", None),
    (UPLOAD, "画像: storage adapterへupload…", None),
    (STORAGE_CONFIG, "storage.tomlを詳細編集…", None),
];

#[derive(Clone)]
struct SourceStamp {
    id: u64,
    revision: u64,
    source: Arc<str>,
    directory: PathBuf,
}
#[derive(Clone, Copy)]
enum ConflictAction {
    Previous,
    Next,
    Adopt(ConflictChoice),
}
struct ConflictRequest {
    id: u64,
    revision: u64,
    selection: Selection,
    path: PathBuf,
    action: ConflictAction,
}
enum Task {
    Refresh,
    Diff(PathBuf),
    Mutation(GitOperation),
    Conflict(ConflictRequest),
    PrepareUpload {
        stamp: SourceStamp,
        reference: Range<usize>,
        config: PathBuf,
    },
    Upload {
        stamp: SourceStamp,
        plan: Box<UploadPlan>,
    },
    Finalize {
        stamp: SourceStamp,
        pending: Box<PendingUpload>,
    },
}
enum Completed {
    Status(Result<GitStatus, GitError>),
    Diff {
        path: PathBuf,
        result: Result<GitDiff, GitError>,
    },
    Mutation(Result<ProcessResult, GitError>),
    Conflict {
        status: Result<GitStatus, GitError>,
        request: ConflictRequest,
    },
    Prepared {
        stamp: SourceStamp,
        plan: Result<Box<UploadPlan>, String>,
    },
    Uploaded {
        stamp: SourceStamp,
        result: UploadRun,
    },
    Finalized {
        stamp: SourceStamp,
        reference: String,
        edit: Result<DestinationEdit, ApplyFailure>,
    },
}
struct Completion {
    identity: RequestIdentity,
    result: Completed,
}
struct Controls {
    host: HWND,
    summary: HWND,
    files: HWND,
    diff: HWND,
    input: HWND,
    remote: HWND,
    input_label: HWND,
    execute: HWND,
    input_cancel: HWND,
    buttons: Vec<(usize, HWND)>,
    ime: Box<Cell<bool>>,
}
pub(super) struct GitPane {
    workspace: Option<WorkspaceIdentity>,
    generation: u64,
    trust: Option<TrustStore>,
    trusted: bool,
    git: PathBuf,
    worker: Option<Receiver<Completion>>,
    mutating: bool,
    cancellation: Option<Cancellation>,
    controls: Option<Controls>,
    status: Option<GitStatus>,
    selected: Vec<PathBuf>,
    visible: Vec<PathBuf>,
    conflicts_only: bool,
    notice: String,
    diff: String,
    list_dirty: bool,
    diff_dirty: bool,
    rendered_summary: String,
    prompt: Option<usize>,
    restore_focus: Option<HWND>,
    /// Kept across workspace changes: a remote side effect cannot be silently forgotten.
    receipt: Option<String>,
}
impl Default for GitPane {
    fn default() -> Self {
        Self {
            workspace: None,
            generation: 0,
            trust: TrustStore::local().ok(),
            trusted: false,
            git: find_git(),
            worker: None,
            mutating: false,
            cancellation: None,
            controls: None,
            status: None,
            selected: vec![],
            visible: vec![],
            conflicts_only: false,
            notice: "Workspaceを開くだけではGitを起動しません。明示的に更新してください。".into(),
            diff: String::new(),
            list_dirty: true,
            diff_dirty: true,
            rendered_summary: String::new(),
            prompt: None,
            restore_focus: None,
            receipt: None,
        }
    }
}
impl Drop for GitPane {
    fn drop(&mut self) {
        if let Some(cancel) = &self.cancellation {
            cancel.cancel();
        }
    }
}
impl GitPane {
    pub(super) unsafe fn owner_draw_button(
        &self,
        owner: HWND,
        message_id: usize,
        item: &DRAWITEMSTRUCT,
    ) -> Option<shell::OwnerDrawState> {
        let controls = self.controls.as_ref()?;
        let id = item.CtlID as usize;
        if owner == HWND::default() || item.CtlType != ODT_BUTTON || message_id != id {
            return None;
        }
        let button = controls
            .buttons
            .iter()
            .find(|(control_id, hwnd)| *control_id == id && *hwnd == item.hwndItem)
            .map(|(_, hwnd)| *hwnd)
            .or_else(|| {
                (id == EXECUTE && item.hwndItem == controls.execute).then_some(controls.execute)
            })
            .or_else(|| {
                (id == INPUT_CANCEL && item.hwndItem == controls.input_cancel)
                    .then_some(controls.input_cancel)
            })?;
        if button == HWND::default()
            || unsafe { GetParent(controls.host).unwrap_or_default() } != owner
            || unsafe { GetDlgCtrlID(controls.host) as usize } != GIT_PANE_HOST
            || unsafe { GetParent(button).unwrap_or_default() } != controls.host
            || unsafe { GetDlgCtrlID(button) as usize } != id
        {
            return None;
        }
        Some(unsafe { shell::owner_draw_state(shell::text_of(button), item) })
    }
}
fn find_git() -> PathBuf {
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|p| p.join("git.exe"))
                .find(|p| p.is_file())
        })
        .and_then(|p| fs::canonicalize(p).ok())
        .unwrap_or_default()
}
fn process_notice(result: &ProcessResult) -> String {
    match &result.outcome { ProcessOutcome::Exited(0)=>"Git操作が完了しました。本文とディスク指紋は自動再読込みしません。".into(),ProcessOutcome::Cancelled=>"取消しました。index / worktree / remoteの副作用はrollbackされません。状態を再確認します。".into(),ProcessOutcome::TimedOut=>"timeoutです。副作用はrollbackされません。状態を再確認します。".into(),other=>format!("Git失敗: {other:?}\n{}",String::from_utf8_lossy(&result.stderr).chars().take(1800).collect::<String>()) }
}
fn git_error(error: GitError) -> String {
    match error {
        GitError::NoWorkspace => "Workspaceがありません。通常編集は続けられます。".into(),
        GitError::Untrusted(e) => format!("Untrusted: {e}"),
        GitError::Invalid(e) => e,
        GitError::IncompleteOutput => {
            "Git一覧は不完全です。部分的な出力は正常一覧として反映しません。".into()
        }
        GitError::Process(p) => process_notice(&p),
    }
}
fn worker(
    identity: RequestIdentity,
    task: Task,
    trust: TrustStore,
    git: PathBuf,
    cancel: Cancellation,
) -> Completion {
    let root = identity.workspace.canonical.clone();
    let generation = identity.generation;
    let client = GitClient {
        executable: git,
        workspace: root.clone(),
        trust: trust.clone(),
    };
    let result = match task {
        Task::Refresh => Completed::Status(
            trust
                .token(&root)
                .map_err(GitError::Untrusted)
                .and_then(|token| client.refresh(&token, &cancel, generation)),
        ),
        Task::Diff(path) => {
            let result = trust
                .token(&root)
                .map_err(GitError::Untrusted)
                .and_then(|token| client.diff(&token, &path, &cancel, generation));
            Completed::Diff { path, result }
        }
        Task::Mutation(operation) => Completed::Mutation(
            trust
                .token(&root)
                .map_err(GitError::Untrusted)
                .and_then(|token| {
                    client.execute(
                        &token,
                        &operation,
                        ExecutionGuard {
                            confirmed: true,
                            presave_completed: true,
                        },
                        &cancel,
                        generation,
                    )
                }),
        ),
        Task::Conflict(request) => Completed::Conflict {
            status: trust
                .token(&root)
                .map_err(GitError::Untrusted)
                .and_then(|token| client.refresh(&token, &cancel, generation)),
            request,
        },
        Task::PrepareUpload {
            stamp,
            reference,
            config,
        } => {
            let plan = StorageAdapter::load(&config).and_then(|adapter| {
                UploadTarget::capture_snapshot(
                    stamp.id,
                    stamp.revision,
                    &stamp.source,
                    reference,
                    &stamp.directory,
                )
                .and_then(|target| UploadPlan::prepare(adapter, target, &root, &trust, generation))
            });
            Completed::Prepared {
                stamp,
                plan: plan.map(Box::new),
            }
        }
        Task::Upload { stamp, plan } => Completed::Uploaded {
            stamp,
            result: plan.run(&trust, UploadConsent::Confirmed, &cancel),
        },
        Task::Finalize { stamp, pending } => {
            let reference = pending.reference().to_owned();
            let edit = pending.edit_snapshot(
                stamp.id,
                stamp.revision,
                &stamp.source,
                generation,
                &root,
                &stamp.directory,
            );
            Completed::Finalized {
                stamp,
                reference,
                edit,
            }
        }
    };
    Completion { identity, result }
}

impl App {
    pub(super) fn git_job_busy(&self) -> bool {
        self.git_ui.worker.is_some()
    }
    pub(super) fn git_mutation_busy(&self) -> bool {
        self.git_ui.worker.is_some() && self.git_ui.mutating
    }
    pub(super) fn git_control_composing(&self) -> bool {
        self.git_ui.controls.as_ref().is_some_and(|c| c.ime.get())
    }
    pub(super) fn git_init(&mut self) {
        if self.git_ui.controls.is_some() || self.shell.main_window == HWND::default() {
            return;
        }
        unsafe {
            let main = self.shell.main_window;
            let host = shell::control(
                main,
                w!("STATIC"),
                "Git pane",
                GIT_PANE_HOST,
                WS_CLIPCHILDREN | WS_CLIPSIBLINGS | shell::CONTAINER_STYLE,
            );
            SetWindowLongPtrW(
                host,
                GWL_EXSTYLE,
                GetWindowLongPtrW(host, GWL_EXSTYLE) | WS_EX_CONTROLPARENT.0 as isize,
            );
            let _ = SetWindowSubclass(host, Some(pane_proc), GIT_PANE_HOST, main.0 as usize);
            let parent = host;
            let summary = shell::control(
                parent,
                w!("EDIT"),
                "Git状態と対象scope",
                SUMMARY,
                WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32),
            );
            let files = shell::control(
                parent,
                w!("LISTBOX"),
                "Git changed files",
                FILES,
                WS_BORDER
                    | WS_VSCROLL
                    | WINDOW_STYLE((LBS_NOTIFY | LBS_EXTENDEDSEL | LBS_NOINTEGRALHEIGHT) as u32),
            );
            let diff = shell::control(
                parent,
                w!("EDIT"),
                "選択ファイルのDiff",
                DIFF_TEXT,
                WS_BORDER
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | WINDOW_STYLE(
                        (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
                    ),
            );
            let input_label = shell::control(
                parent,
                w!("STATIC"),
                "Git入力",
                INPUT_LABEL,
                WINDOW_STYLE(0),
            );
            let input = shell::control(
                parent,
                w!("EDIT"),
                "",
                INPUT,
                WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            );
            let remote = shell::control(
                parent,
                w!("COMBOBOX"),
                "既存remote",
                REMOTE,
                WS_VSCROLL | WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            );
            let execute = shell::control(
                parent,
                w!("BUTTON"),
                "確認へ",
                EXECUTE,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            let input_cancel = shell::control(
                parent,
                w!("BUTTON"),
                "入力取消",
                INPUT_CANCEL,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            let buttons = [
                (REFRESH, "更新"),
                (STAGE, "Stage"),
                (UNSTAGE, "Unstage"),
                (CANCEL, "取消"),
            ]
            .into_iter()
            .map(|(id, label)| {
                (
                    id,
                    shell::control(
                        parent,
                        w!("BUTTON"),
                        label,
                        id,
                        WINDOW_STYLE(BS_OWNERDRAW as u32),
                    ),
                )
            })
            .collect();
            shell::send(diff, EM_SETLIMITTEXT, WPARAM(2 * 1024 * 1024), LPARAM(0));
            shell::send(input, EM_SETLIMITTEXT, WPARAM(4096), LPARAM(0));
            let ime = shell::track_control_ime(input);
            self.git_ui.controls = Some(Controls {
                host,
                summary,
                files,
                diff,
                input,
                remote,
                input_label,
                execute,
                input_cancel,
                buttons,
                ime,
            });
        }
        self.git_sync();
    }
    pub(super) fn git_sync(&mut self) {
        let identity = self
            .persistence
            .workspace_root()
            .and_then(|root| WorkspaceIdentity::read(&root).ok());
        if self.git_ui.workspace != identity {
            if let Some(cancel) = &self.git_ui.cancellation {
                cancel.cancel();
            }
            self.git_ui.generation = self.git_ui.generation.wrapping_add(1);
            self.git_ui.workspace = identity;
            self.git_ui.status = None;
            self.git_ui.selected.clear();
            self.git_ui.visible.clear();
            self.git_ui.diff.clear();
            self.git_ui.list_dirty = true;
            self.git_ui.diff_dirty = true;
            self.git_ui.prompt = None;
            self.git_ui.notice =
                "Workspaceが変わりました。Gitは未実行です。明示的に更新してください。".into();
        }
        self.git_ui.trusted = self
            .git_ui
            .workspace
            .as_ref()
            .zip(self.git_ui.trust.as_ref())
            .is_some_and(|(workspace, store)| store.token(&workspace.canonical).is_ok());
        if !self.git_ui.trusted && self.git_ui.status.is_some() {
            self.git_ui.status = None;
            self.git_ui.list_dirty = true;
            self.git_ui.diff.clear();
            self.git_ui.diff_dirty = true;
        }
        let body = self.feature_panel_rect(PanelId::Git);
        if self.git_ui.list_dirty {
            self.git_rebuild_list();
        }
        let summary = self.git_summary();
        let Some(c) = &self.git_ui.controls else {
            return;
        };
        unsafe {
            let show = body.is_some() && self.shell.enabled;
            let prompting = self.git_ui.prompt.is_some();
            let remote_needed = self
                .git_ui
                .prompt
                .is_some_and(|id| [FETCH, PULL, PUSH].contains(&id));
            let all = [
                (c.host, show),
                (c.summary, show),
                (c.files, show && !prompting),
                (c.diff, show && !prompting),
                (c.input, show && prompting),
                (c.remote, show && remote_needed),
                (c.input_label, show && prompting),
                (c.execute, show && prompting),
                (c.input_cancel, show && prompting),
            ];
            for (h, visible) in all {
                let _ = shell::show_child_control(h, visible);
            }
            for (_, h) in &c.buttons {
                let _ = shell::show_child_control(*h, show && self.git_ui.prompt.is_none());
            }
            if !show {
                return;
            }
            let body = body.unwrap();
            position(c.host, body, self.dpi);
            let r = Rect {
                x: 0.,
                y: 0.,
                width: body.width,
                height: body.height,
            };
            let x = r.x + 5.;
            let width = (r.width - 10.).max(10.);
            let y = r.y + 4.;
            position(
                c.summary,
                Rect {
                    x,
                    y,
                    width,
                    height: 64.,
                },
                self.dpi,
            );
            for (i, (id, h)) in c.buttons.iter().enumerate() {
                position(
                    *h,
                    Rect {
                        x: x + (i % 2) as f32 * (width / 2.),
                        y: y + 66. + (i / 2) as f32 * 25.,
                        width: width / 2. - 3.,
                        height: 23.,
                    },
                    self.dpi,
                );
                shell::enable_owner_button(
                    shell::OwnerButtonPaint {
                        recipient: self.shell.main_window,
                        parent: c.host,
                        parent_id: Some(GIT_PANE_HOST),
                        child: *h,
                        id: *id,
                        tokens: self.shell.tokens,
                        label: shell::text_of(*h),
                    },
                    self.git_enabled(*id),
                );
            }
            let content_y = y + if self.git_ui.prompt.is_some() {
                66.
            } else {
                118.
            };
            let remaining = (r.y + r.height - content_y - 4.).max(1.);
            if self.git_ui.prompt.is_some() {
                position(
                    c.input_label,
                    Rect {
                        x,
                        y: content_y,
                        width,
                        height: 30.,
                    },
                    self.dpi,
                );
                position(
                    c.input,
                    Rect {
                        x,
                        y: content_y + 31.,
                        width,
                        height: 25.,
                    },
                    self.dpi,
                );
                position(
                    c.remote,
                    Rect {
                        x,
                        y: content_y + 60.,
                        width,
                        height: 140.,
                    },
                    self.dpi,
                );
                position(
                    c.execute,
                    Rect {
                        x,
                        y: content_y + 91.,
                        width: width / 2. - 3.,
                        height: 25.,
                    },
                    self.dpi,
                );
                position(
                    c.input_cancel,
                    Rect {
                        x: x + width / 2.,
                        y: content_y + 91.,
                        width: width / 2. - 3.,
                        height: 25.,
                    },
                    self.dpi,
                );
            } else {
                let files_height = (remaining * 0.43).max(26.);
                position(
                    c.files,
                    Rect {
                        x,
                        y: content_y,
                        width,
                        height: files_height,
                    },
                    self.dpi,
                );
                position(
                    c.diff,
                    Rect {
                        x,
                        y: content_y + files_height + 4.,
                        width,
                        height: (remaining - files_height - 4.).max(1.),
                    },
                    self.dpi,
                );
            }
            if self.git_ui.rendered_summary != summary {
                let _ = SetWindowTextW(c.summary, PCWSTR(wide(&summary).as_ptr()));
                self.git_ui.rendered_summary = summary;
            }
            if self.git_ui.diff_dirty {
                let _ = SetWindowTextW(c.diff, PCWSTR(wide(&self.git_ui.diff).as_ptr()));
                self.git_ui.diff_dirty = false;
            }
        }
    }
    fn git_summary(&self) -> String {
        let workspace = self
            .git_ui
            .workspace
            .as_ref()
            .map(|i| i.canonical.display().to_string())
            .unwrap_or_else(|| "NoWorkspace".into());
        let state = if self.git_ui.workspace.is_none() {
            "NoWorkspace — 通常編集は続けられます".into()
        } else if self.git_ui.worker.is_some() {
            "Busy".into()
        } else if !self.git_ui.trusted {
            "Untrusted — Git読取り / 外部実行には端末local Trustが必要".into()
        } else if let Some(s) = &self.git_ui.status {
            format!(
                "{:?}  {}{}{}  index:{} / work:{} / conflict:{}",
                s.state,
                s.branch,
                if s.unborn { " (unborn)" } else { "" },
                if s.detached { " (detached)" } else { "" },
                s.files.iter().filter(|f| f.staged()).count(),
                s.files.iter().filter(|f| f.unstaged()).count(),
                s.files.iter().filter(|f| f.conflicted).count()
            )
        } else {
            "未更新 / UntrustedならTrustが必要".into()
        };
        let scope = self
            .git_ui
            .status
            .as_ref()
            .filter(|s| !s.repository_root.as_os_str().is_empty())
            .map(|s| {
                format!(
                    "Repository全体: {}\nWorkspace: {workspace}（外側のファイルも含み得ます）",
                    s.repository_root.display()
                )
            })
            .unwrap_or_else(|| format!("Workspace: {workspace}"));
        format!(
            "{state}\r\n{scope}\r\n{}{}",
            self.git_ui.notice,
            self.git_ui
                .receipt
                .as_ref()
                .map(|r| format!("\r\n最後のremote receipt（自動再uploadしません）: {r}"))
                .unwrap_or_default()
        )
    }
    fn git_rebuild_list(&mut self) {
        self.git_ui.visible = self
            .git_ui
            .status
            .as_ref()
            .map(|s| {
                s.files
                    .iter()
                    .filter(|f| !self.git_ui.conflicts_only || f.conflicted)
                    .map(|f| f.path.clone())
                    .collect()
            })
            .unwrap_or_default();
        self.git_ui
            .selected
            .retain(|p| self.git_ui.visible.contains(p));
        if let Some(c) = &self.git_ui.controls {
            unsafe {
                shell::send(c.files, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
                if let Some(status) = &self.git_ui.status {
                    for (i, path) in self.git_ui.visible.iter().enumerate() {
                        let Some(file) = status.files.iter().find(|f| f.path == *path) else {
                            continue;
                        };
                        let label = format!(
                            "{}{} {}{}",
                            file.index as char,
                            file.worktree as char,
                            if file.conflicted { "競合 " } else { "" },
                            path.display()
                        );
                        shell::send(
                            c.files,
                            LB_ADDSTRING,
                            WPARAM(0),
                            LPARAM(wide(&label).as_ptr() as isize),
                        );
                        if self.git_ui.selected.contains(path) {
                            shell::send(c.files, LB_SETSEL, WPARAM(1), LPARAM(i as isize));
                        }
                    }
                }
            }
        }
        self.git_ui.list_dirty = false;
    }
    pub(super) unsafe fn git_draw(
        &self,
        target: &ID2D1HwndRenderTarget,
    ) -> windows::core::Result<()> {
        unsafe {
            if self.git_ui.controls.is_none()
                && let Some(r) = self.feature_panel_rect(PanelId::Git)
            {
                let brush = target.CreateSolidColorBrush(&self.shell.tokens.muted, None)?;
                let text = self.text_layout("Git: 明示的に更新してください", r.width - 12., 12.)?;
                target.DrawTextLayout(
                    point(r.x + 6., r.y + 5.),
                    &text,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }
    fn git_start(&mut self, task: Task) -> bool {
        if self.search_busy() {
            self.git_ui.notice = "Workspace置換・復元の完了または取消を待ってください".into();
            return false;
        }
        self.git_sync();
        if self.git_ui.worker.is_some() {
            self.git_ui.notice =
                "前のworkerが終了するまで待ってください。取消は利用できます。".into();
            return false;
        }
        let Some(identity) = self.git_ui.workspace.clone() else {
            self.git_ui.notice = "NoWorkspace — 通常編集は続けられます。".into();
            return false;
        };
        let Some(trust) = self.git_ui.trust.clone() else {
            self.git_ui.notice = "Trust registryを使用できません。".into();
            return false;
        };
        if let Err(e) = trust.token(&identity.canonical) {
            self.git_ui.notice = format!("Untrusted: {e}");
            return false;
        }
        let mutating = matches!(&task, Task::Mutation(_));
        let identity = RequestIdentity {
            generation: self.git_ui.generation,
            workspace: identity,
        };
        let cancel = Cancellation::default();
        let worker_cancel = cancel.clone();
        let git = self.git_ui.git.clone();
        let (tx, rx) = mpsc::channel();
        match thread::Builder::new()
            .name("mdlite-git-storage".into())
            .spawn(move || {
                let result = worker(identity, task, trust, git, worker_cancel);
                let _ = tx.send(result);
            }) {
            Ok(_) => {
                if self.git_ui.restore_focus.is_none() {
                    self.git_ui.restore_focus = Some(unsafe { GetFocus() });
                }
                self.git_ui.worker = Some(rx);
                self.git_ui.mutating = mutating;
                self.git_ui.cancellation = Some(cancel);
                self.git_ui.prompt = None;
                self.git_sync();
                if !self.feature_ime_active()
                    && let Some(controls) = &self.git_ui.controls
                {
                    unsafe {
                        let _ = SetFocus(Some(
                            controls
                                .buttons
                                .iter()
                                .find(|(id, _)| *id == CANCEL)
                                .map(|(_, h)| *h)
                                .unwrap_or(controls.files),
                        ));
                    }
                }
                true
            }
            Err(e) => {
                self.git_ui.notice = format!("workerを開始できません: {e}");
                false
            }
        }
    }
    pub(super) fn git_tick(&mut self, _now: Instant) -> bool {
        self.git_sync();
        let result = match self.git_ui.worker.as_ref().map(Receiver::try_recv) {
            Some(Ok(v)) => v,
            Some(Err(TryRecvError::Disconnected)) => {
                let mutation = self.git_ui.mutating;
                self.git_ui.worker = None;
                self.git_ui.mutating = false;
                self.git_ui.cancellation = None;
                self.git_ui.notice =
                    "workerが結果を返さず終了しました。実行済み副作用は不明です。".into();
                if mutation {
                    let _ = self.git_start(Task::Refresh);
                }
                self.git_sync();
                self.git_restore_focus();
                return true;
            }
            _ => return false,
        };
        let was_cancelled = self
            .git_ui
            .cancellation
            .as_ref()
            .is_some_and(Cancellation::is_cancelled);
        self.git_ui.worker = None;
        self.git_ui.mutating = false;
        self.git_ui.cancellation = None;
        let current = self.git_ui.workspace.as_ref().is_some_and(|i| {
            result.identity.is_current(
                self.git_ui.generation,
                &i.canonical,
                &result.identity.workspace.canonical,
            )
        });
        if !current {
            self.git_ui.notice="古いWorkspace / generationの結果を破棄しました。実行済み副作用はrollbackされません。".into();
            if let Completed::Uploaded { result: run, .. } = result.result {
                self.git_keep_upload_receipt(run);
            }
            self.git_sync();
            self.git_restore_focus();
            return true;
        }
        if was_cancelled
            && matches!(
                &result.result,
                Completed::Prepared { .. }
                    | Completed::Finalized { .. }
                    | Completed::Uploaded { .. }
                    | Completed::Conflict { .. }
            )
        {
            match result.result {
                Completed::Uploaded { result, .. } => self.git_keep_upload_receipt(result),
                Completed::Finalized { reference, .. } => self.git_ui.receipt = Some(reference),
                _ => {}
            }
            self.git_ui.notice="取消しました。本文は変更しません。remote成功のreceiptは保持し、既存副作用はrollbackしません。".into();
            self.git_sync();
            self.git_restore_focus();
            return true;
        }
        match result.result {
            Completed::Status(status) => self.git_apply_status(status),
            Completed::Diff { path, result: diff } => {
                if self.git_ui.selected.len() != 1 || self.git_ui.selected.first() != Some(&path) {
                    self.git_ui.diff.clear();
                    self.git_ui.diff_dirty = true;
                    self.git_ui.notice = "選択が変わったため古いDiffを破棄しました。".into();
                } else {
                    match diff {
                        Ok(diff) => {
                            self.git_ui.diff =
                                format!("対象: {}\r\n{}", path.display(), diff_text(diff));
                            self.git_ui.diff_dirty = true;
                            self.git_ui.notice =
                                "選択ファイルのread-only Diffです。本文・indexは変更していません。"
                                    .into();
                        }
                        Err(e) => self.git_ui.notice = git_error(e),
                    }
                }
            }
            Completed::Mutation(operation) => {
                self.git_ui.notice = operation
                    .map(|p| process_notice(&p))
                    .unwrap_or_else(git_error);
                let _ = self.git_start(Task::Refresh);
            }
            Completed::Conflict { status, request } => {
                match status {
                    Ok(status) => {
                        let file = status
                            .files
                            .iter()
                            .find(|f| f.path == request.path && f.conflicted)
                            .cloned();
                        self.git_apply_status(Ok(status));
                        if let Some(file) = file {
                            self.git_apply_conflict(request, file);
                        } else {
                            self.git_ui.notice="Gitはこのファイルを未mergeと認定していません。本文は保持しました。".into();
                        }
                    }
                    Err(e) => self.git_ui.notice = git_error(e),
                }
            }
            Completed::Prepared { stamp, plan } => self.git_confirm_upload(stamp, plan),
            Completed::Uploaded { stamp, result } => self.git_uploaded(stamp, result),
            Completed::Finalized {
                stamp,
                reference,
                edit,
            } => {
                self.git_ui.receipt = Some(reference.clone());
                match edit {
                    Ok(edit) => {
                        if self.feature_ime_active()
                            || !self.git_stamp_current(&stamp)
                            || self.doc().source().get(edit.range.clone())
                                != Some(edit.expected.as_str())
                        {
                            self.git_ui.notice = format!(
                                "remote成功・本文未反映（stale / IME）。自動再uploadしません。\n{reference}"
                            );
                        } else {
                            let selection = selection_after_source_edit(
                                self.doc().selection(),
                                &edit.range,
                                edit.replacement.len(),
                            );
                            match self.doc_mut().replace_range_with_selection(
                                edit.range,
                                &edit.replacement,
                                selection,
                            ) {
                                Ok(()) => {
                                    self.layout_key = None;
                                    unsafe {
                                        self.after_input();
                                    }
                                    self.git_ui.notice = format!(
                                        "uploadとdestinationの1 Undo更新が完了。local assetを保持しました。\n{reference}"
                                    );
                                }
                                Err(e) => {
                                    self.git_ui.notice =
                                        format!("remote成功・本文更新失敗: {e}\n{reference}")
                                }
                            }
                        }
                    }
                    Err(e) => {
                        self.git_ui.notice =
                            format!("remote成功・本文未反映: {:?}\n{}", e.reason, e.reference)
                    }
                }
            }
        }
        self.git_sync();
        self.git_restore_focus();
        true
    }
    fn git_restore_focus(&mut self) {
        if self.git_job_busy() || self.search_busy() {
            return;
        }
        let Some(previous) = self.git_ui.restore_focus.take() else {
            return;
        };
        if self.feature_ime_active() {
            return;
        }
        let cancel = self.git_ui.controls.as_ref().and_then(|c| {
            c.buttons
                .iter()
                .find(|(id, _)| *id == CANCEL)
                .map(|(_, h)| *h)
        });
        unsafe {
            let focus = GetFocus();
            if cancel == Some(focus) || focus == HWND::default() {
                let target = if IsWindow(Some(previous)).as_bool()
                    && IsWindowVisible(previous).as_bool()
                    && IsWindowEnabled(previous).as_bool()
                {
                    previous
                } else {
                    self.editing_window()
                };
                let _ = SetFocus(Some(target));
            }
        }
    }
    fn git_close_prompt(&mut self) {
        self.git_ui.prompt = None;
        self.git_sync();
        let editor = self.editing_window();
        self.select_surface(editor);
        unsafe {
            let _ = SetFocus(Some(editor));
        }
    }
    fn git_apply_status(&mut self, status: Result<GitStatus, GitError>) {
        match status {
            Ok(s) => self.git_ui.status = Some(s),
            Err(e) => {
                self.git_ui.status = None;
                self.git_ui.notice = git_error(e);
            }
        }
        self.git_ui.list_dirty = true;
    }
    fn git_stamp(&self) -> Option<SourceStamp> {
        Some(SourceStamp {
            id: self.doc().id(),
            revision: self.doc().revision(),
            source: self.doc().source_shared(),
            directory: self.image_directory()?,
        })
    }
    fn git_stamp_current(&self, stamp: &SourceStamp) -> bool {
        self.doc().id() == stamp.id
            && self.doc().revision() == stamp.revision
            && self.image_directory().is_some_and(|p| {
                match (fs::canonicalize(p), fs::canonicalize(&stamp.directory)) {
                    (Ok(a), Ok(b)) => a == b,
                    _ => false,
                }
            })
    }
    fn git_keep_upload_receipt(&mut self, result: UploadRun) {
        match result.outcome {
            UploadOutcome::RemoteSucceeded(p) => self.git_ui.receipt = Some(p.reference().into()),
            UploadOutcome::RemoteSucceededButNotApplied(f) => {
                self.git_ui.receipt = Some(f.reference)
            }
            _ => {}
        }
    }
    fn git_confirm_upload(
        &mut self,
        mut stamp: SourceStamp,
        plan: Result<Box<UploadPlan>, String>,
    ) {
        let plan = match plan {
            Ok(p) => p,
            Err(e) => {
                self.git_ui.notice = e;
                return;
            }
        };
        if self.feature_ime_active() || !self.git_stamp_current(&stamp) {
            self.git_ui.notice =
                "upload準備中に文書・保存先が変わりました。実行していません。".into();
            return;
        }
        let details = plan.confirmation();
        let args = details
            .arguments
            .iter()
            .map(|a| format!("{a:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        let prompt = format!(
            "この端末で信頼済みのWorkspace: {}\nlocal asset: {}\nexecutable: {}\nargv:\n{args}\n文書ID {} / revision {}\n{}\n外部adapterは通信する可能性があります。取消・timeoutでもremote副作用はrollbackされません。実行しますか？",
            details.workspace.display(),
            details.asset.display(),
            details.executable.display(),
            details.document_id,
            details.revision,
            if details.reference_becomes_inline {
                "選択した参照形式画像だけをinlineに変換します。共有定義と他の画像は保持します。"
            } else {
                "画像destinationだけを更新します。alt / title / widthとlocal assetを保持します。"
            }
        );
        if self.git_confirm("画像upload", &prompt) {
            // Preparation consumed the source capture. Keep only ownership metadata
            // while a remote process runs; finalization captures current source again.
            stamp.source = Arc::from("");
            let _ = self.git_start(Task::Upload { stamp, plan });
        } else {
            self.git_ui.notice = "uploadを実行しませんでした。".into();
        }
    }
    fn git_uploaded(&mut self, stamp: SourceStamp, result: UploadRun) {
        match result.outcome {
            UploadOutcome::RemoteSucceeded(pending) => {
                self.git_ui.receipt = Some(pending.reference().into());
                if self.feature_ime_active() || !self.git_stamp_current(&stamp) {
                    self.git_ui.notice = format!(
                        "remote成功・本文未反映（文書 / 保存先変更）。自動再uploadしません。\n{}",
                        pending.reference()
                    );
                    return;
                }
                let Some(current) = self.git_stamp() else {
                    return;
                };
                let reference = pending.reference().to_owned();
                if !self.git_start(Task::Finalize {
                    stamp: current,
                    pending,
                }) {
                    self.git_ui.notice = format!(
                        "remote成功・本文未反映（Trust / Workspace変更）。自動再uploadしません。\n{reference}"
                    );
                }
            }
            UploadOutcome::RemoteSucceededButNotApplied(f) => {
                self.git_ui.receipt = Some(f.reference.clone());
                self.git_ui.notice =
                    format!("remote成功・本文未反映: {:?}\n{}", f.reason, f.reference);
            }
            UploadOutcome::Declined => self.git_ui.notice = "uploadは実行しませんでした。".into(),
            UploadOutcome::Rejected(e) => self.git_ui.notice = format!("upload未実行: {e}"),
            UploadOutcome::Cancelled => {
                self.git_ui.notice =
                    "upload取消。local asset / 本文は保持。remote副作用はrollbackされません。"
                        .into()
            }
            UploadOutcome::TimedOut => {
                self.git_ui.notice =
                    "upload timeout。remote副作用は不明です。自動再uploadしません。".into()
            }
            UploadOutcome::InvalidResponse(e) => {
                self.git_ui.notice =
                    format!("adapterの応答を採用できません: {e}。remote副作用は不明です。")
            }
            UploadOutcome::Failed => {
                self.git_ui.notice = format!(
                    "adapter失敗。本文は保持、remote副作用は不明です。 {:?}",
                    result.process.map(|p| p.outcome)
                )
            }
        }
    }
    fn git_confirm(&mut self, title: &str, text: &str) -> bool {
        unsafe {
            self.release_caret();
        }
        unsafe {
            MessageBoxW(
                Some(self.shell.main_window),
                PCWSTR(wide(text).as_ptr()),
                PCWSTR(wide(title).as_ptr()),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
            ) == IDYES
        }
    }
    fn git_presave(&mut self) -> bool {
        if self.feature_ime_active() {
            return false;
        }
        let dirty: Vec<_> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| t.doc.is_dirty())
            .map(|(i, _)| i)
            .collect();
        if dirty.is_empty() {
            return true;
        }
        if !self.git_confirm("Git前の保存","開いている変更済み文書を先に保存します。保存失敗・取消ならGitを実行しません。\nこの保存が完了した後のGit確認で「いいえ」を選んでも、先の保存は残ります。保存しますか？"){return false;}
        let original = self.active;
        let mut ok = true;
        for index in dirty {
            self.switch_tab(index);
            if !unsafe { self.save(false) } {
                ok = false;
                break;
            }
        }
        self.switch_tab(original.min(self.tabs.len().saturating_sub(1)));
        ok
    }
    fn git_operation(&mut self, operation: GitOperation, description: &str) {
        if self.feature_ime_active() || self.git_ui.worker.is_some() {
            self.git_ui.notice = "IME確定 / 既存worker終了を待ってください。".into();
            return;
        }
        if operation.needs_presave()
            && !matches!(&operation, GitOperation::StageResolved(_))
            && !self.git_presave()
        {
            self.git_ui.notice = "事前保存が完了しなかったためGitを実行しません。".into();
            self.git_close_prompt();
            return;
        }
        let root = self
            .git_ui
            .status
            .as_ref()
            .map(|s| s.repository_root.display().to_string())
            .unwrap_or_default();
        let saved_note = if operation.needs_presave() {
            "この確認を「いいえ」にしても先に完了した保存は残ります。\n"
        } else {
            ""
        };
        let confirmed=self.git_confirm("Git確認",&format!("Repository全体: {root}\n{description}\n{saved_note}自動save / stage / commit / pushの連鎖はありません。取消でも副作用はrollbackされません。実行しますか？"));
        if confirmed {
            if !self.git_start(Task::Mutation(operation)) {
                self.git_close_prompt();
            }
        } else {
            self.git_ui.notice =
                "Gitを実行しませんでした。事前に完了した保存は保持しています。".into();
            self.git_close_prompt();
        }
    }
    fn git_prompt(&mut self, id: usize) {
        self.git_ui.prompt = Some(id);
        let label = match id {
            COMMIT => "Commit message（indexだけ）",
            CREATE => "作成して切替えるbranch",
            SWITCH => "既存branch",
            MERGE => "merge元branch",
            FETCH => "既存remoteを選択",
            PULL => "Pull元branch / 既存remote",
            PUSH => "Pushするbranch / 既存remote",
            _ => "Git",
        };
        if let Some(c) = &self.git_ui.controls {
            unsafe {
                let _ = SetWindowTextW(c.input_label, PCWSTR(wide(label).as_ptr()));
                let initial = if [PULL, PUSH].contains(&id) {
                    self.git_ui
                        .status
                        .as_ref()
                        .filter(|s| !s.detached)
                        .map(|s| s.branch.as_str())
                        .unwrap_or("")
                } else {
                    ""
                };
                let _ = SetWindowTextW(c.input, PCWSTR(wide(initial).as_ptr()));
                shell::send(c.remote, CB_RESETCONTENT, WPARAM(0), LPARAM(0));
                if let Some(status) = &self.git_ui.status {
                    for remote in &status.remotes {
                        shell::send(
                            c.remote,
                            CB_ADDSTRING,
                            WPARAM(0),
                            LPARAM(wide(remote).as_ptr() as isize),
                        );
                    }
                }
                shell::send(c.remote, CB_SETCURSEL, WPARAM(0), LPARAM(0));
            }
        }
        self.git_sync();
        if let Some(c) = &self.git_ui.controls {
            unsafe {
                let _ = SetFocus(Some(if id == FETCH { c.remote } else { c.input }));
            }
        }
    }
    fn git_execute_prompt(&mut self) {
        if self.git_ui.worker.is_some() {
            self.git_ui.notice = "既存workerが終了するまで入力を保持します。".into();
            return;
        }
        let Some(id) = self.git_ui.prompt else {
            return;
        };
        let Some(c) = &self.git_ui.controls else {
            return;
        };
        if c.ime.get() {
            return;
        }
        let input = shell::text_of(c.input);
        let remote = shell::text_of(c.remote);
        if id != FETCH && input.trim().is_empty() {
            self.git_ui.notice = "入力が必要です。".into();
            return;
        }
        let operation = match id {
            COMMIT => GitOperation::Commit {
                message: input.clone(),
            },
            CREATE => GitOperation::CreateBranch(input.clone()),
            SWITCH => GitOperation::SwitchBranch(input.clone()),
            MERGE => GitOperation::Merge(input.clone()),
            FETCH => GitOperation::Fetch {
                remote: remote.clone(),
            },
            PULL => GitOperation::PullFastForward {
                remote: remote.clone(),
                branch: input.clone(),
            },
            PUSH => GitOperation::Push {
                remote: remote.clone(),
                branch: input.clone(),
            },
            _ => return,
        };
        self.git_ui.prompt = None;
        self.git_operation(
            operation,
            &format!(
                "{}\n入力: {input}\nremote: {remote}",
                COMMANDS
                    .iter()
                    .find(|c| c.0 == id)
                    .map(|c| c.1)
                    .unwrap_or("Git")
            ),
        );
    }
    fn git_conflict_file(&self) -> Option<(PathBuf, FileStatus)> {
        let status = self.git_ui.status.as_ref()?;
        let path = self.tabs[self.active].file.as_ref()?.path();
        status
            .files
            .iter()
            .filter(|f| f.conflicted)
            .find(|f| same_file_identity(&status.repository_root.join(&f.path), path))
            .map(|f| (f.path.clone(), f.clone()))
    }
    fn git_request_conflict(&mut self, action: ConflictAction) {
        let Some((path, _)) = self.git_conflict_file() else {
            self.git_ui.notice = "現在の文書はGit未解決一覧にありません。".into();
            return;
        };
        if matches!(action,ConflictAction::Adopt(_))&&!self.git_confirm("競合を一block採用","現在の競合blockだけを1 Undoで置き換えます。保存・stageは別操作です。実行しますか？"){return;}
        let request = ConflictRequest {
            id: self.doc().id(),
            revision: self.doc().revision(),
            selection: self.doc().selection(),
            path,
            action,
        };
        let _ = self.git_start(Task::Conflict(request));
    }
    fn git_apply_conflict(&mut self, request: ConflictRequest, file: FileStatus) {
        if self.feature_ime_active()
            || self.doc().id() != request.id
            || self.doc().revision() != request.revision
            || self.doc().selection() != request.selection
        {
            self.git_ui.notice =
                "競合の確認中に文書・選択が変わりました。本文は保持しました。".into();
            return;
        }
        let blocks = git::conflict_blocks(self.doc().source());
        if blocks.is_empty() {
            self.git_ui.notice = "Git未mergeですが、有効な競合marker blockはありません。".into();
            return;
        }
        match request.action {
            ConflictAction::Previous | ConflictAction::Next => {
                let previous = matches!(request.action, ConflictAction::Previous);
                if let Some(i) =
                    git::adjacent_conflict(self.doc().source(), request.selection.head, previous)
                {
                    let block = &blocks[i];
                    let _ = self
                        .doc_mut()
                        .set_selection(block.range.start, block.range.start);
                    self.layout_key = None;
                    unsafe {
                        self.after_input();
                    }
                }
            }
            ConflictAction::Adopt(choice) => {
                let index = blocks
                    .iter()
                    .position(|b| b.range.contains(&request.selection.head))
                    .unwrap_or_else(|| {
                        git::adjacent_conflict(self.doc().source(), request.selection.head, false)
                            .unwrap_or(0)
                    });
                match git::conflict_edit(self.doc().source(), &file, index, choice) {
                    Ok(edit) => {
                        let selection = selection_after_source_edit(
                            self.doc().selection(),
                            &edit.range,
                            edit.replacement.len(),
                        );
                        match self.doc_mut().replace_range_with_selection(
                            edit.range,
                            &edit.replacement,
                            selection,
                        ) {
                            Ok(()) => {
                                self.layout_key = None;
                                unsafe {
                                    self.after_input();
                                }
                                self.git_ui.notice="一blockを1 Undoで採用しました。保存・stageはまだ行っていません。".into();
                            }
                            Err(e) => self.git_ui.notice = e.to_string(),
                        }
                    }
                    Err(e) => self.git_ui.notice = git_error(e),
                }
            }
        }
    }
    pub(super) fn git_command(&mut self, id: usize) -> bool {
        if !COMMANDS.iter().any(|c| c.0 == id) {
            return false;
        }
        self.show_panel(PanelId::Git);
        self.git_sync();
        if !self.git_enabled(id) {
            self.git_ui.notice =
                "この状態では実行できません（Busy / Trust / 選択 / IMEを確認）。".into();
            self.git_sync();
            return true;
        }
        match id {
            REFRESH => {
                let _ = self.git_start(Task::Refresh);
            }
            TRUST | UNTRUST => {
                let Some(root) = self.git_ui.workspace.as_ref().map(|i| i.canonical.clone()) else {
                    return true;
                };
                let grant = id == TRUST;
                if self.git_confirm("端末local Trust",&format!("Workspace: {}\n{}\nTrustはOS sandboxではありません。Workspaceファイルから自己信頼できません。変更しますか？",root.display(),if grant{"Gitの読取りと外部adapterの明示実行を許可します。"}else{"この端末での外部実行許可を解除します。"})){self.git_ui.notice=match self.git_ui.trust.as_ref().ok_or("Trust registry unavailable".into()).and_then(|s|s.set_trusted(&root,grant)){Ok(())=>"Trustを変更しました。Gitは自動実行しません。".into(),Err(e)=>e};if !grant{self.git_ui.status=None;self.git_ui.list_dirty=true;self.git_ui.diff.clear();self.git_ui.diff_dirty=true;}}
            }
            CANCEL => {
                if let Some(c) = &self.git_ui.cancellation {
                    c.cancel();
                    self.git_ui.notice =
                        "取消を要求しました。副作用のrollbackは保証しません。".into();
                }
            }
            DIFF => {
                self.git_ui.diff.clear();
                self.git_ui.diff_dirty = true;
                if let Some(path) = self.git_ui.selected.first().cloned() {
                    let _ = self.git_start(Task::Diff(path));
                }
            }
            STAGE | UNSTAGE => {
                let paths = self.git_ui.selected.clone();
                let description = format!(
                    "選択した{}ファイルだけ{}:\n{}",
                    paths.len(),
                    if id == UNSTAGE { "Unstage" } else { "Stage" },
                    paths
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                let operation = if id == UNSTAGE {
                    GitOperation::Unstage(paths)
                } else {
                    GitOperation::Stage(paths)
                };
                self.git_operation(operation, &description);
            }
            COMMIT | CREATE | SWITCH | MERGE | FETCH | PULL | PUSH => self.git_prompt(id),
            ABORT => self.git_operation(GitOperation::AbortMerge, "現在のmergeをabortします。"),
            PREVIOUS => self.git_request_conflict(ConflictAction::Previous),
            NEXT => self.git_request_conflict(ConflictAction::Next),
            CURRENT => self.git_request_conflict(ConflictAction::Adopt(ConflictChoice::Current)),
            INCOMING => self.git_request_conflict(ConflictAction::Adopt(ConflictChoice::Incoming)),
            BOTH => self.git_request_conflict(ConflictAction::Adopt(ConflictChoice::Both)),
            RESOLVED => {
                if let Some((path, _)) = self.git_conflict_file() {
                    if !git::conflict_blocks(self.doc().source()).is_empty() {
                        self.git_ui.notice = "未解決markerが残っています。Stageしません。".into();
                    } else if unsafe { self.save(false) } {
                        self.git_operation(
                            GitOperation::StageResolved(path.clone()),
                            &format!("保存済みの一ファイルだけ解決済みStage: {}", path.display()),
                        );
                    } else {
                        self.git_ui.notice = "保存失敗 / 取消のためStageしません。".into();
                    }
                }
            }
            OPEN_CONFLICT => {
                if let Some(status) = &self.git_ui.status
                    && let Some(path) = self.git_ui.selected.first()
                    && status.files.iter().any(|f| f.path == *path && f.conflicted)
                {
                    let target = status.repository_root.join(path);
                    self.open_path(target);
                }
            }
            CONFLICTS => {
                self.git_ui.conflicts_only = !self.git_ui.conflicts_only;
                self.git_ui.selected.clear();
                self.git_ui.list_dirty = true;
            }
            STORAGE_CONFIG => self.git_open_storage_config(),
            UPLOAD => self.git_prepare_upload(),
            _ => {}
        }
        self.git_sync();
        true
    }
    pub(super) fn git_branch_label(&self) -> String {
        let Some(status) = &self.git_ui.status else {
            return "未確認".into();
        };
        match status.state {
            GitState::NoGit => "Gitなし".into(),
            GitState::NoRepository => "repoなし".into(),
            _ => {
                if status.detached {
                    "detached".into()
                } else if status.unborn {
                    format!("{} (unborn)", status.branch)
                } else {
                    status.branch.clone()
                }
            }
        }
    }
    pub(super) fn git_enabled(&self, id: usize) -> bool {
        if !COMMANDS.iter().any(|c| c.0 == id) {
            return false;
        }
        if id == CANCEL {
            return self.git_ui.worker.is_some();
        }
        if self.git_ui.worker.is_some() || self.search_busy() || self.feature_ime_active() {
            return false;
        }
        if self.git_ui.prompt.is_some() {
            return false;
        }
        let workspace = self.git_ui.workspace.is_some();
        if [TRUST, UNTRUST, STORAGE_CONFIG].contains(&id) {
            return workspace
                && (id == STORAGE_CONFIG
                    || (self.git_ui.trust.is_some()
                        && ((id == TRUST && !self.git_ui.trusted)
                            || (id == UNTRUST && self.git_ui.trusted))));
        }
        if !workspace {
            return false;
        }
        if !self.git_ui.trusted {
            return false;
        }
        if id == UPLOAD {
            return self.image_directory().is_some() && self.git_image_selected();
        }
        if id == REFRESH {
            return true;
        }
        let Some(status) = &self.git_ui.status else {
            return false;
        };
        if matches!(status.state, GitState::NoGit | GitState::NoRepository) {
            return false;
        }
        match id {
            DIFF => self.git_ui.selected.len() == 1,
            STAGE | UNSTAGE => !self.git_ui.selected.is_empty(),
            COMMIT => {
                status.files.iter().any(FileStatus::staged)
                    && !status.files.iter().any(|f| f.conflicted)
            }
            FETCH | PULL | PUSH => !status.remotes.is_empty(),
            PREVIOUS | NEXT | CURRENT | INCOMING | BOTH | RESOLVED => {
                self.git_conflict_file().is_some()
            }
            OPEN_CONFLICT => {
                self.git_ui.selected.len() == 1
                    && status
                        .files
                        .iter()
                        .any(|f| f.conflicted && self.git_ui.selected.contains(&f.path))
            }
            _ => true,
        }
    }
    fn git_image_selected(&self) -> bool {
        let Some((id, revision, projection)) = &self.projection_cache else {
            // Source mode can defer semantic selection validation to explicit
            // preparation; it must not require a live-render cache to upload.
            return self.tabs[self.active].raw;
        };
        if *id != self.doc().id() || *revision != self.doc().revision() {
            return self.tabs[self.active].raw;
        }
        let selection = self.doc().selection();
        let mut matches = projection.blocks.iter().filter(|b| {
            matches!(b.kind, BlockKind::Image { .. })
                && (b.source.contains(&selection.head)
                    || (!selection.is_caret()
                        && b.source.start < selection.range().end
                        && b.source.end > selection.range().start))
        });
        let Some(image) = matches.next() else {
            return false;
        };
        if matches.next().is_some() {
            return false;
        }
        let BlockKind::Image { destination, .. } = &image.kind else {
            return false;
        };
        self.image_directory().is_some_and(|directory| {
            mdlite::assets::local_image_path(&directory, destination).is_ok()
        })
    }
    pub(super) fn git_disabled_reason(&self, id: usize) -> &'static str {
        if self.git_enabled(id) {
            return "";
        }
        if id == CANCEL {
            return "実行中のGit / uploadがありません";
        }
        if self.git_ui.workspace.is_none() {
            return "NoWorkspace: Workspaceを開いてください";
        }
        if self.git_job_busy() || self.search_busy() {
            return "Busy: 実行中の処理を完了または取消してください";
        }
        if self.feature_ime_active() {
            return "IMEの入力を確定または取消してください";
        }
        if self.git_ui.prompt.is_some() {
            return "Gitの入力フォームを完了または取消してください";
        }
        if id == TRUST {
            if self.git_ui.trust.is_none() {
                return "端末localのTrust registryを利用できません";
            }
            return "このWorkspaceは既にこの端末で信頼済みです";
        }
        if !self.git_ui.trusted {
            return "Untrusted: この端末でのWorkspace Trustが必要です";
        }
        if id == UPLOAD {
            return "NoImage: 保存済み文書のlocal画像を一つ選択してください";
        }
        let Some(status) = &self.git_ui.status else {
            return "Gitの状態を明示的に更新してください";
        };
        match status.state {
            GitState::NoGit => return "NoGit: git.exeを利用できません",
            GitState::NoRepository => return "NoRepo: Git repositoryではありません",
            _ => {}
        }
        if [FETCH, PULL, PUSH].contains(&id) && status.remotes.is_empty() {
            return "NoRemote: 既存remoteがありません";
        }
        if [DIFF, OPEN_CONFLICT].contains(&id) && self.git_ui.selected.len() != 1 {
            return "ファイルを一つ選択してください";
        }
        if [STAGE, UNSTAGE].contains(&id) && self.git_ui.selected.is_empty() {
            return "NoSelectedFile: 対象ファイルを選択してください";
        }
        if id == COMMIT {
            return if status.files.iter().any(|f| f.conflicted) {
                "未解決のGit競合があります"
            } else {
                "indexにステージ済み変更がありません"
            };
        }
        "現在の文書 / 選択ファイルはGit未解決一覧にありません"
    }
    pub(super) fn git_control(&mut self, id: usize, notification: u16) -> bool {
        match id {
            FILES => {
                if notification == LBN_SELCHANGE as u16 {
                    if let Some(c) = &self.git_ui.controls {
                        unsafe {
                            let count = shell::send(c.files, LB_GETSELCOUNT, WPARAM(0), LPARAM(0))
                                .0
                                .max(0) as usize;
                            let mut indices = vec![0i32; count];
                            shell::send(
                                c.files,
                                LB_GETSELITEMS,
                                WPARAM(count),
                                LPARAM(indices.as_mut_ptr() as isize),
                            );
                            self.git_ui.selected = indices
                                .into_iter()
                                .filter_map(|i| self.git_ui.visible.get(i as usize).cloned())
                                .collect();
                            self.git_ui.diff.clear();
                            self.git_ui.diff_dirty = true;
                            self.git_ui.notice =
                                "選択ファイルのDiffはdouble click / Git Diffで取得できます。"
                                    .into();
                        }
                    }
                } else if notification == LBN_DBLCLK as u16 {
                    let _ = self.git_command(DIFF);
                }
                self.git_sync();
                true
            }
            EXECUTE => {
                if notification == BN_CLICKED as u16 {
                    self.git_execute_prompt();
                    self.git_sync();
                }
                true
            }
            INPUT_CANCEL => {
                self.git_close_prompt();
                true
            }
            SUMMARY | DIFF_TEXT | INPUT | REMOTE | INPUT_LABEL => true,
            _ => false,
        }
    }
    pub(super) fn git_message(&mut self, msg: &MSG) -> bool {
        let Some(c) = &self.git_ui.controls else {
            return false;
        };
        let handles = [
            c.summary,
            c.files,
            c.diff,
            c.input,
            c.remote,
            c.execute,
            c.input_cancel,
        ];
        if !handles.contains(&msg.hwnd) && !c.buttons.iter().any(|(_, h)| *h == msg.hwnd) {
            return false;
        }
        if c.ime.get() {
            return false;
        }
        if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize {
            if self.git_ui.prompt.is_some() {
                self.git_close_prompt();
                unsafe {
                    self.refresh();
                }
            } else if self.git_ui.worker.is_some()
                && let Some(cancel) = &self.git_ui.cancellation
            {
                cancel.cancel();
            }
            return true;
        }
        if msg.message == WM_KEYDOWN
            && msg.wParam.0 == VK_RETURN.0 as usize
            && self.git_ui.prompt.is_some()
            && [c.input, c.remote].contains(&msg.hwnd)
        {
            unsafe {
                let _ = PostMessageW(
                    Some(self.shell.main_window),
                    WM_COMMAND,
                    WPARAM(EXECUTE),
                    LPARAM(c.execute.0 as isize),
                );
            }
            return true;
        }
        if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_TAB.0 as usize {
            unsafe {
                return IsDialogMessageW(self.shell.main_window, msg).as_bool();
            }
        }
        false
    }
    pub(super) fn git_click(&mut self, panel: PanelId, _x: f32, _y: f32) -> bool {
        if panel != PanelId::Git {
            return false;
        }
        if let Some(c) = &self.git_ui.controls {
            unsafe {
                let _ = SetFocus(Some(c.files));
            }
        }
        true
    }
    fn git_open_storage_config(&mut self) {
        let Some(root) = self.persistence.workspace_root() else {
            return;
        };
        match persistence::bounded_path(&root, Path::new(".mdlite/storage.toml")) {
            Ok(path) => {
                if !path.exists() {
                    let result=path.parent().ok_or_else(||std::io::Error::other("Missing config parent")).and_then(fs::create_dir_all).and_then(|()|{use std::io::Write;let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&path)?;file.write_all(b"# Explicit .exe adapter; no execution until Trust and confirmation.\nexecutable = \"\"\nargument = \"{file}\"\nargument = \"{revision}\"\ntimeout_ms = 120000\n")});
                    if let Err(e) = result {
                        self.git_ui.notice = format!("storage.toml作成失敗: {e}");
                        return;
                    }
                }
                self.open_path(path);
            }
            Err(e) => self.git_ui.notice = e.to_string(),
        }
    }
    fn git_prepare_upload(&mut self) {
        let Some(stamp) = self.git_stamp() else {
            self.git_ui.notice = "画像upload前に文書を保存してください。".into();
            return;
        };
        let selection = self.doc().selection();
        let projection = self.cached_projection();
        let images: Vec<_> = projection
            .blocks
            .iter()
            .filter(|b| {
                matches!(b.kind, BlockKind::Image { .. })
                    && (b.source.contains(&selection.head)
                        || (!selection.is_caret()
                            && b.source.start < selection.range().end
                            && b.source.end > selection.range().start))
            })
            .collect();
        if images.len() != 1 {
            self.git_ui.notice = "upload対象のlocal画像を一つ選択してください。".into();
            return;
        }
        let reference = images[0].source.clone();
        let Some(root) = self.persistence.workspace_root() else {
            return;
        };
        match persistence::bounded_path(&root, Path::new(".mdlite/storage.toml")) {
            Ok(config) => {
                let _ = self.git_start(Task::PrepareUpload {
                    stamp,
                    reference,
                    config,
                });
            }
            Err(e) => self.git_ui.notice = e.to_string(),
        }
    }
}
// Children are clipped to this concrete pane host. Notifications still enter the
// existing main-window dispatcher, including its IME queue and owner drawing.
unsafe extern "system" fn pane_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(pane_proc), id);
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        let main = HWND(data as *mut _);
        if id != GIT_PANE_HOST
            || main == HWND::default()
            || GetParent(hwnd).unwrap_or_default() != main
            || GetDlgCtrlID(hwnd) as usize != GIT_PANE_HOST
        {
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        match msg {
            WM_COMMAND if lp.0 != 0 => {
                let child = HWND(lp.0 as *mut _);
                let child_id = wp.0 & 0xffff;
                if git_host_child(hwnd, child, child_id) {
                    return shell::send(main, msg, wp, lp);
                }
            }
            WM_DRAWITEM if lp.0 != 0 => {
                let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                let child_id = item.CtlID as usize;
                if item.CtlType == ODT_BUTTON
                    && wp.0 == child_id
                    && is_git_owner_draw_id(child_id)
                    && git_host_child(hwnd, item.hwndItem, child_id)
                {
                    return shell::send(main, msg, wp, lp);
                }
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC if lp.0 != 0 => {
                return shell::send(main, msg, wp, lp);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
fn is_git_control_id(id: usize) -> bool {
    COMMANDS.iter().any(|(command_id, ..)| *command_id == id)
        || matches!(
            id,
            SUMMARY | FILES | DIFF_TEXT | INPUT | REMOTE | INPUT_LABEL | EXECUTE | INPUT_CANCEL
        )
}
fn is_git_owner_draw_id(id: usize) -> bool {
    matches!(
        id,
        REFRESH | STAGE | UNSTAGE | CANCEL | EXECUTE | INPUT_CANCEL
    )
}
unsafe fn git_host_child(host: HWND, child: HWND, id: usize) -> bool {
    child != HWND::default()
        && is_git_control_id(id)
        && unsafe { GetParent(child).unwrap_or_default() } == host
        && unsafe { GetDlgCtrlID(child) as usize } == id
}
fn position(hwnd: HWND, r: Rect, dpi: f32) {
    unsafe {
        let _ = shell::position_child_control(
            hwnd,
            (r.x * dpi).round() as i32,
            (r.y * dpi).round() as i32,
            (r.width * dpi).round().max(1.) as i32,
            (r.height * dpi).round().max(1.) as i32,
        );
    }
}
fn diff_text(diff: GitDiff) -> String {
    diff.parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            format!(
                "=== {}{} ===\r\n{}\r\n",
                if i == 0 {
                    "index / untracked"
                } else {
                    "worktree"
                },
                if p.truncated { " (truncated)" } else { "" },
                String::from_utf8_lossy(&p.stdout)
            )
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}
fn selection_after_source_edit(
    selection: Selection,
    range: &Range<usize>,
    replacement_len: usize,
) -> Selection {
    let map = |p: usize| {
        if p <= range.start {
            p
        } else if p >= range.end {
            p - (range.end - range.start) + replacement_len
        } else {
            range.start + replacement_len
        }
    };
    Selection {
        anchor: map(selection.anchor),
        head: map(selection.head),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        repo: PathBuf,
        git: PathBuf,
        trust: TrustStore,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "mdlite-native-git-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            let repo = root.join("repo");
            fs::create_dir_all(&repo).unwrap();
            let git = find_git();
            assert!(git.is_file());
            let config = root.join("empty.gitconfig");
            fs::write(&config, "").unwrap();
            for args in [
                vec!["init", "-b", "main"],
                vec!["config", "user.name", "Native fixture"],
                vec!["config", "user.email", "fixture@example.invalid"],
                vec!["config", "core.hooksPath", ".git/no-hooks"],
                vec!["config", "commit.gpgsign", "false"],
                vec!["config", "core.autocrlf", "false"],
            ] {
                let output = std::process::Command::new(&git)
                    .args(args)
                    .current_dir(&repo)
                    .env("GIT_CONFIG_NOSYSTEM", "1")
                    .env("GIT_CONFIG_GLOBAL", &config)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "{}",
                    String::from_utf8_lossy(&output.stderr)
                );
            }
            let trust = TrustStore::at(root.join("registry"));
            trust.set_trusted(&repo, true).unwrap();
            Self {
                root,
                repo,
                git,
                trust,
            }
        }
        fn run(&self, task: Task) -> Completed {
            worker(
                RequestIdentity {
                    generation: 19,
                    workspace: WorkspaceIdentity::read(&self.repo).unwrap(),
                },
                task,
                self.trust.clone(),
                self.git.clone(),
                Cancellation::default(),
            )
            .result
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    struct OwnedWindow(HWND);
    impl Drop for OwnedWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn pump_queued_message(
        fixture: &super::super::tests::MessagePumpFixture,
        target: HWND,
        message_min: u32,
        message_max: u32,
    ) -> bool {
        let mut message = MSG::default();
        assert!(
            unsafe {
                PeekMessageW(
                    &mut message,
                    Some(target),
                    message_min,
                    message_max,
                    PM_REMOVE,
                )
            }
            .as_bool()
        );
        let handled = {
            let mut app = fixture.state.app.borrow_mut();
            unsafe { super::super::drain_native_notifications(&fixture.state, &mut app) };
            app.shell_message(&message)
                || unsafe { app.features_message(&message) }
                || app.git_message(&message)
                || app.search_message(&message)
        };
        if !handled {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        handled
    }
    fn native_caret_owner(hwnd: HWND) -> HWND {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let thread = unsafe { GetWindowThreadProcessId(hwnd, None) };
        unsafe { GetGUIThreadInfo(thread, &mut info).unwrap() };
        info.hwndCaret
    }

    #[test]
    fn git_prompt_escape_restores_caret_for_the_next_queued_character_and_arrow() {
        for compact_surface in [false, true] {
            let mut fixture = super::super::tests::MessagePumpFixture::new(
                super::super::tests::composing_test_app("body"),
            );
            let parking = OwnedWindow(super::super::tests::hidden_test_window_size(160, 80));
            {
                let mut app = fixture.state.app.borrow_mut();
                app.clear_composition();
                app.doc_mut().set_selection(0, 0).unwrap();
                app.shell.enabled = true;
                app.shell.main_window = fixture.hwnd;
                app.shell.panels[PanelId::Git as usize].1 = Rect {
                    x: 760.,
                    y: 350.,
                    width: 260.,
                    height: 340.,
                };
                app.git_init();
            }
            unsafe {
                let _ = SetFocus(Some(parking.0));
                assert_eq!(GetFocus(), parking.0);
                let _ = SetFocus(Some(fixture.hwnd));
                assert_eq!(GetFocus(), fixture.hwnd);
            }
            let app = fixture.state.app.borrow();
            assert_eq!(
                native_caret_owner(fixture.hwnd),
                fixture.hwnd,
                "focus={:?}, input={:?}, parking={:?}, native_caret={:?}/{:?}/{}",
                unsafe { GetFocus() },
                app.git_ui.controls.as_ref().unwrap().input,
                parking.0,
                app.native_caret.size,
                app.native_caret.position,
                app.native_caret.visible,
            );
            drop(app);

            let input;
            let before;
            {
                let mut app = fixture.state.app.borrow_mut();
                app.git_prompt(COMMIT);
                input = app.git_ui.controls.as_ref().unwrap().input;
                assert_eq!(unsafe { GetFocus() }, input);
                before = (
                    app.doc().id(),
                    app.doc().source().to_owned(),
                    app.doc().revision(),
                    app.doc().selection(),
                    app.doc().is_dirty(),
                    app.doc().undo_count(),
                    app.doc().redo_count(),
                );
            }

            if compact_surface {
                let compact = fixture.create_compact();
                {
                    let mut app = fixture.state.app.borrow_mut();
                    app.shell.compact_window = compact;
                }
                assert!(
                    unsafe { PostMessageW(Some(fixture.hwnd), WM_PAINT, WPARAM(0), LPARAM(0)) }
                        .is_ok()
                );
                assert!(!pump_queued_message(
                    &fixture,
                    fixture.hwnd,
                    WM_PAINT,
                    WM_PAINT,
                ));
                let app = fixture.state.app.borrow();
                assert_eq!(app.hwnd, fixture.hwnd);
                assert_eq!(unsafe { GetFocus() }, input);
            }
            assert!(
                unsafe {
                    PostMessageW(
                        Some(input),
                        WM_KEYDOWN,
                        WPARAM(VK_ESCAPE.0 as usize),
                        LPARAM(1),
                    )
                }
                .is_ok()
            );
            assert!(pump_queued_message(&fixture, input, 0, 0));

            let editor = {
                let app = fixture.state.app.borrow();
                let editor = app.editing_window();
                assert_eq!(app.hwnd, editor);
                assert_eq!(unsafe { GetFocus() }, editor);
                assert_eq!(native_caret_owner(editor), editor);
                assert_eq!(
                    (
                        app.doc().id(),
                        app.doc().source().to_owned(),
                        app.doc().revision(),
                        app.doc().selection(),
                        app.doc().is_dirty(),
                        app.doc().undo_count(),
                        app.doc().redo_count(),
                    ),
                    before
                );
                editor
            };

            assert!(
                unsafe { PostMessageW(Some(editor), WM_CHAR, WPARAM('x' as usize), LPARAM(1)) }
                    .is_ok()
            );
            assert!(!pump_queued_message(&fixture, editor, 0, 0));
            assert!(
                unsafe {
                    PostMessageW(
                        Some(editor),
                        WM_KEYDOWN,
                        WPARAM(VK_RIGHT.0 as usize),
                        LPARAM(1),
                    )
                }
                .is_ok()
            );
            assert!(!pump_queued_message(&fixture, editor, 0, 0));

            let app = fixture.state.app.borrow();
            assert_eq!(app.doc().source(), "xbody");
            assert_eq!(app.doc().selection(), Selection::caret(2));
            assert!(app.doc().is_dirty());
            assert_eq!(app.doc().undo_count(), before.5 + 1);
            assert_eq!(app.doc().redo_count(), 0);
            assert_eq!(unsafe { GetFocus() }, editor);
            assert_eq!(native_caret_owner(editor), editor);
        }
    }
    #[test]
    fn native_git_control_visibility_has_no_transient_shows_and_tracks_prompt_and_pane() {
        struct Counts {
            transitions: Cell<usize>,
        }
        unsafe extern "system" fn spy(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                // Hidden descendants need not receive WINDOWPOSCHANGED for a
                // local visibility change. Direct ShowWindow notifications
                // (lParam0) remain distinct from parent show/hide propagation.
                if msg == WM_SHOWWINDOW && lp.0 == 0 {
                    let counts = &*(data as *const Counts);
                    counts.transitions.set(counts.transitions.get() + 1);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        let mut app = super::super::tests::composing_test_app("original😀\n");
        let counts = Box::new(Counts {
            transitions: Cell::new(0),
        });
        // Counts outlive all subclassed children, including failure cleanup.
        let window = OwnedWindow(super::super::tests::hidden_test_window_size(1100, 700));
        let foreground = unsafe { GetForegroundWindow() };
        app.hwnd = window.0;
        app.shell.main_window = window.0;
        app.shell.enabled = true;
        app.shell.panels[PanelId::Git as usize].1 = Rect {
            x: 760.,
            y: 350.,
            width: 260.,
            height: 340.,
        };
        app.clear_composition();
        app.git_init();
        let source = app.doc().source().to_owned();
        let document = (
            app.doc().id(),
            app.doc().revision(),
            app.doc().selection(),
            app.doc().is_dirty(),
            app.doc().undo_count(),
            app.doc().redo_count(),
        );
        let c = app.git_ui.controls.as_ref().unwrap();
        let controls = [
            c.host,
            c.summary,
            c.files,
            c.diff,
            c.input,
            c.remote,
            c.input_label,
            c.execute,
            c.input_cancel,
        ];
        let buttons: Vec<_> = c.buttons.iter().map(|(_, hwnd)| *hwnd).collect();
        for hwnd in controls.iter().chain(&buttons) {
            unsafe {
                let mut pid = 0;
                GetWindowThreadProcessId(*hwnd, Some(&mut pid));
                assert_eq!(pid, std::process::id());
                assert!(IsChild(window.0, *hwnd).as_bool());
                assert!(
                    SetWindowSubclass(
                        *hwnd,
                        Some(spy),
                        0xF19,
                        (&*counts as *const Counts) as usize,
                    )
                    .as_bool()
                );
            }
        }
        for (prompt, pane_visible) in [
            (None, true),
            (Some(COMMIT), true),
            (Some(FETCH), true),
            (Some(PULL), true),
            (Some(PUSH), true),
            (Some(STORAGE_CONFIG), true),
            (Some(CREATE), true),
            (None, true),
            (Some(FETCH), false),
            (None, false),
            (Some(COMMIT), true),
            (None, true),
        ] {
            let prompt_visible = prompt.is_some();
            let remote = prompt.is_some_and(|id| [FETCH, PULL, PUSH].contains(&id));
            // Independent final-state oracle, including the other Git buttons.
            let expected = [
                pane_visible,
                pane_visible,
                pane_visible && !prompt_visible,
                pane_visible && !prompt_visible,
                pane_visible && prompt_visible,
                pane_visible && remote,
                pane_visible && prompt_visible,
                pane_visible && prompt_visible,
                pane_visible && prompt_visible,
            ];
            let expected: Vec<_> = expected
                .into_iter()
                .chain(buttons.iter().map(|_| pane_visible && !prompt_visible))
                .collect();
            let handles: Vec<_> = controls.iter().chain(&buttons).copied().collect();
            let before: Vec<_> = handles
                .iter()
                .map(|h| unsafe { GetWindowLongPtrW(*h, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 })
                .collect();
            let needed = before.iter().zip(&expected).filter(|(a, b)| a != b).count();
            app.git_ui.prompt = prompt;
            app.shell.panels[PanelId::Git as usize].1.width = if pane_visible { 260. } else { 0. };
            counts.transitions.set(0);
            app.git_sync();
            let actual: Vec<_> = handles
                .iter()
                .map(|h| unsafe { GetWindowLongPtrW(*h, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0 })
                .collect();
            assert_eq!(
                actual, expected,
                "final visibility: {prompt:?}, pane={pane_visible}"
            );
            println!(
                "git visibility prompt={prompt:?} pane={pane_visible} actual_transitions={} required={needed}",
                counts.transitions.get()
            );
            assert_eq!(
                counts.transitions.get(),
                needed,
                "transient native visibility changes"
            );
            counts.transitions.set(0);
            app.git_sync();
            assert_eq!(
                counts.transitions.get(),
                0,
                "unchanged native sync must not show/hide"
            );
            assert_eq!(app.doc().source(), source);
            assert_eq!(
                (
                    app.doc().id(),
                    app.doc().revision(),
                    app.doc().selection(),
                    app.doc().is_dirty(),
                    app.doc().undo_count(),
                    app.doc().redo_count()
                ),
                document
            );
            assert_eq!(unsafe { GetForegroundWindow() }, foreground);
            assert!(!app.git_job_busy());
        }
    }
    #[test]
    fn git_owner_draw_requires_registered_child_and_reaches_shell_renderer() {
        let mut app = super::super::tests::composing_test_app("text");
        let window = OwnedWindow(super::super::tests::hidden_test_window_size(1100, 700));
        let other_window = OwnedWindow(super::super::tests::hidden_test_window_size(240, 80));
        app.hwnd = window.0;
        app.shell.main_window = window.0;
        app.shell.enabled = true;
        app.composing = false;
        app.git_init();

        let controls = app.git_ui.controls.as_ref().unwrap();
        let button = controls
            .buttons
            .iter()
            .find(|(id, _)| *id == REFRESH)
            .unwrap()
            .1;
        let dc = unsafe { GetDC(Some(window.0)) };
        assert!(!dc.0.is_null());
        let mut item = DRAWITEMSTRUCT {
            CtlType: ODT_BUTTON,
            CtlID: REFRESH as u32,
            itemID: u32::MAX,
            hwndItem: button,
            hDC: dc,
            rcItem: RECT {
                left: 0,
                top: 0,
                right: 100,
                bottom: 24,
            },
            ..Default::default()
        };
        assert_eq!(
            unsafe { app.git_ui.owner_draw_button(window.0, REFRESH, &item) }
                .unwrap()
                .label,
            "更新"
        );
        assert!(unsafe { app.git_ui.owner_draw_button(other_window.0, REFRESH, &item) }.is_none());
        assert!(unsafe { app.git_ui.owner_draw_button(window.0, STAGE, &item) }.is_none());
        item.CtlType = ODT_LISTBOX;
        assert!(unsafe { app.git_ui.owner_draw_button(window.0, REFRESH, &item) }.is_none());
        item.CtlType = ODT_BUTTON;
        item.hwndItem = other_window.0;
        assert!(unsafe { app.git_ui.owner_draw_button(window.0, REFRESH, &item) }.is_none());
        item.hwndItem = button;
        assert_eq!(
            unsafe {
                app.shell_window_message(
                    window.0,
                    WM_DRAWITEM,
                    WPARAM(REFRESH),
                    LPARAM((&item as *const DRAWITEMSTRUCT) as isize),
                )
            },
            Some(LRESULT(1))
        );
        unsafe {
            ReleaseDC(Some(window.0), dc);
        }
    }
    #[test]
    fn native_git_explicit_worker_fixture_selected_literal_diff_and_untrusted() {
        assert!(!COMMANDS.iter().any(|c| c.0 == 6007));
        let f = Fixture::new();
        fs::write(f.repo.join("[selected].md"), "selected").unwrap();
        fs::write(f.repo.join("s.md"), "must remain untracked").unwrap();
        let Completed::Status(Ok(status)) = f.run(Task::Refresh) else {
            panic!("status fixture failed");
        };
        assert_eq!(status.state, GitState::NoRemote);
        assert!(status.unborn);
        let Completed::Mutation(Ok(result)) = f.run(Task::Mutation(GitOperation::Stage(vec![
            "[selected].md".into(),
        ]))) else {
            panic!("stage fixture failed");
        };
        assert!(result.succeeded());
        let Completed::Status(Ok(status)) = f.run(Task::Refresh) else {
            panic!("post-mutation fixture failed");
        };
        assert_eq!(status.files.iter().filter(|f| f.staged()).count(), 1);
        assert!(
            status
                .files
                .iter()
                .find(|f| f.path == Path::new("s.md"))
                .unwrap()
                .untracked
        );
        let Completed::Diff {
            result: Ok(diff), ..
        } = f.run(Task::Diff("[selected].md".into()))
        else {
            panic!("diff fixture failed");
        };
        assert!(diff_text(diff).contains("selected"));
        f.trust.set_trusted(&f.repo, false).unwrap();
        assert!(matches!(
            f.run(Task::Refresh),
            Completed::Status(Err(GitError::Untrusted(_)))
        ));
    }
    #[test]
    fn native_git_controls_no_open_execution_and_enter_posts_command_preserving_ime() {
        let mut app = super::super::tests::composing_test_app("original😀\n");
        let window = OwnedWindow(super::super::tests::hidden_test_window_size(1100, 700));
        app.hwnd = window.0;
        app.shell.main_window = window.0;
        app.shell.enabled = true;
        app.composing = false;
        app.preedit.clear();
        app.ime_owner = None;
        app.git_init();
        assert!(!app.git_job_busy());
        assert!(!app.git_mutation_busy());
        assert!(app.git_ui.status.is_none());
        assert!(!app.git_enabled(STAGE));
        let before = app.doc().source().to_owned();
        let revision = app.doc().revision();
        app.git_ui.prompt = Some(COMMIT);
        let c = app.git_ui.controls.as_ref().unwrap();
        let input = c.input;
        c.ime.set(true);
        let message = MSG {
            hwnd: input,
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_RETURN.0 as usize),
            ..Default::default()
        };
        assert!(!app.git_message(&message));
        assert!(app.git_control_composing());
        app.git_ui.controls.as_ref().unwrap().ime.set(false);
        assert!(app.git_message(&message));
        let mut posted = MSG::default();
        assert!(
            unsafe {
                PeekMessageW(
                    &mut posted,
                    Some(window.0),
                    WM_COMMAND,
                    WM_COMMAND,
                    PM_REMOVE,
                )
            }
            .as_bool()
        );
        assert_eq!(posted.wParam.0, EXECUTE);
        assert_eq!(app.doc().source(), before);
        assert_eq!(app.doc().revision(), revision);
        let (tx, rx) = mpsc::channel();
        let cancellation = Cancellation::default();
        app.git_ui.worker = Some(rx);
        app.git_ui.prompt = None;
        app.git_ui.mutating = true;
        app.git_ui.cancellation = Some(cancellation.clone());
        assert!(app.git_job_busy());
        assert!(app.git_mutation_busy());
        assert!(app.git_enabled(CANCEL));
        assert!(!app.git_enabled(TRUST));
        assert!(!app.git_enabled(STAGE));
        let escape = MSG {
            hwnd: app.git_ui.controls.as_ref().unwrap().files,
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_ESCAPE.0 as usize),
            ..Default::default()
        };
        assert!(app.git_message(&escape));
        assert!(cancellation.is_cancelled());
        drop(tx);
        assert!(app.git_tick(Instant::now()));
        assert!(!app.git_job_busy());
    }
    #[test]
    fn native_git_conflict_one_undo_and_stale_selection_guard() {
        let source = "<<<<<<< HEAD\r\n現在\r\n||||||| base\r\n祖先\r\n=======\r\nincoming\r\n>>>>>>> topic\r\n後\r\n";
        let mut app = super::super::tests::composing_test_app(source);
        let window = OwnedWindow(super::super::tests::hidden_test_window_size(900, 650));
        app.hwnd = window.0;
        app.composing = false;
        app.preedit.clear();
        app.ime_owner = None;
        let file = FileStatus {
            path: "conflict.md".into(),
            original_path: None,
            index: b'U',
            worktree: b'U',
            untracked: false,
            conflicted: true,
        };
        let selection = Selection::caret(0);
        app.doc_mut().set_selection(0, 0).unwrap();
        let request = ConflictRequest {
            id: app.doc().id(),
            revision: app.doc().revision(),
            selection,
            path: file.path.clone(),
            action: ConflictAction::Adopt(ConflictChoice::Both),
        };
        app.doc_mut().set_selection(1, 1).unwrap();
        app.git_apply_conflict(request, file.clone());
        assert_eq!(app.doc().source(), source);
        app.doc_mut().set_selection(0, 0).unwrap();
        let request = ConflictRequest {
            id: app.doc().id(),
            revision: app.doc().revision(),
            selection,
            path: file.path.clone(),
            action: ConflictAction::Adopt(ConflictChoice::Both),
        };
        app.git_apply_conflict(request, file);
        assert_eq!(app.doc().source(), "現在\r\nincoming\r\n後\r\n");
        assert_eq!(app.doc().undo_count(), 1);
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), source);
    }
    #[test]
    fn native_git_source_edit_preserves_reverse_selection_outside_destination() {
        let source = "A😀 before local.png after 日";
        let start = source.find("local.png").unwrap();
        let range = start..start + "local.png".len();
        let replacement = "https://example.invalid/remote.png";
        let mut doc = Document::new(77, source.into());
        doc.set_selection(source.len(), 0).unwrap();
        let selection = selection_after_source_edit(doc.selection(), &range, replacement.len());
        doc.replace_range_with_selection(range, replacement, selection)
            .unwrap();
        assert_eq!(doc.selection().anchor, doc.source().len());
        assert_eq!(doc.selection().head, 0);
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.undo());
        assert_eq!(doc.source(), source);
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: source.len(),
                head: 0
            }
        );
    }
    #[test]
    fn native_git_storage_worker_snapshot_pipeline_mock_only() {
        let f = Fixture::new();
        let executable = f.root.join("storage-mock.exe");
        let code = f.root.join("storage-mock.rs");
        fs::write(&code,"fn main(){let args:Vec<_>=std::env::args_os().collect();assert!(std::path::Path::new(&args[1]).is_file());assert_eq!(args[2],\"1\");println!(\"https://example.invalid/mock.png?x=1&y=2\");}").unwrap();
        let compiled = std::process::Command::new("rustc.exe")
            .args([
                "--edition",
                "2024",
                "--crate-name",
                "mdlite_native_storage_mock",
            ])
            .arg(&code)
            .arg("-o")
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            compiled.status.success(),
            "{}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        let metadata = f.repo.join(".mdlite");
        fs::create_dir_all(&metadata).unwrap();
        let config = metadata.join("storage.toml");
        fs::write(&config,format!("executable = {}\nargument = \"{{file}}\"\nargument = \"{{revision}}\"\ntimeout_ms = 3000\n",serde_json::to_string(executable.to_str().unwrap()).unwrap())).unwrap();
        fs::write(f.repo.join("asset.png"), b"fixture local asset").unwrap();
        let mut document = Document::new(
            88,
            "![alt](asset.png \"title\") <!-- mdlite-width:480 -->".into(),
        );
        document.replace_range(0..0, "prefix\n").unwrap();
        let before = document.source().to_owned();
        let undo = document.undo_count();
        let capture = || SourceStamp {
            id: document.id(),
            revision: document.revision(),
            source: document.source_shared(),
            directory: f.repo.clone(),
        };
        let Completed::Prepared {
            stamp,
            plan: Ok(plan),
        } = f.run(Task::PrepareUpload {
            stamp: capture(),
            reference: 7..document.source().len(),
            config,
        })
        else {
            panic!("prepare mock failed");
        };
        assert_eq!(plan.confirmation().revision, 1);
        // Fixture-only explicit consent: the production path shows its default-No dialog.
        let Completed::Uploaded {
            result:
                UploadRun {
                    outcome: UploadOutcome::RemoteSucceeded(pending),
                    process: Some(process),
                },
            ..
        } = f.run(Task::Upload { stamp, plan })
        else {
            panic!("mock upload failed");
        };
        assert!(process.succeeded());
        assert_eq!(process.generation, 19);
        let Completed::Finalized { edit: Ok(edit), .. } = f.run(Task::Finalize {
            stamp: capture(),
            pending,
        }) else {
            panic!("mock finalize failed");
        };
        assert_eq!(edit.document_id, 88);
        assert_eq!(edit.revision, 1);
        assert_eq!(
            document.source().get(edit.range.clone()),
            Some(edit.expected.as_str())
        );
        let selection =
            selection_after_source_edit(document.selection(), &edit.range, edit.replacement.len());
        document
            .replace_range_with_selection(edit.range, &edit.replacement, selection)
            .unwrap();
        assert_eq!(document.undo_count(), undo + 1);
        assert!(document.source().contains("mock.png?x=1&amp;y=2"));
        assert!(
            document
                .source()
                .ends_with(" \"title\") <!-- mdlite-width:480 -->")
        );
        assert_eq!(
            fs::read(f.repo.join("asset.png")).unwrap(),
            b"fixture local asset"
        );
        assert!(document.undo());
        assert_eq!(document.source(), before);
    }
    #[test]
    fn native_git_disabled_reasons_match_workspace_trust_selection_remote_and_ime() {
        let f = Fixture::new();
        let mut app = super::super::tests::composing_test_app("text");
        app.composing = false;
        app.preedit.clear();
        app.ime_owner = None;
        assert!(app.git_disabled_reason(STAGE).contains("NoWorkspace"));
        app.git_ui.workspace = Some(WorkspaceIdentity::read(&f.repo).unwrap());
        app.git_ui.trust = Some(f.trust.clone());
        assert!(app.git_disabled_reason(STAGE).contains("Untrusted"));
        app.git_ui.trusted = true;
        app.git_ui.status = Some(GitStatus {
            state: GitState::NoRemote,
            repository_root: f.repo.clone(),
            ..Default::default()
        });
        assert!(app.git_disabled_reason(STAGE).contains("NoSelectedFile"));
        assert!(app.git_disabled_reason(FETCH).contains("NoRemote"));
        assert!(app.git_disabled_reason(UPLOAD).contains("NoImage"));
        app.composing = true;
        assert!(app.git_disabled_reason(STAGE).contains("IME"));
        app.composing = false;
        app.git_ui.status.as_mut().unwrap().state = GitState::NoRepository;
        assert!(app.git_disabled_reason(STAGE).contains("NoRepo"));
        let (_sender, receiver) = mpsc::channel();
        app.git_ui.worker = Some(receiver);
        assert!(app.git_disabled_reason(STAGE).contains("Busy"));
        assert_eq!(app.git_disabled_reason(CANCEL), "");
    }
    #[test]
    fn search_file_mutations_exclude_git_process_start_and_keep_own_cancel() {
        let f = Fixture::new();
        let mut app = super::super::tests::composing_test_app("text");
        app.composing = false;
        app.git_ui.workspace = Some(WorkspaceIdentity::read(&f.repo).unwrap());
        app.git_ui.trust = Some(f.trust.clone());
        app.git_ui.trusted = true;
        app.git_ui.status = Some(GitStatus {
            state: GitState::NoRemote,
            repository_root: f.repo.clone(),
            ..Default::default()
        });
        app.git_ui.selected.push(PathBuf::from("selected.md"));
        assert!(app.git_enabled(STAGE));
        for restoring in [false, true] {
            app.search_ui = super::super::SearchPane::pending_mutation_fixture(restoring);
            assert!(app.search_busy());
            for &(id, _, _) in COMMANDS {
                if id != CANCEL {
                    assert!(!app.git_enabled(id), "{id}");
                }
            }
            assert!(!app.git_start(Task::Refresh));
            assert!(app.git_ui.worker.is_none());
            let (_, receiver) = mpsc::channel();
            app.git_ui.worker = Some(receiver);
            assert!(app.git_enabled(CANCEL));
            app.git_ui.worker = None;
        }
    }
}
