//! Native small icons for the Explorer listbox's extension-based owner draw.

use super::{draw_gdi_text, shell, wide};
use std::{path::Path, rc::Rc};
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, SIZE, WPARAM},
        Graphics::Gdi::{
            CreateFontIndirectW, CreateSolidBrush, DEFAULT_GUI_FONT, DT_NOPREFIX, DT_SINGLELINE,
            DT_VCENTER, DeleteObject, FillRect, GetDC, GetObjectW, GetStockObject,
            GetTextExtentPoint32W, HDC, HFONT, HGDIOBJ, IntersectClipRect, LOGFONTW, ReleaseDC,
            RestoreDC, SaveDC, SelectObject, SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
        },
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        UI::{
            Controls::{DRAWITEMSTRUCT, ODS_SELECTED, ODT_LISTBOX},
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{
                SHFILEINFOW, SHGFI_ICON, SHGFI_OPENICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
                SHGetFileInfoW,
            },
            WindowsAndMessaging::{
                DI_NORMAL, DestroyIcon, DrawIconEx, GetClientRect, GetParent, HICON, IsWindow,
                LB_GETCOUNT, LB_GETITEMDATA, LB_GETTEXT, LB_GETTEXTLEN, SM_CXSMICON, SM_CYSMICON,
                SendMessageW, WM_GETFONT,
            },
        },
    },
    core::PCWSTR,
};

const EXPLORER_LIST_ID: usize = 4690;
const MAX_ICON_CACHE: usize = 64;
const ROW_PADDING_DIP: i32 = 5;
const ICON_GAP_DIP: i32 = 6;
const TREE_INDENT_DIP: usize = 12;
const MAX_TREE_INDENT_DIP: usize = 128;
const ITEM_IS_DIRECTORY: usize = 1;
const ITEM_IS_EXPANDED: usize = 2;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum IconClass {
    Directory { expanded: bool },
    File,
    Extension(String),
}

fn row_metadata(data: usize) -> (usize, bool, bool) {
    (
        data >> 8,
        data & ITEM_IS_DIRECTORY != 0,
        data & ITEM_IS_EXPANDED != 0,
    )
}

fn row_label(row: &str, directory: bool) -> &str {
    if directory {
        row.strip_prefix("▸ ")
            .or_else(|| row.strip_prefix("▾ "))
            .unwrap_or(row)
    } else {
        row.strip_prefix("  ").unwrap_or(row)
    }
}

