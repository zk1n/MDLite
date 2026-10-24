//! A revision-bound native draft over the source-backed property projection.
use super::*;
use mdlite::properties::{Properties, PropertyEdit, PropertyValue};
use windows::Win32::UI::Controls::*;

const EDIT_ID: usize = 0x7d00;
const HISTORY_UNDO: usize = 0x7d01;
const HISTORY_REDO: usize = 0x7d02;
const CHILD_SUBCLASS: usize = 0x7d03;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Field {
    Key(usize),
    Value(usize),
    Item(usize, usize),
    Delete(usize),
    Type(usize),
    AddItem(usize),
    DeleteItem(usize, usize),
    Add,
    Raw(usize),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        app: App,
        window: HWND,
    }
    impl Fixture {
        fn new(source: &str) -> Self {
            let mut app = super::super::tests::composing_test_app(source);
            app.clear_composition();
            app.tabs[0].raw = false;
            app.hwnd = super::super::tests::hidden_test_window_size(900, 900);
            unsafe {
                app.ensure_layout().unwrap();
            }
            Self {
                window: app.hwnd,
                app,
            }
        }
        fn click(&mut self, field: Field) {
            let row = self
                .app
                .properties_ui
                .rows
                .iter()
                .find(|row| row.field == field)
                .unwrap();
            assert!(self.app.properties_click(
                row.area.left + 6.0,
                row.area.top + 6.0 - self.app.tabs[0].scroll,
                false
            ));
        }
        fn replace(&mut self, text: &str) {
            let child = self.app.properties_ui.draft.as_ref().unwrap().hwnd;
            unsafe {
                SetWindowTextW(child, PCWSTR(wide(text).as_ptr())).unwrap();
            }
        }
        fn key(&mut self, key: u16) -> bool {
            let hwnd = self
                .app
                .properties_ui
                .draft
                .as_ref()
                .map_or(self.app.editing_window(), |draft| draft.hwnd);
            self.app.properties_message(&MSG {
                hwnd,
                message: WM_KEYDOWN,
                wParam: WPARAM(key as usize),
                ..Default::default()
            })
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.app.properties_ui.draft.take();
            unsafe {
                let _ = DestroyWindow(self.window);
            }
        }
    }

    #[test]
    fn properties_native_pointer_typing_enter_and_document_history() {
        let source = "---\r\nname: 'old' # keep\r\ncount: 0\r\n---\r\nBody **unchanged**\r\n";
        let mut fixture = Fixture::new(source);
        let revision = fixture.app.doc().revision();
        let undo = fixture.app.doc().undo_count();
        fixture.click(Field::Value(0));
        assert_eq!(fixture.app.doc().source(), source);
        assert_eq!(fixture.app.doc().revision(), revision);
        assert!(!fixture.app.tabs[0].raw);
        let child = fixture.app.properties_ui.draft.as_ref().unwrap().hwnd;
        unsafe {
            shell::send(child, WM_CHAR, WPARAM('新' as usize), LPARAM(0));
        }
        assert_eq!(shell::text_of(child), "新");
        assert!(fixture.key(VK_RETURN.0));
        let edited = fixture.app.doc().source().to_owned();
        assert!(edited.contains("name: '新' # keep\r\n"), "{edited}");
        assert!(edited.ends_with("---\r\nBody **unchanged**\r\n"));
        assert_eq!(fixture.app.doc().undo_count(), undo + 1);
        fixture.app.properties_history(false);
        assert_eq!(fixture.app.doc().source(), source);
        fixture.app.properties_gate();
        fixture.app.properties_history(true);
        assert_eq!(fixture.app.doc().source(), edited);
    }

    #[test]
    fn properties_native_invalid_stale_and_escape_preserve_draft() {
        let source = "---\ncount: 42\nname: kept\n---\nBody\n";
        let mut fixture = Fixture::new(source);
        fixture.click(Field::Value(0));
        fixture.replace("not a number");
        assert!(!fixture.app.properties_gate());
        assert_eq!(fixture.app.doc().source(), source);
        assert_eq!(
            shell::text_of(fixture.app.properties_ui.draft.as_ref().unwrap().hwnd),
            "not a number"
        );
        unsafe {
            fixture.app.command(CMD_RAW);
        }
        assert!(!fixture.app.tabs[0].raw);
        fixture.app.new_document();
        assert_eq!(fixture.app.tabs.len(), 1);
        assert!(fixture.key(VK_ESCAPE.0));
        assert_eq!(fixture.app.doc().source(), source);
        fixture.click(Field::Value(1));
        fixture.replace("pending");
        let eof = fixture.app.doc().byte_len();
        fixture
            .app
            .doc_mut()
            .replace_range(eof..eof, "external")
            .unwrap();
        assert!(!fixture.app.properties_gate());
        assert_eq!(
            shell::text_of(fixture.app.properties_ui.draft.as_ref().unwrap().hwnd),
            "pending"
        );
        assert!(fixture.key(VK_ESCAPE.0));
        assert!(fixture.app.doc().source().ends_with("Body\nexternal"));
    }

    #[test]
    fn properties_native_ime_owns_enter_escape_and_blocks_transitions() {
        let source = "---\nname: before\n---\nBody\n";
        let mut fixture = Fixture::new(source);
        fixture.click(Field::Value(0));
        let child = fixture.app.properties_ui.draft.as_ref().unwrap().hwnd;
        unsafe {
            shell::send(child, WM_IME_STARTCOMPOSITION, WPARAM(0), LPARAM(0));
        }
        assert!(fixture.app.properties_composing());
        assert!(!fixture.key(VK_RETURN.0));
        assert!(!fixture.key(VK_ESCAPE.0));
        assert!(!fixture.app.properties_gate());
        assert!(!fixture.app.save_current());
        assert_eq!(fixture.app.doc().source(), source);
        fixture.replace("結果");
        unsafe {
            shell::send(child, WM_IME_ENDCOMPOSITION, WPARAM(0), LPARAM(0));
        }
        assert!(fixture.key(VK_RETURN.0));
        assert!(fixture.app.doc().source().contains("name: '結果'"));
        assert_eq!(fixture.app.doc().undo_count(), 1);
    }

    #[test]
    fn properties_native_tab_key_rename_list_actions_and_bool() {
        let mut fixture =
            Fixture::new("---\nname: old\ntags: [one, two]\nenabled: false\n---\nBody\n");
        fixture.click(Field::Key(0));
        fixture.replace("title");
        assert!(fixture.key(VK_TAB.0));
        assert!(fixture.app.doc().source().contains("'title': old"));
        assert!(matches!(
            fixture.app.properties_ui.focus,
            Some(Field::Value(0))
        ));
        fixture.app.properties_cancel();
        fixture.click(Field::Item(1, 1));
        fixture.replace("日本語");
        assert!(fixture.key(VK_RETURN.0));
        assert!(fixture.app.doc().source().contains("日本語"));
        fixture.app.properties_history(false);
        assert!(matches!(
            fixture.app.properties_ui.focus,
            Some(Field::Value(1))
        ));
        assert!(fixture.app.doc().source().contains("tags: [one, two]"));
        fixture.app.properties_history(true);
        assert!(fixture.app.doc().source().contains("日本語"));
        fixture.click(Field::Value(2));
        assert!(fixture.app.doc().source().contains("enabled: true"));
        fixture.click(Field::AddItem(1));
        fixture.replace("three");
        assert!(fixture.key(VK_RETURN.0));
        fixture.click(Field::DeleteItem(1, 0));
        assert!(!fixture.app.doc().source().contains("one"));
        fixture.app.properties_history(false);
        assert!(fixture.app.doc().source().contains("one"));
        fixture.app.properties_history(true);
        assert!(!fixture.app.doc().source().contains("one"));
        fixture.click(Field::Add);
        fixture.replace("added");
        assert!(fixture.key(VK_RETURN.0));
        assert!(fixture.app.doc().source().contains("'added': ''"));
    }

    #[test]
    fn properties_native_form_geometry_preserves_body_and_delimiter_order() {
        let mut fixture = Fixture::new(
            "---\ntitle: Long title\n# keep\n\ndescription: |\n  First\n  Second\n---\nBody\n",
        );
        let first = fixture.app.properties_ui.rows.first().unwrap().area.top;
        let last = fixture.app.properties_ui.rows.last().unwrap().area.bottom;
        let closing = fixture
            .app
            .properties_ui
            .model
            .as_ref()
            .unwrap()
            .content_range
            .end;
        let opening_view = fixture
            .app
            .views
            .iter()
            .find(|view| view.source.start == 0)
            .unwrap();
        let closing_view = fixture
            .app
            .views
            .iter()
            .find(|view| view.source.start == closing)
            .unwrap();
        let body = fixture
            .app
            .views
            .iter()
            .find(|view| {
                fixture
                    .app
                    .doc()
                    .source()
                    .get(view.source.clone())
                    .is_some_and(|text| text.starts_with("Body"))
            })
            .unwrap();
        assert!(opening_view.y + opening_view.height <= first + 0.1);
        assert!(
            last <= closing_view.y + 0.1,
            "last {last} closing {}",
            closing_view.y
        );
        assert!(closing_view.y + closing_view.height <= body.y + 0.1);
        let revision = fixture.app.doc().revision();
        fixture.click(Field::Value(1));
        assert!(!fixture.key(VK_RETURN.0), "multiline Enter belongs to EDIT");
        fixture.app.properties_cancel();
        assert_eq!(fixture.app.doc().revision(), revision);
    }

    #[test]
    fn properties_native_save_commits_valid_draft_and_reopens_same_value() {
        let source = "---\r\nname: old # unchanged comment\r\n---\r\nBody\r\n";
        let mut fixture = Fixture::new(source);
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mdlite-properties-{}-{unique}.md",
            std::process::id()
        ));
        fixture.app.tabs[0].file =
            Some(FileSnapshot::create_new(&path, source, FileFormat::Utf8 { bom: true }).unwrap());
        fixture.click(Field::Value(0));
        fixture.replace("保存値");
        assert!(fixture.app.save_current());
        assert!(fixture.app.properties_ui.draft.is_none());
        assert!(!fixture.app.doc().is_dirty());
        let loaded = FileSnapshot::open(&path).unwrap();
        assert_eq!(loaded.source, fixture.app.doc().source());
        let document = Document::new(99, loaded.source);
        assert_eq!(
            Properties::parse(&document).unwrap().entries[0].value,
            PropertyValue::Text("保存値".into())
        );
        assert!(
            std::fs::read(&path)
                .unwrap()
                .starts_with(&[0xef, 0xbb, 0xbf])
        );
        fixture.app.properties_history(false);
        assert_eq!(fixture.app.doc().source(), source);
        assert!(fixture.app.doc().is_dirty());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn properties_native_explicit_number_draft_and_raw_fallback_keep_source() {
        let source = "---\nname: '001'\nnested:\n  key: value\n---\nBody\n";
        let mut fixture = Fixture::new(source);
        assert_eq!(
            fixture
                .app
                .properties_ui
                .model
                .as_ref()
                .unwrap()
                .fallbacks
                .len(),
            1
        );
        unsafe {
            fixture.app.properties_begin(Field::Value(0), true);
        }
        assert_eq!(
            fixture.app.doc().source(),
            source,
            "type choice is a draft until valid commit"
        );
        fixture.replace("invalid");
        assert!(!fixture.app.properties_gate());
        assert_eq!(fixture.app.doc().source(), source);
        fixture.replace("12");
        assert!(fixture.key(VK_RETURN.0));
        assert_eq!(
            Properties::parse(fixture.app.doc()).unwrap().entries[0].value,
            PropertyValue::Number("12".into())
        );
        let revision = fixture.app.doc().revision();
        let preserved = fixture.app.doc().source().to_owned();
        fixture.click(Field::Raw(1));
        assert!(fixture.app.tabs[0].raw);
        assert_eq!(fixture.app.doc().source(), preserved);
        assert_eq!(fixture.app.doc().revision(), revision);
    }

    #[test]
    fn properties_native_history_focus_preserves_delimiters_and_value_terminal() {
        for eol in ["\n", "\r\n"] {
            let source = format!("---{eol}name: value{eol}---{eol}Body");
            let mut fixture = Fixture::new(&source);
            let closing = source.rfind("---").unwrap();
            let terminal = source.find("value").unwrap() + "value".len();
            let revision = fixture.app.doc().revision();
            let history = fixture.app.doc().undo_count();
            // Both recognized markers and their terminating EOLs belong to
            // source editing, including the last property/closing boundary.
            for position in [0, 3, closing, closing + 3] {
                fixture
                    .app
                    .doc_mut()
                    .set_selection(position, position)
                    .unwrap();
                fixture.app.properties_restore_history_focus();
                assert_eq!(fixture.app.doc().selection(), Selection::caret(position));
                assert!(fixture.app.properties_ui.draft.is_none());
            }
            // Actual value bytes and its terminal caret still restore the
            // field, so preserving delimiter ownership cannot disable forms.
            for position in [terminal - 1, terminal] {
                fixture
                    .app
                    .doc_mut()
                    .set_selection(position, position)
                    .unwrap();
                fixture.app.properties_restore_history_focus();
                assert!(matches!(
                    fixture.app.properties_ui.focus,
                    Some(Field::Value(0))
                ));
                assert!(fixture.app.properties_ui.draft.is_some());
                fixture.app.properties_cancel();
                assert_eq!(fixture.app.doc().selection(), Selection::caret(position));
            }
            assert_eq!(fixture.app.doc().source(), source);
            assert_eq!(fixture.app.doc().revision(), revision);
            assert_eq!(fixture.app.doc().undo_count(), history);
            assert!(!fixture.app.doc().is_dirty());
        }
    }

    #[test]
    fn properties_native_focus_and_draft_do_not_append_raw_yaml_rows() {
        for eol in ["\n", "\r\n"] {
            let source =
                format!("---{eol}title: value{eol}tags:{eol}  - first{eol}---{eol}Body{eol}");
            for field in [Field::Key(0), Field::Value(0), Field::Item(1, 0)] {
                let mut fixture = Fixture::new(&source);
                let revision = fixture.app.doc().revision();
                let history = fixture.app.doc().undo_count();
                fixture.click(field.clone());
                let child = fixture.app.properties_ui.draft.as_ref().unwrap().hwnd;
                unsafe {
                    shell::send(child, WM_CHAR, WPARAM('新' as usize), LPARAM(0));
                }
                assert!(fixture.app.properties_control(EDIT_ID, EN_CHANGE as u16));
                let content = fixture
                    .app
                    .properties_ui
                    .model
                    .as_ref()
                    .unwrap()
                    .content_range
                    .clone();
                // The form owns these bytes even though its source anchor has
                // no TextView map. Caret fallback must not append a raw copy.
                assert!(
                    !fixture
                        .app
                        .views
                        .iter()
                        .any(|view| view.source.start < content.end
                            && view.source.end > content.start),
                    "ordinary field {field:?} exposed raw YAML"
                );
                assert!(!fixture.app.tabs[0].raw);
                assert_eq!(shell::text_of(child), "新");
                assert_eq!(fixture.app.doc().source(), source);
                assert_eq!(fixture.app.doc().revision(), revision);
                assert_eq!(fixture.app.doc().undo_count(), history);
                fixture.app.properties_cancel();
                // Explicit Source mode retains the original physical lines.
                unsafe {
                    fixture.app.command(CMD_RAW);
                    fixture.app.ensure_layout().unwrap();
                }
                assert!(fixture.app.tabs[0].raw);
                assert!(
                    fixture
                        .app
                        .views
                        .iter()
                        .any(|view| String::from_utf16_lossy(&view.text) == "title: value")
                );
                assert_eq!(fixture.app.doc().source(), source);
                assert_eq!(fixture.app.doc().revision(), revision);
            }
        }
    }

    fn callback_fixture(source: &str) -> super::super::tests::MessagePumpFixture {
        let mut app = super::super::tests::composing_test_app(source);
        app.clear_composition();
        app.tabs[0].raw = false;
        app.tabs[0].scroll = 0.0;
        let fixture = super::super::tests::MessagePumpFixture::new(app);
        unsafe {
            fixture.state.app.borrow_mut().ensure_layout().unwrap();
        }
        fixture
    }

    fn callback_key(
        fixture: &super::super::tests::MessagePumpFixture,
        key: u16,
        ctrl: bool,
    ) -> bool {
        let hwnd = unsafe { GetFocus() };
        fixture
            .state
            .app
            .borrow_mut()
            .properties_message_with_modifiers(
                &MSG {
                    hwnd,
                    message: WM_KEYDOWN,
                    wParam: WPARAM(key as usize),
                    ..Default::default()
                },
                ctrl,
                false,
            )
    }

    fn callback_char(fixture: &super::super::tests::MessagePumpFixture, character: char) {
        let focus = unsafe { GetFocus() };
        assert!(unsafe { IsChild(fixture.hwnd, focus).as_bool() } || focus == fixture.hwnd);
        for unit in character.to_string().encode_utf16() {
            unsafe {
                shell::send(focus, WM_CHAR, WPARAM(unit as usize), LPARAM(0));
            }
        }
    }

    #[test]
    fn properties_native_enter_next_input_cancel_and_save_keep_native_field_owner() {
        for eol in ["\n", "\r\n"] {
            let source = format!("---{eol}name: old{eol}---{eol}Body{eol}");
            let fixture = callback_fixture(&source);
            unsafe {
                fixture
                    .state
                    .app
                    .borrow_mut()
                    .properties_focus(Field::Value(0));
            }
            callback_char(&fixture, 'B');
            assert!(callback_key(&fixture, VK_RETURN.0, false));
            let committed = format!("---{eol}name: 'B'{eol}---{eol}Body{eol}");
            {
                let app = fixture.state.app.borrow();
                assert_eq!(app.doc().source(), committed);
                assert_eq!(app.doc().undo_count(), 1);
                let draft = app
                    .properties_ui
                    .draft
                    .as_ref()
                    .expect("commit leaves a pristine field");
                assert_eq!(unsafe { GetFocus() }, draft.hwnd);
                assert_eq!(draft.original, "B");
                assert!(!app.properties_pending());
            }
            // Actual subsequent child WM_CHAR/EN_CHANGE must extend a draft,
            // not mutate the quote/EOL or snap to a closing delimiter.
            assert!(!callback_key(&fixture, 0x58, false));
            callback_char(&fixture, 'X');
            {
                let app = fixture.state.app.borrow();
                assert_eq!(app.doc().source(), committed);
                assert_eq!(
                    shell::text_of(app.properties_ui.draft.as_ref().unwrap().hwnd),
                    "BX"
                );
                assert_eq!(app.doc().undo_count(), 1);
            }
            assert!(callback_key(&fixture, VK_ESCAPE.0, false));
            callback_char(&fixture, 'Y');
            {
                let app = fixture.state.app.borrow();
                assert_eq!(app.doc().source(), committed);
                assert_eq!(
                    shell::text_of(app.properties_ui.draft.as_ref().unwrap().hwnd),
                    "BY"
                );
                assert_eq!(app.doc().undo_count(), 1);
            }
            // Use the real parent WM_COMMAND save boundary, including its
            // pre-dispatch gate, without a native modal or clipboard mutation.
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mdlite-next-input-{}-{unique}.md",
                std::process::id()
            ));
            fixture.state.app.borrow_mut().tabs[0].file = Some(
                FileSnapshot::create_new(&path, &source, FileFormat::Utf8 { bom: false }).unwrap(),
            );
            unsafe {
                shell::send(fixture.hwnd, WM_COMMAND, WPARAM(CMD_SAVE), LPARAM(0));
            }
            let saved = format!("---{eol}name: 'BY'{eol}---{eol}Body{eol}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
            callback_char(&fixture, 'Z');
            {
                let app = fixture.state.app.borrow();
                assert_eq!(app.doc().source(), saved);
                assert!(!app.doc().is_dirty());
                assert_eq!(
                    shell::text_of(app.properties_ui.draft.as_ref().unwrap().hwnd),
                    "BYZ"
                );
            }
            assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn properties_native_multiline_ctrl_enter_next_input_stays_uncommitted() {
        for eol in ["\n", "\r\n"] {
            let source = format!("---{eol}description: |{eol}  old{eol}---{eol}Body{eol}");
            let fixture = callback_fixture(&source);
            unsafe {
                fixture
                    .state
                    .app
                    .borrow_mut()
                    .properties_focus(Field::Value(0));
            }
            callback_char(&fixture, 'B');
            assert!(!callback_key(&fixture, VK_RETURN.0, false));
            callback_char(&fixture, '\r');
            callback_char(&fixture, 'C');
            assert!(callback_key(&fixture, VK_RETURN.0, true));
            let expected =
                format!("---{eol}description: |2-{eol}  B{eol}  C{eol}---{eol}Body{eol}");
            assert_eq!(fixture.state.app.borrow().doc().source(), expected);
            callback_char(&fixture, 'X');
            let app = fixture.state.app.borrow();
            assert_eq!(app.doc().source(), expected);
            assert_eq!(
                shell::text_of(app.properties_ui.draft.as_ref().unwrap().hwnd),
                "B\r\nCX"
            );
            assert_eq!(app.doc().undo_count(), 1);
        }
    }

    #[test]
    fn properties_native_virtual_focus_blocks_parent_text_clipboard_and_ime_edits() {
        let source = "---\ntags: [one]\npublished: false\n---\nBody\n";
        let fixture = callback_fixture(source);
        unsafe {
            fixture
                .state
                .app
                .borrow_mut()
                .properties_focus(Field::Value(0));
        }
        assert!(fixture.state.app.borrow().properties_ui.draft.is_none());
        unsafe {
            shell::send(fixture.hwnd, WM_CHAR, WPARAM('X' as usize), LPARAM(0));
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            app.paste("PASTE");
            unsafe {
                app.copy(true);
                app.key_with_modifiers(VK_DELETE.0, false, false);
                app.native_notification(NativeNotification::Start);
                app.native_notification(NativeNotification::Update(ImeUpdate {
                    result: Some("IME".into()),
                    flags: GCS_RESULTSTR.0,
                    ..Default::default()
                }));
                app.native_notification(NativeNotification::End);
            }
            assert_eq!(app.doc().source(), source);
            assert_eq!(app.doc().undo_count(), 0);
            assert!(!app.doc().is_dirty());
            // Explicit body pointer selection releases logical field ownership.
            let body = app
                .views
                .iter()
                .find(|view| view.source.start == source.find("Body").unwrap())
                .unwrap();
            let (x, y) = (body.x + 0.1, body.y + body.height * 0.5);
            app.select_at(x, y, false);
            assert!(!app.properties_block_parent_edit());
        }
        unsafe {
            shell::send(fixture.hwnd, WM_CHAR, WPARAM('X' as usize), LPARAM(0));
        }
        assert_eq!(
            fixture.state.app.borrow().doc().source(),
            source.replace("Body", "XBody")
        );
        // Source remains an explicit raw editing route.
        unsafe {
            shell::send(fixture.hwnd, WM_COMMAND, WPARAM(CMD_RAW), LPARAM(0));
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            let position = app.doc().source().find("false").unwrap();
            app.doc_mut().set_selection(position, position).unwrap();
        }
        unsafe {
            shell::send(fixture.hwnd, WM_CHAR, WPARAM('X' as usize), LPARAM(0));
        }
        assert!(
            fixture
                .state
                .app
                .borrow()
                .doc()
                .source()
                .contains("published: Xfalse")
        );
    }

    #[test]
    fn properties_native_new_open_and_close_existing_release_foreign_field_focus() {
        let source_a = "---\nname: old\n---\nBody A\n";
        let source_b = "---\nother: value\n---\nBody B\n";
        let fixture = callback_fixture(source_a);
        unsafe {
            fixture
                .state
                .app
                .borrow_mut()
                .properties_focus(Field::Value(0));
        }
        callback_char(&fixture, 'B');
        assert!(callback_key(&fixture, VK_RETURN.0, false));
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "mdlite-new-open-{}-{unique}.md",
            std::process::id()
        ));
        std::fs::write(&path, source_b).unwrap();
        {
            let mut app = fixture.state.app.borrow_mut();
            app.open_path(path.clone());
            unsafe {
                app.ensure_layout().unwrap();
            }
            assert_eq!(app.doc().source(), source_b);
            assert!(app.properties_ui.focus.is_none());
            assert!(app.properties_ui.focus_document.is_none());
        }
        unsafe {
            shell::send(
                fixture.hwnd,
                WM_KEYDOWN,
                WPARAM(VK_TAB.0 as usize),
                LPARAM(0),
            );
        }
        {
            let app = fixture.state.app.borrow();
            assert_eq!(
                app.doc().source(),
                source_b,
                "Tab must enter B's form, not indent its opener"
            );
            assert!(matches!(app.properties_ui.focus, Some(Field::Key(0))));
            assert_eq!(app.properties_ui.focus_document, Some(app.doc().id()));
        }
        // B is still clean; closing it selects existing dirty A without a modal.
        unsafe {
            shell::send(fixture.hwnd, WM_COMMAND, WPARAM(CMD_CLOSE), LPARAM(0));
        }
        {
            let mut app = fixture.state.app.borrow_mut();
            assert_eq!(
                app.doc().source(),
                source_a.replace("name: old", "name: 'B'")
            );
            assert!(app.properties_ui.focus.is_none());
            assert!(app.properties_ui.focus_document.is_none());
            app.doc_mut().set_selection(0, 0).unwrap();
            unsafe {
                app.after_input();
            }
        }
        unsafe {
            shell::send(
                fixture.hwnd,
                WM_KEYDOWN,
                WPARAM(VK_TAB.0 as usize),
                LPARAM(0),
            );
        }
        {
            let app = fixture.state.app.borrow();
            assert!(matches!(app.properties_ui.focus, Some(Field::Key(0))));
            assert!(app.doc().source().starts_with("---\nname: 'B'"));
        }
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn properties_native_stale_escape_after_reorder_or_delete_never_reuses_index() {
        for replacement in [
            "---\nb: two\na: one\n---\nBody\n",
            "---\nb: two\n---\nBody\n",
        ] {
            let fixture = callback_fixture("---\na: one\nb: two\n---\nBody\n");
            unsafe {
                fixture
                    .state
                    .app
                    .borrow_mut()
                    .properties_focus(Field::Value(0));
            }
            callback_char(&fixture, 'X');
            let (revision, history) = {
                let mut app = fixture.state.app.borrow_mut();
                let end = app.doc().byte_len();
                app.doc_mut().replace_range(0..end, replacement).unwrap();
                (app.doc().revision(), app.doc().undo_count())
            };
            assert!(callback_key(&fixture, VK_ESCAPE.0, false));
            {
                let app = fixture.state.app.borrow();
                assert!(app.properties_ui.draft.is_none());
                assert!(
                    app.properties_ui.focus.is_none(),
                    "stale Field::Value(0) must not become b"
                );
                assert_eq!(
                    app.doc().selection(),
                    Selection::caret(replacement.find("Body").unwrap())
                );
                assert_eq!(app.doc().source(), replacement);
                assert_eq!(app.doc().revision(), revision);
                assert_eq!(app.doc().undo_count(), history);
            }
        }
    }

    #[test]
    fn properties_native_whole_document_copy_is_readonly_and_cut_stays_guarded() {
        let source = "---\ntags: [one]\n---\nBody\n";
        let fixture = callback_fixture(source);
        unsafe {
            fixture
                .state
                .app
                .borrow_mut()
                .properties_focus(Field::Value(0));
        }
        let before_revision = fixture.state.app.borrow().doc().revision();
        let before_sequence = unsafe { GetClipboardSequenceNumber() };
        // Own a temporary read-only clipboard lock on another thread. Copy's
        // normal clipboard-open boundary will fail without EmptyClipboard or
        // changing any user clipboard data; its routing/source contract is real.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || unsafe {
            let opened = OpenClipboard(None).is_ok();
            let _ = ready_tx.send(opened);
            if opened {
                // Explicit release or sender drop (including panic unwinding)
                // closes the read-only lock; scheduling cannot unlock it early.
                let _ = release_rx.recv();
                let _ = CloseClipboard();
            }
        });
        let opened = ready_rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        if !opened {
            let _ = release_tx.send(());
            holder.join().unwrap();
            panic!("clipboard fixture lock unavailable; no clipboard copy attempted");
        }
        let (copy_status, source_after, revision, history, cut_status) = {
            let mut app = fixture.state.app.borrow_mut();
            unsafe {
                app.key_with_modifiers(0x41, true, false);
            }
            assert_eq!(app.doc().selection().range(), 0..source.len());
            app.status = "read-only copy route".into();
            unsafe {
                app.copy(false);
            }
            let copy_status = app.status.clone();
            unsafe {
                app.copy(true);
            }
            (
                copy_status,
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().undo_count(),
                app.status.clone(),
            )
        };
        let _ = release_tx.send(());
        holder.join().unwrap();
        assert!(
            opened,
            "clipboard fixture lock unavailable; no clipboard writes attempted"
        );
        assert_eq!(
            copy_status, "read-only copy route",
            "copy must reach its normal clipboard boundary"
        );
        assert!(cut_status.contains("プロパティ"));
        assert_eq!(source_after, source);
        assert_eq!(revision, before_revision);
        assert_eq!(history, 0);
        assert_eq!(unsafe { GetClipboardSequenceNumber() }, before_sequence);
    }

    #[test]
    fn properties_native_missing_active_document_blocks_without_dropping_draft() {
        let source = "---\nname: kept\n---\nBody\n";
        let mut fixture = Fixture::new(source);
        fixture.app.active = usize::MAX;
        assert!(!fixture.app.properties_gate());
        assert!(fixture.app.properties_block_parent_edit());
        fixture.app.properties_resume_field();
        assert!(!fixture.app.properties_tab_entry(false));
        fixture.app.active = 0;
        fixture.click(Field::Value(0));
        fixture.replace("pending");
        let draft = fixture.app.properties_ui.draft.as_ref().unwrap().hwnd;
        fixture.app.active = usize::MAX;
        assert!(!fixture.app.properties_gate());
        assert_eq!(
            fixture.app.properties_ui.draft.as_ref().unwrap().hwnd,
            draft
        );
        fixture.app.active = 0;
        // Empty tabs with an orphan draft are not the pristine startup case.
        let tabs = std::mem::take(&mut fixture.app.tabs);
        assert!(!fixture.app.properties_gate());
        assert_eq!(
            fixture.app.properties_ui.draft.as_ref().unwrap().hwnd,
            draft
        );
        fixture.app.tabs = tabs;
        assert_eq!(shell::text_of(draft), "pending");
        assert_eq!(fixture.app.doc().source(), source);
        assert_eq!(fixture.app.doc().undo_count(), 0);
    }
}

