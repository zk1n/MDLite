//! UI-thread persistence. Modal reentry cannot borrow App, so it cannot save.
use super::*;
use mdlite::{
    recovery::{Recovery, RecoveryStore},
    session::{SaveSchedule, Session, SessionDocument, View, WindowPlacement, source_sha256},
    workspace::Workspace,
};
use std::{fs, io, path::Component, time::Duration};

#[derive(Default)]
pub(super) struct DocumentPersistence {
    schedule: SaveSchedule,
    workspace: Option<PathBuf>,
    recovery: Option<Recovery>,
    store: Option<RecoveryStore>,
    pending_view: Option<View>,
    view_anchor: Option<usize>,
    horizontal_anchor: Option<usize>,
    first_visible_line: usize,
    pub(super) compact: bool,
    pub(super) placement: WindowPlacement,
    source_path: Option<PathBuf>,
    pub(super) format: Option<FileFormat>,
    failed_autosave_revision: Option<u64>,
    was_blocked: bool,
    discarded_revision: Option<u64>,
}
impl DocumentPersistence {
    pub(super) fn recovered_source(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }
    pub(super) fn new(workspace: Option<PathBuf>) -> Self {
        Self {
            workspace,
            ..Self::default()
        }
    }
}
pub(super) struct Persistence {
    workspace: Option<Workspace>,
    pub(super) startup_workspace: Option<PathBuf>,
    session: Option<FileSnapshot>,
    session_writable: bool,
    recent: Vec<PathBuf>,
    autosave: bool,
    delay: Duration,
    last_session: Option<Instant>,
    session_source: Option<String>,
    lease: Option<mdlite::instance::WorkspaceLease>,
    deferred_views: Vec<DeferredView>,
}
#[derive(Clone)]
struct DeferredView {
    workspace: PathBuf,
    version: u8,
    record: SessionDocument,
}
impl Default for Persistence {
    fn default() -> Self {
        Self {
            workspace: None,
            startup_workspace: None,
            session: None,
            session_writable: false,
            recent: Vec::new(),
            autosave: true,
            delay: Duration::from_millis(750),
            last_session: None,
            session_source: None,
            lease: None,
            deferred_views: Vec::new(),
        }
    }
}
impl Persistence {
    pub(super) fn workspace_root(&self) -> Option<PathBuf> {
        self.workspace.as_ref().map(|w| w.root().to_owned())
    }
    pub(super) fn recent_paths(&self) -> Vec<PathBuf> {
        self.recent.clone()
    }
}