fn icon_class(label: &str, directory: bool, expanded: bool) -> IconClass {
    if directory {
        IconClass::Directory { expanded }
    } else {
        Path::new(label)
            .extension()
            .filter(|extension| !extension.is_empty())
            .map(|extension| IconClass::Extension(extension.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or(IconClass::File)
    }
}

pub(super) struct ExplorerIcons {
    icons: Vec<(IconClass, Rc<OwnedIcon>)>,
    generation: u64,
}

struct OwnedIcon(HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyIcon(self.0);
        }
    }
}
struct OwnedFont(HFONT);
impl Drop for OwnedFont {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

/// Immutable resources for WM_DRAWITEM while the application is borrowed.
/// Publish/clone an Rc of this value outside any RefCell borrow before painting.
/// Rebuild after list HWND, assigned font, DPI, or theme changes.
pub(super) struct ExplorerPaint {
    hwnd: HWND,
    owner: HWND,
    dpi: u32,
    font: OwnedFont,
    tokens: shell::Tokens,
    icons: Vec<(IconClass, Rc<OwnedIcon>)>,
}

struct SavedDc(HDC, i32);
impl SavedDc {
    unsafe fn new(dc: HDC) -> Option<Self> {
        let saved = unsafe { SaveDC(dc) };
        (saved != 0).then(|| Self(dc, saved))
    }
}
impl Drop for SavedDc {
    fn drop(&mut self) {
        unsafe {
            let _ = RestoreDC(self.0, self.1);
        }
    }
}

struct RowGeometry {
    padding: i32,
    right_padding: i32,
    icon_width: i32,
    icon_height: i32,
    gap: i32,
}
impl RowGeometry {
    fn new(depth: usize, dpi: u32) -> Self {
        let indent = depth
            .saturating_mul(TREE_INDENT_DIP)
            .min(MAX_TREE_INDENT_DIP) as i32;
        Self {
            padding: scale_dip(ROW_PADDING_DIP + indent, dpi),
            right_padding: scale_dip(ROW_PADDING_DIP, dpi),
            icon_width: unsafe { GetSystemMetricsForDpi(SM_CXSMICON, dpi) }.max(scale_dip(16, dpi)),
            icon_height: unsafe { GetSystemMetricsForDpi(SM_CYSMICON, dpi) }
                .max(scale_dip(16, dpi)),
            gap: scale_dip(ICON_GAP_DIP, dpi),
        }
    }
    fn extent(&self, text_width: i32) -> i32 {
        pixel_i32(
            i64::from(self.padding)
                + i64::from(self.right_padding)
                + i64::from(self.icon_width)
                + i64::from(self.gap)
                + i64::from(text_width.max(0)),
        )
    }
}

fn pixel_i32(value: i64) -> i32 {
    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}
fn window_dpi(hwnd: HWND) -> u32 {
    match unsafe { GetDpiForWindow(hwnd) } {
        0 => 96,
        dpi => dpi,
    }
}

unsafe fn assigned_font(hwnd: HWND) -> HGDIOBJ {
    unsafe {
        let font = SendMessageW(hwnd, WM_GETFONT, None, None).0;
        if font == 0 {
            GetStockObject(DEFAULT_GUI_FONT)
        } else {
            HGDIOBJ(font as *mut core::ffi::c_void)
        }
    }
}

unsafe fn text_width(dc: HDC, label: &str) -> Option<i32> {
    let text: Vec<_> = label.encode_utf16().collect();
    if text.is_empty() {
        return Some(0);
    }
    let mut size = SIZE::default();
    unsafe {
        GetTextExtentPoint32W(dc, &text, &mut size)
            .as_bool()
            .then_some(size.cx.max(0))
    }
}

impl Default for ExplorerIcons {
    fn default() -> Self {
        Self {
            icons: Vec::with_capacity(MAX_ICON_CACHE),
            generation: 0,
        }
    }
}

impl ExplorerIcons {
    pub(super) const fn generation(&self) -> u64 {
        self.generation
    }
    fn icon(&mut self, class: &IconClass) -> Option<Rc<OwnedIcon>> {
        if let Some((_, icon)) = self.icons.iter().find(|(cached, _)| cached == class) {
            return Some(Rc::clone(icon));
        }

        let icon = Rc::new(OwnedIcon(unsafe { shell_icon(class) }?));
        if self.icons.len() == MAX_ICON_CACHE {
            self.icons.remove(0);
        }
        self.icons.push((class.clone(), Rc::clone(&icon)));
        self.generation = self.generation.wrapping_add(1);
        Some(icon)
    }

    /// Draws Explorer rows for control 4690 and returns whether it handled the item.
    pub(super) fn draw_offset(
        &mut self,
        item: &DRAWITEMSTRUCT,
        tokens: shell::Tokens,
        offset: i32,
    ) -> bool {
        if item.CtlID as usize != EXPLORER_LIST_ID || item.CtlType != ODT_LISTBOX {
            return false;
        }
        let row = unsafe { current_row(item) };
        let icon = row.as_ref().and_then(|(text, data)| {
            let (_, directory, expanded) = row_metadata(*data);
            self.icon(&icon_class(row_label(text, directory), directory, expanded))
        });
        unsafe {
            draw_row(
                item,
                tokens,
                offset,
                assigned_font(item.hwndItem),
                window_dpi(item.hwndItem),
                row.as_ref(),
                icon.as_ref().map(|icon| icon.0),
            )
        }
    }

    /// Full label width, including the same padding, tree indent and icon as draw_offset.
    pub(super) fn measure_extent(&self, hwnd: HWND) -> Option<i32> {
        unsafe { measure_extent(hwnd, assigned_font(hwnd), window_dpi(hwnd)) }
    }

