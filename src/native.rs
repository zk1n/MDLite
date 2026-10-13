//! Win32 owns the window; the UI-thread App owns documents and derived layouts.
//! The boxed RefCell lives until the message loop exits. Modal dialog reentry
//! uses try_borrow_mut, so no nested callback can alias a mutable App reference.

use mdlite::{
    document::{Document, Selection},
    projection::{BlockKind, BoundaryAffinity, DisplayProjection, ProjectionBlock},
    storage::{FileFormat, FileSnapshot},
    table::{Table, TableAlignment, TableEdit, TableRect},
};
use std::{
    cell::RefCell, collections::HashMap, ops::Range, path::PathBuf, sync::Arc, time::Instant,
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
const FONT: f32 = 17.0;
const CF_TEXT_W: u32 = 13;
const CMD_NEW: usize = 1;
const CMD_OPEN: usize = 2;
const CMD_SAVE: usize = 3;
const CMD_SAVE_AS: usize = 4;
const CMD_CLOSE: usize = 5;
const CMD_RAW: usize = 6;
const ANIMATION_TIMER: usize = 1;
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
}
struct TextView {
    layout: IDWriteTextLayout,
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
    target: Option<ID2D1HwndRenderTarget>,
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
struct App {
    hwnd: HWND,
    graphics: Graphics,
    tabs: Vec<Tab>,
    active: usize,
    next_id: u64,
    views: Vec<TextView>,
    visible_tokens: Vec<VisibleToken>,
    literal_blocks: Vec<Range<usize>>,
    tables: Vec<TableView>,
    images: Vec<ImageView>,
    image_cache: HashMap<PathBuf, CachedImage>,
    minimized: bool,
    animation_timer_armed: bool,
    height: f32,
    width: f32,
    dpi: f32,
    layout_key: Option<(u64, u64, u32, bool)>,
    projection_cache: Option<(u64, u64, Arc<DisplayProjection>)>,
    parsed_tables: Vec<Arc<Table>>,
    dragging: bool,
    dragging_scroll: bool,
    preedit: String,
    composing: bool,
    ime_owner: Option<(u64, u64, usize, usize)>,
    ime_cursor: u32,
    suppress_chars: Vec<u16>,
    pending_surrogate: Option<u16>,
    diagnostic: Option<PathBuf>,
    caret: (f32, f32, f32),
    native_caret: NativeCaretState,
    status: String,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
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
fn caret_intersects_damage(caret: (f32, f32, f32), damage: D2D_RECT_F) -> bool {
    damage.left <= caret.0 + 2.0
        && damage.right >= caret.0
        && damage.top <= caret.1 + caret.2
        && damage.bottom >= caret.1
}
fn caret_in_editor(caret: (f32, f32, f32), right: f32, bottom: f32) -> bool {
    caret.0 >= LEFT && caret.0 + 2.0 <= right && caret.1 >= TOP && caret.1 + caret.2 <= bottom
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

pub fn run() -> windows::core::Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let instance = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_IBEAM)?,
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
        let mut app = Box::new(RefCell::new(App {
            hwnd: HWND::default(),
            graphics,
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            views: Vec::new(),
            visible_tokens: Vec::new(),
            literal_blocks: Vec::new(),
            tables: Vec::new(),
            images: Vec::new(),
            image_cache: HashMap::new(),
            minimized: false,
            animation_timer_armed: false,
            height: 0.0,
            width: 900.0,
            dpi: 1.0,
            layout_key: None,
            projection_cache: None,
            parsed_tables: Vec::new(),
            dragging: false,
            dragging_scroll: false,
            preedit: String::new(),
            composing: false,
            ime_owner: None,
            ime_cursor: 0,
            suppress_chars: Vec::new(),
            pending_surrogate: None,
            diagnostic,
            caret: (LEFT, TOP, 28.0),
            native_caret: NativeCaretState::default(),
            status: String::new(),
        }));
        for path in paths.into_iter().take(6) {
            app.get_mut().open_path(path);
        }
        if app.get_mut().tabs.is_empty() {
            app.get_mut().new_document();
        }
        let pointer = (&*app as *const RefCell<App>).cast();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("MDLiteRustEditor"),
            w!("MDLite — Rust 編集基盤"),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100,
            800,
            None,
            None,
            Some(instance.into()),
            Some(pointer),
        )?;
        app.get_mut().hwnd = hwnd;
        let dark = windows::core::BOOL(1);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const windows::core::BOOL).cast(),
            std::mem::size_of_val(&dark) as u32,
        );
        app.get_mut().dpi = GetDpiForWindow(hwnd) as f32 / 96.0;
        DragAcceptFiles(hwnd, true);
        let _ = ShowWindow(hwnd, SW_SHOW);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        // NCDESTROY has cleared userdata; the last callback has returned.
        drop(app);
        CoUninitialize();
        Ok(())
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: called on the window owner thread; handles and COM resources
    // remain owned by App, and borrowed buffers outlive each synchronous call.
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        if msg == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const RefCell<App>;
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let Ok(mut app) = (*pointer).try_borrow_mut() else {
            if matches!(
                msg,
                WM_CLOSE | WM_COMMAND | WM_KEYDOWN | WM_CHAR | WM_LBUTTONDOWN | WM_DROPFILES
            ) {
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        app.hwnd = hwnd;
        match msg {
            WM_CREATE => LRESULT(0),
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                BeginPaint(hwnd, &mut ps);
                let damage = rect(
                    ps.rcPaint.left as f32 / app.dpi,
                    ps.rcPaint.top as f32 / app.dpi,
                    (ps.rcPaint.right - ps.rcPaint.left) as f32 / app.dpi,
                    (ps.rcPaint.bottom - ps.rcPaint.top) as f32 / app.dpi,
                );
                if let Err(e) = app.paint(damage) {
                    app.status = e.to_string();
                    app.graphics.target = None;
                    app.image_cache.clear();
                    app.layout_key = None;
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_SIZE => {
                app.minimized = wp.0 == SIZE_MINIMIZED as usize;
                if app.minimized {
                    app.sync_animations();
                    return LRESULT(0);
                }
                if let Some(target) = &app.graphics.target {
                    let _ = target.Resize(&D2D_SIZE_U {
                        width: (lp.0 as u32) & 0xffff,
                        height: ((lp.0 as u32) >> 16) & 0xffff,
                    });
                }
                app.layout_key = None;
                app.refresh();
                LRESULT(0)
            }
            WM_DPICHANGED => {
                app.dpi = (wp.0 & 0xffff) as f32 / 96.0;
                app.graphics.target = None;
                app.image_cache.clear();
                app.layout_key = None;
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
                app.native_caret = NativeCaretState::default();
                app.refresh();
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                if app.composing {
                    let _ = app.cancel_composition();
                }
                let _ = DestroyCaret();
                app.native_caret = NativeCaretState::default();
                app.dragging = false;
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let x = (lp.0 as u16 as i16) as f32 / app.dpi;
                let y = ((lp.0 >> 16) as u16 as i16) as f32 / app.dpi;
                let _ = SetFocus(Some(hwnd));
                if y < 28.0 {
                    let mut left = LEFT;
                    for (id, _, width) in TOOLBAR {
                        if x >= left && x < left + width {
                            app.command(id);
                            break;
                        }
                        left += width;
                    }
                } else if y < 74.0 {
                    let tab = ((x - LEFT) / 170.0).floor() as usize;
                    if tab < app.tabs.len() {
                        app.switch_tab(tab);
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
            WM_MOUSEMOVE => {
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
                app.dragging = false;
                app.dragging_scroll = false;
                let _ = ReleaseCapture();
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                app.context_menu();
                LRESULT(0)
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = ((wp.0 >> 16) as u16 as i16) as f32 / 120.0;
                if msg == WM_MOUSEHWHEEL || GetKeyState(VK_SHIFT.0 as i32) < 0 {
                    app.horizontal_scroll(delta * 70.0);
                } else {
                    app.scroll(-delta * 80.0);
                }
                app.refresh();
                LRESULT(0)
            }
            WM_KEYDOWN => {
                app.key(wp.0 as u16);
                LRESULT(0)
            }
            WM_CHAR => {
                app.character(wp.0 as u16);
                LRESULT(0)
            }
            WM_IME_STARTCOMPOSITION => {
                app.composing = true;
                let selection = app.doc().selection();
                app.ime_owner = Some((
                    app.doc().id(),
                    app.doc().revision(),
                    selection.anchor,
                    selection.head,
                ));
                app.preedit.clear();
                app.ime_geometry();
                LRESULT(0)
            }
            WM_IME_COMPOSITION => {
                app.composition(lp.0 as u32);
                app.refresh();
                LRESULT(0)
            }
            WM_IME_ENDCOMPOSITION => {
                app.clear_composition();
                app.refresh();
                LRESULT(0)
            }
            WM_IME_CHAR => LRESULT(0),
            WM_IME_REQUEST => app.ime_request(wp.0 as u32, lp),
            WM_IME_SETCONTEXT => DefWindowProcW(
                hwnd,
                msg,
                wp,
                LPARAM(lp.0 & !(ISC_SHOWUICOMPOSITIONWINDOW as isize)),
            ),
            WM_COMMAND => {
                app.command(wp.0 & 0xffff);
                app.refresh();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == ANIMATION_TIMER => {
                app.animation_tick();
                LRESULT(0)
            }
            WM_DROPFILES => {
                let drop = HDROP(wp.0 as *mut _);
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
                if app.confirm_all() {
                    let _ = DestroyWindow(hwnd);
                } else {
                    app.refresh();
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

impl App {
    fn live_insertion_point(&self, p: usize) -> usize {
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
    fn animation_viewport(&self) -> f32 {
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
            let mut next = None;
            for (path, cached) in &mut self.image_cache {
                if let Some(animation) = &mut cached.animation {
                    let visible = !paused
                        && self.images.iter().any(|image| {
                            image.path.as_ref() == Some(path)
                                && image_intersects_viewport(
                                    image.y,
                                    image.height,
                                    scroll,
                                    viewport,
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
                    LRESULT(
                        (request.dwTargetStrOffset == 0 && request.dwTargetStrLen as usize == len)
                            as isize,
                    )
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
            if cell.is_none() && !image {
                return;
            }
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            if cell.is_some() {
                for (id, label) in [
                    (20, "行を下に追加"),
                    (21, "行を削除（見出しは保護）"),
                    (22, "列を右に追加"),
                    (23, "列を削除（最後の列は保護）"),
                    (24, "列を左揃え"),
                    (25, "列を中央揃え"),
                    (26, "列を右揃え"),
                ] {
                    let label = wide(label);
                    let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(label.as_ptr()));
                }
            }
            if image {
                let _ = AppendMenuW(menu, MF_STRING, 27, w!("画像の幅を縮小（比率固定）"));
                let _ = AppendMenuW(menu, MF_STRING, 28, w!("画像の幅を拡大（比率固定）"));
            }
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            self.release_caret();
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                None,
                self.hwnd,
                None,
            )
            .0 as usize;
            let _ = DestroyMenu(menu);
            self.table_command(command);
            self.after_input();
        }
    }
    fn table_command(&mut self, command: usize) {
        if command == 27 || command == 28 {
            if let Some(image) = self.images.iter().find(|i| {
                i.source.start <= self.doc().selection().head
                    && self.doc().selection().head <= i.source.end
            }) {
                let range = image.source.clone();
                let source = &self.doc().source()[range.clone()];
                let width = (image.width * if command == 27 { 0.8 } else { 1.25 })
                    .round()
                    .max(24.0) as u32;
                let body = source
                    .find("<!-- mdlite-width:")
                    .map(|p| &source[..p])
                    .unwrap_or(source)
                    .trim_end();
                let replacement = format!("{body} <!-- mdlite-width:{width} -->");
                let _ = self.doc_mut().replace_range(range, &replacement);
                self.layout_key = None;
            }
            return;
        }
        let byte = self.doc().selection().head;
        let edit = self.tables.iter().find_map(|t| {
            let p = t.table.cell_at(byte)?;
            match command {
                20 => t.table.insert_row(self.doc().source(), p.row + 1),
                21 => t.table.delete_row(p.row),
                22 => t.table.insert_column(self.doc().source(), p.column + 1),
                23 => t.table.delete_column(self.doc().source(), p.column),
                24..=26 => t.table.set_alignment(
                    p.column,
                    match command {
                        24 => TableAlignment::Left,
                        25 => TableAlignment::Center,
                        _ => TableAlignment::Right,
                    },
                ),
                _ => None,
            }
        });
        if let Some(edit) = edit {
            self.apply_table(edit);
        } else if command != 0 {
            self.status = "この表操作は境界保護のため実行できません".into();
        }
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
        live_horizontal_step(
            self.doc().source(),
            &self.visible_tokens,
            &tables,
            &self.literal_blocks,
            p,
            back,
        )
    }
    fn edit(&mut self, text: &str) {
        let selection = self.doc().selection();
        let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        self.edit_range(range, text);
    }
    fn edit_range(&mut self, range: Range<usize>, text: &str) {
        let range = if !self.tabs[self.active].raw && range.is_empty() {
            let p = self.live_insertion_point(range.start);
            p..p
        } else {
            range
        };
        let mut replacement = text.to_owned();
        let mut gap_caret = None;
        if !self.tabs[self.active].raw {
            if range.is_empty()
                && !replacement.is_empty()
                && let Some(plan) = self.tables.windows(2).find_map(|tables| {
                    table_gap_insertion(
                        &tables[0].table,
                        &tables[1].table,
                        self.doc().source(),
                        range.start,
                        &replacement,
                    )
                })
            {
                gap_caret = Some(plan.caret);
                replacement = plan.replacement;
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
        let plan = if self.tabs[self.active].raw {
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
        let Some(mut plan) = plan else {
            return;
        };
        if let Some(caret) = gap_caret {
            plan.caret = caret;
        }
        self.apply_live_edit(plan);
    }
    fn apply_live_edit(&mut self, plan: LiveEdit) {
        let before_revision = self.doc().revision();
        if let Err(e) = self.doc_mut().replace_range_with_selection(
            plan.range,
            &plan.replacement,
            Selection {
                anchor: plan.caret,
                head: plan.caret,
            },
        ) {
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
            if !self.suppress_chars.is_empty() {
                if self.suppress_chars[0] == unit {
                    self.suppress_chars.remove(0);
                    return;
                }
                self.suppress_chars.clear();
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
            self.edit(&text);
            self.after_input();
        }
    }
    fn clear_composition(&mut self) {
        self.composing = false;
        self.ime_owner = None;
        self.preedit.clear();
        self.ime_cursor = 0;
    }
    unsafe fn cancel_composition(&mut self) -> bool {
        // SAFETY: the context is acquired/released on the owner thread. App's
        // RefCell borrow remains held, so synchronous IMM callbacks cannot
        // borrow another App reference. CPS_CANCEL produces no document edit;
        // acknowledge success locally even if nested notifications were blocked.
        unsafe {
            let context = ImmGetContext(self.hwnd);
            if context.0.is_null() {
                return false;
            }
            let cancelled = ImmNotifyIME(context, NI_COMPOSITIONSTR, CPS_CANCEL, 0).as_bool();
            let _ = ImmReleaseContext(self.hwnd, context);
            if cancelled {
                self.clear_composition();
            }
            cancelled
        }
    }
    unsafe fn composition(&mut self, flags: u32) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let selection = self.doc().selection();
            let current = (
                self.doc().id(),
                self.doc().revision(),
                selection.anchor,
                selection.head,
            );
            if self.ime_owner != Some(current) {
                self.preedit.clear();
                self.composing = false;
                self.ime_owner = None;
                self.status = "古いIME通知は適用していません".into();
                return;
            }
            // IMM documents a notification with no GCS flags as cancellation.
            // It has no result text to read or insert into the canonical source.
            if composition_is_cancelled(flags) {
                self.clear_composition();
                return;
            }
            let context = ImmGetContext(self.hwnd);
            if context.0.is_null() {
                return;
            }
            let read = |kind: IME_COMPOSITION_STRING| {
                let length = ImmGetCompositionStringW(context, kind, None, 0);
                if length <= 0 {
                    return String::new();
                }
                let mut utf16 = vec![0u16; length as usize / 2];
                let result = ImmGetCompositionStringW(
                    context,
                    kind,
                    Some(utf16.as_mut_ptr().cast()),
                    length as u32,
                );
                if result < 0 {
                    String::new()
                } else {
                    String::from_utf16_lossy(&utf16)
                }
            };
            if flags & GCS_RESULTSTR.0 != 0 {
                let committed = read(GCS_RESULTSTR);
                self.preedit.clear();
                if !committed.is_empty() {
                    self.suppress_chars = committed.encode_utf16().collect();
                    self.edit(&committed);
                    let selection = self.doc().selection();
                    self.ime_owner = Some((
                        self.doc().id(),
                        self.doc().revision(),
                        selection.anchor,
                        selection.head,
                    ));
                    self.after_input();
                }
            }
            if flags & GCS_COMPSTR.0 != 0 {
                self.preedit = read(GCS_COMPSTR);
                self.ime_cursor = self.preedit.encode_utf16().count() as u32;
            }
            if flags & GCS_CURSORPOS.0 != 0 {
                let cursor = ImmGetCompositionStringW(context, GCS_CURSORPOS, None, 0);
                if cursor >= 0 {
                    self.ime_cursor =
                        (cursor as u32).min(self.preedit.encode_utf16().count() as u32);
                }
            }
            let _ = ImmReleaseContext(self.hwnd, context);
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
            if self.composing {
                // Composition owns Escape before table exit or other editing.
                // Failed cancellation keeps the local preedit/owner intact.
                if key == VK_ESCAPE.0 && self.cancel_composition() {
                    self.refresh();
                }
                return;
            }
            self.suppress_chars.clear();
            if let Some(redo) = history_shortcut(key, ctrl, shift) {
                self.suppress_chars.push(key - 0x40);
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
                self.table_key(intent);
                self.after_input();
                return;
            }
            if ctrl {
                match key {
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
                    let page = (self.animation_viewport() - TOP).max(FONT * 1.6);
                    let (x, y, _) = self.caret_location();
                    let delta = if key == 0x21 { -page } else { page };
                    let target = self.hit(x, y + delta);
                    let anchor = if shift {
                        self.doc().selection().anchor
                    } else {
                        target
                    };
                    let _ = self.doc_mut().set_selection(anchor, target);
                    self.scroll(delta);
                }
                0x23..=0x28 => self.move_caret(key, shift),
                0x08 | 0x2e => {
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
                            &self.visible_tokens,
                            if self.tabs[self.active].raw {
                                &[]
                            } else {
                                &table_ranges
                            },
                            if self.tabs[self.active].raw {
                                &[]
                            } else {
                                &self.literal_blocks
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
                    let newline = if self.doc().source().contains("\r\n") {
                        "\r\n"
                    } else {
                        "\n"
                    };
                    self.edit(newline);
                }
                0x09 => {
                    self.edit("    ");
                }
                0x1b => {
                    self.dragging = false;
                    let _ = ReleaseCapture();
                }
                _ => {}
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
    fn move_caret(&mut self, key: u16, extend: bool) {
        let selection = self.doc().selection();
        let p = selection.head;
        let mut target = match key {
            0x25 => self.visible_step(p, true),
            0x27 => self.visible_step(p, false),
            0x26 | 0x28 => {
                let (x, y, h) = self.caret_location();
                self.hit(x, y + if key == 0x26 { -h * 0.6 } else { h * 1.4 })
            }
            0x24 | 0x23 => {
                let (x, y, h) = self.caret_location();
                let _ = x;
                self.hit(if key == 0x24 { -10000.0 } else { 10000.0 }, y + h * 0.5)
            }
            _ => p,
        };
        if !self.tabs[self.active].raw
            && let Some(cell) = self.active_cell()
        {
            if key == 0x24 {
                target = cell.start;
            }
            if key == 0x23 {
                target = cell.end;
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
        let _ = self.doc_mut().set_selection(anchor, target);
    }
    unsafe fn after_input(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
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
                let caret_x = self.caret_location().0 - LEFT + current;
                let next = visible_table_offset(
                    current,
                    caret_x,
                    (self.width - LEFT * 2.0 - 12.0).max(1.0),
                    content_width,
                );
                if next != current {
                    self.tabs[self.active].table_scroll.insert(key, next);
                    self.layout_key = None;
                    let _ = self.ensure_layout();
                }
            }
            self.update_caret();
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let max = client.bottom as f32 / self.dpi - 45.0;
            let y = self.caret.1;
            if y < TOP {
                self.scroll(y - TOP);
            } else if y + self.caret.2 > max {
                self.scroll(y + self.caret.2 - max);
            }
            self.refresh();
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
    fn doc_mut(&mut self) -> &mut Document {
        &mut self.tabs[self.active].doc
    }
    fn new_document(&mut self) {
        if self.tabs.len() >= 6 {
            self.status = "同時に開ける文書は6件です".into();
            return;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            doc: Document::new_unsaved(id, String::new()),
            file: None,
            scroll: 0.0,
            raw: false,
            table_scroll: HashMap::new(),
        });
        self.active = self.tabs.len() - 1;
        self.projection_cache = None;
        self.parsed_tables.clear();
        self.clear_images();
        self.layout_key = None;
    }
    fn open_path(&mut self, path: PathBuf) {
        if self.tabs.len() >= 6 {
            self.status = "同時に開ける文書は6件です".into();
            return;
        }
        match FileSnapshot::open(&path) {
            Ok(loaded) => {
                let id = self.next_id;
                self.next_id += 1;
                self.tabs.push(Tab {
                    doc: Document::new(id, loaded.source),
                    file: Some(loaded.snapshot),
                    scroll: 0.0,
                    raw: false,
                    table_scroll: HashMap::new(),
                });
                self.active = self.tabs.len() - 1;
                self.projection_cache = None;
                self.parsed_tables.clear();
                self.clear_images();
                self.layout_key = None;
            }
            Err(e) => self.status = format!("開けません: {e}"),
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
        self.active = index;
        self.projection_cache = None;
        self.parsed_tables.clear();
        self.views.clear();
        self.tables.clear();
        self.clear_images();
        self.layout_key = None;
        self.pending_surrogate = None;
        self.suppress_chars.clear();
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
                if self.tabs[self.active]
                    .file
                    .as_ref()
                    .is_some_and(|f| std::fs::canonicalize(&path).ok().as_deref() == Some(f.path()))
                {
                    return self.save(false);
                }
                let format = self.tabs[self.active]
                    .file
                    .as_ref()
                    .map(|f| f.format())
                    .unwrap_or(FileFormat::Utf8 { bom: false });
                let result = if path.exists() {
                    match FileSnapshot::open(&path) {
                        Ok(mut loaded) => loaded
                            .snapshot
                            .save_with_format(self.doc().source(), format)
                            .map(|()| loaded.snapshot),
                        Err(e) => Err(e),
                    }
                } else {
                    FileSnapshot::create_new(&path, self.doc().source(), format)
                };
                match result {
                    Ok(snapshot) => {
                        self.tabs[self.active].file = Some(snapshot);
                        self.doc_mut().mark_saved();
                        self.status = "保存しました".into();
                        true
                    }
                    Err(e) => {
                        self.status = format!("保存失敗 — 本文は保持しています: {e}");
                        false
                    }
                }
            } else {
                let tab = &mut self.tabs[self.active];
                match tab.file.as_mut().unwrap().save(tab.doc.source()) {
                    Ok(()) => {
                        tab.doc.mark_saved();
                        self.status = "保存しました".into();
                        true
                    }
                    Err(e) => {
                        self.status = format!("保存失敗 — 本文は保持しています: {e}");
                        false
                    }
                }
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
            true
        }
    }
    unsafe fn command(&mut self, id: usize) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.composing {
                return;
            }
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
                        self.layout_key = None;
                    }
                }
                CMD_RAW => {
                    let tab = &mut self.tabs[self.active];
                    tab.raw = !tab.raw;
                    if tab.raw {
                        self.clear_images();
                    }
                    self.layout_key = None;
                }
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
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let format = self.graphics.write.CreateTextFormat(
                w!("Yu Gothic UI"),
                None,
                DWRITE_FONT_WEIGHT_NORMAL,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                size,
                w!("ja-jp"),
            )?;
            format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            format.SetLineSpacing(DWRITE_LINE_SPACING_METHOD_UNIFORM, size * 1.6, size * 1.25)?;
            self.graphics.write.CreateTextLayout(
                &text.encode_utf16().collect::<Vec<_>>(),
                &format,
                width.max(1.0),
                100000.0,
            )
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
            let (mut map, mut before) = mapping;
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
            layout.GetMetrics(&mut metrics)?;
            let height = metrics.height.max(size * 1.6);
            self.views.push(TextView {
                layout,
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
            let mut client = RECT::default();
            GetClientRect(self.hwnd, &mut client)?;
            self.width = (client.right - client.left) as f32 / self.dpi;
            let key = (
                self.doc().id(),
                self.doc().revision(),
                self.width.to_bits(),
                self.tabs[self.active].raw,
            );
            if self.layout_key == Some(key) {
                return Ok(());
            }
            self.views.clear();
            self.visible_tokens.clear();
            self.literal_blocks.clear();
            self.tables.clear();
            self.images.clear();
            let source = self.doc().source_shared();
            let mut y = TOP;
            let width = (self.width - LEFT * 2.0 - 12.0).max(50.0);
            if self.tabs[self.active].raw {
                let map = byte_map(&source, 0);
                y += self.push_view(
                    &source,
                    (map.clone(), map),
                    0..source.len(),
                    (LEFT, y, width),
                    FONT,
                    None,
                )?;
            } else {
                let projection = self.cached_projection();
                self.literal_blocks = projection
                    .blocks
                    .iter()
                    .filter(|block| {
                        matches!(
                            block.kind,
                            BlockKind::FrontMatter | BlockKind::Code { fenced: true, .. }
                        )
                    })
                    .map(|block| block.source.clone())
                    .collect();
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
                    let text = String::from_utf16_lossy(&utf16[block.display_utf16.clone()]);
                    let (map, before) =
                        projection_mapping(&projection, block.display_utf16.clone());
                    let size = match block.kind {
                        BlockKind::Heading { level } => {
                            FONT + (7 - level.min(6) as i32) as f32 * 2.4
                        }
                        _ => FONT,
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
                    y += self.push_view(
                        &text,
                        (map, before),
                        block.source.clone(),
                        (LEFT + indent, y, width - indent),
                        size,
                        None,
                    )? + 10.0;
                    if let Some(view) = self.views.last_mut() {
                        view.marker = marker;
                        view.quoted = quoted;
                        view.thematic_break = matches!(block.kind, BlockKind::ThematicBreak);
                    }
                    if let Some(view) = self.views.last() {
                        Self::apply_projection_styles(
                            &view.layout,
                            &projection,
                            block.display_utf16.clone(),
                        )?;
                    }
                    if let Some(view) = self.views.last_mut() {
                        let mut metrics = DWRITE_TEXT_METRICS::default();
                        view.layout.GetMetrics(&mut metrics)?;
                        let height = metrics.height.max(size * 1.6);
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
                self.push_view("", (vec![0], vec![0]), 0..0, (LEFT, y, width), FONT, None)?;
                y += FONT * 1.6;
            }
            if self
                .views
                .last()
                .is_some_and(|v| v.source.end < source.len())
                || (!source.is_empty() && source.ends_with('\n'))
            {
                let eof = source.len();
                self.push_view(
                    "",
                    (vec![eof], vec![eof]),
                    eof..eof,
                    (LEFT, y, width),
                    FONT,
                    None,
                )?;
                y += FONT * 1.6;
            }
            self.height = y + 40.0;
            self.layout_key = Some(key);
            if !self.tabs[self.active].raw {
                let selection = self.doc().selection();
                if selection.anchor == selection.head {
                    let boundary = self.live_insertion_point(selection.head);
                    let _ = self.doc_mut().set_selection(boundary, boundary);
                }
            }
            Ok(())
        }
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
            } = self.gutter_presentation(projection, block, show_marker, FONT)?;
            let mut height = self.push_view(
                &text,
                (map, before),
                source,
                (LEFT + indent, y, width - indent),
                FONT,
                None,
            )?;
            if let Some(view) = self.views.last_mut() {
                view.marker = marker;
                view.quoted = quoted;
                Self::apply_projection_styles(&view.layout, projection, range)?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                view.layout.GetMetrics(&mut metrics)?;
                height = metrics.height.max(FONT * 1.6);
                view.height = height;
            }
            Ok(height + 10.0)
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
            } = self.gutter_presentation(projection, gutter_block, show_marker, FONT)?;
            let x = LEFT + indent;
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
                FONT,
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
                y: y + FONT * 1.6,
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
                (x, y + FONT * 1.6 + height, available),
                FONT,
                None,
            )?;
            Ok(y + height + FONT * 3.2 + 15.0)
        }
    }
    /// Decode/upload only visible images and one viewport of lookahead.
    /// Cache entries own decoders; views hold bitmap clones only near the viewport.
    unsafe fn prepare_images(&mut self) {
        unsafe {
            let bottom = self.animation_viewport();
            let scroll = self.tabs[self.active].scroll;
            let page = (bottom - TOP).max(1.0);
            let wanted: std::collections::HashSet<_> = self
                .images
                .iter()
                .filter(|image| {
                    image.y + image.height - scroll > TOP - page && image.y - scroll < bottom + page
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
        mut y: f32,
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
            let marker = if let Some(previous) = self.tables.last() {
                table_gap_marker(&source, previous.table.range.end, table.range.start)
            } else if source[..table.range.start].trim().is_empty()
                && physical_line_endings(&source[..table.range.start]) >= 2
            {
                Some(0)
            } else {
                None
            };
            if let Some(marker) = marker {
                self.push_view(
                    "",
                    (vec![marker], vec![marker]),
                    marker..marker,
                    (LEFT, y, available),
                    FONT,
                    None,
                )?;
                y += FONT * 1.6 + 10.0;
            }
            let mut widths = vec![FONT * 4.75; cols];
            for row in &table.rows {
                for (c, cell) in row.cells.iter().enumerate() {
                    let text = &source[cell.content_range.clone()];
                    let layout = self.text_layout(text, FONT * 32.0, FONT)?;
                    let mut m = DWRITE_TEXT_METRICS::default();
                    layout.GetMetrics(&mut m)?;
                    widths[c] = widths[c].max(
                        (m.widthIncludingTrailingWhitespace + 16.0).clamp(FONT * 4.75, FONT * 32.0),
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
                let mut x = LEFT - offset;
                let mut h = FONT * 1.8;
                for (c, cell) in row.cells.iter().enumerate() {
                    let content = &source[cell.content_range.clone()];
                    let (text, map) = unescape_cell(content, cell.content_range.start);
                    h = h.max(
                        self.push_view(
                            &text,
                            (map.clone(), map),
                            cell.content_range.clone(),
                            (x + 6.0, top + 3.0, widths[c] - 12.0),
                            FONT,
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
            Ok(top + 20.0)
        }
    }
    unsafe fn target(&mut self) -> windows::core::Result<ID2D1HwndRenderTarget> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if let Some(t) = &self.graphics.target {
                return Ok(t.clone());
            }
            let mut r = RECT::default();
            GetClientRect(self.hwnd, &mut r)?;
            let properties = D2D1_RENDER_TARGET_PROPERTIES {
                // DEFAULT incurred ~50 MB more private memory at creation in
                // the VM. Keep native Direct2D rendering with the same
                // DirectWrite layout and Win32 input paths.
                r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_IGNORE,
                },
                dpiX: self.dpi * 96.0,
                dpiY: self.dpi * 96.0,
                ..Default::default()
            };
            let target = self.graphics.factory.CreateHwndRenderTarget(
                &properties,
                &D2D1_HWND_RENDER_TARGET_PROPERTIES {
                    hwnd: self.hwnd,
                    pixelSize: D2D_SIZE_U {
                        width: r.right as u32,
                        height: r.bottom as u32,
                    },
                    presentOptions: D2D1_PRESENT_OPTIONS_RETAIN_CONTENTS,
                },
            )?;
            self.graphics.target = Some(target.clone());
            Ok(target)
        }
    }
    unsafe fn paint(&mut self, damage: D2D_RECT_F) -> windows::core::Result<()> {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            if self.minimized {
                return Ok(());
            }
            self.ensure_layout()?;
            self.update_caret();
            self.prepare_images();
            let t = self.target()?;
            let text = t.CreateSolidColorBrush(&color(0.83, 0.85, 0.87), None)?;
            let muted = t.CreateSolidColorBrush(&color(0.48, 0.52, 0.56), None)?;
            let selected = t.CreateSolidColorBrush(&color(0.16, 0.28, 0.39), None)?;
            let border = t.CreateSolidColorBrush(&color(0.25, 0.28, 0.31), None)?;
            let caret = self.composition_caret();
            let caret_damaged = GetFocus() == self.hwnd
                && self.native_caret.visible
                && caret_intersects_damage(caret, damage);
            let hidden_caret = if caret_damaged && HideCaret(Some(self.hwnd)).is_ok() {
                Some(HiddenCaret(self.hwnd))
            } else {
                None
            };
            t.BeginDraw();
            t.PushAxisAlignedClip(&damage, D2D1_ANTIALIAS_MODE_ALIASED);
            t.Clear(Some(&color(0.105, 0.12, 0.14)));
            let mut x = LEFT;
            for (_, label, width) in TOOLBAR {
                let layout = self.text_layout(label, width - 8.0, 12.0)?;
                t.DrawTextLayout(point(x, 4.0), &layout, &muted, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                x += width;
            }
            for (i, tab) in self.tabs.iter().enumerate() {
                let x = LEFT + i as f32 * 170.0;
                if i == self.active {
                    t.FillRectangle(&rect(x - 8.0, 28.0, 162.0, 43.0), &selected);
                }
                let name = tab
                    .file
                    .as_ref()
                    .and_then(|f| f.path().file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "無題".into());
                let label = format!("{}{}", name, if tab.doc.is_dirty() { " •" } else { "" });
                let layout = self.text_layout(&label, 154.0, 14.0)?;
                t.DrawTextLayout(point(x, 34.0), &layout, &text, D2D1_DRAW_TEXT_OPTIONS_CLIP);
            }
            t.DrawLine(
                point(LEFT, 75.0),
                point(self.width - LEFT, 75.0),
                &border,
                1.0,
                None,
            );
            let mut r = RECT::default();
            GetClientRect(self.hwnd, &mut r)?;
            let viewport = r.bottom as f32 / self.dpi - 32.0;
            let scroll = self.tabs[self.active].scroll;
            let page = (viewport - TOP).max(1.0);
            let content = (self.height - TOP).max(page);
            if content > page {
                let thumb = (page * page / content).max(28.0);
                let y = TOP + (page - thumb) * scroll / (content - page);
                t.FillRectangle(&rect(self.width - 10.0, y, 4.0, thumb), &muted);
            }
            t.PushAxisAlignedClip(
                &rect(
                    LEFT - 8.0,
                    TOP,
                    self.width - LEFT * 2.0 + 8.0,
                    (viewport - TOP).max(0.0),
                ),
                D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
            );
            let selection = self.doc().selection();
            let a = selection.anchor.min(selection.head);
            let b = selection.anchor.max(selection.head);
            for view in &self.views {
                let y = view.y - scroll;
                if y + view.height < TOP || y > viewport {
                    continue;
                }
                if a < b {
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
                        &rect(view.x - 6.0, y - 3.0, view.width + 12.0, view.height + 6.0),
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
                    t.DrawLine(
                        point(view.x, line_y),
                        point(view.x + view.width, line_y),
                        &border,
                        1.5,
                        None,
                    );
                }
                t.DrawTextLayout(
                    point(view.x, y),
                    &view.layout,
                    &text,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            for image in &self.images {
                let y = image.y - scroll;
                if y + image.height < TOP || y > viewport {
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
            if !self.preedit.is_empty() {
                let layout = self.text_layout(
                    &self.preedit,
                    self.caret_view()
                        .map(|v| self.composition_width(v, self.caret.0))
                        .unwrap_or(1.0),
                    FONT,
                )?;
                let (x, y, h) = self.caret;
                t.FillRectangle(&rect(x, y, 250.0, h), &selected);
                t.DrawTextLayout(point(x, y), &layout, &text, D2D1_DRAW_TEXT_OPTIONS_NONE);
                t.DrawLine(
                    point(x, y + h - 2.0),
                    point(x + 200.0, y + h - 2.0),
                    &text,
                    1.0,
                    None,
                );
            }
            t.PopAxisAlignedClip();
            let format = self.tabs[self.active]
                .file
                .as_ref()
                .map(|f| match f.format() {
                    FileFormat::Utf8 { bom: false } => "UTF-8".to_owned(),
                    FileFormat::Utf8 { bom: true } => "UTF-8 (BOM)".to_owned(),
                    FileFormat::Cp932 => "CP932".to_owned(),
                })
                .unwrap_or_else(|| "UTF-8（未保存）".into());
            let status = format!(
                "{}  {}  {}  {}",
                if self.tabs[self.active].raw {
                    "ソース"
                } else {
                    "Live"
                },
                format,
                if self.doc().is_dirty() {
                    "未保存の変更あり"
                } else {
                    "保存済み"
                },
                self.status
            );
            let layout = self.text_layout(&status, self.width - LEFT * 2.0, 12.0)?;
            t.DrawTextLayout(
                point(LEFT, viewport + 5.0),
                &layout,
                &muted,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
            );
            t.PopAxisAlignedClip();
            let draw_result = t.EndDraw(None, None);
            drop(hidden_caret);
            draw_result?;
            self.sync_animations();
            self.diagnostic();
            Ok(())
        }
    }
    unsafe fn release_caret(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let _ = DestroyCaret();
            self.native_caret = NativeCaretState::default();
        }
    }
    fn caret_view(&self) -> Option<&TextView> {
        let byte = self.doc().selection().head;
        self.views
            .iter()
            .find(|view| view.source.is_empty() && view.source.start == byte)
            .or_else(|| {
                self.views
                    .iter()
                    .find(|view| view.source.start <= byte && byte <= view.source.end)
            })
    }
    fn caret_location(&self) -> (f32, f32, f32) {
        let byte = self.doc().selection().head;
        let scroll = self.tabs[self.active].scroll;
        let candidate = self.caret_view().or_else(|| self.views.last());
        if let Some(view) = candidate {
            let u = view
                .map
                .partition_point(|&p| p < byte)
                .min(view.map.len().saturating_sub(1));
            let (mut x, mut y) = (0.0, 0.0);
            let mut h = DWRITE_HIT_TEST_METRICS::default();
            unsafe {
                let _ = view
                    .layout
                    .HitTestTextPosition(u as u32, false, &mut x, &mut y, &mut h);
            }
            return (view.x + x, view.y + y - scroll, h.height.max(FONT * 1.6));
        }
        (LEFT, TOP, FONT * 1.6)
    }
    unsafe fn update_caret(&mut self) {
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            self.caret = self.caret_location();
            let (x, y, height) = self.composition_caret();
            if GetFocus() == self.hwnd {
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
                    self.width - LEFT - 12.0
                } else {
                    self.width - LEFT
                };
                let visible = caret_in_editor((x, y, height), right, self.animation_viewport());
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
        let byte = selection.head;
        let view = self.caret_view()?;
        let position = view
            .map
            .partition_point(|&p| p < byte)
            .min(view.map.len().checked_sub(1)?);
        let (mut x, mut y) = (0.0, 0.0);
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        unsafe {
            view.layout
                .HitTestTextPosition(position as u32, false, &mut x, &mut y, &mut metrics)
                .ok()?;
            let origin = (view.x + x, view.y + y - self.tabs[self.active].scroll);
            if self.preedit.is_empty() {
                return Some((origin.0, origin.1, metrics.height));
            }
            let layout = self
                .text_layout(&self.preedit, self.composition_width(view, origin.0), FONT)
                .ok()?;
            layout
                .HitTestTextPosition(utf16, false, &mut x, &mut y, &mut metrics)
                .ok()?;
            Some((origin.0 + x, origin.1 + y, metrics.height))
        }
    }
    fn composition_width(&self, view: &TextView, x: f32) -> f32 {
        (view.x + view.width - x).max(1.0)
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
        // SAFETY: called on the window owner thread; handles and COM resources
        // remain owned by App, and borrowed buffers outlive each synchronous call.
        unsafe {
            let _ = self.ensure_layout();
            self.update_caret();
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let page = (r.bottom as f32 / self.dpi - TOP - 32.0).max(1.0);
            let scroll = self.tabs[self.active]
                .scroll
                .clamp(0.0, (self.height - TOP - page).max(0.0));
            self.tabs[self.active].scroll = scroll;
            let _ = InvalidateRect(Some(self.hwnd), None, false);
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
        let page = (client.bottom as f32 / self.dpi - 32.0 - TOP).max(1.0);
        let max = (self.height - TOP - page).max(0.0);
        self.tabs[self.active].scroll = ((y - TOP) / page).clamp(0.0, 1.0) * max;
    }
    fn horizontal_scroll(&mut self, delta: f32) {
        let byte = self.doc().selection().head;
        if let Some(t) = self.tables.iter().find(|t| t.table.range.contains(&byte)) {
            let available = self.width - LEFT * 2.0;
            let offset = self.tabs[self.active]
                .table_scroll
                .entry(t.table.range.start)
                .or_default();
            *offset = (*offset + delta).clamp(0.0, (t.width - available).max(0.0));
            self.layout_key = None;
        }
    }
    fn hit(&self, x: f32, y: f32) -> usize {
        let y = y + self.tabs[self.active].scroll;
        if let Some(image) = self
            .images
            .iter()
            .find(|image| image_contains_point(image, x, y))
        {
            return if y < image.y + image.height / 2.0 {
                image.source.start
            } else {
                image.source.end
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
            return 0;
        };
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
        let positions = if trailing.as_bool() {
            &view.before
        } else {
            &view.map
        };
        let byte = *positions.get(u).unwrap_or(&view.source.end);
        snap(self.doc().rope(), byte, false)
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
        if self.composing {
            return;
        }
        let byte = self.hit(x, y);
        let anchor = if extend {
            self.doc().selection().anchor
        } else {
            byte
        };
        let _ = self.doc_mut().set_selection(anchor, byte);
    }
    fn diagnostic(&self) {
        let Some(path) = &self.diagnostic else {
            return;
        };
        let sel = self.doc().selection();
        let views:Vec<_>=self.views.iter().map(|v|serde_json::json!({"source_start":v.source.start,"source_end":v.source.end,"x":v.x,"y":v.y-self.tabs[self.active].scroll,"width":v.width,"height":v.height,"table":v.table,"map":v.map})).collect();
        let tables:Vec<_>=self.tables.iter().map(|t|serde_json::json!({"source_start":t.table.range.start,"source_end":t.table.range.end,"y":t.y,"height":t.height,"width":t.width})).collect();
        // Only the opt-in diagnostic path queries the current owner-thread TIP.
        let active_input_profile = active_input_profile_diagnostic();
        let ime_open_status = ime_open_status_diagnostic(self.hwnd);
        let data = serde_json::json!({"document_id":self.doc().id(),"revision":self.doc().revision(),"source":self.doc().source(),"selection":{"anchor":sel.anchor,"head":sel.head},"dirty":self.doc().is_dirty(),"undo_count":self.doc().undo_count(),"redo_count":self.doc().redo_count(),"raw":self.tabs[self.active].raw,"tabs":self.tabs.len(),"scroll":self.tabs[self.active].scroll,"layout_count":self.views.len(),"animation_timer_armed":self.animation_timer_armed,"animations":self.image_cache.iter().filter(|(_,c)|c.animation.is_some()).map(|(p,c)|serde_json::json!({"path":p,"frame":c.frame_sequence,"running":c.animation.as_ref().and_then(|a|a.next_due()).is_some()})).collect::<Vec<_>>(),"tables":self.tables.len(),"table_geometry":tables,"views":views,"ime":{"active":self.composing,"preedit":self.preedit,"open_status":ime_open_status},"active_input_profile":active_input_profile,"caret":{"x":self.caret.0,"y":self.caret.1,"height":self.caret.2},"native_caret":{"size":self.native_caret.size,"position":self.native_caret.position},"status":self.status});
        if let Ok(data) = serde_json::to_vec_pretty(&data) {
            let _ = std::fs::write(path, data);
        }
    }
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
    let eol = table_insert_eol(left, source)?;
    let before = physical_line_endings(source.get(left.range.end..p)?);
    let after = physical_line_endings(source.get(p..right.range.start)?);
    let prefix = eol.repeat(2usize.saturating_sub(before));
    let suffix = eol.repeat(2usize.saturating_sub(after));
    Some(LiveEdit {
        range: p..p,
        caret: p + prefix.len() + text.len(),
        replacement: format!("{prefix}{text}{suffix}"),
    })
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
    tokens: &[VisibleToken],
    tables: &[Range<usize>],
    literal_blocks: &[Range<usize>],
    p: usize,
    back: bool,
) -> Option<LiveEdit> {
    let range = adjacent_line_ending(source, p, back)?;
    // One adjacent physical line ending is one join operation. A protected
    // boundary returns a no-op plan so deletion cannot fall through to a glyph.
    let mut protected = tables
        .iter()
        .any(|table| range.start <= table.end && range.end >= table.start);
    for block in literal_blocks {
        let spans: Vec<_> = tokens
            .iter()
            .flat_map(|token| token.spans.iter())
            .filter(|span| span.start >= block.start && span.end <= block.end)
            .collect();
        let first = spans.iter().map(|span| span.start).min();
        let last = spans.iter().map(|span| span.end).max();
        let opening = first.and_then(|first| adjacent_line_ending(source, first, true));
        let closing = last
            .filter(|&last| {
                last < block.end
                    && source
                        .get(last..block.end)
                        .is_some_and(|tail| !tail.trim().is_empty())
            })
            .and_then(|last| adjacent_line_ending(source, last, true));
        if opening.as_ref() == Some(&range)
            || closing.as_ref() == Some(&range)
            || (spans.is_empty() && range.start >= block.start && range.end <= block.end)
        {
            protected = true;
        }
    }
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
    literal_blocks: &[Range<usize>],
    p: usize,
    back: bool,
) -> usize {
    if let Some(plan) = line_ending_delete(source, tokens, tables, literal_blocks, p, back)
        && !plan.range.is_empty()
    {
        return if back {
            plan.range.start
        } else {
            plan.range.end
        };
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
    if selection.is_empty() {
        return Some(LiveEdit {
            caret: selection.start + text.len(),
            range: selection,
            replacement: text.to_owned(),
        });
    }
    let mut spans: Vec<_> = selected_token(tokens, selection)
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
    let start = merged.first()?.start;
    let end = merged.last()?.end;
    let mut replacement = String::new();
    let mut cursor = start;
    for (index, span) in merged.iter().enumerate() {
        replacement.push_str(source.get(cursor..span.start)?);
        if index == 0 {
            replacement.push_str(text);
        }
        cursor = span.end;
    }
    replacement.push_str(source.get(cursor..end)?);
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
    fn composing_test_app(source: &str) -> App {
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
            }],
            active: 0,
            next_id: 2,
            views: Vec::new(),
            visible_tokens: Vec::new(),
            literal_blocks: Vec::new(),
            tables: Vec::new(),
            images: Vec::new(),
            image_cache: HashMap::new(),
            minimized: false,
            animation_timer_armed: false,
            height: 0.0,
            width: 900.0,
            dpi: 1.0,
            layout_key: None,
            projection_cache: None,
            parsed_tables: Vec::new(),
            dragging: false,
            dragging_scroll: false,
            preedit: "かな".into(),
            composing: true,
            ime_owner: Some((1, 2, selection.anchor, selection.head)),
            ime_cursor: 2,
            suppress_chars: Vec::new(),
            pending_surrogate: None,
            diagnostic: None,
            caret: (LEFT, TOP, 28.0),
            native_caret: NativeCaretState::default(),
            status: String::new(),
        }
    }
    fn hidden_test_window() -> HWND {
        hidden_test_window_size(640, 480)
    }
    fn hidden_test_window_size(width: i32, height: i32) -> HWND {
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
        std::fs::write(directory.join("image.svg"), r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="red"/></svg>"#).unwrap();
        let source = "![fixture](image.svg)";
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
                "image.svg",
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
        std::fs::remove_file(directory.join("image.svg")).unwrap();
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
            assert_eq!(view.height, metrics.height.max(FONT * 1.6));
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
        assert!(image.y >= before_view.y + before_view.height + 10.0 + FONT * 1.6 - 0.01);
        assert!(
            after_view.y >= image.y + image.height + FONT * 1.6 + 15.0 - 0.01,
            "trailing fragment must start below the image"
        );
        let following = app
            .views
            .iter()
            .find(|view| source[view.source.clone()].contains("following paragraph"))
            .unwrap();
        assert!(following.y >= after_view.y + after_view.height + 10.0 - 0.01);
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
            let rule = app
                .visible_tokens
                .iter()
                .find(|token| {
                    token.spans.len() == 1
                        && token.spans[0] == (rule_start..rule_start + syntax.len())
                })
                .unwrap();
            assert!(rule.atomic);
            assert_eq!(rule.text.as_deref(), Some("─"));
            assert_eq!(rule.display_utf16.end - rule.display_utf16.start, 1);
            assert_eq!(
                live_horizontal_step(&source, &app.visible_tokens, &[], &[], rule_start, false,),
                rule_start + syntax.len()
            );
            assert_eq!(
                live_horizontal_step(
                    &source,
                    &app.visible_tokens,
                    &[],
                    &[],
                    rule_start + syntax.len(),
                    true,
                ),
                rule_start
            );

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
            assert_eq!(app.views[0].source, 0..source.len());
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
            assert!(
                app.literal_blocks
                    .iter()
                    .any(|range| { source[range.clone()].starts_with("---\ntitle") })
            );
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
            let app = RefCell::new(app);
            SetWindowLongPtrW(
                window.0,
                GWLP_USERDATA,
                (&app as *const RefCell<App>) as isize,
            );
            window_proc(window.0, WM_KILLFOCUS, WPARAM(0), LPARAM(0));
            let app = app.borrow();
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
            LEFT + 200.0,
            600.0
        ));
        assert!(!caret_in_editor(
            (LEFT - 10.0, TOP, 27.0),
            LEFT + 200.0,
            600.0
        ));
        assert!(!caret_in_editor(
            (LEFT + 210.0, TOP, 27.0),
            LEFT + 200.0,
            600.0
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
    fn horizontal_arrows_cross_adjacent_eol_without_skipping_text_or_entering_metadata_prefix() {
        for eol in ["\n", "\r\n"] {
            let source = format!("hello{eol}");
            let tokens = projected_tokens(&source);
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], &[], source.len(), true),
                5
            );
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], &[], 5, false),
                source.len()
            );
            let source = format!("a{eol}{eol}b");
            let tokens = projected_tokens(&source);
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], &[], 1 + 2 * eol.len(), true),
                1 + eol.len()
            );
            assert_eq!(
                live_horizontal_step(&source, &tokens, &[], &[], 1, false),
                1 + eol.len()
            );
        }
        let source = "---\ntitle: T\n---\n";
        let projection = DisplayProjection::parse(source);
        let tokens = projected_tokens(source);
        let blocks: Vec<_> = projection
            .blocks
            .iter()
            .filter(|block| matches!(block.kind, BlockKind::FrontMatter))
            .map(|block| block.source.clone())
            .collect();
        assert_eq!(
            live_horizontal_step(source, &tokens, &[], &blocks, 4, true),
            4
        );
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
            let tokens = projected_tokens(doc.source());
            let deletion =
                line_ending_delete(doc.source(), &tokens, &[], &[], doc.selection().head, true)
                    .unwrap();
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
                let tokens = projected_tokens(&source);
                let plan = line_ending_delete(&source, &tokens, &[], &[], p, back).unwrap();
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
            let tokens = projected_tokens(&source);
            for (p, back) in [
                (table.range.start, true),
                (table.range.start - 1, false),
                (table.range.end, false),
                (table.range.end + 1, true),
            ] {
                let mut doc = Document::new(33, source.clone());
                doc.set_selection(p, p).unwrap();
                let plan = line_ending_delete(
                    &source,
                    &tokens,
                    std::slice::from_ref(&table.range),
                    &[],
                    p,
                    back,
                )
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
    fn first_frontmatter_and_fence_text_backspace_keeps_hidden_opening_line_ending() {
        for source in ["---\ntitle: T\n---\n", "```\nfirst\n```\n"] {
            let projection = DisplayProjection::parse(source);
            let tokens = projected_tokens(source);
            let blocks: Vec<_> = projection
                .blocks
                .iter()
                .filter(|block| {
                    matches!(
                        block.kind,
                        BlockKind::FrontMatter | BlockKind::Code { fenced: true, .. }
                    )
                })
                .map(|block| block.source.clone())
                .collect();
            let first = tokens[0].spans[0].start;
            assert_eq!(first, 4);
            let mut doc = Document::new(34, source.into());
            doc.set_selection(first, first).unwrap();
            let plan = line_ending_delete(source, &tokens, &[], &blocks, first, true).unwrap();
            assert!(plan.range.is_empty());
            apply_plan(&mut doc, plan);
            assert_eq!(doc.source(), source);
            assert_eq!(doc.selection().head, first);
            assert_eq!(doc.undo_count(), 0);
            assert_eq!(doc.revision(), 0);
        }
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
    fn initial_live_insertion_preserves_heading_and_frontmatter_prefix_and_undo_caret() {
        for (source, expected, first) in [
            ("# ab", "# Xab", 2),
            ("---\ntitle: T\n---\n", "---\nXtitle: T\n---\n", 4),
        ] {
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
}
