//! A native, snapshot-bound common/Workspace settings form.
use super::*;
use mdlite::settings::{
    CapturedSettings, EffectiveSettings, SettingValue, SettingsPatch, ThemeMode,
};
use std::{fs, io, ops::Range};
use windows::Win32::UI::Controls::*;
const APPLY: usize = 210;
const CANCEL: usize = 211;
const RESET: usize = 212;
const DETAIL: usize = 213;
const ENSURE_VISIBLE: u32 = WM_APP + 51;
const RAIL_REQUEST: u32 = WM_APP + 52;
const FONT_REFRESH: u32 = WM_APP + 53;
const CATEGORY_BASE: usize = 300;
const INHERIT_BASE: usize = 1_000;
const ACTION_BASE: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldKind {
    Boolean,
    Integer,
    Theme,
    Text,
    Path,
    Color,
    Shortcut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Category {
    General,
    Appearance,
    Colors,
    Keys,
}

#[derive(Clone)]
struct Field {
    key: String,
    label: String,
    help: String,
    kind: FieldKind,
    category: Category,
}

struct FieldControls {
    label: HWND,
    effective: HWND,
    input: HWND,
    inherit: HWND,
    action: Option<HWND>,
}

struct CategoryControls {
    title: &'static str,
    help: &'static str,
    button: HWND,
    fields: Range<usize>,
    expanded: bool,
}
struct Form {
    file: CapturedSettings,
    expected: Option<Vec<u8>>,
    baseline: Vec<Option<String>>,
    draft: Vec<Option<String>>,
}
struct FormWindow {
    forms: Vec<Form>,
    selected: usize,
    fields: Vec<Field>,
    rows: Vec<FieldControls>,
    categories: Vec<CategoryControls>,
    ime: Vec<(HWND, Box<Cell<bool>>)>,
    scope: HWND,
    note: HWND,
    help: HWND,
    headers: [HWND; 3],
    buttons: [HWND; 4],
    tooltip: HWND,
    tooltip_text: Vec<Vec<u16>>,
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
fn field_specs() -> Vec<Field> {
    let mut fields = vec![
        Field {
            key: "auto_save".into(),
            label: "自動保存".into(),
            help: "文書の変更を自動で保存します。有効／無効を指定できます。未指定なら下の設定層を継承します。".into(),
            kind: FieldKind::Boolean,
            category: Category::General,
        },
        Field {
            key: "auto_save_delay_ms".into(),
            label: "自動保存までの待機時間".into(),
            help: "入力後、自動保存を始めるまでの時間です。単位は ms、範囲は 100～60000 です。空欄は継承です。".into(),
            kind: FieldKind::Integer,
            category: Category::General,
        },
        Field {
            key: "default_memo_workspace".into(),
            label: "既定のメモWorkspace".into(),
            help: "既定で開くメモWorkspaceの絶対パスです。空欄は継承、「空を指定」はこの層の既定先を消します。".into(),
            kind: FieldKind::Path,
            category: Category::General,
        },
        Field {
            key: "theme".into(),
            label: "テーマ".into(),
            help: "画面の配色です。システム／ライト／ダーク／カスタムから選べます。空欄は継承です。".into(),
            kind: FieldKind::Theme,
            category: Category::Appearance,
        },
        Field {
            key: "font_face".into(),
            label: "フォント名".into(),
            help: "アプリの表示に使うフォント名です。空欄は継承です。".into(),
            kind: FieldKind::Text,
            category: Category::Appearance,
        },
        Field {
            key: "font_size_pt".into(),
            label: "文字サイズ".into(),
            help: "アプリ表示の文字サイズです。単位は pt、範囲は 6～96 です。0 は有効値にならず、空欄は継承です。".into(),
            kind: FieldKind::Integer,
            category: Category::Appearance,
        },
    ];
    for name in mdlite::settings::COLOR_NAMES {
        let label = match *name {
            "background" => "背景",
            "foreground" => "本文文字",
            "link" => "リンク",
            "heading" => "見出し",
            "marker" => "マーカー",
            "code_background" => "コード背景",
            "table_background" => "表背景",
            _ => name,
        };
        fields.push(Field {
            key: format!("color.{name}"),
            label: format!("{label}  [color.{name}]"),
            help: format!("{label}の色です。#RRGGBB の6桁16進数で指定します。カスタムテーマで使われ、空欄または継承ボタンで下の層へ戻ります。"),
            kind: FieldKind::Color,
            category: Category::Colors,
        });
    }
    for (command, label) in shell::configurable_commands() {
        if !mdlite::settings::COMMAND_NAMES.contains(&command) {
            continue;
        }
        fields.push(Field {
            key: format!("bind.{command}"),
            label: format!("{label}  [bind.{command}]"),
            help: format!("{label}のキー割当です。Ctrl／Shift／Alt と A～Z、0～9、F1～F24を指定します。空欄／継承は下の層を使い、「解除」は割当なしにします。予約キーや他の項目との重複はApply時に拒否します。"),
            kind: FieldKind::Shortcut,
            category: Category::Keys,
        });
    }
    fields
}

fn value_for_field(field: &Field, layer: &mdlite::settings::SettingsLayer) -> Option<String> {
    match field.key.as_str() {
        "auto_save" => layer.auto_save.map(|value| value.to_string()),
        "auto_save_delay_ms" => layer.auto_save_delay_ms.map(|value| value.to_string()),
        "theme" => layer.theme.map(|value| {
            match value {
                ThemeMode::System => "system",
                ThemeMode::Light => "light",
                ThemeMode::Dark => "dark",
                ThemeMode::Custom => "custom",
            }
            .to_owned()
        }),
        "font_face" => layer.font_face.clone(),
        "font_size_pt" => layer.font_size_pt.map(|value| value.to_string()),
        "default_memo_workspace" => layer
            .default_memo_workspace
            .as_ref()
            .map(|path| path.display().to_string()),
        key if key.starts_with("color.") => layer.colors.get(&key[6..]).cloned(),
        key if key.starts_with("bind.") => layer.keybindings.get(&key[5..]).cloned(),
        _ => None,
    }
}

fn values(file: &CapturedSettings, fields: &[Field]) -> Vec<Option<String>> {
    let layer = file.parsed().layer;
    fields
        .iter()
        .map(|field| value_for_field(field, &layer))
        .collect()
}

fn form_patch(
    before: &[Option<String>],
    after: &[Option<String>],
    fields: &[Field],
) -> Result<SettingsPatch, String> {
    if before.len() != fields.len() || after.len() != fields.len() {
        return Err("設定フォームの項目数が一致しません".into());
    }
    let mut patch = SettingsPatch::default();
    for ((old, new), field) in before.iter().zip(after).zip(fields) {
        if old == new {
            continue;
        }
        let Some(raw) = new else {
            patch.inherit(field.key.clone());
            continue;
        };
        let normalized = if matches!(field.kind, FieldKind::Path | FieldKind::Text) {
            raw.clone()
        } else {
            raw.trim().to_owned()
        };
        let value = &normalized;
        let setting = match field.kind {
            FieldKind::Boolean => SettingValue::Bool(
                value
                    .parse()
                    .map_err(|_| "自動保存は有効または無効を選んでください")?,
            ),
            FieldKind::Integer => {
                SettingValue::Integer(value.parse().map_err(|_| "整数を入力してください")?)
            }
            FieldKind::Theme => SettingValue::Theme(match value.as_str() {
                "system" => ThemeMode::System,
                "light" => ThemeMode::Light,
                "dark" => ThemeMode::Dark,
                "custom" => ThemeMode::Custom,
                _ => return Err("テーマを選んでください".into()),
            }),
            FieldKind::Text => SettingValue::Text(value.clone()),
            FieldKind::Path => SettingValue::Path(PathBuf::from(value)),
            FieldKind::Color | FieldKind::Shortcut => SettingValue::Text(value.clone()),
        };
        patch.set(field.key.clone(), setting);
    }
    Ok(patch)
}

fn merged_settings(forms: &[Form]) -> EffectiveSettings {
    let mut settings = EffectiveSettings::default();
    for form in forms {
        settings.overlay(&form.file.parsed().layer);
    }
    settings
}

fn theme_label(theme: ThemeMode) -> &'static str {
    match theme {
        ThemeMode::System => "システム",
        ThemeMode::Light => "ライト",
        ThemeMode::Dark => "ダーク",
        ThemeMode::Custom => "カスタム",
    }
}

fn color_hex(value: D2D1_COLOR_F) -> String {
    let rgb = shell::rgb(value).0;
    format!(
        "#{:02X}{:02X}{:02X}",
        (rgb >> 16) & 255,
        (rgb >> 8) & 255,
        rgb & 255
    )
}

fn effective_color(name: &str, settings: &EffectiveSettings, tokens: shell::Tokens) -> String {
    if settings.theme == ThemeMode::Custom
        && let Some(value) = settings.colors.get(name)
    {
        return value.clone();
    }
    let value = match (settings.theme, name) {
        (ThemeMode::Light, "background") => return "#F9F9F9".into(),
        (ThemeMode::Light, "foreground") => return "#1E2630".into(),
        (ThemeMode::Light, "link") => return "#285EAA".into(),
        (ThemeMode::Light, "heading") => return "#1E4C99".into(),
        (ThemeMode::Light, "marker") => return "#636D7A".into(),
        (ThemeMode::Light, "code_background") => return "#EDF2F7".into(),
        (ThemeMode::Light, "table_background") => return "#EFF2F4".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "background") => return "#1D1F23".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "foreground") => return "#D1D8E0".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "link") => return "#7CAFF4".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "heading") => return "#B2D1FC".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "marker") => return "#8E99A5".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "code_background") => return "#21262D".into(),
        (ThemeMode::Dark | ThemeMode::Custom, "table_background") => return "#23272C".into(),
        (_, "background") => tokens.background,
        (_, "foreground") => tokens.foreground,
        (_, "link") => tokens.accent,
        (_, "heading") => tokens.heading,
        (_, "marker") => tokens.muted,
        (_, "code_background") => tokens.code_background,
        (_, "table_background") => tokens.panel,
        _ => return "—".into(),
    };
    color_hex(value)
}