// Session/recovery metadata must not follow a junction out of this workspace.
pub(super) fn bounded_path(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let mut path = root.to_owned();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(io::Error::other("Workspace相対pathが不正です"));
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 || metadata.file_type().is_symlink() {
                    return Err(io::Error::other(
                        "Workspace内のreparse pointは使用できません",
                    ));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(path)
}

fn same_view_binding(left: &SessionDocument, right: &SessionDocument) -> bool {
    left.path == right.path
        && left.recovery_record == right.recovery_record
        && left.source_sha256 == right.source_sha256
}
fn capture_session_record(tab: &Tab, path: PathBuf, workspace: &Path) -> SessionDocument {
    let selection = tab.doc.selection();
    let recovery_record = tab
        .persistence
        .recovery
        .as_ref()
        .filter(|r| r.workspace == workspace && r.source == tab.doc.source())
        .and_then(|r| r.record.strip_prefix(workspace).ok())
        .map(Path::to_path_buf);
    SessionDocument {
        path,
        source_sha256: Some(source_sha256(tab.doc.source())),
        recovery_record,
        anchor: selection.anchor,
        head: selection.head,
        affinity: tab.doc.selection_affinity(),
        first_visible_line: tab
            .persistence
            .pending_view
            .as_ref()
            .map_or(tab.persistence.first_visible_line, |v| v.first_visible_line),
        view_anchor: tab
            .persistence
            .pending_view
            .as_ref()
            .and_then(|v| v.anchor)
            .or(tab.persistence.view_anchor)
            .filter(|&offset| tab.doc.byte_to_utf16(offset).is_ok()),
        horizontal_anchor: tab
            .persistence
            .pending_view
            .as_ref()
            .and_then(|v| v.horizontal_anchor)
            .or(tab.persistence.horizontal_anchor)
            .filter(|&offset| tab.doc.byte_to_utf16(offset).is_ok()),
        compact: tab.persistence.compact,
        placement: tab.persistence.placement.clone(),
    }
}
impl App {
    pub(super) unsafe fn start_persistence(&mut self) {
        unsafe {
            if !self.shell.enabled {
                let menu = CreateMenu();
                let file = CreatePopupMenu();
                if let (Ok(menu), Ok(file)) = (menu, file) {
                    let _ = AppendMenuW(file, MF_STRING, CMD_WORKSPACE, w!("Workspaceを開く…"));
                    let _ = AppendMenuW(file, MF_STRING, CMD_RECOVERY, w!("復旧候補…"));
                    let _ =
                        AppendMenuW(file, MF_STRING, CMD_DISCARD_RECOVERY, w!("復旧候補を破棄…"));
                    let _ = AppendMenuW(file, MF_STRING, CMD_COMPARE, w!("ディスクと比較"));
                    let _ = AppendMenuW(file, MF_STRING, CMD_RELOAD, w!("ディスクから再読込み…"));
                    let _ = AppendMenuW(menu, MF_POPUP, file.0 as usize, w!("ファイル"));
                    let _ = SetMenu(self.hwnd, Some(menu));
                }
            }
            SetTimer(Some(self.hwnd), PERSISTENCE_TIMER, 250, None);
            self.load_persistence_settings();
            if let Some(path) = self.persistence.startup_workspace.take() {
                self.open_workspace(path);
            }
        }
    }
    pub(super) unsafe fn notice(&mut self, title: &str, text: &str) {
        unsafe {
            self.release_caret();
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(wide(text).as_ptr()),
                PCWSTR(wide(title).as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
    pub(super) unsafe fn choose_workspace(&mut self) {
        unsafe {
            let dialog: windows::core::Result<IFileOpenDialog> =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER);
            let Ok(dialog) = dialog else {
                self.status = "フォルダー選択を開けません".into();
                return;
            };
            let options = dialog.GetOptions().unwrap_or_default();
            if dialog
                .SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR)
                .is_err()
                || dialog.Show(Some(self.hwnd)).is_err()
            {
                return;
            }
            let Ok(item) = dialog.GetResult() else {
                return;
            };
            let Ok(path) = item.GetDisplayName(SIGDN_FILESYSPATH) else {
                return;
            };
            let selected = path.to_string().ok().map(PathBuf::from);
            CoTaskMemFree(Some(path.0.cast()));
            if let Some(path) = selected {
                self.open_workspace(path);
            }
        }
    }
    unsafe fn open_workspace(&mut self, path: PathBuf) {
        unsafe {
            let workspace = match Workspace::open(path) {
                Ok(w) => w,
                Err(e) => {
                    self.status = format!("Workspaceを開けません: {e}");
                    return;
                }
            };
            if self
                .persistence
                .workspace
                .as_ref()
                .is_some_and(|w| w.identity().same_workspace(workspace.identity()))
            {
                return;
            }
            if self.persistence.workspace.is_some() {
                self.open_workspace_new_process(workspace.root().to_owned());
                return;
            }
            let lease = match mdlite::instance::claim(workspace.root()) {
                Ok(mdlite::instance::Claim::Owned(lease)) => lease,
                Ok(mdlite::instance::Claim::Existing { message }) => {
                    self.notice(
                        "Workspace",
                        "このWorkspaceは別のMDLiteで開かれています。既存windowへ移動します。",
                    );
                    if !mdlite::instance::notify_existing(message) {
                        self.status="Workspaceは既に使用中です。既存windowは応答を確認できませんでした。現在の文書は保持しています".into();
                    }
                    return;
                }
                Err(error) => {
                    self.status = format!(
                        "Workspaceの重複起動を検査できません — 現在の文書は保持しています: {error}"
                    );
                    return;
                }
            };
            let pointer =
                GetWindowLongPtrW(self.shell.main_window, GWLP_USERDATA) as *const WindowState;
            if !pointer.is_null() {
                (*pointer).workspace_message.set(lease.message);
            }
            self.persistence.lease = Some(lease);
            self.persist_session();
            let root = workspace.root().to_owned();
            let state = bounded_path(&root, Path::new(".mdlite/.state"));
            let mut available = state.as_ref().is_ok_and(|p| p.is_dir());
            if !available {
                let prompt = format!(
                    "{}\n\n復旧とsession用の .mdlite/.state と .mdlite/.cache を作成し、.gitignoreへ除外規則を追加しますか？\n「いいえ」でも編集は続けられます。",
                    root.display()
                );
                self.release_caret();
                if MessageBoxW(
                    Some(self.hwnd),
                    PCWSTR(wide(&prompt).as_ptr()),
                    w!("Workspace初期化"),
                    MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
                ) == IDYES
                {
                    match workspace.initialize() {
                        Ok(report) if report.is_complete() => available = true,
                        Ok(report) => self.notice(
                            "Workspace初期化",
                            &format!(
                                "初期化が一部失敗しました。編集は継続できます。\n{:?}",
                                report.failures
                            ),
                        ),
                        Err(e) => self.notice("Workspace初期化", &e.to_string()),
                    }
                }
            }
            self.persistence.workspace = Some(workspace);
            self.persistence.session = None;
            self.persistence.session_source = None;
            self.persistence.session_writable = available;
            self.persistence.recent.clear();
            self.persistence.deferred_views.clear();
            self.load_persistence_settings();
            self.load_panel_layout();
            for tab in &mut self.tabs {
                if tab.persistence.workspace.is_none() {
                    tab.persistence.workspace = Some(root.clone());
                }
            }
            if available {
                self.load_session();
                self.present_recoveries();
            } else {
                self.status =
                    "Workspaceを開きました — 状態領域が利用できず、復旧/sessionは保存できません"
                        .into();
            }
        }
    }
    pub(super) unsafe fn open_workspace_new_process(&mut self, path: PathBuf) {
        unsafe {
            use std::os::windows::process::CommandExt;
            let result = std::env::current_exe().and_then(|exe| {
                std::process::Command::new(exe)
                    .arg(path)
                    .creation_flags(0x0800_0000)
                    .spawn()
            });
            match result {
                Ok(_) => self.status = "別Workspace用のMDLiteを起動しました。開いた結果は新しいwindowで確認してください".into(),
                Err(error) => self.notice(
                    "Workspaceを開けません",
                    &format!("起動できませんでした。現在の本文は保持しています: {error}"),
                ),
            }
        }
    }
    pub(super) fn persistence_tick(&mut self, now: Instant) {
        let delay = self.persistence.delay;
        let mutation_blocked = self.background_mutation_busy();
        let property_blocked = self.properties_pending();
        for index in 0..self.tabs.len() {
            let tab = &mut self.tabs[index];
            tab.persistence
                .schedule
                .changed(tab.doc.revision(), tab.doc.is_dirty(), now, delay);
            let blocked =
                mutation_blocked || (self.composing || property_blocked) && index == self.active;
            if tab.persistence.was_blocked && !blocked && tab.doc.is_dirty() {
                tab.persistence.schedule.autosave_attempted(now, delay);
            }
            tab.persistence.was_blocked = blocked;
            let (save, recover) = tab.persistence.schedule.due(
                now,
                blocked,
                tab.file.is_some(),
                self.persistence.autosave,
            );
            // Recovery is independent of source saves, including failed saves.
            if recover {
                tab.persistence.schedule.recovery_attempted(now);
                if let Err(e) = self.recover_tab(index) {
                    self.status =
                        format!("復旧snapshotを保存できません — 本文は保持しています: {e}");
                }
            }
            if save
                && self.tabs[index].persistence.failed_autosave_revision
                    != Some(self.tabs[index].doc.revision())
            {
                let tab = &mut self.tabs[index];
                tab.persistence.schedule.autosave_attempted(now, delay);
                let document_id = tab.doc.id();
                let result = tab.file.as_mut().unwrap().save(tab.doc.source());
                match result {
                    Ok(()) => {
                        tab.doc.mark_saved();
                        tab.persistence.failed_autosave_revision = None;
                        self.saved_recovery(index);
                        self.clear_external_change(document_id);
                    }
                    Err(e) => {
                        tab.persistence.failed_autosave_revision = Some(tab.doc.revision());
                        self.status = format!("自動保存失敗 — 未保存本文は保持しています: {e}");
                        if matches!(e, mdlite::storage::StorageError::ExternalChange) {
                            self.mark_external_change(document_id);
                        }
                    }
                }
            }
        }
        if !self.composing
            && !mutation_blocked
            && self
                .persistence
                .last_session
                .is_none_or(|last| now.saturating_duration_since(last) >= Duration::from_secs(5))
        {
            self.persistence.last_session = Some(now);
            self.persist_session();
        }
    }
    pub(super) fn rearm_autosave(&mut self, all: bool) {
        let now = Instant::now();
        let delay = self.persistence.delay;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            if (all || index == self.active) && tab.doc.is_dirty() {
                tab.persistence.schedule.autosave_attempted(now, delay);
            }
        }
    }
    fn recover_tab(&mut self, index: usize) -> io::Result<()> {
        let tab = &mut self.tabs[index];
        let root = tab
            .persistence
            .workspace
            .as_deref()
            .ok_or_else(|| io::Error::other("Workspaceがありません"))?;
        if tab.persistence.store.is_none() {
            tab.persistence.store = Some(RecoveryStore::open(root)?);
        }
        let store = tab.persistence.store.as_ref().unwrap();
        let path = store.write_with_metadata(
            &tab.doc,
            tab.file
                .as_ref()
                .map(|f| f.path())
                .or(tab.persistence.source_path.as_deref()),
            tab.file.as_ref(),
            tab.persistence.format,
        )?;
        tab.persistence.recovery = Some(store.read(&path)?);
        tab.persistence.discarded_revision = None;
        Ok(())
    }
    pub(super) fn discard_tab_recovery(&mut self, index: usize) {
        let tab = &mut self.tabs[index];
        // Close-No is explicit discard even if the first recovery deadline has
        // not elapsed. A later session write must not recreate this source.
        tab.persistence.discarded_revision = Some(tab.doc.revision());
        let removed = tab.persistence.recovery.clone();
        let mut discarded = false;
        if let Some(record) = tab.persistence.recovery.as_ref() {
            match RecoveryStore::open(&record.workspace).and_then(|s| s.discard(record)) {
                Ok(()) => {
                    tab.persistence.recovery = None;
                    tab.persistence.store = None;
                    discarded = true;
                }
                Err(e) => {
                    self.status = format!("復旧候補を破棄できません。原本は保持しています: {e}")
                }
            }
        }
        if discarded && let Some(record) = removed {
            self.remove_deferred_recovery(&record);
        }
    }
    pub(super) fn saved_recovery(&mut self, index: usize) {
        // A successful source save is separate from recovery cleanup. Retain
        // candidates whose contents were not the source just saved.
        let tab = &self.tabs[index];
        if let (Some(record), Some(file)) = (&tab.persistence.recovery, &tab.file)
            && file
                .path()
                .strip_prefix(&record.workspace)
                .ok()
                .and_then(|p| bounded_path(&record.workspace, p).ok())
                .is_none()
        {
            self.status = "本文をWorkspace外へ保存しました。元Workspaceの復旧候補は保持しています。再開時は保存先を開くか元Workspaceの復旧候補を選んでください。相対リンクの解決先も変わります".into();
            return;
        }
        if tab
            .persistence
            .recovery
            .as_ref()
            .is_some_and(|r| r.source == tab.doc.source())
        {
            self.discard_tab_recovery(index);
        }
    }
    pub(super) unsafe fn present_recoveries(&mut self) {
        unsafe {
            self.choose_recovery_action(false);
        }
    }
    pub(super) unsafe fn discard_recovery_candidates(&mut self) {
        unsafe {
            self.choose_recovery_action(true);
        }
    }
    unsafe fn choose_recovery_action(&mut self, discard: bool) {
        unsafe {
            let Some(root) = self.persistence.workspace_root() else {
                return;
            };
            let candidates = match RecoveryStore::open(&root).and_then(|s| s.list()) {
                Ok(c) => c,
                Err(e) => {
                    self.notice("復旧候補", &format!("候補を読み取れません: {e}"));
                    return;
                }
            };
            for issue in candidates.issues {
                self.notice("復旧候補 — 原本保持", &issue);
            }
            for candidate in candidates.records {
                // Do not present this process's live periodic snapshot again.
                if self.tabs.iter().any(|t| {
                    t.persistence
                        .recovery
                        .as_ref()
                        .is_some_and(|r| r.record == candidate.record)
                }) {
                    continue;
                }
                let name = candidate
                    .source_path
                    .as_ref()
                    .map_or_else(|| "無題".into(), |p| p.display().to_string());
                let prompt = if discard {
                    format!(
                        "{name}\n\nこの復旧候補を完全に破棄しますか？\nいいえ: 保持\nキャンセル: 終了"
                    )
                } else {
                    format!(
                        "{name}\n\n復旧候補を開きますか？元候補は保持します。\nいいえ: 保持\nキャンセル: 終了\n破棄は「復旧候補を破棄…」から行えます。"
                    )
                };
                let answer = MessageBoxW(
                    Some(self.hwnd),
                    PCWSTR(wide(&prompt).as_ptr()),
                    w!("復旧候補"),
                    MB_YESNOCANCEL | MB_DEFBUTTON2 | MB_ICONQUESTION,
                );
                if answer == IDCANCEL {
                    break;
                }
                if answer == IDYES {
                    if discard {
                        match RecoveryStore::open(&root).and_then(|s| s.discard(&candidate)) {
                            Ok(()) => self.remove_deferred_recovery(&candidate),
                            Err(e) => self.notice("破棄失敗 — 原本保持", &e.to_string()),
                        }
                    } else {
                        self.restore_recovery(candidate);
                    }
                }
            }
        }
    }
    pub(super) fn restore_recovery(&mut self, record: Recovery) {
        let fingerprint = source_sha256(&record.source);
        let bound_view = self
            .persistence
            .deferred_views
            .iter()
            .position(|pending| {
                pending.workspace == record.workspace
                    && pending.record.source_sha256.as_ref() == Some(&fingerprint)
                    && match &pending.record.recovery_record {
                        Some(path) => {
                            same_file_identity(&pending.workspace.join(path), &record.record)
                        }
                        None => {
                            !pending.record.path.as_os_str().is_empty()
                                && record.source_path.as_ref().is_some_and(|path| {
                                    same_file_identity(
                                        path,
                                        &pending.workspace.join(&pending.record.path),
                                    )
                                })
                        }
                    }
            })
            .map(|index| (index, self.persistence.deferred_views[index].clone()));
        let verified_file = record.source_path.as_ref().and_then(|path| {
            if self.open_tab_for_path(path).is_some() {
                return None;
            }
            let relative = path.strip_prefix(&record.workspace).ok()?;
            let path = bounded_path(&record.workspace, relative).ok()?;
            let disk = FileSnapshot::open(path).ok()?;
            (Some(disk.snapshot.baseline_sha256()) == record.baseline_sha256).then_some(disk)
        });
        self.new_document();
        let tab = &mut self.tabs[self.active];
        if let Some(disk) = verified_file {
            tab.doc = Document::new(tab.doc.id(), disk.source);
            let _ = tab.doc.replace_range(0..tab.doc.byte_len(), &record.source);
            tab.file = Some(disk.snapshot);
        } else {
            tab.doc = Document::new_unsaved(tab.doc.id(), record.source.clone());
        }
        tab.persistence.format = record.format;
        tab.persistence.source_path = record.source_path.clone();
        tab.persistence.workspace = Some(record.workspace.clone());
        tab.persistence.recovery = Some(record);
        self.status = "別の未保存文書へ復元しました。元文書と復旧候補は保持しています".into();
        if let Some((index, pending)) = bound_view {
            let session = Session {
                version: pending.version,
                ..Session::default()
            };
            match session.hydrate(&pending.record, &tab.doc) {
                Ok(view) => {
                    let _ = tab.doc.set_selection_with_affinity(
                        view.selection.anchor,
                        view.selection.head,
                        view.affinity,
                    );
                    tab.persistence.pending_view = Some(view);
                    tab.persistence.compact = pending.record.compact;
                    tab.persistence.placement = pending.record.placement;
                    self.persistence.deferred_views.remove(index);
                }
                Err(error) => {
                    self.status =
                        format!("復旧本文は保持しました。session viewを適用できません: {error}")
                }
            }
        }
    }
    unsafe fn load_session(&mut self) {
        unsafe {
            let Some(root) = self.persistence.workspace_root() else {
                return;
            };
            let loaded =
                bounded_path(&root, Path::new(".mdlite/.state/session.toml")).and_then(|p| {
                    match FileSnapshot::open(p) {
                        Ok(f) => Ok(Some(f)),
                        Err(mdlite::storage::StorageError::Io(e))
                            if e.kind() == io::ErrorKind::NotFound =>
                        {
                            Ok(None)
                        }
                        Err(e) => Err(io::Error::other(e)),
                    }
                });
            let loaded = match loaded {
                Ok(Some(f)) => f,
                Ok(None) => return,
                Err(e) => {
                    self.persistence.session_writable = false;
                    self.notice("session読取り失敗 — 原本保持", &e.to_string());
                    return;
                }
            };
            let session = match Session::parse(&loaded.source) {
                Ok(s) => s,
                Err(e) => {
                    self.persistence.session_writable = false;
                    self.notice("session読取り失敗 — 原本保持", e);
                    return;
                }
            };
            self.persistence.session = Some(loaded.snapshot);
            self.persistence.session_source = Some(loaded.source);
            self.persistence.deferred_views = session
                .recovery_views
                .iter()
                .cloned()
                .map(|record| DeferredView {
                    workspace: root.clone(),
                    version: session.version,
                    record,
                })
                .collect();
            let mut selected = None;
            let mut hydrated = std::collections::HashSet::new();
            for (position, record) in session.documents.iter().enumerate() {
                let path = match bounded_path(&root, &record.path).and_then(fs::canonicalize) {
                    Ok(p) => p,
                    Err(e) => {
                        if record.recovery_record.is_some() {
                            self.defer_session_view(&root, session.version, record.clone());
                        }
                        self.notice("session文書を開けません", &e.to_string());
                        continue;
                    }
                };
                // Identity aliases share the existing Document and history.
                let already_open = self.open_tab_for_path(&path);
                if already_open.is_none() {
                    self.open_path(path.clone());
                }
                let Some(index) = self.open_tab_for_path(&path) else {
                    continue;
                };
                if position == session.active {
                    selected = Some(index);
                }
                if record.source_sha256.as_ref().is_some_and(|expected| {
                    *expected != source_sha256(self.tabs[index].doc.source())
                }) {
                    self.defer_session_view(&root, session.version, record.clone());
                    self.status="session viewは保存時の本文に関連付いています。対応する復旧候補を復元するまで保持しています".into();
                    continue;
                }
                if !hydrated.insert(self.tabs[index].doc.id()) {
                    continue;
                }
                if already_open.is_some() && self.tabs[index].doc.is_dirty() {
                    continue;
                }
                match session.hydrate(record, &self.tabs[index].doc) {
                    Ok(view) => {
                        let _ = self.tabs[index].doc.set_selection_with_affinity(
                            view.selection.anchor,
                            view.selection.head,
                            view.affinity,
                        );
                        self.tabs[index].persistence.pending_view = Some(view);
                        self.tabs[index].persistence.compact = record.compact;
                        self.tabs[index].persistence.placement = record.placement.clone();
                    }
                    Err(e) => {
                        // A stale view is local to this document. A readable
                        // session remains writable; never clamp into new text.
                        self.status = format!(
                            "この文書のsession viewを適用できません。本文と他のsession情報は保持しています: {e}"
                        );
                    }
                }
            }
            self.persistence.recent = session
                .recent
                .iter()
                .filter_map(|p| bounded_path(&root, p).ok().filter(|p| p.is_file()))
                .collect();
            if let Some(index) = selected {
                self.switch_tab(index);
            }
            if self.hwnd != HWND::default() {
                let p = session.main;
                let _ = SetWindowPos(
                    self.hwnd,
                    None,
                    p.x,
                    p.y,
                    p.width as i32,
                    p.height as i32,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                if self.shell.enabled {
                    self.monitor_safe_position(self.shell.main_window);
                    if self.tabs[self.active].persistence.compact {
                        self.open_compact();
                    }
                }
            }
        }
    }
    fn defer_session_view(&mut self, workspace: &Path, version: u8, record: SessionDocument) {
        if !self
            .persistence
            .deferred_views
            .iter()
            .any(|view| view.workspace == workspace && same_view_binding(&view.record, &record))
        {
            self.persistence.deferred_views.push(DeferredView {
                workspace: workspace.to_owned(),
                version,
                record,
            });
        }
    }
    fn remove_deferred_recovery(&mut self, record: &Recovery) {
        self.persistence.deferred_views.retain(|pending| {
            pending.workspace != record.workspace
                || !pending.record.recovery_record.as_ref().is_some_and(|path| {
                    same_file_identity(&pending.workspace.join(path), &record.record)
                })
        });
    }
    pub(super) fn restore_pending_view(&mut self) {
        let Some(view) = self.tabs[self.active].persistence.pending_view.take() else {
            return;
        };
        let anchor = view.anchor.unwrap_or_else(|| {
            physical_line_starts(self.doc().source())
                .get(view.first_visible_line)
                .copied()
                .unwrap_or(0)
        });
        if let Some(line) = self
            .views
            .iter()
            .find(|v| v.source.start <= anchor && anchor <= v.source.end)
        {
            let offset = line
                .map
                .partition_point(|&p| p < anchor)
                .min(line.text.len());
            let row_y = layout_caret(&line.layout, offset as u32, false).map_or(0., |(_, y, _)| y);
            self.tabs[self.active].scroll = (line.y + row_y - self.editor_top()).max(0.0);
        }
        if let Some(horizontal) = view.horizontal_anchor {
            let placement = self
                .views
                .iter()
                .find(|v| v.table.is_some() && v.map.contains(&horizontal))
                .and_then(|v| {
                    let position = v.map.partition_point(|&p| p < horizontal).min(v.text.len());
                    layout_caret(&v.layout, position as u32, false)
                        .map(|(x, _, _)| (v.table.unwrap(), v.x + x))
                });
            if let Some((index, x)) = placement {
                let Some(table) = self.tables.get(index) else {
                    return;
                };
                let key = table.table.range.start;
                let old = self.tabs[self.active]
                    .table_scroll
                    .get(&key)
                    .copied()
                    .unwrap_or(0.);
                let max = (table.width - self.editor_width()).max(0.);
                let next = (old + x - self.editor_left()).clamp(0., max);
                self.tabs[self.active].table_scroll.insert(key, next);
                self.layout_key = None;
                unsafe {
                    let _ = self.ensure_layout();
                }
            }
        }
        self.tabs[self.active].persistence.view_anchor = Some(anchor);
        self.tabs[self.active].persistence.horizontal_anchor = view.horizontal_anchor;
        self.tabs[self.active].persistence.first_visible_line = view.first_visible_line;
    }
    pub(super) fn capture_view_anchor(&mut self) {
        if !self.views.is_empty()
            && self.layout_key.is_some_and(|(id, revision, ..)| {
                id == self.doc().id() && revision == self.doc().revision()
            })
        {
            let anchor = self
                .hit_with_affinity(self.editor_left(), self.editor_top() + 0.5)
                .0;
            self.tabs[self.active].persistence.view_anchor = Some(anchor);
            self.capture_horizontal_view_anchor();
        }
    }
    fn capture_horizontal_view_anchor(&mut self) {
        let scroll = self.tabs[self.active].scroll;
        let head = self.doc().selection().head;
        let current = self
            .tables
            .iter()
            .position(|table| table.table.range.contains(&head));
        let view = self.views.iter().find(|view| {
            let Some(index) = view.table else {
                return false;
            };
            current.is_none_or(|current| current == index)
                && self.tables.get(index).is_some_and(|table| {
                    self.tabs[self.active]
                        .table_scroll
                        .get(&table.table.range.start)
                        .is_some_and(|offset| *offset > 0.)
                })
                && view.y + view.height > self.editor_top() + scroll
                && view.y < self.editor_bottom() + scroll
        });
        let anchor = view
            .map(|view| {
                let y = (view.y - scroll + view.height / 2.).clamp(
                    self.editor_top() + 0.5,
                    self.editor_bottom().max(self.editor_top() + 1.) - 0.5,
                );
                self.hit_with_affinity(self.editor_left(), y).0
            })
            .filter(|&offset| self.doc().byte_to_utf16(offset).is_ok());
        self.tabs[self.active].persistence.horizontal_anchor = anchor;
    }
    pub(super) fn record_recent(&mut self, path: &Path) {
        let Ok(path) = fs::canonicalize(path) else {
            return;
        };
        self.persistence
            .recent
            .retain(|p| !same_file_identity(p, &path));
        self.persistence.recent.insert(0, path);
        self.persistence.recent.truncate(100);
    }
    pub(super) fn persist_session(&mut self) {
        if !self.persistence.session_writable {
            return;
        }
        let Some(root) = self.persistence.workspace_root() else {
            return;
        };
        self.capture_view_anchor();
        // Session coordinates must refer to a durable recovery source before
        // they can be used to restore a dirty/untitled document after restart.
        for index in 0..self.tabs.len() {
            let tab = &self.tabs[index];
            if tab.doc.is_dirty()
                && tab.persistence.discarded_revision != Some(tab.doc.revision())
                && tab.persistence.workspace.as_ref() == Some(&root)
                && !tab
                    .persistence
                    .recovery
                    .as_ref()
                    .is_some_and(|r| r.source == tab.doc.source())
                && let Err(error) = self.recover_tab(index)
            {
                self.status = format!(
                    "session view用の復旧本文を保存できません。未保存本文は保持しています: {error}"
                );
            }
        }
        let active_id = self.doc().id();
        let mut session = Session {
            recent: self
                .persistence
                .recent
                .iter()
                .filter_map(|p| p.strip_prefix(&root).ok().map(Path::to_path_buf))
                .collect(),
            ..Session::default()
        };
        let mut bounds = RECT::default();
        if self.hwnd != HWND::default()
            && unsafe {
                GetWindowRect(
                    if self.shell.enabled {
                        self.shell.main_window
                    } else {
                        self.hwnd
                    },
                    &mut bounds,
                )
            }
            .is_ok()
        {
            session.main = WindowPlacement {
                x: bounds.left,
                y: bounds.top,
                width: (bounds.right - bounds.left).max(160) as u32,
                height: (bounds.bottom - bounds.top).max(100) as u32,
            };
        }
        if self.shell.compact_window != HWND::default() {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(self.shell.compact_window, &mut r) }.is_ok() {
                self.tabs[self.active].persistence.placement = WindowPlacement {
                    x: r.left,
                    y: r.top,
                    width: (r.right - r.left) as u32,
                    height: (r.bottom - r.top) as u32,
                };
            }
        }
        for tab in &self.tabs {
            if tab.doc.is_dirty() && tab.persistence.discarded_revision == Some(tab.doc.revision())
            {
                continue;
            }
            let Some(file) = &tab.file else {
                if tab.doc.is_dirty() && tab.persistence.workspace.as_ref() == Some(&root) {
                    let record = capture_session_record(tab, PathBuf::new(), &root);
                    if record.recovery_record.is_some() {
                        session.recovery_views.push(record);
                    }
                }
                continue;
            };
            let Ok(relative) = file.path().strip_prefix(&root) else {
                continue;
            };
            if bounded_path(&root, relative).is_err() {
                continue;
            }
            if tab.doc.id() == active_id {
                session.active = session.documents.len();
            }
            session
                .documents
                .push(capture_session_record(tab, relative.to_owned(), &root));
        }
        for pending in &self.persistence.deferred_views {
            if pending.workspace == root
                && pending.version >= 3
                && !session
                    .recovery_views
                    .iter()
                    .any(|record| same_view_binding(record, &pending.record))
            {
                session.recovery_views.push(pending.record.clone());
            }
        }
        let result = session
            .serialize()
            .map_err(io::Error::other)
            .and_then(|text| {
                if self.persistence.session_source.as_ref() == Some(&text) {
                    return Ok(());
                }
                let path = bounded_path(&root, Path::new(".mdlite/.state/session.toml"))?;
                let result = match self.persistence.session.as_mut() {
                    Some(snapshot) => snapshot.save(&text).map_err(io::Error::other),
                    None => FileSnapshot::create_new(path, &text, FileFormat::default())
                        .map(|f| self.persistence.session = Some(f))
                        .map_err(io::Error::other),
                };
                if result.is_ok() {
                    self.persistence.session_source = Some(text);
                }
                result
            });
        if let Err(e) = result {
            self.status = format!("session保存失敗 — 既存状態は保持しています: {e}");
        }
    }
    pub(super) fn load_persistence_settings(&mut self) {
        use mdlite::settings::{CapturedSettings, resolve_settings};
        let common = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|p| p.join("MDLite/settings.toml"));
        let workspace = self
            .persistence
            .workspace_root()
            .map(|p| p.join(".mdlite/settings.toml"));
        let mut diagnostics = Vec::new();
        let mut capture = |path: Option<PathBuf>| {
            path.and_then(|p| match CapturedSettings::capture(p) {
                Ok(c) => Some(c.parsed()),
                Err(e) => {
                    diagnostics.push(e.to_string());
                    None
                }
            })
        };
        let common = capture(common);
        let workspace = capture(workspace);
        let result = resolve_settings(common.as_ref(), workspace.as_ref());
        self.persistence.autosave = result.settings.auto_save;
        self.persistence.delay =
            Duration::from_millis(u64::from(result.settings.auto_save_delay_ms));
        if self.shell.enabled && self.shell.settings != result.settings {
            unsafe {
                self.apply_shell_settings(result.settings.clone());
            }
        }
        diagnostics.extend(result.diagnostics.into_iter().map(|d| d.message));
        if !diagnostics.is_empty() {
            unsafe {
                self.notice("設定エラー — 安全なfallbackを使用", &diagnostics.join("\n"));
            }
        }
    }
    pub(super) unsafe fn compare_disk(&mut self) {
        unsafe {
            let Some(file) = &self.tabs[self.active].file else {
                self.status = "無題には比較先がありません".into();
                return;
            };
            let result =
                FileSnapshot::open(file.path()).map(|disk| disk.source == self.doc().source());
            match result {
                Ok(true) => self.notice(
                    "ディスク比較",
                    "本文はディスクと一致しています。文書と保存baselineは変更していません。",
                ),
                Ok(false) => self.notice(
                    "ディスク比較",
                    "本文はディスクと異なります。文書と保存baselineは変更していません。",
                ),
                Err(e) => self.notice("ディスク比較失敗", &e.to_string()),
            }
        }
    }
    pub(super) unsafe fn reload_disk(&mut self) {
        unsafe {
            let Some(path) = self.tabs[self.active]
                .file
                .as_ref()
                .map(|f| f.path().to_owned())
            else {
                return;
            };
            if MessageBoxW(
                Some(self.hwnd),
                w!("ディスクを再読込みしますか？未保存本文は先に復旧snapshotへ保存します。"),
                w!("再読込み"),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
            ) != IDYES
            {
                return;
            }
            if let Err(e) = self.reload_disk_from(&path) {
                self.notice("再読込み中止 — 元本文保持", &e.to_string());
            }
        }
    }
    fn reload_disk_from(&mut self, path: &Path) -> io::Result<()> {
        // Both operations finish before replacing the canonical Document.
        if self.doc().is_dirty() {
            self.recover_tab(self.active)?;
        }
        let disk = FileSnapshot::open(path).map_err(io::Error::other)?;
        let old_id = self.doc().id();
        let id = self.next_id;
        self.next_id += 1;
        self.tabs[self.active].doc = Document::new(id, disk.source);
        self.tabs[self.active].file = Some(disk.snapshot);
        self.tabs[self.active].scroll = 0.0;
        self.tabs[self.active].persistence.pending_view = None;
        self.tabs[self.active].persistence.view_anchor = None;
        self.tabs[self.active].persistence.horizontal_anchor = None;
        self.tabs[self.active].persistence.schedule = SaveSchedule::default();
        self.tabs[self.active].persistence.failed_autosave_revision = None;
        self.tabs[self.active].persistence.discarded_revision = None;
        self.tabs[self.active].persistence.store = None;
        self.tabs[self.active].table_scroll.clear();
        self.clear_external_change(old_id);
        self.layout_key = None;
        self.projection_cache = None;
        self.clear_images();
        self.status = "再読込みしました。以前の未保存本文の復旧候補は保持しています".into();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "mdlite-native-persistence-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(path.join(".mdlite/.state")).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn app(&self) -> App {
            let mut app = super::super::tests::composing_test_app("initial");
            app.composing = false;
            app.preedit.clear();
            app.ime_owner = None;
            app.tabs.clear();
            app.persistence.workspace = Some(Workspace::open(&self.0).unwrap());
            app.new_document();
            app
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn persistence_settings_reload_preserves_unchanged_appearance() {
        let fixture = Fixture::new();
        let path = fixture.0.join(".mdlite/settings.toml");
        fs::write(&path, "schema_version = 1\ntheme = \"light\"\n").unwrap();
        let mut app = fixture.app();
        app.hwnd = super::super::tests::hidden_test_window_size(640, 480);
        app.shell.enabled = true;
        app.shell.main_window = app.hwnd;
        let state = Box::new(WindowState::new(app));
        let hwnd = state.app.borrow().hwnd;
        state.main_window.set(hwnd);
        unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWLP_USERDATA,
                (&*state as *const WindowState) as isize,
            );
        }
        let mut app = state.app.borrow_mut();
        app.load_persistence_settings();
        let colors = state.control_colors.borrow().as_ref().unwrap().clone();
        unsafe {
            app.ensure_layout().unwrap();
        }
        let layout = app.layout_key;
        let source = app.doc().source_shared();
        let selection = app.doc().selection();
        let document_before = (
            app.doc().revision(),
            app.doc().is_dirty(),
            app.doc().undo_count(),
            app.doc().redo_count(),
        );
        app.load_persistence_settings();
        assert!(Rc::ptr_eq(
            &colors,
            state.control_colors.borrow().as_ref().unwrap()
        ));
        assert_eq!(app.layout_key, layout);
        fs::write(&path, "schema_version = 1\ntheme = \"dark\"\n").unwrap();
        app.load_persistence_settings();
        assert!(!Rc::ptr_eq(
            &colors,
            state.control_colors.borrow().as_ref().unwrap()
        ));
        assert!(app.layout_key.is_none());
        assert_eq!(app.doc().source(), &*source);
        assert_eq!(app.doc().selection(), selection);
        assert_eq!(
            (
                app.doc().revision(),
                app.doc().is_dirty(),
                app.doc().undo_count(),
                app.doc().redo_count()
            ),
            document_before
        );
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            DestroyWindow(hwnd).unwrap();
        }
    }

    #[test]
    fn persistence_autosave_defers_ime_and_preserves_peer_changes() {
        let fixture = Fixture::new();
        let path = fixture.0.join("note.md");
        fs::write(&path, b"\xef\xbb\xbforiginal\r\n").unwrap();
        let mut app = fixture.app();
        app.open_path(path.clone());
        app.doc_mut().replace_range(0..8, "edited").unwrap();
        let start = Instant::now();
        app.persistence_tick(start);
        app.composing = true;
        app.persistence_tick(start + Duration::from_secs(1));
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbforiginal\r\n");
        assert!(app.doc().is_dirty());
        app.composing = false;
        app.persistence_tick(start + Duration::from_secs(2));
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbforiginal\r\n");
        assert!(app.doc().is_dirty());
        app.persistence_tick(start + Duration::from_millis(2750));
        assert_eq!(fs::read(&path).unwrap(), b"\xef\xbb\xbfedited\r\n");
        assert!(!app.doc().is_dirty());
        app.doc_mut().replace_selection("more").unwrap();
        app.persistence_tick(start + Duration::from_secs(3));
        fs::write(&path, "peer").unwrap();
        app.persistence_tick(start + Duration::from_secs(4));
        assert_eq!(fs::read_to_string(&path).unwrap(), "peer");
        assert!(app.doc().is_dirty());
        assert!(app.status.contains("自動保存失敗"));
    }
    #[test]
    fn persistence_recovery_keeps_one_owned_record_and_preserves_workspace() {
        let fixture = Fixture::new();
        let mut app = fixture.app();
        app.doc_mut().replace_selection("日本語😀\r\n").unwrap();
        let now = Instant::now();
        app.persistence_tick(now);
        app.doc_mut().replace_selection("continued").unwrap();
        app.persistence_tick(now + Duration::from_secs(4));
        app.persistence_tick(now + Duration::from_secs(5));
        let first = app.tabs[app.active]
            .persistence
            .recovery
            .as_ref()
            .unwrap()
            .record
            .clone();
        app.doc_mut().replace_selection("later").unwrap();
        app.persistence_tick(now + Duration::from_secs(10));
        let record = app.tabs[app.active].persistence.recovery.as_ref().unwrap();
        assert_eq!(record.record, first);
        assert_eq!(record.source, app.doc().source());
        assert_eq!(
            RecoveryStore::open(&fixture.0)
                .unwrap()
                .list()
                .unwrap()
                .records
                .len(),
            1
        );
        let other = Fixture::new();
        app.persistence.workspace = Some(Workspace::open(&other.0).unwrap());
        app.doc_mut().replace_selection("elsewhere").unwrap();
        app.persistence_tick(now + Duration::from_secs(15));
        assert_eq!(
            app.tabs[app.active]
                .persistence
                .recovery
                .as_ref()
                .unwrap()
                .workspace,
            fixture.0
        );
        assert!(app.doc().is_dirty());
    }
    #[test]
    fn persistence_restore_uses_verified_baseline_and_preserves_fork_format() {
        let fixture = Fixture::new();
        let path = fixture.0.join("note.md");
        fs::write(&path, b"\xef\xbb\xbforiginal").unwrap();
        let disk = FileSnapshot::open(&path).unwrap();
        let mut doc = Document::new(41, disk.source);
        doc.replace_selection("unsaved").unwrap();
        let store = RecoveryStore::open(&fixture.0).unwrap();
        let record_path = store
            .write_with_file(&doc, Some(&path), Some(&disk.snapshot))
            .unwrap();
        let record = store.read(&record_path).unwrap();
        let mut app = fixture.app();
        app.restore_recovery(record.clone());
        assert!(app.tabs[app.active].file.is_some());
        assert!(app.doc().is_dirty());
        assert_eq!(app.doc().source(), doc.source());
        fs::write(&path, "peer change").unwrap();
        app.restore_recovery(record);
        assert!(app.tabs[app.active].file.is_none());
        assert_eq!(
            app.tabs[app.active].persistence.format,
            Some(FileFormat::Utf8 { bom: true })
        );
        let output = fixture.0.join("recovered.md");
        assert!(app.save_as_to(&output));
        assert!(fs::read(output).unwrap().starts_with(b"\xef\xbb\xbf"));
        assert_eq!(fs::read_to_string(path).unwrap(), "peer change");
    }
    #[test]
    fn persistence_legacy_session_hydrates_aliases_and_writes_byte_schema() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("note.md"), "A😀日\r\nZ").unwrap();
        let text = "schema_version = 2\nactive_index = 0\n[[document]]\npath = \"note.md\"\nselection_begin = 4\nselection_end = 1\nfirst_visible_source_offset = 3\n";
        fs::write(fixture.0.join(".mdlite/.state/session.toml"), text).unwrap();
        let mut app = fixture.app();
        app.persistence.session_writable = true;
        unsafe {
            app.load_session();
        }
        assert_eq!(app.doc().selection(), Selection { anchor: 8, head: 1 });
        assert_eq!(
            app.tabs[app.active]
                .persistence
                .pending_view
                .as_ref()
                .unwrap()
                .anchor,
            Some(5)
        );
        app.persist_session();
        let saved = fs::read_to_string(fixture.0.join(".mdlite/.state/session.toml")).unwrap();
        assert!(saved.starts_with("schema_version = 4"));
        assert!(saved.contains("selection_begin = 8"));
        assert!(!saved.contains("A😀日"));
    }
    #[test]
    fn persistence_reload_failures_preserve_source_history_and_baseline() {
        let fixture = Fixture::new();
        let path = fixture.0.join("note.md");
        fs::write(&path, "disk").unwrap();
        let mut app = fixture.app();
        app.open_path(path.clone());
        app.doc_mut().replace_selection("unsaved").unwrap();
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().undo_count(),
        );
        let baseline = app.tabs[app.active]
            .file
            .as_ref()
            .unwrap()
            .baseline_sha256();
        fs::remove_file(&path).unwrap();
        assert!(app.reload_disk_from(&path).is_err());
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().undo_count()
            ),
            before
        );
        assert_eq!(
            app.tabs[app.active]
                .file
                .as_ref()
                .unwrap()
                .baseline_sha256(),
            baseline
        );
        assert!(app.doc().is_dirty());
        let candidate = app.tabs[app.active].persistence.recovery.as_ref().unwrap();
        assert_eq!(candidate.source, before.0);
        fs::write(&path, "updated disk").unwrap();
        app.reload_disk_from(&path).unwrap();
        assert_eq!(app.doc().source(), "updated disk");
        assert!(!app.doc().is_dirty());
        assert_eq!(
            app.tabs[app.active]
                .persistence
                .recovery
                .as_ref()
                .unwrap()
                .source,
            before.0
        );
        // Failure to create the recovery state must prevent the replacement.
        app.doc_mut().replace_selection("second edit").unwrap();
        app.tabs[app.active].persistence.workspace = None;
        let unsaved = app.doc().source().to_owned();
        fs::write(&path, "third disk").unwrap();
        assert!(app.reload_disk_from(&path).is_err());
        assert_eq!(app.doc().source(), unsaved);
        assert!(app.doc().is_dirty());
    }
    #[test]
    fn persistence_dirty_restart_defers_view_until_exact_recovery_source() {
        let fixture = Fixture::new();
        let path = fixture.0.join("short.md");
        fs::write(&path, "a").unwrap();
        let mut app = fixture.app();
        app.doc_mut().mark_saved();
        app.open_path(path.clone());
        app.persistence.session_writable = true;
        app.persistence.autosave = false;
        app.doc_mut()
            .replace_range(1..1, "😀日本語 dirty body longer than disk")
            .unwrap();
        let end = app.doc().byte_len();
        app.doc_mut()
            .set_selection_with_affinity(end, 1, BoundaryAffinity::Before)
            .unwrap();
        app.tabs[app.active].persistence.view_anchor = Some(5);
        let source = app.doc().source().to_owned();
        let selection = app.doc().selection();
        app.persist_session();
        assert_eq!(fs::read_to_string(&path).unwrap(), "a");
        let session_path = fixture.0.join(".mdlite/.state/session.toml");
        let text = fs::read_to_string(&session_path).unwrap();
        assert!(!text.contains(&source));
        let saved = Session::parse(&text).unwrap();
        assert!(saved.documents[0].recovery_record.is_some());
        let candidate = app.tabs[app.active].persistence.recovery.clone().unwrap();
        let mut restarted = fixture.app();
        restarted.doc_mut().mark_saved();
        restarted.persistence.session_writable = true;
        unsafe {
            restarted.load_session();
        }
        assert!(restarted.persistence.session_writable);
        assert_eq!(restarted.doc().source(), "a");
        assert_eq!(restarted.doc().selection(), Selection::caret(0));
        assert_eq!(restarted.persistence.deferred_views.len(), 1);
        // A periodic rewrite while the user keeps the candidate retains its view.
        restarted.persist_session();
        let pending = Session::parse(&fs::read_to_string(&session_path).unwrap()).unwrap();
        assert_eq!(pending.recovery_views.len(), 1);
        let mut restarted_again = fixture.app();
        restarted_again.doc_mut().mark_saved();
        restarted_again.persistence.session_writable = true;
        unsafe {
            restarted_again.load_session();
        }
        restarted_again.restore_recovery(candidate);
        assert_eq!(restarted_again.doc().source(), source);
        assert_eq!(restarted_again.doc().selection(), selection);
        assert_eq!(
            restarted_again.doc().selection_affinity(),
            BoundaryAffinity::Before
        );
        assert_eq!(
            restarted_again.tabs[restarted_again.active]
                .persistence
                .pending_view
                .as_ref()
                .unwrap()
                .anchor,
            Some(5)
        );
        assert!(restarted_again.persistence.deferred_views.is_empty());
        assert!(restarted_again.persistence.session_writable);
        assert!(
            restarted_again
                .tabs
                .iter()
                .any(|t| t.file.is_some() && t.doc.source() == "a")
        );
    }
    #[test]
    fn persistence_untitled_view_binds_recovery_and_close_no_does_not_recreate() {
        let fixture = Fixture::new();
        let mut app = fixture.app();
        app.persistence.session_writable = true;
        app.doc_mut()
            .replace_selection("A😀日\r\nuntitled")
            .unwrap();
        app.doc_mut().set_selection(8, 1).unwrap();
        let source = app.doc().source().to_owned();
        app.persist_session();
        let candidate = app.tabs[app.active].persistence.recovery.clone().unwrap();
        let session_path = fixture.0.join(".mdlite/.state/session.toml");
        let saved = Session::parse(&fs::read_to_string(&session_path).unwrap()).unwrap();
        assert!(saved.documents.is_empty());
        assert_eq!(saved.recovery_views.len(), 1);
        let mut restarted = fixture.app();
        restarted.doc_mut().mark_saved();
        restarted.persistence.session_writable = true;
        unsafe {
            restarted.load_session();
        }
        restarted.restore_recovery(candidate);
        assert_eq!(restarted.doc().source(), source);
        assert_eq!(
            restarted.doc().selection(),
            Selection { anchor: 8, head: 1 }
        );
        assert!(restarted.tabs[restarted.active].file.is_none());
        restarted.discard_tab_recovery(restarted.active);
        restarted.persist_session();
        assert!(
            RecoveryStore::open(&fixture.0)
                .unwrap()
                .list()
                .unwrap()
                .records
                .is_empty()
        );
        let saved = Session::parse(&fs::read_to_string(&session_path).unwrap()).unwrap();
        assert!(saved.recovery_views.is_empty());
        // The same explicit discard before the first timer also prevents writes.
        let fresh = Fixture::new();
        let mut blank = fresh.app();
        blank.persistence.session_writable = true;
        blank
            .doc_mut()
            .replace_selection("discard before timer")
            .unwrap();
        blank.discard_tab_recovery(blank.active);
        blank.persist_session();
        assert!(
            RecoveryStore::open(&fresh.0)
                .unwrap()
                .list()
                .unwrap()
                .records
                .is_empty()
        );
    }
    #[test]
    fn persistence_stale_legacy_view_is_local_and_allows_future_session_writes() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("a.md"), "a").unwrap();
        let path = fixture.0.join(".mdlite/.state/session.toml");
        fs::write(&path,"schema_version = 3\n[[document]]\npath = \"a.md\"\nselection_begin = 500\nselection_end = 1\n").unwrap();
        let mut app = fixture.app();
        app.doc_mut().mark_saved();
        app.persistence.session_writable = true;
        unsafe {
            app.load_session();
        }
        assert!(app.persistence.session_writable);
        assert_eq!(app.doc().selection(), Selection::caret(0));
        assert!(app.status.contains("viewを適用できません"));
        app.persist_session();
        let current = Session::parse(&fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(current.version, 4);
        assert_eq!(current.documents[0].anchor, 0);
    }
    #[test]
    fn persistence_horizontal_table_view_captures_latest_scroll_and_restores_range_key() {
        let fixture = Fixture::new();
        let path = fixture.0.join("wide.md");
        let cell = "abcdefghijklmnopqrstuvwxyz0123456789";
        let source = format!(
            "prefix\n\n| {cell} | {cell} | {cell} | {cell} | {cell} |\n|---|---|---|---|---|\n| value | value | value | value | value |\n"
        );
        fs::write(&path, &source).unwrap();
        let mut app = fixture.app();
        app.doc_mut().mark_saved();
        app.open_path(path);
        let window = super::super::tests::hidden_test_window_size(640, 480);
        app.hwnd = window;
        let caret = source.find(cell).unwrap();
        app.doc_mut().set_selection(caret, caret).unwrap();
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert!(app.tables[0].table.range.start > 0);
        app.horizontal_scroll(80.);
        unsafe {
            app.ensure_layout().unwrap();
        }
        app.capture_view_anchor();
        let first = app.tabs[app.active].persistence.horizontal_anchor.unwrap();
        app.horizontal_scroll(130.);
        unsafe {
            app.ensure_layout().unwrap();
        }
        app.capture_view_anchor();
        let second = app.tabs[app.active].persistence.horizontal_anchor.unwrap();
        assert_ne!(first, second);
        let record =
            capture_session_record(&app.tabs[app.active], PathBuf::from("wide.md"), &fixture.0);
        let session = Session {
            documents: vec![record],
            ..Session::default()
        };
        let loaded = Session::parse(&session.serialize().unwrap()).unwrap();
        let view = loaded.hydrate(&loaded.documents[0], app.doc()).unwrap();
        assert_eq!(view.horizontal_anchor, Some(second));
        app.tabs[app.active].table_scroll.clear();
        app.layout_key = None;
        unsafe {
            app.ensure_layout().unwrap();
        }
        app.tabs[app.active].persistence.pending_view = Some(view);
        app.restore_pending_view();
        let key = app.tables[0].table.range.start;
        assert!(app.tabs[app.active].table_scroll[&key] > 100.);
        app.capture_view_anchor();
        assert_eq!(
            app.tabs[app.active].persistence.horizontal_anchor,
            Some(second)
        );
        assert_eq!(app.doc().source(), source);
        assert!(!app.doc().is_dirty());
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
    #[test]
    fn persistence_reload_uses_new_document_incarnation_even_at_revision_zero() {
        let fixture = Fixture::new();
        let path = fixture.0.join("reload.md");
        fs::write(&path, "old source").unwrap();
        let mut app = fixture.app();
        app.doc_mut().mark_saved();
        app.open_path(path.clone());
        let old_id = app.doc().id();
        assert_eq!(app.doc().revision(), 0);
        let old_snapshot = app.doc().snapshot();
        fs::write(&path, "new source").unwrap();
        app.reload_disk_from(&path).unwrap();
        assert_ne!(app.doc().id(), old_id);
        assert_eq!(app.doc().revision(), 0);
        assert_eq!(old_snapshot.id, old_id);
        assert_eq!(old_snapshot.rope().to_string(), "old source");
        assert_eq!(app.doc().source(), "new source");
    }
}
