//! App-owned vertical rail for the settings form. Content position stays in its owner.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::Win32::UI::Controls::{SetScrollInfo, ShowScrollBar, WM_MOUSELEAVE};

const CLASS: PCWSTR = w!("MDLiteVerticalRail");
static NEXT_TOKEN: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Range {
    pub content: i32,
    pub page: i32,
    pub position: i32,
}
impl Range {
    fn maximum(self) -> i32 {
        self.content.saturating_sub(self.page.max(1)).max(0)
    }
    fn thumb(self, length: i32, dpi: u32) -> (i32, i32) {
        let length = length.max(0);
        if self.maximum() == 0 || length == 0 {
            return (0, length);
        }
        let minimum = (18_i64 * dpi as i64 / 96).max(1);
        let size = (length as i64 * self.page.max(0) as i64 / self.content.max(1) as i64)
            .max(minimum)
            .min(length as i64) as i32;
        let top = ((length - size) as i64 * self.position.clamp(0, self.maximum()) as i64
            / self.maximum() as i64) as i32;
        (top, size)
    }
    fn at(self, pixel: i32, length: i32, dpi: u32) -> i32 {
        let travel = length - self.thumb(length, dpi).1;
        if travel <= 0 {
            return 0;
        }
        (pixel.clamp(0, travel) as i64 * self.maximum() as i64 / travel as i64) as i32
    }
}

struct State {
    horizontal: bool,
    trailing_gap: Cell<i32>,
    owner: HWND,
    hwnd: Cell<HWND>,
    token: usize,
    message: u32,
    range: Cell<Range>,
    dpi: Cell<u32>,
    tokens: Cell<shell::Tokens>,
    native: Cell<bool>,
    updating: Cell<bool>,
    hover: Cell<bool>,
    drag_offset: Cell<Option<i32>>,
    pending: Cell<Option<i32>>,
}

#[derive(Clone)]
pub(super) struct VerticalRail(Rc<State>);

