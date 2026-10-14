//! Calendar date state, workspace file indexing, and holiday import.

use crate::{
    profiles::{
        ProfileDateTime, ProfileDefinition, ProfileError, ProfileValues, preview_profile_path,
    },
    workspace::{Workspace, WorkspaceIdentity},
};
use encoding_rs::SHIFT_JIS;
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
    fmt, fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CalendarDate {
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

impl CalendarDate {
    pub const fn new(year: i32, month: i32, day: i32) -> Option<Self> {
        if is_valid_date_parts(year, month, day) {
            Some(Self { year, month, day })
        } else {
            None
        }
    }

    pub const fn is_valid(self) -> bool {
        is_valid_date_parts(self.year, self.month, self.day)
    }

    pub const fn days_in_month(year: i32, month: i32) -> Option<i32> {
        if year < 1601 || year > 9999 || month < 1 || month > 12 {
            return None;
        }
        Some(match month {
            2 if is_leap_year(year) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        })
    }

    pub fn add_days(self, days: i32) -> Option<Self> {
        if !self.is_valid() {
            return None;
        }
        let ordinal =
            days_from_civil(self.year, self.month, self.day).checked_add(i64::from(days))?;
        let (year, month, day) = civil_from_days(ordinal);
        Self::new(year, month, day)
    }

    /// Sunday is zero and Saturday is six.
    pub fn weekday_sunday_zero(self) -> Option<u8> {
        if !self.is_valid() {
            return None;
        }
        Some((days_from_civil(self.year, self.month, self.day) + 4).rem_euclid(7) as u8)
    }

    pub fn month_start(self) -> Option<Self> {
        Self::new(self.year, self.month, 1)
    }

    pub fn add_months_clamped(self, months: i32) -> Option<Self> {
        if !self.is_valid() {
            return None;
        }
        let absolute = i64::from(self.year) * 12 + i64::from(self.month - 1) + i64::from(months);
        let year = i32::try_from(absolute.div_euclid(12)).ok()?;
        let month = absolute.rem_euclid(12) as i32 + 1;
        let day = self.day.min(Self::days_in_month(year, month)?);
        Self::new(year, month, day)
    }
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn is_valid_date_parts(year: i32, month: i32, day: i32) -> bool {
    if year < 1601 || year > 9999 || month < 1 || month > 12 || day < 1 {
        return false;
    }
    match CalendarDate::days_in_month(year, month) {
        Some(days) => day <= days,
        None => false,
    }
}

fn days_from_civil(year: i32, month: i32, day: i32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i32, i32, i32) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year as i32, month as i32, day as i32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarMonthGrid {
    pub displayed_month: CalendarDate,
    /// Sunday-first six-week grid; edge dates outside the Windows date range are None.
    pub cells: [Option<CalendarDate>; 42],
}

pub fn month_grid(displayed_month: CalendarDate) -> Option<CalendarMonthGrid> {
    let first = displayed_month.month_start()?;
    let start = days_from_civil(first.year, first.month, first.day)
        - i64::from(first.weekday_sunday_zero()?);
    let mut cells = [None; 42];
    for (index, cell) in cells.iter_mut().enumerate() {
        let (year, month, day) = civil_from_days(start + index as i64);
        *cell = CalendarDate::new(year, month, day);
    }
    Some(CalendarMonthGrid {
        displayed_month: first,
        cells,
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalendarCellMarks {
    pub today: bool,
    pub selected: bool,
    pub hovered: bool,
    pub focused: bool,
    pub holiday: bool,
    pub daily_file: bool,
    pub sunday: bool,
    pub saturday: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CalendarCell {
    pub date: Option<CalendarDate>,
    pub in_displayed_month: bool,
    pub marks: CalendarCellMarks,
}

pub fn decorate_month(
    grid: CalendarMonthGrid,
    today: CalendarDate,
    selected: Option<CalendarDate>,
    hovered: Option<CalendarDate>,
    focused: Option<CalendarDate>,
    holidays: &BTreeSet<CalendarDate>,
    daily_files: &BTreeSet<CalendarDate>,
) -> [CalendarCell; 42] {
    std::array::from_fn(|index| {
        let date = grid.cells[index];
        let marks = date.map_or_else(CalendarCellMarks::default, |date| CalendarCellMarks {
            today: date == today,
            selected: Some(date) == selected,
            hovered: Some(date) == hovered,
            focused: Some(date) == focused,
            holiday: holidays.contains(&date),
            daily_file: daily_files.contains(&date),
            sunday: date.weekday_sunday_zero() == Some(0),
            saturday: date.weekday_sunday_zero() == Some(6),
        });
        CalendarCell {
            date,
            in_displayed_month: date.is_some_and(|date| {
                date.year == grid.displayed_month.year && date.month == grid.displayed_month.month
            }),
            marks,
        }
    })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CalendarChange {
    pub month_changed: bool,
    pub selection_changed: bool,
    /// Set only for click or Enter activation. Month and Today navigation never open a note.
    pub activate_date: Option<CalendarDate>,
}

#[derive(Clone, Debug)]
pub struct CalendarInteraction {
    today: CalendarDate,
    displayed_month: CalendarDate,
    selected: Option<CalendarDate>,
    hovered: Option<CalendarDate>,
    focused: Option<CalendarDate>,
}

impl CalendarInteraction {
    pub fn new(today: CalendarDate) -> Self {
        let today = if today.is_valid() {
            today
        } else {
            CalendarDate::new(2000, 1, 1).expect("fixed date is valid")
        };
        Self {
            today,
            displayed_month: today.month_start().expect("valid date has month"),
            selected: None,
            hovered: None,
            focused: Some(today),
        }
    }

    pub const fn today(&self) -> CalendarDate {
        self.today
    }
    pub const fn displayed_month(&self) -> CalendarDate {
        self.displayed_month
    }
    pub const fn selected(&self) -> Option<CalendarDate> {
        self.selected
    }
    pub const fn hovered(&self) -> Option<CalendarDate> {
        self.hovered
    }
    pub const fn focused(&self) -> Option<CalendarDate> {
        self.focused
    }

    pub fn set_today(&mut self, date: CalendarDate) -> bool {
        if !date.is_valid() {
            return false;
        }
        self.today = date;
        true
    }

    pub fn set_displayed_month(&mut self, date: CalendarDate) -> CalendarChange {
        let Some(month) = date.month_start() else {
            return CalendarChange::default();
        };
        let changed = month != self.displayed_month;
        self.displayed_month = month;
        let current_focus = self.focused.unwrap_or(self.selected.unwrap_or(self.today));
        self.focused = CalendarDate::new(
            month.year,
            month.month,
            current_focus
                .day
                .min(CalendarDate::days_in_month(month.year, month.month).unwrap()),
        );
        CalendarChange {
            month_changed: changed,
            ..CalendarChange::default()
        }
    }

    pub fn set_selection(&mut self, date: Option<CalendarDate>) -> CalendarChange {
        if date.is_some_and(|date| !date.is_valid()) {
            return CalendarChange::default();
        }
        let selection_changed = self.selected != date;
        self.selected = date;
        let mut change = CalendarChange {
            selection_changed,
            ..CalendarChange::default()
        };
        if let Some(date) = date {
            self.focused = Some(date);
            change.month_changed = self.set_displayed_month(date).month_changed;
        }
        change
    }

    pub fn set_hover(&mut self, date: Option<CalendarDate>) -> bool {
        let date = date.filter(|date| date.is_valid());
        if date == self.hovered {
            return false;
        }
        self.hovered = date;
        true
    }

    /// Today changes displayed month, selection, and focus, without activation.
    pub fn select_today(&mut self) -> CalendarChange {
        self.set_selection(Some(self.today))
    }

    /// Arrow movement changes focus only; it never opens a note.
    pub fn move_focus_days(&mut self, days: i32) -> bool {
        if !(-3_660_000..=3_660_000).contains(&days) {
            return false;
        }
        let start = self.focused.unwrap_or(self.selected.unwrap_or(self.today));
        let Some(date) = start.add_days(days) else {
            return false;
        };
        self.focused = Some(date);
        self.set_displayed_month(date);
        true
    }

    /// Page movement preserves the focused day where possible and clamps to the target month.
    pub fn navigate_months(&mut self, months: i32) -> CalendarChange {
        if !(-120_000..=120_000).contains(&months) {
            return CalendarChange::default();
        }
        let start = self.focused.unwrap_or(self.selected.unwrap_or(self.today));
        let Some(target) = self
            .displayed_month
            .add_months_clamped(months)
            .and_then(|month| {
                CalendarDate::new(
                    month.year,
                    month.month,
                    start
                        .day
                        .min(CalendarDate::days_in_month(month.year, month.month)?),
                )
            })
        else {
            return CalendarChange::default();
        };
        let month_changed =
            target.month != self.displayed_month.month || target.year != self.displayed_month.year;
        self.displayed_month = target.month_start().unwrap();
        self.focused = Some(target);
        CalendarChange {
            month_changed,
            ..CalendarChange::default()
        }
    }

    pub fn click_date(&mut self, date: CalendarDate) -> CalendarChange {
        if !date.is_valid() {
            return CalendarChange::default();
        }
        let selection_changed = self.selected != Some(date);
        let month_changed = self.set_displayed_month(date).month_changed;
        self.selected = Some(date);
        self.focused = Some(date);
        CalendarChange {
            month_changed,
            selection_changed,
            activate_date: Some(date),
        }
    }

    pub fn activate_focused(&mut self) -> CalendarChange {
        let date = self.focused.unwrap_or(self.selected.unwrap_or(self.today));
        if !date.is_valid() {
            return CalendarChange::default();
        }
        let selection_changed = self.selected != Some(date);
        self.selected = Some(date);
        self.focused = Some(date);
        CalendarChange {
            selection_changed,
            activate_date: Some(date),
            ..CalendarChange::default()
        }
    }
}

pub fn daily_file_marker(
    workspace: &Workspace,
    daily_profile: &ProfileDefinition,
    date: CalendarDate,
) -> Result<bool, ProfileError> {
    let preview = preview_profile_path(
        workspace,
        daily_profile,
        ProfileDateTime::at_midnight(date)?,
        &ProfileValues::new(),
    )?;
    match fs::symlink_metadata(&preview.path) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(preview.path));
            }
            Ok(metadata.is_file())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProfileError::Io(error)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarFileType {
    Markdown,
    Text,
    Configuration,
}

impl CalendarFileType {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Markdown => "Markdown",
            Self::Text => "Text",
            Self::Configuration => "Configuration",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CalendarListState {
    Reading,
    Zero,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug)]
pub struct CalendarFileEntry {
    pub name: OsString,
    pub relative_path: PathBuf,
    pub extension: String,
    pub file_type: CalendarFileType,
    pub creation_time: Option<SystemTime>,
    pub creation_date_local: Option<CalendarDate>,
}

#[derive(Clone, Debug)]
pub struct CalendarListRequest {
    pub generation: u64,
    pub workspace_identity: WorkspaceIdentity,
    pub workspace_root: PathBuf,
    pub date: CalendarDate,
}

#[derive(Clone, Debug)]
pub struct CalendarListResult {
    pub generation: u64,
    pub workspace_identity: WorkspaceIdentity,
    pub date: CalendarDate,
    pub state: CalendarListState,
    pub files: Vec<CalendarFileEntry>,
    pub workspace_files: Vec<CalendarFileEntry>,
    pub error: Option<String>,
}

impl CalendarListRequest {
    pub fn new(
        workspace: &Workspace,
        generation: u64,
        date: CalendarDate,
    ) -> Result<Self, CalendarError> {
        if !date.is_valid() {
            return Err(CalendarError::InvalidDate);
        }
        Ok(Self {
            generation,
            workspace_identity: workspace.identity().clone(),
            workspace_root: workspace.root().to_path_buf(),
            date,
        })
    }

    pub fn loading_result(&self) -> CalendarListResult {
        self.result(CalendarListState::Reading, Vec::new(), Vec::new(), None)
    }

    pub fn matches(&self, workspace: &Workspace, generation: u64, date: CalendarDate) -> bool {
        self.generation == generation
            && self.date == date
            && self.workspace_identity.same_workspace(workspace.identity())
    }

    /// Run synchronously on a worker. The immutable request identity and
    /// generation make its result safe to discard if the UI target changed.
    pub fn execute(&self, cancelled: impl Fn() -> bool) -> CalendarListResult {
        if cancelled() {
            return self.result(CalendarListState::Cancelled, Vec::new(), Vec::new(), None);
        }
        let root = match fs::canonicalize(&self.workspace_root) {
            Ok(root) if root.is_dir() => root,
            _ => {
                return self.result(
                    CalendarListState::Failed,
                    Vec::new(),
                    Vec::new(),
                    Some("workspace folder is unavailable".to_owned()),
                );
            }
        };
        let mut pending = vec![root.clone()];
        let mut all_files = Vec::new();
        let mut errors = Vec::new();
        while let Some(directory) = pending.pop() {
            if cancelled() {
                return self.result(CalendarListState::Cancelled, Vec::new(), Vec::new(), None);
            }
            let entries = match fs::read_dir(&directory) {
                Ok(entries) => entries,
                Err(error) => {
                    errors.push(format!("directory read failed ({:?})", error.kind()));
                    continue;
                }
            };
            for entry in entries {
                if cancelled() {
                    return self.result(CalendarListState::Cancelled, Vec::new(), Vec::new(), None);
                }
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(error) => {
                        errors.push(format!("directory entry failed ({:?})", error.kind()));
                        continue;
                    }
                };
                let path = entry.path();
                let metadata = match fs::symlink_metadata(&path) {
                    Ok(metadata) => metadata,
                    Err(error) => {
                        errors.push(format!("metadata read failed ({:?})", error.kind()));
                        continue;
                    }
                };
                if is_reparse_point(&metadata) {
                    continue;
                }
                if metadata.is_dir() {
                    if !is_excluded_calendar_directory(&entry.file_name()) {
                        pending.push(path);
                    }
                    continue;
                }
                if !metadata.is_file() {
                    continue;
                }
                let extension = normalized_extension(&path);
                let Some(file_type) = calendar_file_type(&extension) else {
                    continue;
                };
                match looks_binary(&path) {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => {
                        errors.push(format!("file read failed ({:?})", error.kind()));
                        continue;
                    }
                }
                let relative_path = match path.strip_prefix(&root) {
                    Ok(relative) => relative.to_path_buf(),
                    Err(_) => {
                        errors.push("workspace path resolution failed".to_owned());
                        continue;
                    }
                };
                let creation_time = metadata.created().ok();
                let creation_date_local = local_creation_date(&metadata);
                all_files.push(CalendarFileEntry {
                    name: entry.file_name(),
                    relative_path,
                    extension,
                    file_type,
                    creation_time,
                    creation_date_local,
                });
            }
        }
        all_files.sort_by(|left, right| {
            path_sort_key(&left.relative_path).cmp(&path_sort_key(&right.relative_path))
        });
        let files: Vec<_> = all_files
            .iter()
            .filter(|entry| entry.creation_date_local == Some(self.date))
            .cloned()
            .collect();
        let state = if errors.is_empty() {
            if files.is_empty() {
                CalendarListState::Zero
            } else {
                CalendarListState::Ready
            }
        } else {
            CalendarListState::Failed
        };
        self.result(
            state,
            files,
            all_files,
            (!errors.is_empty()).then(|| errors.join("; ")),
        )
    }

    /// Recheck the selected relative path and Windows creation date immediately
    /// before the caller opens it.
    pub fn recheck_before_open(
        &self,
        workspace: &Workspace,
        relative_path: &Path,
    ) -> Result<PathBuf, CalendarError> {
        if !self.workspace_identity.same_workspace(workspace.identity()) {
            return Err(CalendarError::StaleEntry);
        }
        let canonical_root = fs::canonicalize(workspace.root())?;
        let relative = validate_calendar_relative_path(relative_path)?;
        let mut candidate = canonical_root.clone();
        for component in &relative {
            candidate.push(component);
            let metadata = fs::symlink_metadata(&candidate)?;
            if is_reparse_point(&metadata) {
                return Err(CalendarError::ReparsePoint(candidate));
            }
        }
        let canonical = fs::canonicalize(&candidate)?;
        if !canonical.starts_with(&canonical_root) {
            return Err(CalendarError::OutsideWorkspace(candidate));
        }
        let metadata = fs::metadata(&canonical)?;
        if !metadata.is_file() || is_protected_calendar_path(&candidate) {
            return Err(CalendarError::StaleEntry);
        }
        if calendar_file_type(&normalized_extension(&canonical)).is_none()
            || looks_binary(&canonical)?
            || local_creation_date(&metadata) != Some(self.date)
        {
            return Err(CalendarError::StaleEntry);
        }
        Ok(canonical)
    }

    fn result(
        &self,
        state: CalendarListState,
        files: Vec<CalendarFileEntry>,
        workspace_files: Vec<CalendarFileEntry>,
        error: Option<String>,
    ) -> CalendarListResult {
        CalendarListResult {
            generation: self.generation,
            workspace_identity: self.workspace_identity.clone(),
            date: self.date,
            state,
            files,
            workspace_files,
            error,
        }
    }
}

impl CalendarListResult {
    pub fn matches(&self, workspace: &Workspace, generation: u64, date: CalendarDate) -> bool {
        self.generation == generation
            && self.date == date
            && self.workspace_identity.same_workspace(workspace.identity())
    }

    pub fn file_count(&self) -> usize {
        self.files.len()
    }
}

pub fn default_calendar_extensions() -> [&'static str; 12] {
    [
        ".md",
        ".markdown",
        ".txt",
        ".log",
        ".json",
        ".toml",
        ".yaml",
        ".yml",
        ".ini",
        ".cfg",
        ".conf",
        ".csv",
    ]
}

pub fn supports_calendar_text_open(path: &Path) -> bool {
    calendar_file_type(&normalized_extension(path)).is_some()
}

fn calendar_file_type(extension: &str) -> Option<CalendarFileType> {
    if !default_calendar_extensions().contains(&extension) {
        return None;
    }
    Some(if matches!(extension, ".md" | ".markdown") {
        CalendarFileType::Markdown
    } else if matches!(
        extension,
        ".json" | ".toml" | ".yaml" | ".yml" | ".ini" | ".cfg" | ".conf"
    ) {
        CalendarFileType::Configuration
    } else {
        CalendarFileType::Text
    })
}

fn normalized_extension(path: &Path) -> String {
    path.extension()
        .map(|extension| format!(".{}", extension.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

fn is_excluded_calendar_directory(name: &OsStr) -> bool {
    matches!(
        name.to_string_lossy().to_lowercase().as_str(),
        ".git"
            | ".mdlite"
            | "build"
            | "out"
            | "binary"
            | "recovery"
            | "cache"
            | ".cache"
            | "object"
            | "objects"
    )
}

fn path_sort_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn looks_binary(path: &Path) -> io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut bytes = [0u8; 8192];
    let count = file.read(&mut bytes)?;
    Ok(bytes[..count].contains(&0))
}

fn validate_calendar_relative_path(path: &Path) -> Result<Vec<OsString>, CalendarError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || is_protected_calendar_path(path)
    {
        return Err(CalendarError::InvalidPath(path.to_path_buf()));
    }
    Ok(path
        .components()
        .filter_map(|component| match component {
            Component::Normal(name) => Some(name.to_os_string()),
            _ => None,
        })
        .collect())
}

fn is_protected_calendar_path(path: &Path) -> bool {
    path.components().any(|component| match component {
        Component::Normal(name) => {
            matches!(
                name.to_string_lossy().to_lowercase().as_str(),
                ".git" | ".mdlite"
            )
        }
        _ => false,
    })
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn local_creation_date(metadata: &fs::Metadata) -> Option<CalendarDate> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        #[repr(C)]
        struct RawFileTime {
            low: u32,
            high: u32,
        }
        #[repr(C)]
        struct RawSystemTime {
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
            fn FileTimeToLocalFileTime(input: *const RawFileTime, output: *mut RawFileTime) -> i32;
            fn FileTimeToSystemTime(input: *const RawFileTime, output: *mut RawSystemTime) -> i32;
        }
        let ticks = metadata.creation_time();
        let utc = RawFileTime {
            low: ticks as u32,
            high: (ticks >> 32) as u32,
        };
        let mut local = RawFileTime { low: 0, high: 0 };
        let mut system = RawSystemTime {
            year: 0,
            month: 0,
            day_of_week: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        if unsafe {
            FileTimeToLocalFileTime(&utc, &mut local) != 0
                && FileTimeToSystemTime(&local, &mut system) != 0
        } {
            CalendarDate::new(
                i32::from(system.year),
                i32::from(system.month),
                i32::from(system.day),
            )
        } else {
            None
        }
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        None
    }
}

#[derive(Debug)]
pub enum CalendarError {
    Io(io::Error),
    InvalidDate,
    InvalidPath(PathBuf),
    OutsideWorkspace(PathBuf),
    ReparsePoint(PathBuf),
    StaleEntry,
    InvalidHolidayCsv(String),
}

impl fmt::Display for CalendarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::InvalidDate => f.write_str("invalid calendar date"),
            Self::InvalidPath(path) => write!(f, "invalid calendar path: {}", path.display()),
            Self::OutsideWorkspace(path) => {
                write!(f, "calendar path escapes the workspace: {}", path.display())
            }
            Self::ReparsePoint(path) => {
                write!(
                    f,
                    "calendar path crosses a reparse point: {}",
                    path.display()
                )
            }
            Self::StaleEntry => f.write_str("calendar file changed or is no longer eligible"),
            Self::InvalidHolidayCsv(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for CalendarError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for CalendarError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolidayRecord {
    pub date: CalendarDate,
    pub name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolidayStatus<'a> {
    Holiday(&'a str),
    NotHoliday,
    UnknownYear,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HolidayCacheStatus {
    NotRequested,
    Saved,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HolidayImportReport {
    pub records: usize,
    pub first_year: i32,
    pub last_year: i32,
    pub memory_imported: bool,
    pub cache_status: HolidayCacheStatus,
    pub cache_error: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct HolidayCalendar {
    imported: BTreeMap<CalendarDate, String>,
    imported_years: BTreeSet<i32>,
}

impl HolidayCalendar {
    pub fn status(&self, date: CalendarDate) -> HolidayStatus<'_> {
        if !date.is_valid() {
            return HolidayStatus::UnknownYear;
        }
        if let Some(name) = self.imported.get(&date) {
            return HolidayStatus::Holiday(name);
        }
        if let Some((_, _, _, name)) = BUNDLED_HOLIDAYS.iter().find(|(year, month, day, _)| {
            *year == date.year && *month == date.month && *day == date.day
        }) {
            return HolidayStatus::Holiday(name);
        }
        if self.imported_years.contains(&date.year) || (2025..=2027).contains(&date.year) {
            HolidayStatus::NotHoliday
        } else {
            HolidayStatus::UnknownYear
        }
    }

    pub fn bundled_data_version(&self) -> &'static str {
        "内閣府 国民の祝日・休日CSV 2026-02-02取得"
    }

    /// Validate the entire CSV before replacing in-memory imported records.
    /// A cache write failure is returned separately after memory import succeeds.
    pub fn import_csv_bytes(
        &mut self,
        bytes: &[u8],
        cache: Option<(&Workspace, &Path)>,
    ) -> Result<HolidayImportReport, CalendarError> {
        let text = decode_holiday_csv(bytes)?;
        let records = parse_holiday_csv(&text)?;
        let first_year = records.iter().map(|record| record.date.year).min().unwrap();
        let last_year = records.iter().map(|record| record.date.year).max().unwrap();
        let imported: BTreeMap<_, _> = records
            .iter()
            .map(|record| (record.date, record.name.clone()))
            .collect();
        let years = records.iter().map(|record| record.date.year).collect();
        self.imported = imported;
        self.imported_years = years;

        let (cache_status, cache_error) = if let Some((workspace, relative_path)) = cache {
            match self.save_cache(workspace, relative_path) {
                Ok(()) => (HolidayCacheStatus::Saved, None),
                Err(error) => (HolidayCacheStatus::Failed, Some(error.to_string())),
            }
        } else {
            (HolidayCacheStatus::NotRequested, None)
        };
        Ok(HolidayImportReport {
            records: records.len(),
            first_year,
            last_year,
            memory_imported: true,
            cache_status,
            cache_error,
        })
    }

    pub fn import_csv_file(
        &mut self,
        source: impl AsRef<Path>,
        cache: Option<(&Workspace, &Path)>,
    ) -> Result<HolidayImportReport, CalendarError> {
        self.import_csv_bytes(&fs::read(source)?, cache)
    }

    pub fn save_cache(
        &self,
        workspace: &Workspace,
        relative_path: &Path,
    ) -> Result<(), CalendarError> {
        let path = resolve_holiday_cache_path(workspace, relative_path)?;
        let mut csv = String::from("date,name\n");
        for (date, name) in &self.imported {
            csv.push_str(&format!(
                "{:04}-{:02}-{:02},",
                date.year, date.month, date.day
            ));
            csv.push_str(&quote_csv_field(name));
            csv.push('\n');
        }
        atomic_write_cache(&path, csv.as_bytes())
    }

    pub fn imported_records(&self) -> Vec<HolidayRecord> {
        self.imported
            .iter()
            .map(|(date, name)| HolidayRecord {
                date: *date,
                name: name.clone(),
            })
            .collect()
    }
}

const BUNDLED_HOLIDAYS: &[(i32, i32, i32, &str)] = &[
    (2025, 1, 1, "元日"),
    (2025, 1, 13, "成人の日"),
    (2025, 2, 11, "建国記念の日"),
    (2025, 2, 23, "天皇誕生日"),
    (2025, 2, 24, "休日"),
    (2025, 3, 20, "春分の日"),
    (2025, 4, 29, "昭和の日"),
    (2025, 5, 3, "憲法記念日"),
    (2025, 5, 4, "みどりの日"),
    (2025, 5, 5, "こどもの日"),
    (2025, 5, 6, "休日"),
    (2025, 7, 21, "海の日"),
    (2025, 8, 11, "山の日"),
    (2025, 9, 15, "敬老の日"),
    (2025, 9, 23, "秋分の日"),
    (2025, 10, 13, "スポーツの日"),
    (2025, 11, 3, "文化の日"),
    (2025, 11, 23, "勤労感謝の日"),
    (2025, 11, 24, "休日"),
    (2026, 1, 1, "元日"),
    (2026, 1, 12, "成人の日"),
    (2026, 2, 11, "建国記念の日"),
    (2026, 2, 23, "天皇誕生日"),
    (2026, 3, 20, "春分の日"),
    (2026, 4, 29, "昭和の日"),
    (2026, 5, 3, "憲法記念日"),
    (2026, 5, 4, "みどりの日"),
    (2026, 5, 5, "こどもの日"),
    (2026, 5, 6, "休日"),
    (2026, 7, 20, "海の日"),
    (2026, 8, 11, "山の日"),
    (2026, 9, 21, "敬老の日"),
    (2026, 9, 22, "休日"),
    (2026, 9, 23, "秋分の日"),
    (2026, 10, 12, "スポーツの日"),
    (2026, 11, 3, "文化の日"),
    (2026, 11, 23, "勤労感謝の日"),
    (2027, 1, 1, "元日"),
    (2027, 1, 11, "成人の日"),
    (2027, 2, 11, "建国記念の日"),
    (2027, 2, 23, "天皇誕生日"),
    (2027, 3, 21, "春分の日"),
    (2027, 3, 22, "休日"),
    (2027, 4, 29, "昭和の日"),
    (2027, 5, 3, "憲法記念日"),
    (2027, 5, 4, "みどりの日"),
    (2027, 5, 5, "こどもの日"),
    (2027, 7, 19, "海の日"),
    (2027, 8, 11, "山の日"),
    (2027, 9, 20, "敬老の日"),
    (2027, 9, 23, "秋分の日"),
    (2027, 10, 11, "スポーツの日"),
    (2027, 11, 3, "文化の日"),
    (2027, 11, 23, "勤労感謝の日"),
];

fn decode_holiday_csv(bytes: &[u8]) -> Result<String, CalendarError> {
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    if bytes.len() > MAX_BYTES {
        return Err(CalendarError::InvalidHolidayCsv(
            "holiday CSV exceeds the 8 MiB limit".to_owned(),
        ));
    }
    let payload = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if let Ok(text) = std::str::from_utf8(payload) {
        return Ok(text.to_owned());
    }
    let (decoded, had_errors) = SHIFT_JIS.decode_without_bom_handling(payload);
    if had_errors || decoded.contains('\u{FFFD}') {
        return Err(CalendarError::InvalidHolidayCsv(
            "holiday CSV is neither valid UTF-8 nor CP932".to_owned(),
        ));
    }
    Ok(decoded.into_owned())
}

fn parse_holiday_csv(text: &str) -> Result<Vec<HolidayRecord>, CalendarError> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();
    let mut first_nonempty = true;
    for (line_index, raw_line) in text.lines().enumerate() {
        if line_index >= 10_000 {
            return Err(CalendarError::InvalidHolidayCsv(
                "holiday CSV exceeds 10,000 lines".to_owned(),
            ));
        }
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        let fields = parse_csv_line(line).ok_or_else(|| {
            CalendarError::InvalidHolidayCsv(format!(
                "invalid CSV quoting on line {}",
                line_index + 1
            ))
        })?;
        if fields.len() < 2 {
            return Err(CalendarError::InvalidHolidayCsv(format!(
                "holiday CSV needs date and name fields on line {}",
                line_index + 1
            )));
        }
        if first_nonempty {
            first_nonempty = false;
            let header = fields[0].to_lowercase();
            if header.contains("date") || header.contains('日') {
                continue;
            }
        }
        let date = parse_holiday_date(&fields[0]).ok_or_else(|| {
            CalendarError::InvalidHolidayCsv(format!("invalid date on line {}", line_index + 1))
        })?;
        let name = fields[1].trim();
        if name.is_empty() || name.chars().count() > 128 || name.chars().any(char::is_control) {
            return Err(CalendarError::InvalidHolidayCsv(format!(
                "invalid holiday name on line {}",
                line_index + 1
            )));
        }
        if !seen.insert(date) {
            return Err(CalendarError::InvalidHolidayCsv(format!(
                "duplicate holiday date on line {}",
                line_index + 1
            )));
        }
        records.push(HolidayRecord {
            date,
            name: name.to_owned(),
        });
    }
    if records.is_empty() {
        return Err(CalendarError::InvalidHolidayCsv(
            "holiday CSV has no records".to_owned(),
        ));
    }
    Ok(records)
}

fn parse_csv_line(line: &str) -> Option<Vec<String>> {
    let mut fields = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                field.push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => {
                fields.push(field.trim().to_owned());
                field.clear();
            }
            _ => field.push(character),
        }
    }
    if quoted {
        return None;
    }
    fields.push(field.trim().to_owned());
    Some(fields)
}

fn parse_holiday_date(text: &str) -> Option<CalendarDate> {
    let separator = if text.contains('/') {
        '/'
    } else if text.contains('-') {
        '-'
    } else {
        return None;
    };
    let mut parts = text.split(separator);
    let year = parse_ascii_number(parts.next()?)?;
    let month = parse_ascii_number(parts.next()?)?;
    let day = parse_ascii_number(parts.next()?)?;
    if parts.next().is_some() || !(1900..=2200).contains(&year) {
        return None;
    }
    CalendarDate::new(year, month, day)
}

fn parse_ascii_number(text: &str) -> Option<i32> {
    if text.is_empty() || text.len() > 4 || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn quote_csv_field(value: &str) -> String {
    if value.contains([',', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn resolve_holiday_cache_path(
    workspace: &Workspace,
    relative: &Path,
) -> Result<PathBuf, CalendarError> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CalendarError::InvalidPath(relative.to_path_buf()));
    }
    let components: Vec<_> = relative.components().collect();
    if components.len() < 3
        || components[0].as_os_str() != ".mdlite"
        || components[1].as_os_str() != ".cache"
    {
        return Err(CalendarError::InvalidPath(relative.to_path_buf()));
    }
    let root = fs::canonicalize(workspace.root())?;
    let mut path = root.clone();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(CalendarError::InvalidPath(relative.to_path_buf()));
        };
        path.push(name);
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(CalendarError::ReparsePoint(path));
                }
                let canonical = fs::canonicalize(&path)?;
                if !canonical.starts_with(&root) {
                    return Err(CalendarError::OutsideWorkspace(path));
                }
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(CalendarError::InvalidPath(path));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                for remaining in &components[index + 1..] {
                    let Component::Normal(name) = remaining else {
                        return Err(CalendarError::InvalidPath(relative.to_path_buf()));
                    };
                    path.push(name);
                }
                return Ok(path);
            }
            Err(error) => return Err(CalendarError::Io(error)),
        }
    }
    Ok(path)
}

static CACHE_TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn atomic_write_cache(path: &Path, bytes: &[u8]) -> Result<(), CalendarError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let id = CACHE_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".mdlite-holiday-{}-{id}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(CalendarError::Io(error));
    }
    drop(file);
    if let Err(error) = replace_cache_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(CalendarError::Io(error));
    }
    Ok(())
}