    /// Capture cached icons and a generic fallback; never preload every row.
    pub(super) fn paint_snapshot(
        &mut self,
        hwnd: HWND,
        tokens: shell::Tokens,
    ) -> Option<ExplorerPaint> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return None;
            }
            let mut data = LOGFONTW::default();
            if GetObjectW(
                assigned_font(hwnd),
                std::mem::size_of::<LOGFONTW>() as i32,
                Some((&mut data as *mut LOGFONTW).cast()),
            ) == 0
            {
                return None;
            }
            let font = OwnedFont(CreateFontIndirectW(&data));
            if font.0 == HFONT::default() {
                return None;
            }
            let _ = self.icon(&IconClass::File);
            Some(ExplorerPaint {
                hwnd,
                owner: GetParent(hwnd).ok()?,
                dpi: window_dpi(hwnd),
                font,
                tokens,
                icons: self.icons.clone(),
            })
        }
    }
}

impl ExplorerPaint {
    pub(super) fn draw(&self, item: &DRAWITEMSTRUCT, offset: i32) -> bool {
        unsafe {
            if item.hwndItem != self.hwnd
                || item.CtlID as usize != EXPLORER_LIST_ID
                || item.CtlType != ODT_LISTBOX
                || !IsWindow(Some(self.hwnd)).as_bool()
                || GetParent(self.hwnd).ok() != Some(self.owner)
            {
                return false;
            }
            let row = current_row(item);
            let icon = row.as_ref().and_then(|(text, data)| {
                let (_, directory, expanded) = row_metadata(*data);
                let class = icon_class(row_label(text, directory), directory, expanded);
                self.icons
                    .iter()
                    .find(|(cached, _)| cached == &class)
                    .or_else(|| {
                        self.icons
                            .iter()
                            .find(|(cached, _)| cached == &IconClass::File)
                    })
                    .map(|(_, icon)| icon.0)
            });
            draw_row(
                item,
                self.tokens,
                offset,
                self.font.0.into(),
                self.dpi,
                row.as_ref(),
                icon,
            )
        }
    }
}

unsafe fn current_row(item: &DRAWITEMSTRUCT) -> Option<(String, usize)> {
    if item.itemID == u32::MAX {
        return None;
    }
    let row = unsafe { list_item(item.hwndItem, item.itemID) }?;
    (row.1 == item.itemData).then_some(row)
}

unsafe fn list_item(hwnd: HWND, item: u32) -> Option<(String, usize)> {
    let text = unsafe { list_item_text(hwnd, item) }?;
    let data = unsafe { shell::send(hwnd, LB_GETITEMDATA, WPARAM(item as usize), LPARAM(0)).0 };
    (data != -1).then_some((text, data as usize))
}

unsafe fn measure_extent(hwnd: HWND, font: HGDIOBJ, dpi: u32) -> Option<i32> {
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return None;
        }
        let dc = GetDC(Some(hwnd));
        if dc == HDC::default() {
            return None;
        }
        let result = (|| {
            let _state = SavedDc::new(dc)?;
            SelectObject(dc, font);
            let count =
                usize::try_from(shell::send(hwnd, LB_GETCOUNT, WPARAM(0), LPARAM(0)).0).ok()?;
            let mut width = 0;
            for item in 0..count {
                if let Some((text, data)) = list_item(hwnd, item as u32) {
                    let (depth, directory, _) = row_metadata(data);
                    let geometry = RowGeometry::new(depth, dpi);
                    width =
                        width.max(geometry.extent(text_width(dc, row_label(&text, directory))?));
                }
            }
            Some(width)
        })();
        ReleaseDC(Some(hwnd), dc);
        result
    }
}

