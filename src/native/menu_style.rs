//! Scoped native popup painting. Win32 still owns menu tracking, placement and
//! accessibility strings. Item data is an opaque lookup key, never dereferenced.
use super::*;
use windows::Win32::UI::{Accessibility::*, Controls::*};

struct Entry {
    menu: HMENU,
    position: u32,
    original: MENUITEMINFOW,
    label: String,
    shortcut: String,
    width: u32,
    height: u32,
    last: bool,
}
impl Entry {
    fn key(&self) -> usize {
        self as *const Self as usize
    }
    fn separator(&self) -> bool {
        self.original.fType.contains(MFT_SEPARATOR)
    }
    fn enabled(&self) -> bool {
        self.original.fState.0 & MFS_DISABLED.0 == 0
    }
}

#[derive(Clone, Copy)]
struct PendingPlacement {
    menu: RECT,
    work: RECT,
}

pub(super) struct MenuPaint {
    owner: HWND,
    tokens: shell::Tokens,
    scale: f32,
    font: control_style::UiFont,
    background: HBRUSH,
    menus: Vec<(HMENU, HBRUSH)>,
    entries: Vec<Entry>,
    help: Vec<(usize, Vec<u16>)>,
    tooltip: HWND,
    pending_placement: Cell<Option<PendingPlacement>>,
    trace: Option<Rc<PhaseTrace>>,
    styled: bool,
}
impl MenuPaint {
    fn px(&self, dip: f32) -> i32 {
        (dip * self.scale).round().max(1.) as i32
    }
    fn entry(&self, key: usize) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key() == key)
    }
    fn owns(&self, menu: HMENU) -> bool {
        self.menus.iter().any(|(candidate, _)| *candidate == menu)
    }
}

struct MenuTraceScope {
    trace: Option<Rc<PhaseTrace>>,
    phase: &'static str,
}
impl MenuTraceScope {
    fn new(trace: &Option<Rc<PhaseTrace>>, phase: &'static str) -> Self {
        record_phase(trace, phase, "enter");
        Self {
            trace: trace.clone(),
            phase,
        }
    }
    fn completed(&self) {
        record_phase(&self.trace, self.phase, "completed");
    }
}
impl Drop for MenuTraceScope {
    fn drop(&mut self) {
        record_phase(&self.trace, self.phase, "scope_exit");
    }
}

impl Drop for MenuPaint {
    fn drop(&mut self) {
        unsafe {
            if self.tooltip != HWND::default() {
                let _ = DestroyWindow(self.tooltip);
            }
            let _ = DeleteObject(self.background.into());
        }
    }
}