pub(super) struct Row {
    field: Field,
    text: String,
    area: D2D_RECT_F,
}

struct Draft {
    model: Properties,
    field: Field,
    original: String,
    hwnd: HWND,
    composing: Box<Cell<bool>>,
    multiline: bool,
    number: bool,
    before_selection: Selection,
    before_affinity: BoundaryAffinity,
}

#[derive(Default)]
pub(super) struct PropertiesUi {
    pub(super) model: Option<Properties>,
    pub(super) rows: Vec<Row>,
    focus: Option<Field>,
    focus_document: Option<u64>,
    draft: Option<Draft>,
    error: String,
    multiline_field: Option<(u64, usize)>,
    error_area: Option<D2D_RECT_F>,
}

impl Drop for Draft {
    fn drop(&mut self) {
        unsafe {
            // Destroy before releasing the subclass's stable composition cell.
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

unsafe extern "system" fn draft_proc(
    hwnd: HWND,
    message: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let composing = &*(data as *const Cell<bool>);
        match message {
            WM_IME_STARTCOMPOSITION => composing.set(true),
            WM_IME_ENDCOMPOSITION => {
                let result = DefSubclassProc(hwnd, message, wp, lp);
                composing.set(false);
                return result;
            }
            WM_UNDO | EM_UNDO => {
                if !composing.get()
                    && let Ok(parent) = GetParent(hwnd)
                {
                    let _ = PostMessageW(Some(parent), WM_COMMAND, WPARAM(HISTORY_UNDO), LPARAM(0));
                }
                return LRESULT(0);
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(draft_proc), id);
            }
            _ => {}
        }
        DefSubclassProc(hwnd, message, wp, lp)
    }
}

fn value_text(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Text(text) | PropertyValue::Number(text) => text.clone(),
        PropertyValue::Boolean(value) => value.to_string(),
        PropertyValue::Null => "null".into(),
        PropertyValue::List(_) => String::new(),
    }
}