unsafe fn draw_row(
    item: &DRAWITEMSTRUCT,
    tokens: shell::Tokens,
    offset: i32,
    font: HGDIOBJ,
    dpi: u32,
    row: Option<&(String, usize)>,
    icon: Option<HICON>,
) -> bool {
    unsafe {
        let Some(_state) = SavedDc::new(item.hDC) else {
            return false;
        };
        let mut client = RECT::default();
        if GetClientRect(item.hwndItem, &mut client).is_err() {
            return false;
        }
        let viewport = RECT {
            left: client.left,
            right: client.right,
            top: item.rcItem.top,
            bottom: item.rcItem.bottom,
        };
        IntersectClipRect(
            item.hDC,
            client.left,
            client.top,
            client.right,
            client.bottom,
        );
        IntersectClipRect(
            item.hDC,
            viewport.left,
            viewport.top,
            viewport.right,
            viewport.bottom,
        );
        let background = if row.is_some() && item.itemState.0 & ODS_SELECTED.0 != 0 {
            tokens.selected
        } else {
            tokens.panel
        };
        let brush = CreateSolidBrush(shell::rgb(background));
        FillRect(item.hDC, &viewport, brush);
        let _ = DeleteObject(brush.into());
        SelectObject(item.hDC, font);
        SetBkMode(item.hDC, TRANSPARENT);
        SetBkColor(item.hDC, shell::rgb(background));
        SetTextColor(item.hDC, shell::rgb(tokens.foreground));
        if let Some((text, data)) = row {
            let (depth, directory, _) = row_metadata(*data);
            let label = row_label(text, directory);
            let geometry = RowGeometry::new(depth, dpi);
            let icon_x = pixel_i32(
                i64::from(viewport.left) + i64::from(geometry.padding) - i64::from(offset.max(0)),
            );
            let icon_y = pixel_i32(
                i64::from(viewport.top)
                    + ((i64::from(viewport.bottom)
                        - i64::from(viewport.top)
                        - i64::from(geometry.icon_height))
                        / 2)
                    .max(0),
            );
            if !label.is_empty()
                && let Some(icon) = icon
            {
                let _ = DrawIconEx(
                    item.hDC,
                    icon_x,
                    icon_y,
                    icon,
                    geometry.icon_width,
                    geometry.icon_height,
                    0,
                    None,
                    DI_NORMAL,
                );
            }
            if let Some(width) = text_width(item.hDC, label) {
                let left = pixel_i32(
                    i64::from(icon_x) + i64::from(geometry.icon_width) + i64::from(geometry.gap),
                );
                let mut text_rect = RECT {
                    left,
                    right: pixel_i32(i64::from(left) + i64::from(width)),
                    top: viewport.top,
                    bottom: viewport.bottom,
                };
                let _ = draw_gdi_text(
                    item.hDC,
                    label,
                    &mut text_rect,
                    DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX,
                );
            }
        }
        super::control_style::draw_focus(tokens, item, &viewport, row.is_some());
        true
    }
}

unsafe fn list_item_text(hwnd: HWND, item: u32) -> Option<String> {
    if hwnd == HWND::default() {
        return None;
    }
    let index = WPARAM(item as usize);
    let length = unsafe { shell::send(hwnd, LB_GETTEXTLEN, index, LPARAM(0)).0 };
    let length = usize::try_from(length).ok()?;
    let mut text = vec![0u16; length + 1];
    let copied =
        unsafe { shell::send(hwnd, LB_GETTEXT, index, LPARAM(text.as_mut_ptr() as isize)).0 };
    let copied = usize::try_from(copied).ok()?.min(length);
    Some(String::from_utf16_lossy(&text[..copied]))
}

fn scale_dip(value: i32, dpi: u32) -> i32 {
    ((value as f32 * dpi as f32 / 96.).round() as i32).max(1)
}