/// Never changes OS menus or foreign ownerdraw items. A partial installation is
/// rolled back before the snapshot/brush/font can be released.
pub(super) struct MenuScope<'a> {
    state: &'a WindowState,
    paint: Rc<MenuPaint>,
}
impl<'a> MenuScope<'a> {
    pub(super) unsafe fn new(
        state: &'a WindowState,
        owner: HWND,
        menu: HMENU,
        tokens: shell::Tokens,
        scale: f32,
    ) -> Option<Self> {
        unsafe { Self::with_help(state, owner, menu, tokens, scale, &[]) }
    }
    unsafe fn with_help(
        state: &'a WindowState,
        owner: HWND,
        menu: HMENU,
        tokens: shell::Tokens,
        scale: f32,
        help: &[(usize, String)],
    ) -> Option<Self> {
        unsafe {
            let styled = !tokens.high_contrast && !high_contrast();
            if (!styled && help.is_empty())
                || !scale.is_finite()
                || scale <= 0.
                || GetWindowLongPtrW(owner, GWLP_USERDATA) != state as *const _ as isize
                || !state.owns_paint_surface(owner)
            {
                return None;
            }
            let font =
                control_style::UiFont::new((96. * scale).round() as u32, shell::UI_FONT_SIZE)
                    .ok()?;
            let background = CreateSolidBrush(shell::rgb(tokens.panel));
            if background == HBRUSH::default() {
                return None;
            }
            let trace = if help.is_empty() {
                None
            } else {
                state.phase_trace.clone()
            };
            let mut paint = MenuPaint {
                owner,
                tokens,
                scale,
                font,
                background,
                menus: Vec::new(),
                entries: Vec::new(),
                help: help.iter().map(|(id, text)| (*id, wide(text))).collect(),
                tooltip: HWND::default(),
                pending_placement: Cell::new(None),
                trace: trace.clone(),
                styled,
            };
            collect(&mut paint, menu)?;
            if state
                .menu_paint
                .borrow()
                .iter()
                .any(|active| paint.menus.iter().any(|(menu, _)| active.owns(*menu)))
            {
                return None;
            }
            measure(&mut paint);
            if !paint.help.is_empty() {
                {
                    let phase = MenuTraceScope::new(&trace, "menu.tooltip.create");
                    let _native = TooltipOperationScope::new(state);
                    paint.tooltip = CreateWindowExW(
                        WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                        TOOLTIPS_CLASSW,
                        None,
                        WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        CW_USEDEFAULT,
                        Some(owner),
                        None,
                        None,
                        None,
                    )
                    .unwrap_or_default();
                    phase.completed();
                }
                record_phase_value(
                    &trace,
                    "menu.tooltip.create",
                    "available",
                    (paint.tooltip != HWND::default()) as i64,
                );
                if paint.tooltip != HWND::default() {
                    {
                        let _native = TooltipOperationScope::new(state);
                        let phase = MenuTraceScope::new(&trace, "menu.tooltip.inherit_font");
                        let _ = control_style::inherit_font(paint.tooltip, owner);
                        phase.completed();
                    }
                    let mut tool = help_tool(owner, PWSTR(paint.help[0].1.as_mut_ptr()));
                    let tool_count_before = {
                        let phase =
                            MenuTraceScope::new(&trace, "menu.tooltip.tool_count_before_add");
                        let _native = TooltipOperationScope::new(state);
                        let count =
                            shell::send(paint.tooltip, TTM_GETTOOLCOUNT, WPARAM(0), LPARAM(0));
                        phase.completed();
                        count
                    };
                    record_phase_value(
                        &trace,
                        "menu.tooltip.tool_count_before_add",
                        "value",
                        tool_count_before.0 as i64,
                    );
                    let control_version = {
                        let phase = MenuTraceScope::new(&trace, "menu.tooltip.ccm_getversion");
                        let _native = TooltipOperationScope::new(state);
                        let version =
                            shell::send(paint.tooltip, CCM_GETVERSION, WPARAM(0), LPARAM(0));
                        phase.completed();
                        version
                    };
                    record_phase_value(
                        &trace,
                        "menu.tooltip.ccm_getversion",
                        "value",
                        control_version.0 as i64,
                    );
                    record_phase_value(
                        &trace,
                        "menu.tooltip.toolinfo_cbsize",
                        "value",
                        i64::from(tool.cbSize),
                    );
                    record_phase_value(
                        &trace,
                        "menu.tooltip.toolinfo_rust_size",
                        "value",
                        std::mem::size_of::<TTTOOLINFOW>() as i64,
                    );
                    let add_result = {
                        let phase = MenuTraceScope::new(&trace, "menu.tooltip.add_tool");
                        let _native = TooltipOperationScope::new(state);
                        let result = shell::send(
                            paint.tooltip,
                            TTM_ADDTOOLW,
                            WPARAM(0),
                            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                        );
                        phase.completed();
                        result
                    };
                    record_phase_value(
                        &trace,
                        "menu.tooltip.add_tool",
                        "result",
                        add_result.0 as i64,
                    );
                    let tool_count_after = {
                        let phase =
                            MenuTraceScope::new(&trace, "menu.tooltip.tool_count_after_add");
                        let _native = TooltipOperationScope::new(state);
                        let count =
                            shell::send(paint.tooltip, TTM_GETTOOLCOUNT, WPARAM(0), LPARAM(0));
                        phase.completed();
                        count
                    };
                    record_phase_value(
                        &trace,
                        "menu.tooltip.tool_count_after_add",
                        "value",
                        tool_count_after.0 as i64,
                    );
                    {
                        let phase = MenuTraceScope::new(&trace, "menu.tooltip.set_max_width");
                        let _native = TooltipOperationScope::new(state);
                        shell::send(
                            paint.tooltip,
                            TTM_SETMAXTIPWIDTH,
                            WPARAM(0),
                            LPARAM((360. * scale) as isize),
                        );
                        phase.completed();
                    }
                }
            }
            // No entries are added after this point: their addresses are stable
            // opaque keys throughout the tracking scope, including callbacks.
            let scope = Self {
                state,
                paint: Rc::new(paint),
            };
            for (menu, _) in scope.paint.menus.iter().filter(|_| scope.paint.styled) {
                let info = MENUINFO {
                    cbSize: std::mem::size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    hbrBack: scope.paint.background,
                    ..Default::default()
                };
                if SetMenuInfo(*menu, &info).is_err() {
                    return None;
                }
            }
            for entry in scope.paint.entries.iter().filter(|_| scope.paint.styled) {
                let item = MENUITEMINFOW {
                    cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
                    fMask: MIIM_FTYPE | MIIM_DATA,
                    fType: entry.original.fType | MFT_OWNERDRAW,
                    dwItemData: entry.key(),
                    ..Default::default()
                };
                if SetMenuItemInfoW(entry.menu, entry.position, true, &item).is_err() {
                    return None;
                }
            }
            state.menu_paint.borrow_mut().push(Rc::clone(&scope.paint));
            Some(scope)
        }
    }

