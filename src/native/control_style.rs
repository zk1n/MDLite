//! UI-thread-owned fonts and client-only cursors for native child controls.
//! Standard scrollbars, combo dropdown tracking and nonclient cursors stay native.
use super::*;
use windows::Win32::UI::Controls::*;

const FONT_PROPERTY: PCWSTR = w!("MDLite.UiFont.v1");
const FONT_SUBCLASS: usize = 0x4d44_4653;
const CURSOR_SUBCLASS: usize = 0x4d44_4353;
const CURSOR_PROPERTY: PCWSTR = w!("MDLite.ChildCursor.v1");

fn owned_window(hwnd: HWND) -> bool {
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process)) };
    process == std::process::id()
        && thread == unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

pub(super) struct UiFont {
    handle: HFONT,
    root: Option<HWND>,
}
impl UiFont {
    pub(super) fn new(dpi: u32, size_dip: f32) -> Result<Self, String> {
        let height = size_dip * dpi as f32 / 96.;
        if dpi == 0 || !height.is_finite() || height <= 0. || height >= i32::MAX as f32 {
            return Err("A finite positive UI font size and DPI are required".into());
        }
        let mut data = LOGFONTW {
            lfHeight: -height.round().max(1.) as i32,
            lfCharSet: DEFAULT_CHARSET,
            ..Default::default()
        };
        let face = wide(shell::UI_FONT_FAMILY);
        data.lfFaceName[..face.len()].copy_from_slice(&face);
        let handle = unsafe { CreateFontIndirectW(&data) };
        if handle == HFONT::default() {
            return Err(windows::core::Error::from_thread().to_string());
        }
        Ok(Self { handle, root: None })
    }

    #[cfg(test)]
    pub(super) fn handle(&self) -> HFONT {
        self.handle
    }

    /// One owner surface per font. Nested surfaces with their own published font
    /// keep their own size. The property borrows HFONT; it never owns the font.
    pub(super) fn apply_to(&mut self, root: HWND) -> Result<(), String> {
        if !owned_window(root) || self.root.is_some_and(|old| old != root) {
            return Err("UI font requires one surface on its owning UI thread".into());
        }
        unsafe {
            if !SetWindowSubclass(root, Some(font_surface_proc), FONT_SUBCLASS, 0).as_bool() {
                return Err("Cannot install UI font property cleanup".into());
            }
            SetPropW(root, FONT_PROPERTY, Some(HANDLE(self.handle.0)))
                .map_err(|e| e.to_string())?;
            self.root = Some(root);
            let mut update = FontUpdate {
                root,
                font: self.handle,
            };
            let _ = EnumChildWindows(
                Some(root),
                Some(update_font),
                LPARAM((&mut update as *mut FontUpdate) as isize),
            );
        }
        Ok(())
    }

    /// Publish/reassign synchronously before releasing the previous HFONT.
    pub(super) fn replace_on(&mut self, root: HWND, dpi: u32, size_dip: f32) -> Result<(), String> {
        if self.root.is_some_and(|old| old != root) {
            return Err("Cannot move an owned UI font to another surface".into());
        }
        let mut replacement = Self::new(dpi, size_dip)?;
        replacement.apply_to(root)?;
        reassign_font_references(self.handle, replacement.handle);
        let previous = std::mem::replace(self, replacement);
        drop(previous);
        Ok(())
    }
}

unsafe extern "system" fn font_surface_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, FONT_PROPERTY);
            let _ = RemoveWindowSubclass(hwnd, Some(font_surface_proc), id);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

impl Drop for UiFont {
    fn drop(&mut self) {
        // Owners should destroy/reassign controls first. Include owned popups
        // (such as tooltips), which are not EnumChildWindows descendants.
        if let Some(root) = self.root.filter(|root| owned_window(*root)) {
            unsafe {
                if GetPropW(root, FONT_PROPERTY).0 == self.handle.0 {
                    let _ = RemovePropW(root, FONT_PROPERTY);
                }
            }
        }
        reassign_font_references(self.handle, HFONT::default());
        unsafe {
            let _ = DeleteObject(self.handle.into());
        }
    }
}

struct FontUpdate {
    root: HWND,
    font: HFONT,
}

