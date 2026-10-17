//! A native, snapshot-bound common/Workspace settings form.
use super::*;
use mdlite::settings::{CapturedSettings, SettingValue, SettingsPatch, ThemeMode};
use std::{collections::BTreeMap, fs, io};
use windows::Win32::UI::Controls::*;
const APPLY: usize = 210;
const CANCEL: usize = 211;
const RESET: usize = 212;
const DETAIL: usize = 213;
const KEYS: [&str; 6] = [
    "auto_save",
    "auto_save_delay_ms",
    "theme",
    "font_face",
    "font_size_pt",
    "default_memo_workspace",
];
struct Form {
    file: CapturedSettings,
    expected: Option<Vec<u8>>,
    baseline: Vec<String>,
    draft: Vec<String>,
}
struct FormWindow {
    forms: Vec<Form>,
    selected: usize,
    controls: Vec<HWND>,
    ime: Vec<(HWND, Box<Cell<bool>>)>,
    scope: HWND,
    closed: bool,
    detail: Option<PathBuf>,
    tokens: shell::Tokens,
    brush: HBRUSH,
    hwnd: HWND,
    scroll: i32,
}
struct Window {
    form: RefCell<FormWindow>,
}
fn raw(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}
fn values(file: &CapturedSettings) -> Vec<String> {
    let layer = file.parsed().layer;
    vec![
        layer.auto_save.map(|v| v.to_string()).unwrap_or_default(),
        layer
            .auto_save_delay_ms
            .map(|v| v.to_string())
            .unwrap_or_default(),
        layer
            .theme
            .map(|v| {
                match v {
                    ThemeMode::System => "system",
                    ThemeMode::Light => "light",
                    ThemeMode::Dark => "dark",
                    ThemeMode::Custom => "custom",
                }
                .into()
            })
            .unwrap_or_default(),
        layer.font_face.unwrap_or_default(),
        layer
            .font_size_pt
            .map(|v| v.to_string())
            .unwrap_or_default(),
        layer
            .default_memo_workspace
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
        layer
            .colors
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\r\n"),
        layer
            .keybindings
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\r\n"),
    ]
}
fn map_lines(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut result = BTreeMap::new();
    for line in text.lines().filter(|s| !s.trim().is_empty()) {
        let (k, v) = line
            .split_once('=')
            .ok_or("色とbindingは name=value の行で指定してください")?;
        if result.insert(k.trim().into(), v.trim().into()).is_some() {
            return Err("nameが重複しています".into());
        }
    }
    Ok(result)
}
fn form_patch(before: &[String], after: &[String]) -> Result<SettingsPatch, String> {
    let mut patch = SettingsPatch::default();
    for (i, key) in KEYS.iter().enumerate() {
        if before[i] == after[i] {
            continue;
        }
        let value = after[i].trim();
        if value.is_empty() {
            patch.inherit(*key);
            continue;
        }
        let setting = match i {
            0 => SettingValue::Bool(value.parse().map_err(|_| "auto_saveが不正です")?),
            1 | 4 => SettingValue::Integer(value.parse().map_err(|_| "数値が不正です")?),
            2 => SettingValue::Theme(match value {
                "system" => ThemeMode::System,
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                "custom" => ThemeMode::Custom,
                _ => return Err("themeが不正です".into()),
            }),
            3 => SettingValue::Text(value.into()),
            _ => SettingValue::Path(PathBuf::from(value)),
        };
        patch.set(*key, setting);
    }
    for (index, prefix) in [(6, "color."), (7, "bind.")] {
        let original = map_lines(&before[index])?;
        let updated = map_lines(&after[index])?;
        for key in original.keys().chain(updated.keys()) {
            if original.get(key) == updated.get(key) {
                continue;
            }
            let name = format!("{prefix}{key}");
            if let Some(value) = updated.get(key) {
                patch.set(name, SettingValue::Text(value.clone()));
            } else {
                patch.inherit(name);
            }
        }
    }
    Ok(patch)
}
impl FormWindow {
    unsafe fn message(&self, title: &str, text: &str) {
        unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(wide(text).as_ptr()),
                PCWSTR(wide(title).as_ptr()),
                MB_OK | MB_ICONINFORMATION,
            );
        }
    }
    unsafe fn capture_draft(&mut self) {
        unsafe {
            let mut draft = Vec::new();
            for (i, h) in self.controls.iter().enumerate() {
                if i == 0 || i == 2 {
                    let n = shell::send(*h, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    let value = if i == 0 {
                        ["", "true", "false"].get(n as usize)
                    } else {
                        ["", "system", "light", "dark", "custom"].get(n as usize)
                    };
                    draft.push(value.unwrap_or(&"").to_string());
                } else {
                    draft.push(shell::text_of(*h));
                }
            }
            self.forms[self.selected].draft = draft;
        }
    }
    unsafe fn show_draft(&self) {
        unsafe {
            for (i, h) in self.controls.iter().enumerate() {
                let value = &self.forms[self.selected].draft[i];
                if i == 0 || i == 2 {
                    let options = if i == 0 {
                        vec!["", "true", "false"]
                    } else {
                        vec!["", "system", "light", "dark", "custom"]
                    };
                    let index = options.iter().position(|v| *v == value).unwrap_or(0);
                    shell::send(*h, CB_SETCURSEL, WPARAM(index), LPARAM(0));
                } else {
                    let _ = SetWindowTextW(*h, PCWSTR(wide(value).as_ptr()));
                }
            }
        }
    }
    fn peer_check(&self) -> Result<(), String> {
        for form in &self.forms {
            if raw(form.file.target()).map_err(|e| e.to_string())? != form.expected {
                return Err("設定ファイルが外部で変更・作成・削除されています。上書きせずフォームを保持しています。閉じて開き直してください。".into());
            }
        }
        Ok(())
    }
    unsafe fn apply(&mut self) {
        unsafe {
            self.capture_draft();
            let result = self.peer_check().and_then(|_| {
                form_patch(
                    &self.forms[self.selected].baseline,
                    &self.forms[self.selected].draft,
                )
            });
            let patch = match result {
                Ok(p) => p,
                Err(e) => {
                    self.message("設定を保存できません", &e);
                    return;
                }
            };
            let other = self
                .forms
                .get(1 - self.selected)
                .map(|f| f.file.parsed().layer);
            let index = self.selected;
            let result = if index == 0 {
                self.forms[index].file.apply_with_layers(
                    &patch,
                    &[],
                    &other.as_ref().into_iter().collect::<Vec<_>>(),
                )
            } else {
                self.forms[index].file.apply_with_layers(
                    &patch,
                    &other.as_ref().into_iter().collect::<Vec<_>>(),
                    &[],
                )
            };
            match result {
                Ok(_) => {
                    let form = &mut self.forms[index];
                    form.expected =
                        raw(form.file.target()).unwrap_or_else(|_| form.expected.clone());
                    form.baseline = form.draft.clone();
                    self.message(
                        "設定",
                        "保存しました。Workspace/Commonの編集対象は変更していません。",
                    );
                }
                Err(e) => self.message("設定を保存できません — 入力保持", &e.to_string()),
            }
        }
    }
    unsafe fn scroll_to(&mut self, position: i32) {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let content = (590. * GetDpiForWindow(self.hwnd) as f32 / 96.) as i32;
            let next = position.clamp(0, (content - client.bottom).max(0));
            let delta = self.scroll - next;
            self.scroll = next;
            let info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: content,
                nPage: client.bottom.max(0) as u32,
                nPos: next,
                ..Default::default()
            };
            SetScrollInfo(self.hwnd, SB_VERT, &info, true);
            if delta != 0 {
                ScrollWindowEx(
                    self.hwnd,
                    0,
                    delta,
                    None,
                    None,
                    None,
                    None,
                    SW_SCROLLCHILDREN | SW_INVALIDATE | SW_ERASE,
                );
            }
        }
    }
    unsafe fn create_controls(&mut self) {
        unsafe {
            let dpi = GetDpiForWindow(self.hwnd) as f32 / 96.;
            let pos = |h: HWND, x: f32, y: f32, w: f32, hgt: f32| {
                let _ = SetWindowPos(
                    h,
                    None,
                    (x * dpi) as i32,
                    (y * dpi) as i32,
                    (w * dpi) as i32,
                    (hgt * dpi) as i32,
                    SWP_NOZORDER,
                );
                shell::send(
                    h,
                    WM_SETFONT,
                    WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize),
                    LPARAM(1),
                );
            };
            let label = shell::control(
                self.hwnd,
                w!("STATIC"),
                "編集する層（空欄・inheritは下の層を継承）",
                0,
                WINDOW_STYLE(0),
            );
            pos(label, 16., 12., 600., 22.);
            self.scope = shell::control(
                self.hwnd,
                w!("COMBOBOX"),
                "設定の編集対象",
                200,
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            );
            pos(self.scope, 16., 36., 620., 180.);
            for name in ["ユーザー共通", "Workspace"]
                .into_iter()
                .take(self.forms.len())
            {
                shell::send(
                    self.scope,
                    CB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(wide(name).as_ptr() as isize),
                );
            }
            shell::send(self.scope, CB_SETCURSEL, WPARAM(self.selected), LPARAM(0));
            let labels = [
                "自動保存",
                "保存待機 ms (100–60000)",
                "テーマ",
                "フォント",
                "サイズ pt (6–96)",
                "既定のメモWorkspace",
                "Custom色 (name=#RRGGBB)",
                "キー割当て (command=Ctrl+Key)",
            ];
            for (i, name) in labels.into_iter().enumerate() {
                let y = 76.
                    + if i < 6 {
                        i as f32 * 42.
                    } else {
                        252. + (i - 6) as f32 * 98.
                    };
                let label = shell::control(self.hwnd, w!("STATIC"), name, 0, WINDOW_STYLE(0));
                pos(label, 16., y + 4., 215., 24.);
                let combo = i == 0 || i == 2;
                let style = if combo {
                    WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL
                } else if i >= 6 {
                    WS_BORDER
                        | WS_VSCROLL
                        | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN) as u32)
                } else {
                    WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32)
                };
                let control = shell::control(
                    self.hwnd,
                    if combo { w!("COMBOBOX") } else { w!("EDIT") },
                    name,
                    201 + i,
                    style,
                );
                pos(
                    control,
                    236.,
                    y,
                    400.,
                    if combo {
                        180.
                    } else if i >= 6 {
                        90.
                    } else {
                        28.
                    },
                );
                if combo {
                    let options = if i == 0 {
                        vec!["inherit", "true", "false"]
                    } else {
                        vec!["inherit", "system", "light", "dark", "custom"]
                    };
                    for value in options {
                        shell::send(
                            control,
                            CB_ADDSTRING,
                            WPARAM(0),
                            LPARAM(wide(value).as_ptr() as isize),
                        );
                    }
                }
                if !combo {
                    self.ime.push((control, shell::track_control_ime(control)));
                }
                self.controls.push(control);
                let _ = SetWindowTheme(control, w!("Explorer"), None);
            }
            for (i, (id, label)) in [
                (APPLY, "Apply"),
                (CANCEL, "閉じる / Cancel"),
                (RESET, "この層をReset"),
                (DETAIL, "詳細編集…"),
            ]
            .into_iter()
            .enumerate()
            {
                let h = shell::control(
                    self.hwnd,
                    w!("BUTTON"),
                    label,
                    id,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
                pos(h, 16. + i as f32 * 160., 536., 148., 34.);
            }
            self.show_draft();
        }
    }
}
unsafe extern "system" fn form_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, cs.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Window;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let Ok(mut form) = (*pointer).form.try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_SIZE => {
                let position = form.scroll;
                form.scroll_to(position);
                LRESULT(0)
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..Default::default()
                };
                let _ = GetScrollInfo(hwnd, SB_VERT, &mut info);
                let next = match (wp.0 & 0xffff) as i32 {
                    0 => form.scroll - 32,
                    1 => form.scroll + 32,
                    2 => form.scroll - info.nPage as i32,
                    3 => form.scroll + info.nPage as i32,
                    4 | 5 => info.nTrackPos,
                    _ => form.scroll,
                };
                form.scroll_to(next);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let next = form.scroll - ((wp.0 >> 16) as u16 as i16) as i32 / 120 * 48;
                form.scroll_to(next);
                LRESULT(0)
            }
            WM_CLOSE => {
                form.closed = true;
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wp.0 & 0xffff;
                if [
                    EN_SETFOCUS as usize,
                    CBN_SETFOCUS as usize,
                    BN_SETFOCUS as usize,
                ]
                .contains(&(wp.0 >> 16))
                    && lp.0 != 0
                {
                    let mut r = RECT::default();
                    if GetWindowRect(HWND(lp.0 as *mut _), &mut r).is_ok() {
                        let mut top = POINT {
                            x: r.left,
                            y: r.top,
                        };
                        let _ = ScreenToClient(hwnd, &mut top);
                        let mut client = RECT::default();
                        let _ = GetClientRect(hwnd, &mut client);
                        let bottom = top.y + r.bottom - r.top;
                        let next = if top.y < 0 {
                            form.scroll + top.y
                        } else if bottom > client.bottom {
                            form.scroll + bottom - client.bottom
                        } else {
                            form.scroll
                        };
                        form.scroll_to(next);
                    }
                }
                if id == 200 && wp.0 >> 16 == CBN_SELCHANGE as usize {
                    form.capture_draft();
                    let selected =
                        shell::send(form.scope, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
                    if selected < form.forms.len() {
                        form.selected = selected;
                        form.show_draft();
                    }
                } else if id == APPLY {
                    form.apply();
                } else if id == CANCEL {
                    form.closed = true;
                    let _ = DestroyWindow(hwnd);
                } else if id == RESET {
                    form.capture_draft();
                    let selected = form.selected;
                    form.forms[selected].draft = vec![String::new(); 8];
                    form.show_draft();
                    form.apply();
                } else if id == DETAIL {
                    if let Err(e) = form.peer_check() {
                        form.message("詳細編集を開けません", &e);
                    } else {
                        let path = form.forms[form.selected].file.target().to_owned();
                        let result = if path.exists() {
                            Ok(())
                        } else {
                            fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).and_then(
                                |_| {
                                    FileSnapshot::create_new(
                                        &path,
                                        "schema_version = 1\n",
                                        FileFormat::default(),
                                    )
                                    .map(|_| ())
                                    .map_err(io::Error::other)
                                },
                            )
                        };
                        match result {
                            Ok(()) => {
                                form.detail = Some(path);
                                form.closed = true;
                                let _ = DestroyWindow(hwnd);
                            }
                            Err(e) => form.message("詳細編集を開けません", &e.to_string()),
                        }
                    }
                }
                LRESULT(0)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => {
                let dc = HDC(wp.0 as *mut _);
                SetTextColor(dc, shell::rgb(form.tokens.foreground));
                SetBkColor(dc, shell::rgb(form.tokens.panel));
                LRESULT(form.brush.0 as isize)
            }
            WM_ERASEBKGND => {
                let mut r = RECT::default();
                let _ = GetClientRect(hwnd, &mut r);
                FillRect(HDC(wp.0 as *mut _), &r, form.brush);
                LRESULT(1)
            }
            WM_DRAWITEM => {
                let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                FillRect(item.hDC, &item.rcItem, form.brush);
                SetTextColor(item.hDC, shell::rgb(form.tokens.foreground));
                let _ = SetBkMode(item.hDC, TRANSPARENT);
                let mut r = item.rcItem;
                draw_gdi_text(
                    item.hDC,
                    &shell::text_of(item.hwndItem),
                    &mut r,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
                let border = CreateSolidBrush(shell::rgb(form.tokens.border));
                FrameRect(item.hDC, &r, border);
                let _ = DeleteObject(border.into());
                if item.itemState.0 & ODS_FOCUS.0 != 0 {
                    let _ = DrawFocusRect(item.hDC, &r);
                }
                LRESULT(1)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
impl App {
    pub(super) unsafe fn show_settings(&mut self) {
        unsafe {
            if self.composing {
                return;
            }
            let Some(common) = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("MDLite/settings.toml"))
            else {
                self.status = "共通設定の保存先を取得できません".into();
                return;
            };
            let mut forms = Vec::new();
            for path in std::iter::once(common).chain(
                self.persistence
                    .workspace_root()
                    .map(|p| p.join(".mdlite/settings.toml")),
            ) {
                let file = match CapturedSettings::capture(path) {
                    Ok(f) => f,
                    Err(e) => {
                        self.notice("設定を開けません", &e.to_string());
                        return;
                    }
                };
                let expected = match raw(file.target()) {
                    Ok(v) => v,
                    Err(e) => {
                        self.notice("設定を開けません", &e.to_string());
                        return;
                    }
                };
                let baseline = values(&file);
                forms.push(Form {
                    file,
                    expected,
                    draft: baseline.clone(),
                    baseline,
                });
            }
            let brush = CreateSolidBrush(shell::rgb(self.shell.tokens.panel));
            let state = Box::new(Window {
                form: RefCell::new(FormWindow {
                    forms,
                    selected: 0,
                    controls: Vec::new(),
                    ime: Vec::new(),
                    scope: HWND::default(),
                    closed: false,
                    detail: None,
                    tokens: self.shell.tokens,
                    brush,
                    hwnd: HWND::default(),
                    scroll: 0,
                }),
            });
            let class = WNDCLASSW {
                lpfnWndProc: Some(form_proc),
                hInstance: GetModuleHandleW(None).unwrap_or_default().into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: w!("MDLiteSettings"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let hwnd = CreateWindowExW(
                WS_EX_DLGMODALFRAME,
                w!("MDLiteSettings"),
                w!("MDLite 設定"),
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VSCROLL,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                (680. * self.dpi) as i32,
                (630. * self.dpi) as i32,
                Some(self.hwnd),
                None,
                Some(class.hInstance),
                Some((&*state as *const Window).cast()),
            );
            let Ok(hwnd) = hwnd else {
                let _ = DeleteObject(brush.into());
                self.status = "設定windowを作成できません".into();
                return;
            };
            state.form.borrow_mut().hwnd = hwnd;
            state.form.borrow_mut().create_controls();
            self.monitor_safe_position(hwnd);
            self.release_caret();
            let owner = self.hwnd;
            let _ = EnableWindow(owner, false);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let mut msg = MSG::default();
            let mut quit = None;

            while !state.form.borrow().closed {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 <= 0 {
                    quit = Some(msg.wParam);
                    break;
                }
                let composing = state
                    .form
                    .borrow()
                    .ime
                    .iter()
                    .any(|(h, c)| *h == msg.hwnd && c.get());
                if msg.message == WM_KEYDOWN && msg.wParam.0 == VK_ESCAPE.0 as usize && !composing {
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    continue;
                }
                if composing || !IsDialogMessageW(hwnd, &msg).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
            let _ = EnableWindow(owner, true);
            let _ = SetFocus(Some(owner));
            if IsWindow(Some(hwnd)).as_bool() {
                let _ = DestroyWindow(hwnd);
            }
            let detail = state.form.borrow_mut().detail.take();
            let _ = DeleteObject(brush.into());
            self.load_persistence_settings();
            self.install_shell_menu();
            if let Some(path) = detail {
                self.open_path(path);
            }
            self.refresh();
            if let Some(code) = quit {
                PostQuitMessage(code.0 as i32);
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_form_patch_only_changes_edited_layer_fields() {
        let before = vec![
            "true".into(),
            "750".into(),
            "dark".into(),
            "Segoe UI".into(),
            "11".into(),
            String::new(),
            "background=#112233".into(),
            "file.save=Ctrl+S".into(),
        ];
        let mut after = before.clone();
        after[1] = "1250".into();
        after[2].clear();
        let patch = form_patch(&before, &after).unwrap();
        assert_eq!(patch.changes.len(), 2);
        assert_eq!(
            patch.changes["auto_save_delay_ms"],
            Some(SettingValue::Integer(1250))
        );
        assert_eq!(patch.changes["theme"], None);
        assert!(!patch.changes.contains_key("font_face"));
    }
}
