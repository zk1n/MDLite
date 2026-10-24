//! A single non-activating Win32 surface for local link preview content.
//!
//! The owner window retains keyboard focus and all editor state. This child
//! tool window only paints immutable preview data and never launches links.

use super::shell;
use mdlite::link_preview::{PreviewContent, PreviewFallback};
use std::time::Duration;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Controls::{HOVER_DEFAULT, WM_MOUSELEAVE},
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent},
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

const POPUP_WIDTH_DIP: i32 = 460;
const TEXT_POPUP_HEIGHT_DIP: i32 = 330;
const IMAGE_HEADER_DIP: i32 = 44;
const IMAGE_FOOTER_DIP: i32 = 28;

struct PopupState {
    content: Option<PreviewContent>,
    tokens: Option<shell::Tokens>,
}

pub(super) struct Popup {
    hwnd: HWND,
    owner: HWND,
    state: Box<PopupState>,
}

impl Popup {
    pub(super) fn new(owner: HWND) -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(popup_window_proc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW)?,
                lpszClassName: w!("MDLiteLinkPreviewPopup"),
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            let mut state = Box::new(PopupState {
                content: None,
                tokens: None,
            });
            let hwnd = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.lpszClassName,
                w!("MDLite link preview"),
                WS_POPUP | WS_BORDER,
                0,
                0,
                1,
                1,
                Some(owner),
                None,
                Some(instance.into()),
                Some((&mut *state as *mut PopupState).cast()),
            )?;
            Ok(Self { hwnd, owner, state })
        }
    }

    pub(super) fn show_at(
        &mut self,
        cursor: POINT,
        mut content: PreviewContent,
        tokens: shell::Tokens,
    ) {
        composite_image_over(&mut content, shell::rgb(tokens.panel));
        self.state.content = Some(content);
        self.state.tokens = Some(tokens);
        unsafe {
            let dpi = GetDpiForWindow(self.owner).max(96);
            let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
            let width = scale(POPUP_WIDTH_DIP);
            let height = match self.state.content.as_ref() {
                Some(PreviewContent::Image { height, .. }) => {
                    scale(IMAGE_HEADER_DIP + *height as i32 + IMAGE_FOOTER_DIP)
                }
                _ => scale(TEXT_POPUP_HEIGHT_DIP),
            };
            let virtual_left = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let virtual_top = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let virtual_right = virtual_left + GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let virtual_bottom = virtual_top + GetSystemMetrics(SM_CYVIRTUALSCREEN);
            let gap = scale(12);
            let mut x = cursor.x + gap;
            let mut y = cursor.y + gap;
            if x + width > virtual_right {
                x = cursor.x - width - gap;
            }
            if y + height > virtual_bottom {
                y = cursor.y - height - gap;
            }
            x = x.clamp(virtual_left, (virtual_right - width).max(virtual_left));
            y = y.clamp(virtual_top, (virtual_bottom - height).max(virtual_top));
            let _ = SetWindowPos(
                self.hwnd,
                Some(HWND_TOP),
                x,
                y,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }

    pub(super) fn hide(&mut self) {
        self.state.content = None;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    pub(super) fn contains_screen_point(&self, point: POINT) -> bool {
        unsafe {
            if !IsWindowVisible(self.hwnd).as_bool() {
                return false;
            }
            let mut rect = RECT::default();
            GetWindowRect(self.hwnd, &mut rect).is_ok()
                && point.x >= rect.left
                && point.x < rect.right
                && point.y >= rect.top
                && point.y < rect.bottom
        }
    }

    pub(super) fn is_visible(&self) -> bool {
        unsafe { IsWindowVisible(self.hwnd).as_bool() }
    }

    fn owner(&self) -> HWND {
        self.owner
    }
}

impl Drop for Popup {
    fn drop(&mut self) {
        self.hide();
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn popup_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = lparam.0 as *const CREATESTRUCTW;
            if !create.is_null() {
                let state = (*create).lpCreateParams as *mut PopupState;
                if !state.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
                }
            }
            return LRESULT(1);
        }
        match message {
            WM_ERASEBKGND => LRESULT(1),
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_MOUSEMOVE => {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: HOVER_DEFAULT,
                };
                let _ = TrackMouseEvent(&mut track);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                let _ = ShowWindow(hwnd, SW_HIDE);
                LRESULT(0)
            }
            WM_PAINT => {
                paint_popup(hwnd);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn paint_popup(hwnd: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let dc = BeginPaint(hwnd, &mut paint);
        if dc == HDC::default() {
            return;
        }
        let mut area = RECT::default();
        let _ = GetClientRect(hwnd, &mut area);
        let state = popup_state(hwnd);
        let (panel, border_color, foreground, muted, heading) = state
            .and_then(|state| state.tokens)
            .map(|tokens| {
                (
                    shell::rgb(tokens.panel),
                    shell::rgb(tokens.border),
                    shell::rgb(tokens.foreground),
                    shell::rgb(tokens.muted),
                    shell::rgb(tokens.heading),
                )
            })
            .unwrap_or_else(|| {
                (
                    COLORREF(GetSysColor(COLOR_BTNFACE)),
                    COLORREF(GetSysColor(COLOR_3DSHADOW)),
                    COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                    COLORREF(GetSysColor(COLOR_GRAYTEXT)),
                    COLORREF(GetSysColor(COLOR_WINDOWTEXT)),
                )
            });
        let background = CreateSolidBrush(panel);
        let border = CreateSolidBrush(border_color);
        let _ = FillRect(dc, &area, background);
        let _ = FrameRect(dc, &area, border);
        let _ = SetBkMode(dc, TRANSPARENT);
        if let Some(content) = state.and_then(|state| state.content.as_ref()) {
            paint_content(hwnd, dc, area, content, foreground, muted, heading);
        }
        let _ = DeleteObject(background.into());
        let _ = DeleteObject(border.into());
        let _ = EndPaint(hwnd, &paint);
    }
}

fn popup_state(hwnd: HWND) -> Option<&'static PopupState> {
    let pointer = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *const PopupState;
    unsafe { pointer.as_ref() }
}

unsafe fn paint_content(
    hwnd: HWND,
    dc: HDC,
    client: RECT,
    content: &PreviewContent,
    foreground: COLORREF,
    muted: COLORREF,
    heading: COLORREF,
) {
    let dpi = unsafe { GetDpiForWindow(hwnd).max(96) };
    let scale = |value: i32| ((i64::from(value) * i64::from(dpi) + 48) / 96) as i32;
    let left = scale(14);
    let right = client.right - scale(14);
    match content {
        PreviewContent::Text {
            label,
            title,
            excerpt,
            ..
        } => unsafe {
            let _ = SetTextColor(dc, muted);
            draw_text(
                dc,
                label,
                RECT {
                    left,
                    top: scale(11),
                    right,
                    bottom: scale(32),
                },
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let _ = SetTextColor(dc, heading);
            draw_text(
                dc,
                title,
                RECT {
                    left,
                    top: scale(38),
                    right,
                    bottom: scale(82),
                },
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL,
            );
            let _ = SetTextColor(dc, foreground);
            draw_text(
                dc,
                excerpt,
                RECT {
                    left,
                    top: scale(91),
                    right,
                    bottom: client.bottom - scale(12),
                },
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL,
            );
        },
        PreviewContent::Image {
            label,
            width,
            height,
            pixels,
        } => unsafe {
            let _ = SetTextColor(dc, muted);
            draw_text(
                dc,
                label,
                RECT {
                    left,
                    top: scale(10),
                    right,
                    bottom: scale(32),
                },
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let image_width = scale(*width as i32);
            let image_height = scale(*height as i32);
            let image_x = (client.right - image_width) / 2;
            let image_top = scale(40);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: *width as i32,
                    biHeight: -(*height as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let _ = StretchDIBits(
                dc,
                image_x,
                image_top,
                image_width,
                image_height,
                0,
                0,
                *width as i32,
                *height as i32,
                Some(pixels.as_ptr().cast()),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
        },
        PreviewContent::Unavailable { label, reason } => unsafe {
            let _ = SetTextColor(dc, muted);
            draw_text(
                dc,
                label,
                RECT {
                    left,
                    top: scale(12),
                    right,
                    bottom: scale(34),
                },
                DT_LEFT | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            let _ = SetTextColor(dc, foreground);
            draw_text(
                dc,
                fallback_label(reason),
                RECT {
                    left,
                    top: scale(48),
                    right,
                    bottom: scale(110),
                },
                DT_LEFT | DT_WORDBREAK | DT_NOPREFIX | DT_EDITCONTROL,
            );
        },
    }
}

unsafe fn draw_text(dc: HDC, text: &str, mut area: RECT, format: DRAW_TEXT_FORMAT) {
    if text.is_empty() {
        return;
    }
    let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
    unsafe {
        let _ = DrawTextW(dc, &mut utf16, &mut area, format);
    }
}

fn fallback_label(reason: &PreviewFallback) -> &'static str {
    match reason {
        PreviewFallback::ExternalLink => "Webリンクはプレビューしません",
        PreviewFallback::UnsupportedScheme => "未対応のリンク形式です",
        PreviewFallback::UnsafePath => "安全なローカルパスではありません",
        PreviewFallback::OutsideWorkspace => "Workspace内のファイルのみプレビューします",
        PreviewFallback::MissingTarget => "リンク先がありません",
        PreviewFallback::ReadDenied => "リンク先を読み取れません",
        PreviewFallback::UnsupportedType => "未対応のファイル形式です",
        PreviewFallback::TooLarge => "サイズ上限のためプレビューできません",
        PreviewFallback::InvalidText => "テキストを表示できません",
        PreviewFallback::InvalidImage => "画像を表示できません",
        PreviewFallback::FragmentNotFound => "対象の見出しがありません",
    }
}

fn composite_image_over(content: &mut PreviewContent, background: COLORREF) {
    let PreviewContent::Image { pixels, .. } = content else {
        return;
    };
    // COLORREF is 0x00BBGGRR, while image pixels are premultiplied BGRA.
    let background = [
        ((background.0 >> 16) & 0xff) as u8,
        ((background.0 >> 8) & 0xff) as u8,
        (background.0 & 0xff) as u8,
    ];
    for pixel in pixels.as_chunks_mut::<4>().0 {
        let transparency = 255u8.saturating_sub(pixel[3]);
        for channel in 0..3 {
            pixel[channel] = pixel[channel].saturating_add(
                ((u16::from(background[channel]) * u16::from(transparency)) / 255) as u8,
            );
        }
        pixel[3] = 255;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HoverTarget {
    document_id: u64,
    revision: u64,
    destination: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DismissedHover {
    target: HoverTarget,
    x: i32,
    y: i32,
}

/// Per-App hover state. It is inert until a link actually enters hover, so
/// ordinary editor fixtures do not start background threads or native windows.
#[derive(Default)]
pub(super) struct PreviewUi {
    coordinator: Option<mdlite::link_preview::PreviewCoordinator>,
    popup: Option<Popup>,
    target: Option<HoverTarget>,
    pending_generation: Option<u64>,
    cursor: POINT,
    owner: Option<HWND>,
    dismissed_hover: Option<DismissedHover>,
}

impl PreviewUi {
    fn remember_dismissed_hover(&self) -> Option<DismissedHover> {
        Some(DismissedHover {
            target: self.target.as_ref()?.clone(),
            x: self.cursor.x,
            y: self.cursor.y,
        })
    }

    fn dismissed_hover_for_escape(&self) -> Option<DismissedHover> {
        let active =
            self.pending_generation.is_some() || self.popup.as_ref().is_some_and(Popup::is_visible);
        active.then(|| self.remember_dismissed_hover()).flatten()
    }

    /// Suppress only the stationary target dismissed by Escape. A new screen
    /// point, destination, or document revision releases the latch.
    fn suppress_same_hover(&mut self, target: &HoverTarget, cursor: POINT) -> bool {
        let suppressed = self.dismissed_hover.as_ref().is_some_and(|dismissed| {
            dismissed.target == *target && dismissed.x == cursor.x && dismissed.y == cursor.y
        });
        if !suppressed {
            self.dismissed_hover = None;
        }
        suppressed
    }

    fn same_dismissed_pointer(&self, cursor: POINT) -> bool {
        self.dismissed_hover
            .as_ref()
            .is_some_and(|dismissed| dismissed.x == cursor.x && dismissed.y == cursor.y)
    }
}

impl super::App {
    pub(super) fn preview_test_geometry(&self) -> serde_json::Value {
        let popup = self.preview_ui.popup.as_ref();
        serde_json::json!({
            "pending":self.preview_ui.pending_generation.is_some(),
            "visible":popup.is_some_and(Popup::is_visible),
            "hwnd":popup.map(|popup| popup.hwnd.0 as usize),
            "destination":self.preview_ui.target.as_ref().map(|target| &target.destination),
            "content":popup.and_then(|popup| popup.state.content.as_ref()).map(|content| match content {
                PreviewContent::Text { label, title, excerpt, fragment } => serde_json::json!({"kind":"text","label":label,"title":title,"excerpt":excerpt,"fragment":fragment}),
                PreviewContent::Image { label, width, height, .. } => serde_json::json!({"kind":"image","label":label,"width":width,"height":height}),
                PreviewContent::Unavailable { label, reason } => serde_json::json!({"kind":"fallback","label":label,"reason":format!("{reason:?}")})
            })
        })
    }
    /// Submit one eligible link target. The caller passes a screen-space point
    /// and the surface HWND that produced the hover, without changing editor
    /// focus, selection, source, or scroll state.
    pub(super) fn preview_hover(
        &mut self,
        destination: Option<&str>,
        cursor: POINT,
        owner: HWND,
    ) -> bool {
        let now = std::time::Instant::now();
        let typing_recent = self
            .last_typing
            .is_some_and(|typed| now.saturating_duration_since(typed) < Duration::from_millis(450));
        let eligible = self.is_markdown_document()
            && !self.feature_ime_active()
            && !self.dragging
            && !self.dragging_scroll
            && !self.typing
            && !typing_recent;
        let incoming_target = destination.map(|destination| HoverTarget {
            document_id: self.doc().id(),
            revision: self.doc().revision(),
            destination: destination.to_owned(),
        });
        if let Some(target) = incoming_target.as_ref() {
            if self.preview_ui.suppress_same_hover(target, cursor) {
                return false;
            }
        } else if !self.preview_ui.same_dismissed_pointer(cursor) {
            self.preview_ui.dismissed_hover = None;
        }
        let Some(target) = incoming_target.filter(|_| eligible) else {
            return self.preview_cancel();
        };
        {
            let ui = &mut self.preview_ui;
            ui.cursor = cursor;
            if ui.target.as_ref() == Some(&target)
                && (ui.pending_generation.is_some()
                    || ui.popup.as_ref().is_some_and(Popup::is_visible))
                && ui.owner == Some(owner)
            {
                return false;
            }
            if ui.owner != Some(owner) {
                ui.popup = None;
                ui.owner = Some(owner);
            } else if let Some(popup) = ui.popup.as_mut() {
                popup.hide();
            }
            ui.target = Some(target.clone());
            ui.pending_generation = None;
        }

        let source_path = self.tabs[self.active]
            .file
            .as_ref()
            .map(|file| file.path().to_path_buf());
        let source = self.doc().source_shared();
        let workspace_root = self.features.preview_workspace_root();
        let open_documents = self
            .tabs
            .iter()
            .filter_map(|tab| {
                let file = tab.file.as_ref()?;
                Some(mdlite::link_preview::OpenDocumentSnapshot {
                    path: file.path().to_path_buf(),
                    document: tab.doc.snapshot(),
                })
            })
            .collect();
        let request = mdlite::link_preview::PreviewRequest {
            source_document_id: target.document_id,
            source_revision: target.revision,
            source_path,
            source,
            workspace_root,
            destination: target.destination,
            open_documents,
        };
        let ui = &mut self.preview_ui;
        if ui.coordinator.is_none() {
            ui.coordinator = mdlite::link_preview::PreviewCoordinator::new().ok();
        }
        let Some(coordinator) = ui.coordinator.as_ref() else {
            ui.target = None;
            return false;
        };
        let generation = coordinator.request(request);
        ui.pending_generation = Some(generation);
        true
    }

    /// Poll only a pending background request. An inactive document or stale
    /// source revision closes its popup before any result can be drawn.
    pub(super) fn preview_tick(&mut self) -> bool {
        let current_document = (self.doc().id(), self.doc().revision());
        if self
            .preview_ui
            .dismissed_hover
            .as_ref()
            .is_some_and(|dismissed| {
                (dismissed.target.document_id, dismissed.target.revision) != current_document
            })
        {
            self.preview_ui.dismissed_hover = None;
        }
        let stale = self
            .preview_ui
            .target
            .as_ref()
            .is_some_and(|target| (target.document_id, target.revision) != current_document);
        if stale {
            return self.preview_cancel();
        }
        let ui = &mut self.preview_ui;
        let Some(generation) = ui.pending_generation else {
            return false;
        };
        let Some(result) = ui.coordinator.as_ref().and_then(|worker| worker.poll()) else {
            return false;
        };
        let Some(target) = ui.target.as_ref() else {
            ui.pending_generation = None;
            return false;
        };
        if !result.is_current_for(generation, target.document_id, target.revision)
            || (target.document_id, target.revision) != current_document
        {
            ui.pending_generation = None;
            if let Some(popup) = ui.popup.as_mut() {
                popup.hide();
            }
            return false;
        }
        ui.pending_generation = None;
        let Some(owner) = ui.owner else {
            return false;
        };
        if ui
            .popup
            .as_ref()
            .is_some_and(|popup| popup.owner() != owner)
        {
            ui.popup = None;
        }
        if ui.popup.is_none() {
            ui.popup = Popup::new(owner).ok();
        }
        if let Some(popup) = ui.popup.as_mut() {
            popup.show_at(ui.cursor, result.content, self.shell.tokens);
            return true;
        }
        false
    }

    /// Cancel pending resolution and close the one preview popup. The return
    /// value lets Escape consume only when it dismissed preview state.
    pub(super) fn preview_cancel(&mut self) -> bool {
        let ui = &mut self.preview_ui;
        let had_preview =
            ui.pending_generation.is_some() || ui.popup.as_ref().is_some_and(Popup::is_visible);
        if had_preview && let Some(coordinator) = ui.coordinator.as_ref() {
            coordinator.cancel();
        }
        if let Some(popup) = ui.popup.as_mut() {
            popup.hide();
        }
        ui.target = None;
        ui.pending_generation = None;
        had_preview
    }

    /// Escape dismisses the current popup/request and suppresses the synthetic
    /// same-point WM_MOUSEMOVE that can follow hiding the non-activating window.
    pub(super) fn preview_escape(&mut self) -> bool {
        let Some(dismissed_hover) = self.preview_ui.dismissed_hover_for_escape() else {
            return false;
        };
        self.preview_cancel();
        self.preview_ui.dismissed_hover = Some(dismissed_hover);
        true
    }

    /// `WM_MOUSELEAVE` on the editor closes the preview unless the pointer
    /// moved onto the popup so its text or image can be read.
    pub(super) fn preview_pointer_leave(&mut self, cursor: POINT) -> bool {
        if self
            .preview_ui
            .popup
            .as_ref()
            .is_some_and(|popup| popup.contains_screen_point(cursor))
        {
            false
        } else {
            self.preview_ui.dismissed_hover = None;
            self.preview_cancel()
        }
    }
}

#[cfg(test)]
mod preview_escape_tests {
    use super::*;
    use mdlite::link_preview::{PreviewRequest, PreviewResult};
    use std::sync::Arc;

    fn target(document_id: u64, revision: u64, destination: &str) -> HoverTarget {
        HoverTarget {
            document_id,
            revision,
            destination: destination.into(),
        }
    }

    fn request(target: &HoverTarget) -> PreviewRequest {
        PreviewRequest {
            source_document_id: target.document_id,
            source_revision: target.revision,
            source_path: None,
            source: Arc::from("# Late\nold worker result\n"),
            workspace_root: None,
            destination: target.destination.clone(),
            open_documents: Vec::new(),
        }
    }

    fn editor_state(
        app: &super::super::App,
    ) -> (String, super::super::Selection, usize, usize, bool, f32) {
        (
            app.doc().source().to_owned(),
            app.doc().selection(),
            app.doc().undo_count(),
            app.doc().redo_count(),
            app.doc().is_dirty(),
            app.tabs[app.active].scroll,
        )
    }

    fn remember_dismissal(ui: &mut PreviewUi, target: HoverTarget, cursor: POINT) {
        ui.target = Some(target);
        ui.cursor = cursor;
        ui.pending_generation = Some(12);
        ui.dismissed_hover = ui.dismissed_hover_for_escape();
        ui.target = None;
        ui.pending_generation = None;
    }

    #[test]
    fn escape_suppresses_same_point_but_motion_target_or_document_change_releases_it() {
        let mut ui = PreviewUi::default();
        let point = POINT { x: 524, y: 636 };
        let current = target(7, 4, "note.md");
        ui.coordinator = Some(mdlite::link_preview::PreviewCoordinator::new().unwrap());

        remember_dismissal(&mut ui, current.clone(), point);
        assert!(ui.dismissed_hover_for_escape().is_none());
        assert!(ui.same_dismissed_pointer(point));
        assert!(ui.suppress_same_hover(&current, point));
        assert!(ui.pending_generation.is_none());
        assert!(ui.popup.is_none());
        assert_eq!(ui.coordinator.as_ref().unwrap().current_generation(), 0);

        assert!(!ui.suppress_same_hover(
            &current,
            POINT {
                x: point.x + 1,
                y: point.y,
            },
        ));
        assert!(ui.dismissed_hover.is_none());
        let generation = ui.coordinator.as_ref().unwrap().request(request(&current));
        ui.pending_generation = Some(generation);
        assert_eq!(ui.coordinator.as_ref().unwrap().current_generation(), 1);
        ui.coordinator.as_ref().unwrap().cancel();
        ui.pending_generation = None;

        remember_dismissal(&mut ui, current.clone(), point);
        let other_target = target(7, 4, "other.md");
        assert!(!ui.suppress_same_hover(&other_target, point));
        assert!(ui.dismissed_hover.is_none());
        let other_generation = ui
            .coordinator
            .as_ref()
            .unwrap()
            .request(request(&other_target));
        ui.pending_generation = Some(other_generation);
        assert_eq!(other_generation, 3);
        ui.coordinator.as_ref().unwrap().cancel();
        ui.pending_generation = None;

        remember_dismissal(&mut ui, current, point);
        let changed_document = target(7, 5, "note.md");
        assert!(!ui.suppress_same_hover(&changed_document, point));
        assert!(ui.dismissed_hover.is_none());
        let changed_generation = ui
            .coordinator
            .as_ref()
            .unwrap()
            .request(request(&changed_document));
        ui.pending_generation = Some(changed_generation);
        assert_eq!(changed_generation, 5);
    }

    #[test]
    fn escape_cancels_queued_or_late_generation_before_tick_can_accept_it() {
        let coordinator = mdlite::link_preview::PreviewCoordinator::new().unwrap();
        let target = target(7, 4, "#late");
        let request_generation = coordinator.request(request(&target));
        let cancelled_generation = coordinator.cancel();
        let stale = PreviewResult {
            generation: request_generation,
            source_document_id: 7,
            source_revision: 4,
            content: mdlite::link_preview::PreviewContent::Unavailable {
                label: "note.md".into(),
                reason: mdlite::link_preview::PreviewFallback::FragmentNotFound,
            },
        };

        assert_ne!(request_generation, cancelled_generation);
        assert!(!stale.is_current_for(cancelled_generation, 7, 4));
        assert!(coordinator.poll().is_none());
    }

    #[test]
    fn app_escape_blocks_same_point_rehover_and_releases_on_motion_target_or_revision() {
        let mut app = super::super::tests::composing_test_app(
            "[target](#section)\n[other](#other)\n\n# Section\npreview body\n\n# Other\nother body\n",
        );
        app.clear_composition();
        let owner = HWND::default();
        let point = POINT { x: 524, y: 636 };
        let original_editor_state = editor_state(&app);

        assert!(app.preview_hover(Some("#section"), point, owner));
        let first_generation = app
            .preview_ui
            .coordinator
            .as_ref()
            .unwrap()
            .current_generation();
        assert!(app.preview_ui.pending_generation.is_some());
        assert!(app.preview_escape());
        assert!(app.preview_ui.pending_generation.is_none());
        assert!(app.preview_ui.popup.is_none());
        assert_eq!(editor_state(&app), original_editor_state);
        let cancelled_generation = app
            .preview_ui
            .coordinator
            .as_ref()
            .unwrap()
            .current_generation();
        assert_ne!(first_generation, cancelled_generation);

        assert!(!app.preview_hover(Some("#section"), point, owner));
        assert!(app.preview_ui.pending_generation.is_none());
        assert_eq!(
            app.preview_ui
                .coordinator
                .as_ref()
                .unwrap()
                .current_generation(),
            cancelled_generation
        );
        assert_eq!(editor_state(&app), original_editor_state);

        assert!(app.preview_hover(
            Some("#section"),
            POINT {
                x: point.x + 1,
                y: point.y,
            },
            owner,
        ));
        assert!(app.preview_ui.pending_generation.is_some());
        assert_eq!(editor_state(&app), original_editor_state);
        app.preview_cancel();

        assert!(app.preview_hover(Some("#section"), point, owner));
        assert!(app.preview_escape());
        assert!(app.preview_hover(Some("#other"), point, owner));
        assert!(app.preview_ui.pending_generation.is_some());
        assert_eq!(editor_state(&app), original_editor_state);
        app.preview_cancel();

        assert!(app.preview_hover(Some("#section"), point, owner));
        assert!(app.preview_escape());
        app.doc_mut().replace_range(0..0, "changed ").unwrap();
        let edited_state = editor_state(&app);
        assert!(app.preview_hover(Some("#section"), point, owner));
        assert!(app.preview_ui.pending_generation.is_some());
        assert_eq!(editor_state(&app), edited_state);
        app.preview_cancel();
    }
}