fn draft_edit(draft: &Draft, text: String) -> Result<PropertyEdit, String> {
    match draft.field {
        Field::Key(index) => Ok(PropertyEdit::Rename { index, key: text }),
        Field::Item(index, item) => Ok(PropertyEdit::SetListItem {
            index,
            item,
            value: text,
        }),
        Field::Value(index) => {
            let old = &draft.model.entries[index].value;
            if draft.number {
                return Ok(PropertyEdit::SetValue {
                    index,
                    value: PropertyValue::Number(text),
                });
            }
            let value = match old {
                PropertyValue::Text(_) => PropertyValue::Text(text),
                PropertyValue::Number(_) => PropertyValue::Number(text),
                PropertyValue::Null if text == "null" => PropertyValue::Null,
                PropertyValue::Null => {
                    return Err("null の変更は値の型メニューから選択してください".into());
                }
                _ => return Err("この値は専用の操作で変更してください".into()),
            };
            Ok(PropertyEdit::SetValue { index, value })
        }
        _ => Err("編集対象がありません".into()),
    }
}

impl App {
    pub(super) fn properties_test_geometry(&self) -> serde_json::Value {
        let scroll = self.tabs[self.active].scroll;
        serde_json::json!({
            "document_id": self.properties_ui.model.as_ref().map(|model| model.document_id),
            "revision": self.properties_ui.model.as_ref().map(|model| model.revision),
            "range": self.properties_ui.model.as_ref().map(|model| model.range.clone()),
            "rows": self.properties_ui.rows.iter().map(|row| serde_json::json!({
                "field": format!("{:?}",row.field), "text":row.text,
                "x":row.area.left,"y":row.area.top-scroll,
                "width":row.area.right-row.area.left,"height":row.area.bottom-row.area.top
            })).collect::<Vec<_>>(),
            "draft":self.properties_ui.draft.as_ref().map(|draft| serde_json::json!({
                "field": format!("{:?}",draft.field), "hwnd":draft.hwnd.0 as usize,
                "composing":draft.composing.get(), "text":shell::text_of(draft.hwnd), "multiline":draft.multiline
            })),
            "error":self.properties_ui.error
        })
    }
    pub(super) fn properties_marker_anchors(&self) -> Vec<(usize, f32)> {
        match (
            &self.properties_ui.model,
            self.properties_ui.rows.first(),
            self.properties_ui.rows.last(),
        ) {
            (Some(model), Some(first), Some(last)) => vec![
                (model.range.start, first.area.top - 8.0),
                (model.content_range.end, last.area.bottom + 10.0),
            ],
            _ => Vec::new(),
        }
    }