pub(super) fn high_contrast() -> bool {
    unsafe {
        // HIGHCONTRASTW's public ABI; avoids adding an unrelated Accessibility feature.
        #[repr(C)]
        struct HighContrast {
            size: u32,
            flags: u32,
            scheme: *mut u16,
        }
        let mut value = HighContrast {
            size: std::mem::size_of::<HighContrast>() as u32,
            flags: 0,
            scheme: std::ptr::null_mut(),
        };
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            value.size,
            Some((&mut value as *mut HighContrast).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
            || value.flags & 1 != 0 // HCF_HIGHCONTRASTON
    }
}

pub(super) fn is_rail(hwnd: HWND) -> bool {
    unsafe {
        let mut name = [0u16; 32];
        let count = GetClassNameW(hwnd, &mut name);
        &name[..count as usize] == CLASS.as_wide() || super::edit_scroll::is_rail(hwnd)
    }
}

impl VerticalRail {
    pub(super) unsafe fn new(owner: HWND, message: u32, tokens: shell::Tokens) -> Option<Self> {
        unsafe { Self::new_in(owner, owner, message, tokens, false) }
    }

    pub(super) unsafe fn new_sibling(
        owner: HWND,
        message: u32,
        tokens: shell::Tokens,
    ) -> Option<Self> {
        unsafe { Self::new_in(owner, GetParent(owner).ok()?, message, tokens, false) }
    }

    pub(super) unsafe fn new_horizontal_sibling(
        owner: HWND,
        message: u32,
        tokens: shell::Tokens,
    ) -> Option<Self> {
        unsafe { Self::new_in(owner, GetParent(owner).ok()?, message, tokens, true) }
    }

    unsafe fn new_in(
        owner: HWND,
        parent: HWND,
        message: u32,
        tokens: shell::Tokens,
        horizontal: bool,
    ) -> Option<Self> {
        unsafe {
            let instance = GetModuleHandleW(None).ok()?.into();
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(rail_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
                lpszClassName: CLASS,
                ..Default::default()
            });
            let state = Rc::new(State {
                horizontal,
                trailing_gap: Cell::new(0),
                owner,
                hwnd: Cell::new(HWND::default()),
                token: NEXT_TOKEN.fetch_add(1, Ordering::Relaxed),
                message,
                range: Cell::new(Range::default()),
                dpi: Cell::new(96),
                tokens: Cell::new(tokens),
                native: Cell::new(true),
                updating: Cell::new(false),
                hover: Cell::new(false),
                drag_offset: Cell::new(None),
                pending: Cell::new(None),
            });
            // NCCREATE clones this stack-held Rc; no raw ownership on creation failure.
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                CLASS,
                w!("Vertical scroll"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(parent),
                None,
                Some(instance),
                Some((&state as *const Rc<State>).cast()),
            )
            .ok()?;
            (state.hwnd.get() == hwnd).then_some(Self(state))
        }
    }

    pub(super) fn native(&self) -> bool {
        self.0.native.get()
    }

    pub(super) fn hwnd(&self) -> HWND {
        self.0.hwnd.get()
    }

    pub(super) unsafe fn destroy(&self) {
        unsafe {
            let hwnd = self.hwnd();
            if hwnd != HWND::default() {
                let _ = DestroyWindow(hwnd);
            }
        }
    }

    pub(super) unsafe fn cancel(&self) {
        unsafe {
            let state = &self.0;
            let hwnd = state.hwnd.get();
            let redraw = state.hover.get()
                || state.drag_offset.get().is_some()
                || state
                    .pending
                    .get()
                    .is_some_and(|position| position != state.range.get().position);
            self.0.cancel();
            if redraw && hwnd != HWND::default() && state.hwnd.get() == hwnd {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    pub(super) fn take_request(&self, token: usize, hwnd: HWND) -> Option<i32> {
        if token != self.0.token || hwnd != self.0.hwnd.get() || self.0.native.get() {
            return None;
        }
        self.take_pending()
    }

    pub(super) fn has_pending(&self) -> bool {
        self.0.pending.get().is_some()
    }

    pub(super) fn take_pending(&self) -> Option<i32> {
        if self.0.hwnd.get() == HWND::default() || self.0.native.get() {
            return None;
        }
        self.0.pending.take()
    }

    /// Called only after releasing the form borrow. Geometry/mode changes cancel
    /// stale input; a position-only acknowledgment preserves newer queued targets.
    pub(super) unsafe fn update(&self, range: Range, dpi: u32, tokens: shell::Tokens, hc: bool) {
        unsafe {
            let Some((hwnd, redraw)) = self.begin_update(range, dpi, tokens, hc) else {
                return;
            };
            let mut before = WINDOWINFO {
                cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            let geometry_known = GetWindowInfo(hwnd, &mut before).is_ok();
            let s = &self.0;
            if hc {
                let _ = ShowWindow(hwnd, SW_HIDE);
                if s.hwnd.get() == hwnd {
                    SetScrollInfo(s.owner, SB_VERT, &scroll_info(range), true);
                }
            } else {
                let mut client = RECT::default();
                let width = (10. * dpi as f32 / 96.).round().max(1.) as i32;
                // Prove child layout first. A failed setup leaves native scrolling usable.
                let ready = GetClientRect(s.owner, &mut client).is_ok()
                    && SetWindowPos(
                        hwnd,
                        Some(HWND_TOP),
                        client.right - width,
                        0,
                        width,
                        client.bottom,
                        SWP_NOACTIVATE,
                    )
                    .is_ok();
                if ready && s.hwnd.get() == hwnd {
                    let _ = ShowScrollBar(s.owner, SB_VERT, false);
                    let _ = GetClientRect(s.owner, &mut client);
                    let positioned = s.hwnd.get() == hwnd
                        && SetWindowPos(
                            hwnd,
                            Some(HWND_TOP),
                            client.right - width,
                            0,
                            width,
                            client.bottom,
                            SWP_NOACTIVATE,
                        )
                        .is_ok();
                    if positioned {
                        let _ = ShowWindow(
                            hwnd,
                            if range.maximum() > 0 {
                                SW_SHOWNA
                            } else {
                                SW_HIDE
                            },
                        );
                    } else if s.hwnd.get() == hwnd {
                        s.native.set(true);
                        let _ = ShowWindow(hwnd, SW_HIDE);
                        SetScrollInfo(s.owner, SB_VERT, &scroll_info(range), true);
                    }
                } else if s.hwnd.get() == hwnd {
                    s.native.set(true);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    SetScrollInfo(s.owner, SB_VERT, &scroll_info(range), true);
                }
            }
            self.end_update(hwnd, redraw || !geometry_known, &before);
        }
    }

    /// The LISTBOX adapter owns native policy and NC reservation; this only places
    /// its non-focusable sibling rail in the already reserved rectangle.
    pub(super) unsafe fn update_sibling(
        &self,
        range: Range,
        dpi: u32,
        tokens: shell::Tokens,
        native: bool,
        rect: RECT,
    ) -> bool {
        unsafe {
            let Some((hwnd, redraw)) = self.begin_update(range, dpi, tokens, native) else {
                return false;
            };
            let mut before = WINDOWINFO {
                cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            let geometry_known = GetWindowInfo(hwnd, &mut before).is_ok();
            let placed = native
                || SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top,
                    SWP_NOACTIVATE,
                )
                .is_ok();
            if self.hwnd() == hwnd {
                let _ = ShowWindow(
                    hwnd,
                    if !native && placed && range.maximum() > 0 {
                        SW_SHOWNA
                    } else {
                        SW_HIDE
                    },
                );
            }
            self.end_update(hwnd, redraw || !geometry_known, &before);
            placed
        }
    }

    pub(super) unsafe fn update_horizontal_sibling(
        &self,
        range: Range,
        dpi: u32,
        tokens: shell::Tokens,
        native: bool,
        rect: RECT,
        corner: i32,
    ) -> bool {
        unsafe {
            let corner = corner.max(0);
            if self.0.trailing_gap.replace(corner) != corner {
                let hwnd = self.hwnd();
                if hwnd != HWND::default() {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            self.update_sibling(range, dpi, tokens, native, rect)
        }
    }

    unsafe fn begin_update(
        &self,
        range: Range,
        dpi: u32,
        tokens: shell::Tokens,
        native: bool,
    ) -> Option<(HWND, bool)> {
        unsafe {
            let s = &self.0;
            let hwnd = self.hwnd();
            if hwnd == HWND::default() || s.updating.replace(true) {
                return None;
            }
            let old = s.range.replace(range);
            let old_tokens = s.tokens.get();
            let redraw = s.native.get() != native
                || old != range
                || s.dpi.get() != dpi
                || old_tokens.panel != tokens.panel
                || old_tokens.border != tokens.border
                || old_tokens.muted != tokens.muted;
            if old.content != range.content
                || old.page != range.page
                || s.dpi.get() != dpi
                || s.native.get() != native
            {
                s.cancel();
            }
            if s.hwnd.get() != hwnd {
                s.updating.set(false);
                return None;
            }
            s.dpi.set(dpi.max(1));
            s.tokens.set(tokens);
            s.native.set(native);
            Some((hwnd, redraw))
        }
    }

    unsafe fn end_update(&self, hwnd: HWND, redraw: bool, before: &WINDOWINFO) {
        unsafe {
            self.0.updating.set(false);
            if self.hwnd() == hwnd {
                let mut after = WINDOWINFO {
                    cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                    ..Default::default()
                };
                if redraw
                    || GetWindowInfo(hwnd, &mut after).is_err()
                    || before.rcWindow != after.rcWindow
                    || (before.dwStyle.0 ^ after.dwStyle.0) & WS_VISIBLE.0 != 0
                {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
        }
    }
}

pub(super) fn scroll_info(range: Range) -> SCROLLINFO {
    SCROLLINFO {
        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: range.content.saturating_sub(1).max(0),
        nPage: range.page.max(0) as u32,
        nPos: range.position,
        ..Default::default()
    }
}

impl State {
    fn length(&self, client: RECT) -> i32 {
        if self.horizontal {
            (client.right - self.trailing_gap.get()).max(0)
        } else {
            client.bottom.max(0)
        }
    }
    fn coordinate(&self, lp: LPARAM) -> i32 {
        if self.horizontal {
            lp.0 as u16 as i16 as i32
        } else {
            (lp.0 as u32 >> 16) as u16 as i16 as i32
        }
    }
    unsafe fn cancel(&self) {
        unsafe {
            self.drag_offset.set(None);
            self.pending.set(None);
            self.hover.set(false);
            if self.hwnd.get() != HWND::default() && GetCapture() == self.hwnd.get() {
                let _ = ReleaseCapture();
            }
        }
    }
    unsafe fn request(&self, position: i32) {
        unsafe {
            let hwnd = self.hwnd.get();
            if hwnd == HWND::default()
                || self.native.get()
                || !IsWindowEnabled(self.owner).as_bool()
            {
                return;
            }
            self.pending
                .set(Some(position.clamp(0, self.range.get().maximum())));
            let _ = InvalidateRect(Some(hwnd), None, false);
            let _ = PostMessageW(
                Some(self.owner),
                self.message,
                WPARAM(self.token),
                LPARAM(hwnd.0 as isize),
            );
        }
    }
}

unsafe extern "system" fn rail_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            let state = (*(cs.lpCreateParams as *const Rc<State>)).clone();
            state.hwnd.set(hwnd);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(state) as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer); // Lifetime survives native reentry/destruction.
        if msg == WM_NCDESTROY {
            state.cancel();
            state.hwnd.set(HWND::default());
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Rc::from_raw(pointer));
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let y = state.coordinate(lp);
        match msg {
            WM_PAINT | WM_PRINTCLIENT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = if msg == WM_PAINT {
                    BeginPaint(hwnd, &mut ps)
                } else {
                    HDC(wp.0 as *mut _)
                };
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let tokens = state.tokens.get();
                let fill = |rect: &RECT, color| {
                    let brush = CreateSolidBrush(shell::rgb(color));
                    FillRect(dc, rect, brush);
                    let _ = DeleteObject(brush.into());
                };
                fill(&client, tokens.panel);
                let mut range = state.range.get();
                range.position = state.pending.get().unwrap_or(range.position);
                let length = state.length(client);
                let (top, height) = range.thumb(length, state.dpi.get());
                let width = ((if state.hover.get() || state.drag_offset.get().is_some() {
                    10.
                } else {
                    6.
                }) * state.dpi.get() as f32
                    / 96.)
                    .round()
                    .max(1.) as i32;
                let cross = if state.horizontal {
                    client.bottom
                } else {
                    client.right
                };
                let left = (cross - width) / 2;
                let track = if state.horizontal {
                    RECT {
                        left: 0,
                        top: left,
                        right: length,
                        bottom: left + width,
                    }
                } else {
                    RECT {
                        left,
                        top: 0,
                        right: left + width,
                        bottom: length,
                    }
                };
                let thumb = if state.horizontal {
                    RECT {
                        left: top,
                        top: left,
                        right: top + height,
                        bottom: left + width,
                    }
                } else {
                    RECT {
                        left,
                        top,
                        right: left + width,
                        bottom: top + height,
                    }
                };
                fill(&track, tokens.border);
                fill(&thumb, tokens.muted);
                if msg == WM_PAINT {
                    let _ = EndPaint(hwnd, &ps);
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN if !state.native.get() && state.range.get().maximum() > 0 => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let range = state.range.get();
                let length = state.length(client);
                if y >= length {
                    return LRESULT(0);
                }
                let (top, height) = range.thumb(length, state.dpi.get());
                if y >= top && y < top + height {
                    state.drag_offset.set(Some(y - top));
                    SetCapture(hwnd);
                    if state.hwnd.get() == hwnd && GetCapture() != hwnd {
                        state.cancel();
                    }
                } else {
                    let position = state.pending.get().unwrap_or(range.position);
                    let page = if state.horizontal {
                        (range.page / 3 * 2).max(1)
                    } else {
                        range.page
                    };
                    state.request(if y < top {
                        position.saturating_sub(page)
                    } else {
                        position.saturating_add(page)
                    });
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let hot = y >= 0 && y < state.length(client);
                if state.hover.replace(hot) != hot {
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        ..Default::default()
                    };
                    let _ = TrackMouseEvent(&mut track);
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                if let Some(offset) = state.drag_offset.get()
                    && GetCapture() == hwnd
                {
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    state.request(state.range.get().at(
                        y - offset,
                        state.length(client),
                        state.dpi.get(),
                    ));
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(offset) = state.drag_offset.take() {
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    state.request(state.range.get().at(
                        y - offset,
                        state.length(client),
                        state.dpi.get(),
                    ));
                    if GetCapture() == hwnd {
                        let _ = ReleaseCapture();
                    }
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                state.hover.set(false);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_CANCELMODE | WM_ENABLE if msg == WM_CANCELMODE || wp.0 == 0 => {
                state.cancel();
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_CAPTURECHANGED => {
                if state.drag_offset.take().is_some() {
                    state.pending.set(None);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => SendMessageW(state.owner, msg, Some(wp), Some(lp)),
            WM_MOUSEHWHEEL => SendMessageW(state.owner, msg, Some(wp), Some(lp)),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const REQUEST: u32 = WM_APP + 319;
    struct Owner {
        rail: RefCell<Option<VerticalRail>>,
        position: Cell<i32>,
    }
    unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        unsafe {
            if msg == WM_NCCREATE {
                let cs = &*(lp.0 as *const CREATESTRUCTW);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Owner;
            if msg == REQUEST && !pointer.is_null() {
                let owner = &*pointer;
                let rail = { owner.rail.borrow().clone() };
                if let Some(rail) = rail
                    && let Some(target) = rail.take_request(wp.0, HWND(lp.0 as *mut _))
                {
                    owner.position.set(target);
                    let mut range = rail.0.range.get();
                    range.position = target;
                    rail.update(range, rail.0.dpi.get(), shell::Tokens::default(), false);
                }
                return LRESULT(0);
            }
            if msg == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
    struct Fixture {
        parent: HWND,
        rail: VerticalRail,
        owner: Box<Owner>,
    }
    impl Fixture {
        fn new() -> Self {
            unsafe {
                let owner = Box::new(Owner {
                    rail: RefCell::new(None),
                    position: Cell::new(0),
                });
                let instance = GetModuleHandleW(None).unwrap().into();
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(owner_proc),
                    hInstance: instance,
                    lpszClassName: w!("MDLiteRailOwnerTest"),
                    ..Default::default()
                });
                let parent = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("MDLiteRailOwnerTest"),
                    w!("hidden rail test"),
                    WS_OVERLAPPED | WS_VSCROLL,
                    0,
                    0,
                    300,
                    500,
                    None,
                    None,
                    Some(instance),
                    Some((&*owner as *const Owner).cast()),
                )
                .unwrap();
                let rail = VerticalRail::new(parent, REQUEST, shell::Tokens::default()).unwrap();
                *owner.rail.borrow_mut() = Some(rail.clone());
                rail.update(
                    Range {
                        content: 200_000,
                        page: 500,
                        position: 0,
                    },
                    96,
                    shell::Tokens::default(),
                    false,
                );
                Self {
                    parent,
                    rail,
                    owner,
                }
            }
        }
        unsafe fn dispatch_requests(&self) {
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, Some(self.parent), REQUEST, REQUEST, PM_REMOVE)
                    .as_bool()
                {
                    DispatchMessageW(&message);
                }
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.parent);
            }
        }
    }
    unsafe fn pointer(hwnd: HWND, message: u32, y: i32) {
        unsafe {
            SendMessageW(
                hwnd,
                message,
                Some(WPARAM(0)),
                Some(LPARAM(((y as u16 as u32) << 16) as isize)),
            );
        }
    }
    #[test]
    fn settings_rail_full_i32_geometry_and_drag_pointer_up_commit_latest() {
        let range = Range {
            content: i32::MAX,
            page: 500,
            position: 0,
        };
        assert_eq!(range.at(0, 500, 96), 0);
        assert_eq!(range.at(500, 500, 96), i32::MAX - 500);
        let fixture = Fixture::new();
        let rail = &fixture.rail;
        let hwnd = rail.0.hwnd.get();
        unsafe {
            assert_eq!(GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_TABSTOP.0, 0);
            let focus = GetFocus();
            pointer(hwnd, WM_LBUTTONDOWN, 5);
            assert_eq!(GetCapture(), hwnd);
            assert_eq!(
                GetFocus(),
                focus,
                "rail must not take the text/control focus"
            );
            pointer(hwnd, WM_MOUSEMOVE, 320);
            assert!(rail.0.pending.get().unwrap() > 65_535);
            let target = rail.0.pending.get().unwrap();
            let mut ack = rail.0.range.get();
            ack.position = 100;
            rail.update(ack, 96, shell::Tokens::default(), false);
            assert_eq!(
                rail.0.pending.get(),
                Some(target),
                "old position ACK cannot discard newer input"
            );
            assert!(rail.0.drag_offset.get().is_some());
            pointer(hwnd, WM_LBUTTONUP, 440);
            assert_ne!(GetCapture(), hwnd);
            let final_target = rail.0.pending.get().unwrap();
            assert!(final_target > target && final_target > 150_000);
            fixture.dispatch_requests();
            assert_eq!(
                fixture.owner.position.get(),
                final_target,
                "posted owner protocol must preserve all position bits"
            );
            assert_eq!(rail.take_request(rail.0.token, hwnd), None);
        }
    }
    #[test]
    fn settings_rail_identical_hidden_native_updates_are_idempotent() {
        let fixture = Fixture::new();
        let rail = &fixture.rail;
        let mut range = Range {
            content: 200_000,
            page: 500,
            position: 0,
        };
        let mut dpi = 96;
        let mut tokens = shell::Tokens::default();
        let update = |range, dpi, tokens, native| unsafe {
            let mut before = WINDOWINFO {
                cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            GetWindowInfo(rail.hwnd(), &mut before).unwrap();
            let (hwnd, redraw) = rail.begin_update(range, dpi, tokens, native).unwrap();
            rail.end_update(hwnd, redraw, &before);
            redraw
        };
        assert!(
            update(range, dpi, tokens, true),
            "custom to native/hidden transition"
        );
        assert!(
            !update(range, dpi, tokens, true),
            "unchanged native/hidden mode"
        );
        range.position += 1;
        assert!(update(range, dpi, tokens, true), "position");
        assert!(!update(range, dpi, tokens, true));
        range.page += 1;
        assert!(update(range, dpi, tokens, true), "page");
        assert!(!update(range, dpi, tokens, true));
        range.content += 1;
        assert!(update(range, dpi, tokens, true), "content");
        assert!(!update(range, dpi, tokens, true));
        dpi = 144;
        assert!(update(range, dpi, tokens, true), "DPI");
        assert!(!update(range, dpi, tokens, true));
        tokens.panel.r += 0.01;
        assert!(update(range, dpi, tokens, true), "panel color");
        assert!(!update(range, dpi, tokens, true));
        tokens.border.r += 0.01;
        assert!(update(range, dpi, tokens, true), "track color");
        assert!(!update(range, dpi, tokens, true));
        tokens.muted.r += 0.01;
        assert!(update(range, dpi, tokens, true), "thumb color");
        assert!(!update(range, dpi, tokens, true));
        assert!(
            update(range, dpi, tokens, false),
            "native/hidden to custom transition"
        );
        assert!(!update(range, dpi, tokens, false));
        assert!(
            update(range, dpi, tokens, true),
            "custom to native/hidden transition"
        );
        assert!(!update(range, dpi, tokens, true));
        assert!(!unsafe { IsWindowVisible(fixture.parent) }.as_bool());
    }

    #[test]
    fn settings_rail_range_dpi_hc_fit_and_attachment_invalidate_stale_requests() {
        let fixture = Fixture::new();
        let rail = &fixture.rail;
        let hwnd = rail.0.hwnd.get();
        unsafe {
            rail.0.request(150_000);
            assert_eq!(rail.take_request(rail.0.token + 1, hwnd), None);
            let other = Fixture::new();
            assert_eq!(other.rail.take_request(rail.0.token, hwnd), None);
            rail.update(
                Range {
                    content: 200_000,
                    page: 500,
                    position: 150_000,
                },
                144,
                shell::Tokens::default(),
                true,
            );
            assert_eq!(rail.take_pending(), None);
            assert_ne!(
                GetWindowLongW(fixture.parent, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
            assert_eq!(GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let mut info = scroll_info(Range::default());
            GetScrollInfo(fixture.parent, SB_VERT, &mut info).unwrap();
            assert_eq!(info.nPos, 150_000);
            rail.update(
                Range {
                    content: 200,
                    page: 500,
                    position: 0,
                },
                144,
                shell::Tokens::default(),
                false,
            );
            assert_eq!(GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            rail.update(
                Range {
                    content: 200_000,
                    page: 500,
                    position: 0,
                },
                144,
                shell::Tokens::default(),
                false,
            );
            let mut rect = RECT::default();
            GetClientRect(hwnd, &mut rect).unwrap();
            assert_eq!(rect.right, 15);
            assert_ne!(GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            assert_eq!(
                GetWindowLongW(fixture.parent, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
            pointer(hwnd, WM_LBUTTONDOWN, 5);
            assert_eq!(GetCapture(), hwnd);
            pointer(hwnd, WM_MOUSEMOVE, 200);
            rail.update(
                Range {
                    content: 100_000,
                    page: 500,
                    position: 0,
                },
                144,
                shell::Tokens::default(),
                false,
            );
            assert_ne!(GetCapture(), hwnd);
            assert_eq!(rail.take_pending(), None);
        }
    }
    #[test]
    fn settings_rail_cancel_disable_capture_loss_and_destruction_release_state() {
        let fixture = Fixture::new();
        let rail = &fixture.rail;
        let hwnd = rail.0.hwnd.get();
        unsafe {
            for mode in 0..3 {
                pointer(hwnd, WM_LBUTTONDOWN, 5);
                pointer(hwnd, WM_MOUSEMOVE, 250);
                match mode {
                    0 => {
                        SendMessageW(hwnd, WM_CANCELMODE, Some(WPARAM(0)), Some(LPARAM(0)));
                    }
                    1 => {
                        SetCapture(fixture.parent);
                    }
                    _ => {
                        let _ = EnableWindow(hwnd, false);
                    }
                }
                assert_eq!(rail.0.drag_offset.get(), None);
                assert_eq!(rail.0.pending.get(), None);
                assert_ne!(GetCapture(), hwnd);
                if GetCapture() == fixture.parent {
                    ReleaseCapture().unwrap();
                }
                let _ = EnableWindow(hwnd, true);
            }
            pointer(hwnd, WM_LBUTTONDOWN, 5);
            assert_eq!(GetCapture(), hwnd);
            DestroyWindow(hwnd).unwrap();
            assert_ne!(GetCapture(), hwnd);
            assert_eq!(rail.0.hwnd.get(), HWND::default());
            assert_eq!(rail.take_request(rail.0.token, hwnd), None);
        }
    }
}
