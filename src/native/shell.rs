//! The concrete four-pane Win32 shell. It owns presentation, never source.
use super::*;
use mdlite::{
    panel::{PanelId, PanelLayout, Rect, Slot},
    settings::EffectiveSettings,
};
use windows::Win32::UI::Controls::*;

pub(super) const PALETTE: usize = 100;
const SETTINGS: usize = 101;
const COMPACT: usize = 102;
const RETURN: usize = 103;
const RESET: usize = 104;
const PALETTE_EXECUTE: usize = 105;
const PALETTE_CANCEL: usize = 106;
pub(super) const EDITOR_HISTORY_UNDO: usize = 0x7ef0;
const EDITOR_HISTORY_REDO: usize = 0x7ef1;
const ACTIVITY: usize = 110;
const HEADER: usize = 130;
const QUERY: usize = 150;
const RESULTS: usize = 151;
const IDS: [PanelId; 4] = [
    PanelId::Explorer,
    PanelId::Calendar,
    PanelId::Outline,
    PanelId::Git,
];
const NAMES: [&str; 4] = [
    "エクスプローラー",
    "カレンダー",
    "アウトライン",
    "Git（ソース管理）",
];

pub(super) struct OwnerDrawState {
    pub(super) label: String,
    pub(super) enabled: bool,
    pub(super) hovered: bool,
    pub(super) pressed: bool,
    pub(super) focused: bool,
}

// A single registered button's immutable paint inputs, scoped to one native
// enabled-state call. This is not a cache of application or document state.
pub(super) struct OwnerButtonPaint {
    pub(super) recipient: HWND,
    pub(super) parent: HWND,
    pub(super) parent_id: Option<usize>,
    pub(super) child: HWND,
    pub(super) id: usize,
    pub(super) tokens: Tokens,
    pub(super) label: String,
}
impl OwnerButtonPaint {
    pub(super) unsafe fn state(
        &self,
        recipient: HWND,
        message_id: usize,
        item: &DRAWITEMSTRUCT,
    ) -> Option<OwnerDrawState> {
        unsafe {
            if recipient != self.recipient
                || recipient == HWND::default()
                || item.CtlType != ODT_BUTTON
                || item.CtlID as usize != self.id
                || message_id != self.id
                || item.hwndItem != self.child
                || self.child == HWND::default()
                || self.parent == HWND::default()
                || GetParent(self.child).unwrap_or_default() != self.parent
                || GetDlgCtrlID(self.child) as usize != self.id
                || self
                    .parent_id
                    .is_some_and(|id| GetDlgCtrlID(self.parent) as usize != id)
                || (self.parent != recipient
                    && GetParent(self.parent).unwrap_or_default() != recipient)
            {
                return None;
            }
            Some(owner_draw_state(self.label.clone(), item))
        }
    }
}
pub(super) unsafe fn owner_draw_state(label: String, item: &DRAWITEMSTRUCT) -> OwnerDrawState {
    OwnerDrawState {
        label,
        enabled: item.itemState.0 & ODS_DISABLED.0 == 0
            && unsafe { IsWindowEnabled(item.hwndItem).as_bool() },
        hovered: item.itemState.0 & ODS_HOTLIGHT.0 != 0,
        pressed: item.itemState.0 & ODS_SELECTED.0 != 0,
        focused: item.itemState.0 & ODS_FOCUS.0 != 0,
    }
}
pub(super) unsafe fn enable_owner_button(paint: OwnerButtonPaint, enabled: bool) {
    unsafe {
        let child = paint.child;
        if IsWindowEnabled(child).as_bool() == enabled {
            return;
        }
        let pointer = GetWindowLongPtrW(paint.recipient, GWLP_USERDATA) as *const WindowState;
        let _scope = if !pointer.is_null() && (&*pointer).owns_paint_surface(paint.recipient) {
            Some(ButtonPaintScope::new(&*pointer, paint))
        } else {
            None
        };
        // The context's independent RefCell is no longer borrowed here. Native
        // owner drawing may reenter while the caller still exclusively owns App.
        let _ = EnableWindow(child, enabled);
    }
}
const ICONS: [&str; 5] = [
    include_str!("../../assets/ui/icons/explorer.svg"),
    include_str!("../../assets/ui/icons/search.svg"),
    include_str!("../../assets/ui/icons/git-branch.svg"),
    include_str!("../../assets/ui/icons/calendar.svg"),
    include_str!("../../assets/ui/icons/settings.svg"),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    New,
    Open,
    Save,
    SaveAs,
    Close,
    Source,
    Workspace,
    Recovery,
    Compare,
    Reload,
    Discard,
    Palette,
    Settings,
    Compact,
    Return,
    Reset,
    PanelMove(u8),
    PanelResize(bool),
    Feature(usize),
    Search(usize),
    Git(usize),
    Editor(usize),
    Diagnostics,
    Link,
}
impl Command {
    fn id(self) -> usize {
        match self {
            Self::New => CMD_NEW,
            Self::Open => CMD_OPEN,
            Self::Save => CMD_SAVE,
            Self::SaveAs => CMD_SAVE_AS,
            Self::Close => CMD_CLOSE,
            Self::Source => CMD_RAW,
            Self::Workspace => CMD_WORKSPACE,
            Self::Recovery => CMD_RECOVERY,
            Self::Compare => CMD_COMPARE,
            Self::Reload => CMD_RELOAD,
            Self::Discard => CMD_DISCARD_RECOVERY,
            Self::Palette => PALETTE,
            Self::Settings => SETTINGS,
            Self::Compact => COMPACT,
            Self::Return => RETURN,
            Self::Reset => RESET,
            Self::PanelMove(slot) => 160 + slot as usize,
            Self::PanelResize(grow) => {
                if grow {
                    164
                } else {
                    165
                }
            }
            Self::Feature(id) | Self::Search(id) | Self::Git(id) | Self::Editor(id) => id,
            Self::Diagnostics => 167,
            Self::Link => 166,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::New => "新しい文書",
            Self::Open => "文書を開く…",
            Self::Save => "保存",
            Self::SaveAs => "名前を付けて保存…",
            Self::Close => "文書を閉じる",
            Self::Source => "ソース / Live 切替",
            Self::Workspace => "Workspaceを開く…",
            Self::Recovery => "復旧候補…",
            Self::Compare => "ディスクと比較",
            Self::Reload => "ディスクから再読込み…",
            Self::Discard => "復旧候補を破棄…",
            Self::Palette => "コマンドを検索…",
            Self::Settings => "設定…",
            Self::Compact => "Compactへ移す",
            Self::Return => "主windowへ戻す",
            Self::Reset => "パネル配置をリセット",
            Self::PanelMove(slot) => match slot {
                0 => "対象パネルを左上へ",
                1 => "対象パネルを左下へ",
                2 => "対象パネルを右上へ",
                _ => "対象パネルを右下へ",
            },
            Self::PanelResize(grow) => {
                if grow {
                    "対象パネルを高くする"
                } else {
                    "対象パネルを低くする"
                }
            }
            Self::Feature(id) => super::features::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .map_or("操作", |entry| entry.1),
            Self::Search(id) => super::search_ui::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .map_or("検索操作", |entry| entry.1),
            Self::Git(id) => super::git_ui::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .map_or("Git操作", |entry| entry.1),
            Self::Editor(id) => super::editor_actions::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .map_or("編集操作", |entry| entry.1),
            Self::Diagnostics => "診断情報…",
            Self::Link => "キャレット位置のリンクを開く",
        }
    }
    fn binding(self) -> Option<&'static str> {
        match self {
            Self::New => Some("file.new"),
            Self::Open => Some("file.open"),
            Self::Save => Some("file.save"),
            Self::Close => Some("file.close"),
            Self::Palette => Some("view.commandPalette"),
            Self::Search(id) => super::search_ui::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .and_then(|entry| entry.2),
            Self::Git(id) => super::git_ui::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .and_then(|entry| entry.2),
            Self::Feature(id) => super::features::COMMANDS
                .iter()
                .find(|entry| entry.0 == id)
                .and_then(|entry| entry.2),
            _ => None,
        }
    }
}
const COMMANDS: [Command; 24] = [
    Command::New,
    Command::Open,
    Command::Save,
    Command::SaveAs,
    Command::Close,
    Command::Source,
    Command::Workspace,
    Command::Recovery,
    Command::Compare,
    Command::Reload,
    Command::Discard,
    Command::Palette,
    Command::Settings,
    Command::Compact,
    Command::Return,
    Command::Reset,
    Command::PanelMove(0),
    Command::PanelMove(1),
    Command::PanelMove(2),
    Command::PanelMove(3),
    Command::PanelResize(true),
    Command::PanelResize(false),
    Command::Link,
    Command::Diagnostics,
];
fn commands() -> impl Iterator<Item = Command> {
    COMMANDS
        .into_iter()
        .chain(
            super::features::COMMANDS
                .iter()
                .map(|entry| Command::Feature(entry.0)),
        )
        .chain(
            super::search_ui::COMMANDS
                .iter()
                .map(|entry| Command::Search(entry.0)),
        )
        .chain(
            super::git_ui::COMMANDS
                .iter()
                .map(|entry| Command::Git(entry.0)),
        )
        .chain(
            super::editor_actions::COMMANDS
                .iter()
                .map(|entry| Command::Editor(entry.0)),
        )
}

#[derive(Clone, Copy)]
pub(super) struct Tokens {
    pub background: D2D1_COLOR_F,
    pub panel: D2D1_COLOR_F,
    pub foreground: D2D1_COLOR_F,
    pub muted: D2D1_COLOR_F,
    pub selected: D2D1_COLOR_F,
    pub border: D2D1_COLOR_F,
    pub accent: D2D1_COLOR_F,
    pub heading: D2D1_COLOR_F,
    pub code_background: D2D1_COLOR_F,
}