    pub(super) fn properties_shift_markers(&mut self, positions: &[usize], offsets: &[f32]) {
        let Some(model) = &self.properties_ui.model else {
            return;
        };
        let shift = offsets[positions.partition_point(|position| *position <= model.range.start)];
        for row in &mut self.properties_ui.rows {
            row.area.top += shift;
            row.area.bottom += shift;
        }
        if let Some(area) = self.properties_ui.error_area.as_mut() {
            area.top += shift;
            area.bottom += shift;
        }
    }
    pub(super) fn properties_tab_entry(&mut self, back: bool) -> bool {
        if self.tabs.get(self.active).is_none() {
            self.properties_release_focus();
            return false;
        }
        if self.properties_ui.focus_document != Some(self.doc().id()) {
            self.properties_release_focus();
        }
        if self.tabs[self.active].raw
            || self.feature_ime_active()
            || self.properties_ui.focus.is_some()
        {
            return false;
        }
        let Some(model) = &self.properties_ui.model else {
            return false;
        };
        if !model.range.contains(&self.doc().selection().head) {
            return false;
        }
        let field = if back {
            self.properties_ui.rows.last()
        } else {
            self.properties_ui
                .rows
                .iter()
                .find(|row| matches!(row.field, Field::Key(_)))
                .or(self.properties_ui.rows.first())
        }
        .map(|row| row.field.clone());
        if let Some(field) = field {
            unsafe {
                self.properties_focus(field);
            }
            return true;
        }
        false
    }
    pub(super) fn properties_release_focus(&mut self) {
        self.properties_ui.focus = None;
        self.properties_ui.focus_document = None;
    }

