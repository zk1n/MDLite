//! Independent panel groups in the Win32 shell. Presentation never owns source.
use super::*;
use mdlite::{
    panel::{PanelId, PanelLayout, Rect, Side, Slot},
    settings::EffectiveSettings,
};
use windows::Win32::UI::Controls::*;

pub(super) const PALETTE: usize = 100;
pub(super) const CONTAINER_STYLE: WINDOW_STYLE = WINDOW_STYLE(0x000d); // SS_OWNERDRAW
pub(super) const TOP_BAR_HEIGHT: f32 = 36.;
pub(super) const TAB_STRIP_TOP: f32 = TOP_BAR_HEIGHT;
pub(super) const TAB_STRIP_HEIGHT: f32 = 28.;
pub(super) const TAB_LABEL_TOP: f32 = 40.;
pub(super) const TAB_DIVIDER_Y: f32 = 67.;
pub(super) const EDITOR_TOP: f32 = 80.;
pub(super) const UI_FONT_FAMILY: &str = "Yu Gothic UI";
pub(super) const UI_FONT_SIZE: f32 = 12.;
pub(super) const UI_ROW_HEIGHT: f32 = 24.;
const NAV_BACK: usize = 168;
const NAV_FORWARD: usize = 169;
const SETTINGS: usize = 101;
const COMPACT: usize = 102;
const RETURN: usize = 103;
const RESET: usize = 104;
const PALETTE_EXECUTE: usize = 105;
const PALETTE_CANCEL: usize = 106;
const SHELL_MENU: usize = 107;
pub(super) const EDITOR_HISTORY_UNDO: usize = 0x7ef0;
const EDITOR_HISTORY_REDO: usize = 0x7ef1;
const ACTIVITY: usize = 110;
const ACTIVITY_COUNT: usize = 6;
const REGION_LEFT: usize = 108;
const REGION_RIGHT: usize = 109;
const HEADER: usize = 130;
const QUERY: usize = 150;
const RESULTS: usize = 151;
const PALETTE_HELP: usize = 152;
const IDS: [PanelId; 5] = [
    PanelId::Explorer,
    PanelId::Calendar,
    PanelId::Outline,
    PanelId::Git,
    PanelId::Search,
];
const NAMES: [&str; 5] = [
    "エクスプローラー",
    "カレンダー",
    "アウトライン",
    "Git（ソース管理）",
    "検索",
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
// Owned shell inputs remain available during synchronous child drawing while
// layout or a theme update holds App. No document or App references are retained.
pub(super) struct ShellButtonPaint {
    owner: HWND,
    tokens: Tokens,
    buttons: Vec<(usize, HWND, bool)>,
}
impl ShellButtonPaint {
    pub(super) unsafe fn draw(
        &self,
        recipient: HWND,
        message_id: usize,
        item: &DRAWITEMSTRUCT,
    ) -> bool {
        unsafe {
            if recipient != self.owner
                || recipient == HWND::default()
                || item.CtlType != ODT_BUTTON
                || item.CtlID as usize != message_id
                || GetParent(item.hwndItem).ok() != Some(recipient)
                || GetDlgCtrlID(item.hwndItem) as usize != message_id
            {
                return false;
            }
            let Some((_, _, active)) = self.buttons.iter().find(|(id, child, _)| {
                *id == message_id && *child == item.hwndItem && *child != HWND::default()
            }) else {
                return false;
            };
            draw_shell_button(self.tokens, *active, item);
            true
        }
    }
}
unsafe fn valid_shell_erase_button(child: HWND, owner: HWND, id: usize) -> bool {
    unsafe {
        let mut process = 0;
        let thread = GetWindowThreadProcessId(child, Some(&mut process));
        let mut owner_process = 0;
        let owner_thread = GetWindowThreadProcessId(owner, Some(&mut owner_process));
        let mut class = [0u16; 32];
        let length = GetClassNameW(child, &mut class);
        thread == windows_sys::Win32::System::Threading::GetCurrentThreadId()
            && owner_thread == thread
            && process == std::process::id()
            && owner_process == process
            && GetParent(child).ok() == Some(owner)
            && GetDlgCtrlID(child) as usize == id
            && GetWindowLongW(child, GWL_STYLE) as u32 & 0xf == BS_OWNERDRAW as u32
            && length > 0
            && String::from_utf16_lossy(&class[..length as usize]).eq_ignore_ascii_case("Button")
    }
}
unsafe extern "system" fn shell_erase_proc(
    child: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    owner: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_NCDESTROY {
            let _ = RemoveWindowSubclass(child, Some(shell_erase_proc), id);
        } else if msg == WM_ERASEBKGND && valid_shell_erase_button(child, HWND(owner as *mut _), id)
        {
            // Only registered full-rectangle shell owner painters use this
            // subclass. Keep old pixels until their real WM_DRAWITEM completes.
            return LRESULT(1);
        }
        DefSubclassProc(child, msg, wp, lp)
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
const ACTIVITY_ICONS: [&str; ACTIVITY_COUNT] = [
    include_str!("../../assets/ui/icons/explorer.svg"),
    include_str!("../../assets/ui/icons/search.svg"),
    include_str!("../../assets/ui/icons/calendar.svg"),
    include_str!("../../assets/ui/icons/outline.svg"),
    include_str!("../../assets/ui/icons/git-branch.svg"),
    include_str!("../../assets/ui/icons/settings.svg"),
];
const TOP_ACTION_ICONS: [&str; 4] = [
    include_str!("../../assets/ui/icons/file.svg"),
    include_str!("../../assets/ui/icons/save.svg"),
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="m14 6-6 6 6 6M8 12h12"/></svg>"#,
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="m10 6 6 6-6 6M4 12h12"/></svg>"#,
];
const HAMBURGER_ICON: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M4 6h16M4 12h16M4 18h16"/></svg>"#;
const REGION_ICONS: [&str; 2] = [
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="4" width="18" height="16" rx="1"/><path d="M9 4v16"/></svg>"#,
    r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><rect x="3" y="4" width="18" height="16" rx="1"/><path d="M15 4v16"/></svg>"#,
];
const MENU_ICON: &str = include_str!("../../assets/ui/icons/more.svg");

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Command {
    Navigate(bool),
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
    Region(bool),
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
            Self::Navigate(false) => NAV_BACK,
            Self::Navigate(true) => NAV_FORWARD,
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
            Self::Region(true) => REGION_LEFT,
            Self::Region(false) => REGION_RIGHT,
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
            Self::Navigate(false) => "戻る",
            Self::Navigate(true) => "進む",
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
            Self::Reset => "パネル配置と左右領域の表示をリセット",
            Self::Region(true) => "左領域を表示 / 非表示",
            Self::Region(false) => "右領域を表示 / 非表示",
            Self::PanelMove(slot) => match slot {
                0 => "左の独立グループへ",
                1 => "左のグループへ追加",
                2 => "右の独立グループへ",
                _ => "右のグループへ追加",
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
const COMMANDS: [Command; 28] = [
    Command::Navigate(false),
    Command::Navigate(true),
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
    Command::Region(true),
    Command::Region(false),
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

pub(super) fn configurable_commands() -> Vec<(&'static str, &'static str)> {
    commands()
        .filter_map(|command| command.binding().map(|key| (key, command.label())))
        .collect()
}

fn command_menu_group(command: Command) -> usize {
    match command {
        Command::Navigate(_) | Command::Link => 6,
        Command::Feature(_) => 2,
        Command::Editor(_) => 5,
        Command::Search(_) => 3,
        Command::Git(_) => 4,
        Command::Diagnostics
        | Command::Palette
        | Command::Settings
        | Command::Compact
        | Command::Return
        | Command::Reset
        | Command::Region(_)
        | Command::Source
        | Command::PanelMove(_)
        | Command::PanelResize(_) => 1,
        _ => 0,
    }
}

fn menu_mnemonic_group(virtual_key: u16) -> Option<usize> {
    // WM_SYSKEYDOWN uses virtual keys, whose letters are always A-Z. The
    // lowercase code points overlap function/numpad keys (0x73 is F4).
    match virtual_key {
        0x46 => Some(0),
        0x56 => Some(1),
        0x57 => Some(2),
        0x53 => Some(3),
        0x47 => Some(4),
        0x45 => Some(5),
        _ => None,
    }
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
    pub weekend_red: D2D1_COLOR_F,
    pub weekend_blue: D2D1_COLOR_F,
    pub high_contrast: bool,
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
    // Named STATIC hosts are containers, not visible labels. Keep their native
    // names while drawing only the panel background, including during reentry.
    pub(super) unsafe fn draw_container(
        &self,
        owner: HWND,
        id: usize,
        item: &DRAWITEMSTRUCT,
    ) -> bool {
        unsafe {
            if owner == HWND::default()
                || item.CtlType != ODT_STATIC
                || item.CtlID as usize != id
                || GetParent(item.hwndItem).ok() != Some(owner)
                || GetDlgCtrlID(item.hwndItem) as usize != id
                || GetWindowLongPtrW(item.hwndItem, GWL_STYLE) as u32 & 0x1f != CONTAINER_STYLE.0
            {
                return false;
            }
            FillRect(item.hDC, &item.rcItem, self.brush);
            true
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
            weekend_red: color(0.98, 0.60, 0.62),
            weekend_blue: color(0.55, 0.75, 1.0),
            high_contrast: false,
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
    font: Option<control_style::UiFont>,
    query: HWND,
    list: HWND,
    help: HWND,
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
            // The toolbar query outlives this candidate list and its IME state.
            let _ = RemoveWindowSubclass(self.query, Some(control_ime_proc), 71);
            if IsWindow(Some(self.host)).as_bool() {
                let _ = DestroyWindow(self.host);
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
struct NavigationPoint {
    document: u64,
    selection: Selection,
    scroll: f32,
}
pub(super) struct NativeShell {
    pub enabled: bool,
    pub main_window: HWND,
    pub compact_window: HWND,
    main_font: Option<control_style::UiFont>,
    compact_font: Option<control_style::UiFont>,
    pub settings: EffectiveSettings,
    pub tokens: Tokens,
    pub editor: Rect,
    pub panels: [(PanelId, Rect); 5],
    layout: PanelLayout,
    panel_file: Option<FileSnapshot>,
    panel_writable: bool,
    menu: HMENU,
    menu_groups: [HMENU; 7],
    menu_routes: HashMap<usize, HMENU>,
    menu_button: HWND,
    top_actions: Vec<HWND>,
    activity: Vec<HWND>,
    headers: Vec<HWND>,
    command_button: HWND,
    compact_query: HWND,
    region_buttons: [HWND; 2],
    region_focus: [HWND; 2],
    tooltip: HWND,
    drawer: Option<PanelId>,
    splitter: Option<Splitter>,
    palette: Option<PaletteState>,
    last_client: (u32, u32),
    controls_dirty: bool,
    alt_menu_pending: bool,
    active_panel: PanelId,
    pub(super) scroll_hover: bool,
    pub(super) status_eol_cache: Option<(u64, u64, super::presentation::StatusEolSummary)>,
    pub(super) external_change_docs: std::collections::HashSet<u64>,
    main_target: Option<ID2D1DCRenderTarget>,
    compact_target: Option<ID2D1DCRenderTarget>,
    colors: Rc<ControlColors>,
    tooltip_text: Vec<Vec<u16>>,
    nav_back: Vec<NavigationPoint>,
    nav_forward: Vec<NavigationPoint>,
    navigating: bool,
}
impl Default for NativeShell {
    fn default() -> Self {
        Self {
            enabled: false,
            main_window: HWND::default(),
            compact_window: HWND::default(),
            main_font: None,
            compact_font: None,
            settings: EffectiveSettings::default(),
            tokens: Tokens::default(),
            editor: Rect::default(),
            panels: std::array::from_fn(|i| (IDS[i], Rect::default())),
            layout: PanelLayout::default(),
            panel_file: None,
            panel_writable: false,
            menu: HMENU::default(),
            menu_groups: [HMENU::default(); 7],
            menu_routes: HashMap::new(),
            menu_button: HWND::default(),
            top_actions: Vec::new(),
            activity: Vec::new(),
            headers: Vec::new(),
            command_button: HWND::default(),
            compact_query: HWND::default(),
            region_buttons: [HWND::default(); 2],
            region_focus: [HWND::default(); 2],
            tooltip: HWND::default(),
            drawer: None,
            splitter: None,
            palette: None,
            last_client: (0, 0),
            controls_dirty: true,
            alt_menu_pending: false,
            active_panel: PanelId::Explorer,
            scroll_hover: false,
            status_eol_cache: None,
            external_change_docs: std::collections::HashSet::new(),
            main_target: None,
            compact_target: None,
            colors: Rc::new(ControlColors::new(Tokens::default())),
            tooltip_text: Vec::new(),
            nav_back: Vec::new(),
            nav_forward: Vec::new(),
            navigating: false,
        }
    }
}
impl Drop for NativeShell {
    fn drop(&mut self) {
        unsafe {
            if self.menu != HMENU::default() {
                if self.main_window != HWND::default()
                    && IsWindow(Some(self.main_window)).as_bool()
                    && GetMenu(self.main_window) == self.menu
                {
                    let _ = SetMenu(self.main_window, None);
                }
                let _ = DestroyMenu(self.menu);
                self.menu = HMENU::default();
            }
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
        self.layout.displayed(PanelId::Search)
    }
    pub(super) fn active_panel(&self) -> PanelId {
        let focus = unsafe { GetFocus() };
        if let Some(index) = self.headers.iter().position(|h| *h == focus) {
            return IDS[index];
        }
        if let Some(index) = self.activity.iter().position(|h| *h == focus) {
            return match index {
                0 => PanelId::Explorer,
                1 => PanelId::Search,
                2 => PanelId::Calendar,
                3 => PanelId::Outline,
                4 => PanelId::Git,
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
        let role = class.to_string().unwrap_or_default();
        let list_adapter = role.eq_ignore_ascii_case("LISTBOX")
            && style.0 & (LBS_MULTICOLUMN as u32 | LBS_OWNERDRAWVARIABLE as u32) == 0;
        let edit_adapter = role.eq_ignore_ascii_case("EDIT")
            && style.0 & ES_MULTILINE as u32 != 0
            && style.0 & (WS_HSCROLL.0 | WS_VSCROLL.0) != 0;
        let create = |requested: WINDOW_STYLE| {
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
                    | requested,
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
        };
        let mut hwnd = create(if list_adapter {
            WINDOW_STYLE(style.0 & !WS_VSCROLL.0)
        } else {
            style
        });
        if hwnd != HWND::default()
            && list_adapter
            && super::control_scroll::attach_listbox(hwnd, Tokens::default()).is_err()
        {
            // Before publication or population, recreate with the native policy
            // established at creation; toggling V later changes wheel semantics.
            let _ = DestroyWindow(hwnd);
            hwnd = create(style);
        }
        let configure = |hwnd| {
            let _ = control_style::inherit_font(hwnd, parent);
            let _ = control_style::install_child_cursor(hwnd);
        };
        if hwnd != HWND::default() {
            configure(hwnd);
            if edit_adapter && let Err(error) = super::edit_scroll::attach(hwnd, Tokens::default())
            {
                // This HWND is unpublished. Recreate ONCE with its original
                // native policy, including when attach rollback failed.
                eprintln!("Native EDIT scrollbar fallback (ID {id}): {error}");
                if let Err(error) = DestroyWindow(hwnd) {
                    eprintln!("Cannot retire unpublished EDIT (ID {id}): {error}");
                    return HWND::default();
                }
                hwnd = create(style);
                if hwnd != HWND::default() {
                    configure(hwnd);
                }
            }
        }
        hwnd
    }
}

struct NativeRailRefresh {
    tokens: Tokens,
    errors: Vec<String>,
}
unsafe extern "system" fn refresh_native_rails(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
    unsafe {
        let context = data.0 as *mut NativeRailRefresh;
        // Snapshot before any synchronous native call; hold no context borrow.
        let tokens = (*context).tokens;
        let _ = super::control_scroll::refresh_listbox(hwnd, tokens);
        if let Err(error) = super::edit_scroll::refresh(hwnd, tokens) {
            let id = GetDlgCtrlID(hwnd);
            (*context).errors.push(format!("ID {id}: {error}"));
        }
        true.into()
    }
}

/// Refresh already attached native rails below an owned UI-thread surface.
/// No App access is needed during synchronous child/native paint reentry.
pub(super) unsafe fn refresh_native_scrollbars(
    surface: HWND,
    tokens: Tokens,
) -> Result<(), String> {
    unsafe {
        let mut process = 0;
        if GetWindowThreadProcessId(surface, Some(&mut process))
            != windows_sys::Win32::System::Threading::GetCurrentThreadId()
            || process != std::process::id()
        {
            return Err("Native scrollbar refresh requires its owning UI thread".into());
        }
        let mut context = NativeRailRefresh {
            tokens,
            errors: Vec::new(),
        };
        let _ = EnumChildWindows(
            Some(surface),
            Some(refresh_native_rails),
            LPARAM((&mut context as *mut NativeRailRefresh) as isize),
        );
        if context.errors.is_empty() {
            Ok(())
        } else {
            Err(context.errors.join("; "))
        }
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

const QUERY_PREVIOUS_FOCUS: PCWSTR = w!("MDLite.Query.PreviousFocus");
unsafe extern "system" fn toolbar_query_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    id: usize,
    _: usize,
) -> LRESULT {
    unsafe {
        if msg == WM_SETFOCUS {
            if wp.0 != 0 {
                let _ = SetPropW(hwnd, QUERY_PREVIOUS_FOCUS, Some(HANDLE(wp.0 as *mut _)));
            }
        } else if msg == WM_NCDESTROY {
            let _ = RemovePropW(hwnd, QUERY_PREVIOUS_FOCUS);
            let _ = RemoveWindowSubclass(hwnd, Some(toolbar_query_proc), id);
        }
        DefSubclassProc(hwnd, msg, wp, lp)
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
            WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
        ) && IsChild(hwnd, HWND(lp.0 as *mut _)).as_bool()
        {
            let owner = HWND(owner as *mut _);
            let pointer = GetWindowLongPtrW(owner, GWLP_USERDATA) as *const WindowState;
            if let Some(state) = pointer.as_ref()
                && state.owns_paint_surface(owner)
                && GetParent(hwnd).ok() == Some(owner)
            {
                let colors = state.control_colors.borrow().clone();
                if let Some(colors) = colors {
                    return colors.paint(wp);
                }
            }
        }
        if msg == WM_COMMAND
            || (matches!(
                msg,
                WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
            ) && IsChild(hwnd, HWND(lp.0 as *mut _)).as_bool())
        {
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
) -> (Rect, [(PanelId, Rect); 5]) {
    let narrow = width < 760.;
    let mut geometry = layout.geometry((width - 4.).max(0.), height, TOP_BAR_HEIGHT, 24.);
    let mut panels = geometry.panels;
    if narrow {
        geometry.editor = Rect {
            x: 44.0_f32.min((width - 4.).max(0.)),
            width: (width - 48.).max(0.),
            ..geometry.editor
        };
        panels = IDS.map(|id| (id, Rect::default()));
        if let Some(id) = drawer.filter(|id| layout.region_visible(*id) && layout.selected(*id)) {
            let mut overlay = layout.clone();
            let side = overlay.side(id);
            overlay.left_visible = side == Side::Left;
            overlay.right_visible = side == Side::Right;
            overlay.set_width(id, (width - 48.).clamp(1., 300.));
            let g = overlay.geometry(1000., height, TOP_BAR_HEIGHT, 24.);
            for (view, mut r) in g.panels {
                if r.width > 0. {
                    r.x = 48.;
                    r.width = (width - 48.).clamp(0., 300.);
                }
                panels[view as usize].1 = r;
            }
        }
    } else {
        for (_, r) in &mut panels {
            if r.width > 0. {
                r.x += 4.;
            }
        }
    }
    let center = geometry.editor;
    (
        Rect {
            x: center.x + 4. + 20.,
            y: EDITOR_TOP,
            width: (center.width - 40.).max(1.),
            height: (height - EDITOR_TOP - 24.).max(1.),
        },
        panels,
    )
}

fn toggle_region_visibility(layout: &mut PanelLayout, left: bool) {
    if left {
        layout.left_visible = !layout.left_visible;
    } else {
        layout.right_visible = !layout.right_visible;
    }
}
fn toolbar_query_rect(width: f32, compact: bool) -> Rect {
    let left = if compact {
        8.
    } else if width >= 480. {
        172.
    } else {
        40.
    };
    let right = if compact { width - 8. } else { width - 80. };
    let available = (right - left).max(1.);
    let width = available.min(500.);
    Rect {
        x: left + (available - width) / 2.,
        y: 4.,
        width,
        height: 28.,
    }
}
impl App {
    fn navigation_point(&self) -> NavigationPoint {
        NavigationPoint {
            document: self.doc().id(),
            selection: self.doc().selection(),
            scroll: self.tabs[self.active].scroll,
        }
    }
    pub(super) fn record_navigation(&mut self) {
        if self.shell.navigating || self.composing || self.tabs.get(self.active).is_none() {
            return;
        }
        let point = self.navigation_point();
        if self.shell.nav_back.last() != Some(&point) {
            self.shell.nav_back.push(point);
            if self.shell.nav_back.len() > 128 {
                self.shell.nav_back.remove(0);
            }
        }
        self.shell.nav_forward.clear();
    }
    fn navigate_position(&mut self, forward: bool) {
        if self.composing || self.background_mutation_busy() {
            return;
        }
        loop {
            let point = if forward {
                self.shell.nav_forward.pop()
            } else {
                self.shell.nav_back.pop()
            };
            let Some(point) = point else { return };
            let Some(index) = self
                .tabs
                .iter()
                .position(|tab| tab.doc.id() == point.document)
            else {
                continue;
            };
            let current = self.navigation_point();
            if current == point {
                continue;
            }
            if forward {
                self.shell.nav_back.push(current);
            } else {
                self.shell.nav_forward.push(current);
            }
            self.shell.navigating = true;
            self.switch_tab(index);
            self.shell.navigating = false;
            let source = self.doc().source();
            let boundary = |byte: usize| {
                let mut byte = byte.min(source.len());
                while !source.is_char_boundary(byte) {
                    byte -= 1;
                }
                if mdlite::coordinates::is_grapheme_boundary(source, byte).unwrap_or(false) {
                    byte
                } else {
                    mdlite::coordinates::previous_grapheme_boundary(source, byte).unwrap_or(0)
                }
            };
            let selection = Selection {
                anchor: boundary(point.selection.anchor),
                head: boundary(point.selection.head),
            };
            let _ = self
                .doc_mut()
                .set_selection(selection.anchor, selection.head);
            self.tabs[self.active].scroll = point.scroll.max(0.);
            self.layout_key = None;
            unsafe {
                // A successful explicit navigation resumes editing at the
                // restored position. Move focus before layout can disable the
                // invoking Back/Forward button as its history stack empties.
                let _ = SetFocus(Some(self.editing_window()));
                self.after_input();
            }
            return;
        }
    }
    fn activity_view(index: usize) -> Option<PanelId> {
        [
            PanelId::Explorer,
            PanelId::Search,
            PanelId::Calendar,
            PanelId::Outline,
            PanelId::Git,
        ]
        .get(index)
        .copied()
    }
    fn activity_button_visible(&self, index: usize) -> bool {
        let Some(id) = Self::activity_view(index) else {
            return index == ACTIVITY_COUNT - 1;
        };
        let group = self.shell.layout.group(id);
        group.side == Side::Left && group.representative == id
    }
    fn activity_button_active(&self, index: usize) -> bool {
        Self::activity_view(index).is_some_and(|id| {
            self.activity_button_visible(index)
                && self.shell.layout.selected(id)
                && self.shell.layout.region_visible(id)
                && (self.width >= 760.
                    || self
                        .shell
                        .drawer
                        .is_some_and(|drawer| self.shell.layout.group(drawer).representative == id))
        })
    }
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
            "left_region_visible":self.shell.layout.left_visible,"right_region_visible":self.shell.layout.right_visible,"editor_rect":rectangle(editor),"active_editor_rect":rectangle(self.shell.editor),"search_active":self.search_pane_active(),
            "panel_rects":panels.iter().map(|(id,r)|{let state=self.shell.layout.get(*id);serde_json::json!({"id":(["explorer","calendar","outline","git","search"][*id as usize]),"rect":rectangle(*r),"body":rectangle(Rect{x:r.x,y:r.y+30.,width:r.width,height:(r.height-30.).max(0.)}),"hidden":state.hidden,"collapsed":state.collapsed,"side":format!("{:?}",self.shell.layout.side(*id)),"group":format!("{:?}",self.shell.layout.group(*id).representative),"selected":self.shell.layout.selected(*id)})}).collect::<Vec<_>>(),
            "panel_groups":self.shell.layout.groups.iter().map(|g|serde_json::json!({"representative":format!("{:?}",g.representative),"side":format!("{:?}",g.side),"views":g.views.iter().map(|v|format!("{:?}",v)).collect::<Vec<_>>(),"width":g.width,"active":self.shell.layout.active[g.side.index()]==Some(g.representative)})).collect::<Vec<_>>(),"features":self.feature_test_geometry(&panels),"search_busy":self.search_busy(),"git_job_busy":self.git_job_busy(),"mutation_busy":self.background_mutation_busy()})
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
        if active {
            self.show_panel(PanelId::Search);
        } else {
            let side = self.shell.layout.side(PanelId::Search);
            if self.shell.layout.group(PanelId::Search).views.len() > 1 {
                self.shell.layout.get_mut(PanelId::Search).hidden = true;
                self.presentation_changed();
                self.save_panel_layout();
            } else if self.shell.layout.side(PanelId::Explorer) == side {
                self.show_panel(PanelId::Explorer);
            } else if let Some(id) = self
                .shell
                .layout
                .groups
                .iter()
                .find(|g| g.side == side && !g.views.contains(&PanelId::Search))
                .map(|g| g.representative)
            {
                self.show_panel(id);
            } else {
                self.shell.layout.get_mut(PanelId::Search).hidden = true;
                self.presentation_changed();
                self.save_panel_layout();
            }
        }
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
    pub(super) fn content_left(&self) -> f32 {
        self.editor_left() + self.gutter_width
    }
    pub(super) fn content_width(&self) -> f32 {
        (self.editor_right() - self.content_left() - 12.0).max(50.0)
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
    pub(super) fn update_surface_font(&mut self, hwnd: HWND, dpi: u32) {
        let slot = if hwnd == self.shell.main_window {
            &mut self.shell.main_font
        } else if hwnd == self.shell.compact_window {
            &mut self.shell.compact_font
        } else {
            return;
        };
        if let Some(font) = slot {
            let _ = font.replace_on(hwnd, dpi, UI_FONT_SIZE);
        } else if let Ok(mut font) = control_style::UiFont::new(dpi, UI_FONT_SIZE)
            && font.apply_to(hwnd).is_ok()
        {
            *slot = Some(font);
        }
        if let Some(palette) = &mut self.shell.palette
            && unsafe { GetParent(palette.host).ok() } == Some(hwnd)
            && let Some(font) = &mut palette.font
        {
            let _ = font.replace_on(palette.host, dpi, UI_FONT_SIZE);
            unsafe {
                send(
                    palette.list,
                    LB_SETITEMHEIGHT,
                    WPARAM(0),
                    LPARAM((22. * dpi as f32 / 96.).round() as isize),
                );
            }
        }
    }
    unsafe fn ensure_toolbar_query(&mut self, owner: HWND) -> HWND {
        unsafe {
            let compact = owner == self.shell.compact_window && owner != HWND::default();
            let existing = if compact {
                self.shell.compact_query
            } else {
                self.shell.command_button
            };
            if IsWindow(Some(existing)).as_bool() {
                return existing;
            }
            let query = control(
                owner,
                w!("EDIT"),
                "",
                QUERY,
                WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            );
            let _ = SetWindowSubclass(query, Some(toolbar_query_proc), 72, 0);
            send(
                query,
                EM_SETCUEBANNER,
                WPARAM(1),
                LPARAM(wide("コマンドを検索… Ctrl+K").as_ptr() as isize),
            );
            if compact {
                self.shell.compact_query = query;
            } else {
                self.shell.command_button = query;
            }
            query
        }
    }
    pub(super) unsafe fn start_shell(&mut self) {
        unsafe {
            self.shell.enabled = true;
            self.shell.main_window = self.hwnd;
            self.update_surface_font(self.hwnd, GetDpiForWindow(self.hwnd));
            self.shell.menu_button = control(
                self.hwnd,
                w!("BUTTON"),
                "メニュー",
                SHELL_MENU,
                WINDOW_STYLE(BS_OWNERDRAW as u32),
            );
            for (id, label) in [
                (CMD_NEW, "新しい文書"),
                (CMD_SAVE, "保存"),
                (NAV_BACK, "戻る"),
                (NAV_FORWARD, "進む"),
            ] {
                self.shell.top_actions.push(control(
                    self.hwnd,
                    w!("BUTTON"),
                    label,
                    id,
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                ));
            }
            self.ensure_toolbar_query(self.hwnd);
            for (index, label) in ["左領域を表示 / 非表示", "右領域を表示 / 非表示"]
                .into_iter()
                .enumerate()
            {
                self.shell.region_buttons[index] = control(
                    self.hwnd,
                    w!("BUTTON"),
                    label,
                    if index == 0 {
                        REGION_LEFT
                    } else {
                        REGION_RIGHT
                    },
                    WINDOW_STYLE(BS_OWNERDRAW as u32),
                );
            }
            for (i, label) in [
                "エクスプローラー",
                "検索",
                "カレンダー",
                "アウトライン",
                "Git（ソース管理）",
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
            for (id, child, _) in self.shell_button_paint().buttons {
                if !valid_shell_erase_button(child, self.hwnd, id)
                    || !SetWindowSubclass(child, Some(shell_erase_proc), id, self.hwnd.0 as usize)
                        .as_bool()
                {
                    self.status = "Shell描画の初期化に失敗しました".into();
                }
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
            let _ = control_style::inherit_font(self.shell.tooltip, self.hwnd);
            for h in self
                .shell
                .top_actions
                .iter()
                .chain(std::iter::once(&self.shell.menu_button))
                .chain(self.shell.activity.iter())
                .chain(self.shell.region_buttons.iter())
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
            let (
                Ok(menu),
                Ok(file),
                Ok(view),
                Ok(workspace),
                Ok(search),
                Ok(git),
                Ok(editor),
                Ok(navigation),
            ) = (
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
                CreatePopupMenu(),
            )
            else {
                return;
            };
            let mut git_groups = Vec::new();
            for label in ["ブランチ", "リモート", "競合", "画像ストレージ"] {
                let submenu = CreatePopupMenu().ok();
                if let Some(submenu) = submenu {
                    let _ = AppendMenuW(
                        git,
                        MF_POPUP,
                        submenu.0 as usize,
                        PCWSTR(wide(label).as_ptr()),
                    );
                }
                git_groups.push(submenu);
            }
            let mut routes = HashMap::new();
            for command in commands() {
                if command == Command::Git(6027) {
                    continue;
                }
                let mut destination = [file, view, workspace, search, git, editor, navigation]
                    [command_menu_group(command)];
                if let Command::Git(id) = command {
                    let subgroup = match id {
                        6009..=6012 => Some(0),
                        6013..=6015 => Some(1),
                        6016..=6022 | 6025 => Some(2),
                        6023..=6024 => Some(3),
                        _ => None,
                    };
                    if let Some(submenu) = subgroup
                        .and_then(|index| git_groups.get(index))
                        .and_then(|entry| *entry)
                    {
                        destination = submenu;
                    }
                }
                routes.insert(command.id(), destination);
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
            let _ = AppendMenuW(menu, MF_POPUP, navigation.0 as usize, w!("移動"));
            let owner = if self.shell.main_window != HWND::default() {
                self.shell.main_window
            } else {
                self.hwnd
            };
            let attached = GetMenu(owner);
            if attached != HMENU::default() {
                // Only an attached native bar changed. Popup-menu replacement
                // must not synchronously erase all child surfaces before paint.
                if SetMenu(owner, None).is_ok() {
                    let _ = DrawMenuBar(owner);
                }
                if attached != self.shell.menu {
                    let _ = DestroyMenu(attached);
                }
            }
            if self.shell.menu != HMENU::default() && self.shell.menu != attached {
                let _ = DestroyMenu(self.shell.menu);
            }
            self.shell.menu = menu;
            self.shell.menu_groups = [file, view, workspace, search, git, editor, navigation];
            self.shell.menu_routes = routes;
        }
    }
    unsafe fn open_shell_menu(&mut self, group: Option<usize>) {
        unsafe {
            let menu = group
                .and_then(|index| self.shell.menu_groups.get(index).copied())
                .filter(|menu| *menu != HMENU::default())
                .unwrap_or(self.shell.menu);
            if menu == HMENU::default() {
                return;
            }
            for command in commands() {
                let group = command_menu_group(command);
                let destination = self
                    .shell
                    .menu_routes
                    .get(&command.id())
                    .copied()
                    .unwrap_or(self.shell.menu_groups[group]);
                let flags = MF_BYCOMMAND
                    | MF_STRING
                    | if self.command_enabled(command) {
                        MF_ENABLED
                    } else {
                        MF_GRAYED
                    };
                let _ = ModifyMenuW(
                    destination,
                    command.id() as u32,
                    flags,
                    command.id(),
                    PCWSTR(wide(&self.command_label(command)).as_ptr()),
                );
            }
            let mut button = RECT::default();
            let _ = GetWindowRect(self.shell.menu_button, &mut button);
            let focus = GetFocus();
            let selected = self
                .track_command_popup(
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY,
                    button.left,
                    button.bottom,
                )
                .0 as usize;
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));
            if focus != HWND::default() && IsWindow(Some(focus)).as_bool() {
                let _ = SetFocus(Some(focus));
            }
            if selected != 0 {
                self.command(selected);
                self.refresh();
            }
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
        let label = c
            .label()
            .strip_prefix("Git: ")
            .unwrap_or(c.label())
            .split('（')
            .next()
            .unwrap_or(c.label())
            .trim()
            .to_owned();
        match hint {
            Some(h) => format!("{label}\t{h}"),
            None => label,
        }
    }
    pub(super) unsafe fn track_command_popup(
        &self,
        menu: HMENU,
        flags: TRACK_POPUP_MENU_FLAGS,
        x: i32,
        y: i32,
    ) -> windows::core::BOOL {
        let help: Vec<_> = commands()
            .map(|command| (command.id(), self.command_help(command)))
            .collect();
        unsafe {
            menu_style::track_popup_help(
                self.hwnd,
                menu,
                flags,
                x,
                y,
                self.shell.tokens,
                self.dpi,
                &help,
            )
        }
    }
    fn command_help(&self, command: Command) -> String {
        if self.command_enabled(command) {
            match command {
                Command::Navigate(_) => {
                    "文書と編集位置の履歴を移動します。本文のUndo / Redoとは別です。".to_owned()
                }
                Command::PanelMove(_) => {
                    "このviewの所属だけを明示的に変更します。反対側のviewと入れ替えません。"
                        .to_owned()
                }
                _ => command.label().to_owned(),
            }
        } else {
            self.command_disabled_reason(command).to_owned()
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
        if let Command::Navigate(forward) = c {
            let entries = if forward {
                &self.shell.nav_forward
            } else {
                &self.shell.nav_back
            };
            return !self.feature_ime_active()
                && !self.background_mutation_busy()
                && entries.iter().any(|point| {
                    *point != self.navigation_point()
                        && self.tabs.iter().any(|tab| tab.doc.id() == point.document)
                });
        }
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
    fn shell_button_paint(&self) -> ShellButtonPaint {
        let mut buttons = vec![
            (SHELL_MENU, self.shell.menu_button, false),
            (
                REGION_LEFT,
                self.shell.region_buttons[0],
                self.shell.layout.left_visible,
            ),
            (
                REGION_RIGHT,
                self.shell.region_buttons[1],
                self.shell.layout.right_visible,
            ),
        ];
        buttons.extend(
            [CMD_NEW, CMD_SAVE, NAV_BACK, NAV_FORWARD]
                .into_iter()
                .zip(self.shell.top_actions.iter().copied())
                .map(|(id, child)| (id, child, false)),
        );
        buttons.extend(
            self.shell
                .activity
                .iter()
                .enumerate()
                .map(|(i, child)| (ACTIVITY + i, *child, self.activity_button_active(i))),
        );
        buttons.extend(
            self.shell
                .headers
                .iter()
                .enumerate()
                .map(|(i, child)| (HEADER + i, *child, false)),
        );
        ShellButtonPaint {
            owner: self.shell.main_window,
            tokens: self.shell.tokens,
            buttons,
        }
    }
    fn publish_shell_button_paint(&self) {
        if self.defer_theme_updates {
            return;
        }
        unsafe {
            let pointer =
                GetWindowLongPtrW(self.shell.main_window, GWLP_USERDATA) as *const WindowState;
            if let Some(state) = pointer.as_ref()
                && state.owns_paint_surface(self.shell.main_window)
            {
                let retired = state
                    .shell_button_paint
                    .replace(Some(Rc::new(self.shell_button_paint())));
                drop(retired);
            }
        }
    }
    fn publish_shell_control_colors(&self) {
        if self.defer_theme_updates {
            return;
        }
        unsafe {
            let pointer =
                GetWindowLongPtrW(self.shell.main_window, GWLP_USERDATA) as *const WindowState;
            if let Some(state) = pointer.as_ref()
                && state.owns_paint_surface(self.shell.main_window)
            {
                let retired = state
                    .control_colors
                    .replace(Some(Rc::clone(&self.shell.colors)));
                drop(retired);
            }
        }
    }
    pub(super) fn publish_prepared_theme_paint(&mut self) {
        unsafe {
            let pointer =
                GetWindowLongPtrW(self.shell.main_window, GWLP_USERDATA) as *const WindowState;
            if let Some(state) = pointer.as_ref()
                && state.owns_paint_surface(self.shell.main_window)
            {
                let buttons = Rc::new(self.shell_button_paint());
                // Replace every ready slot before dropping any retired GDI
                // resource. No RefCell guard spans a native callback.
                let colors = state
                    .control_colors
                    .replace(Some(Rc::clone(&self.shell.colors)));
                let buttons = state.shell_button_paint.replace(Some(buttons));
                let explorer = self
                    .staged_explorer_paint
                    .take()
                    .map(|next| state.explorer_paint.replace(next));
                drop((colors, buttons, explorer));
            }
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
                    y: EDITOR_TOP,
                    width: (self.width - 48.).max(1.),
                    height: (height - EDITOR_TOP - 24.).max(1.),
                },
                std::array::from_fn(|i| (IDS[i], Rect::default())),
            )
        } else {
            shell_geometry(&self.shell.layout, self.width, height, self.shell.drawer)
        };
        self.shell.editor = editor;
        self.shell.panels = panels;
        self.publish_shell_button_paint();
        if compact {
            unsafe {
                let query = self.ensure_toolbar_query(self.hwnd);
                let rect = toolbar_query_rect(self.width, true);
                let _ = position_child_control(
                    query,
                    (rect.x * self.dpi) as i32,
                    (rect.y * self.dpi) as i32,
                    (rect.width * self.dpi) as i32,
                    (rect.height * self.dpi) as i32,
                );
                self.position_palette();
            }
            return;
        }
        let size = (self.width.to_bits(), height.to_bits());
        for (index, forward) in [(2, false), (3, true)] {
            if let Some(child) = self.shell.top_actions.get(index).copied() {
                let command = Command::Navigate(forward);
                unsafe {
                    enable_owner_button(
                        OwnerButtonPaint {
                            recipient: self.shell.main_window,
                            parent: self.shell.main_window,
                            parent_id: None,
                            child,
                            id: command.id(),
                            tokens: self.shell.tokens,
                            label: command.label().into(),
                        },
                        self.command_enabled(command),
                    );
                }
            }
        }
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
            let actions_visible = self.width >= 480.;
            let _ = show_child_control(self.shell.menu_button, true);
            position(
                self.shell.menu_button,
                Rect {
                    x: 8.,
                    y: 4.,
                    width: 28.,
                    height: 28.,
                },
            );
            for (i, h) in self.shell.top_actions.iter().enumerate() {
                let _ = show_child_control(*h, actions_visible);
                position(
                    *h,
                    Rect {
                        x: 40. + i as f32 * 32.,
                        y: 4.,
                        width: 28.,
                        height: 28.,
                    },
                );
            }
            let mut activity_row = 0;
            for (i, h) in self.shell.activity.iter().enumerate() {
                let visible = self.activity_button_visible(i);
                let y = if i == ACTIVITY_COUNT - 1 {
                    (height - 24. - 36.).max(TOP_BAR_HEIGHT + activity_row as f32 * 36.)
                } else {
                    TOP_BAR_HEIGHT + activity_row as f32 * 36.
                };
                if visible && i != ACTIVITY_COUNT - 1 {
                    activity_row += 1;
                }
                let _ = show_child_control(*h, visible && y + 32. <= height - 24.);
                position(
                    *h,
                    Rect {
                        x: 8.,
                        y,
                        width: 32.,
                        height: 32.,
                    },
                );
            }
            for (i, h) in self.shell.region_buttons.iter().enumerate() {
                let _ = show_child_control(*h, true);
                position(
                    *h,
                    Rect {
                        x: self.width - 72. + i as f32 * 32.,
                        y: 4.,
                        width: 28.,
                        height: 28.,
                    },
                );
            }
            position(
                self.shell.command_button,
                toolbar_query_rect(self.width, false),
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
            if self.defer_theme_updates {
                // New settings/tokens prepare layout now; independent native
                // paint resources stay on the presented theme until completion.
                self.theme_update_pending = true;
            }
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
                    weekend_red: color(0.72, 0.12, 0.16),
                    weekend_blue: color(0.10, 0.31, 0.68),
                    high_contrast: false,
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
                tokens.weekend_red = tokens.foreground;
                tokens.weekend_blue = tokens.foreground;
                tokens.high_contrast = true;
            }
            self.shell.tokens = tokens;
            self.shell.colors = Rc::new(ControlColors::new(tokens));
            self.publish_shell_control_colors();
            self.publish_shell_button_paint();
            self.features_refresh_explorer_paint(false);
            self.layout_key = None;
            self.shell.controls_dirty = true;
            if !self.defer_theme_updates {
                self.apply_shell_native_theme();
            }
        }
    }
    pub(super) unsafe fn apply_shell_native_theme(&mut self) {
        unsafe {
            let tokens = self.shell.tokens;
            let light = match self.shell.settings.theme {
                mdlite::settings::ThemeMode::Light => true,
                mdlite::settings::ThemeMode::System => system_light(),
                _ => false,
            };
            self.diagnostics.refresh_theme(tokens);
            for surface in [self.shell.main_window, self.shell.compact_window] {
                if surface != HWND::default()
                    && let Err(error) = refresh_native_scrollbars(surface, tokens)
                {
                    self.status = format!("スクロール表示の更新に失敗しました: {error}");
                }
            }
            let dark = windows::core::BOOL(i32::from(!light));
            let _ = DwmSetWindowAttribute(
                self.shell.main_window,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                (&dark as *const windows::core::BOOL).cast(),
                std::mem::size_of_val(&dark) as u32,
            );
            // Theme changes affect every child, including controls whose bounds
            // and enabled state are unchanged by the following layout pass.
            for surface in [self.shell.main_window, self.shell.compact_window] {
                if surface != HWND::default() {
                    let _ =
                        RedrawWindow(Some(surface), None, None, RDW_INVALIDATE | RDW_ALLCHILDREN);
                }
            }
        }
    }
    pub(super) unsafe fn load_panel_layout(&mut self) {
        unsafe {
            self.shell.layout = PanelLayout::default();
            self.shell.drawer = None;
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
    pub(super) unsafe fn draw_shell(&self, t: &ID2D1RenderTarget) -> windows::core::Result<()> {
        unsafe {
            if !self.shell.enabled {
                return Ok(());
            }
            let panel = t.CreateSolidColorBrush(&self.shell.tokens.panel, None)?;
            let border = t.CreateSolidColorBrush(&self.shell.tokens.border, None)?;
            let fg = t.CreateSolidColorBrush(&self.shell.tokens.foreground, None)?;
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            if self.hwnd == self.shell.compact_window {
                let text = self.ui_text_layout(
                    "Compact — ×で主windowへ戻す",
                    (self.width - 152.).max(1.),
                    12.,
                )?;
                t.DrawTextLayout(point(16., 8.), &text, &fg, D2D1_DRAW_TEXT_OPTIONS_CLIP);
                return Ok(());
            }
            t.FillRectangle(&rect(0., 0., self.width, TOP_BAR_HEIGHT), &panel);
            for (id, r) in self.shell.panels {
                if r.width == 0. {
                    continue;
                }
                t.FillRectangle(&rect(r.x, r.y, r.width, r.height.min(30.)), &panel);
                if let Some(body) = self.feature_panel_rect(id) {
                    t.FillRectangle(&d2d(body), &panel);
                }
                t.DrawRectangle(&d2d(r), &border, 1., None);
                let name = if id == PanelId::Search {
                    "検索"
                } else {
                    NAMES[id as usize]
                };
                let text = self.ui_text_layout(name, (r.width - 42.).max(1.), 12.)?;
                t.DrawTextLayout(
                    point(r.x + 12., r.y + 7.),
                    &text,
                    &fg,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            if self.shell.compact_window != HWND::default() {
                let label = self.ui_text_layout(
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
}
unsafe fn draw_shell_button(tokens: Tokens, active: bool, item: &DRAWITEMSTRUCT) {
    unsafe {
        let control_dpi = (GetDpiForWindow(item.hwndItem) as f32 / 96.).max(1.);
        let id = item.CtlID as usize;
        let activity = id
            .checked_sub(ACTIVITY)
            .filter(|index| *index < ACTIVITY_COUNT);
        let action = match id {
            CMD_NEW => Some(0),
            CMD_SAVE => Some(1),
            NAV_BACK => Some(2),
            NAV_FORWARD => Some(3),
            _ => None,
        };
        let bg = if item.itemState.0 & ODS_SELECTED.0 != 0 || active {
            tokens.selected
        } else {
            tokens.panel
        };
        let brush = CreateSolidBrush(rgb(bg));
        FillRect(item.hDC, &item.rcItem, brush);
        let _ = DeleteObject(brush.into());
        if active {
            let brush = CreateSolidBrush(rgb(tokens.accent));
            let stripe = RECT {
                left: item.rcItem.left,
                top: item.rcItem.bottom - (3. * control_dpi).round() as i32,
                right: item.rcItem.right,
                bottom: item.rcItem.bottom,
            };
            FillRect(item.hDC, &stripe, brush);
            let _ = DeleteObject(brush.into());
        }
        let icon = if id == SHELL_MENU {
            Some(HAMBURGER_ICON)
        } else if id == REGION_LEFT || id == REGION_RIGHT {
            Some(REGION_ICONS[usize::from(id == REGION_RIGHT)])
        } else if (HEADER..HEADER + IDS.len()).contains(&id) {
            Some(MENU_ICON)
        } else if let Some(index) = action {
            Some(TOP_ACTION_ICONS[index])
        } else {
            activity.map(|index| ACTIVITY_ICONS[index])
        };
        if let Some(icon) = icon {
            draw_icon(
                item.hDC,
                &item.rcItem,
                icon,
                control_dpi,
                tokens.foreground,
                bg,
            );
        } else {
            let text = text_of(item.hwndItem);
            let mut area = item.rcItem;
            let font = HGDIOBJ(send(item.hwndItem, WM_GETFONT, WPARAM(0), LPARAM(0)).0 as *mut _);
            let previous = if font != HGDIOBJ::default() {
                Some(SelectObject(item.hDC, font))
            } else {
                None
            };
            let _ = SetBkMode(item.hDC, TRANSPARENT);
            SetTextColor(item.hDC, rgb(tokens.foreground));
            draw_gdi_text(
                item.hDC,
                &text,
                &mut area,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS,
            );
            if let Some(previous) = previous {
                let _ = SelectObject(item.hDC, previous);
            }
        }
        if item.itemState.0 & ODS_FOCUS.0 != 0 {
            control_style::draw_focus(tokens, item, &item.rcItem, true);
        }
    }
}

fn owner_button_icon(label: &str) -> Option<&'static str> {
    Some(match label {
        "新しいファイル" => include_str!("../../assets/ui/icons/file-plus.svg"),
        "新しいフォルダー" => include_str!("../../assets/ui/icons/folder-plus.svg"),
        "前月" => include_str!("../../assets/ui/icons/chevron-left.svg"),
        "次月" => include_str!("../../assets/ui/icons/chevron-right.svg"),
        "今日" => include_str!("../../assets/ui/icons/calendar.svg"),
        "更新" => include_str!("../../assets/ui/icons/refresh.svg"),
        "Stage" => include_str!("../../assets/ui/icons/add.svg"),
        "Unstage" => include_str!("../../assets/ui/icons/remove.svg"),
        "取消" => include_str!("../../assets/ui/icons/close.svg"),
        "その他のGit操作" => MENU_ICON,
        _ => return None,
    })
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
        if let Some(icon) = owner_button_icon(&state.label) {
            draw_icon(item.hDC, &area, icon, dpi, foreground, background);
        } else {
            let font = HGDIOBJ(send(item.hwndItem, WM_GETFONT, WPARAM(0), LPARAM(0)).0 as *mut _);
            let previous = if font != HGDIOBJ::default() {
                Some(SelectObject(item.hDC, font))
            } else {
                None
            };
            draw_gdi_text(
                item.hDC,
                &state.label,
                &mut area,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
            if let Some(previous) = previous {
                let _ = SelectObject(item.hDC, previous);
            }
        }
        if state.focused {
            let inset = (3. * dpi).round() as i32;
            let focus = RECT {
                left: item.rcItem.left + inset,
                top: item.rcItem.top + inset,
                right: item.rcItem.right - inset,
                bottom: item.rcItem.bottom - inset,
            };
            control_style::draw_focus(tokens, item, &focus, true);
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
                WM_CONTEXTMENU if self.shell.region_buttons.contains(&HWND(wp.0 as *mut _)) => {
                    let origin = HWND(wp.0 as *mut _);
                    let side = if origin == self.shell.region_buttons[0] {
                        Side::Left
                    } else {
                        Side::Right
                    };
                    let mut point = POINT {
                        x: lp.0 as u16 as i16 as i32,
                        y: (lp.0 >> 16) as u16 as i16 as i32,
                    };
                    if point.x == -1 && point.y == -1 {
                        let mut r = RECT::default();
                        let _ = GetWindowRect(origin, &mut r);
                        point = POINT {
                            x: r.left,
                            y: r.bottom,
                        };
                    }
                    self.region_group_menu(side, point.x, point.y);
                    Some(LRESULT(0))
                }
                WM_DRAWITEM => {
                    if lp.0 == 0 {
                        return None;
                    }
                    let item = &*(lp.0 as *const DRAWITEMSTRUCT);
                    if (hwnd == self.shell.main_window || hwnd == self.shell.compact_window)
                        && self.shell.colors.draw_container(hwnd, wp.0, item)
                    {
                        return Some(LRESULT(1));
                    }
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
                    self.shell_button_paint()
                        .draw(hwnd, wp.0, item)
                        .then_some(LRESULT(1))
                }
                WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX | WM_CTLCOLORSTATIC | WM_CTLCOLORBTN
                    if (hwnd == self.shell.main_window || hwnd == self.shell.compact_window)
                        && IsChild(hwnd, HWND(lp.0 as *mut _)).as_bool() =>
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
                WM_COMMAND if wp.0 & 0xffff == QUERY && wp.0 >> 16 == EN_SETFOCUS as usize => {
                    self.open_palette();
                    Some(LRESULT(0))
                }
                WM_COMMAND if wp.0 & 0xffff == QUERY && wp.0 >> 16 == EN_CHANGE as usize => {
                    self.filter_palette();
                    Some(LRESULT(0))
                }
                WM_COMMAND if wp.0 & 0xffff == RESULTS && wp.0 >> 16 == LBN_SELCHANGE as usize => {
                    self.update_palette_help();
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
            if id == SHELL_MENU {
                if !self.composing && !self.feature_ime_active() {
                    self.open_shell_menu(None);
                }
                return true;
            }
            if (ACTIVITY..ACTIVITY + ACTIVITY_COUNT).contains(&id) {
                let index = id - ACTIVITY;
                if !self.activity_button_visible(index) {
                    return true;
                }
                match index {
                    0 => {
                        self.select_panel_group(PanelId::Explorer);
                        self.focus_feature_body(PanelId::Explorer);
                    }
                    1 => {
                        self.select_panel_group(PanelId::Search);
                        self.focus_search_pane_input();
                    }
                    2 => {
                        self.select_panel_group(PanelId::Calendar);
                        self.focus_feature_body(PanelId::Calendar);
                    }
                    3 => {
                        self.select_panel_group(PanelId::Outline);
                        self.focus_feature_body(PanelId::Outline);
                    }
                    4 => {
                        self.select_panel_group(PanelId::Git);
                        self.focus_feature_body(PanelId::Git);
                    }
                    _ => self.show_settings(),
                };
                return true;
            }
            if (HEADER..HEADER + IDS.len()).contains(&id) {
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
                    Command::Navigate(forward) => self.navigate_position(forward),
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
                    Command::Region(left) => self.toggle_region(left),
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
    fn select_panel_group(&mut self, id: PanelId) {
        self.focus_feature_panel(id);
        self.shell.layout.select(id);
        self.shell.layout.show_region_for(id);
        self.shell.drawer = if self.width < 760. { Some(id) } else { None };
        self.presentation_changed();
        self.save_panel_layout();
    }
    pub(super) fn show_panel(&mut self, id: PanelId) {
        self.focus_feature_panel(id);
        self.shell.layout.select(id);
        self.shell.layout.show_region_for(id);
        self.shell.layout.get_mut(id).hidden = false;
        self.shell.layout.get_mut(id).collapsed = false;
        self.shell.drawer = if self.width < 760. { Some(id) } else { None };
        self.presentation_changed();
        self.save_panel_layout();
    }
    fn toggle_region(&mut self, left: bool) {
        let visible = if left {
            self.shell.layout.left_visible
        } else {
            self.shell.layout.right_visible
        };
        let index = usize::from(!left);
        unsafe {
            let focus = GetFocus();
            let mut bounds = RECT::default();
            if visible && GetWindowRect(focus, &mut bounds).is_ok() {
                let mut point = POINT {
                    x: bounds.left,
                    y: bounds.top,
                };
                let _ = ScreenToClient(self.shell.main_window, &mut point);
                if self.shell.panels.iter().any(|(id, rect)| {
                    (self.shell.layout.side(*id) == Side::Left) == left
                        && rect.contains(point.x as f32 / self.dpi, point.y as f32 / self.dpi)
                }) {
                    self.shell.region_focus[index] = focus;
                    let _ = SetFocus(Some(self.editing_window()));
                }
            }
        }
        toggle_region_visibility(&mut self.shell.layout, left);
        self.presentation_changed();
        self.save_panel_layout();
        if !visible {
            unsafe {
                let focus = self.shell.region_focus[index];
                if IsWindow(Some(focus)).as_bool()
                    && IsWindowVisible(focus).as_bool()
                    && IsWindowEnabled(focus).as_bool()
                    && IsChild(self.shell.main_window, focus).as_bool()
                {
                    let _ = SetFocus(Some(focus));
                }
            }
        }
    }
    unsafe fn region_group_menu(&mut self, side: Side, x: i32, y: i32) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            for group in self.shell.layout.groups.iter().filter(|g| g.side == side) {
                let flags = MF_STRING
                    | if self.shell.layout.selected(group.representative) {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    };
                let _ = AppendMenuW(
                    menu,
                    flags,
                    1 + group.representative as usize,
                    PCWSTR(wide(NAMES[group.representative as usize]).as_ptr()),
                );
                for view in &group.views {
                    if self.shell.layout.get(*view).hidden {
                        let label = format!("{}を再表示", NAMES[*view as usize]);
                        let _ = AppendMenuW(
                            menu,
                            MF_STRING,
                            11 + *view as usize,
                            PCWSTR(wide(&label).as_ptr()),
                        );
                    }
                }
            }
            let command = self
                .track_command_popup(menu, TPM_RETURNCMD | TPM_NONOTIFY, x, y)
                .0 as usize;
            let _ = DestroyMenu(menu);
            if (1..=IDS.len()).contains(&command) {
                self.select_panel_group(IDS[command - 1]);
            } else if (11..11 + IDS.len()).contains(&command) {
                self.show_panel(IDS[command - 11]);
            }
        }
    }
    pub(super) unsafe fn panel_menu(&mut self, id: PanelId) {
        unsafe {
            self.shell.active_panel = id;
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            // Keep low-frequency operations beside the content they act on.
            // All commands still use the existing registry and safety guards.
            let operations: Vec<_> = if id == PanelId::Search {
                super::search_ui::COMMANDS
                    .iter()
                    .map(|entry| Command::Search(entry.0))
                    .collect()
            } else if id == PanelId::Git {
                super::git_ui::COMMANDS
                    .iter()
                    .filter(|entry| entry.0 != 6027)
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
            let mut sections = HashMap::new();
            for operation in operations {
                let section = match operation {
                    Command::Feature(4001..=4006) => Some("ファイル操作"),
                    Command::Feature(4007..=4010) => Some("表示・移動"),
                    Command::Feature(4023..=4028) => Some("ノート"),
                    Command::Feature(4080..=4088) => Some("プロファイル"),
                    Command::Git(6009..=6012) => Some("ブランチ"),
                    Command::Git(6013..=6015) => Some("リモート"),
                    Command::Git(6016..=6022 | 6025) => Some("競合"),
                    Command::Git(6023..=6024) => Some("画像ストレージ"),
                    Command::Search(5004..=5006) => Some("置換"),
                    Command::Search(5007..=5008) => Some("置換履歴"),
                    _ => None,
                };
                let destination = if let Some(section) = section {
                    if let Some(submenu) = sections.get(section) {
                        *submenu
                    } else if let Ok(submenu) = CreatePopupMenu() {
                        let _ = AppendMenuW(
                            menu,
                            MF_POPUP,
                            submenu.0 as usize,
                            PCWSTR(wide(section).as_ptr()),
                        );
                        sections.insert(section, submenu);
                        submenu
                    } else {
                        menu
                    }
                } else {
                    menu
                };
                let enabled = self.command_enabled(operation);
                let label = self.command_label(operation);
                let _ = AppendMenuW(
                    destination,
                    MF_STRING | if enabled { MF_ENABLED } else { MF_GRAYED },
                    operation.id(),
                    PCWSTR(wide(&label).as_ptr()),
                );
            }
            let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            let _ = AppendMenuW(menu, MF_STRING, 1, w!("折畳み / 展開"));
            let _ = AppendMenuW(menu, MF_STRING, 2, w!("非表示"));
            if let Ok(placement) = CreatePopupMenu() {
                for (command, label) in [
                    (3, "左へ移動"),
                    (4, "右へ移動"),
                    (5, "独立グループへ戻す"),
                    (6, "グループ全体を左へ"),
                    (7, "グループ全体を右へ"),
                ] {
                    let _ =
                        AppendMenuW(placement, MF_STRING, command, PCWSTR(wide(label).as_ptr()));
                }
                for group in &self.shell.layout.groups {
                    if !group.views.contains(&id) {
                        let label = format!("{}に追加", NAMES[group.representative as usize]);
                        let _ = AppendMenuW(
                            placement,
                            MF_STRING,
                            100 + group.representative as usize,
                            PCWSTR(wide(&label).as_ptr()),
                        );
                    }
                }
                let _ = AppendMenuW(menu, MF_POPUP, placement.0 as usize, w!("配置"));
            }
            let side = self.shell.layout.side(id);
            if let Ok(groups) = CreatePopupMenu() {
                for group in self.shell.layout.groups.iter().filter(|g| g.side == side) {
                    let flags = MF_STRING
                        | if self.shell.layout.selected(group.representative) {
                            MF_CHECKED
                        } else {
                            MF_UNCHECKED
                        };
                    let _ = AppendMenuW(
                        groups,
                        flags,
                        200 + group.representative as usize,
                        PCWSTR(wide(NAMES[group.representative as usize]).as_ptr()),
                    );
                }
                let _ = AppendMenuW(menu, MF_POPUP, groups.0 as usize, w!("この側のグループ"));
            }
            if let Ok(views) = CreatePopupMenu() {
                for view in &self.shell.layout.group(id).views {
                    let flags = MF_STRING
                        | if self.shell.layout.get(*view).hidden {
                            MF_UNCHECKED
                        } else {
                            MF_CHECKED
                        };
                    let _ = AppendMenuW(
                        views,
                        flags,
                        300 + *view as usize,
                        PCWSTR(wide(NAMES[*view as usize]).as_ptr()),
                    );
                }
                let _ = AppendMenuW(menu, MF_POPUP, views.0 as usize, w!("グループ内の表示"));
            }
            let mut p = POINT::default();
            let _ = GetCursorPos(&mut p);
            let command = self
                .track_command_popup(menu, TPM_RETURNCMD | TPM_NONOTIFY, p.x, p.y)
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
                3 => self.shell.layout.split_to(id, Side::Left),
                4 => self.shell.layout.split_to(id, Side::Right),
                5 => {
                    let side = self.shell.layout.side(id);
                    self.shell.layout.split_to(id, side);
                }
                6 => self.shell.layout.move_group(id, Side::Left),
                7 => self.shell.layout.move_group(id, Side::Right),
                100..=104 => self.shell.layout.join(id, IDS[command as usize - 100]),
                200..=204 => {
                    self.select_panel_group(IDS[command as usize - 200]);
                    return;
                }
                300..=304 => {
                    let view = IDS[command as usize - 300];
                    let p = self.shell.layout.get_mut(view);
                    p.hidden = !p.hidden;
                }
                _ => return,
            }
            self.presentation_changed();
            self.save_panel_layout();
        }
    }
    fn splitter_at(&self, x: f32, y: f32) -> Option<Splitter> {
        for (id, r) in self.shell.panels {
            if r.width == 0. {
                continue;
            }
            let left = self.shell.layout.side(id) == Side::Left;
            if self.width >= 760.
                && (x - if left { r.x + r.width } else { r.x }).abs() < 5.
                && y >= r.y
                && y <= r.y + r.height
            {
                return Some(Splitter::Width(id));
            }
            if self
                .shell
                .layout
                .group(id)
                .views
                .iter()
                .copied()
                .skip_while(|v| *v != id)
                .skip(1)
                .any(|v| !self.shell.layout.get(v).hidden)
                && (y - (r.y + r.height)).abs() < 5.
                && r.contains(x, y - 5.)
            {
                return Some(Splitter::Height(id));
            }
        }
        None
    }
    pub(super) unsafe fn begin_shell_splitter(&mut self, x: f32, y: f32) -> bool {
        if !self.shell.enabled
            || self.hwnd != self.shell.main_window
            || self.shell.compact_window != HWND::default()
            || self.composing
            || self.background_mutation_busy()
        {
            return false;
        }
        let Some(splitter) = self.splitter_at(x, y) else {
            return false;
        };
        self.shell.splitter = Some(splitter);
        unsafe { SetCapture(self.hwnd) };
        true
    }
    pub(super) fn surface_cursor_at(&self, x: f32, y: f32) -> PCWSTR {
        if !self.shell.enabled {
            return if y < 74. {
                IDC_HAND
            } else if y < self.height - 24. && x < self.width - 20. {
                IDC_IBEAM
            } else {
                IDC_ARROW
            };
        }
        let editor_active =
            self.shell.compact_window == HWND::default() || self.hwnd == self.shell.compact_window;
        if self.hwnd == self.shell.main_window
            && !self.composing
            && !self.background_mutation_busy()
        {
            if let Some(splitter) = self.shell.splitter.or_else(|| self.splitter_at(x, y)) {
                return match splitter {
                    Splitter::Width(_) => IDC_SIZEWE,
                    Splitter::Height(_) => IDC_SIZENS,
                };
            }
            if self.features_actionable_at(x, y) {
                return IDC_HAND;
            }
            if y < TOP_BAR_HEIGHT {
                return IDC_HAND;
            }
        }
        if editor_active
            && !self.composing
            && (TAB_STRIP_TOP..TAB_DIVIDER_Y + 1.).contains(&y)
            && x >= self.editor_left()
            && x <= self.editor_right()
        {
            let strip = self.tab_strip();
            if x >= self.editor_right() - 24.
                || (strip.overflow && (x < strip.left || x >= strip.right))
                || (strip.tab_width > 0.
                    && x >= strip.left
                    && x < strip.right
                    && ((x - strip.left) / strip.tab_width) < strip.visible_count as f32)
            {
                return IDC_HAND;
            }
        }
        if editor_active
            && x >= self.content_left()
            && x <= self.content_left() + self.content_width()
            && y >= self.editor_top()
            && y < self.editor_top() + self.shell.editor.height
        {
            if !self.composing
                && !self.background_mutation_busy()
                && unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0
                && self
                    .link_destination_at(self.hit_with_affinity(x, y).0)
                    .is_some()
            {
                return IDC_HAND;
            }
            return IDC_IBEAM;
        }
        IDC_ARROW
    }
    pub(super) unsafe fn shell_click(&mut self, x: f32, y: f32) {
        unsafe {
            if self.composing {
                return;
            }
            if self.shell.compact_window != HWND::default() && self.hwnd == self.shell.main_window {
                return;
            }
            if self.begin_shell_splitter(x, y) {
                return;
            }
            for (id, r) in self.shell.panels {
                if r.width == 0. {
                    continue;
                }
                if r.contains(x, y) {
                    self.shell.active_panel = id;
                    if let Some(header) = self.shell.headers.get(id as usize) {
                        let _ = SetFocus(Some(*header));
                    }
                    return;
                }
            }
            if y < TOP_BAR_HEIGHT {
                if (8. ..36.).contains(&x) && (4. ..32.).contains(&y) {
                    self.open_shell_menu(None);
                } else if toolbar_query_rect(self.width, self.shell.compact_window == self.hwnd)
                    .contains(x, y)
                {
                    self.open_palette();
                }
                return;
            }
            if y < TAB_DIVIDER_Y + 1. {
                if x >= self.editor_right() - 24. {
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
                let left = self.shell.layout.side(id) == Side::Left;
                let width = if left { x - 48. } else { self.width - x };
                self.shell.layout.set_width(id, width.clamp(100., 600.));
            }
            Splitter::Height(id) => {
                let views = self.shell.layout.group(id).views.clone();
                let next = views
                    .iter()
                    .copied()
                    .skip_while(|v| *v != id)
                    .skip(1)
                    .find(|v| !self.shell.layout.get(*v).hidden);
                if let Some(next) = next {
                    let upper = self.shell.panels[id as usize].1;
                    let lower = self.shell.panels[next as usize].1;
                    let total = upper.height + lower.height;
                    if total >= 60. {
                        let share = (y - upper.y).clamp(30., total - 30.) / total;
                        let weight =
                            self.shell.layout.get(id).height + self.shell.layout.get(next).height;
                        self.shell.layout.get_mut(id).height = (weight * share).clamp(1., 4096.);
                        self.shell.layout.get_mut(next).height =
                            (weight * (1. - share)).clamp(1., 4096.);
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
            let query = self.ensure_toolbar_query(self.hwnd);
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            let rect = toolbar_query_rect(
                client.right as f32 / self.dpi,
                self.hwnd == self.shell.compact_window,
            );
            let _ = position_child_control(
                query,
                (rect.x * self.dpi) as i32,
                (rect.y * self.dpi) as i32,
                (rect.width * self.dpi) as i32,
                (rect.height * self.dpi) as i32,
            );
            self.release_caret();
            let host = control(
                self.hwnd,
                w!("STATIC"),
                "コマンド候補",
                0,
                WS_BORDER | WS_CLIPCHILDREN | CONTAINER_STYLE | WINDOW_STYLE(0x0100), // SS_NOTIFY
            );
            let _ = SetWindowLongPtrW(host, GWL_EXSTYLE, WS_EX_CONTROLPARENT.0 as isize);
            let _ = SetWindowSubclass(host, Some(palette_host_proc), 1, self.hwnd.0 as usize);
            let list = control(
                host,
                w!("LISTBOX"),
                "コマンド候補",
                RESULTS,
                WS_VSCROLL | WINDOW_STYLE((LBS_NOTIFY | LBS_USETABSTOPS) as u32),
            );
            let help = control(
                host,
                w!("STATIC"),
                "候補の説明",
                PALETTE_HELP,
                WINDOW_STYLE(0x80),
            ); // SS_NOPREFIX
            let font = control_style::UiFont::new(GetDpiForWindow(host), UI_FONT_SIZE)
                .ok()
                .and_then(|mut font| font.apply_to(host).ok().map(|()| font));
            send(
                list,
                LB_SETITEMHEIGHT,
                WPARAM(0),
                LPARAM((22. * self.dpi) as isize),
            );
            let tab = 200_i32;
            send(
                list,
                LB_SETTABSTOPS,
                WPARAM(1),
                LPARAM((&tab as *const i32) as isize),
            );
            let composing = track_control_ime(query);
            self.shell.palette = Some(PaletteState {
                host,
                font,
                query,
                list,
                help,
                commands: Vec::new(),
                focus: if focus == query {
                    HWND(GetPropW(query, QUERY_PREVIOUS_FOCUS).0)
                } else {
                    focus
                },
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
            let owner = GetParent(p.host).unwrap_or(self.shell.main_window);
            let mut client = RECT::default();
            let mut query = RECT::default();
            if GetClientRect(owner, &mut client).is_err()
                || GetWindowRect(p.query, &mut query).is_err()
            {
                return;
            }
            let mut origin = POINT {
                x: query.left,
                y: query.bottom,
            };
            let _ = ScreenToClient(owner, &mut origin);
            let width = (query.right - query.left).max(1).min(client.right.max(1));
            let top = origin.y.min((client.bottom - 1).max(0));
            let row_h = send(p.list, LB_GETITEMHEIGHT, WPARAM(0), LPARAM(0))
                .0
                .max(1) as i32;
            let mut host_info = WINDOWINFO {
                cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            if GetWindowInfo(p.host, &mut host_info).is_err() {
                return;
            }
            let border_x = host_info.rcWindow.right
                - host_info.rcWindow.left
                - (host_info.rcClient.right - host_info.rcClient.left);
            let border_y = host_info.rcWindow.bottom
                - host_info.rcWindow.top
                - (host_info.rcClient.bottom - host_info.rcClient.top);
            let help_height = (2. * UI_ROW_HEIGHT * self.dpi).round() as i32;
            let height = (p.commands.len().clamp(1, 8) as i32 * row_h + help_height + border_y)
                .min((client.bottom - top).max(1));
            let _ = SetWindowPos(
                p.host,
                Some(HWND_TOP),
                origin.x.clamp(0, (client.right - width).max(0)),
                top,
                width,
                height,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = position_child_control(
                p.list,
                0,
                0,
                (width - border_x).max(0),
                (height - border_y - help_height).max(0),
            );
            let list_height = (height - border_y - help_height).max(0);
            let inset = (6. * self.dpi).round() as i32;
            let _ = position_child_control(
                p.help,
                inset,
                list_height,
                (width - border_x - 2 * inset).max(0),
                (height - border_y - list_height).max(0),
            );
        }
    }
    pub(super) unsafe fn focus_palette_after_main_activation(&mut self, owner: HWND) -> bool {
        unsafe {
            let Some(p) = &self.shell.palette else {
                return false;
            };
            if (owner != self.shell.main_window && owner != self.shell.compact_window)
                || GetFocus() != owner
                || GetParent(p.host).ok() != Some(owner)
                || GetParent(p.query).ok() != Some(owner)
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
            if commands.is_empty() {
                send(
                    list,
                    LB_ADDSTRING,
                    WPARAM(0),
                    LPARAM(wide("候補なし").as_ptr() as isize),
                );
                send(list, LB_SETCURSEL, WPARAM(usize::MAX), LPARAM(0));
            } else {
                send(list, LB_SETCURSEL, WPARAM(0), LPARAM(0));
            }
            self.shell.palette.as_mut().unwrap().commands = commands;
            self.update_palette_help();
            self.position_palette();
        }
    }
    unsafe fn update_palette_help(&self) {
        unsafe {
            let Some(p) = &self.shell.palette else { return };
            let index = send(p.list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0;
            let text = usize::try_from(index)
                .ok()
                .and_then(|index| p.commands.get(index))
                .map(|command| self.command_help(*command))
                .unwrap_or_default();
            let _ = SetWindowTextW(p.help, PCWSTR(wide(&text).as_ptr()));
        }
    }
    unsafe fn close_palette(&mut self, cancel: bool) {
        unsafe {
            let Some(p) = self.shell.palette.take() else {
                return;
            };
            let _ = SetFocus(Some(self.editing_window()));
            let _ = ShowWindow(p.host, SW_HIDE);
            let _ = SetWindowTextW(p.query, w!(""));
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
                && p.focus != p.query
                && (p.focus == self.shell.main_window
                    || p.focus == self.shell.compact_window
                    || IsChild(self.shell.main_window, p.focus).as_bool()
                    || IsChild(self.shell.compact_window, p.focus).as_bool())
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
                let inside = msg.hwnd == p.query
                    || msg.hwnd == p.host
                    || IsChild(p.host, msg.hwnd).as_bool();
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
            if !self.shell.enabled {
                return false;
            }
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let alt = GetKeyState(VK_MENU.0 as i32) < 0;
            if msg.message == WM_SYSKEYDOWN
                && msg.wParam.0 as u16 == VK_MENU.0
                && !ctrl
                && !shift
                && !self.composing
                && !self.feature_ime_active()
            {
                self.shell.alt_menu_pending = true;
                return true;
            }
            if msg.message == WM_SYSKEYUP
                && msg.wParam.0 as u16 == VK_MENU.0
                && self.shell.alt_menu_pending
            {
                self.shell.alt_menu_pending = false;
                if !self.composing && !self.feature_ime_active() {
                    self.open_shell_menu(None);
                }
                return true;
            }
            if msg.message == WM_SYSKEYDOWN && self.shell.alt_menu_pending {
                self.shell.alt_menu_pending = false;
                let group = menu_mnemonic_group(msg.wParam.0 as u16);
                if let Some(group) = group {
                    if !self.composing && !self.feature_ime_active() {
                        self.open_shell_menu(Some(group));
                    }
                    return true;
                }
            }
            if !matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN) {
                return false;
            }
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
                        self.update_palette_help();
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
                WS_EX_COMPOSITED,
                w!("MDLiteRustEditor"),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
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
            self.update_surface_font(compact, GetDpiForWindow(compact));
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
            self.shell.compact_query = HWND::default();
            self.shell.compact_font = None;
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
            area.left + (area.right - area.left - size as i32) / 2,
            area.top + (area.bottom - area.top - size as i32) / 2,
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
    fn splitter_native_down_precedes_outline_at_painted_border_and_height_edge() {
        use super::super::{
            GetCapture, IDC_SIZENS, IDC_SIZEWE, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
        };
        let mut seed = super::super::tests::composing_test_app("# A\nbody\n# B\nother\n");
        seed.clear_composition();
        let fixture = super::super::tests::MessagePumpFixture::new(seed);
        let position = |x: i32, y: i32| LPARAM(((y as u32) << 16 | x as u32) as isize);
        let source_state = || {
            let app = fixture.state.app.borrow();
            let doc = app.doc();
            (
                doc.source().to_owned(),
                doc.revision(),
                doc.is_dirty(),
                doc.undo_count(),
                doc.redo_count(),
                doc.selection(),
            )
        };
        unsafe {
            SetWindowPos(
                fixture.hwnd,
                None,
                0,
                0,
                1084,
                761,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
            .unwrap();
            {
                let mut app = fixture.state.app.borrow_mut();
                app.start_shell();
                app.features_init();
                app.shell.layout = mdlite::panel::PanelLayout::default();
                app.shell
                    .layout
                    .move_group(PanelId::Outline, mdlite::panel::Side::Right);
                app.shell.layout.join(PanelId::Search, PanelId::Outline);
                app.shell.layout.join(PanelId::Git, PanelId::Outline);
                app.shell.layout.join(PanelId::Calendar, PanelId::Explorer);
                app.shell.layout.select(PanelId::Explorer);
                app.shell.layout.select(PanelId::Outline);
                app.layout_key = None;
                app.ensure_layout().unwrap();
                let body = app.feature_panel_rect(PanelId::Outline).unwrap();
                assert_eq!((body.x, body.y), (844., 66.));
                for (x, splitter) in [
                    (839., false),
                    (840., true),
                    (844., true),
                    (848., true),
                    (849., false),
                ] {
                    assert_eq!(
                        app.surface_cursor_at(x, 81.).as_ptr() == IDC_SIZEWE.as_ptr(),
                        splitter
                    );
                }
            }
            let before = source_state();
            SendMessageW(
                fixture.hwnd,
                WM_LBUTTONDOWN,
                Some(WPARAM(0)),
                Some(position(844, 81)),
            );
            assert_eq!(GetCapture(), fixture.hwnd);
            assert!(
                !fixture
                    .state
                    .app
                    .borrow_mut()
                    .features_finish_outline_drag(true)
            );
            for (x, width) in [(852, 232.), (860, 224.), (868, 216.)] {
                SendMessageW(
                    fixture.hwnd,
                    WM_MOUSEMOVE,
                    Some(WPARAM(0)),
                    Some(position(x, 81)),
                );
                assert_eq!(
                    fixture
                        .state
                        .app
                        .borrow()
                        .shell
                        .layout
                        .group(PanelId::Outline)
                        .width,
                    width
                );
                assert_eq!(source_state(), before);
            }
            SendMessageW(
                fixture.hwnd,
                WM_LBUTTONUP,
                Some(WPARAM(0)),
                Some(position(868, 81)),
            );
            assert_eq!(GetCapture(), HWND::default());
            assert!(
                !fixture
                    .state
                    .app
                    .borrow_mut()
                    .features_finish_outline_drag(true)
            );
            assert_eq!(source_state(), before);

            // Five DIP into Outline is outside splitter slop and must retain
            // its existing feature selection/drag route.
            {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.layout.set_width(PanelId::Outline, 240.);
                app.layout_key = None;
                app.ensure_layout().unwrap();
            }
            SendMessageW(
                fixture.hwnd,
                WM_LBUTTONDOWN,
                Some(WPARAM(0)),
                Some(position(849, 81)),
            );
            assert!(
                fixture
                    .state
                    .app
                    .borrow_mut()
                    .features_finish_outline_drag(true)
            );
            assert_eq!(
                fixture
                    .state
                    .app
                    .borrow()
                    .shell
                    .layout
                    .group(PanelId::Outline)
                    .width,
                240.
            );
            SendMessageW(fixture.hwnd, WM_LBUTTONUP, Some(WPARAM(0)), Some(LPARAM(0)));
            assert!(
                !fixture
                    .state
                    .app
                    .borrow_mut()
                    .features_finish_outline_drag(true)
            );
            assert_eq!(source_state(), before);

            let (x, y, heights) = {
                let mut app = fixture.state.app.borrow_mut();
                app.shell.layout.get_mut(PanelId::Search).collapsed = false;
                app.layout_key = None;
                app.ensure_layout().unwrap();
                let upper = app.shell.panels[PanelId::Outline as usize].1;
                let x = (upper.x + 24.) as i32;
                let y = (upper.y + upper.height).round() as i32 - 1;
                assert_eq!(
                    app.surface_cursor_at(x as f32, y as f32).as_ptr(),
                    IDC_SIZENS.as_ptr()
                );
                (
                    x,
                    y,
                    (
                        app.shell.layout.get(PanelId::Outline).height,
                        app.shell.layout.get(PanelId::Search).height,
                    ),
                )
            };
            SendMessageW(
                fixture.hwnd,
                WM_LBUTTONDOWN,
                Some(WPARAM(0)),
                Some(position(x, y)),
            );
            assert!(
                !fixture
                    .state
                    .app
                    .borrow_mut()
                    .features_finish_outline_drag(true)
            );
            SendMessageW(
                fixture.hwnd,
                WM_MOUSEMOVE,
                Some(WPARAM(0)),
                Some(position(x, y - 24)),
            );
            {
                let app = fixture.state.app.borrow();
                assert!(app.shell.layout.get(PanelId::Outline).height < heights.0);
                assert!(app.shell.layout.get(PanelId::Search).height > heights.1);
            }
            SendMessageW(
                fixture.hwnd,
                WM_LBUTTONUP,
                Some(WPARAM(0)),
                Some(position(x, y - 24)),
            );
            assert_eq!(GetCapture(), HWND::default());
            assert_eq!(source_state(), before);
            assert!(!IsWindowVisible(fixture.hwnd).as_bool());

            let foreign = super::super::tests::hidden_test_window_size(100, 100);
            {
                let mut app = fixture.state.app.borrow_mut();
                app.hwnd = foreign;
                assert!(!app.begin_shell_splitter(844., 81.));
                app.hwnd = fixture.hwnd;
                app.composing = true;
                assert!(!app.begin_shell_splitter(844., 81.));
                app.composing = false;
                app.shell.compact_window = foreign;
                assert!(!app.begin_shell_splitter(844., 81.));
                app.shell.compact_window = HWND::default();
            }
            DestroyWindow(foreign).unwrap();
            assert_eq!(GetCapture(), HWND::default());
            assert_eq!(source_state(), before);
        }
    }

    #[test]
    #[ignore = "visible NOACTIVATE resize fixture; requires exclusive GUI lease"]
    fn shell_continuous_resize_keeps_frame_target_controls_and_source_synchronized() {
        let mut seed = super::super::tests::composing_test_app("# Resize\n\nbody 日本語");
        seed.clear_composition();
        seed.doc_mut().replace_selection("edit ").unwrap();
        let fixture = super::super::tests::MessagePumpFixture::new(seed);
        let foreground = unsafe { GetForegroundWindow() };
        let snapshot = |app: &App| {
            (
                app.doc().source_shared(),
                app.doc().revision(),
                app.doc().selection(),
                app.doc().is_dirty(),
                app.doc().undo_count(),
                app.doc().redo_count(),
            )
        };
        let before = snapshot(&fixture.state.app.borrow());
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.start_shell();
            app.features_init();
            app.git_init();
            app.search_init();
            app.shell.layout.join(PanelId::Calendar, PanelId::Explorer);
            app.shell.layout.move_group(PanelId::Outline, Side::Right);
            app.shell.layout.join(PanelId::Git, PanelId::Outline);
            app.presentation_changed();
            drop(app);
            let _ = ShowWindow(fixture.hwnd, SW_SHOWNOACTIVATE);
            let _ = UpdateWindow(fixture.hwnd);
            for step in 0..12 {
                let width = 980 + step * 11;
                let height = 700 + (step % 4) * 13;
                SetWindowPos(
                    fixture.hwnd,
                    None,
                    40,
                    40,
                    width,
                    height,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .unwrap();
                {
                    let mut app = fixture.state.app.borrow_mut();
                    app.shell
                        .layout
                        .set_width(PanelId::Explorer, 170. + step as f32 * 7.);
                    app.shell
                        .layout
                        .set_width(PanelId::Outline, 210. - step as f32 * 3.);
                    app.shell.layout.get_mut(PanelId::Explorer).height = 2. + step as f32;
                    app.shell.layout.get_mut(PanelId::Calendar).height = 15. - step as f32;
                    app.presentation_changed();
                }
                assert!(GetUpdateRect(fixture.hwnd, None, false).as_bool());
                let _ = UpdateWindow(fixture.hwnd);
                let mut app = fixture.state.app.borrow_mut();
                let mut client = RECT::default();
                GetClientRect(fixture.hwnd, &mut client).unwrap();
                let target = app.target().unwrap();
                let pixels = target.GetPixelSize();
                assert_eq!(
                    [pixels.width, pixels.height],
                    [client.right as u32, client.bottom as u32]
                );
                let size = target.GetSize();
                PAINT_TEST.with(|state| {
                    assert_eq!(
                        state.borrow().damage,
                        Some(rect(0., 0., size.width, size.height))
                    )
                });
                for (index, header) in app.shell.headers.iter().enumerate() {
                    let panel = app.shell.panels[index].1;
                    let (bounds, shown) = child_control_state(*header).unwrap();
                    assert_eq!(shown, panel.width > 0.);
                    assert_eq!(
                        bounds,
                        [
                            ((panel.x + panel.width - 32.) * app.dpi) as i32,
                            (panel.y * app.dpi) as i32,
                            (32. * app.dpi) as i32,
                            (30. * app.dpi) as i32
                        ]
                    );
                }
                assert!(!app.full_paint);
                assert_eq!(app.painted_frame.unwrap().hwnd, fixture.hwnd.0 as usize);
                assert_eq!(snapshot(&app), before);
                assert_eq!(GetForegroundWindow(), foreground);
                println!(
                    "resize step={step} client={}x{} target={}x{} frame={} source_revision={} source_history_unchanged=true",
                    client.right,
                    client.bottom,
                    pixels.width,
                    pixels.height,
                    app.painted_frame.unwrap().sequence,
                    app.doc().revision()
                );
            }
        }
    }

    #[test]
    fn panel_group_activity_and_right_search_are_independent_of_child_focus() {
        let mut app = super::super::tests::composing_test_app("# keep source\n本文😀");
        app.clear_composition();
        app.width = 1440.;
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().selection(),
            app.doc().undo_count(),
            app.doc().is_dirty(),
        );
        app.shell.layout.join(PanelId::Calendar, PanelId::Explorer);
        app.shell.layout.split_to(PanelId::Outline, Side::Right);
        app.shell.layout.join(PanelId::Git, PanelId::Outline);
        let right = app.shell.layout.group(PanelId::Outline).clone();
        let right_active = app.shell.layout.active[1];
        assert!(app.activity_button_visible(0));
        assert!(app.activity_button_visible(1));
        for index in 2..5 {
            assert!(!app.activity_button_visible(index));
        }
        app.focus_feature_panel(PanelId::Calendar);
        assert!(app.activity_button_active(0));
        app.shell.layout.get_mut(PanelId::Calendar).collapsed = true;
        app.shell.layout.select(PanelId::Search);
        assert!(app.activity_button_active(1));
        let (_, search) = shell_geometry(&app.shell.layout, 1440., 900., None);
        assert_eq!(search[PanelId::Explorer as usize].1, Rect::default());
        assert_eq!(search[PanelId::Calendar as usize].1, Rect::default());
        assert!(search[PanelId::Search as usize].1.width > 0.);
        assert_eq!(app.shell.layout.active[1], right_active);
        assert_eq!(app.shell.layout.group(PanelId::Outline), &right);
        app.shell.layout.select(PanelId::Explorer);
        assert!(app.shell.layout.get(PanelId::Calendar).collapsed);
        let (_, restored) = shell_geometry(&app.shell.layout, 1440., 900., None);
        assert_eq!(restored[PanelId::Calendar as usize].1.height, 30.);
        app.shell.layout.split_to(PanelId::Search, Side::Right);
        let (_, simultaneous) = shell_geometry(&app.shell.layout, 1440., 900., None);
        assert!(simultaneous[PanelId::Explorer as usize].1.width > 0.);
        assert!(simultaneous[PanelId::Search as usize].1.width > 0.);
        assert!(!app.activity_button_visible(1));
        assert_eq!(app.shell.layout.active[0], Some(PanelId::Explorer));
        let saved = app.shell.layout.serialize();
        assert_eq!(PanelLayout::parse(&saved).unwrap(), app.shell.layout);
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().selection(),
                app.doc().undo_count(),
                app.doc().is_dirty()
            ),
            before
        );
    }
    #[test]
    fn shell_region_visibility_and_toolbar_bounds_share_available_geometry() {
        let mut layout = PanelLayout::default();
        let panels = layout.panels.clone();
        for width in [360., 480., 760., 1440.] {
            for left in [false, true] {
                for right in [false, true] {
                    layout.left_visible = left;
                    layout.right_visible = right;
                    let (editor, rects) = shell_geometry(&layout, width, 800., None);
                    assert!(editor.width > 0.);
                    for (id, rect) in rects {
                        if !layout.region_visible(id) {
                            assert_eq!(rect, Rect::default());
                        }
                    }
                    for compact in [false, true] {
                        let query = toolbar_query_rect(width, compact);
                        assert!(query.x >= 0. && query.x + query.width <= width);
                        assert!(query.y + query.height <= TOP_BAR_HEIGHT);
                    }
                }
            }
        }
        assert_eq!(layout.panels, panels);
        layout.left_visible = false;
        let (_, rects) = shell_geometry(&layout, 600., 800., Some(PanelId::Explorer));
        assert_eq!(rects[PanelId::Explorer as usize].1, Rect::default());
    }

    #[test]
    fn shell_toolbar_query_is_persistent_and_dropdown_contains_candidates_only() {
        unsafe {
            let window = super::super::tests::hidden_test_window_size(1100, 700);
            let mut app = super::super::tests::composing_test_app("A😀日Z");
            app.clear_composition();
            app.hwnd = window;
            app.shell.enabled = true;
            app.shell.main_window = window;
            app.update_surface_font(window, 96);
            let document_state = |doc: &Document| {
                (
                    doc.source().to_owned(),
                    doc.revision(),
                    doc.is_dirty(),
                    doc.undo_count(),
                    doc.redo_count(),
                    doc.selection(),
                )
            };
            let before = document_state(app.doc());
            let query = app.ensure_toolbar_query(window);
            for dpi in [1., 150. / 96.] {
                app.dpi = dpi;
                app.open_palette();
                let p = app.shell.palette.as_ref().unwrap();
                assert_eq!(p.query, query);
                assert_eq!(GetParent(query).unwrap(), window);
                assert_eq!(GetParent(p.list).unwrap(), p.host);
                // Candidate list may own sibling scrollbar rails through the
                // existing adapter; it must never own another query or heading.
                assert_eq!(
                    FindWindowExW(Some(window), None, w!("EDIT"), None).unwrap(),
                    query
                );
                assert!(FindWindowExW(Some(window), Some(query), w!("EDIT"), None).is_err());
                assert!(FindWindowExW(Some(p.host), None, w!("EDIT"), None).is_err());
                assert_eq!(
                    FindWindowExW(Some(p.host), None, w!("STATIC"), None).unwrap(),
                    p.help
                );
                assert_eq!(GetDlgCtrlID(p.help), PALETTE_HELP as i32);
                assert_eq!(text_of(p.help), app.command_help(p.commands[0]));
                let mut query_bounds = RECT::default();
                let mut host_bounds = RECT::default();
                let mut list_bounds = RECT::default();
                let mut help_bounds = RECT::default();
                GetWindowRect(query, &mut query_bounds).unwrap();
                GetWindowRect(p.host, &mut host_bounds).unwrap();
                GetWindowRect(p.list, &mut list_bounds).unwrap();
                GetWindowRect(p.help, &mut help_bounds).unwrap();
                assert_eq!(host_bounds.left, query_bounds.left);
                assert_eq!(host_bounds.top, query_bounds.bottom);
                assert_eq!(host_bounds.right, query_bounds.right);
                assert_eq!(help_bounds.top, list_bounds.bottom);
                assert!(help_bounds.left >= host_bounds.left);
                assert!(help_bounds.right <= host_bounds.right);
                assert!(help_bounds.bottom <= host_bounds.bottom);
                app.close_palette(true);
                assert!(IsWindow(Some(query)).as_bool());
                assert_eq!(app.ensure_toolbar_query(window), query);
                assert_eq!(text_of(query), "");
            }
            assert_eq!(document_state(app.doc()), before);
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn shell_native_edit_factory_eligibility_tokens_and_shared_rail_lifecycle() {
        unsafe {
            let owner = super::super::tests::hidden_test_window_size(800, 600);
            let mut font = control_style::UiFont::new(96, 14.).unwrap();
            font.apply_to(owner).unwrap();
            let text = "行 漢字e\u{301}😀\r\n".repeat(100);
            let styles = [
                WS_BORDER | WS_VSCROLL | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
                WS_BORDER
                    | WS_VSCROLL
                    | WS_HSCROLL
                    | WINDOW_STYLE(
                        (ES_MULTILINE | ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_READONLY) as u32,
                    ),
                WS_BORDER | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                WS_BORDER | WINDOW_STYLE(ES_MULTILINE as u32),
            ];
            let mut owned_rails = Vec::new();
            for (index, style) in styles.into_iter().enumerate() {
                let edit = control(owner, w!("EDIT"), &text, 9800 + index, style);
                assert_ne!(edit, HWND::default());
                assert_eq!(GetParent(edit).unwrap(), owner);
                assert_eq!(GetDlgCtrlID(edit), 9800 + index as i32);
                assert_eq!(
                    send(edit, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                    font.handle().0 as isize
                );
                assert_eq!(text_of(edit), text);
                let actual = GetWindowLongW(edit, GWL_STYLE) as u32;
                assert_eq!(
                    actual & (ES_MULTILINE as u32 | ES_AUTOHSCROLL as u32 | ES_READONLY as u32),
                    style.0 & (ES_MULTILINE as u32 | ES_AUTOHSCROLL as u32 | ES_READONLY as u32)
                );
                let attached = super::super::edit_scroll::refresh(edit, Tokens::default()).unwrap();
                assert_eq!(attached, index < 2);
                if index < 2 {
                    let rails = super::super::edit_scroll::rail_handles(edit).unwrap();
                    for rail in rails {
                        assert!(super::super::scrollbar_style::is_rail(rail));
                        assert_eq!(GetParent(rail).unwrap(), owner);
                        owned_rails.push(rail);
                    }
                }
            }
            assert!(!super::super::edit_scroll::refresh(owner, Tokens::default()).unwrap());
            let tokens = Tokens {
                high_contrast: true,
                ..Default::default()
            };
            refresh_native_scrollbars(owner, tokens).unwrap();
            assert!(
                owned_rails
                    .iter()
                    .all(|rail| GetWindowLongW(*rail, GWL_STYLE) as u32 & WS_VISIBLE.0 == 0)
            );
            refresh_native_scrollbars(owner, Tokens::default()).unwrap();
            DestroyWindow(owner).unwrap();
            assert!(
                owned_rails
                    .into_iter()
                    .all(|rail| !IsWindow(Some(rail)).as_bool())
            );
        }
    }

    #[test]
    fn shell_native_font_surfaces_and_cursor_regions_preserve_document() {
        let window = super::super::tests::hidden_test_window_size(1280, 800);
        let mut app = super::super::tests::composing_test_app("# 見出し\n\n本文\n");
        app.clear_composition();
        app.hwnd = window;
        app.shell.enabled = true;
        app.shell.main_window = window;
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().selection(),
            app.doc().undo_count(),
            app.doc().is_dirty(),
        );
        app.update_surface_font(window, 96);
        unsafe {
            let child = control(
                window,
                w!("EDIT"),
                "検索語",
                987,
                WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            );
            assert_eq!(
                send(child, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                app.shell.main_font.as_ref().unwrap().handle().0 as isize
            );
            app.ensure_layout().unwrap();
            let x = app.content_left() + 20.;
            let y = app.editor_top() + 10.;
            assert_eq!(app.surface_cursor_at(x, y).as_ptr(), IDC_IBEAM.as_ptr());
            assert_eq!(
                app.surface_cursor_at(app.editor_left() + 1., y).as_ptr(),
                IDC_ARROW.as_ptr()
            );
            assert_eq!(
                app.surface_cursor_at(app.editor_left() + 20., TAB_LABEL_TOP)
                    .as_ptr(),
                IDC_HAND.as_ptr()
            );
            let panel = app.shell.panels.iter().find(|(_, r)| r.width > 0.).unwrap();
            let left = app.shell.layout.side(panel.0) == Side::Left;
            assert_eq!(
                app.surface_cursor_at(
                    if left {
                        panel.1.x + panel.1.width
                    } else {
                        panel.1.x
                    },
                    panel.1.y + 40.
                )
                .as_ptr(),
                IDC_SIZEWE.as_ptr()
            );
            app.open_palette();
            assert!(app.shell.palette.as_ref().unwrap().font.is_some());
            app.update_surface_font(window, 150);
            let main = app.shell.main_font.as_ref().unwrap().handle();
            assert_eq!(
                send(child, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                main.0 as isize
            );
            let palette = app.shell.palette.as_ref().unwrap();
            let own = palette.font.as_ref().unwrap().handle();
            assert_ne!(main, own);
            assert_eq!(
                send(palette.query, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                main.0 as isize
            );
            assert_eq!(
                send(palette.list, WM_GETFONT, WPARAM(0), LPARAM(0)).0,
                own.0 as isize
            );
            let mut data = LOGFONTW::default();
            assert_ne!(
                GetObjectW(
                    own.into(),
                    std::mem::size_of::<LOGFONTW>() as i32,
                    Some((&mut data as *mut LOGFONTW).cast())
                ),
                0
            );
            assert_eq!(data.lfHeight, -(UI_FONT_SIZE * 150. / 96.).round() as i32);
            app.shell.palette.take();
            DestroyWindow(window).unwrap();
        }
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().selection(),
                app.doc().undo_count(),
                app.doc().is_dirty()
            ),
            before
        );
    }

    #[test]
    fn shell_geometry_narrow_drawer_and_explicit_move_remain_reachable() {
        let mut layout = PanelLayout::default();
        for width in [320., 480., 760., 1100., 1440.] {
            let (editor, panels) = shell_geometry(&layout, width, 800., None);
            assert!(editor.width > 0. && editor.x >= 48. && editor.x + editor.width <= width);
            assert!(editor.y >= 76. && editor.y + editor.height <= 800.);
            if width < 760. {
                assert!(panels.iter().all(|(_, r)| r.width == 0.));
                for id in IDS {
                    layout.select(id);
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
        assert_eq!(panels[PanelId::Outline as usize].1, Rect::default());
    }

    #[test]
    fn narrow_drawer_selection_survives_hiding_and_restoring_each_region() {
        let mut app = super::super::tests::composing_test_app("# keep source\nbody");
        app.width = 600.;
        app.shell.layout.select(PanelId::Search);
        app.shell.layout.split_to(PanelId::Git, Side::Right);
        let source = app.doc().source().to_owned();
        let revision = app.doc().revision();
        let selection = app.doc().selection();
        let slots = app.shell.layout.panels.clone();

        for (left, id) in [(true, PanelId::Search), (false, PanelId::Git)] {
            app.shell.active_panel = id;
            app.shell.drawer = Some(id);
            toggle_region_visibility(&mut app.shell.layout, left);
            assert!(!app.shell.layout.region_visible(id));
            let (_, hidden) = shell_geometry(&app.shell.layout, 600., 800., app.shell.drawer);
            assert_eq!(hidden[id as usize].1, Rect::default());

            toggle_region_visibility(&mut app.shell.layout, left);
            assert!(app.shell.layout.region_visible(id));
            let (_, restored) = shell_geometry(&app.shell.layout, 600., 800., app.shell.drawer);
            assert!(restored[id as usize].1.width >= 200.);
            assert_eq!(app.shell.drawer, Some(id));
            assert_eq!(app.shell.active_panel, id);
            assert_eq!(app.shell.layout.active[0], Some(PanelId::Search));
            assert_eq!(app.shell.layout.panels, slots);
            assert_eq!(app.doc().source(), source);
            assert_eq!(app.doc().revision(), revision);
            assert_eq!(app.doc().selection(), selection);
        }
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

        assert_eq!(app.shell.editor.y, EDITOR_TOP);
        assert_eq!(app.shell.editor.y + app.shell.editor.height, 480. - 24.);
        assert!(
            app.shell.editor.y >= TAB_STRIP_TOP + TAB_STRIP_HEIGHT,
            "compact editor starts below the compact tab strip"
        );
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
        app.shell.layout.set_width(PanelId::Explorer, 310.);
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
    fn alt_menu_mnemonics_leave_function_and_numpad_keys_native() {
        for (key, group) in [
            (0x46, 0),
            (0x56, 1),
            (0x57, 2),
            (0x53, 3),
            (0x47, 4),
            (0x45, 5),
        ] {
            assert_eq!(menu_mnemonic_group(key), Some(group));
        }
        for key in (VK_F1.0..=VK_F12.0).chain(VK_NUMPAD0.0..=VK_NUMPAD9.0) {
            assert_eq!(menu_mnemonic_group(key), None, "virtual key {key:x}");
        }
    }
    #[test]
    fn palette_keyboard_selection_explains_disabled_commands_before_execution() {
        let window = super::super::tests::hidden_test_window_size(900, 600);
        let mut app = super::super::tests::composing_test_app("# heading\ntext\n");
        app.hwnd = window;
        app.clear_composition();
        app.shell.enabled = true;
        app.shell.main_window = window;
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().undo_count(),
        );
        unsafe {
            app.open_palette();
            let query = app.shell.palette.as_ref().unwrap().query;
            SetWindowTextW(query, w!("ソース")).unwrap();
            app.filter_palette();
            assert_eq!(
                app.shell.palette.as_ref().unwrap().commands,
                vec![Command::Source, Command::Editor(38)]
            );
            let help = app.shell.palette.as_ref().unwrap().help;
            assert_eq!(text_of(help), app.command_help(Command::Source));
            assert!(app.shell_message(&MSG {
                hwnd: query,
                message: WM_KEYDOWN,
                wParam: WPARAM(VK_DOWN.0 as usize),
                ..Default::default()
            }));
            assert_eq!(
                text_of(help),
                app.command_disabled_reason(Command::Editor(38))
            );
            assert!(!app.command_enabled(Command::Editor(38)));
            assert!(app.shell.palette.is_some());
            assert_eq!(
                (
                    app.doc().source().to_owned(),
                    app.doc().revision(),
                    app.doc().undo_count()
                ),
                before
            );
            app.shell.palette.take();
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn close_last_clean_tab_keeps_valid_navigation_state() {
        let mut app = super::super::tests::composing_test_app("clean\n");
        let window = super::super::tests::hidden_test_window_size(900, 600);
        app.hwnd = window;
        app.clear_composition();
        let previous = app.doc().id();
        assert!(!app.doc().is_dirty());
        unsafe {
            app.command(CMD_CLOSE);
        }
        assert_eq!(app.tabs.len(), 1);
        assert_ne!(app.doc().id(), previous);
        assert_eq!(app.doc().source(), "");
        assert!(!app.command_enabled(Command::Navigate(false)));
        assert!(!app.command_enabled(Command::Navigate(true)));
        app.tabs.clear();
        app.active = 0;
        app.new_document();
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.doc().source(), "");
        unsafe {
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn new_document_records_origin_and_replaces_forward_branch() {
        let mut app = super::super::tests::composing_test_app("one😀two\nlast\n");
        let window = super::super::tests::hidden_test_window_size(900, 600);
        app.hwnd = window;
        app.clear_composition();
        let origin = app.doc().id();
        app.doc_mut().set_selection(0, 0).unwrap();
        app.record_navigation();
        app.doc_mut().set_selection(7, 7).unwrap();
        app.navigate_position(false);
        assert!(app.command_enabled(Command::Navigate(true)));
        app.new_document();
        assert_ne!(app.doc().id(), origin);
        assert!(!app.command_enabled(Command::Navigate(true)));
        app.navigate_position(false);
        assert_eq!(app.doc().id(), origin);
        assert_eq!(app.doc().selection(), Selection::caret(0));
        unsafe {
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn navigation_positions_branch_without_editing_source_or_undo() {
        let mut app = super::super::tests::composing_test_app("one😀two\nlast\n");
        let window = super::super::tests::hidden_test_window_size(900, 600);
        app.hwnd = window;
        app.clear_composition();
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().undo_count(),
            app.doc().redo_count(),
            app.doc().is_dirty(),
        );
        app.doc_mut().set_selection(0, 0).unwrap();
        app.record_navigation();
        app.doc_mut().set_selection(7, 7).unwrap();
        app.record_navigation();
        app.doc_mut().set_selection(10, 10).unwrap();
        app.navigate_position(false);
        assert_eq!(app.doc().selection(), Selection::caret(7));
        app.navigate_position(false);
        assert_eq!(app.doc().selection(), Selection::caret(0));
        app.navigate_position(true);
        assert_eq!(app.doc().selection(), Selection::caret(7));
        app.record_navigation();
        app.doc_mut().set_selection(11, 11).unwrap();
        assert!(!app.command_enabled(Command::Navigate(true)));
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().undo_count(),
                app.doc().redo_count(),
                app.doc().is_dirty()
            ),
            before
        );
        unsafe {
            DestroyWindow(window).unwrap();
        }
    }
    #[test]
    fn toolbar_query_hit_area_excludes_actions_and_centered_margins() {
        for width in [360., 480., 900., 1600.] {
            let query = toolbar_query_rect(width, false);
            assert!(query.contains(query.x + query.width / 2., 12.));
            assert!(!query.contains(20., 12.));
            assert!(!query.contains(width - 30., 12.));
            assert!(!query.contains(query.x - 1., 12.));
            assert!(!query.contains(query.x + query.width + 1., 12.));
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
    fn shell_owned_button_erase_keeps_private_pixels_and_rejects_foreign_identity() {
        let mut seed = super::super::tests::composing_test_app("button source");
        seed.clear_composition();
        let fixture = super::super::tests::MessagePumpFixture::new(seed);
        unsafe {
            fixture.state.app.borrow_mut().start_shell();
            let source = GetDC(Some(fixture.hwnd));
            let dc = CreateCompatibleDC(Some(source));
            let bitmap = CreateCompatibleBitmap(source, 32, 32);
            ReleaseDC(Some(fixture.hwnd), source);
            let old = SelectObject(dc, bitmap.into());
            let sentinel = COLORREF(0x0012_8931);
            SetPixel(dc, 4, 4, sentinel);
            let button = GetDlgItem(Some(fixture.hwnd), SHELL_MENU as i32).unwrap();
            assert!(valid_shell_erase_button(button, fixture.hwnd, SHELL_MENU));
            assert_eq!(
                SendMessageW(
                    button,
                    WM_ERASEBKGND,
                    Some(WPARAM(dc.0 as usize)),
                    Some(LPARAM(0))
                ),
                LRESULT(1)
            );
            assert_eq!(
                GetPixel(dc, 4, 4),
                sentinel,
                "standalone erase cannot clear a registered full-rect owner button"
            );
            assert!(!valid_shell_erase_button(
                button,
                HWND::default(),
                SHELL_MENU
            ));
            assert!(!valid_shell_erase_button(
                button,
                fixture.hwnd,
                SHELL_MENU + 1
            ));
            let foreign = {
                // Native EDIT initialization can synchronously notify ID107.
                // Match real startup's borrow boundary so this negative paint
                // fixture cannot execute a menu command while it is created.
                let _app = fixture.state.app.borrow_mut();
                control(fixture.hwnd, w!("EDIT"), "", SHELL_MENU, WINDOW_STYLE(0))
            };
            assert!(!valid_shell_erase_button(foreign, fixture.hwnd, SHELL_MENU));
            assert!(!IsWindowVisible(fixture.hwnd).as_bool());
            SelectObject(dc, old);
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(dc);
        }
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
    fn shell_staged_theme_keeps_new_binding_in_rebuilt_native_menu() {
        let mut seed = super::super::tests::composing_test_app("unchanged 日本語");
        seed.clear_composition();
        let fixture = super::super::tests::MessagePumpFixture::new(seed);
        unsafe {
            let mut app = fixture.state.app.borrow_mut();
            app.start_shell();
            let old = fixture.state.control_colors.borrow().clone().unwrap();
            let previous = app.shell.settings.clone();
            app.defer_theme_updates = true;
            let mut next = previous.clone();
            next.theme = mdlite::settings::ThemeMode::Light;
            next.keybindings
                .insert("file.save".into(), "Ctrl+Alt+S".into());
            app.apply_shell_settings(next);
            assert_ne!(
                app.shell.settings, previous,
                "completion must notice changed metadata"
            );
            app.install_shell_menu();
            let menu = app.shell.menu_routes[&CMD_SAVE];
            let mut label = [0u16; 128];
            let length = GetMenuStringW(menu, CMD_SAVE as u32, Some(&mut label), MF_BYCOMMAND);
            assert_eq!(
                String::from_utf16_lossy(&label[..length as usize]),
                "保存\tCtrl+Alt+S"
            );
            assert!(Rc::ptr_eq(
                &old,
                fixture.state.control_colors.borrow().as_ref().unwrap()
            ));
            assert_eq!(
                GetMenu(fixture.hwnd),
                HMENU::default(),
                "popup resources do not create a native menu bar"
            );
        }
    }
    #[test]
    fn shell_palette_single_candidate_keeps_a_full_native_list_row() {
        let window = super::super::tests::hidden_test_window_size(1100, 793);
        let mut app = super::super::tests::composing_test_app("unchanged 本文");
        app.clear_composition();
        app.hwnd = window;
        app.shell.enabled = true;
        app.shell.main_window = window;
        let document_state = |doc: &Document| {
            (
                doc.source().to_owned(),
                doc.revision(),
                doc.is_dirty(),
                doc.undo_count(),
                doc.redo_count(),
                doc.selection(),
            )
        };
        let before = document_state(app.doc());
        unsafe {
            for dpi in [1.0, 1.5625] {
                app.dpi = dpi;
                app.open_palette();
                let query = app.shell.palette.as_ref().unwrap().query;
                let query_text = wide("設定");
                send(
                    query,
                    WM_SETTEXT,
                    WPARAM(0),
                    LPARAM(query_text.as_ptr() as isize),
                );
                app.filter_palette();
                let palette = app.shell.palette.as_ref().unwrap();
                assert_eq!(palette.commands.len(), 1);
                let mut client = RECT::default();
                GetClientRect(palette.list, &mut client).unwrap();
                let item_height =
                    send(palette.list, LB_GETITEMHEIGHT, WPARAM(0), LPARAM(0)).0 as i32;
                assert!(
                    client.bottom >= item_height,
                    "dpi={dpi}: native list client {} must fit item {item_height}",
                    client.bottom
                );
                app.close_palette(true);
            }
            DestroyWindow(window).unwrap();
        }
        assert_eq!(document_state(app.doc()), before);
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
                assert_eq!(GetParent(p.query).unwrap(), window);
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
                assert!(IsWindow(Some(query)).as_bool());
                for child in [host, list] {
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
            assert!(IsWindow(Some(query)).as_bool());
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
            // A claimed button ID cannot turn the Main HWND into an owned child.
            assert_eq!(
                app.shell_window_message(window, WM_DRAWITEM, WPARAM(PALETTE), parameter),
                None
            );
            app.start_shell();
            item.CtlID = SHELL_MENU as u32;
            item.hwndItem = app.shell.menu_button;
            let parameter = LPARAM((&item as *const DRAWITEMSTRUCT) as isize);
            assert_eq!(
                app.shell_window_message(window, WM_DRAWITEM, WPARAM(SHELL_MENU), parameter),
                Some(LRESULT(1))
            );
            ReleaseDC(Some(window), dc);
            DestroyWindow(window).unwrap();
        }
    }
}
