//! Native small icons for the Explorer listbox's extension-based owner draw.

use super::{shell, wide};
use std::path::Path;
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, RECT, WPARAM},
        Graphics::Gdi::{
            CreateSolidBrush, DEFAULT_GUI_FONT, DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE,
            DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, FillRect, GetStockObject, HGDIOBJ,
            SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
        },
        Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL},
        UI::{
            Controls::{DRAWITEMSTRUCT, ODS_FOCUS, ODS_NOFOCUSRECT, ODS_SELECTED},
            HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi},
            Shell::{
                SHFILEINFOW, SHGFI_ICON, SHGFI_OPENICON, SHGFI_SMALLICON, SHGFI_USEFILEATTRIBUTES,
                SHGetFileInfoW,
            },
            WindowsAndMessaging::{
                DI_NORMAL, DestroyIcon, DrawIconEx, LB_GETTEXT, LB_GETTEXTLEN, SM_CXSMICON,
                SM_CYSMICON, SendMessageW, WM_GETFONT,
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
    icons: Vec<(IconClass, windows::Win32::UI::WindowsAndMessaging::HICON)>,
}

impl Default for ExplorerIcons {
    fn default() -> Self {
        Self {
            icons: Vec::with_capacity(MAX_ICON_CACHE),
        }
    }
}

impl ExplorerIcons {
    fn icon(
        &mut self,
        class: &IconClass,
    ) -> Option<windows::Win32::UI::WindowsAndMessaging::HICON> {
        if let Some((_, icon)) = self.icons.iter().find(|(cached, _)| cached == class) {
            return Some(*icon);
        }

        let icon = unsafe { shell_icon(class) }?;
        if self.icons.len() == MAX_ICON_CACHE {
            let (_, oldest) = self.icons.remove(0);
            unsafe {
                let _ = DestroyIcon(oldest);
            }
        }
        self.icons.push((class.clone(), icon));
        Some(icon)
    }

    /// Draws Explorer rows for control 4690 and returns whether it handled the item.
    pub(super) fn draw(&mut self, item: &DRAWITEMSTRUCT, tokens: shell::Tokens) -> bool {
        if item.CtlID as usize != EXPLORER_LIST_ID {
            return false;
        }

        unsafe {
            let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
            let background = if selected {
                tokens.selected
            } else {
                tokens.panel
            };
            let brush = CreateSolidBrush(shell::rgb(background));
            let _ = FillRect(item.hDC, &item.rcItem, brush);
            let _ = DeleteObject(brush.into());

            let text = if item.itemID == u32::MAX {
                None
            } else {
                list_item_text(item.hwndItem, item.itemID)
            };
            if let Some(text) = text {
                let (depth, directory, expanded) = row_metadata(item.itemData);
                let label = row_label(&text, directory);
                let dpi = GetDpiForWindow(item.hwndItem).max(1);
                let icon_width = GetSystemMetricsForDpi(SM_CXSMICON, dpi).max(scale_dip(16, dpi));
                let icon_height = GetSystemMetricsForDpi(SM_CYSMICON, dpi).max(scale_dip(16, dpi));
                let indent_dip = depth
                    .saturating_mul(TREE_INDENT_DIP)
                    .min(MAX_TREE_INDENT_DIP) as i32;
                let padding = scale_dip(ROW_PADDING_DIP + indent_dip, dpi);
                let icon_gap = scale_dip(ICON_GAP_DIP, dpi);
                let icon_x = item.rcItem.left + padding;
                let icon_y = item.rcItem.top
                    + ((item.rcItem.bottom - item.rcItem.top - icon_height) / 2).max(0);
                let text_left = icon_x + icon_width + icon_gap;

                if !label.is_empty()
                    && let Some(icon) = self.icon(&icon_class(label, directory, expanded))
                {
                    let _ = DrawIconEx(
                        item.hDC,
                        icon_x,
                        icon_y,
                        icon,
                        icon_width,
                        icon_height,
                        0,
                        None,
                        DI_NORMAL,
                    );
                }

                let font = SendMessageW(item.hwndItem, WM_GETFONT, None, None).0;
                let font = if font == 0 {
                    GetStockObject(DEFAULT_GUI_FONT)
                } else {
                    HGDIOBJ(font as *mut core::ffi::c_void)
                };
                let old_font = SelectObject(item.hDC, font);
                let _ = SetBkMode(item.hDC, TRANSPARENT);
                SetTextColor(item.hDC, shell::rgb(tokens.foreground));
                if text_left < item.rcItem.right {
                    let mut text_rect = RECT {
                        left: text_left,
                        top: item.rcItem.top,
                        right: item.rcItem.right - padding,
                        bottom: item.rcItem.bottom,
                    };
                    let mut display = wide(label);
                    let _ = DrawTextW(
                        item.hDC,
                        &mut display,
                        &mut text_rect,
                        DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
                    );
                }
                if !old_font.0.is_null() && old_font.0 as isize != -1 {
                    let _ = SelectObject(item.hDC, old_font);
                }
            }

            if item.itemState.0 & ODS_FOCUS.0 != 0 && item.itemState.0 & ODS_NOFOCUSRECT.0 == 0 {
                let _ = DrawFocusRect(item.hDC, &item.rcItem);
            }
        }
        true
    }
}

impl Drop for ExplorerIcons {
    fn drop(&mut self) {
        for (_, icon) in self.icons.drain(..) {
            unsafe {
                let _ = DestroyIcon(icon);
            }
        }
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
    use super::{IconClass, icon_class, row_label, row_metadata};

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