    #[cfg(test)]
    pub(super) unsafe fn with_help_for_test(
        state: &'a WindowState,
        owner: HWND,
        menu: HMENU,
        tokens: shell::Tokens,
        scale: f32,
        help: &[(usize, String)],
    ) -> Option<Self> {
        unsafe { Self::with_help(state, owner, menu, tokens, scale, help) }
    }

    #[cfg(test)]
    pub(super) fn tooltip_handle_for_test(&self) -> HWND {
        self.paint.tooltip
    }

    #[cfg(test)]
    pub(super) fn set_pending_placement_for_test(&self, menu: RECT, work: RECT) {
        self.paint
            .pending_placement
            .set(Some(PendingPlacement { menu, work }));
    }

    #[cfg(test)]
    pub(super) unsafe fn activate_tooltip_for_test(&self) {
        unsafe {
            if let Some((_, text)) = self.paint.help.first() {
                let mut tool = help_tool(self.paint.owner, PWSTR(text.as_ptr() as *mut u16));
                let phase = MenuTraceScope::new(&self.paint.trace, "menu.tooltip.activate");
                let _native = TooltipOperationScope::new(self.state);
                shell::send(
                    self.paint.tooltip,
                    TTM_TRACKACTIVATE,
                    WPARAM(1),
                    LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                );
                phase.completed();
            }
        }
    }
}
impl Drop for MenuScope<'_> {
    fn drop(&mut self) {
        self.paint.pending_placement.set(None);
        self.state
            .menu_paint
            .borrow_mut()
            .retain(|paint| !Rc::ptr_eq(paint, &self.paint));
        unsafe {
            for entry in &self.paint.entries {
                let mut current = item_info(entry.menu, entry.position).unwrap_or_default();
                // Do not overwrite changes made by a later native/menu owner.
                if current.dwItemData != entry.key() {
                    continue;
                }
                current.fMask = MIIM_FTYPE | MIIM_DATA;
                current.fType = MENU_ITEM_TYPE(
                    (current.fType.0 & !MFT_OWNERDRAW.0)
                        | (entry.original.fType.0 & MFT_OWNERDRAW.0),
                );
                current.dwItemData = entry.original.dwItemData;
                let _ = SetMenuItemInfoW(entry.menu, entry.position, true, &current);
            }
            for (menu, original) in &self.paint.menus {
                let mut current = MENUINFO {
                    cbSize: std::mem::size_of::<MENUINFO>() as u32,
                    fMask: MIM_BACKGROUND,
                    ..Default::default()
                };
                if GetMenuInfo(*menu, &mut current).is_ok()
                    && current.hbrBack == self.paint.background
                {
                    current.hbrBack = *original;
                    let _ = SetMenuInfo(*menu, &current);
                }
            }
        }
    }
}

unsafe fn item_info(menu: HMENU, position: u32) -> Option<MENUITEMINFOW> {
    unsafe {
        let mut info = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_DATA | MIIM_STATE | MIIM_SUBMENU | MIIM_ID,
            ..Default::default()
        };
        GetMenuItemInfoW(menu, position, true, &mut info).ok()?;
        Some(info)
    }
}
unsafe fn collect(paint: &mut MenuPaint, menu: HMENU) -> Option<()> {
    unsafe {
        // A repeated handle also rejects cycles before traversing them.
        if paint.owns(menu) {
            return None;
        }
        let mut info = MENUINFO {
            cbSize: std::mem::size_of::<MENUINFO>() as u32,
            fMask: MIM_BACKGROUND,
            ..Default::default()
        };
        GetMenuInfo(menu, &mut info).ok()?;
        paint.menus.push((menu, info.hbrBack));
        let count = GetMenuItemCount(Some(menu));
        if count < 0 {
            return None;
        }
        for position in 0..count as u32 {
            let original = item_info(menu, position)?;
            if original.fType.contains(MFT_OWNERDRAW) || original.fType.contains(MFT_BITMAP) {
                return None;
            }
            let length = GetMenuStringW(menu, position, None, MF_BYPOSITION).max(0) as usize;
            let mut text = vec![0u16; length + 1];
            GetMenuStringW(menu, position, Some(&mut text), MF_BYPOSITION);
            let text = String::from_utf16_lossy(&text[..length]);
            let (label, shortcut) = text.split_once('\t').unwrap_or((&text, ""));
            paint.entries.push(Entry {
                menu,
                position,
                original,
                label: label.into(),
                shortcut: shortcut.into(),
                width: 0,
                height: 0,
                last: position + 1 == count as u32,
            });
            if original.hSubMenu != HMENU::default() {
                collect(paint, original.hSubMenu)?;
            }
        }
        Some(())
    }
}
unsafe fn measure(paint: &mut MenuPaint) {
    unsafe {
        let dc = GetDC(Some(paint.owner));
        let saved = SaveDC(dc);
        SelectObject(dc, paint.font.handle().into());
        let scale = paint.scale;
        for entry in &mut paint.entries {
            let mut rect = RECT::default();
            draw_gdi_text(dc, &entry.label, &mut rect, DT_SINGLELINE | DT_CALCRECT);
            let label_width = rect.right - rect.left;
            rect = RECT::default();
            draw_gdi_text(
                dc,
                &entry.shortcut,
                &mut rect,
                DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX,
            );
            entry.width = (label_width + rect.right - rect.left
                + ((if entry.shortcut.is_empty() { 60. } else { 88. }) * scale).round() as i32)
                .max(1) as u32;
            entry.height = ((if entry.separator() {
                7.
            } else {
                shell::UI_ROW_HEIGHT
            }) * scale)
                .round()
                .max(1.) as u32;
        }
        let _ = RestoreDC(dc, saved);
        ReleaseDC(Some(paint.owner), dc);
    }
}