struct FontReferences {
    from: HFONT,
    to: HFONT,
}
fn reassign_font_references(from: HFONT, to: HFONT) {
    let references = FontReferences { from, to };
    unsafe {
        let _ = EnumThreadWindows(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
            Some(update_surface_font_references),
            LPARAM((&references as *const FontReferences) as isize),
        );
    }
}
unsafe extern "system" fn update_font_reference(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        let references = &*(data.0 as *const FontReferences);
        if SendMessageW(hwnd, WM_GETFONT, None, None).0 == references.from.0 as isize {
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(references.to.0 as usize)),
                Some(LPARAM(1)),
            );
        }
        true.into()
    }
}
unsafe extern "system" fn update_surface_font_references(
    hwnd: HWND,
    data: LPARAM,
) -> windows::core::BOOL {
    unsafe {
        let _ = update_font_reference(hwnd, data);
        let _ = EnumChildWindows(Some(hwnd), Some(update_font_reference), data);
        true.into()
    }
}
unsafe extern "system" fn update_font(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        if scrollbar_style::is_rail(hwnd) {
            return true.into();
        }
        let update = &*(data.0 as *const FontUpdate);
        // A nearer published surface owns its descendants' font.
        let mut ancestor = hwnd;
        while ancestor != update.root {
            if !GetPropW(ancestor, FONT_PROPERTY).is_invalid() {
                return true.into();
            }
            ancestor = GetParent(ancestor).unwrap_or_default();
            if ancestor == HWND::default() {
                return true.into();
            }
        }
        let _ = SendMessageW(
            hwnd,
            WM_SETFONT,
            Some(WPARAM(update.font.0 as usize)),
            Some(LPARAM(1)),
        );
        true.into()
    }
}

/// Used after child or owned-popup creation. A published nearest surface wins.
pub(super) fn inherit_font(child: HWND, parent: HWND) -> Result<(), String> {
    if !owned_window(child)
        || !owned_window(parent)
        || unsafe { GetParent(child).unwrap_or_default() } != parent
    {
        return Err("Font inheritance requires an owned child or popup".into());
    }
    let mut ancestor = parent;
    unsafe {
        while ancestor != HWND::default() {
            let font = GetPropW(ancestor, FONT_PROPERTY);
            if !font.is_invalid() {
                let _ = SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
                return Ok(());
            }
            ancestor = GetParent(ancestor).unwrap_or_default();
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    Button,
    Edit,
    List,
    Native,
    Background,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ClientCursor {
    Arrow,
    Hand,
    Ibeam,
    Native,
}

struct CursorState {
    role: Role,
    #[cfg(test)]
    dropped: Option<Rc<Cell<bool>>>,
}
unsafe extern "system" fn cursor_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(cursor_proc), id);
            if GetPropW(hwnd, CURSOR_PROPERTY).0 as usize == data {
                let _ = RemovePropW(hwnd, CURSOR_PROPERTY);
            }
            drop(Box::from_raw(data as *mut CursorState));
            return DefSubclassProc(hwnd, msg, wp, lp);
        }
        // Forwarded descendant and all native nonclient requests must pass on.
        if msg == WM_SETCURSOR && wp.0 == hwnd.0 as usize && lp.0 as u16 == HTCLIENT as u16 {
            let role = (*(data as *const CursorState)).role;
            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_ok() && ScreenToClient(hwnd, &mut point).as_bool() {
                let resource = match client_cursor(hwnd, role, point) {
                    ClientCursor::Arrow => Some(IDC_ARROW),
                    ClientCursor::Hand => Some(IDC_HAND),
                    ClientCursor::Ibeam => Some(IDC_IBEAM),
                    ClientCursor::Native => None,
                };
                if let Some(resource) = resource
                    && let Ok(cursor) = LoadCursorW(None, resource)
                {
                    let _ = SetCursor(Some(cursor));
                    return LRESULT(1);
                }
            }
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

#[cfg(test)]
impl Drop for CursorState {
    fn drop(&mut self) {
        if let Some(dropped) = &self.dropped {
            dropped.set(true);
        }
    }
}

fn class_role(hwnd: HWND) -> Role {
    let mut name = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut name) };
    match String::from_utf16_lossy(&name[..length.max(0) as usize])
        .to_ascii_uppercase()
        .as_str()
    {
        "BUTTON" => Role::Button,
        "EDIT" => Role::Edit,
        "LISTBOX" => Role::List,
        "COMBOBOX" | "COMBOLBOX" | "SCROLLBAR" => Role::Native,
        _ => Role::Background,
    }
}

