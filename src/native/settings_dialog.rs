//! A native, snapshot-bound common/Workspace settings form.
use super::*;
use mdlite::settings::{CapturedSettings, SettingValue, SettingsPatch, ThemeMode};
use std::{collections::BTreeMap, fs, io};
use windows::Win32::UI::Controls::*;
const APPLY: usize = 210;
const CANCEL: usize = 211;
const RESET: usize = 212;
const DETAIL: usize = 213;
const ENSURE_VISIBLE: u32 = WM_APP + 51;
const RAIL_REQUEST: u32 = WM_APP + 52;
const FONT_REFRESH: u32 = WM_APP + 53;
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
    detail: Option<PathBuf>,
    tokens: shell::Tokens,
    brush: HBRUSH,
    hwnd: HWND,
    scroll: i32,
    wheel_delta: i32,
    dpi: u32,
}
struct Window {
    form: RefCell<FormWindow>,
    font: RefCell<Option<control_style::UiFont>>,
    font_updating: Cell<bool>,
    font_pending: Cell<bool>,
    rail: RefCell<Option<scrollbar_style::VerticalRail>>,
    high_contrast: Cell<bool>,
    // Destruction can reenter while a form command owns its RefCell.
    closed: Cell<bool>,
}
unsafe fn children(hwnd: HWND) -> Vec<HWND> {
    unsafe {
        let mut result = Vec::new();
        let mut child = GetWindow(hwnd, GW_CHILD).ok();
        while let Some(h) = child {
            if !scrollbar_style::is_rail(h) {
                result.push(h);
            }
            child = GetWindow(h, GW_HWNDNEXT).ok();
        }
        result
    }
}
fn scroll_key(key: u16) -> Option<i32> {
    match key {
        k if k == VK_UP.0 => Some(SB_LINEUP.0),
        k if k == VK_DOWN.0 => Some(SB_LINEDOWN.0),
        k if k == VK_PRIOR.0 => Some(SB_PAGEUP.0),
        k if k == VK_NEXT.0 => Some(SB_PAGEDOWN.0),
        k if k == VK_HOME.0 => Some(SB_TOP.0),
        k if k == VK_END.0 => Some(SB_BOTTOM.0),
        _ => None,
    }
}
unsafe extern "system" fn scroll_control_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    kind: usize,
) -> LRESULT {
    unsafe {
        let owner = GetParent(hwnd).unwrap_or_default();
        let dropped = kind == 2
            && matches!(msg, WM_MOUSEWHEEL | WM_KEYDOWN)
            && shell::send(hwnd, CB_GETDROPPEDSTATE, WPARAM(0), LPARAM(0)).0 != 0;
        if msg == WM_MOUSEWHEEL && !dropped {
            return SendMessageW(owner, msg, Some(wp), Some(lp));
        }
        // EDIT caret keys and an open combo's native navigation stay native.
        if msg == WM_KEYDOWN
            && !dropped
            && (kind == 0 || (kind == 2 && [VK_PRIOR.0, VK_NEXT.0].contains(&(wp.0 as u16))))
            && let Some(code) = scroll_key(wp.0 as u16)
        {
            return SendMessageW(
                owner,
                WM_VSCROLL,
                Some(WPARAM(code as usize)),
                Some(LPARAM(0)),
            );
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(scroll_control_proc), 51);
        }
        let result = DefSubclassProc(hwnd, msg, wp, lp);
        if msg == WM_SETFOCUS {
            let _ = PostMessageW(
                Some(owner),
                ENSURE_VISIBLE,
                WPARAM(0),
                LPARAM(hwnd.0 as isize),
            );
        }
        result
    }
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
    unsafe fn content_extent(&self) -> i32 {
        unsafe {
            children(self.hwnd)
                .into_iter()
                .filter_map(|child| {
                    let mut r = RECT::default();
                    GetWindowRect(child, &mut r).ok()?;
                    let mut point = POINT {
                        x: r.left,
                        y: r.bottom,
                    };
                    let _ = ScreenToClient(self.hwnd, &mut point);
                    Some(point.y.saturating_add(self.scroll))
                })
                .max()
                .unwrap_or(0)
                .saturating_add((20. * self.dpi as f32 / 96.).round() as i32)
        }
    }
    unsafe fn ensure_visible(&self, control: HWND) -> Option<i32> {
        unsafe {
            if GetParent(control).ok() != Some(self.hwnd) {
                return None;
            }
            let mut r = RECT::default();
            if GetWindowRect(control, &mut r).is_err() {
                return None;
            }
            let mut top = POINT {
                x: r.left,
                y: r.top,
            };
            let _ = ScreenToClient(self.hwnd, &mut top);
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let bottom = top.y + r.bottom - r.top;
            Some(if top.y < 0 {
                self.scroll.saturating_add(top.y)
            } else if bottom > client.bottom {
                self.scroll
                    .saturating_add(bottom)
                    .saturating_sub(client.bottom)
            } else {
                self.scroll
            })
        }
    }
    unsafe fn change_dpi(&mut self, dpi: u32) -> Vec<(HWND, RECT)> {
        unsafe {
            let factor = dpi as f32 / self.dpi.max(1) as f32;
            let mut positions = Vec::new();
            for child in children(self.hwnd) {
                let mut r = RECT::default();
                if GetWindowRect(child, &mut r).is_err() {
                    continue;
                }
                let mut origin = POINT {
                    x: r.left,
                    y: r.top,
                };
                let _ = ScreenToClient(self.hwnd, &mut origin);
                let left = (origin.x as f32 * factor).round() as i32;
                let top = ((origin.y + self.scroll) as f32 * factor).round() as i32;
                positions.push((
                    child,
                    RECT {
                        left,
                        top,
                        right: left + ((r.right - r.left) as f32 * factor).round() as i32,
                        bottom: top + ((r.bottom - r.top) as f32 * factor).round() as i32,
                    },
                ));
            }
            self.scroll = 0;
            self.dpi = dpi;
            positions
        }
    }
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
    unsafe fn scroll_to(&mut self, position: i32) -> ScrollChange {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let content = self.content_extent();
            let next = position.clamp(0, content.saturating_sub(client.bottom.max(1)).max(0));
            let delta = self.scroll - next;
            self.scroll = next;
            let moves = if delta == 0 {
                Vec::new()
            } else {
                children(self.hwnd)
                    .into_iter()
                    .filter_map(|child| {
                        let mut r = RECT::default();
                        GetWindowRect(child, &mut r).ok()?;
                        let mut point = POINT {
                            x: r.left,
                            y: r.top,
                        };
                        let _ = ScreenToClient(self.hwnd, &mut point);
                        Some((child, point.x, point.y.saturating_add(delta)))
                    })
                    .collect()
            };
            ScrollChange {
                hwnd: self.hwnd,
                range: scrollbar_style::Range {
                    content,
                    page: client.bottom,
                    position: next,
                },
                moves,
                dpi: self.dpi,
                tokens: self.tokens,
            }
        }
    }
    unsafe fn create_controls(&mut self) {
        unsafe {
            self.dpi = GetDpiForWindow(self.hwnd).max(96);
            let dpi = self.dpi as f32 / 96.;
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
                if let Err(error) = edit_scroll::set_explorer_theme(control) {
                    eprintln!("Settings control theme failed: {error}");
                }
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
            for child in children(self.hwnd) {
                let mut name = [0u16; 32];
                let n = GetClassNameW(child, &mut name);
                let class = String::from_utf16_lossy(&name[..n as usize]);
                let kind = if class.eq_ignore_ascii_case("EDIT") {
                    1
                } else if class.eq_ignore_ascii_case("COMBOBOX") {
                    2
                } else {
                    0
                };
                let _ = SetWindowSubclass(child, Some(scroll_control_proc), 51, kind);
            }
        }
    }
}
struct ScrollChange {
    hwnd: HWND,
    range: scrollbar_style::Range,
    moves: Vec<(HWND, i32, i32)>,
    dpi: u32,
    tokens: shell::Tokens,
}
unsafe fn scroll_window(state: &Window, position: i32) {
    unsafe {
        let change = { state.form.borrow_mut().scroll_to(position) };
        // Move only content children, without a form borrow across native callbacks.
        // Moving first keeps reentrant WM_SIZE extent queries in the new coordinates.
        for (child, x, y) in change.moves {
            if GetWindowLongPtrW(change.hwnd, GWLP_USERDATA) != state as *const Window as isize {
                return;
            }
            if GetParent(child).ok() != Some(change.hwnd) {
                continue;
            }
            let _ = SetWindowPos(
                child,
                None,
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        if GetWindowLongPtrW(change.hwnd, GWLP_USERDATA) != state as *const Window as isize {
            return;
        }
        let mut tokens = change.tokens;
        tokens.high_contrast |= state.high_contrast.get();
        if let Err(error) = shell::refresh_native_scrollbars(change.hwnd, tokens) {
            eprintln!("Settings scrollbar refresh failed: {error}");
        }
        let rail = { state.rail.borrow().clone() };
        if let Some(rail) = rail {
            rail.update(
                change.range,
                change.dpi,
                change.tokens,
                state.high_contrast.get(),
            );
        } else {
            SetScrollInfo(
                change.hwnd,
                SB_VERT,
                &scrollbar_style::scroll_info(change.range),
                true,
            );
        }
        let _ = InvalidateRect(Some(change.hwnd), None, true);
    }
}
unsafe fn change_dpi(state: &Window, dpi: u32) {
    unsafe {
        let positions = { state.form.borrow_mut().change_dpi(dpi) };
        for (child, r) in positions {
            let _ = SetWindowPos(
                child,
                None,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        update_font(state);
        scroll_window(state, 0);
    }
}
fn update_font(state: &Window) {
    state.font_pending.set(true);
    let (hwnd, dpi) = {
        let Ok(form) = state.form.try_borrow() else {
            return;
        };
        (form.hwnd, form.dpi)
    };
    if state.font_updating.replace(true) {
        return;
    }
    state.font_pending.set(false);
    let mut font = { state.font.borrow_mut().take() };
    match font.as_mut() {
        Some(font) => {
            let _ = font.replace_on(hwnd, dpi, 16.);
        }
        None => {
            if let Ok(mut created) = control_style::UiFont::new(dpi, 16.)
                && created.apply_to(hwnd).is_ok()
            {
                font = Some(created);
            }
        }
    }
    // Neither RefCell is borrowed while UiFont sends native messages or drops.
    let previous = state.font.replace(font);
    drop(previous);
    state.font_updating.set(false);
    if state.font_pending.get() && !state.closed.get() {
        // Coalesce a nested DPI update after this font's synchronous deliveries.
        unsafe {
            let _ = PostMessageW(Some(hwnd), FONT_REFRESH, WPARAM(0), LPARAM(0));
        }
    }
}
unsafe fn drain_rail_request(state: &Window, wp: WPARAM, lp: LPARAM) {
    unsafe {
        // A modal command may own the form. Leave the coalesced request intact;
        // its command-end drain will apply it after releasing that borrow.
        if state.form.try_borrow_mut().is_err() {
            return;
        }
        let rail = { state.rail.borrow().clone() };
        if let Some(rail) = rail
            && let Some(position) = rail.take_request(wp.0, HWND(lp.0 as *mut _))
        {
            scroll_window(state, position);
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
            (*pointer).closed.set(true);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        if msg == WM_DESTROY {
            (*pointer).closed.set(true);
            return LRESULT(0);
        }
        if msg == RAIL_REQUEST {
            drain_rail_request(&*pointer, wp, lp);
            return LRESULT(0);
        }
        if msg == FONT_REFRESH {
            update_font(&*pointer);
            return LRESULT(0);
        }
        if matches!(msg, WM_SETTINGCHANGE | WM_THEMECHANGED) {
            (*pointer)
                .high_contrast
                .set(scrollbar_style::high_contrast());
            let position = { (*pointer).form.try_borrow().ok().map(|form| form.scroll) };
            if let Some(position) = position {
                scroll_window(&*pointer, position);
            }
            return LRESULT(0);
        }
        if msg == WM_CANCELMODE || (msg == WM_ENABLE && wp.0 == 0) {
            let rail = { (*pointer).rail.borrow().clone() };
            if let Some(rail) = rail {
                rail.cancel();
            }
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let Ok(mut form) = (*pointer).form.try_borrow_mut() else {
            return DefWindowProcW(hwnd, msg, wp, lp);
        };
        match msg {
            WM_GETMINMAXINFO => {
                let limits = &mut *(lp.0 as *mut MINMAXINFO);
                limits.ptMinTrackSize = POINT {
                    x: (680. * form.dpi as f32 / 96.).round() as i32,
                    y: (200. * form.dpi as f32 / 96.).round() as i32,
                };
                LRESULT(0)
            }
            WM_DPICHANGED => {
                let suggested = *(lp.0 as *const RECT);
                drop(form);
                change_dpi(&*pointer, (wp.0 & 0xffff) as u32);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    suggested.left,
                    suggested.top,
                    suggested.right - suggested.left,
                    suggested.bottom - suggested.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            ENSURE_VISIBLE => {
                let position = form.ensure_visible(HWND(lp.0 as *mut _));
                drop(form);
                if let Some(position) = position {
                    scroll_window(&*pointer, position);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if let Some(code) = scroll_key(wp.0 as u16) {
                    drop(form);
                    return SendMessageW(
                        hwnd,
                        WM_VSCROLL,
                        Some(WPARAM(code as usize)),
                        Some(LPARAM(0)),
                    );
                }
                drop(form);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_SIZE => {
                let position = form.scroll;
                drop(form);
                if (*pointer).font_pending.get() {
                    update_font(&*pointer);
                }
                scroll_window(&*pointer, position);
                LRESULT(0)
            }
            WM_VSCROLL => {
                let mut info = SCROLLINFO {
                    cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                    fMask: SIF_ALL,
                    ..Default::default()
                };
                let _ = GetScrollInfo(hwnd, SB_VERT, &mut info);
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let native = (*pointer)
                    .rail
                    .borrow()
                    .as_ref()
                    .is_none_or(|rail| rail.native());
                let next = match (wp.0 & 0xffff) as i32 {
                    0 => form
                        .scroll
                        .saturating_sub((32. * form.dpi as f32 / 96.).round() as i32),
                    1 => form
                        .scroll
                        .saturating_add((32. * form.dpi as f32 / 96.).round() as i32),
                    2 => form.scroll.saturating_sub(client.bottom),
                    3 => form.scroll.saturating_add(client.bottom),
                    4 | 5 if native => info.nTrackPos,
                    6 => 0,
                    7 => i32::MAX,
                    _ => form.scroll,
                };
                drop(form);
                scroll_window(&*pointer, next);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                form.wheel_delta += ((wp.0 >> 16) as u16 as i16) as i32;
                let steps = form.wheel_delta / 120;
                form.wheel_delta %= 120;
                let next = form.scroll.saturating_sub(
                    steps.saturating_mul((48. * form.dpi as f32 / 96.).round() as i32),
                );
                drop(form);
                scroll_window(&*pointer, next);
                LRESULT(0)
            }
            WM_CLOSE => {
                (*pointer).closed.set(true);
                drop(form);
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
                    let position = form.ensure_visible(HWND(lp.0 as *mut _));
                    drop(form);
                    if let Some(position) = position {
                        scroll_window(&*pointer, position);
                    }
                    return LRESULT(0);
                }
                if id == 200 && wp.0 >> 16 == CBN_SELCHANGE as usize {
                    form.capture_draft();
                    let selected =
                        shell::send(form.scope, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0 as usize;
                    if selected < form.forms.len() {
                        form.selected = selected;
                        form.show_draft();
                    }
                } else if id == APPLY && wp.0 >> 16 == BN_CLICKED as usize {
                    form.apply();
                } else if id == CANCEL && wp.0 >> 16 == BN_CLICKED as usize {
                    (*pointer).closed.set(true);
                    drop(form);
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                } else if id == RESET && wp.0 >> 16 == BN_CLICKED as usize {
                    form.capture_draft();
                    let selected = form.selected;
                    form.forms[selected].draft = vec![String::new(); 8];
                    form.show_draft();
                    form.apply();
                } else if id == DETAIL && wp.0 >> 16 == BN_CLICKED as usize {
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
                                (*pointer).closed.set(true);
                                drop(form);
                                let _ = DestroyWindow(hwnd);
                                return LRESULT(0);
                            }
                            Err(e) => form.message("詳細編集を開けません", &e.to_string()),
                        }
                    }
                }
                let position = form.scroll;
                drop(form);
                if (*pointer).font_pending.get() {
                    update_font(&*pointer);
                }
                let pending = {
                    (*pointer)
                        .rail
                        .borrow()
                        .as_ref()
                        .and_then(|rail| rail.take_pending())
                };
                scroll_window(&*pointer, pending.unwrap_or(position));
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
            _ => {
                // Default processing can synchronously send WM_CLOSE/WM_SIZE
                // or enter native scrollbar tracking. Those callbacks must
                // be able to borrow the form before this call returns.
                drop(form);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
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
                font: RefCell::new(None),
                font_updating: Cell::new(false),
                font_pending: Cell::new(false),
                rail: RefCell::new(None),
                high_contrast: Cell::new(scrollbar_style::high_contrast()),
                form: RefCell::new(FormWindow {
                    forms,
                    selected: 0,
                    controls: Vec::new(),
                    ime: Vec::new(),
                    scope: HWND::default(),
                    detail: None,
                    tokens: self.shell.tokens,
                    brush,
                    hwnd: HWND::default(),
                    scroll: 0,
                    wheel_delta: 0,
                    dpi: (self.dpi * 96.).round() as u32,
                }),
                closed: Cell::new(false),
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
                WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_THICKFRAME | WS_VSCROLL,
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
            update_font(&state);
            for child in children(hwnd) {
                let _ = control_style::install_child_cursor(child);
            }
            *state.rail.borrow_mut() =
                scrollbar_style::VerticalRail::new(hwnd, RAIL_REQUEST, self.shell.tokens);
            scroll_window(&state, 0);
            self.monitor_safe_position(hwnd);
            self.release_caret();
            let owner = self.hwnd;
            let _ = EnableWindow(owner, false);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let mut msg = MSG::default();
            let mut quit = None;

            while !state.closed.get() {
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

    struct FormFixture {
        state: Box<Window>,
        hwnd: HWND,
    }
    impl FormFixture {
        fn new() -> Self {
            unsafe {
                let state = Box::new(Window {
                    font: RefCell::new(None),
                    font_updating: Cell::new(false),
                    font_pending: Cell::new(false),
                    rail: RefCell::new(None),
                    high_contrast: Cell::new(true),
                    form: RefCell::new(FormWindow {
                        forms: Vec::new(),
                        selected: 0,
                        controls: Vec::new(),
                        ime: Vec::new(),
                        scope: HWND::default(),
                        detail: None,
                        tokens: shell::Tokens::default(),
                        brush: CreateSolidBrush(COLORREF(0)),
                        hwnd: HWND::default(),
                        scroll: 0,
                        wheel_delta: 0,
                        dpi: 96,
                    }),
                    closed: Cell::new(false),
                });
                let instance = GetModuleHandleW(None).unwrap().into();
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(form_proc),
                    hInstance: instance,
                    lpszClassName: w!("MDLiteSettingsReentryTest"),
                    ..Default::default()
                });
                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("MDLiteSettingsReentryTest"),
                    w!("Settings test"),
                    WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_VSCROLL,
                    0,
                    0,
                    680,
                    400,
                    None,
                    None,
                    Some(instance),
                    Some((&*state as *const Window).cast()),
                )
                .unwrap();
                state.form.borrow_mut().hwnd = hwnd;
                let bottom = shell::control(hwnd, w!("STATIC"), "extent", 0, WINDOW_STYLE(0));
                SetWindowPos(
                    bottom,
                    None,
                    16,
                    560,
                    200,
                    30,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
                scroll_window(&state, 0);
                Self { state, hwnd }
            }
        }
        fn enable_rail(&self) -> HWND {
            unsafe {
                let tokens = self.state.form.borrow().tokens;
                *self.state.rail.borrow_mut() =
                    scrollbar_style::VerticalRail::new(self.hwnd, RAIL_REQUEST, tokens);
                assert!(self.state.rail.borrow().is_some());
                self.state.high_contrast.set(false);
                scroll_window(&self.state, 0);
                let mut child = GetWindow(self.hwnd, GW_CHILD).ok();
                while let Some(hwnd) = child {
                    if scrollbar_style::is_rail(hwnd) {
                        return hwnd;
                    }
                    child = GetWindow(hwnd, GW_HWNDNEXT).ok();
                }
                panic!("rail attachment must own a child HWND");
            }
        }
    }
    impl Drop for FormFixture {
        fn drop(&mut self) {
            unsafe {
                if IsWindow(Some(self.hwnd)).as_bool() {
                    let _ = DestroyWindow(self.hwnd);
                }
                let _ = DeleteObject(self.state.form.borrow().brush.into());
            }
        }
    }

    #[test]
    fn settings_titlebar_close_exits_modal_state_through_default_reentry() {
        let fixture = FormFixture::new();
        unsafe {
            SendMessageW(
                fixture.hwnd,
                WM_SYSCOMMAND,
                Some(WPARAM(SC_CLOSE as usize)),
                Some(LPARAM(0)),
            );
            assert!(!IsWindow(Some(fixture.hwnd)).as_bool());
        }
        assert!(
            fixture.state.closed.get(),
            "destroyed form must end the disabled-owner modal loop"
        );
    }

    #[test]
    fn settings_default_resize_reentry_updates_scroll_page() {
        let fixture = FormFixture::new();
        unsafe {
            let mut before = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            GetScrollInfo(fixture.hwnd, SB_VERT, &mut before).unwrap();
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                680,
                280,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let mut after = before;
            GetScrollInfo(fixture.hwnd, SB_VERT, &mut after).unwrap();
            let mut client = RECT::default();
            GetClientRect(fixture.hwnd, &mut client).unwrap();
            assert_ne!(before.nPage, client.bottom as u32);
            assert_eq!(
                after.nPage, client.bottom as u32,
                "WM_SIZE sent by default WM_WINDOWPOSCHANGED must reach the form"
            );
        }
    }

    #[test]
    fn settings_external_destroy_ends_modal_state_while_form_is_borrowed() {
        let fixture = FormFixture::new();
        let _form = fixture.state.form.borrow_mut();
        unsafe {
            DestroyWindow(fixture.hwnd).unwrap();
        }
        assert!(fixture.state.closed.get());
    }
    #[test]
    fn settings_scroll_measures_children_and_supports_edges_pages_and_visibility() {
        let fixture = FormFixture::new();
        unsafe {
            let h = shell::control(fixture.hwnd, w!("BUTTON"), "bottom", 300, WINDOW_STYLE(0));
            SetWindowPos(h, None, 16, 740, 200, 30, SWP_NOZORDER | SWP_NOACTIVATE).unwrap();
            SendMessageW(fixture.hwnd, WM_SIZE, Some(WPARAM(0)), Some(LPARAM(0)));
            assert_eq!(fixture.state.form.borrow().content_extent(), 790);
            SendMessageW(
                fixture.hwnd,
                WM_KEYDOWN,
                Some(WPARAM(VK_END.0 as usize)),
                Some(LPARAM(0)),
            );
            let bottom = fixture.state.form.borrow().scroll;
            assert!(bottom > 400);
            let mut info = SCROLLINFO {
                cbSize: std::mem::size_of::<SCROLLINFO>() as u32,
                fMask: SIF_ALL,
                ..Default::default()
            };
            GetScrollInfo(fixture.hwnd, SB_VERT, &mut info).unwrap();
            assert_eq!(info.nMax, 789);
            assert_eq!(info.nPos, bottom);
            SendMessageW(
                fixture.hwnd,
                WM_KEYDOWN,
                Some(WPARAM(VK_PRIOR.0 as usize)),
                Some(LPARAM(0)),
            );
            assert!(fixture.state.form.borrow().scroll < bottom);
            SendMessageW(
                fixture.hwnd,
                WM_KEYDOWN,
                Some(WPARAM(VK_HOME.0 as usize)),
                Some(LPARAM(0)),
            );
            assert_eq!(fixture.state.form.borrow().scroll, 0);
            SendMessageW(
                fixture.hwnd,
                ENSURE_VISIBLE,
                Some(WPARAM(0)),
                Some(LPARAM(h.0 as isize)),
            );
            assert!(fixture.state.form.borrow().scroll > 0);
            let mut control_rect = RECT::default();
            GetWindowRect(h, &mut control_rect).unwrap();
            let mut bottom = POINT {
                x: control_rect.left,
                y: control_rect.bottom,
            };
            let _ = ScreenToClient(fixture.hwnd, &mut bottom);
            let mut client = RECT::default();
            GetClientRect(fixture.hwnd, &mut client).unwrap();
            assert!(
                bottom.y <= client.bottom
                    && bottom.y - (control_rect.bottom - control_rect.top) >= 0,
                "focused control must actually be inside the client"
            );
            assert_eq!(
                fixture.state.form.borrow().content_extent(),
                790,
                "scrolling must preserve unscrolled child extent"
            );
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                680,
                1000,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            assert_eq!(fixture.state.form.borrow().scroll, 0);
            assert_eq!(
                GetWindowLongW(fixture.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0,
                "no ghost scrollbar when content fits"
            );
        }
    }
    #[test]
    fn settings_child_wheel_accumulates_and_keeps_closed_combo_selection() {
        let fixture = FormFixture::new();
        unsafe {
            let edit = shell::control(fixture.hwnd, w!("EDIT"), "draft 日本語", 301, WS_BORDER);
            let combo = shell::control(
                fixture.hwnd,
                w!("COMBOBOX"),
                "",
                302,
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32),
            );
            for value in ["one", "two"] {
                shell::send(
                    combo,
                    CB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(wide(value).as_ptr() as isize),
                );
            }
            shell::send(combo, CB_SETCURSEL, WPARAM(0), LPARAM(0));
            SetWindowSubclass(edit, Some(scroll_control_proc), 51, 1).unwrap();
            SetWindowSubclass(combo, Some(scroll_control_proc), 51, 2).unwrap();
            let wheel = |h, delta: i16| {
                SendMessageW(
                    h,
                    WM_MOUSEWHEEL,
                    Some(WPARAM((delta as u16 as usize) << 16)),
                    Some(LPARAM(0)),
                );
            };
            wheel(edit, -60);
            assert_eq!(fixture.state.form.borrow().scroll, 0);
            wheel(edit, -60);
            assert_eq!(fixture.state.form.borrow().scroll, 48);
            wheel(combo, -120);
            assert_eq!(fixture.state.form.borrow().scroll, 96);
            assert_eq!(shell::send(combo, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0, 0);
            assert_eq!(shell::text_of(edit), "draft 日本語");
        }
    }
    #[test]
    fn settings_dpi_rescales_actual_extent_without_losing_scroll_geometry() {
        let fixture = FormFixture::new();
        unsafe {
            let before = fixture.state.form.borrow().content_extent();
            scroll_window(&fixture.state, 100);
            change_dpi(&fixture.state, 150);
            assert_eq!(fixture.state.form.borrow().scroll, 0);
            assert!(
                (fixture.state.form.borrow().content_extent()
                    - (before as f32 * 150. / 96.).round() as i32)
                    .abs()
                    <= 1
            );
            change_dpi(&fixture.state, 96);
            assert!((fixture.state.form.borrow().content_extent() - before).abs() <= 1);
        }
    }
    #[test]
    fn settings_fonts_publish_without_form_borrow_and_coalesce_nested_dpi() {
        struct Observation {
            state: *const Window,
            available: Cell<bool>,
            nested: Cell<bool>,
        }
        unsafe extern "system" fn observe(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            data: usize,
        ) -> LRESULT {
            unsafe {
                if msg == WM_SETFONT && wp.0 != 0 {
                    let observation = &*(data as *const Observation);
                    let state = &*observation.state;
                    let available =
                        state.form.try_borrow_mut().is_ok() && state.font.try_borrow_mut().is_ok();
                    observation
                        .available
                        .set(observation.available.get() && available);
                    if !observation.nested.replace(true) && available {
                        change_dpi(state, 144);
                    }
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        let fixture = FormFixture::new();
        let rail = fixture.enable_rail();
        unsafe {
            let edit = shell::control(fixture.hwnd, w!("EDIT"), "draft 日本語", 303, WS_BORDER);
            let before_rail_font = shell::send(rail, WM_GETFONT, WPARAM(0), LPARAM(0));
            let observation = Observation {
                state: &*fixture.state,
                available: Cell::new(true),
                nested: Cell::new(false),
            };
            SetWindowSubclass(edit, Some(observe), 93, &observation as *const _ as usize).unwrap();
            update_font(&fixture.state);
            assert!(observation.available.get());
            assert!(observation.nested.get());
            assert!(fixture.state.font_pending.get());
            let mut message = MSG::default();
            let _ = PeekMessageW(
                &mut message,
                Some(fixture.hwnd),
                FONT_REFRESH,
                FONT_REFRESH,
                PM_REMOVE,
            );
            shell::send(fixture.hwnd, FONT_REFRESH, WPARAM(0), LPARAM(0));
            assert!(!fixture.state.font_pending.get());
            let font = fixture.state.font.borrow().as_ref().unwrap().handle();
            assert_eq!(
                shell::send(edit, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                font.0 as isize
            );
            let mut data = LOGFONTW::default();
            assert_ne!(
                GetObjectW(
                    font.into(),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut data as *mut LOGFONTW).cast())
                ),
                0
            );
            assert_eq!(
                data.lfHeight,
                -(16. * fixture.state.form.borrow().dpi as f32 / 96.).round() as i32
            );
            assert_eq!(
                shell::send(rail, WM_GETFONT, WPARAM(0), LPARAM(0)),
                before_rail_font
            );
            assert_eq!(shell::text_of(edit), "draft 日本語");
            RemoveWindowSubclass(edit, Some(observe), 93).unwrap();
        }
    }
    #[test]
    fn settings_custom_rail_preserves_extent_child_positions_dpi_and_native_fallback() {
        let fixture = FormFixture::new();
        let rail = fixture.enable_rail();
        unsafe {
            let before = fixture.state.form.borrow().content_extent();
            let mut rail_before = RECT::default();
            GetWindowRect(rail, &mut rail_before).unwrap();
            scroll_window(&fixture.state, i32::MAX);
            assert_eq!(fixture.state.form.borrow().content_extent(), before);
            let mut rail_after = RECT::default();
            GetWindowRect(rail, &mut rail_after).unwrap();
            assert_eq!(
                rail_before, rail_after,
                "fixed rail must not move with content"
            );
            assert_eq!(
                GetWindowLongW(fixture.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
            let last = children(fixture.hwnd)[0];
            let mut control = RECT::default();
            GetWindowRect(last, &mut control).unwrap();
            let mut point = POINT {
                x: control.left,
                y: control.bottom,
            };
            let _ = ScreenToClient(fixture.hwnd, &mut point);
            let mut client = RECT::default();
            GetClientRect(fixture.hwnd, &mut client).unwrap();
            assert!(point.y <= client.bottom && point.y >= control.bottom - control.top);
            change_dpi(&fixture.state, 150);
            assert!(
                (fixture.state.form.borrow().content_extent()
                    - (before as f32 * 150. / 96.).round() as i32)
                    .abs()
                    <= 1
            );
            GetClientRect(rail, &mut client).unwrap();
            assert_eq!(client.right, 16);
            change_dpi(&fixture.state, 96);
            assert!((fixture.state.form.borrow().content_extent() - before).abs() <= 1);
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                680,
                1000,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            assert_eq!(fixture.state.form.borrow().scroll, 0);
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                680,
                280,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            assert_ne!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            fixture.state.high_contrast.set(true); // Exercise mode without changing OS settings.
            scroll_window(&fixture.state, 100);
            assert_ne!(
                GetWindowLongW(fixture.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
            assert_eq!(GetWindowLongW(rail, GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            let mut info = scrollbar_style::scroll_info(scrollbar_style::Range::default());
            GetScrollInfo(fixture.hwnd, SB_VERT, &mut info).unwrap();
            assert_eq!(info.nPos, 100);
            fixture.state.high_contrast.set(false);
            scroll_window(&fixture.state, 100);
            assert_eq!(
                GetWindowLongW(fixture.hwnd, GWL_STYLE) as u32 & WS_VSCROLL.0,
                0
            );
        }
    }
    #[test]
    fn settings_custom_rail_restores_actual_child_coordinates() {
        let fixture = FormFixture::new();
        fixture.enable_rail();
        unsafe {
            let bottom = shell::control(
                fixture.hwnd,
                w!("BUTTON"),
                "far bottom",
                399,
                WINDOW_STYLE(0),
            );
            SetWindowPos(
                bottom,
                None,
                16,
                100_000,
                200,
                30,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let mut stress_rect = RECT::default();
            GetWindowRect(bottom, &mut stress_rect).unwrap();
            let mut stress_origin = POINT {
                x: stress_rect.left,
                y: stress_rect.top,
            };
            let _ = ScreenToClient(fixture.hwnd, &mut stress_origin);
            eprintln!(
                "giant native-child layout not established in this stress fixture: requested_y=100000; GetWindowRect={stress_rect:?}; client_y={}; content_extent={}",
                stress_origin.y,
                fixture.state.form.borrow().content_extent()
            );
            // Native HWND coordinates did not establish that stress layout here.
            // Verify the real settings movement contract within its content scale.
            SetWindowPos(
                bottom,
                None,
                16,
                1200,
                200,
                30,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            let before = fixture.state.form.borrow().content_extent();
            let positions: Vec<_> = children(fixture.hwnd)
                .into_iter()
                .map(|h| {
                    let mut r = RECT::default();
                    GetWindowRect(h, &mut r).unwrap();
                    (h, r)
                })
                .collect();
            scroll_window(&fixture.state, 800);
            assert_eq!(fixture.state.form.borrow().scroll, 800);
            assert_eq!(fixture.state.form.borrow().content_extent(), before);
            scroll_window(&fixture.state, i32::MAX);
            let mut r = RECT::default();
            GetWindowRect(bottom, &mut r).unwrap();
            let mut point = POINT {
                x: r.left,
                y: r.bottom,
            };
            let _ = ScreenToClient(fixture.hwnd, &mut point);
            let mut client = RECT::default();
            GetClientRect(fixture.hwnd, &mut client).unwrap();
            assert!(point.y <= client.bottom && point.y >= r.bottom - r.top);
            scroll_window(&fixture.state, 0);
            for (h, before) in positions {
                let mut after = RECT::default();
                GetWindowRect(h, &mut after).unwrap();
                assert_eq!(
                    after, before,
                    "roundtrip must preserve each content child's absolute origin"
                );
            }
        }
    }
    #[test]
    fn settings_edit_rails_follow_content_without_becoming_form_content() {
        let fixture = FormFixture::new();
        fixture.enable_rail();
        unsafe {
            let edit = shell::control(
                fixture.hwnd,
                w!("EDIT"),
                "",
                207,
                WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
            );
            SetWindowPos(edit, None, 16, 200, 300, 90, SWP_NOZORDER | SWP_NOACTIVATE).unwrap();
            let text = "設定の試料😀\r\n".repeat(40);
            SetWindowTextW(edit, PCWSTR(wide(&text).as_ptr())).unwrap();
            shell::send(edit, EM_SETSEL, WPARAM(3), LPARAM(8));
            let selection = shell::send(edit, EM_GETSEL, WPARAM(0), LPARAM(0)).0;
            let rails = edit_scroll::rail_handles(edit).unwrap();
            let extent = fixture.state.form.borrow().content_extent();
            assert!(
                rails
                    .into_iter()
                    .all(|rail| !children(fixture.hwnd).contains(&rail))
            );
            let mut initial_edit = RECT::default();
            let mut initial_rail = RECT::default();
            GetWindowRect(edit, &mut initial_edit).unwrap();
            GetWindowRect(rails[1], &mut initial_rail).unwrap();
            for position in [100, 0, 100, 0] {
                scroll_window(&fixture.state, position);
                assert_eq!(fixture.state.form.borrow().content_extent(), extent);
                let mut edit_rect = RECT::default();
                let mut rail_rect = RECT::default();
                GetWindowRect(edit, &mut edit_rect).unwrap();
                GetWindowRect(rails[1], &mut rail_rect).unwrap();
                assert_eq!(edit_rect.top, initial_edit.top - position);
                assert_eq!(rail_rect.top, initial_rail.top - position);
                assert_eq!(shell::text_of(edit), text);
                assert_eq!(shell::send(edit, EM_GETMODIFY, WPARAM(0), LPARAM(0)).0, 0);
                assert_eq!(shell::send(edit, EM_CANUNDO, WPARAM(0), LPARAM(0)).0, 0);
                assert_eq!(
                    shell::send(edit, EM_GETSEL, WPARAM(0), LPARAM(0)).0,
                    selection
                );
            }
            fixture.state.high_contrast.set(true);
            scroll_window(&fixture.state, 0);
            assert_eq!(GetWindowLongW(rails[1], GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
            fixture.state.high_contrast.set(false);
            scroll_window(&fixture.state, 0);
            assert_ne!(GetWindowLongW(rails[1], GWL_STYLE) as u32 & WS_VISIBLE.0, 0);
        }
    }
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