fn effective_value(field: &Field, settings: &EffectiveSettings, tokens: shell::Tokens) -> String {
    match field.key.as_str() {
        "auto_save" => if settings.auto_save {
            "有効"
        } else {
            "無効"
        }
        .into(),
        "auto_save_delay_ms" => format!("{} ms", settings.auto_save_delay_ms),
        "theme" => theme_label(settings.theme).into(),
        "font_face" => settings.font_face.clone(),
        "font_size_pt" => format!("{} pt", settings.font_size_pt),
        "default_memo_workspace" => {
            if settings.default_memo_workspace.as_os_str().is_empty() {
                "未指定".into()
            } else {
                settings.default_memo_workspace.display().to_string()
            }
        }
        key if key.starts_with("color.") => effective_color(&key[6..], settings, tokens),
        key if key.starts_with("bind.") => settings.keybindings.get(&key[5..]).map_or_else(
            || "未割当".into(),
            |value| {
                if value.eq_ignore_ascii_case("none") {
                    "解除".into()
                } else {
                    value.clone()
                }
            },
        ),
        _ => "—".into(),
    }
}

fn draft_state(field: &Field, value: Option<&str>) -> &'static str {
    match value {
        None => "継承",
        Some("") if field.kind == FieldKind::Path => "空を指定",
        Some(value) if field.kind == FieldKind::Shortcut && value.eq_ignore_ascii_case("none") => {
            "解除"
        }
        Some(_) => "上書き",
    }
}

fn combo_options(kind: FieldKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        FieldKind::Boolean => &[("継承", ""), ("有効", "true"), ("無効", "false")],
        FieldKind::Theme => &[
            ("継承", ""),
            ("システム", "system"),
            ("ライト", "light"),
            ("ダーク", "dark"),
            ("カスタム", "custom"),
        ],
        _ => &[],
    }
}

