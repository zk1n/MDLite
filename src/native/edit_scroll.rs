//! Native EDIT nonclient exclusion. The EDIT retains its original input, client,
//! scroll ranges and native tracking. Only the scrollbar pixels are replaced.
use super::*;
use windows::Win32::UI::Controls::*;

const PROPERTY: PCWSTR = w!("MDLite.NativeEditScroll.v1");
const RAIL_CLASS: PCWSTR = w!("MDLiteNativeEditRail");
const SUBCLASS: usize = 0x4d44_4553;

pub(super) fn is_rail(hwnd: HWND) -> bool {
    unsafe {
        let mut name = [0u16; 32];
        let count = GetClassNameW(hwnd, &mut name).max(0) as usize;
        &name[..count] == RAIL_CLASS.as_wide()
    }
}

struct OwnedRegion(HRGN);
impl OwnedRegion {
    unsafe fn rectangle(r: RECT) -> Result<Self, String> {
        let handle = unsafe { CreateRectRgn(r.left, r.top, r.right, r.bottom) };
        if handle.0.is_null() {
            Err("Cannot allocate EDIT clip region".into())
        } else {
            Ok(Self(handle))
        }
    }
    unsafe fn copy(&self) -> Result<Self, String> {
        unsafe {
            let result = Self::rectangle(RECT::default())?;
            if CombineRgn(Some(result.0), Some(self.0), None, RGN_COPY).0 == 0 {
                return Err("Cannot copy EDIT clip region".into());
            }
            Ok(result)
        }
    }
    unsafe fn apply(self, hwnd: HWND) -> Result<(), String> {
        if unsafe { SetWindowRgn(hwnd, Some(self.0), true) } == 0 {
            return Err("Cannot set EDIT clip region".into());
        }
        // Ownership transfers only after a successful SetWindowRgn.
        std::mem::forget(self);
        Ok(())
    }
}
impl Drop for OwnedRegion {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

struct State {
    hwnd: HWND,
    parent: HWND,
    id: i32,
    original: Option<OwnedRegion>,
    rails: [Cell<HWND>; 2],
    tokens: Cell<shell::Tokens>,
    updating: Cell<bool>,
    clipped: Cell<bool>,
    suspended: Cell<bool>,
}

unsafe fn owning_thread(hwnd: HWND) -> bool {
    unsafe {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process))
            == windows_sys::Win32::System::Threading::GetCurrentThreadId()
            && process == std::process::id()
    }
}
unsafe fn attached(hwnd: HWND) -> Option<Rc<State>> {
    unsafe {
        if !owning_thread(hwnd) {
            return None;
        }
        let pointer = GetPropW(hwnd, PROPERTY).0 as *const State;
        if pointer.is_null() {
            return None;
        }
        Rc::increment_strong_count(pointer);
        Some(Rc::from_raw(pointer))
    }
}
unsafe fn native_bar(hwnd: HWND, horizontal: bool) -> Option<SCROLLBARINFO> {
    unsafe {
        let mut bar = SCROLLBARINFO {
            cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        GetScrollBarInfo(
            hwnd,
            if horizontal {
                OBJID_HSCROLL
            } else {
                OBJID_VSCROLL
            },
            &mut bar,
        )
        .ok()?;
        (bar.rcScrollBar.right > bar.rcScrollBar.left
            && bar.rcScrollBar.bottom > bar.rcScrollBar.top)
            .then_some(bar)
    }
}

/// Attach only to an owned native multiline EDIT. Parent, ID, style, format rect,
/// selection and history remain native. Attach failure attempts native restoration;
/// a failed restoration is reported separately rather than claiming usable fallback.
pub(super) unsafe fn attach(hwnd: HWND, tokens: shell::Tokens) -> Result<(), String> {
    unsafe {
        if !owning_thread(hwnd) {
            return Err("EDIT rail requires its owning UI thread".into());
        }
        if attached(hwnd).is_some() {
            return refresh(hwnd, tokens).map(|_| ());
        }
        let mut name = [0u16; 32];
        let count = GetClassNameW(hwnd, &mut name).max(0) as usize;
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if !String::from_utf16_lossy(&name[..count]).eq_ignore_ascii_case("EDIT")
            || style & (WS_CHILD.0 | ES_MULTILINE as u32) != WS_CHILD.0 | ES_MULTILINE as u32
            || style & (WS_HSCROLL.0 | WS_VSCROLL.0) == 0
        {
            return Err(
                "EDIT rail requires a native multiline child with native scroll styles".into(),
            );
        }
        let parent = GetParent(hwnd).map_err(|e| e.to_string())?;
        let region = OwnedRegion::rectangle(RECT::default())?;
        let original = (GetWindowRgn(hwnd, region.0).0 != 0).then_some(region);
        let state = Rc::new(State {
            hwnd,
            parent,
            id: GetDlgCtrlID(hwnd),
            original,
            rails: [Cell::new(HWND::default()), Cell::new(HWND::default())],
            tokens: Cell::new(tokens),
            updating: Cell::new(false),
            clipped: Cell::new(false),
            suspended: Cell::new(false),
        });
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?.into();
        let _ = RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(rail_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_ARROW).map_err(|e| e.to_string())?,
            lpszClassName: RAIL_CLASS,
            ..Default::default()
        });
        for index in 0..2 {
            let binding = Rc::new(Rail {
                state: Rc::clone(&state),
                horizontal: index == 0,
                hover: Cell::new(false),
            });
            match CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                RAIL_CLASS,
                w!(""),
                WS_CHILD | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(parent),
                None,
                Some(instance),
                Some((&binding as *const Rc<Rail>).cast()),
            ) {
                Ok(rail) => state.rails[index].set(rail),
                Err(error) => {
                    state.destroy_rails();
                    return Err(error.to_string());
                }
            }
        }
        let pointer = Rc::into_raw(Rc::clone(&state));
        if let Err(error) = SetPropW(hwnd, PROPERTY, Some(HANDLE(pointer.cast_mut().cast()))) {
            state.destroy_rails();
            drop(Rc::from_raw(pointer));
            return Err(error.to_string());
        }
        if !SetWindowSubclass(hwnd, Some(edit_proc), SUBCLASS, pointer as usize).as_bool() {
            let _ = RemovePropW(hwnd, PROPERTY);
            state.destroy_rails();
            drop(Rc::from_raw(pointer));
            return Err("Cannot subclass EDIT scrollbar".into());
        }
        if let Err(error) = state.sync() {
            return Err(native_fallback(hwnd, error));
        }
        Ok(())
    }
}

/// Ok(false) is an unattached control, not a refresh failure. Only an attached
/// EDIT is ever rolled back; failed native restoration retains state for retry.
pub(super) unsafe fn refresh(hwnd: HWND, tokens: shell::Tokens) -> Result<bool, String> {
    unsafe {
        let Some(state) = attached(hwnd) else {
            return Ok(false);
        };
        state.tokens.set(tokens);
        match state.sync() {
            Ok(()) => Ok(true),
            Err(error) => Err(native_fallback(hwnd, error)),
        }
    }
}

/// Explorer performs NC work outside its WM_THEMECHANGED dispatch. Keep the
/// original region through the whole API call, then recapture native geometry.
pub(super) unsafe fn set_explorer_theme(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let Some(state) = attached(hwnd) else {
            return SetWindowTheme(hwnd, w!("Explorer"), None).map_err(|error| error.to_string());
        };
        let updating = state.updating.replace(true);
        state.cancel();
        if !state.valid() {
            state.updating.set(updating);
            return Err("EDIT changed ownership during theme cancellation".into());
        }
        let result = state.restore().and_then(|()| {
            if !state.valid() {
                return Err("EDIT changed ownership during theme region restoration".into());
            }
            SetWindowTheme(hwnd, w!("Explorer"), None).map_err(|error| error.to_string())
        });
        state.updating.set(updating);
        if !state.valid() {
            return Err("EDIT changed ownership during theme update".into());
        }
        if let Err(error) = result {
            return Err(native_fallback(hwnd, error));
        }
        state.sync().map_err(|error| native_fallback(hwnd, error))
    }
}

unsafe fn native_fallback(hwnd: HWND, error: String) -> String {
    unsafe {
        match detach(hwnd) {
            Ok(()) => format!("{error}; original EDIT region restored and adapter detached"),
            Err(cleanup) => format!(
                "{error}; EDIT adapter cleanup failed: {cleanup}; native fallback is not confirmed"
            ),
        }
    }
}
#[cfg(test)]
unsafe fn cancel(hwnd: HWND) {
    unsafe {
        if let Some(state) = attached(hwnd) {
            state.cancel();
        }
    }
}
pub(super) unsafe fn detach(hwnd: HWND) -> Result<(), String> {
    unsafe {
        let state = attached(hwnd).ok_or("EDIT scrollbar is not attached")?;
        let suspended = state.suspended.replace(true);
        state.cancel();
        // Restore first: keep the attached state/rails available for retry if the
        // OS rejects restoration. Do not leave an otherwise active adapter inert.
        if let Err(error) = state.restore() {
            state.suspended.set(suspended);
            return Err(format!("{error}; adapter retained for restoration retry"));
        }
        state.destroy_rails();
        let pointer = RemovePropW(hwnd, PROPERTY).map_err(|e| e.to_string())?.0 as *const State;
        let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), SUBCLASS);
        if !pointer.is_null() {
            drop(Rc::from_raw(pointer));
        }
        Ok(())
    }
}
#[cfg(test)]
pub(super) unsafe fn rail_handles(hwnd: HWND) -> Option<[HWND; 2]> {
    unsafe { attached(hwnd).map(|state| [state.rails[0].get(), state.rails[1].get()]) }
}

