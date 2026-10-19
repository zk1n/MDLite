//! Native Explorer, Quick Open, Outline, links, Calendar, and profile surfaces.
//!
//! This module owns pane state and child controls. It never owns document text
//! and all source edits go through Document transactions on the UI thread.

use super::{App, explorer_icons::ExplorerPaint, shell};
use mdlite::{
    calendar::{
        CalendarCell, CalendarDate, CalendarInteraction, CalendarListRequest, CalendarListResult,
        CalendarListState, CalendarMonthGrid, HolidayCalendar, HolidayStatus, daily_file_marker,
        decorate_month, month_grid,
    },
    explorer_tree::{DEFAULT_TREE_LIMIT, list_expanded},
    navigation::{LinkTarget, Outline, link_target},
    panel::{PanelId, Rect},
    profiles::{
        LoadedProfileTemplate, ProfileCollision, ProfileDateTime, ProfileDefinition, ProfileError,
        ProfileFileSnapshot, ProfileInputDefinition, ProfileValues, add_or_replace_profile,
        create_profile_note, default_profiles, duplicate_profile, initialize_default_templates,
        load_profile_layer, load_profile_template, preview_profile_path, remove_profile,
        resolve_profile_layers, save_profile_layer, save_profile_template,
    },
    workspace::{ExplorerEntry, ExplorerEntryKind, Workspace, WorkspaceError, WorkspaceIdentity},
    workspace_search::{QuickOpen, quick_open},
};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    fs,
    path::{Component, Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Instant, SystemTime},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::{Direct2D::*, DirectWrite::DWRITE_WORD_WRAPPING_NO_WRAP, Gdi::MapWindowPoints},
        System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree},
        UI::{
            Controls::{
                DRAWITEMSTRUCT, ODT_BUTTON, TOOLTIPS_CLASSW, TTF_IDISHWND, TTF_SUBCLASS,
                TTM_ADDTOOLW, TTS_ALWAYSTIP, TTTOOLINFOW,
            },
            Input::KeyboardAndMouse::{
                GetFocus, SetFocus, VK_DOWN, VK_ESCAPE, VK_LEFT, VK_NEXT, VK_PRIOR, VK_RETURN,
                VK_RIGHT, VK_UP,
            },
            Shell::{
                DefSubclassProc, FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI,
                FOF_SILENT, FOS_FORCEFILESYSTEM, FOS_NOCHANGEDIR, FOS_PICKFOLDERS, FileOpenDialog,
                IFileOpenDialog, SHFILEOPSTRUCTW, SHFileOperationW, SIGDN_FILESYSPATH,
                SetWindowSubclass, ShellExecuteW,
            },
            WindowsAndMessaging::*,
        },
    },
    core::{PCWSTR, PWSTR, w},
};

const QUERY_EDIT: usize = 4700;
const PROMPT_EDIT: usize = 4701;
const PROFILE_FIELD_BASE: usize = 4710;
const TEMPLATE_EDIT: usize = 4720;
pub(super) const EXPLORER_LIST: usize = 4690;
const QUICK_OPEN_LIST: usize = 4691;
const CONTROL_BASE: usize = 4800;
const BUTTON_OWNERDRAW: u32 = 0x0000000B;
const BUTTON_NOTIFY: u32 = 0x00000400;
const CALENDAR_GRID_TOP: f32 = 98.0;
const CALENDAR_COMPACT_GRID_TOP: f32 = 72.0;
const CALENDAR_COMPACT_HEIGHT: f32 = 273.0;
const CALENDAR_MIN_BODY_HEIGHT: f32 = 180.0;
const CALENDAR_WEEK_STEP: f32 = 23.0;
const CALENDAR_LIST_GAP: f32 = 6.0;
const CALENDAR_STATUS_HEIGHT: f32 = 17.0;
const CALENDAR_ROW_HEIGHT: f32 = 18.0;
const CALENDAR_CELL_INSET: f32 = 2.0;
const CALENDAR_DAY_FONT_SIZE: f32 = 9.0;
const PROFILE_BUTTON_FOOTER: f32 = 34.0;
const PROFILE_LIST_TOP: f32 = 26.0;
const OUTLINE_ROW_HEIGHT: f32 = 22.0;
const KEY_ESCAPE: u16 = VK_ESCAPE.0;
const KEY_RETURN: u16 = VK_RETURN.0;
const KEY_UP: u16 = VK_UP.0;
const KEY_DOWN: u16 = VK_DOWN.0;
const KEY_LEFT: u16 = VK_LEFT.0;
const KEY_RIGHT: u16 = VK_RIGHT.0;
const KEY_PAGE_UP: u16 = VK_PRIOR.0;
const KEY_PAGE_DOWN: u16 = VK_NEXT.0;

pub(super) const COMMANDS: &[(usize, &str, Option<&'static str>)] = &[
    (4000, "クイックオープン…", Some("file.quickOpen")),
    (4001, "新しいファイル", None),
    (4002, "新しいフォルダー", None),
    (4003, "選択項目をコピー", None),
    (4004, "コピーした項目を貼り付け", None),
    (4005, "名前を変更", None),
    (4006, "ごみ箱へ移動", None),
    (4007, "パスをコピー", None),
    (4008, "エクスプローラーで表示", None),
    (4009, "親フォルダーへ移動", None),
    (4010, "選択項目を開く", None),
    (4011, "別のWorkspaceを開く…", None),
    (4020, "前の月", None),
    (4021, "次の月", None),
    (4022, "今日を選択", None),
    (4023, "デイリーノートを開く / 作成", None),
    (4024, "ミーティングノートを作成", None),
    (4025, "メモを作成", None),
    (4026, "祝日CSVを読み込む…", None),
    (4027, "プロファイル管理", None),
    (4028, "今日のDairyを開く / 作成", None),
    (4090, "カレンダー操作…", None),
    (4040, "見出しへ移動", None),
    (4041, "セクションを上へ", None),
    (4042, "セクションを下へ", None),
    (4080, "プロファイルを追加", None),
    (4081, "プロファイルを編集", None),
    (4082, "プロファイルを複製", None),
    (4083, "Workspaceのプロファイルを削除", None),
    (4084, "テンプレートを編集", None),
    (4085, "プロファイルを保存", None),
    (4086, "キャンセル", None),
    (4087, "テンプレートを保存", None),
    (4088, "選択したプロファイルで作成", None),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FormMode {
    NewProfile,
    EditProfile,
    DuplicateProfile,
    CreateProfileInputs,
    EditTemplate,
    NewFile,
    NewFolder,
    Rename,
}

#[derive(Clone, Debug)]
struct FormState {
    mode: FormMode,
    title: String,
    profile_id: Option<String>,
    date_time: Option<ProfileDateTime>,
    template_snapshot: Option<mdlite::profiles::ProfileTemplateSnapshot>,
    profile_snapshot: Option<ProfileFileSnapshot>,
    original_path: Option<PathBuf>,
    guard_document: u64,
    guard_revision: u64,
}

#[derive(Clone, Debug)]
struct ExplorerState {
    directory: PathBuf,
    entries: Vec<ExplorerEntry>,
    depths: Vec<usize>,
    expanded_rows: Vec<bool>,
    expanded: BTreeSet<PathBuf>,
    complete: bool,
    issues: Vec<String>,
    selected: Vec<PathBuf>,
    drag_source: Option<PathBuf>,
    drag_target: Option<PathBuf>,
    internal_copy: Vec<PathBuf>,
}

impl Default for ExplorerState {
    fn default() -> Self {
        Self {
            directory: PathBuf::new(),
            entries: Vec::new(),
            depths: Vec::new(),
            expanded_rows: Vec::new(),
            expanded: BTreeSet::new(),
            complete: true,
            issues: Vec::new(),
            selected: Vec::new(),
            drag_source: None,
            drag_target: None,
            internal_copy: Vec::new(),
        }
    }
}

struct QuickOpenWorker {
    generation: u64,
    query: String,
    identity: WorkspaceIdentity,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<QuickOpen, String>>,
    handle: JoinHandle<()>,
}

struct QuickOpenState {
    open: bool,
    query: String,
    generation: u64,
    selected: usize,
    result: Option<QuickOpen>,
    worker: Option<QuickOpenWorker>,
    saved_focus: HWND,
    saved_document: u64,
    saved_revision: u64,
    saved_selection: mdlite::document::Selection,
    saved_affinity: mdlite::coordinates::BoundaryAffinity,
}

impl Default for QuickOpenState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            generation: 0,
            selected: 0,
            result: None,
            worker: None,
            saved_focus: HWND::default(),
            saved_document: 0,
            saved_revision: 0,
            saved_selection: mdlite::document::Selection { anchor: 0, head: 0 },
            saved_affinity: mdlite::coordinates::BoundaryAffinity::After,
        }
    }
}

struct CalendarWorker {
    request: CalendarListRequest,
    cancel: Arc<AtomicBool>,
    receiver: Receiver<CalendarListResult>,
    handle: JoinHandle<()>,
}

