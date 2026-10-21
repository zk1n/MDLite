//! Win32 owns the window; the UI-thread App owns documents and derived layouts.
//! WindowState lives until the message loop exits. Synchronous native reentry
//! snapshots IME notifications outside App's borrow and drains them in order.

mod control_scroll;
mod control_style;
mod diagnostics;
mod edit_scroll;
mod editing;
mod editor_actions;
mod explorer_icons;
mod features;
mod git_ui;
mod menu_style;
mod persistence;
mod presentation;
mod scrollbar_style;
mod search_ui;
mod settings_dialog;
mod shell;
use diagnostics::DiagnosticsPane;
use editing::{
    enter_edit, indentation_edit, line_start, logical_line_target, physical_line_starts,
    word_target,
};
use editor_actions::EditorActions;
use features::FeaturePanes;
use persistence::{DocumentPersistence, Persistence};
use search_ui::SearchPane;
use shell::NativeShell;

use mdlite::{
    document::{Document, Selection},
    projection::{BlockKind, BoundaryAffinity, DisplayProjection, ProjectionBlock},
    storage::{FileFormat, FileSnapshot},
    table::{Table, TableAlignment, TableEdit, TableRect},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Instant,
};
use unicode_segmentation::UnicodeSegmentation;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{
            Direct2D::Common::*, Direct2D::*, DirectWrite::*, Dwm::*, Dxgi::Common::*, Gdi::*,
        },
        System::{Com::*, DataExchange::*, LibraryLoader::*, Memory::*},
        UI::{
            Controls::Dialogs::*,
            HiDpi::*,
            Input::{Ime::*, KeyboardAndMouse::*},
            Shell::*,
            TextServices::*,
            WindowsAndMessaging::*,
        },
    },
    core::{Interface, PCWSTR, PWSTR, w},
};
use windows_numerics::Vector2;

const TOP: f32 = 88.0;
const LEFT: f32 = 44.0;
const TAB_WIDTH: f32 = 170.0;
const TAB_SCROLL_BUTTON: f32 = 28.0;
const GUTTER_HORIZONTAL_PADDING: f32 = 12.0;
const GUTTER_LABEL_LEFT: f32 = 4.0;
const FONT: f32 = 17.0;
const CF_TEXT_W: u32 = 13;
const CMD_NEW: usize = 1;
const CMD_OPEN: usize = 2;
const CMD_SAVE: usize = 3;
const CMD_SAVE_AS: usize = 4;
const CMD_CLOSE: usize = 5;
const CMD_RAW: usize = 6;
const CMD_WORKSPACE: usize = 7;
const CMD_RECOVERY: usize = 8;
const CMD_COMPARE: usize = 9;
const CMD_RELOAD: usize = 10;
const CMD_DISCARD_RECOVERY: usize = 11;
const ANIMATION_TIMER: usize = 1;
const PERSISTENCE_TIMER: usize = 2;
// Bounded retained image cache, independent of the historical 50 MB app target.
const IMAGE_CACHE_BYTES: u64 = 512 * 1024 * 1024;
const TOOLBAR: [(usize, &str, f32); 6] = [
    (CMD_NEW, "新規", 65.0),
    (CMD_OPEN, "開く", 65.0),
    (CMD_SAVE, "保存", 65.0),
    (CMD_SAVE_AS, "名前を付けて保存", 150.0),
    (CMD_RAW, "ソース / Live", 120.0),
    (CMD_CLOSE, "閉じる", 65.0),
];

struct Tab {
    doc: Document,
    file: Option<FileSnapshot>,
    scroll: f32,
    raw: bool,
    table_scroll: HashMap<usize, f32>,
    persistence: DocumentPersistence,
}

#[derive(Clone, Copy, Debug)]
struct TabStripLayout {
    first: usize,
    visible_count: usize,
    left: f32,
    right: f32,
    tab_width: f32,
    overflow: bool,
}

fn tab_strip_layout(width: f32, tab_count: usize, first: usize) -> TabStripLayout {
    let left = LEFT;
    let right = (width - LEFT).max(left);
    let available = (right - left).max(0.0);
    let fitting_tabs = (available / TAB_WIDTH).floor() as usize;
    let overflow = tab_count > fitting_tabs.max(1);
    if overflow {
        let tabs_left = left + TAB_SCROLL_BUTTON;
        let tabs_right = (right - TAB_SCROLL_BUTTON).max(tabs_left);
        let tabs_width = tabs_right - tabs_left;
        let visible_count = ((tabs_width / TAB_WIDTH).floor() as usize)
            .max(1)
            .min(tab_count);
        let first = first.min(tab_count.saturating_sub(visible_count));
        TabStripLayout {
            first,
            visible_count,
            left: tabs_left,
            right: tabs_right,
            tab_width: (tabs_width / visible_count as f32).min(200.0),
            overflow,
        }
    } else {
        let visible_count = tab_count;
        TabStripLayout {
            first: 0,
            visible_count,
            left,
            right,
            tab_width: if visible_count == 0 {
                TAB_WIDTH
            } else {
                (available / visible_count as f32).min(TAB_WIDTH)
            },
            overflow,
        }
    }
}

fn canonical_or_destination_path(path: &Path) -> Option<PathBuf> {
    if let Ok(path) = std::fs::canonicalize(path) {
        return Some(path);
    }
    let name = path.file_name()?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Some(std::fs::canonicalize(parent).ok()?.join(name))
}

fn same_file_identity(left: &Path, right: &Path) -> bool {
    let left_path = canonical_or_destination_path(left);
    let right_path = canonical_or_destination_path(right);
    if left_path
        .as_ref()
        .zip(right_path.as_ref())
        .is_some_and(|(left, right)| {
            left == right
                || left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase()
        })
    {
        return true;
    }

    let (Some(left), Some(right)) = (windows_file_identity(left), windows_file_identity(right))
    else {
        return false;
    };
    left == right
}

fn windows_file_identity(path: &Path) -> Option<(u32, u64)> {
    use std::os::windows::io::AsRawHandle;

    let file = std::fs::File::open(path).ok()?;
    let mut info = unsafe {
        std::mem::zeroed::<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION>()
    };
    // SAFETY: the handle remains valid for this call and info points to the
    // writable structure expected by GetFileInformationByHandle.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
            file.as_raw_handle() as _,
            &mut info,
        )
    };
    (ok != 0).then_some((
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
    ))
}
struct TextView {
    layout: IDWriteTextLayout,
    text: Vec<u16>,
    map: Vec<usize>,
    before: Vec<usize>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    table: Option<usize>,
    source: Range<usize>,
    marker: Option<(IDWriteTextLayout, f32)>,
    quoted: bool,
    thematic_break: bool,
}
#[derive(Clone, Copy, Debug)]
struct GutterRow {
    view_index: usize,
    y: f32,
    baseline: f32,
    height: f32,
    line_number: usize,
}
struct GutterPresentation {
    marker: Option<(IDWriteTextLayout, f32)>,
    quoted: bool,
    indent: f32,
}
#[derive(Clone, Debug)]
struct VisibleToken {
    spans: smallvec::SmallVec<[Range<usize>; 1]>,
    // Only transformed or split spelling needs a separate display string.
    text: Option<String>,
    display_utf16: Range<usize>,
    atomic: bool,
}
impl VisibleToken {
    fn spelling<'a>(&'a self, source: &'a str) -> &'a str {
        self.text
            .as_deref()
            .unwrap_or_else(|| &source[self.spans[0].clone()])
    }
}
struct LiveEdit {
    range: Range<usize>,
    replacement: String,
    caret: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TableKeyIntent {
    NextCell,
    PreviousCell,
    NextRow,
    PreviousRow,
    Exit,
}
enum TableKeyAction {
    Move(Selection),
    Edit(TableEdit, Selection),
}
struct TableView {
    table: Arc<Table>,
    y: f32,
    height: f32,
    width: f32,
}
struct ImageView {
    bitmap: Option<ID2D1Bitmap>,
    path: Option<PathBuf>,
    source: Range<usize>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    quoted: bool,
    label: String,
    load_failed: bool,
}
struct PendingListItem {
    block: ProjectionBlock,
    children: Vec<ProjectionBlock>,
    cursor: usize,
    next_child: usize,
    waiting_for: Range<usize>,
    gutter_pending: bool,
}
struct CachedImage {
    bitmap: ID2D1Bitmap,
    width: u32,
    height: u32,
    animation: Option<mdlite::assets::AnimatedImage>,
    frame_sequence: u64,
    retained_bytes: u64,
}
struct Graphics {
    factory: ID2D1Factory,
    write: IDWriteFactory,
    target: Option<ID2D1DCRenderTarget>,
}
#[derive(Default, Clone, Copy)]
struct NativeCaretState {
    size: Option<(i32, i32)>,
    position: Option<(i32, i32)>,
    visible: bool,
}
#[derive(Debug, PartialEq, Eq)]
struct CaretChange {
    recreate: bool,
    move_position: bool,
}
struct HiddenCaret(HWND);
impl Drop for HiddenCaret {
    fn drop(&mut self) {
        // SAFETY: the guard is local to a synchronous owner-thread paint; window
        // destruction is deferred while App is borrowed by this callback.
        unsafe {
            let _ = ShowCaret(Some(self.0));
        }
    }
}

// Each frame owns all clips it pushes and ends its draw exactly once, including
// an early DirectWrite/resource error. A failed frame is discarded by WM_PAINT.
struct DrawFrame<'a> {
    target: &'a ID2D1RenderTarget,
    clips: usize,
    ended: bool,
}
#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum PaintFault {
    Target,
    Drawing,
    EndDraw,
    Bind,
}
#[cfg(test)]
#[derive(Default)]
struct PaintTestState {
    fault: Option<PaintFault>,
    failures: usize,
    began: usize,
    ended: usize,
    unwound_clips: usize,
    damage: Option<D2D_RECT_F>,
    bound_surface: Option<usize>,
}
#[cfg(test)]
thread_local! {
    static PAINT_TEST: RefCell<PaintTestState> = RefCell::new(PaintTestState::default());
}
#[cfg(test)]
fn paint_fault(stage: PaintFault) -> windows::core::Result<()> {
    PAINT_TEST.with(|state| {
        let mut state = state.borrow_mut();
        if state.fault == Some(stage) && state.failures != 0 {
            state.failures -= 1;
            Err(E_FAIL.into())
        } else {
            Ok(())
        }
    })
}
impl<'a> DrawFrame<'a> {
    unsafe fn new(target: &'a ID2D1RenderTarget) -> Self {
        unsafe { target.BeginDraw() };
        #[cfg(test)]
        PAINT_TEST.with(|state| state.borrow_mut().began += 1);
        Self {
            target,
            clips: 0,
            ended: false,
        }
    }
    unsafe fn push_clip(&mut self, rect: &D2D_RECT_F, mode: D2D1_ANTIALIAS_MODE) {
        unsafe { self.target.PushAxisAlignedClip(rect, mode) };
        self.clips += 1;
    }
    unsafe fn pop_clip(&mut self) {
        unsafe { self.target.PopAxisAlignedClip() };
        self.clips -= 1;
    }
    unsafe fn finish(&mut self) -> windows::core::Result<()> {
        #[cfg(test)]
        PAINT_TEST.with(|state| state.borrow_mut().unwound_clips += self.clips);
        while self.clips != 0 {
            unsafe { self.pop_clip() };
        }
        self.ended = true;
        let result = unsafe { self.target.EndDraw(None, None) };
        #[cfg(test)]
        {
            PAINT_TEST.with(|state| state.borrow_mut().ended += 1);
            paint_fault(PaintFault::EndDraw)?;
        }
        result
    }
}
impl Drop for DrawFrame<'_> {
    fn drop(&mut self) {
        if !self.ended {
            let _ = unsafe { self.finish() };
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PaintedFrame {
    hwnd: usize,
    document_id: u64,
    revision: u64,
    sequence: u64,
}
struct App {
    hwnd: HWND,
    graphics: Graphics,
    tabs: Vec<Tab>,
    active: usize,
    tab_scroll: usize,
    next_id: u64,
    views: Vec<TextView>,
    source_line_starts: Vec<usize>,
    gutter_rows: Vec<GutterRow>,
    gutter_width: f32,
    visible_tokens: Vec<VisibleToken>,
    tables: Vec<TableView>,
    images: Vec<ImageView>,
    image_cache: HashMap<PathBuf, CachedImage>,
    minimized: bool,
    full_paint: bool,
    painted_frame: Option<PaintedFrame>,
    paint_retry_pending: bool,
    animation_timer_armed: bool,
    height: f32,
    width: f32,
    dpi: f32,
    layout_key: Option<(u64, u64, u32, bool)>,
    projection_cache: Option<(u64, u64, Arc<DisplayProjection>)>,
    parsed_tables: Vec<Arc<Table>>,
    revealed_source: Option<Range<usize>>,
    dragging: bool,
    dragging_scroll: bool,
    preedit: String,
    composing: bool,
    ime_owner: Option<(u64, u64, usize, usize)>,
    ime_affinity: BoundaryAffinity,
    ime_cursor: u32,
    ime_attributes: Vec<u8>,
    ime_last_result: Option<(String, Option<String>)>,
    reconversion_owner: Option<ReconversionOwner>,
    pending_reconversion: Option<ReconversionOwner>,
    ime_ingress: Rc<ImeIngress>,
    ime_session: Option<ImeEnvelope>,
    ime_generation: u64,
    reconversion_cancel: Option<ReconversionCancel>,
    preferred_x: Option<f32>,
    typing: bool,
    last_typing: Option<Instant>,
    pending_table_authoring: Option<(u64, usize)>,
    close_after_composition: Option<(u32, WPARAM, LPARAM)>,
    pending_surrogate: Option<u16>,
    diagnostic: Option<PathBuf>,
    phase_trace: Option<Rc<PhaseTrace>>,
    caret: (f32, f32, f32),
    native_caret: NativeCaretState,
    status: String,
    persistence: Persistence,
    shell: NativeShell,
    features: FeaturePanes,
    explorer_icons: explorer_icons::ExplorerIcons,
    git_ui: git_ui::GitPane,
    search_ui: SearchPane,
    editor_actions: EditorActions,
    diagnostics: DiagnosticsPane,
    close_target: Option<HWND>,
    settings_requested: bool,
    defer_theme_updates: bool,
    theme_update_pending: bool,
    staged_explorer_paint: Option<Option<Rc<explorer_icons::ExplorerPaint>>>,
}

// App is never mutably aliased. Only owned, synchronous notification snapshots
// may be queued while a modal/native callback holds its RefCell borrow.
struct WindowState {
    phase_trace: Option<Rc<PhaseTrace>>,
    app: RefCell<App>,
    notifications: RefCell<VecDeque<(ImeEnvelope, NativeNotification)>>,
    document_id: Cell<u64>,
    ime_ingress: Rc<ImeIngress>,
    main_window: Cell<HWND>,
    editor_window: Cell<HWND>,
    compact_window: Cell<HWND>,
    workspace_message: Cell<u32>,
    deferred_persistence: Cell<bool>,
    settings_open: Cell<bool>,
    deferred_control_redraw: Cell<bool>,
    button_paint: RefCell<Option<Rc<shell::OwnerButtonPaint>>>,
    shell_button_paint: RefCell<Option<Rc<shell::ShellButtonPaint>>>,
    git_button_paint: RefCell<Option<Rc<Vec<shell::OwnerButtonPaint>>>>,
    control_colors: RefCell<Option<Rc<shell::ControlColors>>>,
    menu_paint: RefCell<Vec<Rc<menu_style::MenuPaint>>>,
    explorer_paint: features::ExplorerPaintSlot,
    deferred_paints: Cell<[Option<(HWND, RECT)>; 2]>,
    ime_input: Cell<Option<ImeInputContext>>,
    ime_dispatch_key: Cell<Option<(HWND, u32, usize, isize)>>,
}
impl WindowState {
    fn new(app: App) -> Self {
        let document_id = Cell::new(app.doc().id());
        let ime_ingress = Rc::clone(&app.ime_ingress);
        let phase_trace = app.phase_trace.clone();
        let explorer_paint = app.features.explorer_paint_handle();
        Self {
            phase_trace,
            app: RefCell::new(app),
            notifications: RefCell::new(VecDeque::new()),
            document_id,
            ime_ingress,
            main_window: Cell::new(HWND::default()),
            editor_window: Cell::new(HWND::default()),
            compact_window: Cell::new(HWND::default()),
            workspace_message: Cell::new(0),
            deferred_persistence: Cell::new(false),
            settings_open: Cell::new(false),
            deferred_control_redraw: Cell::new(false),
            button_paint: RefCell::new(None),
            shell_button_paint: RefCell::new(None),
            git_button_paint: RefCell::new(None),
            control_colors: RefCell::new(None),
            menu_paint: RefCell::new(Vec::new()),
            explorer_paint,
            deferred_paints: Cell::new([None, None]),
            ime_input: Cell::new(None),
            ime_dispatch_key: Cell::new(None),
        }
    }
    fn ime_input_for_ingress(
        &self,
        captured: Option<ImeEnvelope>,
        input: Option<ImeInputContext>,
    ) -> Option<ImeInputContext> {
        // Key recovery predates draining and pretranslation. A fresh session
        // cannot inherit the recovered key's synchronous Escape attribution.
        input.filter(|_| self.ime_ingress.current.get() == captured)
    }
    fn scoped_ime_input(&self, input: Option<ImeInputContext>, message: &MSG) -> ImeInputScope<'_> {
        self.ime_input.set(input);
        self.ime_dispatch_key.set(input.map(|_| {
            (
                message.hwnd,
                message.message,
                message.wParam.0,
                message.lParam.0,
            )
        }));
        ImeInputScope(self)
    }
    fn observe_ime_input_message(&self, hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) {
        if hwnd != self.editor_window.get() || ime_input_resets(msg) {
            self.ime_input.set(None);
            self.ime_dispatch_key.set(None);
        } else if matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN | WM_IME_KEYDOWN)
            && self.ime_dispatch_key.take() != Some((hwnd, msg, wp.0, lp.0))
        {
            // Permit the outer DispatchMessage key once. Any nested/generated
            // other key makes subsequent callback attribution unknown.
            self.ime_input.set(None);
        } else if !matches!(
            msg,
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_IME_KEYDOWN | WM_IME_COMPOSITION | WM_IME_REQUEST
        ) {
            // A non-IME callback can enter an unrelated nested message pump.
            // Conservatively invalidate rather than restore its outer intent.
            self.ime_input.set(None);
            self.ime_dispatch_key.set(None);
        }
    }
    fn defer_paint(&self, hwnd: HWND, rect: RECT) {
        if rect.left >= rect.right || rect.top >= rect.bottom {
            return;
        }
        let index = if hwnd == self.main_window.get() {
            0
        } else if hwnd == self.compact_window.get() {
            1
        } else {
            return;
        };
        let mut paints = self.deferred_paints.get();
        let rect = match paints[index] {
            Some((owner, previous)) if owner == hwnd => RECT {
                left: previous.left.min(rect.left),
                top: previous.top.min(rect.top),
                right: previous.right.max(rect.right),
                bottom: previous.bottom.max(rect.bottom),
            },
            _ => rect,
        };
        paints[index] = Some((hwnd, rect));
        self.deferred_paints.set(paints);
    }
    unsafe fn owns_paint_surface(&self, hwnd: HWND) -> bool {
        unsafe {
            (hwnd == self.main_window.get() || hwnd == self.compact_window.get())
                && IsWindow(Some(hwnd)).as_bool()
                && GetWindowLongPtrW(hwnd, GWLP_USERDATA) == self as *const WindowState as isize
        }
    }
}

// Opt-in, constant-size phase records. No document text, paths or maps enter
// this sidecar; never overwrite an earlier run's evidence or affect editing.
struct PhaseTrace {
    started: Instant,
    file: RefCell<std::fs::File>,
    dropped: Cell<u64>,
}
impl PhaseTrace {
    fn open(diagnostic: &Path) -> Option<Rc<Self>> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(diagnostic.with_extension("phases.jsonl"))
            .ok()?;
        let trace = Rc::new(Self {
            started: Instant::now(),
            file: RefCell::new(file),
            dropped: Cell::new(0),
        });
        trace.record("trace", "opened");
        Some(trace)
    }
    fn record(&self, phase: &'static str, event: &'static str) {
        use std::io::Write;
        let Ok(mut file) = self.file.try_borrow_mut() else {
            self.dropped.set(self.dropped.get().saturating_add(1));
            return;
        };
        if writeln!(
            file,
            "{{\"t_us\":{},\"phase\":\"{}\",\"event\":\"{}\",\"dropped_total\":{}}}",
            self.started.elapsed().as_micros(),
            phase,
            event,
            self.dropped.get()
        )
        .is_err()
        {
            self.dropped.set(self.dropped.get().saturating_add(1));
        }
    }
}
fn record_phase(trace: &Option<Rc<PhaseTrace>>, phase: &'static str, event: &'static str) {
    if let Some(trace) = trace {
        trace.record(phase, event);
    }
}
struct PhaseSpan {
    trace: Option<Rc<PhaseTrace>>,
    phase: &'static str,
}
impl PhaseSpan {
    fn new(trace: &Option<Rc<PhaseTrace>>, phase: &'static str) -> Self {
        record_phase(trace, phase, "begin");
        Self {
            trace: trace.clone(),
            phase,
        }
    }
}
impl Drop for PhaseSpan {
    fn drop(&mut self) {
        record_phase(&self.trace, self.phase, "end");
    }
}

struct ButtonPaintScope<'a> {
    state: &'a WindowState,
    paint: Rc<shell::OwnerButtonPaint>,
}
impl<'a> ButtonPaintScope<'a> {
    fn new(state: &'a WindowState, paint: shell::OwnerButtonPaint) -> Self {
        let paint = Rc::new(paint);
        *state.button_paint.borrow_mut() = Some(Rc::clone(&paint));
        Self { state, paint }
    }
}
impl Drop for ButtonPaintScope<'_> {
    fn drop(&mut self) {
        let mut active = self.state.button_paint.borrow_mut();
        if active
            .as_ref()
            .is_some_and(|paint| Rc::ptr_eq(paint, &self.paint))
        {
            *active = None;
        }
        // A nested native operation supersedes the outer context permanently.
        // Never restore a snapshot whose control may have changed or closed.
    }
}

unsafe fn replay_deferred_paints(state: &WindowState) {
    // Call only after App's borrow has ended. No synchronous paint or refresh:
    // the ordinary message loop renders the accumulated owned dirty regions.
    unsafe {
        for (hwnd, rect) in state
            .deferred_paints
            .replace([None, None])
            .into_iter()
            .flatten()
        {
            if state.owns_paint_surface(hwnd) {
                let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
            }
        }
    }
}
struct ImeInputScope<'a>(&'a WindowState);
impl Drop for ImeInputScope<'_> {
    fn drop(&mut self) {
        // Never restore an outer key after a nested pump or scope invalidation.
        self.0.ime_input.set(None);
        self.0.ime_dispatch_key.set(None);
    }
}
// Local ingress freshness only: IMM carries no native session identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ImeEnvelope {
    document_id: u64,
    editor: HWND,
    generation: u64,
}
#[derive(Default)]
struct ImeIngress {
    next: Cell<u64>,
    current: Cell<Option<ImeEnvelope>>,
}
impl ImeIngress {
    fn allocate(&self) -> u64 {
        let next = self
            .next
            .get()
            .checked_add(1)
            .expect("IME generation exhausted");
        self.next.set(next);
        next
    }
    fn start(&self, document_id: u64, editor: HWND) -> ImeEnvelope {
        let envelope = ImeEnvelope {
            document_id,
            editor,
            generation: self.allocate(),
        };
        self.current.set(Some(envelope));
        envelope
    }
    fn retire(&self, envelope: ImeEnvelope) {
        if self.current.get() == Some(envelope) {
            self.current.set(None);
        }
    }
    fn capture(&self, document_id: u64, editor: HWND, message: u32) -> ImeEnvelope {
        if message == WM_IME_STARTCOMPOSITION {
            return self.start(document_id, editor);
        }
        let envelope = self
            .current
            .get()
            .filter(|envelope| envelope.document_id == document_id && envelope.editor == editor)
            .unwrap_or(ImeEnvelope {
                document_id,
                editor,
                generation: 0,
            });
        if message == WM_IME_ENDCOMPOSITION {
            self.retire(envelope);
        }
        envelope
    }
}
#[derive(Clone, Copy)]
struct ReconversionCancel {
    envelope: ImeEnvelope,
    owner: ReconversionOwner,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ImeCancelOutcome {
    Cancelled,
    NoContext,
    Rejected,
}

enum NativeNotification {
    Start,
    Update(ImeUpdate),
    End,
    FocusLost,
}
#[derive(Default)]
struct ImeUpdate {
    flags: u32,
    result: Option<String>,
    preedit: Option<String>,
    attributes: Option<Vec<u8>>,
    cursor: Option<u32>,
    input_context: Option<ImeInputContext>,
}

#[derive(Clone, Copy)]
struct ImeInputContext {
    document_id: u64,
    virtual_key: u32,
}
#[derive(Clone, Copy)]
struct ReconversionOwner {
    request_id: u64,
    editor: HWND,
    generation: Option<u64>,
    document_id: u64,
    revision: u64,
    selection: Selection,
    affinity: BoundaryAffinity,
}
impl ReconversionOwner {
    fn matches(&self, document: &Document) -> bool {
        self.document_id == document.id()
            && self.revision == document.revision()
            && self.selection == document.selection()
            && self.affinity == document.selection_affinity()
    }
}
fn ime_input_resets(message: u32) -> bool {
    matches!(message, WM_MOUSEFIRST..=WM_MOUSELAST | WM_NCMOUSEMOVE..=WM_NCXBUTTONDBLCLK
        | WM_SETFOCUS | WM_KILLFOCUS | WM_ACTIVATE | WM_ACTIVATEAPP
        | WM_CANCELMODE | WM_CAPTURECHANGED | WM_COMMAND | WM_SYSCOMMAND
        | WM_IME_STARTCOMPOSITION | WM_IME_ENDCOMPOSITION)
}

struct CompositionVisual {
    layout: IDWriteTextLayout,
    range: Range<u32>,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    view_index: usize,
    end_view_index: usize,
    growth: f32,
    row_bottom: f32,
}

// The current editor styles are the DirectWrite font/decorations below. Copy
// runs, not individual characters, including the insertion point's font.
unsafe fn copy_layout_styles(
    from: &IDWriteTextLayout,
    to: &IDWriteTextLayout,
    source: Range<u32>,
    destination: u32,
    repeated_length: Option<u32>,
) -> windows::core::Result<()> {
    unsafe {
        let mut position = source.start;
        while position < source.end {
            let mut end = source.end;
            let mut run = DWRITE_TEXT_RANGE::default();
            let mut weight = DWRITE_FONT_WEIGHT::default();
            from.GetFontWeight(position, &mut weight, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            let mut style = DWRITE_FONT_STYLE::default();
            from.GetFontStyle(position, &mut style, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            let mut size = 0.0;
            from.GetFontSize(position, &mut size, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            let mut family_len = 0;
            from.GetFontFamilyNameLength(position, &mut family_len, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            let mut family = vec![0; family_len as usize + 1];
            from.GetFontFamilyName(position, &mut family, None)?;
            let mut underline = windows::core::BOOL::default();
            from.GetUnderline(position, &mut underline, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            let mut strike = windows::core::BOOL::default();
            from.GetStrikethrough(position, &mut strike, Some(&mut run))?;
            end = end.min(run.startPosition.saturating_add(run.length));
            end = end.max(position + 1).min(source.end);
            let range = DWRITE_TEXT_RANGE {
                startPosition: destination + position - source.start,
                length: repeated_length.unwrap_or(end - position),
            };
            to.SetFontWeight(weight, range)?;
            to.SetFontStyle(style, range)?;
            to.SetFontSize(size, range)?;
            to.SetFontFamilyName(PCWSTR(family.as_ptr()), range)?;
            to.SetUnderline(underline.as_bool(), range)?;
            to.SetStrikethrough(strike.as_bool(), range)?;
            position = end;
        }
        Ok(())
    }
}
fn layout_caret(
    layout: &IDWriteTextLayout,
    position: u32,
    trailing: bool,
) -> Option<(f32, f32, f32)> {
    let mut x = 0.0;
    let mut y = 0.0;
    let mut hit = DWRITE_HIT_TEST_METRICS::default();
    unsafe {
        layout
            .HitTestTextPosition(position, trailing, &mut x, &mut y, &mut hit)
            .ok()?;
        let mut method = DWRITE_LINE_SPACING_METHOD::default();
        let mut spacing = 0.0;
        let mut baseline = 0.0;
        layout
            .GetLineSpacing(&mut method, &mut spacing, &mut baseline)
            .ok()?;
        // Editor formats use uniform native line spacing. Read that format
        // instead of collecting all visual rows every time the caret moves.
        Some((
            x,
            y,
            if method == DWRITE_LINE_SPACING_METHOD_UNIFORM {
                spacing
            } else {
                hit.height
            },
        ))
    }
}

unsafe fn read_ime_update(hwnd: HWND, flags: u32) -> ImeUpdate {
    unsafe {
        let mut update = ImeUpdate {
            flags,
            ..Default::default()
        };
        if composition_is_cancelled(flags) {
            return update;
        }
        let context = ImmGetContext(hwnd);
        if context.0.is_null() {
            return update;
        }
        let read = |kind: IME_COMPOSITION_STRING| -> Option<Vec<u8>> {
            let length = ImmGetCompositionStringW(context, kind, None, 0);
            if length < 0 {
                return None;
            }
            let mut bytes = vec![0; length as usize];
            if length > 0 {
                let actual = ImmGetCompositionStringW(
                    context,
                    kind,
                    Some(bytes.as_mut_ptr().cast()),
                    length as u32,
                );
                if actual < 0 {
                    return None;
                }
                bytes.truncate(actual as usize);
            }
            Some(bytes)
        };
        let text = |kind| {
            read(kind).map(|bytes| {
                String::from_utf16_lossy(
                    &bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                        .collect::<Vec<_>>(),
                )
            })
        };
        if flags & GCS_RESULTSTR.0 != 0 {
            update.result = text(GCS_RESULTSTR);
        }
        if flags & GCS_COMPSTR.0 != 0 {
            update.preedit = text(GCS_COMPSTR);
        }
        if flags & (GCS_COMPATTR.0 | GCS_COMPSTR.0) != 0 {
            update.attributes = read(GCS_COMPATTR);
        }
        if flags & GCS_CURSORPOS.0 != 0 {
            let cursor = ImmGetCompositionStringW(context, GCS_CURSORPOS, None, 0);
            if cursor >= 0 {
                update.cursor = Some(cursor as u32);
            }
        }
        let _ = ImmReleaseContext(hwnd, context);
        update
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn draw_gdi_text(dc: HDC, text: &str, area: &mut RECT, format: DRAW_TEXT_FORMAT) -> i32 {
    if text.is_empty() {
        // DrawTextW may inspect the pointer even with a zero character count;
        // an empty Vec's dangling pointer is not a native empty string.
        return 0;
    }
    let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
    unsafe { DrawTextW(dc, &mut utf16, area, format) }
}
fn color(r: f32, g: f32, b: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a: 1.0 }
}
fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
fn point(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}
fn caret_pixel_size(height: f32, dpi: f32) -> (i32, i32) {
    (
        (2.0 * dpi).round().max(1.0) as i32,
        (height * dpi).round().max(1.0) as i32,
    )
}
fn required_caret_change(
    current: NativeCaretState,
    size: (i32, i32),
    position: (i32, i32),
) -> CaretChange {
    let recreate = current.size != Some(size);
    CaretChange {
        recreate,
        move_position: recreate || current.position != Some(position),
    }
}
fn native_caret_owner() -> Option<HWND> {
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    // A thread has one caret, which native EDIT controls manage themselves.
    // Thread zero would inspect the foreground thread instead of this owner.
    unsafe {
        GetGUIThreadInfo(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            &mut info,
        )
        .ok()?;
    }
    Some(info.hwndCaret)
}
fn caret_intersects_damage(caret: (f32, f32, f32), damage: D2D_RECT_F) -> bool {
    damage.left <= caret.0 + 2.0
        && damage.right >= caret.0
        && damage.top <= caret.1 + caret.2
        && damage.bottom >= caret.1
}
fn caret_in_editor(caret: (f32, f32, f32), left: f32, top: f32, right: f32, bottom: f32) -> bool {
    caret.0 >= left && caret.0 + 2.0 <= right && caret.1 >= top && caret.1 + caret.2 <= bottom
}
fn visible_table_offset(current: f32, caret_x: f32, viewport: f32, content: f32) -> f32 {
    let margin = 6.0_f32.min(viewport / 2.0);
    let mut offset = current;
    if caret_x - current < margin {
        offset = caret_x - margin;
    } else if caret_x - current + 2.0 > viewport - margin {
        offset = caret_x + 2.0 - (viewport - margin);
    }
    offset.clamp(0.0, (content - viewport).max(0.0))
}

fn startup_window_state(
    graphics: Graphics,
    diagnostic: Option<PathBuf>,
    paths: Vec<PathBuf>,
) -> WindowState {
    let phase_trace = diagnostic.as_deref().and_then(PhaseTrace::open);
    let mut app = App {
        hwnd: HWND::default(),
        graphics,
        tabs: Vec::new(),
        active: 0,
        tab_scroll: 0,
        next_id: 1,
        views: Vec::new(),
        source_line_starts: Vec::new(),
        gutter_rows: Vec::new(),
        gutter_width: 0.0,
        visible_tokens: Vec::new(),
        tables: Vec::new(),
        images: Vec::new(),
        image_cache: HashMap::new(),
        minimized: false,
        full_paint: true,
        painted_frame: None,
        paint_retry_pending: false,
        animation_timer_armed: false,
        height: 0.0,
        width: 900.0,
        dpi: 1.0,
        layout_key: None,
        projection_cache: None,
        parsed_tables: Vec::new(),
        revealed_source: None,
        dragging: false,
        dragging_scroll: false,
        preedit: String::new(),
        composing: false,
        ime_owner: None,
        ime_affinity: BoundaryAffinity::After,
        ime_cursor: 0,
        ime_attributes: Vec::new(),
        ime_last_result: None,
        reconversion_owner: None,
        pending_reconversion: None,
        ime_ingress: Rc::default(),
        ime_session: None,
        ime_generation: 0,
        reconversion_cancel: None,
        preferred_x: None,
        typing: false,
        last_typing: None,
        pending_table_authoring: None,
        close_after_composition: None,
        pending_surrogate: None,
        diagnostic,
        phase_trace,
        caret: (LEFT, TOP, 28.0),
        native_caret: NativeCaretState::default(),
        status: String::new(),
        persistence: Persistence::default(),
        shell: NativeShell::default(),
        features: FeaturePanes::default(),
        explorer_icons: explorer_icons::ExplorerIcons::default(),
        git_ui: git_ui::GitPane::default(),
        search_ui: SearchPane::default(),
        editor_actions: EditorActions::default(),
        diagnostics: DiagnosticsPane::default(),
        close_target: None,
        settings_requested: false,
        defer_theme_updates: false,
        theme_update_pending: false,
        staged_explorer_paint: None,
    };
    for path in paths {
        if path.is_dir() {
            app.persistence.startup_workspace = Some(path);
        } else {
            app.open_path(path);
        }
    }
    if app.tabs.is_empty() {
        app.new_document();
    }
    WindowState::new(app)
}

pub fn run() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW)?,
            lpszClassName: w!("MDLiteRustEditor"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let graphics = Graphics {
            factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?,
            write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?,
            target: None,
        };
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        let mut diagnostic = None;
        let mut paths = Vec::new();
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--diagnostic-path" {
                i += 1;
                diagnostic = args.get(i).map(PathBuf::from);
            } else {
                paths.push(PathBuf::from(&args[i]));
            }
            i += 1;
        }
        let mut app = Box::new(startup_window_state(graphics, diagnostic, paths));
        let pointer = (&*app as *const WindowState).cast();
        let hwnd = CreateWindowExW(
            // Direct2D presents through the paint DC. Ancestor compositing
            // repeatedly invalidates that surface after present; native child
            // controls retain their own existing local paint buffers.
            WINDOW_EX_STYLE::default(),
            w!("MDLiteRustEditor"),
            w!("MDLite — Rust 編集基盤"),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            800,
            None,
            None,
            Some(instance.into()),
            Some(pointer),
        )?;
        app.app.get_mut().hwnd = hwnd;
        let dark = windows::core::BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const windows::core::BOOL).cast(),
            std::mem::size_of_val(&dark) as u32,
        );
        app.app.get_mut().dpi = GetDpiForWindow(hwnd) as f32 / 96.0;
        {
            // Startup creates controls and modal native dialogs too. Keep the
            // same reentry guard used by later commands around every callback.
            let mut startup = app.app.borrow_mut();
            startup.start_shell();
            startup.start_persistence();
            startup.features_init();
            startup.git_init();
            startup.search_init();
            startup.editor_actions_init();
            // Initial child placement/synchronization can send many native
            // callbacks. Complete it while hidden so focus/size on ShowWindow
            // cannot leave a visible, unpainted parent waiting for that work.
            let _ = startup.ensure_layout();
        }
        replay_deferred_paints(&app);
        DragAcceptFiles(hwnd, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let key_down = matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN);
            let ingress = app.ime_ingress.current.get();
            let input = if key_down {
                let owned_editor =
                    message.hwnd == app.editor_window.get() && message.hwnd != HWND::default();
                // IMM can recover PROCESSKEY only before TranslateMessage or
                // any helper that may translate/pump this input. Keep MSG intact.
                let key = if owned_editor
                    && message.message == WM_KEYDOWN
                    && message.wParam.0 == VK_PROCESSKEY.0 as usize
                {
                    ImmGetVirtualKey(message.hwnd)
                } else {
                    message.wParam.0 as u32
                };
                owned_editor.then_some(ImeInputContext {
                    document_id: app.document_id.get(),
                    virtual_key: key,
                })
            } else {
                None
            };
            // Lifecycle callbacks queued during pretranslation must be applied
            // before the next key is classified by any input owner.
            drain_native_notifications(&app, &mut app.app.borrow_mut());
            if key_down {
                let mut owner = app.app.borrow_mut();
                owner.features_reset_calendar_click();
                owner.invalidate_stale_reconversion();
                if let Some(cancel) =
                    owner.prepare_reconversion_cancel(ingress, input, message.hwnd)
                {
                    perform_reconversion_cancel(&app, &mut owner, cancel, |editor| {
                        native_cancel_ime(editor)
                    });
                    continue;
                }
            }
            replay_deferred_paints(&app);
            let handled = app.app.borrow_mut().shell_message(&message);
            settings_dialog::run_requested(&app);
            replay_deferred_paints(&app);
            if handled {
                continue;
            }
            let handled = app.app.borrow_mut().features_message(&message);
            replay_deferred_paints(&app);
            if handled {
                continue;
            }
            let handled = app.app.borrow_mut().git_message(&message);
            replay_deferred_paints(&app);
            if handled {
                continue;
            }
            let handled = app.app.borrow_mut().search_message(&message);
            replay_deferred_paints(&app);
            if handled {
                continue;
            }
            let dialog_owner = app.app.borrow().shell_dialog_owner(message.hwnd);
            if let Some(owner) = dialog_owner
                && IsDialogMessageW(owner, &message).as_bool()
            {
                continue;
            }
            {
                // Retrieval, draining and pretranslation helpers have no
                // eligible key context. Only this dispatch interval may attach
                // the captured key to a synchronously received IME update.
                let input = app.ime_input_for_ingress(ingress, input);
                let _input_scope = app.scoped_ime_input(input, &message);
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        // NCDESTROY has cleared userdata; the last callback has returned.
        let phase_trace = app.phase_trace.clone();
        record_phase(&phase_trace, "message_loop", "ended");
        {
            let _phase = PhaseSpan::new(&phase_trace, "app.drop");
            drop(app);
        }
        {
            let _phase = PhaseSpan::new(&phase_trace, "com.uninitialize");
            CoUninitialize();
        }
        Ok(())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            let state = &*(cs.lpCreateParams as *const WindowState);
            if state.main_window.get() == HWND::default() {
                state.main_window.set(hwnd);
                state.editor_window.set(hwnd);
            } else if cs.hwndParent == state.main_window.get()
                && state.compact_window.get() == HWND::default()
            {
                // Compact creation reenters before App can set its input owner.
                // Register the paint surface independently of editor_window.
                state.compact_window.set(hwnd);
            }
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowState;
        if msg == WM_NCDESTROY {
            if !pointer.is_null() {
                let state = &*pointer;
                if state.main_window.get() == hwnd {
                    record_phase(&state.phase_trace, "main.destroy", "nc_destroy");
                    state.shell_button_paint.borrow_mut().take();
                    state.git_button_paint.borrow_mut().take();
                    let retired = state.explorer_paint.replace(None);
                    drop(retired);
                }
                state.button_paint.borrow_mut().take();
                if state.compact_window.get() == hwnd {
                    state.compact_window.set(HWND::default());
                    let mut paints = state.deferred_paints.get();
                    paints[1] = None;
                    state.deferred_paints.set(paints);
                }
            }
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let state = &*pointer;
        // Popup tracking pumps messages while App is already mutably borrowed.
        // These snapshots contain no document/App references.
        if let Some(result) = menu_style::window_message(state, hwnd, msg, wp, lp) {
            return result;
        }
        if msg == WM_CLOSE {
            record_phase(&state.phase_trace, "close", "received");
        }
        state.observe_ime_input_message(hwnd, msg, wp, lp);
        if ((msg == WM_ACTIVATE && matches!(wp.0 & 0xffff, 1 | 2) && (wp.0 >> 16) & 0xffff == 0)
            || (msg == WM_NCACTIVATE && wp.0 != 0 && !IsIconic(hwnd).as_bool()))
            && state.owns_paint_surface(hwnd)
        {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        if msg == WM_SYSCOMMAND && wp.0 & 0xfff0 == SC_CLOSE as usize {
            // DefWindowProc sends WM_CLOSE synchronously for system Close,
            // including Alt+F4. Post it so the normal unsaved/IME gate can
            // borrow App after the originating callback has returned.
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
            return LRESULT(0);
        }
        // Workspace activation must work during a modal borrow without touching
        // documents or inventing IME lifecycle notifications.
        if msg != 0 && msg == state.workspace_message.get() && hwnd == state.main_window.get() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let popup = GetLastActivePopup(hwnd);
            let target = if popup != HWND::default() && IsWindowVisible(popup).as_bool() {
                popup
            } else {
                hwnd
            };
            let activated =
                SetForegroundWindow(target).as_bool() || GetForegroundWindow() == target;
            return LRESULT(isize::from(activated));
        }
        if msg == WM_DESTROY {
            if state.editor_window.get() == hwnd {
                if let Some(envelope) = state
                    .ime_ingress
                    .current
                    .get()
                    .filter(|envelope| envelope.editor == hwnd)
                {
                    state.ime_ingress.retire(envelope);
                    state
                        .notifications
                        .borrow_mut()
                        .push_back((envelope, NativeNotification::End));
                }
                if let Ok(mut app) = state.app.try_borrow_mut()
                    && app.editing_window() == hwnd
                {
                    app.pending_reconversion = None;
                    app.clear_composition();
                }
            }
            if state.main_window.get() == hwnd {
                record_phase(&state.phase_trace, "main.destroy", "post_quit");
                PostQuitMessage(0);
            }
            return LRESULT(0);
        }
        // A Settings modal no longer owns App across its nested message pump.
        // Keep its former input/timer semantics, but let native owner painting
        // use the ordinary App route rather than consume blank child draws.
        let app_borrow = if state.settings_open.get()
            && !matches!(
                msg,
                WM_PAINT
                    | WM_ERASEBKGND
                    | WM_DRAWITEM
                    | WM_MEASUREITEM
                    | WM_CTLCOLORSTATIC
                    | WM_CTLCOLOREDIT
                    | WM_CTLCOLORLISTBOX
                    | WM_CTLCOLORBTN
                    | WM_CTLCOLORDLG
                    | WM_CTLCOLORSCROLLBAR
            ) {
            None
        } else {
            state.app.try_borrow_mut().ok()
        };
        let queue_position = state.notifications.borrow().len();
        let envelope = if (state.editor_window.get() == HWND::default()
            || state.editor_window.get() == hwnd)
            && matches!(
                msg,
                WM_IME_STARTCOMPOSITION | WM_IME_COMPOSITION | WM_IME_ENDCOMPOSITION | WM_KILLFOCUS
            ) {
            Some(
                state
                    .ime_ingress
                    .capture(state.document_id.get(), hwnd, msg),
            )
        } else {
            None
        };
        let notification = match msg {
            WM_IME_STARTCOMPOSITION => Some(NativeNotification::Start),
            WM_IME_COMPOSITION => {
                let input_context = state.ime_input.get();
                let mut update = read_ime_update(hwnd, lp.0 as u32);
                update.input_context = input_context;
                Some(NativeNotification::Update(update))
            }
            WM_IME_ENDCOMPOSITION => Some(NativeNotification::End),
            WM_KILLFOCUS => Some(NativeNotification::FocusLost),
            _ => None,
        };
        let notification =
            if state.editor_window.get() != HWND::default() && state.editor_window.get() != hwnd {
                None
            } else {
                notification
            };
        if let Some(notification) = notification {
            // An IMM read may reenter. Insert ahead of notifications captured
            // by that read, without holding the queue borrow across native work.
            state
                .notifications
                .borrow_mut()
                .insert(queue_position, (envelope.unwrap(), notification));
        }
        let Some(mut app) = app_borrow else {
            if msg == WM_CLOSE {
                record_phase(&state.phase_trace, "close", "app_unavailable_consumed");
            }
            if matches!(
                msg,
                WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN
            ) && state.owns_paint_surface(hwnd)
                && IsChild(hwnd, HWND(lp.0 as *mut _)).as_bool()
            {
                let colors = state.control_colors.borrow().clone();
                if let Some(colors) = colors {
                    return colors.paint(wp);
                }
            }
            if msg == WM_DRAWITEM && lp.0 != 0 {
                let paint = state.shell_button_paint.borrow().clone();
                if let Some(paint) = paint
                    && state.owns_paint_surface(hwnd)
                    && paint.draw(
                        hwnd,
                        wp.0,
                        &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                    )
                {
                    return LRESULT(1);
                }
                if hwnd == state.main_window.get()
                    && state.owns_paint_surface(hwnd)
                    && wp.0 == features::EXPLORER_LIST
                {
                    let item = &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT);
                    if item.CtlID as usize == features::EXPLORER_LIST
                        && item.CtlType == windows::Win32::UI::Controls::ODT_LISTBOX
                        && GetDlgCtrlID(item.hwndItem) as usize == features::EXPLORER_LIST
                        && GetParent(item.hwndItem).ok() == Some(hwnd)
                    {
                        let paint = state.explorer_paint.borrow().clone();
                        if let Some(paint) = paint
                            && paint.draw(item, control_scroll::horizontal_offset(item.hwndItem))
                        {
                            return LRESULT(1);
                        }
                    }
                }
                if state.owns_paint_surface(hwnd) {
                    let colors = state.control_colors.borrow().clone();
                    if let Some(colors) = colors
                        && (colors.draw_container(
                            hwnd,
                            wp.0,
                            &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                        ) || (hwnd == state.main_window.get()
                            && search_ui::draw_owned_button(
                                hwnd,
                                wp.0,
                                &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                                colors.tokens,
                            )))
                    {
                        return LRESULT(1);
                    }
                }
                let paints = state.git_button_paint.borrow().clone();
                if let Some(paints) = paints
                    && state.owns_paint_surface(hwnd)
                {
                    let item = &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT);
                    for paint in paints.iter() {
                        if let Some(button) = paint.state(hwnd, wp.0, item) {
                            shell::draw_owner_button(paint.tokens, item, &button);
                            return LRESULT(1);
                        }
                    }
                }
                // Keep IME observation above this branch unchanged. Clone the
                // owned paint data and release its RefCell before any GDI call.
                let paint = state.button_paint.borrow().clone();
                if let Some(paint) = paint
                    && state.owns_paint_surface(hwnd)
                    && let Some(button) = paint.state(
                        hwnd,
                        wp.0,
                        &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                    )
                {
                    shell::draw_owner_button(
                        paint.tokens,
                        &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                        &button,
                    );
                    return LRESULT(1);
                }
            }
            if msg == WM_PAINT {
                // Begin/EndPaint validates this delivery without leaving a
                // spinning WM_PAINT in a nested native/COM message pump. Retain
                // its damage rather than let DefWindowProc lose the frame.
                let mut paint = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut paint);
                state.defer_paint(hwnd, paint.rcPaint);
                let _ = EndPaint(hwnd, &paint);
                return LRESULT(0);
            }
            if matches!(
                msg,
                WM_DRAWITEM
                    | WM_CTLCOLORSTATIC
                    | WM_CTLCOLOREDIT
                    | WM_CTLCOLORLISTBOX
                    | WM_CTLCOLORBTN
                    | WM_CTLCOLORDLG
                    | WM_CTLCOLORSCROLLBAR
            ) {
                // Modal callbacks may temporarily use the default child paint.
                // Invalidate those children once their themed owner is available.
                state.deferred_control_redraw.set(true);
            }
            if msg == WM_TIMER && wp.0 == PERSISTENCE_TIMER {
                state.deferred_persistence.set(true);
            }
            if envelope.is_some() {
                return LRESULT(0);
            }
            if matches!(
                msg,
                WM_CLOSE
                    | WM_COMMAND
                    | WM_KEYDOWN
                    | WM_CHAR
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONDBLCLK
                    | WM_DROPFILES
                    | WM_IME_REQUEST
            ) {
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        if !state.settings_open.get() && state.deferred_persistence.replace(false) {
            app.rearm_autosave(true);
        }
        state.document_id.set(app.doc().id());
        let surface = if hwnd == app.shell.main_window
            && app.shell.compact_window != HWND::default()
            && matches!(
                msg,
                WM_TIMER
                    | WM_CLOSE
                    | WM_ERASEBKGND
                    | WM_INITMENUPOPUP
                    | WM_DRAWITEM
                    | WM_MEASUREITEM
                    | WM_CTLCOLORSTATIC
                    | WM_CTLCOLOREDIT
                    | WM_CTLCOLORLISTBOX
                    | WM_CTLCOLORBTN
                    | WM_CTLCOLORDLG
                    | WM_CTLCOLORSCROLLBAR
            ) {
            app.shell.compact_window
        } else {
            hwnd
        };
        app.select_surface(surface);
        // A notification delivered while another callback held App is older
        // than this event. In particular, End must release IME ownership before
        // the first ordinary character or Enter is dispatched.
        drain_native_notifications(state, &mut app);
        let result = if envelope.is_some() {
            Some(LRESULT(0))
        } else if (msg == WM_CLOSE || (msg == WM_COMMAND && wp.0 & 0xffff == CMD_CLOSE))
            && app.composing
        {
            record_phase(&state.phase_trace, "close", "ime_deferred");
            if app.close_after_composition.is_none() {
                app.close_after_composition = Some((msg, wp, lp));
                app.close_target = Some(hwnd);
            }
            app.hwnd = app.editing_window();
            if !app.complete_composition() {
                app.close_after_composition = None;
                app.status =
                    "IMEの確定を完了できませんでした — 未保存本文と変換文字は保持しています".into();
                app.refresh();
            }
            Some(LRESULT(0))
        } else if msg == WM_PAINT {
            Some(paint_owned_window(hwnd, state, &mut app))
        } else {
            dispatch_window_message(hwnd, msg, wp, lp, &mut app)
        };
        drain_native_notifications(state, &mut app);
        drop(app);
        settings_dialog::run_requested(state);
        replay_deferred_paints(state);
        result.unwrap_or_else(|| {
            // Standard default processing can synchronously send WM_SIZE,
            // enter a system menu or paint owned children. Release App at the
            // originating default boundary so those callbacks can borrow it.
            let lp = if msg == WM_IME_SETCONTEXT {
                LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize))
            } else {
                lp
            };
            DefWindowProcW(hwnd, msg, wp, lp)
        })
    }
}

unsafe fn native_cancel_ime(editor: HWND) -> ImeCancelOutcome {
    unsafe {
        let context = ImmGetContext(editor);
        if context.0.is_null() {
            return ImeCancelOutcome::NoContext;
        }
        let cancelled = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0).as_bool();
        let _ = ImmReleaseContext(editor, context);
        if cancelled {
            ImeCancelOutcome::Cancelled
        } else {
            ImeCancelOutcome::Rejected
        }
    }
}
unsafe fn perform_reconversion_cancel(
    state: &WindowState,
    app: &mut App,
    cancel: ReconversionCancel,
    operation: impl FnOnce(HWND) -> ImeCancelOutcome,
) {
    unsafe {
        // This key is now an explicit native command. A failed attempt cannot
        // lend its Escape attribution to the legacy synchronous result rule.
        state.ime_input.set(None);
        state.ime_dispatch_key.set(None);
        // App remains borrowed through execute: reentry only captures packets.
        let outcome = operation(cancel.envelope.editor);
        if outcome == ImeCancelOutcome::Cancelled {
            // Publish completion and retain the captured owner before draining.
            app.reconversion_cancel = Some(cancel);
        }
        drain_native_notifications(state, app);
        if outcome == ImeCancelOutcome::Cancelled {
            if app.ime_session == Some(cancel.envelope) {
                app.clear_composition();
            }
            state.ime_ingress.retire(cancel.envelope);
            app.reconversion_cancel = None;
            let editor = app.editing_window();
            if editor == cancel.envelope.editor
                && app
                    .ime_session
                    .as_ref()
                    .is_none_or(|session| session.editor == editor)
                && IsWindow(Some(editor)).as_bool()
            {
                app.select_surface(editor);
            }
        } else {
            app.status = "IME再変換の取消に失敗しました".into();
        }
        app.refresh();
    }
}

unsafe fn drain_native_notifications(state: &WindowState, app: &mut App) {
    unsafe {
        // Settings previously queued lifecycle notifications and persistence
        // while App was borrowed. Painting must not drain those queues early.
        if !state.settings_open.get() {
            // Release the queue borrow before invoking IMM or native callbacks.
            loop {
                let next = state.notifications.borrow_mut().pop_front();
                let Some((owner, notification)) = next else {
                    break;
                };
                app.native_packet(owner, notification);
            }
            app.invalidate_ime_editor();
            state.document_id.set(app.doc().id());
            state.editor_window.set(app.editing_window());
        }
        if state.deferred_control_redraw.replace(false) {
            let main = state.main_window.get();
            let main = if main == HWND::default() {
                app.hwnd
            } else {
                main
            };
            unsafe extern "system" fn invalidate_child(
                hwnd: HWND,
                _: LPARAM,
            ) -> windows::core::BOOL {
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, true);
                }
                windows::core::BOOL(1)
            }
            // Native child painting may reenter while parent layout/paint holds
            // App. Repair only those child surfaces asynchronously: invalidating
            // the parent here can repeat its layout and starve child WM_PAINT.
            let _ = EnumChildWindows(Some(main), Some(invalidate_child), LPARAM(0));
        }
    }
}

unsafe fn paint_owned_window(hwnd: HWND, state: &WindowState, app: &mut App) -> LRESULT {
    unsafe {
        let mut pending = RECT::default();
        let had_damage = GetUpdateRect(hwnd, Some(&mut pending), false).as_bool();
        let prepared = app.prepare_paint();
        // Preparation can synchronously validate damage through the borrowed
        // WM_PAINT route. Re-arm it before acquiring the actual paint DC.
        let deferred = state.deferred_paints.replace([None, None]);
        let mut remaining = [None, None];
        for (index, item) in deferred.into_iter().enumerate() {
            if let Some((surface, damage)) = item {
                if surface == hwnd {
                    let _ = InvalidateRect(Some(hwnd), Some(&damage), false);
                } else {
                    remaining[index] = Some((surface, damage));
                }
            }
        }
        state.deferred_paints.set(remaining);
        if app.full_paint {
            let _ = InvalidateRect(Some(hwnd), None, false);
        } else if had_damage {
            let _ = InvalidateRect(Some(hwnd), Some(&pending), false);
        }
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut ps);
        let damage = rect(
            ps.rcPaint.left as f32 / app.dpi,
            ps.rcPaint.top as f32 / app.dpi,
            (ps.rcPaint.right - ps.rcPaint.left) as f32 / app.dpi,
            (ps.rcPaint.bottom - ps.rcPaint.top) as f32 / app.dpi,
        );
        let result = prepared.and_then(|()| app.paint_prepared_to_dc(dc, damage));
        let retry = result.is_err() && !app.paint_retry_pending;
        let _ = EndPaint(hwnd, &ps);
        if let Err(error) = result {
            app.status = error.to_string();
            app.discard_target();
            app.paint_retry_pending = true;
        } else {
            app.paint_retry_pending = false;
            app.after_paint_release();
        }
        if retry {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        LRESULT(0)
    }
}

unsafe fn dispatch_window_message(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    app: &mut App,
) -> Option<LRESULT> {
    unsafe {
        if matches!(
            msg,
            WM_SIZE
                | WM_DPICHANGED
                | WM_KEYDOWN
                | WM_SYSKEYDOWN
                | WM_CHAR
                | WM_COMMAND
                | WM_KILLFOCUS
                | WM_CANCELMODE
                | WM_MOUSEWHEEL
        ) {
            app.features_reset_calendar_click();
        }
        if msg == WM_CLOSE && (app.search_busy() || app.git_job_busy()) {
            record_phase(&app.phase_trace, "close", "busy_rejected");
            app.status = "処理中です。取消を行い、処理結果を確認してから閉じてください".into();
            app.refresh();
            return Some(LRESULT(0));
        }
        if app.background_mutation_busy()
            && hwnd == app.editing_window()
            && matches!(msg, WM_KEYDOWN | WM_CHAR)
        {
            return Some(LRESULT(0));
        }
        if let Some(result) = app.shell_window_message(hwnd, msg, wp, lp) {
            return Some(result);
        }
        Some(match msg {
            WM_CREATE => LRESULT(0),
            WM_SETCURSOR if wp.0 == hwnd.0 as usize && lp.0 as u16 == HTCLIENT as u16 => {
                let mut point = POINT::default();
                if GetCursorPos(&mut point).is_ok() && ScreenToClient(hwnd, &mut point).as_bool() {
                    let cursor =
                        app.surface_cursor_at(point.x as f32 / app.dpi, point.y as f32 / app.dpi);
                    if let Ok(cursor) = LoadCursorW(None, cursor) {
                        SetCursor(Some(cursor));
                        return Some(LRESULT(1));
                    }
                }
                return None;
            }
            value if value == WM_APP + 1 && app.diagnostic.is_some() => {
                app.refresh();
                LRESULT(1)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_SIZE => {
                app.minimized = wp.0 == SIZE_MINIMIZED as usize;
                if app.minimized {
                    app.sync_animations();
                    return Some(LRESULT(0));
                }
                app.resize_target(D2D_SIZE_U {
                    width: (lp.0 as u32) & 0xffff,
                    height: ((lp.0 as u32) >> 16) & 0xffff,
                });
                app.layout_key = None;
                app.refresh();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                app.dpi = (wp.0 & 0xffff) as f32 / 96.0;
                app.update_surface_font(hwnd, (wp.0 & 0xffff) as u32);
                app.discard_target();
                let r = &*(lp.0 as *const RECT);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                app.refresh();
                LRESULT(0)
            }
            WM_SETFOCUS => {
                if app.focus_palette_after_main_activation(hwnd) {
                    return Some(LRESULT(0));
                }
                app.native_caret = NativeCaretState::default();
                app.refresh();
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                let _ = SetFocus(Some(hwnd));
                if !app
                    .feature_panel_rect(mdlite::panel::PanelId::Calendar)
                    .is_some_and(|r| r.contains(x, y))
                {
                    app.features_reset_calendar_click();
                }
                if app.shell.enabled {
                    if hwnd == app.shell.main_window && app.begin_shell_splitter(x, y) {
                        app.refresh();
                        return Some(LRESULT(0));
                    }
                    for id in [
                        mdlite::panel::PanelId::Explorer,
                        mdlite::panel::PanelId::Calendar,
                        mdlite::panel::PanelId::Outline,
                        mdlite::panel::PanelId::Git,
                    ] {
                        if app.feature_panel_rect(id).is_some_and(|r| r.contains(x, y))
                            && (app.features_click(id, x, y)
                                || app.git_click(id, x, y)
                                || app.search_click(id, x, y))
                        {
                            app.refresh();
                            return Some(LRESULT(0));
                        }
                    }
                    if GetKeyState(VK_CONTROL.0 as i32) < 0
                        && !app.composing
                        && x >= app.editor_left()
                        && x <= app.editor_right()
                        && y >= app.editor_top()
                        && let Some(destination) =
                            app.link_destination_at(app.hit_with_affinity(x, y).0)
                    {
                        app.features_open_link(&destination);
                        app.refresh();
                        return Some(LRESULT(0));
                    }
                    app.shell_click(x, y);
                    app.refresh();
                    return Some(LRESULT(0));
                }
                if y < 28.0 {
                    let mut left = LEFT;
                    for (id, _, width) in TOOLBAR {
                        if x >= left && x < left + width {
                            if id == CMD_CLOSE {
                                // The same queued command enters WindowState's
                                // completion/end gate as menu/window Close.
                                let _ = PostMessageW(Some(hwnd), WM_COMMAND, WPARAM(id), LPARAM(0));
                            } else {
                                app.command(id);
                            }
                            break;
                        }
                        left += width;
                    }
                } else if y < 74.0 {
                    let strip = app.tab_strip();
                    if strip.overflow && (LEFT..LEFT + TAB_SCROLL_BUTTON).contains(&x) {
                        app.scroll_tabs(false);
                    } else if strip.overflow
                        && (app.width - LEFT - TAB_SCROLL_BUTTON..app.width - LEFT).contains(&x)
                    {
                        app.scroll_tabs(true);
                    } else if (strip.left..strip.right).contains(&x) && strip.tab_width > 0.0 {
                        let slot = ((x - strip.left) / strip.tab_width).floor() as usize;
                        if slot < strip.visible_count {
                            app.switch_tab(strip.first + slot);
                        }
                    }
                } else if x > app.width - 20.0 {
                    app.dragging_scroll = true;
                    app.scroll_at(y);
                    SetCapture(hwnd);
                } else if app.toggle_task_at(x, y) {
                    app.after_input();
                } else {
                    app.select_at(x, y, GetKeyState(VK_SHIFT.0 as i32) < 0);
                    app.dragging = true;
                    SetCapture(hwnd);
                }
                app.refresh();
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                if app.shell.enabled
                    && app.features_double_click(mdlite::panel::PanelId::Calendar, x, y)
                {
                    app.refresh();
                    return Some(LRESULT(0));
                }
                app.features_reset_calendar_click();
                // Other surfaces retain the ordinary second-click behavior.
                // Calendar activation above exclusively consumes its double click.
                dispatch_window_message(hwnd, WM_LBUTTONDOWN, wp, lp, app).unwrap_or(LRESULT(0))
            }
            WM_MOUSEMOVE => {
                if GetCapture() == hwnd {
                    let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                    let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                    if let Ok(cursor) = LoadCursorW(None, app.surface_cursor_at(x, y)) {
                        SetCursor(Some(cursor));
                    }
                }
                if app.shell.enabled {
                    let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                    let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                    if app.features_pointer_move(x, y) {
                        app.refresh();
                    }
                    if app.features_drag_move(mdlite::panel::PanelId::Explorer, x, y) {
                        app.refresh();
                        return Some(LRESULT(0));
                    }
                    if !app.dragging
                        && !app.composing
                        && x >= app.editor_left()
                        && x <= app.editor_right()
                        && y >= app.editor_top()
                        && let Some(destination) =
                            app.link_destination_at(app.hit_with_affinity(x, y).0)
                    {
                        app.status = format!("リンク: {destination}（Ctrl+クリックで開く）");
                    }
                }
                if app.shell.enabled && app.shell_mouse_move(lp) {
                    return Some(LRESULT(0));
                }
                if app.dragging_scroll {
                    let dpi = app.dpi;
                    app.scroll_at(((lp.0 >> 16) as u16 as i16) as f32 / dpi);
                    app.refresh();
                }
                if app.dragging {
                    let dpi = app.dpi;
                    app.select_at(
                        (lp.0 as u16 as i16) as f32 / dpi,
                        ((lp.0 >> 16) as u16 as i16) as f32 / dpi,
                        true,
                    );
                    app.refresh();
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if app.shell.enabled {
                    let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                    let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                    app.features_pointer_move(x, y);
                    app.features_finish_outline_drag(false);
                    app.features_drag_end(mdlite::panel::PanelId::Explorer, x, y);
                }
                app.finish_splitter();
                app.dragging = false;
                app.dragging_scroll = false;
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            value if value == WM_APP + 20 => {
                let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                if wp.0 == 0 && app.features_drag_start(mdlite::panel::PanelId::Explorer, x, y) {
                    SetCapture(hwnd);
                }
                LRESULT(0)
            }
            WM_CAPTURECHANGED | WM_CANCELMODE => {
                app.features_finish_outline_drag(true);
                LRESULT(0)
            }
            WM_DRAWITEM => {
                if lp.0 != 0
                    && app.explorer_icons.draw_offset(
                        &*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT),
                        app.shell.tokens,
                        control_scroll::horizontal_offset(
                            (*(lp.0 as *const windows::Win32::UI::Controls::DRAWITEMSTRUCT))
                                .hwndItem,
                        ),
                    )
                {
                    app.features_refresh_explorer_paint(false);
                    LRESULT(1)
                } else {
                    return None;
                }
            }
            WM_RBUTTONUP => {
                app.context_menu();
                LRESULT(0)
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = ((wp.0 >> 16) as u16 as i16) as f32 / 120.0;
                if app.shell.enabled && msg == WM_MOUSEWHEEL {
                    let mut pointer = POINT {
                        x: lp.0 as u16 as i16 as i32,
                        y: (lp.0 >> 16) as u16 as i16 as i32,
                    };
                    let _ = ScreenToClient(hwnd, &mut pointer);
                    if app.features_wheel(
                        pointer.x as f32 / app.dpi,
                        pointer.y as f32 / app.dpi,
                        delta,
                    ) {
                        app.refresh();
                        return Some(LRESULT(0));
                    }
                }
                if app.shell.enabled && GetFocus() != app.editing_window() {
                    return Some(LRESULT(0));
                }
                if msg == WM_MOUSEHWHEEL || GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    app.horizontal_scroll(delta * 70.0);
                } else {
                    app.scroll(-delta * 80.0);
                }
                app.refresh();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wp.0 as u16 == VK_ESCAPE.0 && app.features_finish_outline_drag(true) {
                    let _ = ReleaseCapture();
                    app.refresh();
                    return Some(LRESULT(0));
                }
                app.key(wp.0 as u16);
                LRESULT(0)
            }
            WM_CHAR => {
                app.character(wp.0 as u16);
                LRESULT(0)
            }
            WM_IME_CHAR => LRESULT(0),
            WM_IME_REQUEST => app.ime_request(wp.0 as u32, lp),
            WM_IME_SETCONTEXT => return None,
            WM_COMMAND => {
                let id = wp.0 & 0xffff;
                let notification = (wp.0 >> 16) as u16;
                if app.features_control(id, notification)
                    || app.git_control(id, notification)
                    || app.search_control(id, notification)
                {
                    app.refresh();
                    return Some(LRESULT(0));
                }
                app.command(wp.0 & 0xffff);
                app.refresh();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == ANIMATION_TIMER => {
                app.animation_tick();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == PERSISTENCE_TIMER => {
                let dirty = app.tabs.iter().any(|t| t.doc.is_dirty());
                let status = app.status.clone();
                app.persistence_tick(Instant::now());
                let feature_changed = app.features_tick(Instant::now())
                    | app.git_tick(Instant::now())
                    | app.search_tick(Instant::now());
                if feature_changed && app.shell.main_window != HWND::default() {
                    let _ = InvalidateRect(Some(app.shell.main_window), None, false);
                }
                if feature_changed
                    || dirty != app.tabs.iter().any(|t| t.doc.is_dirty())
                    || status != app.status
                {
                    app.refresh();
                }
                LRESULT(0)
            }
            WM_DROPFILES => {
                let drop = HDROP(wp.0 as *mut _);
                if app.background_mutation_busy() {
                    DragFinish(drop);
                    app.status =
                        "変更処理中のためdropを保留しました。処理後に再度操作してください".into();
                    return Some(LRESULT(0));
                }
                let count = DragQueryFileW(drop, u32::MAX, None);
                for n in 0..count {
                    let len = DragQueryFileW(drop, n, None);
                    let mut p = vec![0; len as usize + 1];
                    DragQueryFileW(drop, n, Some(&mut p));
                    let path = PathBuf::from(String::from_utf16_lossy(&p[..len as usize]));
                    if is_image_path(&path) {
                        app.import_image(&path);
                    } else {
                        app.open_path(path);
                    }
                }
                DragFinish(drop);
                app.refresh();
                LRESULT(0)
            }
            WM_CLOSE => {
                record_phase(&app.phase_trace, "close", "dispatch");
                let confirm_phase = PhaseSpan::new(&app.phase_trace, "close.confirm");
                if app.confirm_all() {
                    drop(confirm_phase);
                    record_phase(&app.phase_trace, "close", "confirmed");
                    {
                        let _phase = PhaseSpan::new(&app.phase_trace, "close.session");
                        app.persist_session();
                    }
                    {
                        let _phase = PhaseSpan::new(&app.phase_trace, "close.destroy_window");
                        let result = DestroyWindow(hwnd);
                        record_phase(
                            &app.phase_trace,
                            "close.destroy_window",
                            if result.is_ok() {
                                "returned_ok"
                            } else {
                                "returned_error"
                            },
                        );
                    }
                } else {
                    drop(confirm_phase);
                    record_phase(&app.phase_trace, "close", "confirmation_rejected");
                    app.refresh();
                }
                LRESULT(0)
            }
            _ => return None,
        })
    }
}

impl App {
    fn live_insertion_point(&self, p: usize) -> usize {
        if self
            .revealed_source
            .as_ref()
            .is_some_and(|range| range.start <= p && p <= range.end)
            || self
                .projection_cache
                .as_ref()
                .is_some_and(|(id, revision, projection)| {
                    (*id, *revision) == (self.doc().id(), self.doc().revision())
                        && projection
                            .editable_source_positions
                            .binary_search(&p)
                            .is_ok()
                        && self
                            .views
                            .iter()
                            .any(|view| view.map.contains(&p) || view.before.contains(&p))
                })
        {
            return p;
        }
        let markers: Vec<_> = self
            .views
            .iter()
            .filter(|v| v.source.is_empty())
            .map(|v| v.source.start)
            .collect();
        resolve_live_point(
            &self.visible_tokens,
            p,
            &markers,
            self.views.first().and_then(|v| v.map.first()).copied(),
        )
    }
    fn live_word_point(&self, origin: usize, target: usize, back: bool) -> usize {
        // Nearest-point resolution is appropriate for insertion, but can snap
        // a word movement back into its origin inside a hidden delimiter gap.
        // Resolve toward the requested direction using actual visible/source-
        // affine endpoints, without exposing the interior of hidden spelling.
        self.visible_tokens
            .iter()
            .flat_map(|token| {
                token
                    .spans
                    .first()
                    .map(|span| span.start)
                    .into_iter()
                    .chain(token.spans.last().map(|span| span.end))
            })
            .chain(
                self.views
                    .iter()
                    .filter(|view| view.source.is_empty())
                    .map(|view| view.source.start),
            )
            .filter(|&point| {
                if back {
                    point < origin && point <= target
                } else {
                    point > origin && point >= target
                }
            })
            .min_by_key(|&point| point.abs_diff(target))
            .unwrap_or(origin)
    }
    fn animation_viewport(&self) -> f32 {
        if self.shell.enabled {
            return self.shell.editor.y + self.shell.editor.height;
        }
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        client.bottom as f32 / self.dpi - 32.0
    }
    unsafe fn sync_animations(&mut self) {
        // SAFETY: all decoder and timer ownership is UI-thread-local. The
        // window timer carries no borrowed pointer or delayed document state.
        unsafe {
            if self.animation_timer_armed {
                let _ = KillTimer(Some(self.hwnd), ANIMATION_TIMER);
                self.animation_timer_armed = false;
            }
            let now = Instant::now();
            let viewport = self.animation_viewport();
            let scroll = self.tabs[self.active].scroll;
            let paused = self.minimized || self.tabs[self.active].raw;
            let editor_top = self.editor_top();
            let mut next = None;
            for (path, cached) in &mut self.image_cache {
                if let Some(animation) = &mut cached.animation {
                    let visible = !paused
                        && self.images.iter().any(|image| {
                            image.path.as_ref() == Some(path)
                                && image_intersects_viewport(
                                    image.y - editor_top + TOP,
                                    image.height,
                                    scroll,
                                    viewport - editor_top + TOP,
                                )
                        });
                    animation.set_visible(visible, now);
                    if let Some(due) = animation.next_due() {
                        next = Some(next.map_or(due, |previous: Instant| previous.min(due)));
                    }
                }
            }
            if let Some(due) = next {
                let milliseconds = due
                    .saturating_duration_since(now)
                    .as_millis()
                    .clamp(10, u32::MAX as u128) as u32;
                self.animation_timer_armed =
                    SetTimer(Some(self.hwnd), ANIMATION_TIMER, milliseconds, None) != 0;
            }
        }
    }
    unsafe fn animation_tick(&mut self) {
        // SAFETY: the bitmap owns its D2D storage; the current frame remains
        // alive until CopyFromMemory returns. InvalidateRect copies its RECT.
        unsafe {
            let _ = KillTimer(Some(self.hwnd), ANIMATION_TIMER);
            self.animation_timer_armed = false;
            let now = Instant::now();
            let mut changed = std::collections::HashSet::new();
            let mut failed = false;
            for (path, cached) in &mut self.image_cache {
                if let Some(animation) = &mut cached.animation {
                    match animation.advance_if_due(now) {
                        Ok(true) => {
                            let frame = animation.current();
                            if frame.width != cached.width || frame.height != cached.height {
                                self.status =
                                    "アニメーションの寸法が途中で変化したため停止しました".into();
                                cached.animation = None;
                                failed = true;
                                continue;
                            }
                            if let Err(error) = cached.bitmap.CopyFromMemory(
                                None,
                                frame.pixels.as_ptr().cast(),
                                frame.width * 4,
                            ) {
                                self.status = format!("アニメーション描画を停止しました: {error}");
                                cached.animation = None;
                                failed = true;
                            } else {
                                cached.frame_sequence += 1;
                                changed.insert(path.clone());
                            }
                        }
                        Ok(false) => {}
                        Err(error) => {
                            self.status = format!("アニメーションdecodeを停止しました: {error}");
                            cached.animation = None;
                            failed = true;
                        }
                    }
                }
            }
            let scroll = self.tabs[self.active].scroll;
            for image in &self.images {
                if image
                    .path
                    .as_ref()
                    .is_some_and(|path| changed.contains(path))
                {
                    let damage = image_damage_rect(image, scroll, self.dpi);
                    let _ = InvalidateRect(Some(self.hwnd), Some(&damage), false);
                }
            }
            if failed {
                let viewport = self.animation_viewport();
                let damage = RECT {
                    left: 0,
                    top: (viewport * self.dpi) as i32,
                    right: (self.width * self.dpi) as i32,
                    bottom: ((viewport + 32.0) * self.dpi) as i32,
                };
                let _ = InvalidateRect(Some(self.hwnd), Some(&damage), false);
            }
            self.sync_animations();
            self.diagnostic();
        }
    }
    unsafe fn ime_request(&mut self, request: u32, parameter: LPARAM) -> LRESULT {
        // SAFETY: IMM supplies these writable buffers only during this
        // synchronous callback; validate the advertised size before writing.
        unsafe {
            match request {
                IMR_RECONVERTSTRING => {
                    let selection = self.doc().selection();
                    let start = selection.anchor.min(selection.head);
                    let end = selection.anchor.max(selection.head);
                    if start == end {
                        return LRESULT(0);
                    }
                    if let Some(cell) = self.active_cell()
                        && (start < cell.start || end > cell.end)
                    {
                        return LRESULT(0);
                    }
                    let text: Vec<u16> = self.doc().source()[start..end].encode_utf16().collect();
                    let header = std::mem::size_of::<RECONVERTSTRING>();
                    let size = header + (text.len() + 1) * 2;
                    if parameter.0 == 0 {
                        return LRESULT(size as isize);
                    }
                    let pointer = parameter.0 as *mut RECONVERTSTRING;
                    if (*pointer).dwSize < size as u32 {
                        return LRESULT(0);
                    }
                    *pointer = RECONVERTSTRING {
                        dwSize: size as u32,
                        dwVersion: 0,
                        dwStrLen: text.len() as u32,
                        dwStrOffset: header as u32,
                        dwCompStrLen: text.len() as u32,
                        dwCompStrOffset: 0,
                        dwTargetStrLen: text.len() as u32,
                        dwTargetStrOffset: 0,
                    };
                    let output = (pointer as *mut u8).add(header) as *mut u16;
                    std::ptr::copy_nonoverlapping(text.as_ptr(), output, text.len());
                    *output.add(text.len()) = 0;
                    self.arm_reconversion();
                    LRESULT(size as isize)
                }
                IMR_CONFIRMRECONVERTSTRING => {
                    if parameter.0 == 0 {
                        return LRESULT(0);
                    }
                    let request = &*(parameter.0 as *const RECONVERTSTRING);
                    let selection = self.doc().selection();
                    let len = self.doc().source()[selection.anchor.min(selection.head)
                        ..selection.anchor.max(selection.head)]
                        .encode_utf16()
                        .count();
                    let confirmed = request.dwTargetStrOffset == 0
                        && request.dwTargetStrLen as usize == len
                        && self
                            .pending_reconversion
                            .or(self.reconversion_owner)
                            .is_some_and(|owner| {
                                owner.editor == self.editing_window() && owner.matches(self.doc())
                            });
                    LRESULT(confirmed as isize)
                }
                IMR_QUERYCHARPOSITION => {
                    if parameter.0 == 0
                        || !(parameter.0 as usize)
                            .is_multiple_of(std::mem::align_of::<IMECHARPOSITION>())
                    {
                        return LRESULT(0);
                    }
                    let info = &mut *(parameter.0 as *mut IMECHARPOSITION);
                    if info.dwSize < std::mem::size_of::<IMECHARPOSITION>() as u32 {
                        return LRESULT(0);
                    }
                    let Some((x, y, h)) = self.composition_position(info.dwCharPos) else {
                        return LRESULT(0);
                    };
                    let mut p = POINT {
                        x: (x * self.dpi).round() as i32,
                        y: (y * self.dpi).round() as i32,
                    };
                    let mut document = RECT::default();
                    if !ClientToScreen(self.hwnd, &mut p).as_bool()
                        || GetClientRect(self.hwnd, &mut document).is_err()
                    {
                        return LRESULT(0);
                    }
                    if self.shell.enabled {
                        document = RECT {
                            left: (self.content_left() * self.dpi).round() as i32,
                            top: (self.editor_top() * self.dpi).round() as i32,
                            right: (self.editor_right() * self.dpi).round() as i32,
                            bottom: (self.editor_bottom() * self.dpi).round() as i32,
                        };
                    }
                    let mut start = POINT {
                        x: document.left,
                        y: document.top,
                    };
                    let mut end = POINT {
                        x: document.right,
                        y: document.bottom,
                    };
                    if !ClientToScreen(self.hwnd, &mut start).as_bool()
                        || !ClientToScreen(self.hwnd, &mut end).as_bool()
                    {
                        return LRESULT(0);
                    }
                    info.pt = p;
                    info.cLineHeight = (h * self.dpi).round().max(1.0) as u32;
                    info.rcDocument = RECT {
                        left: start.x,
                        top: start.y,
                        right: end.x,
                        bottom: end.y,
                    };
                    LRESULT(1)
                }
                _ => LRESULT(0),
            }
        }
    }
    fn image_directory(&self) -> Option<PathBuf> {
        self.tabs[self.active]
            .file
            .as_ref()
            .and_then(|f| f.path().parent())
            .map(PathBuf::from)
    }
    fn import_image(&mut self, path: &std::path::Path) {
        let Some(dir) = self.image_directory() else {
            self.status = "画像を挿入する前に文書を保存してください".into();
            return;
        };
        match mdlite::assets::import_file(&dir, &dir, path) {
            Ok(imported) => self.edit(&imported.markdown),
            Err(e) => self.status = format!("画像の挿入に失敗しました — 本文は保持しています: {e}"),
        }
    }
    unsafe fn paste_clipboard(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if IsClipboardFormatAvailable(8).is_ok() {
                match read_dib(self.hwnd) {
                    Ok((w, h, pixels)) => {
                        let Some(dir) = self.image_directory() else {
                            self.status = "画像を挿入する前に文書を保存してください".into();
                            return;
                        };
                        match mdlite::assets::import_png(&dir, &dir, w, h, &pixels) {
                            Ok(imported) => self.edit(&imported.markdown),
                            Err(e) => self.status = format!("画像の貼り付けに失敗しました: {e}"),
                        }
                    }
                    Err(e) => self.status = format!("画像の貼り付けに失敗しました: {e}"),
                }
                return;
            }
            if let Some(s) = read_clipboard(self.hwnd) {
                self.paste(&s);
            }
        }
    }
    fn apply_table(&mut self, edit: TableEdit) {
        if let Err(e) = self.doc_mut().replace_range(edit.range, &edit.replacement) {
            self.status = e.to_string();
        }
        self.layout_key = None;
    }
    fn paste(&mut self, text: &str) {
        if !self.tabs[self.active].raw && (text.contains('\t') || text.contains('\n')) {
            let p = self.doc().selection().head;
            if let Some(table) = self.tables.iter().find(|t| t.table.cell_at(p).is_some()) {
                let pos = table.table.cell_at(p).unwrap();
                if let Some(edit) = table.table.paste_tsv(self.doc().source(), pos, text) {
                    self.apply_table(edit);
                } else {
                    self.status =
                        "表の貼り付け範囲またはTSVが不正です — 本文は保持しています".into();
                }
                return;
            }
        }
        self.edit(text);
    }
    unsafe fn context_menu(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.composing {
                return;
            }
            let byte = self.doc().selection().head;
            let cell = self.tables.iter().find_map(|t| t.table.cell_at(byte));
            let image = self
                .images
                .iter()
                .any(|i| i.source.start <= byte && byte <= i.source.end);
            if cell.is_none() && !image && !self.shell.enabled {
                return;
            }
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            if self.shell.enabled {
                self.append_shell_context(menu);
            }
            self.editor_actions_sync();
            for &(id, label, _) in editor_actions::COMMANDS {
                let belongs = if (27..=31).contains(&id) {
                    image
                } else {
                    cell.is_some()
                };
                if !belongs || matches!(id, 34 | 39) {
                    continue;
                }
                let flags = MF_STRING
                    | if self.editor_enabled(id) {
                        MF_ENABLED
                    } else {
                        MF_GRAYED
                    };
                let _ = AppendMenuW(menu, flags, id, PCWSTR(wide(label).as_ptr()));
            }
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            self.release_caret();
            let command = menu_style::track_popup(
                self.hwnd,
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                self.shell.tokens,
                self.dpi,
            )
            .0 as usize;
            let _ = DestroyMenu(menu);
            if self.shell.enabled && self.is_shell_command(command) {
                self.command(command);
            } else {
                self.table_command(command);
            }
            self.after_input();
        }
    }
    fn table_command(&mut self, command: usize) {
        self.editor_action(command);
    }
    fn active_cell(&self) -> Option<Range<usize>> {
        if self.tabs[self.active].raw {
            return None;
        }
        let p = self.doc().selection().head;
        self.tables
            .iter()
            .flat_map(|t| &t.table.rows)
            .flat_map(|r| &r.cells)
            .find(|c| c.content_range.start <= p && p <= c.content_range.end)
            .map(|c| c.content_range.clone())
    }
    fn visible_step(&self, p: usize, back: bool) -> usize {
        if self.tabs[self.active].raw {
            return if back {
                mdlite::coordinates::rope_previous_grapheme_boundary(self.doc().rope(), p)
                    .unwrap_or(p)
            } else {
                mdlite::coordinates::rope_next_grapheme_boundary(self.doc().rope(), p).unwrap_or(p)
            };
        }
        let tables: Vec<_> = self
            .tables
            .iter()
            .map(|table| table.table.range.clone())
            .collect();
        let token_target =
            live_horizontal_step(self.doc().source(), &self.visible_tokens, &tables, p, back);
        let markers = self
            .views
            .iter()
            .filter(|view| view.source.is_empty())
            .map(|view| view.source.start)
            .chain(self.projection_cache.iter().flat_map(|(_, _, projection)| {
                projection.editable_source_positions.iter().copied()
            }))
            .filter(|point| {
                !self
                    .tables
                    .iter()
                    .any(|table| table.table.range.contains(point))
                    || self
                        .views
                        .iter()
                        .any(|view| view.source.is_empty() && view.source.start == *point)
            });
        let marker = markers
            .filter(|&point| if back { point < p } else { point > p })
            .min_by_key(|&point| point.abs_diff(p));
        marker
            .filter(|&point| token_target == p || point.abs_diff(p) < token_target.abs_diff(p))
            .unwrap_or(token_target)
    }
    fn edit(&mut self, text: &str) {
        let selection = self.doc().selection();
        let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        self.edit_range(range, text);
    }
    fn edit_range(&mut self, range: Range<usize>, text: &str) {
        let authoring_table = self.pending_table_authoring.is_some_and(|(id, start)| {
            id == self.doc().id()
                && self.tables.iter().any(|table| {
                    table.table.range.start == start
                        && range.start >= table.table.range.start
                        && range.end <= table.table.range.end
                })
        });
        let range = if !self.tabs[self.active].raw && range.is_empty() {
            let p = self.live_insertion_point(range.start);
            p..p
        } else {
            range
        };
        let mut replacement = text.to_owned();
        if !self.tabs[self.active].raw && !authoring_table {
            if range.is_empty()
                && !replacement.is_empty()
                && self
                    .views
                    .iter()
                    .any(|view| view.source.is_empty() && view.source.start == range.start)
                && let Some(plan) = self.tables.iter().find_map(|table| {
                    table_before_insertion(
                        self.doc().source(),
                        table.table.range.start,
                        range.start,
                        &replacement,
                    )
                })
            {
                self.apply_live_edit(plan);
                return;
            }
            if range.is_empty()
                && !replacement.is_empty()
                && self.tables.iter().any(|t| {
                    t.table.range.end == range.start && range.start == self.doc().source().len()
                })
            {
                let newline = if self.doc().source().contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                replacement = format!("{newline}{newline}{replacement}");
            }
            if let Some(cell) = self.active_cell() {
                if range.start < cell.start || range.end > cell.end {
                    self.status = "表の構造はソース表示で編集できます".into();
                    return;
                }
                replacement = replacement
                    .replace('\\', "\\\\")
                    .replace('|', "\\|")
                    .replace(['\r', '\n'], " ");
            } else if self.tables.iter().any(|t| {
                (range.start < t.table.range.end && range.end > t.table.range.start)
                    || (range.is_empty() && t.table.range.contains(&range.start))
            }) {
                self.status = "表をまたぐ編集はソース表示で行ってください".into();
                return;
            }
        }
        let plan = if self.tabs[self.active].raw
            || authoring_table
            || self
                .revealed_source
                .as_ref()
                .is_some_and(|source| source.start <= range.start && range.end <= source.end)
        {
            Some(LiveEdit {
                caret: range.start + replacement.len(),
                range,
                replacement,
            })
        } else {
            live_edit(
                self.doc().source(),
                &self.visible_tokens,
                range,
                &replacement,
            )
        };
        let Some(plan) = plan else {
            return;
        };
        self.apply_live_edit(plan);
    }
    fn apply_live_edit(&mut self, plan: LiveEdit) {
        let before_revision = self.doc().revision();
        let selection = Selection::caret(plan.caret);
        let result = if self.typing {
            self.doc_mut().replace_range_typing_with_selection(
                plan.range,
                &plan.replacement,
                selection,
            )
        } else {
            self.doc_mut()
                .replace_range_with_selection(plan.range, &plan.replacement, selection)
        };
        if let Err(e) = result {
            self.status = e.to_string();
        }
        if self.doc().revision() != before_revision {
            self.layout_key = None;
        }
    }
    fn table_key(&mut self, intent: TableKeyIntent) {
        let p = self.doc().selection().head;
        let action = self.tables.iter().find_map(|view| {
            view.table.cell_at(p)?;
            let boundaries = self.table_body_boundaries(&view.table);
            table_key_action(&view.table, self.doc().source(), p, intent, boundaries)
        });
        if let Some(action) = action {
            let before = self.doc().revision();
            if let Err(error) = apply_table_key_action(self.doc_mut(), action) {
                self.status = error.to_string();
            }
            if self.doc().revision() != before {
                self.layout_key = None;
            }
        }
    }
    fn table_body_boundaries(&self, table: &Table) -> (Option<usize>, Option<usize>) {
        let before = self
            .views
            .iter()
            .filter(|view| view.table.is_none() && view.source.end <= table.range.start)
            .max_by_key(|view| view.source.end)
            .and_then(|view| view.before.last())
            .copied();
        let after = self
            .views
            .iter()
            .filter(|view| view.table.is_none() && view.source.start >= table.range.end)
            .min_by_key(|view| view.source.start)
            .and_then(|view| view.map.first())
            .copied();
        (before, after)
    }
    unsafe fn character(&mut self, unit: u16) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.shell.enabled && GetFocus() != self.editing_window() {
                return;
            }
            if self.composing {
                return;
            }
            if unit < 32 || unit == 127 {
                return;
            }
            if (0xd800..=0xdbff).contains(&unit) {
                self.pending_surrogate = Some(unit);
                return;
            }
            let text = if (0xdc00..=0xdfff).contains(&unit) {
                let Some(high) = self.pending_surrogate.take() else {
                    return;
                };
                String::from_utf16_lossy(&[high, unit])
            } else {
                self.pending_surrogate = None;
                String::from_utf16_lossy(&[unit])
            };
            let now = Instant::now();
            if self.last_typing.is_some_and(|previous| {
                now.duration_since(previous) > std::time::Duration::from_secs(1)
            }) {
                self.doc_mut().break_undo_group();
            }
            self.last_typing = Some(now);
            self.typing = true;
            self.preferred_x = None;
            let previous_tables: Vec<_> = self
                .tables
                .iter()
                .map(|table| table.table.range.start)
                .collect();
            self.edit(&text);
            self.typing = false;
            self.after_input();
            if !self.tabs[self.active].raw
                && let Some(table) = self.tables.iter().find(|table| {
                    !previous_tables.contains(&table.table.range.start)
                        && table.table.range.end == self.doc().selection().head
                        && table.table.rows.len() == 1
                })
            {
                self.pending_table_authoring = Some((self.doc().id(), table.table.range.start));
            }
        }
    }
    fn arm_reconversion(&mut self) {
        if !self.doc().selection().is_caret() {
            // A filled request is pending until START, including a request
            // received while an older native session still owns composition.
            self.pending_reconversion = Some(ReconversionOwner {
                request_id: self.ime_ingress.allocate(),
                editor: self.editing_window(),
                generation: None,
                document_id: self.doc().id(),
                revision: self.doc().revision(),
                selection: self.doc().selection(),
                affinity: self.doc().selection_affinity(),
            });
        }
    }
    fn cancelled_reconversion_result(&self, result: &str, input: Option<ImeInputContext>) -> bool {
        self.reconversion_owner.is_some_and(|owner| {
            owner.matches(self.doc())
                && input.is_some_and(|input| {
                    input.document_id == owner.document_id
                        && input.virtual_key == VK_ESCAPE.0 as u32
                })
                && self.doc().source().get(owner.selection.range()) == Some(result)
        })
    }
    fn invalidate_stale_reconversion(&mut self) {
        self.reconversion_owner = self
            .reconversion_owner
            .filter(|owner| owner.matches(self.doc()));
        self.pending_reconversion = self
            .pending_reconversion
            .filter(|owner| owner.matches(self.doc()));
    }
    fn invalidate_ime_editor(&mut self) {
        let editor = self.editing_window();
        let document_id = self.doc().id();
        if self
            .ime_session
            .is_some_and(|session| session.editor != editor || session.document_id != document_id)
        {
            self.clear_composition();
        }
        self.reconversion_owner = self
            .reconversion_owner
            .filter(|owner| owner.editor == editor && owner.document_id == document_id);
        self.pending_reconversion = self
            .pending_reconversion
            .filter(|owner| owner.editor == editor && owner.document_id == document_id);
    }
    fn prepare_reconversion_cancel(
        &self,
        captured: Option<ImeEnvelope>,
        input: Option<ImeInputContext>,
        editor: HWND,
    ) -> Option<ReconversionCancel> {
        let envelope = captured?;
        let input = input?;
        let owner = self.reconversion_owner?;
        (input.virtual_key == VK_ESCAPE.0 as u32
            && input.document_id == envelope.document_id
            && editor == envelope.editor
            && editor == self.editing_window()
            && self.composing
            && self.ime_session == Some(envelope)
            && self.ime_owner
                == Some((
                    self.doc().id(),
                    self.doc().revision(),
                    self.doc().selection().anchor,
                    self.doc().selection().head,
                ))
            && self.ime_affinity == self.doc().selection_affinity()
            && self.ime_ingress.current.get() == Some(envelope)
            && owner.request_id != 0
            && owner.editor == editor
            && owner.generation == Some(envelope.generation)
            && owner.matches(self.doc()))
        .then_some(ReconversionCancel { envelope, owner })
    }
    unsafe fn native_packet(&mut self, envelope: ImeEnvelope, notification: NativeNotification) {
        unsafe {
            if envelope.document_id != self.doc().id() || envelope.editor != self.editing_window() {
                return;
            }
            if self.reconversion_cancel.is_some_and(|cancel| {
                cancel.envelope == envelope && cancel.owner.generation == Some(envelope.generation)
            }) {
                return;
            }
            match notification {
                NativeNotification::Start if envelope.generation > self.ime_generation => {
                    self.start_composition_for(envelope);
                    self.refresh();
                }
                NativeNotification::Update(update)
                    if self.ime_session.is_none()
                        && envelope.generation == 0
                        && composition_is_cancelled(update.flags) =>
                {
                    self.pending_reconversion = None;
                }
                NativeNotification::End
                    if self.ime_session.is_none() && envelope.generation == 0 =>
                {
                    self.pending_reconversion = None;
                }
                NativeNotification::FocusLost
                    if self.ime_session == Some(envelope)
                        || (self.ime_session.is_none() && envelope.generation == 0) =>
                {
                    self.native_notification(NativeNotification::FocusLost)
                }
                notification if self.ime_session == Some(envelope) => {
                    self.native_notification(notification)
                }
                _ => (),
            }
        }
    }
    fn clear_composition(&mut self) {
        if let Some(session) = self.ime_session.take() {
            self.ime_ingress.retire(session);
        } else {
            self.pending_reconversion = None;
        }
        self.doc_mut().break_undo_group();
        self.composing = false;
        self.ime_owner = None;
        self.reconversion_owner = None;
        self.preedit.clear();
        self.ime_cursor = 0;
        self.ime_attributes.clear();
        self.ime_last_result = None;
        self.pending_surrogate = None;
    }
    unsafe fn cancel_composition(&mut self) -> bool {
        // The ordinary focus/nonreconversion path retains failure ownership.
        let cancelled =
            unsafe { native_cancel_ime(self.editing_window()) } == ImeCancelOutcome::Cancelled;
        if cancelled {
            self.clear_composition();
        }
        cancelled
    }
    #[cfg(test)]
    fn start_composition(&mut self) {
        let envelope = self
            .ime_ingress
            .start(self.doc().id(), self.editing_window());
        self.start_composition_for(envelope);
    }
    fn start_composition_for(&mut self, envelope: ImeEnvelope) {
        let selection = self.doc().selection();
        self.invalidate_stale_reconversion();
        self.reconversion_owner = self
            .pending_reconversion
            .take()
            .filter(|owner| owner.editor == envelope.editor);
        if let Some(owner) = &mut self.reconversion_owner {
            owner.generation = Some(envelope.generation);
        }
        self.ime_session = Some(envelope);
        self.ime_generation = envelope.generation;
        self.doc_mut().break_undo_group();
        self.composing = true;
        self.ime_owner = Some((
            self.doc().id(),
            self.doc().revision(),
            selection.anchor,
            selection.head,
        ));
        self.ime_affinity = self.doc().selection_affinity();
        self.preedit.clear();
        self.ime_attributes.clear();
        self.ime_last_result = None;
        self.ime_cursor = 0;
        self.pending_surrogate = None;
        self.preferred_x = None;
    }
    unsafe fn native_notification(&mut self, notification: NativeNotification) {
        unsafe {
            let updated = matches!(notification, NativeNotification::Update(_));
            match notification {
                NativeNotification::Start => {
                    let envelope = self
                        .ime_ingress
                        .start(self.doc().id(), self.editing_window());
                    self.start_composition_for(envelope);
                }
                NativeNotification::Update(update) => self.apply_ime_update(update),
                NativeNotification::End => {
                    self.clear_composition();
                    self.rearm_autosave(false);
                    if let Some((msg, wp, lp)) = self.close_after_composition.take() {
                        let target = self.close_target.take().unwrap_or(self.hwnd);
                        let _ = PostMessageW(Some(target), msg, wp, lp);
                    }
                }
                NativeNotification::FocusLost => {
                    self.pending_reconversion = None;
                    self.reconversion_owner = None;
                    if self.composing {
                        let _ = self.cancel_composition();
                    }
                    self.release_caret();
                    self.dragging = false;
                }
            }
            if updated {
                self.after_input();
            } else {
                self.refresh();
            }
        }
    }
    unsafe fn complete_composition(&mut self) -> bool {
        unsafe {
            let context = ImmGetContext(self.hwnd);
            if context.0.is_null() {
                return false;
            }
            let completed = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_COMPLETE, 0).as_bool();
            let _ = ImmReleaseContext(self.hwnd, context);
            completed
        }
    }
    #[cfg(test)]
    unsafe fn composition(&mut self, flags: u32) {
        unsafe {
            self.apply_ime_update(read_ime_update(self.hwnd, flags));
        }
    }
    unsafe fn apply_ime_update(&mut self, update: ImeUpdate) {
        unsafe {
            let selection = self.doc().selection();
            let current = (
                self.doc().id(),
                self.doc().revision(),
                selection.anchor,
                selection.head,
            );
            if self.ime_owner != Some(current)
                || self.ime_affinity != self.doc().selection_affinity()
            {
                self.clear_composition();
                self.status = "古いIME通知は適用していません".into();
                return;
            }
            if composition_is_cancelled(update.flags) {
                self.clear_composition();
                return;
            }
            let preserve_cancel = update.result.as_ref().is_some_and(|result| {
                self.cancelled_reconversion_result(result, update.input_context)
            });
            if update.result.is_some() {
                self.reconversion_owner = None;
            }
            let fingerprint = update
                .result
                .as_ref()
                .map(|result| (result.clone(), update.preedit.clone()));
            let duplicate = fingerprint.is_some() && self.ime_last_result == fingerprint;
            if update.result.is_none() && update.preedit.is_some() {
                self.ime_last_result = None;
            }
            if let Some(result) = update.result.filter(|_| !duplicate) {
                self.preedit.clear();
                self.ime_attributes.clear();
                self.ime_cursor = 0;
                if !result.is_empty() {
                    // GCS_RESULTSTR is the one result-insertion path. WM_IME_CHAR
                    // is handled separately, without forwarding to WM_CHAR; no
                    // text-valued suppression queue can eat later normal typing.
                    // ATOK can return the unchanged original as RESULTSTR on
                    // Escape. Preserve the reconversion selection in that one
                    // case; Enter, unknown input and changed results still edit.
                    if !preserve_cancel {
                        self.edit(&result);
                        self.reconversion_owner = None;
                    }
                    let selection = self.doc().selection();
                    self.ime_owner = Some((
                        self.doc().id(),
                        self.doc().revision(),
                        selection.anchor,
                        selection.head,
                    ));
                    self.ime_affinity = self.doc().selection_affinity();
                    self.after_input();
                }
            }
            if let Some(preedit) = update.preedit {
                self.preedit = preedit;
                self.ime_cursor = self.preedit.encode_utf16().count() as u32;
                self.ime_attributes.clear();
            }
            if let Some(attributes) = update.attributes {
                self.ime_attributes = attributes;
            }
            if let Some(cursor) = update.cursor {
                self.ime_cursor = cursor.min(self.preedit.encode_utf16().count() as u32);
            }
            if fingerprint.is_some() {
                self.ime_last_result = fingerprint;
            }
            self.ime_geometry();
        }
    }
    unsafe fn key(&mut self, key: u16) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            self.key_with_modifiers(key, ctrl, shift);
        }
    }
    unsafe fn key_with_modifiers(&mut self, key: u16, ctrl: bool, shift: bool) {
        unsafe {
            if self.shell.enabled && GetFocus() != self.editing_window() {
                return;
            }
            if self.shell.enabled && self.shell_shortcut(key, ctrl, shift) {
                return;
            }
            if self.composing {
                // Composition owns Escape before table exit or other editing.
                // Failed cancellation keeps the local preedit/owner intact.
                if key == VK_ESCAPE.0 {
                    let editor = self.editing_window();
                    let input = Some(ImeInputContext {
                        document_id: self.doc().id(),
                        virtual_key: key as u32,
                    });
                    if let Some(cancel) = self.prepare_reconversion_cancel(
                        self.ime_ingress.current.get(),
                        input,
                        editor,
                    ) {
                        let pointer =
                            GetWindowLongPtrW(editor, GWLP_USERDATA) as *const WindowState;
                        if !pointer.is_null() {
                            perform_reconversion_cancel(&*pointer, self, cancel, |editor| {
                                native_cancel_ime(editor)
                            });
                            return;
                        }
                        self.status =
                            "IME再変換の取消に失敗しました — 編集ウィンドウがありません".into();
                        return;
                    }
                    if self.cancel_composition() {
                        self.refresh();
                    }
                }
                return;
            }
            if let Some((id, start)) = self.pending_table_authoring
                && id == self.doc().id()
                && key == VK_RETURN.0
                && !ctrl
            {
                let action = self
                    .tables
                    .iter()
                    .find(|table| table.table.range.start == start)
                    .and_then(|view| {
                        let position = view.table.rows.first()?.cells.first()?.content_range.end;
                        table_key_action(
                            &view.table,
                            self.doc().source(),
                            position,
                            TableKeyIntent::NextRow,
                            self.table_body_boundaries(&view.table),
                        )
                    });
                self.pending_table_authoring = None;
                if let Some(action) = action {
                    let _ = apply_table_key_action(self.doc_mut(), action);
                    self.layout_key = None;
                }
                self.after_input();
                return;
            }
            if ctrl || matches!(key, 0x08 | 0x09 | 0x0d | 0x1b | 0x21..=0x28 | 0x2e) {
                self.pending_table_authoring = None;
            }
            if ctrl || matches!(key, 0x08 | 0x09 | 0x0d | 0x1b | 0x21..=0x28 | 0x2e) {
                self.doc_mut().break_undo_group();
                self.pending_surrogate = None;
            }
            if let Some(redo) = history_shortcut(key, ctrl, shift) {
                self.preferred_x = None;
                if redo {
                    self.doc_mut().redo();
                } else {
                    self.doc_mut().undo();
                }
                self.after_input();
                return;
            }
            if !self.tabs[self.active].raw
                && self.active_cell().is_some()
                && let Some(intent) = table_key_intent(key, ctrl, shift)
            {
                self.preferred_x = None;
                self.table_key(intent);
                self.after_input();
                return;
            }
            if ctrl {
                self.preferred_x = None;
                match key {
                    0x25..=0x28 => {
                        let selection = self.doc().selection();
                        let target = if matches!(key, 0x25 | 0x27) {
                            word_target(self.doc().source(), selection.head, key == 0x25)
                        } else {
                            logical_line_target(self.doc().source(), selection.head, key == 0x26)
                        };
                        if let Some(cell) = self.active_cell()
                            && (matches!(key, 0x26 | 0x28)
                                || target < cell.start
                                || target > cell.end)
                        {
                            self.move_table(matches!(key, 0x25 | 0x26), matches!(key, 0x25 | 0x27));
                            if shift {
                                let head = self.doc().selection().head;
                                let _ = self.doc_mut().set_selection(selection.anchor, head);
                            }
                            self.after_input();
                            return;
                        }
                        let target = if self.tabs[self.active].raw || matches!(key, 0x26 | 0x28) {
                            target
                        } else if matches!(key, 0x25 | 0x27) {
                            self.live_word_point(selection.head, target, key == 0x25)
                        } else {
                            self.live_insertion_point(target)
                        };
                        let anchor = if shift { selection.anchor } else { target };
                        let _ = self.doc_mut().set_selection(anchor, target);
                    }
                    0x08 | 0x2e => {
                        let selection = self.doc().selection();
                        if selection.is_caret() {
                            let mut target =
                                word_target(self.doc().source(), selection.head, key == 0x08);
                            if let Some(cell) = self.active_cell() {
                                target = target.clamp(cell.start, cell.end);
                            }
                            self.edit_range(
                                target.min(selection.head)..target.max(selection.head),
                                "",
                            );
                        } else {
                            self.edit("");
                        }
                    }
                    0x24 | 0x23 => {
                        let source_target = if key == 0x24 {
                            0
                        } else {
                            self.doc().source().len()
                        };
                        let target = if self.tabs[self.active].raw {
                            source_target
                        } else {
                            self.live_insertion_point(source_target)
                        };
                        let anchor = if shift {
                            self.doc().selection().anchor
                        } else {
                            target
                        };
                        let _ = self.doc_mut().set_selection(anchor, target);
                    }
                    0x4e => self.command(CMD_NEW),
                    0x4f => self.command(CMD_OPEN),
                    0x53 => self.command(if shift { CMD_SAVE_AS } else { CMD_SAVE }),
                    0x57 => self.command(CMD_CLOSE),
                    0x45 => self.command(CMD_RAW),
                    0x41 => {
                        let end = self.doc().source().len();
                        let _ = self.doc_mut().set_selection(0, end);
                    }
                    0x43 => {
                        self.copy(false);
                    }
                    0x58 => {
                        self.copy(true);
                    }
                    0x56 => {
                        self.paste_clipboard();
                    }
                    0x09 => {
                        let next = (self.active + if shift { self.tabs.len() - 1 } else { 1 })
                            % self.tabs.len();
                        self.switch_tab(next);
                    }
                    _ => {}
                }
                self.after_input();
                return;
            }
            match key {
                0x21 | 0x22 => {
                    let page = (self.animation_viewport() - self.editor_top())
                        .max(presentation::line_height(self.font_size()));
                    let (x, y, _) = self.caret_location();
                    let x = *self.preferred_x.get_or_insert(x);
                    let delta = if key == 0x21 { -page } else { page };
                    let (target, affinity) = self.hit_with_affinity(x, y + delta);
                    let anchor = if shift {
                        self.doc().selection().anchor
                    } else {
                        target
                    };
                    let _ = self
                        .doc_mut()
                        .set_selection_with_affinity(anchor, target, affinity);
                    self.scroll(delta);
                }
                0x23..=0x28 => self.move_caret(key, shift),
                0x08 | 0x2e => {
                    self.preferred_x = None;
                    let selection = self.doc().selection();
                    if selection.anchor == selection.head {
                        let p = selection.head;
                        let table_ranges: Vec<_> = self
                            .tables
                            .iter()
                            .map(|view| view.table.range.clone())
                            .collect();
                        if let Some(plan) = line_ending_delete(
                            self.doc().source(),
                            if self.tabs[self.active].raw {
                                &[]
                            } else {
                                &table_ranges
                            },
                            p,
                            key == 0x08,
                        ) {
                            self.apply_live_edit(plan);
                            self.after_input();
                            return;
                        }
                        let range = if self.tabs[self.active].raw {
                            let end = self.visible_step(p, key == 0x08);
                            p.min(end)..p.max(end)
                        } else {
                            let token = if key == 0x08 {
                                self.visible_tokens
                                    .iter()
                                    .filter(|t| t.spans.first().is_some_and(|s| s.start < p))
                                    .max_by_key(|t| t.spans.first().unwrap().start)
                            } else {
                                self.visible_tokens
                                    .iter()
                                    .filter(|t| t.spans.last().is_some_and(|s| s.end > p))
                                    .min_by_key(|t| t.spans.first().unwrap().start)
                            };
                            let Some(token) = token else {
                                return;
                            };
                            token.spans.first().unwrap().start..token.spans.last().unwrap().end
                        };
                        self.edit_range(range, "");
                    } else {
                        self.edit("");
                    }
                }
                0x0d => {
                    self.preferred_x = None;
                    let selection = self.doc().selection().range();
                    if !self.tabs[self.active].raw
                        && self.tables.iter().any(|table| {
                            selection.start < table.table.range.end
                                && selection.end > table.table.range.start
                        })
                    {
                        self.status = "表をまたぐ編集はソース表示で行ってください".into();
                        self.after_input();
                        return;
                    }
                    let projection = self.cached_projection();
                    let p = self.doc().selection().head;
                    let literal = !self.is_markdown_document()
                        || projection.blocks.iter().any(|block| {
                            matches!(block.kind, BlockKind::Code { .. } | BlockKind::FrontMatter)
                                && block.source.start <= p
                                && p < block.source.end
                        });
                    let plan = enter_edit(self.doc().source(), self.doc().selection(), literal);
                    self.apply_live_edit(plan);
                }
                0x09 => {
                    self.preferred_x = None;
                    self.indent(shift);
                }
                0x1b => {
                    self.dragging = false;
                    let _ = ReleaseCapture();
                }
                _ => return,
            }
            self.after_input();
        }
    }
    fn move_table(&mut self, back: bool, horizontal: bool) {
        let p = self.doc().selection().head;
        let target = self.tables.iter().find_map(|view| {
            view.table.cell_at(p)?;
            let candidate =
                table_navigation_target(&view.table, self.doc().rope(), p, back, horizontal)?;
            if let Some(position) = view.table.cell_at(candidate) {
                let cell = &view.table.rows[position.row].cells[position.column];
                Some(if back && horizontal {
                    cell.content_range.end
                } else {
                    cell.content_range.start
                })
            } else {
                let (before, after) = self.table_body_boundaries(&view.table);
                Some(if back { before } else { after }.unwrap_or(p))
            }
        });
        if let Some(target) = target {
            let _ = self.doc_mut().set_selection(target, target);
        }
    }
    fn indent(&mut self, back: bool) {
        let selection = self.doc().selection();
        if self.active_cell().is_some() {
            return;
        }
        if selection.is_caret() && !back {
            self.edit("    ");
            return;
        }
        let (range, replacement, after) = indentation_edit(self.doc().source(), selection, back);
        if !self.tabs[self.active].raw
            && self.tables.iter().any(|table| {
                range.start < table.table.range.end && range.end > table.table.range.start
            })
        {
            self.status = "表の構造はソース表示で編集できます".into();
            return;
        }
        if let Err(error) = self
            .doc_mut()
            .replace_range_with_selection(range, &replacement, after)
        {
            self.status = error.to_string();
        }
        self.layout_key = None;
    }
    fn move_caret(&mut self, key: u16, extend: bool) {
        let selection = self.doc().selection();
        let p = selection.head;
        if !extend && selection.anchor != p && matches!(key, 0x25 | 0x27) {
            let target = if key == 0x25 {
                selection.anchor.min(p)
            } else {
                selection.anchor.max(p)
            };
            let affinity = if target == p {
                self.doc().selection_affinity()
            } else {
                BoundaryAffinity::After
            };
            let _ = self
                .doc_mut()
                .set_selection_with_affinity(target, target, affinity);
            self.preferred_x = None;
            return;
        }
        if !matches!(key, 0x26 | 0x28) {
            self.preferred_x = None;
        }
        let (mut target, mut affinity) = match key {
            0x25 | 0x27 => {
                let target = self.visible_step(p, key == 0x25);
                (
                    target,
                    if target == p {
                        self.doc().selection_affinity()
                    } else {
                        BoundaryAffinity::After
                    },
                )
            }
            0x26 | 0x28 => {
                let (x, _, _) = self.caret_location();
                let x = *self.preferred_x.get_or_insert(x);
                self.vertical_target(x, key == 0x26)
            }
            0x24 | 0x23 => {
                let (x, y, h) = self.caret_location();
                let _ = x;
                self.hit_with_affinity(if key == 0x24 { -10000.0 } else { 10000.0 }, y + h * 0.5)
            }
            _ => (p, self.doc().selection_affinity()),
        };
        if !self.tabs[self.active].raw
            && let Some(cell) = self.active_cell()
        {
            if key == 0x24 {
                target = cell.start;
                affinity = BoundaryAffinity::After;
            }
            if key == 0x23 {
                target = cell.end;
                affinity = BoundaryAffinity::Before;
            }
            if key == 0x25 && p == cell.start {
                self.move_table(true, true);
                if extend {
                    let head = self.doc().selection().head;
                    let _ = self.doc_mut().set_selection(selection.anchor, head);
                }
                return;
            }
            if key == 0x27 && p == cell.end {
                self.move_table(false, true);
                if extend {
                    let head = self.doc().selection().head;
                    let _ = self.doc_mut().set_selection(selection.anchor, head);
                }
                return;
            }
        }
        let anchor = if extend { selection.anchor } else { target };
        let _ = self
            .doc_mut()
            .set_selection_with_affinity(anchor, target, affinity);
    }
    fn vertical_target(&self, x: f32, back: bool) -> (usize, BoundaryAffinity) {
        let (_, y, _) = self.caret_location();
        let scroll = self.tabs[self.active].scroll;
        // DirectWrite visual line metrics account for wrapping and heading
        // heights; a guessed multiple of the current glyph height does not.
        let mut rows = Vec::new();
        for (view_index, view) in self.views.iter().enumerate() {
            let mut count = 0;
            unsafe {
                let _ = view.layout.GetLineMetrics(None, &mut count);
                let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
                if view
                    .layout
                    .GetLineMetrics(Some(&mut lines), &mut count)
                    .is_ok()
                {
                    let mut top = view.y - scroll;
                    for line in lines {
                        rows.push((top, top + line.height, view_index));
                        top += line.height;
                    }
                }
            }
        }
        let current_top = rows
            .iter()
            .filter(|&&(top, bottom, _)| top <= y + 0.5 && y < bottom)
            .map(|&(top, _, _)| top)
            .max_by(f32::total_cmp)
            .unwrap_or(y);
        rows.retain(|&(top, _, _)| {
            if back {
                top < current_top - 0.5
            } else {
                top > current_top + 0.5
            }
        });
        let at_edge = rows.is_empty();
        rows.sort_by(|a, b| {
            (a.0 - current_top)
                .abs()
                .total_cmp(&(b.0 - current_top).abs())
        });
        let mut destination: Option<(f32, f32, usize, BoundaryAffinity)> = None;
        for (top, bottom, view_index) in rows {
            let vertical_distance = (top - current_top).abs();
            if destination.is_some_and(|(distance, _, _, _)| vertical_distance > distance) {
                break;
            }
            let view = &self.views[view_index];
            for (byte, affinity) in self.view_hit_candidates(view, x, (top + bottom) * 0.5 + scroll)
            {
                let (target_x, target_y, _) = self.caret_location_at(byte, affinity);
                // A projected trailing newline can share its After boundary
                // with the next block. Use the side that maps back to this
                // visual row; an unaddressable layout-only row is not a stop.
                if target_y >= top - 0.5
                    && target_y < bottom
                    && if back {
                        target_y < current_top - 0.5
                    } else {
                        target_y > current_top + 0.5
                    }
                {
                    let horizontal_distance = (target_x - x).abs();
                    // Several table cells can own the same destination row.
                    // Keep the caret nearest preferred X, rather than the
                    // first cell created left-to-right. Preserve affinity ties.
                    if destination.is_none_or(|(_, distance, _, _)| horizontal_distance < distance)
                    {
                        destination =
                            Some((vertical_distance, horizontal_distance, byte, affinity));
                    }
                }
            }
        }
        if let Some((_, _, byte, affinity)) = destination {
            return (byte, affinity);
        }
        let edge = if back { 0 } else { self.doc().byte_len() };
        if at_edge && edge != self.doc().selection().head {
            (
                edge,
                if back {
                    BoundaryAffinity::Before
                } else {
                    BoundaryAffinity::After
                },
            )
        } else {
            (self.doc().selection().head, self.doc().selection_affinity())
        }
    }
    unsafe fn after_input(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let same_document_view = self.layout_key.is_some_and(|(id, revision, _, raw)| {
                (id, revision, raw)
                    == (
                        self.doc().id(),
                        self.doc().revision(),
                        self.tabs[self.active].raw,
                    )
            });
            let _ = self.ensure_layout();
            let p = self.doc().selection().head;
            if let Some(table) = self
                .tables
                .iter()
                .find(|table| table.table.cell_at(p).is_some())
            {
                let key = table.table.range.start;
                let content_width = table.width;
                let current = *self.tabs[self.active]
                    .table_scroll
                    .get(&key)
                    .unwrap_or(&0.0);
                let caret_x = self.composition_caret().0 - self.content_left() + current;
                let next =
                    visible_table_offset(current, caret_x, self.content_width(), content_width);
                if next != current {
                    self.tabs[self.active].table_scroll.insert(key, next);
                    self.layout_key = None;
                    let _ = self.ensure_layout();
                }
            }
            self.update_caret();
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let max = self.editor_bottom() - 8.;
            let (_, y, height) = self.composition_caret();
            if y < self.editor_top() {
                self.scroll(y - self.editor_top());
            } else if y + height > max {
                self.scroll(y + height - max);
            }
            self.refresh_damage(same_document_view);
        }
    }
    unsafe fn copy(&mut self, cut: bool) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let selection = self.doc().selection();
            let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
            if range.is_empty() {
                return;
            }
            if !self.tabs[self.active].raw {
                for t in &self.tables {
                    if let (Some(start), Some(end)) = (
                        t.table.cell_at(selection.anchor),
                        t.table.cell_at(selection.head),
                    ) && start != end
                    {
                        let rect = TableRect { start, end };
                        if cut {
                            if let Some(c) = t.table.cut_tsv(self.doc().source(), rect)
                                && write_clipboard(self.hwnd, &c.tsv)
                            {
                                self.apply_table(c.edit);
                            }
                        } else if let Some(tsv) = t.table.copy_tsv(self.doc().source(), rect) {
                            write_clipboard(self.hwnd, &tsv);
                        }
                        return;
                    }
                }
            }
            let text = if self.tabs[self.active].raw {
                self.doc().source()[range.clone()].to_owned()
            } else {
                selected_token(&self.visible_tokens, range.clone())
                    .map(|t| t.spelling(self.doc().source()))
                    .collect()
            };
            if write_clipboard(self.hwnd, &text) && cut {
                self.edit("");
            }
        }
    }
    fn doc(&self) -> &Document {
        &self.tabs[self.active].doc
    }
    fn link_destination_at(&self, byte: usize) -> Option<String> {
        if !self.is_markdown_document() {
            return None;
        }
        let (id, revision, projection) = self.projection_cache.as_ref()?;
        if *id != self.doc().id() || *revision != self.doc().revision() {
            return None;
        }
        projection
            .runs
            .iter()
            .filter(|run| run.source.contains(&byte))
            .find_map(|run| {
                run.kinds.iter().find_map(|kind| {
                    if let mdlite::projection::InlineKind::Link { destination, .. } = kind {
                        Some(destination.clone())
                    } else {
                        None
                    }
                })
            })
    }
    fn is_markdown_document(&self) -> bool {
        self.tabs[self.active]
            .file
            .as_ref()
            .map(|file| file.path())
            .or_else(|| self.tabs[self.active].persistence.recovered_source())
            .is_none_or(|path| {
                path.extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        matches!(
                            extension.to_ascii_lowercase().as_str(),
                            "md" | "markdown" | "mdown"
                        )
                    })
            })
    }
    fn background_mutation_busy(&self) -> bool {
        self.search_busy() || self.git_mutation_busy()
    }
    fn doc_mut(&mut self) -> &mut Document {
        &mut self.tabs[self.active].doc
    }
    fn new_document(&mut self) {
        if self.shell.compact_window != HWND::default() {
            unsafe {
                self.return_from_compact();
            }
        }
        self.record_navigation();
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            doc: Document::new_unsaved(id, String::new()),
            file: None,
            scroll: 0.0,
            raw: false,
            table_scroll: HashMap::new(),
            persistence: DocumentPersistence::new(self.persistence.workspace_root()),
        });
        self.active = self.tabs.len() - 1;
        self.ensure_active_tab_visible();
        self.projection_cache = None;
        self.parsed_tables.clear();
        self.clear_images();
        self.layout_key = None;
    }
    fn open_path(&mut self, path: PathBuf) {
        if self.composing {
            self.status = "IME確定後に文書を開いてください。現在の本文は保持しています".into();
            return;
        }
        if self.shell.compact_window != HWND::default() {
            unsafe {
                self.return_from_compact();
            }
        }
        if let Some(index) = self.open_tab_for_path(&path) {
            if index != self.active {
                self.switch_tab(index);
            }
            self.status = "既に開いている文書を表示しました".into();
            self.record_recent(&path);
            return;
        }
        match FileSnapshot::open(&path) {
            Ok(loaded) => {
                self.record_navigation();
                let id = self.next_id;
                self.next_id += 1;
                self.tabs.push(Tab {
                    doc: Document::new(id, loaded.source),
                    file: Some(loaded.snapshot),
                    scroll: 0.0,
                    raw: false,
                    table_scroll: HashMap::new(),
                    persistence: DocumentPersistence::new(self.persistence.workspace_root()),
                });
                self.active = self.tabs.len() - 1;
                self.ensure_active_tab_visible();
                self.projection_cache = None;
                self.parsed_tables.clear();
                self.clear_images();
                self.layout_key = None;
                self.record_recent(&path);
            }
            Err(e) => self.status = format!("開けません: {e}"),
        }
    }
    fn open_tab_for_path(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            tab.file
                .as_ref()
                .is_some_and(|file| same_file_identity(file.path(), path))
        })
    }
    fn tab_strip(&self) -> TabStripLayout {
        if !self.shell.enabled {
            return tab_strip_layout(self.width, self.tabs.len(), self.tab_scroll);
        }
        let left = self.editor_left() - 20.;
        let right = self.editor_right() - 28.;
        let mut strip = tab_strip_layout(
            (right - left).max(1.) + LEFT * 2.,
            self.tabs.len(),
            self.tab_scroll,
        );
        strip.left += left - LEFT;
        strip.right += left - LEFT;
        strip
    }
    fn ensure_active_tab_visible(&mut self) {
        let strip = self.tab_strip();
        if !strip.overflow {
            self.tab_scroll = 0;
        } else if self.active < strip.first {
            self.tab_scroll = self.active;
        } else if self.active >= strip.first + strip.visible_count {
            self.tab_scroll = self.active + 1 - strip.visible_count;
        }
    }
    fn scroll_tabs(&mut self, forward: bool) {
        let strip = self.tab_strip();
        if !strip.overflow {
            return;
        }
        let max_first = self.tabs.len().saturating_sub(strip.visible_count);
        self.tab_scroll = if forward {
            (strip.first + 1).min(max_first)
        } else {
            strip.first.saturating_sub(1)
        };
    }
    fn tab_name(&self, index: usize) -> String {
        let tab = &self.tabs[index];
        tab.file
            .as_ref()
            .map(|file| file.path().display().to_string())
            .unwrap_or_else(|| {
                tab.persistence.recovered_source().map_or_else(
                    || format!("無題 {}", tab.doc.id()),
                    |p| format!("復旧: {}", p.display()),
                )
            })
    }
    fn tab_display_name(&self, index: usize) -> String {
        let tab = &self.tabs[index];
        let Some(file) = &tab.file else {
            return tab
                .persistence
                .recovered_source()
                .and_then(Path::file_name)
                .map_or_else(
                    || format!("無題 {}", tab.doc.id()),
                    |p| format!("復旧: {}", p.to_string_lossy()),
                );
        };
        let path = file.path();
        let Some(name) = path.file_name() else {
            return path.display().to_string();
        };
        let name = name.to_string_lossy();
        let duplicate_name = self.tabs.iter().enumerate().any(|(other_index, other)| {
            other_index != index
                && other.file.as_ref().is_some_and(|other_file| {
                    other_file
                        .path()
                        .file_name()
                        .is_some_and(|other_name| other_name.to_string_lossy() == name)
                })
        });
        if duplicate_name {
            path.parent()
                .and_then(Path::file_name)
                .map(|parent| format!("{} · {}", parent.to_string_lossy(), name))
                .unwrap_or_else(|| path.display().to_string())
        } else {
            name.into_owned()
        }
    }
    fn clear_images(&mut self) {
        if self.animation_timer_armed {
            unsafe {
                let _ = KillTimer(Some(self.hwnd), ANIMATION_TIMER);
            }
            self.animation_timer_armed = false;
        }
        self.images.clear();
        self.image_cache.clear();
    }
    fn switch_tab(&mut self, index: usize) {
        if self.composing {
            return;
        }
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.record_navigation();
        if self.shell.compact_window != HWND::default() {
            unsafe {
                self.return_from_compact();
            }
        }
        self.capture_view_anchor();
        self.doc_mut().break_undo_group();
        self.active = index;
        self.invalidate_ime_editor();
        self.ensure_active_tab_visible();
        self.doc_mut().break_undo_group();
        self.last_typing = None;
        self.preferred_x = None;
        self.pending_table_authoring = None;
        self.projection_cache = None;
        self.parsed_tables.clear();
        self.views.clear();
        self.tables.clear();
        self.clear_images();
        self.layout_key = None;
        self.pending_surrogate = None;
    }
    unsafe fn dialog(&mut self, save: bool) -> Option<PathBuf> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            self.release_caret();
            let mut buffer = vec![0u16; 32768];
            let filter = wide("Markdown\0*.md;*.markdown\0すべて\0*.*\0");
            let mut ofn = OPENFILENAMEW {
                lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
                hwndOwner: self.hwnd,
                lpstrFilter: PCWSTR(filter.as_ptr()),
                lpstrFile: PWSTR(buffer.as_mut_ptr()),
                nMaxFile: buffer.len() as u32,
                lpstrDefExt: w!("md"),
                Flags: OFN_EXPLORER
                    | OFN_NOCHANGEDIR
                    | OFN_PATHMUSTEXIST
                    | if save {
                        OFN_OVERWRITEPROMPT
                    } else {
                        OFN_FILEMUSTEXIST
                    },
                ..Default::default()
            };
            let ok = if save {
                GetSaveFileNameW(&mut ofn)
            } else {
                GetOpenFileNameW(&mut ofn)
            };
            if !ok.as_bool() {
                return None;
            }
            let len = buffer.iter().position(|&u| u == 0).unwrap_or(buffer.len());
            Some(PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
        }
    }
    unsafe fn save(&mut self, as_new: bool) -> bool {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.composing {
                return false;
            }
            if as_new || self.tabs[self.active].file.is_none() {
                let Some(path) = self.dialog(true) else {
                    return false;
                };
                return self.save_as_to(&path);
            }
            self.save_current()
        }
    }
    fn save_as_to(&mut self, path: &Path) -> bool {
        if self.composing {
            return false;
        }
        if let Some(index) = self.open_tab_for_path(path) {
            if index == self.active {
                return self.save_current();
            }
            self.status = format!(
                "Save As先は既に開いている文書（{}）です。保存を中止しました",
                self.tab_name(index)
            );
            return false;
        }

        let format = self.tabs[self.active]
            .file
            .as_ref()
            .map(|file| file.format())
            .or(self.tabs[self.active].persistence.format)
            .unwrap_or(FileFormat::Utf8 { bom: false });
        let result = if path.exists() {
            match FileSnapshot::open(path) {
                Ok(mut loaded) => loaded
                    .snapshot
                    .save_with_format(self.doc().source(), format)
                    .map(|()| loaded.snapshot),
                Err(error) => Err(error),
            }
        } else {
            FileSnapshot::create_new(path, self.doc().source(), format)
        };
        match result {
            Ok(snapshot) => {
                self.tabs[self.active].file = Some(snapshot);
                self.features_document_rebound();
                self.projection_cache = None;
                self.parsed_tables.clear();
                self.clear_images();
                self.images.clear();
                self.layout_key = None;
                self.doc_mut().mark_saved();
                self.clear_external_change(self.doc().id());
                self.status =
                    "保存しました — 相対リンク・画像は新しい保存先を基準に解決します".into();
                self.saved_recovery(self.active);
                true
            }
            Err(error) => {
                if matches!(error, mdlite::storage::StorageError::ExternalChange) {
                    self.mark_external_change(self.doc().id());
                }
                self.status = format!("保存失敗 — 本文は保持しています: {error}");
                false
            }
        }
    }
    fn save_current(&mut self) -> bool {
        if self.composing {
            return false;
        }
        let tab = &mut self.tabs[self.active];
        let Some(file) = tab.file.as_mut() else {
            return false;
        };
        match file.save(tab.doc.source()) {
            Ok(()) => {
                tab.doc.mark_saved();
                let id = tab.doc.id();
                self.clear_external_change(id);
                self.status = "保存しました".into();
                self.saved_recovery(self.active);
                true
            }
            Err(error) => {
                if matches!(error, mdlite::storage::StorageError::ExternalChange) {
                    self.mark_external_change(self.doc().id());
                }
                self.status = format!("保存失敗 — 本文は保持しています: {error}");
                false
            }
        }
    }
    unsafe fn confirm_current(&mut self) -> bool {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if !self.doc().is_dirty() {
                return true;
            }
            self.release_caret();
            let document_name = self.tabs[self.active]
                .file
                .as_ref()
                .map(|file| file.path().display().to_string())
                .unwrap_or_else(|| format!("無題 {}", self.doc().id()));
            let prompt = wide(&format!(
                "「{document_name}」の変更を保存しますか？\n「いいえ」を選ぶと、この文書の未保存変更を破棄します。"
            ));
            let result = MessageBoxW(
                Some(self.hwnd),
                PCWSTR(prompt.as_ptr()),
                w!("MDLite"),
                MB_YESNOCANCEL | MB_ICONQUESTION,
            );
            if result == IDYES {
                self.save(false)
            } else {
                if result == IDNO {
                    self.discard_tab_recovery(self.active);
                }
                result == IDNO
            }
        }
    }
    unsafe fn confirm_all(&mut self) -> bool {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.composing {
                return false;
            }
            let old = self.active;
            for i in 0..self.tabs.len() {
                self.active = i;
                if !self.confirm_current() {
                    self.active = old;
                    return false;
                }
            }
            self.active = old;
            true
        }
    }
    unsafe fn command(&mut self, id: usize) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.shell_command(id) {
                return;
            }
            if self.composing {
                return;
            }
            self.doc_mut().break_undo_group();
            self.pending_surrogate = None;
            self.preferred_x = None;
            self.pending_table_authoring = None;
            match id {
                CMD_NEW => self.new_document(),
                CMD_OPEN => {
                    if let Some(p) = self.dialog(false) {
                        self.open_path(p);
                    }
                }
                CMD_SAVE => {
                    self.save(false);
                }
                CMD_SAVE_AS => {
                    self.save(true);
                }
                CMD_CLOSE => {
                    if self.confirm_current() {
                        self.clear_images();
                        self.projection_cache = None;
                        self.parsed_tables.clear();
                        self.tabs.remove(self.active);
                        self.active = self.active.saturating_sub(1);
                        if self.tabs.is_empty() {
                            self.new_document();
                        }
                        self.ensure_active_tab_visible();
                        self.layout_key = None;
                    }
                }
                CMD_RAW => {
                    if !self.is_markdown_document() {
                        self.status = "テキスト文書は原文表示を保持します".into();
                        return;
                    }
                    let tab = &mut self.tabs[self.active];
                    tab.raw = !tab.raw;
                    if tab.raw {
                        self.clear_images();
                    }
                    self.layout_key = None;
                }
                CMD_WORKSPACE => self.choose_workspace(),
                CMD_RECOVERY => self.present_recoveries(),
                CMD_DISCARD_RECOVERY => self.discard_recovery_candidates(),
                CMD_COMPARE => self.compare_disk(),
                CMD_RELOAD => self.reload_disk(),
                _ => {}
            }
        }
    }

    unsafe fn text_layout(
        &self,
        text: &str,
        width: f32,
        size: f32,
    ) -> windows::core::Result<IDWriteTextLayout> {
        let family = if self.shell.enabled {
            &self.shell.settings.font_face
        } else {
            shell::UI_FONT_FAMILY
        };
        unsafe { self.text_layout_family(text, width, size, family) }
    }
    unsafe fn ui_text_layout(
        &self,
        text: &str,
        width: f32,
        size: f32,
    ) -> windows::core::Result<IDWriteTextLayout> {
        unsafe { self.text_layout_family(text, width, size, shell::UI_FONT_FAMILY) }
    }
    unsafe fn text_layout_family(
        &self,
        text: &str,
        width: f32,
        size: f32,
        family: &str,
    ) -> windows::core::Result<IDWriteTextLayout> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let format = self.graphics.write.CreateTextFormat(
                PCWSTR(wide(family).as_ptr()),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ja-jp"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            let (line_height, baseline) = presentation::line_spacing(size);
            format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, line_height, baseline)?;
            self.graphics.write.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                width.max(1.0),
                100000.0,
            )
        }
    }
    unsafe fn tab_label_layout(
        &self,
        label: &str,
        width: f32,
        height: f32,
    ) -> windows::core::Result<IDWriteTextLayout> {
        unsafe {
            let layout = self.ui_text_layout(label, width, 14.0)?;
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            layout.SetMaxHeight(height)?;
            Ok(layout)
        }
    }
    unsafe fn measure_gutter_width(&self, line_count: usize) -> windows::core::Result<f32> {
        unsafe {
            let sample = "9".repeat(presentation::line_number_digits(line_count));
            let layout = self.text_layout(&sample, 1024.0, self.font_size())?;
            layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics)?;
            Ok(metrics.widthIncludingTrailingWhitespace + GUTTER_HORIZONTAL_PADDING)
        }
    }
    unsafe fn gutter_presentation(
        &self,
        projection: &DisplayProjection,
        block: &ProjectionBlock,
        show_marker: bool,
        size: f32,
    ) -> windows::core::Result<GutterPresentation> {
        unsafe {
            let (label, quoted) = block_gutter(projection, block);
            let marker_layout = if let Some(label) = label {
                let layout = self.text_layout(&label, 200.0, size)?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut metrics)?;
                Some((layout, metrics.widthIncludingTrailingWhitespace))
            } else {
                None
            };
            let indent = (block.depth.saturating_sub(1) as f32 * 12.0)
                .max(marker_layout.as_ref().map_or(0.0, |(_, width)| width + 8.0));
            let marker = if show_marker { marker_layout } else { None };
            Ok(GutterPresentation {
                marker,
                quoted,
                indent,
            })
        }
    }
    unsafe fn push_view(
        &mut self,
        text: &str,
        mapping: (Vec<usize>, Vec<usize>),
        mut source: Range<usize>,
        geometry: (f32, f32, f32),
        size: f32,
        table: Option<usize>,
    ) -> windows::core::Result<f32> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let (x, y, width) = geometry;
            let reveal = table.is_none() && self.revealed_source.as_ref() == Some(&source);
            let source_text = self.doc().source_shared();
            let text = if reveal {
                source_text[source.clone()].trim_end_matches(['\r', '\n'])
            } else {
                text
            };
            let (mut map, mut before) = if reveal {
                let map = byte_map(text, source.start);
                (map.clone(), map)
            } else {
                mapping
            };
            let tokens = visible_tokens(text, &map, &before, self.doc().source());
            if let Some(start) = tokens
                .iter()
                .flat_map(|token| token.spans.iter())
                .map(|span| span.start)
                .min()
            {
                source.start = source.start.min(start);
            }
            if !self.tabs[self.active].raw {
                for token in &tokens {
                    map[token.display_utf16.start] = token.spans.first().unwrap().start;
                    before[token.display_utf16.end] = token.spans.last().unwrap().end;
                }
            }
            self.visible_tokens.extend(tokens);
            let layout = self.text_layout(text, width, size)?;
            let mut metrics = DWRITE_TEXT_METRICS::default();
            {
                let _phase = PhaseSpan::new(&self.phase_trace, "layout.view_get_metrics");
                layout.GetMetrics(&mut metrics)?;
            }
            let height = metrics.height.max(presentation::line_height(size));
            self.views.push(TextView {
                layout,
                text: text.encode_utf16().collect(),
                map,
                before,
                x,
                y,
                width,
                height,
                table,
                source,
                marker: None,
                quoted: false,
                thematic_break: false,
            });
            Ok(height)
        }
    }
    /// One parsed projection for the active document revision; layout-only
    /// invalidations (width/table scrolling) reuse it and the Document API buffer.
    fn cached_projection(&mut self) -> Arc<DisplayProjection> {
        let id = self.doc().id();
        let revision = self.doc().revision();
        if let Some((cached_id, cached_revision, projection)) = &self.projection_cache
            && (*cached_id, *cached_revision) == (id, revision)
        {
            return Arc::clone(projection);
        }
        let projection = Arc::new(DisplayProjection::parse(self.doc().source()));
        self.parsed_tables = projection
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::Table))
            .filter_map(|block| {
                Table::parse(self.doc().source(), block.source.clone()).map(Arc::new)
            })
            .collect();
        self.projection_cache = Some((id, revision, Arc::clone(&projection)));
        projection
    }

    unsafe fn ensure_layout(&mut self) -> windows::core::Result<()> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if !self.is_markdown_document() {
                self.tabs[self.active].raw = true;
            }
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            self.width = (client.right - client.left) as f32 / self.dpi;
            self.update_shell_geometry(client.bottom as f32 / self.dpi);
            if self.shell.enabled && self.hwnd == self.shell.main_window {
                self.features_sync();
                self.git_sync();
                self.search_sync();
            }
            let key = (
                self.doc().id(),
                self.doc().revision(),
                self.width.to_bits(),
                self.tabs[self.active].raw,
            );
            let reveal = if self.tabs[self.active].raw {
                None
            } else {
                let projection = self.cached_projection();
                let head = self.doc().selection().head;
                projection.source_delimiter_line_at(head).or_else(|| {
                    projection
                        .blocks
                        .iter()
                        .filter(|block| {
                            matches!(
                                block.kind,
                                BlockKind::Paragraph
                                    | BlockKind::Heading { .. }
                                    | BlockKind::ListItem
                            ) && block.source.start <= head
                                && (head < block.source.end
                                    || (head == self.doc().byte_len() && block.source.end == head))
                        })
                        .min_by_key(|block| block.source.len())
                        .map(|block| block.source.clone())
                })
            };
            let reveal = if self.tabs[self.active].raw {
                None
            } else {
                self.search_reveal().or(reveal)
            };
            self.editor_actions_sync();
            if self.layout_key == Some(key) && self.revealed_source == reveal {
                return Ok(());
            }
            let _phase = PhaseSpan::new(&self.phase_trace, "layout.rebuild");
            self.revealed_source = reveal;
            self.views.clear();
            self.visible_tokens.clear();
            self.tables.clear();
            self.images.clear();
            let source = self.doc().source_shared();
            let line_starts = physical_line_starts(&source);
            self.gutter_width = self.measure_gutter_width(line_starts.len())?;
            let mut y = self.editor_top();
            let width = self.content_width();
            self.gutter_rows.clear();
            if self.tabs[self.active].raw {
                let mut start = 0;
                for line in source.split_inclusive('\n') {
                    let text = line.trim_end_matches(['\r', '\n']);
                    let map = byte_map(text, start);
                    y += self.push_view(
                        text,
                        (map.clone(), map),
                        start..start + line.len(),
                        (self.content_left(), y, width),
                        self.font_size(),
                        None,
                    )?;
                    start += line.len();
                }
                if source.is_empty() || source.ends_with('\n') {
                    let eof = source.len();
                    y += self.push_view(
                        "",
                        (vec![eof], vec![eof]),
                        eof..eof,
                        (self.content_left(), y, width),
                        self.font_size(),
                        None,
                    )?;
                }
            } else {
                let projection = self.cached_projection();
                let mut handled_images = std::collections::HashSet::new();
                let retained_paths: std::collections::HashSet<_> = projection
                    .blocks
                    .iter()
                    .filter_map(|b| {
                        if let BlockKind::Image { destination, .. } = &b.kind {
                            self.image_directory().and_then(|dir| {
                                mdlite::assets::local_image_path(&dir, destination).ok()
                            })
                        } else {
                            None
                        }
                    })
                    .collect();
                self.image_cache
                    .retain(|path, _| retained_paths.contains(path));
                let utf16: Vec<u16> = projection.text.encode_utf16().collect();
                let mut pending_list_items: Vec<PendingListItem> = Vec::new();
                for block in &projection.blocks {
                    let mut pending_index = pending_list_items.len();
                    while pending_index > 0 {
                        pending_index -= 1;
                        if block.source.start >= pending_list_items[pending_index].waiting_for.end {
                            let pending = pending_list_items.remove(pending_index);
                            if let Some(pending) = self.layout_list_item_fragments(
                                &projection,
                                &utf16,
                                pending,
                                &mut y,
                                width,
                                &mut handled_images,
                            )? {
                                pending_list_items.push(pending);
                            }
                        }
                    }
                    if matches!(block.kind, BlockKind::Image { .. })
                        && handled_images.contains(&block.source.start)
                    {
                        continue;
                    }
                    if matches!(block.kind, BlockKind::Paragraph) {
                        let children: Vec<_> = projection
                            .blocks
                            .iter()
                            .filter(|child| {
                                matches!(child.kind, BlockKind::Image { .. })
                                    && block.source.start <= child.source.start
                                    && child.source.end <= block.source.end
                            })
                            .collect();
                        if !children.is_empty() {
                            let mut cursor = block.display_utf16.start;
                            let mut gutter_pending = true;
                            for child in children {
                                if cursor < child.display_utf16.start {
                                    y += self.projected_fragment(
                                        &projection,
                                        &utf16,
                                        cursor..child.display_utf16.start,
                                        block,
                                        gutter_pending,
                                        (y, width),
                                    )?;
                                    gutter_pending = false;
                                }
                                if let BlockKind::Image {
                                    destination,
                                    width: preferred,
                                    ..
                                } = &child.kind
                                {
                                    y = self.layout_image(
                                        &projection,
                                        (block, gutter_pending),
                                        destination,
                                        *preferred,
                                        child.source.clone(),
                                        (y, width),
                                    )?;
                                    gutter_pending = false;
                                    handled_images.insert(child.source.start);
                                }
                                cursor = child.display_utf16.end;
                            }
                            if cursor < block.display_utf16.end {
                                y += self.projected_fragment(
                                    &projection,
                                    &utf16,
                                    cursor..block.display_utf16.end,
                                    block,
                                    gutter_pending,
                                    (y, width),
                                )?;
                            }
                            continue;
                        }
                    }
                    if matches!(block.kind, BlockKind::ListItem) {
                        let children: Vec<_> = projection
                            .blocks
                            .iter()
                            .filter(|child| {
                                child.depth == block.depth + 1
                                    && block.source.start <= child.source.start
                                    && child.source.end <= block.source.end
                                    && child.display_utf16.start >= block.display_utf16.start
                                    && child.display_utf16.end <= block.display_utf16.end
                            })
                            .cloned()
                            .collect();
                        if children
                            .iter()
                            .any(|child| matches!(child.kind, BlockKind::Image { .. }))
                        {
                            let pending = PendingListItem {
                                block: block.clone(),
                                children,
                                cursor: block.display_utf16.start,
                                next_child: 0,
                                waiting_for: 0..0,
                                gutter_pending: true,
                            };
                            if let Some(pending) = self.layout_list_item_fragments(
                                &projection,
                                &utf16,
                                pending,
                                &mut y,
                                width,
                                &mut handled_images,
                            )? {
                                pending_list_items.push(pending);
                            }
                            continue;
                        }
                    }
                    let mut own_fragment = None;
                    if matches!(
                        block.kind,
                        BlockKind::List { .. } | BlockKind::Quote | BlockKind::ListItem
                    ) {
                        let first_child = projection
                            .blocks
                            .iter()
                            .filter(|child| {
                                child.depth > block.depth
                                    && block.source.start <= child.source.start
                                    && child.source.end <= block.source.end
                            })
                            .min_by_key(|child| child.display_utf16.start);
                        if let Some(child) = first_child {
                            if !matches!(block.kind, BlockKind::ListItem)
                                || block.display_utf16.start >= child.display_utf16.start
                            {
                                continue;
                            }
                            // Tight list items have no Paragraph event. Preserve
                            // their own leading text before a nested child list.
                            own_fragment = Some(ProjectionBlock {
                                kind: BlockKind::ListItem,
                                source: block.source.start..child.source.start,
                                display_utf16: block.display_utf16.start..child.display_utf16.start,
                                depth: block.depth,
                            });
                        }
                    }
                    let block = own_fragment.as_ref().unwrap_or(block);
                    if matches!(block.kind, BlockKind::Paragraph)
                        && projection.blocks.iter().any(|child| {
                            matches!(child.kind, BlockKind::Image { .. })
                                && child.display_utf16 == block.display_utf16
                        })
                    {
                        continue;
                    }
                    if matches!(block.kind, BlockKind::Table)
                        && let Some(table) = self
                            .parsed_tables
                            .iter()
                            .find(|table| {
                                table.range.start == block.source.start
                                    && (table.range.end == block.source.end
                                        || source
                                            .get(table.range.end..block.source.end)
                                            .is_some_and(|ending| {
                                                matches!(ending, "\n" | "\r" | "\r\n")
                                            }))
                            })
                            .cloned()
                    {
                        y = self.layout_table(table, y, width)?;
                        continue;
                    }
                    if let BlockKind::Image {
                        destination,
                        width: preferred,
                        ..
                    } = &block.kind
                    {
                        y = self.layout_image(
                            &projection,
                            (block, true),
                            destination,
                            *preferred,
                            block.source.clone(),
                            (y, width),
                        )?;
                        continue;
                    }
                    // A literal block's terminating EOL belongs to the source
                    // boundary, not an extra DirectWrite row before the next
                    // physical line. Internal blank lines remain in the layout.
                    let mut display_range = block.display_utf16.clone();
                    if matches!(
                        block.kind,
                        BlockKind::FrontMatter | BlockKind::Code { fenced: true, .. }
                    ) && display_range.end > display_range.start
                        && utf16[display_range.end - 1] == b'\n' as u16
                    {
                        display_range.end -= 1;
                    }
                    let text = String::from_utf16_lossy(&utf16[display_range.clone()]);
                    let literal = matches!(
                        block.kind,
                        BlockKind::FrontMatter | BlockKind::Code { fenced: true, .. }
                    );
                    if literal && text.is_empty() {
                        continue;
                    }
                    let (mut map, mut before) = if display_range.is_empty()
                        && matches!(block.kind, BlockKind::Heading { .. })
                    {
                        // Empty headings share a display boundary with the next
                        // block. That global boundary may already point beyond
                        // this row; its insertion point belongs before its EOL.
                        let point = block.source.start
                            + source[block.source.clone()]
                                .trim_end_matches(['\r', '\n'])
                                .len();
                        (vec![point], vec![point])
                    } else {
                        projection_mapping(&projection, display_range.clone())
                    };
                    let size = match &block.kind {
                        BlockKind::Heading { level } => {
                            self.font_size() * presentation::heading_scale(*level)
                        }
                        _ => self.font_size(),
                    };
                    let (marker, quoted) = block_gutter(&projection, block);
                    let marker = if let Some(label) = marker {
                        let layout = self.text_layout(&label, 200.0, size)?;
                        let mut metrics = DWRITE_TEXT_METRICS::default();
                        layout.GetMetrics(&mut metrics)?;
                        Some((layout, metrics.widthIncludingTrailingWhitespace))
                    } else {
                        None
                    };
                    let indent = (block.depth.saturating_sub(1) as f32 * 12.0)
                        .max(marker.as_ref().map_or(0.0, |(_, width)| width + 8.0));
                    // Delimiter rows have their own source anchors. The body
                    // view must not claim their bytes or pull the closing
                    // delimiter's caret back into the body after a rebuild.
                    let view_source = if literal {
                        let start = projection
                            .source_delimiter_line_at(block.source.start)
                            .map_or(block.source.start, |line| line.end);
                        let end = projection
                            .source_delimiter_line_at(block.source.end.saturating_sub(1))
                            .filter(|line| line.start >= start)
                            .map_or(block.source.end, |line| line.start);
                        let body = &source[start..end];
                        let content = body
                            .strip_suffix("\r\n")
                            .or_else(|| body.strip_suffix(['\r', '\n']))
                            .unwrap_or(body);
                        let end = start + content.len();
                        // Hidden delimiters share global display boundaries
                        // with following blocks. A literal body's terminal
                        // insertion point remains before its own physical EOL.
                        for byte in map.iter_mut().chain(before.iter_mut()) {
                            *byte = (*byte).clamp(start, end);
                        }
                        start..end
                    } else {
                        block.source.clone()
                    };
                    let height = self.push_view(
                        &text,
                        (map, before),
                        view_source,
                        (self.content_left() + indent, y, width - indent),
                        size,
                        None,
                    )?;
                    y += height
                        + if matches!(block.kind, BlockKind::Heading { .. }) {
                            presentation::heading_gap(size)
                        } else {
                            0.0
                        };
                    if let Some(view) = self.views.last_mut() {
                        view.marker = marker;
                        view.quoted = quoted;
                        view.thematic_break = matches!(block.kind, BlockKind::ThematicBreak);
                    }
                    if let Some(view) = self.views.last() {
                        if self.revealed_source.as_ref() == Some(&view.source) {
                            self.apply_source_styles(
                                &view.layout,
                                &projection,
                                view.source.clone(),
                            )?;
                        } else {
                            Self::apply_projection_styles(
                                &view.layout,
                                &projection,
                                display_range,
                            )?;
                        }
                    }
                    if let Some(view) = self.views.last_mut() {
                        let mut metrics = DWRITE_TEXT_METRICS::default();
                        view.layout.GetMetrics(&mut metrics)?;
                        let height = metrics.height.max(presentation::line_height(size));
                        y += height - view.height;
                        view.height = height;
                    }
                }
                while let Some(pending) = pending_list_items.pop() {
                    let mut next = Some(pending);
                    while let Some(pending) = next.take() {
                        next = self.layout_list_item_fragments(
                            &projection,
                            &utf16,
                            pending,
                            &mut y,
                            width,
                            &mut handled_images,
                        )?;
                    }
                }
            }
            if self.views.is_empty() {
                self.push_view(
                    "",
                    (vec![0], vec![0]),
                    0..0,
                    (self.content_left(), y, width),
                    self.font_size(),
                    None,
                )?;
                y += presentation::line_height(self.font_size());
            }
            if !self.tabs[self.active].raw
                && (self
                    .views
                    .last()
                    .is_some_and(|v| v.source.end < source.len())
                    || (!source.is_empty() && source.ends_with('\n')))
            {
                let eof = source.len();
                self.push_view(
                    "",
                    (vec![eof], vec![eof]),
                    eof..eof,
                    (self.content_left(), y, width),
                    self.font_size(),
                    None,
                )?;
                y += presentation::line_height(self.font_size());
            }
            if !self.tabs[self.active].raw {
                y = self.layout_source_markers(&source, y, width, &line_starts)?;
            }
            self.views
                .sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
            self.source_line_starts = line_starts;
            self.rebuild_gutter_rows();
            self.height = y + 40.0;
            self.layout_key = Some(key);
            Ok(())
        }
    }
    unsafe fn apply_source_styles(
        &self,
        layout: &IDWriteTextLayout,
        projection: &DisplayProjection,
        source: Range<usize>,
    ) -> windows::core::Result<()> {
        unsafe {
            let rope = self.doc().rope();
            let source_units = rope.utf16_code_unit_of_byte(source.start);
            for run in &projection.runs {
                if !run.kinds.iter().any(|kind| {
                    use mdlite::projection::InlineKind;
                    matches!(
                        kind,
                        InlineKind::Strong
                            | InlineKind::Emphasis
                            | InlineKind::Code
                            | InlineKind::Link { .. }
                            | InlineKind::Strikethrough
                    )
                }) {
                    continue;
                }
                let start = run.source.start.max(source.start);
                let end = run.source.end.min(source.end);
                if start >= end {
                    continue;
                }
                let range = DWRITE_TEXT_RANGE {
                    startPosition: (rope.utf16_code_unit_of_byte(start) - source_units) as u32,
                    length: (rope.utf16_code_unit_of_byte(end)
                        - rope.utf16_code_unit_of_byte(start)) as u32,
                };
                for kind in &run.kinds {
                    use mdlite::projection::InlineKind;
                    match kind {
                        InlineKind::Strong => {
                            layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?
                        }
                        InlineKind::Emphasis => {
                            layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?
                        }
                        InlineKind::Code => layout.SetFontFamilyName(w!("Cascadia Mono"), range)?,
                        InlineKind::Link { .. } => layout.SetUnderline(true, range)?,
                        InlineKind::Strikethrough => layout.SetStrikethrough(true, range)?,
                        _ => {}
                    }
                }
            }
            Ok(())
        }
    }
    unsafe fn layout_source_markers(
        &mut self,
        source: &str,
        mut bottom: f32,
        width: f32,
        line_starts: &[usize],
    ) -> windows::core::Result<f32> {
        unsafe {
            let existing_view_count = self.views.len();
            let mut mapped_line_starts = Vec::new();
            let mut empty_anchor_positions = Vec::new();
            let mut source_y = Vec::with_capacity(existing_view_count);
            for view in &self.views {
                source_y.push((view.source.start, view.y));
                if view.source.is_empty() {
                    empty_anchor_positions.push(view.source.start);
                    continue;
                }
                let first = line_starts.partition_point(|start| *start < view.source.start);
                let last = line_starts.partition_point(|start| *start <= view.source.end);
                mapped_line_starts.extend(
                    line_starts[first..last]
                        .iter()
                        .copied()
                        .filter(|start| view.map.binary_search(start).is_ok()),
                );
            }
            mapped_line_starts.sort_unstable();
            mapped_line_starts.dedup();
            empty_anchor_positions.sort_unstable();
            empty_anchor_positions.dedup();

            source_y.sort_unstable_by_key(|(source_start, _)| *source_start);
            let mut suffix_min_y = vec![0.0; source_y.len()];
            for index in (0..source_y.len()).rev() {
                suffix_min_y[index] = if index + 1 < source_y.len() {
                    source_y[index].1.min(suffix_min_y[index + 1])
                } else {
                    source_y[index].1
                };
            }
            let next_y = |position: usize| {
                let index = source_y.partition_point(|(source_start, _)| *source_start < position);
                suffix_min_y.get(index).copied()
            };

            let mut table_ranges: Vec<_> = self
                .tables
                .iter()
                .map(|table| table.table.range.clone())
                .collect();
            table_ranges.sort_unstable_by_key(|range| range.start);
            let mut markers = Vec::new();
            for &position in line_starts {
                let end = source[position..]
                    .find(['\r', '\n'])
                    .map_or(source.len(), |offset| position + offset);
                if source[position..end].trim().is_empty()
                    && mapped_line_starts.binary_search(&position).is_err()
                {
                    let table_index = table_ranges
                        .partition_point(|range| range.start <= position)
                        .saturating_sub(1);
                    if !table_ranges
                        .get(table_index)
                        .is_some_and(|range| range.contains(&position))
                    {
                        markers.push((position, true));
                    }
                }
            }
            markers.extend(self.tables.windows(2).filter_map(|tables| {
                let p = tables[0].table.range.end;
                let next_start = tables[1].table.range.start;
                source
                    .get(p..next_start)
                    .filter(|gap| !gap.is_empty() && gap.trim().is_empty())
                    .map(|_| (p, false))
            }));
            let projection = self.cached_projection();
            markers.extend(line_starts.iter().filter_map(|&start| {
                projection
                    .source_delimiter_line_at(start)
                    .map(|line| (line.start, true))
            }));
            markers.sort_by_key(|(p, _)| *p);
            let mut unique_markers: Vec<(usize, bool)> = Vec::with_capacity(markers.len());
            for (position, takes_row) in markers {
                if let Some((previous, previous_takes_row)) = unique_markers.last_mut()
                    && *previous == position
                {
                    *previous_takes_row |= takes_row;
                } else {
                    unique_markers.push((position, takes_row));
                }
            }
            let mut row_positions = Vec::new();
            let mut row_heights = Vec::new();
            let mut reused_row_anchors = Vec::new();
            let row_height = presentation::line_height(self.font_size());
            for (p, takes_row) in unique_markers {
                if empty_anchor_positions.binary_search(&p).is_ok() {
                    if takes_row {
                        row_positions.push(p);
                        row_heights.push(row_height);
                        reused_row_anchors.push(p);
                    }
                    continue;
                }
                let next_y = next_y(p);
                let y = if takes_row {
                    row_positions.push(p);
                    next_y.unwrap_or(bottom)
                } else {
                    self.source_view_at(p, Some(BoundaryAffinity::After))
                        .and_then(|view| {
                            let position = view
                                .map
                                .partition_point(|&byte| byte < p)
                                .min(view.map.len().saturating_sub(1));
                            layout_caret(&view.layout, position as u32, false)
                                .map(|(_, row_y, _)| view.y + row_y)
                        })
                        .or(next_y)
                        .unwrap_or(bottom)
                };
                let revealed_line = self.revealed_source.as_ref().filter(|line| {
                    line.start == p
                        && projection.source_delimiter_line_at(p).as_ref() == Some(*line)
                });
                let text = revealed_line.map_or("", |line| {
                    source[line.clone()].trim_end_matches(['\r', '\n'])
                });
                let map = byte_map(text, p);
                let height = self.push_view(
                    text,
                    (map.clone(), map),
                    revealed_line.cloned().unwrap_or(p..p),
                    (self.content_left(), y, width),
                    self.font_size(),
                    None,
                )?;
                if takes_row {
                    row_heights.push(height);
                }
            }
            let mut row_offsets = vec![0.];
            for height in row_heights {
                row_offsets.push(row_offsets.last().copied().unwrap() + height);
            }
            for (index, view) in self.views.iter_mut().enumerate() {
                let reused_row_anchor = index < existing_view_count
                    && view.source.is_empty()
                    && reused_row_anchors.binary_search(&view.source.start).is_ok();
                let include_equal = index < existing_view_count && !reused_row_anchor;
                let shifts = presentation::source_row_shift_count(
                    &row_positions,
                    view.source.start,
                    include_equal,
                );
                view.y += row_offsets[shifts];
            }
            for image in &mut self.images {
                let shifts =
                    presentation::source_row_shift_count(&row_positions, image.source.start, true);
                image.y += row_offsets[shifts];
            }
            for table in &mut self.tables {
                let shifts = presentation::source_row_shift_count(
                    &row_positions,
                    table.table.range.start,
                    true,
                );
                table.y += row_offsets[shifts];
            }
            let blank_row_y: HashMap<_, _> = self
                .views
                .iter()
                .filter(|view| {
                    view.source.is_empty()
                        && row_positions.binary_search(&view.source.start).is_ok()
                })
                .map(|view| (view.source.start, view.y))
                .collect();
            let table_anchors_before_blank: HashMap<_, _> = self
                .tables
                .iter()
                .filter_map(|table| {
                    let table_line = line_start(source, table.table.range.start);
                    let index = line_starts.binary_search(&table_line).ok()?;
                    let previous_line = *line_starts.get(index.checked_sub(1)?)?;
                    if source_line_is_blank(source, previous_line)
                        && row_positions.binary_search(&previous_line).is_ok()
                    {
                        blank_row_y
                            .get(&previous_line)
                            .map(|&y| (table.table.range.start, y))
                    } else {
                        None
                    }
                })
                .collect();
            for view in &mut self.views {
                if view.source.is_empty()
                    && let Some(&y) = table_anchors_before_blank.get(&view.source.start)
                {
                    view.y = y;
                }
            }
            bottom += row_offsets.last().copied().unwrap();
            let p = self.doc().selection().head;
            if !self
                .tables
                .iter()
                .any(|table| table.table.range.contains(&p))
                && !self
                    .views
                    .iter()
                    .any(|view| view.map.contains(&p) || view.before.contains(&p))
            {
                let start = line_start(source, p);
                let end = source[start..]
                    .find(['\r', '\n'])
                    .map_or(source.len(), |offset| start + offset);
                let y = self
                    .views
                    .iter()
                    .filter(|view| view.source.start >= start)
                    .map(|view| view.y)
                    .min_by(f32::total_cmp)
                    .unwrap_or(bottom);
                let text = &source[start..end];
                let map = byte_map(text, start);
                let existing_views = self.views.len();
                let height = self.push_view(
                    text,
                    (map.clone(), map),
                    start..end,
                    (self.content_left(), y, width),
                    self.font_size(),
                    None,
                )?;
                for view in &mut self.views[..existing_views] {
                    if view.y >= y {
                        view.y += height;
                    }
                }
                for image in &mut self.images {
                    if image.y >= y {
                        image.y += height;
                    }
                }
                for table in &mut self.tables {
                    if table.y >= y {
                        table.y += height;
                    }
                }
                bottom += height;
            }
            Ok(bottom)
        }
    }
    fn rebuild_gutter_rows(&mut self) {
        let _phase = PhaseSpan::new(&self.phase_trace, "layout.gutter_metrics_rows");
        let mut rows = Vec::new();
        let source = self.doc().source();
        for (view_index, view) in self.views.iter().enumerate() {
            let mut count = 0;
            unsafe {
                let _ = view.layout.GetLineMetrics(None, &mut count);
            }
            if count == 0 {
                continue;
            }
            let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
            if unsafe { view.layout.GetLineMetrics(Some(&mut lines), &mut count) }.is_err() {
                continue;
            }
            let mut position = 0usize;
            let mut top = view.y;
            let bottom = view.y + view.height;
            for line in lines.into_iter().take(count as usize) {
                if top >= bottom - 0.5 {
                    break;
                }
                let byte = if view.source.is_empty() {
                    view.source.start
                } else {
                    view.map
                        .get(position)
                        .or_else(|| view.before.get(position))
                        .copied()
                        .unwrap_or(view.source.end)
                };
                let represented_row = if view.source.is_empty() || view.text.is_empty() {
                    presentation::is_source_line_start(&self.source_line_starts, byte)
                        && (source_line_is_blank(source, byte)
                            || self
                                .projection_cache
                                .as_ref()
                                .is_some_and(|(_, _, projection)| {
                                    projection.source_delimiter_line_at(byte).is_some()
                                }))
                } else {
                    !(line.length == 0 && position >= view.text.len())
                };
                if represented_row {
                    let line_number =
                        presentation::source_line_number(&self.source_line_starts, byte);
                    rows.push(GutterRow {
                        view_index,
                        y: top,
                        baseline: line.baseline,
                        height: line.height,
                        line_number,
                    });
                }
                position = position.saturating_add(line.length as usize);
                top += line.height;
            }
        }
        let image_anchors: HashMap<_, _> = self
            .views
            .iter()
            .enumerate()
            .filter(|(_, view)| view.source.is_empty())
            .map(|(index, view)| (view.source.start, index))
            .collect();
        let (line_height, baseline) = presentation::line_spacing(self.font_size());
        for image in &self.images {
            if let Some(&view_index) = image_anchors.get(&image.source.start) {
                rows.push(GutterRow {
                    view_index,
                    y: image.y,
                    baseline,
                    height: line_height,
                    line_number: presentation::source_line_number(
                        &self.source_line_starts,
                        image.source.start,
                    ),
                });
            }
        }
        rows.sort_by(|a: &GutterRow, b| {
            a.y.total_cmp(&b.y)
                .then(a.view_index.cmp(&b.view_index))
                .then(a.line_number.cmp(&b.line_number))
        });
        let mut seen_lines = HashSet::new();
        let mut seen_baselines = HashSet::new();
        let mut unique_rows = Vec::with_capacity(rows.len());
        for row in rows {
            let baseline_y = row.y + row.baseline;
            if !seen_lines.insert(row.line_number)
                || !seen_baselines.insert((baseline_y * 2.0).round() as i32)
            {
                continue;
            }
            unique_rows.push(row);
        }
        self.gutter_rows = unique_rows;
    }
    unsafe fn layout_list_item_fragments(
        &mut self,
        projection: &DisplayProjection,
        utf16: &[u16],
        mut pending: PendingListItem,
        y: &mut f32,
        width: f32,
        handled_images: &mut std::collections::HashSet<usize>,
    ) -> windows::core::Result<Option<PendingListItem>> {
        unsafe {
            while pending.next_child < pending.children.len() {
                let child = &pending.children[pending.next_child];
                if pending.cursor < child.display_utf16.start {
                    *y += self.projected_fragment(
                        projection,
                        utf16,
                        pending.cursor..child.display_utf16.start,
                        &pending.block,
                        pending.gutter_pending,
                        (*y, width),
                    )?;
                    pending.gutter_pending = false;
                }
                if let BlockKind::Image {
                    destination,
                    width: preferred,
                    ..
                } = &child.kind
                {
                    *y = self.layout_image(
                        projection,
                        (&pending.block, pending.gutter_pending),
                        destination,
                        *preferred,
                        child.source.clone(),
                        (*y, width),
                    )?;
                    handled_images.insert(child.source.start);
                    pending.gutter_pending = false;
                    pending.cursor = pending.cursor.max(child.display_utf16.end);
                    pending.next_child += 1;
                    continue;
                }

                if child.display_utf16.start == pending.block.display_utf16.start {
                    pending.gutter_pending = false;
                }
                pending.cursor = pending.cursor.max(child.display_utf16.end);
                pending.waiting_for = child.source.clone();
                pending.next_child += 1;
                return Ok(Some(pending));
            }
            if pending.cursor < pending.block.display_utf16.end {
                *y += self.projected_fragment(
                    projection,
                    utf16,
                    pending.cursor..pending.block.display_utf16.end,
                    &pending.block,
                    pending.gutter_pending,
                    (*y, width),
                )?;
            }
            Ok(None)
        }
    }
    unsafe fn apply_projection_styles(
        layout: &IDWriteTextLayout,
        projection: &DisplayProjection,
        display_range: Range<usize>,
    ) -> windows::core::Result<()> {
        unsafe {
            for run in &projection.runs {
                let start = run.display_utf16.start.max(display_range.start);
                let end = run.display_utf16.end.min(display_range.end);
                if start >= end {
                    continue;
                }
                let range = DWRITE_TEXT_RANGE {
                    startPosition: (start - display_range.start) as u32,
                    length: (end - start) as u32,
                };
                for kind in &run.kinds {
                    use mdlite::projection::InlineKind;
                    match kind {
                        InlineKind::Strong => {
                            layout.SetFontWeight(DWRITE_FONT_WEIGHT_BOLD, range)?
                        }
                        InlineKind::Emphasis => {
                            layout.SetFontStyle(DWRITE_FONT_STYLE_ITALIC, range)?;
                        }
                        InlineKind::Code => {
                            layout.SetFontFamilyName(w!("Cascadia Mono"), range)?;
                        }
                        InlineKind::Link { .. } => layout.SetUnderline(true, range)?,
                        InlineKind::Strikethrough => layout.SetStrikethrough(true, range)?,
                        _ => {}
                    }
                }
            }
            Ok(())
        }
    }
    unsafe fn projected_fragment(
        &mut self,
        projection: &DisplayProjection,
        utf16: &[u16],
        range: Range<usize>,
        block: &ProjectionBlock,
        show_marker: bool,
        geometry: (f32, f32),
    ) -> windows::core::Result<f32> {
        // SAFETY: the projection owns checked source mappings and the layout is
        // retained by this window's App after the synchronous DirectWrite call.
        unsafe {
            let (y, width) = geometry;
            let text = String::from_utf16_lossy(&utf16[range.clone()]);
            let (map, before) = projection_mapping(projection, range.clone());
            let source = *map.first().unwrap()..*map.last().unwrap();
            let GutterPresentation {
                marker,
                quoted,
                indent,
            } = self.gutter_presentation(projection, block, show_marker, self.font_size())?;
            let mut height = self.push_view(
                &text,
                (map, before),
                source,
                (self.content_left() + indent, y, width - indent),
                self.font_size(),
                None,
            )?;
            let minimum_line_height = presentation::line_height(self.font_size());
            if let Some(view) = self.views.last_mut() {
                view.marker = marker;
                view.quoted = quoted;
                Self::apply_projection_styles(&view.layout, projection, range)?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                view.layout.GetMetrics(&mut metrics)?;
                height = metrics.height.max(minimum_line_height);
                view.height = height;
            }
            Ok(height)
        }
    }
    unsafe fn layout_image(
        &mut self,
        projection: &DisplayProjection,
        gutter: (&ProjectionBlock, bool),
        destination: &str,
        preferred: Option<u32>,
        source: Range<usize>,
        geometry: (f32, f32),
    ) -> windows::core::Result<f32> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let (gutter_block, show_marker) = gutter;
            let (y, available) = geometry;
            let GutterPresentation {
                marker,
                quoted,
                indent,
            } =
                self.gutter_presentation(projection, gutter_block, show_marker, self.font_size())?;
            let x = self.content_left() + indent;
            let available = (available - indent).max(1.0);
            let dir = self.tabs[self.active]
                .file
                .as_ref()
                .and_then(|f| f.path().parent())
                .map(PathBuf::from);
            let mut bitmap = None;
            let mut image_path = None;
            let mut width = available.min(preferred.unwrap_or(480) as f32);
            let mut height = 90.0;
            let mut label = format!("画像: {destination}");
            if let Some(dir) = dir {
                match mdlite::assets::local_image_path(&dir, destination).and_then(|path| {
                    let dimensions = self
                        .image_cache
                        .get(&path)
                        .map(|cached| (cached.width, cached.height))
                        .map(Ok)
                        .unwrap_or_else(|| mdlite::assets::image_dimensions(&path));
                    dimensions.map(|(w, h)| (path, w, h))
                }) {
                    Ok((path, w, h)) => {
                        (width, height) = image_dimensions(w, h, preferred, available);
                        bitmap = self
                            .image_cache
                            .get(&path)
                            .map(|cached| cached.bitmap.clone());
                        image_path = Some(path);
                        label = "画像を読み込み中".into();
                    }
                    Err(e) => label = format!("画像を表示できません: {e}"),
                }
            } else {
                label = "画像を表示するには文書を保存してください".into();
            }
            // Source-affine empty text markers keep image boundaries editable.
            self.push_view(
                "",
                (vec![source.start], vec![source.start]),
                source.start..source.start,
                (x, y, available),
                self.font_size(),
                None,
            )?;
            if let Some(view) = self.views.last_mut() {
                view.marker = marker;
                view.quoted = quoted;
            }
            self.images.push(ImageView {
                bitmap,
                path: image_path,
                source: source.clone(),
                x,
                y,
                width,
                height,
                quoted,
                label,
                load_failed: false,
            });
            self.visible_tokens
                .push(image_token(self.doc().source(), source.clone()));
            self.push_view(
                "",
                (vec![source.end], vec![source.end]),
                source.end..source.end,
                (x, y + height, available),
                self.font_size(),
                None,
            )?;
            Ok(y + height)
        }
    }
    /// Decode/upload only visible images and one viewport of lookahead.
    /// Cache entries own decoders; views hold bitmap clones only near the viewport.
    unsafe fn prepare_images(&mut self) {
        unsafe {
            let bottom = self.animation_viewport();
            let scroll = self.tabs[self.active].scroll;
            let page = (bottom - self.editor_top()).max(1.0);
            let wanted: std::collections::HashSet<_> = self
                .images
                .iter()
                .filter(|image| {
                    image.y + image.height - scroll > self.editor_top() - page
                        && image.y - scroll < bottom + page
                })
                .filter_map(|image| image.path.clone())
                .collect();
            for image in &mut self.images {
                if image
                    .path
                    .as_ref()
                    .is_none_or(|path| !wanted.contains(path))
                {
                    image.bitmap = None;
                }
            }
            for index in 0..self.images.len() {
                let Some(path) = self.images[index].path.clone() else {
                    continue;
                };
                if !wanted.contains(&path) || self.images[index].load_failed {
                    continue;
                }
                if !self.image_cache.contains_key(&path) {
                    match self.load_cached_image(&path, &wanted) {
                        Ok(true) => {}
                        Ok(false) => {
                            self.images[index].label = "画像cacheの保持上限に達しました".into();
                            continue;
                        }
                        Err(error) => {
                            self.images[index].label = format!("画像を表示できません: {error}");
                            self.images[index].load_failed = true;
                            continue;
                        }
                    }
                }
                if let Some(cached) = self.image_cache.get(&path) {
                    self.images[index].bitmap = Some(cached.bitmap.clone());
                    self.images[index].label.clear();
                }
            }
        }
    }
    fn reserve_image_cache(
        &mut self,
        estimate: u64,
        wanted: &std::collections::HashSet<PathBuf>,
    ) -> bool {
        let used: u64 = self
            .image_cache
            .values()
            .map(|cached| cached.retained_bytes)
            .sum();
        if used
            .checked_add(estimate)
            .is_none_or(|sum| sum > IMAGE_CACHE_BYTES)
        {
            for image in &mut self.images {
                if image
                    .path
                    .as_ref()
                    .is_none_or(|path| !wanted.contains(path))
                {
                    image.bitmap = None;
                }
            }
            self.image_cache.retain(|path, _| wanted.contains(path));
        }
        let used: u64 = self
            .image_cache
            .values()
            .map(|cached| cached.retained_bytes)
            .sum();
        used.checked_add(estimate)
            .is_some_and(|sum| sum <= IMAGE_CACHE_BYTES)
    }
    unsafe fn load_cached_image(
        &mut self,
        path: &std::path::Path,
        wanted: &std::collections::HashSet<PathBuf>,
    ) -> std::io::Result<bool> {
        unsafe {
            let animated = path.extension().is_some_and(|ext| {
                matches!(
                    ext.to_string_lossy().to_ascii_lowercase().as_str(),
                    "gif" | "webp"
                )
            });
            let (width, height) = mdlite::assets::image_dimensions(path)?;
            let frame = mdlite::assets::decoded_bytes(width, height)?;
            // GIF canvas + CPU current frame + D2D bitmap. Compressed input is
            // accounted conservatively; codec internals and transient decode are
            // additional and this is not a strict process-memory measurement.
            let input = if animated {
                std::fs::metadata(path)?.len()
            } else {
                0
            };
            let estimate = retained_image_bytes(frame, animated, input)?;
            if !self.reserve_image_cache(estimate, wanted) {
                return Ok(false);
            }
            let mut animation = if animated {
                Some(mdlite::assets::AnimatedImage::open(path)?)
            } else {
                None
            };
            let static_image = if animation.is_none() {
                Some(mdlite::assets::decode(path)?)
            } else {
                None
            };
            let decoded = animation
                .as_ref()
                .map(|a| a.current())
                .or(static_image.as_ref())
                .expect("one decoder branch");
            let actual_frame = mdlite::assets::decoded_bytes(decoded.width, decoded.height)?;
            let actual_estimate = retained_image_bytes(actual_frame, animated, input)?;
            if !self.reserve_image_cache(actual_estimate, wanted) {
                return Ok(false);
            }
            let target = self.target().map_err(std::io::Error::other)?;
            let properties = D2D1_BITMAP_PROPERTIES {
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: 96.0,
                dpiY: 96.0,
            };
            let bitmap = target
                .CreateBitmap(
                    D2D_SIZE_U {
                        width: decoded.width,
                        height: decoded.height,
                    },
                    Some(decoded.pixels.as_ptr().cast()),
                    decoded.width * 4,
                    &properties,
                )
                .map_err(std::io::Error::other)?;
            let dimensions = (decoded.width, decoded.height);
            if animation.as_ref().is_some_and(|a| !a.is_animated()) {
                animation = None;
            }
            self.image_cache.insert(
                path.into(),
                CachedImage {
                    bitmap,
                    width: dimensions.0,
                    height: dimensions.1,
                    retained_bytes: if animation.is_some() {
                        actual_estimate
                    } else {
                        actual_frame
                    },
                    animation,
                    frame_sequence: 0,
                },
            );
            Ok(true)
        }
    }

    unsafe fn layout_table(
        &mut self,
        table: Arc<Table>,
        y: f32,
        available: f32,
    ) -> windows::core::Result<f32> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let index = self.tables.len();
            let cols = table.rows.first().map(|r| r.cells.len()).unwrap_or(0);
            if cols == 0 {
                return Ok(y);
            }
            let source = self.doc().source_shared();
            // Every table owns a real insertion point immediately before its
            // source range, including after an edit shifts the range start.
            let mut markers = vec![table.range.start];
            if let Some(previous) = self.tables.last() {
                markers.extend(table_gap_marker(
                    &source,
                    previous.table.range.end,
                    table.range.start,
                ));
            } else if source[..table.range.start].trim().is_empty() {
                markers.push(0);
            }
            markers.sort_unstable();
            markers.dedup();
            for marker in markers {
                self.push_view(
                    "",
                    (vec![marker], vec![marker]),
                    marker..marker,
                    (self.content_left(), y, available),
                    self.font_size(),
                    None,
                )?;
                // The table start marker shares the header-row baseline. Real
                // blank source rows are inserted once after block layout.
            }
            let mut widths = vec![self.font_size() * 4.75; cols];
            for row in &table.rows {
                for (c, cell) in row.cells.iter().enumerate() {
                    let text = &source[cell.content_range.clone()];
                    let layout =
                        self.text_layout(text, self.font_size() * 32.0, self.font_size())?;
                    let mut m = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut m)?;
                    widths[c] = widths[c].max(
                        (m.widthIncludingTrailingWhitespace + 16.0)
                            .clamp(self.font_size() * 4.75, self.font_size() * 32.0),
                    );
                }
            }
            let total: f32 = widths.iter().sum();
            let offset = *self.tabs[self.active]
                .table_scroll
                .get(&table.range.start)
                .unwrap_or(&0.0);
            let offset = offset.clamp(0.0, (total - available).max(0.0));
            let mut top = y;
            for row in &table.rows {
                let first = self.views.len();
                let mut x = self.content_left() - offset;
                let mut h = presentation::line_height(self.font_size()) + 6.0;
                for (c, cell) in row.cells.iter().enumerate() {
                    let content = &source[cell.content_range.clone()];
                    let (text, map) = unescape_cell(content, cell.content_range.start);
                    h = h.max(
                        self.push_view(
                            &text,
                            (map.clone(), map),
                            cell.content_range.clone(),
                            (x + 6.0, top + 3.0, widths[c] - 12.0),
                            self.font_size(),
                            Some(index),
                        )? + 6.0,
                    );
                    x += widths[c];
                    if let Some(view) = self.views.last() {
                        view.layout.SetTextAlignment(match table.alignments[c] {
                            TableAlignment::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
                            TableAlignment::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
                            TableAlignment::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
                        })?;
                    }
                }
                for view in &mut self.views[first..] {
                    view.height = h - 6.0;
                }
                top += h;
            }
            self.tables.push(TableView {
                table,
                y,
                height: top - y,
                width: total,
            });
            Ok(top)
        }
    }
    fn discard_target(&mut self) {
        for view in &self.views {
            // A failed frame may leave target brushes on retained layouts.
            let _ = unsafe {
                view.layout.SetDrawingEffect(
                    None::<&windows::core::IUnknown>,
                    DWRITE_TEXT_RANGE {
                        startPosition: 0,
                        length: view.text.len() as u32,
                    },
                )
            };
        }
        self.graphics.target = None;
        self.image_cache.clear();
        for image in &mut self.images {
            image.bitmap = None;
        }
        self.layout_key = None;
        self.full_paint = true;
    }
    unsafe fn resize_target(&mut self, _size: D2D_SIZE_U) {
        // BindDC reads the current full client rectangle on the next paint and
        // resizes its backing bitmap. No screen DC is kept between callbacks.
        self.full_paint = true;
    }
    unsafe fn target(&mut self) -> windows::core::Result<ID2D1DCRenderTarget> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if let Some(t) = &self.graphics.target {
                return Ok(t.clone());
            }
            #[cfg(test)]
            paint_fault(PaintFault::Target)?;
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                // Feasibility path: Direct2D renders into each BeginPaint DC.
                // Memory/presentation performance is not established by a build.
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: self.dpi * 96.0,
                dpiY: self.dpi * 96.0,
                ..Default::default()
            };
            let target = self.graphics.factory.CreateDCRenderTarget(&properties)?;
            self.graphics.target = Some(target.clone());
            self.full_paint = true;
            Ok(target)
        }
    }
    unsafe fn draw_gutter_numbers(
        &self,
        target: &ID2D1RenderTarget,
        scroll: f32,
        viewport: f32,
        composition: Option<&CompositionVisual>,
        foreground: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        unsafe {
            let font_size = self.font_size();
            let number_width = (self.gutter_width - GUTTER_HORIZONTAL_PADDING).max(1.0);
            let label_baseline = presentation::line_spacing(font_size).1;
            let top = self.editor_top();
            let first = if let Some(visual) = composition {
                let before_end = self
                    .gutter_rows
                    .partition_point(|row| row.view_index <= visual.view_index);
                let first_before = self.gutter_rows[..before_end]
                    .partition_point(|row| row.y + row.height < scroll + top);
                if first_before < before_end {
                    first_before
                } else {
                    let after_start = self
                        .gutter_rows
                        .partition_point(|row| row.view_index <= visual.end_view_index);
                    after_start
                        + self.gutter_rows[after_start..].partition_point(|row| {
                            row.y + visual.growth + row.height < scroll + top
                        })
                }
            } else {
                self.gutter_rows
                    .partition_point(|row| row.y + row.height < scroll + top)
            };
            for row in &self.gutter_rows[first..] {
                let Some(view) = self.views.get(row.view_index) else {
                    continue;
                };
                if composition.is_some_and(|visual| {
                    row.view_index > visual.view_index && row.view_index <= visual.end_view_index
                }) {
                    continue;
                }
                let view_growth = composition.map_or(0.0, |visual| {
                    if view.y >= visual.row_bottom - 0.1 {
                        visual.growth
                    } else {
                        0.0
                    }
                });
                let table_growth = composition.map_or(0.0, |visual| {
                    if view.table.is_some() && (view.y - scroll - visual.y).abs() < 0.1 {
                        visual.growth
                    } else {
                        0.0
                    }
                });
                let row_y = row.y - scroll + view_growth + table_growth;
                if row_y > viewport {
                    if composition.is_none_or(|visual| row.view_index > visual.end_view_index) {
                        break;
                    }
                    continue;
                }
                if row_y + row.height < top {
                    continue;
                }
                let label =
                    self.text_layout(&row.line_number.to_string(), number_width, font_size)?;
                label.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                label.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                let label_y = row_y + row.baseline - label_baseline;
                target.DrawTextLayout(
                    point(self.editor_left() + GUTTER_LABEL_LEFT, label_y),
                    &label,
                    foreground,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }
    #[cfg(test)]
    unsafe fn paint(&mut self, damage: D2D_RECT_F) -> windows::core::Result<()> {
        unsafe { tests::paint_into_private_dc(self, damage) }
    }
    unsafe fn prepare_paint(&mut self) -> windows::core::Result<()> {
        unsafe {
            if self.minimized {
                return Ok(());
            }
            let layout_invalid = self.layout_key.is_none();
            self.ensure_layout()?;
            let inactive_main = self.shell.enabled
                && self.hwnd == self.shell.main_window
                && self.shell.compact_window != HWND::default();
            if !inactive_main {
                self.ensure_active_tab_visible();
                self.update_caret();
                self.prepare_images();
            }
            self.target()?;
            self.full_paint |= layout_invalid;
            Ok(())
        }
    }
    unsafe fn paint_prepared_to_dc(
        &mut self,
        dc: HDC,
        damage: D2D_RECT_F,
    ) -> windows::core::Result<()> {
        unsafe {
            let _phase = PhaseSpan::new(&self.phase_trace, "paint.total");
            if self.minimized {
                return Ok(());
            }
            let inactive_main = self.shell.enabled
                && self.hwnd == self.shell.main_window
                && self.shell.compact_window != HWND::default();
            // No cold resource preparation is permitted with a paint DC held.
            let t = self
                .graphics
                .target
                .clone()
                .ok_or_else(|| windows::core::Error::from(E_FAIL))?;
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            if dc == HDC::default() {
                return Err(E_FAIL.into());
            }
            #[cfg(test)]
            paint_fault(PaintFault::Bind)?;
            t.BindDC(dc, &client)?;
            #[cfg(test)]
            PAINT_TEST.with(|state| {
                let mut state = state.borrow_mut();
                state.bound_surface = Some(self.hwnd.0 as usize);
            });
            let damage = if self.full_paint {
                let size = t.GetSize();
                rect(0.0, 0.0, size.width, size.height)
            } else {
                damage
            };
            let text = t.CreateSolidColorBrush(&self.shell.tokens.foreground, None)?;
            #[cfg(test)]
            PAINT_TEST.with(|state| state.borrow_mut().damage = Some(damage));
            let muted = t.CreateSolidColorBrush(&self.shell.tokens.muted, None)?;
            let selected = t.CreateSolidColorBrush(&self.shell.tokens.selected, None)?;
            let border = t.CreateSolidColorBrush(&self.shell.tokens.border, None)?;
            let heading = t.CreateSolidColorBrush(&self.shell.tokens.heading, None)?;
            let code_background =
                t.CreateSolidColorBrush(&self.shell.tokens.code_background, None)?;
            let table_background = t.CreateSolidColorBrush(&self.shell.tokens.panel, None)?;
            let link = t.CreateSolidColorBrush(&self.shell.tokens.accent, None)?;
            let caret = self.composition_caret();
            let caret_damaged = GetFocus() == self.hwnd
                && self.native_caret.visible
                && caret_intersects_damage(caret, damage);
            let hidden_caret = if caret_damaged && HideCaret(Some(self.hwnd)).is_ok() {
                Some(HiddenCaret(self.hwnd))
            } else {
                None
            };
            let mut frame = DrawFrame::new(&t);
            frame.push_clip(&damage, D2D1_ANTIALIAS_MODE_ALIASED);
            t.Clear(Some(&self.shell.tokens.background));
            self.draw_shell(&t)?;
            if inactive_main {
                // The inactive main surface shows panels and the Compact
                // placeholder, while sharing the same target/error lifecycle.
                #[cfg(test)]
                paint_fault(PaintFault::Drawing)?;
                self.features_draw(&t)?;
                self.git_draw(&t)?;
                {
                    let _phase = PhaseSpan::new(&self.phase_trace, "paint.frame_finish");
                    frame.finish()?;
                }
                self.full_paint = false;
                return Ok(());
            }
            let mut x = self.editor_left();
            if !self.shell.enabled {
                for (_, label, width) in TOOLBAR {
                    let layout = self.ui_text_layout(label, width - 8.0, 12.0)?;
                    t.DrawTextLayout(point(x, 4.0), &layout, &muted, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                    x += width;
                }
            }
            let strip = self.tab_strip();
            for slot in 0..strip.visible_count {
                let i = strip.first + slot;
                let tab = &self.tabs[i];
                let x = strip.left + slot as f32 * strip.tab_width;
                let (tab_top, tab_height, label_y) = if self.shell.enabled {
                    (
                        shell::TAB_STRIP_TOP,
                        shell::TAB_STRIP_HEIGHT,
                        shell::TAB_LABEL_TOP,
                    )
                } else {
                    (28.0, 43.0, 34.0)
                };
                if i == self.active {
                    t.FillRectangle(&rect(x, tab_top, strip.tab_width, tab_height), &selected);
                }
                let name = self.tab_display_name(i);
                let label = format!("{}{}", name, if tab.doc.is_dirty() { " •" } else { "" });
                let layout =
                    self.tab_label_layout(&label, (strip.tab_width - 32.0).max(1.0), tab_height)?;
                frame.push_clip(
                    &rect(x, tab_top, strip.tab_width, tab_height),
                    D2D1_ANTIALIAS_MODE_ALIASED,
                );
                t.DrawTextLayout(
                    point(x + 8., label_y),
                    &layout,
                    &text,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
                frame.pop_clip();
            }
            if self.shell.enabled {
                let close = self.ui_text_layout("×", 18., 14.)?;
                for slot in 0..strip.visible_count {
                    t.DrawTextLayout(
                        point(
                            strip.left + (slot + 1) as f32 * strip.tab_width - 22.,
                            shell::TAB_LABEL_TOP,
                        ),
                        &close,
                        &muted,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    );
                }
                let add = self.ui_text_layout("+", 24., 18.)?;
                t.DrawTextLayout(
                    point(self.editor_right() - 24., shell::TAB_STRIP_TOP + 2.),
                    &add,
                    &muted,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            if strip.overflow {
                let previous = self.ui_text_layout("‹", 18.0, 18.0)?;
                let next = self.ui_text_layout("›", 18.0, 18.0)?;
                let previous_brush = if strip.first > 0 { &text } else { &muted };
                let next_brush = if strip.first + strip.visible_count < self.tabs.len() {
                    &text
                } else {
                    &muted
                };
                t.DrawTextLayout(
                    point(
                        strip.left - TAB_SCROLL_BUTTON + 5.0,
                        if self.shell.enabled {
                            shell::TAB_STRIP_TOP
                        } else {
                            32.0
                        },
                    ),
                    &previous,
                    previous_brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
                t.DrawTextLayout(
                    point(
                        strip.right + 5.0,
                        if self.shell.enabled {
                            shell::TAB_STRIP_TOP
                        } else {
                            32.0
                        },
                    ),
                    &next,
                    next_brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            let divider_y = if self.shell.enabled {
                shell::TAB_DIVIDER_Y
            } else {
                75.0
            };
            t.DrawLine(
                point(self.editor_left(), divider_y),
                point(self.editor_right(), divider_y),
                &border,
                1.0,
                None,
            );
            let mut r = RECT::default();
            GetClientRect(self.hwnd, &mut r)?;
            let viewport = self.editor_bottom();
            let scroll = self.tabs[self.active].scroll;
            let page = (viewport - self.editor_top()).max(1.0);
            let composition = self.composition_visual();
            let content = (self.height + composition.as_ref().map_or(0.0, |visual| visual.growth)
                - self.editor_top())
            .max(page);
            if content > page {
                let thumb = (page * page / content).max(28.0);
                let y = self.editor_top() + (page - thumb) * scroll / (content - page);
                t.FillRectangle(
                    &rect(
                        self.editor_right() - 6.0,
                        y,
                        if self.dragging_scroll || self.shell.scroll_hover {
                            10.
                        } else {
                            6.
                        },
                        thumb,
                    ),
                    &muted,
                );
            }
            frame.push_clip(
                &rect(
                    self.editor_left() - 8.0,
                    self.editor_top(),
                    self.gutter_width + 8.0,
                    (viewport - self.editor_top()).max(0.0),
                ),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            self.draw_gutter_numbers(&t, scroll, viewport, composition.as_ref(), &muted)?;
            frame.pop_clip();
            frame.push_clip(
                &rect(
                    self.content_left() - 8.0,
                    self.editor_top(),
                    (self.editor_right() - self.content_left() + 8.0).max(0.0),
                    (viewport - self.editor_top()).max(0.0),
                ),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            let selection = self.doc().selection();
            let a = selection.anchor.min(selection.head);
            let b = selection.anchor.max(selection.head);
            for (index, view) in self.views.iter().enumerate() {
                if composition.as_ref().is_some_and(|visual| {
                    index > visual.view_index && index <= visual.end_view_index
                }) {
                    continue;
                }
                let growth = composition.as_ref().map_or(0.0, |visual| {
                    if view.y >= visual.row_bottom - 0.1 {
                        visual.growth
                    } else {
                        0.0
                    }
                });
                let y = view.y - scroll + growth;
                let row_growth = composition.as_ref().map_or(0.0, |visual| {
                    if view.table.is_some() && (view.y - scroll - visual.y).abs() < 0.1 {
                        visual.growth
                    } else {
                        0.0
                    }
                });
                if y + view.height + row_growth < self.editor_top() || y > viewport {
                    continue;
                }
                if view.table.is_some() {
                    t.FillRectangle(
                        &rect(
                            view.x - 6.,
                            y - 3.,
                            view.width + 12.,
                            view.height + row_growth + 6.,
                        ),
                        &table_background,
                    );
                }
                if self.view_has_code_background(view) {
                    t.FillRectangle(
                        &rect(
                            view.x - 4.,
                            y - 2.,
                            view.width + 8.,
                            view.height + row_growth + 4.,
                        ),
                        &code_background,
                    );
                }
                self.paint_inline_code_backgrounds(&t, view, view.x, y, &code_background)?;
                self.apply_link_color(view, &link)?;
                #[cfg(test)]
                paint_fault(PaintFault::Drawing)?;
                let view_text = if self.view_is_heading(view) {
                    &heading
                } else {
                    &text
                };
                self.search_highlight_view(&t, view, view.x, y)?;
                if a < b && !self.composing {
                    let start = view.map.partition_point(|&p| p < a);
                    let end = view.map.partition_point(|&p| p < b);
                    if end > start {
                        let mut count = 0;
                        let _ = view.layout.HitTestTextRange(
                            start as u32,
                            (end - start) as u32,
                            view.x,
                            y,
                            None,
                            &mut count,
                        );
                        let mut hits = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
                        let _ = view.layout.HitTestTextRange(
                            start as u32,
                            (end - start) as u32,
                            view.x,
                            y,
                            Some(&mut hits),
                            &mut count,
                        );
                        for hit in hits {
                            t.FillRectangle(
                                &rect(hit.left, hit.top, hit.width, hit.height),
                                &selected,
                            );
                        }
                    }
                }
                if view.table.is_some() {
                    t.DrawRectangle(
                        &rect(
                            view.x - 6.0,
                            y - 3.0,
                            view.width + 12.0,
                            view.height + row_growth + 6.0,
                        ),
                        &border,
                        1.0,
                        None,
                    );
                }
                if let Some((marker, width)) = &view.marker {
                    t.DrawTextLayout(
                        point(view.x - width - 8.0, y),
                        marker,
                        &muted,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                    );
                }
                if view.quoted {
                    t.DrawLine(
                        point(view.x - 5.0, y),
                        point(view.x - 5.0, y + view.height),
                        &border,
                        2.0,
                        None,
                    );
                }
                if view.thematic_break {
                    let line_y = y + view.height * 0.5;
                    let mut metrics = DWRITE_TEXT_METRICS::default();
                    view.layout.GetMetrics(&mut metrics)?;
                    let line_x = view.x + metrics.widthIncludingTrailingWhitespace + 8.0;
                    if line_x < view.x + view.width {
                        t.DrawLine(
                            point(line_x, line_y),
                            point(view.x + view.width, line_y),
                            &border,
                            1.5,
                            None,
                        );
                    }
                }
                if let Some(visual) = composition
                    .as_ref()
                    .filter(|visual| visual.view_index == index)
                {
                    let clip_height = visual.height;
                    frame.push_clip(
                        &rect(visual.x, visual.y, visual.width, clip_height),
                        D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    );
                    for (range, targeted) in
                        composition_attribute_ranges(&self.ime_attributes, visual.range.clone())
                    {
                        let mut count = 0;
                        let _ = visual.layout.HitTestTextRange(
                            range.start,
                            range.end - range.start,
                            visual.x,
                            visual.y,
                            None,
                            &mut count,
                        );
                        let mut hits = vec![DWRITE_HIT_TEST_METRICS::default(); count as usize];
                        visual.layout.HitTestTextRange(
                            range.start,
                            range.end - range.start,
                            visual.x,
                            visual.y,
                            Some(&mut hits),
                            &mut count,
                        )?;
                        for hit in hits {
                            if targeted {
                                t.FillRectangle(
                                    &rect(hit.left, hit.top, hit.width, hit.height),
                                    &selected,
                                );
                            }
                            t.DrawLine(
                                point(hit.left, hit.top + hit.height - 1.0),
                                point(hit.left + hit.width, hit.top + hit.height - 1.0),
                                &muted,
                                if targeted { 2.0 } else { 1.0 },
                                None,
                            );
                        }
                    }
                    {
                        let _phase = PhaseSpan::new(&self.phase_trace, "paint.paragraph_draw");
                        t.DrawTextLayout(
                            point(visual.x, visual.y),
                            &visual.layout,
                            view_text,
                            D2D1_DRAW_TEXT_OPTIONS_CLIP,
                        );
                    }
                    frame.pop_clip();
                } else {
                    let _phase = PhaseSpan::new(&self.phase_trace, "paint.paragraph_draw");
                    t.DrawTextLayout(
                        point(view.x, y),
                        &view.layout,
                        view_text,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    );
                }
                // Drawing effects borrow this frame's target brush; retained layouts must not keep it alive.
                view.layout.SetDrawingEffect(
                    None::<&windows::core::IUnknown>,
                    DWRITE_TEXT_RANGE {
                        startPosition: 0,
                        length: view.text.len() as u32,
                    },
                )?;
            }
            for image in &self.images {
                if self.composing && a < image.source.end && b > image.source.start {
                    continue;
                }
                let y = image.y - scroll
                    + composition.as_ref().map_or(0.0, |visual| {
                        if image.y >= visual.row_bottom - 0.1 {
                            visual.growth
                        } else {
                            0.0
                        }
                    });
                if y + image.height < self.editor_top() || y > viewport {
                    continue;
                }
                let r = rect(image.x, y, image.width, image.height);
                if image.quoted {
                    t.DrawLine(
                        point(image.x - 5.0, y),
                        point(image.x - 5.0, y + image.height),
                        &border,
                        2.0,
                        None,
                    );
                }
                if let Some(bitmap) = &image.bitmap {
                    t.DrawBitmap(
                        bitmap,
                        Some(&r),
                        1.0,
                        D2D1_BITMAP_INTERPOLATION_MODE_LINEAR,
                        None,
                    );
                } else {
                    t.DrawRectangle(&r, &border, 1.0, None);
                    let layout = self.text_layout(&image.label, image.width - 12.0, 13.0)?;
                    t.DrawTextLayout(
                        point(image.x + 6.0, y + 6.0),
                        &layout,
                        &muted,
                        D2D1_DRAW_TEXT_OPTIONS_CLIP,
                    );
                }
                if a < image.source.end && b > image.source.start {
                    t.DrawRectangle(&r, &selected, 3.0, None);
                }
            }
            frame.pop_clip();
            self.draw_shell(&t)?;
            self.features_draw(&t)?;
            self.git_draw(&t)?;
            self.draw_editor_status(&t, viewport, &muted, &border)?;
            frame.pop_clip();
            let draw_result = {
                let _phase = PhaseSpan::new(&self.phase_trace, "paint.frame_finish");
                frame.finish()
            };
            drop(hidden_caret);
            draw_result?;
            self.full_paint = false;
            self.painted_frame = Some(PaintedFrame {
                hwnd: self.hwnd.0 as usize,
                document_id: self.doc().id(),
                revision: self.doc().revision(),
                sequence: self
                    .painted_frame
                    .map_or(1, |frame| frame.sequence.saturating_add(1)),
            });
            Ok(())
        }
    }
    unsafe fn after_paint_release(&mut self) {
        unsafe {
            let inactive_main = self.shell.enabled
                && self.hwnd == self.shell.main_window
                && self.shell.compact_window != HWND::default();
            if !self.minimized && !inactive_main {
                self.sync_animations();
                self.diagnostic();
            }
        }
    }
    unsafe fn release_caret(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            // Focus loss can be queued until a child EDIT has already created
            // its caret. DestroyCaret has no HWND argument: only destroy a
            // caret currently owned by one of our editor surfaces.
            if native_caret_owner().is_some_and(|owner| {
                owner != HWND::default()
                    && [self.hwnd, self.shell.main_window, self.shell.compact_window]
                        .contains(&owner)
            }) {
                let _ = DestroyCaret();
            }
            self.native_caret = NativeCaretState::default();
        }
    }
    #[cfg(test)]
    fn caret_view(&self) -> Option<&TextView> {
        let byte = self.doc().selection().head;
        self.source_view(byte)
    }
    fn source_view(&self, byte: usize) -> Option<&TextView> {
        self.source_view_at(
            byte,
            (byte == self.doc().selection().head).then(|| self.doc().selection_affinity()),
        )
    }
    fn source_view_at(&self, byte: usize, affinity: Option<BoundaryAffinity>) -> Option<&TextView> {
        if affinity == Some(BoundaryAffinity::After)
            && let Some(view) = self
                .views
                .iter()
                .filter(|view| !view.source.is_empty() && view.source.start == byte)
                .min_by_key(|view| view.source.len())
        {
            return Some(view);
        }
        if affinity == Some(BoundaryAffinity::Before)
            && let Some(view) = self
                .views
                .iter()
                .filter(|view| !view.source.is_empty() && view.before.contains(&byte))
                .min_by_key(|view| {
                    // Adjacent blocks share projection boundaries. A following
                    // short block may retain this byte in before[] even though
                    // the preceding block owns its source and caret geometry.
                    (
                        !(view.source.start <= byte && byte <= view.source.end),
                        view.source.len(),
                    )
                })
        {
            // A marker before a literal block owns the Before side of its
            // start. The following block must not pull that caret downwards.
            if view.source.start == byte
                && let Some(marker) = self
                    .views
                    .iter()
                    .find(|v| v.source.is_empty() && v.source.start == byte)
            {
                return Some(marker);
            }
            return Some(view);
        }
        self.views
            .iter()
            .find(|view| view.source.is_empty() && view.source.start == byte)
            .or_else(|| {
                self.views
                    .iter()
                    .filter(|view| view.map.contains(&byte) || view.before.contains(&byte))
                    .min_by_key(|view| {
                        (
                            !(view.source.start <= byte && byte <= view.source.end),
                            view.source.len(),
                        )
                    })
            })
            .or_else(|| {
                self.views
                    .iter()
                    .filter(|view| view.source.start <= byte && byte <= view.source.end)
                    .min_by_key(|view| view.source.len())
            })
    }
    fn caret_location(&self) -> (f32, f32, f32) {
        let byte = self.doc().selection().head;
        self.caret_location_at(byte, self.doc().selection_affinity())
    }
    fn caret_location_at(&self, byte: usize, affinity: BoundaryAffinity) -> (f32, f32, f32) {
        let scroll = self.tabs[self.active].scroll;
        let candidate = self
            .source_view_at(byte, Some(affinity))
            .or_else(|| self.views.last());
        if let Some(view) = candidate {
            let u = view
                .map
                .partition_point(|&p| p < byte)
                .min(view.map.len().saturating_sub(1));
            let trailing = affinity == BoundaryAffinity::Before && u > 0;
            let position = if trailing {
                view.before
                    .partition_point(|&p| p < byte)
                    .min(view.text.len())
                    .saturating_sub(1)
            } else {
                u
            };
            if let Some((x, y, height)) = layout_caret(&view.layout, position as u32, trailing) {
                return (view.x + x, view.y + y - scroll, height);
            }
        }
        (
            self.content_left(),
            self.editor_top(),
            presentation::line_height(self.font_size()),
        )
    }
    unsafe fn update_caret(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            self.caret = self.caret_location();
            let (x, y, height) = self.composition_caret();
            if GetFocus() == self.hwnd && self.hwnd == self.editing_window() {
                if native_caret_owner() != Some(self.hwnd) {
                    self.native_caret = NativeCaretState::default();
                }
                let size = caret_pixel_size(height, self.dpi);
                let position = ((x * self.dpi).round() as i32, (y * self.dpi).round() as i32);
                let change = required_caret_change(self.native_caret, size, position);
                if change.recreate {
                    self.release_caret();
                    if CreateCaret(self.hwnd, None, size.0, size.1).is_ok() {
                        self.native_caret.size = Some(size);
                    }
                }
                if self.native_caret.size.is_some()
                    && change.move_position
                    && SetCaretPos(position.0, position.1).is_ok()
                {
                    self.native_caret.position = Some(position);
                }
                let right = if self.active_cell().is_some() {
                    self.editor_right() - 12.0
                } else {
                    self.editor_right()
                };
                let visible = caret_in_editor(
                    (x, y, height),
                    self.content_left(),
                    self.editor_top(),
                    right,
                    self.animation_viewport(),
                );
                if self.native_caret.size.is_some() && visible != self.native_caret.visible {
                    let result = if visible {
                        ShowCaret(Some(self.hwnd))
                    } else {
                        HideCaret(Some(self.hwnd))
                    };
                    if result.is_ok() {
                        self.native_caret.visible = visible;
                    }
                }
            }
            self.ime_geometry();
        }
    }
    /// Position in the exact DirectWrite composition layout drawn by paint.
    /// No fallback coordinates are reported as a successful IMM query.
    fn composition_position(&self, utf16: u32) -> Option<(f32, f32, f32)> {
        let selection = self.doc().selection();
        if !self.composing
            || self.ime_affinity != self.doc().selection_affinity()
            || self.ime_owner
                != Some((
                    self.doc().id(),
                    self.doc().revision(),
                    selection.anchor,
                    selection.head,
                ))
        {
            return None;
        }
        let (id, revision, _, raw) = self.layout_key?;
        if (id, revision, raw)
            != (
                self.doc().id(),
                self.doc().revision(),
                self.tabs[self.active].raw,
            )
        {
            return None;
        }
        mdlite::coordinates::utf16_to_byte(&self.preedit, utf16 as usize).ok()?;
        let visual = self.composition_visual()?;
        let trailing = self.preedit.is_empty()
            && selection.is_caret()
            && self.ime_affinity == BoundaryAffinity::Before
            && visual.range.start > 0;
        let position = if trailing {
            visual.range.start - 1
        } else {
            visual.range.start + utf16
        };
        let (x, y, height) = layout_caret(&visual.layout, position, trailing)?;
        Some((visual.x + x, visual.y + y, height))
    }
    fn composition_visual(&self) -> Option<CompositionVisual> {
        if !self.composing {
            return None;
        }
        let selection = self.doc().selection();
        if self.ime_owner
            != Some((
                self.doc().id(),
                self.doc().revision(),
                selection.anchor,
                selection.head,
            ))
            || self.ime_affinity != self.doc().selection_affinity()
        {
            return None;
        }
        let range = selection.range();
        let view = self.source_view(range.start)?;
        let end_view = if range.is_empty() {
            view
        } else {
            self.source_view(range.end)?
        };
        let view_index = self
            .views
            .iter()
            .position(|candidate| std::ptr::eq(view, candidate))?;
        let end_view_index = self
            .views
            .iter()
            .position(|candidate| std::ptr::eq(end_view, candidate))?;
        let start = view
            .map
            .partition_point(|&p| p < range.start)
            .min(view.text.len());
        let end = end_view
            .before
            .partition_point(|&p| p < range.end)
            .min(end_view.text.len());
        let preedit: Vec<_> = self.preedit.encode_utf16().collect();
        let mut text = Vec::with_capacity(start + preedit.len() + end_view.text.len() - end);
        text.extend_from_slice(&view.text[..start]);
        text.extend_from_slice(&preedit);
        text.extend_from_slice(&end_view.text[end..]);
        unsafe {
            // A single transient layout reflows the prefix, preedit and suffix
            // together. Its inherited format shares the committed baseline,
            // line spacing, fallback, width and wrapping policy.
            let format = view.layout.cast::<IDWriteTextFormat>().ok()?;
            let layout = self
                .graphics
                .write
                .CreateTextLayout(&text, &format, view.width, 100000.0)
                .ok()?;
            copy_layout_styles(&view.layout, &layout, 0..start as u32, 0, None).ok()?;
            copy_layout_styles(
                &end_view.layout,
                &layout,
                end as u32..end_view.text.len() as u32,
                (start + preedit.len()) as u32,
                None,
            )
            .ok()?;
            if !preedit.is_empty() {
                let at = if start == view.text.len() {
                    start.saturating_sub(1)
                } else {
                    start
                } as u32;
                // Copy the insertion character's font onto the whole preedit.
                copy_layout_styles(
                    &view.layout,
                    &layout,
                    at..at + 1,
                    start as u32,
                    Some(preedit.len() as u32),
                )
                .ok()?;
            }
            let mut metrics = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut metrics).ok()?;
            Some(CompositionVisual {
                layout,
                range: start as u32..(start + preedit.len()) as u32,
                x: view.x,
                y: view.y - self.tabs[self.active].scroll,
                width: view.width,
                height: metrics.height.max(view.height),
                view_index,
                end_view_index,
                growth: metrics.height.max(view.height) - (end_view.y + end_view.height - view.y),
                row_bottom: end_view.y + end_view.height,
            })
        }
    }
    fn composition_caret(&self) -> (f32, f32, f32) {
        self.composition_position(self.ime_cursor)
            .unwrap_or(self.caret)
    }
    unsafe fn ime_geometry(&self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let context = ImmGetContext(self.hwnd);
            if context.0.is_null() {
                return;
            }
            let (x, y, h) = self.composition_caret();
            let p = POINT {
                x: (x * self.dpi) as i32,
                y: (y * self.dpi) as i32,
            };
            let bottom = POINT {
                x: p.x,
                y: ((y + h) * self.dpi) as i32,
            };
            let _ = ImmSetCompositionWindow(
                context,
                &COMPOSITIONFORM {
                    dwStyle: CFS_POINT,
                    ptCurrentPos: p,
                    ..Default::default()
                },
            );
            let _ = ImmSetCandidateWindow(
                context,
                &CANDIDATEFORM {
                    dwIndex: 0,
                    dwStyle: CFS_EXCLUDE,
                    ptCurrentPos: bottom,
                    rcArea: RECT {
                        left: p.x,
                        top: p.y,
                        right: p.x + 2,
                        bottom: bottom.y,
                    },
                },
            );
            let _ = ImmReleaseContext(self.hwnd, context);
        }
    }
    unsafe fn refresh(&mut self) {
        unsafe { self.refresh_damage(false) }
    }
    unsafe fn refresh_damage(&mut self, editor_only: bool) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            // A new external refresh permits one bounded recovery attempt.
            self.paint_retry_pending = false;
            let _ = self.ensure_layout();
            self.restore_pending_view();
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let page = (self.editor_bottom() - self.editor_top()).max(1.0);
            let height = self.height
                + self
                    .composition_visual()
                    .as_ref()
                    .map_or(0.0, |visual| visual.growth);
            let scroll = self.tabs[self.active]
                .scroll
                .clamp(0.0, (height - self.editor_top() - page).max(0.0));
            self.tabs[self.active].scroll = scroll;
            // Reveal, resize, or restoration can reduce the scroll extent.
            // Native caret coordinates and visibility must use that final
            // viewport, rather than retaining a pre-clamp offscreen position.
            self.update_caret();
            if editor_only && !self.full_paint {
                let left = if self.shell.enabled {
                    self.shell.editor.x
                } else {
                    0.
                };
                let editor = RECT {
                    left: (left * self.dpi).floor() as i32,
                    top: (self.editor_top() * self.dpi).floor() as i32,
                    right: (self.editor_right() * self.dpi).ceil() as i32,
                    bottom: (self.editor_bottom() * self.dpi).ceil() as i32,
                };
                let status = RECT {
                    top: (self.editor_bottom() * self.dpi).floor() as i32,
                    ..r
                };
                let _ = InvalidateRect(Some(self.hwnd), Some(&editor), false);
                let _ = InvalidateRect(Some(self.hwnd), Some(&status), false);
            } else {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
            self.sync_animations();
            self.diagnostic();
        }
    }
    fn scroll(&mut self, delta: f32) {
        let tab = &mut self.tabs[self.active];
        tab.scroll = (tab.scroll + delta).max(0.0);
    }
    fn scroll_at(&mut self, y: f32) {
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        let page = (self.editor_bottom() - self.editor_top()).max(1.0);
        let max = (self.height - self.editor_top() - page).max(0.0);
        self.tabs[self.active].scroll = ((y - self.editor_top()) / page).clamp(0.0, 1.0) * max;
    }
    fn horizontal_scroll(&mut self, delta: f32) {
        let byte = self.doc().selection().head;
        if let Some(t) = self.tables.iter().find(|t| t.table.range.contains(&byte)) {
            let available = self.content_width();
            let offset = self.tabs[self.active]
                .table_scroll
                .entry(t.table.range.start)
                .or_default();
            *offset = (*offset + delta).clamp(0.0, (t.width - available).max(0.0));
            self.layout_key = None;
            if self.hwnd != HWND::default() && unsafe { self.ensure_layout().is_ok() } {
                self.capture_view_anchor();
            }
        }
    }
    #[cfg(test)]
    fn hit(&self, x: f32, y: f32) -> usize {
        self.hit_with_affinity(x, y).0
    }
    fn hit_with_affinity(&self, x: f32, y: f32) -> (usize, BoundaryAffinity) {
        let y = y + self.tabs[self.active].scroll;
        if let Some(image) = self
            .images
            .iter()
            .find(|image| image_contains_point(image, x, y))
        {
            return if y < image.y + image.height / 2.0 {
                (image.source.start, BoundaryAffinity::After)
            } else {
                (image.source.end, BoundaryAffinity::After)
            };
        }
        let Some(view) = self.views.iter().min_by(|a, b| {
            let distance = |v: &TextView| {
                let dy = if y < v.y {
                    v.y - y
                } else if y > v.y + v.height {
                    y - v.y - v.height
                } else {
                    0.0
                };
                let dx = if x < v.x {
                    v.x - x
                } else if x > v.x + v.width {
                    x - v.x - v.width
                } else {
                    0.0
                };
                dy * 10000.0 + dx
            };
            distance(a).total_cmp(&distance(b))
        }) else {
            return (0, BoundaryAffinity::After);
        };
        self.view_hit_candidates(view, x, y)[0]
    }
    fn view_hit_candidates(
        &self,
        view: &TextView,
        x: f32,
        y: f32,
    ) -> [(usize, BoundaryAffinity); 2] {
        let mut trailing = windows::core::BOOL::default();
        let mut inside = windows::core::BOOL::default();
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            let _ = view.layout.HitTestPoint(
                x - view.x,
                y - view.y,
                &mut trailing,
                &mut inside,
                &mut metrics,
            );
        }
        let u = metrics.textPosition as usize
            + if trailing.as_bool() {
                metrics.length as usize
            } else {
                0
            };
        let candidate = |before| {
            let positions = if before { &view.before } else { &view.map };
            let byte = *positions.get(u).unwrap_or(&view.source.end);
            (
                snap(self.doc().rope(), byte, false),
                if before {
                    BoundaryAffinity::Before
                } else {
                    BoundaryAffinity::After
                },
            )
        };
        [
            candidate(trailing.as_bool()),
            candidate(!trailing.as_bool()),
        ]
    }
    fn toggle_task_at(&mut self, x: f32, y: f32) -> bool {
        if self.composing || self.tabs[self.active].raw {
            return false;
        }
        let Some((id, revision, projection)) = &self.projection_cache else {
            return false;
        };
        if (*id, *revision) != (self.doc().id(), self.doc().revision()) {
            return false;
        }
        let y = y + self.tabs[self.active].scroll;
        let Some(view) = self.views.iter().find(|view| {
            x >= view.x && x <= view.x + view.width && y >= view.y && y <= view.y + view.height
        }) else {
            return false;
        };
        let mut trailing = windows::core::BOOL::default();
        let mut inside = windows::core::BOOL::default();
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        if unsafe {
            view.layout.HitTestPoint(
                x - view.x,
                y - view.y,
                &mut trailing,
                &mut inside,
                &mut metrics,
            )
        }
        .is_err()
            || !inside.as_bool()
        {
            return false;
        }
        let Some(&byte) = view.map.get(metrics.textPosition as usize) else {
            return false;
        };
        let Some(run) = projection.runs.iter().find(|run| {
            run.source.contains(&byte)
                && run
                    .kinds
                    .iter()
                    .any(|kind| matches!(kind, mdlite::projection::InlineKind::TaskMarker { .. }))
        }) else {
            return false;
        };
        let range = run.source.clone();
        let selection = self.doc().selection();
        let Some(replacement) =
            task_marker_replacement(self.doc().source().get(range.clone()).unwrap_or_default())
        else {
            return false;
        };
        if let Err(error) =
            self.doc_mut()
                .replace_range_with_selection(range, replacement, selection)
        {
            self.status = error.to_string();
        }
        true
    }
    fn select_at(&mut self, x: f32, y: f32, extend: bool) {
        if self.composing || x < self.content_left() {
            return;
        }
        let (byte, affinity) = self.hit_with_affinity(x, y);
        self.preferred_x = None;
        self.pending_table_authoring = None;
        self.pending_surrogate = None;
        let anchor = if extend {
            self.doc().selection().anchor
        } else {
            byte
        };
        let _ = self
            .doc_mut()
            .set_selection_with_affinity(anchor, byte, affinity);
    }
    fn diagnostic(&self) {
        let Some(path) = &self.diagnostic else {
            return;
        };
        let _phase = PhaseSpan::new(&self.phase_trace, "diagnostic.total");
        let build_phase = PhaseSpan::new(&self.phase_trace, "diagnostic.build");
        let sel = self.doc().selection();
        let views:Vec<_>=self.views.iter().map(|v|serde_json::json!({"source_start":v.source.start,"source_end":v.source.end,"x":v.x,"y":v.y-self.tabs[self.active].scroll,"width":v.width,"height":v.height,"table":v.table,"map":v.map})).collect();
        let tables:Vec<_>=self.tables.iter().map(|t|serde_json::json!({"source_start":t.table.range.start,"source_end":t.table.range.end,"y":t.y,"height":t.height,"width":t.width})).collect();
        // Only the opt-in diagnostic path queries the current owner-thread TIP.
        let (active_input_profile, ime_open_status) = {
            let _phase = PhaseSpan::new(&self.phase_trace, "diagnostic.input_profile");
            (
                active_input_profile_diagnostic(),
                ime_open_status_diagnostic(self.hwnd),
            )
        };
        let painted_frame = self.painted_frame.map(|frame| {
            serde_json::json!({
                "hwnd": frame.hwnd, "document_id": frame.document_id,
                "revision": frame.revision, "sequence": frame.sequence,
            })
        });
        let data = serde_json::json!({"parent_root":self.parent_root_diagnostic(),"document_id":self.doc().id(),"revision":self.doc().revision(),"painted_frame":painted_frame,"source":self.doc().source(),"selection":{"anchor":sel.anchor,"head":sel.head},"dirty":self.doc().is_dirty(),"undo_count":self.doc().undo_count(),"redo_count":self.doc().redo_count(),"raw":self.tabs[self.active].raw,"tabs":self.tabs.len(),"scroll":self.tabs[self.active].scroll,"layout_count":self.views.len(),"animation_timer_armed":self.animation_timer_armed,"animations":self.image_cache.iter().filter(|(_,c)|c.animation.is_some()).map(|(p,c)|serde_json::json!({"path":p,"frame":c.frame_sequence,"running":c.animation.as_ref().and_then(|a|a.next_due()).is_some()})).collect::<Vec<_>>(),"tables":self.tables.len(),"table_geometry":tables,"views":views,"ime":{"active":self.composing,"preedit":self.preedit,"open_status":ime_open_status},"active_input_profile":active_input_profile,"caret":{"x":self.caret.0,"y":self.caret.1,"height":self.caret.2},"native_caret":{"size":self.native_caret.size,"position":self.native_caret.position},"status":self.status});
        let mut data = data;
        data["line_numbers"] = serde_json::json!({
            "physical_line_count": self.source_line_starts.len(),
            "gutter_row_count": self.gutter_rows.len(),
            "gutter_width_dip": self.gutter_width,
            "first_line": self.gutter_rows.first().map(|row| row.line_number),
            "last_line": self.gutter_rows.last().map(|row| row.line_number),
        });
        data["selection_affinity"] =
            serde_json::json!(format!("{:?}", self.doc().selection_affinity()));
        let (x, y, height) = self.composition_caret();
        data["composition_caret"] = serde_json::json!({"x": x, "y": y, "height": height});
        drop(build_phase);
        let serialized = {
            let _phase = PhaseSpan::new(&self.phase_trace, "diagnostic.serialize");
            serde_json::to_vec_pretty(&data)
        };
        if let Ok(data) = serialized {
            let _phase = PhaseSpan::new(&self.phase_trace, "diagnostic.write");
            let _ = std::fs::write(path, data);
        }
    }
}

fn source_line_is_blank(source: &str, start: usize) -> bool {
    if start > source.len() || !source.is_char_boundary(start) {
        return false;
    }
    let end = source[start..]
        .find(['\r', '\n'])
        .map_or(source.len(), |offset| start + offset);
    source[start..end].trim().is_empty()
}

/// Read presentation decisions from the existing parser block tree. Gutter
/// decorations never enter the source/view edit mapping or document history.
fn block_gutter(projection: &DisplayProjection, block: &ProjectionBlock) -> (Option<String>, bool) {
    let contains = |parent: &ProjectionBlock, child: &ProjectionBlock| {
        parent.source.start <= child.source.start && child.source.end <= parent.source.end
    };
    let quoted = projection
        .blocks
        .iter()
        .any(|parent| matches!(parent.kind, BlockKind::Quote) && contains(parent, block));
    let Some(item) = projection
        .blocks
        .iter()
        .filter(|item| matches!(item.kind, BlockKind::ListItem) && contains(item, block))
        .max_by_key(|item| item.depth)
    else {
        return (None, quoted);
    };
    if item.display_utf16.start != block.display_utf16.start {
        return (None, quoted);
    }
    let Some(list) = projection
        .blocks
        .iter()
        .filter(|list| {
            matches!(list.kind, BlockKind::List { .. })
                && list.depth < item.depth
                && contains(list, item)
        })
        .max_by_key(|list| list.depth)
    else {
        return (None, quoted);
    };
    let BlockKind::List { ordered } = list.kind else {
        unreachable!()
    };
    let marker = ordered.map_or_else(
        || "•".to_owned(),
        |start| {
            let previous = projection
                .blocks
                .iter()
                .filter(|sibling| {
                    matches!(sibling.kind, BlockKind::ListItem)
                        && sibling.depth == item.depth
                        && contains(list, sibling)
                        && sibling.source.start < item.source.start
                })
                .count() as u64;
            format!("{}.", start.saturating_add(previous))
        },
    );
    (Some(marker), quoted)
}

fn composition_is_cancelled(flags: u32) -> bool {
    let gcs_flags = GCS_COMPATTR.0
        | GCS_COMPCLAUSE.0
        | GCS_COMPREADSTR.0
        | GCS_COMPREADATTR.0
        | GCS_COMPREADCLAUSE.0
        | GCS_COMPSTR.0
        | GCS_CURSORPOS.0
        | GCS_DELTASTART.0
        | GCS_RESULTCLAUSE.0
        | GCS_RESULTREADCLAUSE.0
        | GCS_RESULTREADSTR.0
        | GCS_RESULTSTR.0;
    flags & gcs_flags == 0
}
fn composition_attribute_ranges(attributes: &[u8], range: Range<u32>) -> Vec<(Range<u32>, bool)> {
    let mut runs: Vec<(Range<u32>, bool)> = Vec::new();
    for position in range.clone() {
        let attribute = attributes
            .get((position - range.start) as usize)
            .copied()
            .unwrap_or(ATTR_INPUT as u8);
        let targeted = matches!(
            attribute as u32,
            ATTR_TARGET_CONVERTED | ATTR_TARGET_NOTCONVERTED
        );
        if let Some((previous, _)) = runs.last_mut().filter(|(_, target)| *target == targeted) {
            previous.end = position + 1;
        } else {
            runs.push((position..position + 1, targeted));
        }
    }
    runs
}

fn ime_open_status_diagnostic(hwnd: HWND) -> serde_json::Value {
    // Keep this context local to the opt-in owner-UI-STA diagnostic query.
    // Acquisition/release follows the existing native IMM call sites; the
    // guard also releases the context if JSON construction unwinds.
    struct Context(HWND, HIMC);
    impl Drop for Context {
        fn drop(&mut self) {
            // SAFETY: this context was acquired for this HWND on the same
            // thread, and the synchronous diagnostic cannot destroy App.
            unsafe {
                let _ = ImmReleaseContext(self.0, self.1);
            }
        }
    }
    // SAFETY: diagnostic() runs on the window's owner UI thread inside run()'s
    // STA. Both queries inspect the context without activating/changing input.
    unsafe {
        let context = ImmGetContext(hwnd);
        if context.0.is_null() {
            // IMM does not document a GetLastError contract here. Do not
            // report an unrelated last error or treat unavailable as closed.
            return serde_json::json!({"available":false,"open":null,"error":"no_input_context"});
        }
        let context = Context(hwnd, context);
        // FALSE is a closed IME, not an API error.
        let open = ImmGetOpenStatus(context.1).as_bool();
        serde_json::json!({"available":true,"open":open,"error":null})
    }
}

fn active_input_profile_diagnostic() -> serde_json::Value {
    // SAFETY: diagnostic() runs on App's owner UI thread inside run()'s STA.
    // These local COM queries inspect its current profile without activating a
    // thread manager/profile or modifying input. Interfaces and returned BSTR
    // are owned RAII values, released before the query returns.
    unsafe {
        let profiles: ITfInputProcessorProfiles = match CoCreateInstance(
            &CLSID_TF_InputProcessorProfiles,
            None,
            CLSCTX_INPROC_SERVER,
        ) {
            Ok(profiles) => profiles,
            Err(error) => {
                return serde_json::json!({"status":"error","stage":"create_profile_query","hresult":format!("0x{:08X}", error.code().0 as u32)});
            }
        };
        let manager: ITfInputProcessorProfileMgr = match profiles.cast() {
            Ok(manager) => manager,
            Err(error) => {
                return serde_json::json!({"status":"error","stage":"profile_manager_interface","hresult":format!("0x{:08X}", error.code().0 as u32)});
            }
        };
        let mut profile = TF_INPUTPROCESSORPROFILE::default();
        // Preserve S_FALSE: the generated Result wrapper treats it as success.
        let result = (manager.vtable().GetActiveProfile)(
            manager.as_raw(),
            &GUID_TFCAT_TIP_KEYBOARD,
            &mut profile,
        );
        if result != S_OK {
            return serde_json::json!({"status":if result==S_FALSE {"unknown"} else {"error"},"stage":"get_active_profile","hresult":format!("0x{:08X}", result.0 as u32)});
        }
        let active = profile.dwFlags & TF_IPP_FLAG_ACTIVE != 0;
        let kind = match profile.dwProfileType {
            TF_PROFILETYPE_INPUTPROCESSOR => "text_service",
            TF_PROFILETYPE_KEYBOARDLAYOUT => "keyboard_layout",
            _ => "unknown",
        };
        let (description, description_error) =
            if profile.dwProfileType == TF_PROFILETYPE_INPUTPROCESSOR {
                match profiles.GetLanguageProfileDescription(
                    &profile.clsid,
                    profile.langid,
                    &profile.guidProfile,
                ) {
                    Ok(description) => (Some(description.to_string()), None),
                    Err(error) => (None, Some(format!("0x{:08X}", error.code().0 as u32))),
                }
            } else {
                (None, None)
            };
        serde_json::json!({
            "status": if active && kind!="unknown" {"active"} else {"unknown"},
            "stage":"get_active_profile","hresult":"0x00000000",
            "active":active,"profile_type":kind,"profile_type_value":profile.dwProfileType,
            "language_id":format!("0x{:04X}",profile.langid),
            "clsid":format!("{:?}",profile.clsid),"profile_guid":format!("{:?}",profile.guidProfile),
            "category_guid":format!("{:?}",profile.catid),"flags":profile.dwFlags,"capabilities":profile.dwCaps,
            "hkl":format!("0x{:X}",profile.hkl.0 as usize),
            "substitute_hkl":format!("0x{:X}",profile.hklSubstitute.0 as usize),
            "description":description,"description_error_hresult":description_error,
        })
    }
}

fn byte_map(text: &str, base: usize) -> Vec<usize> {
    let mut map = Vec::new();
    for (byte, ch) in text.char_indices() {
        for _ in 0..ch.len_utf16() {
            map.push(base + byte);
        }
    }
    map.push(base + text.len());
    map
}
fn projection_mapping(
    projection: &DisplayProjection,
    range: Range<usize>,
) -> (Vec<usize>, Vec<usize>) {
    // A run owns the complete raw spelling of an escaped first character.
    // Its range distinguishes that spelling from unrelated hidden delimiters
    // at the same display boundary; no Markdown syntax is inferred here.
    let mut after: Vec<_> = (range.start..=range.end)
        .map(|u| {
            projection
                .source_byte_at_utf16(u, BoundaryAffinity::After)
                .unwrap_or(projection.source_len)
        })
        .collect();
    let before: Vec<_> = (range.start..=range.end)
        .map(|u| {
            projection
                .source_byte_at_utf16(u, BoundaryAffinity::Before)
                .unwrap_or(projection.source_len)
        })
        .collect();
    for run in &projection.runs {
        if run.display_utf16.start >= range.start && run.display_utf16.start <= range.end {
            let position = run.display_utf16.start - range.start;
            if run.source.start == before[position] && run.source.start < after[position] {
                after[position] = run.source.start;
            }
        }
    }
    (after, before)
}
fn table_navigation_target(
    table: &Table,
    source: &crop::Rope,
    p: usize,
    back: bool,
    horizontal: bool,
) -> Option<usize> {
    let position = table.cell_at(p)?;
    let target = if horizontal {
        let index = position.row * table.columns + position.column;
        let next = if back {
            index.checked_sub(1)
        } else {
            index.checked_add(1)
        };
        next.and_then(|index| {
            table
                .rows
                .get(index / table.columns)
                .and_then(|row| row.cells.get(index % table.columns))
        })
    } else {
        let row = if back {
            position.row.checked_sub(1)
        } else {
            position.row.checked_add(1)
        };
        row.and_then(|row| {
            table
                .rows
                .get(row)
                .and_then(|row| row.cells.get(position.column))
        })
    };
    let byte = target
        .map(|cell| cell.content_range.start)
        .unwrap_or(if back {
            table.range.start.saturating_sub(1)
        } else {
            table.range.end
        });
    Some(snap(source, byte, !back))
}
fn task_marker_replacement(raw: &str) -> Option<&'static str> {
    match raw {
        "[ ]" => Some("[x]"),
        "[x]" | "[X]" => Some("[ ]"),
        _ => None,
    }
}
fn history_shortcut(key: u16, ctrl: bool, shift: bool) -> Option<bool> {
    match (key, ctrl, shift) {
        (0x5a, true, false) => Some(false),
        (0x5a, true, true) | (0x59, true, _) => Some(true),
        _ => None,
    }
}
fn table_key_intent(key: u16, ctrl: bool, shift: bool) -> Option<TableKeyIntent> {
    match key {
        0x09 if !ctrl => Some(if shift {
            TableKeyIntent::PreviousCell
        } else {
            TableKeyIntent::NextCell
        }),
        0x0d => Some(if ctrl {
            TableKeyIntent::Exit
        } else if shift {
            TableKeyIntent::PreviousRow
        } else {
            TableKeyIntent::NextRow
        }),
        0x1b => Some(TableKeyIntent::Exit),
        _ => None,
    }
}
fn table_key_action(
    table: &Table,
    source: &str,
    p: usize,
    intent: TableKeyIntent,
    boundaries: (Option<usize>, Option<usize>),
) -> Option<TableKeyAction> {
    let position = table.cell_at(p)?;
    if intent == TableKeyIntent::Exit {
        return table_exit_action(table, source, boundaries.1);
    }
    let target = match intent {
        TableKeyIntent::NextCell if position.column + 1 < table.columns => {
            Some((position.row, position.column + 1))
        }
        TableKeyIntent::NextCell if position.row + 1 < table.rows.len() => {
            Some((position.row + 1, 0))
        }
        TableKeyIntent::PreviousCell if position.column > 0 => {
            Some((position.row, position.column - 1))
        }
        TableKeyIntent::PreviousCell if position.row > 0 => {
            Some((position.row - 1, table.columns - 1))
        }
        TableKeyIntent::NextRow if position.row + 1 < table.rows.len() => {
            Some((position.row + 1, position.column))
        }
        TableKeyIntent::PreviousRow if position.row > 0 => {
            Some((position.row - 1, position.column))
        }
        _ => None,
    };
    if let Some((row, column)) = target {
        let range = &table.rows[row].cells[column].content_range;
        return Some(TableKeyAction::Move(Selection {
            anchor: range.start,
            head: range.end,
        }));
    }
    if matches!(intent, TableKeyIntent::NextCell | TableKeyIntent::NextRow) {
        let edit = table.insert_row(source, table.rows.len())?;
        let row_text = edit.replacement.trim_start_matches(['\r', '\n']);
        let prefix = edit.replacement.len() - row_text.len();
        let row = mdlite::table::parse_table_row(row_text, edit.range.start + prefix);
        let column = if intent == TableKeyIntent::NextCell {
            0
        } else {
            position.column
        };
        let range = &row.cells.get(column)?.content_range;
        let selection = Selection {
            anchor: range.start,
            head: range.end,
        };
        return Some(TableKeyAction::Edit(edit, selection));
    }
    if let Some(point) = boundaries.0 {
        return Some(TableKeyAction::Move(Selection {
            anchor: point,
            head: point,
        }));
    }
    let eol = table_insert_eol(table, source)?;
    let prefix = &source[..table.range.start];
    let missing = 2usize.saturating_sub(physical_line_endings(prefix));
    if missing == 0 {
        return Some(TableKeyAction::Move(Selection { anchor: 0, head: 0 }));
    }
    Some(TableKeyAction::Edit(
        TableEdit {
            range: 0..0,
            replacement: eol.repeat(missing),
        },
        Selection { anchor: 0, head: 0 },
    ))
}
fn table_insert_eol(table: &Table, source: &str) -> Option<String> {
    let edit = table.insert_row(source, table.rows.len())?;
    let row = edit.replacement.trim_start_matches(['\r', '\n']);
    Some(edit.replacement[..edit.replacement.len() - row.len()].to_owned())
}
fn physical_line_endings(text: &str) -> usize {
    let bytes = text.as_bytes();
    bytes
        .iter()
        .enumerate()
        .filter(|&(index, byte)| {
            *byte == b'\n' || (*byte == b'\r' && bytes.get(index + 1) != Some(&b'\n'))
        })
        .count()
}
fn table_gap_marker(source: &str, start: usize, end: usize) -> Option<usize> {
    let gap = source.get(start..end)?;
    if !gap.trim().is_empty() || physical_line_endings(gap) < 2 {
        return None;
    }
    let first = gap
        .bytes()
        .position(|byte| byte == b'\r' || byte == b'\n')?;
    let base = start + first;
    adjacent_line_ending(source, base, false).map(|range| range.end)
}
fn table_before_insertion(
    source: &str,
    table_start: usize,
    p: usize,
    text: &str,
) -> Option<LiveEdit> {
    let after = source.get(p..table_start)?;
    if !after.trim().is_empty() {
        return None;
    }
    let eol = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let before = source.get(..p)?;
    let body_end = before.trim_end_matches(char::is_whitespace).len();
    let prefix = if body_end == 0 {
        String::new()
    } else {
        eol.repeat(2usize.saturating_sub(physical_line_endings(&before[body_end..])))
    };
    let suffix = eol.repeat(2usize.saturating_sub(physical_line_endings(after)));
    Some(LiveEdit {
        range: p..p,
        caret: p + prefix.len() + text.len(),
        replacement: format!("{prefix}{text}{suffix}"),
    })
}
#[cfg(test)]
fn table_gap_insertion(
    left: &Table,
    right: &Table,
    source: &str,
    p: usize,
    text: &str,
) -> Option<LiveEdit> {
    if table_gap_marker(source, left.range.end, right.range.start) != Some(p) {
        return None;
    }
    table_before_insertion(source, right.range.start, p, text)
}
fn table_exit_action(table: &Table, source: &str, after: Option<usize>) -> Option<TableKeyAction> {
    let tail = source.get(table.range.end..)?;
    if !tail.trim().is_empty()
        && let Some(point) = after.filter(|&point| point < source.len())
    {
        return Some(TableKeyAction::Move(Selection {
            anchor: point,
            head: point,
        }));
    }
    let last_table = mdlite::table::find_tables(source)
        .into_iter()
        .last()
        .unwrap_or_else(|| table.clone());
    let eof_tail = source.get(last_table.range.end..)?;
    let eol = table_insert_eol(&last_table, source)?;
    let missing = 2usize.saturating_sub(physical_line_endings(eof_tail));
    if missing == 0 {
        return Some(TableKeyAction::Move(Selection {
            anchor: source.len(),
            head: source.len(),
        }));
    }
    let replacement = eol.repeat(missing);
    let point = source.len() + replacement.len();
    Some(TableKeyAction::Edit(
        TableEdit {
            range: source.len()..source.len(),
            replacement,
        },
        Selection {
            anchor: point,
            head: point,
        },
    ))
}
fn apply_table_key_action(
    document: &mut Document,
    action: TableKeyAction,
) -> Result<(), mdlite::document::DocumentError> {
    match action {
        TableKeyAction::Move(selection) => document.set_selection(selection.anchor, selection.head),
        TableKeyAction::Edit(edit, selection) => {
            document.replace_range_with_selection(edit.range, &edit.replacement, selection)
        }
    }
}
fn visible_tokens(
    text: &str,
    after: &[usize],
    before: &[usize],
    source: &str,
) -> Vec<VisibleToken> {
    let mut offset = 0;
    let mut tokens = Vec::new();
    for grapheme in text.graphemes(true) {
        let display_start = offset;
        let mut spans = smallvec::SmallVec::<[Range<usize>; 1]>::new();
        for character in grapheme.chars() {
            let next = offset + character.len_utf16();
            if let (Some(&mapped_start), Some(&mapped_end)) = (after.get(offset), before.get(next))
            {
                let mut start = mapped_start;
                let mut end = mapped_end;
                if start >= end {
                    for boundary in [offset, next] {
                        if let (Some(&a), Some(&b)) = (before.get(boundary), after.get(boundary))
                            && a < b
                            && source
                                .get(a..b)
                                .is_some_and(|raw| raw.starts_with('&') && raw.ends_with(';'))
                        {
                            start = a;
                            end = b;
                            break;
                        }
                    }
                }
                if start < end {
                    if let Some(previous) =
                        spans.last_mut().filter(|previous| previous.end >= start)
                    {
                        previous.end = previous.end.max(end);
                    } else {
                        spans.push(start..end);
                    }
                }
            }
            offset = next;
        }
        if !spans.is_empty() {
            let atomic = !spans
                .iter()
                .flat_map(|span| source.get(span.clone()).unwrap_or_default().bytes())
                .eq(grapheme.bytes());
            tokens.push(VisibleToken {
                text: (atomic || spans.len() != 1).then(|| grapheme.to_owned()),
                spans,
                display_utf16: display_start..offset,
                atomic,
            });
        }
    }
    tokens
}
fn live_caret_boundary(tokens: &[VisibleToken], p: usize, markers: &[usize]) -> usize {
    if markers.contains(&p)
        || tokens.iter().any(|token| {
            token.spans.iter().any(|span| {
                p == span.start || p == span.end || (!token.atomic && span.contains(&p))
            })
        })
    {
        return p;
    }
    tokens
        .iter()
        .flat_map(|token| token.spans.iter().flat_map(|span| [span.start, span.end]))
        .chain(markers.iter().copied())
        .min_by_key(|&boundary| (boundary.abs_diff(p), boundary))
        .unwrap_or(p)
}
fn resolve_live_point(
    tokens: &[VisibleToken],
    p: usize,
    markers: &[usize],
    empty_fallback: Option<usize>,
) -> usize {
    if markers.contains(&p) {
        p
    } else if tokens.is_empty() {
        empty_fallback.unwrap_or(p)
    } else {
        live_caret_boundary(tokens, p, markers)
    }
}
fn image_token(source: &str, range: Range<usize>) -> VisibleToken {
    VisibleToken {
        spans: smallvec::smallvec![range.clone()],
        text: Some(source[range].to_owned()),
        display_utf16: 0..0,
        atomic: true,
    }
}
fn adjacent_line_ending(source: &str, p: usize, back: bool) -> Option<Range<usize>> {
    let bytes = source.as_bytes();
    if p > bytes.len() {
        return None;
    }
    if back && p > 0 {
        match bytes[p - 1] {
            b'\n' => Some(
                if p > 1 && bytes[p - 2] == b'\r' {
                    p - 2
                } else {
                    p - 1
                }..p,
            ),
            b'\r' => Some(
                p - 1..if bytes.get(p) == Some(&b'\n') {
                    p + 1
                } else {
                    p
                },
            ),
            _ => None,
        }
    } else if !back {
        match bytes.get(p) {
            Some(b'\r') => Some(
                p..if bytes.get(p + 1) == Some(&b'\n') {
                    p + 2
                } else {
                    p + 1
                },
            ),
            Some(b'\n') => Some(
                if p > 0 && bytes[p - 1] == b'\r' {
                    p - 1
                } else {
                    p
                }..p + 1,
            ),
            _ => None,
        }
    } else {
        None
    }
}
fn line_ending_delete(
    source: &str,
    tables: &[Range<usize>],
    p: usize,
    back: bool,
) -> Option<LiveEdit> {
    let range = adjacent_line_ending(source, p, back)?;
    // One adjacent physical line ending is one join operation. A protected
    // boundary returns a no-op plan so deletion cannot fall through to a glyph.
    let protected = tables
        .iter()
        .any(|table| range.start <= table.end && range.end >= table.start);
    Some(if protected {
        LiveEdit {
            range: p..p,
            replacement: String::new(),
            caret: p,
        }
    } else {
        LiveEdit {
            caret: range.start,
            range,
            replacement: String::new(),
        }
    })
}
fn live_horizontal_step(
    source: &str,
    tokens: &[VisibleToken],
    tables: &[Range<usize>],
    p: usize,
    back: bool,
) -> usize {
    // A physical line ending is a navigation boundary even when the previous
    // delimiter line is currently decorated rather than raw.
    if let Some(range) = adjacent_line_ending(source, p, back)
        && !tables
            .iter()
            .any(|table| range.start >= table.start && range.end <= table.end)
    {
        return if back { range.start } else { range.end };
    }
    if back {
        tokens
            .iter()
            .filter(|token| token.spans.first().is_some_and(|span| span.start < p))
            .filter_map(|token| token.spans.first().map(|span| span.start))
            .max()
            .unwrap_or(p)
    } else {
        tokens
            .iter()
            .filter(|token| token.spans.last().is_some_and(|span| span.end > p))
            .filter_map(|token| token.spans.last().map(|span| span.end))
            .min()
            .unwrap_or(p)
    }
}
fn selected_token(
    tokens: &[VisibleToken],
    selection: Range<usize>,
) -> impl Iterator<Item = &VisibleToken> {
    tokens.iter().filter(move |token| {
        token
            .spans
            .iter()
            .any(|span| span.start < selection.end && span.end > selection.start)
    })
}
fn live_edit(
    source: &str,
    tokens: &[VisibleToken],
    selection: Range<usize>,
    text: &str,
) -> Option<LiveEdit> {
    // Replace visible spelling spans while copying hidden gaps verbatim.
    // The entire bounded result is one document transaction; caret stays at
    // the inserted text, before any retained closing delimiters or destinations.
    if selection.is_empty()
        || source
            .get(selection.clone())?
            .chars()
            .all(char::is_whitespace)
    {
        return Some(LiveEdit {
            caret: selection.start + text.len(),
            range: selection,
            replacement: text.to_owned(),
        });
    }
    let mut spans: Vec<_> = selected_token(tokens, selection.clone())
        .flat_map(|token| token.spans.iter().cloned())
        .collect();
    spans.sort_by_key(|span| span.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for span in spans {
        if let Some(previous) = merged
            .last_mut()
            .filter(|previous| previous.end >= span.start)
        {
            previous.end = previous.end.max(span.end);
        } else {
            merged.push(span);
        }
    }
    let mut start = merged.first()?.start;
    let mut end = merged.last()?.end;
    if selection.start < start
        && source
            .get(selection.start..start)?
            .chars()
            .all(char::is_whitespace)
    {
        start = selection.start;
    }
    if end < selection.end
        && source
            .get(end..selection.end)?
            .chars()
            .all(char::is_whitespace)
    {
        end = selection.end;
    }
    let mut replacement = String::new();
    let mut cursor = start;
    for (index, span) in merged.iter().enumerate() {
        let gap = source.get(cursor..span.start)?;
        // Selected paragraph whitespace is source content, not a Markdown
        // delimiter. Retaining it made a multi-paragraph replacement keep
        // invisible blank lines between the replacement and surviving suffix.
        if !gap.chars().all(char::is_whitespace) {
            replacement.push_str(gap);
        }
        if index == 0 {
            replacement.push_str(text);
        }
        cursor = span.end;
    }
    let tail = source.get(cursor..end)?;
    if !tail.chars().all(char::is_whitespace) {
        replacement.push_str(tail);
    }
    Some(LiveEdit {
        range: start..end,
        replacement,
        caret: start + text.len(),
    })
}
fn image_intersects_viewport(y: f32, height: f32, scroll: f32, bottom: f32) -> bool {
    y + height - scroll > TOP && y - scroll < bottom && bottom > TOP
}
fn image_contains_point(image: &ImageView, x: f32, y: f32) -> bool {
    x >= image.x && x <= image.x + image.width && y >= image.y && y <= image.y + image.height
}
fn image_damage_rect(image: &ImageView, scroll: f32, dpi: f32) -> RECT {
    RECT {
        left: (image.x * dpi).floor() as i32,
        top: ((image.y - scroll) * dpi).floor() as i32,
        right: ((image.x + image.width) * dpi).ceil() as i32,
        bottom: ((image.y - scroll + image.height) * dpi).ceil() as i32,
    }
}
fn retained_image_bytes(frame: u64, animated: bool, input: u64) -> std::io::Result<u64> {
    frame
        .checked_mul(if animated { 3 } else { 1 })
        .and_then(|bytes| bytes.checked_add(input))
        .ok_or_else(|| std::io::Error::other("画像cache保持量のoverflow"))
}
fn image_dimensions(
    intrinsic_width: u32,
    intrinsic_height: u32,
    preferred: Option<u32>,
    available: f32,
) -> (f32, f32) {
    let limit = available.max(1.0);
    let width = preferred
        .map(|width| width.max(1) as f32)
        .unwrap_or((intrinsic_width as f32).min(480.0))
        .min(limit);
    (
        width,
        width * intrinsic_height as f32 / intrinsic_width.max(1) as f32,
    )
}
fn unescape_cell(text: &str, base: usize) -> (String, Vec<usize>) {
    let mut output = String::new();
    let mut map = Vec::new();
    let mut iter = text.char_indices().peekable();
    while let Some((byte, mut ch)) = iter.next() {
        if ch == '\\' && iter.peek().is_some_and(|(_, c)| *c == '|' || *c == '\\') {
            ch = iter.next().unwrap().1;
        }
        output.push(ch);
        for _ in 0..ch.len_utf16() {
            map.push(base + byte);
        }
    }
    map.push(base + text.len());
    (output, map)
}
fn snap(source: &crop::Rope, byte: usize, forward: bool) -> usize {
    let mut byte = byte.min(source.byte_len());
    while !source.is_char_boundary(byte) {
        byte -= 1;
    }
    if mdlite::coordinates::rope_is_grapheme_boundary(source, byte).unwrap_or(false) {
        return byte;
    }
    if forward {
        mdlite::coordinates::rope_next_grapheme_boundary(source, byte).unwrap_or(byte)
    } else {
        mdlite::coordinates::rope_previous_grapheme_boundary(source, byte).unwrap_or(byte)
    }
}
fn is_image_path(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| {
        matches!(
            e.to_string_lossy().to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg"
        )
    })
}
fn decode_dib(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), String> {
    let read = |p: usize| {
        bytes
            .get(p..p + 4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    };
    let header = read(0).ok_or("DIB header missing")? as usize;
    if header < 40 || header > bytes.len() {
        return Err("unsupported DIB header".into());
    }
    let width = read(4).ok_or("width missing")? as i32;
    let height = read(8).ok_or("height missing")? as i32;
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err("invalid DIB dimensions".into());
    }
    let height_abs = height.unsigned_abs();
    if width as u64 * height_abs as u64 > 16_777_216 {
        return Err("image exceeds pixel limit".into());
    }
    let planes = u16::from_le_bytes(bytes[12..14].try_into().unwrap());
    let bits = u16::from_le_bytes(bytes[14..16].try_into().unwrap()) as usize;
    if planes != 1 || !matches!(bits, 24 | 32) || read(16) != Some(0) {
        return Err("clipboard image must be uncompressed RGB24/RGB32".into());
    }
    let stride = (width as usize * bits).div_ceil(32) * 4;
    let needed = header
        .checked_add(
            stride
                .checked_mul(height_abs as usize)
                .ok_or("DIB overflow")?,
        )
        .ok_or("DIB overflow")?;
    if needed > bytes.len() {
        return Err("truncated clipboard image".into());
    }
    let mut rgba = vec![0u8; width as usize * height_abs as usize * 4];
    for y in 0..height_abs as usize {
        let input_y = if height > 0 {
            height_abs as usize - y - 1
        } else {
            y
        };
        for x in 0..width as usize {
            let input = header + input_y * stride + x * (bits / 8);
            let output = (y * width as usize + x) * 4;
            rgba[output..output + 4].copy_from_slice(&[
                bytes[input + 2],
                bytes[input + 1],
                bytes[input],
                255,
            ]);
        }
    }
    Ok((width as u32, height_abs, rgba))
}
unsafe fn read_dib(hwnd: HWND) -> Result<(u32, u32, Vec<u8>), String> {
    // SAFETY: called on the window owner thread; handles and COM resources
    // remain owned by App, and borrowed buffers outlive each synchronous call.
    unsafe {
        OpenClipboard(Some(hwnd)).map_err(|e| e.to_string())?;
        let result = (|| {
            let handle = GetClipboardData(8).map_err(|e| e.to_string())?;
            let global = HGLOBAL(handle.0);
            let pointer = GlobalLock(global) as *const u8;
            if pointer.is_null() {
                return Err("clipboard lock failed".into());
            }
            let size = GlobalSize(global);
            let result = decode_dib(std::slice::from_raw_parts(pointer, size));
            let _ = GlobalUnlock(global);
            result
        })();
        let _ = CloseClipboard();
        result
    }
}
unsafe fn read_clipboard(hwnd: HWND) -> Option<String> {
    // SAFETY: called on the window owner thread; handles and COM resources
    // remain owned by App, and borrowed buffers outlive each synchronous call.
    unsafe {
        if OpenClipboard(Some(hwnd)).is_err() {
            return None;
        }
        let result = (|| {
            let handle = GetClipboardData(CF_TEXT_W).ok()?;
            let global = HGLOBAL(handle.0);
            let ptr = GlobalLock(global) as *const u16;
            if ptr.is_null() {
                return None;
            }
            let length = GlobalSize(global) / 2;
            let slice = std::slice::from_raw_parts(ptr, length);
            let end = slice.iter().position(|&u| u == 0).unwrap_or(length);
            let text = String::from_utf16(&slice[..end]).ok();
            let _ = GlobalUnlock(global);
            text
        })();
        let _ = CloseClipboard();
        result
    }
}
unsafe fn write_clipboard(hwnd: HWND, text: &str) -> bool {
    // SAFETY: called on the window owner thread; handles and COM resources
    // remain owned by App, and borrowed buffers outlive each synchronous call.
    unsafe {
        let utf16 = wide(text);
        let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, utf16.len() * 2) else {
            return false;
        };
        let pointer = GlobalLock(memory) as *mut u16;
        if pointer.is_null() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        std::ptr::copy_nonoverlapping(utf16.as_ptr(), pointer, utf16.len());
        let _ = GlobalUnlock(memory);
        if OpenClipboard(Some(hwnd)).is_err() {
            let _ = GlobalFree(Some(memory));
            return false;
        }
        let success =
            EmptyClipboard().is_ok() && SetClipboardData(CF_TEXT_W, Some(HANDLE(memory.0))).is_ok();
        let _ = CloseClipboard();
        if !success {
            let _ = GlobalFree(Some(memory));
        }
        success
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owner_draw_counted_text_keeps_empty_surface_and_draws_japanese_caption() {
        struct Surface(HDC, HBITMAP, HGDIOBJ);
        impl Drop for Surface {
            fn drop(&mut self) {
                unsafe {
                    let _ = SelectObject(self.0, self.2);
                    let _ = DeleteObject(self.1.into());
                    let _ = DeleteDC(self.0);
                }
            }
        }
        unsafe {
            let dc = CreateCompatibleDC(None);
            assert!(!dc.0.is_null());
            let bitmap = CreateCompatibleBitmap(dc, 160, 40);
            assert!(!bitmap.0.is_null());
            let surface = Surface(dc, bitmap, SelectObject(dc, bitmap.into()));
            let bounds = RECT {
                left: 0,
                top: 0,
                right: 160,
                bottom: 40,
            };
            let _ = FillRect(dc, &bounds, HBRUSH(GetStockObject(WHITE_BRUSH).0));
            let _ = SelectObject(dc, GetStockObject(DEFAULT_GUI_FONT));
            let _ = SetTextColor(dc, COLORREF(0));
            let _ = SetBkMode(dc, TRANSPARENT);
            let font = GetCurrentObject(dc, OBJ_FONT);
            let mut area = bounds;
            assert_eq!(
                draw_gdi_text(dc, "", &mut area, DT_SINGLELINE | DT_NOPREFIX),
                0
            );
            assert_eq!(area, bounds);
            assert_eq!(GetCurrentObject(dc, OBJ_FONT), font);
            assert!((0..40).all(|y| (0..160).all(|x| GetPixel(dc, x, y) == COLORREF(0xffffff))));
            assert!(
                draw_gdi_text(
                    dc,
                    "更新 日本語 A&B",
                    &mut area,
                    DT_SINGLELINE | DT_NOPREFIX
                ) > 0
            );
            assert_eq!(area, bounds);
            assert_eq!(GetCurrentObject(dc, OBJ_FONT), font);
            assert!((0..40).any(|y| (0..160).any(|x| GetPixel(dc, x, y) == COLORREF(0))));
            drop(surface);
        }
    }

    fn startup_test_graphics() -> Graphics {
        unsafe {
            Graphics {
                factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap(),
                write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap(),
                target: None,
            }
        }
    }

    fn startup_fixture_path(index: usize) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mdlite-startup-{}-{nonce}-{index}.md",
            std::process::id()
        ))
    }

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new(label: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mdlite-native-{label}-{}-{nonce}",
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn phase_trace_preserves_prior_evidence_and_counts_reentrant_drops() {
        let directory = TestDirectory::new("phase-trace");
        let diagnostic = directory.path().join("fixture.json");
        let sidecar = diagnostic.with_extension("phases.jsonl");
        let trace = PhaseTrace::open(&diagnostic).unwrap();
        assert!(PhaseTrace::open(&diagnostic).is_none());
        {
            let _file = trace.file.borrow_mut();
            trace.record("test", "reentrant");
        }
        {
            let _phase = PhaseSpan::new(&Some(Rc::clone(&trace)), "test.scope");
        }
        drop(trace);
        let records: Vec<serde_json::Value> = std::fs::read_to_string(&sidecar)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["event"], "opened");
        assert_eq!(records[1]["event"], "begin");
        assert_eq!(records[2]["event"], "end");
        assert_eq!(records[2]["dropped_total"], 1);
        assert!(
            records
                .windows(2)
                .all(|pair| pair[0]["t_us"].as_u64() <= pair[1]["t_us"].as_u64())
        );
        assert!(
            records
                .iter()
                .all(|record| record.as_object().unwrap().len() == 4)
        );
        let before = std::fs::read(&sidecar).unwrap();
        {
            let _phase = PhaseSpan::new(&None, "disabled");
        }
        assert_eq!(std::fs::read(&sidecar).unwrap(), before);
    }

    #[test]
    fn document_identity_duplicate_open_activates_existing_for_path_aliases() {
        let directory = TestDirectory::new("duplicate-open");
        let original = directory.path().join("note.md");
        std::fs::write(&original, "alpha beta").unwrap();
        let alias = directory.path().join("hard-link.md");
        std::fs::hard_link(&original, &alias).unwrap();
        let nested = directory.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        let other = nested.join("note.md");
        std::fs::write(&other, "other text").unwrap();

        let mut app = composing_test_app("unsaved");
        app.clear_composition();
        app.open_path(original.clone());
        let document_id = app.doc().id();
        app.doc_mut().set_selection(0, 5).unwrap();
        app.doc_mut().replace_selection("omega").unwrap();
        app.doc_mut().set_selection(0, 5).unwrap();
        app.doc_mut().replace_selection("OMEGA").unwrap();
        assert!(app.doc_mut().undo());
        app.doc_mut().set_selection(10, 5).unwrap();
        let source = app.doc().source().to_owned();
        let selection = app.doc().selection();
        let undo_count = app.doc().undo_count();
        let redo_count = app.doc().redo_count();
        assert!(app.doc().is_dirty());

        app.open_path(other);
        assert_eq!(app.active, 2);
        assert_ne!(app.tab_display_name(1), app.tab_display_name(2));

        app.open_path(directory.path().join(".").join("note.md"));
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active, 1);
        assert_eq!(app.doc().id(), document_id);
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().selection(), selection);
        assert_eq!(app.doc().undo_count(), undo_count);
        assert_eq!(app.doc().redo_count(), redo_count);
        assert!(app.doc().is_dirty());

        app.switch_tab(2);
        app.open_path(alias);
        assert_eq!(app.tabs.len(), 3);
        assert_eq!(app.active, 1);
        assert_eq!(app.doc().id(), document_id);
        assert_eq!(app.doc().selection(), selection);
        assert!(app.doc_mut().redo());
        assert_eq!(app.doc().source(), "OMEGA beta");
    }

    #[test]
    fn document_identity_save_as_refuses_clean_or_dirty_open_target() {
        for (dirty_target, remove_target) in [(false, false), (true, false), (false, true)] {
            let label = match (dirty_target, remove_target) {
                (true, _) => "save-as-dirty-target",
                (_, true) => "save-as-removed-target",
                _ => "save-as-clean-target",
            };
            let directory = TestDirectory::new(label);
            let source_path = directory.path().join("source.md");
            let target_path = directory.path().join("target.md");
            std::fs::write(&source_path, "source text").unwrap();
            std::fs::write(&target_path, "target text").unwrap();

            let mut app = composing_test_app("unsaved");
            app.clear_composition();
            app.open_path(source_path.clone());
            app.open_path(target_path.clone());
            if dirty_target {
                app.switch_tab(2);
                app.doc_mut().set_selection(0, 6).unwrap();
                app.doc_mut().replace_selection("changed").unwrap();
            }
            app.switch_tab(1);
            app.doc_mut().set_selection(0, 6).unwrap();
            app.doc_mut().replace_selection("edited").unwrap();

            let source_state = (
                app.doc().id(),
                app.doc().source().to_owned(),
                app.doc().selection(),
                app.doc().undo_count(),
                app.doc().redo_count(),
                app.doc().is_dirty(),
                app.tabs[1].file.as_ref().unwrap().path().to_owned(),
            );
            let target_state = (
                app.tabs[2].doc.id(),
                app.tabs[2].doc.source().to_owned(),
                app.tabs[2].doc.selection(),
                app.tabs[2].doc.undo_count(),
                app.tabs[2].doc.redo_count(),
                app.tabs[2].doc.is_dirty(),
            );
            let disk_bytes = std::fs::read(&target_path).unwrap();
            if remove_target {
                std::fs::remove_file(&target_path).unwrap();
            }

            assert!(!app.save_as_to(&target_path));
            assert!(app.status.contains("既に開いている文書"));
            if remove_target {
                assert!(!target_path.exists());
            } else {
                assert_eq!(std::fs::read(&target_path).unwrap(), disk_bytes);
            }
            assert_eq!(
                (
                    app.doc().id(),
                    app.doc().source().to_owned(),
                    app.doc().selection(),
                    app.doc().undo_count(),
                    app.doc().redo_count(),
                    app.doc().is_dirty(),
                    app.tabs[1].file.as_ref().unwrap().path().to_owned(),
                ),
                source_state
            );
            assert_eq!(
                (
                    app.tabs[2].doc.id(),
                    app.tabs[2].doc.source().to_owned(),
                    app.tabs[2].doc.selection(),
                    app.tabs[2].doc.undo_count(),
                    app.tabs[2].doc.redo_count(),
                    app.tabs[2].doc.is_dirty(),
                ),
                target_state
            );
        }
    }

    #[test]
    fn document_identity_save_as_own_alias_uses_current_safe_save() {
        let directory = TestDirectory::new("save-as-self-alias");
        let path = directory.path().join("note.md");
        std::fs::write(&path, "original text").unwrap();

        let mut app = composing_test_app("unsaved");
        app.clear_composition();
        app.open_path(path.clone());
        app.doc_mut().set_selection(0, 8).unwrap();
        app.doc_mut().replace_selection("updated").unwrap();
        let document_id = app.doc().id();
        let undo_count = app.doc().undo_count();

        let alias = directory.path().join(".").join("note.md");
        assert!(app.save_as_to(&alias));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "updated text");
        assert_eq!(app.doc().id(), document_id);
        assert!(!app.doc().is_dirty());
        assert_eq!(app.doc().undo_count(), undo_count);
        assert_eq!(
            app.tabs[app.active].file.as_ref().unwrap().path(),
            path.canonicalize().unwrap()
        );
    }

    #[test]
    fn ui_font_is_independent_of_configurable_editor_font_and_document_state() {
        let mut app = composing_test_app("日本語 **本文** 👩‍💻\r\n");
        app.clear_composition();
        app.shell.enabled = true;
        app.shell.settings.font_face = "Cascadia Mono".into();
        let before = document_state(app.doc());
        unsafe {
            let editor = app.text_layout("本文", 300., FONT).unwrap();
            let ui = app.ui_text_layout("アウトライン", 300., 12.).unwrap();
            let mut editor_family = [0u16; 64];
            let mut ui_family = [0u16; 64];
            editor
                .GetFontFamilyName(0, &mut editor_family, None)
                .unwrap();
            ui.GetFontFamilyName(0, &mut ui_family, None).unwrap();
            let family = |name: &[u16]| {
                String::from_utf16_lossy(&name[..name.iter().position(|c| *c == 0).unwrap()])
            };
            assert_eq!(family(&editor_family), "Cascadia Mono");
            assert_eq!(family(&ui_family), shell::UI_FONT_FAMILY);
        }
        assert_eq!(document_state(app.doc()), before);
    }

    #[test]
    fn tab_labels_stay_single_line_and_bounded_without_document_changes() {
        let directory = TestDirectory::new("tab-label-layout");
        let file_name = format!("{}{}.md", "日本語".repeat(12), "a".repeat(90));
        let path = directory.path().join(&file_name);
        std::fs::write(&path, "body").unwrap();

        let mut app = composing_test_app("body");
        app.clear_composition();
        app.tabs[0].file = Some(FileSnapshot::open(&path).unwrap().snapshot);
        app.doc_mut().set_selection(4, 4).unwrap();
        app.doc_mut().replace_selection("!").unwrap();
        app.doc_mut().set_selection(2, 2).unwrap();
        let full_name = app.tab_display_name(0);
        assert_eq!(full_name, file_name);
        let document_before = document_state(app.doc());
        let labels = [
            format!("{full_name} •"),
            format!("{} •", "日本語".repeat(80)),
            format!("{}save.md •", "manual-".repeat(30)),
        ];

        for shell_enabled in [false, true] {
            app.shell.enabled = shell_enabled;
            let strip = app.tab_strip();
            let width = (strip.tab_width - 32.0).max(1.0);
            let height = if shell_enabled {
                shell::TAB_STRIP_HEIGHT
            } else {
                43.0
            };
            for label in &labels {
                let wrapped = unsafe { app.text_layout(label, width, 14.0).unwrap() };
                let mut wrapped_metrics = DWRITE_TEXT_METRICS::default();
                unsafe { wrapped.GetMetrics(&mut wrapped_metrics).unwrap() };
                assert!(
                    wrapped_metrics.lineCount > 1,
                    "fixture did not wrap: {label}"
                );

                let layout = unsafe { app.tab_label_layout(label, width, height).unwrap() };
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe { layout.GetMetrics(&mut metrics).unwrap() };
                assert_eq!(metrics.lineCount, 1, "tab label wrapped: {label}");
                assert!(metrics.widthIncludingTrailingWhitespace > width);
                assert!(metrics.height <= height);
                assert_eq!(metrics.layoutWidth, width);
            }
            assert_eq!(app.tab_display_name(0), full_name);
            assert_eq!(document_state(app.doc()), document_before);
            let after = app.tab_strip();
            assert_eq!(after.first, strip.first);
            assert_eq!(after.visible_count, strip.visible_count);
            assert_eq!(after.left, strip.left);
            assert_eq!(after.right, strip.right);
            assert_eq!(after.tab_width, strip.tab_width);
            assert_eq!(after.overflow, strip.overflow);
        }
    }

    #[test]
    fn document_identity_tab_overflow_remains_reachable_by_ctrl_tab() {
        let directory = TestDirectory::new("tab-overflow");
        let mut app = composing_test_app("unsaved");
        app.clear_composition();
        for index in 0..8 {
            let path = directory.path().join(format!("{index}.md"));
            std::fs::write(&path, format!("document {index}")).unwrap();
            app.open_path(path);
        }

        assert_eq!(app.tabs.len(), 9);
        assert!(app.tab_strip().overflow);
        let visible = app.tab_strip();
        assert!(app.active >= visible.first);
        assert!(app.active < visible.first + visible.visible_count);

        let previous_first = visible.first;
        app.scroll_tabs(false);
        assert_eq!(app.tab_strip().first, previous_first - 1);
        assert!(app.active >= app.tab_strip().first + app.tab_strip().visible_count);
        app.switch_tab(0);
        assert_eq!(app.tab_strip().first, 0);

        app.hwnd = hidden_test_window();
        unsafe {
            app.key_with_modifiers(0x09, true, false);
        }
        assert_eq!(app.active, 1);
        assert!(app.active >= app.tab_strip().first);
        assert!(app.active < app.tab_strip().first + app.tab_strip().visible_count);
        app.switch_tab(8);
        unsafe {
            app.key_with_modifiers(0x09, true, false);
        }
        assert_eq!(app.active, 0);
        assert!(app.active >= app.tab_strip().first);
        assert!(app.active < app.tab_strip().first + app.tab_strip().visible_count);
        unsafe {
            let _ = DestroyWindow(app.hwnd);
        }
    }

    struct EditorFixture {
        state: Box<WindowState>,
        hwnd: HWND,
    }

    // Real default processing sends WM_SIZE synchronously. Keep the routine
    // regression hidden; the standard system-command counterpart needs an
    // exclusive GUI lease because DefWindowProc can show a maximized window.
    fn check_default_native_resize(visible_system_commands: bool) {
        thread_local! {
            static EVENTS: RefCell<Vec<(u32, usize, bool)>> = const { RefCell::new(Vec::new()) };
        }
        unsafe extern "system" fn probe(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            unsafe {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowState;
                if let Some(state) = pointer.as_ref()
                    && matches!(
                        msg,
                        WM_SYSCOMMAND | WM_SYSCHAR | WM_SYSKEYUP | WM_WINDOWPOSCHANGED | WM_SIZE
                    )
                {
                    let available = state.app.try_borrow_mut().is_ok();
                    println!(
                        "default-boundary callback hwnd={hwnd:?} msg={msg:#x} wp={:#x} app_available={available} foreground={:?}",
                        wp.0,
                        GetForegroundWindow()
                    );
                    EVENTS.with(|events| events.borrow_mut().push((msg, wp.0, available)));
                }
                if msg == WM_SYSCOMMAND && wp.0 & 0xfff0 == SC_KEYMENU as usize {
                    // Observe the real default SYSCHAR ancestry without opening
                    // an interactive system menu in this programmatic fixture.
                    return LRESULT(0);
                }
                if msg == WM_WINDOWPOSCHANGING && lp.0 != 0 {
                    (*(lp.0 as *mut WINDOWPOS)).flags |= SWP_NOACTIVATE;
                }
                window_proc(hwnd, msg, wp, lp)
            }
        }
        struct OwnedWindows(Vec<HWND>);
        impl Drop for OwnedWindows {
            fn drop(&mut self) {
                unsafe {
                    for hwnd in self.0.iter().rev() {
                        let _ = DestroyWindow(*hwnd);
                    }
                    let mut message = MSG::default();
                    let _ = PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE);
                }
            }
        }
        let mut app = composing_test_app("before\r\n日本語\r\nafter");
        app.clear_composition();
        app.doc_mut().set_selection(3, 1).unwrap();
        let state = Box::new(WindowState::new(app));
        let mut owned = OwnedWindows(Vec::new());
        let foreground = unsafe { GetForegroundWindow() };
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(probe),
                hInstance: instance,
                lpszClassName: w!("MDLiteDefaultBoundaryRegression"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let main = {
                let _borrow = state.app.borrow_mut();
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class.lpszClassName,
                    w!("MDLite owned system-command regression"),
                    WS_OVERLAPPEDWINDOW,
                    130,
                    130,
                    450,
                    480,
                    None,
                    None,
                    Some(instance),
                    Some((&*state as *const WindowState).cast()),
                )
                .unwrap()
            };
            owned.0.push(main);
            let compact = {
                let _borrow = state.app.borrow_mut();
                CreateWindowExW(
                    WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                    class.lpszClassName,
                    w!("MDLite owned compact regression"),
                    WS_POPUP,
                    180,
                    180,
                    360,
                    400,
                    Some(main),
                    None,
                    Some(instance),
                    Some((&*state as *const WindowState).cast()),
                )
                .unwrap()
            };
            owned.0.push(compact);
            assert_eq!(state.main_window.get(), main);
            assert_eq!(state.compact_window.get(), compact);
            {
                let mut app = state.app.borrow_mut();
                app.hwnd = main;
                app.shell.main_window = main;
                app.shell.compact_window = compact;
                app.target().unwrap();
                app.select_surface(compact);
                app.target().unwrap();
                app.select_surface(main);
            }
            let before = document_state(state.app.borrow().doc());
            EVENTS.with(|events| events.borrow_mut().clear());
            println!(
                "default-boundary fixture pid={} main={main:?} compact={compact:?} foreground={foreground:?} initially_visible={}",
                std::process::id(),
                IsWindowVisible(main).as_bool()
            );
            let mut mismatches = Vec::new();
            let mut check = |hwnd: HWND, label: &str| {
                let current_foreground = GetForegroundWindow();
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client).unwrap();
                let mut app = state.app.borrow_mut();
                app.select_surface(hwnd);
                paint_into_private_dc(
                    &mut app,
                    rect(0.0, 0.0, client.right as f32, client.bottom as f32),
                )
                .unwrap();
                let target = app.target().unwrap();
                let size = target.GetPixelSize();
                let same = [size.width, size.height] == [client.right as u32, client.bottom as u32];
                println!(
                    "default-boundary {label} hwnd={:?} client={}x{} target={}x{} visible={} iconic={} app_minimized={} foreground={:?} match={same} EndDraw=Ok",
                    hwnd,
                    client.right,
                    client.bottom,
                    size.width,
                    size.height,
                    IsWindowVisible(hwnd).as_bool(),
                    IsIconic(hwnd).as_bool(),
                    app.minimized,
                    current_foreground
                );
                assert_eq!(current_foreground, foreground, "fixture changed foreground");
                if !same {
                    mismatches.push(label.to_owned());
                }
                assert_eq!(document_state(app.doc()), before);
            };
            if visible_system_commands {
                println!("default-boundary before SYSCHAR space");
                SendMessageW(
                    main,
                    WM_SYSCHAR,
                    Some(WPARAM(' ' as usize)),
                    Some(LPARAM(1 << 29)),
                );
                assert_eq!(GetForegroundWindow(), foreground);
                println!("default-boundary before SC_MAXIMIZE");
                SendMessageW(
                    main,
                    WM_SYSCOMMAND,
                    Some(WPARAM(SC_MAXIMIZE as usize)),
                    Some(LPARAM(0)),
                );
                check(main, "maximize");
                println!("default-boundary before SC_RESTORE");
                SendMessageW(
                    main,
                    WM_SYSCOMMAND,
                    Some(WPARAM(SC_RESTORE as usize)),
                    Some(LPARAM(0)),
                );
                check(main, "restore");
            }
            for (hwnd, width, height, label) in [
                (main, 520, 530, "main-position"),
                (compact, 410, 440, "compact-position"),
            ] {
                println!("default-boundary before SetWindowPos {label}");
                SetWindowPos(
                    hwnd,
                    None,
                    130,
                    130,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                )
                .unwrap();
                check(hwnd, label);
            }
            let events = EVENTS.with(|events| events.borrow().clone());
            println!("default-boundary events={events:?}");
            let sizes: Vec<_> = events.iter().filter(|event| event.0 == WM_SIZE).collect();
            assert!(
                !sizes.is_empty(),
                "real default processing must send WM_SIZE"
            );
            if visible_system_commands {
                let keymenu: Vec<_> = events
                    .iter()
                    .filter(|event| {
                        event.0 == WM_SYSCOMMAND && event.1 & 0xfff0 == SC_KEYMENU as usize
                    })
                    .collect();
                assert!(
                    !keymenu.is_empty(),
                    "real default SYSCHAR must generate SC_KEYMENU"
                );
                assert!(
                    keymenu.iter().all(|event| event.2),
                    "default menu ancestry retained App borrow"
                );
            }
            assert!(
                mismatches.is_empty(),
                "native client and D2D target differ: {mismatches:?}"
            );
            assert!(
                sizes.iter().all(|event| event.2),
                "nested WM_SIZE retained App borrow"
            );
            assert_eq!(document_state(state.app.borrow().doc()), before);
            if visible_system_commands {
                println!("default-boundary before final SC_MINIMIZE; dispose without restoring");
                SendMessageW(
                    main,
                    WM_SYSCOMMAND,
                    Some(WPARAM(SC_MINIMIZE as usize)),
                    Some(LPARAM(0)),
                );
                let iconic = IsIconic(main).as_bool();
                let app_minimized = state.app.borrow().minimized;
                let current_foreground = GetForegroundWindow();
                println!(
                    "default-boundary minimize iconic={iconic} app_minimized={app_minimized} foreground={current_foreground:?}"
                );
                assert_eq!(current_foreground, foreground);
                assert!(iconic);
                assert_eq!(iconic, app_minimized);
                assert_eq!(document_state(state.app.borrow().doc()), before);
            }
        }
    }

    #[test]
    fn editor_default_window_position_resizes_both_owned_targets() {
        check_default_native_resize(false);
    }

    #[test]
    #[ignore = "temporarily visible NOACTIVATE fixture; requires exclusive GUI lease"]
    fn editor_default_system_commands_release_app_before_native_reentry() {
        check_default_native_resize(true);
    }

    #[test]
    fn child_control_updates_skip_noops_and_follow_live_geometry_visibility_and_parent() {
        struct Counts {
            position: Cell<u32>,
            show: Cell<u32>,
        }
        unsafe extern "system" fn spy(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
            unsafe {
                if msg == WM_NCCREATE {
                    let create = &*(lp.0 as *const CREATESTRUCTW);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
                }
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Counts;
                if let Some(counts) = pointer.as_ref() {
                    if msg == WM_WINDOWPOSCHANGING {
                        counts.position.set(counts.position.get() + 1);
                    }
                    if msg == WM_SHOWWINDOW {
                        counts.show.set(counts.show.get() + 1);
                    }
                }
                if msg == WM_NCDESTROY {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        let counts = Box::new(Counts {
            position: Cell::new(0),
            show: Cell::new(0),
        });
        let parent = hidden_test_window_size(640, 480);
        let other = hidden_test_window_size(640, 480);
        struct OwnedWindows(Vec<HWND>);
        impl Drop for OwnedWindows {
            fn drop(&mut self) {
                unsafe {
                    for hwnd in &self.0 {
                        let mut process = 0;
                        GetWindowThreadProcessId(*hwnd, Some(&mut process));
                        if process == std::process::id() && IsWindow(Some(*hwnd)).as_bool() {
                            let _ = DestroyWindow(*hwnd);
                        }
                    }
                }
            }
        }
        let mut windows = OwnedWindows(vec![parent, other]);
        let foreground = unsafe { GetForegroundWindow() };
        unsafe {
            let class = WNDCLASSW {
                lpfnWndProc: Some(spy),
                hInstance: GetModuleHandleW(None).unwrap().into(),
                lpszClassName: w!("MDLiteChildControlRegression"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let mut make = || {
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class.lpszClassName,
                    w!("child"),
                    WS_CHILD | WS_VISIBLE,
                    0,
                    0,
                    10,
                    10,
                    Some(parent),
                    None,
                    Some(class.hInstance),
                    Some((&*counts as *const Counts).cast()),
                )
                .unwrap();
                windows.0.push(hwnd);
                hwnd
            };
            let child = make();
            shell::position_child_control(child, 17, 23, 123, 29).unwrap();
            assert!(counts.position.get() > 0);
            counts.position.set(0);
            shell::position_child_control(child, 17, 23, 123, 29).unwrap();
            assert_eq!(
                counts.position.get(),
                0,
                "unchanged placement must not generate native position callbacks"
            );
            assert!(
                !IsWindowVisible(child).as_bool(),
                "hidden parent hides effective visibility"
            );
            assert_ne!(GetWindowLongPtrW(child, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let _ = shell::show_child_control(child, true);
            assert_eq!(
                counts.position.get(),
                0,
                "already locally shown child must not reposition"
            );
            let _ = shell::show_child_control(child, false);
            assert_eq!(GetWindowLongPtrW(child, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let changed_show = counts.show.get();
            let _ = shell::show_child_control(child, false);
            assert_eq!(counts.show.get(), changed_show);
            let _ = shell::show_child_control(child, true);
            shell::position_child_control(child, 24, 31, 134, 33).unwrap();
            let mut actual = RECT::default();
            GetWindowRect(child, &mut actual).unwrap();
            assert_eq!(
                [
                    actual.left,
                    actual.top,
                    actual.right - actual.left,
                    actual.bottom - actual.top
                ],
                [24, 31, 134, 33],
                "zero-offset parent mapping is valid"
            );
            SetWindowPos(
                other,
                None,
                300,
                200,
                640,
                480,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            SetParent(child, Some(other)).unwrap();
            shell::position_child_control(child, 17, 23, 123, 29).unwrap();
            GetWindowRect(child, &mut actual).unwrap();
            assert_eq!(
                [
                    actual.left,
                    actual.top,
                    actual.right - actual.left,
                    actual.bottom - actual.top
                ],
                [317, 223, 123, 29]
            );
            SetWindowLongPtrW(
                other,
                GWL_EXSTYLE,
                GetWindowLongPtrW(other, GWL_EXSTYLE) | WS_EX_LAYOUTRTL.0 as isize,
            );
            SetWindowPos(
                other,
                None,
                300,
                200,
                640,
                480,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            )
            .unwrap();
            shell::position_child_control(child, 17, 23, 123, 29).unwrap();
            counts.position.set(0);
            shell::position_child_control(child, 17, 23, 123, 29).unwrap();
            assert_eq!(
                counts.position.get(),
                0,
                "mirrored parent RECT mapping must find the live no-op"
            );
            DestroyWindow(child).unwrap();
            let replacement = make();
            shell::position_child_control(replacement, 17, 23, 123, 29).unwrap();
            GetWindowRect(replacement, &mut actual).unwrap();
            assert_eq!(
                [
                    actual.left,
                    actual.top,
                    actual.right - actual.left,
                    actual.bottom - actual.top
                ],
                [17, 23, 123, 29]
            );
            DestroyWindow(replacement).unwrap();
            DestroyWindow(other).unwrap();
            DestroyWindow(parent).unwrap();
            assert_eq!(GetForegroundWindow(), foreground);
        }
    }
    #[test]
    fn revealed_source_styles_keep_utf16_ranges_after_unicode_prefix_and_clipping() {
        let source = "prefix😀\n\n前置 e\u{301} **太字😀** と ***両方😀***、~~取消~~、[リンク](u) と `code` 末尾";
        let fixture = EditorFixture::new(source, false);
        let app = fixture.state.app.borrow();
        let projection = DisplayProjection::parse(source);
        // A nonzero source origin and a clip inside bold text exercise metric
        // subtraction independently of display/projection coordinates.
        for start in [source.find("前置").unwrap(), source.find("字😀").unwrap()] {
            let layout = unsafe {
                app.text_layout(&source[start..], 900., app.font_size())
                    .unwrap()
            };
            unsafe {
                app.apply_source_styles(&layout, &projection, start..source.len())
                    .unwrap()
            };
            for (needle, bold, italic) in [
                ("字😀", true, false),
                ("両方😀", true, true),
                ("末尾", false, false),
            ] {
                let byte = source.find(needle).unwrap();
                let position = source[start..byte].encode_utf16().count() as u32;
                let mut weight = DWRITE_FONT_WEIGHT_NORMAL;
                let mut style = DWRITE_FONT_STYLE_NORMAL;
                unsafe {
                    layout.GetFontWeight(position, &mut weight, None).unwrap();
                    layout.GetFontStyle(position, &mut style, None).unwrap();
                }
                assert_eq!(
                    weight == DWRITE_FONT_WEIGHT_BOLD,
                    bold,
                    "{needle} at {start}"
                );
                assert_eq!(
                    style == DWRITE_FONT_STYLE_ITALIC,
                    italic,
                    "{needle} at {start}"
                );
            }
            for (needle, strike) in [("リンク", false), ("取消", true)] {
                let position = source[start..source.find(needle).unwrap()]
                    .encode_utf16()
                    .count() as u32;
                let mut applied = windows::core::BOOL::default();
                unsafe {
                    if strike {
                        layout
                            .GetStrikethrough(position, &mut applied, None)
                            .unwrap();
                    } else {
                        layout.GetUnderline(position, &mut applied, None).unwrap();
                    }
                }
                assert!(applied.as_bool(), "{needle} at {start}");
            }
        }
        assert_eq!(app.doc().source(), source);
        assert!(!app.doc().is_dirty());
    }

    impl EditorFixture {
        fn new(source: &str, raw: bool) -> Self {
            let mut app = composing_test_app(source);
            app.clear_composition();
            app.tabs[0].raw = raw;
            app.tabs[0].scroll = 0.0;
            app.hwnd = hidden_test_window_size(450, 480);
            unsafe {
                app.ensure_layout().unwrap();
            }
            let hwnd = app.hwnd;
            let fixture = Self {
                state: Box::new(WindowState::new(app)),
                hwnd,
            };
            unsafe {
                SetWindowLongPtrW(
                    hwnd,
                    GWLP_USERDATA,
                    (&*fixture.state as *const WindowState) as isize,
                );
            }
            fixture
        }
        fn message(&self, msg: u32, value: usize) {
            unsafe {
                window_proc(self.hwnd, msg, WPARAM(value), LPARAM(0));
            }
        }
    }
    impl Drop for EditorFixture {
        fn drop(&mut self) {
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    struct PaintBitmap {
        dc: HDC,
        bitmap: HBITMAP,
        old: HGDIOBJ,
        client: RECT,
    }
    impl PaintBitmap {
        unsafe fn new(hwnd: HWND) -> windows::core::Result<Self> {
            unsafe {
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client)?;
                let source = GetDC(Some(hwnd));
                let dc = CreateCompatibleDC(Some(source));
                let bitmap =
                    CreateCompatibleBitmap(source, client.right.max(1), client.bottom.max(1));
                ReleaseDC(Some(hwnd), source);
                if dc == HDC::default() || bitmap == HBITMAP::default() {
                    if bitmap != HBITMAP::default() {
                        let _ = DeleteObject(bitmap.into());
                    }
                    if dc != HDC::default() {
                        let _ = DeleteDC(dc);
                    }
                    return Err(E_FAIL.into());
                }
                let old = SelectObject(dc, bitmap.into());
                if old == HGDIOBJ::default() || old.0 as isize == -1 {
                    let _ = DeleteObject(bitmap.into());
                    let _ = DeleteDC(dc);
                    return Err(E_FAIL.into());
                }
                Ok(Self {
                    dc,
                    bitmap,
                    old,
                    client,
                })
            }
        }
    }
    impl Drop for PaintBitmap {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.dc, self.old);
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }
    pub(super) unsafe fn paint_into_private_dc(
        app: &mut App,
        damage: D2D_RECT_F,
    ) -> windows::core::Result<()> {
        unsafe {
            let surface = PaintBitmap::new(app.hwnd)?;
            app.prepare_paint()?;
            let result = app.paint_prepared_to_dc(surface.dc, damage);
            drop(surface);
            if result.is_ok() {
                app.after_paint_release();
            }
            result
        }
    }

    pub(super) struct MessagePumpFixture {
        pub(super) state: Box<WindowState>,
        pub(super) hwnd: HWND,
        instance: HINSTANCE,
        class_name: Vec<u16>,
        compact: Option<HWND>,
    }
    impl MessagePumpFixture {
        pub(super) fn new(app: App) -> Self {
            static NEXT_CLASS_ID: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(1);
            let instance = HINSTANCE(unsafe { GetModuleHandleW(None).unwrap().0 });
            let class_name: Vec<u16> = format!(
                "MDLiteGitPromptMessagePumpTest-{}-{}",
                std::process::id(),
                NEXT_CLASS_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
            let class_name_pcwstr = PCWSTR(class_name.as_ptr());
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class_name_pcwstr,
                ..Default::default()
            };
            unsafe { assert_ne!(RegisterClassW(&class), 0) };
            let state = Box::new(WindowState::new(app));
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class_name_pcwstr,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    900,
                    700,
                    None,
                    None,
                    Some(instance),
                    Some((&*state as *const WindowState).cast()),
                )
                .unwrap()
            };
            state.app.borrow_mut().hwnd = hwnd;
            Self {
                state,
                hwnd,
                instance,
                class_name,
                compact: None,
            }
        }
        pub(super) fn create_compact(&mut self) -> HWND {
            let class_name = PCWSTR(self.class_name.as_ptr());
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class_name,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    640,
                    480,
                    Some(self.hwnd),
                    None,
                    Some(self.instance),
                    Some((&*self.state as *const WindowState).cast()),
                )
                .unwrap()
            };
            self.compact = Some(hwnd);
            hwnd
        }
    }
    impl Drop for MessagePumpFixture {
        fn drop(&mut self) {
            unsafe {
                if let Some(compact) = self.compact.take() {
                    let _ = DestroyWindow(compact);
                }
                let _ = DestroyWindow(self.hwnd);
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, WM_QUIT, WM_QUIT, PM_REMOVE);
                let _ = UnregisterClassW(PCWSTR(self.class_name.as_ptr()), Some(self.instance));
            }
        }
    }

    #[test]
    fn e08_table_end_boundary_preserves_structural_edit_and_undo() {
        for eol in ["\n", "\r\n"] {
            let source = format!(
                "| H | J |{eol}| --- | --- |{eol}| first | one |{eol}{eol}| K | L |{eol}| --- | --- |"
            );
            let fixture = EditorFixture::new(&source, false);
            let p;
            let gap;
            let next_table;
            {
                let mut app = fixture.state.app.borrow_mut();
                assert_eq!(app.tables.len(), 2, "expected two tables ({eol:?})");
                p = app.tables[0].table.range.end;
                next_table = app.tables[1].table.range.start;
                gap = table_gap_marker(&source, p, next_table).unwrap();
                app.doc_mut().set_selection(p, p).unwrap();
                unsafe {
                    app.after_input();
                }
                assert!(
                    app.views.iter().any(|view| view.source == (gap..gap)),
                    "table-gap marker {gap} disappeared ({eol:?})"
                );
                assert!(
                    app.views
                        .iter()
                        .any(|view| view.source == (next_table..next_table)),
                    "next-table marker {next_table} disappeared ({eol:?})"
                );
                assert!(
                    app.views.iter().any(|view| view.source == (p..p)),
                    "table-end marker {p} is missing ({eol:?})"
                );
                assert!(
                    !app.views.iter().any(|view| {
                        view.table.is_none() && !view.source.is_empty() && view.map.contains(&p)
                    }),
                    "previous raw-row fallback still covers table end {p} ({eol:?})"
                );
                assert_eq!(app.caret_view().unwrap().source, p..p);
            }

            fixture.message(WM_CHAR, 'x' as usize);
            let mut expected = source[..p].to_owned();
            expected.push_str(&eol.repeat(2));
            expected.push('x');
            expected.push_str(&source[p..]);
            let mut app = fixture.state.app.borrow_mut();
            assert_eq!(app.doc().source(), expected, "edited source ({eol:?})");
            assert_eq!(
                app.doc().selection(),
                Selection::caret(p + eol.len() * 2 + 1),
                "edited caret ({eol:?})"
            );
            assert_eq!(app.tables.len(), 2, "both tables must survive ({eol:?})");
            assert_eq!(app.doc().undo_count(), 1, "one edit ({eol:?})");
            unsafe {
                app.key_with_modifiers(0x5a, true, false);
            }
            assert_eq!(
                app.doc().source(),
                source,
                "undo restores exact source ({eol:?})"
            );
            assert_eq!(app.doc().selection(), Selection::caret(p));
            assert_eq!(app.tables.len(), 2, "both tables survive undo ({eol:?})");
            assert_eq!(
                app.doc().undo_count(),
                0,
                "undo consumes the edit ({eol:?})"
            );
            assert!(!app.doc().is_dirty(), "undo restores clean state ({eol:?})");
        }
    }

    #[test]
    fn review_table_boundary_enter_typing_undo_source_and_geometry() {
        for eol in ["\n", "\r\n"] {
            let table = format!("| H |{eol}|---|{eol}| cell |{eol}");
            let sources = [
                table.clone(),
                format!("before{eol}{eol}{table}after"),
                format!("{table}{eol}{table}after"),
            ];
            for (index, source) in sources.iter().enumerate() {
                let fixture = EditorFixture::new(source, false);
                let p;
                {
                    let mut app = fixture.state.app.borrow_mut();
                    p = app.tables[if index == 2 { 1 } else { 0 }].table.range.start;
                    if index == 0 {
                        let eof = app.doc().byte_len();
                        app.doc_mut().set_selection(eof, eof).unwrap();
                        unsafe {
                            app.key_with_modifiers(VK_HOME.0, true, false);
                        }
                        assert_eq!(app.doc().selection(), Selection::caret(0));
                    } else {
                        app.doc_mut().set_selection(p, p).unwrap();
                    }
                    unsafe {
                        app.after_input();
                    }
                }
                fixture.message(WM_KEYDOWN, VK_RETURN.0 as usize);
                let after_enter = format!("{}{eol}{}", &source[..p], &source[p..]);
                let q = p + eol.len();
                {
                    let app = fixture.state.app.borrow();
                    assert_eq!(app.doc().source(), after_enter);
                    assert_eq!(app.doc().selection(), Selection::caret(q));
                    assert!(
                        app.views.iter().any(|view| view.source == (q..q)),
                        "shifted table start {q} has no exact source-affine marker ({eol:?}, fixture {index})"
                    );
                    assert_eq!(app.caret_view().unwrap().source, q..q);
                    let caret_y = app.caret_location().1;
                    let table_y = app.tables[if index == 2 { 1 } else { 0 }].y;
                    assert!(
                        caret_y < table_y,
                        "inserted blank source row must provide a caret baseline before the table ({eol:?}, fixture {index}, caret y={caret_y}, table y={table_y})"
                    );
                }
                fixture.message(WM_CHAR, '前' as usize);
                let app = &mut *fixture.state.app.borrow_mut();
                assert_eq!(
                    app.doc().source(),
                    format!("{}前{eol}{eol}{}", &after_enter[..q], &after_enter[q..])
                );
                assert_eq!(app.doc().selection(), Selection::caret(q + "前".len()));
                unsafe {
                    app.key_with_modifiers(0x5a, true, false);
                }
                assert_eq!(app.doc().source(), after_enter);
                assert_eq!(app.doc().selection(), Selection::caret(q));
                assert_eq!(app.caret_view().unwrap().source, q..q);
                unsafe {
                    app.key_with_modifiers(0x5a, true, false);
                }
                assert_eq!(app.doc().source(), *source);
                assert_eq!(app.doc().selection(), Selection::caret(p));
                assert_eq!(app.caret_view().unwrap().source, p..p);
                assert!(!app.doc().is_dirty());
            }
        }
    }
    #[test]
    fn review_leading_table_home_is_visible_and_accepts_before_table_typing() {
        for source in [
            "| H |\n|---|\n| cell |\n",
            "| left | right |\n| --- | --- |\n| cell | value |\n\n```rust\nlet source = \"Markdown\";\n## asdkl\n```\n\n",
            "   | H |\n   |---|\n   | cell |\n",
        ] {
            let fixture = EditorFixture::new(source, false);
            let mut app = fixture.state.app.borrow_mut();
            let original_table = app.doc().source()[app.tables[0].table.range.clone()].to_owned();
            unsafe {
                app.key_with_modifiers(VK_HOME.0, true, false);
            }
            assert_eq!(app.doc().selection(), Selection::caret(0));
            assert!(
                app.source_view(0).is_some(),
                "document start needs actual source geometry"
            );
            unsafe {
                app.character('前' as u16);
            }
            assert_eq!(app.doc().source(), format!("前\n\n{source}"));
            assert_eq!(app.doc().selection(), Selection::caret("前".len()));
            assert_eq!(app.tables.len(), 1);
            assert_eq!(
                &app.doc().source()[app.tables[0].table.range.clone()],
                original_table
            );
            assert_eq!(app.caret_view().unwrap().source.start, 0);
            assert!(app.caret_location().1 < app.tables[0].y);
            unsafe {
                app.key_with_modifiers(0x5a, true, false);
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(app.doc().selection(), Selection::caret(0));
            assert!(!app.doc().is_dirty());
            assert_eq!(app.caret_view().unwrap().source, 0..0);
        }
    }
    #[test]
    fn review_live_control_word_crosses_fences_with_progress_and_selection() {
        let mut projected = composing_test_app("e*́* next");
        projected.visible_tokens = projected_tokens(projected.doc().source());
        assert_eq!(
            projected.live_word_point(0, 1, false),
            "e*́".len(),
            "word motion must use whole visible grapheme endpoints across hidden style spans"
        );
        let fixture = EditorFixture::new("```\nx\n```\n", false);
        let mut app = fixture.state.app.borrow_mut();
        unsafe {
            app.key_with_modifiers(VK_RIGHT.0, true, true);
        }
        assert!(
            app.doc().selection().head > 0,
            "CtrlRight must not snap back to zero"
        );
        assert_eq!(app.doc().selection().anchor, 0);
        let mut previous = 0;
        while app.doc().selection().head < app.doc().byte_len() {
            let p = app.doc().selection().head;
            assert!(p > previous, "CtrlRight stalled at {p}");
            previous = p;
            unsafe {
                app.key_with_modifiers(VK_RIGHT.0, true, true);
            }
        }
        while app.doc().selection().head > 0 {
            let p = app.doc().selection().head;
            unsafe {
                app.key_with_modifiers(VK_LEFT.0, true, true);
            }
            assert!(app.doc().selection().head < p, "CtrlLeft stalled at {p}");
        }
        assert_eq!(app.doc().source(), "```\nx\n```\n");
        assert_eq!(app.doc().undo_count(), 0);
        drop(app);
        for source in [
            "```\n日本語 e\u{301} 👩‍💻\n```\n",
            "```\r\nx\r\n```\r\n",
            "**日本語** next",
        ] {
            let fixture = EditorFixture::new(source, false);
            let mut app = fixture.state.app.borrow_mut();
            let boundaries: Vec<_> = source
                .grapheme_indices(true)
                .map(|(p, _)| p)
                .chain(Some(source.len()))
                .collect();
            let mut steps = 0;
            while app.doc().selection().head < source.len() {
                let previous = app.doc().selection().head;
                unsafe {
                    app.key_with_modifiers(VK_RIGHT.0, true, false);
                }
                assert!(
                    app.doc().selection().head > previous,
                    "forward stall {source} at {previous}"
                );
                assert!(boundaries.contains(&app.doc().selection().head));
                steps += 1;
                assert!(steps <= boundaries.len());
            }
            while app.doc().selection().head > 0 {
                let previous = app.doc().selection().head;
                unsafe {
                    app.key_with_modifiers(VK_LEFT.0, true, false);
                }
                assert!(
                    app.doc().selection().head < previous,
                    "backward stall {source} at {previous}"
                );
                assert!(boundaries.contains(&app.doc().selection().head));
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(app.doc().undo_count(), 0);
        }
    }
    #[test]
    fn review_actual_toolbar_click_posts_close_command_during_composition() {
        let fixture = EditorFixture::new("unsaved", false);
        unsafe {
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                900,
                480,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            app.layout_key = None;
            unsafe {
                app.ensure_layout().unwrap();
            }
            app.start_composition();
            app.preedit = "かな".into();
        }
        let x = LEFT
            + TOOLBAR
                .iter()
                .take(5)
                .map(|(_, _, width)| width)
                .sum::<f32>()
            + 10.0;
        unsafe {
            window_proc(
                fixture.hwnd,
                WM_LBUTTONDOWN,
                WPARAM(0),
                LPARAM((x as isize) | (12 << 16)),
            );
        }
        let mut message = MSG::default();
        unsafe {
            assert!(
                PeekMessageW(
                    &mut message,
                    Some(fixture.hwnd),
                    WM_COMMAND,
                    WM_COMMAND,
                    PM_REMOVE
                )
                .as_bool(),
                "toolbar close must use the shared command path"
            );
        }
        assert_eq!(message.wParam.0, CMD_CLOSE);
        struct DetachedContext(HWND, HIMC);
        impl Drop for DetachedContext {
            fn drop(&mut self) {
                unsafe {
                    ImmAssociateContext(self.0, self.1);
                }
            }
        }
        let _context = DetachedContext(fixture.hwnd, unsafe {
            ImmAssociateContext(fixture.hwnd, HIMC::default())
        });
        unsafe {
            window_proc(
                fixture.hwnd,
                message.message,
                message.wParam,
                message.lParam,
            );
        }
        let app = fixture.state.app.borrow();
        assert!(app.composing);
        assert_eq!(app.preedit, "かな");
        assert_eq!(app.doc().source(), "unsaved");
        assert!(
            app.status.contains("IMEの確定を完了できませんでした"),
            "queued toolbar command must reach completion gate"
        );
        assert!(app.close_after_composition.is_none());
        assert_eq!(app.doc().undo_count(), 0);
    }
    #[test]
    fn review_toolbar_pending_close_resumes_only_after_result_and_real_end() {
        let fixture = EditorFixture::new("unsaved", false);
        {
            let mut app = fixture.state.app.borrow_mut();
            app.start_composition();
            app.preedit = "かな".into();
            // Isolate the already-completed IMM request state; no real IME or
            // dialog is driven by this synthetic owned lifecycle test.
            app.close_after_composition = Some((WM_COMMAND, WPARAM(CMD_CLOSE), LPARAM(0)));
            fixture.state.notifications.borrow_mut().push_back((
                app.ime_session.unwrap(),
                NativeNotification::Update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("確定".into()),
                    ..Default::default()
                }),
            ));
            fixture.message(WM_IME_ENDCOMPOSITION, 0);
            assert!(app.composing);
            assert_eq!(app.doc().source(), "unsaved");
        }
        fixture.message(WM_NULL, 0);
        let mut message = MSG::default();
        unsafe {
            assert!(
                PeekMessageW(
                    &mut message,
                    Some(fixture.hwnd),
                    WM_COMMAND,
                    WM_COMMAND,
                    PM_REMOVE
                )
                .as_bool()
            );
        }
        assert_eq!(message.wParam.0, CMD_CLOSE);
        let app = fixture.state.app.borrow();
        assert_eq!(app.doc().source(), "確定unsaved");
        assert!(app.doc().is_dirty());
        assert_eq!(app.doc().undo_count(), 1);
        assert!(!app.composing);
        assert!(app.close_after_composition.is_none());
        // The posted normal CMD_CLOSE is intentionally not dispatched to the
        // unsaved dialog; visible Save/No/Cancel acceptance remains Root-owned.
        assert!(unsafe { IsWindow(Some(fixture.hwnd)).as_bool() });
    }
    #[test]
    fn startup_initializer_creates_default_document_before_window_state() {
        let state = startup_window_state(startup_test_graphics(), None, Vec::new());
        assert_eq!(state.document_id.get(), 1);
        assert!(state.notifications.borrow().is_empty());
        let app = state.app.borrow();
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active, 0);
        assert_eq!(app.doc().id(), 1);
        assert!(app.doc().source().is_empty());
        assert!(app.doc().is_dirty());
        assert_eq!(app.doc().undo_count(), 0);
        assert_eq!(app.doc().redo_count(), 0);
        assert_eq!(app.next_id, 2);
    }

    #[test]
    fn startup_initializer_opens_multiple_synthetic_files_before_window_state() {
        let first = startup_fixture_path(1);
        let second = startup_fixture_path(2);
        std::fs::write(&first, "# first\n").unwrap();
        std::fs::write(&second, "## second\n").unwrap();

        let state = startup_window_state(
            startup_test_graphics(),
            None,
            vec![first.clone(), second.clone()],
        );
        assert_eq!(state.document_id.get(), 2);
        assert!(state.notifications.borrow().is_empty());
        {
            let app = state.app.borrow();
            assert_eq!(app.tabs.len(), 2);
            assert_eq!(app.active, 1);
            assert_eq!(app.tabs[0].doc.id(), 1);
            assert_eq!(app.tabs[0].doc.source(), "# first\n");
            assert!(!app.tabs[0].doc.is_dirty());
            assert_eq!(app.tabs[0].doc.undo_count(), 0);
            assert_eq!(app.tabs[1].doc.id(), 2);
            assert_eq!(app.tabs[1].doc.source(), "## second\n");
            assert!(!app.tabs[1].doc.is_dirty());
            assert_eq!(app.tabs[1].doc.undo_count(), 0);
            assert_eq!(app.next_id, 3);
        }
        drop(state);
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn editor_reentrant_end_is_queued_then_input_and_commands_resume() {
        let fixture = EditorFixture::new("body", false);
        {
            let mut app = fixture.state.app.borrow_mut();
            app.start_composition();
            app.preedit = "かな".into();
            fixture.message(WM_IME_ENDCOMPOSITION, 0);
            assert!(app.composing);
            assert_eq!(fixture.state.notifications.borrow().len(), 1);
        }
        fixture.message(WM_NULL, 0);
        assert!(!fixture.state.app.borrow().composing);
        fixture.message(WM_KEYDOWN, VK_END.0 as usize);
        fixture.message(WM_KEYDOWN, VK_RETURN.0 as usize);
        fixture.message(WM_CHAR, '次' as usize);
        assert_eq!(fixture.state.app.borrow().doc().source(), "body\n次");
        fixture.message(WM_COMMAND, CMD_RAW);
        assert!(fixture.state.app.borrow().tabs[0].raw);
    }
    #[test]
    fn editor_queued_ime_end_precedes_first_character_and_enter() {
        for enter in [false, true] {
            let fixture = EditorFixture::new("body", false);
            {
                let mut app = fixture.state.app.borrow_mut();
                app.doc_mut().set_selection(4, 4).unwrap();
                app.start_composition();
                app.preedit = "かな".into();
                fixture.message(WM_IME_ENDCOMPOSITION, 0);
                assert_eq!(fixture.state.notifications.borrow().len(), 1);
            }
            // The first real input must observe the already-delivered End;
            // no synthetic WM_NULL/paint/timer may clear it first.
            fixture.message(
                if enter { WM_KEYDOWN } else { WM_CHAR },
                if enter {
                    VK_RETURN.0 as usize
                } else {
                    'x' as usize
                },
            );
            let app = fixture.state.app.borrow();
            assert_eq!(app.doc().source(), if enter { "body\n" } else { "bodyx" });
            assert_eq!(app.doc().selection(), Selection::caret(5));
            assert_eq!(app.doc().undo_count(), 1);
            assert!(!app.composing);
            assert!(fixture.state.notifications.borrow().is_empty());
        }
    }
    #[test]
    fn editor_wrapped_full_fence_source_reserves_measured_height() {
        let opening = format!("```{}", "long-info ".repeat(24));
        let source = format!("{opening}\nx\n```\n\nafter");
        let mut app = composing_test_app(&source);
        app.clear_composition();
        app.doc_mut().set_selection(1, 1).unwrap();
        app.tabs[0].scroll = 0.0;
        app.hwnd = hidden_test_window_size(220, 480);
        let before = document_state(app.doc());
        unsafe {
            app.ensure_layout().unwrap();
        }
        let fence = app
            .views
            .iter()
            .find(|view| {
                view.source.start == 0
                    && view
                        .text
                        .starts_with(&opening.encode_utf16().collect::<Vec<_>>())
            })
            .expect("the full fenced source must remain directly editable");
        let displayed = String::from_utf16(&fence.text).unwrap();
        assert_eq!(displayed, opening, "only the active delimiter line is raw");
        assert_eq!(
            source[fence.source.clone()].trim_end_matches(['\r', '\n']),
            displayed
        );
        assert_eq!(fence.map.first(), Some(&0));
        // Interior UTF-16 boundaries map to the literal source; the terminal
        // projection boundary may also own the following blank-line anchor.
        for (index, byte) in fence.map.iter().take(fence.text.len()).enumerate() {
            assert_eq!(*byte, index);
        }
        assert!(fence.height > app.font_size() * 1.6 * 2.0);
        let following = app
            .views
            .iter()
            .filter(|view| !view.source.is_empty() && view.source.start > fence.source.start)
            .min_by(|a, b| a.y.total_cmp(&b.y))
            .unwrap();
        assert!(
            following.y >= fence.y + fence.height - 0.01,
            "fence bottom {} overlaps following view at {}",
            fence.y + fence.height,
            following.y
        );
        assert_eq!(document_state(app.doc()), before);
        unsafe {
            DestroyWindow(app.hwnd).unwrap();
        }
    }

    #[test]
    #[ignore = "visible focus fixture; requires exclusive GUI lease"]
    fn explicit_navigation_toolbar_returns_editor_focus_before_first_followup_character() {
        let source_a = "source A first\n日本語\n";
        let source_b = "source B 日本語 and ASCII\n";
        let mut app = composing_test_app(source_a);
        app.clear_composition();
        let mut fixture = MessagePumpFixture::new(app);
        let main = fixture.hwnd;
        let (id_a, id_b, before_a, before_b) = unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.start_shell();
            app.doc_mut().set_selection(0, 0).unwrap();
            app.record_navigation();
            app.doc_mut().set_selection(14, 14).unwrap();
            let id_a = app.doc().id();
            let before_a = document_state(app.doc());
            app.new_document();
            let id_b = app.doc().id();
            let active = app.active;
            app.tabs[active].doc = Document::new(id_b, source_b.into());
            app.doc_mut()
                .set_selection(source_b.len(), source_b.len())
                .unwrap();
            let before_b = document_state(app.doc());
            app.refresh();
            let _ = ShowWindow(main, SW_SHOWNOACTIVATE);
            (id_a, id_b, before_a, before_b)
        };
        unsafe {
            SetFocus(Some(main)).unwrap();
            let back = GetDlgItem(Some(main), 168).unwrap();
            let forward = GetDlgItem(Some(main), 169).unwrap();
            for (button, expected_document, expected_position) in
                [(back, id_a, 14), (forward, id_b, source_b.len())]
            {
                assert!(IsWindowEnabled(button).as_bool());
                SendMessageW(
                    button,
                    WM_LBUTTONDOWN,
                    Some(WPARAM(1)),
                    Some(LPARAM(5 | (5 << 16))),
                );
                assert_eq!(
                    GetFocus(),
                    button,
                    "native mouse-down must focus the real navigation button"
                );
                SendMessageW(
                    button,
                    WM_LBUTTONUP,
                    Some(WPARAM(0)),
                    Some(LPARAM(5 | (5 << 16))),
                );
                let app = fixture.state.app.borrow();
                assert_eq!(app.doc().id(), expected_document);
                assert_eq!(app.doc().selection(), Selection::caret(expected_position));
                println!(
                    "navigation pid={} button={button:?} enabled={} focus={:?} editor={main:?}",
                    std::process::id(),
                    IsWindowEnabled(button).as_bool(),
                    GetFocus()
                );
            }
            assert!(
                !IsWindowEnabled(forward).as_bool(),
                "Forward must disable itself after consuming the last forward entry"
            );
            assert_eq!(
                GetFocus(),
                main,
                "successful Forward must retain a keyboard target after its button disables"
            );
            {
                let app = fixture.state.app.borrow();
                assert_eq!(
                    document_state(
                        &app.tabs
                            .iter()
                            .find(|tab| tab.doc.id() == id_a)
                            .unwrap()
                            .doc
                    ),
                    before_a
                );
                assert_eq!(document_state(app.doc()), before_b);
            }
            SendMessageW(
                GetFocus(),
                WM_CHAR,
                Some(WPARAM('X' as usize)),
                Some(LPARAM(0)),
            );
            {
                let mut app = fixture.state.app.borrow_mut();
                assert_eq!(app.doc().source(), format!("{source_b}X"));
                app.key_with_modifiers(0x5a, true, false);
                assert_eq!(app.doc().source(), source_b);
                assert_eq!(app.doc().selection(), before_b.6);
                assert_eq!(app.doc().is_dirty(), before_b.3);
                assert_eq!(app.doc().undo_count(), before_b.4);
                assert_eq!(app.doc().redo_count(), before_b.5 + 1);
            }

            let other = shell::control(
                main,
                w!("BUTTON"),
                "owned no-op focus",
                999,
                WINDOW_STYLE(0),
            );
            SetFocus(Some(other)).unwrap();
            {
                let mut app = fixture.state.app.borrow_mut();
                let restored = document_state(app.doc());
                app.command(169); // Empty forward stack is a no-op.
                assert_eq!(GetFocus(), other);
                app.tabs.retain(|tab| tab.doc.id() != id_a);
                app.active = 0;
                app.command(168); // Only closed-document targets remain.
                assert_eq!(GetFocus(), other);
                assert_eq!(document_state(app.doc()), restored);
                app.doc_mut().set_selection(0, 0).unwrap();
                app.record_navigation();
                app.doc_mut()
                    .set_selection(source_b.len(), source_b.len())
                    .unwrap();
            }
            let compact = fixture.create_compact();
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.compact_window = compact;
                app.select_surface(compact);
                let _ = ShowWindow(compact, SW_SHOWNOACTIVATE);
                let child = shell::control(
                    compact,
                    w!("BUTTON"),
                    "owned Compact focus",
                    999,
                    WINDOW_STYLE(0),
                );
                SetFocus(Some(child)).unwrap();
                app.command(168);
                assert_eq!(GetFocus(), compact);
                assert_eq!(app.editing_window(), compact);
                assert_eq!(app.doc().selection(), Selection::caret(0));
                assert_eq!(app.doc().source(), source_b);
            }
            let _ = ShowWindow(compact, SW_HIDE);
            let _ = ShowWindow(main, SW_HIDE);
        }
    }
    #[test]
    #[ignore = "visible focus fixture; requires exclusive GUI lease"]
    fn native_caret_ownership_preserves_focused_query_through_palette_and_diagnostic() {
        let fixture = EditorFixture::new("selected body", false);
        fixture.state.main_window.set(fixture.hwnd);
        let directory = TestDirectory::new("native-caret-ownership");
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.enabled = true;
            app.shell.main_window = fixture.hwnd;
            app.start_shell();
            let settings = app.shell.settings.clone();
            app.apply_shell_settings(settings);
            let _ = ShowWindow(fixture.hwnd, SW_SHOWNOACTIVATE);
            let query = GetDlgItem(Some(fixture.hwnd), 150).unwrap();
            app.diagnostic = Some(directory.path().join("diagnostic.json"));
            SetFocus(Some(fixture.hwnd)).unwrap();
            app.refresh();
            assert_eq!(native_caret_owner(), Some(fixture.hwnd));
            assert!(app.native_caret.size.is_some());
            let cached_editor_caret = app.native_caret;
            let before = document_state(app.doc());

            SetFocus(Some(query)).unwrap();
            assert_eq!(GetFocus(), query);
            assert_eq!(
                native_caret_owner(),
                Some(query),
                "native EDIT must first create its real caret"
            );
            // Click ingress already focuses the toolbar query before opening
            // candidates. Opening and deferred editor focus loss may not destroy
            // that native caret or depend on a second focus transition.
            app.open_palette();
            assert_eq!(GetFocus(), query);
            assert_eq!(
                native_caret_owner(),
                Some(query),
                "open_palette destroyed the focused EDIT caret"
            );
            app.native_notification(NativeNotification::FocusLost);
            assert_eq!(native_caret_owner(), Some(query));
            app.refresh();
            app.diagnostic();
            assert_eq!(GetFocus(), query);
            assert_eq!(native_caret_owner(), Some(query));
            println!(
                "native query ownership pid={} editor={:?} query={query:?} focus={:?} caret={:?}",
                std::process::id(),
                fixture.hwnd,
                GetFocus(),
                native_caret_owner()
            );
            assert!(directory.path().join("diagnostic.json").exists());
            assert_eq!(document_state(app.doc()), before);

            // Simulate stale cached geometry after native child ownership. The
            // editor must recreate its own caret even at unchanged size/position.
            app.native_caret = cached_editor_caret;
            SetFocus(Some(fixture.hwnd)).unwrap();
            app.update_caret();
            assert_eq!(native_caret_owner(), Some(fixture.hwnd));
            assert!(app.native_caret.size.is_some());
            app.release_caret();
            assert_eq!(native_caret_owner(), Some(HWND::default()));
            app.update_caret();
            assert_eq!(native_caret_owner(), Some(fixture.hwnd));
            assert_eq!(document_state(app.doc()), before);
            app.release_caret();
            let _ = ShowWindow(fixture.hwnd, SW_HIDE);
        }
    }
    #[test]
    #[ignore = "visible focus fixture; requires exclusive GUI lease"]
    fn native_caret_ownership_tracks_main_compact_and_native_edit_children() {
        let fixture = EditorFixture::new("main and Compact source", false);
        let compact = EditorFixture::new("unused fixture source", false);
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.enabled = true;
            app.shell.main_window = fixture.hwnd;
            app.shell.compact_window = compact.hwnd;
            let _ = ShowWindow(fixture.hwnd, SW_SHOWNOACTIVATE);
            let _ = ShowWindow(compact.hwnd, SW_SHOWNOACTIVATE);
            app.select_surface(compact.hwnd);
            SetFocus(Some(compact.hwnd)).unwrap();
            app.refresh();
            assert_eq!(native_caret_owner(), Some(compact.hwnd));
            let before = document_state(app.doc());
            let input = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WS_BORDER,
                4,
                4,
                200,
                24,
                Some(compact.hwnd),
                None,
                None,
                None,
            )
            .unwrap();
            SetFocus(Some(input)).unwrap();
            assert_eq!(native_caret_owner(), Some(input));
            app.release_caret();
            assert_eq!(native_caret_owner(), Some(input));
            app.refresh();
            assert_eq!(native_caret_owner(), Some(input));
            SetFocus(Some(compact.hwnd)).unwrap();
            app.update_caret();
            assert_eq!(native_caret_owner(), Some(compact.hwnd));
            app.release_caret();
            app.shell.compact_window = HWND::default();
            app.select_surface(fixture.hwnd);
            SetFocus(Some(fixture.hwnd)).unwrap();
            app.refresh();
            assert_eq!(native_caret_owner(), Some(fixture.hwnd));
            assert_eq!(document_state(app.doc()), before);
            app.release_caret();
            let _ = ShowWindow(compact.hwnd, SW_HIDE);
            let _ = ShowWindow(fixture.hwnd, SW_HIDE);
        }
    }
    #[test]
    #[ignore = "visible focus fixture; requires exclusive GUI lease"]
    fn palette_main_activation_restores_only_a_live_visible_query() {
        let fixture = EditorFixture::new("selected body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.enabled = true;
                app.shell.main_window = fixture.hwnd;
                let settings = app.shell.settings.clone();
                app.apply_shell_settings(settings);
                let _ = ShowWindow(fixture.hwnd, SW_SHOWNOACTIVATE);
                app.open_palette();
            }
            let query = {
                let children = fixture.state.app.borrow();
                // The toolbar owns the sole query; the host owns candidates.
                let main = children.shell.main_window;
                let mut found = HWND::default();
                unsafe extern "system" fn find_query(
                    hwnd: HWND,
                    data: LPARAM,
                ) -> windows::core::BOOL {
                    unsafe {
                        if GetDlgCtrlID(hwnd) == 150 {
                            *(data.0 as *mut HWND) = hwnd;
                        }
                        windows::core::BOOL(1)
                    }
                }
                let _ = EnumChildWindows(
                    Some(main),
                    Some(find_query),
                    LPARAM((&mut found as *mut HWND) as isize),
                );
                found
            };
            assert_ne!(query, HWND::default());
            let before = document_state(fixture.state.app.borrow().doc());
            let _ = SetFocus(Some(fixture.hwnd));
            fixture.message(WM_SETFOCUS, 0);
            assert_eq!(GetFocus(), query);
            let _ = EnableWindow(query, false);
            let _ = SetFocus(Some(fixture.hwnd));
            fixture.message(WM_SETFOCUS, 0);
            assert_eq!(GetFocus(), fixture.hwnd);
            let _ = EnableWindow(query, true);
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            let _ = ShowWindow(fixture.hwnd, SW_HIDE);
        }
    }
    #[test]
    fn named_static_containers_paint_without_captions_in_normal_and_borrowed_states() {
        use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODT_STATIC};
        let mut app = composing_test_app("body");
        app.clear_composition();
        let mut fixture = MessagePumpFixture::new(app);
        let compact = fixture.create_compact();
        unsafe {
            {
                let mut app = fixture.state.app.borrow_mut();
                assert_eq!(app.hwnd, compact); // Native WM_SIZE selected the new surface.
                app.select_surface(fixture.hwnd);
                app.shell.enabled = true;
                app.shell.main_window = fixture.hwnd;
                let settings = app.shell.settings.clone();
                app.apply_shell_settings(settings);
                app.git_init();
                app.open_palette();
            }
            let hosts = [
                (
                    FindWindowExW(Some(fixture.hwnd), None, w!("STATIC"), w!("コマンド候補"))
                        .unwrap(),
                    "コマンド候補",
                ),
                (GetDlgItem(Some(fixture.hwnd), 6508).unwrap(), "Git pane"),
            ];
            let screen = GetDC(Some(fixture.hwnd));
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, 96, 24);
            let previous = SelectObject(dc, bitmap.into());
            let expected = shell::rgb(fixture.state.app.borrow().shell.tokens.panel);
            let before = document_state(fixture.state.app.borrow().doc());
            let check_host = |owner: HWND, host: HWND, name: &str| {
                assert_eq!(shell::text_of(host), name);
                let id = GetDlgCtrlID(host) as usize;
                let item = DRAWITEMSTRUCT {
                    CtlType: ODT_STATIC,
                    CtlID: id as u32,
                    hwndItem: host,
                    hDC: dc,
                    rcItem: RECT {
                        left: 0,
                        top: 0,
                        right: 96,
                        bottom: 24,
                    },
                    ..Default::default()
                };
                assert!(
                    !fixture
                        .state
                        .control_colors
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .draw_container(owner, id + 1, &item)
                );
                for borrowed in [false, true] {
                    let _borrow = borrowed.then(|| fixture.state.app.borrow_mut());
                    PatBlt(dc, 0, 0, 96, 24, BLACKNESS).unwrap();
                    assert_eq!(
                        window_proc(
                            owner,
                            WM_DRAWITEM,
                            WPARAM(id),
                            LPARAM((&item as *const DRAWITEMSTRUCT) as isize)
                        ),
                        LRESULT(1)
                    );
                    for y in 0..24 {
                        for x in 0..96 {
                            assert_eq!(GetPixel(dc, x, y), expected);
                        }
                    }
                }
            };
            for (host, name) in hosts {
                check_host(fixture.hwnd, host, name);
            }
            fixture.state.app.borrow_mut().shell_command(106);
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.compact_window = compact;
                app.select_surface(compact);
                app.open_palette();
            }
            let host =
                FindWindowExW(Some(compact), None, w!("STATIC"), w!("コマンド候補")).unwrap();
            check_host(compact, host, "コマンド候補");
            for owner in [fixture.hwnd, compact] {
                let mut app = fixture.state.app.borrow_mut();
                app.shell_command(106);
                app.select_surface(owner);
                app.open_palette();
                drop(app);
                let host =
                    FindWindowExW(Some(owner), None, w!("STATIC"), w!("コマンド候補")).unwrap();
                for (id, message) in [(150, WM_CTLCOLOREDIT), (151, WM_CTLCOLORLISTBOX)] {
                    let recipient = if id == 150 { owner } else { host };
                    let child = GetDlgItem(Some(recipient), id).unwrap();
                    for borrowed in [false, true] {
                        let _borrow = borrowed.then(|| fixture.state.app.borrow_mut());
                        SetBkColor(dc, COLORREF(0));
                        let brush = SendMessageW(
                            recipient,
                            message,
                            Some(WPARAM(dc.0 as usize)),
                            Some(LPARAM(child.0 as isize)),
                        );
                        assert_ne!(brush.0, 0);
                        assert_eq!(GetBkColor(dc), expected);
                    }
                }
                if owner == compact {
                    for borrowed in [false, true] {
                        let _borrow = borrowed.then(|| fixture.state.app.borrow_mut());
                        SetBkColor(dc, COLORREF(0));
                        SendMessageW(
                            host,
                            WM_CTLCOLOREDIT,
                            Some(WPARAM(dc.0 as usize)),
                            Some(LPARAM(fixture.hwnd.0 as isize)),
                        );
                        assert_ne!(
                            GetBkColor(dc),
                            expected,
                            "foreign HWND must not use palette colors"
                        );
                    }
                }
            }
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ReleaseDC(Some(fixture.hwnd), screen);
        }
    }
    #[test]
    fn search_owned_pushbutton_paints_normal_and_borrowed_states() {
        use windows::Win32::UI::Controls::{
            DRAWITEMSTRUCT, ODS_DISABLED, ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED, ODT_BUTTON,
        };
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.enabled = true;
                app.shell.main_window = fixture.hwnd;
                let settings = app.shell.settings.clone();
                app.apply_shell_settings(settings);
                app.search_init();
            }
            let child = GetDlgItem(Some(fixture.hwnd), 5614).unwrap();
            let screen = GetDC(Some(child));
            let dc = CreateCompatibleDC(Some(screen));
            let bitmap = CreateCompatibleBitmap(screen, 180, 32);
            let previous = SelectObject(dc, bitmap.into());
            let tokens = fixture.state.app.borrow().shell.tokens;
            let before = document_state(fixture.state.app.borrow().doc());
            for state in [
                0,
                ODS_FOCUS.0,
                ODS_SELECTED.0,
                ODS_DISABLED.0,
                ODS_HOTLIGHT.0,
            ] {
                let item = DRAWITEMSTRUCT {
                    CtlType: ODT_BUTTON,
                    CtlID: 5614,
                    hwndItem: child,
                    hDC: dc,
                    rcItem: RECT {
                        left: 0,
                        top: 0,
                        right: 180,
                        bottom: 32,
                    },
                    itemState: windows::Win32::UI::Controls::ODS_FLAGS(state),
                    ..Default::default()
                };
                let expected = shell::rgb(if state & (ODS_SELECTED.0 | ODS_HOTLIGHT.0) != 0 {
                    tokens.selected
                } else {
                    tokens.panel
                });
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_DRAWITEM,
                        WPARAM(5614),
                        LPARAM((&item as *const DRAWITEMSTRUCT) as isize)
                    ),
                    LRESULT(1)
                );
                assert_eq!(GetPixel(dc, 2, 2), expected);
                {
                    let _borrow = fixture.state.app.borrow_mut();
                    assert_eq!(
                        window_proc(
                            fixture.hwnd,
                            WM_DRAWITEM,
                            WPARAM(5614),
                            LPARAM((&item as *const DRAWITEMSTRUCT) as isize)
                        ),
                        LRESULT(1)
                    );
                    assert_eq!(GetPixel(dc, 2, 2), expected);
                }
            }
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            ReleaseDC(Some(child), screen);
        }
    }
    #[test]
    fn search_control_color_reentry_keeps_the_owned_theme() {
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.enabled = true;
                app.shell.main_window = fixture.hwnd;
                let settings = app.shell.settings.clone();
                app.apply_shell_settings(settings);
                app.search_init();
            }
            let child = GetDlgItem(Some(fixture.hwnd), 5602).unwrap();
            let dc = GetDC(Some(child));
            let expected = shell::rgb(fixture.state.app.borrow().shell.tokens.panel);
            let before = document_state(fixture.state.app.borrow().doc());
            {
                let _borrow = fixture.state.app.borrow_mut();
                let brush = window_proc(
                    fixture.hwnd,
                    WM_CTLCOLOREDIT,
                    WPARAM(dc.0 as usize),
                    LPARAM(child.0 as isize),
                );
                assert_ne!(brush.0, 0);
                assert_eq!(GetBkColor(dc), expected);
            }
            ReleaseDC(Some(child), dc);
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
        }
    }
    #[test]
    fn shell_buttons_draw_during_app_borrow_with_current_theme_and_owned_identity() {
        use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODT_BUTTON, ODT_STATIC};
        let mut seed = composing_test_app("unsaved 日本語");
        seed.clear_composition();
        seed.doc_mut().replace_selection("x").unwrap();
        let fixture = MessagePumpFixture::new(seed);
        unsafe {
            fixture.state.app.borrow_mut().start_shell();
            let before = document_state(fixture.state.app.borrow().doc());
            let source = GetDC(Some(fixture.hwnd));
            let dc = CreateCompatibleDC(Some(source));
            let bitmap = CreateCompatibleBitmap(source, 40, 32);
            ReleaseDC(Some(fixture.hwnd), source);
            let previous = SelectObject(dc, bitmap.into());
            fixture.state.deferred_control_redraw.set(false);
            let mut all_pixels = Vec::new();
            for theme in [
                mdlite::settings::ThemeMode::Light,
                mdlite::settings::ThemeMode::Dark,
            ] {
                {
                    let mut app = fixture.state.app.borrow_mut();
                    let mut settings = app.shell.settings.clone();
                    settings.theme = theme;
                    app.apply_shell_settings(settings);
                    app.ensure_layout().unwrap();
                }
                let tokens = fixture.state.app.borrow().shell.tokens;
                for id in [
                    107, CMD_NEW, CMD_SAVE, 168, 169, 108, 109, 110, 111, 112, 113, 114, 115, 130,
                    131, 132, 133, 134,
                ] {
                    let child = GetDlgItem(Some(fixture.hwnd), id as i32).unwrap();
                    let item = DRAWITEMSTRUCT {
                        CtlType: ODT_BUTTON,
                        CtlID: id as u32,
                        hwndItem: child,
                        hDC: dc,
                        rcItem: RECT {
                            left: 0,
                            top: 0,
                            right: 40,
                            bottom: 32,
                        },
                        ..Default::default()
                    };
                    let render = || {
                        PatBlt(dc, 0, 0, 40, 32, BLACKNESS).unwrap();
                        assert_eq!(
                            SendMessageW(
                                fixture.hwnd,
                                WM_DRAWITEM,
                                Some(WPARAM(id)),
                                Some(LPARAM((&item as *const DRAWITEMSTRUCT) as isize))
                            ),
                            LRESULT(1)
                        );
                        (0..32)
                            .flat_map(|y| (0..40).map(move |x| GetPixel(dc, x, y)))
                            .collect::<Vec<_>>()
                    };
                    let ordinary = render();
                    let borrowed = {
                        let _app = fixture.state.app.borrow_mut();
                        render()
                    };
                    assert_eq!(ordinary, borrowed, "theme={theme:?} id={id}");
                    let background = borrowed[0];
                    let ink = (4..26)
                        .flat_map(|y| (8..32).map(move |x| y * 40 + x))
                        .filter(|index| borrowed[*index] != background)
                        .count();
                    assert!(ink > 4, "missing icon id={id}; interior ink pixels={ink}");
                    if id == 107 {
                        assert_eq!(borrowed[0], shell::rgb(tokens.panel));
                    }
                    if id == 108 {
                        assert_eq!(borrowed[0], shell::rgb(tokens.selected));
                    }
                    all_pixels.push(borrowed);
                    let paint = fixture.state.shell_button_paint.borrow().clone().unwrap();
                    let mut wrong = item;
                    wrong.CtlID += 1;
                    assert!(!paint.draw(fixture.hwnd, id, &wrong));
                    wrong = item;
                    wrong.CtlType = ODT_STATIC;
                    assert!(!paint.draw(fixture.hwnd, id, &wrong));
                    assert!(!paint.draw(fixture.hwnd, id + 1, &item));
                    assert!(!paint.draw(HWND::default(), id, &item));
                    let foreign = shell::control(
                        fixture.hwnd,
                        w!("BUTTON"),
                        "foreign same id",
                        id,
                        WINDOW_STYLE(BS_OWNERDRAW as u32),
                    );
                    wrong = item;
                    wrong.hwndItem = foreign;
                    assert!(!paint.draw(fixture.hwnd, id, &wrong));
                    DestroyWindow(foreign).unwrap();
                }
            }
            assert_ne!(
                all_pixels[0], all_pixels[18],
                "light and dark must not reuse stale colors"
            );
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            assert!(!fixture.state.deferred_control_redraw.get());
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
        }
    }
    #[test]
    fn editor_reentrant_control_paint_waits_for_themed_owner() {
        let fixture = EditorFixture::new("body", false);
        unsafe {
            let child = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("owned repaint fixture"),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE(BS_OWNERDRAW as u32),
                0,
                0,
                80,
                24,
                Some(fixture.hwnd),
                None,
                None,
                None,
            )
            .unwrap();
            let before = document_state(fixture.state.app.borrow().doc());
            {
                let _borrow = fixture.state.app.borrow_mut();
                // Modal callbacks cannot borrow App to theme this native child.
                window_proc(
                    fixture.hwnd,
                    WM_CTLCOLORBTN,
                    WPARAM(0),
                    LPARAM(child.0 as isize),
                );
                assert!(fixture.state.deferred_control_redraw.get());
                fixture.message(WM_NULL, 0);
                assert!(fixture.state.deferred_control_redraw.get());
            }
            fixture.message(WM_NULL, 0);
            assert!(!fixture.state.deferred_control_redraw.get());
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            // Hidden children do not expose an update region. Root's visible
            // modal scenario verifies the resulting themed pixels separately.
        }
    }
    #[test]
    fn themed_popup_snapshot_reentry_disabled_and_native_restore() {
        use windows::Win32::UI::Controls::*;
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            let child = CreatePopupMenu().unwrap();
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 12, w!("保存(&S)\tCtrl+S")).unwrap();
            AppendMenuW(menu, MF_POPUP, child.0 as usize, w!("表示")).unwrap();
            AppendMenuW(child, MF_STRING, 13, w!("日本語項目(&J)")).unwrap();
            AppendMenuW(child, MF_SEPARATOR, 0, PCWSTR::null()).unwrap();
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_DATA,
                dwItemData: 42,
                ..Default::default()
            };
            SetMenuItemInfoW(menu, 0, true, &info).unwrap();
            let mut background = MENUINFO {
                cbSize: std::mem::size_of::<MENUINFO>() as u32,
                fMask: MIM_BACKGROUND,
                ..Default::default()
            };
            GetMenuInfo(menu, &mut background).unwrap();
            let old_background = background.hbrBack;
            let before = document_state(fixture.state.app.borrow().doc());
            let scope = menu_style::MenuScope::new(
                &fixture.state,
                fixture.hwnd,
                menu,
                shell::Tokens::default(),
                1.,
            )
            .expect("standard-contrast fixture");
            info.fMask = MIIM_DATA | MIIM_FTYPE | MIIM_STATE;
            GetMenuItemInfoW(menu, 0, true, &mut info).unwrap();
            assert!(info.fType.contains(MFT_OWNERDRAW));
            assert_ne!(info.fState.0 & MFS_DISABLED.0, 0);
            assert_ne!(info.dwItemData, 42);
            let mut text = [0u16; 64];
            let length = GetMenuStringW(menu, 0, Some(&mut text), MF_BYPOSITION);
            assert_eq!(
                String::from_utf16_lossy(&text[..length as usize]),
                "保存(&S)\tCtrl+S"
            );
            let dc = CreateCompatibleDC(None);
            let bitmap = CreateBitmap(320, 40, 1, 32, None);
            let old = SelectObject(dc, bitmap.into());
            let mut measure = MEASUREITEMSTRUCT {
                CtlType: ODT_MENU,
                itemID: 12,
                itemData: info.dwItemData,
                ..Default::default()
            };
            let mut item = DRAWITEMSTRUCT {
                CtlType: ODT_MENU,
                itemID: 12,
                itemData: info.dwItemData,
                itemState: ODS_FLAGS(ODS_SELECTED.0 | ODS_DISABLED.0),
                hwndItem: HWND(menu.0),
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 40,
                },
                ..Default::default()
            };
            {
                let app = fixture.state.app.borrow_mut();
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_INITMENUPOPUP,
                        WPARAM(child.0 as usize),
                        LPARAM(0)
                    ),
                    LRESULT(0)
                );
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_MENUCHAR,
                        WPARAM('s' as usize),
                        LPARAM(menu.0 as isize)
                    ),
                    LRESULT((MNC_IGNORE << 16) as isize)
                );
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_MENUCHAR,
                        WPARAM('j' as usize),
                        LPARAM(child.0 as isize)
                    ),
                    LRESULT((MNC_EXECUTE << 16) as isize)
                );
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_MEASUREITEM,
                        WPARAM(0),
                        LPARAM((&mut measure as *mut MEASUREITEMSTRUCT) as isize)
                    ),
                    LRESULT(1)
                );
                assert!(
                    measure.itemWidth > 100
                        && measure.itemHeight == shell::UI_ROW_HEIGHT.round() as u32
                );
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_DRAWITEM,
                        WPARAM(0),
                        LPARAM((&item as *const DRAWITEMSTRUCT) as isize)
                    ),
                    LRESULT(1)
                );
                // A disabled selected item uses panel, never active hover color.
                assert_eq!(
                    GetPixel(dc, 8, 8),
                    shell::rgb(shell::Tokens::default().panel)
                );
                assert_eq!(document_state(app.doc()), before);
                item.hwndItem = HWND(child.0);
                assert!(
                    menu_style::window_message(
                        &fixture.state,
                        fixture.hwnd,
                        WM_DRAWITEM,
                        WPARAM(0),
                        LPARAM((&item as *const DRAWITEMSTRUCT) as isize)
                    )
                    .is_none()
                );
            }
            let _ = SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            drop(scope);
            assert!(fixture.state.menu_paint.borrow().is_empty());
            GetMenuItemInfoW(menu, 0, true, &mut info).unwrap();
            assert!(!info.fType.contains(MFT_OWNERDRAW));
            assert_eq!(info.dwItemData, 42);
            assert_ne!(info.fState.0 & MFS_DISABLED.0, 0);
            GetMenuInfo(menu, &mut background).unwrap();
            assert_eq!(background.hbrBack, old_background);
            DestroyMenu(menu).unwrap();
        }
    }
    #[test]
    fn themed_popup_scope_high_contrast_foreign_items_and_unwind() {
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            let menu = CreatePopupMenu().unwrap();
            AppendMenuW(menu, MF_STRING, 12, w!("項目")).unwrap();
            let tokens = shell::Tokens {
                high_contrast: true,
                ..Default::default()
            };
            assert!(
                menu_style::MenuScope::new(&fixture.state, fixture.hwnd, menu, tokens, 1.)
                    .is_none()
            );
            let mut info = MENUITEMINFOW {
                cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_DATA,
                ..Default::default()
            };
            GetMenuItemInfoW(menu, 0, true, &mut info).unwrap();
            assert!(!info.fType.contains(MFT_OWNERDRAW));
            let outer = menu_style::MenuScope::new(
                &fixture.state,
                fixture.hwnd,
                menu,
                shell::Tokens::default(),
                1.,
            )
            .unwrap();
            // Tracking the same handles must not replace the active snapshot.
            assert!(
                menu_style::MenuScope::new(
                    &fixture.state,
                    fixture.hwnd,
                    menu,
                    shell::Tokens::default(),
                    1.
                )
                .is_none()
            );
            assert_eq!(fixture.state.menu_paint.borrow().len(), 1);
            drop(outer);
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _scope = menu_style::MenuScope::new(
                    &fixture.state,
                    fixture.hwnd,
                    menu,
                    shell::Tokens::default(),
                    1.,
                )
                .unwrap();
                panic!("menu unwind fixture");
            }));
            assert!(result.is_err());
            assert!(fixture.state.menu_paint.borrow().is_empty());
            GetMenuItemInfoW(menu, 0, true, &mut info).unwrap();
            assert!(!info.fType.contains(MFT_OWNERDRAW));
            info.fType |= MFT_OWNERDRAW;
            info.dwItemData = 99;
            SetMenuItemInfoW(menu, 0, true, &info).unwrap();
            assert!(
                menu_style::MenuScope::new(
                    &fixture.state,
                    fixture.hwnd,
                    menu,
                    shell::Tokens::default(),
                    1.
                )
                .is_none()
            );
            GetMenuItemInfoW(menu, 0, true, &mut info).unwrap();
            assert!(info.fType.contains(MFT_OWNERDRAW));
            assert_eq!(info.dwItemData, 99);
            DestroyMenu(menu).unwrap();
        }
    }
    #[test]
    fn scoped_button_paint_is_owned_guarded_and_preserves_document() {
        use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODT_BUTTON, ODT_LISTBOX};
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        unsafe {
            let child = shell::control(
                fixture.hwnd,
                w!("BUTTON"),
                "caption",
                34,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            let other = shell::control(
                fixture.hwnd,
                w!("BUTTON"),
                "other",
                34,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            let dc = GetDC(Some(fixture.hwnd));
            let mut item = DRAWITEMSTRUCT {
                CtlType: ODT_BUTTON,
                CtlID: 34,
                hwndItem: child,
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 24,
                },
                ..Default::default()
            };
            let mut label = String::from("owned caption");
            let before = document_state(fixture.state.app.borrow().doc());
            let scope = ButtonPaintScope::new(
                &fixture.state,
                shell::OwnerButtonPaint {
                    recipient: fixture.hwnd,
                    parent: fixture.hwnd,
                    parent_id: None,
                    child,
                    id: 34,
                    tokens: shell::Tokens::default(),
                    label: label.clone(),
                },
            );
            label.clear();
            let paint = fixture.state.button_paint.borrow().clone().unwrap();
            assert_eq!(
                paint.state(fixture.hwnd, 34, &item).unwrap().label,
                "owned caption"
            );
            assert!(paint.state(other, 34, &item).is_none());
            assert!(paint.state(fixture.hwnd, 35, &item).is_none());
            item.CtlType = ODT_LISTBOX;
            assert!(paint.state(fixture.hwnd, 34, &item).is_none());
            item.CtlType = ODT_BUTTON;
            item.hwndItem = other;
            assert!(paint.state(fixture.hwnd, 34, &item).is_none());
            item.hwndItem = child;
            let _ = EnableWindow(child, false);
            assert!(!paint.state(fixture.hwnd, 34, &item).unwrap().enabled);
            let _ = EnableWindow(child, true);
            item.itemState.0 = windows::Win32::UI::Controls::ODS_DISABLED.0;
            assert!(!paint.state(fixture.hwnd, 34, &item).unwrap().enabled);
            item.itemState.0 = 0;
            {
                let app = fixture.state.app.borrow_mut();
                assert_eq!(
                    window_proc(
                        fixture.hwnd,
                        WM_DRAWITEM,
                        WPARAM(34),
                        LPARAM((&item as *const DRAWITEMSTRUCT) as isize),
                    ),
                    LRESULT(1)
                );
                assert_eq!(document_state(app.doc()), before);
                assert!(!fixture.state.deferred_control_redraw.get());
            }
            SetWindowLongPtrW(child, GWLP_ID, 35);
            assert!(paint.state(fixture.hwnd, 34, &item).is_none());
            SetWindowLongPtrW(child, GWLP_ID, 34);
            let _ = SetParent(child, Some(other));
            assert!(paint.state(fixture.hwnd, 34, &item).is_none());
            let _ = SetParent(child, Some(fixture.hwnd));
            let host = shell::control(
                fixture.hwnd,
                w!("STATIC"),
                "host",
                6508,
                WINDOW_STYLE::default(),
            );
            let hosted = shell::OwnerButtonPaint {
                recipient: fixture.hwnd,
                parent: host,
                parent_id: Some(6508),
                child,
                id: 34,
                tokens: shell::Tokens::default(),
                label: "hosted".into(),
            };
            let _ = SetParent(child, Some(host));
            assert!(hosted.state(fixture.hwnd, 34, &item).is_some());
            SetWindowLongPtrW(host, GWLP_ID, 6509);
            assert!(hosted.state(fixture.hwnd, 34, &item).is_none());
            DestroyWindow(child).unwrap();
            assert!(paint.state(fixture.hwnd, 34, &item).is_none());
            drop(scope);
            assert!(fixture.state.button_paint.borrow().is_none());
            ReleaseDC(Some(fixture.hwnd), dc);
        }
    }
    #[test]
    fn scoped_button_paint_nested_and_unwound_scopes_never_restore_stale_data() {
        let fixture = EditorFixture::new("body", false);
        let paint = |label: &str| shell::OwnerButtonPaint {
            recipient: fixture.hwnd,
            parent: fixture.hwnd,
            parent_id: None,
            child: HWND::default(),
            id: 34,
            tokens: shell::Tokens::default(),
            label: label.into(),
        };
        let outer = ButtonPaintScope::new(&fixture.state, paint("outer"));
        {
            let _inner = ButtonPaintScope::new(&fixture.state, paint("inner"));
            assert_eq!(
                fixture.state.button_paint.borrow().as_ref().unwrap().label,
                "inner"
            );
        }
        assert!(fixture.state.button_paint.borrow().is_none());
        drop(outer);
        assert!(fixture.state.button_paint.borrow().is_none());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _scope = ButtonPaintScope::new(&fixture.state, paint("unwind"));
            panic!("scope unwind fixture");
        }));
        assert!(result.is_err());
        assert!(fixture.state.button_paint.borrow().is_none());
    }
    fn visible_paint_fixture(source: &str) -> MessagePumpFixture {
        let mut app = composing_test_app(source);
        app.clear_composition();
        let fixture = MessagePumpFixture::new(app);
        // Hidden HWNDs need not retain an update region. This owned, inactive
        // surface exercises the actual Win32 invalidation/WM_PAINT contract.
        unsafe {
            let _ = ShowWindow(fixture.hwnd, SW_SHOWNOACTIVATE);
        }
        fixture
    }

    #[test]
    fn composited_dc_hidden_binding_damage_resize_and_compact_keep_source() {
        let mut seed = composing_test_app("# DC fixture\nsource 日本語\n");
        seed.clear_composition();
        seed.doc_mut().replace_selection("x").unwrap();
        let mut fixture = MessagePumpFixture::new(seed);
        let compact = fixture.create_compact();
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| *state.borrow_mut() = PaintTestState::default());
        unsafe {
            assert!(
                !IsWindowVisible(fixture.hwnd).as_bool() && !IsWindowVisible(compact).as_bool()
            );
            let focus = GetFocus();
            let mut app = fixture.state.app.borrow_mut();
            app.shell.main_window = fixture.hwnd;
            app.shell.compact_window = compact;
            app.select_surface(fixture.hwnd);
            let main = PaintBitmap::new(fixture.hwnd).unwrap();
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(main.dc, rect(0., 0., 900., 700.))
                .unwrap();
            let main_target = app.target().unwrap();
            let size = main_target.GetPixelSize();
            assert_eq!(
                (size.width, size.height),
                (main.client.right as u32, main.client.bottom as u32)
            );
            let sentinel = COLORREF(0x0031_72b4);
            SetPixel(
                main.dc,
                main.client.right - 2,
                main.client.bottom - 2,
                sentinel,
            );
            SetPixel(main.dc, 425, 305, sentinel);
            let saved = SaveDC(main.dc);
            IntersectClipRect(main.dc, 420, 300, 430, 310);
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(main.dc, rect(420., 300., 10., 10.))
                .unwrap();
            RestoreDC(main.dc, saved).ok().unwrap();
            assert_eq!(
                GetPixel(main.dc, main.client.right - 2, main.client.bottom - 2),
                sentinel,
                "BeginPaint-equivalent DC clipping preserves outside damage"
            );
            let actual = GetPixel(main.dc, 425, 305).0;
            let expected = shell::rgb(app.shell.tokens.background).0;
            for shift in [0, 8, 16] {
                assert!(
                    ((actual >> shift) & 255).abs_diff((expected >> shift) & 255) <= 1,
                    "D2D BGRA conversion may round one level; the damaged pixel must be repainted"
                );
            }
            app.select_surface(compact);
            let small = PaintBitmap::new(compact).unwrap();
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(small.dc, rect(0., 0., 640., 480.))
                .unwrap();
            assert_ne!(
                app.target().unwrap().as_raw(),
                main_target.as_raw(),
                "separate surface resource domains"
            );
            PAINT_TEST
                .with(|state| assert_eq!(state.borrow().bound_surface, Some(compact.0 as usize)));
            app.select_surface(fixture.hwnd);
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(main.dc, rect(0., 0., 900., 700.))
                .unwrap();
            assert_eq!(app.target().unwrap().as_raw(), main_target.as_raw());
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                960,
                720,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            app.resize_target(D2D_SIZE_U {
                width: 960,
                height: 720,
            });
            let resized = PaintBitmap::new(fixture.hwnd).unwrap();
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(resized.dc, rect(1., 1., 2., 2.))
                .unwrap();
            let pixels = app.target().unwrap().GetPixelSize();
            assert_eq!(
                (pixels.width, pixels.height),
                (resized.client.right as u32, resized.client.bottom as u32)
            );
            let frame = app.painted_frame;
            PAINT_TEST.with(|state| {
                let mut state = state.borrow_mut();
                state.fault = Some(PaintFault::Bind);
                state.failures = 1;
            });
            assert!(
                app.paint_prepared_to_dc(resized.dc, rect(1., 1., 2., 2.))
                    .is_err()
            );
            assert_eq!(app.painted_frame, frame, "failed BindDC publishes no frame");
            app.discard_target();
            app.prepare_paint().unwrap();
            app.paint_prepared_to_dc(resized.dc, rect(1., 1., 2., 2.))
                .unwrap();
            assert_ne!(app.target().unwrap().as_raw(), main_target.as_raw());
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(GetFocus(), focus);
        }
    }
    #[test]
    fn composited_dc_hidden_native_paint_bind_failure_uses_existing_recovery() {
        let mut seed = composing_test_app("native paint source");
        seed.clear_composition();
        let fixture = MessagePumpFixture::new(seed);
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| {
            *state.borrow_mut() = PaintTestState {
                fault: Some(PaintFault::Bind),
                failures: 1,
                ..Default::default()
            }
        });
        unsafe {
            SendMessageW(fixture.hwnd, WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
            assert!(fixture.state.app.borrow().paint_retry_pending);
            assert!(fixture.state.app.borrow().graphics.target.is_none());
            SendMessageW(fixture.hwnd, WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
            assert!(!fixture.state.app.borrow().paint_retry_pending);
            assert!(fixture.state.app.borrow().graphics.target.is_some());
            PAINT_TEST.with(|state| {
                let state = state.borrow();
                assert_eq!((state.began, state.ended), (1, 1));
                assert_eq!(state.bound_surface, Some(fixture.hwnd.0 as usize));
            });
            assert!(!IsWindowVisible(fixture.hwnd).as_bool());
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
        }
    }

    #[test]
    fn paint_recovery_hidden_startup_prepares_controls_before_first_frame() {
        let fixture = EditorFixture::new("# Startup\n\nbody", false);
        let before = document_state(fixture.state.app.borrow().doc());
        unsafe {
            fixture.state.main_window.set(fixture.hwnd);
            fixture.state.editor_window.set(fixture.hwnd);
            SetWindowLongPtrW(
                fixture.hwnd,
                GWLP_WNDPROC,
                window_proc as *const () as isize,
            );
            let mut app = fixture.state.app.borrow_mut();
            app.start_shell();
            app.features_init();
            app.git_init();
            app.search_init();
            app.editor_actions_init();
            app.ensure_layout().unwrap();
            assert!(!IsWindowVisible(fixture.hwnd).as_bool());
            assert!(app.layout_key.is_some());
            assert!(app.graphics.target.is_none());
            let editor = app.shell.editor;
            app.refresh();
            assert_eq!(app.shell.editor, editor);
            app.paint(rect(1., 2., 3., 4.)).unwrap();
            assert!(!app.full_paint);
            assert_eq!(document_state(app.doc()), before);
        }
    }

    #[test]
    fn paint_recovery_activation_repaints_owned_surfaces_before_app_borrow() {
        let mut fixture = visible_paint_fixture("activation body");
        let main = fixture.hwnd;
        let compact = fixture.create_compact();
        unsafe {
            let _ = ShowWindow(compact, SW_SHOWNOACTIVATE);
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.enabled = true;
            app.shell.main_window = main;
            app.shell.compact_window = compact;
            unsafe {
                app.select_surface(main);
                app.target().unwrap();
                app.select_surface(compact);
                app.target().unwrap();
                app.select_surface(main);
            }
        }
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| *state.borrow_mut() = PaintTestState::default());
        unsafe {
            for (hwnd, message, activation) in [
                (main, WM_ACTIVATE, WA_ACTIVE as usize),
                (main, WM_ACTIVATE, WA_CLICKACTIVE as usize),
                (compact, WM_ACTIVATE, WA_ACTIVE as usize),
                (compact, WM_ACTIVATE, WA_CLICKACTIVE as usize),
                (main, WM_NCACTIVATE, 1),
                (compact, WM_NCACTIVATE, 1),
            ] {
                SetFocus(Some(hwnd)).unwrap();
                assert_eq!(GetFocus(), hwnd, "fixture must focus the target first");
                let _ = ValidateRect(Some(hwnd), None);
                assert!(!GetUpdateRect(hwnd, None, false).as_bool());
                SendMessageW(hwnd, message, Some(WPARAM(activation)), Some(LPARAM(0)));
                assert_eq!(GetFocus(), hwnd, "activation changed thread focus");
                let mut update = RECT::default();
                assert!(GetUpdateRect(hwnd, Some(&mut update), false).as_bool());
                let mut client = RECT::default();
                GetClientRect(hwnd, &mut client).unwrap();
                assert_eq!(
                    [update.left, update.top, update.right, update.bottom],
                    [client.left, client.top, client.right, client.bottom],
                    "activation must invalidate the full owned surface"
                );
                let before_paint = PAINT_TEST.with(|state| {
                    let state = state.borrow();
                    (state.began, state.ended)
                });
                let _ = UpdateWindow(hwnd);
                assert_eq!(GetFocus(), hwnd, "painting changed thread focus");
                PAINT_TEST.with(|state| {
                    let state = state.borrow();
                    assert_eq!(
                        (state.began, state.ended),
                        (before_paint.0 + 1, before_paint.1 + 1),
                        "activation must reach one complete WM_PAINT"
                    );
                });
                assert!(!GetUpdateRect(hwnd, None, false).as_bool());
                assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            }

            for (hwnd, activation) in [
                (main, WA_INACTIVE as usize),
                (main, WA_ACTIVE as usize | (1 << 16)),
                (compact, WA_INACTIVE as usize),
                (compact, WA_CLICKACTIVE as usize | (1 << 16)),
            ] {
                SetFocus(Some(hwnd)).unwrap();
                assert_eq!(GetFocus(), hwnd);
                let _ = ValidateRect(Some(hwnd), None);
                SendMessageW(hwnd, WM_ACTIVATE, Some(WPARAM(activation)), Some(LPARAM(0)));
                assert!(!GetUpdateRect(hwnd, None, false).as_bool());
            }

            for hwnd in [main, compact] {
                SetFocus(Some(hwnd)).unwrap();
                let _ = ValidateRect(Some(hwnd), None);
                SendMessageW(hwnd, WM_NCACTIVATE, Some(WPARAM(0)), Some(LPARAM(0)));
                assert!(!GetUpdateRect(hwnd, None, false).as_bool());
            }

            for (hwnd, message, activation) in [
                (main, WM_ACTIVATE, WA_ACTIVE as usize),
                (compact, WM_ACTIVATE, WA_CLICKACTIVE as usize),
                (main, WM_NCACTIVATE, 1),
                (compact, WM_NCACTIVATE, 1),
            ] {
                SetFocus(Some(hwnd)).unwrap();
                assert_eq!(GetFocus(), hwnd);
                let _ = ValidateRect(Some(hwnd), None);
                {
                    let _app = fixture.state.app.borrow_mut();
                    SendMessageW(hwnd, message, Some(WPARAM(activation)), Some(LPARAM(0)));
                    assert_eq!(GetFocus(), hwnd);
                    assert!(GetUpdateRect(hwnd, None, false).as_bool());
                    let _ = UpdateWindow(hwnd);
                    assert!(
                        fixture
                            .state
                            .deferred_paints
                            .get()
                            .iter()
                            .flatten()
                            .any(|(deferred, _)| *deferred == hwnd)
                    );
                    assert!(!GetUpdateRect(hwnd, None, false).as_bool());
                }
                SendMessageW(fixture.hwnd, WM_NULL, Some(WPARAM(0)), Some(LPARAM(0)));
                assert!(GetUpdateRect(hwnd, None, false).as_bool());
                assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
                let before_paint = PAINT_TEST.with(|state| {
                    let state = state.borrow();
                    (state.began, state.ended)
                });
                let _ = UpdateWindow(hwnd);
                PAINT_TEST.with(|state| {
                    let state = state.borrow();
                    assert_eq!(
                        (state.began, state.ended),
                        (before_paint.0 + 1, before_paint.1 + 1),
                        "replayed paint must complete once App is available"
                    );
                });
                assert_eq!(GetFocus(), hwnd);
                assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            }

            let _ = ShowWindow(main, SW_SHOWMINNOACTIVE);
            assert!(IsIconic(main).as_bool(), "test main window must be iconic");
            let _ = ValidateRect(Some(main), None);
            assert!(!GetUpdateRect(main, None, false).as_bool());
            SendMessageW(main, WM_NCACTIVATE, Some(WPARAM(1)), Some(LPARAM(0)));
            assert!(!GetUpdateRect(main, None, false).as_bool());
        }
        drop(fixture);
        unsafe {
            assert!(!IsWindow(Some(main)).as_bool());
            assert!(!IsWindow(Some(compact)).as_bool());
        }
    }

    #[test]
    fn paint_recovery_unwinds_clips_and_bounds_persistent_failure() {
        let fixture = visible_paint_fixture("unsaved [link](https://example.test)");
        fixture
            .state
            .app
            .borrow_mut()
            .doc_mut()
            .replace_selection("x")
            .unwrap();
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| {
            *state.borrow_mut() = PaintTestState {
                fault: Some(PaintFault::Drawing),
                failures: 2,
                ..Default::default()
            }
        });
        unsafe {
            let _ = InvalidateRect(Some(fixture.hwnd), None, false);
            let _ = UpdateWindow(fixture.hwnd);
            for view in &fixture.state.app.borrow().views {
                for position in 0..view.text.len() as u32 {
                    let mut effect = None;
                    view.layout
                        .GetDrawingEffect(position, &mut effect, None)
                        .unwrap();
                    assert!(
                        effect.is_none(),
                        "failed target brushes must leave retained layouts"
                    );
                }
            }
            assert!(
                GetUpdateRect(fixture.hwnd, None, false).as_bool(),
                "one retry must remain after EndPaint"
            );
            assert!(fixture.state.app.borrow().graphics.target.is_none());
            let _ = UpdateWindow(fixture.hwnd);
            assert!(
                !GetUpdateRect(fixture.hwnd, None, false).as_bool(),
                "persistent failure must stop retrying"
            );
            PAINT_TEST.with(|state| {
                let state = state.borrow();
                assert_eq!((state.began, state.ended, state.unwound_clips), (2, 2, 4));
            });
            fixture.state.app.borrow_mut().refresh();
            let _ = UpdateWindow(fixture.hwnd);
        }
        let app = fixture.state.app.borrow();
        assert!(app.graphics.target.is_some());
        assert!(!app.full_paint);
        assert!(!app.paint_retry_pending);
        assert_eq!(document_state(app.doc()), before);
        PAINT_TEST.with(|state| {
            let state = state.borrow();
            assert_eq!((state.began, state.ended), (3, 3));
        });
    }

    #[test]
    fn paint_recovery_target_and_enddraw_failures_recreate_full_frame() {
        for fault in [PaintFault::Target, PaintFault::Bind, PaintFault::EndDraw] {
            let fixture = visible_paint_fixture("retained body");
            let before = document_state(fixture.state.app.borrow().doc());
            PAINT_TEST.with(|state| {
                *state.borrow_mut() = PaintTestState {
                    fault: Some(fault),
                    failures: 1,
                    ..Default::default()
                }
            });
            unsafe {
                let partial = RECT {
                    left: 1,
                    top: 2,
                    right: 8,
                    bottom: 9,
                };
                let _ = InvalidateRect(Some(fixture.hwnd), Some(&partial), false);
                let _ = UpdateWindow(fixture.hwnd);
                assert!(fixture.state.app.borrow().graphics.target.is_none());
                assert!(GetUpdateRect(fixture.hwnd, None, false).as_bool());
                let _ = UpdateWindow(fixture.hwnd);
                let mut app = fixture.state.app.borrow_mut();
                let size = app.target().unwrap().GetSize();
                PAINT_TEST.with(|state| {
                    let state = state.borrow();
                    assert_eq!(state.damage, Some(rect(0., 0., size.width, size.height)));
                    let expected = if matches!(fault, PaintFault::Target | PaintFault::Bind) {
                        1
                    } else {
                        2
                    };
                    assert_eq!((state.began, state.ended), (expected, expected));
                });
                let small = rect(2., 3., 4., 5.);
                app.paint(small).unwrap();
                PAINT_TEST.with(|state| assert_eq!(state.borrow().damage, Some(small)));
                assert_eq!(document_state(app.doc()), before);
            }
        }
    }

    #[test]
    fn paint_recovery_inactive_main_and_compact_share_target_recovery() {
        let mut fixture = visible_paint_fixture("body owned by Compact");
        let compact = fixture.create_compact();
        {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.enabled = true;
            app.shell.main_window = fixture.hwnd;
            app.shell.compact_window = compact;
        }
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| {
            *state.borrow_mut() = PaintTestState {
                fault: Some(PaintFault::Drawing),
                failures: 1,
                ..Default::default()
            }
        });
        unsafe {
            let _ = InvalidateRect(Some(fixture.hwnd), None, false);
            let _ = UpdateWindow(fixture.hwnd);
            assert!(fixture.state.app.borrow().graphics.target.is_none());
            assert!(GetUpdateRect(fixture.hwnd, None, false).as_bool());
            let _ = UpdateWindow(fixture.hwnd);
            let mut app = fixture.state.app.borrow_mut();
            PAINT_TEST.with(|state| {
                assert_eq!(state.borrow().bound_surface, Some(fixture.hwnd.0 as usize))
            });
            PAINT_TEST.with(|state| {
                let state = state.borrow();
                assert_eq!((state.began, state.ended, state.unwound_clips), (2, 2, 2));
            });
            for surface in [compact, fixture.hwnd] {
                app.select_surface(surface);
                assert!(app.full_paint);
                app.paint(rect(1., 2., 3., 4.)).unwrap();
                PAINT_TEST.with(|state| {
                    assert_eq!(state.borrow().bound_surface, Some(surface.0 as usize))
                });
                assert!(!app.full_paint);
            }
            assert_eq!(document_state(app.doc()), before);
        }
    }

    #[test]
    fn paint_recovery_frame_identity_tracks_only_successful_editor_frames() {
        let mut seed = composing_test_app("frame body");
        seed.clear_composition();
        let mut fixture = MessagePumpFixture::new(seed);
        let compact = fixture.create_compact();
        PAINT_TEST.with(|state| *state.borrow_mut() = PaintTestState::default());
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.select_surface(fixture.hwnd);
            assert_eq!(app.painted_frame, None);
            app.paint(rect(0., 0., 450., 480.)).unwrap();
            let first = app.painted_frame.unwrap();
            assert_eq!(first.hwnd, fixture.hwnd.0 as usize);
            assert_eq!(first.document_id, app.doc().id());
            assert_eq!(first.revision, app.doc().revision());
            app.doc_mut().replace_selection("new ").unwrap();
            assert_ne!(first.revision, app.doc().revision());
            let before = document_state(app.doc());
            app.refresh();
            assert_eq!(
                app.painted_frame,
                Some(first),
                "refresh is not a rendered frame"
            );
            app.minimized = true;
            app.paint(rect(0., 0., 450., 480.)).unwrap();
            assert_eq!(app.painted_frame, Some(first));
            app.minimized = false;
            PAINT_TEST.with(|state| {
                let mut state = state.borrow_mut();
                state.fault = Some(PaintFault::EndDraw);
                state.failures = 1;
            });
            assert!(app.paint(rect(0., 0., 450., 480.)).is_err());
            assert_eq!(
                app.painted_frame,
                Some(first),
                "failed EndDraw cannot certify a frame"
            );
            app.discard_target();
            app.paint(rect(0., 0., 450., 480.)).unwrap();
            assert_eq!(app.painted_frame.unwrap().sequence, first.sequence + 1);
            assert_eq!(app.painted_frame.unwrap().revision, app.doc().revision());
            let editor_frame = app.painted_frame;
            app.shell.enabled = true;
            app.shell.main_window = fixture.hwnd;
            app.shell.compact_window = compact;
            app.paint(rect(0., 0., 450., 480.)).unwrap();
            assert_eq!(
                app.painted_frame, editor_frame,
                "main placeholder is not the editor"
            );
            app.select_surface(compact);
            app.paint(rect(0., 0., 450., 480.)).unwrap();
            assert_eq!(app.painted_frame.unwrap().hwnd, compact.0 as usize);
            assert_eq!(app.painted_frame.unwrap().sequence, first.sequence + 2);
            assert_eq!(document_state(app.doc()), before);
        }
    }

    #[test]
    fn paint_recovery_resize_invalidates_frame_and_discards_failed_target() {
        let fixture = EditorFixture::new("resize body", false);
        let before = document_state(fixture.state.app.borrow().doc());
        PAINT_TEST.with(|state| *state.borrow_mut() = PaintTestState::default());
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            let small = rect(2., 3., 4., 5.);
            app.paint(small).unwrap();
            let target = app.target().unwrap();
            let size = target.GetPixelSize();
            app.resize_target(size);
            app.paint(small).unwrap();
            let full = target.GetSize();
            PAINT_TEST.with(|state| {
                assert_eq!(
                    state.borrow().damage,
                    Some(rect(0., 0., full.width, full.height))
                )
            });
            PAINT_TEST.with(|state| {
                let mut state = state.borrow_mut();
                state.fault = Some(PaintFault::Bind);
                state.failures = 1;
            });
            app.resize_target(size);
            assert!(
                app.paint(small).is_err(),
                "resize's next DC binding can fail"
            );
            app.discard_target();
            assert!(app.graphics.target.is_none());
            app.paint(small).unwrap();
            assert_ne!(app.target().unwrap().as_raw(), target.as_raw());
            PAINT_TEST.with(|state| {
                assert_eq!(
                    state.borrow().damage,
                    Some(rect(0., 0., full.width, full.height))
                )
            });
            assert_eq!(document_state(app.doc()), before);
        }
    }

    #[test]
    fn borrowed_update_window_retains_title_outline_damage_until_normal_replay() {
        let foreground = unsafe { GetForegroundWindow() };
        let fixture = visible_paint_fixture("# Outline damage fixture\nunchanged body😀\n");
        fixture.state.main_window.set(fixture.hwnd);
        let (title, outline, dpi) = {
            let mut app = fixture.state.app.borrow_mut();
            unsafe { app.start_shell() };
            app.focus_feature_panel(mdlite::panel::PanelId::Outline);
            assert!(unsafe { app.shell_command(162) }); // Existing Right-independent-group command.
            unsafe { app.refresh() };
            let explorer = app.shell.panels[mdlite::panel::PanelId::Explorer as usize].1;
            let outline = app.shell.panels[mdlite::panel::PanelId::Outline as usize].1;
            assert!(
                explorer.width > 50. && outline.width > 50.,
                "fixture must expose both paint domains"
            );
            let pixels = |r: mdlite::panel::Rect, top: f32, bottom: f32| RECT {
                left: ((r.x + 8.) * app.dpi).round() as i32,
                top: ((r.y + top) * app.dpi).round() as i32,
                right: ((r.x + r.width - 40.) * app.dpi).round() as i32,
                bottom: ((r.y + bottom) * app.dpi).round() as i32,
            };
            (
                pixels(explorer, 4., 26.),
                pixels(outline, 32., 78.),
                app.dpi,
            )
        };
        unsafe {
            replay_deferred_paints(&fixture.state);
            let _ = UpdateWindow(fixture.hwnd);
            let before = document_state(fixture.state.app.borrow().doc());
            let before_frame = fixture
                .state
                .app
                .borrow()
                .painted_frame
                .expect("initial fixture frame");
            let mut pending = RECT::default();
            assert!(
                !GetUpdateRect(fixture.hwnd, Some(&mut pending), false).as_bool(),
                "fixture baseline must start without unrelated Main damage"
            );
            assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
            PAINT_TEST.with(|state| *state.borrow_mut() = PaintTestState::default());
            let merged = RECT {
                left: title.left.min(outline.left),
                top: title.top.min(outline.top),
                right: title.right.max(outline.right),
                bottom: title.bottom.max(outline.bottom),
            };
            {
                let app = fixture.state.app.borrow_mut();
                for region in [title, outline] {
                    assert!(InvalidateRect(Some(fixture.hwnd), Some(&region), false).as_bool());
                    let _ = UpdateWindow(fixture.hwnd);
                    assert_eq!(
                        app.painted_frame,
                        Some(before_frame),
                        "borrowed BeginPaint/EndPaint cannot publish a frame"
                    );
                    assert!(
                        !GetUpdateRect(fixture.hwnd, Some(&mut pending), false).as_bool(),
                        "borrowed delivery validates native damage"
                    );
                }
                assert_eq!(
                    fixture.state.deferred_paints.get(),
                    [Some((fixture.hwnd, merged)), None],
                    "actual borrowed WM_PAINT retains the union of both damaged domains"
                );
                PAINT_TEST.with(|state| {
                    assert_eq!(
                        state.borrow().began,
                        0,
                        "borrowed route never starts D2D drawing"
                    )
                });
                assert_eq!(document_state(app.doc()), before);
            }
            replay_deferred_paints(&fixture.state);
            assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
            assert!(GetUpdateRect(fixture.hwnd, Some(&mut pending), false).as_bool());
            assert_eq!(
                pending, merged,
                "replay restores precisely the retained native damage"
            );
            assert_eq!(
                fixture.state.app.borrow().painted_frame,
                Some(before_frame),
                "replay schedules ordinary painting without drawing synchronously"
            );
            let _ = UpdateWindow(fixture.hwnd);
            let app = fixture.state.app.borrow();
            let after = app.painted_frame.expect("ordinary replay frame");
            assert!(after.sequence > before_frame.sequence);
            assert_eq!(
                (after.hwnd, after.document_id, after.revision),
                (
                    fixture.hwnd.0 as usize,
                    before_frame.document_id,
                    before_frame.revision
                )
            );
            PAINT_TEST.with(|state| {
                let state = state.borrow();
                let damage = state.damage.expect("ordinary D2D damage binding");
                assert!(state.began > 0 && state.ended > 0);
                assert!(
                    damage.left <= merged.left as f32 / dpi
                        && damage.top <= merged.top as f32 / dpi
                        && damage.right >= merged.right as f32 / dpi
                        && damage.bottom >= merged.bottom as f32 / dpi,
                    "ordinary paint must cover both retained regions"
                );
            });
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(GetForegroundWindow(), foreground);
            eprintln!(
                "borrowed UpdateWindow title={title:?} outline={outline:?} retained={merged:?}; frame {} -> {}; ordinary damage replay covered both regions; screen pixels NOT_ASSERTED",
                before_frame.sequence, after.sequence
            );
        }
    }
    #[test]
    fn deferred_parent_paint_merges_owned_surfaces_and_preserves_editor_state() {
        let fixture = EditorFixture::new("body", false);
        let compact = hidden_test_window_size(640, 480);
        let foreign = hidden_test_window_size(640, 480);
        fixture.state.main_window.set(fixture.hwnd);
        fixture.state.editor_window.set(compact);
        fixture.state.compact_window.set(compact);
        {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.main_window = fixture.hwnd;
            app.shell.compact_window = compact;
        }
        let pointer = (&*fixture.state as *const WindowState) as isize;
        unsafe {
            SetWindowLongPtrW(compact, GWLP_USERDATA, pointer);
            assert!(fixture.state.owns_paint_surface(fixture.hwnd));
            assert!(fixture.state.owns_paint_surface(compact));
            assert!(!fixture.state.owns_paint_surface(foreign));
        }
        let before = document_state(fixture.state.app.borrow().doc());
        let first = RECT {
            left: 4,
            top: 8,
            right: 24,
            bottom: 30,
        };
        let second = RECT {
            left: 10,
            top: 3,
            right: 40,
            bottom: 20,
        };
        fixture.state.defer_paint(fixture.hwnd, first);
        fixture.state.defer_paint(fixture.hwnd, second);
        fixture.state.defer_paint(compact, first);
        fixture.state.defer_paint(foreign, second);
        assert_eq!(
            fixture.state.deferred_paints.get(),
            [
                Some((
                    fixture.hwnd,
                    RECT {
                        left: 4,
                        top: 3,
                        right: 40,
                        bottom: 30
                    }
                )),
                Some((compact, first)),
            ]
        );
        {
            let _borrow = fixture.state.app.borrow_mut();
            fixture.message(WM_NULL, 0);
            assert!(
                fixture
                    .state
                    .deferred_paints
                    .get()
                    .iter()
                    .all(Option::is_some)
            );
        }
        fixture.message(WM_NULL, 0);
        assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
        assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
        unsafe {
            replay_deferred_paints(&fixture.state);
            assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
            SetWindowLongPtrW(compact, GWLP_USERDATA, 0);
            DestroyWindow(compact).unwrap();
            DestroyWindow(foreign).unwrap();
        }
    }
    #[test]
    fn deferred_parent_paint_owns_inactive_compact_from_native_creation() {
        struct Compact(HWND, HINSTANCE);
        impl Drop for Compact {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                    let _ = UnregisterClassW(w!("MDLiteDeferredPaintOwnershipTest"), Some(self.1));
                }
            }
        }
        let fixture = EditorFixture::new("body", false);
        fixture.state.main_window.set(fixture.hwnd);
        fixture.state.editor_window.set(fixture.hwnd);
        let before = document_state(fixture.state.app.borrow().doc());
        unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: w!("MDLiteDeferredPaintOwnershipTest"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let compact = {
                let _borrow = fixture.state.app.borrow_mut();
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class.lpszClassName,
                    w!(""),
                    WS_POPUP,
                    0,
                    0,
                    640,
                    480,
                    Some(fixture.hwnd),
                    None,
                    Some(instance),
                    Some((&*fixture.state as *const WindowState).cast()),
                )
                .unwrap()
            };
            let owned_compact = Compact(compact, instance);
            // Creation happens under App's borrow, before the input owner can
            // switch. Both owned surfaces still need independent paint identity.
            assert_eq!(fixture.state.editor_window.get(), fixture.hwnd);
            assert_eq!(fixture.state.compact_window.get(), compact);
            assert!(fixture.state.owns_paint_surface(compact));
            let region = RECT {
                left: 2,
                top: 3,
                right: 40,
                bottom: 30,
            };
            for active in [fixture.hwnd, compact] {
                {
                    let mut app = fixture.state.app.borrow_mut();
                    app.shell.main_window = fixture.hwnd;
                    app.shell.compact_window = if active == compact {
                        compact
                    } else {
                        HWND::default()
                    };
                    assert_eq!(app.editing_window(), active);
                }
                fixture.state.editor_window.set(active);
                fixture.state.defer_paint(fixture.hwnd, region);
                fixture.state.defer_paint(compact, region);
                assert_eq!(
                    fixture.state.deferred_paints.get(),
                    [Some((fixture.hwnd, region)), Some((compact, region)),]
                );
                replay_deferred_paints(&fixture.state);
                assert_eq!(fixture.state.editor_window.get(), active);
                assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
                assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
            }
            fixture.state.app.borrow_mut().shell.compact_window = HWND::default();
            fixture.state.editor_window.set(fixture.hwnd);
            fixture.state.defer_paint(fixture.hwnd, region);
            fixture.state.defer_paint(compact, region);
            {
                let _borrow = fixture.state.app.borrow_mut();
                drop(owned_compact);
            }
            assert_eq!(fixture.state.compact_window.get(), HWND::default());
            assert_eq!(
                fixture.state.deferred_paints.get(),
                [Some((fixture.hwnd, region)), None]
            );
            replay_deferred_paints(&fixture.state);
            assert_eq!(fixture.state.editor_window.get(), fixture.hwnd);
            assert!(!fixture.state.owns_paint_surface(compact));
            assert_eq!(document_state(fixture.state.app.borrow().doc()), before);
        }
    }
    #[test]
    fn deferred_parent_paint_does_not_replay_to_retired_or_foreign_userdata() {
        let fixture = EditorFixture::new("body", false);
        let compact = hidden_test_window_size(640, 480);
        fixture.state.main_window.set(fixture.hwnd);
        fixture.state.editor_window.set(compact);
        fixture.state.compact_window.set(compact);
        let region = RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 30,
        };
        fixture.state.defer_paint(compact, region);
        // A HWND without this WindowState's userdata is not an owned surface.
        unsafe {
            assert!(!fixture.state.owns_paint_surface(compact));
            replay_deferred_paints(&fixture.state);
        }
        assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
        unsafe {
            SetWindowLongPtrW(
                compact,
                GWLP_USERDATA,
                (&*fixture.state as *const WindowState) as isize,
            );
            assert!(fixture.state.owns_paint_surface(compact));
        }
        fixture.state.defer_paint(compact, region);
        fixture.state.compact_window.set(HWND::default());
        unsafe {
            assert!(!fixture.state.owns_paint_surface(compact));
            replay_deferred_paints(&fixture.state);
            SetWindowLongPtrW(compact, GWLP_USERDATA, 0);
            DestroyWindow(compact).unwrap();
            assert!(!fixture.state.owns_paint_surface(compact));
        }
        assert_eq!(fixture.state.deferred_paints.get(), [None, None]);
    }
    #[test]
    fn editor_system_close_posts_normal_close_outside_app_borrow() {
        unsafe extern "system" fn close_probe(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
        ) -> LRESULT {
            if msg == WM_CLOSE {
                // Avoid a dialog/destruction in this hidden delivery probe.
                return LRESULT(0);
            }
            unsafe { window_proc(hwnd, msg, wp, lp) }
        }
        let fixture = EditorFixture::new("body", false);
        unsafe {
            let original = SetWindowLongPtrW(
                fixture.hwnd,
                GWLP_WNDPROC,
                close_probe as *const () as isize,
            );
            fixture.message(WM_SYSCOMMAND, SC_CLOSE as usize);
            let mut message = MSG::default();
            let posted = PeekMessageW(
                &mut message,
                Some(fixture.hwnd),
                WM_CLOSE,
                WM_CLOSE,
                PM_REMOVE,
            )
            .as_bool();
            SetWindowLongPtrW(fixture.hwnd, GWLP_WNDPROC, original);
            assert!(
                posted,
                "system Close must reach the ordinary unsaved/IME close gate after the current borrow"
            );
            assert_eq!(fixture.state.app.borrow().doc().source(), "body");
        }
    }
    #[test]
    fn editor_close_waits_for_result_and_real_end_without_discarding_unsaved_source() {
        let fixture = EditorFixture::new("unsaved body", false);
        let before = fixture.state.app.borrow().doc().source().to_owned();
        {
            let mut app = fixture.state.app.borrow_mut();
            app.start_composition();
            app.preedit = "未確定".into();
            app.close_after_composition = Some((WM_CLOSE, WPARAM(0), LPARAM(0)));
            fixture.state.notifications.borrow_mut().push_back((
                app.ime_session.unwrap(),
                NativeNotification::Update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("確定".into()),
                    ..Default::default()
                }),
            ));
            fixture.message(WM_IME_ENDCOMPOSITION, 0);
            assert_eq!(app.doc().source(), before);
            assert!(app.composing);
        }
        fixture.message(WM_NULL, 0);
        let app = fixture.state.app.borrow();
        assert_eq!(app.doc().source(), format!("確定{before}"));
        assert!(!app.composing);
        assert!(app.doc().is_dirty());
        assert_eq!(app.doc().undo_count(), 1);
        assert_eq!(app.close_after_composition, None);
        drop(app);
        // Only inspect the owned window queue; do not dispatch the close to an
        // unsaved-prompt dialog in a hidden fixture. Visible acceptance owns it.
        let mut message = MSG::default();
        unsafe {
            assert!(
                PeekMessageW(
                    &mut message,
                    Some(fixture.hwnd),
                    WM_CLOSE,
                    WM_CLOSE,
                    PM_REMOVE
                )
                .as_bool()
            );
        }
        assert_eq!(message.message, WM_CLOSE);
        assert!(unsafe { IsWindow(Some(fixture.hwnd)).as_bool() });
    }
    #[test]
    fn editor_result_next_preedit_empty_update_duplicate_and_normal_same_character() {
        let fixture = EditorFixture::new("a日本語b", false);
        let mut app = fixture.state.app.borrow_mut();
        app.doc_mut().set_selection(1, 10).unwrap();
        app.start_composition();
        let before_undo = app.doc().undo_count();
        for _ in 0..2 {
            unsafe {
                app.apply_ime_update(ImeUpdate {
                    flags: GCS_RESULTSTR.0 | GCS_COMPSTR.0 | GCS_CURSORPOS.0,
                    result: Some("確".into()),
                    preedit: Some("次".into()),
                    cursor: Some(1),
                    ..Default::default()
                });
            }
        }
        assert_eq!(app.doc().source(), "a確b");
        assert_eq!(app.doc().undo_count(), before_undo + 1);
        assert!(app.composing);
        assert_eq!(app.preedit, "次");
        unsafe {
            app.apply_ime_update(ImeUpdate {
                flags: GCS_COMPSTR.0,
                preedit: Some(String::new()),
                ..Default::default()
            });
        }
        assert!(app.composing, "empty preedit is not an end notification");
        app.clear_composition();
        unsafe {
            app.character('確' as u16);
        }
        assert_eq!(
            app.doc().source(),
            "a確確b",
            "normal typing equal to the commit is not swallowed"
        );
        unsafe {
            app.key_with_modifiers(0x5a, true, false);
        }
        assert_eq!(app.doc().source(), "a確b");
        unsafe {
            app.key_with_modifiers(0x5a, true, false);
        }
        assert_eq!(app.doc().source(), "a日本語b");
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: 1,
                head: 10
            }
        );
    }
    #[test]
    fn editor_live_authoring_heading_delimiters_enter_undo_and_followup() {
        let fixture = EditorFixture::new("", false);
        for unit in "## asdkl".encode_utf16() {
            fixture.message(WM_CHAR, unit as usize);
        }
        {
            let app = fixture.state.app.borrow();
            assert_eq!(app.doc().source(), "## asdkl");
            assert_eq!(app.doc().selection(), Selection::caret(8));
            assert_eq!(app.doc().undo_count(), 1);
            assert!(
                app.projection_cache
                    .as_ref()
                    .unwrap()
                    .2
                    .blocks
                    .iter()
                    .any(|block| matches!(block.kind, BlockKind::Heading { level: 2 }))
            );
        }
        fixture.message(WM_KEYDOWN, VK_RETURN.0 as usize);
        fixture.message(WM_KEYDOWN, VK_RETURN.0 as usize);
        for unit in "**日本語**".encode_utf16() {
            fixture.message(WM_CHAR, unit as usize);
        }
        let mut app = fixture.state.app.borrow_mut();
        let expected = "## asdkl\n\n**日本語**";
        assert_eq!(app.doc().source(), expected);
        assert_eq!(app.doc().selection(), Selection::caret(expected.len()));
        let selection = app.doc().selection();
        unsafe {
            app.key_with_modifiers(0x5a, true, false);
            app.key_with_modifiers(0x59, true, false);
        }
        assert_eq!(app.doc().source(), expected);
        assert_eq!(app.doc().selection(), selection);
        unsafe {
            app.character('続' as u16);
        }
        assert_eq!(app.doc().source(), format!("{expected}続"));
    }
    #[test]
    fn live_heading_end_before_affinity_keeps_owning_view_and_direct_pointer_hit() {
        let source = "# Heading 日本語\n\n---\n\n```rust\n# literal\n\n---\n```\n\nSetext\n---\n";
        let fixture = EditorFixture::new(source, false);
        let mut app = fixture.state.app.borrow_mut();
        let end = source.find('\n').unwrap();
        assert_eq!(end, 19);
        unsafe { app.key_with_modifiers(VK_END.0, false, false) };
        assert_eq!(app.doc().selection(), Selection::caret(end));
        assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::Before);
        let heading = app
            .views
            .iter()
            .find(|view| view.source == (0..20))
            .unwrap();
        let rule = app.views.iter().find(|view| view.thematic_break).unwrap();
        assert!(heading.before.contains(&end));
        assert!(
            rule.before.contains(&end),
            "the global shared boundary is a legitimate Before choice"
        );
        assert!(!rule.source.contains(&end));
        println!(
            "heading end={end} chosen={:?} rule_before={:?}",
            app.source_view(end).unwrap().source,
            rule.before
        );
        assert_eq!(app.source_view(end).unwrap().source, heading.source);
        let (x, y, height) = app.caret_location();
        assert!(y >= heading.y && y < heading.y + heading.height);
        // Match the registered harness's rounded native-caret pixel point.
        let click_x = x.round();
        let click_y = y.round() + (height.round() / 2.0).floor();
        let before = document_state(app.doc());
        app.select_at(click_x, click_y, false);
        assert_eq!(app.doc().selection(), Selection::caret(end));
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().revision(), before.2);
        assert_eq!(app.doc().undo_count(), before.4);
        assert_eq!(app.doc().redo_count(), before.5);
        assert_eq!(app.doc().is_dirty(), before.3);
    }
    #[test]
    fn live_heading_direct_hit_reveal_and_scroll_refresh_keep_caret_geometry() {
        for level in 1..=6 {
            for title in [
                "",
                "日本語 👩‍💻 é heading",
                "wrapped 日本語 ".repeat(18).as_str(),
            ] {
                let source = format!("before\n\n{} {title}\n\nafter\n", "#".repeat(level));
                let mut app = composing_test_app(&source);
                app.clear_composition();
                app.tabs[0].scroll = 0.0;
                app.hwnd = hidden_test_window_size(450, 480);
                unsafe { app.ensure_layout().unwrap() };
                let heading = app
                    .cached_projection()
                    .blocks
                    .iter()
                    .find(|block| matches!(block.kind, BlockKind::Heading { .. }))
                    .unwrap()
                    .clone();
                // Probe the actual display hit-test before raw heading reveal,
                // then follow selection -> layout -> caret -> input. No focus
                // or activation API is used by this hidden geometry regression.
                let view = app
                    .views
                    .iter()
                    .find(|view| view.source == heading.source)
                    .unwrap();
                let mut points = Vec::new();
                for u in [0, view.text.len() / 2, view.text.len()] {
                    if let Some((x, y, h)) = layout_caret(&view.layout, u as u32, false) {
                        points.push((view.x + x + 0.1, view.y + y + h * 0.5));
                    }
                }
                for (x, y) in points {
                    app.doc_mut().set_selection(0, 0).unwrap();
                    unsafe { app.ensure_layout().unwrap() };
                    app.select_at(x, y, false);
                    let p = app.doc().selection().head;
                    assert!(
                        heading.source.contains(&p) || p == heading.source.end,
                        "level={level} title={title:?} p={p}"
                    );
                    unsafe { app.refresh() };
                    assert_eq!(app.caret, app.caret_location());
                    let (cx, cy, h) = app.caret_location();
                    assert!(cx.is_finite() && cy.is_finite() && h > 0.0);
                    assert!(app.source_view(p).is_some());
                    for inserted in ["A", "字"] {
                        let before = document_state(app.doc());
                        app.edit(inserted);
                        let mut expected = source.clone();
                        expected.insert_str(p, inserted);
                        assert_eq!(app.doc().source(), expected);
                        assert!(app.doc_mut().undo());
                        assert_eq!(app.doc().source(), source);
                        assert_eq!(app.doc().selection(), Selection::caret(p));
                        assert_eq!(app.doc().undo_count(), before.4);
                        unsafe { app.ensure_layout().unwrap() };
                    }
                }
                app.tabs[0].scroll = app.height;
                unsafe { app.refresh() };
                assert_eq!(
                    app.caret,
                    app.caret_location(),
                    "refresh must commit caret after scroll clamp: level={level} title={title:?}"
                );
                unsafe { DestroyWindow(app.hwnd).unwrap() };
            }
        }
    }
    #[test]
    fn editor_live_front_matter_vertical_sequence_reaches_both_edges_without_cycle() {
        let source = "---\ntitle: 架空の編集試料\n---\n\n# MDLite 編集基盤\n日本語と **太字**、*強調*、`code`、[リンク](#表)を編集します。\n絵文字 👩‍💻 と結合文字 e\u{301} の境界を保持します\n";
        assert_vertical_sequence(source);
        assert_vertical_sequence(&source.replace('\n', "\r\n"));
    }
    #[test]
    fn live_delimiter_reveal_follows_caret_and_reconstructed_block_revision() {
        for eol in ["\n", "\r\n"] {
            for (opening, body, closing) in [
                ("---", "title: value", "---"),
                ("```rust", "let value = 1;", "```"),
                ("```", "body 👩‍💻 é", "```"),
            ] {
                let source = format!("{opening}{eol}{body}{eol}{closing}{eol}{eol}tail");
                let mut app = composing_test_app(&source);
                app.clear_composition();
                app.hwnd = hidden_test_window_size(640, 480);
                let end = source.rfind(closing).unwrap();
                let before = document_state(app.doc());
                for position in [0, opening.len() + eol.len(), end, source.len(), end] {
                    app.doc_mut().set_selection(position, position).unwrap();
                    unsafe { app.ensure_layout().unwrap() };
                    let active = app.cached_projection().source_delimiter_line_at(position);
                    if let Some(line) = active {
                        assert_eq!(app.revealed_source.as_ref(), Some(&line));
                        let view = app.views.iter().find(|view| view.source == line).unwrap();
                        assert_eq!(
                            String::from_utf16_lossy(&view.text),
                            source[line].trim_end_matches(['\r', '\n'])
                        );
                        let (x, y, height) = app.caret_location();
                        assert_eq!(app.hit(x + 0.1, y + height * 0.5), position);
                    }
                }
                assert_eq!(app.doc().revision(), before.2);
                assert_eq!(app.doc().undo_count(), before.4);
                assert!(!app.doc().is_dirty());
                app.edit_range(end..end + closing.len(), "");
                unsafe { app.ensure_layout().unwrap() };
                assert!(app.doc_mut().undo());
                app.doc_mut().set_selection(end + 1, end + 1).unwrap();
                unsafe { app.ensure_layout().unwrap() };
                app.edit("X");
                let mut expected = source.clone();
                expected.insert(end + 1, 'X');
                assert_eq!(app.doc().source(), expected);
                assert!(app.doc_mut().undo());
                assert_eq!(app.doc().source(), source);
                unsafe { DestroyWindow(app.hwnd).unwrap() };
            }
        }
    }
    #[test]
    fn unclosed_fence_keeps_quote_and_list_looking_body_lines_and_tail_visible() {
        for body in ["> ```\ntail", "- ```\ntail", "  - ```\ntail"] {
            let source = format!("```\n{body}");
            let mut app = composing_test_app(&source);
            app.clear_composition();
            let tail = source.find("tail").unwrap();
            app.doc_mut().set_selection(tail, tail).unwrap();
            app.hwnd = hidden_test_window_size(640, 480);
            let before = document_state(app.doc());
            unsafe { app.ensure_layout().unwrap() };
            assert!(
                app.views
                    .iter()
                    .any(|view| String::from_utf16_lossy(&view.text) == body),
                "literal body was truncated: {source:?}"
            );
            let (x, y, height) = app.caret_location();
            assert_eq!(app.hit(x + 0.1, y + height * 0.5), tail);
            assert_eq!(document_state(app.doc()), before);
            unsafe { DestroyWindow(app.hwnd).unwrap() };
        }
    }
    #[test]
    fn front_matter_body_has_one_row_and_terminal_mapping_before_its_own_eol() {
        for eol in ["\n", "\r\n"] {
            let source = format!("---{eol}title: value{eol}---{eol}{eol}following");
            let mut app = composing_test_app(&source);
            app.clear_composition();
            app.hwnd = hidden_test_window_size(640, 480);
            let before = document_state(app.doc());
            unsafe { app.ensure_layout().unwrap() };
            let start = 3 + eol.len();
            let end = start + "title: value".len();
            let body = app
                .views
                .iter()
                .find(|view| String::from_utf16_lossy(&view.text) == "title: value")
                .unwrap();
            assert_eq!(body.source, start..end);
            assert!(
                body.map
                    .iter()
                    .chain(&body.before)
                    .all(|&byte| (start..=end).contains(&byte))
            );
            assert_eq!(body.map.last(), Some(&end));
            assert_eq!(body.before.last(), Some(&end));
            assert!((body.height - presentation::line_height(app.font_size())).abs() < 0.1);
            let closing = end + eol.len();
            let row_height = presentation::line_height(app.font_size());
            let opening_y = app
                .source_view_at(0, Some(BoundaryAffinity::After))
                .unwrap()
                .y;
            let closing_y = app
                .source_view_at(closing, Some(BoundaryAffinity::After))
                .unwrap()
                .y;
            assert!((closing_y - opening_y - 2. * row_height).abs() < 0.1);
            for byte in start..=end {
                let (x, y, height) = app.caret_location_at(byte, BoundaryAffinity::After);
                assert_eq!(app.hit(x + 0.1, y + height * 0.5), byte);
            }
            assert_eq!(document_state(app.doc()), before);
            unsafe { DestroyWindow(app.hwnd).unwrap() };
        }
    }
    #[test]
    fn editor_live_front_matter_delimiters_edit_and_undo_through_native_input() {
        for eol in ["\n", "\r\n"] {
            let source = format!("---{eol}title: 日本語 👩‍💻{eol}---{eol}{eol}# body{eol}");
            for p in [0, source.rfind("---").unwrap()] {
                let fixture = EditorFixture::new(&source, false);
                {
                    let mut app = fixture.state.app.borrow_mut();
                    app.doc_mut().set_selection(p, p).unwrap();
                    unsafe {
                        app.after_input();
                    }
                    let rows: Vec<_> = app.gutter_rows.iter().map(|r| r.line_number).collect();
                    assert_eq!(rows, vec![1, 2, 3, 4, 5, 6]);
                    let (x, y, height) = app.caret_location();
                    let (hit, _) = app.hit_with_affinity(x + 0.5, y + height * 0.5);
                    assert_eq!(hit, p, "delimiter hit-test at {p}");
                }
                fixture.message(WM_KEYDOWN, VK_DELETE.0 as usize);
                {
                    let mut app = fixture.state.app.borrow_mut();
                    let mut edited = source.clone();
                    edited.remove(p);
                    assert_eq!(app.doc().source(), edited);
                    assert_eq!(app.doc().undo_count(), 1);
                    assert!(app.doc().is_dirty());
                    unsafe {
                        app.key_with_modifiers(0x5a, true, false);
                    }
                    assert_eq!(app.doc().source(), source);
                    assert_eq!(app.doc().selection(), Selection::caret(p));
                    assert!(!app.doc().is_dirty());
                    unsafe {
                        app.key_with_modifiers(0x59, true, false);
                    }
                    assert_eq!(app.doc().source(), edited);
                    unsafe {
                        app.key_with_modifiers(0x5a, true, false);
                    }
                }
                fixture.message(WM_CHAR, '字' as usize);
                let app = fixture.state.app.borrow();
                let mut edited = source.clone();
                edited.insert(p, '字');
                assert_eq!(app.doc().source(), edited);
                assert_eq!(app.doc().selection().head, p + '字'.len_utf8());
            }
        }
    }
    #[test]
    fn editor_live_markdown_vertical_sequence_keeps_selection_and_edges() {
        let source = "# Heading\n\nparagraph **bold** and *italic*, `code` and [link](url)\nnext 👩‍💻 e\u{301}\n\n- item\n- second\n\n> quote\n> next\n\n```rust\nlet n = 1;\n```\n\n| H | J |\n|---|---|\n| one | 日本語 |\n| two | next |\n\n![image](missing.png)\n\nlast\n";
        assert_vertical_sequence(source);
        assert_vertical_sequence(&source.replace('\n', "\r\n"));
    }
    fn assert_vertical_sequence(source: &str) {
        let fixture = EditorFixture::new(source, false);
        let mut app = fixture.state.app.borrow_mut();
        let before = (
            app.doc().revision(),
            app.doc().undo_count(),
            app.doc().redo_count(),
            app.doc().is_dirty(),
        );
        app.doc_mut()
            .set_selection(source.len(), source.len())
            .unwrap();
        unsafe {
            app.after_input();
        }
        for (key, edge, extend) in [
            (VK_UP.0, 0, false),
            (VK_DOWN.0, source.len(), false),
            (VK_UP.0, 0, true),
        ] {
            let anchor = app.doc().selection().head;
            let mut visited = std::collections::HashSet::new();
            let mut trace = Vec::new();
            while app.doc().selection().head != edge {
                let p = app.doc().selection().head;
                let affinity = app.doc().selection_affinity();
                let y = app.caret_location().1 + app.tabs[app.active].scroll;
                let layout_before = app.layout_key;
                let revealed_before = app.revealed_source.clone();
                trace.push((p, format!("{affinity:?}"), y));
                assert!(
                    visited.insert((p, format!("{affinity:?}"))),
                    "vertical cycle key={key}: {trace:?}"
                );
                app.move_caret(key, extend);
                let target = app.doc().selection().head;
                let target_affinity = app.doc().selection_affinity();
                assert_eq!(app.layout_key, layout_before);
                assert_eq!(app.revealed_source, revealed_before);
                let chosen_y =
                    app.caret_location_at(target, target_affinity).1 + app.tabs[app.active].scroll;
                assert!(
                    if key == VK_UP.0 {
                        chosen_y < y || (target == edge && chosen_y <= y)
                    } else {
                        chosen_y > y || (target == edge && chosen_y >= y)
                    },
                    "same-layout visual direction key={key}: ({p}, {affinity:?}, {y}) -> ({target}, {target_affinity:?}, {chosen_y})"
                );
                unsafe {
                    app.after_input();
                }
                if extend {
                    assert_eq!(app.doc().selection().anchor, anchor);
                }
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(
                (
                    app.doc().revision(),
                    app.doc().undo_count(),
                    app.doc().redo_count(),
                    app.doc().is_dirty()
                ),
                before
            );
        }
    }
    #[test]
    fn editor_live_vertical_marker_edges_keep_horizontal_affinity() {
        let fixture = EditorFixture::new("---\nvalue: text\n---\n\n# body\n", false);
        let mut app = fixture.state.app.borrow_mut();
        app.doc_mut()
            .set_selection_with_affinity(0, 0, BoundaryAffinity::Before)
            .unwrap();
        unsafe {
            app.after_input();
        }
        let before = (app.doc().selection_affinity(), app.caret_location());
        app.move_caret(VK_LEFT.0, false);
        unsafe {
            app.after_input();
        }
        assert_eq!(
            (app.doc().selection_affinity(), app.caret_location()),
            before
        );
    }
    #[test]
    fn editor_live_table_vertical_movement_retains_second_column_and_shift_anchor() {
        let source = "| A | B |\n|---|---|\n| one | two |\n| three | four |\n";
        let fixture = EditorFixture::new(source, false);
        let mut app = fixture.state.app.borrow_mut();
        let two = source.find("two").unwrap();
        let four = source.find("four").unwrap();
        let before = (
            app.doc().revision(),
            app.doc().is_dirty(),
            app.doc().undo_count(),
            app.doc().redo_count(),
        );
        app.doc_mut().set_selection(two, two).unwrap();
        unsafe {
            app.after_input();
        }
        let preferred_x = app.caret_location().0;
        for (key, target, extend) in [
            (VK_DOWN.0, four, false),
            (VK_UP.0, two, false),
            (VK_DOWN.0, four, true),
            (VK_UP.0, two, true),
        ] {
            let y = app.caret_location().1 + app.tabs[app.active].scroll;
            app.move_caret(key, extend);
            unsafe {
                app.after_input();
            }
            assert_eq!(
                app.doc().selection().head,
                target,
                "key={key}, extend={extend}"
            );
            if extend {
                assert_eq!(app.doc().selection().anchor, two);
            }
            assert!((app.caret_location().0 - preferred_x).abs() < 1.0);
            let target_y = app.caret_location().1 + app.tabs[app.active].scroll;
            assert!(if key == VK_UP.0 {
                target_y < y
            } else {
                target_y > y
            });
        }
        assert_eq!(app.doc().source(), source);
        assert_eq!(
            (
                app.doc().revision(),
                app.doc().is_dirty(),
                app.doc().undo_count(),
                app.doc().redo_count()
            ),
            before
        );
    }
    #[test]
    fn editor_visual_rows_keep_preferred_x_across_short_line() {
        let fixture = EditorFixture::new("abcdefghijk\nx\nabcdefghijk", true);
        let mut app = fixture.state.app.borrow_mut();
        app.doc_mut().set_selection(8, 8).unwrap();
        let x = app.caret_location().0;
        app.move_caret(VK_DOWN.0, false);
        assert_eq!(app.doc().selection().head, 13);
        app.move_caret(VK_DOWN.0, false);
        assert!((app.caret_location().0 - x).abs() < 1.0);
        assert_eq!(app.doc().selection().head, 22);
        app.move_caret(VK_UP.0, true);
        assert_eq!(app.doc().selection().anchor, 22);
        app.move_caret(VK_UP.0, true);
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: 22,
                head: 8
            }
        );
    }
    #[test]
    fn editor_wrapped_end_keeps_trailing_row_and_undo_visual_side() {
        for raw in [false, true] {
            let fixture = EditorFixture::new(&"word ".repeat(45), raw);
            fixture.message(WM_KEYDOWN, VK_END.0 as usize);
            let mut app = fixture.state.app.borrow_mut();
            let view = &app.views[0];
            let mut count = 0;
            let mut lines = vec![DWRITE_LINE_METRICS::default(); 100];
            unsafe {
                view.layout
                    .GetLineMetrics(Some(&mut lines), &mut count)
                    .unwrap();
            }
            assert!(count > 1);
            assert_eq!(app.doc().selection().head, lines[0].length as usize);
            let (_, y, _) = app.caret_location();
            assert!(
                y < view.y + lines[0].height,
                "End source {} drawn at y={y}, next row begins {}",
                app.doc().selection().head,
                view.y + lines[0].height
            );
            let before = app.caret_location();
            let selection = app.doc().selection();
            unsafe {
                app.character('x' as u16);
                app.key_with_modifiers(0x5a, true, false);
            }
            assert_eq!(app.doc().selection(), selection);
            assert_eq!(app.caret_location(), before);
            app.move_caret(VK_DOWN.0, false);
            assert!(app.caret_location().1 > before.1);
            app.move_caret(VK_UP.0, false);
            assert_eq!(app.caret_location(), before);
            app.start_composition();
            assert_eq!(app.composition_position(0).unwrap(), before);
            app.doc_mut()
                .set_selection_with_affinity(
                    selection.anchor,
                    selection.head,
                    BoundaryAffinity::After,
                )
                .unwrap();
            assert!(
                app.composition_position(0).is_none(),
                "same offset with a different side is a stale IME owner"
            );
            app.clear_composition();
            app.select_at(10000.0, TOP + FONT * 0.5, false);
            assert_eq!(app.doc().selection().head, lines[0].length as usize);
            assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::Before);
            assert_eq!(app.caret_location(), before);
        }
    }
    #[test]
    fn editor_enter_continuation_exit_code_indent_and_shift_tab() {
        for (source, literal, expected) in [
            ("- item", false, "- item\n- "),
            ("9. item", false, "9. item\n10. "),
            ("> - [x] item", false, "> - [x] item\n> - [ ] "),
            ("> quoted", false, "> quoted\n> "),
            ("- ", false, ""),
            ("> ", false, ""),
            ("    let x = 1;", true, "    let x = 1;\n    "),
        ] {
            let mut doc = Document::new(1, source.into());
            doc.set_selection(source.len(), source.len()).unwrap();
            let plan = enter_edit(source, doc.selection(), literal);
            apply_plan(&mut doc, plan);
            assert_eq!(doc.source(), expected);
            assert!(doc.undo());
            assert_eq!(doc.source(), source);
        }
        let fixture = EditorFixture::new("    first\n    次", false);
        let mut app = fixture.state.app.borrow_mut();
        let end = app.doc().byte_len();
        app.doc_mut().set_selection(end, 0).unwrap();
        unsafe {
            app.key_with_modifiers(VK_TAB.0, false, true);
        }
        assert_eq!(app.doc().source(), "first\n次");
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: "first\n次".len(),
                head: 0
            }
        );
    }
    #[test]
    fn editor_transient_layout_preserves_heading_baseline_styles_and_suffix_reflow() {
        let fixture = EditorFixture::new("## abc 日本語 suffix", false);
        let mut app = fixture.state.app.borrow_mut();
        let p = "## abc ".len();
        app.doc_mut().set_selection(p, p).unwrap();
        app.start_composition();
        app.preedit = "長い変換文字列".repeat(10);
        app.ime_cursor = 2;
        let visual = app.composition_visual().unwrap();
        let view = app.caret_view().unwrap();
        let mut source_size = 0.0;
        let mut transient_size = 0.0;
        unsafe {
            view.layout
                .GetFontSize(p as u32, &mut source_size, None)
                .unwrap();
            visual
                .layout
                .GetFontSize(visual.range.start, &mut transient_size, None)
                .unwrap();
        }
        assert_eq!(source_size, transient_size);
        assert!(source_size > FONT);
        assert!(visual.height > view.height);
        let position = app.composition_position(app.ime_cursor).unwrap();
        let mut x = 0.0;
        let mut y = 0.0;
        let mut hit = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            visual
                .layout
                .HitTestTextPosition(
                    visual.range.start + app.ime_cursor,
                    false,
                    &mut x,
                    &mut y,
                    &mut hit,
                )
                .unwrap();
        }
        assert_eq!(position.0, visual.x + x);
        assert_eq!(position.1, visual.y + y);
        assert!(position.2 >= hit.height);
        assert_eq!(app.doc().source(), "## abc 日本語 suffix");
        assert_eq!(
            composition_attribute_ranges(
                &[
                    ATTR_INPUT as u8,
                    ATTR_TARGET_CONVERTED as u8,
                    ATTR_TARGET_CONVERTED as u8
                ],
                7..10
            ),
            vec![(7..8, false), (8..10, true)]
        );
    }
    #[test]
    fn editor_transient_font_and_baseline_match_paragraph_emphasis_code_and_table() {
        for source in [
            "plain 日本語 tail",
            "## head 日本語",
            "**bold 日本語**",
            "*italic 日本語*",
            "`code 日本語`",
            "```rust\n日本語\n```",
            "| H |\n|---|\n| 日本語 tail |",
        ] {
            let fixture = EditorFixture::new(source, false);
            let mut app = fixture.state.app.borrow_mut();
            let p = source.find("日本語").unwrap();
            app.doc_mut().set_selection(p, p).unwrap();
            unsafe {
                app.after_input();
            }
            let committed = app.caret_location();
            app.start_composition();
            app.preedit = "かな😀".into();
            let visual = app.composition_visual().unwrap();
            let view = app.caret_view().unwrap();
            let original = view.map.partition_point(|&byte| byte < p) as u32;
            unsafe {
                let mut old_weight = DWRITE_FONT_WEIGHT::default();
                let mut new_weight = DWRITE_FONT_WEIGHT::default();
                view.layout
                    .GetFontWeight(original, &mut old_weight, None)
                    .unwrap();
                visual
                    .layout
                    .GetFontWeight(visual.range.start, &mut new_weight, None)
                    .unwrap();
                assert_eq!(old_weight, new_weight, "font weight {source}");
                let mut old_style = DWRITE_FONT_STYLE::default();
                let mut new_style = DWRITE_FONT_STYLE::default();
                view.layout
                    .GetFontStyle(original, &mut old_style, None)
                    .unwrap();
                visual
                    .layout
                    .GetFontStyle(visual.range.start, &mut new_style, None)
                    .unwrap();
                assert_eq!(old_style, new_style, "font style {source}");
                let mut old_family = [0; 64];
                let mut new_family = [0; 64];
                view.layout
                    .GetFontFamilyName(original, &mut old_family, None)
                    .unwrap();
                visual
                    .layout
                    .GetFontFamilyName(visual.range.start, &mut new_family, None)
                    .unwrap();
                assert_eq!(old_family, new_family, "font family {source}");
            }
            let position = app.composition_position(0).unwrap();
            assert_eq!(position.1, committed.1, "baseline {source}");
            assert_eq!(position.2, committed.2, "line height {source}");
            assert_eq!(app.doc().source(), source);
        }
    }
    #[test]
    fn editor_transient_composition_after_affinity_uses_following_paragraph_view() {
        const SOURCE: &str = "alpha beta\n\n# Heading\n**bold**\n> quote\n> quoted next\n- item\n- continued item\n~~~text\ncode\n~~~\n日本語今日ハイメ編か";
        let fixture = EditorFixture::new(SOURCE, false);
        let mut app = fixture.state.app.borrow_mut();
        let boundary = SOURCE.find("日本語").unwrap();
        let selection = Selection {
            anchor: SOURCE.len(),
            head: boundary,
        };
        assert_eq!(boundary, 94);

        app.doc_mut()
            .set_selection_with_affinity(selection.anchor, selection.head, BoundaryAffinity::After)
            .unwrap();
        unsafe {
            app.after_input();
        }
        assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);

        let following_index = app
            .views
            .iter()
            .position(|view| !view.source.is_empty() && view.source == (boundary..SOURCE.len()))
            .unwrap();
        let following_source = app.views[following_index].source.clone();
        let following_y = app.views[following_index].y;
        assert_eq!(following_source, boundary..SOURCE.len());
        assert_eq!(following_source.len(), 30);
        let body = app
            .views
            .iter()
            .find(|view| String::from_utf16_lossy(&view.text) == "code")
            .unwrap();
        assert!(
            body.source.end < boundary,
            "hidden closing delimiter must not alias the following paragraph"
        );
        assert!(following_y >= body.y + body.height);

        let before_candidate = app
            .views
            .iter()
            .filter(|view| !view.source.is_empty() && view.before.contains(&boundary))
            .min_by_key(|view| view.source.len())
            .unwrap();
        let before_source = before_candidate.source.clone();
        let before_y = before_candidate.y;
        app.doc_mut()
            .set_selection_with_affinity(selection.anchor, selection.head, BoundaryAffinity::Before)
            .unwrap();
        let before_view = app.source_view(boundary).unwrap();
        assert_eq!(before_view.source, before_source);
        assert_eq!(before_view.y, before_y);

        app.doc_mut()
            .set_selection_with_affinity(selection.anchor, selection.head, BoundaryAffinity::After)
            .unwrap();
        let selected_view = app.source_view(boundary).unwrap();
        assert_eq!(selected_view.source, following_source);
        assert_eq!(selected_view.y, following_y);
        let eof_view = app.source_view(SOURCE.len()).unwrap();
        assert_eq!(eof_view.source, following_source);
        assert_eq!(eof_view.y, following_y);
        assert_eq!(app.doc().source(), SOURCE);
        assert_eq!(app.doc().selection(), selection);

        let before_composition = document_state(app.doc());
        let before_affinity = app.doc().selection_affinity();
        app.start_composition();
        app.preedit = "かな".into();
        let visual = app.composition_visual().unwrap();
        assert_eq!(visual.view_index, following_index);
        assert_eq!(visual.end_view_index, following_index);
        assert_eq!(visual.y, following_y - app.tabs[app.active].scroll);
        assert_eq!(document_state(app.doc()), before_composition);
        assert_eq!(app.doc().selection_affinity(), before_affinity);
    }
    #[test]
    fn editor_transient_selection_replaces_across_paragraph_views_from_range_start() {
        let fixture = EditorFixture::new("first paragraph\n\nsecond paragraph\n\nafter", false);
        let mut app = fixture.state.app.borrow_mut();
        let first = 6;
        let last = app.doc().source().find("paragraph\n\nafter").unwrap();
        app.doc_mut().set_selection(first, last).unwrap();
        unsafe {
            app.after_input();
        }
        app.start_composition();
        app.preedit = "変換".into();
        let visual = app.composition_visual().unwrap();
        assert_eq!(visual.x, app.source_view(first).unwrap().x);
        assert_eq!(visual.y, app.source_view(first).unwrap().y);
        assert!(visual.end_view_index > visual.view_index);
        assert!(visual.growth < 0.0);
        let before = app.doc().source().to_owned();
        unsafe {
            app.apply_ime_update(ImeUpdate {
                flags: GCS_RESULTSTR.0,
                result: Some("変換".into()),
                ..Default::default()
            });
        }
        app.clear_composition();
        assert_eq!(
            app.doc().source(),
            format!("{}変換{}", &before[..first], &before[last..])
        );
        assert!(app.doc_mut().undo());
        unsafe {
            app.after_input();
        }
        assert_eq!(app.doc().source(), before);
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: first,
                head: last
            }
        );
    }
    #[test]
    fn editor_source_composition_copies_affected_line_and_preserves_crlf_eof() {
        let source = format!("{}\r\n日本語\r\n", "before\r\n".repeat(800));
        let fixture = EditorFixture::new(&source, true);
        let mut app = fixture.state.app.borrow_mut();
        let p = source.find("日本語").unwrap() + 3;
        app.doc_mut().set_selection(p, p).unwrap();
        app.start_composition();
        app.preedit = "かな".into();
        let visual = app.composition_visual().unwrap();
        let view = app.source_view(p).unwrap();
        assert_eq!(view.text.len(), 3);
        assert_eq!(visual.range, 1..3);
        assert_eq!(app.doc().source(), source);
        app.clear_composition();
        let eof = source.len();
        app.doc_mut().set_selection(eof, eof).unwrap();
        assert_eq!(app.source_view(eof).unwrap().map, vec![eof]);
        assert_eq!(
            app.views
                .iter()
                .filter(|view| view.source.is_empty() && view.source.start == eof)
                .count(),
            1
        );
    }
    #[test]
    fn editor_live_blank_selection_replacement_delete_and_undo_keep_exact_endpoints() {
        for eol in ["\n", "\r\n"] {
            let source = format!("{eol}{eol}text{eol}{eol}");
            let fixture = EditorFixture::new(&source, false);
            let mut app = fixture.state.app.borrow_mut();
            let end = 2 * eol.len();
            app.doc_mut().set_selection(end, 0).unwrap();
            unsafe {
                app.character('次' as u16);
            }
            assert_eq!(app.doc().source(), format!("次text{eol}{eol}"));
            unsafe {
                app.key_with_modifiers(0x5a, true, false);
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(
                app.doc().selection(),
                Selection {
                    anchor: end,
                    head: 0
                }
            );
            unsafe {
                app.key_with_modifiers(VK_DELETE.0, false, false);
            }
            assert_eq!(app.doc().source(), format!("text{eol}{eol}"));
            let last = app.doc().byte_len();
            app.doc_mut()
                .set_selection(last - 2 * eol.len(), last)
                .unwrap();
            unsafe {
                app.key_with_modifiers(VK_BACK.0, false, false);
            }
            assert_eq!(app.doc().source(), "text");
        }
    }
    #[test]
    fn editor_control_table_arrows_select_rows_and_words_without_entering_pipes() {
        let source = "| H | J |\n|---|---|\n| one two | 日本語 |\n| last | 次 |\n\nafter";
        let fixture = EditorFixture::new(source, false);
        let mut app = fixture.state.app.borrow_mut();
        let table = Arc::clone(&app.tables[0].table);
        let first = table.rows[1].cells[0].content_range.start;
        app.doc_mut().set_selection(first, first).unwrap();
        unsafe {
            app.key_with_modifiers(VK_DOWN.0, true, true);
        }
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: first,
                head: table.rows[2].cells[0].content_range.start
            }
        );
        unsafe {
            app.key_with_modifiers(VK_UP.0, true, true);
        }
        assert_eq!(app.doc().selection(), Selection::caret(first));
        let end = table.rows[1].cells[0].content_range.end;
        app.doc_mut().set_selection(end, end).unwrap();
        unsafe {
            app.key_with_modifiers(VK_RIGHT.0, true, false);
        }
        assert_eq!(
            app.doc().selection().head,
            table.rows[1].cells[1].content_range.start
        );
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().undo_count(), 0);
        unsafe {
            app.key_with_modifiers(VK_HOME.0, true, false);
        }
        assert_eq!(app.doc().selection(), Selection::caret(0));
        assert_eq!(app.caret_view().unwrap().source, 0..0);
    }
    #[test]
    fn editor_live_fence_boundaries_blank_lines_and_eof_round_trip_without_snap() {
        for eol in ["\n", "\r\n"] {
            let source =
                format!("before{eol}{eol}```rust{eol}let source = \"Markdown\";{eol}```{eol}{eol}");
            let fixture = EditorFixture::new(&source, false);
            let mut app = fixture.state.app.borrow_mut();
            let outside = source.rfind("```").unwrap() + 3 + eol.len();
            app.doc_mut().set_selection(outside, outside).unwrap();
            unsafe {
                app.after_input();
            }
            assert_eq!(app.doc().selection().head, outside);
            assert_eq!(app.caret_view().unwrap().map[0], outside);
            for unit in "## asdkl".encode_utf16() {
                unsafe {
                    app.character(unit);
                }
            }
            assert_eq!(
                app.doc().source(),
                format!("{}## asdkl{}", &source[..outside], &source[outside..])
            );
            assert!(
                app.projection_cache
                    .as_ref()
                    .unwrap()
                    .2
                    .blocks
                    .iter()
                    .any(|block| matches!(block.kind, BlockKind::Heading { level: 2 }))
            );
            assert!(app.doc_mut().undo());
            unsafe {
                app.after_input();
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(app.doc().selection(), Selection::caret(outside));
            app.doc_mut().set_selection(0, 0).unwrap();
            let mut visited = std::collections::HashSet::new();
            while app.doc().selection().head < source.len() {
                let p = app.doc().selection().head;
                assert!(visited.insert(p), "right-arrow stalled at {p}");
                app.move_caret(VK_RIGHT.0, false);
                unsafe {
                    app.after_input();
                }
            }
            assert!(visited.contains(&outside));
            let mut visited = std::collections::HashSet::new();
            while app.doc().selection().head > 0 {
                let p = app.doc().selection().head;
                assert!(visited.insert(p), "left-arrow stalled at {p}");
                app.move_caret(VK_LEFT.0, false);
                unsafe {
                    app.after_input();
                }
            }
            assert_eq!(app.doc().source(), source);
        }
    }
    #[test]
    fn editor_home_end_page_control_lines_unicode_selection_and_idle_group() {
        for raw in [false, true] {
            let source = "日本語 e\u{301} 👩‍💻 🇯🇵\r\n\r\nnext line\r\nlast";
            let fixture = EditorFixture::new(source, raw);
            let mut app = fixture.state.app.borrow_mut();
            let next = source.find("next").unwrap();
            unsafe {
                app.key_with_modifiers(VK_DOWN.0, true, true);
                assert_eq!(app.doc().selection().anchor, 0);
                assert_eq!(app.doc().selection().head, source.find("\r\n").unwrap() + 2);
                app.key_with_modifiers(VK_DOWN.0, true, true);
                assert_eq!(app.doc().selection().head, next);
                app.key_with_modifiers(VK_UP.0, true, true);
                assert_eq!(app.doc().selection().anchor, 0);
                assert_eq!(app.doc().selection().head, source.find("\r\n").unwrap() + 2);
                app.key_with_modifiers(VK_END.0, true, true);
                assert_eq!(
                    app.doc().selection(),
                    Selection {
                        anchor: 0,
                        head: source.len()
                    }
                );
                app.key_with_modifiers(VK_HOME.0, true, false);
                assert_eq!(app.doc().selection(), Selection::caret(0));
                app.key_with_modifiers(VK_NEXT.0, false, true);
                assert!(app.doc().selection().head > 0);
                assert_eq!(app.doc().selection().anchor, 0);
                app.key_with_modifiers(VK_PRIOR.0, false, false);
                assert!(app.doc().selection().is_caret());
            }
            app.doc_mut().set_selection(next + 4, next + 4).unwrap();
            unsafe {
                app.after_input();
                app.key_with_modifiers(VK_HOME.0, false, false);
            }
            assert_eq!(app.doc().selection().head, next);
            unsafe {
                app.key_with_modifiers(VK_END.0, false, false);
            }
            assert_eq!(app.doc().selection().head, source.find("\r\nlast").unwrap());
            let boundaries: Vec<_> = source
                .grapheme_indices(true)
                .map(|(p, _)| p)
                .chain(Some(source.len()))
                .collect();
            for &p in &boundaries {
                for back in [false, true] {
                    assert!(
                        boundaries.contains(&word_target(source, p, back)),
                        "word move split grapheme at {p}"
                    );
                }
            }
        }
        let fixture = EditorFixture::new("", false);
        let mut app = fixture.state.app.borrow_mut();
        unsafe {
            app.character('a' as u16);
            app.character('b' as u16);
        }
        assert_eq!(app.doc().undo_count(), 1);
        app.last_typing = Some(Instant::now() - std::time::Duration::from_secs(2));
        unsafe {
            app.character('c' as u16);
        }
        assert_eq!(app.doc().undo_count(), 2);
        unsafe {
            app.key_with_modifiers(0x5a, true, false);
        }
        assert_eq!(app.doc().source(), "ab");
        unsafe {
            app.character('x' as u16);
        }
        assert_eq!(app.doc().source(), "abx");
        assert_eq!(app.doc().redo_count(), 0);
    }
    #[test]
    fn editor_ctrl_vertical_tracks_physical_starts_with_hidden_heading() {
        for eol in ["\n", "\r\n"] {
            let source = format!("alpha beta{eol}{eol}# Heading text");
            let blank_start = "alpha beta".len() + eol.len();
            let heading_start = blank_start + eol.len();
            for raw in [false, true] {
                let fixture = EditorFixture::new(&source, raw);
                let mut app = fixture.state.app.borrow_mut();
                if !raw {
                    let projection = &app.projection_cache.as_ref().unwrap().2;
                    assert!(projection.blocks.iter().any(|block| {
                        block.source.start == heading_start
                            && matches!(block.kind, BlockKind::Heading { .. })
                    }));
                }
                assert!(
                    app.views
                        .iter()
                        .any(|view| view.map.contains(&(heading_start + 2))),
                    "heading body was not laid out (raw={raw}, eol={eol:?})"
                );

                let before = document_state(app.doc());
                for (key, expected) in [
                    (VK_DOWN.0, blank_start),
                    (VK_DOWN.0, heading_start),
                    (VK_UP.0, blank_start),
                    (VK_UP.0, 0),
                ] {
                    unsafe { app.key_with_modifiers(key, true, false) };
                    assert_eq!(
                        app.doc().selection(),
                        Selection::caret(expected),
                        "physical line start (raw={raw}, eol={eol:?}, key={key:#x})"
                    );
                }

                app.doc_mut().set_selection(2, 2).unwrap();
                for (key, expected) in [
                    (VK_DOWN.0, blank_start),
                    (VK_DOWN.0, heading_start),
                    (VK_UP.0, blank_start),
                    (VK_UP.0, 0),
                ] {
                    unsafe { app.key_with_modifiers(key, true, true) };
                    assert_eq!(
                        app.doc().selection(),
                        Selection {
                            anchor: 2,
                            head: expected,
                        },
                        "Shift anchor or physical line start (raw={raw}, eol={eol:?}, key={key:#x})"
                    );
                }

                let after = document_state(app.doc());
                assert_eq!(after.0, before.0);
                assert_eq!(after.1, before.1);
                assert_eq!(after.2, before.2);
                assert_eq!(after.3, before.3);
                assert_eq!(after.4, before.4);
                assert_eq!(after.5, before.5);
            }
        }
    }
    #[test]
    fn editor_typed_markdown_transitions_keep_source_caret_and_code_literal() {
        for source in [
            "*italic*",
            "**bold**",
            "`code`",
            "[label](https://example.invalid)",
            "> quote",
            "- item",
            "---",
            "```rust\n## literal\n```",
            "| A | B |\n|---|---|",
        ] {
            let fixture = EditorFixture::new("", false);
            let mut app = fixture.state.app.borrow_mut();
            for character in source.chars() {
                if character == '\n' {
                    unsafe {
                        app.key_with_modifiers(VK_RETURN.0, false, false);
                    }
                } else {
                    for unit in character.encode_utf16(&mut [0; 2]).iter().copied() {
                        unsafe {
                            app.character(unit);
                        }
                    }
                }
            }
            assert_eq!(app.doc().source(), source, "source authoring {source}");
            assert_eq!(
                app.doc().selection().head,
                source.len(),
                "caret authoring {source}"
            );
            if source.starts_with("```") {
                assert!(
                    !app.projection_cache
                        .as_ref()
                        .unwrap()
                        .2
                        .blocks
                        .iter()
                        .any(|block| matches!(block.kind, BlockKind::Heading { .. }))
                );
            }
        }
        let fixture = EditorFixture::new("", false);
        let mut app = fixture.state.app.borrow_mut();
        for character in "| A | B |\n|---|---|".chars() {
            unsafe {
                if character == '\n' {
                    app.key_with_modifiers(VK_RETURN.0, false, false);
                } else {
                    app.character(character as u16);
                }
            }
        }
        unsafe {
            app.key_with_modifiers(VK_RETURN.0, false, false);
        }
        assert!(app.active_cell().is_some());
        for unit in "one".encode_utf16() {
            unsafe {
                app.character(unit);
            }
        }
        unsafe {
            app.key_with_modifiers(VK_TAB.0, false, false);
        }
        for unit in "two".encode_utf16() {
            unsafe {
                app.character(unit);
            }
        }
        let table = Table::parse(app.doc().source(), 0..app.doc().byte_len()).unwrap();
        assert_eq!(
            app.doc().source()[table.rows[1].cells[0].content_range.clone()].trim(),
            "one"
        );
        assert_eq!(
            app.doc().source()[table.rows[1].cells[1].content_range.clone()].trim(),
            "two"
        );
    }
    #[test]
    fn editor_control_arrows_extend_and_delete_unicode_words() {
        let mut app = composing_test_app("alpha　日本語 beta");
        app.clear_composition();
        app.tabs[0].raw = true;
        unsafe {
            app.key_with_modifiers(VK_RIGHT.0, true, true);
            assert_eq!(
                app.doc().selection(),
                Selection {
                    anchor: 0,
                    head: "alpha　".len()
                }
            );
            app.key_with_modifiers(VK_DELETE.0, true, false);
            assert_eq!(app.doc().source(), "日本語 beta");
            app.key_with_modifiers(VK_END.0, true, false);
            app.key_with_modifiers(VK_BACK.0, true, false);
            assert_eq!(app.doc().source(), "日本語 ");
        }
    }
    #[test]
    fn editor_arrow_collapses_selection_without_an_extra_step() {
        let mut app = composing_test_app("a日本語b");
        app.clear_composition();
        app.tabs[0].raw = true;
        app.doc_mut().set_selection(1, 10).unwrap();
        app.move_caret(VK_LEFT.0, false);
        assert_eq!(app.doc().selection(), Selection::caret(1));
        app.doc_mut().set_selection(10, 1).unwrap();
        app.move_caret(VK_RIGHT.0, false);
        assert_eq!(app.doc().selection(), Selection::caret(10));
    }
    pub(super) fn composing_test_app(source: &str) -> App {
        let mut doc = Document::new(1, source.into());
        doc.replace_selection("x").unwrap();
        assert!(doc.undo());
        let selection = doc.selection();
        App {
            hwnd: HWND::default(),
            graphics: unsafe {
                Graphics {
                    factory: D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).unwrap(),
                    write: DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap(),
                    target: None,
                }
            },
            tabs: vec![Tab {
                doc,
                file: None,
                scroll: 17.0,
                raw: false,
                table_scroll: HashMap::new(),
                persistence: DocumentPersistence::default(),
            }],
            active: 0,
            tab_scroll: 0,
            next_id: 2,
            views: Vec::new(),
            source_line_starts: Vec::new(),
            gutter_rows: Vec::new(),
            gutter_width: 0.0,
            visible_tokens: Vec::new(),
            tables: Vec::new(),
            images: Vec::new(),
            image_cache: HashMap::new(),
            minimized: false,
            full_paint: true,
            painted_frame: None,
            paint_retry_pending: false,
            animation_timer_armed: false,
            height: 0.0,
            width: 900.0,
            dpi: 1.0,
            layout_key: None,
            projection_cache: None,
            parsed_tables: Vec::new(),
            revealed_source: None,
            dragging: false,
            dragging_scroll: false,
            preedit: "かな".into(),
            composing: true,
            ime_owner: Some((1, 2, selection.anchor, selection.head)),
            ime_affinity: BoundaryAffinity::After,
            ime_cursor: 2,
            ime_attributes: Vec::new(),
            ime_last_result: None,
            reconversion_owner: None,
            pending_reconversion: None,
            ime_ingress: Rc::default(),
            ime_session: None,
            ime_generation: 0,
            reconversion_cancel: None,
            preferred_x: None,
            typing: false,
            last_typing: None,
            pending_table_authoring: None,
            close_after_composition: None,
            pending_surrogate: None,
            diagnostic: None,
            phase_trace: None,
            caret: (LEFT, TOP, 28.0),
            native_caret: NativeCaretState::default(),
            status: String::new(),
            persistence: Persistence::default(),
            shell: NativeShell::default(),
            features: FeaturePanes::default(),
            explorer_icons: explorer_icons::ExplorerIcons::default(),
            git_ui: git_ui::GitPane::default(),
            search_ui: SearchPane::default(),
            editor_actions: EditorActions::default(),
            diagnostics: DiagnosticsPane::default(),
            close_target: None,
            settings_requested: false,
            defer_theme_updates: false,
            theme_update_pending: false,
            staged_explorer_paint: None,
        }
    }
    fn hidden_test_window() -> HWND {
        hidden_test_window_size(640, 480)
    }
    pub(super) fn hidden_test_window_size(width: i32, height: i32) -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                width,
                height,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        }
    }
    fn query_test_app(preedit: &str) -> App {
        let mut app = composing_test_app("| H |\n|---|\n| a日本語😀b |\n");
        app.preedit = preedit.into();
        let table = mdlite::table::find_tables(app.doc().source()).remove(0);
        let range = table.rows[1].cells[0].content_range.clone();
        let byte = range.start + 1;
        app.doc_mut().set_selection(byte, byte).unwrap();
        app.ime_owner = Some((app.doc().id(), app.doc().revision(), byte, byte));
        let source = app.doc().source_shared();
        let map = byte_map(&source[range.clone()], range.start);
        unsafe {
            app.push_view(
                &source[range.clone()],
                (map.clone(), map),
                range,
                (LEFT - 24.0, TOP + 40.0, 120.0),
                FONT,
                Some(0),
            )
            .unwrap();
        }
        app.tables.push(TableView {
            table: Arc::new(table),
            y: TOP + 40.0,
            height: 30.0,
            width: 120.0,
        });
        app.layout_key = Some((
            app.doc().id(),
            app.doc().revision(),
            app.width.to_bits(),
            false,
        ));
        app
    }

    #[test]
    fn composition_positions_use_requested_utf16_actual_layout_scroll_and_table_origin() {
        let mut app = query_test_app("日本語😀\n次の行👩‍💻");
        let first = app.composition_position(0).unwrap();
        let middle = app.composition_position(3).unwrap();
        let end = app
            .composition_position(app.preedit.encode_utf16().count() as u32)
            .unwrap();
        assert!(middle.0 > first.0);
        assert!(end.1 > first.1);
        assert!(first.0 < LEFT + FONT * 2.0); // actual horizontally scrolled table view
        assert!(app.composition_position(4).is_none()); // surrogate midpoint
        assert!(app.composition_position(1000).is_none());
        app.tabs[0].scroll += 30.0;
        let scrolled = app.composition_position(0).unwrap();
        assert_eq!(scrolled.0, first.0);
        assert_eq!(scrolled.1, first.1 - 30.0);
        app.doc_mut().replace_range(0..0, "x").unwrap();
        assert!(app.composition_position(0).is_none());
    }

    #[test]
    fn native_query_response_checks_size_ownership_and_screen_pixels() {
        let mut app = query_test_app("日本語😀\n次");
        // Own hidden built-in window; no input, activation, clipboard, or existing app.
        app.hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                170,
                230,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        app.dpi = 1.5;
        let position = app.composition_position(3).unwrap();
        let mut origin = POINT::default();
        unsafe {
            assert!(ClientToScreen(app.hwnd, &mut origin).as_bool());
        }
        let mut query = IMECHARPOSITION {
            dwSize: std::mem::size_of::<IMECHARPOSITION>() as u32,
            dwCharPos: 3,
            ..Default::default()
        };
        let pointer =
            |query: &mut IMECHARPOSITION| LPARAM((query as *mut IMECHARPOSITION) as isize);
        unsafe {
            assert_eq!(
                app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                    .0,
                1
            );
            assert_eq!(query.pt.x, origin.x + (position.0 * app.dpi).round() as i32);
            assert_eq!(query.pt.y, origin.y + (position.1 * app.dpi).round() as i32);
            assert_eq!(query.cLineHeight, (position.2 * app.dpi).round() as u32);
            assert_eq!(
                (query.rcDocument.left, query.rcDocument.top),
                (origin.x, origin.y)
            );
            assert_eq!(
                (
                    query.rcDocument.right - query.rcDocument.left,
                    query.rcDocument.bottom - query.rcDocument.top
                ),
                (640, 480)
            );
            for requested in [0, 1, 3, 5, app.preedit.encode_utf16().count() as u32] {
                query.dwCharPos = requested;
                let expected = app.composition_position(requested).unwrap();
                assert_eq!(
                    app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                        .0,
                    1
                );
                assert_eq!(query.dwCharPos, requested);
                assert_eq!(query.pt.x, origin.x + (expected.0 * app.dpi).round() as i32);
                assert_eq!(query.pt.y, origin.y + (expected.1 * app.dpi).round() as i32);
            }
            query.dwCharPos = 4;
            assert_eq!(
                app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                    .0,
                0
            );
            query.dwSize = 1;
            assert_eq!(
                app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                    .0,
                0
            );
            assert_eq!(app.ime_request(IMR_QUERYCHARPOSITION, LPARAM(0)).0, 0);
            assert_eq!(app.ime_request(IMR_QUERYCHARPOSITION, LPARAM(1)).0, 0);
            query.dwSize = std::mem::size_of::<IMECHARPOSITION>() as u32;
            query.dwCharPos = 0;
            let key = app.layout_key.take();
            assert_eq!(
                app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                    .0,
                0
            );
            app.layout_key = key;
            app.ime_owner = Some((99, app.doc().revision(), 1, 1));
            assert_eq!(
                app.ime_request(IMR_QUERYCHARPOSITION, pointer(&mut query))
                    .0,
                0
            );
            DestroyWindow(app.hwnd).unwrap();
        }
    }

    #[test]
    fn history_keys_dispatch_in_raw_live_table_and_respect_ime_priority() {
        assert_eq!(history_shortcut(0x5a, false, true), None);
        for raw in [false, true] {
            for source in ["body", "| h |\n|---|\n| v |\n"] {
                let mut app = composing_test_app(source);
                app.clear_composition();
                app.tabs[0].raw = raw;
                if source.starts_with('|') {
                    let table = mdlite::table::find_tables(source).remove(0);
                    let range = table.rows[1].cells[0].content_range.clone();
                    app.doc_mut().set_selection(range.start, range.end).unwrap();
                    app.doc_mut().replace_range(range, "u").unwrap();
                    app.tables.push(TableView {
                        table: Arc::new(table),
                        y: TOP,
                        height: 80.0,
                        width: 120.0,
                    });
                    assert_eq!(app.active_cell().is_some(), !raw);
                } else {
                    app.doc_mut().replace_range(0..0, "猫").unwrap();
                }
                let after = app.doc().source_shared();
                unsafe {
                    app.key_with_modifiers(0x5a, true, false);
                    assert_eq!(app.doc().source(), source);
                    app.character(0x1a);
                    assert_eq!(app.doc().source(), source);
                    app.key_with_modifiers(0x5a, true, true);
                    assert_eq!(app.doc().source(), &*after);
                    app.key_with_modifiers(0x5a, true, false);
                    app.key_with_modifiers(0x59, true, false);
                    app.character(0x19);
                    assert_eq!(app.doc().source(), &*after);
                    app.composing = true;
                    let before = document_state(app.doc());
                    app.key_with_modifiers(0x5a, true, false);
                    app.key_with_modifiers(0x59, true, false);
                    assert_eq!(document_state(app.doc()), before);
                }
            }
        }
    }

    #[test]
    fn parser_cache_reuses_revision_for_selection_and_layout_changes() {
        let mut app = composing_test_app("**body**");
        let first = app.cached_projection();
        app.doc_mut().set_selection(2, 4).unwrap();
        app.layout_key = None;
        assert!(Arc::ptr_eq(&first, &app.cached_projection()));
        app.doc_mut().replace_selection("new").unwrap();
        assert!(!Arc::ptr_eq(&first, &app.cached_projection()));
    }

    #[test]
    fn task_click_edits_only_marker_and_undo_restores_case_and_selection() {
        let source = "- [X] **done**\r\n- [ ] todo\r\n";
        let mut app = composing_test_app(source);
        app.clear_composition();
        app.doc_mut()
            .set_selection(source.len(), source.len())
            .unwrap();
        let before_selection = app.doc().selection();
        let projection = app.cached_projection();
        let (map, before) =
            projection_mapping(&projection, 0..projection.text.encode_utf16().count());
        unsafe {
            app.push_view(
                &projection.text,
                (map, before),
                0..source.len(),
                (LEFT, TOP, 500.0),
                FONT,
                None,
            )
            .unwrap();
        }
        let marker = projection
            .runs
            .iter()
            .find(|run| {
                run.kinds
                    .iter()
                    .any(|kind| matches!(kind, mdlite::projection::InlineKind::TaskMarker { .. }))
            })
            .unwrap();
        let position = app.views[0]
            .map
            .iter()
            .position(|&byte| byte == marker.source.start)
            .unwrap();
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        let (mut x, mut y) = (0.0, 0.0);
        unsafe {
            app.views[0]
                .layout
                .HitTestTextPosition(position as u32, false, &mut x, &mut y, &mut metrics)
                .unwrap();
        }
        app.tabs[0].scroll = 0.0;
        assert!(app.toggle_task_at(
            LEFT + x + metrics.width / 2.0,
            TOP + y + metrics.height / 2.0
        ));
        assert_eq!(app.doc().source(), "- [ ] **done**\r\n- [ ] todo\r\n");
        assert_eq!(app.doc().selection(), before_selection);
        assert_eq!(app.doc().undo_count(), 1);
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().selection(), before_selection);
        assert_eq!(task_marker_replacement("[z]"), None);
    }

    #[test]
    fn ordinary_display_tokens_borrow_source_and_inline_their_span() {
        let source = "plain é 👩‍💻 &amp;";
        let tokens = projected_tokens(source);
        let ordinary = &tokens[0];
        assert!(ordinary.text.is_none());
        assert!(!ordinary.spans.spilled());
        assert_eq!(ordinary.spelling(source), "p");
        let entity = tokens.last().unwrap();
        assert_eq!(entity.spelling(source), "&");
        assert_eq!(entity.text.as_deref(), Some("&"));
        assert_eq!(retained_image_bytes(16, false, 0).unwrap(), 16);
        assert_eq!(retained_image_bytes(16, true, 7).unwrap(), 55);
        assert!(retained_image_bytes(u64::MAX, true, 0).is_err());
    }

    #[test]
    fn raw_flag_navigation_and_delete_keep_original_regional_indicator_context() {
        let source = "a🇦🇧🇨🇩🇪z";
        let mut app = composing_test_app(source);
        app.clear_composition();
        app.tabs[0].raw = true;
        assert_eq!(app.visible_step(9, false), 17);
        assert_eq!(app.visible_step(17, true), 9);
        assert_eq!(snap(app.doc().rope(), 13, false), 9);
        app.doc_mut().set_selection(17, 17).unwrap();
        unsafe {
            app.key_with_modifiers(0x08, false, false);
        }
        assert_eq!(app.doc().source(), "a🇦🇧🇪z");
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().selection(), Selection::caret(17));
    }

    #[test]
    fn offscreen_images_probe_only_then_reuse_upload_and_evict_all_owners() {
        let directory = std::env::temp_dir().join(format!(
            "mdlite-native-image-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        image::RgbaImage::from_pixel(40, 20, image::Rgba([255, 0, 0, 255]))
            .save(directory.join("image.png"))
            .unwrap();
        let source = "![fixture](image.png)";
        let mut app = composing_test_app(source);
        app.clear_composition();
        app.tabs[0].scroll = 0.0;
        app.tabs[0].file = Some(
            FileSnapshot::create_new(
                directory.join("fixture.md"),
                source,
                FileFormat::Utf8 { bom: false },
            )
            .unwrap(),
        );
        unsafe {
            let projection = DisplayProjection::parse(source);
            let image = projection
                .blocks
                .iter()
                .find(|block| matches!(block.kind, BlockKind::Image { .. }))
                .unwrap();
            app.layout_image(
                &projection,
                (image, true),
                "image.png",
                None,
                0..source.len(),
                (10000.0, 500.0),
            )
            .unwrap();
        }
        assert_eq!((app.images[0].width, app.images[0].height), (40.0, 20.0));
        assert!(app.images[0].bitmap.is_none());
        assert!(app.image_cache.is_empty());
        assert!(app.graphics.target.is_none());
        app.hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        unsafe {
            app.prepare_images();
        }
        assert!(app.image_cache.is_empty());
        assert!(app.graphics.target.is_none());
        app.images[0].y = TOP;
        unsafe {
            app.prepare_images();
        }
        let bitmap = app.images[0].bitmap.as_ref().unwrap().as_raw();
        let path = app.images[0].path.clone().unwrap();
        assert_eq!(app.image_cache[&path].retained_bytes, 40 * 20 * 4);
        unsafe {
            app.prepare_images();
        }
        assert_eq!(app.images[0].bitmap.as_ref().unwrap().as_raw(), bitmap);
        app.images[0].y = 10000.0;
        assert!(app.reserve_image_cache(IMAGE_CACHE_BYTES, &Default::default()));
        assert!(app.image_cache.is_empty());
        assert!(app.images[0].bitmap.is_none());
        assert!(!app.reserve_image_cache(IMAGE_CACHE_BYTES + 1, &Default::default()));
        app.animation_timer_armed = true;
        app.clear_images();
        assert!(!app.animation_timer_armed);
        unsafe {
            DestroyWindow(app.hwnd).unwrap();
        }
        drop(app);
        std::fs::remove_file(directory.join("fixture.md")).unwrap();
        std::fs::remove_file(directory.join("image.png")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn list_gutter_decisions_honor_start_nested_depth_and_loose_paragraphs() {
        let source = "7. first\n   continued\n8. second\n\n   another paragraph\n\n   - nested one\n   - nested two\n9. third\n\n> quoted\n";
        let projection = DisplayProjection::parse(source);
        let paragraph = |needle: &str| {
            projection
                .blocks
                .iter()
                .find(|block| {
                    matches!(block.kind, BlockKind::Paragraph)
                        && source[block.source.clone()].contains(needle)
                })
                .unwrap()
        };
        assert_eq!(
            block_gutter(&projection, paragraph("first")).0.as_deref(),
            Some("7.")
        );
        assert_eq!(
            block_gutter(&projection, paragraph("second")).0.as_deref(),
            Some("8.")
        );
        assert_eq!(block_gutter(&projection, paragraph("another")).0, None);
        assert_eq!(
            block_gutter(&projection, paragraph("third")).0.as_deref(),
            Some("9.")
        );
        let nested: Vec<_> = projection
            .blocks
            .iter()
            .filter(|block| {
                matches!(block.kind, BlockKind::ListItem)
                    && source[block.source.clone()].contains("nested")
                    && !source[block.source.clone()].contains("second")
            })
            .collect();
        assert_eq!(nested.len(), 2);
        for item in nested {
            assert_eq!(block_gutter(&projection, item).0.as_deref(), Some("•"));
        }
        assert!(block_gutter(&projection, paragraph("quoted")).1);
    }

    #[test]
    fn native_list_gutters_keep_tight_parent_text_and_code_font_without_edits() {
        let source =
            "7. first\n   - nested\n8. second\n\n> quoted\n\n```rust\nlet value = 1;\n```\n";
        let mut app = composing_test_app(source);
        app.clear_composition();
        let caret = source.find("first").unwrap();
        app.doc_mut().set_selection(caret, caret).unwrap();
        let before = document_state(app.doc());
        app.hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_POPUP,
                0,
                0,
                640,
                480,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(document_state(app.doc()), before);
        assert_eq!(
            app.views
                .iter()
                .filter(|view| view.marker.is_some())
                .count(),
            3
        );
        assert_eq!(
            app.views
                .iter()
                .filter(|view| source[view.source.clone()].contains("first"))
                .count(),
            1
        );
        assert!(app.views.iter().any(|view| view.quoted));
        for view in &app.views {
            if let Some((layout, width)) = &view.marker {
                let mut metrics = DWRITE_TEXT_METRICS::default();
                unsafe {
                    layout.GetMetrics(&mut metrics).unwrap();
                }
                assert!(*width > 0.0 && metrics.height > 0.0);
                assert!(view.x - width - 8.0 >= LEFT - 0.01);
            }
        }
        let code = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains("let value"))
            .unwrap();
        let mut family = [0u16; 64];
        unsafe {
            code.layout.GetFontFamilyName(0, &mut family, None).unwrap();
        }
        let length = family.iter().position(|&unit| unit == 0).unwrap();
        assert_eq!(
            String::from_utf16(&family[..length]).unwrap(),
            "Cascadia Mono"
        );
        let layouts: Vec<_> = app.views.iter().map(|view| view.layout.as_raw()).collect();
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(
            layouts,
            app.views
                .iter()
                .map(|view| view.layout.as_raw())
                .collect::<Vec<_>>()
        );
        assert_eq!(document_state(app.doc()), before);
        unsafe {
            DestroyWindow(app.hwnd).unwrap();
        }
    }

    #[test]
    fn live_reveal_at_eof_requires_the_active_block_to_end_at_eof() {
        const SOURCE: &str = concat!(
            "alpha beta\n\n# Heading\n**bold**\n> quote\n> quoted next\n",
            "- item\n- continued item\n~~~text\ncode\n~~~\n",
        );
        const FINAL_PARAGRAPH: &str = "alpha **bold**";
        assert_eq!(SOURCE.len(), 94);

        let projection = DisplayProjection::parse(SOURCE);
        let first_list = projection
            .blocks
            .iter()
            .find(|block| {
                matches!(block.kind, BlockKind::ListItem)
                    && SOURCE[block.source.clone()].contains("- item")
            })
            .unwrap()
            .source
            .clone();
        let final_paragraph = DisplayProjection::parse(FINAL_PARAGRAPH)
            .blocks
            .iter()
            .find(|block| matches!(block.kind, BlockKind::Paragraph))
            .unwrap()
            .source
            .clone();

        let assert_reveal = |source: &str, head: usize, expected: Option<Range<usize>>| {
            let mut app = composing_test_app(source);
            app.clear_composition();
            app.doc_mut().set_selection(head, head).unwrap();
            let before = document_state(app.doc());
            app.hwnd = hidden_test_window();
            unsafe {
                app.ensure_layout().unwrap();
            }
            assert_eq!(app.revealed_source, expected);
            assert_eq!(document_state(app.doc()), before);
            unsafe {
                DestroyWindow(app.hwnd).unwrap();
            }
        };

        assert_reveal(SOURCE, SOURCE.len(), None);
        assert_reveal(SOURCE, first_list.start + 3, Some(first_list));
        assert_reveal(
            FINAL_PARAGRAPH,
            FINAL_PARAGRAPH.len(),
            Some(final_paragraph),
        );
    }

    #[test]
    fn native_list_images_keep_all_parent_fragments_and_one_gutter_per_item() {
        let source = concat!(
            "- lead ![one](one.png) middle ![two](two.png) **end**\n",
            "- ![first](first.png) trailing\n",
            "- parent\n",
            "  - nested ![inner](inner.png) nested-tail\n",
            "- loose first\n\n",
            "  second paragraph before ![loose](loose.png) loose-tail\n",
            "> - quoted before ![quoted](quoted.png) quoted-tail\n",
        );
        let mut app = composing_test_app(source);
        app.clear_composition();
        let caret = source.find("lead").unwrap();
        app.doc_mut().set_selection(caret, caret).unwrap();
        let before = document_state(app.doc());
        app.hwnd = hidden_test_window();
        unsafe {
            app.ensure_layout().unwrap();
        }

        assert_eq!(app.images.len(), 6);
        assert_eq!(
            app.views
                .iter()
                .filter(|view| view.marker.is_some())
                .count(),
            6
        );
        assert!(app.views.iter().any(|view| view.quoted));
        assert!(
            app.images
                .iter()
                .find(|image| source[image.source.clone()].contains("![quoted]"))
                .unwrap()
                .quoted
        );
        for text in [
            "lead",
            "middle",
            "end",
            "trailing",
            "parent",
            "nested ",
            "nested-tail",
            "loose first",
            "second paragraph before",
            "loose-tail",
            "quoted before",
            "quoted-tail",
        ] {
            assert_eq!(
                app.views
                    .iter()
                    .filter(|view| source[view.source.clone()].contains(text))
                    .count(),
                1,
                "text fragment {text:?} should have one native view"
            );
        }
        let view_y = |needle: &str| {
            app.views
                .iter()
                .find(|view| source[view.source.clone()].contains(needle))
                .unwrap()
                .y
        };
        let image_y = |needle: &str| {
            app.images
                .iter()
                .find(|image| source[image.source.clone()].contains(needle))
                .unwrap()
                .y
        };
        assert!(view_y("lead") < image_y("![one]"));
        assert!(image_y("![one]") < view_y("middle"));
        assert!(view_y("middle") < image_y("![two]"));
        assert!(image_y("![two]") < view_y("end"));
        let end_source = source.find("end").unwrap();
        let end_view = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains("end"))
            .unwrap();
        let end_position = end_view
            .map
            .iter()
            .position(|&source_byte| source_byte == end_source)
            .unwrap();
        let mut weight = DWRITE_FONT_WEIGHT_NORMAL;
        unsafe {
            end_view
                .layout
                .GetFontWeight(end_position as u32, &mut weight, None)
                .unwrap();
        }
        assert_eq!(weight, DWRITE_FONT_WEIGHT_BOLD);

        let first_image = app
            .images
            .iter()
            .find(|image| source[image.source.clone()].contains("![first]"))
            .unwrap();
        assert!(app.views.iter().any(|view| {
            view.marker.is_some() && view.source.start == first_image.source.start
        }));
        let mut ranges: Vec<_> = app
            .visible_tokens
            .iter()
            .flat_map(|token| token.spans.iter().cloned())
            .collect();
        ranges.sort_by_key(|range| range.start);
        assert!(
            ranges.windows(2).all(|pair| pair[0].end <= pair[1].start),
            "parent fragments and nested descendants must not overlap: {ranges:?}"
        );
        assert_eq!(document_state(app.doc()), before);
        let end_range = end_source..end_source + "end".len();
        app.doc_mut()
            .set_selection(end_range.start, end_range.end)
            .unwrap();
        let edit = live_edit(source, &app.visible_tokens, end_range.clone(), "x").unwrap();
        app.doc_mut()
            .replace_range_with_selection(
                edit.range,
                &edit.replacement,
                Selection {
                    anchor: edit.caret,
                    head: edit.caret,
                },
            )
            .unwrap();
        assert_eq!(app.doc().source(), source.replacen("end", "x", 1));
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), source);
        assert_eq!(
            app.doc().selection(),
            Selection {
                anchor: end_range.start,
                head: end_range.end,
            }
        );
        unsafe {
            DestroyWindow(app.hwnd).unwrap();
        }
    }

    #[test]
    fn live_blank_rows_measure_against_source_and_literal_dwrite_rows() {
        fn source_row_y(app: &App, byte: usize) -> f32 {
            let view = app
                .source_view_at(byte, Some(BoundaryAffinity::After))
                .expect("source byte should have a presentation row");
            let position = view
                .map
                .iter()
                .position(|&mapped| mapped == byte)
                .or_else(|| view.before.iter().position(|&mapped| mapped == byte))
                .expect("source byte should map to a DirectWrite position");
            let (_, row_y, _) = layout_caret(&view.layout, position as u32, false)
                .expect("DirectWrite should locate the source row");
            view.y + row_y
        }

        let mut spacing = Vec::new();
        for blank_lines in [0usize, 1, 2] {
            let source = format!("before{}after", "\n".repeat(blank_lines + 1));
            let mut gaps = Vec::new();
            for raw in [true, false] {
                let mut app = composing_test_app(&source);
                app.clear_composition();
                app.tabs[0].raw = raw;
                app.hwnd = hidden_test_window_size(720, 520);
                let before = document_state(app.doc());
                unsafe {
                    app.ensure_layout().unwrap();
                }
                let before_y = source_row_y(&app, source.find("before").unwrap());
                let after_y = source_row_y(&app, source.find("after").unwrap());
                let gap = after_y - before_y;
                let first_view = app
                    .source_view_at(
                        source.find("before").unwrap(),
                        Some(BoundaryAffinity::After),
                    )
                    .unwrap();
                let mut count = 0;
                unsafe {
                    let _ = first_view.layout.GetLineMetrics(None, &mut count);
                }
                let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
                unsafe {
                    first_view
                        .layout
                        .GetLineMetrics(Some(&mut lines), &mut count)
                        .unwrap();
                }
                let line_height = lines[0].height;
                let expected_gap = line_height * (blank_lines + 1) as f32;
                assert!(
                    (gap - expected_gap).abs() <= 1.0,
                    "raw={raw} blank_lines={blank_lines}: expected {expected_gap}px from DirectWrite rows, got {gap}px"
                );
                gaps.push(gap);
                assert_eq!(document_state(app.doc()), before);
                unsafe { DestroyWindow(app.hwnd).unwrap() };
            }
            assert!(
                (gaps[0] - gaps[1]).abs() <= 1.0,
                "blank_lines={blank_lines}: Live and source layout must share physical row spacing"
            );
            spacing.push(gaps[0]);
        }
        assert!(spacing[0] < spacing[1] && spacing[1] < spacing[2]);

        let literal = "```text\nline\n```\n\nfollowing";
        let mut app = composing_test_app(literal);
        app.clear_composition();
        app.hwnd = hidden_test_window_size(720, 520);
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(app.source_line_starts, physical_line_starts(literal));
        let code = app
            .views
            .iter()
            .find(|view| String::from_utf16_lossy(&view.text).contains("line"))
            .unwrap();
        let mut count = 0;
        unsafe {
            let _ = code.layout.GetLineMetrics(None, &mut count);
        }
        let mut lines = vec![DWRITE_LINE_METRICS::default(); count as usize];
        unsafe {
            code.layout
                .GetLineMetrics(Some(&mut lines), &mut count)
                .unwrap();
        }
        assert_eq!(
            lines.len(),
            1,
            "the decorated body view owns its own physical row"
        );
        let following = app
            .views
            .iter()
            .find(|view| String::from_utf16_lossy(&view.text).contains("following"))
            .unwrap();
        assert!(
            (following.y - code.y - code.height - 2.0 * lines[0].height).abs() <= 1.0,
            "the hidden closing delimiter row and physical blank row retain separate baselines"
        );
        assert!(
            (source_row_y(&app, 13) - source_row_y(&app, 0) - 2.0 * lines[0].height).abs() <= 1.0
        );
        unsafe { DestroyWindow(app.hwnd).unwrap() };

        let table_source = "before\n\n| A | B |\n|---|---|\n| x | y |\n\nafter";
        let mut app = composing_test_app(table_source);
        app.clear_composition();
        app.hwnd = hidden_test_window_size(720, 520);
        unsafe {
            app.ensure_layout().unwrap();
        }
        let table = app.tables.first().unwrap();
        let table_start = app
            .source_view_at(table.table.range.start, Some(BoundaryAffinity::After))
            .unwrap();
        assert!(table_start.source.is_empty());
        let line_height = presentation::line_height(app.font_size());
        assert!(
            (table.y - table_start.y - line_height).abs() <= 1.0,
            "table-start anchor should share the preceding blank-row baseline"
        );
        let before_y = app
            .views
            .iter()
            .find(|view| table_source[view.source.clone()].contains("before"))
            .unwrap()
            .y;
        let after_y = app
            .views
            .iter()
            .find(|view| table_source[view.source.clone()].contains("after"))
            .unwrap()
            .y;
        assert!(
            (table.y - before_y - 2.0 * line_height).abs() <= 1.0,
            "table_start={} before_row={} line_height={}",
            table.y,
            before_y,
            line_height
        );
        assert!((after_y - table.y - table.height - line_height).abs() <= 1.0);
        unsafe { DestroyWindow(app.hwnd).unwrap() };

        let image_source = "before\n\n![image](image.png)\n\nafter";
        let mut app = composing_test_app(image_source);
        app.clear_composition();
        app.hwnd = hidden_test_window_size(720, 520);
        unsafe {
            app.ensure_layout().unwrap();
        }
        let line_height = presentation::line_height(app.font_size());
        let image = app.images.first().unwrap();
        let before_y = app
            .views
            .iter()
            .find(|view| image_source[view.source.clone()].contains("before"))
            .unwrap()
            .y;
        let after_y = app
            .views
            .iter()
            .find(|view| image_source[view.source.clone()].contains("after"))
            .unwrap()
            .y;
        assert!((image.y - before_y - 2.0 * line_height).abs() <= 1.0);
        let image_start = app
            .source_view_at(image.source.start, Some(BoundaryAffinity::After))
            .unwrap();
        let image_end = app
            .source_view_at(image.source.end, Some(BoundaryAffinity::After))
            .unwrap();
        assert!(image_start.source.is_empty());
        assert!(image_end.source.is_empty());
        assert!((image_start.y - image.y).abs() <= 1.0);
        assert!(
            (image_end.y - image.y - image.height).abs() <= 1.0,
            "image_y={} height={} end_anchor_y={} following_y={} font_size={} row_height={}",
            image.y,
            image.height,
            image_end.y,
            after_y,
            app.font_size(),
            line_height
        );
        assert!((after_y - image.y - image.height - line_height).abs() <= 1.0);
        unsafe { DestroyWindow(app.hwnd).unwrap() };
    }

    #[test]
    fn gutter_width_and_rows_follow_physical_source_lines() {
        let source = format!(
            "{}\n\n| A | B | C | D | E |\n|---|---|---|---|---|\n| x | y | z | q | r |\n\n![image](image.png)",
            "wrapped text ".repeat(80)
        );
        let mut app = composing_test_app(&source);
        app.clear_composition();
        app.hwnd = hidden_test_window_size(360, 480);
        let (one_digit, six_digits) = unsafe {
            (
                app.measure_gutter_width(9).unwrap(),
                app.measure_gutter_width(100_000).unwrap(),
            )
        };
        assert!(six_digits > one_digit);
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(app.source_line_starts, physical_line_starts(&source));
        let labels: Vec<_> = app.gutter_rows.iter().map(|row| row.line_number).collect();
        assert!(!labels.is_empty());
        assert!(
            labels.windows(2).all(|pair| pair[0] < pair[1]),
            "wraps, table cells, and image boundary markers share one label per physical source line: {labels:?}"
        );
        assert!(
            labels
                .iter()
                .all(|line| *line <= app.source_line_starts.len())
        );

        let content_left = app.content_left();
        let table = app.tables.first().unwrap();
        app.tabs[0]
            .table_scroll
            .insert(table.table.range.start, table.width);
        app.layout_key = None;
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(
            app.content_left(),
            content_left,
            "table scrolling keeps the gutter fixed"
        );

        let before_selection = app.doc().selection();
        app.select_at(app.content_left() - 1.0, app.editor_top() + 5.0, false);
        assert_eq!(
            app.doc().selection(),
            before_selection,
            "the gutter is not interactive"
        );
        unsafe { DestroyWindow(app.hwnd).unwrap() };
    }

    #[test]
    fn gutter_rows_include_literal_delimiters_and_skip_unrepresented_trailing_metrics() {
        fn assert_rows(source: &str, expected: &[usize], raw: bool) {
            let mut app = composing_test_app(source);
            app.clear_composition();
            app.tabs[0].raw = raw;
            app.hwnd = hidden_test_window_size(720, 520);
            let before = document_state(app.doc());
            unsafe {
                app.ensure_layout().unwrap();
            }
            let actual: Vec<_> = app.gutter_rows.iter().map(|row| row.line_number).collect();
            assert_eq!(actual, expected, "source={source:?} raw={raw}");
            let mut baselines: Vec<_> = app
                .gutter_rows
                .iter()
                .map(|row| row.y + row.baseline)
                .collect();
            baselines.sort_by(f32::total_cmp);
            assert!(
                baselines.windows(2).all(|pair| {
                    (pair[0] * 2.0).round() as i32 != (pair[1] * 2.0).round() as i32
                })
            );
            assert_eq!(document_state(app.doc()), before);
            unsafe { DestroyWindow(app.hwnd).unwrap() };
        }

        assert_rows(
            "```text\nline\n```\n\nfollowing\n",
            &[1, 2, 3, 4, 5, 6],
            false,
        );
        assert_rows(
            "---\ntitle: note\n---\n\nbody\n",
            &[1, 2, 3, 4, 5, 6],
            false,
        );
        assert_rows("one\n\nthree\n", &[1, 2, 3, 4], true);
    }

    #[test]
    fn live_fenced_source_rows_hit_edit_language_delimiters_and_restore_exact_history() {
        for eol in ["\n", "\r\n"] {
            for info in ["", "rust", "unknown-language"] {
                let source = format!(
                    "````{info}{eol}# literal **raw** | --- 👩‍💻 é{eol}{eol}{}{eol}```{eol}````{eol}{eol}~~~{eol}{eol}~~~{eol}",
                    "long raw code ".repeat(35)
                );
                let mut app = composing_test_app(&source);
                app.clear_composition();
                app.hwnd = hidden_test_window_size(450, 480);
                let before = document_state(app.doc());
                unsafe { app.ensure_layout().unwrap() };
                assert_eq!(document_state(app.doc()), before);
                let rows: Vec<_> = app.gutter_rows.iter().map(|row| row.line_number).collect();
                assert_eq!(
                    rows,
                    (1..=physical_line_starts(&source).len()).collect::<Vec<_>>()
                );
                assert!(app.tables.is_empty() && app.images.is_empty());
                assert_eq!(
                    app.cached_projection()
                        .blocks
                        .iter()
                        .filter(|block| matches!(block.kind, BlockKind::Code { fenced: true, .. }))
                        .count(),
                    2
                );
                assert!(
                    app.views
                        .iter()
                        .filter(|view| !view.source.is_empty())
                        .all(|view| !view.thematic_break)
                );
                // Fence and info-string positions use the same source map as
                // text and the gutter; editing is one ordinary source transaction.
                for p in [
                    0,
                    2,
                    4,
                    4 + info.len(),
                    source.find("# literal").unwrap(),
                    source.find("````").unwrap() + 1,
                    source.rfind("~~~").unwrap() + 2,
                ] {
                    app.doc_mut().set_selection(p, p).unwrap();
                    unsafe { app.ensure_layout().unwrap() };
                    let (x, y, h) = app.caret_location();
                    assert_eq!(
                        app.hit(x + 0.1, y + h * 0.5),
                        p,
                        "{eol:?} info={info:?} p={p}"
                    );
                    app.select_at(x + 0.1, y + h * 0.5, false);
                    assert_eq!(app.doc().selection(), Selection::caret(p));
                    app.edit("字");
                    let mut expected = source.clone();
                    expected.insert(p, '字');
                    assert_eq!(app.doc().source(), expected);
                    assert!(app.doc_mut().undo());
                    assert_eq!(app.doc().source(), source);
                    assert_eq!(app.doc().selection(), Selection::caret(p));
                    assert!(app.doc_mut().redo());
                    assert_eq!(app.doc().source(), expected);
                    assert!(app.doc_mut().undo());
                }
                let opening_info = 4..4 + info.len();
                // Native input refreshes the active source line before editing.
                // The idle projection intentionally hides this info string.
                app.doc_mut()
                    .set_selection(opening_info.start, opening_info.end)
                    .unwrap();
                unsafe { app.ensure_layout().unwrap() };
                app.edit_range(opening_info.clone(), "python");
                assert_eq!(&app.doc().source()[4..10], "python");
                assert!(app.doc_mut().undo());
                app.doc_mut()
                    .set_selection(opening_info.start, opening_info.end)
                    .unwrap();
                unsafe { app.ensure_layout().unwrap() };
                app.edit_range(opening_info, "");
                assert!(app.doc().source().starts_with(&format!("````{eol}")));
                if !info.is_empty() {
                    assert!(app.doc_mut().undo());
                }
                assert_eq!(app.doc().source(), source);
                let closing = source.find(&format!("````{eol}{eol}")).unwrap();
                app.doc_mut().set_selection(closing, closing + 4).unwrap();
                unsafe { app.ensure_layout().unwrap() };
                app.edit("");
                assert!(!app.doc().source().contains(&format!("````{eol}{eol}")));
                unsafe { app.ensure_layout().unwrap() };
                assert!(app.doc_mut().undo());
                assert_eq!(app.doc().source(), source);
                assert_eq!(
                    app.doc().selection(),
                    Selection {
                        anchor: closing,
                        head: closing + 4
                    }
                );
                unsafe { DestroyWindow(app.hwnd).unwrap() };
            }
        }
    }

    #[test]
    fn wrapped_code_image_fragments_use_post_style_metrics_for_layout() {
        let before_code = "iiiiiiii ".repeat(5);
        let after_code = "iiiiiiii ".repeat(6);
        let source = format!(
            "- `{before_code}` ![image](image.png) `{after_code}`\n\nfollowing paragraph\n"
        );
        let mut app = composing_test_app(&source);
        app.clear_composition();
        let caret = source.find("iiii").unwrap();
        app.doc_mut().set_selection(caret, caret).unwrap();
        let before = document_state(app.doc());
        app.hwnd = hidden_test_window_size(360, 480);
        unsafe {
            app.ensure_layout().unwrap();
        }

        let before_view = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains(&before_code))
            .unwrap();
        let after_view = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains(&after_code))
            .unwrap();
        for view in [before_view, after_view] {
            let mut metrics = DWRITE_TEXT_METRICS::default();
            unsafe {
                view.layout.GetMetrics(&mut metrics).unwrap();
            }
            assert!(metrics.height > FONT * 1.6, "inline code should wrap");
            assert_eq!(
                view.height,
                metrics
                    .height
                    .max(presentation::line_height(app.font_size()))
            );
        }
        let mut family = [0u16; 64];
        unsafe {
            before_view
                .layout
                .GetFontFamilyName(0, &mut family, None)
                .unwrap();
        }
        let length = family.iter().position(|&unit| unit == 0).unwrap();
        assert_eq!(
            String::from_utf16(&family[..length]).unwrap(),
            "Cascadia Mono"
        );

        let image = app
            .images
            .iter()
            .find(|image| source[image.source.clone()].contains("![image]"))
            .unwrap();
        assert!(image.y >= before_view.y + before_view.height - 1.0);
        assert!(after_view.y >= image.y + image.height - 1.0);
        let following = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains("following paragraph"))
            .unwrap();
        assert!(
            (following.y
                - after_view.y
                - after_view.height
                - presentation::line_height(app.font_size()))
            .abs()
                <= 1.0,
            "the physical blank row should provide exactly one following baseline"
        );
        assert_eq!(document_state(app.doc()), before);
        unsafe {
            DestroyWindow(app.hwnd).unwrap();
        }
    }

    #[test]
    fn thematic_break_markers_draw_and_edit_as_exact_source_tokens() {
        for syntax in ["***", "---", "___"] {
            let source = format!("before\n\n{syntax}\n\nafter");
            let mut app = composing_test_app(&source);
            app.clear_composition();
            app.hwnd = hidden_test_window();
            let before_layout = document_state(app.doc());
            unsafe {
                app.ensure_layout().unwrap();
            }
            assert_eq!(document_state(app.doc()), before_layout);
            let rule_views: Vec<_> = app
                .views
                .iter()
                .filter(|view| view.thematic_break)
                .collect();
            assert_eq!(rule_views.len(), 1);
            assert!(rule_views[0].width > 100.0);
            let rule_start = source.find(syntax).unwrap();
            assert!(source[rule_views[0].source.clone()].starts_with(syntax));
            assert_eq!(String::from_utf16_lossy(&rule_views[0].text), syntax);
            assert!((rule_start..rule_start + syntax.len()).all(|byte| {
                app.visible_tokens.iter().any(|token| {
                    !token.atomic
                        && token.spans.as_slice() == std::slice::from_ref(&(byte..byte + 1))
                })
            }));
            let geometry = (rule_views[0].y, rule_views[0].height);
            assert_eq!(
                live_horizontal_step(&source, &app.visible_tokens, &[], rule_start, false,),
                rule_start + 1
            );
            assert_eq!(
                live_horizontal_step(
                    &source,
                    &app.visible_tokens,
                    &[],
                    rule_start + syntax.len(),
                    true,
                ),
                rule_start + syntax.len() - 1
            );
            for byte in rule_start..=rule_start + syntax.len() {
                app.doc_mut().set_selection(byte, byte).unwrap();
                unsafe { app.ensure_layout().unwrap() };
                let view = app.views.iter().find(|view| view.thematic_break).unwrap();
                assert_eq!((view.y, view.height), geometry);
                assert_eq!(String::from_utf16_lossy(&view.text), syntax);
                let (x, y, h) = app.caret_location();
                assert_eq!(app.hit(x + 0.1, y + h * 0.5), byte);
            }

            app.doc_mut()
                .set_selection(rule_start, rule_start + syntax.len())
                .unwrap();
            let edit = live_edit(
                &source,
                &app.visible_tokens,
                rule_start..rule_start + syntax.len(),
                "",
            )
            .unwrap();
            app.doc_mut()
                .replace_range_with_selection(
                    edit.range,
                    &edit.replacement,
                    Selection {
                        anchor: edit.caret,
                        head: edit.caret,
                    },
                )
                .unwrap();
            assert_eq!(app.doc().source(), source.replacen(syntax, "", 1));
            assert_eq!(app.doc().undo_count(), 1);
            assert!(app.doc_mut().undo());
            assert_eq!(app.doc().source(), source);
            assert_eq!(
                app.doc().selection(),
                Selection {
                    anchor: rule_start,
                    head: rule_start + syntax.len(),
                }
            );

            app.tabs[app.active].raw = true;
            app.layout_key = None;
            unsafe {
                app.ensure_layout().unwrap();
            }
            assert!(app.views.iter().all(|view| !view.thematic_break));
            assert_eq!(app.views.first().unwrap().source.start, 0);
            assert_eq!(app.views.last().unwrap().source.end, source.len());
            assert!(
                app.views
                    .windows(2)
                    .all(|views| views[0].source.end == views[1].source.start)
            );
            assert!((rule_start..rule_start + syntax.len()).all(|byte| {
                app.visible_tokens
                    .iter()
                    .any(|token| token.spans.len() == 1 && token.spans[0] == (byte..byte + 1))
            }));
            unsafe {
                DestroyWindow(app.hwnd).unwrap();
            }
        }
    }

    #[test]
    fn native_thematic_breaks_leave_literal_contexts_and_table_cells_unchanged() {
        let base = concat!(
            "---\ntitle: ---\n---\n\n",
            "```md\n---\n***\n```\n\n",
            "`---` and \\***\n\n",
            "| A | B |\n| --- | --- |\n| x --- y | `---` |",
        );
        for ending in ["", "\n", "\r\n"] {
            let source = format!("{base}{ending}");
            let mut app = composing_test_app(&source);
            app.clear_composition();
            let caret = source.find("title").unwrap();
            app.doc_mut().set_selection(caret, caret).unwrap();
            let before = document_state(app.doc());
            app.hwnd = hidden_test_window();
            unsafe {
                app.ensure_layout().unwrap();
            }
            assert_eq!(
                app.views.iter().filter(|view| view.thematic_break).count(),
                0
            );
            assert_eq!(app.tables.len(), 1, "final EOL {ending:?}");
            assert!(app.cached_projection().blocks.iter().any(|block| matches!(
                block.kind,
                BlockKind::FrontMatter
            )
                && source[block.source.clone()].starts_with("---\ntitle")));
            assert_eq!(
                app.tables[0].table.range.end,
                source.len() - ending.len(),
                "table editing excludes final EOL {ending:?}"
            );
            assert_eq!(
                source[app.tables[0].table.rows[1].cells[0].content_range.clone()].trim(),
                "x --- y"
            );
            assert_eq!(
                source[app.tables[0].table.rows[1].cells[1].content_range.clone()].trim(),
                "`---`"
            );
            assert_eq!(document_state(app.doc()), before);
            unsafe {
                DestroyWindow(app.hwnd).unwrap();
            }
        }
    }

    fn document_state(doc: &Document) -> (String, u64, u64, bool, usize, usize, Selection) {
        (
            doc.source().into(),
            doc.id(),
            doc.revision(),
            doc.is_dirty(),
            doc.undo_count(),
            doc.redo_count(),
            doc.selection(),
        )
    }
    fn ime_reconversion_test_app(affinity: BoundaryAffinity) -> App {
        #[repr(C)]
        struct Buffer {
            request: RECONVERTSTRING,
            text: [u16; 4],
        }
        let mut app = composing_test_app("prefix日本語");
        app.clear_composition();
        app.tabs[0].raw = true;
        app.doc_mut()
            .set_selection_with_affinity(15, 6, affinity)
            .unwrap();
        assert_eq!(
            unsafe { app.ime_request(IMR_RECONVERTSTRING, LPARAM(0)) }.0,
            std::mem::size_of::<Buffer>() as isize
        );
        assert!(app.reconversion_owner.is_none());
        let mut buffer = Buffer {
            request: RECONVERTSTRING {
                dwSize: std::mem::size_of::<Buffer>() as u32,
                ..Default::default()
            },
            text: [0; 4],
        };
        assert_eq!(
            unsafe {
                app.ime_request(
                    IMR_RECONVERTSTRING,
                    LPARAM((&mut buffer as *mut Buffer) as isize),
                )
            }
            .0,
            std::mem::size_of::<Buffer>() as isize
        );
        assert_eq!(buffer.text, [0x65e5, 0x672c, 0x8a9e, 0]);
        // A filled request is sufficient; CONFIRM is optional and START may
        // follow the request. No native IME is involved in this owned buffer.
        app.start_composition();
        assert!(app.reconversion_owner.unwrap().matches(app.doc()));
        app.preedit = "日本語".into();
        app
    }
    fn owned_cancel_input(app: &App, key: u16) -> Option<ImeInputContext> {
        Some(ImeInputContext {
            document_id: app.doc().id(),
            virtual_key: key as u32,
        })
    }
    fn owned_cancel_snapshot(app: &App) -> ReconversionCancel {
        app.prepare_reconversion_cancel(
            app.ime_ingress.current.get(),
            owned_cancel_input(app, VK_ESCAPE.0),
            app.editing_window(),
        )
        .unwrap()
    }
    fn owned_result(text: &str) -> NativeNotification {
        NativeNotification::Update(ImeUpdate {
            flags: GCS_RESULTSTR.0,
            result: Some(text.into()),
            ..Default::default()
        })
    }
    #[test]
    fn ime_owned_cancel_success_drains_callbacks_without_source_or_selection_edits() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            let state = WindowState::new(ime_reconversion_test_app(affinity));
            let mut app = state.app.borrow_mut();
            let before = document_state(app.doc());
            let cancel = owned_cancel_snapshot(&app);
            unsafe {
                perform_reconversion_cancel(&state, &mut app, cancel, |editor| {
                    assert_eq!(editor, cancel.envelope.editor);
                    assert!(state.app.try_borrow_mut().is_err());
                    // Callbacks snapshot while the native operation holds App.
                    state
                        .notifications
                        .borrow_mut()
                        .push_back((cancel.envelope, owned_result("changed callback")));
                    state
                        .notifications
                        .borrow_mut()
                        .push_back((cancel.envelope, NativeNotification::End));
                    ImeCancelOutcome::Cancelled
                });
            }
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(app.doc().selection_affinity(), affinity);
            assert!(app.ime_session.is_none());
            assert!(app.ime_ingress.current.get().is_none());
            assert!(app.reconversion_owner.is_none());
            assert!(app.reconversion_cancel.is_none());
            assert!(state.notifications.borrow().is_empty());
            // A callback first arriving after completion has no live ingress.
            // No ambient Escape or text equality is consulted.
            let late =
                state
                    .ime_ingress
                    .capture(app.doc().id(), app.editing_window(), WM_IME_COMPOSITION);
            assert_eq!(late.generation, 0);
            unsafe {
                app.native_packet(late, owned_result("日本語"));
            }
            assert_eq!(document_state(app.doc()), before);
            // The next ordinary Enter acts immediately on the preserved selection.
            unsafe {
                app.key_with_modifiers(VK_RETURN.0, false, false);
            }
            assert_eq!(app.doc().source(), "prefix\n");
            assert_eq!(app.doc().undo_count(), before.4 + 1);
            assert!(app.doc_mut().undo());
            assert_eq!(app.doc().source(), before.0);
            assert_eq!(app.doc().selection(), before.6);
            assert_eq!(app.doc().selection_affinity(), affinity);
        }
    }
    #[test]
    fn ime_owned_cancel_failed_outcomes_preserve_state_and_allow_only_fresh_attempt() {
        for outcome in [ImeCancelOutcome::NoContext, ImeCancelOutcome::Rejected] {
            let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
            let mut app = state.app.borrow_mut();
            let before = document_state(app.doc());
            let preedit = app.preedit.clone();
            let owner = app.ime_owner;
            let cancel = owned_cancel_snapshot(&app);
            unsafe {
                perform_reconversion_cancel(&state, &mut app, cancel, |_| outcome);
            }
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(app.preedit, preedit);
            assert_eq!(app.ime_owner, owner);
            assert_eq!(app.ime_session, Some(cancel.envelope));
            assert_eq!(state.ime_ingress.current.get(), Some(cancel.envelope));
            assert!(app.reconversion_owner.is_some());
            assert!(app.reconversion_cancel.is_none());
            assert!(app.status.contains("失敗"));
            let fresh = owned_cancel_snapshot(&app);
            unsafe {
                perform_reconversion_cancel(&state, &mut app, fresh, |_| {
                    ImeCancelOutcome::Cancelled
                });
            }
            assert!(!app.composing);
            assert_eq!(document_state(app.doc()), before);
        }
    }
    #[test]
    fn ime_owned_cancel_failed_direct_escape_cannot_use_legacy_result_suppression() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
        let input = owned_cancel_input(&state.app.borrow(), VK_ESCAPE.0);
        let message = MSG {
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_ESCAPE.0 as usize),
            ..Default::default()
        };
        let _scope = state.scoped_ime_input(input, &message);
        let mut app = state.app.borrow_mut();
        let before = document_state(app.doc());
        let cancel = owned_cancel_snapshot(&app);
        unsafe {
            perform_reconversion_cancel(&state, &mut app, cancel, |_| {
                state.notifications.borrow_mut().push_back((
                    cancel.envelope,
                    NativeNotification::Update(ImeUpdate {
                        flags: GCS_RESULTSTR.0,
                        result: Some("日本語".into()),
                        input_context: state.ime_input.get(),
                        ..Default::default()
                    }),
                ));
                ImeCancelOutcome::Rejected
            });
        }
        assert_eq!(app.doc().selection(), Selection::caret(15));
        assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);
        assert_eq!(app.doc().source(), before.0);
        assert_eq!(app.doc().revision(), before.2);
        assert_eq!(
            (app.doc().undo_count(), app.doc().redo_count()),
            (before.4, before.5)
        );
        assert!(state.ime_input.get().is_none());
        assert!(app.reconversion_cancel.is_none());
    }
    #[test]
    fn ime_owned_cancel_failed_native_result_keeps_ordinary_commit_semantics() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
        let mut app = state.app.borrow_mut();
        let cancel = owned_cancel_snapshot(&app);
        let undo = app.doc().undo_count();
        unsafe {
            perform_reconversion_cancel(&state, &mut app, cancel, |_| {
                state
                    .notifications
                    .borrow_mut()
                    .push_back((cancel.envelope, owned_result("にほんご")));
                ImeCancelOutcome::Rejected
            });
        }
        assert_eq!(app.doc().source(), "prefixにほんご");
        assert_eq!(app.doc().undo_count(), undo + 1);
        assert!(app.reconversion_owner.is_none());
        assert!(app.reconversion_cancel.is_none());
    }
    #[test]
    fn ime_owned_cancel_captured_old_packets_cannot_edit_or_end_fresh_same_owner_session() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::After));
        let mut app = state.app.borrow_mut();
        let before = document_state(app.doc());
        let cancel = owned_cancel_snapshot(&app);
        let mut fresh = None;
        unsafe {
            perform_reconversion_cancel(&state, &mut app, cancel, |_| {
                let mut queue = state.notifications.borrow_mut();
                queue.push_back((cancel.envelope, owned_result("stale result")));
                let next = state.ime_ingress.capture(
                    cancel.envelope.document_id,
                    cancel.envelope.editor,
                    WM_IME_STARTCOMPOSITION,
                );
                assert_ne!(next, cancel.envelope);
                fresh = Some(next);
                queue.push_back((next, NativeNotification::Start));
                queue.push_back((cancel.envelope, NativeNotification::End));
                queue.push_back((next, owned_result("日本語")));
                ImeCancelOutcome::Cancelled
            });
        }
        let fresh = fresh.unwrap();
        assert_eq!(app.ime_session, Some(fresh));
        assert_eq!(state.ime_ingress.current.get(), Some(fresh));
        assert!(app.composing);
        assert_eq!(app.doc().source(), before.0);
        assert_eq!(app.doc().revision(), before.2);
        assert_eq!(app.doc().selection(), Selection::caret(15));
        assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);
        assert_eq!(app.doc().undo_count(), before.4);
        unsafe {
            app.native_packet(cancel.envelope, owned_result("stale result"));
            app.native_packet(cancel.envelope, NativeNotification::End);
            app.native_packet(cancel.envelope, NativeNotification::Start);
        }
        assert_eq!(app.ime_session, Some(fresh));
        assert_eq!(app.doc().source(), before.0);
        // A raw callback arriving NOW is assigned the fresh ingress. IMM has no
        // old-native provenance; do not fake an old generation or suppress it.
        let raw =
            state
                .ime_ingress
                .capture(app.doc().id(), app.editing_window(), WM_IME_COMPOSITION);
        assert_eq!(raw, fresh);
    }
    #[test]
    fn ime_owned_cancel_fresh_identical_and_changed_genuine_commits_reset_fingerprint() {
        let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
        let original_epoch = app.ime_session.unwrap();
        app.clear_composition();
        let before = document_state(app.doc());
        let mut epochs = vec![original_epoch.generation];
        for _ in 0..2 {
            app.doc_mut()
                .set_selection_with_affinity(15, 6, BoundaryAffinity::Before)
                .unwrap();
            app.arm_reconversion();
            app.start_composition();
            let epoch = app.ime_session.unwrap();
            assert!(!epochs.contains(&epoch.generation));
            epochs.push(epoch.generation);
            unsafe {
                app.native_packet(epoch, owned_result("日本語"));
            }
            assert_eq!(app.doc().selection(), Selection::caret(15));
            assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);
            assert_eq!(app.doc().revision(), before.2);
            assert!(app.reconversion_owner.is_none());
            let end = app.ime_ingress.capture(
                app.doc().id(),
                app.editing_window(),
                WM_IME_ENDCOMPOSITION,
            );
            unsafe {
                app.native_packet(end, NativeNotification::End);
            }
        }
        assert_eq!(app.doc().undo_count(), before.4);
        for result in ["にほんご", "日本語"] {
            let end = app.doc().source().len();
            app.doc_mut().set_selection(6, end).unwrap();
            app.start_composition();
            let epoch = app.ime_session.unwrap();
            let undo = app.doc().undo_count();
            unsafe {
                app.native_packet(epoch, owned_result(result));
            }
            assert_eq!(app.doc().source(), format!("prefix{result}"));
            assert_eq!(app.doc().undo_count(), undo + 1);
            unsafe {
                app.native_packet(epoch, NativeNotification::End);
            }
            assert!(app.doc_mut().undo());
            assert!(app.doc_mut().redo());
            assert_eq!(app.doc().source(), format!("prefix{result}"));
        }
    }
    #[test]
    fn ime_owned_cancel_admission_revalidates_key_editor_document_and_session() {
        for case in 0..12 {
            let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
            let captured = app.ime_session;
            let mut editor = app.editing_window();
            let mut input = owned_cancel_input(&app, VK_ESCAPE.0);
            match case {
                0 => app.reconversion_owner = None,
                1 => input = None,
                2 => input.as_mut().unwrap().virtual_key = VK_RETURN.0 as u32,
                3 => input.as_mut().unwrap().virtual_key = VK_PROCESSKEY.0 as u32,
                4 => input.as_mut().unwrap().document_id += 1,
                5 => editor = HWND(std::ptr::dangling_mut()),
                6 => app.doc_mut().set_selection(6, 15).unwrap(),
                7 => app
                    .doc_mut()
                    .set_selection_with_affinity(15, 6, BoundaryAffinity::After)
                    .unwrap(),
                8 => app.reconversion_owner.as_mut().unwrap().revision += 1,
                9 => {
                    app.start_composition();
                }
                10 => app.ime_owner = None,
                11 => {
                    app.ime_ingress.start(app.doc().id(), editor);
                }
                _ => unreachable!(),
            }
            assert!(
                app.prepare_reconversion_cancel(captured, input, editor)
                    .is_none(),
                "case {case}"
            );
        }
    }
    #[test]
    fn ime_owned_cancel_confirm_preserves_filled_request_and_start_adopts_once() {
        #[repr(C)]
        struct Buffer {
            request: RECONVERTSTRING,
            text: [u16; 4],
        }
        let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
        app.clear_composition();
        unsafe {
            assert!(app.ime_request(IMR_RECONVERTSTRING, LPARAM(0)).0 > 0);
        }
        assert!(app.pending_reconversion.is_none());
        let mut buffer = Buffer {
            request: RECONVERTSTRING {
                dwSize: 1,
                ..Default::default()
            },
            text: [0; 4],
        };
        let ptr = LPARAM((&mut buffer as *mut Buffer) as isize);
        unsafe {
            assert_eq!(app.ime_request(IMR_RECONVERTSTRING, ptr).0, 0);
        }
        assert!(app.pending_reconversion.is_none());
        buffer.request.dwSize = std::mem::size_of::<Buffer>() as u32;
        assert_eq!(
            buffer.request.dwSize as usize,
            std::mem::size_of::<Buffer>()
        );
        unsafe {
            assert!(app.ime_request(IMR_RECONVERTSTRING, ptr).0 > 0);
        }
        let request = app.pending_reconversion.unwrap().request_id;
        assert!(app.pending_reconversion.unwrap().generation.is_none());
        unsafe {
            assert_eq!(app.ime_request(IMR_CONFIRMRECONVERTSTRING, ptr).0, 1);
        }
        assert_eq!(app.pending_reconversion.unwrap().request_id, request);
        app.start_composition();
        let first = app.ime_session.unwrap();
        assert_eq!(
            app.reconversion_owner.unwrap().generation,
            Some(first.generation)
        );
        unsafe {
            assert_eq!(app.ime_request(IMR_CONFIRMRECONVERTSTRING, ptr).0, 1);
        }
        assert_eq!(app.reconversion_owner.unwrap().request_id, request);
        assert_eq!(app.ime_session, Some(first));
        app.start_composition();
        assert_ne!(app.ime_session, Some(first));
        assert!(app.reconversion_owner.is_none());
        unsafe {
            assert_eq!(app.ime_request(IMR_CONFIRMRECONVERTSTRING, ptr).0, 0);
        }
    }
    #[test]
    fn ime_owned_cancel_filled_request_during_old_session_does_not_replace_active_owner() {
        let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
        let old = app.reconversion_owner.unwrap();
        let old_epoch = app.ime_session.unwrap();
        app.arm_reconversion();
        let pending = app.pending_reconversion.unwrap();
        assert_ne!(pending.request_id, old.request_id);
        assert_eq!(app.reconversion_owner.unwrap().request_id, old.request_id);
        // The old END cannot delete a later independently filled request.
        unsafe {
            app.native_packet(old_epoch, NativeNotification::End);
        }
        assert_eq!(
            app.pending_reconversion.unwrap().request_id,
            pending.request_id
        );
        app.start_composition();
        assert_ne!(app.ime_session, Some(old_epoch));
        assert_eq!(
            app.reconversion_owner.unwrap().request_id,
            pending.request_id
        );
        assert!(app.pending_reconversion.is_none());
    }
    #[test]
    fn ime_owned_cancel_result_plus_preedit_zero_flags_focus_and_editor_changes() {
        let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
        unsafe {
            app.apply_ime_update(ImeUpdate {
                flags: GCS_RESULTSTR.0 | GCS_COMPSTR.0,
                result: Some(String::new()),
                preedit: Some("next".into()),
                ..Default::default()
            });
        }
        assert!(app.reconversion_owner.is_none());
        assert_eq!(app.preedit, "next");
        let epoch = app.ime_session.unwrap();
        unsafe {
            app.native_packet(epoch, NativeNotification::Update(ImeUpdate::default()));
        }
        assert!(app.ime_session.is_none());
        assert!(app.ime_ingress.current.get().is_none());
        unsafe {
            app.native_packet(epoch, NativeNotification::End);
        }
        let mut app = ime_reconversion_test_app(BoundaryAffinity::After);
        let before = document_state(app.doc());
        let epoch = app.ime_session;
        let preedit = app.preedit.clone();
        unsafe {
            app.native_notification(NativeNotification::FocusLost);
        }
        assert_eq!(app.ime_session, epoch); // no HWND/HIMC: native failure
        assert_eq!(app.preedit, preedit);
        assert!(app.reconversion_owner.is_none());
        app.shell.compact_window = HWND(std::ptr::dangling_mut());
        app.invalidate_ime_editor();
        assert!(app.ime_session.is_none());
        assert_eq!(document_state(app.doc()), before);
    }
    #[test]
    fn ime_owned_cancel_main_paint_surface_keeps_compact_editor_ownership() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
        let mut app = state.app.borrow_mut();
        app.clear_composition();
        let main = app.hwnd;
        let compact = HWND(2usize as *mut _);
        app.shell.compact_window = compact;
        app.arm_reconversion();
        app.start_composition();
        app.hwnd = main; // incidental Main paint did not change editing_window
        let cancel = owned_cancel_snapshot(&app);
        assert_eq!(cancel.envelope.editor, compact);
        assert_ne!(cancel.envelope.editor, app.hwnd);
        let before = document_state(app.doc());
        unsafe {
            perform_reconversion_cancel(&state, &mut app, cancel, |editor| {
                assert_eq!(editor, compact);
                ImeCancelOutcome::Rejected
            });
        }
        assert_eq!(app.ime_session, Some(cancel.envelope));
        assert_eq!(document_state(app.doc()), before);
    }
    #[test]
    fn ime_owned_cancel_success_refreshes_current_compact_surface_after_main_paint() {
        let source = format!("{}日本語", "long text ".repeat(80));
        let foreground = unsafe { GetForegroundWindow() };
        let mut fixture = visible_paint_fixture(&source);
        let main = fixture.hwnd;
        let compact = fixture.create_compact();
        fixture.state.main_window.set(main);
        fixture.state.editor_window.set(compact);
        {
            let mut app = fixture.state.app.borrow_mut();
            app.shell.enabled = true;
            app.shell.main_window = main;
            app.shell.compact_window = compact;
            app.select_surface(compact);
            app.tabs[0].raw = true;
            let word_start = source.len() - "日本語".len();
            app.doc_mut()
                .set_selection_with_affinity(source.len(), word_start, BoundaryAffinity::After)
                .unwrap();
            app.arm_reconversion();
            app.start_composition();
            app.preedit = "日本語".into();
            assert_eq!(app.editing_window(), compact);
            assert_eq!(app.ime_session.map(|session| session.editor), Some(compact));
        }
        unsafe {
            let _ = ShowWindow(compact, SW_SHOWNOACTIVATE);
            let mut main_client = RECT::default();
            let mut compact_client = RECT::default();
            GetClientRect(main, &mut main_client).unwrap();
            GetClientRect(compact, &mut compact_client).unwrap();
            assert!(compact_client.right < main_client.right);
            let _ = UpdateWindow(main);
            let _ = UpdateWindow(compact);
        }
        assert_eq!(unsafe { GetForegroundWindow() }, foreground);

        {
            let mut app = fixture.state.app.borrow_mut();
            assert_eq!(app.hwnd, compact);
            unsafe {
                app.refresh();
            }
        }
        unsafe {
            let _ = UpdateWindow(compact);
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            app.select_surface(main);
            unsafe {
                app.refresh();
            }
        }
        unsafe {
            let _ = UpdateWindow(main);
        }

        let (cancel, main_caret, main_layout, before) = {
            let app = fixture.state.app.borrow();
            assert_eq!(app.hwnd, main);
            assert_eq!(app.editing_window(), compact);
            assert!(
                app.ime_session
                    .is_some_and(|session| session.editor == compact)
            );
            assert!(!unsafe { GetUpdateRect(compact, None, false).as_bool() });
            (
                owned_cancel_snapshot(&app),
                app.caret,
                app.layout_key,
                document_state(app.doc()),
            )
        };
        assert_eq!(cancel.envelope.editor, compact);

        {
            let mut app = fixture.state.app.borrow_mut();
            unsafe {
                perform_reconversion_cancel(&fixture.state, &mut app, cancel, |editor| {
                    assert_eq!(editor, compact);
                    ImeCancelOutcome::Cancelled
                });
            }
            assert_eq!(document_state(app.doc()), before);
            assert!(app.ime_session.is_none());
            assert!(!app.composing);
            assert_eq!(app.hwnd, compact, "completion refresh stayed on Main");
            assert!(
                app.full_paint,
                "Compact surface switch did not require a full paint"
            );
            assert_ne!(
                app.layout_key, main_layout,
                "Compact layout was not recomputed"
            );
            assert_ne!(app.caret, main_caret, "Compact caret was not refreshed");
        }
        unsafe {
            assert!(
                GetUpdateRect(compact, None, false).as_bool(),
                "successful cancellation did not invalidate Compact"
            );
            assert!(
                !GetUpdateRect(main, None, false).as_bool(),
                "successful cancellation invalidated Main instead of Compact"
            );
            assert_eq!(GetForegroundWindow(), foreground);
        }
    }
    #[test]
    fn ime_owned_cancel_pending_request_zero_flags_and_end_are_idempotent() {
        for message in [WM_IME_COMPOSITION, WM_IME_ENDCOMPOSITION] {
            let mut app = ime_reconversion_test_app(BoundaryAffinity::After);
            app.clear_composition();
            app.arm_reconversion();
            assert!(app.pending_reconversion.is_some());
            let envelope = app
                .ime_ingress
                .capture(app.doc().id(), app.editing_window(), message);
            let notification = if message == WM_IME_COMPOSITION {
                NativeNotification::Update(ImeUpdate::default())
            } else {
                NativeNotification::End
            };
            unsafe {
                app.native_packet(envelope, notification);
            }
            assert!(app.pending_reconversion.is_none());
            unsafe {
                app.native_packet(envelope, NativeNotification::End);
            }
        }
    }
    #[test]
    fn ime_owned_cancel_superseded_recovered_escape_cannot_suppress_new_session_result() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
        let captured = state.ime_ingress.current.get();
        let input = owned_cancel_input(&state.app.borrow(), VK_ESCAPE.0);
        {
            let mut app = state.app.borrow_mut();
            app.arm_reconversion();
            let fresh = state.ime_ingress.capture(
                app.doc().id(),
                app.editing_window(),
                WM_IME_STARTCOMPOSITION,
            );
            state
                .notifications
                .borrow_mut()
                .push_back((fresh, NativeNotification::Start));
            unsafe {
                drain_native_notifications(&state, &mut app);
            }
            assert!(app.reconversion_owner.is_some());
            assert!(
                app.prepare_reconversion_cancel(captured, input, app.editing_window())
                    .is_none()
            );
        }
        let current_input = state.ime_input_for_ingress(captured, input);
        assert!(current_input.is_none());
        let message = MSG {
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_PROCESSKEY.0 as usize),
            ..Default::default()
        };
        let _scope = state.scoped_ime_input(current_input, &message);
        let mut app = state.app.borrow_mut();
        let fresh = app.ime_session.unwrap();
        unsafe {
            app.native_packet(
                fresh,
                NativeNotification::Update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("日本語".into()),
                    input_context: state.ime_input.get(),
                    ..Default::default()
                }),
            );
        }
        assert_eq!(app.doc().selection(), Selection::caret(15));
        assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);
        assert!(app.reconversion_owner.is_none());
    }
    #[test]
    fn ime_owned_cancel_fresh_start_retires_prior_reconversion() {
        let mut app = ime_reconversion_test_app(BoundaryAffinity::Before);
        assert!(app.reconversion_owner.is_some());
        app.start_composition();
        assert!(
            app.reconversion_owner.is_none(),
            "fresh START inherited cancellation eligibility"
        );
    }
    #[test]
    fn ime_owned_cancel_empty_result_retires_reconversion() {
        let mut app = ime_reconversion_test_app(BoundaryAffinity::After);
        unsafe {
            app.apply_ime_update(ImeUpdate {
                flags: GCS_RESULTSTR.0,
                result: Some(String::new()),
                ..Default::default()
            })
        };
        assert!(
            app.reconversion_owner.is_none(),
            "empty result retained cancellation eligibility"
        );
    }
    #[test]
    fn ime_reconversion_pending_request_survives_keyframe_before_start() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            for stale in [false, true] {
                let mut app = composing_test_app("prefix日本語");
                app.clear_composition();
                app.tabs[0].raw = true;
                app.doc_mut()
                    .set_selection_with_affinity(15, 6, affinity)
                    .unwrap();
                app.arm_reconversion();
                assert!(!app.composing);
                if stale {
                    app.doc_mut()
                        .set_selection_with_affinity(6, 15, affinity)
                        .unwrap();
                }
                let before = document_state(app.doc());
                // This is the production validation used after a new key frame
                // drains older notifications, before START has been received.
                app.invalidate_stale_reconversion();
                assert_eq!(app.pending_reconversion.is_some(), !stale);
                app.start_composition();
                unsafe {
                    app.apply_ime_update(ImeUpdate {
                        flags: GCS_RESULTSTR.0,
                        result: Some("日本語".into()),
                        input_context: Some(ImeInputContext {
                            document_id: app.doc().id(),
                            virtual_key: VK_ESCAPE.0 as u32,
                        }),
                        ..Default::default()
                    });
                    app.native_notification(NativeNotification::End);
                }
                if stale {
                    assert_eq!(app.doc().selection(), Selection::caret(15));
                } else {
                    assert_eq!(document_state(app.doc()), before);
                    assert_eq!(app.doc().selection_affinity(), affinity);
                }
            }
        }
    }
    #[test]
    fn ime_reconversion_escape_original_result_preserves_direction_affinity_and_history() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            let mut app = ime_reconversion_test_app(affinity);
            let before = document_state(app.doc());
            unsafe {
                app.apply_ime_update(ImeUpdate {
                    flags: GCS_RESULTSTR.0
                        | GCS_RESULTREADSTR.0
                        | GCS_RESULTREADCLAUSE.0
                        | GCS_RESULTCLAUSE.0,
                    result: Some("日本語".into()),
                    input_context: Some(ImeInputContext {
                        document_id: app.doc().id(),
                        virtual_key: VK_ESCAPE.0 as u32,
                    }),
                    ..Default::default()
                });
                app.native_notification(NativeNotification::End);
            }
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(app.doc().selection_affinity(), affinity);
            assert!(app.reconversion_owner.is_none());
            assert!(!app.composing);
        }
    }
    #[test]
    fn ime_reconversion_enter_unknown_nonreconversion_and_stale_context_commit_normally() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            for case in 0..9 {
                let mut app = ime_reconversion_test_app(affinity);
                let before = document_state(app.doc());
                let mut input = Some(ImeInputContext {
                    document_id: app.doc().id(),
                    virtual_key: VK_ESCAPE.0 as u32,
                });
                match case {
                    0 => input.as_mut().unwrap().virtual_key = VK_RETURN.0 as u32,
                    1 => input = None,
                    2 => input.as_mut().unwrap().virtual_key = VK_PROCESSKEY.0 as u32,
                    3 => input.as_mut().unwrap().virtual_key = 0,
                    4 => app.reconversion_owner = None,
                    5 => input.as_mut().unwrap().document_id += 1,
                    6 => app.reconversion_owner.as_mut().unwrap().revision += 1,
                    7 => {
                        app.reconversion_owner.as_mut().unwrap().affinity = match affinity {
                            BoundaryAffinity::Before => BoundaryAffinity::After,
                            BoundaryAffinity::After => BoundaryAffinity::Before,
                        }
                    }
                    8 => {
                        app.reconversion_owner.as_mut().unwrap().selection = Selection {
                            anchor: 6,
                            head: 15,
                        }
                    }
                    _ => unreachable!(),
                }
                unsafe {
                    app.apply_ime_update(ImeUpdate {
                        flags: GCS_RESULTSTR.0,
                        result: Some("日本語".into()),
                        input_context: input,
                        ..Default::default()
                    });
                }
                assert_eq!(app.doc().selection(), Selection::caret(15), "case {case}");
                assert_eq!(app.doc().selection_affinity(), BoundaryAffinity::After);
                assert_eq!(app.doc().source(), before.0);
                assert_eq!(app.doc().revision(), before.2);
                assert_eq!(app.doc().is_dirty(), before.3);
                assert_eq!(
                    (app.doc().undo_count(), app.doc().redo_count()),
                    (before.4, before.5)
                );
                assert!(app.reconversion_owner.is_none());
            }
        }
    }
    #[test]
    fn ime_reconversion_candidate_escape_then_enter_uses_owned_notification_key() {
        let state = WindowState::new(ime_reconversion_test_app(BoundaryAffinity::Before));
        let id = state.document_id.get();
        let message = MSG {
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_PROCESSKEY.0 as usize),
            ..Default::default()
        };
        {
            let _scope = state.scoped_ime_input(
                Some(ImeInputContext {
                    document_id: id,
                    virtual_key: VK_ESCAPE.0 as u32,
                }),
                &message,
            );
            let mut app = state.app.borrow_mut();
            let before = document_state(app.doc());
            unsafe {
                app.apply_ime_update(ImeUpdate {
                    flags: GCS_COMPSTR.0,
                    preedit: Some("にほんご".into()),
                    input_context: state.ime_input.get(),
                    ..Default::default()
                });
            }
            assert_eq!(document_state(app.doc()), before);
        }
        assert!(state.ime_input.get().is_none());
        {
            let _scope = state.scoped_ime_input(
                Some(ImeInputContext {
                    document_id: id,
                    virtual_key: VK_RETURN.0 as u32,
                }),
                &message,
            );
            state.notifications.borrow_mut().push_back((
                state.ime_ingress.current.get().unwrap(),
                NativeNotification::Update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("日本語".into()),
                    input_context: state.ime_input.get(),
                    ..Default::default()
                }),
            ));
        }
        // Draining occurs outside the next key's eligible scope. The already
        // owned Enter snapshot remains Enter after its own scope has ended.
        assert!(state.ime_input.get().is_none());
        unsafe { drain_native_notifications(&state, &mut state.app.borrow_mut()) };
        assert_eq!(state.app.borrow().doc().selection(), Selection::caret(15));
        assert!(state.ime_input.get().is_none());
    }
    #[test]
    fn ime_reconversion_late_callback_and_nested_pumps_have_no_escape_attribution() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            for phase in 0..5 {
                let state = WindowState::new(ime_reconversion_test_app(affinity));
                let message = MSG {
                    message: WM_KEYDOWN,
                    wParam: WPARAM(VK_PROCESSKEY.0 as usize),
                    ..Default::default()
                };
                let captured_escape = Some(ImeInputContext {
                    document_id: state.document_id.get(),
                    virtual_key: VK_ESCAPE.0 as u32,
                });
                {
                    let _old_scope = state.scoped_ime_input(captured_escape, &message);
                    assert!(state.ime_input.get().is_some());
                }
                assert!(state.ime_input.get().is_none());
                // Phase 0: callback after dispatch / during the next retrieval.
                // Phase 1: a later Escape is captured locally, but its drain and
                // pretranslation helpers must still have no eligible context.
                let _new_scope = if phase >= 2 {
                    Some(state.scoped_ime_input(captured_escape, &message))
                } else {
                    None
                };
                if phase == 2 {
                    state.observe_ime_input_message(
                        HWND::default(),
                        WM_TIMER,
                        WPARAM(0),
                        LPARAM(0),
                    );
                } else if phase == 3 {
                    // The outer dispatch key is eligible once; a nested key,
                    // even with the same raw values, invalidates attribution.
                    state.observe_ime_input_message(
                        message.hwnd,
                        message.message,
                        message.wParam,
                        message.lParam,
                    );
                    assert!(state.ime_input.get().is_some());
                    state.observe_ime_input_message(
                        message.hwnd,
                        message.message,
                        message.wParam,
                        message.lParam,
                    );
                }
                if phase == 4 {
                    state.observe_ime_input_message(
                        message.hwnd,
                        message.message,
                        message.wParam,
                        message.lParam,
                    );
                }
                assert_eq!(state.ime_input.get().is_some(), phase == 4);
                let mut app = state.app.borrow_mut();
                let before = document_state(app.doc());
                unsafe {
                    app.apply_ime_update(ImeUpdate {
                        flags: GCS_RESULTSTR.0,
                        result: Some("日本語".into()),
                        input_context: state.ime_input.get(),
                        ..Default::default()
                    });
                }
                if phase == 4 {
                    assert_eq!(document_state(app.doc()), before);
                    assert_eq!(app.doc().selection_affinity(), affinity);
                } else {
                    assert_eq!(app.doc().selection(), Selection::caret(15), "phase {phase}");
                }
                assert_eq!(app.doc().source(), before.0);
                assert_eq!(app.doc().revision(), before.2);
                assert_eq!(
                    (app.doc().undo_count(), app.doc().redo_count()),
                    (before.4, before.5)
                );
            }
        }
    }
    #[test]
    fn ime_reconversion_changed_result_keeps_one_undo_redo_and_original_affinity() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            let mut app = ime_reconversion_test_app(affinity);
            let original_selection = app.doc().selection();
            let undo = app.doc().undo_count();
            unsafe {
                app.apply_ime_update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("にほんご".into()),
                    input_context: Some(ImeInputContext {
                        document_id: app.doc().id(),
                        virtual_key: VK_ESCAPE.0 as u32,
                    }),
                    ..Default::default()
                });
                app.native_notification(NativeNotification::End);
            }
            assert_eq!(app.doc().source(), "prefixにほんご");
            assert_eq!(app.doc().undo_count(), undo + 1);
            assert!(app.doc_mut().undo());
            assert_eq!(app.doc().source(), "prefix日本語");
            assert_eq!(app.doc().selection(), original_selection);
            assert_eq!(app.doc().selection_affinity(), affinity);
            assert!(app.doc_mut().redo());
            assert_eq!(app.doc().source(), "prefixにほんご");
            assert_eq!(app.doc().selection(), Selection::caret(18));
        }
    }
    #[test]
    fn ime_reconversion_plain_cancel_focus_loss_and_stale_composition_preserve_source() {
        for affinity in [BoundaryAffinity::Before, BoundaryAffinity::After] {
            let mut app = ime_reconversion_test_app(affinity);
            let before = document_state(app.doc());
            unsafe { app.apply_ime_update(ImeUpdate::default()) };
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(app.doc().selection_affinity(), affinity);
            assert!(app.reconversion_owner.is_none());

            let mut app = ime_reconversion_test_app(affinity);
            let before = document_state(app.doc());
            unsafe { app.native_notification(NativeNotification::FocusLost) };
            assert_eq!(document_state(app.doc()), before);
            assert_eq!(app.doc().selection_affinity(), affinity);
            assert!(app.reconversion_owner.is_none());
            assert!(ime_input_resets(WM_KILLFOCUS));

            app.ime_owner = None;
            unsafe {
                app.apply_ime_update(ImeUpdate {
                    flags: GCS_RESULTSTR.0,
                    result: Some("日本語".into()),
                    input_context: Some(ImeInputContext {
                        document_id: app.doc().id(),
                        virtual_key: VK_ESCAPE.0 as u32,
                    }),
                    ..Default::default()
                });
            }
            assert_eq!(document_state(app.doc()), before);
            assert!(app.reconversion_owner.is_none());
        }
        for message in [
            WM_MOUSEMOVE,
            WM_LBUTTONDOWN,
            WM_COMMAND,
            WM_IME_ENDCOMPOSITION,
        ] {
            assert!(ime_input_resets(message));
        }
        for message in [WM_IME_COMPOSITION, WM_PAINT, WM_TIMER, WM_KEYUP] {
            assert!(!ime_input_resets(message));
        }
    }
    #[test]
    fn ime_cancel_notification_and_end_clear_only_local_composition() {
        for flags in [0, CS_NOMOVECARET, CS_INSERTCHAR] {
            let mut app = composing_test_app("before\r\n日本語 👩‍💻\r\nafter");
            app.doc_mut().set_selection(8, 17).unwrap();
            let selection = app.doc().selection();
            app.ime_owner = Some((1, app.doc().revision(), selection.anchor, selection.head));
            app.tabs.push(Tab {
                doc: Document::new_unsaved(2, "other unsaved tab".into()),
                file: None,
                scroll: 0.0,
                raw: false,
                table_scroll: HashMap::new(),
                persistence: DocumentPersistence::default(),
            });
            let before: Vec<_> = app
                .tabs
                .iter()
                .map(|tab| document_state(&tab.doc))
                .collect();
            unsafe { app.composition(flags) };
            assert!(!app.composing);
            assert!(app.preedit.is_empty());
            assert_eq!(app.ime_owner, None);
            assert_eq!(app.ime_cursor, 0);
            assert_eq!(app.active, 0);
            assert_eq!(app.tabs[0].scroll, 17.0);
            let after: Vec<_> = app
                .tabs
                .iter()
                .map(|tab| document_state(&tab.doc))
                .collect();
            assert_eq!(after, before);
            // A subsequent end notification is idempotent; it cannot erase or
            // manufacture a result transaction or disturb the other tab.
            app.clear_composition();
            assert_eq!(app.doc().redo_count(), 1);
            assert_eq!(document_state(app.doc()), before[0]);
        }
    }
    #[test]
    fn ime_cancellation_does_not_misclassify_result_preedit_or_cursor_flags() {
        for flag in [
            GCS_COMPATTR,
            GCS_COMPCLAUSE,
            GCS_COMPREADSTR,
            GCS_COMPREADATTR,
            GCS_COMPREADCLAUSE,
            GCS_COMPSTR,
            GCS_CURSORPOS,
            GCS_DELTASTART,
            GCS_RESULTCLAUSE,
            GCS_RESULTREADCLAUSE,
            GCS_RESULTREADSTR,
            GCS_RESULTSTR,
        ] {
            assert!(!composition_is_cancelled(flag.0));
            assert!(!composition_is_cancelled(flag.0 | CS_NOMOVECARET));
        }
    }
    #[test]
    fn focus_loss_preserves_pending_composition_without_native_context() {
        // Exercise the production handler on a hidden owned window. Removing
        // only its context makes cancellation fail without desktop input.
        unsafe {
            struct Window(HWND, HIMC);
            impl Drop for Window {
                fn drop(&mut self) {
                    unsafe {
                        SetWindowLongPtrW(self.0, GWLP_USERDATA, 0);
                        ImmAssociateContext(self.0, self.1);
                        let _ = DestroyWindow(self.0);
                    }
                }
            }
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("hidden focus-loss preservation test"),
                WINDOW_STYLE::default(),
                0,
                0,
                1,
                1,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .unwrap();
            let window = Window(hwnd, ImmAssociateContext(hwnd, HIMC::default()));
            assert!(!IsWindowVisible(window.0).as_bool());
            assert!(ImmGetContext(window.0).0.is_null());
            let mut app = composing_test_app("before\r\n日本語 👩‍💻\r\nafter");
            let selection = app.doc().selection();
            app.ime_owner = Some((1, app.doc().revision(), selection.anchor, selection.head));
            app.dragging = true;
            let before = document_state(app.doc());
            let owner = app.ime_owner;
            let app = WindowState::new(app);
            SetWindowLongPtrW(
                window.0,
                GWLP_USERDATA,
                (&app as *const WindowState) as isize,
            );
            window_proc(window.0, WM_KILLFOCUS, WPARAM(0), LPARAM(0));
            let app = app.app.borrow();
            assert!(app.composing);
            assert_eq!(app.preedit, "かな");
            assert_eq!(app.ime_owner, owner);
            assert_eq!(app.ime_cursor, 2);
            assert_eq!(document_state(app.doc()), before);
            assert!(!app.dragging);
            assert_eq!(app.native_caret.size, None);
            assert!(!app.native_caret.visible);
        }
    }
    #[test]
    fn ime_failed_cancel_and_composing_escape_enter_preserve_table_and_history() {
        let source = "| H | J |\r\n| --- | --- |\r\n| AA | BB |";
        let mut app = composing_test_app(source);
        let table = mdlite::table::find_tables(source).remove(0);
        let cell = table.rows[1].cells[1].content_range.clone();
        app.doc_mut().set_selection(cell.start, cell.end).unwrap();
        app.ime_owner = Some((1, app.doc().revision(), cell.start, cell.end));
        app.tables.push(TableView {
            table: Arc::new(table),
            y: TOP,
            height: 60.0,
            width: 200.0,
        });
        assert!(app.active_cell().is_some());
        assert_eq!(
            table_key_intent(VK_ESCAPE.0, false, false),
            Some(TableKeyIntent::Exit)
        );
        let before = document_state(app.doc());
        let owner = app.ime_owner;
        // This fixture has no HWND/context, so cancellation must fail without
        // pretending the native IME has relinquished its pending composition.
        unsafe {
            assert!(!app.cancel_composition());
            app.key(VK_ESCAPE.0);
            app.key(VK_RETURN.0);
        }
        assert!(app.composing);
        assert_eq!(app.preedit, "かな");
        assert_eq!(app.ime_owner, owner);
        assert_eq!(app.ime_cursor, 2);
        assert_eq!(document_state(app.doc()), before);
        assert_eq!(app.tables.len(), 1);
    }
    #[test]
    fn ime_default_key_forwarding_is_posted_without_input_or_activation() {
        use std::cell::Cell;
        #[derive(Default)]
        struct Probe {
            app_borrow: RefCell<()>,
            hold_borrow: Cell<bool>,
            generated_keys: Cell<usize>,
            blocked_keys: Cell<usize>,
        }
        unsafe extern "system" fn probe_proc(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
        ) -> LRESULT {
            unsafe {
                if msg == WM_NCCREATE {
                    let cs = &*(lp.0 as *const CREATESTRUCTW);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
                }
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Probe;
                if !pointer.is_null() {
                    let probe = &*pointer;
                    if matches!(msg, WM_IME_KEYDOWN | WM_IME_KEYUP) {
                        let _borrow = probe
                            .hold_borrow
                            .get()
                            .then(|| probe.app_borrow.borrow_mut());
                        return DefWindowProcW(hwnd, msg, wp, lp);
                    }
                    if matches!(msg, WM_KEYDOWN | WM_KEYUP) {
                        probe.generated_keys.set(probe.generated_keys.get() + 1);
                        if probe.app_borrow.try_borrow_mut().is_err() {
                            probe.blocked_keys.set(probe.blocked_keys.get() + 1);
                        }
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
        }
        struct Window(HWND, HINSTANCE);
        impl Drop for Window {
            fn drop(&mut self) {
                unsafe {
                    let _ = DestroyWindow(self.0);
                    let _ = UnregisterClassW(w!("MDLiteImeForwardingTest"), Some(self.1));
                }
            }
        }
        // This hidden window receives only direct test messages. It is never
        // shown, focused, activated, or sent OS keyboard/clipboard input.
        unsafe {
            let probe = Probe::default();
            let instance = HINSTANCE(GetModuleHandleW(None).unwrap().0);
            let class = WNDCLASSW {
                lpfnWndProc: Some(probe_proc),
                hInstance: instance,
                lpszClassName: w!("MDLiteImeForwardingTest"),
                ..Default::default()
            };
            assert_ne!(RegisterClassW(&class), 0);
            let window = Window(
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    class.lpszClassName,
                    w!("hidden IME forwarding test"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(instance),
                    Some((&probe as *const Probe).cast()),
                )
                .unwrap(),
                instance,
            );
            assert!(!IsWindowVisible(window.0).as_bool());
            for hold_borrow in [true, false] {
                probe.hold_borrow.set(hold_borrow);
                probe.generated_keys.set(0);
                probe.blocked_keys.set(0);
                SendMessageW(
                    window.0,
                    WM_IME_KEYDOWN,
                    Some(WPARAM(0x1b)),
                    Some(LPARAM(1)),
                );
                SendMessageW(
                    window.0,
                    WM_IME_KEYUP,
                    Some(WPARAM(0x1b)),
                    Some(LPARAM(0xC0000001)),
                );
                let synchronous = probe.generated_keys.get();
                let mut message = MSG::default();
                while PeekMessageW(
                    &mut message,
                    Some(window.0),
                    WM_KEYDOWN,
                    WM_KEYUP,
                    PM_REMOVE,
                )
                .as_bool()
                {
                    DispatchMessageW(&message);
                }
                eprintln!(
                    "hold_borrow={hold_borrow} synchronous={synchronous} generated={} blocked={}",
                    probe.generated_keys.get(),
                    probe.blocked_keys.get()
                );
                assert_eq!(synchronous, 0);
                assert_eq!(probe.generated_keys.get(), 2);
                assert_eq!(probe.blocked_keys.get(), 0);
            }
        }
    }
    #[test]
    fn keyboard_table_scroll_keeps_caret_inside_only_local_viewport_and_clamps() {
        assert_eq!(visible_table_offset(0.0, 450.0, 200.0, 600.0), 258.0);
        assert_eq!(visible_table_offset(300.0, 80.0, 200.0, 600.0), 74.0);
        assert_eq!(visible_table_offset(0.0, 700.0, 200.0, 600.0), 400.0);
        assert_eq!(visible_table_offset(30.0, 2.0, 200.0, 600.0), 0.0);
        assert_eq!(visible_table_offset(0.0, 90.0, 200.0, 100.0), 0.0);
        assert!(caret_in_editor(
            (LEFT + 192.0, TOP, 27.0),
            LEFT,
            TOP,
            LEFT + 200.0,
            600.0
        ));
        assert!(!caret_in_editor(
            (LEFT - 10.0, TOP, 27.0),
            LEFT,
            TOP,
            LEFT + 200.0,
            600.0
        ));
        assert!(!caret_in_editor(
            (LEFT + 210.0, TOP, 27.0),
            LEFT,
            TOP,
            LEFT + 200.0,
            600.0
        ));
        assert!(caret_in_editor(
            (335.8, 82.5, 23.5),
            335.8,
            shell::EDITOR_TOP,
            1084.,
            735.
        ));
        assert!(caret_in_editor(
            (24., 82.5, 23.5),
            24.,
            shell::EDITOR_TOP,
            776.,
            650.
        ));
        assert!(!caret_in_editor(
            (335.8, 79., 23.5),
            335.8,
            shell::EDITOR_TOP,
            1084.,
            735.
        ));
    }
    fn dispatch_table_key(
        doc: &mut Document,
        key: u16,
        ctrl: bool,
        shift: bool,
        boundaries: (Option<usize>, Option<usize>),
    ) {
        let p = doc.selection().head;
        let table = mdlite::table::find_tables(doc.source())
            .into_iter()
            .find(|table| table.cell_at(p).is_some())
            .unwrap();
        let intent = table_key_intent(key, ctrl, shift).unwrap();
        let action = table_key_action(&table, doc.source(), p, intent, boundaries).unwrap();
        apply_table_key_action(doc, action).unwrap();
    }
    #[test]
    fn table_tab_enter_and_shift_select_full_destination_content() {
        let source = "before\n\n| H | J |\n| --- | --- |\n| AA | BB |\n| CC | DD |\n\nafter";
        let table = mdlite::table::find_tables(source).remove(0);
        let a = table.rows[1].cells[0].content_range.clone();
        let b = table.rows[1].cells[1].content_range.clone();
        let d = table.rows[2].cells[1].content_range.clone();
        let mut doc = Document::new(41, source.into());
        doc.set_selection(a.start, a.start).unwrap();
        dispatch_table_key(
            &mut doc,
            0x09,
            false,
            false,
            (Some(6), Some(source.find("after").unwrap())),
        );
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: b.start,
                head: b.end
            }
        );
        dispatch_table_key(&mut doc, 0x0d, false, false, (None, None));
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: d.start,
                head: d.end
            }
        );
        dispatch_table_key(&mut doc, 0x0d, false, true, (None, None));
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: b.start,
                head: b.end
            }
        );
        dispatch_table_key(&mut doc, 0x09, false, true, (None, None));
        assert_eq!(
            doc.selection(),
            Selection {
                anchor: a.start,
                head: a.end
            }
        );
        assert_eq!(doc.source(), source);
        assert_eq!(doc.undo_count(), 0);
        assert!(!doc.is_dirty());
    }
    #[test]
    fn end_tab_and_enter_append_once_with_correct_selected_column_and_eol() {
        for eol in ["\n", "\r\n"] {
            for (key, column) in [(0x09, 0), (0x0d, 1)] {
                let source = format!("| H | J |{eol}| --- | --- |");
                let table = mdlite::table::find_tables(&source).remove(0);
                let start = table.rows[0].cells[1].content_range.end;
                let mut doc = Document::new(42, source.clone());
                doc.set_selection(start, start).unwrap();
                dispatch_table_key(&mut doc, key, false, false, (None, None));
                assert_eq!(doc.source(), format!("{source}{eol}|  |  |"));
                let new_table = mdlite::table::find_tables(doc.source()).remove(0);
                assert_eq!(new_table.rows.len(), 2);
                let range = &new_table.rows[1].cells[column].content_range;
                assert_eq!(
                    doc.selection(),
                    Selection {
                        anchor: range.start,
                        head: range.end
                    }
                );
                let after = doc.selection();
                assert_eq!(doc.undo_count(), 1);
                assert!(doc.undo());
                assert_eq!(doc.source(), source);
                assert_eq!(doc.selection().head, start);
                assert!(doc.redo());
                assert_eq!(doc.selection(), after);
            }
        }
    }
    #[test]
    fn control_enter_and_escape_exit_to_existing_paragraph_or_create_eof_paragraph() {
        for (key, ctrl) in [(0x0d, true), (0x1b, false)] {
            let source = "| H | J |\n| --- | --- |\n\nafter";
            let table = mdlite::table::find_tables(source).remove(0);
            let p = table.rows[0].cells[0].content_range.start;
            let after = source.find("after").unwrap();
            let mut doc = Document::new(43, source.into());
            doc.set_selection(p, p).unwrap();
            dispatch_table_key(&mut doc, key, ctrl, false, (None, Some(after)));
            assert_eq!(doc.source(), source);
            assert_eq!(
                doc.selection(),
                Selection {
                    anchor: after,
                    head: after
                }
            );
            assert_eq!(doc.undo_count(), 0);
            for eol in ["\n", "\r\n"] {
                let source = format!("| H | J |{eol}| --- | --- |{eol}");
                let table = mdlite::table::find_tables(&source).remove(0);
                let p = table.rows[0].cells[0].content_range.start;
                let mut doc = Document::new(44, source.clone());
                doc.set_selection(p, p).unwrap();
                dispatch_table_key(&mut doc, key, ctrl, false, (None, Some(source.len())));
                assert_eq!(doc.source(), format!("{source}{eol}"));
                assert_eq!(doc.selection().head, doc.source().len());
                assert_eq!(doc.undo_count(), 1);
                let after = doc.selection();
                assert!(doc.undo());
                assert_eq!(doc.source(), source);
                assert_eq!(doc.selection().head, p);
                assert!(doc.redo());
                assert_eq!(doc.selection(), after);
            }
        }
    }
    #[test]
    fn backward_table_exit_creates_usable_before_marker_only_if_missing() {
        let source = "| H | J |\n| --- | --- |";
        for key in [0x09, 0x0d] {
            let table = mdlite::table::find_tables(source).remove(0);
            let p = table.rows[0].cells[0].content_range.start;
            let mut doc = Document::new(45, source.into());
            doc.set_selection(p, p).unwrap();
            dispatch_table_key(&mut doc, key, false, true, (None, None));
            assert_eq!(doc.source(), format!("\n\n{source}"));
            assert_eq!(doc.selection(), Selection { anchor: 0, head: 0 });
            assert_eq!(
                resolve_live_point(
                    &projected_tokens(doc.source()),
                    0,
                    &[0, doc.source().len()],
                    None
                ),
                0
            );
            assert_eq!(doc.undo_count(), 1);
            assert!(doc.undo());
            assert_eq!(doc.source(), source);
            assert_eq!(doc.selection().head, p);
        }
    }
    #[test]
    fn adjacent_tables_exit_into_real_blank_gap_without_mutation_and_typing_preserves_both_tables()
    {
        for eol in ["\n", "\r\n"] {
            let source =
                format!("| H | J |{eol}| --- | --- |{eol}{eol}| K | L |{eol}| --- | --- |");
            let tables = mdlite::table::find_tables(&source);
            assert_eq!(tables.len(), 2);
            let marker =
                table_gap_marker(&source, tables[0].range.end, tables[1].range.start).unwrap();
            let p = tables[0].rows[0].cells[0].content_range.start;
            let mut doc = Document::new(46, source.clone());
            doc.set_selection(p, p).unwrap();
            dispatch_table_key(&mut doc, 0x1b, false, false, (None, Some(marker)));
            assert_eq!(doc.source(), source);
            assert_eq!(doc.selection().head, marker);
            assert_eq!(doc.undo_count(), 0);
            let plan =
                table_gap_insertion(&tables[0], &tables[1], doc.source(), marker, "body").unwrap();
            apply_plan(&mut doc, plan);
            assert_eq!(
                doc.source(),
                format!(
                    "| H | J |{eol}| --- | --- |{eol}{eol}body{eol}{eol}| K | L |{eol}| --- | --- |"
                )
            );
            assert_eq!(mdlite::table::find_tables(doc.source()).len(), 2);
            assert_eq!(doc.selection().head, doc.source().find("body").unwrap() + 4);
            assert_eq!(doc.undo_count(), 1);
            assert!(doc.undo());
            assert_eq!(doc.source(), source);
        }
    }
    #[test]
    fn arrow_key_intents_cannot_append_or_edit_table_under_repeat() {
        let source = "| H | J |\n| --- | --- |\n| A | B |";
        let table = mdlite::table::find_tables(source).remove(0);
        let p = table.rows[0].cells[0].content_range.start;
        let mut doc = Document::new(47, source.into());
        doc.set_selection(p, p).unwrap();
        for _ in 0..20 {
            for key in [0x25, 0x26, 0x27, 0x28] {
                assert_eq!(table_key_intent(key, false, false), None);
                let target = table_navigation_target(
                    &table,
                    doc.rope(),
                    doc.selection().head,
                    key == 0x25 || key == 0x26,
                    key == 0x25 || key == 0x27,
                )
                .unwrap_or(doc.selection().head);
                doc.set_selection(target, target).unwrap();
            }
        }
        assert_eq!(doc.source(), source);
        assert!(!doc.is_dirty());
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.undo_count(), 0);
        assert_eq!(mdlite::table::find_tables(doc.source())[0].rows.len(), 2);
    }
    #[test]
    fn explicit_image_size_can_enlarge_intrinsic_and_preserves_ratio_and_viewport_limit() {
        assert_eq!(image_dimensions(32, 24, None, 500.0), (32.0, 24.0));
        assert_eq!(image_dimensions(32, 24, Some(40), 500.0), (40.0, 30.0));
        assert_eq!(image_dimensions(32, 24, Some(40), 36.0), (36.0, 27.0));
        assert_eq!(image_dimensions(320, 100, Some(384), 500.0), (384.0, 120.0));
    }
    #[test]
    fn native_caret_tracks_created_pixels_through_focus_heading_dpi_and_same_position() {
        let position = (44, 88);
        let normal = caret_pixel_size(FONT * 1.6, 1.0);
        let heading = caret_pixel_size(48.0, 1.0);
        let mut actual = NativeCaretState::default();
        assert_eq!(
            required_caret_change(actual, normal, position),
            CaretChange {
                recreate: true,
                move_position: true
            }
        );
        actual.size = Some(normal);
        actual.position = Some(position);
        assert_eq!(
            required_caret_change(actual, normal, position),
            CaretChange {
                recreate: false,
                move_position: false
            }
        );
        assert_eq!(
            required_caret_change(actual, heading, position),
            CaretChange {
                recreate: true,
                move_position: true
            }
        );
        actual.size = Some(heading);
        actual.position = Some(position);
        assert_eq!(
            required_caret_change(actual, heading, (61, 88)),
            CaretChange {
                recreate: false,
                move_position: true
            }
        );
        actual = NativeCaretState::default();
        assert_eq!(
            required_caret_change(actual, heading, position),
            CaretChange {
                recreate: true,
                move_position: true
            }
        );
        actual.size = Some(heading);
        actual.position = Some(position);
        let new_dpi = caret_pixel_size(48.0, 1.5);
        assert_eq!(new_dpi, (3, 72));
        assert_eq!(
            required_caret_change(actual, new_dpi, (66, 132)),
            CaretChange {
                recreate: true,
                move_position: true
            }
        );
        actual.size = Some(new_dpi);
        actual.position = Some((66, 132));
        assert_eq!(
            required_caret_change(actual, new_dpi, (66, 132)),
            CaretChange {
                recreate: false,
                move_position: false
            }
        );
        assert!(!caret_intersects_damage(
            (44.0, 88.0, 48.0),
            rect(200.0, 400.0, 100.0, 100.0)
        ));
        assert!(caret_intersects_damage(
            (44.0, 88.0, 48.0),
            rect(43.0, 87.0, 5.0, 50.0)
        ));
    }
    #[test]
    fn horizontal_arrows_cross_adjacent_eol_and_enter_front_matter_delimiters() {
        for eol in ["\n", "\r\n"] {
            let source = format!("hello{eol}");
            let tokens = projected_tokens(&source);
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], source.len(), true),
                5
            );
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], 5, false),
                source.len()
            );
            let source = format!("a{eol}{eol}b");
            let tokens = projected_tokens(&source);
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], 1 + 2 * eol.len(), true),
                1 + eol.len()
            );
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], 1, false),
                1 + eol.len()
            );
        }
        let source = "---\ntitle: T\n---\n";
        let tokens = projected_tokens(source);
        assert_eq!(live_horizontal_step(source, &tokens, &[], 4, true), 3);
    }
    fn apply_plan(doc: &mut Document, plan: LiveEdit) {
        doc.replace_range_with_selection(
            plan.range,
            &plan.replacement,
            Selection {
                anchor: plan.caret,
                head: plan.caret,
            },
        )
        .unwrap();
    }
    #[test]
    fn eof_enter_backspace_removes_only_one_atomic_lf_or_crlf_with_exact_history() {
        for eol in ["\n", "\r\n"] {
            let mut doc = Document::new(31, "hello".into());
            doc.set_selection(5, 5).unwrap();
            let tokens = projected_tokens(doc.source());
            let enter = live_edit(doc.source(), &tokens, 5..5, eol).unwrap();
            apply_plan(&mut doc, enter);
            assert_eq!(doc.source(), format!("hello{eol}"));
            let after_enter = doc.selection();
            let deletion =
                line_ending_delete(doc.source(), &[], doc.selection().head, true).unwrap();
            assert_eq!(deletion.range, 5..5 + eol.len());
            apply_plan(&mut doc, deletion);
            assert_eq!(doc.source(), "hello");
            assert_eq!(doc.selection(), Selection { anchor: 5, head: 5 });
            assert_eq!(doc.undo_count(), 2);
            assert!(doc.undo());
            assert_eq!(doc.source(), format!("hello{eol}"));
            assert_eq!(doc.selection(), after_enter);
            assert!(doc.redo());
            assert_eq!(doc.source(), "hello");
            assert_eq!(doc.selection().head, 5);
        }
    }
    #[test]
    fn paragraph_backspace_and_delete_join_one_physical_line_ending() {
        for eol in ["\n", "\r\n"] {
            let source = format!("a{eol}{eol}b");
            for (back, p) in [(true, 1 + eol.len() * 2), (false, 1)] {
                let mut doc = Document::new(32, source.clone());
                doc.set_selection(p, p).unwrap();
                let plan = line_ending_delete(&source, &[], p, back).unwrap();
                apply_plan(&mut doc, plan);
                assert_eq!(doc.source(), format!("a{eol}b"));
                assert_eq!(doc.undo_count(), 1);
                let after = doc.selection();
                assert!(doc.undo());
                assert_eq!(doc.source(), source);
                assert_eq!(doc.selection(), Selection { anchor: p, head: p });
                assert!(doc.redo());
                assert_eq!(doc.source(), format!("a{eol}b"));
                assert_eq!(doc.selection(), after);
            }
        }
        assert_eq!(adjacent_line_ending("a\r\nb", 2, true), Some(1..3));
        assert_eq!(adjacent_line_ending("a\r\nb", 2, false), Some(1..3));
    }
    #[test]
    fn table_separators_before_and_after_header_only_and_data_tables_are_protected() {
        for rows in ["", "| A | B |\n"] {
            let source = format!("before\n\n| H | J |\n| --- | --- |\n{rows}\nafter\n");
            let table = mdlite::table::find_tables(&source).remove(0);
            for (p, back) in [
                (table.range.start, true),
                (table.range.start - 1, false),
                (table.range.end, false),
                (table.range.end + 1, true),
            ] {
                let mut doc = Document::new(33, source.clone());
                doc.set_selection(p, p).unwrap();
                let plan = line_ending_delete(&source, std::slice::from_ref(&table.range), p, back)
                    .unwrap();
                assert!(plan.range.is_empty());
                apply_plan(&mut doc, plan);
                assert_eq!(doc.source(), source);
                assert_eq!(doc.revision(), 0);
                assert_eq!(doc.undo_count(), 0);
                assert_eq!(mdlite::table::find_tables(doc.source())[0], table);
            }
        }
    }
    #[test]
    fn first_fence_text_backspace_edits_visible_opening_line_ending_and_undo_restores_it() {
        let source = "```\nfirst\n```\n";
        let first = source.find("first").unwrap();
        let mut doc = Document::new(34, source.into());
        doc.set_selection(first, first).unwrap();
        let plan = line_ending_delete(source, &[], first, true).unwrap();
        assert_eq!(plan.range, 3..4);
        apply_plan(&mut doc, plan);
        assert_eq!(doc.source(), "```first\n```\n");
        assert_eq!(doc.selection().head, 3);
        assert_eq!(doc.undo_count(), 1);
        assert!(doc.undo());
        assert_eq!(doc.source(), source);
        assert_eq!(doc.selection().head, first);
    }
    #[test]
    fn real_image_token_and_exact_layout_markers_keep_before_after_insertions_and_undo() {
        let source = "![画像](image.png) <!-- mdlite-width:120 -->";
        let range = 0..source.len();
        let tokens = vec![image_token(source, range.clone())];
        let markers = [range.start, range.end];
        assert!(tokens[0].atomic);
        assert_eq!(tokens[0].spans.as_slice(), std::slice::from_ref(&range));
        for (p, text, expected) in [
            (0, "前", format!("前{source}")),
            (source.len(), "後", format!("{source}後")),
        ] {
            let point = resolve_live_point(&tokens, p, &markers, Some(0));
            assert_eq!(point, p);
            let mut doc = Document::new(35, source.into());
            doc.set_selection(p, p).unwrap();
            let plan = live_edit(source, &tokens, point..point, text).unwrap();
            apply_plan(&mut doc, plan);
            assert_eq!(doc.source(), expected);
            assert_eq!(doc.selection().head, p + text.len());
            assert_eq!(doc.undo_count(), 1);
            assert!(doc.undo());
            assert_eq!(doc.source(), source);
            assert_eq!(doc.selection(), Selection { anchor: p, head: p });
            assert!(doc.redo());
            assert_eq!(doc.source(), expected);
            assert_eq!(doc.selection().head, p + text.len());
        }
        assert_eq!(
            resolve_live_point(&[], source.len(), &markers, Some(0)),
            source.len()
        );
    }
    #[test]
    fn initial_live_insertion_respects_visible_frontmatter_and_hidden_heading_prefix() {
        {
            let (source, expected, first) = ("# ab", "# Xab", 2);
            let tokens = projected_tokens(source);
            let point = live_caret_boundary(&tokens, 0, &[]);
            assert_eq!(point, first);
            let mut doc = Document::new(27, source.into());
            doc.set_selection(point, point).unwrap();
            let edit = live_edit(source, &tokens, point..point, "X").unwrap();
            doc.replace_range_with_selection(
                edit.range,
                &edit.replacement,
                Selection {
                    anchor: edit.caret,
                    head: edit.caret,
                },
            )
            .unwrap();
            assert_eq!(doc.source(), expected);
            assert_eq!(doc.selection().head, first + 1);
            assert!(doc.undo());
            assert_eq!(doc.source(), source);
            assert_eq!(
                doc.selection(),
                Selection {
                    anchor: first,
                    head: first
                }
            );
            assert!(!doc.is_dirty());
        }
    }
    #[test]
    fn initial_front_matter_native_input_reveals_and_edits_the_opening_line() {
        let source = "---\ntitle: T\n---\n";
        let fixture = EditorFixture::new(source, false);
        fixture.message(WM_CHAR, 'X' as usize);
        let mut app = fixture.state.app.borrow_mut();
        assert_eq!(app.doc().source(), "X---\ntitle: T\n---\n");
        assert_eq!(app.doc().selection(), Selection::caret(1));
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().selection(), Selection::caret(0));
        assert!(!app.doc().is_dirty());
    }
    #[test]
    fn hidden_closing_boundary_and_atomic_spelling_resolve_to_visible_insertions() {
        let source = "**a**b";
        let tokens = projected_tokens(source);
        assert_eq!(live_caret_boundary(&tokens, 4, &[]), 3);
        assert_eq!(live_caret_boundary(&tokens, 3, &[]), 3);
        assert_eq!(live_caret_boundary(&tokens, 5, &[]), 5);
        assert_eq!(perform_live_edit(source, 3..3, "x"), "**ax**b");
        assert_eq!(perform_live_edit(source, 5..5, "x"), "**a**xb");
        let source = "&amp;z";
        let tokens = projected_tokens(source);
        assert_eq!(live_caret_boundary(&tokens, 3, &[]), 5);
        let source = "last\n";
        let tokens = projected_tokens(source);
        assert_eq!(
            live_caret_boundary(&tokens, source.len(), &[source.len()]),
            source.len()
        );
    }
    #[test]
    fn repeated_table_navigation_moves_cells_without_source_or_history_edits() {
        let source = "before\n\n| A | B |\n| --- | --- |\n| C | D |\n\nafter\n";
        let table = mdlite::table::find_tables(source).remove(0);
        let mut doc = Document::new(17, source.into());
        let first = table.rows[0].cells[0].content_range.start;
        doc.set_selection(first, first).unwrap();
        let expected = [
            table.rows[0].cells[1].content_range.start,
            table.rows[1].cells[0].content_range.start,
            table.rows[1].cells[1].content_range.start,
            table.range.end,
        ];
        for value in expected {
            let target =
                table_navigation_target(&table, doc.rope(), doc.selection().head, false, true)
                    .unwrap();
            assert_eq!(target, value);
            doc.set_selection(target, target).unwrap();
        }
        for _ in 0..20 {
            if let Some(target) =
                table_navigation_target(&table, doc.rope(), doc.selection().head, false, true)
            {
                doc.set_selection(target, target).unwrap();
            }
        }
        let b = table.rows[0].cells[1].content_range.start;
        assert_eq!(
            table_navigation_target(&table, doc.rope(), b, false, false),
            Some(table.rows[1].cells[1].content_range.start)
        );
        assert_eq!(doc.source(), source);
        assert_eq!(doc.revision(), 0);
        assert_eq!(doc.undo_count(), 0);
        assert_eq!(mdlite::table::find_tables(doc.source())[0].rows.len(), 2);
    }
    fn projected_tokens(source: &str) -> Vec<VisibleToken> {
        let projection = DisplayProjection::parse(source);
        let end = projection.text.encode_utf16().count();
        let (after, before) = projection_mapping(&projection, 0..end);
        visible_tokens(&projection.text, &after, &before, source)
    }
    fn perform_live_edit(source: &str, range: Range<usize>, replacement: &str) -> String {
        let tokens = projected_tokens(source);
        let edit = live_edit(source, &tokens, range, replacement)
            .unwrap_or_else(|| panic!("no visible edit for {source:?}; tokens={tokens:?}"));
        let mut result = source.to_owned();
        result.replace_range(edit.range, &edit.replacement);
        result
    }
    #[test]
    fn visible_delete_and_replace_preserve_emphasis_and_link_syntax() {
        assert_eq!(perform_live_edit("**a**b", 2..5, ""), "****b");
        assert_eq!(perform_live_edit("**a**b", 2..6, "x"), "**x**");
        let source = "[a](https://example.invalid)b";
        assert_eq!(
            perform_live_edit(source, 1..source.len() - 1, ""),
            "[](https://example.invalid)b"
        );
        assert_eq!(
            perform_live_edit(source, 1..source.len(), "x"),
            "[x](https://example.invalid)"
        );
    }
    #[test]
    fn visible_tokens_keep_entity_escape_and_combining_emoji_spelling() {
        let source = "&amp;\\|e\u{301}👨‍👩‍👧‍👦";
        let tokens = projected_tokens(source);
        assert_eq!(tokens.len(), 4);
        assert_eq!(tokens[0].spans.as_slice(), std::slice::from_ref(&(0..5)));
        assert_eq!(tokens[1].spans.as_slice(), std::slice::from_ref(&(5..7)));
        assert_eq!(tokens[2].spelling(source), "e\u{301}");
        assert_eq!(tokens[3].spelling(source), "👨‍👩‍👧‍👦");
        assert_eq!(perform_live_edit(source, 0..5, ""), "\\|e\u{301}👨‍👩‍👧‍👦");
        let emoji_start = source.find('👨').unwrap();
        assert_eq!(
            perform_live_edit(source, emoji_start..source.len(), "!"),
            "&amp;\\|e\u{301}!"
        );
    }
    #[test]
    fn expanded_entity_remains_one_atomic_source_spelling() {
        let source = "**&fjlig;** z";
        let tokens = projected_tokens(source);
        assert_eq!(
            tokens
                .iter()
                .map(|t| t.spelling(source))
                .collect::<String>(),
            "fj z"
        );
        assert_eq!(tokens[0].spans.as_slice(), std::slice::from_ref(&(2..9)));
        assert_eq!(tokens[1].spans.as_slice(), std::slice::from_ref(&(2..9)));
        assert_eq!(perform_live_edit(source, 2..9, "x"), "**x** z");
    }
    #[test]
    fn code_and_frontmatter_backslashes_remain_visible_content() {
        for source in ["`a\\*b`", "```\na\\*b\n```", "---\nfield: a\\*b\n---\n"] {
            let star = source.find('*').unwrap();
            assert_eq!(
                perform_live_edit(source, star..star + 1, ""),
                source.replacen('*', "", 1)
            );
            let slash = source.find('\\').unwrap();
            assert!(
                projected_tokens(source)
                    .iter()
                    .any(|token| token.spelling(source) == "\\"
                        && token.spans.len() == 1
                        && token.spans[0] == (slash..slash + 1))
            );
        }
        assert_eq!(
            perform_live_edit("a\\*b", 1..3, ""),
            "ab",
            "tokens={:?}",
            projected_tokens("a\\*b")
        );
    }
    #[test]
    fn cut_copies_visible_text_and_replacement_history_restores_exact_caret() {
        let source = "**a**[b](target)c";
        let tokens = projected_tokens(source);
        let selected = 2..source.len();
        assert_eq!(
            selected_token(&tokens, selected.clone())
                .map(|t| t.spelling(source))
                .collect::<String>(),
            "abc"
        );
        let edit = live_edit(source, &tokens, selected.clone(), "😀").unwrap();
        let mut document = Document::new(7, source.into());
        document
            .set_selection(selected.start, selected.end)
            .unwrap();
        document
            .replace_range_with_selection(
                edit.range,
                &edit.replacement,
                Selection {
                    anchor: edit.caret,
                    head: edit.caret,
                },
            )
            .unwrap();
        assert_eq!(document.source(), "**😀**[](target)");
        assert_eq!(document.selection(), Selection { anchor: 6, head: 6 });
        assert_eq!(document.undo_count(), 1);
        assert!(document.undo());
        assert_eq!(document.source(), source);
        assert_eq!(
            document.selection(),
            Selection {
                anchor: selected.start,
                head: selected.end
            }
        );
        assert!(document.redo());
        assert_eq!(document.selection(), Selection { anchor: 6, head: 6 });
    }
    #[test]
    fn combining_grapheme_across_style_boundary_preserves_hidden_gap() {
        let source = "**e**\u{301}z";
        let tokens = projected_tokens(source);
        assert_eq!(tokens[0].spelling(source), "e\u{301}");
        assert_eq!(tokens[0].spans.as_slice(), &[2..3, 5..7]);
        assert_eq!(perform_live_edit(source, 2..7, ""), "****z");
    }
    #[test]
    fn image_hit_edges_exclude_gutter_and_damage_respects_scroll_and_dpi() {
        let image = ImageView {
            bitmap: None,
            path: None,
            source: 3..9,
            x: LEFT + 9.5,
            y: 200.25,
            width: 13.25,
            height: 7.0,
            quoted: false,
            label: String::new(),
            load_failed: false,
        };
        assert!(image_contains_point(&image, image.x, image.y));
        assert!(image_contains_point(
            &image,
            image.x + image.width,
            image.y + image.height
        ));
        assert!(!image_contains_point(
            &image,
            LEFT + 1.0,
            image.y + image.height / 2.0
        ));

        let damage = image_damage_rect(&image, 11.0, 1.5);
        assert_eq!(
            (damage.left, damage.top, damage.right, damage.bottom),
            (80, 283, 101, 295)
        );

        let mut app = composing_test_app("0123456789");
        app.clear_composition();
        app.tabs[0].scroll = 0.0;
        app.images.push(image);
        let image = &app.images[0];
        assert_eq!(app.hit(image.x, image.y + 0.25), image.source.start);
        assert_eq!(
            app.hit(image.x + image.width, image.y + image.height - 0.25),
            image.source.end
        );
        assert_ne!(
            app.hit(LEFT + 1.0, image.y + image.height / 2.0),
            image.source.start
        );
    }

    #[test]
    fn animation_only_runs_for_intersecting_image() {
        assert!(image_intersects_viewport(100.0, 40.0, 0.0, 600.0));
        assert!(!image_intersects_viewport(700.0, 40.0, 0.0, 600.0));
        assert!(!image_intersects_viewport(10.0, 40.0, 0.0, 600.0));
        assert!(image_intersects_viewport(700.0, 40.0, 650.0, 600.0));
        assert!(!image_intersects_viewport(100.0, 40.0, 0.0, TOP));
    }
    #[test]
    fn dib_decode_checks_bounds_and_bottom_up_rows() {
        let mut dib = vec![0u8; 48];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24u16.to_le_bytes());
        dib[40..43].copy_from_slice(&[0, 0, 255]);
        dib[44..47].copy_from_slice(&[255, 0, 0]);
        let (w, h, rgba) = decode_dib(&dib).unwrap();
        assert_eq!((w, h), (1, 2));
        assert_eq!(rgba, [0, 0, 255, 255, 255, 0, 0, 255]);
        assert!(decode_dib(&dib[..47]).is_err());
        dib[16] = 1;
        assert!(decode_dib(&dib).is_err());
    }
    #[test]
    fn escaped_pipe_is_one_visible_source_token() {
        let (text, map) = unescape_cell("a\\|😀", 10);
        assert_eq!(text, "a|😀");
        assert_eq!(map, vec![10, 11, 13, 13, 17]);
        assert_eq!(&"a\\|😀"[map[1] - 10..map[2] - 10], "\\|");
    }
    #[test]
    fn navigation_preserves_graphemes_and_eof() {
        let source = "e\u{301}😀\r\n日本";
        assert_eq!(
            mdlite::coordinates::rope_next_grapheme_boundary(&crop::Rope::from(source), 0).unwrap(),
            3
        );
        assert_eq!(
            mdlite::coordinates::rope_previous_grapheme_boundary(&crop::Rope::from(source), 7)
                .unwrap(),
            3
        );
        assert_eq!(
            mdlite::coordinates::rope_next_grapheme_boundary(&crop::Rope::from(source), 7).unwrap(),
            9
        );
        assert_eq!(
            snap(&crop::Rope::from(source), source.len(), false),
            source.len()
        );
        assert_eq!(snap(&crop::Rope::from(source), 2, false), 0);
    }
    #[test]
    fn save_as_re_resolves_relative_images_and_plain_text_stays_literal() {
        let directory = std::env::temp_dir().join(format!(
            "mdlite-save-as-images-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = directory.join("a");
        let second = directory.join("b");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        for (dir, w, h) in [(&first, 40, 20), (&second, 80, 30)] {
            image::RgbaImage::from_pixel(w, h, image::Rgba([255, 0, 0, 255]))
                .save(dir.join("image.png"))
                .unwrap();
        }
        let source = "![fixture](image.png)";
        let mut app = composing_test_app(source);
        app.clear_composition();
        app.tabs[0].file = Some(
            FileSnapshot::create_new(
                first.join("note.md"),
                source,
                FileFormat::Utf8 { bom: false },
            )
            .unwrap(),
        );
        let projection = DisplayProjection::parse(source);
        let block = projection
            .blocks
            .iter()
            .find(|b| matches!(b.kind, BlockKind::Image { .. }))
            .unwrap();
        unsafe {
            app.layout_image(
                &projection,
                (block, true),
                "image.png",
                None,
                0..source.len(),
                (10000., 500.),
            )
            .unwrap();
        }
        assert_eq!((app.images[0].width, app.images[0].height), (40., 20.));
        app.layout_key = Some((
            app.doc().id(),
            app.doc().revision(),
            app.width.to_bits(),
            false,
        ));
        assert!(app.save_as_to(&second.join("note.md")));
        assert!(app.images.is_empty());
        assert!(app.image_cache.is_empty());
        assert!(app.layout_key.is_none());
        unsafe {
            app.layout_image(
                &projection,
                (block, true),
                "image.png",
                None,
                0..source.len(),
                (10000., 500.),
            )
            .unwrap();
        }
        assert_eq!((app.images[0].width, app.images[0].height), (80., 30.));
        assert_eq!(app.doc().source(), source);
        let literal = "# heading\n**not formatting**\n| a | b |\n|---|---|\n";
        for extension in ["ini", "csv", "cfg", "conf", "txt"] {
            let path = directory.join(format!("literal.{extension}"));
            std::fs::write(&path, literal).unwrap();
            app.open_path(path);
            assert!(!app.is_markdown_document());
            app.hwnd = hidden_test_window();
            unsafe {
                app.ensure_layout().unwrap();
                app.command(CMD_RAW);
            }
            assert!(app.tabs[app.active].raw);
            assert!(app.tables.is_empty());
            assert_eq!(app.doc().source(), literal);
            assert!(!app.doc().is_dirty());
            assert!(!app.editor_enabled(20));
            if extension == "ini" {
                let end = app.doc().byte_len();
                app.doc_mut().replace_range(end..end, "- literal").unwrap();
                let end = app.doc().byte_len();
                app.doc_mut().set_selection(end, end).unwrap();
                unsafe {
                    app.key_with_modifiers(VK_RETURN.0, false, false);
                }
                assert!(app.doc().source().ends_with("- literal\n"));
            }
            unsafe {
                DestroyWindow(app.hwnd).unwrap();
            }
            app.hwnd = HWND::default();
        }
        drop(app);
        let cleanup = std::fs::canonicalize(&directory).unwrap();
        let temporary_root = std::fs::canonicalize(std::env::temp_dir()).unwrap();
        assert_eq!(cleanup.parent(), Some(temporary_root.as_path()));
        assert_eq!(cleanup.file_name(), directory.file_name());
        std::fs::remove_dir_all(cleanup).unwrap();
    }
}
