//! Native table and image operations. Commands edit the active Document once;
//! this module keeps only source-range context and the two header controls.

use super::{App, shell, wide};
use mdlite::{
    document::Selection,
    projection::BlockKind,
    table::{Table, TableEdit, TablePosition},
};
use std::ops::Range;
use windows::{
    Win32::{
        Foundation::*,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{EnableWindow, SetFocus},
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, w},
};

const ROW_BELOW: usize = 20;
const DELETE_ROW: usize = 21;
const COLUMN_RIGHT: usize = 22;
const DELETE_COLUMN: usize = 23;
const ALIGN_LEFT: usize = 24;
const ALIGN_CENTER: usize = 25;
const ALIGN_RIGHT: usize = 26;
const IMAGE_NARROWER: usize = 27;
const IMAGE_WIDER: usize = 28;
const IMAGE_320: usize = 29;
const IMAGE_480: usize = 30;
const IMAGE_640: usize = 31;
const ROW_ABOVE: usize = 32;
const COLUMN_LEFT: usize = 33;
const TABLE_MENU: usize = 34;
const TABLE_SOURCE: usize = 38;
const TABLE_OPERATIONS: usize = 39;
const TOOLBAR_WIDTH: f32 = 110.0;
const TOOLBAR_LABEL: &str = "表・画像 ▼";