    pub(super) fn properties_resume_field(&mut self) {
        let Some(tab) = self.tabs.get(self.active) else {
            return;
        };
        if tab.raw || self.properties_ui.focus_document != Some(tab.doc.id()) {
            return;
        }
        let Some(field) = self.properties_ui.focus.clone() else {
            return;
        };
        unsafe {
            self.properties_focus(field);
            if let Some(draft) = &self.properties_ui.draft {
                // Keep subsequent native editing at the decoded value end,
                // rather than selecting and replacing the committed value.
                let end = GetWindowTextLengthW(draft.hwnd).max(0) as usize;
                shell::send(draft.hwnd, EM_SETSEL, WPARAM(end), LPARAM(end as isize));
            }
        }
    }

    fn properties_exit_to_body(&mut self) {
        let point = self.properties_ui.model.as_ref().and_then(|model| {
            self.views
                .iter()
                .find(|view| {
                    view.source.start >= model.range.end
                        && self
                            .projection_cache
                            .as_ref()
                            .is_none_or(|(_, _, projection)| {
                                projection
                                    .source_delimiter_line_at(view.source.start)
                                    .is_none()
                            })
                })
                .and_then(|view| view.map.first().copied())
        });
        if let Some(point) = point {
            self.properties_release_focus();
            let _ = self.doc_mut().set_selection(point, point);
            unsafe {
                let _ = SetFocus(Some(self.editing_window()));
                self.after_input();
            }
        } else if self.properties_ui.model.is_some() {
            // A front-matter-only document has no body target yet. Keep a
            // neutral virtual control, never a stale property ordinal/caret.
            self.properties_ui.focus = Some(Field::Add);
            self.properties_ui.focus_document = Some(self.doc().id());
        }
    }

    /// A parent surface cannot edit bytes represented by a property field or
    /// consume text input while a document-bound virtual form control owns it.
    pub(super) fn properties_block_parent_edit(&mut self) -> bool {
        let Some(tab) = self.tabs.get(self.active) else {
            return true;
        };
        if tab.raw {
            return false;
        }
        let selection = self.doc().selection().range();
        let owned = self.properties_ui.focus_document == Some(self.doc().id())
            && self.properties_ui.focus.is_some()
            || self
                .properties_ui
                .draft
                .as_ref()
                .is_some_and(|draft| draft.model.document_id == self.doc().id())
            || self.properties_ui.model.as_ref().is_some_and(|model| {
                model.document_id == self.doc().id()
                    && model.revision == self.doc().revision()
                    && if selection.is_empty() {
                        model.content_range.contains(&selection.start)
                    } else {
                        selection.start < model.content_range.end
                            && selection.end > model.content_range.start
                    }
            });
        if owned {
            self.pending_surrogate = None;
            self.status = "プロパティの値を選択して入力してください".into();
            self.properties_ui.error = self.status.clone();
        }
        owned
    }

    pub(super) fn properties_composing(&self) -> bool {
        self.properties_ui
            .draft
            .as_ref()
            .is_some_and(|draft| draft.composing.get())
    }

    pub(super) fn properties_pending(&self) -> bool {
        self.properties_ui.draft.as_ref().is_some_and(|draft| {
            draft.composing.get() || shell::text_of(draft.hwnd) != draft.original
        })
    }

    /// False means the caller must leave the current operation and draft intact.
    pub(super) fn properties_gate(&mut self) -> bool {
        let current = self
            .tabs
            .get(self.active)
            .map(|tab| (tab.doc.id(), tab.raw));
        let Some((document, raw)) = current else {
            // Startup opens/creates the first document before WindowState and
            // its native callbacks exist. That pristine bootstrap may proceed;
            // an invalid active index or orphan draft must stay blocked.
            if self.tabs.is_empty() && self.properties_ui.draft.is_none() {
                self.properties_release_focus();
                return true;
            }
            self.properties_ui.error = "編集中の文書を確認できません。入力は保持しています".into();
            return false;
        };
        let Some(draft) = self.properties_ui.draft.take() else {
            if raw || self.properties_ui.focus_document != Some(document) {
                self.properties_release_focus();
            }
            return true;
        };
        let outcome = if draft.composing.get() {
            Err("IME入力を確定または取消してください".to_owned())
        } else if draft.model.document_id != self.doc().id()
            || draft.model.revision != self.doc().revision()
        {
            Err(
                "原文が変わりました。入力は保持しています。取消して現在の値を開き直してください"
                    .into(),
            )
        } else {
            let text = shell::text_of(draft.hwnd).replace("\r\n", "\n");
            if text == draft.original && !draft.number {
                Ok(false)
            } else {
                draft_edit(&draft, text)
                    .and_then(|edit| {
                        draft
                            .model
                            .prepare(self.doc(), edit)
                            .map_err(|e| e.to_string())
                    })
                    .and_then(|transaction| {
                        transaction.apply(self.doc_mut()).map_err(|e| e.to_string())
                    })
            }
        };
        match outcome {
            Ok(changed) => {
                let had_focus = unsafe { GetFocus() == draft.hwnd };
                self.properties_ui.focus = Some(draft.field.clone());
                self.properties_ui.focus_document = Some(draft.model.document_id);
                drop(draft);
                self.properties_ui.error.clear();
                if changed {
                    self.layout_key = None;
                    self.projection_cache = None;
                }
                if had_focus {
                    unsafe {
                        let _ = SetFocus(Some(self.editing_window()));
                    }
                }
                true
            }
            Err(error) => {
                self.properties_ui.error = error;
                let child = draft.hwnd;
                self.properties_ui.draft = Some(draft);
                self.layout_key = None;
                unsafe {
                    let _ = SetFocus(Some(child));
                    self.refresh();
                }
                false
            }
        }
    }

    fn properties_cancel(&mut self) {
        if let Some(draft) = self.properties_ui.draft.take()
            && (draft.model.document_id, draft.model.revision)
                == (self.doc().id(), self.doc().revision())
        {
            let _ = self.doc_mut().set_selection_with_affinity(
                draft.before_selection.anchor,
                draft.before_selection.head,
                draft.before_affinity,
            );
        }
        self.properties_ui.error.clear();
        self.properties_release_focus();
        unsafe {
            let _ = SetFocus(Some(self.editing_window()));
            self.refresh();
        }
    }