struct CalendarPane {
    interaction: CalendarInteraction,
    listing: Option<CalendarListResult>,
    request: Option<CalendarListRequest>,
    list_generation: u64,
    worker: Option<CalendarWorker>,
    retired: Vec<JoinHandle<()>>,
    holidays: HolidayCalendar,
    holiday_loaded: bool,
    daily_markers: BTreeSet<CalendarDate>,
    selected_file: usize,
    file_focused: bool,
    click_target: Option<CalendarClickTarget>,
    scroll: usize,
    current_month: Option<CalendarMonthGrid>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CalendarClickTarget {
    Date(CalendarDate),
    File(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CalendarLayout {
    grid_top: f32,
    week_step: f32,
    cell_height: f32,
    list_top: f32,
    rows_top: f32,
    status_height: f32,
    visible_rows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct CalendarCellDrawBounds {
    cell: Rect,
    label: Rect,
}

fn calendar_cell_draw_bounds(
    body: Rect,
    layout: CalendarLayout,
    index: usize,
) -> Option<CalendarCellDrawBounds> {
    if index >= 42 || body.width <= 0.0 || body.height <= 0.0 {
        return None;
    }
    let cell_width = ((body.width - 10.0) / 7.0).max(1.0);
    let column = index % 7;
    let row = index / 7;
    let raw_cell = Rect {
        x: body.x + 6.0 + column as f32 * cell_width,
        y: body.y + layout.grid_top + row as f32 * layout.week_step,
        width: (cell_width - 2.0).max(0.0),
        height: layout.cell_height.max(0.0),
    };
    let cell = intersect_rect(raw_cell, body)?;
    let label = intersect_rect(
        Rect {
            x: cell.x + 1.0,
            y: cell.y + 1.0,
            width: (cell.width - 2.0).max(0.0),
            height: (cell.height - 2.0).max(0.0),
        },
        cell,
    )?;
    Some(CalendarCellDrawBounds { cell, label })
}

fn intersect_rect(a: Rect, b: Rect) -> Option<Rect> {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > left && bottom > top).then_some(Rect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
}

fn calendar_day_label(day: i32, daily_file: bool, holiday: bool) -> String {
    format!(
        "{day}{}{}",
        if daily_file { "•" } else { "" },
        if holiday { "祝" } else { "" }
    )
}

fn calendar_cell_color(tokens: shell::Tokens, cell: &CalendarCell) -> super::D2D1_COLOR_F {
    if tokens.high_contrast && (cell.marks.selected || cell.marks.focused) {
        let rgb = unsafe {
            windows::Win32::Graphics::Gdi::GetSysColor(
                windows::Win32::Graphics::Gdi::COLOR_HIGHLIGHTTEXT,
            )
        };
        return super::color(
            (rgb & 255) as f32 / 255.,
            ((rgb >> 8) & 255) as f32 / 255.,
            ((rgb >> 16) & 255) as f32 / 255.,
        );
    }
    let ink = if cell.marks.holiday || cell.marks.sunday {
        tokens.weekend_red
    } else if cell.marks.saturday {
        tokens.weekend_blue
    } else {
        tokens.foreground
    };
    if cell.in_displayed_month || cell.marks.selected || cell.marks.focused || tokens.high_contrast
    {
        ink
    } else {
        // Dim adjacent months without replacing the weekend hue with grey.
        super::color(
            ink.r * 0.7 + tokens.panel.r * 0.3,
            ink.g * 0.7 + tokens.panel.g * 0.3,
            ink.b * 0.7 + tokens.panel.b * 0.3,
        )
    }
}

fn calendar_day_font_size(label: &str, width: f32) -> f32 {
    let units = label
        .chars()
        .map(|character| {
            if character.is_ascii_digit() {
                0.55
            } else {
                1.0
            }
        })
        .sum::<f32>()
        .max(1.0);
    (width.max(0.0) / units).clamp(1.0, CALENDAR_DAY_FONT_SIZE)
}

fn calendar_layout(body_height: f32) -> CalendarLayout {
    let body_height = body_height.max(0.0);
    let compact = body_height < CALENDAR_COMPACT_HEIGHT;
    let grid_top = if compact {
        CALENDAR_COMPACT_GRID_TOP
    } else {
        CALENDAR_GRID_TOP
    };
    let full_grid_height = 6.0 * CALENDAR_WEEK_STEP;
    let status_height = if body_height
        >= grid_top
            + full_grid_height
            + CALENDAR_LIST_GAP
            + CALENDAR_STATUS_HEIGHT
            + CALENDAR_ROW_HEIGHT
    {
        CALENDAR_STATUS_HEIGHT
    } else {
        0.0
    };
    let reserved = CALENDAR_LIST_GAP + status_height + CALENDAR_ROW_HEIGHT;
    let week_step =
        ((body_height - grid_top - reserved).max(0.0) / 6.0).clamp(14.0, CALENDAR_WEEK_STEP);
    let grid_height = 6.0 * week_step;
    let list_top = grid_top + grid_height + CALENDAR_LIST_GAP;
    let rows_top = list_top + status_height;
    let visible_rows = ((body_height - rows_top).max(0.0) / CALENDAR_ROW_HEIGHT).floor() as usize;
    CalendarLayout {
        grid_top,
        week_step,
        cell_height: (week_step - CALENDAR_CELL_INSET).max(0.0),
        list_top,
        rows_top,
        status_height,
        visible_rows,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ProfileListLayout {
    rows_top: f32,
    visible_rows: usize,
}

fn profile_list_layout(body_height: f32) -> ProfileListLayout {
    let rows_top = PROFILE_LIST_TOP;
    let visible_rows = ((body_height - PROFILE_BUTTON_FOOTER - rows_top).max(0.0)
        / CALENDAR_ROW_HEIGHT)
        .floor() as usize;
    ProfileListLayout {
        rows_top,
        visible_rows,
    }
}

fn calendar_form_active(form: Option<&FormState>) -> bool {
    form.is_some_and(|form| {
        matches!(
            form.mode,
            FormMode::NewProfile
                | FormMode::EditProfile
                | FormMode::DuplicateProfile
                | FormMode::CreateProfileInputs
                | FormMode::EditTemplate
        )
    })
}

fn outline_visible_rows(body_height: f32) -> usize {
    ((body_height - 8.0).max(0.0) / OUTLINE_ROW_HEIGHT).floor() as usize
}

fn outline_scroll_for_selection(
    scroll: usize,
    selected: usize,
    count: usize,
    visible: usize,
) -> usize {
    if count == 0 || visible == 0 {
        return 0;
    }
    let scroll = if selected < scroll {
        selected
    } else if selected >= scroll.saturating_add(visible) {
        selected + 1 - visible
    } else {
        scroll
    };
    scroll.min(count.saturating_sub(visible))
}

fn scroll_list(current: usize, count: usize, visible: usize, delta: f32) -> usize {
    if count <= visible || !delta.is_finite() || delta == 0.0 {
        return current.min(count.saturating_sub(visible));
    }
    let rows = (delta.abs() * 3.0).ceil().max(1.0) as usize;
    let max = count.saturating_sub(visible);
    if delta > 0.0 {
        current.saturating_sub(rows)
    } else {
        current.saturating_add(rows).min(max)
    }
}

impl Default for CalendarPane {
    fn default() -> Self {
        Self {
            interaction: CalendarInteraction::new(local_now().date),
            listing: None,
            request: None,
            list_generation: 0,
            worker: None,
            retired: Vec::new(),
            holidays: HolidayCalendar::default(),
            holiday_loaded: false,
            daily_markers: BTreeSet::new(),
            selected_file: 0,
            file_focused: false,
            click_target: None,
            scroll: 0,
            current_month: None,
        }
    }
}

#[derive(Default)]
struct ProfilePane {
    layer: Vec<ProfileDefinition>,
    snapshot: Option<ProfileFileSnapshot>,
    effective: Vec<ProfileDefinition>,
    selected: usize,
    common_error: Option<String>,
    workspace_error: Option<String>,
    profile_template: Option<LoadedProfileTemplate>,
    manager_open: bool,
}

struct FeatureControls {
    owner: HWND,
    query: HWND,
    query_ime: Box<Cell<bool>>,
    prompt: HWND,
    prompt_ime: Box<Cell<bool>>,
    profile_fields: Vec<HWND>,
    profile_labels: Vec<HWND>,
    // Subclass callbacks hold pointers to these cells; vector growth must not move them.
    #[allow(clippy::vec_box)]
    profile_ime: Vec<Box<Cell<bool>>>,
    template: HWND,
    template_ime: Box<Cell<bool>>,
    explorer_list: HWND,
    quick_open_list: HWND,
    quick_open_list_key: Option<(u64, bool)>,
    buttons: Vec<(usize, usize, HWND, PanelId)>,
    tooltip: HWND,
    tooltip_text: Vec<Vec<u16>>,
}

impl Default for FeatureControls {
    fn default() -> Self {
        Self {
            owner: HWND::default(),
            query: HWND::default(),
            query_ime: Box::new(Cell::new(false)),
            prompt: HWND::default(),
            prompt_ime: Box::new(Cell::new(false)),
            profile_fields: Vec::new(),
            profile_labels: Vec::new(),
            profile_ime: Vec::new(),
            template: HWND::default(),
            template_ime: Box::new(Cell::new(false)),
            explorer_list: HWND::default(),
            quick_open_list: HWND::default(),
            quick_open_list_key: None,
            buttons: Vec::new(),
            tooltip: HWND::default(),
            tooltip_text: Vec::new(),
        }
    }
}

pub(super) type ExplorerPaintSlot = Rc<RefCell<Option<Rc<ExplorerPaint>>>>;

#[derive(Clone, Copy, PartialEq, Eq)]
struct ExplorerPaintKey {
    hwnd: HWND,
    font: isize,
    dpi: u32,
    colors: [u32; 3],
    icons: u64,
}

pub(super) struct FeaturePanes {
    initialized: bool,
    workspace: Option<Workspace>,
    workspace_identity: Option<WorkspaceIdentity>,
    explorer: ExplorerState,
    explorer_list_dirty: bool,
    explorer_paint: ExplorerPaintSlot,
    explorer_paint_key: Option<ExplorerPaintKey>,
    quick_open: QuickOpenState,
    outline: Option<Outline>,
    outline_selected: usize,
    outline_scroll: usize,
    outline_document: u64,
    outline_revision: u64,
    outline_drag: Option<(Outline, usize, f32, f32, bool, Option<usize>)>,
    calendar: CalendarPane,
    profiles: ProfilePane,
    form: Option<FormState>,
    prompt_text: String,
    profile_form_text: Vec<String>,
    template_text: String,
    active_panel: PanelId,
    controls: FeatureControls,
    pending_path_copy: Option<Vec<PathBuf>>,
    pending_open_file: Option<PathBuf>,
    pending_daily: Option<CalendarDate>,
    pending_move_to: Option<PathBuf>,
    pending_focus_restore: bool,
    status: Option<String>,
    retired_workers: Vec<JoinHandle<()>>,
}

impl Default for FeaturePanes {
    fn default() -> Self {
        Self {
            initialized: false,
            workspace: None,
            workspace_identity: None,
            explorer: ExplorerState::default(),
            explorer_list_dirty: true,
            explorer_paint: Rc::new(RefCell::new(None)),
            explorer_paint_key: None,
            quick_open: QuickOpenState::default(),
            outline: None,
            outline_selected: 0,
            outline_scroll: 0,
            outline_document: 0,
            outline_revision: 0,
            outline_drag: None,
            calendar: CalendarPane::default(),
            profiles: ProfilePane::default(),
            form: None,
            prompt_text: String::new(),
            profile_form_text: vec![String::new(); 7],
            template_text: String::new(),
            active_panel: PanelId::Explorer,
            controls: FeatureControls::default(),
            pending_path_copy: None,
            pending_open_file: None,
            pending_daily: None,
            pending_move_to: None,
            pending_focus_restore: false,
            status: None,
            retired_workers: Vec::new(),
        }
    }
}

impl Drop for FeaturePanes {
    fn drop(&mut self) {
        let retired = self.explorer_paint.replace(None);
        drop(retired);
        if let Some(worker) = self.quick_open.worker.take() {
            worker.cancel.store(true, Ordering::Release);
        }
        if let Some(worker) = self.calendar.worker.take() {
            worker.cancel.store(true, Ordering::Release);
        }
        for worker in &self.retired_workers {
            let _ = worker.is_finished();
        }
        unsafe {
            for (_, _, hwnd, _) in &self.controls.buttons {
                if IsWindow(Some(*hwnd)).as_bool() {
                    let _ = DestroyWindow(*hwnd);
                }
            }
            for hwnd in &self.controls.profile_fields {
                if IsWindow(Some(*hwnd)).as_bool() {
                    let _ = DestroyWindow(*hwnd);
                }
            }
            for hwnd in &self.controls.profile_labels {
                if IsWindow(Some(*hwnd)).as_bool() {
                    let _ = DestroyWindow(*hwnd);
                }
            }
            for hwnd in [
                self.controls.query,
                self.controls.prompt,
                self.controls.template,
                self.controls.explorer_list,
                self.controls.quick_open_list,
                self.controls.tooltip,
            ] {
                if IsWindow(Some(hwnd)).as_bool() {
                    let _ = DestroyWindow(hwnd);
                }
            }
        }
    }
}

const PROFILE_LABEL_BASE: usize = 4730;
const PROFILE_FIELDS: [&str; 7] = [
    "識別子",
    "表示名",
    "保存先のパターン",
    "ファイル名のパターン",
    "テンプレートのパス",
    "重複時の動作",
    "入力: id|名前|required|初期値",
];
const EXPLORER_BUTTONS: [usize; 3] = [4001, 4002, 4011];
const CALENDAR_BUTTONS: [usize; 8] = [4020, 4021, 4022, 4023, 4024, 4025, 4026, 4027];
const CALENDAR_OPERATIONS_BUTTON: usize = 4090;
const PROFILE_BUTTONS: [usize; 6] = [4080, 4081, 4082, 4083, 4084, 4088];
const FORM_BUTTONS: [usize; 3] = [4085, 4086, 4087];

impl FeaturePanes {
    pub(super) fn explorer_paint_handle(&self) -> ExplorerPaintSlot {
        Rc::clone(&self.explorer_paint)
    }
    pub(super) unsafe fn owner_draw_button(
        &self,
        owner: HWND,
        message_id: usize,
        item: &DRAWITEMSTRUCT,
    ) -> Option<shell::OwnerDrawState> {
        if self.controls.owner != owner
            || item.CtlType != ODT_BUTTON
            || item.CtlID as usize != message_id
        {
            return None;
        }
        let (control_id, command, hwnd, _) =
            self.controls
                .buttons
                .iter()
                .find(|(control_id, _, hwnd, _)| {
                    *control_id == item.CtlID as usize && *hwnd == item.hwndItem
                })?;
        if *hwnd == HWND::default()
            || unsafe { GetParent(*hwnd).unwrap_or_default() } != owner
            || unsafe { GetDlgCtrlID(*hwnd) as u32 } != *control_id as u32
        {
            return None;
        }
        let (_, label, _) = COMMANDS.iter().find(|(id, _, _)| id == command)?;
        Some(unsafe { shell::owner_draw_state(localized_label(*command, label), item) })
    }

    unsafe fn create_controls(&mut self, owner: HWND, dpi: f32) {
        unsafe {
            self.controls.owner = owner;
            self.controls.query = shell::control(
                owner,
                w!("EDIT"),
                "",
                QUERY_EDIT,
                WINDOW_STYLE(0x0080 | 0x0008),
            );
            self.controls.query_ime = shell::track_control_ime(self.controls.query);
            self.controls.prompt = shell::control(
                owner,
                w!("EDIT"),
                "",
                PROMPT_EDIT,
                WINDOW_STYLE(0x0080 | 0x0008),
            );
            self.controls.prompt_ime = shell::track_control_ime(self.controls.prompt);
            for (index, label) in PROFILE_FIELDS.iter().enumerate() {
                let label_hwnd = shell::control(
                    owner,
                    w!("STATIC"),
                    label,
                    PROFILE_LABEL_BASE + index,
                    WINDOW_STYLE(0),
                );
                let edit = shell::control(
                    owner,
                    w!("EDIT"),
                    "",
                    PROFILE_FIELD_BASE + index,
                    WINDOW_STYLE(if index == 6 {
                        0x0004 | 0x0040 | 0x1000 | 0x0080 | 0x0008 | 0x0020_0000
                    } else {
                        0x0080 | 0x0008
                    }),
                );
                self.controls.profile_labels.push(label_hwnd);
                self.controls.profile_fields.push(edit);
                self.controls
                    .profile_ime
                    .push(shell::track_control_ime(edit));
            }
            self.controls.template = shell::control(
                owner,
                w!("EDIT"),
                "",
                TEMPLATE_EDIT,
                WINDOW_STYLE(0x0004 | 0x0040 | 0x1000 | 0x0080 | 0x0008 | 0x0020_0000),
            );
            self.controls.template_ime = shell::track_control_ime(self.controls.template);
            self.controls.explorer_list = shell::control(
                owner,
                w!("LISTBOX"),
                "Workspaceの項目",
                EXPLORER_LIST,
                WINDOW_STYLE(0x0001 | 0x0010 | 0x0040 | 0x0100 | 0x0800),
            );
            let _ = shell::send(
                self.controls.explorer_list,
                LB_SETITEMHEIGHT,
                WPARAM(0),
                LPARAM((20.0 * dpi).round().max(1.0) as isize),
            );
            let _ = SetWindowSubclass(
                self.controls.explorer_list,
                Some(explorer_list_proc),
                EXPLORER_LIST,
                0,
            );
            self.controls.quick_open_list = shell::control(
                owner,
                w!("LISTBOX"),
                "クイックオープン候補",
                QUICK_OPEN_LIST,
                WINDOW_STYLE(0x0001 | 0x0100 | 0x0040 | 0x0020_0000 | 0x0080_0000),
            );

            let mut commands = Vec::new();
            commands.extend(EXPLORER_BUTTONS.map(|command| (command, PanelId::Explorer)));
            commands.extend(
                CALENDAR_BUTTONS
                    .into_iter()
                    .filter(|command| matches!(command, 4020..=4022))
                    .map(|command| (command, PanelId::Calendar)),
            );
            commands.push((CALENDAR_OPERATIONS_BUTTON, PanelId::Calendar));
            commands.extend(PROFILE_BUTTONS.map(|command| (command, PanelId::Calendar)));
            commands.extend(FORM_BUTTONS.map(|command| (command, PanelId::Calendar)));
            for (index, (command, panel)) in commands.into_iter().enumerate() {
                let Some((_, label, _)) = COMMANDS.iter().find(|(id, _, _)| *id == command) else {
                    continue;
                };
                let control_id = CONTROL_BASE + index;
                let hwnd = shell::control(
                    owner,
                    w!("BUTTON"),
                    &localized_label(command, label),
                    control_id,
                    WINDOW_STYLE(BUTTON_OWNERDRAW | BUTTON_NOTIFY),
                );
                self.controls
                    .buttons
                    .push((control_id, command, hwnd, panel));
            }
            self.controls.tooltip = CreateWindowExW(
                WS_EX_TOPMOST,
                TOOLTIPS_CLASSW,
                None,
                WS_POPUP | WINDOW_STYLE(TTS_ALWAYSTIP),
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(owner),
                None,
                None,
                None,
            )
            .unwrap_or_default();
            if self.controls.tooltip != HWND::default() {
                let _ = super::control_style::inherit_font(self.controls.tooltip, owner);
                for (_, command, hwnd, _) in &self.controls.buttons {
                    let Some((_, label, _)) = COMMANDS.iter().find(|(id, _, _)| id == command)
                    else {
                        continue;
                    };
                    let mut text = super::wide(label);
                    let text_ptr = PWSTR(text.as_mut_ptr());
                    self.controls.tooltip_text.push(text);
                    let mut tool = TTTOOLINFOW {
                        cbSize: std::mem::size_of::<TTTOOLINFOW>() as u32,
                        uFlags: TTF_IDISHWND | TTF_SUBCLASS,
                        hwnd: owner,
                        uId: hwnd.0 as usize,
                        lpszText: text_ptr,
                        ..Default::default()
                    };
                    let _ = SendMessageW(
                        self.controls.tooltip,
                        TTM_ADDTOOLW,
                        Some(WPARAM(0)),
                        Some(LPARAM((&mut tool as *mut TTTOOLINFOW) as isize)),
                    );
                }
            }
            for hwnd in [
                self.controls.query,
                self.controls.prompt,
                self.controls.template,
                self.controls.explorer_list,
                self.controls.quick_open_list,
            ]
            .into_iter()
            .chain(self.controls.profile_fields.iter().copied())
            .chain(self.controls.profile_labels.iter().copied())
            .chain(self.controls.buttons.iter().map(|button| button.2))
            {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }

    fn position_controls(&self, app: &App, layout: FeatureControlLayout) {
        unsafe {
            let position = |hwnd: HWND, rect: Rect| {
                let _ = shell::position_child_control(
                    hwnd,
                    (rect.x * layout.dpi) as i32,
                    (rect.y * layout.dpi) as i32,
                    (rect.width.max(0.) * layout.dpi) as i32,
                    (rect.height.max(0.) * layout.dpi) as i32,
                );
            };
            let show = |hwnd: HWND, visible: bool| {
                let _ = shell::show_child_control(hwnd, visible);
            };

            let explorer = layout.explorer.unwrap_or_default();
            let query_visible = self.quick_open.open;
            let editor = layout.editor;
            position(
                self.controls.query,
                Rect {
                    x: editor.x + 12.,
                    y: editor.y + 8.,
                    width: (editor.width - 24.).max(0.),
                    height: 26.,
                },
            );
            show(self.controls.query, query_visible && editor.width > 0.);
            position(
                self.controls.quick_open_list,
                Rect {
                    x: editor.x + 12.,
                    y: editor.y + 36.,
                    width: (editor.width - 24.).max(0.),
                    height: (editor.height * 0.55)
                        .clamp(96., 280.)
                        .min((editor.height - 64.).max(0.)),
                },
            );
            show(
                self.controls.quick_open_list,
                query_visible && editor.width > 0.,
            );
            position(
                self.controls.explorer_list,
                Rect {
                    x: explorer.x + 2.,
                    y: explorer.y + 32.,
                    width: (explorer.width - 4.).max(0.),
                    height: (explorer.height - 36.).max(0.),
                },
            );
            let _ = shell::send(
                self.controls.explorer_list,
                LB_SETITEMHEIGHT,
                WPARAM(0),
                LPARAM((20.0 * layout.dpi).round().max(1.0) as isize),
            );
            show(
                self.controls.explorer_list,
                !self.quick_open.open
                    && !self.form.as_ref().is_some_and(|form| {
                        matches!(
                            form.mode,
                            FormMode::NewFile | FormMode::NewFolder | FormMode::Rename
                        )
                    })
                    && explorer.width > 0.,
            );

            let prompt_mode = self.form.as_ref().map(|form| form.mode);
            let prompt_visible = matches!(
                prompt_mode,
                Some(
                    FormMode::NewFile
                        | FormMode::NewFolder
                        | FormMode::Rename
                        | FormMode::CreateProfileInputs
                )
            );
            let prompt_panel = if matches!(
                prompt_mode,
                Some(FormMode::NewFile | FormMode::NewFolder | FormMode::Rename)
            ) {
                explorer
            } else {
                layout.calendar.unwrap_or_default()
            };
            position(
                self.controls.prompt,
                Rect {
                    x: prompt_panel.x + 8.,
                    y: prompt_panel.y + 38.,
                    width: (prompt_panel.width - 16.).max(0.),
                    height: 26.,
                },
            );
            show(
                self.controls.prompt,
                prompt_visible && prompt_panel.width > 0.,
            );

            let profile_form = matches!(
                prompt_mode,
                Some(FormMode::NewProfile | FormMode::EditProfile | FormMode::DuplicateProfile)
            );
            let input_form = prompt_mode == Some(FormMode::CreateProfileInputs);
            let form_panel = layout.calendar.unwrap_or_default();
            for index in 0..self.controls.profile_fields.len() {
                let visible = profile_form || (input_form && index == 6);
                let (label_rect, field_rect) = if input_form && index == 6 {
                    let label = Rect {
                        x: form_panel.x + 6.,
                        y: form_panel.y + 24.,
                        width: (form_panel.width - 12.).max(0.),
                        height: 18.,
                    };
                    let input = profile_inputs_rect(form_panel);
                    (label, input)
                } else {
                    let field = profile_form_field_rect(form_panel, index);
                    (
                        Rect {
                            x: form_panel.x + 6.,
                            y: field.y,
                            width: 92.,
                            height: field.height,
                        },
                        Rect {
                            x: form_panel.x + 100.,
                            y: field.y,
                            width: (form_panel.width - 106.).max(0.),
                            height: field.height,
                        },
                    )
                };
                position(self.controls.profile_labels[index], label_rect);
                position(self.controls.profile_fields[index], field_rect);
                show(
                    self.controls.profile_labels[index],
                    visible && form_panel.width > 0.,
                );
                show(
                    self.controls.profile_fields[index],
                    visible && form_panel.width > 0.,
                );
            }

            let template_visible = prompt_mode == Some(FormMode::EditTemplate);
            position(
                self.controls.template,
                Rect {
                    x: form_panel.x + 6.,
                    y: form_panel.y + 34.,
                    width: (form_panel.width - 12.).max(0.),
                    height: (form_panel.height - 82.).max(0.),
                },
            );
            show(
                self.controls.template,
                template_visible && form_panel.width > 0.,
            );

            let toolbar = self.toolbar_layout(&layout);
            for (control_id, command, hwnd, panel) in &self.controls.buttons {
                let rect = toolbar.get(control_id).copied().unwrap_or_default();
                position(*hwnd, rect);
                let visible = rect.width > 0. && self.button_visible(*command, *panel, prompt_mode);
                show(*hwnd, visible);
                let enabled = !app.background_mutation_busy() && app.features_enabled(*command);
                shell::enable_owner_button(
                    shell::OwnerButtonPaint {
                        recipient: self.controls.owner,
                        parent: self.controls.owner,
                        parent_id: None,
                        child: *hwnd,
                        id: *control_id,
                        tokens: app.shell.tokens,
                        label: COMMANDS
                            .iter()
                            .find(|(id, _, _)| id == command)
                            .map(|(_, label, _)| localized_label(*command, label))
                            .unwrap_or_else(|| shell::text_of(*hwnd)),
                    },
                    enabled,
                );
            }
        }
    }

    fn toolbar_layout(&self, layout: &FeatureControlLayout) -> HashMap<usize, Rect> {
        let mut positions = HashMap::new();
        for (panel, commands, rect) in [
            (
                PanelId::Explorer,
                &EXPLORER_BUTTONS[..],
                layout.explorer.unwrap_or_default(),
            ),
            (
                PanelId::Calendar,
                &CALENDAR_BUTTONS[..],
                layout.calendar.unwrap_or_default(),
            ),
        ] {
            let compact_calendar = panel == PanelId::Calendar
                && rect.height < CALENDAR_COMPACT_HEIGHT
                && layout.form.is_none()
                && !self.profiles.manager_open;
            let commands: Vec<_> = if compact_calendar {
                vec![CALENDAR_OPERATIONS_BUTTON]
            } else {
                commands
                    .iter()
                    .copied()
                    .filter(|command| self.button_visible(*command, panel, layout.form))
                    .collect()
            };
            let columns = if compact_calendar {
                1
            } else if panel == PanelId::Explorer {
                commands.len().clamp(1, 2)
            } else {
                4
            };
            for (index, command) in commands.iter().enumerate() {
                if let Some((control_id, _, _, _)) = self
                    .controls
                    .buttons
                    .iter()
                    .find(|button| button.1 == *command)
                {
                    if compact_calendar {
                        let width = (rect.width - 10.0).clamp(0.0, 96.0);
                        if width >= 64.0 {
                            positions.insert(
                                *control_id,
                                Rect {
                                    x: rect.x + 5.0,
                                    y: rect.y + 3.0,
                                    width,
                                    height: 24.0,
                                },
                            );
                        }
                        continue;
                    }
                    let row = index / columns;
                    let column = index % columns;
                    let width = (rect.width - 10. - (columns as f32 - 1.) * 2.) / columns as f32;
                    positions.insert(
                        *control_id,
                        Rect {
                            x: rect.x + 5. + column as f32 * (width + 2.),
                            y: rect.y + 3. + row as f32 * 26.,
                            width: width.max(0.),
                            height: 24.,
                        },
                    );
                }
            }
        }

        let form = self.form.as_ref();
        let calendar = layout.calendar.unwrap_or_default();
        let form_commands: &[usize] = match form.map(|form| form.mode) {
            Some(FormMode::EditTemplate) => &[4087, 4086],
            Some(FormMode::NewProfile | FormMode::EditProfile | FormMode::DuplicateProfile) => {
                &[4085, 4086]
            }
            Some(FormMode::CreateProfileInputs) => &[4088, 4086],
            _ if self.profiles.manager_open => &PROFILE_BUTTONS,
            _ => &[],
        };
        for (index, command) in form_commands.iter().enumerate() {
            if let Some((control_id, _, _, _)) = self
                .controls
                .buttons
                .iter()
                .find(|button| button.1 == *command)
            {
                let width = (calendar.width - 12.) / form_commands.len().max(1) as f32;
                positions.insert(
                    *control_id,
                    Rect {
                        x: calendar.x + 6. + index as f32 * width,
                        y: calendar.y + calendar.height - 34.,
                        width: (width - 2.).max(0.),
                        height: 26.,
                    },
                );
            }
        }
        positions
    }

    fn button_visible(&self, command: usize, panel: PanelId, form: Option<FormMode>) -> bool {
        if panel == PanelId::Explorer {
            return if self.workspace.is_some() {
                matches!(command, 4001 | 4002)
            } else {
                command == 4011
            };
        }
        if panel == PanelId::Outline {
            return false;
        }
        if panel != PanelId::Calendar {
            return false;
        }
        if let Some(mode) = form {
            return match mode {
                FormMode::EditTemplate => matches!(command, 4086 | 4087),
                FormMode::NewProfile | FormMode::EditProfile | FormMode::DuplicateProfile => {
                    matches!(command, 4085 | 4086)
                }
                FormMode::CreateProfileInputs => matches!(command, 4086 | 4088),
                _ => false,
            };
        }
        if self.profiles.manager_open {
            return matches!(command, 4080..=4084 | 4088);
        }
        matches!(command, 4020..=4022 | CALENDAR_OPERATIONS_BUTTON)
    }

    fn synchronize(&mut self, snapshot: FeatureSnapshot) {
        let identity_changed = match (&self.workspace_identity, &snapshot.workspace) {
            (None, None) => false,
            (Some(old), Some(new)) => !old.same_workspace(new.identity()),
            _ => true,
        };
        if identity_changed {
            self.cancel_workers();
            self.explorer_list_dirty = true;
            self.workspace = snapshot.workspace.clone();
            self.workspace_identity = snapshot
                .workspace
                .as_ref()
                .map(|workspace| workspace.identity().clone());
            self.explorer = ExplorerState::default();
            self.quick_open = QuickOpenState::default();
            self.calendar = CalendarPane::default();
            self.profiles = ProfilePane::default();
            self.form = None;
            self.pending_path_copy = None;
            if let Some(workspace) = self.workspace.clone() {
                self.load_profile_definitions(&workspace);
                self.load_holiday_cache(&workspace);
                let _ = self.refresh_explorer();
                self.calendar
                    .interaction
                    .set_selection(Some(snapshot.today.date));
            }
        }
        self.active_panel = snapshot.active_panel;
        if self.outline_document != snapshot.document_id
            || self.outline_revision != snapshot.revision
        {
            self.outline_document = snapshot.document_id;
            self.outline_revision = snapshot.revision;
            if let Some(outline) = snapshot.outline {
                self.outline = Some(outline);
                self.outline_selected = self
                    .outline
                    .as_ref()
                    .map_or(0, |outline| outline.headings.len().saturating_sub(1));
                self.outline_scroll = 0;
            }
        }
        if snapshot.today.date != self.calendar.interaction.today() {
            self.calendar.interaction.set_today(snapshot.today.date);
        }
        let month = self.calendar.interaction.displayed_month();
        if self
            .calendar
            .current_month
            .is_none_or(|grid| grid.displayed_month != month)
        {
            self.calendar.current_month = month_grid(month);
            self.refresh_daily_markers();
        }
        if let Some(selected) = self.calendar.interaction.selected() {
            let needs_list = self.calendar.listing.as_ref().is_none_or(|result| {
                result.date != selected
                    || self
                        .workspace_identity
                        .as_ref()
                        .is_none_or(|identity| !result.workspace_identity.same_workspace(identity))
            });
            if needs_list {
                self.start_calendar_list(selected);
            }
        }
    }

    fn ensure_outline_selection_visible(&mut self, visible_rows: usize) {
        let count = self
            .outline
            .as_ref()
            .map_or(0, |outline| outline.headings.len());
        self.outline_scroll = outline_scroll_for_selection(
            self.outline_scroll,
            self.outline_selected,
            count,
            visible_rows,
        );
    }

    fn refresh_explorer(&mut self) -> Result<(), WorkspaceError> {
        let workspace = self
            .workspace
            .as_ref()
            .ok_or_else(|| WorkspaceError::InvalidPath(PathBuf::from("no workspace selected")))?;
        let directory = if self.explorer.directory.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            self.explorer.directory.clone()
        };
        let listing = list_expanded(
            workspace,
            &directory,
            &self.explorer.expanded,
            DEFAULT_TREE_LIMIT,
        )?;
        self.explorer.directory = directory;
        self.explorer.entries = listing.rows.iter().map(|row| row.entry.clone()).collect();
        self.explorer.depths = listing.rows.iter().map(|row| row.depth).collect();
        self.explorer.expanded_rows = listing.rows.iter().map(|row| row.expanded).collect();
        self.explorer.complete = listing.complete;
        self.explorer_list_dirty = true;
        self.explorer.issues = listing
            .issues
            .into_iter()
            .map(|issue| format!("{}: {}", issue.path.display(), issue.message))
            .collect();
        if !self.explorer.complete && self.explorer.issues.is_empty() {
            self.explorer
                .issues
                .push("Explorerのツリー表示上限に達しました".to_owned());
        }
        if let Some(issue) = self.explorer.issues.first() {
            self.status = Some(format!(
                "エクスプローラー: {} 件を表示できません — {issue}",
                self.explorer.issues.len()
            ));
        } else if self
            .status
            .as_deref()
            .is_some_and(|status| status.starts_with("エクスプローラー:"))
        {
            self.status = None;
        }
        Ok(())
    }

    fn load_profile_definitions(&mut self, workspace: &Workspace) {
        let workspace_path = workspace.root().join(".mdlite/profiles.toml");
        let common_path = common_profiles_path();
        let common = common_path.as_ref().map(load_profile_layer).transpose();
        let layer = load_profile_layer(&workspace_path);
        match (common, layer) {
            (Ok(common), Ok(layer)) => {
                let common_profiles = common.map_or_else(Vec::new, |loaded| loaded.profiles);
                self.profiles.common_error = None;
                self.profiles.workspace_error = None;
                self.profiles.snapshot = Some(layer.snapshot.clone());
                self.profiles.layer = layer.profiles;
                match resolve_profile_layers(&common_profiles, &self.profiles.layer) {
                    Ok(effective) => self.profiles.effective = effective,
                    Err(error) => {
                        self.profiles.workspace_error = Some(error.to_string());
                        self.profiles.effective = default_profiles();
                    }
                }
            }
            (Err(error), _) => {
                self.profiles.common_error = Some(error.to_string());
                self.profiles.effective = default_profiles();
            }
            (_, Err(error)) => {
                self.profiles.workspace_error = Some(error.to_string());
                self.profiles.effective = default_profiles();
            }
        }
        self.calendar.list_generation = self.calendar.list_generation.wrapping_add(1);
        self.calendar.listing = None;
        self.calendar.click_target = None;
        self.profiles.selected = self
            .profiles
            .selected
            .min(self.profiles.effective.len().saturating_sub(1));
    }

    fn load_holiday_cache(&mut self, workspace: &Workspace) {
        let relative = Path::new(".mdlite/.cache/holidays.csv");
        if let Ok(path) = checked_workspace_path(workspace, relative, true)
            && path.is_file()
            && let Err(error) = self.calendar.holidays.import_csv_file(&path, None)
        {
            self.status = Some(format!("祝日cacheを保持しました: {error}"));
        }
        self.calendar.holiday_loaded = true;
    }

    fn refresh_daily_markers(&mut self) {
        self.calendar.daily_markers.clear();
        let (Some(workspace), Some(grid)) = (&self.workspace, self.calendar.current_month) else {
            return;
        };
        let Some(daily) = self
            .profiles
            .effective
            .iter()
            .find(|profile| profile.id == "daily")
        else {
            return;
        };
        for date in grid.cells.into_iter().flatten() {
            if daily_file_marker(workspace, daily, date).unwrap_or(false) {
                self.calendar.daily_markers.insert(date);
            }
        }
    }

    fn start_calendar_list(&mut self, date: CalendarDate) {
        self.calendar.file_focused = false;
        self.calendar.scroll = 0;
        self.calendar.selected_file = 0;
        self.calendar.listing = None;
        self.calendar.request = None;
        let Some(workspace) = self.workspace.clone() else {
            return;
        };
        if let Some(worker) = self.calendar.worker.take() {
            worker.cancel.store(true, Ordering::Release);
            self.calendar.retired.push(worker.handle);
        }
        self.calendar.list_generation = self.calendar.list_generation.wrapping_add(1);
        let mut request =
            match CalendarListRequest::new(&workspace, self.calendar.list_generation, date) {
                Ok(request) => request,
                Err(error) => {
                    self.status = Some(error.to_string());
                    return;
                }
            };
        request.daily_profile = self
            .profiles
            .effective
            .iter()
            .find(|profile| profile.id == "daily")
            .cloned();
        self.calendar.request = Some(request.clone());
        self.calendar.listing = Some(request.loading_result());
        let worker_request = request.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            let result = worker_request.execute(|| worker_cancel.load(Ordering::Acquire));
            let _ = sender.send(result);
        });
        self.calendar.worker = Some(CalendarWorker {
            request,
            cancel,
            receiver,
            handle,
        });
    }

    fn start_quick_open(&mut self, query: String, recent: Vec<PathBuf>) {
        let Some(workspace) = self.workspace.clone() else {
            self.quick_open.result = None;
            return;
        };
        if let Some(worker) = self.quick_open.worker.take() {
            worker.cancel.store(true, Ordering::Release);
            self.retired_workers.push(worker.handle);
        }
        self.quick_open.generation = self.quick_open.generation.wrapping_add(1);
        self.quick_open.query = query.clone();
        self.quick_open.selected = 0;
        self.quick_open.result = None;
        let generation = self.quick_open.generation;
        let identity = workspace.identity().clone();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();
        let worker_query = query.clone();
        let handle = thread::spawn(move || {
            let result = quick_open(&workspace, &worker_query, &recent, &worker_cancel);
            let _ = sender.send(result);
        });
        self.quick_open.worker = Some(QuickOpenWorker {
            generation,
            query,
            identity,
            cancel,
            receiver,
            handle,
        });
    }

    fn poll_workers(&mut self) -> Vec<FeatureEvent> {
        let mut events = Vec::new();
        let quick_ready =
            self.quick_open
                .worker
                .as_ref()
                .and_then(|worker| match worker.receiver.try_recv() {
                    Ok(result) => Some(Some(result)),
                    Err(TryRecvError::Disconnected) => Some(None),
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(quick_ready) = quick_ready
            && let Some(worker) = self.quick_open.worker.take()
        {
            if let Some(result) = quick_ready {
                let matches = self.workspace_identity.as_ref().is_some_and(|identity| {
                    identity.same_workspace(&worker.identity)
                        && worker.generation == self.quick_open.generation
                        && worker.query == self.quick_open.query
                });
                if matches {
                    match result {
                        Ok(result) => {
                            self.quick_open.selected = self
                                .quick_open
                                .selected
                                .min(result.paths.len().saturating_sub(1));
                            self.quick_open.result = Some(result);
                            events.push(FeatureEvent::QuickOpen(Ok(())));
                        }
                        Err(error) => {
                            self.quick_open.result = None;
                            events.push(FeatureEvent::QuickOpen(Err(error)));
                        }
                    }
                }
            }
            let _ = worker.handle.join();
        }
        let calendar_ready =
            self.calendar
                .worker
                .as_ref()
                .and_then(|worker| match worker.receiver.try_recv() {
                    Ok(result) => Some(Some(result)),
                    Err(TryRecvError::Disconnected) => Some(None),
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(calendar_ready) = calendar_ready
            && let Some(worker) = self.calendar.worker.take()
        {
            if let Some(result) = calendar_ready {
                let matches = self.workspace.as_ref().is_some_and(|workspace| {
                    worker.request.matches(
                        workspace,
                        self.calendar.list_generation,
                        worker.request.date,
                    ) && worker.request.daily_profile.as_ref()
                        == self
                            .profiles
                            .effective
                            .iter()
                            .find(|profile| profile.id == "daily")
                        && result.matches(
                            workspace,
                            self.calendar.list_generation,
                            worker.request.date,
                        )
                        && self.calendar.interaction.selected() == Some(worker.request.date)
                        && self.workspace_identity.as_ref().is_some_and(|identity| {
                            identity.same_workspace(&result.workspace_identity)
                        })
                });
                if matches {
                    self.calendar.selected_file = 0;
                    self.calendar.listing = Some(result.clone());
                    events.push(FeatureEvent::Calendar(result));
                }
            }
            let _ = worker.handle.join();
        }
        reap_workers(&mut self.retired_workers);
        reap_workers(&mut self.calendar.retired);
        events
    }

    fn cancel_workers(&mut self) {
        if let Some(worker) = self.quick_open.worker.take() {
            worker.cancel.store(true, Ordering::Release);
            self.retired_workers.push(worker.handle);
        }
        if let Some(worker) = self.calendar.worker.take() {
            worker.cancel.store(true, Ordering::Release);
            self.calendar.retired.push(worker.handle);
        }
    }

    fn draw(&self, app: &App, target: &ID2D1HwndRenderTarget) -> windows::core::Result<()> {
        if !app.shell.enabled || app.hwnd != app.shell.main_window {
            return Ok(());
        }
        let tokens = app.shell.tokens;
        let foreground = unsafe { target.CreateSolidColorBrush(&tokens.foreground, None)? };
        let muted = unsafe { target.CreateSolidColorBrush(&tokens.muted, None)? };
        let selected = unsafe { target.CreateSolidColorBrush(&tokens.selected, None)? };
        let border = unsafe { target.CreateSolidColorBrush(&tokens.border, None)? };
        let accent = unsafe { target.CreateSolidColorBrush(&tokens.accent, None)? };
        let panel_background = unsafe { target.CreateSolidColorBrush(&tokens.panel, None)? };
        for panel in [PanelId::Explorer, PanelId::Calendar, PanelId::Outline] {
            let Some(body) = app.feature_panel_rect(panel) else {
                continue;
            };
            match panel {
                PanelId::Explorer if !app.search_pane_active() => {
                    self.draw_explorer(app, target, body, &foreground)?
                }
                PanelId::Explorer => {}
                PanelId::Calendar => self.draw_calendar(
                    app,
                    target,
                    body,
                    &foreground,
                    &muted,
                    &selected,
                    &border,
                    &accent,
                )?,
                PanelId::Outline => {
                    self.draw_outline(app, target, body, &foreground, &muted, &selected, &border)?
                }
                PanelId::Git => {}
            }
        }
        if self.quick_open.open {
            let editor = Rect {
                x: app.editor_left(),
                y: app.editor_top(),
                width: app.editor_width(),
                height: (app.editor_bottom() - app.editor_top()).max(0.),
            };
            let list_height = (editor.height * 0.55)
                .clamp(96., 280.)
                .min((editor.height - 64.).max(0.));
            let overlay = super::rect(
                editor.x + 10.,
                editor.y + 4.,
                (editor.width - 20.).max(0.),
                list_height + 56.,
            );
            unsafe {
                target.FillRectangle(&overlay, &panel_background);
                target.DrawRectangle(&overlay, &border, 1., None);
            }
            let status = match &self.quick_open.result {
                None => "検索中…".to_owned(),
                Some(result) => {
                    let mut message = if result.paths.is_empty() {
                        "一致するファイルはありません".to_owned()
                    } else {
                        format!("{}件を表示", result.paths.len())
                    };
                    if result.more {
                        message.push_str(" · 候補が続きます。入力で絞り込んでください");
                    }
                    if !result.complete {
                        message.push_str(" · 走査は未完了です");
                    }
                    message
                }
            };
            draw_feature_text(
                app,
                target,
                &status,
                editor.x + 20.,
                editor.y + list_height + 40.,
                editor.width - 40.,
                11.,
                &muted,
            )?;
        }
        Ok(())
    }

    fn draw_explorer(
        &self,
        app: &App,
        target: &ID2D1HwndRenderTarget,
        body: Rect,
        foreground: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        let title = self.workspace.as_ref().map_or_else(
            || "Workspaceを開いてください".to_owned(),
            |workspace| {
                if self.explorer.directory == Path::new(".") {
                    workspace.root().display().to_string()
                } else {
                    format!(
                        "{}  ›  {}",
                        workspace.root().display(),
                        self.explorer.directory.display()
                    )
                }
            },
        );
        draw_feature_text(
            app,
            target,
            &title,
            body.x + 8.,
            body.y + 31.,
            body.width - 16.,
            12.,
            foreground,
        )?;
        if self.form.as_ref().is_some_and(|form| {
            matches!(
                form.mode,
                FormMode::NewFile | FormMode::NewFolder | FormMode::Rename
            )
        }) {
            draw_feature_text(
                app,
                target,
                "Enter: 保存 / Esc: キャンセル",
                body.x + 8.,
                body.y + 68.,
                body.width - 16.,
                11.,
                foreground,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Explicit render brushes share this frame and target.
    fn draw_calendar(
        &self,
        app: &App,
        target: &ID2D1HwndRenderTarget,
        body: Rect,
        foreground: &ID2D1SolidColorBrush,
        muted: &ID2D1SolidColorBrush,
        selected: &ID2D1SolidColorBrush,
        border: &ID2D1SolidColorBrush,
        accent: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        let profile_list = self.profiles.manager_open && self.form.is_none();
        if let Some(form) = self
            .form
            .as_ref()
            .filter(|form| calendar_form_active(Some(form)))
        {
            draw_feature_text(
                app,
                target,
                &form.title,
                body.x + 8.,
                body.y + 5.,
                body.width - 16.,
                13.,
                accent,
            )?;
            return Ok(());
        }
        if profile_list {
            draw_feature_text(
                app,
                target,
                "プロファイル",
                body.x + 8.,
                body.y + 6.,
                body.width - 16.,
                12.,
                foreground,
            )?;
            let layout = profile_list_layout(body.height);
            let visible_profiles = self
                .profiles
                .effective
                .len()
                .saturating_sub(self.calendar.scroll);
            let visible_profiles = visible_profiles.min(layout.visible_rows);
            if visible_profiles == 0 {
                draw_feature_text(
                    app,
                    target,
                    "プロファイルがありません",
                    body.x + 8.,
                    body.y + layout.rows_top,
                    body.width - 16.,
                    11.,
                    muted,
                )?;
            }
            for (index, profile) in self
                .profiles
                .effective
                .iter()
                .enumerate()
                .skip(self.calendar.scroll)
                .take(layout.visible_rows)
            {
                let y = body.y
                    + layout.rows_top
                    + (index - self.calendar.scroll) as f32 * CALENDAR_ROW_HEIGHT;
                if index == self.profiles.selected {
                    unsafe {
                        target.FillRectangle(
                            &super::rect(body.x + 3., y, body.width - 6., 17.),
                            selected,
                        );
                    }
                }
                draw_feature_text(
                    app,
                    target,
                    &profile.name,
                    body.x + 8.,
                    y + 1.,
                    body.width - 16.,
                    10.,
                    foreground,
                )?;
            }
            if let (Some(workspace), Some(profile)) = (
                self.workspace.as_ref(),
                self.profiles.effective.get(self.profiles.selected),
            ) {
                let date_time = local_now();
                let values: ProfileValues = profile
                    .inputs
                    .iter()
                    .map(|input| (input.id.clone(), input.default_value.clone()))
                    .collect();
                let preview_text =
                    match preview_profile_path(workspace, profile, date_time, &values) {
                        Ok(preview) if preview.missing_required_inputs.is_empty() => {
                            format!("作成先: {}", preview.relative_path.display())
                        }
                        Ok(preview) => {
                            format!("入力が必要: {}", preview.missing_required_inputs.join(", "))
                        }
                        Err(error) => format!("作成先を確認できません: {error}"),
                    };
                let preview_y =
                    body.y + layout.rows_top + visible_profiles as f32 * CALENDAR_ROW_HEIGHT + 1.0;
                if visible_profiles > 0
                    && preview_y + 9.0 <= body.y + body.height - PROFILE_BUTTON_FOOTER
                {
                    draw_feature_text(
                        app,
                        target,
                        &preview_text,
                        body.x + 8.,
                        preview_y,
                        body.width - 16.,
                        9.,
                        muted,
                    )?;
                }
            }
            return Ok(());
        }
        if body.height < CALENDAR_MIN_BODY_HEIGHT {
            draw_feature_text(
                app,
                target,
                "カレンダーを見るにはパネルを広げてください",
                body.x + 8.,
                body.y + 34.,
                body.width - 16.,
                11.,
                muted,
            )?;
            return Ok(());
        }
        let layout = calendar_layout(body.height);
        let month = self.calendar.interaction.displayed_month();
        draw_feature_text(
            app,
            target,
            &format!("{}年 {:02}月", month.year, month.month),
            body.x + 8.,
            body.y + (layout.grid_top - 41.0).max(0.0),
            body.width - 16.,
            14.,
            foreground,
        )?;
        let Some(grid) = self.calendar.current_month else {
            return Ok(());
        };
        let holidays: BTreeSet<_> = grid
            .cells
            .into_iter()
            .flatten()
            .filter(|date| {
                matches!(
                    self.calendar.holidays.status(*date),
                    HolidayStatus::Holiday(_)
                )
            })
            .collect();
        let cells = decorate_month(
            grid,
            self.calendar.interaction.today(),
            self.calendar.interaction.selected(),
            self.calendar.interaction.hovered(),
            self.calendar.interaction.focused(),
            &holidays,
            &self.calendar.daily_markers,
        );
        let width = ((body.width - 10.) / 7.).max(1.0);
        let x0 = body.x + 5.;
        let tokens = app.shell.tokens;
        let red = unsafe { target.CreateSolidColorBrush(&tokens.weekend_red, None)? };
        let blue = unsafe { target.CreateSolidColorBrush(&tokens.weekend_blue, None)? };
        for (column, label) in ["日", "月", "火", "水", "木", "金", "土"]
            .iter()
            .enumerate()
        {
            draw_feature_text(
                app,
                target,
                label,
                x0 + column as f32 * width,
                body.y + (layout.grid_top - 18.0).max(0.0),
                width,
                11.,
                match column {
                    0 => &red,
                    6 => &blue,
                    _ => muted,
                },
            )?;
        }
        for (index, cell) in cells.iter().enumerate() {
            let row = index / 7;
            let y = body.y + layout.grid_top + row as f32 * layout.week_step;
            if y + layout.cell_height > body.y + body.height {
                break;
            }
            let Some(bounds) = calendar_cell_draw_bounds(body, layout, index) else {
                continue;
            };
            let cell_rect = super::rect(
                bounds.cell.x,
                bounds.cell.y,
                bounds.cell.width,
                bounds.cell.height,
            );
            if cell.marks.selected || cell.marks.focused {
                unsafe {
                    target.FillRectangle(&cell_rect, selected);
                }
            } else if cell.marks.today {
                unsafe {
                    target.DrawRectangle(&cell_rect, accent, 1., None);
                }
            } else if cell.marks.hovered {
                unsafe {
                    target.DrawRectangle(&cell_rect, border, 1., None);
                }
            }
            if let Some(date) = cell.date {
                let label = calendar_day_label(date.day, cell.marks.daily_file, cell.marks.holiday);
                let ink = calendar_cell_color(tokens, cell);
                let brush = unsafe { target.CreateSolidColorBrush(&ink, None)? };
                draw_feature_text_single_line(
                    app,
                    target,
                    &label,
                    bounds.label,
                    calendar_day_font_size(&label, bounds.label.width),
                    &brush,
                )?;
            }
        }
        let list_y = body.y + layout.list_top;
        if let Some(result) = &self.calendar.listing {
            let status = match result.state {
                CalendarListState::Reading => "一覧を読み込み中…".to_owned(),
                CalendarListState::Zero => "この日のファイルはありません".to_owned(),
                CalendarListState::Ready => format!("この日: {} 件", result.files.len()),
                CalendarListState::Failed => result
                    .error
                    .clone()
                    .unwrap_or_else(|| "一覧を取得できません".to_owned()),
                CalendarListState::Cancelled => "一覧の取得を中止しました".to_owned(),
            };
            if layout.status_height > 0.0 {
                draw_feature_text_single_line(
                    app,
                    target,
                    &status,
                    Rect {
                        x: body.x + 8.,
                        y: list_y,
                        width: body.width - 16.,
                        height: layout.status_height,
                    },
                    10.,
                    muted,
                )?;
            }
            for (index, file) in result
                .files
                .iter()
                .enumerate()
                .skip(self.calendar.scroll)
                .take(layout.visible_rows)
            {
                let y = body.y
                    + layout.rows_top
                    + (index - self.calendar.scroll) as f32 * CALENDAR_ROW_HEIGHT;
                if index == self.calendar.selected_file {
                    unsafe {
                        target.FillRectangle(
                            &super::rect(body.x + 3., y, body.width - 6., 17.),
                            selected,
                        );
                    }
                }
                draw_feature_text(
                    app,
                    target,
                    &file.relative_path.display().to_string(),
                    body.x + 8.,
                    y + 1.,
                    body.width - 16.,
                    10.,
                    foreground,
                )?;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Explicit render brushes share this frame and target.
    fn draw_outline(
        &self,
        app: &App,
        target: &ID2D1HwndRenderTarget,
        body: Rect,
        foreground: &ID2D1SolidColorBrush,
        muted: &ID2D1SolidColorBrush,
        selected: &ID2D1SolidColorBrush,
        border: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        let Some(outline) = self.outline.as_ref() else {
            return Ok(());
        };
        if !outline.is_current(app.doc()) {
            draw_feature_text(
                app,
                target,
                "アウトラインを更新中…",
                body.x + 8.,
                body.y + 8.,
                body.width - 16.,
                12.,
                muted,
            )?;
            return Ok(());
        }
        for (index, heading) in outline
            .headings
            .iter()
            .enumerate()
            .skip(self.outline_scroll)
            .take(200)
        {
            let y = body.y + 8. + (index - self.outline_scroll) as f32 * 22.;
            if y + 22. > body.y + body.height {
                break;
            }
            if index == self.outline_selected {
                unsafe {
                    target.FillRectangle(
                        &super::rect(body.x + 3., y, body.width - 6., 21.),
                        selected,
                    );
                }
            }
            let indent = (heading.level.saturating_sub(1) as f32 * 12.).min(body.width / 3.);
            draw_feature_text(
                app,
                target,
                &heading.label,
                body.x + 8. + indent,
                y + 2.,
                body.width - indent - 16.,
                12.,
                foreground,
            )?;
        }
        if let Some((_, _, _, _, true, Some(index))) = &self.outline_drag {
            let y = body.y + 8. + index.saturating_sub(self.outline_scroll) as f32 * 22.;
            if y >= body.y && y < body.y + body.height {
                unsafe {
                    target.FillRectangle(&super::rect(body.x + 3., y, body.width - 6., 2.), border);
                }
            }
        }
        unsafe {
            target.DrawRectangle(
                &super::rect(body.x + 2., body.y + 4., body.width - 4., body.height - 8.),
                border,
                0.5,
                None,
            );
        }
        Ok(())
    }

    fn calendar_file_action(&self) -> Option<FeatureAction> {
        let workspace = self.workspace.as_ref()?;
        let request = self.calendar.request.as_ref()?;
        let listing = self.calendar.listing.as_ref()?;
        if !request.matches(workspace, self.calendar.list_generation, request.date)
            || self.calendar.interaction.selected() != Some(request.date)
            || !listing.matches(workspace, request.generation, request.date)
        {
            return None;
        }
        let file = listing.files.get(self.calendar.selected_file)?;
        Some(FeatureAction::OpenCalendarFile {
            request: Box::new(request.clone()),
            relative_path: file.relative_path.clone(),
        })
    }

    fn double_click(
        &mut self,
        panel: PanelId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Option<FeatureAction> {
        let previous = self.calendar.click_target.take();
        self.click(panel, x, y, width, height);
        let current = self.calendar.click_target.take();
        if previous.is_none() || previous != current {
            return Some(FeatureAction::Refresh);
        }
        match current? {
            CalendarClickTarget::Date(date) => Some(FeatureAction::OpenDaily(date)),
            CalendarClickTarget::File(_) => self.calendar_file_action(),
        }
    }

    fn click(
        &mut self,
        panel: PanelId,
        x: f32,
        y: f32,
        panel_width: f32,
        panel_height: f32,
    ) -> Option<FeatureAction> {
        self.calendar.click_target = None;
        if panel == PanelId::Outline {
            let row_top = y - 8.;
            if row_top < 0. {
                return None;
            }
            let index = self.outline_scroll + (row_top / 22.) as usize;
            if self
                .outline
                .as_ref()
                .is_some_and(|outline| index < outline.headings.len())
            {
                self.outline_selected = index;
                return Some(FeatureAction::Command(4040));
            }
            return None;
        }
        if panel == PanelId::Calendar {
            if self.form.is_some() {
                return None;
            }
            if self.profiles.manager_open {
                let layout = profile_list_layout(panel_height);
                if y < layout.rows_top {
                    return None;
                }
                let row = ((y - layout.rows_top) / CALENDAR_ROW_HEIGHT) as usize;
                let index = row + self.calendar.scroll;
                if row < layout.visible_rows && index < self.profiles.effective.len() {
                    self.profiles.selected = index;
                    return Some(FeatureAction::Refresh);
                }
                return None;
            }
            if panel_height < CALENDAR_MIN_BODY_HEIGHT {
                return None;
            }
            let layout = calendar_layout(panel_height);
            let rel_x = x - 5.;
            let rel_y = y - layout.grid_top;
            if rel_x >= 0. && (0. ..6. * layout.week_step).contains(&rel_y) {
                // The caller supplies panel-local coordinates; the grid uses seven equal columns.
                let cell_width = ((panel_width - 10.) / 7.).max(1.);
                let column = ((rel_x - 1.0) / cell_width) as usize;
                if rel_x < 1.0 || column > 6 {
                    return None;
                }
                let x_in_cell = rel_x - (column as f32 * cell_width + 1.0);
                let row = (rel_y / layout.week_step) as usize;
                let y_in_cell = rel_y - row as f32 * layout.week_step;
                if x_in_cell >= cell_width - 2.0 || y_in_cell >= layout.cell_height {
                    return None;
                }
                let index = row * 7 + column;
                if let Some(date) = self
                    .calendar
                    .current_month
                    .and_then(|grid| grid.cells[index])
                {
                    self.calendar.interaction.click_date(date);
                    self.calendar.file_focused = false;
                    self.start_calendar_list(date);
                    self.calendar.click_target = Some(CalendarClickTarget::Date(date));
                    return Some(FeatureAction::Refresh);
                }
                return None;
            }
            if y >= layout.rows_top {
                let row = ((y - layout.rows_top) / CALENDAR_ROW_HEIGHT) as usize;
                if row >= layout.visible_rows {
                    return None;
                }
                let index = row + self.calendar.scroll;
                let workspace = self.workspace.as_ref()?;
                let request = self.calendar.request.as_ref()?;
                let listing = self.calendar.listing.as_ref()?;
                if !request.matches(workspace, self.calendar.list_generation, request.date)
                    || self.calendar.interaction.selected() != Some(request.date)
                    || !listing.matches(workspace, request.generation, request.date)
                {
                    return None;
                }
                let file = listing.files.get(index)?;
                self.calendar.selected_file = index;
                self.calendar.file_focused = true;
                self.calendar.click_target =
                    Some(CalendarClickTarget::File(file.relative_path.clone()));
                return Some(FeatureAction::Refresh);
            }
        }
        None
    }

    fn drag_start_path(&mut self, path: &Path) -> bool {
        let Some(entry) = self
            .explorer
            .entries
            .iter()
            .find(|entry| entry.path.as_path() == path)
        else {
            return false;
        };
        if !self
            .explorer
            .selected
            .iter()
            .any(|selected| selected == path)
        {
            self.explorer.selected.clear();
            self.explorer.selected.push(entry.path.clone());
        }
        self.explorer.drag_source = Some(entry.path.clone());
        self.explorer.drag_target = None;
        true
    }

    fn drag_move_to(&mut self, target: Option<PathBuf>) -> bool {
        if self.explorer.drag_source.is_none() {
            return false;
        }
        let target = target.filter(|path| {
            self.explorer
                .entries
                .iter()
                .any(|entry| entry.path == *path && entry.kind == ExplorerEntryKind::Folder)
        });
        let changed = target != self.explorer.drag_target;
        self.explorer.drag_target = target;
        changed
    }

    fn drag_end(&mut self) -> Option<PathBuf> {
        let _source = self.explorer.drag_source.take()?;
        self.explorer.drag_target.take()
    }

    fn keyboard_action(
        &mut self,
        message: &MSG,
        focus: HWND,
        composing: bool,
        active: PanelId,
        panel_focused: bool,
    ) -> Option<FeatureAction> {
        if message.message != WM_KEYDOWN || composing || self.control_composing() {
            return None;
        }
        let key = message.wParam.0 as u16;
        if self.quick_open.open {
            match key {
                KEY_ESCAPE => {
                    self.quick_open.open = false;
                    return Some(FeatureAction::Command(4000));
                }
                KEY_UP => {
                    self.quick_open.selected = self.quick_open.selected.saturating_sub(1);
                    return Some(FeatureAction::Refresh);
                }
                KEY_DOWN => {
                    let max = self
                        .quick_open
                        .result
                        .as_ref()
                        .map_or(0, |result| result.paths.len().saturating_sub(1));
                    self.quick_open.selected = (self.quick_open.selected + 1).min(max);
                    return Some(FeatureAction::Refresh);
                }
                KEY_RETURN => {
                    let path = self
                        .quick_open
                        .result
                        .as_ref()?
                        .paths
                        .get(self.quick_open.selected)?
                        .clone();
                    self.quick_open.open = false;
                    return Some(FeatureAction::OpenFile(path));
                }
                _ => return None,
            }
        }
        if focus == self.controls.prompt {
            return match key {
                KEY_RETURN => Some(FeatureAction::Command(4085)),
                KEY_ESCAPE => Some(FeatureAction::Command(4086)),
                _ => None,
            };
        }
        if focus == self.controls.template {
            return (key == KEY_ESCAPE).then_some(FeatureAction::Command(4086));
        }
        if let Some(index) = self
            .controls
            .profile_fields
            .iter()
            .position(|hwnd| *hwnd == focus)
        {
            return match key {
                KEY_ESCAPE => Some(FeatureAction::Command(4086)),
                KEY_RETURN if index < 6 => Some(FeatureAction::Command(4085)),
                _ => None,
            };
        }
        if [self.controls.query, self.controls.explorer_list].contains(&focus)
            || self
                .controls
                .buttons
                .iter()
                .any(|(_, _, hwnd, _)| *hwnd == focus)
        {
            return None;
        }
        if !panel_focused {
            return None;
        }
        match active {
            PanelId::Calendar => {
                self.calendar.click_target = None;
                if key != KEY_RETURN {
                    self.calendar.file_focused = false;
                }
                match key {
                    KEY_LEFT => {
                        self.calendar.interaction.move_focus_days(-1);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_RIGHT => {
                        self.calendar.interaction.move_focus_days(1);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_UP => {
                        self.calendar.interaction.move_focus_days(-7);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_DOWN => {
                        self.calendar.interaction.move_focus_days(7);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_PAGE_UP => {
                        self.calendar.interaction.navigate_months(-1);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_PAGE_DOWN => {
                        self.calendar.interaction.navigate_months(1);
                        Some(FeatureAction::Refresh)
                    }
                    KEY_RETURN if self.calendar.file_focused => self.calendar_file_action(),
                    KEY_RETURN => Some(FeatureAction::OpenDaily(
                        self.calendar
                            .interaction
                            .focused()
                            .unwrap_or(self.calendar.interaction.today()),
                    )),
                    _ => None,
                }
            }
            PanelId::Explorer => match key {
                KEY_UP => {
                    self.explorer.selected = self
                        .explorer
                        .entries
                        .get(
                            self.explorer
                                .entries
                                .iter()
                                .position(|entry| self.explorer.selected.contains(&entry.path))
                                .unwrap_or(0)
                                .saturating_sub(1),
                        )
                        .map(|entry| vec![entry.path.clone()])
                        .unwrap_or_default();
                    Some(FeatureAction::Refresh)
                }
                KEY_DOWN => {
                    let start = self
                        .explorer
                        .entries
                        .iter()
                        .position(|entry| self.explorer.selected.contains(&entry.path))
                        .unwrap_or(0);
                    self.explorer.selected = self
                        .explorer
                        .entries
                        .get((start + 1).min(self.explorer.entries.len().saturating_sub(1)))
                        .map(|entry| vec![entry.path.clone()])
                        .unwrap_or_default();
                    Some(FeatureAction::Refresh)
                }
                KEY_RETURN => {
                    let entry = self
                        .explorer
                        .entries
                        .iter()
                        .find(|entry| self.explorer.selected.contains(&entry.path))?
                        .clone();
                    if entry.kind == ExplorerEntryKind::Folder {
                        self.explorer.directory = entry.path;
                        Some(FeatureAction::Refresh)
                    } else {
                        Some(FeatureAction::OpenFile(entry.path))
                    }
                }
                _ => None,
            },
            PanelId::Outline => match key {
                KEY_UP => {
                    self.outline_selected = self.outline_selected.saturating_sub(1);
                    Some(FeatureAction::Refresh)
                }
                KEY_DOWN => {
                    self.outline_selected = (self.outline_selected + 1).min(
                        self.outline
                            .as_ref()
                            .map_or(0, |outline| outline.headings.len().saturating_sub(1)),
                    );
                    Some(FeatureAction::Refresh)
                }
                KEY_RETURN => Some(FeatureAction::Command(4040)),
                _ => None,
            },
            PanelId::Git => None,
        }
    }

    fn control_composing(&self) -> bool {
        self.controls.query_ime.get()
            || self.controls.prompt_ime.get()
            || self.controls.template_ime.get()
            || self.controls.profile_ime.iter().any(|ime| ime.get())
    }

    fn sync_explorer_control(&mut self) {
        if self.controls.explorer_list == HWND::default()
            || !unsafe { IsWindow(Some(self.controls.explorer_list)).as_bool() }
        {
            return;
        }
        unsafe {
            let list = self.controls.explorer_list;
            let restored_top = shell::send(list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0))
                .0
                .max(0) as usize;
            if self.explorer_list_dirty {
                let _ = shell::send(list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
                for (index, entry) in self.explorer.entries.iter().enumerate() {
                    let is_folder = entry.kind == ExplorerEntryKind::Folder;
                    let is_expanded = self
                        .explorer
                        .expanded_rows
                        .get(index)
                        .copied()
                        .unwrap_or(false);
                    let prefix = match (is_folder, is_expanded) {
                        (true, true) => "▾ ",
                        (true, false) => "▸ ",
                        (false, _) => "  ",
                    };
                    let label = format!("{prefix}{}", entry.name.to_string_lossy());
                    let text = super::wide(&label);
                    let item = shell::send(
                        list,
                        LB_ADDSTRING,
                        WPARAM(0),
                        LPARAM(text.as_ptr() as isize),
                    )
                    .0;
                    if let Ok(item) = usize::try_from(item) {
                        let depth = self.explorer.depths.get(index).copied().unwrap_or(0);
                        let item_data =
                            (depth << 8) | is_folder as usize | ((is_expanded as usize) << 1);
                        let _ = shell::send(
                            list,
                            LB_SETITEMDATA,
                            WPARAM(item),
                            LPARAM(item_data as isize),
                        );
                    }
                }
                self.explorer_list_dirty = false;
            }
            let _ = shell::send(list, LB_SETSEL, WPARAM(0), LPARAM(-1));
            for path in &self.explorer.selected {
                if let Some(index) = self
                    .explorer
                    .entries
                    .iter()
                    .position(|entry| entry.path == *path)
                {
                    let _ = shell::send(list, LB_SETSEL, WPARAM(1), LPARAM(index as isize));
                }
            }
            if !self.explorer.entries.is_empty() {
                let top = restored_top.min(self.explorer.entries.len().saturating_sub(1));
                let _ = shell::send(list, LB_SETTOPINDEX, WPARAM(top), LPARAM(0));
            }
        }
        if self.controls.quick_open_list == HWND::default()
            || !unsafe { IsWindow(Some(self.controls.quick_open_list)).as_bool() }
        {
            return;
        }
        unsafe {
            let list = self.controls.quick_open_list;
            let key = (self.quick_open.generation, self.quick_open.result.is_some());
            if self.controls.quick_open_list_key != Some(key) {
                let _ = shell::send(list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
                if let Some(result) = &self.quick_open.result {
                    for path in &result.paths {
                        let relative = path
                            .strip_prefix(
                                self.workspace
                                    .as_ref()
                                    .map_or(Path::new(""), Workspace::root),
                            )
                            .unwrap_or(path);
                        let text = super::wide(&relative.display().to_string());
                        let _ = shell::send(
                            list,
                            LB_ADDSTRING,
                            WPARAM(0),
                            LPARAM(text.as_ptr() as isize),
                        );
                    }
                }
                self.controls.quick_open_list_key = Some(key);
            }
            if self
                .quick_open
                .result
                .as_ref()
                .is_some_and(|result| !result.paths.is_empty())
            {
                let _ = shell::send(
                    list,
                    LB_SETCURSEL,
                    WPARAM(self.quick_open.selected),
                    LPARAM(0),
                );
            }
        }
    }
}

#[allow(clippy::too_many_arguments)] // Direct text draw primitive, with explicit position and brush.
fn draw_feature_text(
    app: &App,
    target: &ID2D1HwndRenderTarget,
    text: &str,
    x: f32,
    y: f32,
    width: f32,
    size: f32,
    brush: &ID2D1SolidColorBrush,
) -> windows::core::Result<()> {
    let layout = unsafe { app.ui_text_layout(text, width.max(1.), size)? };
    unsafe {
        target.DrawTextLayout(
            super::point(x, y),
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
    }
    Ok(())
}

fn draw_feature_text_single_line(
    app: &App,
    target: &ID2D1HwndRenderTarget,
    text: &str,
    bounds: Rect,
    size: f32,
    brush: &ID2D1SolidColorBrush,
) -> windows::core::Result<()> {
    let layout = unsafe { app.ui_text_layout(text, bounds.width.max(1.0), size)? };
    unsafe {
        layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
        layout.SetMaxHeight(bounds.height.max(1.0))?;
        target.DrawTextLayout(
            super::point(bounds.x, bounds.y),
            &layout,
            brush,
            D2D1_DRAW_TEXT_OPTIONS_CLIP,
        );
    }
    Ok(())
}

unsafe extern "system" fn explorer_list_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    unsafe {
        let result = DefSubclassProc(hwnd, message, wparam, lparam);
        if message == WM_LBUTTONDOWN {
            let parent = GetParent(hwnd).unwrap_or_default();
            let mut point = POINT {
                x: (lparam.0 as u16 as i16) as i32,
                y: ((lparam.0 >> 16) as u16 as i16) as i32,
            };
            let _ = MapWindowPoints(Some(hwnd), Some(parent), std::slice::from_mut(&mut point));
            let packed = ((point.y as u16 as u32) << 16) | point.x as u16 as u32;
            let _ = PostMessageW(
                Some(parent),
                WM_APP + 20,
                WPARAM(0),
                LPARAM(packed as isize),
            );
        }
        result
    }
}

fn reap_workers(workers: &mut Vec<JoinHandle<()>>) {
    let mut pending = Vec::new();
    for worker in workers.drain(..) {
        if worker.is_finished() {
            let _ = worker.join();
        } else {
            pending.push(worker);
        }
    }
    *workers = pending;
}

enum FeatureEvent {
    QuickOpen(Result<(), String>),
    Calendar(CalendarListResult),
}

struct FeatureSnapshot {
    workspace: Option<Workspace>,
    active_panel: PanelId,
    document_id: u64,
    revision: u64,
    outline: Option<Outline>,
    today: ProfileDateTime,
}

impl FeatureSnapshot {
    fn capture(app: &mut App) -> Self {
        let document_id = app.doc().id();
        let revision = app.doc().revision();
        let outline = if !app.is_markdown_document() {
            Some(Outline {
                document_id,
                revision,
                headings: Vec::new(),
            })
        } else if app
            .features
            .outline
            .as_ref()
            .is_some_and(|outline| outline.is_current(app.doc()))
        {
            None
        } else {
            let projection = app.cached_projection();
            Some(Outline::build(app.doc(), &projection))
        };
        let workspace = app.persistence.workspace_root().and_then(|root| {
            if let Some(workspace) = app
                .features
                .workspace
                .as_ref()
                .filter(|workspace| workspace.root() == root.as_path())
            {
                Some(workspace.clone())
            } else {
                Workspace::open(root).ok()
            }
        });
        Self {
            workspace,
            active_panel: app.shell.active_panel(),
            document_id,
            revision,
            outline,
            today: local_now(),
        }
    }
}

struct FeatureControlLayout {
    dpi: f32,
    editor: Rect,
    explorer: Option<Rect>,
    calendar: Option<Rect>,
    form: Option<FormMode>,
}

impl FeatureControlLayout {
    fn capture(app: &App) -> Self {
        Self {
            dpi: app.dpi,
            editor: Rect {
                x: app.editor_left(),
                y: app.editor_top(),
                width: app.editor_width(),
                height: (app.editor_bottom() - app.editor_top()).max(0.),
            },
            explorer: if app.search_pane_active() {
                None
            } else {
                app.feature_panel_rect(PanelId::Explorer)
            },
            calendar: app.feature_panel_rect(PanelId::Calendar),
            form: app.features.form.as_ref().map(|form| form.mode),
        }
    }
}

fn profile_form_field_rect(panel: Rect, index: usize) -> Rect {
    let top = panel.y + 26.0;
    let button_top = panel.y + (panel.height - 34.0).max(0.0);
    let content_bottom = (button_top - 6.0).max(top);
    let step = ((content_bottom - top - 42.0).max(0.0) / 6.0).min(26.0);
    let height = if index == 6 { 42.0 } else { step.min(22.0) };
    Rect {
        x: panel.x,
        y: top + index as f32 * step,
        width: panel.width,
        height,
    }
}

fn profile_inputs_rect(panel: Rect) -> Rect {
    Rect {
        x: panel.x + 6.0,
        y: panel.y + 44.0,
        width: (panel.width - 12.0).max(0.0),
        height: (panel.height - 84.0).max(0.0),
    }
}

enum FeatureAction {
    Refresh,
    Command(usize),
    OpenFile(PathBuf),
    OpenCalendarFile {
        request: Box<CalendarListRequest>,
        relative_path: PathBuf,
    },
    OpenDaily(CalendarDate),
}

impl App {
    pub(super) fn feature_test_geometry(&self, panels: &[(PanelId, Rect); 4]) -> serde_json::Value {
        let body = |id: PanelId| {
            let r = panels[id as usize].1;
            Rect {
                x: r.x,
                y: r.y + 30.,
                width: r.width,
                height: (r.height - 30.).max(0.),
            }
        };
        let calendar = body(PanelId::Calendar);
        let outline = body(PanelId::Outline);
        let profile_list = self.features.profiles.manager_open && self.features.form.is_none();
        let calendar_form = calendar_form_active(self.features.form.as_ref());
        let calendar_layout = calendar_layout(calendar.height);
        let cells = if profile_list || calendar_form || calendar.height < CALENDAR_MIN_BODY_HEIGHT {
            Vec::new()
        } else {
            self.features
                .calendar
                .current_month
                .map(|grid| {
                    grid.cells
                        .iter()
                        .enumerate()
                        .filter_map(|(index, date)| {
                            let date = (*date)?;
                            let cell_width = (calendar.width - 10.) / 7.;
                            let x = calendar.x + 5. + (index % 7) as f32 * cell_width + 1.0;
                            let y = calendar.y
                                + calendar_layout.grid_top
                                + (index / 7) as f32 * calendar_layout.week_step;
                            (y + calendar_layout.cell_height <= calendar.y + calendar.height).then(
                                || {
                                    serde_json::json!({
                                        "year": date.year,
                                        "month": date.month,
                                        "day": date.day,
                                        "x": x,
                                        "y": y,
                                        "width": cell_width - 2.0,
                                        "height": calendar_layout.cell_height
                                    })
                                },
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let profile_layout = profile_list_layout(calendar.height);
        let rows = self
            .features
            .outline
            .as_ref()
            .filter(|value| value.is_current(self.doc()))
            .map(|value| {
                value
                    .headings
                    .iter()
                    .enumerate()
                    .skip(self.features.outline_scroll)
                    .take(200)
                    .filter_map(|(index, heading)| {
                        let y = outline.y
                            + 8.
                            + (index - self.features.outline_scroll) as f32 * OUTLINE_ROW_HEIGHT;
                        (y + OUTLINE_ROW_HEIGHT <= outline.y + outline.height).then(|| {
                            serde_json::json!({
                                "index": index,
                                "source_start": heading.source.start,
                                "x": outline.x + 3.,
                                "y": y,
                                "width": (outline.width - 6.).max(0.),
                                "height": OUTLINE_ROW_HEIGHT - 1.
                            })
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        serde_json::json!({
            "calendar_cells": cells,
            "calendar_file_rows": if profile_list || calendar_form {
                0
            } else {
                calendar_layout.visible_rows
            },
            "calendar_profile_rows": if profile_list {
                profile_layout.visible_rows
            } else {
                0
            },
            "outline_rows": rows,
            "quick_open": {
                "open": self.features.quick_open.open,
                "busy": self.features.quick_open.worker.is_some(),
                "shown_count": self.features.quick_open.result.as_ref().map(|r|r.paths.len()),
                "more": self.features.quick_open.result.as_ref().map(|r|r.more),
                "complete": self.features.quick_open.result.as_ref().map(|r|r.complete)
            }
        })
    }
    pub(super) unsafe fn features_init(&mut self) {
        unsafe {
            if self.features.initialized || !self.shell.enabled {
                return;
            }
            self.features
                .create_controls(self.shell.main_window, self.dpi);
            self.features.initialized = true;
            self.features_sync();
        }
    }

    pub(super) fn features_sync(&mut self) {
        let snapshot = FeatureSnapshot::capture(self);
        self.features.synchronize(snapshot);
        if let Some(body) = self.feature_panel_rect(PanelId::Outline) {
            self.features
                .ensure_outline_selection_visible(outline_visible_rows(body.height));
        }
        let explorer_changed = self.features.explorer_list_dirty;
        self.features.sync_explorer_control();
        let layout = FeatureControlLayout::capture(self);
        self.features.position_controls(self, layout);
        self.features_refresh_explorer_paint(explorer_changed);
        if let Some(status) = self.features.status.take() {
            self.status = status;
        }
    }

    pub(super) fn features_refresh_explorer_paint(&mut self, force: bool) {
        let list = self.features.controls.explorer_list;
        if list == HWND::default() || !unsafe { IsWindow(Some(list)).as_bool() } {
            self.features.explorer_paint_key = None;
            let retired = self.features.explorer_paint.replace(None);
            drop(retired);
            return;
        }
        let tokens = self.shell.tokens;
        let mut key = ExplorerPaintKey {
            hwnd: list,
            font: unsafe { shell::send(list, WM_GETFONT, WPARAM(0), LPARAM(0)).0 },
            dpi: unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(list) },
            colors: [
                shell::rgb(tokens.panel).0,
                shell::rgb(tokens.selected).0,
                shell::rgb(tokens.foreground).0,
            ],
            icons: self.explorer_icons.generation(),
        };
        if !force && self.features.explorer_paint_key == Some(key) {
            return;
        }
        let geometry_changed = force
            || self.features.explorer_paint_key.is_none_or(|previous| {
                previous.hwnd != key.hwnd || previous.font != key.font || previous.dpi != key.dpi
            });
        // Preparation may reenter; the old immutable snapshot remains available.
        let next = self
            .explorer_icons
            .paint_snapshot(list, tokens)
            .map(Rc::new);
        key.icons = self.explorer_icons.generation();
        self.features.explorer_paint_key = next.as_ref().map(|_| key);
        // Retiring font/icon handles can make native calls. Never drop them
        // while the publication slot is borrowed by a synchronous callback.
        let retired = self.features.explorer_paint.replace(next);
        drop(retired);
        // Native H geometry changes can synchronously request painting.
        if geometry_changed && let Some(extent) = self.explorer_icons.measure_extent(list) {
            let _ = unsafe { super::control_scroll::set_horizontal_extent(list, extent) };
        }
    }

    pub(super) fn features_document_rebound(&mut self) {
        self.features.outline = None;
        self.features.outline_document = 0;
        self.features.outline_revision = 0;
        self.features.outline_selected = 0;
        self.features.outline_scroll = 0;
        self.features.outline_drag = None;
    }

    pub(super) unsafe fn features_draw(
        &self,
        target: &ID2D1HwndRenderTarget,
    ) -> windows::core::Result<()> {
        self.features.draw(self, target)
    }

    pub(super) fn features_reset_calendar_click(&mut self) {
        self.features.calendar.click_target = None;
    }

    pub(super) unsafe fn features_double_click(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        if panel != PanelId::Calendar {
            return false;
        }
        if self.feature_ime_active() || self.background_mutation_busy() {
            return true;
        }
        let Some(body) = self.feature_panel_rect(panel) else {
            return false;
        };
        if x < body.x || y < body.y || x >= body.x + body.width || y >= body.y + body.height {
            return false;
        }
        self.focus_feature_body(panel);
        if let Some(action) =
            self.features
                .double_click(panel, x - body.x, y - body.y, body.width, body.height)
        {
            self.run_feature_action(action);
            self.features_sync();
            unsafe {
                self.refresh();
            }
        }
        true
    }

    pub(super) unsafe fn features_click(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        if !matches!(panel, PanelId::Calendar | PanelId::Outline) {
            return false;
        }
        if self.feature_ime_active() || self.background_mutation_busy() {
            return true;
        }
        let Some(body) = self.feature_panel_rect(panel) else {
            return false;
        };
        if x < body.x || y < body.y || x >= body.x + body.width || y >= body.y + body.height {
            return false;
        }
        self.focus_feature_body(panel);
        if panel == PanelId::Outline
            && let Some(outline) = self
                .features
                .outline
                .clone()
                .filter(|o| o.is_current(self.doc()))
        {
            let index = self.features.outline_scroll
                + ((y - body.y - 8.).max(0.) / OUTLINE_ROW_HEIGHT) as usize;
            if index < outline.headings.len() {
                self.features.outline_drag = Some((outline, index, x, y, false, None));
                unsafe {
                    super::SetCapture(self.hwnd);
                }
            }
        }
        let action = self
            .features
            .click(panel, x - body.x, y - body.y, body.width, body.height);
        if let Some(action) = action {
            self.run_feature_action(action);
            self.features_sync();
            unsafe {
                self.refresh();
            }
            true
        } else {
            true
        }
    }

    pub(super) fn features_actionable_at(&self, x: f32, y: f32) -> bool {
        if self.feature_ime_active() || self.background_mutation_busy() {
            return false;
        }
        if let Some(body) = self
            .feature_panel_rect(PanelId::Outline)
            .filter(|r| r.contains(x, y))
        {
            if y < body.y + 8. {
                return false;
            }
            let row = ((y - body.y - 8.) / OUTLINE_ROW_HEIGHT) as usize;
            return row < outline_visible_rows(body.height)
                && self
                    .features
                    .outline
                    .as_ref()
                    .filter(|outline| outline.is_current(self.doc()))
                    .is_some_and(|outline| {
                        self.features.outline_scroll + row < outline.headings.len()
                    });
        }
        let Some(body) = self
            .feature_panel_rect(PanelId::Calendar)
            .filter(|r| r.contains(x, y))
        else {
            return false;
        };
        if self.features.form.is_some() {
            return false;
        }
        if self.features.profiles.manager_open {
            let layout = profile_list_layout(body.height);
            if y < body.y + layout.rows_top {
                return false;
            }
            let row = ((y - body.y - layout.rows_top) / CALENDAR_ROW_HEIGHT) as usize;
            return row < layout.visible_rows
                && self.features.calendar.scroll + row < self.features.profiles.effective.len();
        }
        if body.height < CALENDAR_MIN_BODY_HEIGHT {
            return false;
        }
        let layout = calendar_layout(body.height);
        if self.features.calendar.current_month.is_some_and(|grid| {
            grid.cells.iter().enumerate().any(|(index, date)| {
                date.is_some()
                    && calendar_cell_draw_bounds(body, layout, index)
                        .is_some_and(|bounds| bounds.cell.contains(x, y))
            })
        }) {
            return true;
        }
        if y < body.y + layout.rows_top {
            return false;
        }
        let row = ((y - body.y - layout.rows_top) / CALENDAR_ROW_HEIGHT) as usize;
        if row >= layout.visible_rows {
            return false;
        }
        let (Some(workspace), Some(request), Some(listing)) = (
            &self.features.workspace,
            &self.features.calendar.request,
            &self.features.calendar.listing,
        ) else {
            return false;
        };
        request.matches(
            workspace,
            self.features.calendar.list_generation,
            request.date,
        ) && self.features.calendar.interaction.selected() == Some(request.date)
            && listing.matches(workspace, request.generation, request.date)
            && self.features.calendar.scroll + row < listing.files.len()
    }

    pub(super) fn features_pointer_move(&mut self, x: f32, y: f32) -> bool {
        if let Some((_, _, start_x, start_y, moved, before)) = &mut self.features.outline_drag {
            *moved |= (x - *start_x).abs() > 4. || (y - *start_y).abs() > 4.;
            *before = None;
        }
        let target = self
            .feature_panel_rect(PanelId::Outline)
            .filter(|r| r.contains(x, y))
            .map(|r| {
                self.features.outline_scroll
                    + ((y - r.y - 8.).max(0.) / OUTLINE_ROW_HEIGHT) as usize
            });
        if let Some((outline, _, _, _, _, before)) = &mut self.features.outline_drag {
            *before = target.filter(|index| *index < outline.headings.len());
            return true;
        }
        let date = self
            .feature_panel_rect(PanelId::Calendar)
            .filter(|r| r.contains(x, y))
            .and_then(|r| {
                if r.height < CALENDAR_MIN_BODY_HEIGHT
                    || self.features.profiles.manager_open
                    || self.features.form.is_some()
                {
                    return None;
                }
                let layout = calendar_layout(r.height);
                let ry = y - r.y - layout.grid_top;
                let cx = x - r.x - 5.;
                let width = ((r.width - 10.) / 7.).max(1.);
                if !(0. ..6. * layout.week_step).contains(&ry) || cx < 1. || cx >= 7. * width - 1. {
                    return None;
                }
                let column = ((cx - 1.0) / width) as usize;
                let row = (ry / layout.week_step) as usize;
                let x_in_cell = cx - (column as f32 * width + 1.0);
                let y_in_cell = ry - row as f32 * layout.week_step;
                if column > 6 || x_in_cell >= width - 2.0 || y_in_cell >= layout.cell_height {
                    return None;
                }
                self.features
                    .calendar
                    .current_month
                    .and_then(|grid| grid.cells[row * 7 + column])
            });
        self.features.calendar.interaction.set_hover(date)
    }

    pub(super) fn features_wheel(&mut self, x: f32, y: f32, delta: f32) -> bool {
        if !delta.is_finite() || delta == 0.0 {
            return false;
        }
        if let Some(body) = self
            .feature_panel_rect(PanelId::Outline)
            .filter(|body| body.contains(x, y))
        {
            let count = self
                .features
                .outline
                .as_ref()
                .filter(|outline| outline.is_current(self.doc()))
                .map_or(0, |outline| outline.headings.len());
            self.features.outline_scroll = scroll_list(
                self.features.outline_scroll,
                count,
                outline_visible_rows(body.height),
                delta,
            );
            return true;
        }
        if let Some(body) = self
            .feature_panel_rect(PanelId::Calendar)
            .filter(|body| body.contains(x, y))
        {
            if self.features.form.is_some() || body.height < CALENDAR_MIN_BODY_HEIGHT {
                return true;
            }
            if self.features.profiles.manager_open {
                let layout = profile_list_layout(body.height);
                if y >= body.y + layout.rows_top {
                    self.features.calendar.scroll = scroll_list(
                        self.features.calendar.scroll,
                        self.features.profiles.effective.len(),
                        layout.visible_rows,
                        delta,
                    );
                }
            } else {
                let layout = calendar_layout(body.height);
                if y >= body.y + layout.rows_top {
                    let count = self
                        .features
                        .calendar
                        .listing
                        .as_ref()
                        .map_or(0, |listing| listing.files.len());
                    self.features.calendar.scroll = scroll_list(
                        self.features.calendar.scroll,
                        count,
                        layout.visible_rows,
                        delta,
                    );
                }
            }
            return true;
        }
        false
    }
    pub(super) fn features_finish_outline_drag(&mut self, cancel: bool) -> bool {
        let Some((outline, from, _, _, moved, before)) = self.features.outline_drag.take() else {
            return false;
        };
        if cancel || !moved {
            return true;
        }
        if let Some(before) = before {
            let blocked = self.feature_ime_active() || self.background_mutation_busy();
            match outline.move_section(self.doc_mut(), from, before, blocked) {
                Ok(()) => unsafe {
                    self.after_input();
                },
                Err(error) => self.status = error.into(),
            }
        }
        true
    }
    pub(super) unsafe fn features_drag_start(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        if panel != PanelId::Explorer {
            return false;
        }
        let Some(path) = self.explorer_path_at(x, y) else {
            return false;
        };
        self.features.drag_start_path(&path)
    }

    pub(super) unsafe fn features_drag_move(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        if panel != PanelId::Explorer {
            return false;
        }
        self.features.drag_move_to(self.explorer_path_at(x, y))
    }

    pub(super) unsafe fn features_drag_end(&mut self, panel: PanelId, x: f32, y: f32) -> bool {
        if panel != PanelId::Explorer {
            return false;
        }
        let _ = self.features.drag_move_to(self.explorer_path_at(x, y));
        let Some(destination) = self.features.drag_end() else {
            return false;
        };
        self.features.pending_move_to = Some(destination);
        self.run_feature_action(FeatureAction::Command(4004));
        true
    }

    fn explorer_path_at(&self, x: f32, y: f32) -> Option<PathBuf> {
        let list = self.features.controls.explorer_list;
        if list == HWND::default() {
            return None;
        }
        let mut point = POINT {
            x: (x * self.dpi).round() as i32,
            y: (y * self.dpi).round() as i32,
        };
        unsafe {
            MapWindowPoints(
                Some(self.shell.main_window),
                Some(list),
                std::slice::from_mut(&mut point),
            );
            let packed = ((point.y as u16 as u32) << 16) | point.x as u16 as u32;
            let hit =
                shell::send(list, LB_ITEMFROMPOINT, WPARAM(0), LPARAM(packed as isize)).0 as u32;
            if hit >> 16 != 0 {
                return None;
            }
            self.features
                .explorer
                .entries
                .get((hit & 0xffff) as usize)
                .map(|entry| entry.path.clone())
        }
    }

    pub(super) unsafe fn features_control(&mut self, id: usize, notification: u16) -> bool {
        unsafe {
            if self.search_pane_active()
                && (id == EXPLORER_LIST
                    || (id == PROMPT_EDIT
                        && self.features.form.as_ref().is_some_and(|form| {
                            matches!(
                                form.mode,
                                FormMode::NewFile | FormMode::NewFolder | FormMode::Rename
                            )
                        }))
                    || self
                        .features
                        .controls
                        .buttons
                        .iter()
                        .any(|(control_id, _, _, panel)| {
                            *control_id == id && *panel == PanelId::Explorer
                        }))
            {
                return true;
            }
            if id == QUICK_OPEN_LIST {
                if notification == LBN_SELCHANGE as u16 || notification == LBN_DBLCLK as u16 {
                    let selected = shell::send(
                        self.features.controls.quick_open_list,
                        LB_GETCURSEL,
                        WPARAM(0),
                        LPARAM(0),
                    )
                    .0;
                    if selected >= 0 {
                        self.features.quick_open.selected = selected as usize;
                    }
                    if notification == LBN_DBLCLK as u16
                        && let Some(path) = self
                            .features
                            .quick_open
                            .result
                            .as_ref()
                            .and_then(|result| result.paths.get(self.features.quick_open.selected))
                            .cloned()
                    {
                        self.features.pending_open_file = Some(path);
                        self.features.quick_open.open = false;
                        self.features.pending_focus_restore = true;
                        self.features_sync();
                        let _ = PostMessageW(
                            Some(self.features.controls.owner),
                            WM_COMMAND,
                            WPARAM(4010),
                            LPARAM(0),
                        );
                    }
                    return true;
                }
                return false;
            }
            if id == EXPLORER_LIST {
                if notification == LBN_SELCHANGE as u16 || notification == LBN_DBLCLK as u16 {
                    let list = self.features.controls.explorer_list;
                    let count = shell::send(list, LB_GETCOUNT, WPARAM(0), LPARAM(0))
                        .0
                        .max(0) as usize;
                    let visible = self
                        .features
                        .explorer
                        .entries
                        .iter()
                        .map(|entry| entry.path.clone())
                        .collect::<BTreeSet<_>>();
                    let mut selected = Vec::new();
                    let current = if notification == LBN_DBLCLK as u16 {
                        shell::send(list, LB_GETCURSEL, WPARAM(0), LPARAM(0)).0
                    } else {
                        -1
                    };
                    if let Ok(index) = usize::try_from(current)
                        && let Some(entry) = self.features.explorer.entries.get(index)
                    {
                        selected.push(entry.path.clone());
                    }
                    for index in 0..count {
                        if shell::send(list, LB_GETSEL, WPARAM(index), LPARAM(0)).0 > 0
                            && let Some(entry) = self.features.explorer.entries.get(index)
                            && !selected.contains(&entry.path)
                        {
                            selected.push(entry.path.clone());
                        }
                    }
                    selected.extend(
                        self.features
                            .explorer
                            .selected
                            .iter()
                            .filter(|path| !visible.contains(*path))
                            .cloned(),
                    );
                    self.features.explorer.selected = selected;
                    if notification == LBN_DBLCLK as u16 {
                        let _ = PostMessageW(
                            Some(self.features.controls.owner),
                            WM_COMMAND,
                            WPARAM(4010),
                            LPARAM(0),
                        );
                    }
                    return true;
                }
                return false;
            }
            if u32::from(notification) == EN_CHANGE {
                let text = if id == QUERY_EDIT {
                    shell::text_of(self.features.controls.query)
                } else if id == PROMPT_EDIT {
                    shell::text_of(self.features.controls.prompt)
                } else if (PROFILE_FIELD_BASE..PROFILE_FIELD_BASE + 7).contains(&id) {
                    shell::text_of(self.features.controls.profile_fields[id - PROFILE_FIELD_BASE])
                } else if id == TEMPLATE_EDIT {
                    shell::text_of(self.features.controls.template)
                } else {
                    return false;
                };
                if id == QUERY_EDIT {
                    let recent = self.persistence.recent_paths();
                    self.features.start_quick_open(text, recent);
                    self.features.sync_explorer_control();
                } else if id == PROMPT_EDIT {
                    self.features.prompt_text = text;
                } else if (PROFILE_FIELD_BASE..PROFILE_FIELD_BASE + 7).contains(&id) {
                    self.features.profile_form_text[id - PROFILE_FIELD_BASE] = text;
                } else {
                    self.features.template_text = text;
                }
                return true;
            }
            if notification != BN_CLICKED as u16 {
                return false;
            }
            let Some(command) = self
                .features
                .controls
                .buttons
                .iter()
                .find_map(|(control_id, command, _, _)| (*control_id == id).then_some(*command))
            else {
                return false;
            };
            let _ = PostMessageW(
                Some(self.features.controls.owner),
                WM_COMMAND,
                WPARAM(command),
                LPARAM(0),
            );
            true
        }
    }

    pub(super) unsafe fn features_message(&mut self, message: &MSG) -> bool {
        unsafe {
            if message.message == WM_KEYDOWN
                && message.wParam.0 as u16 == KEY_ESCAPE
                && self.features.outline_drag.is_some()
                && !self.feature_ime_active()
            {
                self.features_finish_outline_drag(true);
                let _ = super::ReleaseCapture();
                self.refresh();
                return true;
            }
            if message.message == WM_KEYDOWN
                && !self.search_pane_active()
                && self.features.controls.explorer_list != HWND::default()
                && GetFocus() == self.features.controls.explorer_list
                && !self.feature_ime_active()
                && self.explorer_list_key(message.wParam.0 as u16)
            {
                self.features.sync_explorer_control();
                self.refresh();
                return true;
            }
            if self.search_pane_active() && !self.features.quick_open.open {
                return false;
            }
            let was_quick_open = self.features.quick_open.open;
            let action = self.features.keyboard_action(
                message,
                GetFocus(),
                self.feature_ime_active(),
                self.shell.active_panel(),
                self.feature_body_focused(),
            );
            if let Some(action) = action {
                if was_quick_open && !self.features.quick_open.open {
                    self.features.pending_focus_restore = true;
                }
                self.run_feature_action(action);
                self.features_sync();
                self.refresh();
                true
            } else {
                false
            }
        }
    }

    pub(super) fn features_tick(&mut self, _now: Instant) -> bool {
        let events = self.features.poll_workers();
        let changed = !events.is_empty();
        for event in events {
            match event {
                FeatureEvent::QuickOpen(result) => {
                    if let Err(error) = result {
                        self.features.status = Some(error);
                    }
                }
                FeatureEvent::Calendar(result) => {
                    if result.state == CalendarListState::Failed {
                        self.features.status = result.error.clone();
                    }
                }
            }
        }
        if changed {
            self.features_sync();
        }
        changed
    }

    pub(super) unsafe fn features_command(&mut self, id: usize) -> bool {
        if !COMMANDS.iter().any(|(command, _, _)| *command == id) {
            return false;
        }
        if id == 4000 && self.feature_ime_active() {
            self.features.status =
                Some("入力確定後にクイックオープンを切り替えてください".to_owned());
            return true;
        }
        if id == 4000 && self.features.pending_focus_restore {
            self.features.pending_focus_restore = false;
            self.restore_quick_open_focus();
            self.features_sync();
            return true;
        }
        if self.feature_ime_active()
            && matches!(
                id,
                4001..=4011 | 4023..=4028 | 4040..=4042 | 4080..=4088 | 4090
            )
        {
            self.features.status = Some("入力確定後に操作してください".to_owned());
            return true;
        }
        if id == CALENDAR_OPERATIONS_BUTTON {
            self.open_calendar_operations_popup();
            return true;
        }
        if id == 4010 {
            if self.features.pending_focus_restore {
                self.features.pending_focus_restore = false;
                self.restore_quick_open_focus();
            }
            if let Some(path) = self.features.pending_open_file.take() {
                self.open_feature_file(path);
                return true;
            }
        }
        if id == 4023
            && let Some(date) = self.features.pending_daily.take()
        {
            let _ = self.create_calendar_daily(date);
            return true;
        }
        match id {
            4000 => self.open_quick_open(),
            4001 => self.start_name_prompt(FormMode::NewFile),
            4002 => self.start_name_prompt(FormMode::NewFolder),
            4003 => self.copy_selected_paths(),
            4004 => {
                if let Some(destination) = self.features.pending_move_to.take() {
                    self.move_selected_to(destination);
                    true
                } else {
                    self.paste_files()
                }
            }
            4005 => self.start_name_prompt(FormMode::Rename),
            4006 => self.recycle_selected(),
            4007 => self.copy_path_payload(),
            4008 => self.reveal_selected(),
            4009 => self.explorer_parent(),
            4010 => self.open_selected_explorer_file(),
            4011 => self.open_another_workspace(),
            4020 => self.navigate_month(-1),
            4021 => self.navigate_month(1),
            4022 => self.select_today(),
            4023 => self.create_calendar_daily(
                self.features
                    .calendar
                    .interaction
                    .selected()
                    .unwrap_or(self.features.calendar.interaction.today()),
            ),
            4024 => self.create_calendar_profile("meeting"),
            4025 => self.create_calendar_profile("memo"),
            4026 => self.import_holidays(),
            4027 => self.toggle_profile_manager(),
            4028 => self.create_calendar_profile("daily"),
            4040 => self.open_outline_selection(),
            4041 => self.move_outline_selection(-1),
            4042 => self.move_outline_selection(1),
            4080 => self.begin_profile_form(FormMode::NewProfile, None),
            4081 => self.begin_profile_form(FormMode::EditProfile, None),
            4082 => self.begin_profile_form(FormMode::DuplicateProfile, None),
            4083 => self.delete_workspace_profile(),
            4084 => self.begin_template_edit(),
            4085 => {
                if self.features.form.as_ref().is_some_and(|form| {
                    matches!(
                        form.mode,
                        FormMode::NewFile | FormMode::NewFolder | FormMode::Rename
                    )
                }) {
                    self.save_name_prompt()
                } else {
                    self.save_profile_form()
                }
            }
            4086 => self.cancel_feature_form(),
            4087 => self.save_template_edit(),
            4088 => self.create_form_profile(),
            _ => false,
        }
    }

    fn open_calendar_operations_popup(&mut self) {
        unsafe {
            let Ok(menu) = CreatePopupMenu() else {
                return;
            };
            for command in CALENDAR_BUTTONS {
                let Some((_, label, _)) = COMMANDS.iter().find(|(id, _, _)| *id == command) else {
                    continue;
                };
                let label = super::wide(label);
                let mut flags = MF_STRING;
                if !self.features_enabled(command) {
                    flags |= MF_GRAYED;
                }
                if AppendMenuW(menu, flags, command, PCWSTR(label.as_ptr())).is_err() {
                    let _ = DestroyMenu(menu);
                    return;
                }
            }
            let mut point = POINT::default();
            if GetCursorPos(&mut point).is_ok() {
                let selected = super::menu_style::track_popup(
                    self.hwnd,
                    menu,
                    TPM_RETURNCMD | TPM_NONOTIFY | TPM_LEFTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
                    point.x,
                    point.y,
                    self.shell.tokens,
                    self.dpi,
                )
                .0;
                let _ = DestroyMenu(menu);
                if selected > 0 {
                    let _ = self.features_command(selected as usize);
                }
            } else {
                let _ = DestroyMenu(menu);
            }
        }
    }

    pub(super) fn features_enabled(&self, id: usize) -> bool {
        let workspace = self.features.workspace.is_some();
        let selected = !self.features.explorer.selected.is_empty();
        let active_outline = self
            .features
            .outline
            .as_ref()
            .is_some_and(|outline| outline.is_current(self.doc()));
        let profile_selected = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected)
            .is_some();
        let open_documents = self.open_document_paths();
        let selected_open = self
            .features
            .explorer
            .selected
            .iter()
            .any(|path| is_open_or_ancestor(path, &open_documents));
        let selected_profile_id = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected)
            .map(|profile| profile.id.as_str());
        let profile_defined_in_workspace = selected_profile_id.is_some_and(|id| {
            self.features
                .profiles
                .layer
                .iter()
                .any(|profile| profile.id == id)
        });
        if feature_requires_ime_end(id) && self.feature_ime_active() {
            return false;
        }
        if feature_writes_workspace(id) && self.background_mutation_busy() {
            return false;
        }
        match id {
            4000 => workspace,
            4011 => true,
            4001 | 4002 | 4026 | 4027 => workspace,
            4003 | 4008 => workspace && selected,
            4005 => workspace && self.features.explorer.selected.len() == 1 && !selected_open,
            4006 => workspace && selected && !selected_open,
            4007 => workspace && (selected || self.features.pending_path_copy.is_some()),
            4010 => {
                workspace
                    && (selected
                        || self.features.pending_open_file.is_some()
                        || (self.features.quick_open.open
                            && self
                                .features
                                .quick_open
                                .result
                                .as_ref()
                                .is_some_and(|result| !result.paths.is_empty())))
            }
            4004 => {
                workspace
                    && (!self.features.explorer.internal_copy.is_empty()
                        || (self.features.pending_move_to.is_some() && !selected_open))
            }
            4009 => {
                workspace
                    && !self.features.explorer.directory.as_os_str().is_empty()
                    && self.features.explorer.directory != Path::new(".")
            }
            CALENDAR_OPERATIONS_BUTTON => true,
            4020..=4022 => true,
            4023 | 4028 => {
                workspace
                    && self
                        .features
                        .profiles
                        .effective
                        .iter()
                        .any(|p| p.id == "daily")
            }
            4024 => {
                workspace
                    && self
                        .features
                        .profiles
                        .effective
                        .iter()
                        .any(|p| p.id == "meeting")
            }
            4025 => {
                workspace
                    && self
                        .features
                        .profiles
                        .effective
                        .iter()
                        .any(|p| p.id == "memo")
            }
            4040 => active_outline && !self.features.outline.as_ref().unwrap().headings.is_empty(),
            4041 | 4042 => {
                active_outline && self.features.outline.as_ref().unwrap().headings.len() > 1
            }
            4080 => workspace,
            4081 | 4082 | 4084 => workspace && profile_selected,
            4083 => workspace && profile_selected && profile_defined_in_workspace,
            4085 => self.features.form.as_ref().is_some_and(|form| {
                matches!(
                    form.mode,
                    FormMode::NewFile
                        | FormMode::NewFolder
                        | FormMode::Rename
                        | FormMode::NewProfile
                        | FormMode::EditProfile
                        | FormMode::DuplicateProfile
                )
            }),
            4086 => self.features.form.is_some(),
            4087 => self
                .features
                .form
                .as_ref()
                .is_some_and(|form| form.mode == FormMode::EditTemplate),
            4088 => self
                .features
                .form
                .as_ref()
                .is_some_and(|form| form.mode == FormMode::CreateProfileInputs),
            _ => false,
        }
    }

    pub(super) fn features_disabled_reason(&self, id: usize) -> &'static str {
        if feature_requires_ime_end(id) && self.feature_ime_active() {
            return "IME入力の確定後に操作できます";
        }
        if feature_writes_workspace(id) && self.background_mutation_busy() {
            return "別のWorkspace変更処理が完了するまで待ってください";
        }
        let workspace = self.features.workspace.is_some();
        let selected = !self.features.explorer.selected.is_empty();
        let open_documents = self.open_document_paths();
        let selected_open = self
            .features
            .explorer
            .selected
            .iter()
            .any(|path| is_open_or_ancestor(path, &open_documents));
        let profile = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected);
        let has_outline =
            self.features.outline.as_ref().is_some_and(|outline| {
                outline.is_current(self.doc()) && !outline.headings.is_empty()
            });
        match id {
            4000 | 4001 | 4002 | 4003 | 4004 | 4005 | 4006 | 4007 | 4008 | 4009 | 4010 | 4023
            | 4024 | 4025 | 4026 | 4027 | 4028 | 4080 | 4081 | 4082 | 4083 | 4084 | 4088
                if !workspace =>
            {
                "Workspaceを開いてください"
            }
            4010 if !selected
                && self.features.pending_open_file.is_none()
                && !self.features.quick_open.open =>
            {
                "開くファイルを選択してください"
            }
            4005 if self.features.explorer.selected.len() != 1 => {
                "名前を変更する項目を1つ選択してください"
            }
            4005 | 4006 if selected_open => "開いている文書またはその親フォルダーは操作できません",
            4003 | 4006 | 4007 | 4008 if !selected => "Explorerで項目を選択してください",
            4004 if self.features.pending_move_to.is_some() && selected_open => {
                "開いている文書またはその親フォルダーは移動できません"
            }
            4004 if self.features.explorer.internal_copy.is_empty()
                && self.features.pending_move_to.is_none() =>
            {
                "貼り付けるWorkspace内のコピー対象がありません"
            }
            4009 if self.features.explorer.directory == Path::new(".") => {
                "Workspaceのrootには親フォルダーがありません"
            }
            4023 | 4028
                if !self
                    .features
                    .profiles
                    .effective
                    .iter()
                    .any(|p| p.id == "daily") =>
            {
                "デイリーノートのプロファイルがありません"
            }
            4024 if !self
                .features
                .profiles
                .effective
                .iter()
                .any(|p| p.id == "meeting") =>
            {
                "ミーティングのプロファイルがありません"
            }
            4025 if !self
                .features
                .profiles
                .effective
                .iter()
                .any(|p| p.id == "memo") =>
            {
                "メモのプロファイルがありません"
            }
            4040 if !has_outline => "移動できる見出しがありません",
            4041 | 4042 if !has_outline => "移動できる見出しがありません",
            4041 | 4042
                if !self
                    .features
                    .outline
                    .as_ref()
                    .is_some_and(|outline| outline.headings.len() > 1) =>
            {
                "移動できる別のセクションがありません"
            }
            4081..=4084 if profile.is_none() => "プロファイルを選択してください",
            4083 if profile.is_some_and(|profile| {
                !self
                    .features
                    .profiles
                    .layer
                    .iter()
                    .any(|item| item.id == profile.id)
            }) =>
            {
                "Workspaceで定義したプロファイルだけ削除できます"
            }
            4085..=4088 if self.features.form.is_none() => "操作するフォームがありません",
            4085 if self.features.form.as_ref().is_some_and(|form| {
                matches!(
                    form.mode,
                    FormMode::CreateProfileInputs | FormMode::EditTemplate
                )
            }) =>
            {
                "このフォームでは別の保存操作を使ってください"
            }
            4085 if !self.features.form.as_ref().is_some_and(|form| {
                matches!(
                    form.mode,
                    FormMode::NewFile
                        | FormMode::NewFolder
                        | FormMode::Rename
                        | FormMode::NewProfile
                        | FormMode::EditProfile
                        | FormMode::DuplicateProfile
                )
            }) =>
            {
                "保存できるフォームを開いてください"
            }
            4087 if !self
                .features
                .form
                .as_ref()
                .is_some_and(|form| form.mode == FormMode::EditTemplate) =>
            {
                "テンプレート編集フォームを開いてください"
            }
            4088 if !self
                .features
                .form
                .as_ref()
                .is_some_and(|form| form.mode == FormMode::CreateProfileInputs) =>
            {
                "プロファイル入力フォームを開いてください"
            }
            _ => "現在の状態では実行できません",
        }
    }

    fn open_quick_open(&mut self) -> bool {
        if self.features.quick_open.open {
            self.features.quick_open.open = false;
            self.restore_quick_open_focus();
        } else {
            self.features.quick_open.saved_focus = unsafe { GetFocus() };
            self.features.quick_open.saved_document = self.doc().id();
            self.features.quick_open.saved_revision = self.doc().revision();
            self.features.quick_open.saved_selection = self.doc().selection();
            self.features.quick_open.saved_affinity = self.doc().selection_affinity();
            self.features.quick_open.open = true;
            self.features.quick_open.query.clear();
            self.features.quick_open.selected = 0;
            self.features
                .start_quick_open(String::new(), self.persistence.recent_paths());
            unsafe {
                let _ = SetWindowTextW(
                    self.features.controls.query,
                    PCWSTR(super::wide("").as_ptr()),
                );
                let _ = SetFocus(Some(self.features.controls.query));
            }
        }
        self.features_sync();
        if self.features.quick_open.open {
            unsafe {
                let _ = SetFocus(Some(self.features.controls.query));
            }
        }
        true
    }

    fn restore_quick_open_focus(&mut self) {
        unsafe {
            let same_document = self.doc().id() == self.features.quick_open.saved_document
                && self.doc().revision() == self.features.quick_open.saved_revision;
            if same_document {
                let selection = self.features.quick_open.saved_selection;
                let affinity = self.features.quick_open.saved_affinity;
                let _ = self.doc_mut().set_selection_with_affinity(
                    selection.anchor,
                    selection.head,
                    affinity,
                );
            }
            let target = self.features.quick_open.saved_focus;
            let target =
                if same_document && target != HWND::default() && IsWindow(Some(target)).as_bool() {
                    target
                } else {
                    self.editing_window()
                };
            let _ = SetFocus(Some(target));
        }
    }

    fn open_feature_file(&mut self, path: PathBuf) {
        if self.feature_ime_active() {
            self.features.status = Some("入力確定後にファイルを開いてください".to_owned());
            return;
        }
        let Some(workspace) = self.features.workspace.as_ref() else {
            self.features.status = Some("Workspaceが開かれていません".to_owned());
            return;
        };
        let canonical = match verify_workspace_entry(workspace, &path) {
            Ok(path) => path,
            Err(error) => {
                self.features.status = Some(format!("選択したファイルを安全に開けません: {error}"));
                return;
            }
        };
        let relative = match canonical.strip_prefix(workspace.root()) {
            Ok(relative) => relative,
            Err(_) => {
                self.features.status = Some("Workspace外のファイルはここから開けません".to_owned());
                return;
            }
        };
        let parent = relative.parent().unwrap_or_else(|| Path::new("."));
        match workspace.list_directory(parent) {
            Ok(listing)
                if listing.entries.iter().any(|entry| {
                    entry.path == canonical && entry.kind == ExplorerEntryKind::File
                }) =>
            {
                self.open_path(canonical);
                self.features.status = None;
            }
            Ok(_) => {
                self.features.status =
                    Some("保護された項目または通常ファイル以外は開けません".to_owned())
            }
            Err(error) => self.features.status = Some(format!("ファイルを検証できません: {error}")),
        }
    }

    fn open_calendar_file(&mut self, request: CalendarListRequest, relative_path: PathBuf) {
        let current = self.features.workspace.as_ref().is_some_and(|workspace| {
            let request_is_current =
                self.features
                    .calendar
                    .request
                    .as_ref()
                    .is_some_and(|active| {
                        active.generation == request.generation
                            && active.date == request.date
                            && active
                                .workspace_identity
                                .same_workspace(&request.workspace_identity)
                    });
            request.matches(
                workspace,
                self.features.calendar.list_generation,
                request.date,
            ) && request.daily_profile.as_ref()
                == self
                    .features
                    .profiles
                    .effective
                    .iter()
                    .find(|profile| profile.id == "daily")
                && request_is_current
                && self.features.calendar.interaction.selected() == Some(request.date)
                && self
                    .features
                    .calendar
                    .listing
                    .as_ref()
                    .is_some_and(|listing| {
                        listing.matches(workspace, request.generation, request.date)
                    })
        });
        if !current {
            self.features.status = Some("カレンダー一覧が変わったため項目を開けません".to_owned());
            return;
        }
        let checked_path = self
            .features
            .workspace
            .as_ref()
            .ok_or(mdlite::calendar::CalendarError::StaleEntry)
            .and_then(|workspace| request.recheck_before_open(workspace, &relative_path));
        match checked_path {
            Ok(path) => self.open_feature_file(path),
            Err(error) => {
                self.features.start_calendar_list(request.date);
                self.features.status = Some(format!(
                    "選択したカレンダー項目は変更されたため開けません: {error}"
                ));
            }
        }
    }

    fn create_calendar_daily(&mut self, date: CalendarDate) -> bool {
        let date_time = ProfileDateTime::at_midnight(date).unwrap_or_else(|_| local_now());
        self.prepare_profile_at("daily", date_time)
    }

    fn create_calendar_profile(&mut self, profile_id: &str) -> bool {
        self.prepare_profile_at(profile_id, local_now())
    }

    fn prepare_profile_at(&mut self, profile_id: &str, date_time: ProfileDateTime) -> bool {
        if self.feature_ime_active() {
            self.features.status = Some("入力確定後にノートを作成してください".to_owned());
            return true;
        }
        let Some(workspace) = self.features.workspace.clone() else {
            return false;
        };
        let Some(profile) = self
            .features
            .profiles
            .effective
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            self.features.status = Some(format!("プロファイルがありません: {profile_id}"));
            return false;
        };
        let values: ProfileValues = profile
            .inputs
            .iter()
            .map(|input| (input.id.clone(), input.default_value.clone()))
            .collect();
        let preview = match preview_profile_path(&workspace, &profile, date_time, &values) {
            Ok(preview) => preview,
            Err(error) => {
                self.features.status = Some(format!("作成先を確認できません: {error}"));
                return false;
            }
        };
        if !preview.missing_required_inputs.is_empty() {
            let initial = profile
                .inputs
                .iter()
                .map(|input| format!("{}={}", input.id, input.default_value))
                .collect::<Vec<_>>()
                .join("\n");
            self.features.profile_form_text[6] = initial.clone();
            self.features.form = Some(FormState {
                mode: FormMode::CreateProfileInputs,
                title: format!("{} — 入力", profile.name),
                profile_id: Some(profile.id.clone()),
                date_time: Some(date_time),
                template_snapshot: None,
                profile_snapshot: None,
                original_path: None,
                guard_document: self.doc().id(),
                guard_revision: self.doc().revision(),
            });
            unsafe {
                if let Some(field) = self.features.controls.profile_fields.get(6).copied() {
                    let _ = SetWindowTextW(field, PCWSTR(super::wide(&initial).as_ptr()));
                    let _ = SetFocus(Some(field));
                }
            }
            return true;
        }
        self.create_profile_note_for(profile, date_time, values);
        true
    }

    fn create_profile_note_for(
        &mut self,
        profile: ProfileDefinition,
        date_time: ProfileDateTime,
        values: ProfileValues,
    ) {
        if self.feature_ime_active() {
            self.features.status = Some("入力確定後にノートを作成してください".to_owned());
            return;
        }
        let Some(workspace) = self.features.workspace.clone() else {
            self.features.status = Some("Workspaceが開かれていません".to_owned());
            return;
        };
        match create_profile_note(&workspace, &profile, date_time, &values) {
            Ok(created) => {
                self.open_path(created.path);
                if created.created {
                    let caret = created.cursor_byte_offset.min(self.doc().byte_len());
                    let _ = self.doc_mut().set_selection(caret, caret);
                    unsafe {
                        self.after_input();
                    }
                }
                self.features.form = None;
                self.features.status = None;
                self.features_sync();
            }
            Err(ProfileError::RequiredInputs(missing)) => {
                self.features.status = Some(format!(
                    "必須入力を指定してください: {}",
                    missing.join(", ")
                ))
            }
            Err(error) => self.features.status = Some(format!("ノートを作成できません: {error}")),
        }
    }

    fn navigate_month(&mut self, amount: i32) -> bool {
        self.features_reset_calendar_click();
        self.features.calendar.interaction.navigate_months(amount);
        self.features_sync();
        true
    }

    fn select_today(&mut self) -> bool {
        self.features_reset_calendar_click();
        self.features.calendar.interaction.select_today();
        self.features_sync();
        true
    }

    fn open_outline_selection(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(outline) = self.features.outline.as_ref() else {
            return false;
        };
        if !outline.is_current(self.doc()) {
            self.features.status = Some("アウトラインを更新しています".to_owned());
            return true;
        }
        let Some(heading) = outline.headings.get(self.features.outline_selected) else {
            return false;
        };
        let offset = heading.source.start;
        let _ = self.doc_mut().set_selection(offset, offset);
        unsafe {
            self.after_input();
        }
        true
    }

    fn move_outline_selection(&mut self, direction: i32) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(outline) = self.features.outline.clone() else {
            return false;
        };
        if !outline.is_current(self.doc()) {
            self.features.status = Some("アウトラインが古いため移動できません".to_owned());
            return true;
        }
        let from = self.features.outline_selected;
        let Some(selected) = outline.headings.get(from) else {
            return true;
        };
        let level = selected.level;
        let peer = if direction < 0 {
            (0..from)
                .rev()
                .take_while(|&index| outline.headings[index].level >= level)
                .find(|&index| outline.headings[index].level == level)
        } else {
            (from + 1..outline.headings.len())
                .take_while(|&index| outline.headings[index].level >= level)
                .find(|&index| outline.headings[index].level == level)
        };
        let Some(peer) = peer else {
            return true;
        };
        let selected_start = if direction < 0 {
            outline.headings[peer].source.start
        } else {
            let selected_start = selected.source.start;
            let moved = &self.doc().source()[outline.headings[peer].section.clone()];
            let separator_len = if moved.ends_with('\n') {
                0
            } else if self.doc().source().contains("\r\n") {
                2
            } else {
                1
            };
            selected_start + moved.len() + separator_len
        };
        let (move_from, before) = if direction < 0 {
            (from, peer)
        } else {
            (peer, from)
        };
        let blocked = self.background_mutation_busy();
        let result = outline.move_section(self.doc_mut(), move_from, before, blocked);
        match result {
            Ok(()) => {
                unsafe {
                    self.after_input();
                }
                self.features_sync();
                self.features.outline_selected = self
                    .features
                    .outline
                    .as_ref()
                    .and_then(|outline| {
                        outline
                            .headings
                            .iter()
                            .position(|heading| heading.source.start == selected_start)
                    })
                    .unwrap_or(from);
                if let Some(body) = self.feature_panel_rect(PanelId::Outline) {
                    self.features
                        .ensure_outline_selection_visible(outline_visible_rows(body.height));
                }
                self.features.status = None;
            }
            Err(error) => self.features.status = Some(error.to_owned()),
        }
        true
    }

    fn toggle_profile_manager(&mut self) -> bool {
        self.features.profiles.manager_open = !self.features.profiles.manager_open;
        self.features.calendar.scroll = 0;
        if self.features.profiles.manager_open
            && let Some(workspace) = self.features.workspace.clone()
        {
            match initialize_default_templates(&workspace) {
                Ok(report) if !report.failures.is_empty() => {
                    self.features.status = Some(format!(
                        "テンプレートの一部を準備できません: {}",
                        report.failures[0].message
                    ))
                }
                Err(error) => {
                    self.features.status = Some(format!("テンプレートを確認できません: {error}"))
                }
                _ => self.features.status = None,
            }
        }
        self.features_sync();
        true
    }

    fn begin_profile_form(&mut self, mode: FormMode, _source: Option<String>) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let selected = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected)
            .cloned();
        let mut original_id = None;
        let profile = match mode {
            FormMode::NewProfile => ProfileDefinition {
                id: String::new(),
                name: String::new(),
                directory: PathBuf::new(),
                filename: PathBuf::new(),
                template_path: PathBuf::new(),
                collision: ProfileCollision::Sequence,
                inputs: Vec::new(),
            },
            FormMode::EditProfile | FormMode::DuplicateProfile => {
                let Some(source) = selected else {
                    self.features.status = Some("プロファイルを選択してください".to_owned());
                    return false;
                };
                original_id = Some(source.id.clone());
                if mode == FormMode::DuplicateProfile {
                    let id = available_duplicate_id(&self.features.profiles.effective, &source.id);
                    match duplicate_profile(
                        &self.features.profiles.effective,
                        &source.id,
                        id,
                        format!("{}（コピー）", source.name),
                    ) {
                        Ok(profile) => profile,
                        Err(error) => {
                            self.features.status = Some(error.to_string());
                            return false;
                        }
                    }
                } else {
                    source
                }
            }
            _ => return false,
        };
        self.features.profile_form_text = profile_form_fields(&profile);
        self.features.form = Some(FormState {
            mode,
            title: match mode {
                FormMode::NewProfile => "プロファイルを追加".to_owned(),
                FormMode::EditProfile => "プロファイルを編集".to_owned(),
                _ => "プロファイルを複製".to_owned(),
            },
            profile_id: original_id,
            date_time: None,
            template_snapshot: None,
            profile_snapshot: self.features.profiles.snapshot.clone(),
            original_path: None,
            guard_document: self.doc().id(),
            guard_revision: self.doc().revision(),
        });
        unsafe {
            for (index, value) in self.features.profile_form_text.iter().enumerate() {
                if let Some(hwnd) = self.features.controls.profile_fields.get(index) {
                    let _ = SetWindowTextW(*hwnd, PCWSTR(super::wide(value).as_ptr()));
                }
            }
            let _ = SetFocus(Some(self.features.controls.profile_fields[0]));
        }
        self.features_sync();
        true
    }

    fn save_profile_form(&mut self) -> bool {
        let Some(form) = self.features.form.clone() else {
            return false;
        };
        if self.feature_ime_active() {
            return true;
        }
        if form.guard_document != self.doc().id() || form.guard_revision != self.doc().revision() {
            self.features.status =
                Some("文書が変わったため、フォームを確認してから保存してください".to_owned());
            return true;
        }
        let profile = match parse_profile_fields(&self.features.profile_form_text) {
            Ok(profile) => profile,
            Err(error) => {
                self.features.status = Some(error.to_string());
                return true;
            }
        };
        let Some(workspace) = self.features.workspace.clone() else {
            return false;
        };
        let path = workspace.root().join(".mdlite/profiles.toml");
        let Some(snapshot) = form.profile_snapshot.as_ref() else {
            self.features.status = Some("プロファイルの保存状態を取得できません".to_owned());
            return true;
        };
        let mut layer = self.features.profiles.layer.clone();
        if let Some(old_id) = form.profile_id.as_deref() {
            remove_profile(&mut layer, old_id);
        }
        if form.mode == FormMode::DuplicateProfile
            && let Some(old_id) = form.profile_id.as_deref()
            && let Err(error) = duplicate_profile(
                &self.features.profiles.effective,
                old_id,
                profile.id.clone(),
                profile.name.clone(),
            )
        {
            self.features.status = Some(error.to_string());
            return true;
        }
        if let Err(error) = add_or_replace_profile(&mut layer, profile) {
            self.features.status = Some(error.to_string());
            return true;
        }
        match save_profile_layer(&path, &layer, snapshot) {
            Ok(snapshot) => {
                self.features.profiles.layer = layer;
                self.features.profiles.snapshot = Some(snapshot);
                if let Some(workspace) = self.features.workspace.clone() {
                    self.features.load_profile_definitions(&workspace);
                }
                self.features.form = None;
                self.features.status = None;
                self.features_sync();
            }
            Err(error) => {
                self.features.status = Some(format!(
                    "プロファイルを保存できません。外部変更を保持しました: {error}"
                ))
            }
        }
        true
    }

    fn delete_workspace_profile(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(profile) = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected)
            .cloned()
        else {
            return false;
        };
        if !self
            .features
            .profiles
            .layer
            .iter()
            .any(|entry| entry.id == profile.id)
        {
            self.features.status =
                Some("Workspaceで定義したプロファイルだけ削除できます".to_owned());
            return true;
        }
        let prompt = super::wide(&format!(
            "Workspaceのプロファイル「{}」を削除しますか？",
            profile.name
        ));
        if unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(prompt.as_ptr()),
                w!("プロファイル削除"),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
            )
        } != IDYES
        {
            return true;
        }
        let Some(workspace) = self.features.workspace.clone() else {
            return false;
        };
        let Some(snapshot) = self.features.profiles.snapshot.as_ref() else {
            return false;
        };
        let mut layer = self.features.profiles.layer.clone();
        remove_profile(&mut layer, &profile.id);
        match save_profile_layer(
            workspace.root().join(".mdlite/profiles.toml"),
            &layer,
            snapshot,
        ) {
            Ok(snapshot) => {
                self.features.profiles.layer = layer;
                self.features.profiles.snapshot = Some(snapshot);
                self.features.load_profile_definitions(&workspace);
                self.features.status = None;
            }
            Err(error) => {
                self.features.status = Some(format!("プロファイルを削除できません: {error}"))
            }
        }
        self.features_sync();
        true
    }

    fn begin_template_edit(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(workspace) = self.features.workspace.clone() else {
            return false;
        };
        let Some(profile) = self
            .features
            .profiles
            .effective
            .get(self.features.profiles.selected)
            .cloned()
        else {
            return false;
        };
        match load_profile_template(&workspace, &profile) {
            Ok(loaded) => {
                self.features.template_text = loaded.source.clone();
                self.features.form = Some(FormState {
                    mode: FormMode::EditTemplate,
                    title: format!("{} — テンプレート", profile.name),
                    profile_id: Some(profile.id),
                    date_time: None,
                    template_snapshot: Some(loaded.snapshot),
                    profile_snapshot: None,
                    original_path: None,
                    guard_document: self.doc().id(),
                    guard_revision: self.doc().revision(),
                });
                unsafe {
                    let _ = SetWindowTextW(
                        self.features.controls.template,
                        PCWSTR(super::wide(&self.features.template_text).as_ptr()),
                    );
                    let _ = SetFocus(Some(self.features.controls.template));
                }
                self.features_sync();
            }
            Err(error) => self.features.status = Some(format!("テンプレートを開けません: {error}")),
        }
        true
    }

    fn save_template_edit(&mut self) -> bool {
        let Some(form) = self.features.form.clone() else {
            return false;
        };
        if self.feature_ime_active() {
            return true;
        }
        if form.guard_document != self.doc().id() || form.guard_revision != self.doc().revision() {
            self.features.status =
                Some("文書が変わったため、テンプレートを確認してください".to_owned());
            return true;
        }
        let Some(workspace) = self.features.workspace.clone() else {
            return false;
        };
        let Some(profile) = self
            .features
            .profiles
            .effective
            .iter()
            .find(|profile| Some(profile.id.as_str()) == form.profile_id.as_deref())
            .cloned()
        else {
            return false;
        };
        let Some(snapshot) = form.template_snapshot.as_ref() else {
            return false;
        };
        match save_profile_template(&workspace, &profile, &self.features.template_text, snapshot) {
            Ok(snapshot) => {
                self.features.profiles.profile_template = Some(LoadedProfileTemplate {
                    source: self.features.template_text.clone(),
                    snapshot,
                });
                self.features.form = None;
                self.features.status = None;
                self.features_sync();
            }
            Err(error) => {
                self.features.status = Some(format!(
                    "テンプレートを保存できません。外部変更を保持しました: {error}"
                ))
            }
        }
        true
    }

    fn create_form_profile(&mut self) -> bool {
        let Some(form) = self.features.form.clone() else {
            return false;
        };
        if self.feature_ime_active() {
            return true;
        }
        if form.guard_document != self.doc().id() || form.guard_revision != self.doc().revision() {
            self.features.status =
                Some("文書が変わったため、入力内容を確認してください".to_owned());
            return true;
        }
        let Some(profile_id) = form.profile_id.as_deref() else {
            return false;
        };
        let Some(profile) = self
            .features
            .profiles
            .effective
            .iter()
            .find(|profile| profile.id == profile_id)
            .cloned()
        else {
            return false;
        };
        let values = match parse_profile_values(&self.features.profile_form_text[6]) {
            Ok(values) => values,
            Err(error) => {
                self.features.status = Some(error);
                return true;
            }
        };
        let Some(date_time) = form.date_time else {
            return false;
        };
        match preview_profile_path(
            self.features.workspace.as_ref().unwrap(),
            &profile,
            date_time,
            &values,
        ) {
            Ok(preview) if !preview.missing_required_inputs.is_empty() => {
                self.features.status = Some(format!(
                    "必須入力を指定してください: {}",
                    preview.missing_required_inputs.join(", ")
                ));
                return true;
            }
            Err(error) => {
                self.features.status = Some(format!("作成先を確認できません: {error}"));
                return true;
            }
            _ => {}
        }
        self.create_profile_note_for(profile, date_time, values);
        true
    }

    fn cancel_feature_form(&mut self) -> bool {
        if self.feature_ime_active() {
            self.features.status = Some("入力確定後にフォームを閉じてください".to_owned());
            return true;
        }
        if self.features.form.take().is_none() {
            return false;
        }
        self.features.profile_form_text = vec![String::new(); 7];
        self.features.prompt_text.clear();
        self.features.template_text.clear();
        unsafe {
            let _ = SetFocus(Some(self.editing_window()));
        }
        self.features_sync();
        true
    }

    fn start_name_prompt(&mut self, mode: FormMode) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let original_path = if mode == FormMode::Rename {
            if self.features.explorer.selected.len() != 1 {
                self.features.status = Some("名前を変更する項目を1つ選択してください".to_owned());
                return false;
            }
            self.features.explorer.selected.first().cloned()
        } else {
            None
        };
        let initial = original_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let title = match mode {
            FormMode::NewFile => "新しいファイル".to_owned(),
            FormMode::NewFolder => "新しいフォルダー".to_owned(),
            _ => "名前を変更".to_owned(),
        };
        self.features.prompt_text = initial.clone();
        self.features.form = Some(FormState {
            mode,
            title,
            profile_id: None,
            date_time: None,
            template_snapshot: None,
            profile_snapshot: None,
            original_path,
            guard_document: self.doc().id(),
            guard_revision: self.doc().revision(),
        });
        unsafe {
            let _ = SetWindowTextW(
                self.features.controls.prompt,
                PCWSTR(super::wide(&initial).as_ptr()),
            );
            let _ = SetFocus(Some(self.features.controls.prompt));
        }
        self.features_sync();
        true
    }

    fn save_name_prompt(&mut self) -> bool {
        let Some(form) = self.features.form.clone() else {
            return false;
        };
        if self.feature_ime_active() {
            return true;
        }
        if form.guard_document != self.doc().id() || form.guard_revision != self.doc().revision() {
            self.features.status = Some("文書が変わったため、操作を中止しました".to_owned());
            return true;
        }
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        let name = self.features.prompt_text.trim();
        if let Err(error) = validate_workspace_name(name) {
            self.features.status = Some(error);
            return true;
        }
        let result: Result<Option<PathBuf>, String> = match form.mode {
            FormMode::NewFile => workspace
                .create_file(&self.features.explorer.directory, name)
                .map(Some)
                .map_err(|error| error.to_string()),
            FormMode::NewFolder => workspace
                .create_folder(&self.features.explorer.directory, name)
                .map(|_| None)
                .map_err(|error| error.to_string()),
            FormMode::Rename => rename_workspace_item(
                workspace,
                form.original_path.as_deref().unwrap_or(Path::new("")),
                name,
                &self.open_document_paths(),
            ),
            _ => return false,
        };
        match result {
            Ok(path) => {
                self.features.form = None;
                self.features.status = None;
                if let Some(path) = &path {
                    self.features.explorer.selected = vec![path.clone()];
                }
                self.features.refresh_explorer().unwrap_or_else(|error| {
                    self.features.status =
                        Some(format!("項目は操作しましたが一覧を更新できません: {error}"));
                });
                if form.mode == FormMode::NewFile
                    && let Some(path) = path
                {
                    self.open_path(path);
                }
            }
            Err(error) => self.features.status = Some(format!("項目を操作できません: {error}")),
        }
        self.features_sync();
        true
    }

    fn open_document_paths(&self) -> Vec<PathBuf> {
        self.tabs
            .iter()
            .filter_map(|tab| tab.file.as_ref().map(|file| file.path().to_path_buf()))
            .collect()
    }

    fn copy_selected_paths(&mut self) -> bool {
        if self.features.explorer.selected.is_empty() {
            return false;
        }
        self.features.explorer.internal_copy = self.features.explorer.selected.clone();
        self.features.status = Some(format!(
            "{} 件をWorkspace内コピー対象にしました",
            self.features.explorer.internal_copy.len()
        ));
        self.features_sync();
        true
    }

    fn copy_path_payload(&mut self) -> bool {
        if self.features.explorer.selected.is_empty() {
            return false;
        }
        self.features.pending_path_copy = Some(self.features.explorer.selected.clone());
        true
    }

    fn paste_files(&mut self) -> bool {
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        let result = workspace.copy_items(
            &self.features.explorer.internal_copy,
            &self.features.explorer.directory,
        );
        match result {
            Ok(report) => {
                self.features.explorer.selected = report.completed.clone();
                self.features.status =
                    operation_status("コピー", &report.completed, &report.failures);
                if let Err(error) = self.features.refresh_explorer() {
                    self.features.status = Some(format!("コピー後の一覧更新に失敗: {error}"));
                }
            }
            Err(error) => self.features.status = Some(format!("コピーできません: {error}")),
        }
        self.features_sync();
        true
    }

    fn move_selected_to(&mut self, destination: PathBuf) {
        let Some(workspace) = self.features.workspace.as_ref() else {
            return;
        };
        let open = self.open_document_paths();
        match workspace.move_items(&self.features.explorer.selected, destination, &open) {
            Ok(report) => {
                self.features.explorer.selected.clear();
                self.features.status =
                    operation_status("移動", &report.completed, &report.failures);
                if let Err(error) = self.features.refresh_explorer() {
                    self.features.status = Some(format!("移動後の一覧更新に失敗: {error}"));
                }
            }
            Err(error) => self.features.status = Some(format!("移動できません: {error}")),
        }
        self.features_sync();
    }

    fn recycle_selected(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        let selected = self.features.explorer.selected.clone();
        if selected.is_empty() {
            return false;
        }
        let open = self.open_document_paths();
        for source in &selected {
            if is_open_or_ancestor(source, &open) {
                self.features.status = Some(format!(
                    "開いている文書またはその親フォルダーは移動できません: {}",
                    source.display()
                ));
                return true;
            }
            if let Err(error) = verify_workspace_entry(workspace, source) {
                self.features.status = Some(format!("削除対象を確認できません: {error}"));
                return true;
            }
        }
        let prompt = super::wide(&format!(
            "選択した {} 件をWindowsのごみ箱へ移動しますか？",
            selected.len()
        ));
        if unsafe {
            MessageBoxW(
                Some(self.hwnd),
                PCWSTR(prompt.as_ptr()),
                w!("ごみ箱へ移動"),
                MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
            )
        } != IDYES
        {
            return true;
        }
        match recycle_paths(&selected) {
            Ok(()) => {
                self.features.explorer.selected.clear();
                self.features.status = Some("選択項目をごみ箱へ移動しました".to_owned());
                let _ = self.features.refresh_explorer();
            }
            Err(error) => self.features.status = Some(format!("ごみ箱へ移動できません: {error}")),
        }
        self.features_sync();
        true
    }

    fn reveal_selected(&mut self) -> bool {
        let Some(path) = self.features.explorer.selected.first().cloned() else {
            return false;
        };
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        let path = match verify_workspace_entry(workspace, &path) {
            Ok(path) => path,
            Err(error) => {
                self.features.status = Some(format!("エクスプローラーで表示できません: {error}"));
                return true;
            }
        };
        let target = if path.is_dir() {
            path.clone()
        } else {
            path.parent().unwrap_or(Path::new(".")).to_path_buf()
        };
        let (file, params) = if target == path && path.is_dir() {
            (target, String::new())
        } else {
            (
                PathBuf::from("explorer.exe"),
                format!("/select,\"{}\"", path.display()),
            )
        };
        let result = unsafe {
            ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(super::wide(&file.to_string_lossy()).as_ptr()),
                PCWSTR(super::wide(&params).as_ptr()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            self.features.status = Some("エクスプローラーを起動できませんでした".to_owned());
        }
        true
    }

    fn explorer_parent(&mut self) -> bool {
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        let current = &self.features.explorer.directory;
        let relative = current.strip_prefix(workspace.root()).unwrap_or(current);
        if relative.as_os_str().is_empty() || relative == Path::new(".") {
            return true;
        }
        let parent = relative.parent().unwrap_or_else(|| Path::new("."));
        self.features.explorer.directory = parent.to_path_buf();
        self.features.explorer.selected.clear();
        if let Err(error) = self.features.refresh_explorer() {
            self.features.status = Some(format!("親フォルダーを開けません: {error}"));
        }
        self.features_sync();
        true
    }

    fn open_selected_explorer_file(&mut self) -> bool {
        let path = self
            .features
            .pending_open_file
            .take()
            .or_else(|| {
                self.features
                    .explorer
                    .selected
                    .iter()
                    .find(|path| {
                        self.features
                            .explorer
                            .entries
                            .iter()
                            .any(|entry| entry.path == **path)
                    })
                    .cloned()
            })
            .or_else(|| self.features.explorer.selected.first().cloned());
        let Some(path) = path else {
            return false;
        };
        if self.toggle_explorer_folder(&path) {
            return true;
        }
        self.open_feature_file(path);
        true
    }

    fn toggle_explorer_folder(&mut self, path: &Path) -> bool {
        if !self
            .features
            .explorer
            .entries
            .iter()
            .any(|entry| entry.path == path && entry.kind == ExplorerEntryKind::Folder)
        {
            return false;
        }
        if !self.features.explorer.expanded.remove(path) {
            self.features.explorer.expanded.insert(path.to_path_buf());
        }
        if let Err(error) = self.features.refresh_explorer() {
            self.features.status = Some(format!("フォルダーを展開できません: {error}"));
        }
        true
    }

    fn explorer_list_key(&mut self, key: u16) -> bool {
        let Some(path) = self
            .features
            .explorer
            .selected
            .iter()
            .find(|path| {
                self.features
                    .explorer
                    .entries
                    .iter()
                    .any(|entry| entry.path == **path)
            })
            .cloned()
        else {
            return false;
        };
        let Some(index) = self
            .features
            .explorer
            .entries
            .iter()
            .position(|entry| entry.path == path)
        else {
            return false;
        };
        let entry = &self.features.explorer.entries[index];
        let is_folder = entry.kind == ExplorerEntryKind::Folder;
        match key {
            KEY_RETURN => self.open_selected_explorer_file(),
            KEY_RIGHT if is_folder => {
                if !self
                    .features
                    .explorer
                    .expanded_rows
                    .get(index)
                    .copied()
                    .unwrap_or(false)
                {
                    return self.toggle_explorer_folder(&path);
                }
                true
            }
            KEY_LEFT => {
                if is_folder
                    && self
                        .features
                        .explorer
                        .expanded_rows
                        .get(index)
                        .copied()
                        .unwrap_or(false)
                {
                    return self.toggle_explorer_folder(&path);
                }
                let Some(parent) = path.parent() else {
                    return false;
                };
                if self
                    .features
                    .explorer
                    .entries
                    .iter()
                    .any(|entry| entry.path == parent && entry.kind == ExplorerEntryKind::Folder)
                {
                    self.select_visible_explorer_path(parent);
                    true
                } else {
                    false
                }
            }
            _ => false,
        }
    }

    fn select_visible_explorer_path(&mut self, path: &Path) {
        let visible = self
            .features
            .explorer
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>();
        let mut selected = vec![path.to_path_buf()];
        selected.extend(
            self.features
                .explorer
                .selected
                .iter()
                .filter(|selected| !visible.contains(*selected))
                .cloned(),
        );
        self.features.explorer.selected = selected;
    }

    fn open_another_workspace(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        if let Some(path) = unsafe { choose_folder(self.hwnd) } {
            unsafe {
                self.open_workspace_new_process(path);
            }
        }
        true
    }

    fn import_holidays(&mut self) -> bool {
        if self.feature_ime_active() {
            return true;
        }
        let Some(path) = (unsafe { choose_file(self.hwnd) }) else {
            return true;
        };
        let Some(workspace) = self.features.workspace.as_ref() else {
            return false;
        };
        match self.features.calendar.holidays.import_csv_file(
            path,
            Some((workspace, Path::new(".mdlite/.cache/holidays.csv"))),
        ) {
            Ok(report) => {
                self.features.status = report
                    .cache_error
                    .map(|error| {
                        format!(
                            "祝日{}件を読み込みましたがcache保存に失敗: {error}",
                            report.records
                        )
                    })
                    .or_else(|| Some(format!("祝日{}件を読み込みました", report.records)));
                self.features.calendar.holiday_loaded = true;
                self.features_sync();
            }
            Err(error) => self.features.status = Some(format!("祝日CSVを読み込めません: {error}")),
        }
        true
    }

    pub(super) fn features_open_link(&mut self, destination: &str) {
        if self.feature_ime_active() || !self.is_markdown_document() {
            return;
        }
        let document_path = self.tabs[self.active]
            .file
            .as_ref()
            .map(|file| file.path().to_path_buf());
        match link_target(destination, document_path.as_deref()) {
            Ok(LinkTarget::Anchor(anchor)) => {
                let projection = mdlite::projection::DisplayProjection::parse(self.doc().source());
                let outline = Outline::build(self.doc(), &projection);
                match outline.anchor(&anchor) {
                    Ok(offset) => {
                        let _ = self.doc_mut().set_selection(offset, offset);
                        unsafe {
                            self.after_input();
                        }
                    }
                    Err(error) => self.status = error.to_owned(),
                }
            }
            Ok(LinkTarget::Local { path, anchor }) => {
                let in_workspace = self
                    .features
                    .workspace
                    .as_ref()
                    .is_some_and(|workspace| path.starts_with(workspace.root()));
                if !in_workspace
                    && unsafe {
                        MessageBoxW(
                            Some(self.hwnd),
                            PCWSTR(
                                super::wide(&format!(
                                    "Workspace外の文書を開きますか？\n{}",
                                    path.display()
                                ))
                                .as_ptr(),
                            ),
                            w!("リンクを開く"),
                            MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
                        )
                    } != IDYES
                {
                    return;
                }
                self.open_path(path.clone());
                if let Some(anchor) = anchor
                    && self.tabs[self.active]
                        .file
                        .as_ref()
                        .is_some_and(|file| file.path() == path)
                {
                    let projection =
                        mdlite::projection::DisplayProjection::parse(self.doc().source());
                    let outline = Outline::build(self.doc(), &projection);
                    match outline.anchor(&anchor) {
                        Ok(offset) => {
                            let _ = self.doc_mut().set_selection(offset, offset);
                            unsafe {
                                self.after_input();
                            }
                        }
                        Err(error) => self.status = error.to_owned(),
                    }
                }
            }
            Ok(LinkTarget::External(url)) => {
                if unsafe {
                    MessageBoxW(
                        Some(self.hwnd),
                        PCWSTR(super::wide(&format!("外部リンクを開きますか？\n{url}")).as_ptr()),
                        w!("外部リンク"),
                        MB_YESNO | MB_DEFBUTTON2 | MB_ICONQUESTION,
                    )
                } == IDYES
                {
                    let result = unsafe {
                        ShellExecuteW(
                            Some(self.hwnd),
                            w!("open"),
                            PCWSTR(super::wide(&url).as_ptr()),
                            PCWSTR::null(),
                            PCWSTR::null(),
                            SW_SHOWNORMAL,
                        )
                    };
                    if result.0 as isize <= 32 {
                        self.status = "外部リンクを起動できませんでした".to_owned();
                    }
                }
            }
            Err(error) => self.status = error.to_owned(),
        }
        unsafe {
            self.refresh();
        }
    }

    pub(super) fn take_feature_path_copy_payload(&mut self) -> Option<Vec<PathBuf>> {
        self.features.pending_path_copy.take()
    }

    pub(super) fn features_control_composing(&self) -> bool {
        self.features.control_composing()
    }

    pub(super) fn feature_ime_active(&self) -> bool {
        self.composing
            || self.shell.control_composing()
            || self.features_control_composing()
            || self.search_control_composing()
            || self.git_control_composing()
    }

    fn run_feature_action(&mut self, action: FeatureAction) {
        let id = match action {
            FeatureAction::Refresh => return,
            FeatureAction::Command(id) => id,
            FeatureAction::OpenFile(path) => {
                self.features.pending_open_file = Some(path);
                4010
            }
            FeatureAction::OpenCalendarFile {
                request,
                relative_path,
            } => {
                self.open_calendar_file(*request, relative_path);
                return;
            }
            FeatureAction::OpenDaily(date) => {
                self.features.pending_daily = Some(date);
                4023
            }
        };
        unsafe {
            if PostMessageW(
                Some(self.features.controls.owner),
                WM_COMMAND,
                WPARAM(id),
                LPARAM(0),
            )
            .is_err()
            {
                self.features.status = Some("操作をwindowへ送信できませんでした".to_owned());
                if id == 4010 {
                    self.features.pending_open_file = None;
                }
                if id == 4023 {
                    self.features.pending_daily = None;
                }
            }
        }
    }
}

fn localized_label(command: usize, label: &str) -> String {
    match command {
        4000 => "検索".to_owned(),
        4001 => "新しいファイル".to_owned(),
        4002 => "新しいフォルダー".to_owned(),
        4003 => "コピー".to_owned(),
        4004 => "貼付".to_owned(),
        4005 => "改名".to_owned(),
        4006 => "削除".to_owned(),
        4007 => "パス".to_owned(),
        4008 => "表示".to_owned(),
        4009 => "上へ".to_owned(),
        4011 => "Workspaceを開く…".to_owned(),
        4020 => "前月".to_owned(),
        4021 => "次月".to_owned(),
        4022 => "今日".to_owned(),
        4023 => "日誌".to_owned(),
        4024 => "会議".to_owned(),
        4025 => "メモ".to_owned(),
        4026 => "祝日".to_owned(),
        4027 => "管理".to_owned(),
        4028 => "今日の日誌".to_owned(),
        CALENDAR_OPERATIONS_BUTTON => "操作…".to_owned(),
        4040 => "移動".to_owned(),
        4041 => "上へ".to_owned(),
        4042 => "下へ".to_owned(),
        4080 => "追加".to_owned(),
        4081 => "編集".to_owned(),
        4082 => "複製".to_owned(),
        4083 => "削除".to_owned(),
        4084 => "雛形".to_owned(),
        4085 | 4087 => "保存".to_owned(),
        4086 => "取消".to_owned(),
        4088 => "作成".to_owned(),
        _ => label.to_owned(),
    }
}

fn feature_requires_ime_end(id: usize) -> bool {
    matches!(id, 4000..=4011 | 4023..=4028 | 4040..=4042 | 4080..=4088 | 4090)
}

fn feature_writes_workspace(id: usize) -> bool {
    matches!(
        id,
        4001 | 4002 | 4004 | 4005 | 4006 | 4023..=4026 | 4028 | 4083 | 4085 | 4087 | 4088
    )
}

fn local_now() -> ProfileDateTime {
    #[repr(C)]
    struct SystemTimeLocal {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLocalTime(value: *mut SystemTimeLocal);
    }
    let mut value = SystemTimeLocal {
        year: 2000,
        month: 1,
        day_of_week: 6,
        day: 1,
        hour: 0,
        minute: 0,
        second: 0,
        milliseconds: 0,
    };
    unsafe {
        GetLocalTime(&mut value);
    }
    let date = CalendarDate::new(
        i32::from(value.year),
        i32::from(value.month),
        i32::from(value.day),
    )
    .unwrap_or(CalendarDate::new(2000, 1, 1).unwrap());
    ProfileDateTime::new(date, value.hour.min(23) as u8, value.minute.min(59) as u8).unwrap_or(
        ProfileDateTime {
            date,
            hour: 0,
            minute: 0,
        },
    )
}

fn common_profiles_path() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|path| PathBuf::from(path).join("MDLite").join("profiles.toml"))
}

fn checked_workspace_path(
    workspace: &Workspace,
    relative: &Path,
    allow_missing: bool,
) -> Result<PathBuf, String> {
    if relative.is_absolute() {
        return Err("absolute paths are not allowed".to_owned());
    }
    let mut current = workspace.root().to_path_buf();
    let components: Vec<_> = relative
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(Ok(name.to_os_string())),
            Component::CurDir => None,
            _ => Some(Err("path escapes the workspace".to_owned())),
        })
        .collect::<Result<_, _>>()?;
    for (index, name) in components.iter().enumerate() {
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(format!("reparse point in path: {}", current.display()));
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(format!(
                        "path component is not a directory: {}",
                        current.display()
                    ));
                }
            }
            Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => {
                for remaining in components.iter().skip(index + 1) {
                    current.push(remaining);
                }
                return Ok(current);
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    let canonical_root = fs::canonicalize(workspace.root()).map_err(|error| error.to_string())?;
    match fs::canonicalize(&current) {
        Ok(canonical) if path_within(&canonical_root, &canonical) => Ok(canonical),
        Ok(_) => Err("resolved path is outside the workspace".to_owned()),
        Err(error) if allow_missing && error.kind() == std::io::ErrorKind::NotFound => Ok(current),
        Err(error) => Err(error.to_string()),
    }
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & 0x400 != 0
}

fn path_within(root: &Path, candidate: &Path) -> bool {
    let root = root
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    let candidate = candidate.to_string_lossy().to_ascii_lowercase();
    candidate == root
        || candidate.starts_with(&format!("{root}\\"))
        || candidate.starts_with(&format!("{root}/"))
}

fn validate_workspace_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() || name == "." || name == ".." {
        return Err("名前を入力してください".to_owned());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Err("名前に使用できない文字が含まれています".to_owned());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("名前の末尾に空白やピリオドは使えません".to_owned());
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
    {
        return Err("Windowsで予約された名前です".to_owned());
    }
    Ok(())
}

fn same_path_case_insensitive(left: &Path, right: &Path) -> bool {
    fs::canonicalize(left)
        .unwrap_or_else(|_| left.to_path_buf())
        .to_string_lossy()
        .eq_ignore_ascii_case(
            &fs::canonicalize(right)
                .unwrap_or_else(|_| right.to_path_buf())
                .to_string_lossy(),
        )
}

fn is_open_or_ancestor(path: &Path, open_documents: &[PathBuf]) -> bool {
    let canonical = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let lower = canonical.to_string_lossy().to_ascii_lowercase();
    open_documents.iter().any(|open| {
        let open = fs::canonicalize(open).unwrap_or_else(|_| open.clone());
        let open_lower = open.to_string_lossy().to_ascii_lowercase();
        open_lower == lower
            || open_lower.starts_with(&format!("{}\\", lower.trim_end_matches('\\')))
    })
}

fn verify_workspace_entry(workspace: &Workspace, path: &Path) -> Result<PathBuf, String> {
    let relative = path
        .strip_prefix(workspace.root())
        .map_err(|_| "path is outside the workspace".to_owned())?;
    let canonical = checked_workspace_path(workspace, relative, false)?;
    let parent = canonical
        .parent()
        .ok_or_else(|| "path has no parent".to_owned())?;
    let listing = workspace
        .list_directory(parent)
        .map_err(|error| error.to_string())?;
    if listing
        .entries
        .iter()
        .any(|entry| same_path_case_insensitive(&entry.path, &canonical))
    {
        Ok(canonical)
    } else {
        Err("item is protected, unsupported, or changed since selection".to_owned())
    }
}

fn rename_workspace_item(
    workspace: &Workspace,
    source: &Path,
    name: &str,
    open_documents: &[PathBuf],
) -> Result<Option<PathBuf>, String> {
    validate_workspace_name(name)?;
    let source = verify_workspace_entry(workspace, source)?;
    if same_path_case_insensitive(&source, workspace.root()) {
        return Err("workspace root cannot be renamed".to_owned());
    }
    if is_open_or_ancestor(&source, open_documents) {
        return Err("an open document or its parent folder cannot be renamed".to_owned());
    }
    let parent = source
        .parent()
        .ok_or_else(|| "item has no parent folder".to_owned())?;
    let target = parent.join(name);
    if fs::symlink_metadata(&target).is_ok() {
        return Err(format!("destination already exists: {}", target.display()));
    }
    let relative = source
        .strip_prefix(workspace.root())
        .map_err(|_| "path is outside workspace".to_owned())?;
    if relative.components().next().is_some_and(|component| {
        component
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(".mdlite")
    }) {
        return Err("workspace data cannot be renamed here".to_owned());
    }
    fs::rename(&source, &target).map_err(|error| error.to_string())?;
    Ok(Some(target))
}

fn operation_status(
    label: &str,
    completed: &[PathBuf],
    failures: &[mdlite::workspace::OperationFailure],
) -> Option<String> {
    if failures.is_empty() {
        return Some(format!("{label}: {} 件完了", completed.len()));
    }
    let partial = if completed.is_empty() {
        String::new()
    } else {
        format!("{} 件完了、", completed.len())
    };
    Some(format!(
        "{label}: {partial}{} 件失敗 — {}",
        failures.len(),
        failures[0].message
    ))
}

fn profile_form_fields(profile: &ProfileDefinition) -> Vec<String> {
    vec![
        profile.id.clone(),
        profile.name.clone(),
        profile.directory.to_string_lossy().into_owned(),
        profile.filename.to_string_lossy().into_owned(),
        profile.template_path.to_string_lossy().into_owned(),
        match profile.collision {
            ProfileCollision::OpenExisting => "open".to_owned(),
            ProfileCollision::Sequence => "sequence".to_owned(),
        },
        profile
            .inputs
            .iter()
            .map(|input| {
                format!(
                    "{}|{}|{}|{}",
                    input.id,
                    input.label,
                    if input.required {
                        "required"
                    } else {
                        "optional"
                    },
                    input.default_value
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    ]
}

fn parse_profile_fields(fields: &[String]) -> Result<ProfileDefinition, ProfileError> {
    if fields.len() < 7 {
        return Err(ProfileError::InvalidDefinition(
            "フォームの項目が不足しています".to_owned(),
        ));
    }
    let collision = match fields[5].trim().to_ascii_lowercase().as_str() {
        "open" | "existing" | "既存を開く" => ProfileCollision::OpenExisting,
        "sequence" | "number" | "連番" => ProfileCollision::Sequence,
        _ => {
            return Err(ProfileError::InvalidDefinition(
                "重複時の動作には open または sequence を指定してください".to_owned(),
            ));
        }
    };
    let inputs = fields[6]
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let parts: Vec<_> = line.splitn(4, '|').collect();
            if parts.len() != 4 {
                return Err(ProfileError::InvalidDefinition(
                    "入力行は id|名前|required|初期値 の形式にしてください".to_owned(),
                ));
            }
            let required = match parts[2].trim().to_ascii_lowercase().as_str() {
                "required" | "true" | "yes" | "必須" => true,
                "optional" | "false" | "no" | "任意" => false,
                _ => {
                    return Err(ProfileError::InvalidDefinition(
                        "入力のrequired欄は required または optional です".to_owned(),
                    ));
                }
            };
            Ok(ProfileInputDefinition {
                id: parts[0].trim().to_owned(),
                label: parts[1].trim().to_owned(),
                required,
                default_value: parts[3].to_owned(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ProfileDefinition {
        id: fields[0].trim().to_owned(),
        name: fields[1].trim().to_owned(),
        directory: PathBuf::from(fields[2].trim()),
        filename: PathBuf::from(fields[3].trim()),
        template_path: PathBuf::from(fields[4].trim()),
        collision,
        inputs,
    })
}

fn parse_profile_values(text: &str) -> Result<ProfileValues, String> {
    let mut values = ProfileValues::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let (id, value) = line
            .split_once('=')
            .or_else(|| line.split_once('|'))
            .ok_or_else(|| "入力行は id=値 の形式にしてください".to_owned())?;
        let id = id.trim();
        if id.is_empty() {
            return Err("入力IDが空です".to_owned());
        }
        if values.insert(id.to_owned(), value.to_owned()).is_some() {
            return Err(format!("入力IDが重複しています: {id}"));
        }
    }
    Ok(values)
}

fn available_duplicate_id(profiles: &[ProfileDefinition], source_id: &str) -> String {
    let base = format!("{source_id}-copy");
    if !profiles.iter().any(|profile| profile.id == base) {
        return base;
    }
    for sequence in 2..=999 {
        let candidate = format!("{base}-{sequence}");
        if !profiles.iter().any(|profile| profile.id == candidate) {
            return candidate;
        }
    }
    format!(
        "{base}-{}",
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    )
}

unsafe fn choose_path(owner: HWND, folders: bool) -> Option<PathBuf> {
    unsafe {
        let dialog: windows::core::Result<IFileOpenDialog> =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER);
        let dialog = dialog.ok()?;
        let options = dialog.GetOptions().ok()?;
        let mut flags = options | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR;
        if folders {
            flags |= FOS_PICKFOLDERS;
        }
        if dialog.SetOptions(flags).is_err() || dialog.Show(Some(owner)).is_err() {
            return None;
        }
        let item = dialog.GetResult().ok()?;
        let display = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = display.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(display.0.cast()));
        path
    }
}

unsafe fn choose_folder(owner: HWND) -> Option<PathBuf> {
    unsafe { choose_path(owner, true) }
}
unsafe fn choose_file(owner: HWND) -> Option<PathBuf> {
    unsafe { choose_path(owner, false) }
}

fn recycle_paths(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("削除対象がありません".to_owned());
    }
    let mut from = Vec::<u16>::new();
    for path in paths {
        let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
        from.extend(path.to_string_lossy().encode_utf16());
        from.push(0);
    }
    from.push(0);
    let flags = (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT).0 as u16;
    let mut operation = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(from.as_ptr()),
        fFlags: flags,
        ..Default::default()
    };
    let result = unsafe { SHFileOperationW(&mut operation) };
    if result != 0 {
        return Err(std::io::Error::from_raw_os_error(result).to_string());
    }
    if operation.fAnyOperationsAborted.as_bool() {
        return Err(
            "Windows reports that the recycle operation was cancelled or incomplete".to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::{
        Controls::{ODS_FOCUS, ODS_HOTLIGHT, ODS_SELECTED},
        Input::KeyboardAndMouse::EnableWindow,
    };

    fn explorer_list_for_test(owner: HWND) -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("LISTBOX"),
                w!(""),
                WS_CHILD | WINDOW_STYLE(0x0001 | 0x0010 | 0x0040 | 0x0100 | 0x0800),
                0,
                0,
                180,
                40,
                Some(owner),
                None,
                None,
                None,
            )
            .unwrap()
        }
    }

    struct ExplorerPaintFixture {
        main: super::super::tests::MessagePumpFixture,
        list: HWND,
        dc: windows::Win32::Graphics::Gdi::HDC,
        bitmap: windows::Win32::Graphics::Gdi::HBITMAP,
        previous: windows::Win32::Graphics::Gdi::HGDIOBJ,
    }
    impl ExplorerPaintFixture {
        fn new() -> Self {
            use windows::Win32::Graphics::Gdi::*;
            let mut app = super::super::tests::composing_test_app("# 原文\n本文 e\u{301} 👩‍💻\n");
            app.clear_composition();
            let main = super::super::tests::MessagePumpFixture::new(app);
            let list;
            unsafe {
                let mut app = main.state.app.borrow_mut();
                app.shell.enabled = true;
                app.shell.main_window = main.hwnd;
                app.features.controls.owner = main.hwnd;
                list = shell::control(
                    main.hwnd,
                    w!("LISTBOX"),
                    "Explorer paint",
                    EXPLORER_LIST,
                    WINDOW_STYLE(
                        (LBS_OWNERDRAWFIXED
                            | LBS_HASSTRINGS
                            | LBS_EXTENDEDSEL
                            | LBS_NOINTEGRALHEIGHT) as u32,
                    ),
                );
                app.features.controls.explorer_list = list;
                app.features.explorer.entries.push(ExplorerEntry {
                    name: "fixture_日本語.md".into(),
                    path: PathBuf::from("synthetic/fixture_日本語.md"),
                    kind: ExplorerEntryKind::File,
                });
                app.features.explorer.depths.push(0);
                app.features_sync();
                SetWindowPos(list, None, 0, 0, 160, 32, SWP_NOZORDER | SWP_NOACTIVATE).unwrap();
            }
            unsafe {
                let source = GetDC(Some(main.hwnd));
                let dc = CreateCompatibleDC(Some(source));
                let bitmap = CreateCompatibleBitmap(source, 160, 32);
                ReleaseDC(Some(main.hwnd), source);
                let previous = SelectObject(dc, bitmap.into());
                Self {
                    main,
                    list,
                    dc,
                    bitmap,
                    previous,
                }
            }
        }
        fn item(&self) -> DRAWITEMSTRUCT {
            DRAWITEMSTRUCT {
                CtlType: windows::Win32::UI::Controls::ODT_LISTBOX,
                CtlID: EXPLORER_LIST as u32,
                itemID: 0,
                hwndItem: self.list,
                hDC: self.dc,
                itemData: 0,
                rcItem: RECT {
                    left: 0,
                    top: 0,
                    right: 160,
                    bottom: 32,
                },
                ..Default::default()
            }
        }
        fn draw(&self, owner: HWND, id: usize, item: &DRAWITEMSTRUCT) -> LRESULT {
            unsafe {
                super::super::window_proc(
                    owner,
                    WM_DRAWITEM,
                    WPARAM(id),
                    LPARAM((item as *const DRAWITEMSTRUCT) as isize),
                )
            }
        }
    }
    impl Drop for ExplorerPaintFixture {
        fn drop(&mut self) {
            use windows::Win32::Graphics::Gdi::*;
            unsafe {
                SelectObject(self.dc, self.previous);
                let _ = DeleteObject(self.bitmap.into());
                let _ = DeleteDC(self.dc);
            }
        }
    }

    #[test]
    fn explorer_runtime_snapshot_paints_reentrant_reset_and_rejects_foreign_routes() {
        use windows::Win32::Graphics::Gdi::GetPixel;
        struct Probe {
            slot: ExplorerPaintSlot,
            calls: Cell<usize>,
            released: Cell<bool>,
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
                if msg == LB_GETTEXTLEN {
                    let probe = &*(data as *const Probe);
                    probe.calls.set(probe.calls.get() + 1);
                    probe
                        .released
                        .set(probe.released.get() && probe.slot.try_borrow_mut().is_ok());
                }
                DefSubclassProc(hwnd, msg, wp, lp)
            }
        }
        let fixture = ExplorerPaintFixture::new();
        let probe = Probe {
            slot: Rc::clone(&fixture.main.state.explorer_paint),
            calls: Cell::new(0),
            released: Cell::new(true),
        };
        unsafe {
            SetWindowSubclass(
                fixture.list,
                Some(observe),
                776,
                (&probe as *const Probe) as usize,
            )
            .unwrap();
        }
        let mut item = fixture.item();
        let app = fixture.main.state.app.borrow_mut();
        let before = (
            app.doc().source().to_owned(),
            app.doc().revision(),
            app.doc().is_dirty(),
            app.doc().undo_count(),
            app.doc().redo_count(),
            app.doc().selection(),
        );
        assert_eq!(
            fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
            LRESULT(1)
        );
        assert!(probe.calls.get() > 0 && probe.released.get());
        assert!(!fixture.main.state.deferred_control_redraw.get());
        assert_ne!(
            fixture.draw(fixture.main.hwnd, EXPLORER_LIST + 1, &item),
            LRESULT(1)
        );
        item.CtlID += 1;
        assert_ne!(
            fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
            LRESULT(1)
        );
        item.CtlID -= 1;
        item.CtlType = ODT_BUTTON;
        assert_ne!(
            fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
            LRESULT(1)
        );
        item.CtlType = windows::Win32::UI::Controls::ODT_LISTBOX;
        unsafe {
            SetWindowLongPtrW(fixture.list, GWLP_ID, (EXPLORER_LIST + 1) as isize);
            assert_ne!(
                fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
                LRESULT(1)
            );
            SetWindowLongPtrW(fixture.list, GWLP_ID, EXPLORER_LIST as isize);
            let other = super::super::tests::hidden_test_window_size(160, 32);
            assert_ne!(fixture.draw(other, EXPLORER_LIST, &item), LRESULT(1));
            DestroyWindow(other).unwrap();
            shell::send(fixture.list, LB_RESETCONTENT, WPARAM(0), LPARAM(0));
            assert_eq!(
                fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
                LRESULT(1)
            );
            let background = shell::rgb(app.shell.tokens.panel);
            assert!((0..32).all(|y| (0..160).all(|x| GetPixel(fixture.dc, x, y) == background)));
            let _ =
                windows::Win32::UI::Shell::RemoveWindowSubclass(fixture.list, Some(observe), 776);
        }
        assert_eq!(
            (
                app.doc().source().to_owned(),
                app.doc().revision(),
                app.doc().is_dirty(),
                app.doc().undo_count(),
                app.doc().redo_count(),
                app.doc().selection()
            ),
            before
        );
    }

    #[test]
    fn explorer_runtime_snapshot_refreshes_content_cache_theme_and_font() {
        let fixture = ExplorerPaintFixture::new();
        let original = fixture.main.state.explorer_paint.borrow().clone().unwrap();
        {
            let mut app = fixture.main.state.app.borrow_mut();
            app.features_refresh_explorer_paint(false);
        }
        assert!(Rc::ptr_eq(
            &original,
            fixture.main.state.explorer_paint.borrow().as_ref().unwrap()
        ));
        let mut font = super::super::control_style::UiFont::new(96, 28.).unwrap();
        font.apply_to(fixture.main.hwnd).unwrap();
        {
            let mut app = fixture.main.state.app.borrow_mut();
            app.features_refresh_explorer_paint(false);
        }
        let changed_font = fixture.main.state.explorer_paint.borrow().clone().unwrap();
        assert!(!Rc::ptr_eq(&original, &changed_font));
        {
            let mut app = fixture.main.state.app.borrow_mut();
            app.shell.tokens.panel = super::super::color(0.2, 0.3, 0.4);
            app.features_refresh_explorer_paint(false);
        }
        let changed_theme = fixture.main.state.explorer_paint.borrow().clone().unwrap();
        assert!(!Rc::ptr_eq(&changed_font, &changed_theme));
        unsafe {
            shell::send(
                fixture.list,
                LB_ADDSTRING,
                WPARAM(0),
                LPARAM(super::super::wide("  fresh.cache_generation_test").as_ptr() as isize),
            );
        }
        let mut item = fixture.item();
        item.itemID = 1;
        assert_eq!(
            fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &item),
            LRESULT(1)
        );
        let changed_icons = fixture.main.state.explorer_paint.borrow().clone().unwrap();
        assert!(!Rc::ptr_eq(&changed_theme, &changed_icons));
        {
            let mut app = fixture.main.state.app.borrow_mut();
            assert_eq!(
                app.features.explorer_paint_key.unwrap().icons,
                app.explorer_icons.generation()
            );
            app.features_refresh_explorer_paint(false);
            assert!(Rc::ptr_eq(
                &changed_icons,
                fixture.main.state.explorer_paint.borrow().as_ref().unwrap()
            ));
            app.features.explorer_list_dirty = true;
            app.features_sync();
        }
        assert!(!Rc::ptr_eq(
            &changed_icons,
            fixture.main.state.explorer_paint.borrow().as_ref().unwrap()
        ));
        {
            let _app = fixture.main.state.app.borrow_mut();
            assert_eq!(
                fixture.draw(fixture.main.hwnd, EXPLORER_LIST, &fixture.item()),
                LRESULT(1)
            );
        }
        unsafe {
            DestroyWindow(fixture.main.hwnd).unwrap();
        }
        assert!(fixture.main.state.explorer_paint.borrow().is_none());
    }

    fn send_explorer_key(app: &mut App, list: HWND, key: u16) -> bool {
        let message = MSG {
            hwnd: list,
            message: WM_KEYDOWN,
            wParam: WPARAM(key as usize),
            ..Default::default()
        };
        unsafe { app.features_message(&message) }
    }

    fn profiles_with_required_topic() -> Vec<ProfileDefinition> {
        default_profiles()
            .into_iter()
            .map(|mut profile| {
                profile.inputs.push(ProfileInputDefinition {
                    id: "topic".to_owned(),
                    label: "Topic".to_owned(),
                    default_value: String::new(),
                    required: true,
                });
                profile
            })
            .collect()
    }

    fn calendar_creation_app(workspace: &Workspace) -> App {
        let mut app = super::super::tests::composing_test_app("# heading\n");
        app.composing = false;
        app.features.workspace = Some(workspace.clone());
        app.features.workspace_identity = Some(workspace.identity().clone());
        app.features.profiles.effective = profiles_with_required_topic();
        app
    }

    fn assert_datetime_between_local_now(
        actual: ProfileDateTime,
        before: ProfileDateTime,
        after: ProfileDateTime,
    ) {
        assert!(actual.date == before.date || actual.date == after.date);
        if before.date == after.date {
            let before_minute = (before.hour, before.minute);
            let actual_minute = (actual.hour, actual.minute);
            let after_minute = (after.hour, after.minute);
            assert!(before_minute <= actual_minute && actual_minute <= after_minute);
        }
    }

    #[test]
    fn calendar_weekend_holiday_and_adjacent_month_colors_follow_date_marks() {
        let march = CalendarDate::new(2027, 3, 1).unwrap();
        let holiday = CalendarDate::new(2027, 3, 20).unwrap();
        let cells = decorate_month(
            month_grid(march).unwrap(),
            march,
            None,
            None,
            None,
            &BTreeSet::from([holiday]),
            &BTreeSet::new(),
        );
        let tokens = shell::Tokens::default();
        let cell = |month, day| {
            cells
                .iter()
                .find(|cell| cell.date == CalendarDate::new(2027, month, day))
                .unwrap()
        };
        let red = calendar_cell_color(tokens, cell(3, 20));
        assert!(cell(3, 20).marks.saturday && cell(3, 20).marks.holiday);
        assert!(red.r > red.b, "a Saturday holiday uses the red holiday hue");
        let sunday = calendar_cell_color(tokens, cell(3, 21));
        assert_eq!((red.r, red.g, red.b), (sunday.r, sunday.g, sunday.b));
        let saturday = calendar_cell_color(tokens, cell(3, 27));
        let adjacent = calendar_cell_color(tokens, cell(4, 3));
        assert!(saturday.b > saturday.r && adjacent.b > adjacent.r);
        assert!(
            adjacent.b < saturday.b,
            "adjacent months dim while retaining weekend hue"
        );
        let mut selected = *cell(4, 3);
        selected.marks.selected = true;
        let selected_ink = calendar_cell_color(tokens, &selected);
        assert_eq!(
            selected_ink.b, saturday.b,
            "selection retains full text contrast"
        );
        let high_contrast = shell::Tokens {
            high_contrast: true,
            weekend_red: tokens.foreground,
            weekend_blue: tokens.foreground,
            ..tokens
        };
        let ink = calendar_cell_color(high_contrast, cell(4, 3));
        assert_eq!(ink.r, high_contrast.foreground.r);
    }

    #[test]
    fn calendar_layout_reserves_grid_and_rows_at_compact_and_default_heights() {
        for body_height in [180.0, 200.0, 234.0, 250.0, 270.0, 273.0, 320.0] {
            let layout = calendar_layout(body_height);
            assert!(layout.grid_top + 5.0 * layout.week_step + layout.cell_height <= body_height);
            assert!(
                layout.rows_top + layout.visible_rows as f32 * CALENDAR_ROW_HEIGHT <= body_height
            );
        }
        assert_eq!(calendar_layout(270.0).grid_top, CALENDAR_COMPACT_GRID_TOP);
        assert_eq!(calendar_layout(270.0).visible_rows, 2);
        assert_eq!(calendar_layout(250.0).status_height, 0.0);
        assert_eq!(calendar_layout(273.0).grid_top, CALENDAR_GRID_TOP);
        assert_eq!(calendar_layout(320.0).visible_rows, 3);
    }

    #[test]
    fn calendar_day_labels_keep_both_markers_on_one_bounded_line() {
        assert_eq!(calendar_day_label(31, true, true), "31•祝");
        assert_eq!(calendar_day_label(31, true, false), "31•");
        assert_eq!(calendar_day_label(31, false, true), "31祝");

        for body in [
            Rect {
                x: 13.0,
                y: 7.0,
                width: 180.0,
                height: 180.0,
            },
            Rect {
                x: 0.0,
                y: 0.0,
                width: 260.0,
                height: 234.0,
            },
            Rect {
                x: 6.0,
                y: 12.0,
                width: 320.0,
                height: 320.0,
            },
        ] {
            let layout = calendar_layout(body.height);
            let right = body.x + body.width;
            let bottom = body.y + body.height;
            for index in 0..42 {
                let Some(bounds) = calendar_cell_draw_bounds(body, layout, index) else {
                    continue;
                };
                assert!(bounds.cell.x >= body.x);
                assert!(bounds.cell.y >= body.y);
                assert!(bounds.cell.x + bounds.cell.width <= right);
                assert!(bounds.cell.y + bounds.cell.height <= bottom);
                assert!(bounds.label.x >= bounds.cell.x);
                assert!(bounds.label.y >= bounds.cell.y);
                assert!(bounds.label.x + bounds.label.width <= bounds.cell.x + bounds.cell.width);
                assert!(bounds.label.y + bounds.label.height <= bounds.cell.y + bounds.cell.height);
                let size = calendar_day_font_size("31•祝", bounds.label.width);
                assert!(size > 0.0 && size <= CALENDAR_DAY_FONT_SIZE);
                assert!(size < bounds.cell.height);
            }
        }
    }

    #[test]
    fn feature_owner_draw_requires_the_registered_parent_and_button_identity() {
        let owner = super::super::tests::hidden_test_window_size(240, 80);
        let other_owner = super::super::tests::hidden_test_window_size(240, 80);
        unsafe {
            let button = shell::control(
                owner,
                w!("BUTTON"),
                "検索",
                CONTROL_BASE,
                WINDOW_STYLE(BUTTON_OWNERDRAW | BUTTON_NOTIFY),
            );
            let wrong_id_button = shell::control(
                owner,
                w!("BUTTON"),
                "別のコントロール",
                CONTROL_BASE + 1,
                WINDOW_STYLE(BUTTON_OWNERDRAW | BUTTON_NOTIFY),
            );
            let mut features = FeaturePanes::default();
            features.controls.owner = owner;
            features
                .controls
                .buttons
                .push((CONTROL_BASE, 4000, button, PanelId::Explorer));
            let mut item = DRAWITEMSTRUCT {
                CtlType: ODT_BUTTON,
                CtlID: CONTROL_BASE as u32,
                hwndItem: button,
                ..Default::default()
            };

            let drawn = features
                .owner_draw_button(owner, CONTROL_BASE, &item)
                .expect("the registered Feature child belongs to this owner");
            assert_eq!(drawn.label, "検索");
            assert!(drawn.enabled);

            item.itemState.0 = ODS_HOTLIGHT.0 | ODS_SELECTED.0 | ODS_FOCUS.0;
            let drawn = features
                .owner_draw_button(owner, CONTROL_BASE, &item)
                .unwrap();
            assert!(drawn.hovered && drawn.pressed && drawn.focused);

            let _ = EnableWindow(button, false);
            assert!(
                !features
                    .owner_draw_button(owner, CONTROL_BASE, &item)
                    .unwrap()
                    .enabled
            );
            let _ = EnableWindow(button, true);

            assert!(
                features
                    .owner_draw_button(other_owner, CONTROL_BASE, &item)
                    .is_none()
            );
            assert!(
                features
                    .owner_draw_button(owner, CONTROL_BASE + 1, &item)
                    .is_none()
            );
            item.CtlType = windows::Win32::UI::Controls::ODT_LISTBOX;
            assert!(
                features
                    .owner_draw_button(owner, CONTROL_BASE, &item)
                    .is_none()
            );
            item.CtlType = ODT_BUTTON;
            item.hwndItem = wrong_id_button;
            features.controls.buttons[0].2 = wrong_id_button;
            assert!(
                features
                    .owner_draw_button(owner, CONTROL_BASE, &item)
                    .is_none()
            );

            features.controls.owner = other_owner;
            item.hwndItem = button;
            features.controls.buttons[0].2 = button;
            assert!(
                features
                    .owner_draw_button(other_owner, CONTROL_BASE, &item)
                    .is_none()
            );

            let _ = DestroyWindow(button);
            let _ = DestroyWindow(wrong_id_button);
            let _ = DestroyWindow(owner);
            let _ = DestroyWindow(other_owner);
        }
    }

    #[test]
    fn calendar_toolbar_bounds_do_not_overlap_month_or_weekday_headers() {
        let mut features = FeaturePanes::default();
        let mut calendar_commands = CALENDAR_BUTTONS.to_vec();
        calendar_commands.push(CALENDAR_OPERATIONS_BUTTON);
        features.controls.buttons = calendar_commands
            .iter()
            .enumerate()
            .map(|(index, &command)| {
                (
                    CONTROL_BASE + index,
                    command,
                    HWND::default(),
                    PanelId::Calendar,
                )
            })
            .collect();
        let overlaps = |a: Rect, b: Rect| {
            a.x < b.x + b.width
                && a.x + a.width > b.x
                && a.y < b.y + b.height
                && a.y + a.height > b.y
        };

        for body_height in [180.0, 200.0, 234.0, 250.0, 270.0, 273.0, 320.0] {
            let body = Rect {
                x: 0.0,
                y: 0.0,
                width: 260.0,
                height: body_height,
            };
            let controls = FeatureControlLayout {
                dpi: 1.0,
                editor: Rect::default(),
                explorer: None,
                calendar: Some(body),
                form: None,
            };
            let buttons = features.toolbar_layout(&controls);
            let compact = body_height < CALENDAR_COMPACT_HEIGHT;
            assert_eq!(buttons.len(), if compact { 1 } else { 3 });
            let layout = calendar_layout(body_height);
            let month = Rect {
                x: 8.0,
                y: (layout.grid_top - 41.0).max(0.0),
                width: body.width - 16.0,
                height: 14.0,
            };
            let weekdays = Rect {
                x: 5.0,
                y: (layout.grid_top - 18.0).max(0.0),
                width: body.width - 10.0,
                height: 11.0,
            };
            if compact {
                let button = buttons.values().next().unwrap();
                assert_eq!(button.width, 96.0);
                assert_eq!(localized_label(CALENDAR_OPERATIONS_BUTTON, ""), "操作…");
            }
            for button in buttons.values() {
                assert!(button.y + button.height <= body_height);
                assert!(
                    !overlaps(*button, month),
                    "button {button:?} overlaps month at {body_height}"
                );
                assert!(
                    !overlaps(*button, weekdays),
                    "button {button:?} overlaps weekdays at {body_height}"
                );
            }
            if compact {
                assert!(buttons.values().all(|button| button.y == 3.0));
            } else {
                assert!(buttons.values().any(|button| button.y == 3.0));
                assert!(buttons.values().all(|button| button.y == 3.0));
            }
        }
        assert!(!CALENDAR_BUTTONS.contains(&4007));
        assert_eq!(
            CALENDAR_BUTTONS,
            [4020, 4021, 4022, 4023, 4024, 4025, 4026, 4027]
        );
    }

    #[test]
    fn profile_manager_rows_and_seven_field_forms_fit_above_their_buttons() {
        let panel = Rect {
            x: 10.0,
            y: 20.0,
            width: 260.0,
            height: 320.0,
        };
        let button_top = panel.y + panel.height - PROFILE_BUTTON_FOOTER;
        let content_bottom = button_top - 6.0;
        for index in 0..PROFILE_FIELDS.len() {
            let field = profile_form_field_rect(panel, index);
            let label = Rect {
                x: panel.x + 6.0,
                y: field.y,
                width: 92.0,
                height: field.height,
            };
            let input = Rect {
                x: panel.x + 100.0,
                y: field.y,
                width: panel.width - 106.0,
                height: field.height,
            };
            assert!(field.y + field.height <= content_bottom);
            assert!(label.x + label.width <= panel.x + panel.width);
            assert!(input.x + input.width <= panel.x + panel.width);
        }
        let inputs = profile_inputs_rect(panel);
        assert!(inputs.y + inputs.height <= content_bottom);

        let mut features = FeaturePanes::default();
        features.profiles.manager_open = true;
        features.controls.buttons = PROFILE_BUTTONS
            .iter()
            .enumerate()
            .map(|(index, &command)| {
                (
                    CONTROL_BASE + index,
                    command,
                    HWND::default(),
                    PanelId::Calendar,
                )
            })
            .collect();
        let controls = FeatureControlLayout {
            dpi: 1.0,
            editor: Rect::default(),
            explorer: None,
            calendar: Some(panel),
            form: None,
        };
        let buttons = features.toolbar_layout(&controls);
        let profile_layout = profile_list_layout(panel.height);
        assert_eq!(buttons.len(), PROFILE_BUTTONS.len());
        assert!(profile_layout.visible_rows > 0);
        assert!(
            panel.y
                + profile_layout.rows_top
                + profile_layout.visible_rows as f32 * CALENDAR_ROW_HEIGHT
                <= button_top
        );
        assert!(buttons.values().all(|button| {
            button.y >= button_top
                && button.y + button.height <= panel.y + panel.height
                && button.x >= panel.x
                && button.x + button.width <= panel.x + panel.width
        }));
    }

    #[test]
    fn outline_toolbar_moves_whole_parent_sections_in_one_undo() {
        let source = "# A\n## child\nchild body\n# B\nb\n";
        for (command, selected, expected, selected_label) in [
            (4042, 0, "# B\nb\n# A\n## child\nchild body\n", "A"),
            (4041, 2, "# B\nb\n# A\n## child\nchild body\n", "B"),
        ] {
            let mut app = super::super::tests::composing_test_app(source);
            app.composing = false;
            let projection = app.cached_projection();
            app.features.outline = Some(Outline::build(app.doc(), &projection));
            app.features.outline_document = app.doc().id();
            app.features.outline_revision = app.doc().revision();
            app.features.outline_selected = selected;
            let undo_count = app.doc().undo_count();

            assert!(unsafe { app.features_command(command) });

            assert_eq!(app.doc().source(), expected);
            assert_eq!(app.doc().undo_count(), undo_count + 1);
            let outline = app.features.outline.as_ref().unwrap();
            assert_eq!(
                outline.headings[app.features.outline_selected].label,
                selected_label
            );
            app.doc_mut().undo();
            assert_eq!(app.doc().source(), source);
        }
    }

    #[test]
    fn outline_drag_escape_from_header_cancels_capture_without_edit() {
        let mut app = super::super::tests::composing_test_app("# A\nbody\n# B\nother\n");
        app.composing = false;
        let projection = app.cached_projection();
        let outline = Outline::build(app.doc(), &projection);
        let source = app.doc().source().to_owned();
        let undo_count = app.doc().undo_count();
        app.features.outline_drag = Some((outline, 0, 10.0, 10.0, true, Some(1)));

        let owner = super::super::tests::hidden_test_window_size(220, 80);
        app.hwnd = owner;
        let header = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("BUTTON"),
                w!("Outline header"),
                WS_CHILD,
                0,
                0,
                120,
                24,
                Some(owner),
                None,
                None,
                None,
            )
            .unwrap()
        };
        unsafe {
            let _ = super::super::SetCapture(header);
        }
        assert_eq!(unsafe { super::super::GetCapture() }, header);
        let message = MSG {
            hwnd: header,
            message: WM_KEYDOWN,
            wParam: WPARAM(KEY_ESCAPE as usize),
            ..Default::default()
        };

        assert!(unsafe { app.features_message(&message) });
        assert!(app.features.outline_drag.is_none());
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().undo_count(), undo_count);
        assert_eq!(unsafe { super::super::GetCapture() }, HWND::default());

        unsafe {
            let _ = DestroyWindow(header);
            let _ = DestroyWindow(owner);
        }
    }

    #[test]
    fn feature_buttons_use_shared_command_enablement_gates() {
        let mut app = super::super::tests::composing_test_app("# A\n# B\n");
        app.composing = false;
        let projection = app.cached_projection();
        app.features.outline = Some(Outline::build(app.doc(), &projection));
        assert!(app.features_enabled(4041));

        app.composing = true;
        assert!(!app.features_enabled(4041));
        app.composing = false;
        app.doc_mut().replace_range(4..8, "").unwrap();
        app.features_sync();
        assert!(!app.features_enabled(4041));
    }

    #[test]
    fn calendar_discards_async_result_after_profile_change() {
        let root = std::env::temp_dir().join(format!(
            "mdlite-calendar-profile-generation-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let date = local_now().date;
        let mut features = FeaturePanes::default();
        features.workspace = Some(workspace.clone());
        features.workspace_identity = Some(workspace.identity().clone());
        features.profiles.effective = default_profiles();
        let mut request = CalendarListRequest::new(&workspace, 1, date).unwrap();
        request.daily_profile = Some(features.profiles.effective[0].clone());
        features.calendar.list_generation = 1;
        features.calendar.interaction.set_selection(Some(date));
        let (sender, receiver) = mpsc::channel();
        sender.send(request.loading_result()).unwrap();
        features.calendar.worker = Some(CalendarWorker {
            request,
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
            handle: thread::spawn(|| {}),
        });
        features.profiles.effective[0].directory = "Changed".into();
        assert!(features.poll_workers().is_empty());
        assert!(features.calendar.listing.is_none());
        drop(features);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn calendar_click_requires_os_doubleclick_same_target_and_reset() {
        let mut features = FeaturePanes::default();
        let date = CalendarDate::new(2026, 10, 19).unwrap();
        let grid = month_grid(date).unwrap();
        features.calendar.current_month = Some(grid);
        let index = grid
            .cells
            .iter()
            .position(|cell| *cell == Some(date))
            .unwrap();
        let width = 260.0;
        let cell_width = (width - 10.0) / 7.0;
        let layout = calendar_layout(320.0);
        let x = 5.0 + (index % 7) as f32 * cell_width + 2.0;
        let y = layout.grid_top + (index / 7) as f32 * layout.week_step + 1.0;
        for _ in 0..2 {
            assert!(matches!(
                features.click(PanelId::Calendar, x, y, width, 320.0),
                Some(FeatureAction::Refresh)
            ));
        }
        assert!(matches!(
            features.double_click(PanelId::Calendar, x + cell_width, y, width, 320.0),
            Some(FeatureAction::Refresh)
        ));
        features.click(PanelId::Calendar, x, y, width, 320.0);
        features.calendar.click_target = None;
        assert!(matches!(
            features.double_click(PanelId::Calendar, x, y, width, 320.0),
            Some(FeatureAction::Refresh)
        ));
        features.click(PanelId::Calendar, x, y, width, 320.0);
        assert!(
            matches!(features.double_click(PanelId::Calendar, x, y, width, 320.0), Some(FeatureAction::OpenDaily(day)) if day == date)
        );
        assert!(features.calendar.click_target.is_none());
    }

    #[test]
    fn calendar_daily_click_uses_midnight_while_today_and_generic_routes_capture_local_now() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mdlite-calendar-clock-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let past = CalendarDate::new(2001, 2, 3).unwrap();

        let owner = super::super::tests::hidden_test_window_size(260, 340);
        let mut click_app = super::super::tests::composing_test_app("# heading\n");
        click_app.composing = false;
        click_app.features.controls.owner = owner;
        let grid = month_grid(past).unwrap();
        click_app.features.calendar.current_month = Some(grid);
        let index = grid
            .cells
            .iter()
            .position(|cell| *cell == Some(past))
            .unwrap();
        let cell_width = (260.0 - 10.0) / 7.0;
        let layout = calendar_layout(320.0);
        let x = 5.0 + (index % 7) as f32 * cell_width + 1.0 + cell_width / 2.0;
        let y = layout.grid_top + (index / 7) as f32 * layout.week_step + 1.0;
        let action = click_app
            .features
            .click(PanelId::Calendar, x, y, 260.0, 320.0)
            .unwrap();
        assert!(matches!(&action, FeatureAction::Refresh));
        assert_eq!(click_app.features.pending_daily, None);
        let action = click_app
            .features
            .double_click(PanelId::Calendar, x, y, 260.0, 320.0)
            .unwrap();
        assert!(matches!(&action, FeatureAction::OpenDaily(date) if *date == past));
        click_app.run_feature_action(action);
        assert_eq!(click_app.features.pending_daily, Some(past));
        click_app.features.workspace = Some(workspace.clone());
        click_app.features.workspace_identity = Some(workspace.identity().clone());
        click_app.features.profiles.effective = profiles_with_required_topic();
        assert!(unsafe { click_app.features_command(4023) });
        let daily_form = click_app.features.form.as_ref().unwrap();
        assert_eq!(daily_form.profile_id.as_deref(), Some("daily"));
        assert_eq!(
            daily_form.date_time,
            Some(ProfileDateTime::at_midnight(past).unwrap())
        );
        assert!(workspace.list_directory(".").unwrap().entries.is_empty());
        assert!(unsafe { click_app.features_command(4086) });
        assert!(click_app.features.form.is_none());
        assert!(workspace.list_directory(".").unwrap().entries.is_empty());
        drop(click_app);
        unsafe {
            let _ = DestroyWindow(owner);
        }

        for (command, profile_id) in [(4028, "daily"), (4024, "meeting"), (4025, "memo")] {
            let mut app = calendar_creation_app(&workspace);
            let _ = app.features.calendar.interaction.set_selection(Some(past));
            let before = local_now();
            assert!(unsafe { app.features_command(command) });
            let after = local_now();
            let form = app.features.form.as_ref().unwrap();
            assert_eq!(form.profile_id.as_deref(), Some(profile_id));
            let captured = form.date_time.unwrap();
            assert_ne!(captured.date, past);
            assert_datetime_between_local_now(captured, before, after);
            assert!(workspace.list_directory(".").unwrap().entries.is_empty());

            if command == 4024 {
                assert!(unsafe { app.features_command(4088) });
                assert_eq!(
                    app.features.form.as_ref().unwrap().date_time,
                    Some(captured)
                );
                assert!(workspace.list_directory(".").unwrap().entries.is_empty());
            }
            assert!(unsafe { app.features_command(4086) });
            assert!(app.features.form.is_none());
            assert!(workspace.list_directory(".").unwrap().entries.is_empty());
            drop(app);
        }

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn calendar_file_action_rechecks_request_missing_and_binary_entries_before_open() {
        for scenario in ["stale", "missing", "binary"] {
            let nonce = SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "mdlite-calendar-action-{scenario}-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            let path = root.join("note.md");
            fs::write(&path, "# calendar file\n").unwrap();
            let workspace = Workspace::open(&root).unwrap();
            let date = local_now().date;
            let request = CalendarListRequest::new(&workspace, 1, date).unwrap();
            let listing = request.execute(|| false);
            assert!(
                listing
                    .files
                    .iter()
                    .any(|file| file.relative_path == Path::new("note.md")),
                "fixture should be in today's Calendar listing: {scenario}"
            );

            let mut app = super::super::tests::composing_test_app("unsaved");
            app.composing = false;
            app.features.workspace = Some(workspace.clone());
            app.features.workspace_identity = Some(workspace.identity().clone());
            app.features.calendar.request = Some(request.clone());
            app.features.calendar.list_generation = request.generation;
            app.features.calendar.listing = Some(listing);
            let _ = app.features.calendar.interaction.set_selection(Some(date));
            let layout = calendar_layout(320.0);
            let action = app
                .features
                .click(PanelId::Calendar, 0.0, layout.rows_top + 1.0, 260.0, 320.0)
                .unwrap();
            assert!(matches!(action, FeatureAction::Refresh));
            let action = app.features.calendar_file_action().unwrap();
            assert!(matches!(action, FeatureAction::OpenCalendarFile { .. }));

            match scenario {
                "stale" => app.features.calendar.list_generation += 1,
                "missing" => fs::remove_file(&path).unwrap(),
                "binary" => fs::write(&path, [0, 1, 2, 3]).unwrap(),
                _ => unreachable!(),
            }
            let tab_count = app.tabs.len();
            app.run_feature_action(action);
            assert_eq!(app.tabs.len(), tab_count, "must not open {scenario} entry");
            assert!(app.features.status.is_some());
            drop(app);
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn explorer_list_sync_preserves_hidden_selection_and_scroll_position() {
        let owner = super::super::tests::hidden_test_window_size(220, 80);
        let list = explorer_list_for_test(owner);
        let mut features = FeaturePanes::default();
        features.controls.explorer_list = list;
        features.explorer.entries = (0..12)
            .map(|index| ExplorerEntry {
                name: format!("entry-{index}.md").into(),
                path: PathBuf::from(format!("entry-{index}.md")),
                kind: ExplorerEntryKind::File,
            })
            .collect();
        features.explorer.selected = vec![PathBuf::from("entry-9.md")];

        features.sync_explorer_control();
        unsafe {
            let _ = shell::send(list, LB_SETTOPINDEX, WPARAM(6), LPARAM(0));
        }
        assert_eq!(
            unsafe { shell::send(list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0)).0 },
            6
        );

        features.sync_explorer_control();
        assert_eq!(
            unsafe { shell::send(list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0)).0 },
            6
        );
        assert_eq!(
            unsafe { shell::send(list, LB_GETSEL, WPARAM(9), LPARAM(0)).0 },
            1
        );

        features.explorer_list_dirty = true;
        features.sync_explorer_control();
        assert_eq!(
            unsafe { shell::send(list, LB_GETTOPINDEX, WPARAM(0), LPARAM(0)).0 },
            6
        );
        assert_eq!(
            unsafe { shell::send(list, LB_GETSEL, WPARAM(9), LPARAM(0)).0 },
            1
        );

        drop(features);
        unsafe {
            let _ = DestroyWindow(owner);
        }
    }

    #[test]
    fn explorer_list_enter_and_arrows_expand_folders_from_native_messages() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mdlite-explorer-keys-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("folder/nested")).unwrap();
        fs::write(root.join("folder/nested/child.md"), "child").unwrap();
        fs::write(root.join("root.md"), "root").unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let folder = workspace
            .list_directory(".")
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.name == "folder")
            .unwrap();
        let owner = super::super::tests::hidden_test_window_size(220, 80);
        let list = explorer_list_for_test(owner);
        let mut app = super::super::tests::composing_test_app("# heading\n");
        app.composing = false;
        app.hwnd = owner;
        app.features.controls.owner = owner;
        app.features.controls.explorer_list = list;
        app.features.workspace = Some(workspace.clone());
        app.features.workspace_identity = Some(workspace.identity().clone());
        app.features.refresh_explorer().unwrap();
        app.features.explorer.selected = vec![folder.path.clone()];
        app.features.sync_explorer_control();

        unsafe {
            let _ = SetFocus(Some(list));
        }
        assert_eq!(unsafe { GetFocus() }, list);
        assert!(!app.feature_ime_active());
        assert_eq!(app.features.explorer.selected, vec![folder.path.clone()]);
        for key in [KEY_RETURN, KEY_LEFT, KEY_RIGHT] {
            assert!(send_explorer_key(&mut app, list, key));
            assert_eq!(
                app.features.explorer.expanded.contains(&folder.path),
                key != KEY_LEFT,
                "folder expansion state after key {key}"
            );
        }
        let nested = workspace.root().join("folder/nested");
        app.features.explorer.selected = vec![nested.clone()];
        app.features.sync_explorer_control();
        assert!(send_explorer_key(&mut app, list, KEY_RETURN));
        assert!(app.features.explorer.expanded.contains(&nested));
        app.features.explorer.selected = vec![folder.path.clone()];
        app.features.sync_explorer_control();
        assert!(send_explorer_key(&mut app, list, KEY_LEFT));
        assert!(!app.features.explorer.expanded.contains(&folder.path));
        assert!(app.features.explorer.expanded.contains(&nested));
        assert!(send_explorer_key(&mut app, list, KEY_RIGHT));
        assert!(
            app.features
                .explorer
                .entries
                .iter()
                .any(|entry| entry.path == nested && entry.kind == ExplorerEntryKind::Folder)
        );
        assert!(
            app.features
                .explorer
                .entries
                .iter()
                .any(|entry| { entry.path == workspace.root().join("folder/nested/child.md") })
        );

        drop(app);
        unsafe {
            let _ = DestroyWindow(owner);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn explorer_selection_notifications_keep_collapsed_rows_and_set_owner_draw_data() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mdlite-explorer-selection-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("folder")).unwrap();
        fs::write(root.join("folder/hidden.md"), "hidden").unwrap();
        fs::write(root.join("visible.md"), "visible").unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let folder = workspace
            .list_directory(".")
            .unwrap()
            .entries
            .into_iter()
            .find(|entry| entry.name == "folder")
            .unwrap();
        let hidden = folder.path.join("hidden.md");
        let owner = super::super::tests::hidden_test_window_size(220, 80);
        let list = explorer_list_for_test(owner);
        let mut app = super::super::tests::composing_test_app("# heading\n");
        app.features.controls.owner = owner;
        app.features.controls.explorer_list = list;
        app.features.workspace = Some(workspace.clone());
        app.features.workspace_identity = Some(workspace.identity().clone());
        app.features.explorer.expanded.insert(folder.path.clone());
        app.features.refresh_explorer().unwrap();
        assert!(
            app.features
                .explorer
                .entries
                .iter()
                .any(|entry| entry.path == hidden)
        );
        app.features.explorer.selected = vec![hidden.clone()];
        app.features.explorer.expanded.remove(&folder.path);
        app.features.refresh_explorer().unwrap();
        app.features.sync_explorer_control();

        let folder_index = app
            .features
            .explorer
            .entries
            .iter()
            .position(|entry| entry.path == folder.path)
            .unwrap();
        let visible_index = app
            .features
            .explorer
            .entries
            .iter()
            .position(|entry| entry.path == workspace.root().join("visible.md"))
            .unwrap();
        let folder_data = unsafe {
            shell::send(list, LB_GETITEMDATA, WPARAM(folder_index), LPARAM(0)).0 as usize
        };
        assert_eq!(folder_data >> 8, 0);
        assert_eq!(folder_data & 1, 1);
        assert_eq!(folder_data & 2, 0);
        unsafe {
            let _ = shell::send(list, LB_SETSEL, WPARAM(1), LPARAM(visible_index as isize));
            assert!(app.features_control(EXPLORER_LIST, LBN_SELCHANGE as u16));
        }
        assert!(app.features.explorer.selected.contains(&hidden));
        assert!(
            app.features
                .explorer
                .selected
                .contains(&workspace.root().join("visible.md"))
        );

        drop(app);
        unsafe {
            let _ = DestroyWindow(owner);
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn list_scroll_keeps_outline_and_calendar_selection_reachable() {
        assert_eq!(outline_scroll_for_selection(0, 9, 10, 3), 7);
        assert_eq!(outline_scroll_for_selection(4, 1, 10, 3), 1);
        assert_eq!(scroll_list(0, 10, 3, -1.0), 3);
        assert_eq!(scroll_list(7, 10, 3, -1.0), 7);
        assert_eq!(scroll_list(3, 10, 3, 1.0), 0);
    }

    #[test]
    fn unsaved_link_targets_only_require_a_path_for_relative_local_links() {
        for destination in [
            "https://example.invalid/path",
            "http://example.invalid/path",
            "mailto:help@example.invalid",
        ] {
            assert_eq!(
                link_target(destination, None).unwrap(),
                LinkTarget::External(destination.to_owned())
            );
        }
        assert!(matches!(
            link_target("#section", None),
            Ok(LinkTarget::Anchor(_))
        ));
        assert_eq!(
            link_target("notes/target.md", None).unwrap_err(),
            "save document before opening relative links"
        );
    }

    #[test]
    fn feature_snapshot_reuses_current_outline_for_same_revision() {
        let mut app = super::super::tests::composing_test_app("# Heading\nbody\n");
        app.composing = false;
        let projection = app.cached_projection();
        let mut outline = Outline::build(app.doc(), &projection);
        outline.headings[0].label = "cached outline".to_owned();
        app.features.outline = Some(outline);
        let cached_ptr = app.features.outline.as_ref().unwrap() as *const Outline;

        let snapshot = FeatureSnapshot::capture(&mut app);

        assert!(snapshot.outline.is_none());
        let retained = app.features.outline.as_ref().unwrap();
        assert_eq!(retained as *const Outline, cached_ptr);
        assert_eq!(retained.headings[0].label, "cached outline");
    }

    #[test]
    fn feature_snapshot_suppresses_outline_for_plain_text_documents() {
        let mut app = super::super::tests::composing_test_app("# Looks like Markdown\n");
        app.composing = false;
        let path = std::env::temp_dir().join(format!(
            "mdlite-feature-plain-{}.ini",
            SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, "# Looks like Markdown\n").unwrap();
        app.tabs[0].file = Some(mdlite::storage::FileSnapshot::open(&path).unwrap().snapshot);
        fs::remove_file(&path).unwrap();

        let snapshot = FeatureSnapshot::capture(&mut app);

        let outline = snapshot.outline.as_ref().unwrap();
        assert!(outline.headings.is_empty());
        assert_eq!(outline.document_id, app.doc().id());
        assert_eq!(outline.revision, app.doc().revision());
    }

    #[test]
    fn profile_form_codec_preserves_input_defaults_with_delimiters() {
        let mut profile = default_profiles().remove(0);
        profile.inputs.push(ProfileInputDefinition {
            id: "topic".to_owned(),
            label: "話題".to_owned(),
            required: true,
            default_value: "meeting|notes".to_owned(),
        });

        let fields = profile_form_fields(&profile);
        assert_eq!(parse_profile_fields(&fields).unwrap(), profile);
    }

    #[test]
    fn profile_values_keep_equals_in_value_and_reject_duplicate_ids() {
        let values = parse_profile_values("topic=alpha=beta\nproject=MDLite").unwrap();
        assert_eq!(values.get("topic").map(String::as_str), Some("alpha=beta"));
        assert!(parse_profile_values("topic=one\ntopic=two").is_err());
    }

    #[test]
    fn explorer_name_validation_rejects_windows_path_syntax() {
        for invalid in ["", "..", "bad/name", "CON", "folder."] {
            assert!(validate_workspace_name(invalid).is_err(), "{invalid:?}");
        }
        assert!(validate_workspace_name("議事録 2026.md").is_ok());
    }

    #[test]
    fn command_gates_separate_navigation_from_workspace_writes() {
        assert!(feature_requires_ime_end(4000));
        assert!(feature_requires_ime_end(4041));
        assert!(!feature_requires_ime_end(4020));
        assert!(feature_requires_ime_end(4028));
        assert!(feature_writes_workspace(4004));
        assert!(feature_writes_workspace(4026));
        assert!(feature_writes_workspace(4028));
        assert!(!feature_writes_workspace(4003));
        assert!(!feature_writes_workspace(4008));
        assert!(feature_requires_ime_end(CALENDAR_OPERATIONS_BUTTON));
        assert!(!feature_writes_workspace(CALENDAR_OPERATIONS_BUTTON));
        assert!(
            COMMANDS.iter().any(|(id, label, _)| {
                *id == 4028 && *label == "今日のDairyを開く / 作成"
            })
        );
        assert!(!CALENDAR_BUTTONS.contains(&4028));
    }

    #[test]
    fn today_daily_command_uses_workspace_profile_and_ime_gates() {
        let nonce = SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "mdlite-calendar-clock-gates-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::open(&root).unwrap();
        let mut app = super::super::tests::composing_test_app("# heading\n");
        app.composing = false;
        assert!(!app.features_enabled(4028));
        assert_eq!(
            app.features_disabled_reason(4028),
            "Workspaceを開いてください"
        );

        app.features.workspace = Some(workspace.clone());
        app.features.workspace_identity = Some(workspace.identity().clone());
        app.features.profiles.effective = default_profiles();
        assert!(app.features_enabled(4028));

        app.composing = true;
        assert!(!app.features_enabled(4028));
        assert_eq!(
            app.features_disabled_reason(4028),
            "IME入力の確定後に操作できます"
        );
        assert!(unsafe { app.features_command(4028) });
        assert!(app.features.form.is_none());

        app.composing = false;
        app.features.profiles.effective.clear();
        assert!(!app.features_enabled(4028));
        assert_eq!(
            app.features_disabled_reason(4028),
            "デイリーノートのプロファイルがありません"
        );

        drop(app);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn pane_keys_do_not_consume_editor_navigation_and_drag_cancel_is_unchanged() {
        let mut features = FeaturePanes::default();
        let focus = HWND(std::ptr::dangling_mut());
        let message = MSG {
            hwnd: focus,
            message: WM_KEYDOWN,
            wParam: WPARAM(VK_LEFT.0 as usize),
            ..Default::default()
        };
        assert!(
            features
                .keyboard_action(&message, focus, false, PanelId::Calendar, false)
                .is_none()
        );
        assert!(
            features
                .keyboard_action(&message, focus, false, PanelId::Calendar, true)
                .is_some()
        );
        let mut app =
            super::super::tests::composing_test_app("# A\nbody\n## child\ntext\n# B\nlast\n");
        app.composing = false;
        let projection = app.cached_projection();
        let outline = Outline::build(app.doc(), &projection);
        let source = app.doc().source().to_owned();
        let undo = app.doc().undo_count();
        app.features.outline_drag = Some((outline.clone(), 0, 0., 0., true, Some(2)));
        assert!(app.features_finish_outline_drag(true));
        assert_eq!(app.doc().source(), source);
        assert_eq!(app.doc().undo_count(), undo);
        app.features.outline_drag = Some((outline, 0, 0., 0., true, Some(2)));
        app.doc_mut().replace_range(0..0, "x").unwrap();
        let changed = app.doc().source().to_owned();
        let undo = app.doc().undo_count();
        assert!(app.features_finish_outline_drag(false));
        assert_eq!(app.doc().source(), changed);
        assert_eq!(app.doc().undo_count(), undo);
    }
}