fn client_cursor(hwnd: HWND, role: Role, point: POINT) -> ClientCursor {
    unsafe {
        let mut target = hwnd;
        loop {
            if !IsWindowEnabled(target).as_bool() {
                return ClientCursor::Arrow;
            }
            // Stop at the surface; an enabled modal dialog can have a disabled
            // owner, which is different from a disabled child ancestor.
            if GetWindowLongPtrW(target, GWL_STYLE) as u32 & WS_CHILD.0 == 0 {
                break;
            }
            target = GetParent(target).unwrap_or_default();
        }
        match role {
            Role::Button
                if GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & BS_TYPEMASK as u32
                    != BS_GROUPBOX as u32 =>
            {
                ClientCursor::Hand
            }
            Role::Edit => ClientCursor::Ibeam,
            Role::Native => ClientCursor::Native,
            Role::List => {
                let packed =
                    LPARAM(((point.y as u16 as u32) << 16 | point.x as u16 as u32) as isize);
                let hit =
                    SendMessageW(hwnd, LB_ITEMFROMPOINT, Some(WPARAM(0)), Some(packed)).0 as u32;
                let index = hit & 0xffff;
                let mut rect = RECT::default();
                if hit >> 16 == 0
                    && SendMessageW(
                        hwnd,
                        LB_GETITEMRECT,
                        Some(WPARAM(index as usize)),
                        Some(LPARAM((&mut rect as *mut RECT) as isize)),
                    )
                    .0 != LB_ERR as isize
                    && point.x >= rect.left
                    && point.x < rect.right
                    && point.y >= rect.top
                    && point.y < rect.bottom
                {
                    ClientCursor::Hand
                } else {
                    ClientCursor::Arrow
                }
            }
            _ => ClientCursor::Arrow,
        }
    }
}