    fn properties_apply(&mut self, model: &Properties, edit: PropertyEdit) -> bool {
        let anchor = match &edit {
            PropertyEdit::Rename { index, .. } => {
                model.entries.get(*index).map(|entry| entry.key_range.start)
            }
            PropertyEdit::SetValue { index, .. } | PropertyEdit::AddListItem { index, .. } => model
                .entries
                .get(*index)
                .map(|entry| entry.value_range.start),
            PropertyEdit::SetListItem { index, item, .. }
            | PropertyEdit::DeleteListItem { index, item } => {
                model.entries.get(*index).map(|entry| {
                    entry
                        .list_item_range(*item)
                        .map_or(entry.value_range.start, |range| range.start)
                })
            }
            PropertyEdit::Delete { index } => {
                model.entries.get(*index).map(|entry| entry.range.start)
            }
            PropertyEdit::Add { .. } => Some(model.content_range.end),
        };
        let result = model.prepare(self.doc(), edit).and_then(|transaction| {
            if let Some(anchor) = anchor {
                let _ = self.doc_mut().set_selection(anchor, anchor);
            }
            transaction.apply(self.doc_mut())
        });
        match result {
            Ok(changed) => {
                self.properties_ui.error.clear();
                if changed {
                    self.projection_cache = None;
                    self.layout_key = None;
                    unsafe {
                        self.refresh();
                    }
                }
                true
            }
            Err(error) => {
                self.properties_ui.error = error.to_string();
                unsafe {
                    self.refresh();
                }
                false
            }
        }
    }

    unsafe fn properties_focus(&mut self, field: Field) {
        unsafe {
            self.properties_begin(field, false);
        }
    }

    unsafe fn properties_begin(&mut self, field: Field, number: bool) {
        unsafe {
            if !self.properties_gate() {
                return;
            }
            let _ = self.ensure_layout();
            let Some(model) = self.properties_ui.model.clone() else {
                return;
            };
            let Some(row) = self
                .properties_ui
                .rows
                .iter()
                .find(|row| row.field == field)
            else {
                return;
            };
            let text = match field {
                Field::Key(index) => model.entries[index].key.clone(),
                Field::Value(index) => value_text(&model.entries[index].value),
                Field::Item(index, item) => match &model.entries[index].value {
                    PropertyValue::List(items) => items[item].clone(),
                    _ => return,
                },
                _ => row.text.clone(),
            };
            let before_selection = self.doc().selection();
            let before_affinity = self.doc().selection_affinity();
            let row_top = row.area.top;
            let row_bottom = row.area.bottom;
            let multiline = !number
                && match field {
                    Field::Value(index) => {
                        matches!(&model.entries[index].value, PropertyValue::Text(text) if text.contains('\n') || model.entries[index].key == "description" || self.properties_ui.multiline_field == Some((model.document_id, index)))
                    }
                    _ => false,
                };
            self.properties_ui.focus = Some(field.clone());
            self.properties_ui.focus_document = Some(model.document_id);
            let editable = number
                || matches!(field, Field::Key(_) | Field::Item(_, _))
                || matches!(field, Field::Value(index) if !matches!(model.entries[index].value, PropertyValue::Boolean(_) | PropertyValue::List(_) | PropertyValue::Null));
            let page_top = self.editor_top() + self.tabs[self.active].scroll;
            let page_bottom = self.editor_bottom() + self.tabs[self.active].scroll;
            if row_top < page_top {
                self.scroll(row_top - page_top);
            }
            if row_bottom > page_bottom {
                self.scroll(row_bottom - page_bottom);
            }
            if editable {
                let wrap = multiline
                    || matches!(field, Field::Value(index) if model.entries[index].key == "title");
                let style = WINDOW_STYLE(
                    ES_LEFT as u32
                        | if wrap {
                            ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | ES_WANTRETURN as u32
                        } else {
                            ES_AUTOHSCROLL as u32
                        },
                );
                let hwnd = shell::control(
                    self.editing_window(),
                    w!("EDIT"),
                    &text.replace('\n', "\r\n"),
                    EDIT_ID,
                    style | WS_BORDER,
                );
                if hwnd == HWND::default() {
                    self.properties_ui.error = "入力欄を作成できませんでした".into();
                    return;
                }
                let composing = Box::new(Cell::new(false));
                if !SetWindowSubclass(
                    hwnd,
                    Some(draft_proc),
                    CHILD_SUBCLASS,
                    (&*composing as *const Cell<bool>) as usize,
                )
                .as_bool()
                {
                    let _ = DestroyWindow(hwnd);
                    self.properties_ui.error = "入力欄のIME管理を開始できませんでした".into();
                    return;
                }
                shell::send(hwnd, EM_SETLIMITTEXT, WPARAM(256 * 1024), LPARAM(0));
                let anchor = match field {
                    Field::Key(index) => model.entries[index].key_range.start,
                    Field::Value(index) => model.entries[index].value_range.start,
                    Field::Item(index, item) => model.entries[index]
                        .list_item_range(item)
                        .map_or(model.entries[index].value_range.start, |range| range.start),
                    _ => self.doc().selection().head,
                };
                let _ = self.doc_mut().set_selection(anchor, anchor);
                self.properties_ui.draft = Some(Draft {
                    model,
                    field,
                    original: text,
                    hwnd,
                    composing,
                    multiline,
                    number,
                    before_selection,
                    before_affinity,
                });
                self.properties_position();
                let _ = SetFocus(Some(hwnd));
                shell::send(hwnd, EM_SETSEL, WPARAM(0), LPARAM(-1));
            } else {
                let _ = SetFocus(Some(self.editing_window()));
            }
            self.refresh();
        }
    }

    fn properties_activate(&mut self, field: Field) {
        if !self.properties_gate() {
            return;
        }
        let _ = unsafe { self.ensure_layout() };
        let Some(model) = self.properties_ui.model.clone() else {
            return;
        };
        self.properties_ui.focus = Some(field.clone());
        self.properties_ui.focus_document = Some(model.document_id);
        let edit = match field.clone() {
            Field::Delete(index) => Some(PropertyEdit::Delete { index }),
            Field::Type(index) => {
                unsafe {
                    self.properties_type_menu(index);
                }
                return;
            }
            Field::AddItem(index) => Some(PropertyEdit::AddListItem {
                index,
                value: String::new(),
            }),
            Field::DeleteItem(index, item) => Some(PropertyEdit::DeleteListItem { index, item }),
            Field::Value(index)
                if matches!(model.entries[index].value, PropertyValue::Boolean(_)) =>
            {
                let PropertyValue::Boolean(value) = model.entries[index].value else {
                    unreachable!()
                };
                Some(PropertyEdit::SetValue {
                    index,
                    value: PropertyValue::Boolean(!value),
                })
            }
            Field::Add => {
                let mut key = "property".to_owned();
                let mut n = 1;
                while model
                    .prepare(
                        self.doc(),
                        PropertyEdit::Add {
                            key: key.clone(),
                            value: PropertyValue::Text(String::new()),
                        },
                    )
                    .is_err()
                    && n <= 2048
                {
                    key = format!("property{n}");
                    n += 1;
                }
                Some(PropertyEdit::Add {
                    key,
                    value: PropertyValue::Text(String::new()),
                })
            }
            Field::Raw(_) => {
                let _ = self
                    .doc_mut()
                    .set_selection(model.range.start, model.range.start);
                self.tabs[self.active].raw = true;
                self.layout_key = None;
                unsafe {
                    self.after_input();
                }
                return;
            }
            _ => {
                unsafe {
                    self.properties_focus(field);
                }
                return;
            }
        };
        if let Some(edit) = edit {
            let adding = matches!(field, Field::Add);
            let adding_item = match field {
                Field::AddItem(index) => Some(index),
                _ => None,
            };
            if self.properties_apply(&model, edit) {
                if adding {
                    let count = self
                        .properties_ui
                        .model
                        .as_ref()
                        .map_or(0, |model| model.entries.len());
                    if count > 0 {
                        unsafe {
                            self.properties_focus(Field::Key(count - 1));
                        }
                    }
                } else if let Some(index) = adding_item {
                    if let Some(PropertyValue::List(items)) = self
                        .properties_ui
                        .model
                        .as_ref()
                        .and_then(|model| model.entries.get(index))
                        .map(|entry| &entry.value)
                    {
                        let last = items.len().saturating_sub(1);
                        unsafe {
                            self.properties_focus(Field::Item(index, last));
                        }
                    }
                } else {
                    self.properties_ui.focus = Some(
                        if self.properties_ui.rows.iter().any(|row| row.field == field) {
                            field
                        } else {
                            Field::Add
                        },
                    );
                    self.properties_ui.focus_document = Some(self.doc().id());
                }
            }
        }
    }

