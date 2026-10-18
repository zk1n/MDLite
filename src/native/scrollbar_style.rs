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
        &name[..count as usize] == CLASS.as_wide()
    }
}

impl VerticalRail {
    pub(super) unsafe fn new(owner: HWND, message: u32, tokens: shell::Tokens) -> Option<Self> {
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
                w!("Settings vertical scroll"),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                Some(owner),
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

    pub(super) unsafe fn cancel(&self) {
        unsafe {
            self.0.cancel();
        }
    }

    pub(super) fn take_request(&self, token: usize, hwnd: HWND) -> Option<i32> {
        if token != self.0.token || hwnd != self.0.hwnd.get() || self.0.native.get() {
            return None;
        }
        self.take_pending()
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
            let s = &self.0;
            let hwnd = s.hwnd.get();
            if hwnd == HWND::default() || s.updating.replace(true) {
                return;
            }
            let old = s.range.replace(range);
            if old.content != range.content
                || old.page != range.page
                || s.dpi.get() != dpi
                || s.native.get() != hc
            {
                s.cancel();
            }
            if s.hwnd.get() != hwnd {
                s.updating.set(false);
                return;
            }
            s.dpi.set(dpi.max(1));
            s.tokens.set(tokens);
            s.native.set(hc);
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
            s.updating.set(false);
            if s.hwnd.get() == hwnd {
                let _ = InvalidateRect(Some(hwnd), None, false);
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
        let y = ((lp.0 as u32 >> 16) as u16 as i16) as i32;
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut ps);
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
                let (top, height) = range.thumb(client.bottom, state.dpi.get());
                let width = ((if state.hover.get() || state.drag_offset.get().is_some() {
                    10.
                } else {
                    6.
                }) * state.dpi.get() as f32
                    / 96.)
                    .round()
                    .max(1.) as i32;
                let left = (client.right - width) / 2;
                fill(
                    &RECT {
                        left,
                        top: 0,
                        right: left + width,
                        bottom: client.bottom,
                    },
                    tokens.border,
                );
                fill(
                    &RECT {
                        left,
                        top,
                        right: left + width,
                        bottom: top + height,
                    },
                    tokens.muted,
                );
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN if !state.native.get() && state.range.get().maximum() > 0 => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let range = state.range.get();
                let (top, height) = range.thumb(client.bottom, state.dpi.get());
                if y >= top && y < top + height {
                    state.drag_offset.set(Some(y - top));
                    SetCapture(hwnd);
                    if state.hwnd.get() == hwnd && GetCapture() != hwnd {
                        state.cancel();
                    }
                } else {
                    let position = state.pending.get().unwrap_or(range.position);
                    state.request(if y < top {
                        position.saturating_sub(range.page)
                    } else {
                        position.saturating_add(range.page)
                    });
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if !state.hover.replace(true) {
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
                    state.request(
                        state
                            .range
                            .get()
                            .at(y - offset, client.bottom, state.dpi.get()),
                    );
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(offset) = state.drag_offset.take() {
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    state.request(
                        state
                            .range
                            .get()
                            .at(y - offset, client.bottom, state.dpi.get()),
                    );
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