// Native controls ask their parent for colors synchronously, including while
// layout owns App. Own the brush with these paint-only inputs across reentry.
pub(super) struct ControlColors {
    pub(super) tokens: Tokens,
    brush: HBRUSH,
}
impl ControlColors {
    fn new(tokens: Tokens) -> Self {
        Self {
            tokens,
            brush: unsafe { CreateSolidBrush(rgb(tokens.panel)) },
        }
    }
    pub(super) unsafe fn paint(&self, wp: WPARAM) -> LRESULT {
        unsafe {
            let dc = HDC(wp.0 as *mut _);
            SetTextColor(dc, rgb(self.tokens.foreground));
            SetBkColor(dc, rgb(self.tokens.panel));
            LRESULT(self.brush.0 as isize)
        }
    }
}
impl Drop for ControlColors {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.brush.into());
        }
    }
}
impl Default for Tokens {
    fn default() -> Self {
        Self {
            background: color(0.115, 0.125, 0.14),
            panel: color(0.14, 0.155, 0.175),
            foreground: color(0.82, 0.85, 0.88),
            muted: color(0.56, 0.60, 0.65),
            selected: color(0.19, 0.25, 0.33),
            border: color(0.24, 0.27, 0.31),
            accent: color(0.49, 0.69, 0.96),
            heading: color(0.70, 0.82, 0.99),
            code_background: color(0.13, 0.15, 0.18),
        }
    }
}
#[derive(Clone, Copy)]
enum Splitter {
    Width(PanelId),
    Height(PanelId),
}
struct PaletteState {
    host: HWND,
    font: HFONT,
    label: HWND,
    query: HWND,
    list: HWND,
    commands: Vec<Command>,
    focus: HWND,
    document: u64,
    revision: u64,
    selection: Selection,
    affinity: BoundaryAffinity,
    composing: Box<Cell<bool>>,
}
impl Drop for PaletteState {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.host)).as_bool() {
                let _ = DestroyWindow(self.host);
            }
            let _ = DeleteObject(self.font.into());
        }
    }
}
pub(super) struct NativeShell {
    pub enabled: bool,
    pub main_window: HWND,
    pub compact_window: HWND,
    pub settings: EffectiveSettings,
    pub tokens: Tokens,
    pub editor: Rect,
    pub panels: [(PanelId, Rect); 4],
    layout: PanelLayout,
    panel_file: Option<FileSnapshot>,
    panel_writable: bool,
    activity: Vec<HWND>,
    headers: Vec<HWND>,
    command_button: HWND,
    tooltip: HWND,
    search: bool,
    drawer: Option<PanelId>,
    splitter: Option<Splitter>,
    palette: Option<PaletteState>,
    last_client: (u32, u32),
    controls_dirty: bool,
    active_activity: usize,
    active_panel: PanelId,
    pub(super) scroll_hover: bool,
    pub(super) status_eol_cache: Option<(u64, u64, super::presentation::StatusEolSummary)>,
    pub(super) external_change_docs: std::collections::HashSet<u64>,
    main_target: Option<ID2D1HwndRenderTarget>,
    compact_target: Option<ID2D1HwndRenderTarget>,
    colors: Rc<ControlColors>,
    tooltip_text: Vec<Vec<u16>>,
}
impl Default for NativeShell {
    fn default() -> Self {
        Self {
            enabled: false,
            main_window: HWND::default(),
            compact_window: HWND::default(),
            settings: EffectiveSettings::default(),
            tokens: Tokens::default(),
            editor: Rect::default(),
            panels: std::array::from_fn(|i| (IDS[i], Rect::default())),
            layout: PanelLayout::default(),
            panel_file: None,
            panel_writable: false,
            activity: Vec::new(),
            headers: Vec::new(),
            command_button: HWND::default(),
            tooltip: HWND::default(),
            search: false,
            drawer: None,
            splitter: None,
            palette: None,
            last_client: (0, 0),
            controls_dirty: true,
            active_activity: 0,
            active_panel: PanelId::Explorer,
            scroll_hover: false,
            status_eol_cache: None,
            external_change_docs: std::collections::HashSet::new(),
            main_target: None,
            compact_target: None,
            colors: Rc::new(ControlColors::new(Tokens::default())),
            tooltip_text: Vec::new(),
        }
    }
}
impl NativeShell {
    pub(super) fn control_composing(&self) -> bool {
        self.palette
            .as_ref()
            .is_some_and(|palette| palette.composing.get())
    }
    pub(super) fn search_mode(&self) -> bool {
        self.search
    }
    pub(super) fn active_panel(&self) -> PanelId {
        let focus = unsafe { GetFocus() };
        if let Some(index) = self.headers.iter().position(|h| *h == focus) {
            return IDS[index];
        }
        if let Some(index) = self.activity.iter().position(|h| *h == focus) {
            return match index {
                0 | 1 => PanelId::Explorer,
                2 => PanelId::Git,
                3 => PanelId::Calendar,
                _ => self.active_panel,
            };
        }
        self.active_panel
    }
}
pub(super) fn rgb(c: D2D1_COLOR_F) -> COLORREF {
    COLORREF((c.r * 255.) as u32 | ((c.g * 255.) as u32) << 8 | ((c.b * 255.) as u32) << 16)
}
fn from_rgb(c: u32) -> D2D1_COLOR_F {
    color(
        (c & 255) as f32 / 255.,
        ((c >> 8) & 255) as f32 / 255.,
        ((c >> 16) & 255) as f32 / 255.,
    )
}

fn apply_custom_color_token(tokens: &mut Tokens, key: &str, value: &str) {
    let Ok(value) = u32::from_str_radix(value.trim_start_matches('#'), 16) else {
        return;
    };
    let color = color(
        (value >> 16 & 255) as f32 / 255.,
        (value >> 8 & 255) as f32 / 255.,
        (value & 255) as f32 / 255.,
    );
    match key {
        "background" => tokens.background = color,
        "foreground" => tokens.foreground = color,
        "link" => tokens.accent = color,
        "heading" => tokens.heading = color,
        "marker" => tokens.muted = color,
        "code_background" => tokens.code_background = color,
        "table_background" => tokens.panel = color,
        _ => {}
    }
}
fn d2d(r: Rect) -> D2D_RECT_F {
    rect(r.x, r.y, r.width, r.height)
}
pub(super) fn text_of(hwnd: HWND) -> String {
    unsafe {
        let mut text = vec![0u16; GetWindowTextLengthW(hwnd).max(0) as usize + 1];
        let n = GetWindowTextW(hwnd, &mut text);
        String::from_utf16_lossy(&text[..n as usize])
    }
}
pub(super) unsafe fn control(
    parent: HWND,
    class: PCWSTR,
    label: &str,
    id: usize,
    style: WINDOW_STYLE,
) -> HWND {
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            class,
            PCWSTR(wide(label).as_ptr()),
            WS_CHILD
                | WS_VISIBLE
                | WS_CLIPSIBLINGS
                | if class.to_string().is_ok_and(|s| s == "STATIC") {
                    WINDOW_STYLE(0)
                } else {
                    WS_TABSTOP
                }
                | style,
            0,
            0,
            10,
            10,
            Some(parent),
            Some(HMENU(id as *mut _)),
            None,
            None,
        )
        .unwrap_or_default()
    }
}

unsafe fn child_control_state(hwnd: HWND) -> Option<([i32; 4], bool)> {
    unsafe {
        let mut process = 0;
        let thread = GetWindowThreadProcessId(hwnd, Some(&mut process));
        if process != std::process::id()
            || thread != windows_sys::Win32::System::Threading::GetCurrentThreadId()
        {
            return None;
        }
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE) as u32;
        if style & WS_CHILD.0 == 0 {
            return None;
        }
        let parent = GetParent(hwnd).ok()?;
        let mut parent_process = 0;
        if GetWindowThreadProcessId(parent, Some(&mut parent_process)) != thread
            || parent_process != process
        {
            return None;
        }
        let mut bounds = RECT::default();
        GetWindowRect(hwnd, &mut bounds).ok()?;
        let mut points = [
            POINT {
                x: bounds.left,
                y: bounds.top,
            },
            POINT {
                x: bounds.right,
                y: bounds.bottom,
            },
        ];
        // Zero is also a successful zero-offset map. Two points preserve RECT
        // mapping when a parent uses mirrored coordinates.
        SetLastError(ERROR_SUCCESS);
        if MapWindowPoints(None, Some(parent), &mut points) == 0 && GetLastError() != ERROR_SUCCESS
        {
            return None;
        }
        if GetParent(hwnd).ok() != Some(parent) || !IsWindow(Some(hwnd)).as_bool() {
            return None;
        }
        Some((
            [
                points[0].x,
                points[0].y,
                points[1].x - points[0].x,
                points[1].y - points[0].y,
            ],
            style & WS_VISIBLE.0 != 0,
        ))
    }
}