    /// Type changes are explicit; plain text that resembles a date/number stays text.
    unsafe fn properties_type_menu(&mut self, index: usize) {
        unsafe {
            if !self.properties_gate() {
                return;
            }
            let _ = self.ensure_layout();
            let Some(model) = self.properties_ui.model.clone() else {
                return;
            };
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            for (id, label) in [
                (1, "文字列"),
                (2, "複数行文字列"),
                (3, "数値"),
                (4, "true / false"),
                (5, "リスト"),
                (6, "null"),
            ] {
                let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide(label).as_ptr()));
            }
            let mut position = POINT::default();
            let _ = GetCursorPos(&mut position);
            let chosen = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                position.x,
                position.y,
                None,
                self.hwnd,
                None,
            )
            .0;
            let _ = DestroyMenu(menu);
            let old = value_text(&model.entries[index].value);
            if chosen == 3 {
                self.properties_begin(Field::Value(index), true);
                return;
            }
            let value = match chosen {
                1 | 2 => PropertyValue::Text(old),
                4 => PropertyValue::Boolean(false),
                5 => PropertyValue::List(vec![old]),
                6 => PropertyValue::Null,
                _ => return,
            };
            self.properties_ui.multiline_field =
                (chosen == 2).then_some((model.document_id, index));
            if self.properties_apply(&model, PropertyEdit::SetValue { index, value })
                && matches!(chosen, 1..=3)
            {
                self.properties_focus(Field::Value(index));
            }
        }
    }

    pub(super) fn properties_message(&mut self, message: &MSG) -> bool {
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) < 0 };
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) < 0 };
        self.properties_message_with_modifiers(message, ctrl, shift)
    }

    fn properties_message_with_modifiers(
        &mut self,
        message: &MSG,
        ctrl: bool,
        shift: bool,
    ) -> bool {
        if self.tabs[self.active].raw {
            return false;
        }
        if !matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
            return false;
        }
        let child = self.properties_ui.draft.as_ref().map(|draft| draft.hwnd);
        let owns_focus = child == Some(message.hwnd)
            || (message.hwnd == self.editing_window()
                && self.properties_ui.focus_document == Some(self.doc().id())
                && self.properties_ui.focus.is_some());
        if !owns_focus {
            return false;
        }
        if self.properties_composing() || message.wParam.0 == VK_PROCESSKEY.0 as usize {
            return false;
        }
        let key = message.wParam.0 as u16;
        if let Some(redo) = history_shortcut(key, ctrl, shift) {
            self.properties_history(redo);
            return true;
        }
        if key == VK_ESCAPE.0 {
            let field = self.properties_ui.focus.clone();
            let resume = self.properties_ui.draft.as_ref().is_some_and(|draft| {
                (draft.model.document_id, draft.model.revision)
                    == (self.doc().id(), self.doc().revision())
            });
            self.properties_cancel();
            if resume {
                self.properties_ui.focus = field;
                self.properties_ui.focus_document = Some(self.doc().id());
                self.properties_resume_field();
            } else {
                // A stale ordinal may now name another key after reorder or
                // deletion. Exit to a real body point instead of reusing it.
                self.properties_exit_to_body();
            }
            return true;
        }
        if key == VK_TAB.0 && !ctrl {
            let current = self.properties_ui.focus.clone();
            if !self.properties_gate() {
                return true;
            }
            let _ = unsafe { self.ensure_layout() };
            let fields: Vec<_> = self
                .properties_ui
                .rows
                .iter()
                .map(|row| row.field.clone())
                .collect();
            let position = fields
                .iter()
                .position(|field| Some(field) == current.as_ref())
                .unwrap_or(0);
            let next = if shift {
                position.checked_sub(1)
            } else {
                Some(position + 1).filter(|next| *next < fields.len())
            };
            if let Some(next) = next {
                unsafe {
                    self.properties_focus(fields[next].clone());
                }
            } else {
                self.properties_exit_to_body();
            }
            return true;
        }
        if key == VK_RETURN.0 {
            if self
                .properties_ui
                .draft
                .as_ref()
                .is_some_and(|draft| draft.multiline)
                && !ctrl
            {
                return false;
            }
            if let Some(field) = self.properties_ui.focus.clone() {
                if child.is_some() {
                    if self.properties_gate() {
                        unsafe {
                            self.refresh();
                        }
                        // Commit is complete; a pristine native field owns the
                        // next character, clipboard operation and IME session.
                        self.properties_resume_field();
                    }
                } else {
                    self.properties_activate(field);
                }
            }
            return true;
        }
        if key == VK_SPACE.0 && child.is_none() {
            if let Some(field) = self.properties_ui.focus.clone() {
                self.properties_activate(field);
            }
            return true;
        }
        if (key == VK_F2.0 || (key == VK_DOWN.0 && unsafe { GetKeyState(VK_MENU.0 as i32) < 0 }))
            && let Some(Field::Value(index) | Field::Key(index) | Field::Item(index, _)) =
                self.properties_ui.focus.clone()
        {
            unsafe {
                self.properties_type_menu(index);
            }
            return true;
        }
        if ctrl && matches!(key, 0x53 | 0x57 | 0x45 | 0x09) {
            if !self.properties_gate() {
                return true;
            }
            unsafe {
                match key {
                    0x53 => self.command(if shift { CMD_SAVE_AS } else { CMD_SAVE }),
                    0x57 => self.command(CMD_CLOSE),
                    0x45 => self.command(CMD_RAW),
                    _ => self.switch_tab((self.active + 1) % self.tabs.len()),
                }
                self.refresh();
            }
            return true;
        }
        false
    }

    pub(super) fn properties_history(&mut self, redo: bool) {
        if !self.properties_gate() {
            return;
        }
        if redo {
            self.doc_mut().redo();
        } else {
            self.doc_mut().undo();
        }
        self.layout_key = None;
        unsafe {
            self.refresh();
        }
        self.properties_restore_history_focus();
    }

    pub(super) fn properties_restore_history_focus(&mut self) {
        if self.tabs[self.active].raw || self.properties_ui.draft.is_some() {
            return;
        }
        let _ = unsafe { self.ensure_layout() };
        let head = self.doc().selection().head;
        // Recognized delimiters retain native source/caret ownership. In
        // particular, the final property's range ends at the closing line.
        if self
            .cached_projection()
            .source_delimiter_line_at(head)
            .is_some()
        {
            return;
        }
        let Some(model) = &self.properties_ui.model else {
            return;
        };
        let source = self.doc().source();
        let field = model
            .entries
            .iter()
            .enumerate()
            .find_map(|(index, entry)| {
                // Keep the value's terminal caret, but not physical EOL bytes
                // included by multiline/list source ranges or the next line.
                let value_end = entry.value_range.start
                    + source[entry.value_range.clone()]
                        .trim_end_matches(['\r', '\n'])
                        .len();
                if entry.key_range.start <= head && head <= entry.key_range.end {
                    Some(Field::Key(index))
                } else if entry.value_range.start <= head && head <= value_end {
                    if let PropertyValue::List(items) = &entry.value
                        && let Some(item) = (0..items.len()).find(|&item| {
                            entry
                                .list_item_range(item)
                                .is_some_and(|range| range.start <= head && head <= range.end)
                        })
                    {
                        return Some(Field::Item(index, item));
                    }
                    Some(Field::Value(index))
                } else {
                    None
                }
            })
            .or_else(|| (head == model.content_range.end).then_some(Field::Add));
        if let Some(field) = field {
            unsafe {
                self.properties_focus(field);
            }
        }
    }

    pub(super) fn properties_control(&mut self, id: usize, notification: u16) -> bool {
        match id {
            EDIT_ID => {
                if notification == EN_CHANGE as u16 && self.properties_ui.draft.is_some() {
                    self.layout_key = None;
                    unsafe {
                        self.refresh();
                    }
                }
                true
            }
            HISTORY_UNDO | HISTORY_REDO => {
                self.properties_history(id == HISTORY_REDO);
                true
            }
            _ => false,
        }
    }

    pub(super) fn properties_click(&mut self, x: f32, y: f32, right: bool) -> bool {
        if y < self.editor_top() || y >= self.editor_bottom() {
            return false;
        }
        let position = y + self.tabs[self.active].scroll;
        let field = self
            .properties_ui
            .rows
            .iter()
            .find(|row| {
                x >= row.area.left
                    && x < row.area.right
                    && position >= row.area.top
                    && position < row.area.bottom
            })
            .map(|row| row.field.clone());
        let Some(field) = field else {
            // Spacing inside the form is not an implicit raw/body edit target.
            return self
                .properties_ui
                .rows
                .first()
                .zip(self.properties_ui.rows.last())
                .is_some_and(|(first, last)| {
                    x >= self.content_left()
                        && x < self.editor_right()
                        && position >= first.area.top
                        && position < last.area.bottom
                });
        };
        if right {
            if let Field::Key(index) | Field::Value(index) | Field::Item(index, _) = field {
                unsafe {
                    self.properties_type_menu(index);
                }
            }
        } else {
            self.properties_activate(field);
        }
        true
    }

    pub(super) unsafe fn properties_position(&self) {
        unsafe {
            let Some(draft) = &self.properties_ui.draft else {
                return;
            };
            let Some(row) = self
                .properties_ui
                .rows
                .iter()
                .find(|row| row.field == draft.field)
            else {
                return;
            };
            let top = row.area.top - self.tabs[self.active].scroll;
            let bottom = row.area.bottom - self.tabs[self.active].scroll;
            let visible = bottom > self.editor_top() && top < self.editor_bottom();
            let clipped_top = top.max(self.editor_top());
            let clipped_bottom = bottom.min(self.editor_bottom());
            let _ = ShowWindow(draft.hwnd, if visible { SW_SHOWNA } else { SW_HIDE });
            let _ = SetWindowPos(
                draft.hwnd,
                None,
                (row.area.left * self.dpi) as i32,
                (clipped_top * self.dpi) as i32,
                ((row.area.right - row.area.left) * self.dpi) as i32,
                ((clipped_bottom - clipped_top).max(1.0) * self.dpi) as i32,
                SWP_NOACTIVATE | SWP_NOZORDER,
            );
        }
    }

    pub(super) unsafe fn properties_layout(
        &mut self,
        y: f32,
        width: f32,
    ) -> windows::core::Result<f32> {
        unsafe {
            let Some(model) = Properties::parse(self.doc()) else {
                return Ok(y);
            };
            let left = self.content_left();
            let width = width.max(100.0);
            let key_width = (width * 0.25).clamp(72.0, 180.0).min(width * 0.4);
            let value_left = left + key_width + 8.0;
            let right = left + width - 8.0;
            let line = presentation::line_height(self.font_size());
            let mut next = y + 8.0;
            let mut rows = Vec::new();
            self.properties_ui.error_area = None;
            rows.push(Row {
                field: Field::Raw(0),
                text: "YAML ↗".into(),
                area: rect(left, next, width, line),
            });
            next += line + 6.0;
            for (index, property) in model.entries.iter().enumerate() {
                let value = match &property.value {
                    PropertyValue::Boolean(value) => {
                        format!("{} {}", if *value { "☑" } else { "☐" }, value)
                    }
                    PropertyValue::List(items) => format!("{} 項目", items.len()),
                    PropertyValue::Text(text) if text.is_empty() => "\"\"".into(),
                    value => value_text(value),
                };
                let measuring = self
                    .properties_ui
                    .draft
                    .as_ref()
                    .filter(|draft| draft.field == Field::Value(index))
                    .map(|draft| shell::text_of(draft.hwnd))
                    .unwrap_or_else(|| value.clone());
                let layout = self.text_layout(
                    &measuring,
                    (right - value_left - 68.0).max(20.0),
                    self.font_size(),
                )?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut metrics)?;
                let height = metrics.height.max(line) + 8.0;
                rows.push(Row {
                    field: Field::Key(index),
                    text: property.key.clone(),
                    area: rect(left, next, key_width, height),
                });
                rows.push(Row {
                    field: Field::Value(index),
                    text: value,
                    area: rect(
                        value_left,
                        next,
                        (right - value_left - 68.0).max(20.0),
                        height,
                    ),
                });
                rows.push(Row {
                    field: Field::Type(index),
                    text: "⋯".into(),
                    area: rect(right - 60.0, next, 28.0, height),
                });
                rows.push(Row {
                    field: Field::Delete(index),
                    text: "−".into(),
                    area: rect(right - 28.0, next, 28.0, height),
                });
                next += height + 4.0;
                if let PropertyValue::List(items) = &property.value {
                    for (item, text) in items.iter().enumerate() {
                        let layout = self.text_layout(
                            text,
                            (right - value_left - 68.0).max(20.0),
                            self.font_size(),
                        )?;
                        layout.GetMetrics(&mut metrics)?;
                        let height = metrics.height.max(line) + 8.0;
                        rows.push(Row {
                            field: Field::Item(index, item),
                            text: text.clone(),
                            area: rect(
                                value_left,
                                next,
                                (right - value_left - 68.0).max(20.0),
                                height,
                            ),
                        });
                        rows.push(Row {
                            field: Field::DeleteItem(index, item),
                            text: "−".into(),
                            area: rect(right - 28.0, next, 28.0, height),
                        });
                        next += height + 4.0;
                    }
                    rows.push(Row {
                        field: Field::AddItem(index),
                        text: "+ 項目".into(),
                        area: rect(value_left, next, (right - value_left).max(20.0), line),
                    });
                    next += line + 4.0;
                }
                if !self.properties_ui.error.is_empty()
                    && self.properties_ui.draft.as_ref().is_some_and(|draft| matches!(draft.field, Field::Key(i) | Field::Value(i) | Field::Item(i, _) if i == index))
                {
                    self.properties_ui.error_area = Some(rect(value_left, next, (right - value_left).max(20.0), line * 2.0));
                    next += line * 2.0 + 4.0;
                }
            }
            for (fallback_index, fallback) in model.fallbacks.iter().enumerate() {
                let excerpt: String = self.doc().source()[fallback.range.clone()]
                    .trim()
                    .chars()
                    .take(320)
                    .collect();
                let text = format!("{} — {}", excerpt, fallback.reason);
                let layout = self.text_layout(&text, width, self.font_size())?;
                let mut metrics = DWRITE_TEXT_METRICS::default();
                layout.GetMetrics(&mut metrics)?;
                rows.push(Row {
                    field: Field::Raw(fallback_index + 1),
                    text,
                    area: rect(left, next, width, metrics.height.max(line) + 8.0),
                });
                next += metrics.height.max(line) + 12.0;
            }
            rows.push(Row {
                field: Field::Add,
                text: "+ プロパティ".into(),
                area: rect(left, next, width, line + 8.0),
            });
            next += line + 18.0;
            if !self.properties_ui.error.is_empty() && self.properties_ui.error_area.is_none() {
                self.properties_ui.error_area = Some(rect(left, next, width, line * 2.0));
                next += line * 2.0;
            }
            self.properties_ui.model = Some(model);
            self.properties_ui.rows = rows;
            Ok(next)
        }
    }

    pub(super) unsafe fn properties_draw(
        &self,
        target: &ID2D1RenderTarget,
        scroll: f32,
    ) -> windows::core::Result<()> {
        unsafe {
            let foreground = target.CreateSolidColorBrush(&self.shell.tokens.foreground, None)?;
            let muted = target.CreateSolidColorBrush(&self.shell.tokens.muted, None)?;
            let border = target.CreateSolidColorBrush(&self.shell.tokens.border, None)?;
            let selected = target.CreateSolidColorBrush(&self.shell.tokens.selected, None)?;
            for row in &self.properties_ui.rows {
                let y = row.area.top - scroll;
                if y + row.area.bottom - row.area.top < self.editor_top()
                    || y > self.editor_bottom()
                {
                    continue;
                }
                let area = rect(
                    row.area.left,
                    y,
                    row.area.right - row.area.left,
                    row.area.bottom - row.area.top,
                );
                if self.properties_ui.focus.as_ref() == Some(&row.field) {
                    target.FillRectangle(&area, &selected);
                }
                let text = &row.text;
                let layout = self.text_layout(
                    text,
                    (area.right - area.left - 6.0).max(1.0),
                    self.font_size(),
                )?;
                target.DrawTextLayout(
                    point(area.left + 3.0, area.top + 2.0),
                    &layout,
                    if matches!(row.field, Field::Key(_)) {
                        &muted
                    } else {
                        &foreground
                    },
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
                if matches!(row.field, Field::Value(_) | Field::Item(_, _)) {
                    target.DrawLine(
                        point(area.left, area.bottom),
                        point(area.right, area.bottom),
                        &border,
                        1.0,
                        None,
                    );
                }
            }
            if !self.properties_ui.error.is_empty()
                && let Some(area) = self.properties_ui.error_area
            {
                let layout = self.text_layout(
                    &self.properties_ui.error,
                    self.content_width(),
                    self.font_size(),
                )?;
                target.DrawTextLayout(
                    point(area.left, area.top - scroll),
                    &layout,
                    &foreground,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }
}