fn high_contrast() -> bool {
    let mut contrast = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    unsafe {
        // On an unreadable OS setting, preserve the native accessible menu.
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            Some((&mut contrast as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_err()
            || contrast.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
}

pub(super) unsafe fn track_popup(
    owner: HWND,
    menu: HMENU,
    flags: TRACK_POPUP_MENU_FLAGS,
    x: i32,
    y: i32,
    tokens: shell::Tokens,
    scale: f32,
) -> windows::core::BOOL {
    unsafe {
        let pointer = GetWindowLongPtrW(owner, GWLP_USERDATA) as *const WindowState;
        let _scope = if pointer.is_null() {
            None
        } else {
            MenuScope::new(&*pointer, owner, menu, tokens, scale)
        };
        TrackPopupMenu(menu, flags, x, y, None, owner, None)
    }
}

fn help_tool(owner: HWND, text: PWSTR) -> TTTOOLINFOW {
    TTTOOLINFOW {
        cbSize: shell::TOOLTIP_INFO_SIZE,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: text,
        ..Default::default()
    }
}

fn menu_bounds(paint: &MenuPaint, menu: HMENU) -> Option<RECT> {
    let mut bounds: Option<RECT> = None;
    for entry in paint.entries.iter().filter(|entry| entry.menu == menu) {
        let mut item = RECT::default();
        unsafe {
            GetMenuItemRect(Some(paint.owner), menu, entry.position, &mut item).ok()?;
        }
        if let Some(bounds) = bounds.as_mut() {
            bounds.left = bounds.left.min(item.left);
            bounds.top = bounds.top.min(item.top);
            bounds.right = bounds.right.max(item.right);
            bounds.bottom = bounds.bottom.max(item.bottom);
        } else {
            bounds = Some(item);
        }
    }
    bounds
}

fn menu_stack_bounds(paint: &MenuPaint, menu: HMENU) -> Option<RECT> {
    let mut bounds = menu_bounds(paint, menu)?;
    let mut child = menu;
    while let Some(parent) = paint
        .entries
        .iter()
        .find(|entry| entry.original.hSubMenu == child)
        .map(|entry| entry.menu)
    {
        let parent_bounds = menu_bounds(paint, parent)?;
        bounds.left = bounds.left.min(parent_bounds.left);
        bounds.top = bounds.top.min(parent_bounds.top);
        bounds.right = bounds.right.max(parent_bounds.right);
        bounds.bottom = bounds.bottom.max(parent_bounds.bottom);
        child = parent;
    }
    Some(bounds)
}

fn menu_work_area(bounds: &RECT) -> Option<RECT> {
    unsafe {
        let monitor = MonitorFromRect(bounds, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(monitor, &mut info).as_bool() {
            return None;
        }
        let work = info.rcWork;
        (work.right > work.left && work.bottom > work.top).then_some(work)
    }
}

fn help_position(menu: RECT, tip: SIZE, work: RECT, gap: i32) -> Option<POINT> {
    if tip.cx <= 0 || tip.cy <= 0 || gap < 0 || work.right <= work.left || work.bottom <= work.top {
        return None;
    }
    let max_x = work.right.saturating_sub(tip.cx);
    let max_y = work.bottom.saturating_sub(tip.cy);
    if max_x < work.left || max_y < work.top {
        return None;
    }
    let right = menu.right.saturating_add(gap);
    if right >= work.left && right <= max_x {
        return Some(POINT {
            x: right,
            y: menu.top.clamp(work.top, max_y),
        });
    }
    let left = menu.left.saturating_sub(gap).saturating_sub(tip.cx);
    if left >= work.left && left <= max_x {
        return Some(POINT {
            x: left,
            y: menu.top.clamp(work.top, max_y),
        });
    }
    let below = menu.bottom.saturating_add(gap);
    if below >= work.top && below <= max_y {
        return Some(POINT {
            x: menu.left.clamp(work.left, max_x),
            y: below,
        });
    }
    let above = menu.top.saturating_sub(gap).saturating_sub(tip.cy);
    if above >= work.top && above <= max_y {
        return Some(POINT {
            x: menu.left.clamp(work.left, max_x),
            y: above,
        });
    }
    None
}

unsafe fn position_tooltip_on_show(state: &WindowState, paint: &MenuPaint) -> LRESULT {
    unsafe {
        let phase = MenuTraceScope::new(&paint.trace, "menu.tooltip.ttn_show");
        let Some(pending) = paint.pending_placement.get() else {
            record_phase(&paint.trace, "menu.tooltip.ttn_show", "failure_no_pending");
            record_phase_value(&paint.trace, "menu.tooltip.ttn_show", "custom_position", 0);
            phase.completed();
            return LRESULT(0);
        };

        let mut tooltip_rect = RECT::default();
        let rect_available = {
            let _native = TooltipOperationScope::new(state);
            let rect_phase = MenuTraceScope::new(&paint.trace, "menu.tooltip.ttn_show_window_rect");
            let available = GetWindowRect(paint.tooltip, &mut tooltip_rect).is_ok();
            rect_phase.completed();
            available
        };
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "native_rect_available",
            rect_available as i64,
        );
        if !rect_available {
            record_phase(&paint.trace, "menu.tooltip.ttn_show", "failure_window_rect");
            record_phase_value(&paint.trace, "menu.tooltip.ttn_show", "custom_position", 0);
            phase.completed();
            return LRESULT(0);
        }

        let tip = SIZE {
            cx: tooltip_rect.right.saturating_sub(tooltip_rect.left),
            cy: tooltip_rect.bottom.saturating_sub(tooltip_rect.top),
        };
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "native_width",
            i64::from(tip.cx),
        );
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "native_height",
            i64::from(tip.cy),
        );
        if tip.cx <= 0 || tip.cy <= 0 {
            record_phase(
                &paint.trace,
                "menu.tooltip.ttn_show",
                "failure_invalid_size",
            );
            record_phase_value(&paint.trace, "menu.tooltip.ttn_show", "custom_position", 0);
            phase.completed();
            return LRESULT(0);
        }

        let position = {
            let placement = MenuTraceScope::new(&paint.trace, "menu.tooltip.ttn_show_placement");
            let position = help_position(pending.menu, tip, pending.work, paint.px(8.));
            placement.completed();
            position
        };
        let Some(position) = position else {
            record_phase(&paint.trace, "menu.tooltip.ttn_show", "failure_no_fit");
            record_phase_value(
                &paint.trace,
                "menu.tooltip.ttn_show",
                "placement_available",
                0,
            );
            record_phase_value(&paint.trace, "menu.tooltip.ttn_show", "custom_position", 0);
            phase.completed();
            return LRESULT(0);
        };
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "placement_available",
            1,
        );
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "x",
            i64::from(position.x),
        );
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "y",
            i64::from(position.y),
        );
        let positioned = {
            let _native = TooltipOperationScope::new(state);
            let set_position =
                MenuTraceScope::new(&paint.trace, "menu.tooltip.ttn_show_set_position");
            let positioned = SetWindowPos(
                paint.tooltip,
                None,
                position.x,
                position.y,
                0,
                0,
                SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOZORDER,
            )
            .is_ok();
            set_position.completed();
            positioned
        };
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "set_window_pos_succeeded",
            positioned as i64,
        );
        if !positioned {
            record_phase(
                &paint.trace,
                "menu.tooltip.ttn_show",
                "failure_set_window_pos",
            );
        }
        record_phase_value(
            &paint.trace,
            "menu.tooltip.ttn_show",
            "custom_position",
            positioned as i64,
        );
        phase.completed();
        LRESULT(if positioned { 1 } else { 0 })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn track_popup_help(
    owner: HWND,
    menu: HMENU,
    flags: TRACK_POPUP_MENU_FLAGS,
    x: i32,
    y: i32,
    tokens: shell::Tokens,
    scale: f32,
    help: &[(usize, String)],
) -> windows::core::BOOL {
    unsafe {
        let pointer = GetWindowLongPtrW(owner, GWLP_USERDATA) as *const WindowState;
        let trace = if help.is_empty() || pointer.is_null() {
            None
        } else {
            (*pointer).phase_trace.clone()
        };
        let tracking = MenuTraceScope::new(&trace, "menu.track_popup");
        record_phase_value(
            &trace,
            "menu.track_popup",
            "help_entry_count",
            help.len() as i64,
        );
        let _scope = if pointer.is_null() {
            None
        } else {
            MenuScope::with_help(&*pointer, owner, menu, tokens, scale, help)
        };
        let result = TrackPopupMenu(menu, flags, x, y, None, owner, None);
        tracking.completed();
        result
    }
}