pub(super) const COMMANDS: &[(usize, &str, Option<&'static str>)] = &[
    (ROW_BELOW, "行を下に追加", None),
    (DELETE_ROW, "行を削除（見出し・最後の行を保護）", None),
    (COLUMN_RIGHT, "列を右に追加", None),
    (DELETE_COLUMN, "列を削除（最後の列を保護）", None),
    (ALIGN_LEFT, "列を左揃え", None),
    (ALIGN_CENTER, "列を中央揃え", None),
    (ALIGN_RIGHT, "列を右揃え", None),
    (IMAGE_NARROWER, "画像の幅を縮小（比率固定）", None),
    (IMAGE_WIDER, "画像の幅を拡大（比率固定）", None),
    (IMAGE_320, "画像幅を320 DIPにする", None),
    (IMAGE_480, "画像幅を480 DIPにする", None),
    (IMAGE_640, "画像幅を640 DIPにする", None),
    (ROW_ABOVE, "行を上に追加", None),
    (COLUMN_LEFT, "列を左に追加", None),
    (TABLE_MENU, "表・画像の操作メニューを開く", None),
    (TABLE_SOURCE, "表のソース / Liveを切り替え", None),
    (TABLE_OPERATIONS, "表・画像の操作にフォーカス", None),
];

#[derive(Clone, Debug)]
struct ActionContext {
    document_id: u64,
    revision: u64,
    selection: Selection,
    table_index: Option<usize>,
    table_position: Option<TablePosition>,
    table_range: Option<Range<usize>>,
    image_range: Option<Range<usize>>,
}

#[derive(Default)]
pub(super) struct EditorActions {
    main_button: HWND,
    compact_button: HWND,
    context: Option<ActionContext>,
}

impl EditorActions {
    pub(super) fn sync(&mut self, app: &mut App) {
        if !app.is_markdown_document() {
            self.context = None;
            unsafe { self.sync_controls(app) };
            return;
        }
        let projection = app.cached_projection();
        let selection = app.doc().selection();
        let byte = selection.head;
        let table_index = app
            .parsed_tables
            .iter()
            .position(|table| table.range.start <= byte && byte <= table.range.end);
        let table_position = table_index
            .and_then(|index| app.parsed_tables.get(index))
            .and_then(|table| table.cell_at(byte));
        let table_range = table_index
            .and_then(|index| app.parsed_tables.get(index))
            .map(|table| table.range.clone());
        let selected_images: Vec<_> = projection
            .blocks
            .iter()
            .filter(|block| matches!(&block.kind, BlockKind::Image { .. }))
            .filter(|block| image_selection_matches(&block.source, selection))
            .map(|block| block.source.clone())
            .take(2)
            .collect();
        self.context = Some(ActionContext {
            document_id: app.doc().id(),
            revision: app.doc().revision(),
            selection,
            table_index,
            table_position,
            table_range,
            image_range: (selected_images.len() == 1).then(|| selected_images[0].clone()),
        });
        unsafe { self.sync_controls(app) };
    }

    pub(super) fn command(&mut self, app: &mut App, id: usize) -> bool {
        if !is_editor_command(id) {
            return false;
        }
        self.sync(app);
        if !self.enabled(app, id) {
            app.status = self.disabled_reason(app, id).to_owned();
            return true;
        }
        match id {
            TABLE_SOURCE => {
                let raw = &mut app.tabs[app.active].raw;
                *raw = !*raw;
                if *raw {
                    app.clear_images();
                }
                app.layout_key = None;
                self.sync(app);
                unsafe { app.refresh() };
            }
            TABLE_MENU => self.open_menu(app),
            TABLE_OPERATIONS => {
                self.focus_toolbar(app, TABLE_OPERATIONS);
            }
            IMAGE_NARROWER | IMAGE_WIDER | IMAGE_320 | IMAGE_480 | IMAGE_640 => {
                self.set_image_width(app, id);
            }
            _ => self.apply_table_action(app, id),
        }
        true
    }

    pub(super) fn enabled(&self, app: &App, id: usize) -> bool {
        if !is_editor_command(id)
            || !app.is_markdown_document()
            || app.feature_ime_active()
            || app.background_mutation_busy()
        {
            return false;
        }
        let Some(context) = self.current_context(app) else {
            return false;
        };
        if matches!(id, TABLE_MENU | TABLE_OPERATIONS) {
            return context.table_range.is_some() || context.image_range.is_some();
        }
        if id == TABLE_SOURCE {
            return context.table_range.is_some();
        }
        if matches!(
            id,
            ROW_BELOW | DELETE_ROW | COLUMN_RIGHT | DELETE_COLUMN | ALIGN_LEFT
                ..=ALIGN_RIGHT | ROW_ABOVE | COLUMN_LEFT
        ) {
            return table_action_available_for_context(app, context, id);
        }
        image_replacement(app, context, id).is_some()
    }

    pub(super) fn disabled_reason(&self, app: &App, id: usize) -> &'static str {
        if !is_editor_command(id) {
            return "";
        }
        if app.feature_ime_active() {
            return "IME入力を確定または取消してから実行してください";
        }
        if app.background_mutation_busy() {
            return "実行中の変更処理を完了または取消してください";
        }
        let Some(context) = self.current_context(app) else {
            return "編集位置を更新してください";
        };
        if matches!(id, TABLE_MENU | TABLE_OPERATIONS) {
            return if context.table_range.is_some() || context.image_range.is_some() {
                ""
            } else {
                "表のセルまたは画像にキャレットを置いてください"
            };
        }
        if id == TABLE_SOURCE {
            return if context.table_range.is_some() {
                ""
            } else {
                "表の中にキャレットを置いてください"
            };
        }
        if matches!(
            id,
            ROW_BELOW | DELETE_ROW | COLUMN_RIGHT | DELETE_COLUMN | ALIGN_LEFT
                ..=ALIGN_RIGHT | ROW_ABOVE | COLUMN_LEFT
        ) {
            if context.table_position.is_none() {
                return "表のセル内にキャレットを置いてください";
            }
            if table_action_available_for_context(app, context, id) {
                return "";
            }
            return "最小構造を保つため、この表操作は実行できません";
        }
        if context.image_range.is_none() {
            return "画像を選択してください";
        }
        if matches!(id, IMAGE_NARROWER | IMAGE_WIDER) && app.tabs[app.active].raw {
            return "比率調整にはLive表示が必要です";
        }
        if image_replacement(app, context, id).is_none() {
            return if matches!(id, IMAGE_NARROWER | IMAGE_WIDER) {
                "Live画像の幅を取得できないか、幅の上限に達しています"
            } else {
                "画像の幅は既に指定値か、対応形式ではありません"
            };
        }
        ""
    }

    pub(super) fn focus_toolbar(&mut self, app: &mut App, id: usize) -> bool {
        if id != TABLE_OPERATIONS || app.feature_ime_active() || app.background_mutation_busy() {
            return false;
        }
        self.sync(app);
        if !self.enabled(app, id) {
            return false;
        }
        let button = self.active_button(app);
        if button == HWND::default() {
            return false;
        }
        unsafe {
            let _ = SetFocus(Some(button));
        }
        true
    }

    unsafe fn sync_controls(&mut self, app: &App) {
        if !app.shell.enabled {
            return;
        }
        let main = app.shell.main_window;
        if main != HWND::default()
            && (self.main_button == HWND::default()
                || !unsafe { IsWindow(Some(self.main_button)).as_bool() })
        {
            self.main_button = unsafe {
                shell::control(
                    main,
                    w!("BUTTON"),
                    TOOLBAR_LABEL,
                    TABLE_MENU,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                )
            };
        }
        let compact = app.shell.compact_window;
        if compact != HWND::default()
            && (self.compact_button == HWND::default()
                || !unsafe { IsWindow(Some(self.compact_button)).as_bool() })
        {
            self.compact_button = unsafe {
                shell::control(
                    compact,
                    w!("BUTTON"),
                    TOOLBAR_LABEL,
                    TABLE_MENU,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                )
            };
        }
        let target_context = self
            .context
            .as_ref()
            .is_some_and(|context| context.table_range.is_some() || context.image_range.is_some());
        let button_enabled =
            target_context && !app.feature_ime_active() && !app.background_mutation_busy();
        if self.main_button != HWND::default() {
            let main_active = compact == HWND::default() && target_context;
            unsafe {
                let _ = ShowWindow(
                    self.main_button,
                    if main_active { SW_SHOW } else { SW_HIDE },
                );
                let _ = EnableWindow(self.main_button, button_enabled);
                if main_active && app.hwnd == main {
                    position_button(self.main_button, app.editor_right() - 114.0, 42.0, main);
                }
            }
        }
        if self.compact_button != HWND::default()
            && !unsafe { IsWindow(Some(self.compact_button)).as_bool() }
        {
            self.compact_button = HWND::default();
        }
        if self.compact_button != HWND::default() {
            let compact_active = compact != HWND::default() && target_context;
            unsafe {
                let _ = ShowWindow(
                    self.compact_button,
                    if compact_active { SW_SHOW } else { SW_HIDE },
                );
                let _ = EnableWindow(self.compact_button, button_enabled);
                if compact_active && app.hwnd == compact {
                    position_button(self.compact_button, app.width - 114.0, 10.0, compact);
                }
            }
        }
    }

    fn open_menu(&mut self, app: &mut App) {
        if !self.focus_toolbar(app, TABLE_OPERATIONS) {
            return;
        }
        let button = self.active_button(app);
        let Ok(menu) = (unsafe { CreatePopupMenu() }) else {
            return;
        };
        for (id, label, _) in COMMANDS.iter().copied() {
            if matches!(id, TABLE_MENU | TABLE_OPERATIONS) {
                continue;
            }
            let flags = MF_STRING
                | if self.enabled(app, id) {
                    MF_ENABLED
                } else {
                    MF_GRAYED
                };
            let label = wide(label);
            unsafe {
                let _ = AppendMenuW(menu, flags, id, PCWSTR(label.as_ptr()));
            }
        }
        let mut bounds = RECT::default();
        if unsafe { GetWindowRect(button, &mut bounds) }.is_err() {
            unsafe {
                let _ = DestroyMenu(menu);
            }
            return;
        }
        let selected = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                bounds.left,
                bounds.bottom,
                None,
                app.hwnd,
                None,
            )
        }
        .0 as usize;
        unsafe {
            let _ = DestroyMenu(menu);
        }
        if selected != 0 {
            self.command(app, selected);
        }
        unsafe { app.after_input() };
    }

    fn active_button(&self, app: &App) -> HWND {
        if app.shell.compact_window != HWND::default() {
            self.compact_button
        } else {
            self.main_button
        }
    }

    fn apply_table_action(&mut self, app: &mut App, id: usize) {
        let Some(context) = self.current_context(app) else {
            return;
        };
        let Some(edit) = table_edit_for_context(app, context, id) else {
            return;
        };
        app.doc_mut().break_undo_group();
        app.apply_table(edit);
        self.sync(app);
        unsafe { app.after_input() };
    }

    fn set_image_width(&mut self, app: &mut App, id: usize) {
        let Some(context) = self.current_context(app) else {
            return;
        };
        let Some(range) = context.image_range.clone() else {
            return;
        };
        let Some(replacement) = image_replacement(app, context, id) else {
            return;
        };
        app.doc_mut().break_undo_group();
        if let Err(error) = app.doc_mut().replace_range(range, &replacement) {
            app.status = error.to_string();
        } else {
            app.layout_key = None;
        }
        self.sync(app);
        unsafe { app.after_input() };
    }

    fn current_context(&self, app: &App) -> Option<&ActionContext> {
        self.context.as_ref().filter(|context| {
            context.document_id == app.doc().id()
                && context.revision == app.doc().revision()
                && context.selection == app.doc().selection()
        })
    }
}

