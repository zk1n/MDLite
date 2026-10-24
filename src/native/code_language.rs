//! A small source-backed language menu in a separate fenced-block header row.
use super::*;
use mdlite::code_fence::CodeLanguage;

const HEADER_HEIGHT: f32 = 25.;
pub(super) const COMMAND: usize = 40;
const LANGUAGES: &[&str] = &[
    "",
    "text",
    "rust",
    "python",
    "javascript",
    "typescript",
    "json",
    "yaml",
    "toml",
    "bash",
    "powershell",
    "html",
    "css",
    "sql",
    "c",
    "cpp",
    "csharp",
    "java",
    "go",
    "markdown",
];

struct Header {
    fence: CodeLanguage,
    bounds: mdlite::panel::Rect,
}
#[derive(Default)]
pub(super) struct CodeLanguageUi {
    headers: Vec<Header>,
}

impl App {
    pub(super) fn code_language_geometry(&self) -> serde_json::Value {
        serde_json::json!(self.code_languages.headers.iter().map(|header| serde_json::json!({
            "language":header.fence.language,"token_start":header.fence.token.start,"token_end":header.fence.token.end,
            "block_start":header.fence.block.start,"block_end":header.fence.block.end,
            "x":header.bounds.x,"y":header.bounds.y-self.tabs[self.active].scroll,
            "width":header.bounds.width,"height":header.bounds.height
        })).collect::<Vec<_>>())
    }
    pub(super) fn code_language_layout(&mut self, mut bottom: f32) -> f32 {
        self.code_languages.headers.clear();
        if self.tabs[self.active].raw {
            return bottom;
        }
        let projection = self.cached_projection();
        let fences = CodeLanguage::all(self.doc(), &projection);
        for fence in fences {
            let Some(y) = self
                .views
                .iter()
                .filter(|view| {
                    view.source.start >= fence.opening.start && view.source.start < fence.block.end
                })
                .map(|view| view.y)
                .min_by(f32::total_cmp)
            else {
                continue;
            };
            for view in &mut self.views {
                if view.y >= y {
                    view.y += HEADER_HEIGHT;
                }
            }
            for table in &mut self.tables {
                if table.y >= y {
                    table.y += HEADER_HEIGHT;
                }
            }
            for image in &mut self.images {
                if image.y >= y {
                    image.y += HEADER_HEIGHT;
                }
            }
            let width = (self.editor_right() - self.content_left()).clamp(1., 180.);
            let bounds = mdlite::panel::Rect {
                x: self.editor_right() - width,
                y,
                width,
                height: HEADER_HEIGHT,
            };
            self.code_languages.headers.push(Header { fence, bounds });
            bottom += HEADER_HEIGHT;
        }
        bottom
    }