pub(super) unsafe fn window_message(
    state: &WindowState,
    owner: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    if msg == WM_NOTIFY {
        if lp.0 == 0 {
            return None;
        }
        let notification = unsafe { &*(lp.0 as *const NMHDR) };
        if notification.code != TTN_SHOW {
            return None;
        }
        let active = state.menu_paint.borrow().clone();
        if let Some(paint) = active
            .iter()
            .rev()
            .find(|paint| paint.owner == owner && paint.tooltip == notification.hwndFrom)
        {
            return Some(unsafe { position_tooltip_on_show(state, paint) });
        }
        return None;
    }
    if !matches!(
        msg,
        WM_INITMENUPOPUP | WM_MEASUREITEM | WM_DRAWITEM | WM_MENUCHAR | WM_MENUSELECT
    ) {
        return None;
    }
    // Release the slot borrow before GDI/native calls can reenter.
    let active = state.menu_paint.borrow().clone();
    for paint in active.iter().rev().filter(|paint| paint.owner == owner) {
        let _callback = paint
            .trace
            .as_ref()
            .map(|trace| MessagePhaseSpan::new(Rc::clone(trace), "menu.owner_callback", msg));
        unsafe {
            match msg {
                WM_MENUSELECT if paint.tooltip != HWND::default() => {
                    paint.pending_placement.set(None);
                    let mut tool = help_tool(owner, PWSTR::null());
                    {
                        let phase = MenuTraceScope::new(&paint.trace, "menu.tooltip.deactivate");
                        let _native = TooltipOperationScope::new(state);
                        shell::send(
                            paint.tooltip,
                            TTM_TRACKACTIVATE,
                            WPARAM(0),
                            LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                        );
                        phase.completed();
                    }
                    let id = wp.0 & 0xffff;
                    let flags = (wp.0 >> 16) as u32;
                    if flags & (MF_POPUP.0 | MF_SEPARATOR.0) == 0
                        && let Some((_, text)) = paint.help.iter().find(|(key, _)| *key == id)
                    {
                        tool.lpszText = PWSTR(text.as_ptr() as *mut u16);
                        let menu = HMENU(lp.0 as *mut _);
                        let bounds = {
                            let phase = MenuTraceScope::new(&paint.trace, "menu.tooltip.bounds");
                            let _native = TooltipOperationScope::new(state);
                            let bounds = menu_stack_bounds(paint, menu);
                            phase.completed();
                            bounds
                        };
                        record_phase_value(
                            &paint.trace,
                            "menu.tooltip.bounds",
                            "available",
                            bounds.is_some() as i64,
                        );
                        if let Some(bounds) = bounds {
                            for (field, value) in [
                                ("left", bounds.left),
                                ("top", bounds.top),
                                ("right", bounds.right),
                                ("bottom", bounds.bottom),
                            ] {
                                record_phase_value(
                                    &paint.trace,
                                    "menu.tooltip.bounds",
                                    field,
                                    i64::from(value),
                                );
                            }
                            let work = {
                                let phase =
                                    MenuTraceScope::new(&paint.trace, "menu.tooltip.work_area");
                                let _native = TooltipOperationScope::new(state);
                                let work = menu_work_area(&bounds);
                                phase.completed();
                                work
                            };
                            record_phase_value(
                                &paint.trace,
                                "menu.tooltip.work_area",
                                "available",
                                work.is_some() as i64,
                            );
                            if let Some(work) = work {
                                for (field, value) in [
                                    ("left", work.left),
                                    ("top", work.top),
                                    ("right", work.right),
                                    ("bottom", work.bottom),
                                ] {
                                    record_phase_value(
                                        &paint.trace,
                                        "menu.tooltip.work_area",
                                        field,
                                        i64::from(value),
                                    );
                                }
                                paint
                                    .pending_placement
                                    .set(Some(PendingPlacement { menu: bounds, work }));
                                record_phase_value(
                                    &paint.trace,
                                    "menu.tooltip.pending_placement",
                                    "available",
                                    1,
                                );
                                {
                                    let phase = MenuTraceScope::new(
                                        &paint.trace,
                                        "menu.tooltip.text_update",
                                    );
                                    let _native = TooltipOperationScope::new(state);
                                    shell::send(
                                        paint.tooltip,
                                        TTM_UPDATETIPTEXTW,
                                        WPARAM(0),
                                        LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                                    );
                                    phase.completed();
                                }
                                let phase =
                                    MenuTraceScope::new(&paint.trace, "menu.tooltip.activate");
                                let _native = TooltipOperationScope::new(state);
                                shell::send(
                                    paint.tooltip,
                                    TTM_TRACKACTIVATE,
                                    WPARAM(1),
                                    LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                                );
                                phase.completed();
                            } else {
                                record_phase(
                                    &paint.trace,
                                    "menu.tooltip.pending_placement",
                                    "failure_no_work_area",
                                );
                            }
                        } else {
                            record_phase(
                                &paint.trace,
                                "menu.tooltip.pending_placement",
                                "failure_no_menu_bounds",
                            );
                        }
                        if paint.pending_placement.get().is_none() {
                            record_phase_value(
                                &paint.trace,
                                "menu.tooltip.pending_placement",
                                "available",
                                0,
                            );
                        }
                    }
                    // Win32 retains menu selection/accessibility processing.
                }
                WM_INITMENUPOPUP if paint.styled && paint.owns(HMENU(wp.0 as *mut _)) => {
                    return Some(LRESULT(0));
                }
                WM_MEASUREITEM if wp.0 == 0 && lp.0 != 0 => {
                    let item = &mut *(lp.0 as *mut MEASUREITEMSTRUCT);
                    if item.CtlType == ODT_MENU
                        && let Some(entry) = paint.entry(item.itemData)
                    {
                        item.itemWidth = entry.width;
                        item.itemHeight = entry.height;
                        return Some(LRESULT(1));
                    }
                }
                WM_DRAWITEM if wp.0 == 0 && lp.0 != 0 => {
                    let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                    if item.CtlType == ODT_MENU
                        && let Some(entry) = paint.entry(item.itemData)
                        && item.hwndItem.0 == entry.menu.0
                    {
                        draw(paint, entry, item);
                        return Some(LRESULT(1));
                    }
                }
                WM_MENUCHAR if paint.styled && paint.owns(HMENU(lp.0 as *mut _)) => {
                    return Some(menu_char(paint, HMENU(lp.0 as *mut _), wp.0 as u16));
                }
                _ => {}
            }
        }
    }
    None
}