impl FormWindow {
    unsafe fn content_extent(&self) -> i32 {
        unsafe {
            children(self.hwnd)
                .into_iter()
                .filter(|child| GetWindowLongW(*child, GWL_STYLE) as u32 & WS_VISIBLE.0 != 0)
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
            for (index, (field, row)) in self.fields.iter().zip(&self.rows).enumerate() {
                if matches!(field.kind, FieldKind::Boolean | FieldKind::Theme) {
                    let selected = shell::send(row.input, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                    draft.push(
                        combo_options(field.kind)
                            .get(selected as usize)
                            .and_then(|(_, value)| {
                                (!value.is_empty()).then(|| (*value).to_owned())
                            }),
                    );
                } else {
                    let text = shell::text_of(row.input);
                    let previous = self.forms[self.selected].draft[index].as_deref();
                    draft.push(
                        if text.is_empty() && field.kind == FieldKind::Path && previous == Some("")
                        {
                            Some(String::new())
                        } else {
                            (!text.is_empty()).then_some(text)
                        },
                    );
                }
            }
            self.forms[self.selected].draft = draft;
            self.update_row_summaries();
        }
    }
    unsafe fn show_draft(&self) {
        unsafe {
            for (index, (field, row)) in self.fields.iter().zip(&self.rows).enumerate() {
                let value = self.forms[self.selected].draft[index].as_deref();
                if matches!(field.kind, FieldKind::Boolean | FieldKind::Theme) {
                    let selected = combo_options(field.kind)
                        .iter()
                        .position(|(_, option)| Some(*option) == value)
                        .unwrap_or(0);
                    shell::send(row.input, CB_SETCURSEL, WPARAM(selected), LPARAM(0));
                } else {
                    let _ =
                        SetWindowTextW(row.input, PCWSTR(wide(value.unwrap_or_default()).as_ptr()));
                }
            }
            self.update_row_summaries();
        }
    }
    unsafe fn update_row_summaries(&self) {
        unsafe {
            let effective = merged_settings(&self.forms);
            for (index, (field, row)) in self.fields.iter().zip(&self.rows).enumerate() {
                let value = effective_value(field, &effective, self.tokens);
                let state = draft_state(field, self.forms[self.selected].draft[index].as_deref());
                let text = format!("{value} / {state}");
                let _ = SetWindowTextW(row.effective, PCWSTR(wide(&text).as_ptr()));
            }
        }
    }
    unsafe fn update_help(&self, control: HWND) {
        unsafe {
            let help = self
                .rows
                .iter()
                .zip(&self.fields)
                .find(|(row, _)| {
                    [row.label, row.effective, row.input, row.inherit, row.action.unwrap_or_default()]
                        .contains(&control)
                })
                .map(|(_, field)| field.help.as_str())
                .or_else(|| {
                    self.categories
                        .iter()
                        .find(|category| category.button == control)
                        .map(|category| category.help)
                })
                .or_else(|| {
                    (control == self.scope).then_some(
                        "編集対象の層を選びます。ユーザー共通の下書きはWorkspaceへ切り替えても保持されます。",
                    )
                })
                .unwrap_or("設定の項目へTabで移動すると、ここに補足を表示します。hoverでも同じ説明を確認できます。");
            let _ = SetWindowTextW(self.help, PCWSTR(wide(help).as_ptr()));
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
                    &self.fields,
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
                    self.update_row_summaries();
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
    unsafe fn add_tooltip(&mut self, control: HWND, text: &str) {
        unsafe {
            if self.tooltip == HWND::default() {
                self.tooltip = CreateWindowExW(
                    WS_EX_TOPMOST | WS_EX_NOACTIVATE,
                    TOOLTIPS_CLASSW,
                    None,
                    WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    Some(self.hwnd),
                    None,
                    None,
                    None,
                )
                .unwrap_or_default();
                if self.tooltip != HWND::default() {
                    shell::send(
                        self.tooltip,
                        TTM_SETMAXTIPWIDTH,
                        WPARAM(0),
                        LPARAM((360. * self.dpi as f32 / 96.) as isize),
                    );
                }
            }
            if self.tooltip == HWND::default() || control == HWND::default() {
                return;
            }
            self.tooltip_text.push(wide(text));
            let text = self.tooltip_text.last_mut().unwrap();
            let mut tool = TTTOOLINFOW {
                cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                hwnd: self.hwnd,
                uId: control.0 as usize,
                lpszText: PWSTR(text.as_mut_ptr()),
                ..Default::default()
            };
            shell::send(
                self.tooltip,
                TTM_ADDTOOLW,
                WPARAM(0),
                LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
            );
        }
    }
    unsafe fn place(&self, control: HWND, x: i32, y: i32, width: i32, height: i32, visible: bool) {
        unsafe {
            let dpi = self.dpi as f32 / 96.;
            let flags = SWP_NOZORDER
                | SWP_NOACTIVATE
                | if visible {
                    SWP_SHOWWINDOW
                } else {
                    SWP_HIDEWINDOW
                };
            let _ = SetWindowPos(
                control,
                None,
                (x as f32 * dpi).round() as i32,
                ((y + self.scroll) as f32 * dpi).round() as i32,
                (width as f32 * dpi).round() as i32,
                (height as f32 * dpi).round() as i32,
                flags,
            );
        }
    }
    unsafe fn layout_controls(&mut self) {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let scale = self.dpi as f32 / 96.;
            let width = (client.right as f32 / scale).round() as i32;
            self.place(self.scope, 128, 12, 220, 28, true);
            self.place(self.note, 366, 10, (width - 382).max(180), 38, true);
            self.place(self.help, 16, 48, (width - 32).max(220), 34, true);
            self.place(self.headers[0], 16, 84, 280, 22, true);
            self.place(self.headers[1], 304, 84, 190, 22, true);
            self.place(self.headers[2], 502, 84, (width - 534).max(220), 22, true);

            let right = width - 16;
            let inherit_x = right - 72;
            let action_x = inherit_x - 80;
            let mut y = 112;
            let row_height = shell::UI_ROW_HEIGHT.round() as i32 + 10;
            for category in &self.categories {
                self.place(category.button, 16, y, (width - 32).max(240), 28, true);
                y += 34;
                for index in category.fields.clone() {
                    let row = &self.rows[index];
                    let visible = category.expanded;
                    self.place(row.label, 16, y, 280, row_height, visible);
                    self.place(row.effective, 304, y, 190, row_height, visible);
                    let has_action = row.action.is_some();
                    let input_right = if has_action {
                        action_x - 8
                    } else {
                        inherit_x - 8
                    };
                    self.place(
                        row.input,
                        502,
                        y + 2,
                        (input_right - 502).max(80),
                        row_height - 4,
                        visible,
                    );
                    self.place(row.inherit, inherit_x, y + 2, 72, row_height - 4, visible);
                    if let Some(action) = row.action {
                        self.place(action, action_x, y + 2, 72, row_height - 4, visible);
                    }
                    y += row_height;
                }
                if !category.expanded {
                    y -= (category.fields.len() as i32) * row_height;
                }
            }
            y += 8;
            let button_width = ((width - 32 - 24) / 4).max(120);
            for (index, button) in self.buttons.iter().enumerate() {
                self.place(
                    *button,
                    16 + index as i32 * (button_width + 8),
                    y,
                    button_width,
                    36,
                    true,
                );
            }
        }
    }
    unsafe fn create_controls(&mut self) {
        unsafe {
            self.dpi = GetDpiForWindow(self.hwnd).max(96);
            let scope_label =
                shell::control(self.hwnd, w!("STATIC"), "編集する層", 0, WINDOW_STYLE(0));
            self.place(scope_label, 16, 12, 104, 28, true);
            self.scope = shell::control(
                self.hwnd,
                w!("COMBOBOX"),
                "設定の編集対象",
                200,
                WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL,
            );
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
            self.note = shell::control(
                self.hwnd,
                w!("STATIC"),
                "共通設定の後にWorkspace設定が適用されます。列は実効値／この層の下書きです。空欄は継承、falseや0は指定値です。",
                0,
                WINDOW_STYLE(0),
            );
            self.help = shell::control(
                self.hwnd,
                w!("STATIC"),
                "項目へTabで移動すると補足を表示します。hoverでも同じ説明を確認できます。",
                0,
                WINDOW_STYLE(0),
            );
            self.headers = [
                shell::control(
                    self.hwnd,
                    w!("STATIC"),
                    "項目 / 識別キー",
                    0,
                    WINDOW_STYLE(0),
                ),
                shell::control(
                    self.hwnd,
                    w!("STATIC"),
                    "実効値 / この層の下書き",
                    0,
                    WINDOW_STYLE(0),
                ),
                shell::control(self.hwnd, w!("STATIC"), "この層の値", 0, WINDOW_STYLE(0)),
            ];
            let category_specs = [
                (
                    Category::General,
                    "一般／保存",
                    "保存待機や既定Workspaceを設定します。共通設定の後にWorkspace設定が優先されます。",
                    true,
                ),
                (
                    Category::Appearance,
                    "表示／テーマ",
                    "テーマと文字表示を設定します。適用前の値は下書きとして保持されます。",
                    true,
                ),
                (
                    Category::Colors,
                    "カスタム色",
                    "実装済みの色項目だけを編集します。色は#RRGGBBで入力し、カスタムテーマで使います。",
                    false,
                ),
                (
                    Category::Keys,
                    "キー割当",
                    "登録済みcommandだけを編集します。解除は下の層の割当を継承せず、割当なしを指定します。",
                    false,
                ),
            ];
            for (index, (category, title, help, expanded)) in category_specs.into_iter().enumerate()
            {
                let start = self
                    .fields
                    .iter()
                    .position(|field| field.category == category)
                    .unwrap_or(self.fields.len());
                let end = self
                    .fields
                    .iter()
                    .rposition(|field| field.category == category)
                    .map_or(start, |last| last + 1);
                let label = format!("{}  {title}", if expanded { "−" } else { "＋" });
                let button = shell::control(
                    self.hwnd,
                    w!("BUTTON"),
                    &label,
                    CATEGORY_BASE + index,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
                self.categories.push(CategoryControls {
                    title,
                    help,
                    button,
                    fields: start..end,
                    expanded,
                });
                self.add_tooltip(button, help);
            }
            let fields = self.fields.clone();
            for (index, field) in fields.iter().enumerate() {
                let label =
                    shell::control(self.hwnd, w!("STATIC"), &field.label, 0, WINDOW_STYLE(0));
                let effective = shell::control(self.hwnd, w!("STATIC"), "", 0, WINDOW_STYLE(0));
                let combo = matches!(field.kind, FieldKind::Boolean | FieldKind::Theme);
                let input = shell::control(
                    self.hwnd,
                    if combo { w!("COMBOBOX") } else { w!("EDIT") },
                    "",
                    400 + index,
                    if combo {
                        WINDOW_STYLE(CBS_DROPDOWNLIST as u32) | WS_VSCROLL
                    } else {
                        WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32)
                    },
                );
                if combo {
                    for (display, _) in combo_options(field.kind) {
                        shell::send(
                            input,
                            CB_ADDSTRING,
                            WPARAM(0),
                            LPARAM(wide(display).as_ptr() as isize),
                        );
                    }
                } else {
                    self.ime.push((input, shell::track_control_ime(input)));
                }
                let inherit = shell::control(
                    self.hwnd,
                    w!("BUTTON"),
                    "継承",
                    INHERIT_BASE + index,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
                let action = match field.kind {
                    FieldKind::Path => Some("空を指定"),
                    FieldKind::Shortcut => Some("解除"),
                    _ => None,
                }
                .map(|label| {
                    shell::control(
                        self.hwnd,
                        w!("BUTTON"),
                        label,
                        ACTION_BASE + index,
                        WINDOW_STYLE(BS_OWNERDRAW as u32),
                    )
                });
                self.rows.push(FieldControls {
                    label,
                    effective,
                    input,
                    inherit,
                    action,
                });
                if let Err(error) = edit_scroll::set_explorer_theme(input) {
                    eprintln!("Settings control theme failed: {error}");
                }
                self.add_tooltip(label, &field.help);
                self.add_tooltip(effective, &field.help);
                self.add_tooltip(input, &field.help);
                self.add_tooltip(inherit, &field.help);
                if let Some(action) = action {
                    let help = match field.kind {
                        FieldKind::Path => {
                            "この層の既定Workspaceを空にします。継承ボタンで下の層へ戻ります。"
                        }
                        FieldKind::Shortcut => {
                            "このcommandのショートカットを解除します。下の層から継承したい場合は継承ボタンを使います。"
                        }
                        _ => "",
                    };
                    self.add_tooltip(action, help);
                }
            }
            for (index, (id, label)) in [
                (APPLY, "適用"),
                (CANCEL, "閉じる（キャンセル）"),
                (RESET, "この層をリセット"),
                (DETAIL, "詳細編集…"),
            ]
            .into_iter()
            .enumerate()
            {
                self.buttons[index] = shell::control(
                    self.hwnd,
                    w!("BUTTON"),
                    label,
                    id,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
            }
            self.add_tooltip(self.buttons[0], "この層の変更を検証し、安全に保存します。競合や不正値がある場合は入力を保持します。");
            self.add_tooltip(
                self.buttons[1],
                "未適用の下書きを保存せず閉じます。すでに適用した設定は残ります。",
            );
            self.add_tooltip(self.buttons[2], "この層の管理項目を下書きで継承へ戻します。保存するには「適用」を押してください。未知の設定は保持します。");
            self.add_tooltip(self.buttons[3], "現在の設定ファイルを詳細編集で開きます。普段の色やキー割当には各項目のフォームを使ってください。");
            self.add_tooltip(self.scope, "編集対象の層を選びます。ユーザー共通の下書きはWorkspaceへ切り替えても保持されます。");
            self.layout_controls();
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
        state.form.borrow_mut().layout_controls();
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
            let _ = font.replace_on(hwnd, dpi, shell::UI_FONT_SIZE);
        }
        None => {
            if let Ok(mut created) = control_style::UiFont::new(dpi, shell::UI_FONT_SIZE)
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
unsafe fn close_window(hwnd: HWND, state: &Window) {
    unsafe {
        state.closed.set(true);
        // Keep an eligible activation target while Windows destroys the modal
        // surface instead of leaving its owner disabled during activation handoff.
        if let Ok(owner) = GetWindow(hwnd, GW_OWNER) {
            let _ = EnableWindow(owner, true);
        }
        let _ = DestroyWindow(hwnd);
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
                    x: (820. * form.dpi as f32 / 96.).round() as i32,
                    y: (240. * form.dpi as f32 / 96.).round() as i32,
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
                form.layout_controls();
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
                drop(form);
                close_window(hwnd, &*pointer);
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
                    let control = HWND(lp.0 as *mut _);
                    form.update_help(control);
                    let position = form.ensure_visible(control);
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
                } else if id >= CATEGORY_BASE
                    && id < CATEGORY_BASE + form.categories.len()
                    && wp.0 >> 16 == BN_CLICKED as usize
                {
                    form.capture_draft();
                    let category = &mut form.categories[id - CATEGORY_BASE];
                    category.expanded = !category.expanded;
                    let title = format!(
                        "{}  {}",
                        if category.expanded { "−" } else { "＋" },
                        category.title
                    );
                    let _ = SetWindowTextW(category.button, PCWSTR(wide(&title).as_ptr()));
                    form.layout_controls();
                } else if id == APPLY && wp.0 >> 16 == BN_CLICKED as usize {
                    form.apply();
                } else if id == CANCEL && wp.0 >> 16 == BN_CLICKED as usize {
                    drop(form);
                    close_window(hwnd, &*pointer);
                    return LRESULT(0);
                } else if id == RESET && wp.0 >> 16 == BN_CLICKED as usize {
                    form.capture_draft();
                    let selected = form.selected;
                    form.forms[selected].draft = vec![None; form.fields.len()];
                    form.show_draft();
                } else if wp.0 >> 16 == BN_CLICKED as usize && lp.0 != 0 {
                    let control = HWND(lp.0 as *mut _);
                    if let Some(index) = form.rows.iter().position(|row| row.inherit == control) {
                        let selected = form.selected;
                        form.forms[selected].draft[index] = None;
                        form.show_draft();
                    } else if let Some(index) =
                        form.rows.iter().position(|row| row.action == Some(control))
                    {
                        let value = match form.fields[index].kind {
                            FieldKind::Path => Some(String::new()),
                            FieldKind::Shortcut => Some("NONE".into()),
                            _ => None,
                        };
                        if let Some(value) = value {
                            let selected = form.selected;
                            form.forms[selected].draft[index] = Some(value);
                            form.show_draft();
                        }
                    }
                } else if (EN_CHANGE as usize == wp.0 >> 16 || CBN_SELCHANGE as usize == wp.0 >> 16)
                    && id >= 400
                    && id < 400 + form.rows.len()
                {
                    form.capture_draft();
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
                                drop(form);
                                close_window(hwnd, &*pointer);
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
struct PreparedSettings {
    state: Box<Window>,
    hwnd: HWND,
    owner: HWND,
}

struct SettingsOutcome {
    detail: Option<PathBuf>,
    quit: Option<WPARAM>,
}

impl App {
    pub(super) fn show_settings(&mut self) {
        if !self.composing {
            self.settings_requested = true;
        }
    }

    unsafe fn prepare_settings(&mut self) -> Option<PreparedSettings> {
        unsafe {
            if self.composing {
                return None;
            }
            let Some(common) = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("MDLite/settings.toml"))
            else {
                self.status = "共通設定の保存先を取得できません".into();
                return None;
            };
            let paths: Vec<_> = std::iter::once(common)
                .chain(
                    self.persistence
                        .workspace_root()
                        .map(|p| p.join(".mdlite/settings.toml")),
                )
                .collect();
            self.prepare_settings_files(paths)
        }
    }

    unsafe fn prepare_settings_files(
        &mut self,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Option<PreparedSettings> {
        unsafe {
            let fields = field_specs();
            let mut forms = Vec::new();
            for path in paths {
                let file = match CapturedSettings::capture(path) {
                    Ok(f) => f,
                    Err(e) => {
                        self.notice("設定を開けません", &e.to_string());
                        return None;
                    }
                };
                let expected = match raw(file.target()) {
                    Ok(v) => v,
                    Err(e) => {
                        self.notice("設定を開けません", &e.to_string());
                        return None;
                    }
                };
                let baseline = values(&file, &fields);
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
                    fields,
                    rows: Vec::new(),
                    categories: Vec::new(),
                    ime: Vec::new(),
                    scope: HWND::default(),
                    note: HWND::default(),
                    help: HWND::default(),
                    headers: [HWND::default(); 3],
                    buttons: [HWND::default(); 4],
                    tooltip: HWND::default(),
                    tooltip_text: Vec::new(),
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
                (960. * self.dpi) as i32,
                (700. * self.dpi) as i32,
                Some(self.hwnd),
                None,
                Some(class.hInstance),
                Some((&*state as *const Window).cast()),
            );
            let Ok(hwnd) = hwnd else {
                let _ = DeleteObject(brush.into());
                self.status = "設定windowを作成できません".into();
                return None;
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
            Some(PreparedSettings {
                state,
                hwnd,
                owner: self.hwnd,
            })
        }
    }

    unsafe fn complete_settings(&mut self, outcome: SettingsOutcome) {
        unsafe {
            let previous_settings = self.shell.settings.clone();
            self.load_persistence_settings();
            if self.shell.settings != previous_settings {
                self.install_shell_menu();
            }
            if let Some(path) = outcome.detail {
                self.open_path(path);
            }
            self.refresh();
            if let Some(code) = outcome.quit {
                PostQuitMessage(code.0 as i32);
            }
        }
    }
}

impl PreparedSettings {
    unsafe fn pump(self) -> SettingsOutcome {
        unsafe {
            let Self { state, hwnd, owner } = self;
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
                // Disabled owners do not receive new physical input, but keys
                // already queued for an editor/child must not run in this pump.
                if (msg.hwnd == owner || IsChild(owner, msg.hwnd).as_bool())
                    && ((WM_KEYFIRST..=WM_KEYLAST).contains(&msg.message)
                        || (WM_MOUSEFIRST..=WM_MOUSELAST).contains(&msg.message))
                {
                    continue;
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
            if IsWindow(Some(hwnd)).as_bool() {
                let _ = DestroyWindow(hwnd);
            }
            let _ = SetFocus(Some(owner));
            let detail = state.form.borrow_mut().detail.take();
            let _ = DeleteObject(state.form.borrow().brush.into());
            SettingsOutcome { detail, quit }
        }
    }
}

// Called only at a native command/pretranslation boundary, after its RefMut
// has been dropped. Form state owns all data used by the nested modal pump.
pub(super) unsafe fn run_requested(state: &WindowState) {
    unsafe {
        if state.settings_open.get() {
            return;
        }
        let prepared = {
            let mut app = state.app.borrow_mut();
            if !std::mem::take(&mut app.settings_requested) {
                return;
            }
            state.settings_open.set(true);
            app.prepare_settings()
        };
        if let Some(prepared) = prepared {
            run_prepared(state, prepared);
        } else {
            state.settings_open.set(false);
            let mut app = state.app.borrow_mut();
            drain_native_notifications(state, &mut app);
        }
    }
}

unsafe fn run_prepared(state: &WindowState, prepared: PreparedSettings) {
    unsafe {
        let outcome = prepared.pump();
        let mut app = state.app.borrow_mut();
        app.complete_settings(outcome);
        state.settings_open.set(false);
        if state.deferred_persistence.replace(false) {
            app.rearm_autosave(true);
        }
        drain_native_notifications(state, &mut app);
        drop(app);
        replay_deferred_paints(state);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    struct FormFixture {
        state: Box<Window>,
        hwnd: HWND,
    }
    struct SettingsTempDir(PathBuf);
    impl SettingsTempDir {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "mdlite-settings-form-{}-{stamp}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for SettingsTempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
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
                        fields: Vec::new(),
                        rows: Vec::new(),
                        categories: Vec::new(),
                        ime: Vec::new(),
                        scope: HWND::default(),
                        note: HWND::default(),
                        help: HWND::default(),
                        headers: [HWND::default(); 3],
                        buttons: [HWND::default(); 4],
                        tooltip: HWND::default(),
                        tooltip_text: Vec::new(),
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
    fn settings_modal_boundary_paints_main_without_editing_and_restores_owner() {
        const PROBE: u32 = WM_APP + 61;
        const WATCHDOG: usize = 793;
        struct Probe {
            state: *const WindowState,
            button: HWND,
            dc: HDC,
            called: Cell<bool>,
            app_available: Cell<bool>,
            owner_disabled: Cell<bool>,
            timer_deferred: Cell<bool>,
            draw_return: Cell<isize>,
            watchdog: Cell<bool>,
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
                let probe = &*(data as *const Probe);
                if msg == PROBE {
                    let state = &*probe.state;
                    probe.called.set(true);
                    probe.app_available.set(state.app.try_borrow_mut().is_ok());
                    probe
                        .owner_disabled
                        .set(!IsWindowEnabled(state.main_window.get()).as_bool());
                    probe.timer_deferred.set(state.deferred_persistence.get());
                    let item = DRAWITEMSTRUCT {
                        CtlType: ODT_BUTTON,
                        CtlID: CMD_SAVE as u32,
                        hwndItem: probe.button,
                        hDC: probe.dc,
                        rcItem: RECT {
                            left: 0,
                            top: 0,
                            right: 160,
                            bottom: 32,
                        },
                        ..Default::default()
                    };
                    probe.draw_return.set(
                        window_proc(
                            state.main_window.get(),
                            WM_DRAWITEM,
                            WPARAM(CMD_SAVE),
                            LPARAM((&item as *const DRAWITEMSTRUCT) as isize),
                        )
                        .0,
                    );
                    let _ = KillTimer(Some(hwnd), WATCHDOG);
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    return LRESULT(0);
                }
                if msg == WM_TIMER && wp.0 == WATCHDOG {
                    probe.watchdog.set(true);
                    let _ = KillTimer(Some(hwnd), WATCHDOG);
                    let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    return LRESULT(0);
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        let temp = SettingsTempDir::new();
        let path = temp.0.join("settings.toml");
        let settings_bytes = b"schema_version = 1\n";
        fs::write(&path, settings_bytes).unwrap();
        let mut app = super::super::tests::composing_test_app("# fixture\nbody 日本語\n");
        app.clear_composition();
        app.doc_mut().set_selection(2, 5).unwrap();
        app.doc_mut().replace_selection("selected").unwrap();
        let fixture = super::super::tests::MessagePumpFixture::new(app);
        let snapshot = |app: &App| {
            (
                app.doc().id(),
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().is_dirty(),
                app.doc().selection(),
                app.doc().undo_count(),
                app.doc().redo_count(),
                app.tabs.len(),
            )
        };
        unsafe {
            {
                let mut app = fixture.state.app.borrow_mut();
                app.start_shell();
                app.features_init();
                app.ensure_layout().unwrap();
                // All routes keep the same Settings request method and IME gate.
                app.show_settings();
                assert!(std::mem::take(&mut app.settings_requested));
            }
            SetFocus(Some(fixture.hwnd)).unwrap();
            let before = snapshot(&fixture.state.app.borrow());
            let button = GetDlgItem(Some(fixture.hwnd), CMD_SAVE as i32).unwrap();
            let query = GetDlgItem(Some(fixture.hwnd), 150).unwrap();
            let query_before = shell::text_of(query);
            fixture.state.settings_open.set(true);
            let prepared = fixture
                .state
                .app
                .borrow_mut()
                .prepare_settings_files([path.clone()])
                .unwrap();
            let modal = prepared.hwnd;
            let dc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: 160,
                    biHeight: -32,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(dc), &info, DIB_RGB_COLORS, &mut bits, None, 0).unwrap();
            let previous = SelectObject(dc, bitmap.into());
            let sentinel = COLORREF(0x005a2bc3);
            let brush = CreateSolidBrush(sentinel);
            FillRect(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: 160,
                    bottom: 32,
                },
                brush,
            );
            let _ = DeleteObject(brush.into());
            let probe = Probe {
                state: &*fixture.state,
                button,
                dc,
                called: Cell::new(false),
                app_available: Cell::new(false),
                owner_disabled: Cell::new(false),
                timer_deferred: Cell::new(false),
                draw_return: Cell::new(0),
                watchdog: Cell::new(false),
            };
            assert!(
                SetWindowSubclass(modal, Some(observe), 793, (&probe as *const Probe) as usize)
                    .as_bool()
            );
            SetTimer(Some(modal), WATCHDOG, 5_000, None);
            for (hwnd, message, value) in [
                (fixture.hwnd, WM_CHAR, b'x' as usize),
                (query, WM_CHAR, b'y' as usize),
                (fixture.hwnd, WM_COMMAND, CMD_NEW),
                (fixture.hwnd, WM_COMMAND, 101), // Settings reentry is blocked.
                (fixture.hwnd, WM_TIMER, PERSISTENCE_TIMER),
                (fixture.hwnd, WM_CLOSE, 0),
                (modal, PROBE, 0),
            ] {
                PostMessageW(Some(hwnd), message, WPARAM(value), LPARAM(0)).unwrap();
            }
            // The production boundary pumps the owned form without a RefMut,
            // then reacquires App for the ordinary completion path.
            run_prepared(&fixture.state, prepared);
            let changed = (0..32)
                .flat_map(|y| (0..160).map(move |x| (x, y)))
                .filter(|(x, y)| GetPixel(dc, *x, *y) != sentinel)
                .count();
            println!(
                "settings-boundary called={} app_available={} owner_disabled={} timer_deferred={} draw_return={} dib_changed={} owner_enabled={} focus={:?} owner={:?} watchdog={}",
                probe.called.get(),
                probe.app_available.get(),
                probe.owner_disabled.get(),
                probe.timer_deferred.get(),
                probe.draw_return.get(),
                changed,
                IsWindowEnabled(fixture.hwnd).as_bool(),
                GetFocus(),
                fixture.hwnd,
                probe.watchdog.get()
            );
            assert!(probe.called.get() && !probe.watchdog.get());
            assert!(probe.app_available.get() && probe.owner_disabled.get());
            assert!(probe.timer_deferred.get());
            assert_eq!(probe.draw_return.get(), 1);
            assert_eq!(changed, 160 * 32);
            assert!(!IsWindow(Some(modal)).as_bool());
            assert!(IsWindowEnabled(fixture.hwnd).as_bool());
            assert_eq!(GetFocus(), fixture.hwnd);
            assert_eq!(native_caret_owner(), Some(fixture.hwnd));
            assert!(!fixture.state.settings_open.get());
            assert!(!fixture.state.deferred_persistence.get());
            assert!(!fixture.state.app.borrow().settings_requested);
            assert_eq!(snapshot(&fixture.state.app.borrow()), before);
            assert_eq!(shell::text_of(query), query_before);
            assert_eq!(fs::read(path).unwrap(), settings_bytes);
            let (mut expected, position) = {
                let app = fixture.state.app.borrow();
                (app.doc().source().to_owned(), app.doc().selection().head)
            };
            expected.insert(position, 'z');
            SendMessageW(
                fixture.hwnd,
                WM_CHAR,
                Some(WPARAM(b'z' as usize)),
                Some(LPARAM(0)),
            );
            let app = fixture.state.app.borrow();
            assert_eq!(app.doc().source(), expected);
            assert_eq!(app.doc().selection(), Selection::caret(position + 1));
            drop(app);
            SelectObject(dc, previous);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
        }
    }

    #[test]
    fn settings_close_reenables_owner_before_destroying_modal_window() {
        thread_local! {
            static EVENTS: RefCell<Vec<(u32, bool)>> = const { RefCell::new(Vec::new()) };
        }
        unsafe extern "system" fn probe(
            hwnd: HWND,
            msg: u32,
            wp: WPARAM,
            lp: LPARAM,
            _: usize,
            owner: usize,
        ) -> LRESULT {
            unsafe {
                if matches!(msg, WM_DESTROY | WM_NCDESTROY) {
                    let enabled = IsWindowEnabled(HWND(owner as *mut _)).as_bool();
                    EVENTS.with(|events| events.borrow_mut().push((msg, enabled)));
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        for command in [None, Some(CANCEL)] {
            let owner = FormFixture::new();
            let fixture = FormFixture::new();
            EVENTS.with(|events| events.borrow_mut().clear());
            unsafe {
                SetWindowLongPtrW(fixture.hwnd, GWLP_HWNDPARENT, owner.hwnd.0 as isize);
                assert!(
                    SetWindowSubclass(fixture.hwnd, Some(probe), 1, owner.hwnd.0 as usize)
                        .as_bool()
                );
                let _ = EnableWindow(owner.hwnd, false);
                if let Some(id) = command {
                    SendMessageW(fixture.hwnd, WM_COMMAND, Some(WPARAM(id)), Some(LPARAM(0)));
                } else {
                    SendMessageW(fixture.hwnd, WM_CLOSE, Some(WPARAM(0)), Some(LPARAM(0)));
                }
            }
            EVENTS.with(|events| {
                println!(
                    "settings-close command={command:?} destruction={:?}",
                    events.borrow()
                );
                assert_eq!(
                    &*events.borrow(),
                    &[(WM_DESTROY, true), (WM_NCDESTROY, true)]
                );
            });
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
                -(shell::UI_FONT_SIZE * fixture.state.form.borrow().dpi as f32 / 96.).round()
                    as i32
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
        let fields = field_specs();
        let index = |key: &str| fields.iter().position(|field| field.key == key).unwrap();
        let mut before = vec![None; fields.len()];
        before[index("auto_save")] = Some("true".into());
        before[index("auto_save_delay_ms")] = Some("750".into());
        before[index("theme")] = Some("dark".into());
        before[index("font_face")] = Some("Segoe UI".into());
        before[index("font_size_pt")] = Some("11".into());
        before[index("color.background")] = Some("#112233".into());
        before[index("bind.file.save")] = Some("Ctrl+S".into());
        let mut after = before.clone();
        after[index("auto_save_delay_ms")] = Some("1250".into());
        after[index("theme")] = None;
        let patch = form_patch(&before, &after, &fields).unwrap();
        assert_eq!(patch.changes.len(), 2);
        assert_eq!(
            patch.changes["auto_save_delay_ms"],
            Some(SettingValue::Integer(1250))
        );
        assert_eq!(patch.changes["theme"], None);
        assert!(!patch.changes.contains_key("font_face"));
    }

    #[test]
    fn settings_form_fields_come_from_supported_registries() {
        let fields = field_specs();
        let colors = fields
            .iter()
            .filter(|field| field.kind == FieldKind::Color)
            .map(|field| field.key.strip_prefix("color.").unwrap())
            .collect::<Vec<_>>();
        assert_eq!(colors, mdlite::settings::COLOR_NAMES);
        let commands = fields
            .iter()
            .filter(|field| field.kind == FieldKind::Shortcut)
            .map(|field| field.key.strip_prefix("bind.").unwrap())
            .collect::<std::collections::BTreeSet<_>>();
        let registered = shell::configurable_commands()
            .into_iter()
            .filter_map(|(key, _)| {
                mdlite::settings::COMMAND_NAMES
                    .contains(&key)
                    .then_some(key)
            })
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(commands, registered);
        assert!(
            fields
                .iter()
                .filter(|field| field.kind == FieldKind::Color || field.kind == FieldKind::Shortcut)
                .all(|field| field.label.contains('[') && !field.help.is_empty())
        );
    }

    #[test]
    fn settings_form_keeps_false_zero_empty_and_unbind_separate_from_inherit() {
        let fields = field_specs();
        let index = |key: &str| fields.iter().position(|field| field.key == key).unwrap();
        let before = vec![None; fields.len()];
        let mut after = before.clone();
        after[index("auto_save")] = Some("false".into());
        after[index("auto_save_delay_ms")] = Some("0".into());
        after[index("default_memo_workspace")] = Some(String::new());
        after[index("bind.file.save")] = Some("NONE".into());
        let patch = form_patch(&before, &after, &fields).unwrap();
        assert_eq!(patch.changes["auto_save"], Some(SettingValue::Bool(false)));
        assert_eq!(
            patch.changes["auto_save_delay_ms"],
            Some(SettingValue::Integer(0))
        );
        assert_eq!(
            patch.changes["default_memo_workspace"],
            Some(SettingValue::Path(PathBuf::new()))
        );
        assert_eq!(
            patch.changes["bind.file.save"],
            Some(SettingValue::Text("NONE".into()))
        );
        let inherited = form_patch(&after, &before, &fields).unwrap();
        assert_eq!(inherited.changes["default_memo_workspace"], None);
        assert_eq!(inherited.changes["bind.file.save"], None);
        assert_eq!(
            draft_state(&fields[index("default_memo_workspace")], Some("")),
            "空を指定"
        );
        assert_eq!(
            draft_state(&fields[index("bind.file.save")], Some("NONE")),
            "解除"
        );
    }

    #[test]
    fn settings_form_reset_keeps_selected_changes_draft_only_until_apply() {
        const RESET_TIMER: usize = 794;
        struct ResetProbe {
            root: HWND,
            notice_seen: Cell<bool>,
        }
        struct ResetTimer(HWND);
        impl Drop for ResetTimer {
            fn drop(&mut self) {
                unsafe {
                    // Invalidate first: KillTimer alone leaves queued WM_TIMER
                    // callbacks that must not access the scoped probe.
                    let _ = RemovePropW(self.0, w!("MDLiteSettingsResetTestProbe"));
                    let _ = KillTimer(Some(self.0), RESET_TIMER);
                }
            }
        }
        unsafe extern "system" fn dismiss_unexpected_notice(hwnd: HWND, _: u32, id: usize, _: u32) {
            unsafe {
                if id != RESET_TIMER {
                    return;
                }
                let property = GetPropW(hwnd, w!("MDLiteSettingsResetTestProbe"));
                if property.0.is_null() {
                    return;
                }
                let probe = &*(property.0 as *const ResetProbe);
                if hwnd != probe.root {
                    return;
                }
                let popup = GetLastActivePopup(hwnd);
                let mut class = [0u16; 32];
                let length = GetClassNameW(popup, &mut class);
                let mut pid = 0;
                GetWindowThreadProcessId(popup, Some(&mut pid));
                if popup != hwnd
                    && GetWindow(popup, GW_OWNER).ok() == Some(hwnd)
                    && pid == std::process::id()
                    && String::from_utf16_lossy(&class[..length.max(0) as usize]) == "#32770"
                {
                    probe.notice_seen.set(true);
                    // An old RESET implementation enters MessageBoxW's nested pump.
                    // End only this fixture's own notice, so the regression fails
                    // rather than waiting for a person to dismiss it.
                    let _ = EndDialog(popup, 1);
                }
            }
        }

        let root = SettingsTempDir::new();
        let common_path = root.0.join("common.toml");
        let workspace_path = root.0.join("workspace.toml");
        let common_bytes = b"schema_version = 1\nauto_save_delay_ms = 750\nfont_face = \"Segoe UI\"\n# common sentinel\nunknown_common = \"retain\"\n";
        let workspace_bytes = b"schema_version = 1\nauto_save = false\nauto_save_delay_ms = 1200\nfont_face = \"Yu Gothic\"\n# workspace sentinel\nunknown_workspace = \"retain\"\n";
        fs::write(&common_path, common_bytes).unwrap();
        fs::write(&workspace_path, workspace_bytes).unwrap();
        let fields = field_specs();
        let font_index = fields
            .iter()
            .position(|field| field.key == "font_face")
            .unwrap();
        let fixture = FormFixture::new();
        unsafe {
            {
                let mut form = fixture.state.form.borrow_mut();
                form.fields = fields.clone();
                for path in [&common_path, &workspace_path] {
                    let file = CapturedSettings::capture(path).unwrap();
                    let baseline = values(&file, &fields);
                    form.forms.push(Form {
                        file,
                        expected: raw(path).unwrap(),
                        baseline: baseline.clone(),
                        draft: baseline,
                    });
                }
                form.forms[0].draft[font_index] = Some("Common draft retained".into());
                form.selected = 1;
                form.create_controls();
            }
            let (baselines, expected, other_draft, reset, cancel, font_edit) = {
                let form = fixture.state.form.borrow();
                (
                    form.forms
                        .iter()
                        .map(|f| f.baseline.clone())
                        .collect::<Vec<_>>(),
                    form.forms
                        .iter()
                        .map(|f| f.expected.clone())
                        .collect::<Vec<_>>(),
                    form.forms[0].draft.clone(),
                    form.buttons[2],
                    form.buttons[1],
                    form.rows[font_index].input,
                )
            };
            SetWindowTextW(
                font_edit,
                PCWSTR(wide("Workspace unapplied draft").as_ptr()),
            )
            .unwrap();
            let probe = ResetProbe {
                root: fixture.hwnd,
                notice_seen: Cell::new(false),
            };
            SetPropW(
                fixture.hwnd,
                w!("MDLiteSettingsResetTestProbe"),
                Some(HANDLE((&probe as *const ResetProbe).cast_mut().cast())),
            )
            .unwrap();
            let timer = ResetTimer(fixture.hwnd);
            assert_ne!(
                SetTimer(
                    Some(fixture.hwnd),
                    RESET_TIMER,
                    50,
                    Some(dismiss_unexpected_notice)
                ),
                0
            );
            // No form borrow is held across the actual command's synchronous
            // reentry or an old implementation's nested notice loop.
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((BN_CLICKED as usize) << 16) | RESET)),
                Some(LPARAM(reset.0 as isize)),
            );
            drop(timer);
            assert!(
                GetPropW(fixture.hwnd, w!("MDLiteSettingsResetTestProbe"))
                    .0
                    .is_null()
            );
            // Simulate queued callbacks after invalidation, including the exact
            // old timer ID. Neither path may interpret an ID as an address.
            dismiss_unexpected_notice(fixture.hwnd, WM_TIMER, RESET_TIMER, 0);
            dismiss_unexpected_notice(fixture.hwnd, WM_TIMER, usize::MAX, 0);
            assert!(
                !probe.notice_seen.get(),
                "Reset must not Apply or open a save notice"
            );
            {
                let form = fixture.state.form.borrow();
                assert_eq!(form.selected, 1);
                assert!(form.forms[1].draft.iter().all(Option::is_none));
                assert_eq!(form.forms[0].draft, other_draft);
                for (index, layer) in form.forms.iter().enumerate() {
                    assert_eq!(layer.baseline, baselines[index]);
                    assert_eq!(layer.expected, expected[index]);
                    assert_eq!(values(&layer.file, &fields), baselines[index]);
                }
                for (field, row) in form.fields.iter().zip(&form.rows) {
                    if matches!(field.kind, FieldKind::Boolean | FieldKind::Theme) {
                        assert_eq!(
                            shell::send(row.input, CB_GETCURSEL, WPARAM(0), LPARAM(0)).0,
                            0
                        );
                    } else {
                        assert_eq!(shell::text_of(row.input), "");
                    }
                    assert!(shell::text_of(row.effective).ends_with(" / 継承"));
                }
            }
            assert_eq!(fs::read(&common_path).unwrap(), common_bytes);
            assert_eq!(fs::read(&workspace_path).unwrap(), workspace_bytes);
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((BN_CLICKED as usize) << 16) | CANCEL)),
                Some(LPARAM(cancel.0 as isize)),
            );
            assert!(fixture.state.closed.get());
            assert!(!IsWindow(Some(fixture.hwnd)).as_bool());
            assert_eq!(fs::read(&common_path).unwrap(), common_bytes);
            assert_eq!(fs::read(&workspace_path).unwrap(), workspace_bytes);
        }
    }

    #[test]
    fn settings_form_drafts_survive_scope_collapse_resize_and_dpi() {
        let root = SettingsTempDir::new();
        let common_dir = root.0.join("common");
        let workspace_dir = root.0.join("workspace/.mdlite");
        fs::create_dir_all(&common_dir).unwrap();
        fs::create_dir_all(&workspace_dir).unwrap();
        let fields = field_specs();
        let common = CapturedSettings::capture(common_dir.join("settings.toml")).unwrap();
        let workspace = CapturedSettings::capture(workspace_dir.join("settings.toml")).unwrap();
        let common_values = values(&common, &fields);
        let workspace_values = values(&workspace, &fields);
        let fixture = FormFixture::new();
        unsafe {
            {
                let mut form = fixture.state.form.borrow_mut();
                form.fields = fields.clone();
                form.forms = vec![
                    Form {
                        file: common,
                        expected: None,
                        baseline: common_values.clone(),
                        draft: common_values,
                    },
                    Form {
                        file: workspace,
                        expected: None,
                        baseline: workspace_values.clone(),
                        draft: workspace_values,
                    },
                ];
                form.create_controls();
            }
            let font_index = fields
                .iter()
                .position(|field| field.key == "font_face")
                .unwrap();
            let font_edit = fixture.state.form.borrow().rows[font_index].input;
            SetWindowTextW(font_edit, PCWSTR(wide("Meiryo").as_ptr())).unwrap();
            fixture.state.form.borrow_mut().capture_draft();
            assert_eq!(
                fixture.state.form.borrow().forms[0].draft[font_index].as_deref(),
                Some("Meiryo")
            );

            let appearance_button = fixture.state.form.borrow().categories[1].button;
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((BN_CLICKED as usize) << 16) | (CATEGORY_BASE + 1))),
                Some(LPARAM(appearance_button.0 as isize)),
            );
            assert_eq!(
                GetWindowLongW(font_edit, GWL_STYLE) as u32 & WS_VISIBLE.0,
                0,
                "collapsing a section hides its controls"
            );
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((BN_CLICKED as usize) << 16) | (CATEGORY_BASE + 1))),
                Some(LPARAM(appearance_button.0 as isize)),
            );
            assert_eq!(shell::text_of(font_edit), "Meiryo");

            let scope = fixture.state.form.borrow().scope;
            shell::send(scope, CB_SETCURSEL, WPARAM(1), LPARAM(0));
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((CBN_SELCHANGE as usize) << 16) | 200)),
                Some(LPARAM(scope.0 as isize)),
            );
            assert_eq!(fixture.state.form.borrow().selected, 1);
            assert_eq!(shell::text_of(font_edit), "");
            SetWindowTextW(font_edit, PCWSTR(wide("Yu Gothic").as_ptr())).unwrap();
            fixture.state.form.borrow_mut().capture_draft();

            shell::send(scope, CB_SETCURSEL, WPARAM(0), LPARAM(0));
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((CBN_SELCHANGE as usize) << 16) | 200)),
                Some(LPARAM(scope.0 as isize)),
            );
            assert_eq!(shell::text_of(font_edit), "Meiryo");

            SetWindowTextW(font_edit, PCWSTR(wide("Meiryo Pro").as_ptr())).unwrap();
            fixture.state.form.borrow_mut().capture_draft();
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                1000,
                700,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            assert_eq!(shell::text_of(font_edit), "Meiryo Pro");
            change_dpi(&fixture.state, 150);
            assert_eq!(shell::text_of(font_edit), "Meiryo Pro");
            assert_eq!(
                fixture.state.form.borrow().forms[0].draft[font_index].as_deref(),
                Some("Meiryo Pro")
            );

            shell::send(scope, CB_SETCURSEL, WPARAM(1), LPARAM(0));
            SendMessageW(
                fixture.hwnd,
                WM_COMMAND,
                Some(WPARAM(((CBN_SELCHANGE as usize) << 16) | 200)),
                Some(LPARAM(scope.0 as isize)),
            );
            assert_eq!(shell::text_of(font_edit), "Yu Gothic");
        }
    }
}