    pub(super) unsafe fn code_language_draw(
        &self,
        target: &ID2D1RenderTarget,
        scroll: f32,
    ) -> windows::core::Result<()> {
        unsafe {
            let ink = target.CreateSolidColorBrush(&self.shell.tokens.muted, None)?;
            let background =
                target.CreateSolidColorBrush(&self.shell.tokens.code_background, None)?;
            for header in &self.code_languages.headers {
                let y = header.bounds.y - scroll;
                if y < self.editor_top() || y + HEADER_HEIGHT > self.editor_bottom() {
                    continue;
                }
                target.FillRectangle(
                    &rect(
                        self.content_left() - 6.,
                        y,
                        (self.editor_right() - self.content_left() + 12.).max(1.),
                        HEADER_HEIGHT,
                    ),
                    &background,
                );
                let label = if header.fence.language.is_empty() {
                    "未指定"
                } else {
                    &header.fence.language
                };
                let layout = self.ui_text_layout(
                    &format!("{label} ▾"),
                    (header.bounds.width - 12.).max(1.),
                    12.,
                )?;
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                layout.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
                layout.SetMaxHeight(HEADER_HEIGHT - 4.)?;
                target.DrawTextLayout(
                    point(header.bounds.x + 6., y + 3.),
                    &layout,
                    &ink,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }

    pub(super) fn code_language_click(&mut self, x: f32, y: f32) -> bool {
        if self.composing || self.feature_ime_active() {
            return false;
        }
        let scroll = self.tabs[self.active].scroll;
        let Some(index) = self.code_languages.headers.iter().position(|header| {
            y >= self.editor_top()
                && y < self.editor_bottom()
                && header.bounds.contains(x, y + scroll)
        }) else {
            return false;
        };
        self.code_language_menu(index);
        true
    }

    pub(super) fn code_language_available(&self) -> bool {
        let caret = self.doc().selection().head;
        !self.tabs[self.active].raw
            && !self.composing
            && !self.feature_ime_active()
            && self
                .code_languages
                .headers
                .iter()
                .any(|header| header.fence.block.start <= caret && caret <= header.fence.block.end)
    }

    pub(super) fn code_language_menu_at_caret(&mut self) {
        if self.composing || self.feature_ime_active() {
            return;
        }
        let caret = self.doc().selection().head;
        if let Some(index) =
            self.code_languages.headers.iter().position(|header| {
                header.fence.block.start <= caret && caret <= header.fence.block.end
            })
        {
            self.code_language_menu(index);
        }
    }

    fn code_language_menu(&mut self, index: usize) {
        let Some(header) = self.code_languages.headers.get(index) else {
            return;
        };
        let fence = header.fence.clone();
        let bounds = header.bounds;
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            let mut choices: Vec<&str> = LANGUAGES.to_vec();
            if !choices.contains(&fence.language.as_str()) {
                choices.push(&fence.language);
            }
            let can_clear = fence.can_clear(self.doc());
            for (index, language) in choices.iter().enumerate() {
                let label = if language.is_empty() && !can_clear {
                    "未指定（追加infoあり）"
                } else if language.is_empty() {
                    "未指定"
                } else {
                    language
                };
                let label = wide(label);
                let flags = MF_STRING
                    | if language.is_empty() && !can_clear {
                        MF_GRAYED
                    } else {
                        MF_ENABLED
                    }
                    | if *language == fence.language {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                let _ = AppendMenuW(menu, flags, index + 1, PCWSTR(label.as_ptr()));
            }
            let mut position = POINT {
                x: (bounds.x * self.dpi).round() as i32,
                y: ((bounds.y + bounds.height - self.tabs[self.active].scroll)
                    .clamp(self.editor_top(), self.editor_bottom())
                    * self.dpi)
                    .round() as i32,
            };
            let _ = ClientToScreen(self.hwnd, &mut position);
            let selected = menu_style::track_popup(
                self.hwnd,
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                position.x,
                position.y,
                self.shell.tokens,
                self.dpi,
            )
            .0 as usize;
            let _ = DestroyMenu(menu);
            if let Some(language) = selected.checked_sub(1).and_then(|index| choices.get(index)) {
                match fence.set(self.doc_mut(), language) {
                    Ok(true) => self.after_input(),
                    Ok(false) => {}
                    Err(error) => self.status = error.to_owned(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_header_has_its_own_row_before_source_and_body_without_source_edits() {
        let source = "```rust\nlet a = 1;\n```\n\nafter\n";
        let mut app = super::super::tests::composing_test_app(source);
        app.clear_composition();
        let window = super::super::tests::hidden_test_window_size(800, 700);
        app.hwnd = window;
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_eq!(app.code_languages.headers.len(), 1);
        let header = &app.code_languages.headers[0];
        assert!(header.bounds.width <= app.editor_right() - app.content_left() + 0.1);
        for view in &app.views {
            if view.source.start < header.fence.block.end {
                assert!(view.y >= header.bounds.y + HEADER_HEIGHT - 0.1);
            }
        }
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().undo_count(), 0);
        app.tabs[app.active].raw = true;
        app.layout_key = None;
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert!(app.code_languages.headers.is_empty());
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
}