unsafe fn fill(dc: HDC, rect: &RECT, color: D2D1_COLOR_F) {
    unsafe {
        SetDCBrushColor(dc, shell::rgb(color));
        FillRect(dc, rect, HBRUSH(GetStockObject(DC_BRUSH).0));
    }
}
unsafe fn draw(paint: &MenuPaint, entry: &Entry, item: &DRAWITEMSTRUCT) {
    unsafe {
        let saved = SaveDC(item.hDC);
        SelectObject(item.hDC, paint.font.handle().into());
        SetBkMode(item.hDC, TRANSPARENT);
        let disabled = !entry.enabled() || item.itemState.0 & (ODS_DISABLED.0 | ODS_GRAYED.0) != 0;
        let selected = !disabled && item.itemState.0 & ODS_SELECTED.0 != 0;
        let tokens = paint.tokens;
        let background = if selected {
            tokens.selected
        } else {
            tokens.panel
        };
        fill(item.hDC, &item.rcItem, background);
        // Theme the content edge without subclassing OS popup windows.
        let mut edge = item.rcItem;
        edge.right = edge.left + paint.px(1.);
        fill(item.hDC, &edge, tokens.border);
        edge = item.rcItem;
        edge.left = edge.right - paint.px(1.);
        fill(item.hDC, &edge, tokens.border);
        if entry.position == 0 {
            edge = item.rcItem;
            edge.bottom = edge.top + paint.px(1.);
            fill(item.hDC, &edge, tokens.border);
        }
        if entry.last {
            edge = item.rcItem;
            edge.top = edge.bottom - paint.px(1.);
            fill(item.hDC, &edge, tokens.border);
        }
        if entry.separator() {
            let mut line = item.rcItem;
            line.left += paint.px(12.);
            line.right -= paint.px(12.);
            line.top = (line.top + line.bottom) / 2;
            line.bottom = line.top + paint.px(1.);
            fill(item.hDC, &line, tokens.border);
        } else {
            SetTextColor(
                item.hDC,
                shell::rgb(if disabled {
                    disabled_color(tokens.muted, background)
                } else {
                    tokens.foreground
                }),
            );
            let mut rect = item.rcItem;
            rect.left += paint.px(30.);
            rect.right -= paint.px(28.);
            let mut flags = DT_SINGLELINE | DT_VCENTER | DT_LEFT;
            if item.itemState.0 & ODS_NOACCEL.0 != 0 {
                flags |= DT_HIDEPREFIX;
            }
            draw_gdi_text(item.hDC, &entry.label, &mut rect, flags);
            draw_gdi_text(
                item.hDC,
                &entry.shortcut,
                &mut rect,
                DT_SINGLELINE | DT_VCENTER | DT_RIGHT | DT_NOPREFIX,
            );
            if item.itemState.0 & ODS_CHECKED.0 != 0 || entry.original.fState.contains(MFS_CHECKED)
            {
                rect = item.rcItem;
                rect.right = rect.left + paint.px(28.);
                draw_gdi_text(
                    item.hDC,
                    "✓",
                    &mut rect,
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
            if entry.original.hSubMenu != HMENU::default() {
                rect = item.rcItem;
                rect.left = rect.right - paint.px(24.);
                draw_gdi_text(
                    item.hDC,
                    "▸",
                    &mut rect,
                    DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
                );
            }
        }
        let _ = RestoreDC(item.hDC, saved);
    }
}

fn luminance(color: D2D1_COLOR_F) -> f32 {
    let linear = |value: f32| {
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}
fn contrast(a: D2D1_COLOR_F, b: D2D1_COLOR_F) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
fn disabled_color(muted: D2D1_COLOR_F, background: D2D1_COLOR_F) -> D2D1_COLOR_F {
    if contrast(muted, background) >= 3. {
        return muted;
    }
    let white = D2D1_COLOR_F {
        r: 1.,
        g: 1.,
        b: 1.,
        a: 1.,
    };
    let black = D2D1_COLOR_F {
        r: 0.,
        g: 0.,
        b: 0.,
        a: 1.,
    };
    let target = if contrast(white, background) > contrast(black, background) {
        white
    } else {
        black
    };
    for step in 1..=10 {
        let t = step as f32 / 10.;
        let value = D2D1_COLOR_F {
            r: muted.r + (target.r - muted.r) * t,
            g: muted.g + (target.g - muted.g) * t,
            b: muted.b + (target.b - muted.b) * t,
            a: 1.,
        };
        if contrast(value, background) >= 3. {
            return value;
        }
    }
    target
}

fn mnemonic(label: &str) -> Option<char> {
    let mut chars = label.chars();
    while let Some(ch) = chars.next() {
        if ch == '&' {
            let next = chars.next()?;
            if next != '&' {
                return Some(next);
            }
        }
    }
    None
}
unsafe fn menu_char(paint: &MenuPaint, menu: HMENU, key: u16) -> LRESULT {
    let key = char::from_u32(key as u32)
        .unwrap_or('\0')
        .to_lowercase()
        .to_string();
    let matches: Vec<_> = paint
        .entries
        .iter()
        .filter(|entry| {
            entry.menu == menu
                && entry.enabled()
                && !entry.separator()
                && mnemonic(&entry.label).is_some_and(|ch| ch.to_lowercase().to_string() == key)
        })
        .collect();
    let Some(first) = matches.first() else {
        return LRESULT((MNC_IGNORE << 16) as isize);
    };
    let (entry, action) = if matches.len() == 1 {
        (*first, MNC_EXECUTE)
    } else {
        let selected = matches.iter().position(|entry| unsafe {
            item_info(menu, entry.position).is_some_and(|info| info.fState.contains(MFS_HILITE))
        });
        (
            matches[selected.map_or(0, |index| (index + 1) % matches.len())],
            MNC_SELECT,
        )
    };
    LRESULT((entry.position | (action << 16)) as isize)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_menu_color_has_readable_contrast_for_dark_light_and_custom() {
        for value in [0., 0.14, 0.4, 0.6, 1.] {
            let background = D2D1_COLOR_F {
                r: value,
                g: value,
                b: value,
                a: 1.,
            };
            assert!(contrast(disabled_color(background, background), background) >= 3.);
        }
        assert_eq!(mnemonic("Save && close (&S)"), Some('S'));
        assert_eq!(mnemonic("日本語"), None);
    }

    #[test]
    fn menu_help_tooltip_avoids_menu_on_right_left_and_at_work_area_edge() {
        let tip = SIZE { cx: 180, cy: 70 };
        let work = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 700,
        };
        assert_eq!(
            help_position(
                RECT {
                    left: 60,
                    top: 100,
                    right: 260,
                    bottom: 500,
                },
                tip,
                work,
                8,
            ),
            Some(POINT { x: 268, y: 100 })
        );
        assert_eq!(
            help_position(
                RECT {
                    left: 700,
                    top: 100,
                    right: 820,
                    bottom: 500,
                },
                tip,
                work,
                8,
            ),
            Some(POINT { x: 512, y: 100 })
        );

        let edge_work = RECT {
            left: -800,
            top: 40,
            right: 0,
            bottom: 600,
        };
        assert_eq!(
            help_position(
                RECT {
                    left: -200,
                    top: 540,
                    right: -20,
                    bottom: 580,
                },
                tip,
                edge_work,
                8,
            ),
            Some(POINT { x: -388, y: 530 })
        );
        assert_eq!(
            help_position(
                RECT {
                    left: 0,
                    top: 0,
                    right: 1000,
                    bottom: 700,
                },
                tip,
                work,
                8,
            ),
            None
        );
    }
}