impl App {
    pub(super) unsafe fn editor_actions_init(&mut self) {
        let mut actions = EditorActions::default();
        actions.sync(self);
        self.editor_actions = actions;
    }

    pub(super) fn editor_actions_sync(&mut self) {
        let mut actions = std::mem::take(&mut self.editor_actions);
        actions.sync(self);
        self.editor_actions = actions;
    }

    pub(super) fn editor_action(&mut self, id: usize) -> bool {
        let mut actions = std::mem::take(&mut self.editor_actions);
        let handled = actions.command(self, id);
        self.editor_actions = actions;
        handled
    }

    pub(super) fn editor_enabled(&self, id: usize) -> bool {
        self.editor_actions.enabled(self, id)
    }

    pub(super) fn editor_disabled_reason(&self, id: usize) -> &'static str {
        self.editor_actions.disabled_reason(self, id)
    }
}

fn position_button(button: HWND, x: f32, y: f32, owner: HWND) {
    unsafe {
        let dpi = GetDpiForWindow(owner).max(1) as f32 / 96.0;
        let _ = SetWindowPos(
            button,
            None,
            (x.max(0.0) * dpi).round() as i32,
            (y.max(0.0) * dpi).round() as i32,
            (TOOLBAR_WIDTH * dpi).round() as i32,
            (28.0 * dpi).round() as i32,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

fn is_editor_command(id: usize) -> bool {
    COMMANDS.iter().any(|(command, _, _)| *command == id)
}

fn image_selection_matches(source: &Range<usize>, selection: Selection) -> bool {
    if selection.is_caret() {
        source.start <= selection.head && selection.head <= source.end
    } else {
        source.start < selection.range().end && source.end > selection.range().start
    }
}

fn table_edit_for_context(app: &App, context: &ActionContext, id: usize) -> Option<TableEdit> {
    let table = app.parsed_tables.get(context.table_index?)?;
    let position = context.table_position?;
    table_edit_for_action(table, app.doc().source(), position, id)
}

fn table_action_available_for_context(app: &App, context: &ActionContext, id: usize) -> bool {
    let Some(table) = context
        .table_index
        .and_then(|index| app.parsed_tables.get(index))
    else {
        return false;
    };
    let Some(position) = context.table_position else {
        return false;
    };
    table_action_available(table, position, id)
}

fn table_action_available(table: &Table, position: TablePosition, id: usize) -> bool {
    match id {
        ROW_BELOW => !table.rows.is_empty() && position.row < table.rows.len(),
        DELETE_ROW => position.row > 0 && position.row < table.rows.len() && table.rows.len() > 2,
        COLUMN_RIGHT | COLUMN_LEFT => table.columns > 0 && position.column < table.columns,
        DELETE_COLUMN => position.column < table.columns && table.columns > 1,
        ALIGN_LEFT | ALIGN_CENTER | ALIGN_RIGHT => {
            table.delimiter.cells.get(position.column).is_some()
        }
        ROW_ABOVE => !table.rows.is_empty() && position.row > 0 && position.row < table.rows.len(),
        _ => false,
    }
}

fn table_edit_for_action(
    table: &Table,
    source: &str,
    position: TablePosition,
    id: usize,
) -> Option<TableEdit> {
    match id {
        ROW_BELOW => table.insert_row(source, position.row + 1),
        DELETE_ROW => table.delete_row(position.row),
        COLUMN_RIGHT => table.insert_column(source, position.column + 1),
        DELETE_COLUMN => table.delete_column(source, position.column),
        ALIGN_LEFT => table.set_alignment(position.column, mdlite::table::TableAlignment::Left),
        ALIGN_CENTER => table.set_alignment(position.column, mdlite::table::TableAlignment::Center),
        ALIGN_RIGHT => table.set_alignment(position.column, mdlite::table::TableAlignment::Right),
        ROW_ABOVE => table.insert_row(source, position.row),
        COLUMN_LEFT => table.insert_column(source, position.column),
        _ => None,
    }
}

fn image_replacement(app: &App, context: &ActionContext, id: usize) -> Option<String> {
    let range = context.image_range.as_ref()?;
    let source = app.doc().source().get(range.clone())?;
    let width = match id {
        IMAGE_320 => 320,
        IMAGE_480 => 480,
        IMAGE_640 => 640,
        IMAGE_NARROWER | IMAGE_WIDER if !app.tabs[app.active].raw => {
            let image = app.images.iter().find(|image| &image.source == range)?;
            ratio_width(image.width, id)?
        }
        _ => return None,
    };
    set_image_width(source, width)
}

fn ratio_width(current: f32, id: usize) -> Option<u32> {
    if !current.is_finite() || current <= 0.0 {
        return None;
    }
    let factor = match id {
        IMAGE_NARROWER => 0.8,
        IMAGE_WIDER => 1.25,
        _ => return None,
    };
    Some((current * factor).round().clamp(24.0, 16_384.0) as u32)
}

fn set_image_width(source: &str, width: u32) -> Option<String> {
    let replacement = mdlite::image_reference::set_width(source, width).ok()?;
    (replacement != source).then_some(replacement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdlite::document::Document;

    const TABLE: &str = "| H1 | H2 |\n| --- | --- |\n| a | b |\n| c | d |\n";

    #[test]
    fn registry_ids_are_unique_and_within_the_shared_range() {
        let ids: Vec<_> = COMMANDS.iter().map(|entry| entry.0).collect();
        for (index, id) in ids.iter().enumerate() {
            assert!((20..=39).contains(id));
            assert!(!ids[..index].contains(id));
        }
        for id in [
            ROW_BELOW,
            DELETE_ROW,
            COLUMN_RIGHT,
            DELETE_COLUMN,
            ALIGN_LEFT,
            ALIGN_CENTER,
            ALIGN_RIGHT,
            IMAGE_NARROWER,
            IMAGE_WIDER,
            IMAGE_320,
            IMAGE_480,
            IMAGE_640,
            ROW_ABOVE,
            COLUMN_LEFT,
            TABLE_MENU,
            TABLE_SOURCE,
            TABLE_OPERATIONS,
        ] {
            assert!(is_editor_command(id));
        }
    }

    #[test]
    fn table_entry_edits_insert_above_and_left_in_one_undo() {
        let table = mdlite::table::find_tables(TABLE).remove(0);
        let position = TablePosition { row: 2, column: 1 };
        for (id, expected_rows, expected_columns) in [
            (ROW_BELOW, 4, 2),
            (ROW_ABOVE, 4, 2),
            (COLUMN_RIGHT, 3, 3),
            (COLUMN_LEFT, 3, 3),
        ] {
            let edit = table_edit_for_action(&table, TABLE, position, id).unwrap();
            let mut document = Document::new(1, TABLE.to_owned());
            document
                .replace_range(edit.range, &edit.replacement)
                .unwrap();
            assert_eq!(document.undo_count(), 1);
            let updated = document.source().to_owned();
            let parsed = mdlite::table::find_tables(&updated).remove(0);
            assert_eq!(parsed.rows.len(), expected_rows);
            assert_eq!(parsed.columns, expected_columns);
            assert!(document.undo());
            assert_eq!(document.source(), TABLE);
        }
    }

    #[test]
    fn table_entry_edits_keep_header_and_minimum_structure_protected() {
        let source = "| H |\n| --- |\n| only |\n";
        let table = mdlite::table::find_tables(source).remove(0);
        assert!(
            table_edit_for_action(
                &table,
                source,
                TablePosition { row: 0, column: 0 },
                DELETE_ROW
            )
            .is_none()
        );
        assert!(
            table_edit_for_action(
                &table,
                source,
                TablePosition { row: 1, column: 0 },
                DELETE_ROW
            )
            .is_none()
        );
        assert!(
            table_edit_for_action(
                &table,
                source,
                TablePosition { row: 1, column: 0 },
                DELETE_COLUMN
            )
            .is_none()
        );
    }

    #[test]
    fn cheap_table_availability_matches_core_edits_at_boundaries() {
        for source in [
            TABLE,
            "| H |\n| --- |\n| only |\n",
            "| H1 | H2 |\n| :--- | ---: |\n| one | two |\n",
        ] {
            for table in mdlite::table::find_tables(source) {
                for row in 0..table.rows.len() {
                    for column in 0..table.columns {
                        let position = TablePosition { row, column };
                        for id in [
                            ROW_BELOW,
                            DELETE_ROW,
                            COLUMN_RIGHT,
                            DELETE_COLUMN,
                            ALIGN_LEFT,
                            ALIGN_CENTER,
                            ALIGN_RIGHT,
                            ROW_ABOVE,
                            COLUMN_LEFT,
                        ] {
                            assert_eq!(
                                table_action_available(&table, position, id),
                                table_edit_for_action(&table, source, position, id).is_some(),
                                "id={id}, row={row}, column={column}, source={source:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn fixed_image_widths_preserve_markdown_and_legacy_spelling() {
        let markdown = "![alt](<image (1).png> \"title\") <!-- mdlite-width:320 -->";
        assert_eq!(
            set_image_width(markdown, 480).as_deref(),
            Some("![alt](<image (1).png> \"title\") <!-- mdlite-width:480 -->")
        );
        let legacy = "<img src='image.png' alt='alt' title=\"title\" width='320'>";
        assert_eq!(
            set_image_width(legacy, 640).as_deref(),
            Some("<img src='image.png' alt='alt' title=\"title\" width='640'>")
        );
        assert!(set_image_width(markdown, 320).is_none());
    }

    #[test]
    fn ratio_widths_are_bounded_and_reject_invalid_geometry() {
        assert_eq!(ratio_width(100.0, IMAGE_NARROWER), Some(80));
        assert_eq!(ratio_width(100.0, IMAGE_WIDER), Some(125));
        assert_eq!(ratio_width(20.0, IMAGE_NARROWER), Some(24));
        assert_eq!(ratio_width(20_000.0, IMAGE_WIDER), Some(16_384));
        assert_eq!(ratio_width(f32::NAN, IMAGE_WIDER), None);
        assert_eq!(ratio_width(100.0, TABLE_SOURCE), None);
    }
}
