//! Fixed-height native LISTBOX vertical adapter. The LISTBOX still owns content,
//! selection, keys and its horizontal axis; only wheel routing and the V rail differ.
use super::*;
use std::cell::OnceCell;
use windows::Win32::UI::Controls::*;

const PROPERTY: PCWSTR = w!("MDLite.ListVertical.v1");
const SUBCLASS: usize = 0x4d44_4c56;
const REQUEST: u32 = WM_APP + 63;
const H_REQUEST: u32 = WM_APP + 64;

unsafe fn same_thread(hwnd: HWND) -> bool {
    unsafe {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut process))
            == windows_sys::Win32::System::Threading::GetCurrentThreadId()
            && process == std::process::id()
    }
}

struct State {
    hwnd: Cell<HWND>,
    rail: scrollbar_style::VerticalRail,
    tokens: Cell<shell::Tokens>,
    native: Cell<bool>,
    reserve: Cell<i32>,
    bottom: Cell<i32>,
    wheel: Cell<i32>,
    updating: Cell<bool>,
    shown: Cell<bool>,
    horizontal: OnceCell<Horizontal>,
}
struct Horizontal {
    rail: scrollbar_style::VerticalRail,
    extent: Cell<i32>,
    offset: Cell<i32>,
    wheel: Cell<i32>,
}

/// Call only for a LISTBOX created without WS_VSCROLL. Keep its original H style.
/// Variable-height/multicolumn/dropdown lists retain their original native policy.
pub(super) unsafe fn attach_listbox(hwnd: HWND, tokens: shell::Tokens) -> Result<(), String> {
    unsafe {
        if !same_thread(hwnd) {
            return Err("LISTBOX rail requires its owning UI thread".into());
        }
        let existing = GetPropW(hwnd, PROPERTY);
        if !existing.is_invalid() {
            return refresh_listbox(hwnd, tokens);
        }
        let mut name = [0u16; 32];
        let length = GetClassNameW(hwnd, &mut name);
        let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
        if !String::from_utf16_lossy(&name[..length as usize]).eq_ignore_ascii_case("LISTBOX")
            || style & (WS_VSCROLL.0 | LBS_MULTICOLUMN as u32 | LBS_OWNERDRAWVARIABLE as u32) != 0
            || GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_LEFTSCROLLBAR.0 != 0
        {
            return Err(
                "Vertical rail requires a fixed-height LTR LISTBOX created without WS_VSCROLL"
                    .into(),
            );
        }
        let rail = scrollbar_style::VerticalRail::new_sibling(hwnd, REQUEST, tokens)
            .ok_or("Cannot create LISTBOX vertical rail")?;
        let state = Rc::new(State {
            hwnd: Cell::new(hwnd),
            rail,
            tokens: Cell::new(tokens),
            native: Cell::new(false),
            reserve: Cell::new(0),
            bottom: Cell::new(0),
            wheel: Cell::new(0),
            updating: Cell::new(false),
            shown: Cell::new(style & WS_VISIBLE.0 != 0),
            horizontal: OnceCell::new(),
        });
        let pointer = Rc::into_raw(state.clone());
        if let Err(error) = SetPropW(hwnd, PROPERTY, Some(HANDLE(pointer.cast_mut().cast()))) {
            state.rail.destroy();
            drop(Rc::from_raw(pointer));
            return Err(error.to_string());
        }
        if !SetWindowSubclass(hwnd, Some(list_proc), SUBCLASS, pointer as usize).as_bool() {
            let _ = RemovePropW(hwnd, PROPERTY);
            state.rail.destroy();
            drop(Rc::from_raw(pointer));
            return Err("Cannot attach LISTBOX vertical subclass".into());
        }
        // Sibling rail occupies the reserved NC strip, never the text client area.
        SetWindowLongW(hwnd, GWL_STYLE, (style | WS_CLIPSIBLINGS.0) as i32);
        let _ = EnableWindow(state.rail.hwnd(), IsWindowEnabled(hwnd).as_bool());
        state.sync(
            GetDpiForWindow(hwnd).max(1),
            scrollbar_style::high_contrast(),
        );
        Ok(())
    }
}

pub(super) unsafe fn refresh_listbox(hwnd: HWND, tokens: shell::Tokens) -> Result<(), String> {
    unsafe {
        if !same_thread(hwnd) {
            return Err("LISTBOX rail requires its owning UI thread".into());
        }
        let pointer = GetPropW(hwnd, PROPERTY).0 as *const State;
        if pointer.is_null() {
            return Err("LISTBOX vertical rail is not attached".into());
        }
        Rc::increment_strong_count(pointer);
        let state = Rc::from_raw(pointer);
        state.tokens.set(tokens);
        state.sync(
            GetDpiForWindow(hwnd).max(1),
            scrollbar_style::high_contrast(),
        );
        Ok(())
    }
}