/// Independent of IME/scroll subclasses. Install only on the creating UI thread.
pub(super) fn install_child_cursor(hwnd: HWND) -> Result<(), String> {
    if !owned_window(hwnd) {
        return Err("Cursor subclass requires an owned UI-thread window".into());
    }
    unsafe {
        if !GetPropW(hwnd, CURSOR_PROPERTY).is_invalid() {
            return Ok(());
        }
        let state = Box::new(CursorState {
            role: class_role(hwnd),
            #[cfg(test)]
            dropped: None,
        });
        let pointer = Box::into_raw(state);
        if let Err(error) = SetPropW(hwnd, CURSOR_PROPERTY, Some(HANDLE(pointer.cast()))) {
            drop(Box::from_raw(pointer));
            return Err(error.to_string());
        }
        if !SetWindowSubclass(hwnd, Some(cursor_proc), CURSOR_SUBCLASS, pointer as usize).as_bool()
        {
            let _ = RemovePropW(hwnd, CURSOR_PROPERTY);
            drop(Box::from_raw(pointer));
            return Err("Cannot install native control cursor subclass".into());
        }
        // Editable combo children need the EDIT policy; its popup/scrollbars
        // remain entirely native. No dropdown lifetime or tracking is changed.
        if class_role(hwnd) == Role::Native {
            let mut info = COMBOBOXINFO {
                cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            if GetComboBoxInfo(hwnd, &mut info).is_ok() && class_role(info.hwndItem) == Role::Edit {
                install_child_cursor(info.hwndItem)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Window(HWND);
    impl Window {
        fn new() -> Self {
            Self(super::super::tests::hidden_test_window_size(480, 320))
        }
    }
    impl Drop for Window {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.0);
            }
        }
    }
    fn child(parent: HWND, class: PCWSTR, style: WINDOW_STYLE) -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class,
                w!("日本語"),
                WS_CHILD | style,
                0,
                0,
                240,
                180,
                Some(parent),
                None,
                None,
                None,
            )
            .unwrap()
        }
    }
    fn font_of(hwnd: HWND) -> HFONT {
        HFONT(unsafe { SendMessageW(hwnd, WM_GETFONT, None, None).0 } as *mut _)
    }
    fn font_exists(font: HFONT) -> bool {
        let mut data = LOGFONTW::default();
        (unsafe {
            GetObjectW(
                font.into(),
                std::mem::size_of::<LOGFONTW>() as i32,
                Some((&mut data as *mut LOGFONTW).cast()),
            )
        }) != 0
    }
    struct FontObservation {
        root: HWND,
        old: HFONT,
        published_before_delivery: Cell<bool>,
        old_alive_during_delivery: Cell<bool>,
    }
    unsafe extern "system" fn observe_font(
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
        _: usize,
        data: usize,
    ) -> LRESULT {
        unsafe {
            if msg == WM_SETFONT && wp.0 != 0 {
                let observation = &*(data as *const FontObservation);
                observation
                    .published_before_delivery
                    .set(GetPropW(observation.root, FONT_PROPERTY).0 as usize == wp.0);
                observation
                    .old_alive_during_delivery
                    .set(font_exists(observation.old));
            }
            DefSubclassProc(hwnd, msg, wp, lp)
        }
    }

    #[test]
    fn ui_font_replacement_publication_lifetime_and_late_children() {
        let window = Window::new();
        let edit = child(window.0, w!("EDIT"), WINDOW_STYLE(0));
        let mut font = UiFont::new(96, 14.).unwrap();
        font.apply_to(window.0).unwrap();
        assert_eq!(font_of(edit), font.handle());
        let popup = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!("owned popup"),
                WS_POPUP,
                0,
                0,
                100,
                30,
                Some(window.0),
                None,
                None,
                None,
            )
            .unwrap()
        };
        inherit_font(popup, window.0).unwrap();
        assert_eq!(font_of(popup), font.handle());
        let old = font.handle();
        let observation = FontObservation {
            root: window.0,
            old,
            published_before_delivery: Cell::new(false),
            old_alive_during_delivery: Cell::new(false),
        };
        unsafe {
            assert!(
                SetWindowSubclass(
                    edit,
                    Some(observe_font),
                    42,
                    &observation as *const _ as usize
                )
                .as_bool()
            );
        }
        font.replace_on(window.0, 144, 14.).unwrap();
        assert!(observation.published_before_delivery.get());
        assert!(observation.old_alive_during_delivery.get());
        unsafe {
            let _ = RemoveWindowSubclass(edit, Some(observe_font), 42);
        }
        assert!(!font_exists(old));
        assert_eq!(font_of(edit), font.handle());
        assert_eq!(font_of(popup), font.handle());
        let late = child(window.0, w!("BUTTON"), WINDOW_STYLE(0));
        inherit_font(late, window.0).unwrap();
        assert_eq!(font_of(late), font.handle());
        let current = font.handle();
        drop(font);
        assert_eq!(font_of(edit), HFONT::default());
        assert_eq!(font_of(late), HFONT::default());
        assert_eq!(font_of(popup), HFONT::default());
        assert!(unsafe { GetPropW(window.0, FONT_PROPERTY) }.is_invalid());
        assert!(!font_exists(current));
    }

    #[test]
    fn ui_font_nested_surface_keeps_its_font_and_destroyed_owner_is_safe() {
        assert!(UiFont::new(0, 14.).is_err());
        assert!(UiFont::new(96, f32::NAN).is_err());
        let window = Window::new();
        let panel = child(window.0, w!("STATIC"), WINDOW_STYLE(0));
        let edit = child(panel, w!("EDIT"), WINDOW_STYLE(0));
        let mut nested = UiFont::new(96, 16.).unwrap();
        nested.apply_to(panel).unwrap();
        let mut outer = UiFont::new(96, 14.).unwrap();
        outer.apply_to(window.0).unwrap();
        outer.replace_on(window.0, 144, 14.).unwrap();
        assert_eq!(font_of(edit), nested.handle());
        let mut data = LOGFONTW::default();
        unsafe {
            GetObjectW(
                outer.handle().into(),
                std::mem::size_of::<LOGFONTW>() as i32,
                Some((&mut data as *mut LOGFONTW).cast()),
            );
        }
        assert_eq!(
            String::from_utf16_lossy(&data.lfFaceName).trim_end_matches('\0'),
            shell::UI_FONT_FAMILY
        );
        let other = Window::new();
        assert!(outer.apply_to(other.0).is_err());
        drop(window);
        drop(nested);
        drop(outer);
    }

    #[test]
    fn child_cursors_distinguish_disabled_empty_rows_and_native_combo_parts() {
        let window = Window::new();
        let button = child(window.0, w!("BUTTON"), WINDOW_STYLE(0));
        let edit = child(window.0, w!("EDIT"), WINDOW_STYLE(0));
        let list = child(window.0, w!("LISTBOX"), WS_VSCROLL);
        let point = POINT { x: 5, y: 5 };
        assert_eq!(
            client_cursor(button, class_role(button), point),
            ClientCursor::Hand
        );
        assert_eq!(
            client_cursor(edit, class_role(edit), point),
            ClientCursor::Ibeam
        );
        unsafe {
            let _ = EnableWindow(button, false);
        }
        assert_eq!(
            client_cursor(button, class_role(button), point),
            ClientCursor::Arrow
        );
        unsafe {
            let _ = EnableWindow(window.0, false);
        }
        assert_eq!(
            client_cursor(edit, class_role(edit), point),
            ClientCursor::Arrow
        );
        unsafe {
            let _ = EnableWindow(window.0, true);
        }
        assert_eq!(
            client_cursor(list, class_role(list), point),
            ClientCursor::Arrow
        );
        unsafe {
            let _ = SendMessageW(
                list,
                LB_ADDSTRING,
                Some(WPARAM(0)),
                Some(LPARAM(w!("項目").as_ptr() as isize)),
            );
        }
        assert_eq!(
            client_cursor(list, class_role(list), point),
            ClientCursor::Hand
        );
        assert_eq!(
            client_cursor(list, class_role(list), POINT { x: 5, y: 140 }),
            ClientCursor::Arrow
        );
        unsafe {
            let _ = EnableWindow(list, false);
        }
        assert_eq!(
            client_cursor(list, class_role(list), point),
            ClientCursor::Arrow
        );
        let combo = child(
            window.0,
            w!("COMBOBOX"),
            WINDOW_STYLE(CBS_DROPDOWN as u32) | WS_VSCROLL,
        );
        install_child_cursor(combo).unwrap();
        assert_eq!(
            client_cursor(combo, class_role(combo), point),
            ClientCursor::Native
        );
        let mut info = COMBOBOXINFO {
            cbSize: std::mem::size_of::<COMBOBOXINFO>() as u32,
            ..Default::default()
        };
        unsafe {
            GetComboBoxInfo(combo, &mut info).unwrap();
        }
        assert_eq!(class_role(info.hwndItem), Role::Edit);
        assert_eq!(
            client_cursor(info.hwndItem, class_role(info.hwndItem), point),
            ClientCursor::Ibeam
        );
        assert!(!unsafe { GetPropW(info.hwndItem, CURSOR_PROPERTY) }.is_invalid());
    }

    #[test]
    fn child_cursor_subclass_is_idempotent_and_releases_state_on_destroy() {
        let window = Window::new();
        let button = child(window.0, w!("BUTTON"), WINDOW_STYLE(0));
        unsafe extern "system" fn next_proc(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            _: usize,
        ) -> LRESULT {
            if msg == WM_SETCURSOR {
                LRESULT(123)
            } else {
                unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
            }
        }
        unsafe {
            assert!(SetWindowSubclass(button, Some(next_proc), 77, 0).as_bool());
        }
        install_child_cursor(button).unwrap();
        unsafe {
            assert_eq!(
                SendMessageW(
                    button,
                    WM_SETCURSOR,
                    Some(WPARAM(button.0 as usize)),
                    Some(LPARAM(HTVSCROLL as isize))
                )
                .0,
                123
            );
            assert_eq!(
                SendMessageW(
                    button,
                    WM_SETCURSOR,
                    Some(WPARAM(window.0.0 as usize)),
                    Some(LPARAM(HTCLIENT as isize))
                )
                .0,
                123
            );
        }
        let first = unsafe { GetPropW(button, CURSOR_PROPERTY) }.0 as usize;
        assert_ne!(first, 0);
        install_child_cursor(button).unwrap();
        let second = unsafe { GetPropW(button, CURSOR_PROPERTY) }.0 as usize;
        assert_eq!(first, second);
        let dropped = Rc::new(Cell::new(false));
        unsafe {
            (*(first as *mut CursorState)).dropped = Some(Rc::clone(&dropped));
            DestroyWindow(button).unwrap();
        }
        assert!(dropped.get());
        assert!(install_child_cursor(HWND::default()).is_err());
    }
}