fn replace_cache_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        };
        let source: Vec<u16> = source.as_os_str().encode_wide().chain([0]).collect();
        let destination: Vec<u16> = destination.as_os_str().encode_wide().chain([0]).collect();
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            loop {
                let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("mdlite-calendar-test-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("fixture create failed: {error}"),
                }
            }
        }

        fn workspace(&self) -> Workspace {
            Workspace::open(&self.0).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn month_grid_has_sunday_first_42_valid_cells_and_handles_range_edges() {
        let grid = month_grid(CalendarDate::new(2024, 2, 15).unwrap()).unwrap();
        assert_eq!(grid.displayed_month, CalendarDate::new(2024, 2, 1).unwrap());
        assert_eq!(grid.cells.len(), 42);
        assert_eq!(grid.cells[0], CalendarDate::new(2024, 1, 28));
        assert!(grid.cells.contains(&CalendarDate::new(2024, 2, 29)));
        assert!(grid.cells.iter().all(Option::is_some));

        let first_month = month_grid(CalendarDate::new(1601, 1, 1).unwrap()).unwrap();
        assert_eq!(first_month.cells[0], None);
        assert_eq!(first_month.cells[1], CalendarDate::new(1601, 1, 1));
        assert_eq!(
            CalendarDate::new(2000, 2, 29)
                .unwrap()
                .add_months_clamped(1),
            CalendarDate::new(2000, 3, 29)
        );
        assert_eq!(
            CalendarDate::new(2024, 1, 31)
                .unwrap()
                .add_months_clamped(1),
            CalendarDate::new(2024, 2, 29)
        );
    }

    #[test]
    fn navigation_focus_selection_and_activation_are_separate() {
        let today = CalendarDate::new(2024, 3, 31).unwrap();
        let mut state = CalendarInteraction::new(today);
        let page = state.navigate_months(-1);
        assert!(page.month_changed);
        assert_eq!(
            state.displayed_month(),
            CalendarDate::new(2024, 2, 1).unwrap()
        );
        assert_eq!(state.focused(), CalendarDate::new(2024, 2, 29));
        assert_eq!(state.selected(), None);
        assert_eq!(page.activate_date, None);

        let today_change = state.select_today();
        assert!(today_change.selection_changed);
        assert_eq!(today_change.activate_date, None);
        assert_eq!(state.selected(), Some(today));
        let clicked = state.click_date(CalendarDate::new(2024, 2, 14).unwrap());
        assert_eq!(clicked.activate_date, CalendarDate::new(2024, 2, 14));
        assert_eq!(state.selected(), CalendarDate::new(2024, 2, 14));
        state.set_hover(Some(CalendarDate::new(2024, 2, 15).unwrap()));
        assert_eq!(state.hovered(), CalendarDate::new(2024, 2, 15));
    }

    #[test]
    fn supported_text_extensions_align_with_calendar_open_and_binary_check() {
        for extension in [
            ".md",
            ".markdown",
            ".txt",
            ".log",
            ".json",
            ".toml",
            ".yaml",
            ".yml",
            ".ini",
            ".cfg",
            ".conf",
            ".csv",
        ] {
            assert!(supports_calendar_text_open(Path::new(&format!(
                "note{extension}"
            ))));
        }
        assert!(!supports_calendar_text_open(Path::new("image.png")));
        assert_eq!(
            calendar_file_type(".ini"),
            Some(CalendarFileType::Configuration)
        );
        assert_eq!(default_calendar_extensions().len(), 12);
    }

    #[test]
    fn day_index_uses_creation_date_and_carries_workspace_generation() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        fs::create_dir(fixture.0.join("records")).unwrap();
        let file = fixture.0.join("records/2001-01-01.ini");
        fs::write(&file, "theme=light\n").unwrap();
        fs::write(fixture.0.join("records/binary.csv"), [0, 1, 2]).unwrap();
        fs::write(fixture.0.join("records/image.png"), "skip").unwrap();
        let date = local_creation_date(&fs::metadata(&file).unwrap()).unwrap();
        let request = CalendarListRequest::new(&workspace, 4, date).unwrap();
        assert_eq!(request.loading_result().state, CalendarListState::Reading);
        assert!(!request.matches(&workspace, 5, date));
        assert!(request.matches(&workspace, 4, date));
        let result = request.execute(|| false);
        assert_eq!(result.state, CalendarListState::Ready);
        assert_eq!(result.file_count(), 1);
        assert_eq!(
            result.files[0].relative_path,
            PathBuf::from("records/2001-01-01.ini")
        );
        assert_eq!(result.files[0].file_type, CalendarFileType::Configuration);
        assert_eq!(result.files[0].creation_date_local, Some(date));
        assert_eq!(result.workspace_files.len(), 1);
        assert!(
            request
                .recheck_before_open(&workspace, &result.files[0].relative_path)
                .is_ok()
        );

        fs::remove_file(&file).unwrap();
        assert!(matches!(
            request.recheck_before_open(&workspace, &result.files[0].relative_path),
            Err(CalendarError::Io(_))
        ));
        assert_eq!(request.execute(|| true).state, CalendarListState::Cancelled);
    }

    #[test]
    fn daily_marker_requires_a_regular_file() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let profile = crate::profiles::default_profiles().remove(0);
        let date = CalendarDate::new(2026, 8, 9).unwrap();
        let path = preview_profile_path(
            &workspace,
            &profile,
            ProfileDateTime::at_midnight(date).unwrap(),
            &ProfileValues::new(),
        )
        .unwrap()
        .path;
        fs::create_dir_all(&path).unwrap();
        assert!(!daily_file_marker(&workspace, &profile, date).unwrap());
        fs::remove_dir_all(&path).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "note").unwrap();
        assert!(daily_file_marker(&workspace, &profile, date).unwrap());
    }

    #[test]
    fn holiday_import_decodes_utf8_bom_and_keeps_memory_on_cache_or_csv_failure() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let mut calendar = HolidayCalendar::default();
        assert_eq!(
            calendar.status(CalendarDate::new(2026, 1, 1).unwrap()),
            HolidayStatus::Holiday("元日")
        );
        assert_eq!(
            calendar.status(CalendarDate::new(2030, 1, 1).unwrap()),
            HolidayStatus::UnknownYear
        );
        let csv = b"\xEF\xBB\xBFdate,name\r\n2025/1/1,\"New, Year\"\r\n2028-02-29,Leap Day\r\n";
        let report = calendar
            .import_csv_bytes(
                csv,
                Some((&workspace, Path::new(".mdlite/.cache/holidays.csv"))),
            )
            .unwrap();
        assert!(report.memory_imported);
        assert_eq!(report.cache_status, HolidayCacheStatus::Failed);
        assert!(report.cache_error.is_some());
        assert_eq!(
            calendar.status(CalendarDate::new(2025, 1, 1).unwrap()),
            HolidayStatus::Holiday("New, Year")
        );
        assert_eq!(
            calendar.status(CalendarDate::new(2028, 2, 29).unwrap()),
            HolidayStatus::Holiday("Leap Day")
        );
        assert_eq!(
            calendar.status(CalendarDate::new(2029, 1, 1).unwrap()),
            HolidayStatus::UnknownYear
        );

        let duplicate = b"date,name\n2027-01-01,New Year\n2027-01-01,Duplicate\n";
        assert!(matches!(
            calendar.import_csv_bytes(duplicate, None),
            Err(CalendarError::InvalidHolidayCsv(_))
        ));
        assert_eq!(
            calendar.status(CalendarDate::new(2025, 1, 1).unwrap()),
            HolidayStatus::Holiday("New, Year")
        );

        fs::create_dir_all(fixture.0.join(".mdlite/.cache")).unwrap();
        let saved = calendar
            .import_csv_bytes(
                csv,
                Some((&workspace, Path::new(".mdlite/.cache/holidays.csv"))),
            )
            .unwrap();
        assert_eq!(saved.cache_status, HolidayCacheStatus::Saved);
        assert!(fixture.0.join(".mdlite/.cache/holidays.csv").exists());
    }

    #[test]
    fn holiday_import_accepts_cp932_and_validates_real_dates_and_names() {
        let source = "date,name\n2026/1/1,元日\n";
        let (encoded, _, had_errors) = SHIFT_JIS.encode(source);
        assert!(!had_errors);
        let mut calendar = HolidayCalendar::default();
        calendar.import_csv_bytes(&encoded, None).unwrap();
        assert_eq!(
            calendar.status(CalendarDate::new(2026, 1, 1).unwrap()),
            HolidayStatus::Holiday("元日")
        );
        assert!(matches!(
            parse_holiday_csv("date,name\n2026-02-30,Bad\n"),
            Err(CalendarError::InvalidHolidayCsv(_))
        ));
        assert!(matches!(
            parse_holiday_csv("date,name\n2026-01-01,\n"),
            Err(CalendarError::InvalidHolidayCsv(_))
        ));
    }
}