unsafe fn shell_icon(class: &IconClass) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
    let (sample, attributes, open) = match class {
        IconClass::Directory { expanded } => {
            ("folder".to_owned(), FILE_ATTRIBUTE_DIRECTORY, *expanded)
        }
        IconClass::File => ("file".to_owned(), FILE_ATTRIBUTE_NORMAL, false),
        IconClass::Extension(extension) => {
            (format!("file.{extension}"), FILE_ATTRIBUTE_NORMAL, false)
        }
    };
    let sample = wide(&sample);
    let mut info = SHFILEINFOW::default();
    let flags = SHGFI_ICON | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES;
    let flags = if open { flags | SHGFI_OPENICON } else { flags };
    let result = unsafe {
        SHGetFileInfoW(
            PCWSTR(sample.as_ptr()),
            attributes,
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            flags,
        )
    };
    (result != 0 && !info.hIcon.0.is_null()).then_some(info.hIcon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{
        Win32::{
            Foundation::COLORREF,
            Graphics::Gdi::{
                CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, GetBkMode, GetClipBox,
                GetCurrentObject, GetPixel, GetTextColor, HBITMAP, OBJ_FONT, OPAQUE, SetBkColor,
                SetPixel,
            },
            UI::{
                Controls::ODS_SELECTED,
                WindowsAndMessaging::{
                    CreateWindowExW, DestroyWindow, GetIconInfo, ICONINFO, LB_ADDSTRING,
                    LB_RESETCONTENT, LB_SETITEMDATA, LBS_EXTENDEDSEL, LBS_HASSTRINGS,
                    LBS_NOINTEGRALHEIGHT, LBS_OWNERDRAWFIXED, SWP_NOACTIVATE, SWP_NOMOVE,
                    SWP_NOZORDER, SetWindowPos, WINDOW_EX_STYLE, WINDOW_STYLE, WM_SETFONT,
                    WS_CHILD,
                },
            },
        },
        core::w,
    };

    struct List {
        owner: HWND,
        hwnd: HWND,
        font: HFONT,
    }
    impl List {
        fn new(text: &str, data: usize, width: i32) -> Self {
            unsafe {
                let owner = super::super::tests::hidden_test_window_size(1024, 64);
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("LISTBOX"),
                    w!(""),
                    WS_CHILD
                        | WINDOW_STYLE(
                            (LBS_OWNERDRAWFIXED
                                | LBS_HASSTRINGS
                                | LBS_EXTENDEDSEL
                                | LBS_NOINTEGRALHEIGHT) as u32,
                        ),
                    0,
                    0,
                    width,
                    32,
                    Some(owner),
                    None,
                    None,
                    None,
                )
                .unwrap();
                let mut font_data = LOGFONTW {
                    lfHeight: -18,
                    ..Default::default()
                };
                let face = wide("Consolas");
                font_data.lfFaceName[..face.len()].copy_from_slice(&face);
                let font = CreateFontIndirectW(&font_data);
                shell::send(hwnd, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(0));
                shell::send(
                    hwnd,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(wide(text).as_ptr() as isize),
                );
                shell::send(hwnd, LB_SETITEMDATA, WPARAM(0), LPARAM(data as isize));
                Self { owner, hwnd, font }
            }
        }
        fn item(&self, dc: HDC, data: usize) -> DRAWITEMSTRUCT {
            DRAWITEMSTRUCT {
                CtlType: ODT_LISTBOX,
                CtlID: EXPLORER_LIST_ID as u32,
                itemID: 0,
                hwndItem: self.hwnd,
                hDC: dc,
                itemData: data,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 1024,
                    bottom: 32,
                },
                ..Default::default()
            }
        }
    }
    impl Drop for List {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.owner).unwrap();
                let _ = DeleteObject(self.font.into());
            }
        }
    }
    struct Surface(HDC, HBITMAP, HGDIOBJ);
    impl Surface {
        fn new(hwnd: HWND) -> Self {
            unsafe {
                let source = GetDC(Some(hwnd));
                let dc = CreateCompatibleDC(Some(source));
                let bitmap = CreateCompatibleBitmap(source, 1024, 64);
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

    #[test]
    fn explorer_horizontal_render_matches_full_content_crop_and_restores_dc() {
        let list = List::new(
            &format!("  {}終端.md", "日本語_long_name_".repeat(12)),
            2 << 8,
            1024,
        );
        let full = Surface::new(list.hwnd);
        let clipped = Surface::new(list.hwnd);
        let mut icons = ExplorerIcons::default();
        let mut item = list.item(full.0, 2 << 8);
        item.itemState = ODS_SELECTED;
        assert!(icons.draw_offset(&item, shell::Tokens::default(), 0));
        unsafe {
            SetWindowPos(
                list.hwnd,
                None,
                0,
                0,
                160,
                32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            SetTextColor(clipped.0, COLORREF(0x123456));
            SetBkColor(clipped.0, COLORREF(0x654321));
            SetBkMode(clipped.0, OPAQUE);
            SetPixel(clipped.0, 200, 10, COLORREF(0xabcdef));
            let font = GetCurrentObject(clipped.0, OBJ_FONT);
            let mut before_clip = RECT::default();
            GetClipBox(clipped.0, &mut before_clip);
            item.hDC = clipped.0;
            assert!(icons.draw_offset(&item, shell::Tokens::default(), 100));
            assert_eq!(GetTextColor(clipped.0), COLORREF(0x123456));
            assert_eq!(GetBkMode(clipped.0), 2);
            assert_eq!(GetCurrentObject(clipped.0, OBJ_FONT), font);
            assert_eq!(
                GetPixel(clipped.0, 200, 10),
                COLORREF(0xabcdef),
                "drawing must stay inside the native client clip"
            );
            let mut after_clip = RECT::default();
            GetClipBox(clipped.0, &mut after_clip);
            assert_eq!(before_clip, after_clip);
            assert!(
                (0..32)
                    .all(|y| (0..160)
                        .all(|x| GetPixel(clipped.0, x, y) == GetPixel(full.0, x + 100, y))),
                "translated content must equal a crop of its full label, without ellipsis"
            );
            assert!(icons.measure_extent(list.hwnd).unwrap() > 1024);
            item.itemID = u32::MAX;
            item.itemState.0 = windows::Win32::UI::Controls::ODS_FOCUS.0 | ODS_SELECTED.0;
            item.hDC = full.0;
            assert!(icons.draw_offset(&item, shell::Tokens::default(), 0));
            item.hDC = clipped.0;
            assert!(icons.draw_offset(&item, shell::Tokens::default(), 100));
            assert!(
                (0..32)
                    .all(|y| (0..160).all(|x| GetPixel(clipped.0, x, y) == GetPixel(full.0, x, y))),
                "focus and background must stay in viewport coordinates"
            );
            assert!(
                (0..32).all(|y| (0..160).all(|x| {
                    GetPixel(clipped.0, x, y) == shell::rgb(shell::Tokens::default().panel)
                })),
                "an empty list has no selected row or focus rectangle"
            );
        }
    }

    #[test]
    fn explorer_extent_uses_assigned_font_and_saturates_coordinates() {
        let list = List::new("  WWWWWW日本語.md", 0, 160);
        let icons = ExplorerIcons::default();
        let normal = icons.measure_extent(list.hwnd).unwrap();
        unsafe {
            let mut data = LOGFONTW {
                lfHeight: -36,
                ..Default::default()
            };
            let face = wide("Consolas");
            data.lfFaceName[..face.len()].copy_from_slice(&face);
            let large = OwnedFont(CreateFontIndirectW(&data));
            shell::send(list.hwnd, WM_SETFONT, WPARAM(large.0.0 as usize), LPARAM(0));
            assert!(icons.measure_extent(list.hwnd).unwrap() > normal);
            shell::send(list.hwnd, LB_SETITEMDATA, WPARAM(0), LPARAM(3 << 8));
            let indented = icons.measure_extent(list.hwnd).unwrap();
            shell::send(list.hwnd, LB_SETITEMDATA, WPARAM(0), LPARAM(0));
            assert_eq!(
                indented - icons.measure_extent(list.hwnd).unwrap(),
                scale_dip(36, window_dpi(list.hwnd))
            );
            shell::send(
                list.hwnd,
                WM_SETFONT,
                WPARAM(list.font.0 as usize),
                LPARAM(0),
            );
        }
        assert_eq!(RowGeometry::new(usize::MAX, 96).extent(i32::MAX), i32::MAX);
        assert_eq!(pixel_i32(i64::MIN), i32::MIN);
        assert_eq!(pixel_i32(i64::MAX), i32::MAX);
    }

    #[test]
    fn explorer_snapshot_capture_does_not_preload_unbounded_row_classes() {
        let list = List::new("  initial.md", 0, 160);
        unsafe {
            for index in 0..MAX_ICON_CACHE + 16 {
                let text = wide(&format!("  row.mdlite_probe_{index}"));
                shell::send(
                    list.hwnd,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(text.as_ptr() as isize),
                );
            }
        }
        let mut icons = ExplorerIcons::default();
        assert!(icons.icon(&IconClass::File).is_some());
        let generation = icons.generation();
        for _ in 0..2 {
            let snapshot = icons
                .paint_snapshot(list.hwnd, shell::Tokens::default())
                .unwrap();
            assert!(!snapshot.icons.is_empty());
            assert_eq!(
                icons.generation(),
                generation,
                "capture must not churn the icon cache for offscreen rows"
            );
        }
    }

    #[test]
    fn explorer_snapshot_owns_resources_and_handles_reset_without_application_borrow() {
        use std::cell::RefCell;
        let mut list = List::new("  snapshot_日本語.md", 0, 160);
        let surface = Surface::new(list.hwnd);
        let icons = RefCell::new(ExplorerIcons::default());
        let snapshot = icons
            .borrow_mut()
            .paint_snapshot(list.hwnd, shell::Tokens::default())
            .unwrap();
        assert!(
            !snapshot.icons.is_empty(),
            "native icon resource fixture must be present"
        );
        let item = list.item(surface.0, 0);
        unsafe {
            shell::send(list.hwnd, WM_SETFONT, WPARAM(0), LPARAM(0));
            assert!(DeleteObject(list.font.into()).as_bool());
            list.font = HFONT::default();
        }
        {
            let mut held = icons.borrow_mut();
            held.icons.clear();
            assert!(snapshot.draw(&item, 40));
        }
        drop(icons);
        unsafe {
            let mut font_data = LOGFONTW::default();
            assert_ne!(
                GetObjectW(
                    snapshot.font.0.into(),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut font_data as *mut LOGFONTW).cast())
                ),
                0
            );
            assert_eq!(font_data.lfHeight, -18);
            for (_, icon) in &snapshot.icons {
                let mut data = ICONINFO::default();
                GetIconInfo(icon.0, &mut data).unwrap();
                let _ = DeleteObject(data.hbmColor.into());
                let _ = DeleteObject(data.hbmMask.into());
            }
            shell::send(list.hwnd, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            assert!(snapshot.draw(&item, i32::MAX));
            assert!((0..32).all(|y| {
                (0..160).all(|x| {
                    GetPixel(surface.0, x, y) == shell::rgb(shell::Tokens::default().panel)
                })
            }));
            let mut foreign = item;
            foreign.CtlID += 1;
            assert!(!snapshot.draw(&foreign, 0));
            drop(list);
            assert!(!snapshot.draw(&item, 0));
        }
    }

    #[test]
    fn explorer_tree_rows_select_directory_and_extension_classes() {
        let data = (2 << 8) | super::ITEM_IS_DIRECTORY | super::ITEM_IS_EXPANDED;
        let (depth, directory, expanded) = row_metadata(data);
        assert_eq!(depth, 2);
        assert!(directory && expanded);
        assert_eq!(
            icon_class(row_label("▾ docs", directory), directory, expanded),
            IconClass::Directory { expanded: true }
        );

        let (_, directory, expanded) = row_metadata(0);
        let label = row_label("  README.MD", directory);
        assert!(!directory);
        assert_eq!(
            icon_class(label, directory, expanded),
            IconClass::Extension("md".to_owned())
        );

        let label = row_label("  ▸notes.txt", directory);
        assert_eq!(
            icon_class(label, directory, expanded),
            IconClass::Extension("txt".to_owned())
        );
        assert_eq!(icon_class("LICENSE", false, false), IconClass::File);
    }
}