unsafe extern "system" fn palette_host_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    owner: usize,
) -> LRESULT {
    unsafe {
        if matches!(
            msg,
            WM_COMMAND | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
        ) {
            return SendMessageW(HWND(owner as *mut _), msg, Some(wp), Some(lp));
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}

pub(super) unsafe fn position_child_control(
    hwnd: HWND,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> windows::core::Result<()> {
    unsafe {
        if child_control_state(hwnd).is_some_and(|(bounds, _)| bounds == [x, y, width, height]) {
            return Ok(());
        }
        SetWindowPos(
            hwnd,
            None,
            x,
            y,
            width,
            height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        )
    }
}

pub(super) unsafe fn show_child_control(hwnd: HWND, visible: bool) -> windows::core::BOOL {
    unsafe {
        if child_control_state(hwnd).is_some_and(|(_, shown)| shown == visible) {
            return windows::core::BOOL(i32::from(visible));
        }
        if !visible && (GetFocus() == hwnd || IsChild(hwnd, GetFocus()).as_bool()) {
            let _ = SetFocus(Some(GetAncestor(hwnd, GA_ROOT)));
        }
        ShowWindow(hwnd, if visible { SW_SHOW } else { SW_HIDE })
    }
}
fn shell_geometry(
    layout: &PanelLayout,
    width: f32,
    height: f32,
    drawer: Option<PanelId>,
) -> (Rect, [(PanelId, Rect); 4]) {
    let mut layout = layout.clone();
    let narrow = width < 760.;
    if narrow {
        for p in &mut layout.panels {
            p.hidden = true;
        }
    }
    let geometry = layout.geometry((width - 4.).max(0.), height, 36., 24.);
    let mut panels = geometry.panels.map(|(id, mut r)| {
        if r.width > 0. {
            r.x += 4.;
        }
        (id, r)
    });
    if narrow && let Some(id) = drawer {
        panels[id as usize].1 = Rect {
            x: 48.,
            y: 36.,
            width: (width - 48.).clamp(0., 300.),
            height: (height - 60.).max(0.),
        };
    }
    let center = geometry.editor;
    (
        Rect {
            x: center.x + 4. + 20.,
            y: 88.,
            width: (center.width - 40.).max(1.),
            height: (height - 112.).max(1.),
        },
        panels,
    )
}
impl App {
    pub(super) fn parent_root_diagnostic(&self) -> serde_json::Value {
        unsafe {
            let main = self.shell.main_window;
            if main == HWND::default() || !IsWindow(Some(main)).as_bool() {
                return serde_json::Value::Null;
            }
            let raw_dpi = GetDpiForWindow(main);
            if raw_dpi == 0 {
                return serde_json::Value::Null;
            }
            let dpi = raw_dpi as f32 / 96.;
            let mut client = RECT::default();
            let mut window = RECT::default();
            if GetClientRect(main, &mut client).is_err()
                || GetWindowRect(main, &mut window).is_err()
            {
                return serde_json::Value::Null;
            }
            let (editor, panels) = shell_geometry(
                &self.shell.layout,
                (client.right - client.left) as f32 / dpi,
                (client.bottom - client.top) as f32 / dpi,
                self.shell.drawer,
            );
            let rectangle =
                |r: Rect| serde_json::json!({"x":r.x,"y":r.y,"width":r.width,"height":r.height});
            serde_json::json!({"schema":1,"hwnd":main.0 as usize,"editor_hwnd":self.editing_window().0 as usize,"compact_hwnd":self.shell.compact_window.0 as usize,"dpi":dpi,"dpi_raw":raw_dpi,
            "client_rect":{"left":client.left,"top":client.top,"right":client.right,"bottom":client.bottom},"window_rect":{"left":window.left,"top":window.top,"right":window.right,"bottom":window.bottom},
            "editor_rect":rectangle(editor),"active_editor_rect":rectangle(self.shell.editor),"search_active":self.search_pane_active(),
            "panel_rects":panels.iter().map(|(id,r)|{let state=self.shell.layout.get(*id);serde_json::json!({"id":(["explorer","calendar","outline","git"][*id as usize]),"rect":rectangle(*r),"body":rectangle(Rect{x:r.x,y:r.y+30.,width:r.width,height:(r.height-30.).max(0.)}),"hidden":state.hidden,"collapsed":state.collapsed,"slot":format!("{:?}",state.slot)})}).collect::<Vec<_>>(),
            "features":self.feature_test_geometry(&panels),"search_busy":self.search_busy(),"git_job_busy":self.git_job_busy(),"mutation_busy":self.background_mutation_busy()})
        }
    }
    pub(super) fn feature_panel_rect(&self, id: PanelId) -> Option<Rect> {
        let rect = self.shell.panels[id as usize].1;
        (self.hwnd == self.shell.main_window && rect.width > 0. && rect.height > 30.).then_some(
            Rect {
                x: rect.x,
                y: rect.y + 30.,
                width: rect.width,
                height: (rect.height - 30.).max(0.),
            },
        )
    }
    pub(super) fn search_pane_active(&self) -> bool {
        self.shell.search_mode()
    }
    pub(super) fn set_search_pane_active(&mut self, active: bool) {
        // The visibility helper hands off focus only from controls being hidden.
        // Search's open path then prepares and focuses its visible input.
        self.shell.search = active;
        self.show_panel(PanelId::Explorer);
    }
    pub(super) fn feature_body_focused(&self) -> bool {
        let focus = unsafe { GetFocus() };
        self.shell.headers.contains(&focus)
    }
    pub(super) fn focus_feature_body(&mut self, id: PanelId) {
        self.shell.active_panel = id;
        unsafe {
            let _ = SetFocus(Some(self.shell.headers[id as usize]));
        }
    }
    pub(super) fn focus_feature_panel(&mut self, id: PanelId) {
        self.shell.active_panel = id;
    }
    pub(super) fn editor_left(&self) -> f32 {
        if self.shell.enabled {
            self.shell.editor.x
        } else {
            LEFT
        }
    }
    pub(super) fn editor_top(&self) -> f32 {
        if self.shell.enabled {
            self.shell.editor.y
        } else {
            TOP
        }
    }
    pub(super) fn editor_width(&self) -> f32 {
        if self.shell.enabled {
            self.shell.editor.width
        } else {
            (self.width - LEFT * 2.).max(1.)
        }
    }
    pub(super) fn editor_right(&self) -> f32 {
        self.editor_left() + self.editor_width()
    }
    pub(super) fn editor_bottom(&self) -> f32 {
        if self.shell.enabled {
            self.shell.editor.y + self.shell.editor.height
        } else {
            self.animation_viewport()
        }
    }
    pub(super) fn font_size(&self) -> f32 {
        if self.shell.enabled {
            self.shell.settings.font_size_pt as f32 * 96. / 72.
        } else {
            FONT
        }
    }
    pub(super) fn editing_window(&self) -> HWND {
        if self.shell.compact_window != HWND::default() {
            self.shell.compact_window
        } else {
            self.hwnd
        }
    }
    pub(super) fn select_surface(&mut self, hwnd: HWND) {
        if self.hwnd != hwnd {
            if self.animation_timer_armed {
                unsafe {
                    let _ = KillTimer(Some(self.hwnd), ANIMATION_TIMER);
                }
                self.animation_timer_armed = false;
            }
            if self.hwnd == self.shell.main_window {
                self.shell.main_target = self.graphics.target.take();
            } else if self.hwnd == self.shell.compact_window {
                self.shell.compact_target = self.graphics.target.take();
            }
            self.graphics.target = if hwnd == self.shell.main_window {
                self.shell.main_target.take()
            } else {
                self.shell.compact_target.take()
            };
            self.layout_key = None;
            self.full_paint = true;
            self.image_cache.clear();
        }
        self.hwnd = hwnd;
        if self.shell.enabled && unsafe { IsWindow(Some(hwnd)).as_bool() } {
            self.dpi = unsafe { GetDpiForWindow(hwnd) } as f32 / 96.;
            self.minimized = unsafe { IsIconic(hwnd).as_bool() };
        }
    }
    pub(super) unsafe fn start_shell(&mut self) {
        unsafe {
            self.shell.enabled = true;
            self.shell.main_window = self.hwnd;
            self.shell.command_button = control(
                self.hwnd,
                w!("BUTTON"),
                "コマンドを検索… Ctrl+K",
                PALETTE,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            for (i, label) in [
                "エクスプローラー",
                "検索",
                "Git（ソース管理）",
                "カレンダー",
                "設定",
            ]
            .into_iter()
            .enumerate()
            {
                let h = control(
                    self.hwnd,
                    w!("BUTTON"),
                    label,
                    ACTIVITY + i,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
                self.shell.activity.push(h);
            }
            for (i, label) in NAMES.into_iter().enumerate() {
                self.shell.headers.push(control(
                    self.hwnd,
                    w!("BUTTON"),
                    &format!("{label}の配置・表示"),
                    HEADER + i,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                ));
            }
            self.shell.tooltip = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP),
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
            for h in self
                .shell
                .activity
                .iter()
                .chain(self.shell.headers.iter())
                .chain(std::iter::once(&self.shell.command_button))
            {
                self.shell.tooltip_text.push(wide(&text_of(*h)));
                let mut tool = TTTOOLINFOW {
                    cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                    uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                    hwnd: self.hwnd,
                    uId: h.0 as usize,
                    lpszText: PWSTR(self.shell.tooltip_text.last_mut().unwrap().as_mut_ptr()),
                    ..Default::default()
                };
                send(
                    self.shell.tooltip,
                    TTM_ADDTOOLW,
                    WPARAM(0),
                    LPARAM((&mut tool as *mut TTTOOLINFOW) as isize),
                );
            }
            self.apply_shell_settings(self.shell.settings.clone());
            self.install_shell_menu();
        }
    }
    pub(super) unsafe fn install_shell_menu(&mut self) {
        unsafe {
            let (Ok(menu), Ok(file), Ok(view), Ok(workspace), Ok(search), Ok(git), Ok(editor)) = (
                CreateMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
            ) else {
                return;
            };
            for command in commands() {
                let destination = if matches!(command, Command::Feature(_)) {
                    workspace
                } else if matches!(command, Command::Editor(_)) {
                    editor
                } else if matches!(command, Command::Search(_)) {
                    search
                } else if matches!(command, Command::Git(_)) {
                    git
                } else if matches!(
                    command,
                    Command::Diagnostics
                        | Command::Palette
                        | Command::Settings
                        | Command::Compact
                        | Command::Return
                        | Command::Reset
                        | Command::Source
                        | Command::PanelMove(_)
                        | Command::PanelResize(_)
                ) {
                    view
                } else {
                    file
                };
                let flags = MF_STRING
                    | if self.command_enabled(command) {
                        MENU_ITEM_FLAGS(0)
                    } else {
                        MF_GRAYED
                    };
                let _ = AppendMenuW(
                    destination,
                    flags,
                    command.id(),
                    PCWSTR(wide(&self.command_label(command)).as_ptr()),
                );
            }
            let _ = AppendMenuW(menu, MF_POPUP, file.0 as usize, w!("ファイル"));
            let _ = AppendMenuW(menu, MF_POPUP, view.0 as usize, w!("表示"));
            let _ = AppendMenuW(
                menu,
                MF_POPUP,
                workspace.0 as usize,
                w!("Workspace・ノート"),
            );
            let _ = AppendMenuW(menu, MF_POPUP, search.0 as usize, w!("検索・置換"));
            let _ = AppendMenuW(menu, MF_POPUP, git.0 as usize, w!("Git・storage"));
            let _ = AppendMenuW(menu, MF_POPUP, editor.0 as usize, w!("表・画像"));
            let owner = if self.shell.main_window != HWND::default() {
                self.shell.main_window
            } else {
                self.hwnd
            };
            let old = GetMenu(owner);
            let _ = SetMenu(owner, Some(menu));
            if old != HMENU::default() {
                let _ = DestroyMenu(old);
            }
            let _ = DrawMenuBar(owner);
        }
    }
    pub(super) unsafe fn append_shell_context(&self, menu: HMENU) {
        unsafe {
            for command in [
                Command::Save,
                Command::Source,
                Command::Compact,
                Command::Settings,
                Command::Link,
            ] {
                let _ = AppendMenuW(
                    menu,
                    MF_STRING
                        | if self.command_enabled(command) {
                            MF_ENABLED
                        } else {
                            MF_GRAYED
                        },
                    command.id(),
                    PCWSTR(wide(&self.command_label(command)).as_ptr()),
                );
            }
        }
    }
    pub(super) fn is_shell_command(&self, id: usize) -> bool {
        commands().any(|c| c.id() == id)
    }
    fn command_label(&self, c: Command) -> String {
        let hint = c
            .binding()
            .and_then(|k| self.shell.settings.keybindings.get(k))
            .map(String::as_str)
            .or(match c {
                Command::SaveAs => Some("Ctrl+Shift+S"),
                Command::Editor(38) => Some("Ctrl+Shift+T"),
                Command::Editor(39) => Some("Ctrl+Alt+T"),
                Command::PanelMove(0) => Some("Ctrl+Alt+1"),
                Command::PanelMove(1) => Some("Ctrl+Alt+2"),
                Command::PanelMove(2) => Some("Ctrl+Alt+3"),
                Command::PanelMove(_) => Some("Ctrl+Alt+4"),
                Command::PanelResize(true) => Some("Ctrl+Alt+Up"),
                Command::PanelResize(false) => Some("Ctrl+Alt+Down"),
                _ => None,
            });
        let label = if self.command_enabled(c) {
            c.label().to_owned()
        } else {
            format!("{} — {}", c.label(), self.command_disabled_reason(c))
        };
        match hint {
            Some(h) => format!("{label}\t{h}"),
            None => label,
        }
    }
    fn command_disabled_reason(&self, c: Command) -> &'static str {
        if self.feature_ime_active() {
            return "入力を確定してから操作してください";
        }
        if self.background_mutation_busy() {
            return "変更処理を取消すか完了を待ってください";
        }
        match c {
            Command::Editor(id) => self.editor_disabled_reason(id),
            Command::Feature(id) => self.features_disabled_reason(id),
            Command::Search(id) => self.search_disabled_reason(id),
            Command::Git(id) => self.git_disabled_reason(id),
            Command::Link => "リンクにキャレットを置いてください",
            Command::Compare | Command::Reload => "保存済み文書が必要です",
            Command::Recovery | Command::Discard => "Workspaceを開いてください",
            Command::Compact => "既にCompactで表示しています",
            Command::Return => "Compactではありません",
            _ => "現在の状態では実行できません",
        }
    }
    fn command_enabled(&self, c: Command) -> bool {
        match c {
            Command::Editor(id) => return self.editor_enabled(id),
            Command::Feature(id) => {
                return !self.background_mutation_busy() && self.features_enabled(id);
            }
            Command::Search(id) => {
                return (!self.background_mutation_busy() || id == 5010) && self.search_enabled(id);
            }
            Command::Git(id) => return self.git_enabled(id),
            _ => {}
        }
        if self.background_mutation_busy()
            && !matches!(
                c,
                Command::Palette
                    | Command::Return
                    | Command::PanelMove(_)
                    | Command::PanelResize(_)
            )
        {
            return false;
        }
        if self.feature_ime_active() {
            return false;
        }
        match c {
            Command::Link => self
                .link_destination_at(self.doc().selection().head)
                .is_some(),
            Command::Return => self.shell.compact_window != HWND::default(),
            Command::Compact => self.shell.compact_window == HWND::default(),
            Command::Compare | Command::Reload => self.tabs[self.active].file.is_some(),
            Command::Recovery | Command::Discard => self.persistence.workspace_root().is_some(),
            _ => true,
        }
    }
    pub(super) fn update_shell_geometry(&mut self, height: f32) {
        if !self.shell.enabled {
            return;
        }
        let compact = self.hwnd == self.shell.compact_window;
        let (editor, panels) = if compact {
            (
                Rect {
                    x: 24.,
                    y: TOP,
                    width: (self.width - 48.).max(1.),
                    height: (height - TOP - 24.).max(1.),
                },
                std::array::from_fn(|i| (IDS[i], Rect::default())),
            )
        } else {
            shell_geometry(&self.shell.layout, self.width, height, self.shell.drawer)
        };
        self.shell.editor = editor;
        self.shell.panels = panels;
        if compact {
            return;
        }
        let size = (self.width.to_bits(), height.to_bits());
        if size == self.shell.last_client && !self.shell.controls_dirty {
            return;
        }
        self.shell.last_client = size;
        self.shell.controls_dirty = false;
        unsafe {
            let dpi = self.dpi;
            let position = |h: HWND, r: Rect| {
                let _ = position_child_control(
                    h,
                    (r.x * dpi) as i32,
                    (r.y * dpi) as i32,
                    (r.width * dpi).max(0.) as i32,
                    (r.height * dpi).max(0.) as i32,
                );
            };
            for (i, h) in self.shell.activity.iter().enumerate() {
                position(
                    *h,
                    Rect {
                        x: 0.,
                        y: if i == 4 {
                            (height - 68.).max(36.)
                        } else {
                            36. + i as f32 * 44.
                        },
                        width: 48.,
                        height: 44.,
                    },
                );
            }
            let w = (self.width - 160.).clamp(80., 500.);
            position(
                self.shell.command_button,
                Rect {
                    x: (self.width - w) / 2.,
                    y: 4.,
                    width: w,
                    height: 28.,
                },
            );
            for (i, h) in self.shell.headers.iter().enumerate() {
                let r = panels[i].1;
                let _ = show_child_control(*h, r.width > 0.);
                position(
                    *h,
                    Rect {
                        x: r.x + r.width - 32.,
                        y: r.y,
                        width: 32.,
                        height: 30.,
                    },
                );
            }
            self.position_palette();
        }
    }
    pub(super) unsafe fn apply_shell_settings(&mut self, settings: EffectiveSettings) {
        unsafe {
            use mdlite::settings::ThemeMode;
            self.shell.settings = settings;
            let light = match self.shell.settings.theme {
                ThemeMode::Light => true,
                ThemeMode::System => system_light(),
                _ => false,
            };
            let mut tokens = if light {
                Tokens {
                    background: color(0.98, 0.98, 0.98),
                    panel: color(0.94, 0.95, 0.96),
                    foreground: color(0.12, 0.15, 0.19),
                    muted: color(0.39, 0.43, 0.48),
                    selected: color(0.80, 0.87, 0.96),
                    border: color(0.76, 0.79, 0.82),
                    accent: color(0.16, 0.37, 0.67),
                    heading: color(0.12, 0.30, 0.60),
                    code_background: color(0.93, 0.95, 0.97),
                }
            } else {
                Tokens::default()
            };
            if self.shell.settings.theme == ThemeMode::Custom {
                for (k, v) in &self.shell.settings.colors {
                    apply_custom_color_token(&mut tokens, k, v);
                }
            }
            #[repr(C)]
            struct Contrast {
                size: u32,
                flags: u32,
                scheme: PWSTR,
            }
            let mut contrast = Contrast {
                size: std::mem::size_of::<Contrast>() as u32,
                flags: 0,
                scheme: PWSTR::null(),
            };
            let _ = SystemParametersInfoW(
                SYSTEM_PARAMETERS_INFO_ACTION(0x42),
                contrast.size,
                Some((&mut contrast as *mut Contrast).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            );
            if contrast.flags & 1 != 0 {
                tokens.background = from_rgb(GetSysColor(COLOR_WINDOW));
                tokens.panel = from_rgb(GetSysColor(COLOR_BTNFACE));
                tokens.foreground = from_rgb(GetSysColor(COLOR_WINDOWTEXT));
                tokens.muted = tokens.foreground;
                tokens.border = tokens.foreground;
                tokens.selected = from_rgb(GetSysColor(COLOR_HIGHLIGHT));
                tokens.accent = tokens.foreground;
                tokens.heading = tokens.foreground;
                tokens.code_background = tokens.background;
            }
            self.shell.tokens = tokens;
            self.shell.colors = Rc::new(ControlColors::new(tokens));
            let pointer =
                GetWindowLongPtrW(self.shell.main_window, GWLP_USERDATA) as *const WindowState;
            if !pointer.is_null() && (&*pointer).owns_paint_surface(self.shell.main_window) {
                *(&*pointer).control_colors.borrow_mut() = Some(Rc::clone(&self.shell.colors));
            }
            let dark = windows::core::BOOL(i32::from(!light));
            let _ = DwmSetWindowAttribute(
                self.shell.main_window,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const windows::core::BOOL).cast(),
                std::mem::size_of_val(&dark) as u32,
            );
            self.layout_key = None;
            self.shell.controls_dirty = true;
            let _ = InvalidateRect(Some(self.hwnd), None, true);
        }
    }
    pub(super) unsafe fn load_panel_layout(&mut self) {
        unsafe {
            self.shell.layout = PanelLayout::default();
            self.shell.panel_file = None;
            self.shell.panel_writable = false;
            let Some(root) = self.persistence.workspace_root() else {
                return;
            };
            let path = match super::persistence::bounded_path(
                &root,
                Path::new(".mdlite/.state/panel-layout.toml"),
            ) {
                Ok(p) => p,
                Err(e) => {
                    self.status = e.to_string();
                    return;
                }
            };
            match FileSnapshot::open(path) {
                Ok(loaded) => match PanelLayout::parse(&loaded.source) {
                    Ok(layout) => {
                        self.shell.layout = layout;
                        self.shell.panel_file = Some(loaded.snapshot);
                        self.shell.panel_writable = true;
                    }
                    Err(e) => {
                        self.notice("パネル配置を読み取れません — 原本保持", e);
                    }
                },
                Err(mdlite::storage::StorageError::Io(e))
                    if e.kind() == std::io::ErrorKind::NotFound =>
                {
                    self.shell.panel_writable = true
                }
                Err(e) => self.notice("パネル配置を読み取れません — 原本保持", &e.to_string()),
            }
            self.shell.controls_dirty = true;
            self.layout_key = None;
        }
    }
    fn save_panel_layout(&mut self) {
        if !self.shell.panel_writable {
            return;
        }
        let Some(root) = self.persistence.workspace_root() else {
            return;
        };
        let text = self.shell.layout.serialize();
        let result = match self.shell.panel_file.as_mut() {
            Some(file) => file.save(&text),
            None => {
                match super::persistence::bounded_path(
                    &root,
                    Path::new(".mdlite/.state/panel-layout.toml"),
                ) {
                    Ok(path) => FileSnapshot::create_new(path, &text, FileFormat::default())
                        .map(|f| self.shell.panel_file = Some(f)),
                    Err(e) => Err(e.into()),
                }
            }
        };
        if let Err(e) = result {
            self.status = format!("パネル配置保存失敗 — 原本保持: {e}");
        }
    }
    pub(super) unsafe fn draw_shell(&self, t: &ID2D1HwndRenderTarget) -> windows::core::Result<()> {
        unsafe {
            if !self.shell.enabled {
                return Ok(());
            }
            let panel = t.CreateSolidColorBrush(&self.shell.tokens.panel, None)?;
            let border = t.CreateSolidColorBrush(&self.shell.tokens.border, None)?;
            let fg = t.CreateSolidColorBrush(&self.shell.tokens.foreground, None)?;
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let h = r.bottom as f32 / self.dpi;
            if self.hwnd == self.shell.compact_window {
                let text = self.text_layout(
                    "Compact — ×で主windowへ戻す",
                    (self.width - 152.).max(1.),
                    12.,
                )?;
                t.DrawTextLayout(point(16., 8.), &text, &fg, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                return Ok(());
            }
            t.FillRectangle(&rect(0., 0., self.width, 36.), &panel);
            t.FillRectangle(&rect(0., 36., 48., (h - 60.).max(0.)), &panel);
            for (id, r) in self.shell.panels {
                if r.width == 0. {
                    continue;
                }
                t.FillRectangle(&rect(r.x, r.y, r.width, r.height.min(30.)), &panel);
                if let Some(body) = self.feature_panel_rect(id) {
                    t.FillRectangle(&d2d(body), &panel);
                }
                t.DrawRectangle(&d2d(r), &border, 1., None);
                let name = if id == PanelId::Explorer && self.search_pane_active() {
                    "検索"
                } else {
                    NAMES[id as usize]
                };
                let text = self.text_layout(name, (r.width - 42.).max(1.), 12.)?;
                t.DrawTextLayout(
                    point(r.x + 12., r.y + 7.),
                    &text,
                    &fg,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            if self.shell.compact_window != HWND::default() {
                let label = self.text_layout(
                    "文書はCompactで編集中です。表示メニューから主windowへ戻せます。",
                    self.editor_width(),
                    14.,
                )?;
                t.DrawTextLayout(
                    point(self.editor_left(), self.editor_top()),
                    &label,
                    &fg,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }
    pub(super) unsafe fn draw_shell_button(&self, item: &DRAWITEMSTRUCT) {
        unsafe {
            let control_dpi = (GetDpiForWindow(item.hwndItem) as f32 / 96.).max(1.);
            let selected = item.itemState.0 & ODS_SELECTED.0 != 0;
            let bg = if selected || item.CtlID as usize == ACTIVITY + self.shell.active_activity {
                self.shell.tokens.selected
            } else {
                self.shell.tokens.panel
            };
            let brush = CreateSolidBrush(rgb(bg));
            FillRect(item.hDC, &item.rcItem, brush);
            let _ = DeleteObject(brush.into());
            let id = item.CtlID as usize;
            if id == ACTIVITY + self.shell.active_activity {
                let brush = CreateSolidBrush(rgb(self.shell.tokens.accent));
                let stripe = RECT {
                    left: 0,
                    top: 0,
                    right: (3. * control_dpi).round() as i32,
                    bottom: item.rcItem.bottom,
                };
                FillRect(item.hDC, &stripe, brush);
                let _ = DeleteObject(brush.into());
            }
            if (ACTIVITY..ACTIVITY + 5).contains(&id) {
                draw_icon(
                    item.hDC,
                    &item.rcItem,
                    ICONS[id - ACTIVITY],
                    control_dpi,
                    self.shell.tokens.foreground,
                    bg,
                );
            } else {
                let text = if (HEADER..HEADER + 4).contains(&id) {
                    "⋯".into()
                } else {
                    text_of(item.hwndItem)
                };
                let mut area = item.rcItem;
                let _ = SetBkMode(item.hDC, TRANSPARENT);
                SetTextColor(item.hDC, rgb(self.shell.tokens.foreground));
                draw_gdi_text(
                    item.hDC,
                    &text,
                    &mut area,
                    DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS,
                );
            }
            if item.itemState.0 & ODS_FOCUS.0 != 0 {
                let _ = DrawFocusRect(item.hDC, &item.rcItem);
            }
        }
    }
}
pub(super) unsafe fn draw_owner_button(
    tokens: Tokens,
    item: &DRAWITEMSTRUCT,
    state: &OwnerDrawState,
) {
    unsafe {
        let dpi = (GetDpiForWindow(item.hwndItem) as f32 / 96.).max(1.);
        let active = state.hovered || state.pressed;
        let background = if active {
            tokens.selected
        } else {
            tokens.panel
        };
        let border = if active || state.focused {
            tokens.accent
        } else {
            tokens.border
        };
        let foreground = if state.enabled {
            tokens.foreground
        } else {
            tokens.muted
        };
        let brush = CreateSolidBrush(rgb(background));
        FillRect(item.hDC, &item.rcItem, brush);
        let _ = DeleteObject(brush.into());

        let border_brush = CreateSolidBrush(rgb(border));
        FrameRect(item.hDC, &item.rcItem, border_brush);
        let _ = DeleteObject(border_brush.into());

        let mut area = RECT {
            left: item.rcItem.left + (4. * dpi).round() as i32,
            top: item.rcItem.top + (1. * dpi).round() as i32,
            right: item.rcItem.right - (4. * dpi).round() as i32,
            bottom: item.rcItem.bottom - (1. * dpi).round() as i32,
        };
        if state.pressed {
            let offset = (1. * dpi).round() as i32;
            area.left += offset;
            area.top += offset;
            area.right += offset;
            area.bottom += offset;
        }
        let _ = SetBkMode(item.hDC, TRANSPARENT);
        SetTextColor(item.hDC, rgb(foreground));
        draw_gdi_text(
            item.hDC,
            &state.label,
            &mut area,
            DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        if state.focused {
            let inset = (3. * dpi).round() as i32;
            let focus = RECT {
                left: item.rcItem.left + inset,
                top: item.rcItem.top + inset,
                right: item.rcItem.right - inset,
                bottom: item.rcItem.bottom - inset,
            };
            let _ = DrawFocusRect(item.hDC, &focus);
        }
    }
}
impl App {
    pub(super) unsafe fn shell_window_message(
        &mut self,
        hwnd: HWND,
        msg: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> Option<LRESULT> {
        unsafe {
            if !self.shell.enabled {
                return None;
            }
            match msg {
                WM_GETMINMAXINFO => {
                    let info = &mut *(lp.0 as *mut MINMAXINFO);
                    info.ptMinTrackSize = POINT {
                        x: (360. * self.dpi) as i32,
                        y: (260. * self.dpi) as i32,
                    };
                    Some(LRESULT(0))
                }
                WM_DRAWITEM => {
                    if lp.0 == 0 {
                        return None;
                    }
                    let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                    let id = item.CtlID as usize;
                    if hwnd == self.shell.main_window
                        && super::search_ui::draw_owned_button(hwnd, wp.0, item, self.shell.tokens)
                    {
                        return Some(LRESULT(1));
                    }
                    if hwnd == self.shell.main_window
                        && let Some(state) = self.features.owner_draw_button(hwnd, wp.0, item)
                    {
                        draw_owner_button(self.shell.tokens, item, &state);
                        return Some(LRESULT(1));
                    }
                    if hwnd == self.shell.main_window
                        && let Some(state) = self.git_ui.owner_draw_button(hwnd, wp.0, item)
                    {
                        draw_owner_button(self.shell.tokens, item, &state);
                        return Some(LRESULT(1));
                    }
                    if let Some(state) = self.editor_actions.owner_draw_button(
                        hwnd,
                        self.shell.main_window,
                        self.shell.compact_window,
                        wp.0,
                        item,
                    ) {
                        draw_owner_button(self.shell.tokens, item, &state);
                        return Some(LRESULT(1));
                    }
                    if id != PALETTE
                        && !(ACTIVITY..ACTIVITY + 5).contains(&id)
                        && !(HEADER..HEADER + 4).contains(&id)
                    {
                        return None;
                    }
                    self.draw_shell_button(item);
                    Some(LRESULT(1))
                }
                WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
                    if hwnd == self.shell.main_window =>
                {
                    Some(self.shell.colors.paint(wp))
                }
                WM_SETTINGCHANGE | WM_THEMECHANGED => {
                    self.apply_shell_settings(self.shell.settings.clone());
                    self.refresh();
                    Some(LRESULT(0))
                }
                WM_INITMENUPOPUP => {
                    let menu = HMENU(wp.0 as *mut _);
                    for c in commands() {
                        let flags = MF_BYCOMMAND
                            | MF_STRING
                            | if self.command_enabled(c) {
                                MF_ENABLED
                            } else {
                                MF_GRAYED
                            };
                        let _ = ModifyMenuW(
                            menu,
                            c.id() as u32,
                            flags,
                            c.id(),
                            PCWSTR(wide(&self.command_label(c)).as_ptr()),
                        );
                    }
                    Some(LRESULT(0))
                }
                WM_TIMER
                    if hwnd == self.shell.main_window
                        && self.shell.compact_window != HWND::default() =>
                {
                    self.select_surface(self.shell.compact_window);
                    None
                }
                WM_COMMAND if wp.0 & 0xffff == QUERY && wp.0 >> 16 == EN_CHANGE as usize => {
                    self.filter_palette();
                    Some(LRESULT(0))
                }
                WM_CLOSE if hwnd == self.shell.compact_window => {
                    self.return_from_compact();
                    Some(LRESULT(0))
                }
                WM_KEYDOWN | WM_CHAR if hwnd != self.editing_window() => Some(LRESULT(0)),
                _ => None,
            }
        }
    }
    pub(super) unsafe fn shell_command(&mut self, id: usize) -> bool {
        unsafe {
            if !self.shell.enabled {
                return false;
            }
            if matches!(id, EDITOR_HISTORY_UNDO | EDITOR_HISTORY_REDO) {
                if self.composing || self.feature_ime_active() || self.background_mutation_busy() {
                    return true;
                }
                self.doc_mut().break_undo_group();
                self.pending_surrogate = None;
                self.pending_table_authoring = None;
                self.preferred_x = None;
                if id == EDITOR_HISTORY_REDO {
                    self.doc_mut().redo();
                } else {
                    self.doc_mut().undo();
                }
                self.after_input();
                return true;
            }
            if id == PALETTE_EXECUTE {
                self.execute_palette();
                return true;
            }
            if id == PALETTE_CANCEL {
                self.close_palette(true);
                return true;
            }
            if (ACTIVITY..ACTIVITY + 5).contains(&id) {
                if id - ACTIVITY < 4 {
                    self.shell.active_activity = id - ACTIVITY;
                }
                match id - ACTIVITY {
                    0 => self.set_search_pane_active(false),
                    1 => {
                        self.command(5001);
                    }
                    2 => self.show_panel(PanelId::Git),
                    3 => self.show_panel(PanelId::Calendar),
                    _ => self.show_settings(),
                };
                return true;
            }
            if (HEADER..HEADER + 4).contains(&id) {
                self.panel_menu(IDS[id - HEADER]);
                return true;
            }
            if let Some(c) = commands().find(|c| c.id() == id) {
                if !self.command_enabled(c) {
                    return true;
                }
                if self.hwnd == self.shell.main_window
                    && self.shell.compact_window != HWND::default()
                    && c != Command::Return
                {
                    self.return_from_compact();
                }
                if self.shell.compact_window != HWND::default()
                    && matches!(
                        c,
                        Command::New
                            | Command::Open
                            | Command::Close
                            | Command::Workspace
                            | Command::Feature(_)
                            | Command::Search(_)
                            | Command::Git(_)
                    )
                {
                    self.return_from_compact();
                }
                match c {
                    Command::Feature(id) => {
                        self.features_command(id);
                        if let Some(paths) = self.take_feature_path_copy_payload() {
                            let text = paths
                                .iter()
                                .map(|path| path.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join("\r\n");
                            if !write_clipboard(self.shell.main_window, &text) {
                                self.status = "パスをコピーできませんでした".into();
                            }
                        }
                    }
                    Command::Editor(id) => {
                        self.editor_action(id);
                    }
                    Command::Diagnostics => {
                        self.diagnostics_command();
                    }
                    Command::Search(id) => {
                        self.search_command(id);
                    }
                    Command::Git(id) => {
                        self.git_command(id);
                    }
                    Command::Link => {
                        if let Some(destination) =
                            self.link_destination_at(self.doc().selection().head)
                        {
                            self.features_open_link(&destination);
                        }
                    }
                    Command::Palette => self.open_palette(),
                    Command::Settings => self.show_settings(),
                    Command::Compact => self.open_compact(),
                    Command::Return => self.return_from_compact(),
                    Command::PanelMove(slot) => {
                        let id = self.shell.active_panel();
                        self.shell.layout.move_to(
                            id,
                            [
                                Slot::LeftTop,
                                Slot::LeftBottom,
                                Slot::RightTop,
                                Slot::RightBottom,
                            ][slot as usize],
                        );
                        self.presentation_changed();
                        self.save_panel_layout();
                    }
                    Command::PanelResize(grow) => {
                        let id = self.shell.active_panel();
                        let p = self.shell.layout.get_mut(id);
                        p.height = (p.height + if grow { 24. } else { -24. }).clamp(30., 4096.);
                        self.presentation_changed();
                        self.save_panel_layout();
                    }
                    Command::Reset => {
                        self.shell.layout = PanelLayout::default();
                        self.shell.drawer = None;
                        self.presentation_changed();
                        self.save_panel_layout();
                    }
                    _ => return false,
                }
                return true;
            }
            false
        }
    }
    fn presentation_changed(&mut self) {
        self.capture_view_anchor();
        self.shell.controls_dirty = true;
        self.layout_key = None;
        unsafe {
            self.refresh();
        }
    }
    pub(super) fn show_panel(&mut self, id: PanelId) {
        self.focus_feature_panel(id);
        self.shell.layout.get_mut(id).hidden = false;
        self.shell.layout.get_mut(id).collapsed = false;
        self.shell.drawer = if self.width < 760. { Some(id) } else { None };
        self.presentation_changed();
        self.save_panel_layout();
    }
    unsafe fn panel_menu(&mut self, id: PanelId) {
        unsafe {
            self.shell.active_panel = id;
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            // Keep low-frequency operations beside the content they act on.
            // All commands still use the existing registry and safety guards.
            let operations: Vec<_> = if id == PanelId::Explorer && self.search_pane_active() {
                super::search_ui::COMMANDS
                    .iter()
                    .map(|entry| Command::Search(entry.0))
                    .collect()
            } else if id == PanelId::Git {
                super::git_ui::COMMANDS
                    .iter()
                    .map(|entry| Command::Git(entry.0))
                    .collect()
            } else {
                super::features::COMMANDS
                    .iter()
                    .filter(|entry| match id {
                        PanelId::Explorer => (4000..=4011).contains(&entry.0),
                        PanelId::Calendar => {
                            (4020..=4028).contains(&entry.0) || (4080..=4088).contains(&entry.0)
                        }
                        PanelId::Outline => (4040..=4042).contains(&entry.0),
                        _ => false,
                    })
                    .map(|entry| Command::Feature(entry.0))
                    .collect()
            };
            for operation in operations {
                let enabled = self.command_enabled(operation);
                let label = self.command_label(operation);
                let _ = AppendMenuW(
                    menu,
                    MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED },
                    operation.id(),
                    PCWSTR(wide(&label).as_ptr()),
                );
            }
            if id == PanelId::Explorer && !self.search_pane_active() {
                let _ = AppendMenuW(menu, MF_STRING, CMD_WORKSPACE, w!("Workspaceを開く…"));
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            for (i, label) in [
                "折畳み / 展開",
                "非表示",
                "左上へ移動",
                "左下へ移動",
                "右上へ移動",
                "右下へ移動",
            ]
            .into_iter()
            .enumerate()
            {
                let _ = AppendMenuW(menu, MF_STRING, i + 1, PCWSTR(wide(label).as_ptr()));
            }
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY,
                p.x,
                p.y,
                None,
                self.hwnd,
                None,
            )
            .0;
            let _ = DestroyMenu(menu);
            if command >= 1000 || command as usize == CMD_WORKSPACE {
                self.command(command as usize);
                return;
            }
            match command {
                1 => {
                    let panel = self.shell.layout.get_mut(id);
                    panel.collapsed = !panel.collapsed;
                }
                2 => {
                    self.shell.layout.get_mut(id).hidden = true;
                    self.shell.drawer = None;
                }
                3..=6 => self.shell.layout.move_to(
                    id,
                    [
                        Slot::LeftTop,
                        Slot::LeftBottom,
                        Slot::RightTop,
                        Slot::RightBottom,
                    ][command as usize - 3],
                ),
                _ => return,
            }
            self.presentation_changed();
            self.save_panel_layout();
        }
    }
    pub(super) unsafe fn shell_click(&mut self, x: f32, y: f32) {
        unsafe {
            if self.composing {
                return;
            }
            if self.shell.compact_window != HWND::default() && self.hwnd == self.shell.main_window {
                return;
            }
            for (id, r) in self.shell.panels {
                if r.width == 0. {
                    continue;
                }
                let left = matches!(
                    self.shell.layout.get(id).slot,
                    Slot::LeftTop | Slot::LeftBottom
                );
                if (self.width >= 760.)
                    && (x - (if left { r.x + r.width } else { r.x })).abs() < 5.
                    && y >= r.y
                    && y <= r.y + r.height
                {
                    self.shell.splitter = Some(Splitter::Width(id));
                    SetCapture(self.hwnd);
                    return;
                }
                if (y - (r.y + r.height)).abs() < 5. && r.contains(x, y - 5.) {
                    self.shell.splitter = Some(Splitter::Height(id));
                    SetCapture(self.hwnd);
                    return;
                }
                if r.contains(x, y) {
                    self.shell.active_panel = id;
                    if let Some(header) = self.shell.headers.get(id as usize) {
                        let _ = SetFocus(Some(*header));
                    }
                    return;
                }
            }
            if y < 36. {
                self.open_palette();
                return;
            }
            if y < 76. {
                if x >= self.editor_right() {
                    self.command(CMD_NEW);
                    return;
                }
                let strip = self.tab_strip();
                if strip.overflow && x < strip.left {
                    self.scroll_tabs(false);
                } else if strip.overflow && x >= strip.right {
                    self.scroll_tabs(true);
                } else if strip.tab_width > 0. && x >= strip.left && x < strip.right {
                    let index = strip.first + ((x - strip.left) / strip.tab_width) as usize;
                    if index < self.tabs.len() {
                        self.switch_tab(index);
                        if x > strip.left + (index - strip.first + 1) as f32 * strip.tab_width - 22.
                        {
                            let _ = PostMessageW(
                                Some(self.hwnd),
                                WM_COMMAND,
                                WPARAM(CMD_CLOSE),
                                LPARAM(0),
                            );
                        }
                    }
                }
                return;
            }
            if !self.shell.editor.contains(x, y) {
                return;
            }
            if x > self.editor_right() - 12. {
                self.dragging_scroll = true;
                self.scroll_at(y);
                SetCapture(self.hwnd);
                return;
            }
            if self.toggle_task_at(x, y) {
                self.after_input();
            } else {
                self.select_at(x, y, GetKeyState(VK_SHIFT.0 as i32) < 0);
                self.dragging = true;
                SetCapture(self.hwnd);
            }
        }
    }
    pub(super) fn shell_mouse_move(&mut self, lp: LPARAM) -> bool {
        let x = (lp.0 as u16 as i16) as f32 / self.dpi;
        let y = ((lp.0 >> 16) as u16 as i16) as f32 / self.dpi;
        let hover = self.shell.editor.contains(x, y) && x > self.editor_right() - 12.;
        if self.shell.scroll_hover != hover {
            self.shell.scroll_hover = hover;
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
        let Some(splitter) = self.shell.splitter else {
            return false;
        };
        let x = (lp.0 as u16 as i16) as f32 / self.dpi;
        let y = ((lp.0 >> 16) as u16 as i16) as f32 / self.dpi;
        match splitter {
            Splitter::Width(id) => {
                let slot = self.shell.layout.get(id).slot;
                let left = matches!(slot, Slot::LeftTop | Slot::LeftBottom);
                let width = if left { x - 48. } else { self.width - x };
                for p in &mut self.shell.layout.panels {
                    if matches!(p.slot, Slot::LeftTop | Slot::LeftBottom) == left {
                        p.width = width.clamp(100., 600.);
                    }
                }
            }
            Splitter::Height(id) => {
                let left = matches!(
                    self.shell.layout.get(id).slot,
                    Slot::LeftTop | Slot::LeftBottom
                );
                let total = (self.shell.editor.height + 52.).max(80.);
                for p in &mut self.shell.layout.panels {
                    if matches!(p.slot, Slot::LeftTop | Slot::LeftBottom) == left {
                        p.height = if matches!(p.slot, Slot::LeftTop | Slot::RightTop) {
                            (y - 36.).clamp(30., total - 30.)
                        } else {
                            (total - y + 36.).clamp(30., total - 30.)
                        };
                    }
                }
            }
        }
        self.presentation_changed();
        true
    }
    pub(super) fn finish_splitter(&mut self) {
        if self.shell.splitter.take().is_some() {
            self.save_panel_layout();
        }
    }
    pub(super) unsafe fn open_palette(&mut self) {
        unsafe {
            if let Some(p) = &self.shell.palette {
                let _ = SetFocus(Some(p.query));
                return;
            }
            if self.composing {
                return;
            }
            let focus = GetFocus();
            self.release_caret();
            let host = control(
                self.hwnd,
                w!("STATIC"),
                "コマンドパレット",
                0,
                WS_BORDER | WS_CLIPCHILDREN | WINDOW_STYLE(0x0100), // SS_NOTIFY: host receives its background clicks.
            );
            let _ = SetWindowLongPtrW(host, GWL_EXSTYLE, WS_EX_CONTROLPARENT.0 as isize);
            let _ = SetWindowSubclass(host, Some(palette_host_proc), 1, self.hwnd.0 as usize);
            let label = control(
                host,
                w!("STATIC"),
                "コマンド検索 · Escで閉じる",
                0,
                WINDOW_STYLE(0),
            );
            let query = control(
                host,
                w!("EDIT"),
                "",
                QUERY,
                WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            );
            let list = control(
                host,
                w!("LISTBOX"),
                "コマンド候補",
                RESULTS,
                WS_BORDER | WS_VSCROLL | WINDOW_STYLE(LBS_NOTIFY as u32),
            );
            let mut font_data = LOGFONTW {
                lfHeight: -(16. * self.dpi) as i32,
                ..Default::default()
            };
            let face = wide("Segoe UI");
            font_data.lfFaceName[..face.len()].copy_from_slice(&face);
            let font = CreateFontIndirectW(&font_data);
            for child in [label, query, list] {
                send(child, WM_SETFONT, WPARAM(font.0 as usize), LPARAM(1));
            }
            send(
                list,
                LB_SETITEMHEIGHT,
                WPARAM(0),
                LPARAM((24. * self.dpi) as isize),
            );
            let composing = track_control_ime(query);
            self.shell.palette = Some(PaletteState {
                host,
                font,
                label,
                query,
                list,
                commands: Vec::new(),
                focus,
                document: self.doc().id(),
                revision: self.doc().revision(),
                selection: self.doc().selection(),
                affinity: self.doc().selection_affinity(),
                composing,
            });
            self.position_palette();
            self.filter_palette();
            let _ = SetFocus(Some(query));
        }
    }
    unsafe fn position_palette(&self) {
        unsafe {
            let Some(p) = &self.shell.palette else {
                return;
            };
            let mut client = RECT::default();
            if GetClientRect(
                GetParent(p.host).unwrap_or(self.shell.main_window),
                &mut client,
            )
            .is_err()
            {
                return;
            }
            let dpi = self.dpi.max(0.1);
            let margin = (12. * dpi) as i32;
            let width = (client.right - 2 * margin).max(1).min((600. * dpi) as i32);
            let top = ((40. * dpi) as i32).min((client.bottom - 1).max(0));
            let height = (client.bottom - top - margin)
                .max(1)
                .min((380. * dpi) as i32);
            let _ = SetWindowPos(
                p.host,
                Some(HWND_TOP),
                (client.right - width) / 2,
                top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let pad = (8. * dpi) as i32;
            let inner = (width - 2 * pad - 2).max(0);
            let label_h = ((24. * dpi) as i32).min((height - 2 * pad).max(0));
            let query_h = ((30. * dpi) as i32).min((height - 2 * pad - label_h).max(0));
            let _ = position_child_control(p.label, pad, pad, inner, label_h);
            let _ = position_child_control(p.query, pad, pad + label_h, inner, query_h);
            let list_y = pad + label_h + query_h + (6. * dpi) as i32;
            let _ = position_child_control(
                p.list,
                pad,
                list_y,
                inner,
                (height - list_y - pad - 2).max(0),
            );
        }
    }
    pub(super) unsafe fn focus_palette_after_main_activation(&mut self, owner: HWND) -> bool {
        unsafe {
            let Some(p) = &self.shell.palette else {
                return false;
            };
            if owner != self.shell.main_window
                || GetFocus() != owner
                || GetParent(p.host).ok() != Some(owner)
                || GetParent(p.query).ok() != Some(p.host)
                || !IsWindowVisible(p.host).as_bool()
                || !IsWindowVisible(p.query).as_bool()
                || !IsWindowEnabled(p.query).as_bool()
            {
                return false;
            }
            let query = p.query;
            self.release_caret();
            let _ = SetFocus(Some(query));
            GetFocus() == query
        }
    }
    unsafe fn filter_palette(&mut self) {
        unsafe {
            let Some(p) = self.shell.palette.as_ref() else {
                return;
            };
            let query = text_of(p.query).to_lowercase();
            let list = p.list;
            let commands: Vec<_> = commands()
                .filter(|c| {
                    *c != Command::Palette && self.command_label(*c).to_lowercase().contains(&query)
                })
                .collect();
            send(list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            for c in &commands {
                let label = self.command_label(*c);
                send(
                    list,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(wide(&label).as_ptr() as isize),
                );
            }
            send(list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
            self.shell.palette.as_mut().unwrap().commands = commands;
        }
    }
    unsafe fn close_palette(&mut self, cancel: bool) {
        unsafe {
            let Some(p) = self.shell.palette.take() else {
                return;
            };
            let _ = SetFocus(Some(self.editing_window()));
            let _ = ShowWindow(p.host, SW_HIDE);
            if cancel && p.document == self.doc().id() && p.revision == self.doc().revision() {
                let _ = self.doc_mut().set_selection_with_affinity(
                    p.selection.anchor,
                    p.selection.head,
                    p.affinity,
                );
            }
            let focus = if IsWindow(Some(p.focus)).as_bool()
                && IsWindowVisible(p.focus).as_bool()
                && IsWindowEnabled(p.focus).as_bool()
                && (p.focus == self.shell.main_window
                    || IsChild(self.shell.main_window, p.focus).as_bool())
            {
                p.focus
            } else {
                self.editing_window()
            };
            let _ = SetFocus(Some(focus));
            drop(p);
            self.refresh();
        }
    }
    unsafe fn execute_palette(&mut self) {
        unsafe {
            let command = self.shell.palette.as_ref().and_then(|p| {
                let n = send(p.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                usize::try_from(n)
                    .ok()
                    .and_then(|n| p.commands.get(n).copied())
            });
            if let Some(command) = command
                && !self.command_enabled(command)
            {
                self.status = self.command_disabled_reason(command).into();
                self.filter_palette();
                return;
            }
            self.close_palette(false);
            if let Some(command) = command {
                self.command(command.id());
                self.refresh();
            }
        }
    }
    pub(super) fn shell_dialog_owner(&self, hwnd: HWND) -> Option<HWND> {
        if !self.shell.enabled || hwnd == self.editing_window() {
            return None;
        }
        if self
            .shell
            .palette
            .as_ref()
            .is_some_and(|p| p.composing.get())
        {
            return None;
        }
        Some(self.hwnd)
    }
    pub(super) fn shell_message(&mut self, msg: &MSG) -> bool {
        unsafe {
            let root = GetAncestor(msg.hwnd, GA_ROOT);
            let owner = if root == HWND::default() {
                self.hwnd
            } else {
                root
            };
            if let Some(p) = &self.shell.palette {
                let inside = msg.hwnd == p.host || IsChild(p.host, msg.hwnd).as_bool();
                let pointer_down = matches!(
                    msg.message,
                    WM_LBUTTONDOWN
                        | WM_RBUTTONDOWN
                        | WM_MBUTTONDOWN
                        | WM_LBUTTONDBLCLK
                        | WM_RBUTTONDBLCLK
                        | WM_MBUTTONDBLCLK
                ) || (msg.hwnd != owner
                    && matches!(
                        msg.message,
                        WM_NCLBUTTONDOWN
                            | WM_NCRBUTTONDOWN
                            | WM_NCMBUTTONDOWN
                            | WM_NCLBUTTONDBLCLK
                            | WM_NCRBUTTONDBLCLK
                            | WM_NCMBUTTONDBLCLK
                    ));
                if pointer_down && !inside && GetParent(p.host).ok() == Some(owner) {
                    let target = if msg.hwnd == self.shell.main_window {
                        let x = msg.lParam.0 as u16 as i16 as f32 / self.dpi;
                        let y = (msg.lParam.0 >> 16) as u16 as i16 as f32 / self.dpi;
                        if self.shell.editor.contains(x, y) {
                            self.editing_window()
                        } else {
                            self.shell
                                .panels
                                .iter()
                                .find(|(_, rect)| rect.contains(x, y))
                                .and_then(|(id, _)| self.shell.headers.get(*id as usize))
                                .copied()
                                .unwrap_or(HWND::default())
                        }
                    } else {
                        msg.hwnd
                    };
                    self.close_palette(true);
                    if IsWindowVisible(target).as_bool()
                        && IsWindowEnabled(target).as_bool()
                        && (target == owner || IsChild(owner, target).as_bool())
                    {
                        let _ = SetFocus(Some(target));
                    }
                    return true;
                }
                if matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                    && !p.composing.get()
                    && msg.wParam.0 as u16 == VK_ESCAPE.0
                    && GetParent(p.host).ok() == Some(owner)
                {
                    self.close_palette(true);
                    return true;
                }
            }
            if let Some(p) = &self.shell.palette
                && msg.hwnd == p.query
            {
                if msg.message == WM_IME_STARTCOMPOSITION {
                    p.composing.set(true);
                }
                if msg.message == WM_IME_ENDCOMPOSITION {
                    p.composing.set(false);
                }
                if p.composing.get() {
                    return false;
                }
            }
            if !self.shell.enabled || !matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                return false;
            }
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;
            if !self.feature_ime_active()
                && !alt
                && self.editor_actions.active_button(self) == msg.hwnd
                && let Some(redo) = history_shortcut(msg.wParam.0 as u16, ctrl, shift)
            {
                let command = if redo {
                    EDITOR_HISTORY_REDO
                } else {
                    EDITOR_HISTORY_UNDO
                };
                let _ = PostMessageW(Some(owner), WM_COMMAND, WPARAM(command), LPARAM(0));
                return true;
            }
            if !self.feature_ime_active() {
                for c in commands() {
                    let global = matches!(
                        c,
                        Command::Palette | Command::Search(_) | Command::Feature(4000)
                    );
                    if msg.hwnd != self.editing_window() && !global {
                        continue;
                    }
                    if let Some(binding) = c
                        .binding()
                        .and_then(|k| self.shell.settings.keybindings.get(k))
                        && shortcut_matches(binding, msg.wParam.0 as u16, ctrl, shift, alt)
                    {
                        let _ = PostMessageW(Some(owner), WM_COMMAND, WPARAM(c.id()), LPARAM(0));
                        return true;
                    }
                }
            }
            if !self.feature_ime_active()
                && ctrl
                && msg.wParam.0 as u16 == 0x54
                && ((shift && !alt && msg.hwnd == self.editing_window()) || (alt && !shift))
            {
                let id = if alt { 39 } else { 38 };
                let _ = PostMessageW(Some(owner), WM_COMMAND, WPARAM(id), LPARAM(0));
                return true;
            }
            if !self.feature_ime_active() && ctrl && alt {
                let id = match msg.wParam.0 as u16 {
                    0x31..=0x34 => Some(160 + msg.wParam.0 - 0x31),
                    v if v == VK_UP.0 => Some(164),
                    v if v == VK_DOWN.0 => Some(165),
                    _ => None,
                };
                if let Some(id) = id {
                    let _ = PostMessageW(Some(owner), WM_COMMAND, WPARAM(id), LPARAM(0));
                    return true;
                }
            }

            if let Some(p) = &self.shell.palette
                && (msg.hwnd == p.query || msg.hwnd == p.list)
            {
                match msg.wParam.0 as u16 {
                    v if v == VK_RETURN.0 => {
                        let _ = PostMessageW(
                            Some(owner),
                            WM_COMMAND,
                            WPARAM(PALETTE_EXECUTE),
                            LPARAM(0),
                        );
                        return true;
                    }
                    v if v == VK_TAB.0 => {
                        let _ = SetFocus(Some(if msg.hwnd == p.query { p.list } else { p.query }));
                        return true;
                    }
                    v if v == VK_DOWN.0 || v == VK_UP.0 => {
                        let n = send(p.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
                        let next = (n + if v == VK_DOWN.0 { 1 } else { -1 })
                            .clamp(0, p.commands.len().saturating_sub(1) as isize);
                        send(p.list, LB_SETCURSEL, WPARAM(next as usize), LPARAM(0));
                        return true;
                    }
                    _ => {}
                }
            }
            if msg.hwnd != self.editing_window() {
                return false;
            }
            false
        }
    }
    pub(super) unsafe fn shell_shortcut(&mut self, key: u16, ctrl: bool, shift: bool) -> bool {
        unsafe {
            for c in commands() {
                let Some(binding) = c
                    .binding()
                    .and_then(|k| self.shell.settings.keybindings.get(k))
                else {
                    continue;
                };
                if shortcut_matches(binding, key, ctrl, shift, GetKeyState(VK_MENU.0 as i32) < 0) {
                    self.command(c.id());
                    return true;
                }
            }
            if ctrl && shift && key == 0x53 {
                self.command(CMD_SAVE_AS);
                return true;
            }
            ctrl && matches!(key, 0x4e | 0x4f | 0x53 | 0x57 | 0x4b)
        }
    }
    pub(super) unsafe fn open_compact(&mut self) {
        unsafe {
            if self.composing || self.shell.compact_window != HWND::default() {
                return;
            }
            self.capture_view_anchor();
            self.release_caret();
            let main = self.shell.main_window;
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const WindowState;
            if pointer.is_null() {
                return;
            }
            let title = wide(&format!("{} — Compact", self.tab_display_name(self.active)));
            let placement = self.tabs[self.active].persistence.placement.clone();
            let compact = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("MDLiteRustEditor"),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW,
                placement.x,
                placement.y,
                placement.width.min(800) as i32,
                placement.height.min(700) as i32,
                Some(main),
                None,
                Some(GetModuleHandleW(None).unwrap_or_default().into()),
                Some(pointer.cast()),
            );
            let Ok(compact) = compact else {
                self.status = "Compact windowを作成できません。主windowに保持しています".into();
                return;
            };
            self.shell.compact_window = compact;
            (*pointer).editor_window.set(compact);
            self.tabs[self.active].persistence.compact = true;
            self.select_surface(compact);
            self.dpi = GetDpiForWindow(compact) as f32 / 96.;
            self.monitor_safe_position(compact);
            let _ = ShowWindow(compact, SW_SHOW);
            let _ = SetFocus(Some(compact));
            self.layout_key = None;
            self.refresh();
            let _ = InvalidateRect(Some(main), None, true);
        }
    }
    pub(super) unsafe fn return_from_compact(&mut self) {
        unsafe {
            let compact = self.shell.compact_window;
            if compact == HWND::default() || self.composing {
                return;
            }
            let mut bounds = RECT::default();
            if GetWindowRect(compact, &mut bounds).is_ok() {
                self.tabs[self.active].persistence.placement = mdlite::session::WindowPlacement {
                    x: bounds.left,
                    y: bounds.top,
                    width: (bounds.right - bounds.left) as u32,
                    height: (bounds.bottom - bounds.top) as u32,
                };
            }
            self.capture_view_anchor();
            self.release_caret();
            let main = self.shell.main_window;
            self.tabs[self.active].persistence.compact = false;
            self.select_surface(main);
            self.shell.compact_window = HWND::default();
            self.shell.compact_target = None;
            let pointer = GetWindowLongPtrW(main, GWLP_USERDATA) as *const WindowState;
            if !pointer.is_null() {
                (*pointer).editor_window.set(main);
            }
            let _ = DestroyWindow(compact);
            self.dpi = GetDpiForWindow(main) as f32 / 96.;
            self.shell.controls_dirty = true;
            self.layout_key = None;
            let _ = SetFocus(Some(main));
            self.refresh();
        }
    }
    pub(super) unsafe fn monitor_safe_position(&self, hwnd: HWND) {
        unsafe {
            let mut bounds = RECT::default();
            if GetWindowRect(hwnd, &mut bounds).is_err() {
                return;
            }
            let monitor = MonitorFromRect(&bounds, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let work = info.rcWork;
            let width = (bounds.right - bounds.left)
                .min(work.right - work.left)
                .max(160);
            let height = (bounds.bottom - bounds.top)
                .min(work.bottom - work.top)
                .max(100);
            let x = bounds
                .left
                .clamp(work.left, (work.right - width).max(work.left));
            let y = bounds
                .top
                .clamp(work.top, (work.bottom - height).max(work.top));
            let _ = SetWindowPos(
                hwnd,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}
fn shortcut_matches(binding: &str, key: u16, ctrl: bool, shift: bool, alt: bool) -> bool {
    let upper = binding.to_uppercase();
    let parts: Vec<_> = upper.split('+').map(str::trim).collect();
    let Some(last) = parts.last() else {
        return false;
    };
    let code = if last.len() == 1 {
        last.as_bytes()[0] as u16
    } else if let Some(f) = last.strip_prefix('F').and_then(|s| s.parse::<u16>().ok()) {
        0x6f + f
    } else {
        0
    };
    code != 0
        && code == key
        && parts.contains(&"CTRL") == ctrl
        && parts.contains(&"SHIFT") == shift
        && parts.contains(&"ALT") == alt
}
fn system_light() -> bool {
    unsafe {
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegGetValueW(
                key: *mut std::ffi::c_void,
                sub: *const u16,
                name: *const u16,
                flags: u32,
                kind: *mut u32,
                data: *mut std::ffi::c_void,
                size: *mut u32,
            ) -> i32;
        }
        let mut value = 1u32;
        let mut size = 4;
        let status = RegGetValueW(
            0x80000001u32 as i32 as isize as *mut _,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize").as_ptr(),
            wide("AppsUseLightTheme").as_ptr(),
            0x10,
            std::ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        );
        status != 0 || value != 0
    }
}
unsafe fn draw_icon(dc: HDC, area: &RECT, svg: &str, dpi: f32, fg: D2D1_COLOR_F, bg: D2D1_COLOR_F) {
    unsafe {
        let svg = svg.replace(
            "currentColor",
            &format!(
                "#{:02x}{:02x}{:02x}",
                (fg.r * 255.) as u8,
                (fg.g * 255.) as u8,
                (fg.b * 255.) as u8
            ),
        );
        let Ok(tree) = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()) else {
            return;
        };
        let size = (20. * dpi).round().max(1.) as u32;
        let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(size, size) else {
            return;
        };
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(size as f32 / 24., size as f32 / 24.),
            &mut pixmap.as_mut(),
        );
        let mut bytes = Vec::with_capacity((size * size * 4) as usize);
        for p in pixmap.data().as_chunks::<4>().0 {
            let a = p[3] as f32 / 255.;
            bytes.extend_from_slice(&[
                (p[2] as f32 + bg.b * 255. * (1. - a)) as u8,
                (p[1] as f32 + bg.g * 255. * (1. - a)) as u8,
                (p[0] as f32 + bg.r * 255. * (1. - a)) as u8,
                255,
            ]);
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        SetDIBitsToDevice(
            dc,
            (area.right - area.left - size as i32) / 2,
            (area.bottom - area.top - size as i32) / 2,
            size,
            size,
            0,
            0,
            0,
            size,
            bytes.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
        );
    }
}

pub(super) unsafe fn send(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe { SendMessageW(hwnd, msg, Some(wp), Some(lp)) }
}

pub(super) unsafe extern "system" fn control_ime_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let composing = &*(data as *const Cell<bool>);
        if msg == WM_IME_STARTCOMPOSITION {
            composing.set(true);
        }
        if msg == WM_IME_ENDCOMPOSITION {
            composing.set(false);
        }
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(hwnd, Some(control_ime_proc), id);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
    }
}
pub(super) unsafe fn track_control_ime(hwnd: HWND) -> Box<Cell<bool>> {
    unsafe {
        let composing = Box::new(Cell::new(false));
        let _ = SetWindowSubclass(
            hwnd,
            Some(control_ime_proc),
            71,
            (&*composing as *const Cell<bool>) as usize,
        );
        composing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_geometry_narrow_drawer_and_slot_swap_remain_reachable() {
        let mut layout = PanelLayout::default();
        for width in [320., 480., 760., 1100., 1440.] {
            let (editor, panels) = shell_geometry(&layout, width, 800., None);
            assert!(editor.width > 0. && editor.x >= 48. && editor.x + editor.width <= width);
            assert!(editor.y >= 76. && editor.y + editor.height <= 800.);
            if width < 760. {
                assert!(panels.iter().all(|(_, r)| r.width == 0.));
                for id in IDS {
                    let (_, drawer) = shell_geometry(&layout, width, 800., Some(id));
                    let r = drawer[id as usize].1;
                    assert!(r.width >= 200. && r.contains(r.x + 20., r.y + 10.));
                    assert!(r.x + r.width <= width);
                }
            }
        }
        layout.move_to(PanelId::Calendar, Slot::RightTop);
        let (_, panels) = shell_geometry(&layout, 1440., 900., None);
        assert!(panels[PanelId::Calendar as usize].1.x > 700.);
        assert!(panels[PanelId::Outline as usize].1.x < 400.);
    }

    #[test]
    fn compact_editor_body_starts_below_tab_strip_and_keeps_bottom_margin() {
        let compact = super::super::tests::hidden_test_window_size(640, 480);
        let mut app = super::super::tests::composing_test_app("body");
        app.hwnd = compact;
        app.width = 640.;
        app.shell.enabled = true;
        app.shell.compact_window = compact;
        app.update_shell_geometry(480.);

        assert_eq!(app.shell.editor.y, TOP);
        assert_eq!(app.shell.editor.y + app.shell.editor.height, 480. - 24.);
        assert!(app.shell.editor.y >= 75., "compact tabs end at y=75 DIP");
        unsafe {
            let _ = DestroyWindow(compact);
        }
    }

    #[test]
    fn custom_heading_and_code_background_colors_reach_shell_tokens() {
        let mut tokens = Tokens::default();
        apply_custom_color_token(&mut tokens, "heading", "#123456");
        apply_custom_color_token(&mut tokens, "code_background", "#abcdef");

        assert_eq!(tokens.heading.r, 0x12 as f32 / 255.);
        assert_eq!(tokens.heading.g, 0x34 as f32 / 255.);
        assert_eq!(tokens.heading.b, 0x56 as f32 / 255.);
        assert_eq!(tokens.code_background.r, 0xab as f32 / 255.);
        assert_eq!(tokens.code_background.g, 0xcd as f32 / 255.);
        assert_eq!(tokens.code_background.b, 0xef as f32 / 255.);
    }

    #[test]
    fn shell_final_rect_font_and_panel_changes_preserve_source_and_hit_coordinates() {
        let mut app = super::super::tests::composing_test_app(
            "日本語😀 text that wraps across the editor.\n\n| A | B |\n|---|---|\n| x | y |\n",
        );
        let window = super::super::tests::hidden_test_window_size(1280, 800);
        app.hwnd = window;
        app.composing = false;
        app.preedit.clear();
        app.ime_owner = None;
        app.shell.enabled = true;
        app.tabs[app.active].scroll = 0.;
        app.shell.main_window = window;
        app.shell.settings.font_size_pt = 14;
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().is_dirty(),
            app.doc().undo_count(),
            app.doc().selection(),
        );
        unsafe {
            app.ensure_layout().unwrap();
        }
        let initial = app.shell.editor;
        let (first_x, first_y, height) = app.caret_location();
        assert!(first_x >= initial.x && first_y >= initial.y && height > 0.);
        let hit = app.hit_with_affinity(first_x, first_y + height / 2.).0;
        assert!(app.doc().source().is_char_boundary(hit));
        app.shell.layout.move_to(PanelId::Calendar, Slot::RightTop);
        app.shell.layout.get_mut(PanelId::Explorer).width = 310.;
        app.shell.settings.font_size_pt = 18;
        app.layout_key = None;
        unsafe {
            app.ensure_layout().unwrap();
        }
        assert_ne!(app.shell.editor, initial);
        assert!(app.feature_panel_rect(PanelId::Calendar).is_some());
        assert!(!app.search_pane_active());
        app.focus_feature_panel(PanelId::Calendar);
        assert!(
            app.views
                .iter()
                .filter(|v| v.table.is_none())
                .all(|v| v.x >= app.editor_left())
        );
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().is_dirty(),
                app.doc().undo_count(),
                app.doc().selection()
            ),
            before
        );
        unsafe {
            let _ = DestroyWindow(window);
        }
    }
    #[test]
    fn shell_focus_guard_and_compact_surface_share_canonical_history() {
        let mut app = super::super::tests::composing_test_app("one😀two");
        app.composing = false;
        app.preedit.clear();
        app.shell.enabled = true;
        app.shell.main_window = HWND(101usize as *mut _);
        app.hwnd = app.shell.main_window;
        let initial = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().selection(),
        );
        unsafe {
            app.character('x' as u16);
            app.key_with_modifiers(VK_BACK.0, false, false);
        }
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().selection()
            ),
            initial
        );
        let id = app.doc().id();
        app.shell.compact_window = HWND(102usize as *mut _);
        app.select_surface(app.shell.compact_window);
        assert_eq!(app.editing_window(), app.shell.compact_window);
        app.doc_mut().replace_selection("compact").unwrap();
        let edited = app.doc().source().to_owned();
        app.select_surface(app.shell.main_window);
        assert_eq!(app.doc().id(), id);
        assert_eq!(app.doc().source(), edited);
        assert_eq!(app.editing_window(), app.shell.compact_window);
        assert!(app.doc_mut().undo());
        assert_eq!(app.doc().source(), initial.0);
        assert!(!app.command_enabled(Command::Compact));
        assert!(app.command_enabled(Command::Return));
    }
    #[test]
    fn shell_binding_dispatch_respects_remapping_and_reserved_commands() {
        let mut app = super::super::tests::composing_test_app("source");
        app.composing = false;
        app.shell.enabled = true;
        app.shell
            .settings
            .keybindings
            .insert("file.save".into(), "Ctrl+Alt+S".into());
        assert!(!shortcut_matches("Ctrl+Alt+S", 0x53, true, false, false));
        assert!(shortcut_matches("Ctrl+Alt+S", 0x53, true, false, true));
        assert!(shortcut_matches("Ctrl+Shift+S", 0x53, true, true, false));
        assert!(unsafe { app.shell_shortcut(0x53, true, false) });
        assert_eq!(app.command_label(Command::Save), "保存\tCtrl+Alt+S");
        app.composing = true;
        assert!(!app.command_enabled(Command::Source));
        assert!(!app.command_enabled(Command::Palette));
    }
    #[test]
    fn shell_palette_host_bounds_dismissal_and_invalid_focus_are_owned() {
        let window = super::super::tests::hidden_test_window_size(1100, 793);
        let mut app = super::super::tests::composing_test_app("selected 😀 text");
        app.clear_composition();
        app.hwnd = window;
        app.shell.enabled = true;
        app.shell.main_window = window;
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().selection(),
        );
        unsafe {
            app.ensure_layout().unwrap();
            for width in [1086, 700, 1500, 450] {
                let _ = SetWindowPos(
                    window,
                    None,
                    0,
                    0,
                    width,
                    793,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                app.open_palette();
                let p = app.shell.palette.as_ref().unwrap();
                assert_eq!(GetParent(p.query).unwrap(), p.host);
                assert_eq!(GetParent(p.list).unwrap(), p.host);
                assert_ne!(
                    GetWindowLongPtrW(p.host, GWL_STYLE) as u32 & WS_CLIPCHILDREN.0,
                    0
                );
                let host = p.host;
                let query = p.query;
                let list = p.list;
                app.position_palette();
                let (bounds, _) = child_control_state(host).unwrap();
                let mut client = RECT::default();
                GetClientRect(window, &mut client).unwrap();
                assert!(bounds[0] >= 0 && bounds[1] >= 0);
                assert!(bounds[0] + bounds[2] <= client.right);
                assert!(bounds[1] + bounds[3] <= client.bottom);
                // Any owned surface's Escape dismisses; no key reaches the editor.
                let escape = MSG {
                    hwnd: window,
                    message: WM_KEYDOWN,
                    wParam: WPARAM(VK_ESCAPE.0 as usize),
                    ..Default::default()
                };
                assert!(app.shell_message(&escape));
                assert!(app.shell.palette.is_none());
                for child in [host, query, list] {
                    assert!(!IsWindow(Some(child)).as_bool());
                }
                app.open_palette();
                let outside = MSG {
                    hwnd: window,
                    message: WM_LBUTTONDOWN,
                    ..Default::default()
                };
                assert!(app.shell_message(&outside));
                assert!(app.shell.palette.is_none());
            }
            let previous = control(window, w!("EDIT"), "previous", 8000, WINDOW_STYLE(0));
            let _ = SetFocus(Some(previous));
            app.open_palette();
            DestroyWindow(previous).unwrap();
            app.close_palette(true);
            let scroller = control(window, w!("LISTBOX"), "outside scroll", 8001, WS_VSCROLL);
            app.open_palette();
            let scrollbar_click = MSG {
                hwnd: scroller,
                message: WM_NCLBUTTONDOWN,
                wParam: WPARAM(HTVSCROLL as usize),
                ..Default::default()
            };
            assert!(app.shell_message(&scrollbar_click));
            assert!(app.shell.palette.is_none());
            DestroyWindow(window).unwrap();
        }
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().selection()
            ),
            before
        );
    }
    #[test]
    fn shell_palette_query_cancel_and_native_ime_keep_document_selection() {
        let mut app = super::super::tests::composing_test_app("A😀日Z");
        let window = super::super::tests::hidden_test_window_size(1100, 700);
        app.hwnd = window;
        app.shell.enabled = true;
        app.shell.main_window = window;
        app.composing = false;
        app.preedit.clear();
        app.ime_owner = None;
        app.tabs[app.active].scroll = 0.;
        app.doc_mut().set_selection(8, 1).unwrap();
        let source = app.doc().source().to_owned();
        let revision = app.doc().revision();
        unsafe {
            app.ensure_layout().unwrap();
            app.open_palette();
            let query = app.shell.palette.as_ref().unwrap().query;
            send(query, WM_IME_STARTCOMPOSITION, WPARAM(0), LPARAM(0));
            assert!(app.shell.palette.as_ref().unwrap().composing.get());
            let escape = MSG {
                hwnd: query,
                message: WM_KEYDOWN,
                wParam: WPARAM(VK_ESCAPE.0 as usize),
                ..Default::default()
            };
            assert!(!app.shell_message(&escape));
            assert_eq!(app.shell_dialog_owner(query), None);
            send(query, WM_IME_ENDCOMPOSITION, WPARAM(0), LPARAM(0));
            assert!(!app.shell.palette.as_ref().unwrap().composing.get());
            app.doc_mut().set_selection(0, 0).unwrap();
            app.close_palette(true);
            assert_eq!(app.doc().selection(), Selection { anchor: 8, head: 1 });
            assert!(!IsWindow(Some(query)).as_bool());
            app.open_palette();
            let query = app.shell.palette.as_ref().unwrap().query;
            let _ = SetWindowTextW(query, w!("ソース"));
            app.filter_palette();
            assert_eq!(
                app.shell.palette.as_ref().unwrap().commands,
                vec![Command::Source, Command::Editor(38)]
            );
            assert!(!app.command_enabled(Command::Editor(38)));
            // A disabled table action stays visible and cannot close the palette or mutate text.
            let list = app.shell.palette.as_ref().unwrap().list;
            send(list, LB_SETCURSEL, WPARAM(1), LPARAM(0));
            app.execute_palette();
            assert!(app.shell.palette.is_some());
            assert_eq!(app.doc().source(), source);
            send(list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
            app.execute_palette();
            assert!(app.tabs[app.active].raw);
            assert!(app.shell.palette.is_none());
            let _ = DestroyWindow(window);
        }
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().revision(), revision);
        assert_eq!(app.doc().selection(), Selection { anchor: 8, head: 1 });
    }
    #[test]
    fn shell_owner_draw_does_not_consume_explorer_rows() {
        let window = super::super::tests::hidden_test_window_size(640, 480);
        let mut app = super::super::tests::composing_test_app("text");
        app.composing = false;
        app.hwnd = window;
        app.shell.enabled = true;
        app.shell.main_window = window;
        unsafe {
            let dc = GetDC(Some(window));
            assert!(!dc.0.is_null());
            let mut item = DRAWITEMSTRUCT {
                CtlType: ODT_LISTBOX,
                CtlID: 4690,
                itemID: u32::MAX,
                hwndItem: window,
                hDC: dc,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 100,
                    bottom: 24,
                },
                ..Default::default()
            };
            let parameter = LPARAM((&item as *const DRAWITEMSTRUCT) as isize);
            assert!(
                app.shell_window_message(window, WM_DRAWITEM, WPARAM(4690), parameter)
                    .is_none()
            );
            assert_eq!(
                super::super::dispatch_window_message(
                    window,
                    WM_DRAWITEM,
                    WPARAM(4690),
                    parameter,
                    &mut app
                ),
                Some(LRESULT(1))
            );
            item.CtlType = ODT_BUTTON;
            item.CtlID = PALETTE as u32;
            let parameter = LPARAM((&item as *const DRAWITEMSTRUCT) as isize);
            assert_eq!(
                app.shell_window_message(window, WM_DRAWITEM, WPARAM(PALETTE), parameter),
                Some(LRESULT(1))
            );
            ReleaseDC(Some(window), dc);
            DestroyWindow(window).unwrap();
        }
    }
}