impl State {
    unsafe fn valid(&self) -> bool {
        unsafe {
            IsWindow(Some(self.hwnd)).as_bool()
                && GetParent(self.hwnd).ok() == Some(self.parent)
                && GetDlgCtrlID(self.hwnd) == self.id
                && GetPropW(self.hwnd, PROPERTY).0 == (self as *const State).cast_mut().cast()
        }
    }
    unsafe fn cancel(&self) {
        unsafe {
            if self.valid() {
                SendMessageW(self.hwnd, WM_CANCELMODE, None, None);
            }
        }
    }
    unsafe fn destroy_rails(&self) {
        for rail in &self.rails {
            let hwnd = rail.replace(HWND::default());
            unsafe {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }
    unsafe fn hide_rails(&self) {
        unsafe {
            for rail in &self.rails {
                let _ = ShowWindow(rail.get(), SW_HIDE);
            }
        }
    }
    unsafe fn restore(&self) -> Result<(), String> {
        unsafe {
            if !self.clipped.replace(false) {
                return Ok(());
            }
            // SetWindowRgn may synchronously deliver WINDOWPOSCHANGED. Restore
            // must not re-enter adaptation, including outside sync()/during detach.
            let updating = self.updating.replace(true);
            let result = if let Some(region) = &self.original {
                region.copy().and_then(|copy| copy.apply(self.hwnd))
            } else if SetWindowRgn(self.hwnd, None, true) == 0 {
                Err("Cannot restore native EDIT region".into())
            } else {
                Ok(())
            };
            self.updating.set(updating);
            if result.is_err() {
                self.clipped.set(true);
            }
            result
        }
    }
    unsafe fn sync(&self) -> Result<(), String> {
        unsafe {
            if !self.valid() {
                self.hide_rails();
                return self.restore();
            }
            if self.updating.replace(true) {
                return Ok(());
            }
            struct Reset<'a>(&'a Cell<bool>);
            impl Drop for Reset<'_> {
                fn drop(&mut self) {
                    self.0.set(false);
                }
            }
            let _reset = Reset(&self.updating);
            let native = self.tokens.get().high_contrast || scrollbar_style::high_contrast();
            if native
                || self.suspended.get()
                || GetWindowLongW(self.hwnd, GWL_STYLE) as u32 & WS_VISIBLE.0 == 0
                || !IsWindowEnabled(self.hwnd).as_bool()
            {
                self.cancel();
                self.hide_rails();
                return self.restore();
            }
            let mut outer = RECT::default();
            GetWindowRect(self.hwnd, &mut outer).map_err(|e| e.to_string())?;
            let full = RECT {
                left: 0,
                top: 0,
                right: outer.right - outer.left,
                bottom: outer.bottom - outer.top,
            };
            let result = if let Some(original) = &self.original {
                original.copy()?
            } else {
                OwnedRegion::rectangle(full)?
            };
            let style = GetWindowLongW(self.hwnd, GWL_STYLE) as u32;
            let bars = [true, false].map(|horizontal| {
                let flag = if horizontal { WS_HSCROLL } else { WS_VSCROLL };
                (style & flag.0 != 0)
                    .then(|| native_bar(self.hwnd, horizontal))
                    .flatten()
            });
            let mut cuts = Vec::with_capacity(3);
            for bar in bars.iter().flatten() {
                cuts.push(RECT {
                    left: bar.rcScrollBar.left - outer.left,
                    top: bar.rcScrollBar.top - outer.top,
                    right: bar.rcScrollBar.right - outer.left,
                    bottom: bar.rcScrollBar.bottom - outer.top,
                });
            }
            if let [Some(h), Some(v)] = bars {
                cuts.push(RECT {
                    left: v.rcScrollBar.left - outer.left,
                    top: h.rcScrollBar.top - outer.top,
                    right: v.rcScrollBar.right - outer.left,
                    bottom: h.rcScrollBar.bottom - outer.top,
                });
            }
            for cut in cuts {
                let cut = OwnedRegion::rectangle(cut)?;
                if CombineRgn(Some(result.0), Some(result.0), Some(cut.0), RGN_DIFF).0 == 0 {
                    self.hide_rails();
                    self.restore()?;
                    return Err("Cannot exclude native EDIT scrollbar".into());
                }
            }
            let old = OwnedRegion::rectangle(RECT::default())?;
            let same = GetWindowRgn(self.hwnd, old.0).0 != 0 && EqualRgn(old.0, result.0).as_bool();
            if !same {
                result.apply(self.hwnd)?;
            }
            self.clipped.set(true);
            for (index, bar) in bars.into_iter().enumerate() {
                let rail = self.rails[index].get();
                if let Some(bar) = bar {
                    let mut point = POINT {
                        x: bar.rcScrollBar.left,
                        y: bar.rcScrollBar.top,
                    };
                    let _ = ScreenToClient(self.parent, &mut point);
                    SetWindowPos(
                        rail,
                        Some(HWND_TOP),
                        point.x,
                        point.y,
                        bar.rcScrollBar.right - bar.rcScrollBar.left,
                        bar.rcScrollBar.bottom - bar.rcScrollBar.top,
                        SWP_NOACTIVATE | SWP_SHOWWINDOW,
                    )
                    .map_err(|e| e.to_string())?;
                    let _ = InvalidateRect(Some(rail), None, false);
                    let _ = UpdateWindow(rail);
                } else {
                    let _ = ShowWindow(rail, SW_HIDE);
                }
            }
            Ok(())
        }
    }
}

unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let pointer = data as *const State;
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        if msg == WM_NCDESTROY {
            state.cancel();
            state.destroy_rails();
            let _ = RemovePropW(hwnd, PROPERTY);
            let _ = RemoveWindowSubclass(hwnd, Some(edit_proc), id);
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if (msg == WM_ENABLE && wp.0 == 0)
            || (msg == WM_SHOWWINDOW && wp.0 == 0)
            || matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED)
        {
            state.cancel();
        }
        if msg == WM_SHOWWINDOW {
            state.suspended.set(wp.0 == 0);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if matches!(
            msg,
            WM_HSCROLL
                | WM_VSCROLL
                | WM_MOUSEWHEEL
                | WM_MOUSEHWHEEL
                | WM_KEYDOWN
                // Translated characters, IME results and programmatic native
                // edits finish after KEYDOWN. Refresh from their completed state.
                | WM_CHAR
                | WM_IME_CHAR
                | WM_IME_COMPOSITION
                | WM_IME_ENDCOMPOSITION
                | WM_SETTEXT
                | EM_REPLACESEL
                | EM_UNDO
                | WM_UNDO
                | WM_CLEAR
                | WM_CUT
                | WM_PASTE
                | EM_SETSEL
                | EM_LINESCROLL
                | EM_SCROLL
                | EM_SCROLLCARET
                | EM_SETRECT
                | EM_SETRECTNP
                | EM_SETMARGINS
                | EM_SETTABSTOPS
                | WM_SETFONT
                | WM_SIZE
                | WM_WINDOWPOSCHANGED
                | WM_STYLECHANGED
                | WM_ENABLE
                | WM_SHOWWINDOW
                | WM_SETTINGCHANGE
                | WM_THEMECHANGED
                | WM_DPICHANGED_AFTERPARENT
                | WM_NCLBUTTONDOWN
                | WM_CAPTURECHANGED
        ) && state.sync().is_err()
        {
            state.cancel();
            state.hide_rails();
            let _ = state.restore();
        }
        result
    }
}

