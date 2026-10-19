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
                if paint.tooltip != HWND::default() {
                    let _ = control_style::inherit_font(paint.tooltip, owner);
                    let mut tool = help_tool(owner, PWSTR(paint.help[0].1.as_mut_ptr()));
                    shell::send(
                        paint.tooltip,
                        TTM_ADDTOOLW,
                        WPARAM(0),
                        LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                    );
                    shell::send(
                        paint.tooltip,
                        TTM_SETMAXTIPWIDTH,
                        WPARAM(0),
                        LPARAM((360. * scale) as isize),
                    );
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
}
impl Drop for MenuScope<'_> {
    fn drop(&mut self) {
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
        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
        uFlags: TTF_TRACK | TTF_ABSOLUTE,
        hwnd: owner,
        uId: 1,
        lpszText: text,
        ..Default::default()
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
        let _scope = if pointer.is_null() {
            None
        } else {
            MenuScope::with_help(&*pointer, owner, menu, tokens, scale, help)
        };
        TrackPopupMenu(menu, flags, x, y, None, owner, None)
    }
}

pub(super) unsafe fn window_message(
    state: &WindowState,
    owner: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> Option<LRESULT> {
    if !matches!(
        msg,
        WM_INITMENUPOPUP | WM_MEASUREITEM | WM_DRAWITEM | WM_MENUCHAR | WM_MENUSELECT
    ) {
        return None;
    }
    // Release the slot borrow before GDI/native calls can reenter.
    let active = state.menu_paint.borrow().clone();
    for paint in active.iter().rev().filter(|paint| paint.owner == owner) {
        unsafe {
            match msg {
                WM_MENUSELECT if paint.tooltip != HWND::default() => {
                    let mut tool = help_tool(owner, PWSTR::null());
                    shell::send(
                        paint.tooltip,
                        TTM_TRACKACTIVATE,
                        WPARAM(0),
                        LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                    );
                    let id = wp.0 & 0xffff;
                    let flags = (wp.0 >> 16) as u32;
                    if flags & (MF_POPUP.0 | MF_SEPARATOR.0) == 0
                        && let Some((_, text)) = paint.help.iter().find(|(key, _)| *key == id)
                    {
                        tool.lpszText = PWSTR(text.as_ptr() as *mut u16);
                        let mut item = RECT::default();
                        if GetMenuItemRect(
                            Some(owner),
                            HMENU(lp.0 as *mut _),
                            paint
                                .entries
                                .iter()
                                .find(|entry| {
                                    entry.menu.0 as isize == lp.0
                                        && entry.original.wID as usize == id
                                })
                                .map_or(0, |entry| entry.position),
                            &mut item,
                        )
                        .is_ok()
                        {
                            shell::send(
                                paint.tooltip,
                                TTM_UPDATETIPTEXTW,
                                WPARAM(0),
                                LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                            );
                            let x = item.left;
                            let y = item.bottom + paint.px(3.);
                            shell::send(
                                paint.tooltip,
                                TTM_TRACKPOSITION,
                                WPARAM(0),
                                LPARAM(((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as isize),
                            );
                            shell::send(
                                paint.tooltip,
                                TTM_TRACKACTIVATE,
                                WPARAM(1),
                                LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
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
}