unsafe fn attached(hwnd: HWND) -> Option<Rc<State>> {
    unsafe {
        if !same_thread(hwnd) {
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

/// Explorer's owner-drawn content owns this pixel offset. Ordinary list H stays native.
pub(super) unsafe fn set_horizontal_extent(hwnd: HWND, pixels: i32) -> Result<(), String> {
    unsafe {
        let state = attached(hwnd).ok_or("Explorer horizontal model needs an attached LISTBOX")?;
        if state.horizontal.get().is_none() {
            let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
            if style & WS_HSCROLL.0 != 0
                || style & LBS_OWNERDRAWFIXED as u32 == 0
                || style & LBS_HASSTRINGS as u32 == 0
            {
                return Err("Explorer horizontal model requires fixed owner-drawn strings created without H".into());
            }
            let rail = scrollbar_style::VerticalRail::new_horizontal_sibling(
                hwnd,
                H_REQUEST,
                state.tokens.get(),
            )
            .ok_or("Cannot create Explorer horizontal rail")?;
            let _ = EnableWindow(rail.hwnd(), IsWindowEnabled(hwnd).as_bool());
            let horizontal = Horizontal {
                rail,
                extent: Cell::new(0),
                offset: Cell::new(0),
                wheel: Cell::new(0),
            };
            if let Err(unused) = state.horizontal.set(horizontal) {
                unused.rail.destroy();
            }
        }
        state.horizontal.get().unwrap().extent.set(pixels.max(0));
        state.sync(GetDpiForWindow(hwnd).max(1), state.native.get());
        let _ = InvalidateRect(Some(hwnd), None, false);
        Ok(())
    }
}

pub(super) fn horizontal_offset(hwnd: HWND) -> i32 {
    unsafe {
        attached(hwnd)
            .and_then(|state| state.horizontal.get().map(|h| h.offset.get()))
            .unwrap_or(0)
    }
}

pub(super) unsafe fn set_horizontal_offset(hwnd: HWND, offset: i32) -> bool {
    unsafe { attached(hwnd).is_some_and(|state| state.set_offset(offset)) }
}

fn horizontal_unit(hwnd: HWND) -> i32 {
    unsafe {
        let font = SendMessageW(hwnd, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0)));
        let font = if font.0 == 0 {
            GetStockObject(SYSTEM_FONT)
        } else {
            HGDIOBJ(font.0 as *mut _)
        };
        let dc = GetDC(Some(hwnd));
        if dc == HDC::default() {
            return 1;
        }
        let old = SelectObject(dc, font);
        let mut size = SIZE::default();
        // The native dialog base X unit uses the alphabet average, not
        // TEXTMETRIC.tmAveCharWidth or the process-global system-font unit.
        let measured = GetTextExtentPoint32W(
            dc,
            w!("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz").as_wide(),
            &mut size,
        )
        .as_bool();
        SelectObject(dc, old);
        ReleaseDC(Some(hwnd), dc);
        if measured {
            ((size.cx / 26 + 1) / 2).max(1)
        } else {
            1
        }
    }
}

fn reservations(
    width: i32,
    height: i32,
    count: i32,
    item_height: i32,
    extent: Option<i32>,
    thickness: i32,
) -> (i32, i32) {
    let (mut v, mut h) = (0, 0);
    // Each rail can only make the other necessary: two monotone steps cover both axes.
    for _ in 0..2 {
        v = if count > ((height - h).max(0) / item_height.max(1)).max(1) {
            thickness
        } else {
            0
        };
        h = if extent.is_some_and(|value| value > (width - v).max(0)) {
            thickness
        } else {
            0
        };
    }
    (v, h)
}

fn wheel_target(
    remainder: &Cell<i32>,
    delta: i32,
    flags: usize,
    lines: u32,
    range: scrollbar_style::Range,
) -> i32 {
    if flags & 0x0c != 0 {
        return range.position;
    } // Native LISTBOX ignores Shift/Ctrl wheel.
    let amount = if lines == u32::MAX {
        range.page.max(1) as u32
    } else {
        lines.min(range.page.max(1) as u32)
    };
    // Native LISTBOX preserves fractions in line units, e.g. three lines/notch:
    // +60,+60 scrolls one line then two, not zero then three.
    let accumulated = remainder.get() as i64 + delta as i64 * amount as i64;
    let units = accumulated / 120;
    remainder.set((accumulated % 120) as i32);
    (range.position as i64 - units).clamp(0, (range.content - range.page).max(0) as i64) as i32
}

impl State {
    unsafe fn valid(&self, hwnd: HWND) -> bool {
        unsafe {
            hwnd != HWND::default()
                && self.hwnd.get() == hwnd
                && GetPropW(hwnd, PROPERTY).0 == (self as *const State).cast_mut().cast()
        }
    }
    unsafe fn range(&self, hwnd: HWND) -> scrollbar_style::Range {
        unsafe {
            let count = SendMessageW(hwnd, LB_GETCOUNT, Some(WPARAM(0)), Some(LPARAM(0)))
                .0
                .max(0) as i32;
            let position = SendMessageW(hwnd, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0)))
                .0
                .max(0) as i32;
            let height = SendMessageW(hwnd, LB_GETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(0)))
                .0
                .max(1) as i32;
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            scrollbar_style::Range {
                content: count,
                page: (client.bottom / height).max(1),
                position,
            }
        }
    }
    unsafe fn h_range(&self, hwnd: HWND) -> scrollbar_style::Range {
        unsafe {
            let h = self.horizontal.get().unwrap();
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            scrollbar_style::Range {
                content: h.extent.get(),
                page: client.right.max(1),
                position: h.offset.get(),
            }
        }
    }
    unsafe fn set_offset(&self, offset: i32) -> bool {
        unsafe {
            let hwnd = self.hwnd.get();
            let Some(h) = self.horizontal.get() else {
                return false;
            };
            if !self.valid(hwnd) {
                return false;
            }
            let range = self.h_range(hwnd);
            let next = offset.clamp(0, (range.content - range.page).max(0));
            if h.offset.replace(next) == next {
                return false;
            }
            self.sync(GetDpiForWindow(hwnd).max(1), self.native.get());
            let _ = InvalidateRect(Some(hwnd), None, false);
            true
        }
    }
    unsafe fn cancel(&self) {
        unsafe {
            self.rail.cancel();
            if let Some(h) = self.horizontal.get() {
                h.rail.cancel();
            }
        }
    }
    unsafe fn horizontal_scroll(&self, code: usize) -> bool {
        unsafe {
            let hwnd = self.hwnd.get();
            let range = self.h_range(hwnd);
            let unit = horizontal_unit(hwnd);
            let page = (range.page / 3 * 2).max(1);
            let next = match code {
                0 => range.position.saturating_sub(unit),
                1 => range.position.saturating_add(unit),
                2 => range.position.saturating_sub(page),
                3 => range.position.saturating_add(page),
                4 | 5 if self.native.get() => {
                    let mut info = SCROLLINFO {
                        cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                        fMask: SIF_TRACKPOS,
                        ..Default::default()
                    };
                    if GetScrollInfo(hwnd, SB_HORZ, &mut info).is_err() {
                        return false;
                    }
                    info.nTrackPos
                }
                6 => 0,
                7 => i32::MAX,
                _ => return false,
            };
            self.set_offset(next)
        }
    }
    unsafe fn horizontal_wheel(&self, wp: WPARAM) {
        unsafe {
            if wp.0 & 0x0c != 0 {
                return;
            }
            let h = self.horizontal.get().unwrap();
            let hwnd = self.hwnd.get();
            let range = self.h_range(hwnd);
            let unit = horizontal_unit(hwnd);
            let mut chars = 0u32;
            if SystemParametersInfoW(
                SPI_GETWHEELSCROLLCHARS,
                0,
                Some((&mut chars as *mut u32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_err()
            {
                return;
            }
            let chars = chars.min((range.page / unit).max(1) as u32);
            let delta = (wp.0 >> 16) as u16 as i16 as i32;
            let accumulated = h.wheel.get() as i64 + delta as i64 * chars as i64;
            h.wheel.set((accumulated % 120) as i32);
            let next = (range.position as i64 + accumulated / 120 * unit as i64)
                .clamp(0, i32::MAX as i64) as i32;
            self.set_offset(next);
        }
    }
    unsafe fn sync(&self, dpi: u32, hc: bool) {
        unsafe {
            let hwnd = self.hwnd.get();
            if !self.valid(hwnd) || self.updating.replace(true) {
                return;
            }
            let changed = self.native.replace(hc) != hc;
            let h = self.horizontal.get();
            if changed {
                self.cancel();
            }
            if changed && !hc {
                let _ = ShowScrollBar(hwnd, if h.is_some() { SB_BOTH } else { SB_VERT }, false);
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let old_v = self.reserve.get();
            let old_h = self.bottom.get();
            let range = self.range(hwnd);
            let item_height = SendMessageW(hwnd, LB_GETITEMHEIGHT, Some(WPARAM(0)), Some(LPARAM(0)))
                .0
                .max(1) as i32;
            let (v, bottom) = if hc {
                (0, 0)
            } else {
                reservations(
                    client.right + old_v,
                    client.bottom + old_h,
                    range.content,
                    item_height,
                    h.map(|value| value.extent.get()),
                    (10. * dpi as f32 / 96.).round().max(1.) as i32,
                )
            };
            self.reserve.set(v);
            self.bottom.set(bottom);
            if changed || v != old_v || bottom != old_h {
                self.cancel();
                if self.valid(hwnd) {
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
            }
            if hc {
                // Native range/page changes may make the other scrollbar necessary.
                // Two bounded metadata passes establish both actual viewports.
                for _ in 0..2 {
                    if !self.valid(hwnd) {
                        break;
                    }
                    if h.is_some() {
                        SetScrollInfo(
                            hwnd,
                            SB_HORZ,
                            &scrollbar_style::scroll_info(self.h_range(hwnd)),
                            true,
                        );
                    }
                    SetScrollInfo(
                        hwnd,
                        SB_VERT,
                        &scrollbar_style::scroll_info(self.range(hwnd)),
                        true,
                    );
                }
            }
            if self.valid(hwnd) {
                let _ = GetClientRect(hwnd, &mut client);
                let parent = GetParent(hwnd).unwrap_or_default();
                let mut origin = POINT::default();
                let _ = ClientToScreen(hwnd, &mut origin);
                let _ = ScreenToClient(parent, &mut origin);
                let range = self.range(hwnd);
                let hidden = !self.shown.get();
                let vrect = RECT {
                    left: origin.x + client.right,
                    top: origin.y,
                    right: origin.x + client.right + v,
                    bottom: origin.y + client.bottom,
                };
                let mut placed =
                    self.rail
                        .update_sibling(range, dpi, self.tokens.get(), hc || hidden, vrect);
                if let Some(h) = h {
                    let mut range = self.h_range(hwnd);
                    let next = h.offset.get().clamp(0, (range.content - range.page).max(0));
                    if h.offset.replace(next) != next {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    range.position = next;
                    if hc {
                        SetScrollInfo(hwnd, SB_HORZ, &scrollbar_style::scroll_info(range), true);
                    }
                    let rect = RECT {
                        left: origin.x,
                        top: origin.y + client.bottom,
                        right: origin.x + client.right + v,
                        bottom: origin.y + client.bottom + bottom,
                    };
                    placed &= h.rail.update_horizontal_sibling(
                        range,
                        dpi,
                        self.tokens.get(),
                        hc || hidden,
                        rect,
                        v,
                    );
                }
                if !placed && self.valid(hwnd) {
                    self.native.set(true);
                    self.reserve.set(0);
                    self.bottom.set(0);
                    self.cancel();
                    self.rail
                        .update_sibling(range, dpi, self.tokens.get(), true, vrect);
                    if let Some(h) = h {
                        h.rail.update_horizontal_sibling(
                            self.h_range(hwnd),
                            dpi,
                            self.tokens.get(),
                            true,
                            RECT::default(),
                            0,
                        );
                        SetScrollInfo(
                            hwnd,
                            SB_HORZ,
                            &scrollbar_style::scroll_info(self.h_range(hwnd)),
                            true,
                        );
                    }
                    SetScrollInfo(hwnd, SB_VERT, &scrollbar_style::scroll_info(range), true);
                    let _ = SetWindowPos(
                        hwnd,
                        None,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                    );
                }
            }
            self.updating.set(false);
        }
    }
    unsafe fn wheel(&self, wp: WPARAM) {
        unsafe {
            let hwnd = self.hwnd.get();
            let mut lines = 0u32;
            if SystemParametersInfoW(
                SPI_GETWHEELSCROLLLINES,
                0,
                Some((&mut lines as *mut u32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .is_err()
            {
                return;
            }
            let range = self.range(hwnd);
            let delta = (wp.0 >> 16) as u16 as i16 as i32;
            let target = wheel_target(&self.wheel, delta, wp.0 & 0xffff, lines, range);
            if target != range.position && self.valid(hwnd) {
                SendMessageW(
                    hwnd,
                    LB_SETTOPINDEX,
                    Some(WPARAM(target as usize)),
                    Some(LPARAM(0)),
                );
            }
        }
    }
}

unsafe extern "system" fn list_proc(
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
            state.hwnd.set(HWND::default());
            let _ = RemovePropW(hwnd, PROPERTY);
            let _ = RemoveWindowSubclass(hwnd, Some(list_proc), id);
            state.rail.destroy();
            if let Some(h) = state.horizontal.get() {
                h.rail.destroy();
            }
            drop(Rc::from_raw(pointer));
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if msg == WM_NCCALCSIZE {
            let result = DefSubclassProc(hwnd, msg, wp, lp);
            if state.valid(hwnd) {
                let rect = if wp.0 != 0 {
                    &mut (*(lp.0 as *mut NCCALCSIZE_PARAMS)).rgrc[0]
                } else {
                    &mut *(lp.0 as *mut RECT)
                };
                rect.right = (rect.right - state.reserve.get()).max(rect.left);
                rect.bottom = (rect.bottom - state.bottom.get()).max(rect.top);
            }
            return result;
        }
        // WM_SHOWWINDOW arrives before WS_VISIBLE necessarily reflects the new
        // state. SWP show/hide can also arrive without that notification.
        if msg == WM_SHOWWINDOW {
            state.shown.set(wp.0 != 0);
        }
        if msg == WM_WINDOWPOSCHANGED {
            let flags = (*(lp.0 as *const WINDOWPOS)).flags;
            if flags.0 & SWP_SHOWWINDOW.0 != 0 {
                state.shown.set(true);
            }
            if flags.0 & SWP_HIDEWINDOW.0 != 0 {
                state.shown.set(false);
            }
        }
        if msg == WM_CANCELMODE || msg == WM_ENABLE || (msg == WM_SHOWWINDOW && wp.0 == 0) {
            if msg != WM_ENABLE || wp.0 == 0 {
                state.cancel();
            }
            if msg == WM_ENABLE {
                let _ = EnableWindow(state.rail.hwnd(), wp.0 != 0);
                if let Some(h) = state.horizontal.get() {
                    let _ = EnableWindow(h.rail.hwnd(), wp.0 != 0);
                }
            }
            if msg == WM_SHOWWINDOW {
                let _ = ShowWindow(state.rail.hwnd(), SW_HIDE);
                if let Some(h) = state.horizontal.get() {
                    let _ = ShowWindow(h.rail.hwnd(), SW_HIDE);
                }
            }
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        if msg == REQUEST {
            if let Some(position) = state.rail.take_request(wp.0, HWND(lp.0 as *mut _)) {
                SendMessageW(
                    hwnd,
                    LB_SETTOPINDEX,
                    Some(WPARAM(position as usize)),
                    Some(LPARAM(0)),
                );
            }
            return LRESULT(0);
        }
        if msg == H_REQUEST {
            if let Some(h) = state.horizontal.get()
                && let Some(offset) = h.rail.take_request(wp.0, HWND(lp.0 as *mut _))
            {
                set_horizontal_offset(hwnd, offset);
            }
            return LRESULT(0);
        }
        if state.horizontal.get().is_some() && msg == WM_HSCROLL {
            state.horizontal_scroll(wp.0 & 0xffff);
            return LRESULT(0);
        }
        if state.horizontal.get().is_some() && msg == WM_MOUSEHWHEEL {
            state.horizontal_wheel(wp);
            return LRESULT(0);
        }
        if msg == WM_MOUSEWHEEL {
            state.wheel(wp);
            return LRESULT(0);
        }
        if msg == WM_VSCROLL && state.native.get() && matches!(wp.0 & 0xffff, 4 | 5) {
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_TRACKPOS,
                ..Default::default()
            };
            if GetScrollInfo(hwnd, SB_VERT, &mut info).is_ok() {
                SendMessageW(
                    hwnd,
                    LB_SETTOPINDEX,
                    Some(WPARAM(info.nTrackPos.max(0) as usize)),
                    Some(LPARAM(0)),
                );
            }
            return LRESULT(0);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if state.valid(hwnd)
            && matches!(
                msg,
                LB_ADDSTRING
                    | LB_INSERTSTRING
                    | LB_DELETESTRING
                    | LB_RESETCONTENT
                    | LB_SETCOUNT
                    | LB_SETTOPINDEX
                    | LB_SETCURSEL
                    | LB_SETSEL
                    | LB_SELECTSTRING
                    | LB_SETCARETINDEX
                    | WM_KEYDOWN
                    | WM_CHAR
                    | WM_SIZE
                    | WM_SHOWWINDOW
                    | WM_WINDOWPOSCHANGED
                    | WM_SETFONT
                    | WM_VSCROLL
                    | WM_HSCROLL
                    | WM_MOUSEHWHEEL
                    | WM_LBUTTONDOWN
                    | WM_LBUTTONUP
                    | WM_MOUSEMOVE
                    | WM_TIMER
                    | WM_SETTINGCHANGE
                    | WM_THEMECHANGED
                    | WM_DPICHANGED_AFTERPARENT
            )
        {
            let hc = if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED) {
                scrollbar_style::high_contrast()
            } else {
                state.native.get()
            };
            state.sync(GetDpiForWindow(hwnd).max(1), hc);
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Surface(HDC, HBITMAP, HGDIOBJ);
    impl Surface {
        unsafe fn new(hwnd: HWND, width: i32, height: i32) -> Self {
            unsafe {
                let source = GetDC(Some(hwnd));
                let dc = CreateCompatibleDC(Some(source));
                let bitmap = CreateCompatibleBitmap(source, width, height);
                ReleaseDC(Some(hwnd), source);
                assert!(dc != HDC::default() && bitmap != HBITMAP::default());
                Self(dc, bitmap, SelectObject(dc, bitmap.into()))
            }
        }
    }
    impl Drop for Surface {
        fn drop(&mut self) {
            unsafe {
                SelectObject(self.0, self.2);
                let _ = DeleteObject(self.1.into());
                let _ = DeleteDC(self.0);
            }
        }
    }

    unsafe fn native_row_pixels(hwnd: HWND) -> Vec<u32> {
        unsafe {
            let mut window = RECT::default();
            GetWindowRect(hwnd, &mut window).unwrap();
            let mut origin = POINT::default();
            ClientToScreen(hwnd, &mut origin).unwrap();
            let surface =
                Surface::new(hwnd, window.right - window.left, window.bottom - window.top);
            SendMessageW(
                hwnd,
                WM_PRINT,
                Some(WPARAM(surface.0.0 as usize)),
                Some(LPARAM(
                    (PRF_CLIENT | PRF_ERASEBKGND | PRF_NONCLIENT) as isize,
                )),
            );
            let mut pixels = Vec::new();
            for y in 3..15 {
                for x in 10..170 {
                    pixels.push(
                        GetPixel(
                            surface.0,
                            x + origin.x - window.left,
                            y + origin.y - window.top,
                        )
                        .0,
                    );
                }
            }
            pixels
        }
    }
    struct Fixture {
        host: HWND,
        native: HWND,
        adapted: HWND,
    }
    impl Fixture {
        fn new(count: usize) -> Self {
            Self::with_style(
                count,
                LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_NODATA | LBS_OWNERDRAWFIXED,
            )
        }
        fn with_style(count: usize, flags: i32) -> Self {
            Self::with_horizontal(count, flags, true)
        }
        fn explorer(count: usize) -> Self {
            Self::with_horizontal(
                count,
                LBS_NOTIFY
                    | LBS_NOINTEGRALHEIGHT
                    | LBS_HASSTRINGS
                    | LBS_OWNERDRAWFIXED
                    | LBS_EXTENDEDSEL,
                false,
            )
        }
        fn with_horizontal(count: usize, flags: i32, native_h: bool) -> Self {
            unsafe {
                let host = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("hidden list adapter"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    700,
                    400,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let make = |x, vertical| {
                    CreateWindowExW(
                        WINDOW_EX_STYLE(0),
                        w!("LISTBOX"),
                        w!("synthetic rows"),
                        WS_CHILD
                            | WS_VISIBLE
                            | WS_BORDER
                            | if vertical || native_h {
                                WS_HSCROLL
                            } else {
                                WINDOW_STYLE(0)
                            }
                            | WINDOW_STYLE(flags as u32)
                            | if vertical {
                                WS_VSCROLL
                            } else {
                                WINDOW_STYLE(0)
                            },
                        x,
                        10,
                        300,
                        220,
                        Some(host),
                        None,
                        None,
                        None,
                    )
                    .unwrap()
                };
                let fixture = Self {
                    host,
                    native: make(10, true),
                    adapted: make(350, false),
                };
                for h in [fixture.native, fixture.adapted] {
                    if flags & LBS_NODATA != 0 {
                        assert_ne!(
                            SendMessageW(h, LB_SETCOUNT, Some(WPARAM(count)), Some(LPARAM(0))).0,
                            LB_ERR as isize
                        );
                    } else {
                        for index in 0..count {
                            let text = wide(&format!(
                                "Synthetic row {index:04} {}",
                                "abcdefghij ".repeat(12)
                            ));
                            assert_ne!(
                                SendMessageW(
                                    h,
                                    LB_ADDSTRING,
                                    Some(WPARAM(0)),
                                    Some(LPARAM(text.as_ptr() as isize))
                                )
                                .0,
                                LB_ERR as isize
                            );
                        }
                    }
                    SendMessageW(
                        h,
                        LB_SETHORIZONTALEXTENT,
                        Some(WPARAM(1800)),
                        Some(LPARAM(0)),
                    );
                    SendMessageW(h, LB_SETCURSEL, Some(WPARAM(0)), Some(LPARAM(0)));
                }
                let mut name = [0u16; 32];
                let n = GetClassNameW(fixture.adapted, &mut name);
                eprintln!(
                    "LISTBOX_ATTACH class={} style={:08x}",
                    String::from_utf16_lossy(&name[..n as usize]),
                    GetWindowLongW(fixture.adapted, GWL_STYLE) as u32
                );
                attach_listbox(fixture.adapted, shell::Tokens::default()).unwrap();
                fixture
            }
        }
        unsafe fn state(&self) -> Rc<State> {
            unsafe {
                let pointer = GetPropW(self.adapted, PROPERTY).0 as *const State;
                Rc::increment_strong_count(pointer);
                Rc::from_raw(pointer)
            }
        }
        unsafe fn send_both(&self, msg: u32, wp: WPARAM, lp: LPARAM) {
            unsafe {
                for h in [self.native, self.adapted] {
                    SendMessageW(h, msg, Some(wp), Some(lp));
                }
            }
        }
        unsafe fn equivalent(&self, label: &str) {
            unsafe {
                for message in [LB_GETTOPINDEX, LB_GETCURSEL, LB_GETCARETINDEX] {
                    let a = SendMessageW(self.native, message, Some(WPARAM(0)), Some(LPARAM(0))).0;
                    let b = SendMessageW(self.adapted, message, Some(WPARAM(0)), Some(LPARAM(0))).0;
                    assert_eq!(a, b, "{label} message={message:x}");
                }
                let mut a = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                let mut b = a;
                GetScrollInfo(self.native, SB_HORZ, &mut a).unwrap();
                GetScrollInfo(self.adapted, SB_HORZ, &mut b).unwrap();
                assert_eq!(
                    (a.nMin, a.nMax, a.nPos),
                    (b.nMin, b.nMax, b.nPos),
                    "native H content state at {label}"
                );
                assert_eq!(
                    GetWindowLongW(self.adapted, GWL_STYLE) as u32 & WS_VSCROLL.0,
                    0,
                    "no native V reappearance at {label}"
                );
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            unsafe {
                let rail = GetPropW(self.adapted, PROPERTY).0 as *const State;
                let sibling = if rail.is_null() {
                    HWND::default()
                } else {
                    (*rail).rail.hwnd()
                };
                let _ = DestroyWindow(self.host);
                assert!(!IsWindow(Some(self.native)).as_bool());
                assert!(!IsWindow(Some(self.adapted)).as_bool());
                if sibling != HWND::default() {
                    assert!(!IsWindow(Some(sibling)).as_bool());
                    assert_ne!(GetCapture(), sibling);
                }
            }
        }
    }

    // Stock LISTBOX partial H-wheel deltas are shared across these native probes.
    // Keep their balanced sequences exclusive; custom offsets remain per control.
    static NATIVE_HORIZONTAL_WHEEL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn explorer_horizontal_native_increments_and_real_content() {
        let _wheel = NATIVE_HORIZONTAL_WHEEL.lock().unwrap();
        let f = Fixture::with_style(50, LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_HASSTRINGS);
        unsafe {
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(
                f.native,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            let source = GetDC(Some(f.native));
            let old = SelectObject(source, font);
            let mut metrics = TEXTMETRICW::default();
            GetTextMetricsW(source, &mut metrics).unwrap();
            SelectObject(source, old);
            ReleaseDC(Some(f.native), source);
            let mut chars = 0u32;
            SystemParametersInfoW(
                SPI_GETWHEELSCROLLCHARS,
                0,
                Some((&mut chars as *mut u32).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
            .unwrap();
            eprintln!(
                "H_NATIVE chars={chars} average={} maxchar={}",
                metrics.tmAveCharWidth, metrics.tmMaxCharWidth
            );
            let mut previous = native_row_pixels(f.native);
            let mut last = 0;
            for code in [
                SB_LINERIGHT,
                SB_LINERIGHT,
                SB_LINELEFT,
                SB_PAGERIGHT,
                SB_PAGELEFT,
                SB_RIGHT,
                SB_LEFT,
            ] {
                SendMessageW(
                    f.native,
                    WM_HSCROLL,
                    Some(WPARAM(code.0 as usize)),
                    Some(LPARAM(0)),
                );
                let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                GetScrollInfo(f.native, SB_HORZ, &mut info).unwrap();
                let pixels = native_row_pixels(f.native);
                let maximum = info.nMax - (info.nPage as i32 - 1).max(0);
                let expected = match code.0 {
                    0 => last - horizontal_unit(f.native),
                    1 => last + horizontal_unit(f.native),
                    2 => last - (info.nPage as i32 / 3 * 2).max(1),
                    3 => last + (info.nPage as i32 / 3 * 2).max(1),
                    6 => 0,
                    7 => maximum,
                    _ => unreachable!(),
                }
                .clamp(0, maximum);
                assert_eq!(
                    info.nPos, expected,
                    "native H increment contract code={}",
                    code.0
                );
                if expected != last {
                    assert_ne!(
                        previous, pixels,
                        "native H must move actual rendered content"
                    );
                }
                last = info.nPos;
                let mut item = RECT::default();
                SendMessageW(
                    f.native,
                    LB_GETITEMRECT,
                    Some(WPARAM(0)),
                    Some(LPARAM((&mut item as *mut RECT) as isize)),
                );
                eprintln!(
                    "H_NATIVE code={} pos={} page={} item={item:?} actual_pixels_changed={}",
                    code.0,
                    info.nPos,
                    info.nPage,
                    previous != pixels
                );
                previous = pixels;
            }
            for (delta, flags) in [
                (60_i16, 0),
                (60, 0),
                (-60, 0),
                (-60, 0),
                (120, 4),
                (-120, 4),
                (120, 8),
                (-120, 8),
            ] {
                SendMessageW(
                    f.native,
                    WM_MOUSEHWHEEL,
                    Some(WPARAM(((delta as u16 as usize) << 16) | flags)),
                    Some(LPARAM(0)),
                );
                let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                GetScrollInfo(f.native, SB_HORZ, &mut info).unwrap();
                let pixels = native_row_pixels(f.native);
                eprintln!(
                    "H_NATIVE hwheel={delta} flags={flags} pos={} actual_pixels_changed={}",
                    info.nPos,
                    previous != pixels
                );
                previous = pixels;
            }
        }
    }
    #[test]
    fn explorer_horizontal_units_match_stock_ui_and_scaled_assigned_fonts() {
        let _wheel = NATIVE_HORIZONTAL_WHEEL.lock().unwrap();
        let f = Fixture::explorer(50);
        unsafe {
            set_horizontal_extent(f.adapted, 1800).unwrap();
            SetWindowPos(
                f.native,
                None,
                0,
                0,
                307,
                227,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let mut data = LOGFONTW {
                lfHeight: -24,
                ..Default::default()
            };
            let name = wide(shell::UI_FONT_FAMILY);
            data.lfFaceName[..name.len()].copy_from_slice(&name);
            let scaled = CreateFontIndirectW(&data);
            assert_ne!(scaled, HFONT::default());
            for font in [
                GetStockObject(SYSTEM_FONT),
                GetStockObject(DEFAULT_GUI_FONT),
                scaled.into(),
            ] {
                f.send_both(WM_SETFONT, WPARAM(font.0 as usize), LPARAM(0));
                f.send_both(WM_HSCROLL, WPARAM(SB_LEFT.0 as usize), LPARAM(0));
                f.send_both(WM_HSCROLL, WPARAM(SB_LINERIGHT.0 as usize), LPARAM(0));
                let mut native = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                GetScrollInfo(f.native, SB_HORZ, &mut native).unwrap();
                let mut actual = LOGFONTW::default();
                GetObjectW(
                    font,
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut actual as *mut LOGFONTW).cast()),
                );
                eprintln!(
                    "H_FONT height={} alphabet_unit={} native_line={}",
                    actual.lfHeight,
                    horizontal_unit(f.native),
                    native.nPos
                );
                assert_eq!(native.nPos, horizontal_unit(f.native));
                assert_eq!(horizontal_offset(f.adapted), native.nPos);
                f.send_both(WM_MOUSEHWHEEL, WPARAM(120 << 16), LPARAM(0));
                GetScrollInfo(f.native, SB_HORZ, &mut native).unwrap();
                assert_eq!(horizontal_offset(f.adapted), native.nPos);
            }
            f.send_both(
                WM_SETFONT,
                WPARAM(GetStockObject(SYSTEM_FONT).0 as usize),
                LPARAM(0),
            );
            let _ = DeleteObject(scaled.into());
        }
    }

    #[test]
    fn explorer_horizontal_messages_wheel_and_viewport_match_native() {
        let _wheel = NATIVE_HORIZONTAL_WHEEL.lock().unwrap();
        let f = Fixture::explorer(50);
        unsafe {
            set_horizontal_extent(f.adapted, 1800).unwrap();
            // Equal text viewports despite the custom rails being 7px thinner.
            SetWindowPos(
                f.native,
                None,
                0,
                0,
                307,
                227,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let mut a = RECT::default();
            let mut b = RECT::default();
            GetClientRect(f.native, &mut a).unwrap();
            GetClientRect(f.adapted, &mut b).unwrap();
            assert_eq!((a.right, a.bottom), (b.right, b.bottom));
            let check = |label: &str| {
                let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                GetScrollInfo(f.native, SB_HORZ, &mut info).unwrap();
                assert_eq!(
                    horizontal_offset(f.adapted),
                    info.nPos,
                    "H semantic content offset {label}"
                );
                for msg in [LB_GETTOPINDEX, LB_GETCURSEL, LB_GETCARETINDEX] {
                    assert_eq!(
                        SendMessageW(f.native, msg, Some(WPARAM(0)), Some(LPARAM(0))).0,
                        SendMessageW(f.adapted, msg, Some(WPARAM(0)), Some(LPARAM(0))).0,
                        "selection/V stays native {label}"
                    );
                }
            };
            for code in [
                SB_LINERIGHT,
                SB_LINERIGHT,
                SB_LINELEFT,
                SB_PAGERIGHT,
                SB_PAGELEFT,
                SB_RIGHT,
                SB_LEFT,
            ] {
                f.send_both(WM_HSCROLL, WPARAM(code.0 as usize), LPARAM(0));
                check("scroll request");
            }
            for (delta, flags) in [
                (60_i16, 0),
                (60, 0),
                (-60, 0),
                (-60, 0),
                (30, 0),
                (30, 0),
                (30, 0),
                (30, 0),
                (-120, 0),
                (120, 4),
                (-120, 4),
                (120, 8),
                (-120, 8),
            ] {
                let mut chars = 0u32;
                SystemParametersInfoW(
                    SPI_GETWHEELSCROLLCHARS,
                    0,
                    Some((&mut chars as *mut u32).cast()),
                    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                )
                .unwrap();
                let state = f.state();
                let horizontal = state.horizontal.get().unwrap();
                let mut native = scrollbar_style::scroll_info(scrollbar_style::Range::default());
                GetScrollInfo(f.native, SB_HORZ, &mut native).unwrap();
                let mut native_client = RECT::default();
                let mut adapted_client = RECT::default();
                GetClientRect(f.native, &mut native_client).unwrap();
                GetClientRect(f.adapted, &mut adapted_client).unwrap();
                let native_unit = horizontal_unit(f.native);
                let adapted_unit = horizontal_unit(f.adapted);
                eprintln!(
                    "H_WHEEL before delta={delta} flags={flags} native_font={:#x} adapted_font={:#x} native_unit={native_unit} adapted_unit={adapted_unit} native_width={} adapted_width={} chars={chars} effective_chars={} remainder={} custom={} native={}",
                    SendMessageW(f.native, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))).0,
                    SendMessageW(f.adapted, WM_GETFONT, Some(WPARAM(0)), Some(LPARAM(0))).0,
                    native_client.right,
                    adapted_client.right,
                    chars.min((adapted_client.right / adapted_unit).max(1) as u32),
                    horizontal.wheel.get(),
                    horizontal.offset.get(),
                    native.nPos
                );
                f.send_both(
                    WM_MOUSEHWHEEL,
                    WPARAM(((delta as u16 as usize) << 16) | flags),
                    LPARAM(0),
                );
                GetScrollInfo(f.native, SB_HORZ, &mut native).unwrap();
                eprintln!(
                    "H_WHEEL after delta={delta} flags={flags} remainder={} custom={} native={}",
                    horizontal.wheel.get(),
                    horizontal.offset.get(),
                    native.nPos
                );
                check(&format!("horizontal wheel delta={delta} flags={flags}"));
            }
            for key in [VK_DOWN, VK_UP, VK_NEXT, VK_PRIOR, VK_END, VK_HOME] {
                f.send_both(WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0));
                check("native vertical key");
            }
            let font = GetStockObject(DEFAULT_GUI_FONT);
            f.send_both(WM_SETFONT, WPARAM(font.0 as usize), LPARAM(0));
            for code in [SB_LINERIGHT, SB_PAGERIGHT, SB_LEFT] {
                f.send_both(WM_HSCROLL, WPARAM(code.0 as usize), LPARAM(0));
                check("assigned font");
            }
        }
    }

    #[test]
    fn explorer_horizontal_model_full_i32_combined_reservations_hc_and_lifecycle() {
        let f = Fixture::explorer(500);
        unsafe {
            set_horizontal_extent(f.adapted, 200_000).unwrap();
            let state = f.state();
            let h = state.horizontal.get().unwrap();
            let rail = h.rail.hwnd();
            let mut client = RECT::default();
            GetClientRect(f.adapted, &mut client).unwrap();
            assert_eq!((state.reserve.get(), state.bottom.get()), (10, 10));
            let mut vrect = RECT::default();
            let mut hrect = RECT::default();
            GetWindowRect(state.rail.hwnd(), &mut vrect).unwrap();
            GetWindowRect(rail, &mut hrect).unwrap();
            assert_eq!(vrect.bottom, hrect.top);
            assert_eq!(hrect.right, vrect.right);
            assert_eq!(hrect.right - hrect.left, client.right + 10);
            assert_eq!(
                GetWindowLongW(f.adapted, GWL_STYLE) as u32 & (WS_VSCROLL.0 | WS_HSCROLL.0),
                0
            );
            assert!(set_horizontal_offset(f.adapted, 150_000));
            assert_eq!(horizontal_offset(f.adapted), 150_000);
            set_horizontal_offset(f.adapted, 0);
            let pointer = |message, x: i32| {
                SendMessageW(
                    rail,
                    message,
                    Some(WPARAM(0)),
                    Some(LPARAM((x as u16 as u32 | 5 << 16) as isize)),
                );
            };
            pointer(WM_LBUTTONDOWN, 5);
            assert_eq!(GetCapture(), rail);
            pointer(WM_MOUSEMOVE, 200);
            pointer(WM_LBUTTONUP, 220);
            let mut message = MSG::default();
            while PeekMessageW(
                &mut message,
                Some(f.adapted),
                H_REQUEST,
                H_REQUEST,
                PM_REMOVE,
            )
            .as_bool()
            {
                DispatchMessageW(&message);
            }
            assert!(horizontal_offset(f.adapted) > 65_535);
            assert_ne!(GetCapture(), rail);
            let current = horizontal_offset(f.adapted);
            pointer(WM_LBUTTONDOWN, client.right + 5);
            assert_ne!(GetCapture(), rail);
            assert_eq!(horizontal_offset(f.adapted), current);
            let surface = Surface::new(rail, client.right + 10, 10);
            SendMessageW(
                rail,
                WM_PRINTCLIENT,
                Some(WPARAM(surface.0.0 as usize)),
                Some(LPARAM(0)),
            );
            assert_eq!(
                GetPixel(surface.0, client.right + 5, 5),
                shell::rgb(shell::Tokens::default().panel),
                "corner is background, never a second track/hit region"
            );
            state.sync(144, false);
            assert_eq!((state.reserve.get(), state.bottom.get()), (15, 15));
            state.sync(96, true);
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
            GetScrollInfo(f.adapted, SB_HORZ, &mut info).unwrap();
            assert_eq!(info.nPos, horizontal_offset(f.adapted));
            assert_eq!(info.nMax, 199_999);
            SendMessageW(
                f.adapted,
                WM_HSCROLL,
                Some(WPARAM(SB_PAGELEFT.0 as usize)),
                Some(LPARAM(0)),
            );
            assert!(horizontal_offset(f.adapted) < current);
            state.sync(96, false);
            assert_eq!(
                GetWindowLongW(f.adapted, GWL_STYLE) as u32 & (WS_VSCROLL.0 | WS_HSCROLL.0),
                0
            );
            let _ = ShowWindow(f.adapted, SW_HIDE);
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let _ = ShowWindow(f.adapted, SW_SHOWNA);
            assert_ne!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            set_horizontal_extent(f.adapted, 0).unwrap();
            assert_eq!(horizontal_offset(f.adapted), 0);
            assert_eq!(state.bottom.get(), 0);
            SendMessageW(f.adapted, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            assert_eq!(state.reserve.get(), 0);
            drop(surface);
            drop(state);
            let owner = f.adapted;
            drop(f);
            assert!(!IsWindow(Some(owner)).as_bool());
            assert!(!IsWindow(Some(rail)).as_bool());
            assert_eq!(horizontal_offset(owner), 0);
        }
    }

    #[test]
    fn explorer_horizontal_renderer_shows_known_suffix_beyond_u16_offset() {
        let f = Fixture::explorer(0);
        unsafe {
            SetWindowLongPtrW(f.adapted, GWLP_ID, 4690);
            let font = GetStockObject(DEFAULT_GUI_FONT);
            SendMessageW(
                f.adapted,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(0)),
            );
            let prefix = "W".repeat(12_000);
            let suffix = "TARGET_uniqueGlyph_0123456789_".repeat(12) + ".md";
            let label = wide(&(prefix.clone() + &suffix));
            SendMessageW(
                f.adapted,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(label.as_ptr() as isize)),
            );
            SendMessageW(f.adapted, LB_SETITEMDATA, Some(WPARAM(0)), Some(LPARAM(0)));
            let dc = GetDC(Some(f.adapted));
            let old = SelectObject(dc, font);
            let mut prefix_size = SIZE::default();
            GetTextExtentPoint32W(
                dc,
                &wide(&prefix)[..prefix.encode_utf16().count()],
                &mut prefix_size,
            )
            .unwrap();
            SelectObject(dc, old);
            ReleaseDC(Some(f.adapted), dc);
            assert!(prefix_size.cx > 65_535);
            let mut icons = super::super::explorer_icons::ExplorerIcons::default();
            let extent = icons.measure_extent(f.adapted).unwrap();
            assert!(extent > prefix_size.cx + 300);
            let snapshot = icons
                .paint_snapshot(f.adapted, shell::Tokens::default())
                .unwrap();
            set_horizontal_extent(f.adapted, extent).unwrap();
            set_horizontal_offset(f.adapted, prefix_size.cx);
            assert_eq!(horizontal_offset(f.adapted), prefix_size.cx);
            let long = Surface::new(f.adapted, 320, 32);
            let short = Surface::new(f.adapted, 320, 32);
            let item = |dc| DRAWITEMSTRUCT {
                CtlType: ODT_LISTBOX,
                CtlID: 4690,
                itemID: 0,
                itemAction: ODA_DRAWENTIRE,
                hwndItem: f.adapted,
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 298,
                    bottom: 32,
                },
                ..Default::default()
            };
            assert!(snapshot.draw(&item(long.0), horizontal_offset(f.adapted)));
            SendMessageW(f.adapted, LB_RESETCONTENT, Some(WPARAM(0)), Some(LPARAM(0)));
            let label = wide(&suffix);
            SendMessageW(
                f.adapted,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(label.as_ptr() as isize)),
            );
            SendMessageW(f.adapted, LB_SETITEMDATA, Some(WPARAM(0)), Some(LPARAM(0)));
            set_horizontal_offset(f.adapted, 0);
            assert!(snapshot.draw(&item(short.0), 0));
            let background = shell::rgb(shell::Tokens::default().panel);
            let mut non_background = 0;
            for y in 1..16 {
                for x in 64..280 {
                    let pixel = GetPixel(short.0, x, y);
                    if pixel != background {
                        non_background += 1;
                    }
                    assert_eq!(
                        GetPixel(long.0, x, y),
                        pixel,
                        "large offset must render known suffix, not wrapped/blank content at {x},{y}"
                    );
                }
            }
            assert!(
                non_background > 10,
                "comparison must contain actual glyph pixels"
            );
        }
    }

    #[test]
    #[ignore = "visible NOACTIVATE rail invalidation regression; requires exclusive GUI lease"]
    fn list_vertical_identical_owner_updates_preserve_clean_rail_and_visual_changes() {
        let fixture = Fixture::explorer(100);
        let foreground = unsafe { GetForegroundWindow() };
        unsafe {
            let state = fixture.state();
            let rail = state.rail.hwnd();
            let _ = ShowWindow(fixture.host, SW_SHOWNOACTIVATE);
            assert!(IsWindowVisible(rail).as_bool());
            SendMessageW(
                fixture.adapted,
                LB_SETSEL,
                Some(WPARAM(0)),
                Some(LPARAM(-1)),
            );
            let bounds = || {
                let mut rect = RECT::default();
                GetWindowRect(rail, &mut rect).unwrap();
                [rect.left, rect.top, rect.right, rect.bottom]
            };
            let damage = || {
                let mut rect = RECT::default();
                GetUpdateRect(rail, Some(&mut rect), false)
                    .as_bool()
                    .then_some([rect.left, rect.top, rect.right, rect.bottom])
            };
            let clean = || {
                let _ = UpdateWindow(rail);
                assert!(
                    damage().is_none(),
                    "test-only baseline rail paint must finish"
                );
            };
            let range = state.range(fixture.adapted);
            let rect = bounds();
            let selection = SendMessageW(
                fixture.adapted,
                LB_GETSELCOUNT,
                Some(WPARAM(0)),
                Some(LPARAM(0)),
            )
            .0;
            for (label, message, wp, lp) in [
                ("identical LB_SETSEL none", LB_SETSEL, WPARAM(0), LPARAM(-1)),
                (
                    "identical LB_SETTOPINDEX",
                    LB_SETTOPINDEX,
                    WPARAM(range.position as usize),
                    LPARAM(0),
                ),
            ] {
                clean();
                SendMessageW(fixture.adapted, message, Some(wp), Some(lp));
                assert!(state.range(fixture.adapted) == range);
                assert_eq!(bounds(), rect);
                assert_eq!(
                    SendMessageW(
                        fixture.adapted,
                        LB_GETSELCOUNT,
                        Some(WPARAM(0)),
                        Some(LPARAM(0))
                    )
                    .0,
                    selection
                );
                assert!(
                    damage().is_none(),
                    "{label} must not dirty an unchanged rail"
                );
                println!(
                    "rail-cause {label}: damage={:?} range={}/{}/{} rect={rect:?} selection={selection} state_unchanged=true",
                    damage(),
                    range.content,
                    range.page,
                    range.position
                );
            }
            clean();
            refresh_listbox(fixture.adapted, shell::Tokens::default()).unwrap();
            assert!(state.range(fixture.adapted) == range);
            assert_eq!(bounds(), rect);
            assert!(
                damage().is_none(),
                "identical refresh must leave a clean rail clean"
            );
            println!(
                "rail-cause identical theme refresh: damage={:?} state_unchanged=true",
                damage()
            );

            let _ = InvalidateRect(Some(rail), None, false);
            refresh_listbox(fixture.adapted, shell::Tokens::default()).unwrap();
            assert!(
                damage().is_some(),
                "identical refresh must preserve independently pending damage"
            );

            clean();
            let _ = ShowWindow(fixture.adapted, SW_HIDE);
            assert!(!IsWindowVisible(rail).as_bool());
            for native_policy in [false, true] {
                state.sync(96, native_policy);
                // Paint the fixture's existing damage with the real rail
                // renderer; do not validate/discard pending regions by hand.
                SendMessageW(rail, WM_PAINT, Some(WPARAM(0)), Some(LPARAM(0)));
                assert!(damage().is_none());
                let hidden_range = state.range(fixture.adapted);
                let hidden_bounds = bounds();
                for (message, wp, lp) in [
                    (LB_SETSEL, WPARAM(0), LPARAM(-1)),
                    (
                        LB_SETTOPINDEX,
                        WPARAM(hidden_range.position as usize),
                        LPARAM(0),
                    ),
                ] {
                    SendMessageW(fixture.adapted, message, Some(wp), Some(lp));
                    assert!(state.range(fixture.adapted) == hidden_range);
                    assert_eq!(bounds(), hidden_bounds);
                    assert!(
                        damage().is_none(),
                        "unchanged hidden/native rail must stay clean"
                    );
                }
                state.sync(96, native_policy);
                assert!(damage().is_none(), "identical hidden/native sync");
            }
            state.sync(96, false);
            let _ = ShowWindow(fixture.adapted, SW_SHOWNA);
            assert!(IsWindowVisible(rail).as_bool());
            assert!(
                damage().is_some(),
                "showing the rail must retain its visibility repaint"
            );
            assert_eq!(GetForegroundWindow(), foreground);

            clean();
            SendMessageW(
                fixture.adapted,
                LB_SETTOPINDEX,
                Some(WPARAM(1)),
                Some(LPARAM(0)),
            );
            assert_eq!(state.range(fixture.adapted).position, 1);
            assert!(damage().is_some());
            println!("rail-cause changed position: damage={:?}", damage());
            clean();
            let mut tokens = shell::Tokens::default();
            tokens.muted.r = (tokens.muted.r + 0.2).min(1.);
            refresh_listbox(fixture.adapted, tokens).unwrap();
            assert!(damage().is_some());
            println!("rail-cause changed theme: damage={:?}", damage());
            clean();
            SetWindowPos(
                fixture.adapted,
                None,
                350,
                10,
                320,
                250,
                SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .unwrap();
            assert_ne!(bounds(), rect);
            assert!(damage().is_some());
            println!(
                "rail-cause changed geometry: damage={:?} rect={:?}",
                damage(),
                bounds()
            );
            clean();
            SendMessageW(rail, WM_MOUSEMOVE, Some(WPARAM(0)), Some(LPARAM(5 << 16)));
            assert!(damage().is_some());
            println!("rail-cause native hover: damage={:?}", damage());
            clean();
            SendMessageW(rail, WM_CANCELMODE, Some(WPARAM(0)), Some(LPARAM(0)));
            assert!(damage().is_some());
            println!(
                "rail-cause native cancel after hover: damage={:?}",
                damage()
            );
            clean();
            SendMessageW(rail, WM_MOUSEMOVE, Some(WPARAM(0)), Some(LPARAM(5 << 16)));
            clean();
            assert_ne!(
                GetCapture(),
                rail,
                "public cancellation case must have no capture"
            );
            state.rail.cancel();
            refresh_listbox(fixture.adapted, state.tokens.get()).unwrap();
            assert!(
                damage().is_some(),
                "public hover-only cancellation must redraw after identical update"
            );
            clean();
            state.rail.cancel();
            refresh_listbox(fixture.adapted, state.tokens.get()).unwrap();
            assert!(
                damage().is_none(),
                "unchanged public cancellation must leave a clean rail clean"
            );
            let _ = InvalidateRect(Some(rail), None, false);
            state.rail.cancel();
            assert!(
                damage().is_some(),
                "public cancellation must preserve independently pending damage"
            );
            println!(
                "rail-cause no-capture public cancellation redraw/no-op/independent damage: verified"
            );
            set_horizontal_extent(fixture.adapted, 1800).unwrap();
            let horizontal = state.horizontal.get().unwrap();
            let hrail = horizontal.rail.hwnd();
            let mut hbounds = RECT::default();
            GetWindowRect(hrail, &mut hbounds).unwrap();
            let mut origin = POINT {
                x: hbounds.left,
                y: hbounds.top,
            };
            assert!(ScreenToClient(fixture.host, &mut origin).as_bool());
            let hrect = RECT {
                left: origin.x,
                top: origin.y,
                right: origin.x + hbounds.right - hbounds.left,
                bottom: origin.y + hbounds.bottom - hbounds.top,
            };
            let hrange = state.h_range(fixture.adapted);
            let corner = state.reserve.get() + 1;
            let _ = UpdateWindow(hrail);
            assert!(!GetUpdateRect(hrail, None, false).as_bool());
            assert!(horizontal.rail.update_horizontal_sibling(
                hrange,
                96,
                state.tokens.get(),
                false,
                hrect,
                corner
            ));
            assert!(
                GetUpdateRect(hrail, None, false).as_bool(),
                "corner-only change must redraw horizontal thumb geometry"
            );
            let _ = UpdateWindow(hrail);
            assert!(horizontal.rail.update_horizontal_sibling(
                hrange,
                96,
                state.tokens.get(),
                false,
                hrect,
                corner
            ));
            assert!(
                !GetUpdateRect(hrail, None, false).as_bool(),
                "identical horizontal update must leave the rail clean"
            );
            println!("rail-cause horizontal corner-only redraw and identical update: verified");
            assert_eq!(GetForegroundWindow(), foreground);
        }
    }

    #[test]
    fn list_vertical_wheel_partial_modifiers_and_native_keys_match() {
        let f = Fixture::new(500);
        unsafe {
            for (sequence_number, sequence) in [
                vec![(60, 0), (60, 0), (-60, 0), (-60, 0)],
                vec![(30, 0); 4]
                    .into_iter()
                    .chain(vec![(-30, 0); 4])
                    .collect(),
                vec![(60, 0), (60, 4), (60, 0), (-120, 0)],
                vec![
                    (120, 4),
                    (-120, 4),
                    (120, 8),
                    (-120, 8),
                    (120, 12),
                    (-120, 12),
                ],
                vec![(240, 0), (-240, 0)],
            ]
            .into_iter()
            .enumerate()
            {
                f.send_both(LB_SETTOPINDEX, WPARAM(100), LPARAM(0));
                for (step, (delta, flags)) in sequence.into_iter().enumerate() {
                    f.send_both(
                        WM_MOUSEWHEEL,
                        WPARAM(((delta as i16 as u16 as usize) << 16) | flags),
                        LPARAM(0),
                    );
                    eprintln!(
                        "WHEEL_STEP seq={sequence_number} step={step} delta={delta} flags={flags} native={} adapter={} remainder={}",
                        SendMessageW(f.native, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0))).0,
                        SendMessageW(f.adapted, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0))).0,
                        f.state().wheel.get()
                    );
                    f.equivalent(&format!(
                        "wheel seq={sequence_number} step={step} delta={delta} flags={flags}"
                    ));
                }
            }
            for key in [VK_DOWN, VK_UP, VK_NEXT, VK_PRIOR, VK_END, VK_HOME] {
                f.send_both(WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0));
                f.equivalent(&format!("native key={key:?}"));
            }
            f.send_both(LB_SETCURSEL, WPARAM(430), LPARAM(0));
            f.equivalent("selection ensure-visible");
            let mut selected = RECT::default();
            SendMessageW(
                f.adapted,
                LB_GETITEMRECT,
                Some(WPARAM(430)),
                Some(LPARAM((&mut selected as *mut RECT) as isize)),
            );
            let mut client = RECT::default();
            GetClientRect(f.adapted, &mut client).unwrap();
            assert!(selected.top >= 0 && selected.bottom <= client.bottom);
        }
    }

    #[test]
    fn list_vertical_wheel_actual_string_styles_edges_and_native_key_mix_match() {
        unsafe {
            for flags in [
                LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | LBS_HASSTRINGS,
                LBS_NOTIFY
                    | LBS_NOINTEGRALHEIGHT
                    | LBS_HASSTRINGS
                    | LBS_OWNERDRAWFIXED
                    | LBS_EXTENDEDSEL,
            ] {
                let f = Fixture::with_style(500, flags);
                for top in [0, 100, 499] {
                    f.send_both(LB_SETTOPINDEX, WPARAM(top), LPARAM(0));
                    for delta in [60_i16, 60, -60, -60, -30, -30, -30, -30, 120] {
                        f.send_both(
                            WM_MOUSEWHEEL,
                            WPARAM((delta as u16 as usize) << 16),
                            LPARAM(0),
                        );
                        f.equivalent(&format!("real style={flags:x} edge={top} delta={delta}"));
                    }
                }
                for key in [VK_UP, VK_DOWN, VK_NEXT, VK_PRIOR, VK_HOME, VK_END] {
                    f.send_both(WM_MOUSEWHEEL, WPARAM((60u16 as usize) << 16), LPARAM(0));
                    f.send_both(WM_KEYDOWN, WPARAM(key.0 as usize), LPARAM(0));
                    f.send_both(WM_MOUSEWHEEL, WPARAM((60u16 as usize) << 16), LPARAM(0));
                    f.equivalent("partial wheel/native key/partial wheel");
                }
                for msg in [LB_GETSELCOUNT, LB_GETSEL] {
                    assert_eq!(
                        SendMessageW(f.native, msg, Some(WPARAM(40)), Some(LPARAM(0))).0,
                        SendMessageW(f.adapted, msg, Some(WPARAM(40)), Some(LPARAM(0))).0
                    );
                }
            }
        }
    }
    #[test]
    fn list_vertical_range_font_resize_geometry_and_horizontal_dc_end() {
        let f = Fixture::new(500);
        unsafe {
            let state = f.state();
            let rail = state.rail.hwnd();
            let mut client = RECT::default();
            GetClientRect(f.adapted, &mut client).unwrap();
            let mut item = RECT::default();
            SendMessageW(
                f.adapted,
                LB_GETITEMRECT,
                Some(WPARAM(0)),
                Some(LPARAM((&mut item as *mut RECT) as isize)),
            );
            assert_eq!(
                item.right, client.right,
                "LISTBOX text/selection rectangle must exclude reserved rail"
            );
            let mut h = scrollbar_style::scroll_info(scrollbar_style::Range::default());
            GetScrollInfo(f.adapted, SB_HORZ, &mut h).unwrap();
            assert_eq!(
                h.nPage, client.right as u32,
                "native H viewport must use the reserved text client width"
            );
            let mut rr = RECT::default();
            GetWindowRect(rail, &mut rr).unwrap();
            let mut hs = SCROLLBARINFO {
                cbSize: std::mem::size_of::<SCROLLBARINFO>() as u32,
                ..Default::default()
            };
            GetScrollBarInfo(f.adapted, OBJID_HSCROLL, &mut hs).unwrap();
            assert!(
                rr.bottom <= hs.rcScrollBar.top,
                "sibling rail must end above native H"
            );
            let mut origin = POINT {
                x: client.right,
                y: 0,
            };
            ClientToScreen(f.adapted, &mut origin).unwrap();
            assert_eq!((rr.left, rr.top), (origin.x, origin.y));
            assert_eq!(rr.right - rr.left, 10);
            // Render only this hidden control into an offscreen DC; no desktop pixels.
            let dc = GetDC(Some(f.host));
            let memory = CreateCompatibleDC(Some(dc));
            let bitmap = CreateCompatibleBitmap(dc, 320, 260);
            let old = SelectObject(memory, bitmap.into());
            let sentinel = CreateSolidBrush(COLORREF(0x00030201));
            FillRect(
                memory,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 260,
                },
                sentinel,
            );
            SendMessageW(
                f.adapted,
                WM_PRINT,
                Some(WPARAM(memory.0 as usize)),
                Some(LPARAM(
                    (PRF_NONCLIENT | PRF_CLIENT | PRF_ERASEBKGND) as isize,
                )),
            );
            let mut window = RECT::default();
            GetWindowRect(f.adapted, &mut window).unwrap();
            let sample = GetPixel(
                memory,
                hs.rcScrollBar.right - window.left - 3,
                (hs.rcScrollBar.top + hs.rcScrollBar.bottom) / 2 - window.top,
            );
            SelectObject(memory, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteObject(sentinel.into());
            let _ = DeleteDC(memory);
            ReleaseDC(Some(f.host), dc);
            assert_ne!(
                sample,
                COLORREF(0x00030201),
                "native H end must actually render into the offscreen DC"
            );
            let stock = GetStockObject(DEFAULT_GUI_FONT);
            f.send_both(WM_SETFONT, WPARAM(stock.0 as usize), LPARAM(0));
            f.equivalent("font");
            for hwnd in [f.native, f.adapted] {
                SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    320,
                    260,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
            }
            f.equivalent("resize");
            f.send_both(LB_SETCOUNT, WPARAM(6), LPARAM(0));
            f.equivalent("items fit");
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            f.send_both(LB_SETCOUNT, WPARAM(700), LPARAM(0));
            f.equivalent("items restore");
            assert_ne!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            state.sync(144, false);
            GetWindowRect(rail, &mut rr).unwrap();
            assert_eq!(rr.right - rr.left, 15);
        }
    }

    #[test]
    fn list_vertical_full_i32_drag_hc_restore_and_lifetime() {
        let f = Fixture::new(200_000);
        unsafe {
            let state = f.state();
            let rail = state.rail.hwnd();
            f.send_both(LB_SETTOPINDEX, WPARAM(150_000), LPARAM(0));
            f.equivalent("full i32 native content index");
            f.send_both(LB_SETTOPINDEX, WPARAM(0), LPARAM(0));
            let pointer = |msg, y: i32| {
                SendMessageW(
                    rail,
                    msg,
                    Some(WPARAM(0)),
                    Some(LPARAM(((y as u16 as u32) << 16) as isize)),
                );
            };
            pointer(WM_LBUTTONDOWN, 5);
            assert_eq!(GetCapture(), rail);
            pointer(WM_MOUSEMOVE, 170);
            pointer(WM_LBUTTONUP, 160);
            assert_ne!(GetCapture(), rail);
            let mut message = MSG::default();
            while PeekMessageW(&mut message, Some(f.adapted), REQUEST, REQUEST, PM_REMOVE).as_bool()
            {
                DispatchMessageW(&message);
            }
            let top = SendMessageW(f.adapted, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0))).0;
            assert!(
                top > 150_000,
                "custom drag must deliver all position bits to LB_SETTOPINDEX: {top}"
            );
            state.sync(96, true);
            assert_ne!(
                GetWindowLongW(f.adapted, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
            let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
            GetScrollInfo(f.adapted, SB_VERT, &mut info).unwrap();
            assert_eq!(info.nPos, top as i32);
            SendMessageW(
                f.adapted,
                WM_VSCROLL,
                Some(WPARAM(SB_PAGEDOWN.0 as usize)),
                Some(LPARAM(0)),
            );
            assert!(
                SendMessageW(f.adapted, LB_GETTOPINDEX, Some(WPARAM(0)), Some(LPARAM(0))).0 > top
            );
            state.sync(96, false);
            f.send_both(LB_SETTOPINDEX, WPARAM(150_000), LPARAM(0));
            f.equivalent("HC return custom");
            let focus = GetFocus();
            pointer(WM_LBUTTONDOWN, 5);
            SendMessageW(rail, WM_CANCELMODE, Some(WPARAM(0)), Some(LPARAM(0)));
            assert_ne!(GetCapture(), rail);
            assert_eq!(GetFocus(), focus);
            DestroyWindow(f.adapted).unwrap();
            assert!(!IsWindow(Some(rail)).as_bool());
            assert_eq!(state.hwnd.get(), HWND::default());
        }
    }

    #[test]
    fn list_vertical_sibling_follows_owner_visibility_enable_and_cancel() {
        let f = Fixture::new(500);
        unsafe {
            let state = f.state();
            let rail = state.rail.hwnd();
            let _ = ShowWindow(f.adapted, SW_HIDE);
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let _ = ShowWindow(f.adapted, SW_SHOWNA);
            assert_ne!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            SetWindowPos(
                f.adapted,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_HIDEWINDOW,
            )
            .unwrap();
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            SetWindowPos(
                f.adapted,
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_SHOWWINDOW | SWP_NOACTIVATE,
            )
            .unwrap();
            assert_ne!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            SendMessageW(rail, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(LPARAM(5 << 16)));
            assert_eq!(GetCapture(), rail);
            let _ = EnableWindow(f.adapted, false);
            assert_ne!(GetCapture(), rail);
            assert!(!IsWindowEnabled(rail).as_bool());
            let _ = EnableWindow(f.adapted, true);
            assert!(IsWindowEnabled(rail).as_bool());
            SendMessageW(rail, WM_LBUTTONDOWN, Some(WPARAM(0)), Some(LPARAM(5 << 16)));
            assert_eq!(GetCapture(), rail);
            SendMessageW(f.adapted, WM_CANCELMODE, Some(WPARAM(0)), Some(LPARAM(0)));
            assert_ne!(GetCapture(), rail);
        }
    }
    #[test]
    fn list_vertical_wheel_honors_zero_lines_page_and_clamps_without_global_changes() {
        let range = scrollbar_style::Range {
            content: 200_000,
            page: 11,
            position: 100_000,
        };
        assert_eq!(wheel_target(&Cell::new(0), -120, 0, 0, range), 100_000);
        assert_eq!(
            wheel_target(&Cell::new(0), -120, 0, u32::MAX, range),
            100_011
        );
        assert_eq!(wheel_target(&Cell::new(0), -240, 0, 100, range), 100_022);
        assert_eq!(
            wheel_target(
                &Cell::new(0),
                32767,
                0,
                u32::MAX,
                scrollbar_style::Range {
                    position: 0,
                    ..range
                }
            ),
            0
        );
    }
}