struct Rail {
    state: Rc<State>,
    horizontal: bool,
    hover: Cell<bool>,
}
unsafe extern "system" fn rail_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            let rail = &*(create.lpCreateParams as *const Rc<Rail>);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(Rc::clone(rail)) as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Rail;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        Rc::increment_strong_count(pointer);
        let rail = Rc::from_raw(pointer);
        let state = &rail.state;
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            drop(Rc::from_raw(pointer));
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        match msg {
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(hwnd, &mut paint);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let tokens = state.tokens.get();
                let background = CreateSolidBrush(shell::rgb(tokens.panel));
                FillRect(dc, &client, background);
                let _ = DeleteObject(background.into());
                if let Some(bar) = native_bar(state.hwnd, rail.horizontal) {
                    let thickness = ((if rail.hover.get() || GetCapture() == state.hwnd {
                        10.
                    } else {
                        6.
                    }) * GetDpiForWindow(hwnd).max(1) as f32
                        / 96.)
                        .round()
                        .max(1.) as i32;
                    let cross = if rail.horizontal {
                        client.bottom
                    } else {
                        client.right
                    };
                    let start = (cross - thickness) / 2;
                    let length = if rail.horizontal {
                        client.right
                    } else {
                        client.bottom
                    };
                    let rect = |a, b| {
                        if rail.horizontal {
                            RECT {
                                left: a,
                                top: start,
                                right: b,
                                bottom: start + thickness,
                            }
                        } else {
                            RECT {
                                left: start,
                                top: a,
                                right: start + thickness,
                                bottom: b,
                            }
                        }
                    };
                    for (r, color) in [
                        (rect(0, length), tokens.border),
                        (rect(bar.xyThumbTop, bar.xyThumbBottom), tokens.muted),
                    ] {
                        let brush = CreateSolidBrush(shell::rgb(color));
                        FillRect(dc, &r, brush);
                        let _ = DeleteObject(brush.into());
                    }
                }
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let mut point = POINT {
                    x: lp.0 as u16 as i16 as i32,
                    y: (lp.0 >> 16) as u16 as i16 as i32,
                };
                let _ = ClientToScreen(hwnd, &mut point);
                if state.valid()
                    && state.clipped.get()
                    && !state.suspended.get()
                    && IsWindowVisible(state.hwnd).as_bool()
                    && IsWindowEnabled(state.hwnd).as_bool()
                    && IsWindowEnabled(state.parent).as_bool()
                    && GetCapture() == HWND::default()
                    && GetKeyState(VK_LBUTTON.0 as i32) < 0
                    && GetAncestor(state.hwnd, GA_ROOT) == GetForegroundWindow()
                    && native_bar(state.hwnd, rail.horizontal).is_some_and(|bar| {
                        point.x >= bar.rcScrollBar.left
                            && point.x < bar.rcScrollBar.right
                            && point.y >= bar.rcScrollBar.top
                            && point.y < bar.rcScrollBar.bottom
                    })
                {
                    let packed = ((point.y as u16 as u32) << 16) | point.x as u16 as u32;
                    SendMessageW(
                        state.hwnd,
                        WM_NCLBUTTONDOWN,
                        Some(WPARAM(if rail.horizontal {
                            HTHSCROLL
                        } else {
                            HTVSCROLL
                        } as usize)),
                        Some(LPARAM(packed as isize)),
                    );
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                rail.hover.set(true);
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    ..Default::default()
                };
                let _ = TrackMouseEvent(&mut track);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                rail.hover.set(false);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_CANCELMODE => {
                state.cancel();
                LRESULT(0)
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => SendMessageW(state.hwnd, msg, Some(wp), Some(lp)),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
        sync::mpsc,
        time::{Duration, Instant},
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationTextPattern, UIA_TextPatternId,
    };

    struct Region(Option<HRGN>);
    impl Region {
        fn rectangle(r: RECT) -> Self {
            let handle = unsafe { CreateRectRgn(r.left, r.top, r.right, r.bottom) };
            assert!(!handle.0.is_null());
            Self(Some(handle))
        }
        fn handle(&self) -> HRGN {
            self.0.unwrap()
        }
        fn transfer_to(&mut self, hwnd: HWND) {
            assert_ne!(unsafe { SetWindowRgn(hwnd, self.0, false) }, 0);
            // Successful SetWindowRgn transfers ownership; never query/delete this handle.
            self.0 = None;
        }
    }
    impl Drop for Region {
        fn drop(&mut self) {
            if let Some(handle) = self.0 {
                unsafe {
                    let _ = DeleteObject(handle.into());
                }
            }
        }
    }

    fn bar_info(hwnd: HWND, object: OBJECT_IDENTIFIER) -> SCROLLBARINFO {
        let mut info = SCROLLBARINFO {
            cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetScrollBarInfo(hwnd, object, &mut info).unwrap();
        }
        info
    }

    fn relative_bar(hwnd: HWND, object: OBJECT_IDENTIFIER) -> Option<RECT> {
        let bar = bar_info(hwnd, object).rcScrollBar;
        let mut window = RECT::default();
        unsafe {
            GetWindowRect(hwnd, &mut window).unwrap();
        }
        (bar.right > bar.left && bar.bottom > bar.top).then_some(RECT {
            left: bar.left - window.left,
            top: bar.top - window.top,
            right: bar.right - window.left,
            bottom: bar.bottom - window.top,
        })
    }

    fn current_region(hwnd: HWND) -> Option<Region> {
        let region = Region::rectangle(RECT::default());
        (unsafe { GetWindowRgn(hwnd, region.handle()) }.0 != 0).then_some(region)
    }

    /// Explicit refresh only. This prototype installs no runtime subclass or host.
    fn exclude_native_bars(hwnd: HWND, native_mode: bool) -> Vec<RECT> {
        if native_mode {
            if current_region(hwnd).is_some() {
                assert_ne!(unsafe { SetWindowRgn(hwnd, None, false) }, 0);
            }
            return Vec::new();
        }
        let mut window = RECT::default();
        unsafe {
            GetWindowRect(hwnd, &mut window).unwrap();
        }
        let mut region = Region::rectangle(RECT {
            left: 0,
            top: 0,
            right: window.right - window.left,
            bottom: window.bottom - window.top,
        });
        let style = unsafe { GetWindowLongW(hwnd, GWL_STYLE) } as u32;
        let mut excluded = Vec::new();
        let horizontal = (style & WS_HSCROLL.0 != 0)
            .then(|| relative_bar(hwnd, OBJID_HSCROLL))
            .flatten();
        let vertical = (style & WS_VSCROLL.0 != 0)
            .then(|| relative_bar(hwnd, OBJID_VSCROLL))
            .flatten();
        excluded.extend(horizontal);
        excluded.extend(vertical);
        if let (Some(h), Some(v)) = (horizontal, vertical) {
            excluded.push(RECT {
                left: v.left,
                top: h.top,
                right: v.right,
                bottom: h.bottom,
            });
        }
        for r in &excluded {
            let cut = Region::rectangle(*r);
            assert_ne!(
                unsafe {
                    CombineRgn(
                        Some(region.handle()),
                        Some(region.handle()),
                        Some(cut.handle()),
                        RGN_DIFF,
                    )
                }
                .0,
                0
            );
        }
        let unchanged = current_region(hwnd)
            .is_some_and(|old| unsafe { EqualRgn(old.handle(), region.handle()).as_bool() });
        if !unchanged {
            region.transfer_to(hwnd);
        }
        excluded
    }

    fn assert_excluded(hwnd: HWND, excluded: &[RECT]) {
        let region =
            current_region(hwnd).expect("explicit region must be readable as a new owned copy");
        for rect in excluded {
            assert!(!unsafe { RectInRegion(region.handle(), rect).as_bool() });
        }
        let mut client = RECT::default();
        let mut origin = POINT::default();
        let mut window = RECT::default();
        unsafe {
            GetClientRect(hwnd, &mut client).unwrap();
            let _ = ClientToScreen(hwnd, &mut origin);
            GetWindowRect(hwnd, &mut window).unwrap();
        }
        let x = origin.x - window.left;
        let y = origin.y - window.top;
        for (px, py) in [
            (x, y),
            (x + client.right - 1, y),
            (x, y + client.bottom - 1),
            (x + client.right - 1, y + client.bottom - 1),
        ] {
            assert!(
                unsafe { PtInRegion(region.handle(), px, py).as_bool() },
                "region must retain every text-client corner"
            );
        }
    }

    struct Pair {
        owner: HWND,
        reference: HWND,
        clipped: HWND,
        font: HFONT,
    }
    impl Pair {
        fn new() -> Self {
            Self::with_wrap(false)
        }
        fn with_wrap(wrap: bool) -> Self {
            unsafe {
                let owner = super::super::tests::hidden_test_window_size(1100, 600);
                let create = |id: usize, x| {
                    CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        w!("EDIT"),
                        w!(""),
                        WS_CHILD
                            | WS_VISIBLE
                            | WS_BORDER
                            | WS_VSCROLL
                            | WS_TABSTOP
                            | if wrap {
                                WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32)
                            } else {
                                WS_HSCROLL
                                    | WINDOW_STYLE(
                                        (ES_MULTILINE | ES_AUTOHSCROLL | ES_AUTOVSCROLL) as u32,
                                    )
                            },
                        x,
                        0,
                        450,
                        250,
                        Some(owner),
                        Some(HMENU(id as *mut _)),
                        None,
                        None,
                    )
                    .unwrap()
                };
                let reference = create(7801, 0);
                let clipped = create(7802, 500);
                let mut pair = Self {
                    owner,
                    reference,
                    clipped,
                    font: HFONT::default(),
                };
                pair.font(160);
                pair.text(&indexed_text());
                pair.both(EM_SETSEL, WPARAM(3), LPARAM(8));
                pair
            }
        }
        fn both(&self, msg: u32, wp: WPARAM, lp: LPARAM) {
            unsafe {
                SendMessageW(self.reference, msg, Some(wp), Some(lp));
                SendMessageW(self.clipped, msg, Some(wp), Some(lp));
            }
        }
        fn text(&self, text: &str) {
            self.both(EM_SETLIMITTEXT, WPARAM(0x7fff_ffff), LPARAM(0));
            self.both(WM_SETTEXT, WPARAM(0), LPARAM(wide(text).as_ptr() as isize));
        }
        fn font(&mut self, size: i32) {
            let mut data = LOGFONTW {
                lfHeight: -size,
                ..Default::default()
            };
            let face = wide(shell::UI_FONT_FAMILY);
            data.lfFaceName[..face.len()].copy_from_slice(&face);
            let next = unsafe { CreateFontIndirectW(&data) };
            assert_ne!(next, HFONT::default());
            self.both(WM_SETFONT, WPARAM(next.0 as usize), LPARAM(0));
            unsafe {
                let _ = DeleteObject(self.font.into());
            }
            self.font = next;
        }
    }
    impl Drop for Pair {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.owner).unwrap();
                let _ = DeleteObject(self.font.into());
            }
        }
    }

    fn large_vertical_text() -> String {
        (0..70000)
            .map(|index| format!("行{index:05} 漢字 e\u{301} 😀\r\n"))
            .collect()
    }

    fn indexed_text() -> String {
        let mut text = String::new();
        for index in 0..2000 {
            text.push(char::from_u32(0x4e00 + index).unwrap());
            if index % 31 == 0 {
                text.push_str(&format!("⟦{index:04X}⟧e\u{301}👩‍💻\t"));
            }
        }
        for row in 0..60 {
            text.push_str(&format!("\r\n行{row:03} 日本語 e\u{301} 👩‍💻"));
        }
        text
    }

    fn scroll(hwnd: HWND, axis: SCROLLBAR_CONSTANTS) -> Option<[i64; 5]> {
        let mut info = SCROLLINFO {
            cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
            fMask: SIF_ALL,
            ..Default::default()
        };
        unsafe { GetScrollInfo(hwnd, axis, &mut info) }.ok()?;
        Some([
            i64::from(info.nMin),
            i64::from(info.nMax),
            i64::from(info.nPage),
            i64::from(info.nPos),
            i64::from(info.nTrackPos),
        ])
    }

    #[derive(Debug, PartialEq)]
    struct Semantics {
        style: isize,
        client: RECT,
        format: RECT,
        h: Option<[i64; 5]>,
        v: Option<[i64; 5]>,
        first_line: isize,
        near_index: u16,
        near_position: isize,
        text: String,
        selection: [u32; 2],
        undo: isize,
    }
    fn semantics(hwnd: HWND) -> Semantics {
        unsafe {
            let mut client = RECT::default();
            let mut format = RECT::default();
            GetClientRect(hwnd, &mut client).unwrap();
            SendMessageW(
                hwnd,
                EM_GETRECT,
                Some(WPARAM(0)),
                Some(LPARAM((&mut format as *mut RECT) as isize)),
            );
            let point =
                (((format.top + 8) as u16 as u32) << 16) | ((format.left + 8) as u16 as u32);
            let near_index = SendMessageW(
                hwnd,
                EM_CHARFROMPOS,
                Some(WPARAM(0)),
                Some(LPARAM(point as isize)),
            )
            .0 as u16;
            let near_position = SendMessageW(
                hwnd,
                EM_POSFROMCHAR,
                Some(WPARAM(near_index as usize)),
                Some(LPARAM(0)),
            )
            .0;
            let mut selection = [0u32; 2];
            SendMessageW(
                hwnd,
                EM_GETSEL,
                Some(WPARAM(selection.as_mut_ptr() as usize)),
                Some(LPARAM(selection.as_mut_ptr().add(1) as isize)),
            );
            Semantics {
                style: GetWindowLongPtrW(hwnd, GWL_STYLE),
                client,
                format,
                h: scroll(hwnd, SB_HORZ),
                v: scroll(hwnd, SB_VERT),
                first_line: SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0,
                near_index,
                near_position,
                text: shell::text_of(hwnd),
                selection,
                undo: SendMessageW(hwnd, EM_CANUNDO, None, None).0,
            }
        }
    }

    /// WM_PRINTCLIENT is only a client-content witness; it does not prove OS-region clipping.
    fn client_hash(hwnd: HWND) -> u64 {
        unsafe {
            let mut client = RECT::default();
            GetClientRect(hwnd, &mut client).unwrap();
            let source = GetDC(Some(hwnd));
            let dc = CreateCompatibleDC(Some(source));
            let bitmap = CreateCompatibleBitmap(source, client.right, client.bottom);
            let previous = SelectObject(dc, bitmap.into());
            ReleaseDC(Some(hwnd), source);
            let brush = CreateSolidBrush(COLORREF(0x27a16c));
            FillRect(dc, &client, brush);
            let _ = DeleteObject(brush.into());
            SendMessageW(
                hwnd,
                WM_PRINTCLIENT,
                Some(WPARAM(dc.0 as usize)),
                Some(LPARAM((PRF_CLIENT | PRF_ERASEBKGND) as isize)),
            );
            let mut hash = DefaultHasher::new();
            for y in 0..client.bottom {
                for x in 0..client.right {
                    GetPixel(dc, x, y).0.hash(&mut hash);
                }
            }
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
            hash.finish()
        }
    }

    #[test]
    fn edit_region_preserves_native_numeric_h_range_client_and_unicode_content() {
        let pair = Pair::new();
        assert_eq!(semantics(pair.reference), semantics(pair.clipped));
        let before = semantics(pair.clipped);
        let initial_pixels = client_hash(pair.clipped);
        let excluded = exclude_native_bars(pair.clipped, false);
        assert!(excluded.len() >= 2);
        assert_excluded(pair.clipped, &excluded);
        assert_eq!(semantics(pair.clipped), before);
        assert_eq!(client_hash(pair.reference), client_hash(pair.clipped));
        pair.both(EM_LINESCROLL, WPARAM(800), LPARAM(0));
        let moved = semantics(pair.clipped);
        assert_eq!(semantics(pair.reference), moved);
        assert!(
            moved.h.unwrap()[3] > 65535,
            "numeric native H range must be exercised; high-H ink remains unverified"
        );
        assert_ne!(moved.near_index, before.near_index);
        let moved_pixels = client_hash(pair.clipped);
        assert_eq!(client_hash(pair.reference), moved_pixels);
        assert_ne!(
            initial_pixels, moved_pixels,
            "indexed native viewport raster must actually change"
        );
        pair.both(EM_LINESCROLL, WPARAM(0), LPARAM(10));
        assert_eq!(semantics(pair.reference), semantics(pair.clipped));
        assert!(semantics(pair.clipped).first_line > 0);
        assert_eq!(unsafe { GetParent(pair.clipped).unwrap() }, pair.owner);
        assert_eq!(unsafe { GetDlgCtrlID(pair.clipped) }, 7802);
        assert!(!unsafe { IsWindowVisible(pair.owner).as_bool() });
        println!(
            "REGION_GATE nativeH={} near_index={} client={:?} pixels_before={initial_pixels:016x} pixels_after={moved_pixels:016x} region_exclusions={} hidden=true high_h_ink_unverified=true",
            moved.h.unwrap()[3],
            moved.near_index,
            moved.client,
            excluded.len()
        );
    }

    #[test]
    fn edit_region_refresh_and_native_mode_preserve_native_resize_font_undo() {
        let mut pair = Pair::new();
        let start = semantics(pair.clipped);
        pair.both(
            EM_REPLACESEL,
            WPARAM(1),
            LPARAM(wide("変更").as_ptr() as isize),
        );
        assert_eq!(semantics(pair.reference), semantics(pair.clipped));
        assert_ne!(semantics(pair.clipped).undo, 0);
        let excluded = exclude_native_bars(pair.clipped, false);
        assert_excluded(pair.clipped, &excluded);
        pair.both(EM_UNDO, WPARAM(0), LPARAM(0));
        assert_eq!(semantics(pair.reference), semantics(pair.clipped));
        assert_eq!(semantics(pair.clipped).text, start.text);
        for small in [true, false] {
            pair.text(if small { "短い" } else { &start.text });
            pair.font(if small { 18 } else { 160 });
            unsafe {
                for hwnd in [pair.reference, pair.clipped] {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        if small { 520 } else { 450 },
                        if small { 400 } else { 250 },
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
            }
            let expected = semantics(pair.reference);
            let excluded = exclude_native_bars(pair.clipped, false);
            assert_excluded(pair.clipped, &excluded);
            assert_eq!(semantics(pair.clipped), expected);
            let stable = current_region(pair.clipped).unwrap();
            exclude_native_bars(pair.clipped, false);
            assert!(unsafe {
                EqualRgn(
                    stable.handle(),
                    current_region(pair.clipped).unwrap().handle(),
                )
                .as_bool()
            });
            assert_eq!(semantics(pair.clipped), expected);
            exclude_native_bars(pair.clipped, true);
            assert!(current_region(pair.clipped).is_none());
            assert_eq!(semantics(pair.clipped), expected);
        }
        println!(
            "REGION_GATE refresh_idempotent=true native_mode_remove=true hwnd_retained=true undo_preserved=true; native mode is a local geometry/region test, not system HC acceptance"
        );
    }

    #[derive(Debug, PartialEq)]
    struct UiaNode {
        kind: i32,
        rectangle: [i32; 4],
        offscreen: bool,
    }
    #[derive(Debug)]
    struct UiaObservation {
        nodes: Vec<UiaNode>,
        text_hash: Option<u64>,
    }

    fn uia_observe(hwnd: HWND) -> Result<UiaObservation, String> {
        let raw = hwnd.0 as usize;
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            use windows::core::Interface;
            unsafe {
                let result = CoInitializeEx(None, COINIT_MULTITHREADED).ok().and_then(|_| {
                let result = (|| -> windows::core::Result<UiaObservation> {
                    let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
                    let element = automation.ElementFromHandle(HWND(raw as *mut _))?;
                    let node = |value: &windows::Win32::UI::Accessibility::IUIAutomationElement| -> windows::core::Result<UiaNode> {
                        let r = value.CurrentBoundingRectangle()?;
                        Ok(UiaNode { kind: value.CurrentControlType()?.0, rectangle: [r.left, r.top, r.right, r.bottom], offscreen: value.CurrentIsOffscreen()?.as_bool() })
                    };
                    let mut nodes = vec![node(&element)?];
                    let walker = automation.RawViewWalker()?;
                    let mut child = walker.GetFirstChildElement(&element).ok();
                    for _ in 0..8 {
                        let Some(current) = child else { break; };
                        nodes.push(node(&current)?);
                        child = walker.GetNextSiblingElement(&current).ok();
                    }
                    let text_hash = element.GetCurrentPattern(UIA_TextPatternId).ok()
                        .and_then(|pattern| pattern.cast::<IUIAutomationTextPattern>().ok())
                        .and_then(|pattern| pattern.DocumentRange().ok())
                        .and_then(|range| range.GetText(-1).ok()).map(|text| { let mut hash = DefaultHasher::new(); text.to_string().hash(&mut hash); hash.finish() });
                    Ok(UiaObservation { nodes, text_hash })
                })();
                CoUninitialize();
                result
            }).map_err(|error| error.to_string());
                let _ = sender.send(result);
            }
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match receiver.recv_timeout(Duration::from_millis(2)) {
                Ok(result) => {
                    worker.join().unwrap();
                    return result;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("UIA worker disconnected".into());
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if Instant::now() >= deadline {
                return Err("UIA bounded query timed out; visibility unverified".into());
            }
            unsafe {
                let mut message = MSG::default();
                while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }

    #[test]
    fn edit_region_hidden_uia_observes_native_bounds_without_claiming_visible_clipping() {
        let pair = Pair::new();
        let before = uia_observe(pair.clipped);
        let excluded = exclude_native_bars(pair.clipped, false);
        assert_excluded(pair.clipped, &excluded);
        let Ok(before) = before else {
            println!("REGION_UIA unavailable={before:?}; factory gate remains pending");
            return;
        };
        let after = uia_observe(pair.clipped);
        if let Ok(after) = &after {
            assert_eq!(before.text_hash, after.text_hash);
            assert_eq!(before.nodes.first(), after.nodes.first());
        }
        println!(
            "REGION_UIA before={before:?} after={after:?} h_native_state={:?} v_native_state={:?}; all HWNDs hidden, so visible excluded-scrollbar UIA correctness is UNVERIFIED",
            bar_info(pair.clipped, OBJID_HSCROLL).rgstate,
            bar_info(pair.clipped, OBJID_VSCROLL).rgstate
        );
    }

    const GATE_TITLE: PCWSTR = w!("MDLite Native EDIT Scroll Gate");
    const GATE_SNAPSHOT: u32 = WM_APP + 421;
    const GATE_REFERENCE: i32 = 7901;
    const GATE_CLIPPED: i32 = 7902;

    struct InteractiveGate {
        hwnd: Cell<HWND>,
        reference: Cell<HWND>,
        clipped: Cell<HWND>,
        rails: [Cell<HWND>; 2],
        delegating: Cell<HWND>,
        closed: Cell<bool>,
        ready: Cell<bool>,
        io_failed: Cell<bool>,
        sequence: Cell<u64>,
        started: Instant,
        directory: PathBuf,
        brush: HBRUSH,
    }

    fn rectangle_value(r: RECT) -> serde_json::Value {
        serde_json::json!([r.left, r.top, r.right, r.bottom])
    }

    fn fixture_sha256(text: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for unit in text.encode_utf16() {
            hash.update(unit.to_le_bytes());
        }
        format!("{:x}", hash.finalize())
    }

    fn gate_bar(hwnd: HWND, horizontal: bool) -> Option<SCROLLBARINFO> {
        let mut info = SCROLLBARINFO {
            cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetScrollBarInfo(
                hwnd,
                if horizontal {
                    OBJID_HSCROLL
                } else {
                    OBJID_VSCROLL
                },
                &mut info,
            )
        }
        .ok()?;
        Some(info)
    }

    fn gate_control_state(hwnd: HWND, raster: bool) -> serde_json::Value {
        unsafe {
            if hwnd == HWND::default() || !IsWindow(Some(hwnd)).as_bool() {
                return serde_json::json!({"hwnd": hwnd.0 as usize, "valid": false});
            }
            let mut outer = RECT::default();
            let mut client = RECT::default();
            let mut format = RECT::default();
            let _ = GetWindowRect(hwnd, &mut outer);
            let _ = GetClientRect(hwnd, &mut client);
            SendMessageW(
                hwnd,
                EM_GETRECT,
                Some(WPARAM(0)),
                Some(LPARAM((&mut format as *mut RECT) as isize)),
            );
            let first_line = SendMessageW(hwnd, EM_GETFIRSTVISIBLELINE, None, None).0;
            let first_index =
                SendMessageW(hwnd, EM_LINEINDEX, Some(WPARAM(first_line as usize)), None).0;
            let first_position = SendMessageW(
                hwnd,
                EM_POSFROMCHAR,
                Some(WPARAM(first_index as usize)),
                None,
            )
            .0;
            let point =
                (((format.top + 8) as u16 as u32) << 16) | ((format.left + 8) as u16 as u32);
            let near = SendMessageW(
                hwnd,
                EM_CHARFROMPOS,
                Some(WPARAM(0)),
                Some(LPARAM(point as isize)),
            )
            .0 as u16;
            let position = SendMessageW(
                hwnd,
                EM_POSFROMCHAR,
                Some(WPARAM(near as usize)),
                Some(LPARAM(0)),
            )
            .0;
            let mut selection = [0u32; 2];
            SendMessageW(
                hwnd,
                EM_GETSEL,
                Some(WPARAM(selection.as_mut_ptr() as usize)),
                Some(LPARAM(selection.as_mut_ptr().add(1) as isize)),
            );
            let bar = |horizontal| {
                gate_bar(hwnd, horizontal).map(|value| serde_json::json!({
            "screen_rect": rectangle_value(value.rcScrollBar), "line_button": value.dxyLineButton,
            "thumb_start": value.xyThumbTop, "thumb_end": value.xyThumbBottom, "states": value.rgstate
        }))
            };
            serde_json::json!({
                "hwnd": hwnd.0 as usize, "valid": true, "id": GetDlgCtrlID(hwnd),
                "parent": GetParent(hwnd).unwrap_or_default().0 as usize,
                "style": GetWindowLongPtrW(hwnd, GWL_STYLE), "screen_rect": rectangle_value(outer),
                "client": rectangle_value(client), "format": rectangle_value(format),
                "h_min_max_page_pos_track": scroll(hwnd, SB_HORZ), "v_min_max_page_pos_track": scroll(hwnd, SB_VERT),
                "h_bar": bar(true), "v_bar": bar(false), "near_index16": near,
                "near_x_signed16": position as u16 as i16, "near_y_signed16": ((position as u64 >> 16) as u16) as i16,
                "first_line": first_line, "first_index":first_index,
                "first_anchor_x_signed16":first_position as u16 as i16,
                "first_anchor_y_signed16":((first_position as u64 >> 16) as u16) as i16,
                "selection": selection, "can_undo": SendMessageW(hwnd, EM_CANUNDO, None, None).0,
                "modify": SendMessageW(hwnd, EM_GETMODIFY, None, None).0,
                "text_utf16_sha256": fixture_sha256(&shell::text_of(hwnd)),
                "client_raster_hash": raster.then(|| format!("{:016x}", client_hash(hwnd))),
            })
        }
    }

    impl InteractiveGate {
        fn emit(&self, event: &str, detail: serde_json::Value, raster: bool) {
            use std::io::Write;
            if !self.ready.get() {
                return;
            }
            let sequence = self.sequence.get().saturating_add(1);
            self.sequence.set(sequence);
            let region_excludes = current_region(self.clipped.get()).and_then(|region| {
                let mut window = RECT::default();
                unsafe { GetWindowRect(self.clipped.get(), &mut window) }.ok()?;
                Some([true, false].into_iter().all(|horizontal| {
                    gate_bar(self.clipped.get(), horizontal).is_some_and(|bar| {
                        let r = bar.rcScrollBar;
                        let relative = RECT {
                            left: r.left - window.left,
                            top: r.top - window.top,
                            right: r.right - window.left,
                            bottom: r.bottom - window.top,
                        };
                        !unsafe { RectInRegion(region.handle(), &relative).as_bool() }
                    })
                }))
            });
            let value = serde_json::json!({"schema":1,"sequence":sequence,"elapsed_ms":self.started.elapsed().as_millis(),
            "event":event,"detail":detail,"pid":std::process::id(),"host":self.hwnd.get().0 as usize,
            "foreground":unsafe { GetForegroundWindow().0 as usize },"focus":unsafe { GetFocus().0 as usize },
            "capture":unsafe { GetCapture().0 as usize },"delegating":self.delegating.get().0 as usize,
            "region_excludes_native_bars":region_excludes,
            "reference":gate_control_state(self.reference.get(),raster),"clipped":gate_control_state(self.clipped.get(),raster)});
            let write = || -> Result<(), Box<dyn std::error::Error>> {
                let mut line = serde_json::to_vec(&value)?;
                line.push(b'\n');
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(self.directory.join("events.jsonl"))?
                    .write_all(&line)?;
                let temporary = self.directory.join("state.tmp.json");
                std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
                std::fs::rename(temporary, self.directory.join("state.json"))?;
                Ok(())
            };
            if let Err(error) = write()
                && !self.io_failed.replace(true)
            {
                eprintln!("scroll gate receipt failure: {error}");
            }
        }
        fn cancel(&self) {
            for edit in [self.reference.get(), self.clipped.get()] {
                if unsafe { IsWindow(Some(edit)).as_bool() } {
                    unsafe {
                        SendMessageW(edit, WM_CANCELMODE, None, None);
                    }
                }
            }
        }
        fn sync_rails(&self) {
            unsafe {
                if attached(self.clipped.get()).is_some() {
                    let _ = refresh(self.clipped.get(), shell::Tokens::default());
                }
            }
        }
    }

    unsafe extern "system" fn gate_edit_proc(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        id: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            let pointer = data as *const InteractiveGate;
            Rc::increment_strong_count(pointer);
            let gate = Rc::from_raw(pointer);
            let scroll_event = matches!(msg, WM_HSCROLL | WM_VSCROLL);
            let detail = || serde_json::json!({"edit":hwnd.0 as usize,"message":msg,"code":wp.0 & 0xffff,"position16":(wp.0 >> 16) & 0xffff,"lparam":lp.0});
            if scroll_event {
                gate.emit("native_scroll_before", detail(), false);
            }
            if msg == WM_NCLBUTTONDOWN {
                gate.emit("native_nc_enter", detail(), false);
            }
            if msg == WM_NCDESTROY {
                let _ = RemoveWindowSubclass(hwnd, Some(gate_edit_proc), id);
                drop(Rc::from_raw(pointer));
            }
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if scroll_event || matches!(msg, WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_KEYDOWN) {
                gate.sync_rails();
                gate.emit("native_scroll_after", detail(), false);
            }
            if matches!(msg, WM_CAPTURECHANGED | WM_CANCELMODE) {
                gate.sync_rails();
                gate.emit("native_capture_or_cancel", detail(), false);
            }
            if msg == WM_NCLBUTTONDOWN {
                gate.sync_rails();
                gate.emit("native_nc_return", detail(), true);
            }
            result
        }
    }

    unsafe extern "system" fn gate_host_proc(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        unsafe {
            if msg == WM_NCCREATE {
                let create = &*(lp.0 as *const CREATESTRUCTW);
                let gate = &*(create.lpCreateParams as *const Rc<InteractiveGate>);
                gate.hwnd.set(hwnd);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, Rc::into_raw(Rc::clone(gate)) as isize);
            }
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const InteractiveGate;
            if pointer.is_null() {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            Rc::increment_strong_count(pointer);
            let gate = Rc::from_raw(pointer);
            match msg {
                WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                    let dc = HDC(wp.0 as *mut _);
                    SetTextColor(dc, shell::rgb(shell::Tokens::default().foreground));
                    SetBkColor(dc, shell::rgb(shell::Tokens::default().panel));
                    LRESULT(gate.brush.0 as isize)
                }
                WM_ERASEBKGND => {
                    let mut r = RECT::default();
                    let _ = GetClientRect(hwnd, &mut r);
                    FillRect(HDC(wp.0 as *mut _), &r, gate.brush);
                    LRESULT(1)
                }
                WM_COMMAND => {
                    gate.emit(
                        "parent_notification",
                        serde_json::json!({"id":wp.0&0xffff,"notification":wp.0>>16,"child":lp.0}),
                        false,
                    );
                    LRESULT(0)
                }
                GATE_SNAPSHOT => {
                    gate.emit("snapshot_requested", serde_json::Value::Null, true);
                    LRESULT(0)
                }
                WM_CANCELMODE => {
                    gate.cancel();
                    gate.emit("host_cancel", serde_json::Value::Null, true);
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
                WM_CLOSE => {
                    gate.emit("close_requested", serde_json::Value::Null, true);
                    gate.cancel();
                    let _ = DestroyWindow(hwnd);
                    LRESULT(0)
                }
                WM_DESTROY => {
                    gate.closed.set(true);
                    PostQuitMessage(0);
                    LRESULT(0)
                }
                WM_NCDESTROY => {
                    gate.ready.set(false);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Rc::from_raw(pointer));
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
                _ => DefWindowProcW(hwnd, msg, wp, lp),
            }
        }
    }

    /// Root launches a copied test executable with the exact filter and --ignored.
    /// This fixture performs no automatic input. It ends only on the owned WM_CLOSE.
    pub(super) fn edit_region_interactive_native_nc_delegation_gate() {
        let directory = PathBuf::from(
            std::env::var_os("MDLITE_SCROLL_GATE_DIRECTORY")
                .expect("explicit run directory required"),
        );
        assert!(directory.is_absolute() && directory.is_dir());
        for name in ["identity.json", "events.jsonl", "state.json"] {
            assert!(
                !directory.join(name).exists(),
                "fresh run directory required"
            );
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("events.jsonl"))
            .unwrap();
        unsafe {
            let instance = GetModuleHandleW(None).unwrap().into();
            let class = wide(&format!(
                "MDLiteNativeEditScrollGate-{}",
                std::process::id()
            ));
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(gate_host_proc),
                hInstance: instance,
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            });
            let gate = Rc::new(InteractiveGate {
                hwnd: Cell::new(HWND::default()),
                reference: Cell::new(HWND::default()),
                clipped: Cell::new(HWND::default()),
                rails: [Cell::new(HWND::default()), Cell::new(HWND::default())],
                delegating: Cell::new(HWND::default()),
                closed: Cell::new(false),
                ready: Cell::new(false),
                io_failed: Cell::new(false),
                sequence: Cell::new(0),
                started: Instant::now(),
                directory: directory.clone(),
                brush: CreateSolidBrush(shell::rgb(shell::Tokens::default().panel)),
            });
            let host = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                PCWSTR(class.as_ptr()),
                GATE_TITLE,
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                1100,
                430,
                None,
                None,
                Some(instance),
                Some((&gate as *const Rc<InteractiveGate>).cast()),
            )
            .unwrap();
            for (index, title) in ["Native reference", "Region excluded + native delegation"]
                .into_iter()
                .enumerate()
            {
                let label = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    PCWSTR(wide(title).as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    24 + index as i32 * 540,
                    18,
                    500,
                    28,
                    Some(host),
                    None,
                    Some(instance),
                    None,
                )
                .unwrap();
                shell::send(
                    label,
                    WM_SETFONT,
                    WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize),
                    LPARAM(0),
                );
            }
            let large_v =
                std::env::var("MDLITE_SCROLL_GATE_MODE").ok().as_deref() == Some("LARGEV");
            let mut data = LOGFONTW {
                lfHeight: if large_v { -14 } else { -160 },
                ..Default::default()
            };
            let face = wide(shell::UI_FONT_FAMILY);
            data.lfFaceName[..face.len()].copy_from_slice(&face);
            let font = CreateFontIndirectW(&data);
            assert_ne!(font, HFONT::default());
            let text = if large_v {
                large_vertical_text()
            } else {
                indexed_text()
            };
            for (index, id) in [GATE_REFERENCE, GATE_CLIPPED].into_iter().enumerate() {
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!(""),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_BORDER
                        | WS_HSCROLL
                        | WS_VSCROLL
                        | WS_TABSTOP
                        | WINDOW_STYLE(
                            (ES_MULTILINE | ES_AUTOHSCROLL | ES_AUTOVSCROLL | ES_READONLY) as u32,
                        ),
                    24 + index as i32 * 540,
                    58,
                    450,
                    250,
                    Some(host),
                    Some(HMENU(id as usize as *mut _)),
                    Some(instance),
                    None,
                )
                .unwrap();
                if index == 0 {
                    gate.reference.set(edit);
                } else {
                    gate.clipped.set(edit);
                }
                shell::send(edit, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(0));
                shell::send(edit, EM_SETLIMITTEXT, WPARAM(0x7fff_ffff), LPARAM(0));
                shell::send(
                    edit,
                    WM_SETTEXT,
                    WPARAM(0),
                    LPARAM(wide(&text).as_ptr() as isize),
                );
                shell::send(edit, EM_SETSEL, WPARAM(3), LPARAM(8));
                if index == 1 {
                    attach(edit, shell::Tokens::default()).unwrap();
                }
                let pointer = Rc::into_raw(Rc::clone(&gate));
                if !SetWindowSubclass(edit, Some(gate_edit_proc), 791, pointer as usize).as_bool() {
                    drop(Rc::from_raw(pointer));
                    panic!("native gate subclass unavailable");
                }
            }
            let handles = rail_handles(gate.clipped.get()).unwrap();
            for (cell, handle) in gate.rails.iter().zip(handles) {
                cell.set(handle);
            }
            gate.sync_rails();
            let identity = serde_json::json!({"schema":2,"production_adapter":true,"mode":if large_v {"LARGEV"} else {"LEGACY_H"},"test":"native::edit_scroll::edit_region_interactive_native_nc_delegation_gate","title":"MDLite Native EDIT Scroll Gate","pid":std::process::id(),"thread":windows_sys::Win32::System::Threading::GetCurrentThreadId(),"host":host.0 as usize,"reference":gate.reference.get().0 as usize,"clipped":gate.clipped.get().0 as usize,"horizontal_rail":gate.rails[0].get().0 as usize,"vertical_rail":gate.rails[1].get().0 as usize,"snapshot_message":GATE_SNAPSHOT,"cancel_message":WM_CANCELMODE,"close_message":WM_CLOSE,"fixture_utf16_sha256":fixture_sha256(&text),"foreground_attempts":1});
            std::fs::write(
                directory.join("identity.json"),
                serde_json::to_vec_pretty(&identity).unwrap(),
            )
            .unwrap();
            gate.ready.set(true);
            let _ = ShowWindow(host, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(host);
            let activated = SetForegroundWindow(host).as_bool();
            gate.emit("prepared_foreground_attempt",serde_json::json!({"api_result":activated,"actual_foreground_is_host":GetForegroundWindow()==host,"attempts":1}),true);
            println!(
                "SCROLL_GATE_READY pid={} host={} foreground={} directory={}",
                std::process::id(),
                host.0 as usize,
                GetForegroundWindow() == host,
                directory.display()
            );
            let mut message = MSG::default();
            while !gate.closed.get() && GetMessageW(&mut message, None, 0, 0).0 > 0 {
                if !IsDialogMessageW(host, &message).as_bool() {
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            if IsWindow(Some(host)).as_bool() {
                gate.cancel();
                let _ = DestroyWindow(host);
            }
            let _ = DeleteObject(font.into());
            let _ = DeleteObject(gate.brush.into());
            let _ = UnregisterClassW(PCWSTR(class.as_ptr()), Some(instance));
            assert!(!gate.io_failed.get(), "receipt IO failed");
        }
    }

    #[test]
    fn edit_adapter_region_client_font_resize_undo_hc_and_lifecycle() {
        unsafe {
            let mut pair = Pair::new();
            pair.font(14);
            let before = semantics(pair.reference);
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            assert_eq!(semantics(pair.clipped), before);
            let cuts: Vec<_> = [OBJID_HSCROLL, OBJID_VSCROLL]
                .into_iter()
                .filter_map(|id| relative_bar(pair.clipped, id))
                .collect();
            assert_excluded(pair.clipped, &cuts);
            let rails = rail_handles(pair.clipped).unwrap();
            for (horizontal, rail) in [true, false].into_iter().zip(rails) {
                let mut rect = RECT::default();
                GetWindowRect(rail, &mut rect).unwrap();
                assert_eq!(
                    rectangle_value(rect),
                    rectangle_value(native_bar(pair.clipped, horizontal).unwrap().rcScrollBar)
                );
                assert_eq!(GetParent(rail).unwrap(), pair.owner);
                assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_TABSTOP.0, 0);
            }
            pair.both(
                EM_REPLACESEL,
                WPARAM(1),
                LPARAM(wide("変更").as_ptr() as isize),
            );
            assert_ne!(semantics(pair.clipped).undo, 0);
            pair.both(EM_UNDO, WPARAM(0), LPARAM(0));
            assert_eq!(semantics(pair.clipped), semantics(pair.reference));
            for size in [18, 32, 14] {
                pair.font(size);
                for hwnd in [pair.reference, pair.clipped] {
                    SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        500,
                        350,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
                refresh(pair.clipped, shell::Tokens::default()).unwrap();
                assert_eq!(semantics(pair.clipped), semantics(pair.reference));
            }
            let tokens = shell::Tokens {
                high_contrast: true,
                ..Default::default()
            };
            refresh(pair.clipped, tokens).unwrap();
            assert!(current_region(pair.clipped).is_none());
            assert!(
                rails
                    .iter()
                    .all(|rail| GetWindowLongW(*rail, GWL_STYLE) as u32 & WS_VISIBLE.0 == 0)
            );
            refresh(pair.clipped, shell::Tokens::default()).unwrap();
            assert!(current_region(pair.clipped).is_some());
            assert_eq!(semantics(pair.clipped), semantics(pair.reference));
            let _ = EnableWindow(pair.clipped, false);
            assert!(current_region(pair.clipped).is_none());
            let _ = EnableWindow(pair.clipped, true);
            let _ = ShowWindow(pair.clipped, SW_HIDE);
            assert!(current_region(pair.clipped).is_none());
            let _ = ShowWindow(pair.clipped, SW_SHOWNOACTIVATE);
            refresh(pair.clipped, shell::Tokens::default()).unwrap();
            DestroyWindow(pair.clipped).unwrap();
            assert!(GetPropW(pair.clipped, PROPERTY).is_invalid());
            assert!(rails.iter().all(|rail| !IsWindow(Some(*rail)).as_bool()));
            assert_eq!(GetCapture(), HWND::default());
        }
    }

    #[test]
    fn edit_adapter_preserves_original_region_and_detaches_cleanly() {
        unsafe {
            let pair = Pair::new();
            let custom = Region::rectangle(RECT {
                left: 0,
                top: 0,
                right: 449,
                bottom: 249,
            });
            let saved = OwnedRegion::rectangle(RECT::default()).unwrap();
            assert_ne!(
                CombineRgn(Some(saved.0), Some(custom.handle()), None, RGN_COPY).0,
                0
            );
            let mut custom = custom;
            custom.transfer_to(pair.clipped);
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            let rails = rail_handles(pair.clipped).unwrap();
            let state = attached(pair.clipped).unwrap();
            state.restore().unwrap();
            assert!(!state.updating.get());
            assert!(!state.clipped.get());
            assert!(EqualRgn(saved.0, current_region(pair.clipped).unwrap().handle()).as_bool());
            // Nested restore preserves an existing sync guard, too.
            refresh(pair.clipped, shell::Tokens::default()).unwrap();
            state.updating.set(true);
            state.restore().unwrap();
            assert!(state.updating.get());
            state.updating.set(false);
            refresh(pair.clipped, shell::Tokens::default()).unwrap();
            detach(pair.clipped).unwrap();
            let restored = current_region(pair.clipped).unwrap();
            assert!(EqualRgn(saved.0, restored.handle()).as_bool());
            assert!(attached(pair.clipped).is_none());
            assert!(
                rails
                    .into_iter()
                    .all(|rail| !IsWindow(Some(rail)).as_bool())
            );
            assert_eq!(semantics(pair.clipped), semantics(pair.reference));
        }
    }

    #[test]
    fn edit_adapter_preserves_native_large_vertical_ranges_and_source() {
        unsafe {
            let mut pair = Pair::new();
            pair.font(14);
            pair.text(&large_vertical_text());
            pair.both(EM_SETSEL, WPARAM(3), LPARAM(8));
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            let original = semantics(pair.clipped);
            assert!(original.v.unwrap()[1] > 65535);
            // Actual native relative scrolling, not synthetic thumb state or a
            // substitute for the separate physical NC tracking gate.
            for target in [65540, 69979, 57634, 0] {
                let first =
                    SendMessageW(pair.reference, EM_GETFIRSTVISIBLELINE, None, None).0 as i32;
                pair.both(EM_LINESCROLL, WPARAM(0), LPARAM((target - first) as isize));
                let actual = semantics(pair.clipped);
                assert_eq!(actual.first_line, target as isize);
                assert_eq!(actual.v.unwrap()[3], target as i64);
                assert_eq!(actual, semantics(pair.reference));
            }
            cancel(pair.clipped);
            SendMessageW(pair.reference, WM_CANCELMODE, None, None);
            let after = semantics(pair.clipped);
            assert_eq!(after, semantics(pair.reference));
            assert_eq!(after.text, original.text);
            assert_eq!(after.selection, original.selection);
            assert_eq!(after.undo, original.undo);
            assert_eq!(after.style, original.style);
            assert_eq!(after.client, original.client);
            assert_eq!(after.format, original.format);
        }
    }

    #[test]
    #[ignore = "temporarily visible NOACTIVATE fixture; requires exclusive GUI lease"]
    fn edit_adapter_completed_native_mutations_refresh_sibling_geometry_and_paint() {
        struct Observation {
            edit: HWND,
            horizontal: bool,
            refreshes: Cell<usize>,
            paints: Cell<usize>,
            native: Cell<Option<([i64; 5], [i32; 2])>>,
        }
        impl Observation {
            unsafe fn native_geometry(&self) -> Option<([i64; 5], [i32; 2])> {
                unsafe {
                    let bar = native_bar(self.edit, self.horizontal)?;
                    let range = scroll(self.edit, if self.horizontal { SB_HORZ } else { SB_VERT })?;
                    Some((range, [bar.xyThumbTop, bar.xyThumbBottom]))
                }
            }
        }
        unsafe extern "system" fn observe(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            id: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                let pointer = data as *const Observation;
                Rc::increment_strong_count(pointer);
                let observation = Rc::from_raw(pointer);
                if msg == WM_WINDOWPOSCHANGING {
                    observation.refreshes.set(observation.refreshes.get() + 1);
                    observation.native.set(observation.native_geometry());
                }
                if msg == WM_PAINT {
                    observation.paints.set(observation.paints.get() + 1);
                }
                if msg == WM_NCDESTROY {
                    let _ = RemoveWindowSubclass(hwnd, Some(observe), id);
                    drop(Rc::from_raw(pointer));
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        unsafe {
            let mut pair = Pair::new();
            pair.font(14);
            let text = (0..100)
                .map(|row| format!("行{row:03} {}\r\n", "漢字e\u{301}😀 ".repeat(30)))
                .collect::<String>();
            pair.text(&text);
            let end = text.encode_utf16().count();
            pair.both(EM_SETSEL, WPARAM(end), LPARAM(end as isize));
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            let rails = rail_handles(pair.clipped).unwrap();
            let observations = [true, false].map(|horizontal| {
                Rc::new(Observation {
                    edit: pair.clipped,
                    horizontal,
                    refreshes: Cell::new(0),
                    paints: Cell::new(0),
                    native: Cell::new(None),
                })
            });
            for (rail, observation) in rails.into_iter().zip(&observations) {
                let pointer = Rc::into_raw(Rc::clone(observation));
                if !SetWindowSubclass(rail, Some(observe), 792, pointer as usize).as_bool() {
                    drop(Rc::from_raw(pointer));
                    panic!("Cannot observe owned sibling rail");
                }
            }
            SetWindowTextW(
                pair.owner,
                w!("MDLite Native EDIT Mutation Gate (NOACTIVATE)"),
            )
            .unwrap();
            let foreground = GetForegroundWindow();
            let _ = ShowWindow(pair.owner, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(pair.owner);
            assert!(IsWindowVisible(pair.owner).as_bool());
            assert!(
                rails
                    .into_iter()
                    .all(|rail| IsWindowVisible(rail).as_bool())
            );
            println!(
                "MUTATION_GATE_VISIBLE pid={} owner={} reference={} clipped={} rails={:?} foreground_before={} foreground_after={} activation_calls=0",
                std::process::id(),
                pair.owner.0 as usize,
                pair.reference.0 as usize,
                pair.clipped.0 as usize,
                rails.map(|rail| rail.0 as usize),
                foreground.0 as usize,
                GetForegroundWindow().0 as usize
            );
            let insertion = wide(&"追加 e\u{301}😀\r\n".repeat(20));
            let steps = [
                (WM_CHAR, WPARAM('漢' as usize), LPARAM(1)),
                (WM_CHAR, WPARAM(13), LPARAM(1)),
                (
                    EM_REPLACESEL,
                    WPARAM(1),
                    LPARAM(insertion.as_ptr() as isize),
                ),
                (EM_LINESCROLL, WPARAM(5), LPARAM(-20)),
                (EM_SETSEL, WPARAM(0), LPARAM(0)),
                (EM_SCROLLCARET, WPARAM(0), LPARAM(0)),
                (EM_UNDO, WPARAM(0), LPARAM(0)),
            ];
            for (message, wp, lp) in steps {
                let native_before = semantics(pair.reference);
                let before = observations
                    .each_ref()
                    .map(|value| (value.refreshes.get(), value.paints.get()));
                for rail in rails {
                    let _ = ValidateRect(Some(rail), None);
                }
                pair.both(message, wp, lp);
                let native_after = semantics(pair.reference);
                if matches!(message, WM_CHAR | EM_REPLACESEL) {
                    assert_ne!(
                        native_after.text, native_before.text,
                        "fixture did not actually mutate native text"
                    );
                }
                if message == EM_LINESCROLL {
                    assert_ne!(
                        native_after.first_line, native_before.first_line,
                        "fixture did not actually scroll native text"
                    );
                }
                assert_eq!(
                    semantics(pair.clipped),
                    native_after,
                    "native message {message:#x}"
                );
                for ((rail, observation), (refreshes, paints)) in
                    rails.into_iter().zip(&observations).zip(before)
                {
                    assert!(
                        observation.refreshes.get() > refreshes,
                        "no sibling refresh after native message {message:#x}"
                    );
                    assert_eq!(
                        observation.native.get(),
                        observation.native_geometry(),
                        "stale native geometry after {message:#x}"
                    );
                    let mut dirty = RECT::default();
                    assert!(
                        observation.paints.get() > paints
                            || GetUpdateRect(rail, Some(&mut dirty), false).as_bool(),
                        "no sibling paint/update after {message:#x}"
                    );
                    let mut rect = RECT::default();
                    GetWindowRect(rail, &mut rect).unwrap();
                    assert_eq!(
                        rectangle_value(rect),
                        rectangle_value(
                            native_bar(pair.clipped, observation.horizontal)
                                .unwrap()
                                .rcScrollBar
                        )
                    );
                    println!(
                        "MUTATION_GATE_STATE message={message:#x} rail={} refresh_delta={} paint_delta={} native={:?}",
                        rail.0 as usize,
                        observation.refreshes.get() - refreshes,
                        observation.paints.get() - paints,
                        observation.native.get()
                    );
                }
            }
            assert!(IsWindowVisible(pair.owner).as_bool());
            assert!(observations.iter().all(|value| value.paints.get() > 0));
            let windows = [pair.owner, pair.reference, pair.clipped, rails[0], rails[1]];
            drop(pair);
            assert!(
                windows
                    .into_iter()
                    .all(|hwnd| !IsWindow(Some(hwnd)).as_bool())
            );
            println!(
                "MUTATION_GATE_CLEANUP pid={} owned_windows_destroyed=true; visible native mutation equivalence/geometry/WM_PAINT verified; no pixel, physical IME or Human proof",
                std::process::id()
            );
        }
    }

    #[test]
    #[ignore = "visible NOACTIVATE Settings exposure comparison; requires exclusive GUI lease and evidence directory"]
    fn edit_adapter_settings_exposure_native_comparison_probe() {
        struct Owner(HWND);
        impl Drop for Owner {
            fn drop(&mut self) {
                unsafe {
                    if IsWindow(Some(self.0)).as_bool() {
                        let _ = DestroyWindow(self.0);
                    }
                    println!(
                        "EXPOSURE_PROBE_OWNER_DROP pid={} hwnd={} destroyed={}",
                        std::process::id(),
                        self.0.0 as usize,
                        !IsWindow(Some(self.0)).as_bool()
                    );
                }
            }
        }
        unsafe fn record(
            stage: &str,
            owner: HWND,
            controls: [HWND; 2],
            directory: &Path,
            records: &mut Vec<serde_json::Value>,
        ) {
            unsafe {
                let mut outer = RECT::default();
                let mut client = RECT::default();
                GetWindowRect(owner, &mut outer).unwrap();
                GetClientRect(owner, &mut client).unwrap();
                let values=controls.map(|hwnd| {
                    let mut value=gate_control_state(hwnd,false);
                    value["exstyle"]=serde_json::json!(GetWindowLongPtrW(hwnd,GWL_EXSTYLE));
                    value["font"]=serde_json::json!(SendMessageW(hwnd,WM_GETFONT,None,None).0);
                    let mut raw=SCROLLBARINFO{cbSize:std::mem::size_of::<SCROLLBARINFO>() as u32,..Default::default()};
                    let result=GetScrollBarInfo(hwnd,OBJID_VSCROLL,&mut raw);
                    value["raw_v_api"]=serde_json::json!({"ok":result.is_ok(),"error":result.err().map(|error|error.code().0),
                        "bounds":rectangle_value(raw.rcScrollBar),"thumb":[raw.xyThumbTop,raw.xyThumbBottom],"states":raw.rgstate});
                    let region=Region::rectangle(RECT::default());
                    let result=GetWindowRgn(hwnd,region.handle()).0;
                    let mut bounds=RECT::default();
                    if result!=0 { let _=GetRgnBox(region.handle(),&mut bounds); }
                    value["region"]=serde_json::json!({"result":result,"bounds":rectangle_value(bounds)});
                    value["adapter"]=attached(hwnd).map(|state| {
                        serde_json::json!({"suspended":state.suspended.get(),"clipped":state.clipped.get(),
                            "updating":state.updating.get(),"token_hc":state.tokens.get().high_contrast,
                            "original_region":state.original.is_some(),"rails":state.rails.each_ref().map(|cell| {
                                let rail=cell.get();let mut r=RECT::default();let _=GetWindowRect(rail,&mut r);
                                serde_json::json!({"hwnd":rail.0 as usize,"visible":IsWindowVisible(rail).as_bool(),
                                    "style":GetWindowLongW(rail,GWL_STYLE),"bounds":rectangle_value(r)})
                            })})
                    }).unwrap_or(serde_json::Value::Null);
                    value
                });
                let value = serde_json::json!({"stage":stage,"pid":std::process::id(),"owner":owner.0 as usize,
                    "parent_bounds":rectangle_value(outer),"parent_client":rectangle_value(client),
                    "actual_hc":scrollbar_style::high_contrast(),"foreground":GetForegroundWindow().0 as usize,
                    "reference":values[0],"adapted":values[1]});
                println!("EXPOSURE_PROBE {}", serde_json::to_string(&value).unwrap());
                records.push(value);
                std::fs::write(
                    directory.join("probe.json"),
                    serde_json::to_vec_pretty(records).unwrap(),
                )
                .unwrap();
                // Persist raw evidence before any acceptance assertion fails.
                let reference = semantics(controls[0]);
                let adapted = semantics(controls[1]);
                assert_eq!(
                    adapted.client, reference.client,
                    "{stage}: native client parity"
                );
                assert_eq!(
                    adapted.format, reference.format,
                    "{stage}: native format parity"
                );
                assert_eq!(adapted.h, reference.h, "{stage}: native H range parity");
                assert_eq!(adapted.v, reference.v, "{stage}: native V range parity");
                assert_eq!(
                    adapted.first_line, reference.first_line,
                    "{stage}: native display-row parity"
                );
                assert_eq!(adapted.text, reference.text, "{stage}: source parity");
                assert_eq!(
                    adapted.selection, reference.selection,
                    "{stage}: selection parity"
                );
                assert_eq!(adapted.undo, reference.undo, "{stage}: native Undo parity");
                assert_eq!(
                    SendMessageW(controls[1], EM_GETMODIFY, None, None).0,
                    SendMessageW(controls[0], EM_GETMODIFY, None, None).0,
                    "{stage}: native Modify parity"
                );
                if let Some(reference) = gate_bar(controls[0], false)
                    .filter(|bar| bar.rcScrollBar.bottom > bar.rcScrollBar.top)
                {
                    let bar = gate_bar(controls[1], false)
                        .expect("adapted native SBI must remain available");
                    assert_eq!(
                        bar.rcScrollBar.right - bar.rcScrollBar.left,
                        reference.rcScrollBar.right - reference.rcScrollBar.left,
                        "{stage}: native bar width"
                    );
                    assert_eq!(
                        bar.rcScrollBar.bottom - bar.rcScrollBar.top,
                        reference.rcScrollBar.bottom - reference.rcScrollBar.top,
                        "{stage}: native bar height"
                    );
                    assert_eq!(
                        [bar.xyThumbTop, bar.xyThumbBottom],
                        [reference.xyThumbTop, reference.xyThumbBottom],
                        "{stage}: native thumb geometry"
                    );
                    let state =
                        attached(controls[1]).expect("accepted adapter must remain attached");
                    let native =
                        state.tokens.get().high_contrast || scrollbar_style::high_contrast();
                    assert_eq!(
                        GetWindowLongW(state.rails[1].get(), GWL_STYLE) as u32 & WS_VISIBLE.0 != 0,
                        !native,
                        "{stage}: modern rail policy/visibility"
                    );
                    if !native {
                        assert_excluded(
                            controls[1],
                            &[relative_bar(controls[1], OBJID_VSCROLL).unwrap()],
                        );
                    }
                }
            }
        }
        unsafe {
            let directory = PathBuf::from(
                std::env::var_os("MDLITE_EDIT_EXPOSURE_DIRECTORY")
                    .expect("explicit evidence directory required"),
            );
            assert!(directory.is_absolute() && directory.is_dir());
            assert!(!directory.join("probe.json").exists());
            let owner = Owner(
                CreateWindowExW(
                    WS_EX_DLGMODALFRAME,
                    w!("STATIC"),
                    w!("MDLite Settings EDIT Exposure Probe (NOACTIVATE)"),
                    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_VSCROLL,
                    40,
                    40,
                    1100,
                    630,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap(),
            );
            let initial_font = control_style::UiFont::new(96, 14.).unwrap();
            let mut settings_font = control_style::UiFont::new(96, 16.).unwrap();
            let style = WS_CHILD
                | WS_VISIBLE
                | WS_CLIPSIBLINGS
                | WS_TABSTOP
                | WS_BORDER
                | WS_VSCROLL
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32);
            let controls = [207, 208].map(|id| {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("EDIT"),
                    w!("Custom色 (name=#RRGGBB)"),
                    style,
                    0,
                    0,
                    10,
                    10,
                    Some(owner.0),
                    Some(HMENU(id as usize as *mut _)),
                    None,
                    None,
                )
                .unwrap()
            });
            let mut records = Vec::new();
            record("created_10x10", owner.0, controls, &directory, &mut records);
            for edit in controls {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(initial_font.handle().0 as usize)),
                    Some(LPARAM(1)),
                );
                control_style::install_child_cursor(edit).unwrap();
            }
            record(
                "factory_font_cursor",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            attach(controls[1], shell::Tokens::default()).unwrap();
            record(
                "factory_adapter_attached",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for (index, edit) in controls.into_iter().enumerate() {
                SetWindowPos(
                    edit,
                    None,
                    16 + index as i32 * 536,
                    328,
                    400,
                    90,
                    SWP_NOZORDER,
                )
                .unwrap();
            }
            record(
                "settings_position_y328",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for edit in controls {
                set_explorer_theme(edit).unwrap();
            }
            record(
                "settings_explorer_theme",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for edit in controls {
                SetWindowTextW(edit, w!("")).unwrap();
            }
            record(
                "settings_show_empty_draft",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            settings_font.apply_to(owner.0).unwrap();
            record(
                "settings_font_16",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            let outer_rail =
                scrollbar_style::VerticalRail::new(owner.0, WM_APP + 425, shell::Tokens::default())
                    .unwrap();
            let mut client = RECT::default();
            GetClientRect(owner.0, &mut client).unwrap();
            outer_rail.update(
                scrollbar_style::Range {
                    content: 590,
                    page: client.bottom,
                    position: 0,
                },
                96,
                shell::Tokens::default(),
                false,
            );
            shell::refresh_native_scrollbars(owner.0, shell::Tokens::default()).unwrap();
            record(
                "settings_outer_rail_refresh_before_show",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            let foreground = GetForegroundWindow();
            let _ = ShowWindow(owner.0, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(owner.0);
            record(
                "visible_parent_630",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            assert!(IsWindowVisible(owner.0).as_bool());
            for edit in controls {
                assert_eq!(
                    GetWindowLongW(edit, GWL_STYLE) as u32,
                    0x54211044,
                    "Settings final style mismatch; do not invent border removal"
                );
            }
            SetWindowPos(
                owner.0,
                None,
                0,
                0,
                1100,
                330,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            record(
                "parent_resized_330_before_refresh",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            shell::refresh_native_scrollbars(owner.0, shell::Tokens::default()).unwrap();
            record(
                "parent_resized_330_after_refresh",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            GetClientRect(owner.0, &mut client).unwrap();
            for (label, y) in [
                ("partial_exposure", client.bottom - 45),
                ("actual_minus_299_exposure", 29),
            ] {
                for (index, edit) in controls.into_iter().enumerate() {
                    SetWindowPos(
                        edit,
                        None,
                        16 + index as i32 * 536,
                        y,
                        0,
                        0,
                        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
                record(
                    &format!("{label}_before_refresh"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
                shell::refresh_native_scrollbars(owner.0, shell::Tokens::default()).unwrap();
                record(
                    &format!("{label}_after_refresh"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
                let _ = RedrawWindow(
                    Some(owner.0),
                    None,
                    None,
                    RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME | RDW_UPDATENOW,
                );
                record(
                    &format!("{label}_after_paint"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
            }
            let text = (0..25)
                .map(|row| format!("行{row:02}e\u{301}😀ABCDEFG\r\n"))
                .collect::<String>();
            assert_eq!(text.encode_utf16().count(), 400);
            for unit in text.replace("\r\n", "\r").encode_utf16() {
                for edit in controls {
                    SendMessageW(edit, WM_CHAR, Some(WPARAM(unit as usize)), Some(LPARAM(1)));
                }
            }
            record(
                "native_unicode_filled",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for edit in controls {
                assert_eq!(shell::text_of(edit), text);
            }
            for edit in controls {
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                SendMessageW(edit, EM_SCROLLCARET, None, None);
            }
            record(
                "native_home_before_refresh",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            shell::refresh_native_scrollbars(owner.0, shell::Tokens::default()).unwrap();
            record(
                "native_home_after_refresh",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            let _ = RedrawWindow(
                Some(owner.0),
                None,
                None,
                RDW_INVALIDATE | RDW_ALLCHILDREN | RDW_FRAME | RDW_UPDATENOW,
            );
            record(
                "native_home_after_paint",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for edit in controls {
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(400)), Some(LPARAM(400)));
                SendMessageW(edit, EM_SCROLLCARET, None, None);
            }
            record("native_end", owner.0, controls, &directory, &mut records);
            for edit in controls {
                SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                SendMessageW(edit, EM_SCROLLCARET, None, None);
            }
            record(
                "native_home_again",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            let baseline = controls.map(semantics);
            let modified = controls.map(|edit| SendMessageW(edit, EM_GETMODIFY, None, None).0);
            for iteration in 0..2 {
                for edit in controls {
                    set_explorer_theme(edit).unwrap();
                }
                record(
                    &format!("repeat_explorer_theme_{iteration}"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
            }
            for size in [18., 16.] {
                settings_font.replace_on(owner.0, 96, size).unwrap();
                record(
                    &format!("explorer_font_{size}"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
            }
            let hc = shell::Tokens {
                high_contrast: true,
                ..Default::default()
            };
            shell::refresh_native_scrollbars(owner.0, hc).unwrap();
            record(
                "explorer_hc_native",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            shell::refresh_native_scrollbars(owner.0, shell::Tokens::default()).unwrap();
            record(
                "explorer_hc_modern_restored",
                owner.0,
                controls,
                &directory,
                &mut records,
            );
            for width in [460, 400] {
                for edit in controls {
                    SetWindowPos(
                        edit,
                        None,
                        0,
                        0,
                        width,
                        90,
                        SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                    )
                    .unwrap();
                }
                record(
                    &format!("explorer_actual_edit_width_{width}"),
                    owner.0,
                    controls,
                    &directory,
                    &mut records,
                );
            }
            for (index, edit) in controls.into_iter().enumerate() {
                let current = semantics(edit);
                assert_eq!(current.text, text);
                assert_eq!(current.selection, baseline[index].selection);
                assert_eq!(current.undo, baseline[index].undo);
                assert_eq!(
                    SendMessageW(edit, EM_GETMODIFY, None, None).0,
                    modified[index]
                );
            }
            let windows = [owner.0, controls[0], controls[1], outer_rail.hwnd()];
            drop(owner);
            assert!(
                windows
                    .into_iter()
                    .all(|hwnd| !IsWindow(Some(hwnd)).as_bool())
            );
            println!(
                "EXPOSURE_PROBE_CLEANUP pid={} owned_windows_destroyed=true foreground_before={} foreground_after={} activation_calls=0",
                std::process::id(),
                foreground.0 as usize,
                GetForegroundWindow().0 as usize
            );
        }
    }

    #[test]
    fn edit_adapter_true_wrap_uses_native_display_rows_without_horizontal_style() {
        unsafe {
            let mut pair = Pair::with_wrap(true);
            pair.font(14);
            let source = (0..100)
                .map(|i| format!("行{i:03} {}\r\n", "漢字e\u{301}😀 ".repeat(80)))
                .collect::<String>();
            pair.text(&source);
            pair.both(EM_SETSEL, WPARAM(3), LPARAM(8));
            let before = semantics(pair.clipped);
            assert_eq!(
                before.style as u32 & (WS_HSCROLL.0 | ES_AUTOHSCROLL as u32),
                0
            );
            let lines = SendMessageW(pair.clipped, EM_GETLINECOUNT, None, None).0;
            assert!(
                lines > 100,
                "true wrap needs more native display rows than source rows"
            );
            attach(pair.clipped, shell::Tokens::default()).unwrap();
            pair.both(EM_LINESCROLL, WPARAM(0), LPARAM(50));
            assert_eq!(semantics(pair.clipped), semantics(pair.reference));
            assert_eq!(
                SendMessageW(pair.clipped, EM_GETFIRSTVISIBLELINE, None, None).0,
                50
            );
            assert_eq!(semantics(pair.clipped).text, before.text);
            assert_eq!(semantics(pair.clipped).selection, before.selection);
            assert_eq!(
                semantics(pair.clipped).style as u32 & (WS_HSCROLL.0 | ES_AUTOHSCROLL as u32),
                0
            );
        }
    }
}

#[cfg(test)]
#[test]
#[ignore = "interactive native NC tracking gate; requires explicit run directory and foreground GUI lease"]
fn edit_region_interactive_native_nc_delegation_gate() {
    tests::edit_region_interactive_native_nc_delegation_gate();
}
