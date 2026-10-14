//! Profile definitions, template rendering, and safe note creation.

use crate::{calendar::CalendarDate, workspace::Workspace};
use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsStr,
    fmt, fs,
    fs::OpenOptions,
    io::{self, Write},
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProfileCollision {
    OpenExisting,
    Sequence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileInputDefinition {
    pub id: String,
    pub label: String,
    pub default_value: String,
    pub required: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileDefinition {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub filename: PathBuf,
    /// Relative to the workspace's .mdlite directory.
    pub template_path: PathBuf,
    pub collision: ProfileCollision,
    pub inputs: Vec<ProfileInputDefinition>,
}

pub type ProfileValues = BTreeMap<String, String>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProfileDateTime {
    pub date: CalendarDate,
    pub hour: u8,
    pub minute: u8,
}

impl ProfileDateTime {
    pub fn new(date: CalendarDate, hour: u8, minute: u8) -> Result<Self, ProfileError> {
        if !date.is_valid() || hour > 23 || minute > 59 {
            return Err(ProfileError::InvalidDateTime);
        }
        Ok(Self { date, hour, minute })
    }

    pub fn at_midnight(date: CalendarDate) -> Result<Self, ProfileError> {
        Self::new(date, 0, 0)
    }
}

#[derive(Debug)]
pub enum ProfileError {
    Io(io::Error),
    InvalidDefinition(String),
    UnknownToken(String),
    RequiredInputs(Vec<String>),
    MultipleCursors,
    InvalidDateTime,
    InvalidPath(PathBuf),
    OutsideWorkspace(PathBuf),
    ReparsePoint(PathBuf),
    NotDirectory(PathBuf),
    NotFile(PathBuf),
    InvalidUtf8(PathBuf),
    InvalidSchema(String),
    ProfileConflict(PathBuf),
    SequenceExhausted(PathBuf),
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::InvalidDefinition(message) => f.write_str(message),
            Self::UnknownToken(token) => write!(f, "unknown template token: {token}"),
            Self::RequiredInputs(ids) => write!(f, "required inputs are empty: {}", ids.join(", ")),
            Self::MultipleCursors => f.write_str("a template can contain at most one cursor token"),
            Self::InvalidDateTime => f.write_str("invalid profile date or time"),
            Self::InvalidPath(path) => {
                write!(f, "invalid relative profile path: {}", path.display())
            }
            Self::OutsideWorkspace(path) => {
                write!(f, "profile path escapes the workspace: {}", path.display())
            }
            Self::ReparsePoint(path) => write!(
                f,
                "profile path crosses a reparse point: {}",
                path.display()
            ),
            Self::NotDirectory(path) => write!(
                f,
                "profile path component is not a directory: {}",
                path.display()
            ),
            Self::NotFile(path) => write!(
                f,
                "profile target is not a regular file: {}",
                path.display()
            ),
            Self::InvalidUtf8(path) => {
                write!(f, "profile file is not valid UTF-8: {}", path.display())
            }
            Self::InvalidSchema(message) => f.write_str(message),
            Self::ProfileConflict(path) => write!(
                f,
                "profile file changed after it was loaded: {}",
                path.display()
            ),
            Self::SequenceExhausted(path) => {
                write!(f, "no available sequence name for {}", path.display())
            }
        }
    }
}

impl std::error::Error for ProfileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ProfileError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub fn default_profiles() -> Vec<ProfileDefinition> {
    vec![
        ProfileDefinition {
            id: "daily".to_owned(),
            name: "デイリーノート".to_owned(),
            directory: PathBuf::from("Dairy/{{date:yyyy}}/{{date:yyyyMM}}"),
            filename: PathBuf::from("{{date:yyyyMMdd}}.md"),
            template_path: PathBuf::from("templates/daily.md"),
            collision: ProfileCollision::OpenExisting,
            inputs: Vec::new(),
        },
        ProfileDefinition {
            id: "meeting".to_owned(),
            name: "Meeting".to_owned(),
            directory: PathBuf::from("Meeting/{{date:yyyy}}/{{date:yyyyMM}}"),
            filename: PathBuf::from("{{date:yyyyMMdd}}.md"),
            template_path: PathBuf::from("templates/meeting.md"),
            collision: ProfileCollision::Sequence,
            inputs: Vec::new(),
        },
        ProfileDefinition {
            id: "memo".to_owned(),
            name: "Memo".to_owned(),
            directory: PathBuf::from("Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}"),
            filename: PathBuf::from("{{date:yyyyMMdd}}.md"),
            template_path: PathBuf::from("templates/memo.md"),
            collision: ProfileCollision::Sequence,
            inputs: Vec::new(),
        },
    ]
}

pub fn validate_profile(profile: &ProfileDefinition) -> Result<(), ProfileError> {
    if profile.id.is_empty()
        || !profile
            .id
            .chars()
            .all(|character| character.is_alphanumeric() || matches!(character, '.' | '_' | '-'))
        || !safe_toml_string(&profile.id)
        || profile.name.trim().is_empty()
        || !safe_toml_string(&profile.name)
    {
        return Err(ProfileError::InvalidDefinition(
            "profile id or name is invalid".to_owned(),
        ));
    }
    let mut input_ids = HashSet::new();
    for input in &profile.inputs {
        if input.id.is_empty()
            || !input
                .id
                .chars()
                .all(|character| character.is_alphanumeric() || matches!(character, '_' | '-'))
            || input.label.trim().is_empty()
            || !safe_toml_string(&input.id)
            || !safe_toml_string(&input.label)
            || !safe_toml_string(&input.default_value)
            || !input_ids.insert(input.id.as_str())
        {
            return Err(ProfileError::InvalidDefinition(
                "profile input id, label, default, or uniqueness is invalid".to_owned(),
            ));
        }
    }
    validate_pattern_path(&profile.directory, false)?;
    validate_pattern_path(&profile.filename, true)?;
    validate_relative_path(&profile.template_path)?;
    if profile.template_path.to_string_lossy().contains("{{") {
        return Err(ProfileError::InvalidDefinition(
            "template path cannot contain tokens".to_owned(),
        ));
    }
    if is_protected_relative(&profile.directory) || is_protected_relative(&profile.template_path) {
        return Err(ProfileError::InvalidDefinition(
            "profile path targets protected workspace data".to_owned(),
        ));
    }
    for pattern in [
        profile.directory.to_string_lossy().into_owned(),
        profile.filename.to_string_lossy().into_owned(),
    ] {
        let mut cursor = 0;
        while let Some(start) = pattern[cursor..].find("{{") {
            let start = cursor + start;
            let end = pattern[start + 2..]
                .find("}}")
                .map(|end| start + 2 + end + 2)
                .ok_or_else(|| ProfileError::UnknownToken(pattern[start..].to_owned()))?;
            validate_token(&pattern[start..end], profile, false)?;
            cursor = end;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfilePreview {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub missing_required_inputs: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct RenderedNote {
    pub source: String,
    /// UTF-8 byte offset suitable for the Rust document selection.
    pub cursor_byte_offset: usize,
    pub missing_required_inputs: Vec<String>,
}

pub fn preview_profile_path(
    workspace: &Workspace,
    profile: &ProfileDefinition,
    date_time: ProfileDateTime,
    values: &ProfileValues,
) -> Result<ProfilePreview, ProfileError> {
    validate_profile(profile)?;
    if ProfileDateTime::new(date_time.date, date_time.hour, date_time.minute).is_err() {
        return Err(ProfileError::InvalidDateTime);
    }
    let directory = render_fragment(
        &profile.directory.to_string_lossy(),
        profile,
        date_time,
        values,
        false,
    )?;
    let filename = render_fragment(
        &profile.filename.to_string_lossy(),
        profile,
        date_time,
        values,
        false,
    )?;
    let mut missing = directory.missing_required_inputs;
    extend_unique(&mut missing, filename.missing_required_inputs);
    extend_unique(&mut missing, required_input_gaps(profile, values));
    let relative_path = PathBuf::from(directory.text).join(filename.text);
    validate_rendered_path(&relative_path)?;
    let path = inspect_workspace_path(workspace, &relative_path, false)?;
    Ok(ProfilePreview {
        path,
        relative_path,
        missing_required_inputs: missing,
    })
}

pub fn render_profile_body(
    profile: &ProfileDefinition,
    template: &str,
    date_time: ProfileDateTime,
    values: &ProfileValues,
) -> Result<RenderedNote, ProfileError> {
    validate_profile(profile)?;
    if ProfileDateTime::new(date_time.date, date_time.hour, date_time.minute).is_err() {
        return Err(ProfileError::InvalidDateTime);
    }
    let rendered = render_fragment(template, profile, date_time, values, true)?;
    let mut missing = rendered.missing_required_inputs;
    extend_unique(&mut missing, required_input_gaps(profile, values));
    Ok(RenderedNote {
        cursor_byte_offset: rendered.cursor.unwrap_or(rendered.text.len()),
        source: rendered.text,
        missing_required_inputs: missing,
    })
}

fn required_input_gaps(profile: &ProfileDefinition, values: &ProfileValues) -> Vec<String> {
    profile
        .inputs
        .iter()
        .filter(|input| {
            input.required
                && values
                    .get(&input.id)
                    .unwrap_or(&input.default_value)
                    .is_empty()
        })
        .map(|input| input.id.clone())
        .collect()
}

#[derive(Debug)]
struct Fragment {
    text: String,
    cursor: Option<usize>,
    missing_required_inputs: Vec<String>,
}

fn render_fragment(
    input: &str,
    profile: &ProfileDefinition,
    date_time: ProfileDateTime,
    values: &ProfileValues,
    allow_cursor: bool,
) -> Result<Fragment, ProfileError> {
    let mut output = String::with_capacity(input.len());
    let mut missing = Vec::new();
    let mut cursor = None;
    let mut offset = 0;
    while let Some(open_rel) = input[offset..].find("{{") {
        let open = offset + open_rel;
        output.push_str(&input[offset..open]);
        let close = input[open + 2..]
            .find("}}")
            .map(|close_rel| open + 2 + close_rel + 2)
            .ok_or_else(|| ProfileError::UnknownToken(input[open..].to_owned()))?;
        let token = &input[open..close];
        if token == "{{cursor}}" {
            if !allow_cursor {
                return Err(ProfileError::UnknownToken(token.to_owned()));
            }
            if cursor.replace(output.len()).is_some() {
                return Err(ProfileError::MultipleCursors);
            }
        } else if let Some(value) = date_token(token, date_time) {
            output.push_str(&value);
        } else if token == "{{title}}" {
            append_input(profile, values, "title", &mut output, &mut missing)
                .ok_or_else(|| ProfileError::UnknownToken(token.to_owned()))?;
        } else if let Some(id) = token
            .strip_prefix("{{input:")
            .and_then(|token| token.strip_suffix("}}"))
        {
            append_input(profile, values, id, &mut output, &mut missing)
                .ok_or_else(|| ProfileError::UnknownToken(token.to_owned()))?;
        } else {
            return Err(ProfileError::UnknownToken(token.to_owned()));
        }
        offset = close;
    }
    output.push_str(&input[offset..]);
    Ok(Fragment {
        text: output,
        cursor,
        missing_required_inputs: missing,
    })
}

fn append_input(
    profile: &ProfileDefinition,
    values: &ProfileValues,
    id: &str,
    output: &mut String,
    missing: &mut Vec<String>,
) -> Option<()> {
    let input = profile.inputs.iter().find(|input| input.id == id)?;
    let value = values.get(id).unwrap_or(&input.default_value);
    if input.required && value.is_empty() && !missing.iter().any(|missing| missing == id) {
        missing.push(id.to_owned());
    }
    output.push_str(value);
    Some(())
}

fn date_token(token: &str, date_time: ProfileDateTime) -> Option<String> {
    let date = date_time.date;
    let month = format!("{:02}", date.month);
    let day = format!("{:02}", date.day);
    let year = format!("{:04}", date.year);
    let hour = format!("{:02}", date_time.hour);
    let minute = format!("{:02}", date_time.minute);
    Some(match token {
        "{{date:yyyy}}" => year,
        "{{date:MM}}" => month,
        "{{date:dd}}" => day,
        "{{date:HH}}" => hour,
        "{{date:mm}}" => minute,
        "{{date:yyyyMM}}" => format!("{year}{month}"),
        "{{date:yyyyMMdd}}" => format!("{year}{month}{day}"),
        "{{date:yyyyMMddHHmm}}" => format!("{year}{month}{day}{hour}{minute}"),
        "{{date:yyyy-MM-dd}}" => format!("{year}-{month}-{day}"),
        "{{date:yyyy-MM-dd HH-mm}}" => format!("{year}-{month}-{day} {hour}-{minute}"),
        _ => return None,
    })
}

fn validate_token(
    token: &str,
    profile: &ProfileDefinition,
    allow_cursor: bool,
) -> Result<(), ProfileError> {
    if date_token(
        token,
        ProfileDateTime {
            date: CalendarDate::new(2000, 1, 1).unwrap(),
            hour: 0,
            minute: 0,
        },
    )
    .is_some()
        || (allow_cursor && token == "{{cursor}}")
        || (token == "{{title}}" && profile.inputs.iter().any(|input| input.id == "title"))
        || token
            .strip_prefix("{{input:")
            .and_then(|token| token.strip_suffix("}}"))
            .is_some_and(|id| profile.inputs.iter().any(|input| input.id == id))
    {
        Ok(())
    } else {
        Err(ProfileError::UnknownToken(token.to_owned()))
    }
}

fn validate_pattern_path(path: &Path, filename: bool) -> Result<(), ProfileError> {
    let value = path.to_string_lossy();
    if value.is_empty() || value.contains(['"', '\r', '\n']) {
        return Err(ProfileError::InvalidPath(path.to_path_buf()));
    }
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') || normalized.ends_with('/') {
        return Err(ProfileError::InvalidPath(path.to_path_buf()));
    }
    let parts: Vec<_> = normalized.split('/').collect();
    if (filename && parts.len() != 1)
        || parts
            .iter()
            .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err(ProfileError::InvalidPath(path.to_path_buf()));
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<(), ProfileError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ProfileError::InvalidPath(path.to_path_buf()));
    }
    Ok(())
}

fn validate_rendered_path(path: &Path) -> Result<(), ProfileError> {
    validate_relative_path(path)?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(ProfileError::InvalidPath(path.to_path_buf()));
        };
        validate_windows_name(name)?;
    }
    if is_protected_relative(path) {
        return Err(ProfileError::InvalidPath(path.to_path_buf()));
    }
    Ok(())
}

fn validate_windows_name(name: &OsStr) -> Result<(), ProfileError> {
    let Some(name) = name.to_str() else {
        return Err(ProfileError::InvalidPath(PathBuf::from(name)));
    };
    let base = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    let reserved = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && (base.starts_with("COM") || base.starts_with("LPT"))
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    if name.is_empty()
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        || reserved
    {
        return Err(ProfileError::InvalidPath(PathBuf::from(name)));
    }
    Ok(())
}

fn safe_toml_string(value: &str) -> bool {
    !value.contains(['"', '\r', '\n'])
}

fn is_protected_relative(path: &Path) -> bool {
    let components: Vec<_> = path
        .components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    components.iter().enumerate().any(|(index, component)| {
        component == ".git"
            || (component == ".mdlite"
                && components
                    .get(index + 1)
                    .is_some_and(|next| next == ".state" || next == ".cache"))
    })
}

fn extend_unique(target: &mut Vec<String>, values: Vec<String>) {
    for value in values {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileFileSnapshot {
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct LoadedProfileLayer {
    pub profiles: Vec<ProfileDefinition>,
    pub snapshot: ProfileFileSnapshot,
}

pub fn load_profile_layer(path: impl AsRef<Path>) -> Result<LoadedProfileLayer, ProfileError> {
    let path = path.as_ref();
    let bytes = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(ProfileError::Io(error)),
    };
    let profiles = match bytes.as_deref() {
        Some(bytes) => parse_profile_file(path, bytes)?,
        None => Vec::new(),
    };
    Ok(LoadedProfileLayer {
        profiles,
        snapshot: ProfileFileSnapshot { bytes },
    })
}

pub fn resolve_profile_layers(
    common: &[ProfileDefinition],
    workspace: &[ProfileDefinition],
) -> Result<Vec<ProfileDefinition>, ProfileError> {
    let mut resolved = default_profiles();
    overlay_profiles(&mut resolved, common)?;
    overlay_profiles(&mut resolved, workspace)?;
    Ok(resolved)
}

fn overlay_profiles(
    destination: &mut Vec<ProfileDefinition>,
    layer: &[ProfileDefinition],
) -> Result<(), ProfileError> {
    let mut ids = HashSet::new();
    for profile in layer {
        validate_profile(profile)?;
        if !ids.insert(profile.id.as_str()) {
            return Err(ProfileError::InvalidDefinition(format!(
                "duplicate profile id: {}",
                profile.id
            )));
        }
        if let Some(existing) = destination
            .iter_mut()
            .find(|existing| existing.id == profile.id)
        {
            *existing = profile.clone();
        } else {
            destination.push(profile.clone());
        }
    }
    Ok(())
}

pub fn save_profile_layer(
    path: impl AsRef<Path>,
    profiles: &[ProfileDefinition],
    snapshot: &ProfileFileSnapshot,
) -> Result<ProfileFileSnapshot, ProfileError> {
    let path = path.as_ref();
    validate_profile_set(profiles)?;
    let current = match fs::read(path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(ProfileError::Io(error)),
    };
    if current != snapshot.bytes {
        return Err(ProfileError::ProfileConflict(path.to_path_buf()));
    }
    let contents = serialize_profile_file(profiles)?;
    atomic_replace(path, contents.as_bytes(), snapshot)?;
    Ok(ProfileFileSnapshot {
        bytes: Some(contents.into_bytes()),
    })
}

pub fn add_or_replace_profile(
    profiles: &mut Vec<ProfileDefinition>,
    profile: ProfileDefinition,
) -> Result<(), ProfileError> {
    validate_profile(&profile)?;
    if let Some(existing) = profiles.iter_mut().find(|entry| entry.id == profile.id) {
        *existing = profile;
    } else {
        profiles.push(profile);
    }
    validate_profile_set(profiles)
}

pub fn duplicate_profile(
    profiles: &[ProfileDefinition],
    source_id: &str,
    new_id: String,
    new_name: String,
) -> Result<ProfileDefinition, ProfileError> {
    let mut duplicate = profiles
        .iter()
        .find(|profile| profile.id == source_id)
        .cloned()
        .ok_or_else(|| {
            ProfileError::InvalidDefinition(format!("profile not found: {source_id}"))
        })?;
    duplicate.id = new_id;
    duplicate.name = new_name;
    validate_profile(&duplicate)?;
    if profiles.iter().any(|profile| profile.id == duplicate.id) {
        return Err(ProfileError::InvalidDefinition(format!(
            "duplicate profile id: {}",
            duplicate.id
        )));
    }
    Ok(duplicate)
}

pub fn remove_profile(profiles: &mut Vec<ProfileDefinition>, id: &str) -> bool {
    let before = profiles.len();
    profiles.retain(|profile| profile.id != id);
    profiles.len() != before
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileTemplateSnapshot {
    bytes: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct LoadedProfileTemplate {
    pub source: String,
    pub snapshot: ProfileTemplateSnapshot,
}

pub fn load_profile_template(
    workspace: &Workspace,
    profile: &ProfileDefinition,
) -> Result<LoadedProfileTemplate, ProfileError> {
    validate_profile(profile)?;
    let relative = PathBuf::from(".mdlite").join(&profile.template_path);
    let path = inspect_workspace_path(workspace, &relative, false)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(path));
            }
            if !metadata.is_file() {
                return Err(ProfileError::NotFile(path));
            }
            let bytes = fs::read(&path)?;
            let source =
                String::from_utf8(bytes.clone()).map_err(|_| ProfileError::InvalidUtf8(path))?;
            Ok(LoadedProfileTemplate {
                source,
                snapshot: ProfileTemplateSnapshot { bytes: Some(bytes) },
            })
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(LoadedProfileTemplate {
            source: String::new(),
            snapshot: ProfileTemplateSnapshot { bytes: None },
        }),
        Err(error) => Err(ProfileError::Io(error)),
    }
}

pub fn save_profile_template(
    workspace: &Workspace,
    profile: &ProfileDefinition,
    source: &str,
    snapshot: &ProfileTemplateSnapshot,
) -> Result<ProfileTemplateSnapshot, ProfileError> {
    validate_profile(profile)?;
    let relative = PathBuf::from(".mdlite").join(&profile.template_path);
    let path = inspect_workspace_path(workspace, &relative, false)?;
    let current = match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(path));
            }
            if !metadata.is_file() {
                return Err(ProfileError::NotFile(path));
            }
            Some(fs::read(&path)?)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(ProfileError::Io(error)),
    };
    if current != snapshot.bytes {
        return Err(ProfileError::ProfileConflict(path));
    }
    let expected = ProfileFileSnapshot {
        bytes: snapshot.bytes.clone(),
    };
    atomic_replace(&path, source.as_bytes(), &expected)?;
    Ok(ProfileTemplateSnapshot {
        bytes: Some(source.as_bytes().to_vec()),
    })
}

fn parse_profile_file(path: &Path, bytes: &[u8]) -> Result<Vec<ProfileDefinition>, ProfileError> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text =
        std::str::from_utf8(bytes).map_err(|_| ProfileError::InvalidUtf8(path.to_path_buf()))?;
    let mut profiles = Vec::new();
    let mut current: Option<ProfileDefinition> = None;
    let mut current_input: Option<ProfileInputDefinition> = None;
    let mut schema_seen = false;

    for (line_index, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[profiles]]" {
            if let Some(input) = current_input.take() {
                current
                    .as_mut()
                    .expect("input always has parent")
                    .inputs
                    .push(input);
            }
            if let Some(profile) = current.take() {
                profiles.push(profile);
            }
            current = Some(blank_profile());
            continue;
        }
        if line == "[[profiles.inputs]]" {
            let profile = current.as_mut().ok_or_else(|| {
                ProfileError::InvalidSchema(
                    "profile input must follow a profile definition".to_owned(),
                )
            })?;
            if let Some(input) = current_input.take() {
                profile.inputs.push(input);
            }
            current_input = Some(ProfileInputDefinition {
                id: String::new(),
                label: String::new(),
                default_value: String::new(),
                required: false,
            });
            continue;
        }
        let (key, value) = line.split_once('=').ok_or_else(|| {
            ProfileError::InvalidSchema(format!("invalid profile file line {}", line_index + 1))
        })?;
        let key = key.trim();
        let value = value.trim();
        if current.is_none() && key == "schema_version" {
            if value != "1" || schema_seen {
                return Err(ProfileError::InvalidSchema(
                    "unsupported or duplicate profile schema_version".to_owned(),
                ));
            }
            schema_seen = true;
            continue;
        }
        let profile = current.as_mut().ok_or_else(|| {
            ProfileError::InvalidSchema("profile field must follow [[profiles]]".to_owned())
        })?;
        if let Some(input) = current_input.as_mut() {
            match key {
                "required" => {
                    input.required = match value {
                        "true" => true,
                        "false" => false,
                        _ => {
                            return Err(ProfileError::InvalidSchema(
                                "profile input required must be true or false".to_owned(),
                            ));
                        }
                    }
                }
                "id" => input.id = parse_quoted(value)?,
                "label" => input.label = parse_quoted(value)?,
                "default" => input.default_value = parse_quoted(value)?,
                _ => {
                    return Err(ProfileError::InvalidSchema(format!(
                        "unknown profile input field: {key}"
                    )));
                }
            }
            continue;
        }
        match key {
            "id" => profile.id = parse_quoted(value)?,
            "name" => profile.name = parse_quoted(value)?,
            "directory" => profile.directory = PathBuf::from(parse_quoted(value)?),
            "filename" => profile.filename = PathBuf::from(parse_quoted(value)?),
            "template" => profile.template_path = PathBuf::from(parse_quoted(value)?),
            "collision" => {
                profile.collision = match parse_quoted(value)?.as_str() {
                    "open-existing" => ProfileCollision::OpenExisting,
                    "sequence" => ProfileCollision::Sequence,
                    _ => {
                        return Err(ProfileError::InvalidSchema(
                            "collision must be open-existing or sequence".to_owned(),
                        ));
                    }
                }
            }
            "sequence_format" => {
                if parse_quoted(value)? != "_%02d" {
                    return Err(ProfileError::InvalidSchema(
                        "sequence_format supports only _%02d".to_owned(),
                    ));
                }
            }
            _ => {
                return Err(ProfileError::InvalidSchema(format!(
                    "unknown profile field: {key}"
                )));
            }
        }
    }
    if let Some(input) = current_input {
        current
            .as_mut()
            .expect("input always has parent")
            .inputs
            .push(input);
    }
    if let Some(profile) = current {
        profiles.push(profile);
    }
    if !schema_seen {
        return Err(ProfileError::InvalidSchema(
            "profiles.toml has no schema_version".to_owned(),
        ));
    }
    validate_profile_set(&profiles)?;
    Ok(profiles)
}

fn blank_profile() -> ProfileDefinition {
    ProfileDefinition {
        id: String::new(),
        name: String::new(),
        directory: PathBuf::new(),
        filename: PathBuf::new(),
        template_path: PathBuf::new(),
        collision: ProfileCollision::OpenExisting,
        inputs: Vec::new(),
    }
}

fn parse_quoted(value: &str) -> Result<String, ProfileError> {
    let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    else {
        return Err(ProfileError::InvalidSchema(
            "profile values must be quoted strings".to_owned(),
        ));
    };
    if !safe_toml_string(value) {
        return Err(ProfileError::InvalidSchema(
            "profile strings cannot contain quotes or line breaks".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

fn validate_profile_set(profiles: &[ProfileDefinition]) -> Result<(), ProfileError> {
    let mut ids = HashSet::new();
    for profile in profiles {
        validate_profile(profile)?;
        if !ids.insert(profile.id.as_str()) {
            return Err(ProfileError::InvalidDefinition(format!(
                "duplicate profile id: {}",
                profile.id
            )));
        }
    }
    Ok(())
}

fn serialize_profile_file(profiles: &[ProfileDefinition]) -> Result<String, ProfileError> {
    validate_profile_set(profiles)?;
    let mut output = String::from("schema_version = 1\n");
    for profile in profiles {
        output.push_str("\n[[profiles]]\n");
        write_field(&mut output, "id", &profile.id);
        write_field(&mut output, "name", &profile.name);
        write_field(&mut output, "directory", &toml_path(&profile.directory));
        write_field(&mut output, "filename", &toml_path(&profile.filename));
        write_field(&mut output, "template", &toml_path(&profile.template_path));
        write_field(
            &mut output,
            "collision",
            match profile.collision {
                ProfileCollision::OpenExisting => "open-existing",
                ProfileCollision::Sequence => "sequence",
            },
        );
        if profile.collision == ProfileCollision::Sequence {
            write_field(&mut output, "sequence_format", "_%02d");
        }
        for input in &profile.inputs {
            output.push_str("\n[[profiles.inputs]]\n");
            write_field(&mut output, "id", &input.id);
            write_field(&mut output, "label", &input.label);
            write_field(&mut output, "default", &input.default_value);
            output.push_str("required = ");
            output.push_str(if input.required { "true\n" } else { "false\n" });
        }
    }
    Ok(output)
}

fn write_field(output: &mut String, key: &str, value: &str) {
    output.push_str(key);
    output.push_str(" = \"");
    output.push_str(value);
    output.push_str("\"\n");
}

fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

fn atomic_replace(
    path: &Path,
    contents: &[u8],
    expected: &ProfileFileSnapshot,
) -> Result<(), ProfileError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(path.to_path_buf()));
            }
            if !metadata.is_file() {
                return Err(ProfileError::NotFile(path.to_path_buf()));
            }
            Some(())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(ProfileError::Io(error)),
    };
    if metadata.is_some() != expected.bytes.is_some() || fs::read(path).ok() != expected.bytes {
        return Err(ProfileError::ProfileConflict(path.to_path_buf()));
    }
    let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".mdlite-profile-{}-{id}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temp);
        return Err(ProfileError::Io(error));
    }
    drop(file);
    if fs::read(path).ok() != expected.bytes {
        let _ = fs::remove_file(&temp);
        return Err(ProfileError::ProfileConflict(path.to_path_buf()));
    }
    if let Err(error) = replace_file(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(ProfileError::Io(error));
    }
    Ok(())
}

fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
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

#[derive(Debug, Default)]
pub struct TemplateInitializationReport {
    pub created_directories: Vec<PathBuf>,
    pub created_files: Vec<PathBuf>,
    pub preserved_files: Vec<PathBuf>,
    pub failures: Vec<TemplateInitializationFailure>,
}

#[derive(Debug)]
pub struct TemplateInitializationFailure {
    pub path: PathBuf,
    pub message: String,
}

impl TemplateInitializationReport {
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }
}

pub fn initialize_default_templates(
    workspace: &Workspace,
) -> Result<TemplateInitializationReport, ProfileError> {
    let mut report = TemplateInitializationReport::default();
    let directories = create_workspace_directories(workspace, Path::new(".mdlite/templates"))?;
    report.created_directories = directories;
    let templates = [
        ("daily", "# {{date:yyyy-MM-dd}}\n\n{{cursor}}\n"),
        ("meeting", "# Meeting {{date:yyyy-MM-dd}}\n\n{{cursor}}\n"),
        ("memo", "# Memo {{date:yyyy-MM-dd}}\n\n{{cursor}}\n"),
    ];
    for (name, contents) in templates {
        let relative = PathBuf::from(".mdlite/templates").join(format!("{name}.md"));
        let path = inspect_workspace_path(workspace, &relative, false)?;
        match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(ProfileError::ReparsePoint(path));
                }
                if !metadata.is_file() {
                    report.failures.push(TemplateInitializationFailure {
                        path,
                        message: "template path is not a regular file".to_owned(),
                    });
                } else {
                    report.preserved_files.push(path);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match create_new_file(&path, contents.as_bytes()) {
                    Ok(()) => report.created_files.push(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(&path)?;
                        if is_reparse_point(&metadata) {
                            return Err(ProfileError::ReparsePoint(path));
                        }
                        if metadata.is_file() {
                            report.preserved_files.push(path);
                        } else {
                            report.failures.push(TemplateInitializationFailure {
                                path,
                                message: "template path is not a regular file".to_owned(),
                            });
                        }
                    }
                    Err(error) => report.failures.push(TemplateInitializationFailure {
                        path,
                        message: error.to_string(),
                    }),
                }
            }
            Err(error) => report.failures.push(TemplateInitializationFailure {
                path,
                message: error.to_string(),
            }),
        }
    }
    Ok(report)
}

#[derive(Debug)]
pub struct NoteCreationResult {
    pub path: PathBuf,
    pub created: bool,
    pub cursor_byte_offset: usize,
    pub created_directories: Vec<PathBuf>,
}

pub fn create_profile_note(
    workspace: &Workspace,
    profile: &ProfileDefinition,
    date_time: ProfileDateTime,
    values: &ProfileValues,
) -> Result<NoteCreationResult, ProfileError> {
    let preview = preview_profile_path(workspace, profile, date_time, values)?;
    let base_relative = preview.relative_path;

    // Daily open-existing succeeds without consulting its template.
    if profile.collision == ProfileCollision::OpenExisting {
        let path = inspect_workspace_path(workspace, &base_relative, false)?;
        if let Some(metadata) = existing_metadata(&path)? {
            if !metadata.is_file() {
                return Err(ProfileError::NotFile(path));
            }
            return Ok(NoteCreationResult {
                path,
                created: false,
                cursor_byte_offset: 0,
                created_directories: Vec::new(),
            });
        }
    }

    let template_relative = PathBuf::from(".mdlite").join(&profile.template_path);
    let template_path = inspect_workspace_path(workspace, &template_relative, true)?;
    let template_bytes = fs::read(&template_path)?;
    let template = String::from_utf8(template_bytes)
        .map_err(|_| ProfileError::InvalidUtf8(template_path.clone()))?;
    let rendered = render_profile_body(profile, &template, date_time, values)?;
    if !rendered.missing_required_inputs.is_empty() {
        return Err(ProfileError::RequiredInputs(
            rendered.missing_required_inputs,
        ));
    }
    let parent_relative = base_relative
        .parent()
        .ok_or_else(|| ProfileError::InvalidPath(base_relative.clone()))?;
    let created_directories = create_workspace_directories(workspace, parent_relative)?;
    let mut sequence = 0;
    loop {
        let relative = if sequence == 0 {
            base_relative.clone()
        } else {
            sequence_path(&base_relative, sequence)?
        };
        validate_rendered_path(&relative)?;
        let path = inspect_workspace_path(workspace, &relative, false)?;
        if let Some(metadata) = existing_metadata(&path)? {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(path));
            }
            if profile.collision == ProfileCollision::OpenExisting {
                if !metadata.is_file() {
                    return Err(ProfileError::NotFile(path));
                }
                return Ok(NoteCreationResult {
                    path,
                    created: false,
                    cursor_byte_offset: 0,
                    created_directories,
                });
            }
            sequence += 1;
            if sequence > 9999 {
                return Err(ProfileError::SequenceExhausted(base_relative));
            }
            continue;
        }
        match create_new_file(&path, rendered.source.as_bytes()) {
            Ok(()) => {
                return Ok(NoteCreationResult {
                    path,
                    created: true,
                    cursor_byte_offset: rendered.cursor_byte_offset,
                    created_directories,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if profile.collision == ProfileCollision::OpenExisting {
                    let metadata = fs::symlink_metadata(&path)?;
                    if is_reparse_point(&metadata) {
                        return Err(ProfileError::ReparsePoint(path));
                    }
                    if !metadata.is_file() {
                        return Err(ProfileError::NotFile(path));
                    }
                    return Ok(NoteCreationResult {
                        path,
                        created: false,
                        cursor_byte_offset: 0,
                        created_directories,
                    });
                }
                sequence += 1;
                if sequence > 9999 {
                    return Err(ProfileError::SequenceExhausted(base_relative));
                }
            }
            Err(error) => return Err(ProfileError::Io(error)),
        }
    }
}

fn sequence_path(path: &Path, sequence: usize) -> Result<PathBuf, ProfileError> {
    let filename = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| ProfileError::InvalidPath(path.to_path_buf()))?;
    let parsed = Path::new(filename);
    let extension = parsed.extension().and_then(OsStr::to_str).unwrap_or("");
    let stem = parsed
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or(filename);
    let name = if extension.is_empty() {
        format!("{stem}_{sequence:02}")
    } else {
        format!("{stem}_{sequence:02}.{extension}")
    };
    Ok(path.parent().unwrap_or(Path::new("")).join(name))
}

fn create_new_file(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        return match fs::remove_file(path) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(io::Error::new(
                cleanup_error.kind(),
                format!("write failed ({error}); partial file remains: {cleanup_error}"),
            )),
        };
    }
    Ok(())
}

fn existing_metadata(path: &Path) -> Result<Option<fs::Metadata>, ProfileError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if is_reparse_point(&metadata) {
                return Err(ProfileError::ReparsePoint(path.to_path_buf()));
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ProfileError::Io(error)),
    }
}

fn create_workspace_directories(
    workspace: &Workspace,
    relative: &Path,
) -> Result<Vec<PathBuf>, ProfileError> {
    validate_rendered_path(relative)?;
    let root = canonical_workspace_root(workspace)?;
    let mut current = root.clone();
    let mut created = Vec::new();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return Err(ProfileError::InvalidPath(relative.to_path_buf()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(ProfileError::ReparsePoint(current));
                }
                if !metadata.is_dir() {
                    return Err(ProfileError::NotDirectory(current));
                }
                ensure_inside(&root, &current)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&current) {
                    Ok(()) => created.push(current.clone()),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        let metadata = fs::symlink_metadata(&current)?;
                        if is_reparse_point(&metadata) {
                            return Err(ProfileError::ReparsePoint(current));
                        }
                        if !metadata.is_dir() {
                            return Err(ProfileError::NotDirectory(current));
                        }
                    }
                    Err(error) => return Err(ProfileError::Io(error)),
                }
            }
            Err(error) => return Err(ProfileError::Io(error)),
        }
    }
    Ok(created)
}

fn inspect_workspace_path(
    workspace: &Workspace,
    relative: &Path,
    must_exist: bool,
) -> Result<PathBuf, ProfileError> {
    validate_relative_path(relative)?;
    if is_protected_relative(relative) {
        return Err(ProfileError::InvalidPath(relative.to_path_buf()));
    }
    let root = canonical_workspace_root(workspace)?;
    let components: Vec<_> = relative.components().collect();
    let mut current = root.clone();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(ProfileError::InvalidPath(relative.to_path_buf()));
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_reparse_point(&metadata) {
                    return Err(ProfileError::ReparsePoint(current));
                }
                ensure_inside(&root, &current)?;
                if index + 1 < components.len() && !metadata.is_dir() {
                    return Err(ProfileError::NotDirectory(current));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if must_exist {
                    return Err(ProfileError::Io(error));
                }
                for remaining in &components[index + 1..] {
                    let Component::Normal(name) = remaining else {
                        return Err(ProfileError::InvalidPath(relative.to_path_buf()));
                    };
                    current.push(name);
                }
                return Ok(current);
            }
            Err(error) => return Err(ProfileError::Io(error)),
        }
    }
    if must_exist && !current.exists() {
        return Err(ProfileError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            "profile path not found",
        )));
    }
    Ok(current)
}

fn canonical_workspace_root(workspace: &Workspace) -> Result<PathBuf, ProfileError> {
    Ok(fs::canonicalize(workspace.root())?)
}

fn ensure_inside(root: &Path, candidate: &Path) -> Result<(), ProfileError> {
    let canonical = fs::canonicalize(candidate)?;
    if canonical.starts_with(root) {
        Ok(())
    } else {
        Err(ProfileError::OutsideWorkspace(candidate.to_path_buf()))
    }
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
                    .join(format!("mdlite-profile-test-{}-{id}", std::process::id()));
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

    fn sample_time() -> ProfileDateTime {
        ProfileDateTime::new(CalendarDate::new(2026, 3, 4).unwrap(), 9, 7).unwrap()
    }

    #[test]
    fn defaults_and_tokens_keep_dairy_path_and_utf8_cursor_offset() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let profiles = default_profiles();
        assert_eq!(
            profiles[0].directory,
            PathBuf::from("Dairy/{{date:yyyy}}/{{date:yyyyMM}}")
        );
        let preview = preview_profile_path(
            &workspace,
            &profiles[0],
            sample_time(),
            &ProfileValues::new(),
        )
        .unwrap();
        assert_eq!(
            preview.relative_path,
            PathBuf::from("Dairy/2026/202603/20260304.md")
        );
        assert_eq!(profiles[0].collision, ProfileCollision::OpenExisting);
        assert_eq!(profiles[1].collision, ProfileCollision::Sequence);
        assert_eq!(
            profiles[2].directory,
            PathBuf::from("Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}")
        );

        let mut profile = profiles[0].clone();
        profile.inputs.push(ProfileInputDefinition {
            id: "title".to_owned(),
            label: "Title".to_owned(),
            default_value: String::new(),
            required: true,
        });
        validate_profile(&profile).unwrap();
        let mut values = ProfileValues::new();
        values.insert("title".to_owned(), "猫".to_owned());
        let rendered = render_profile_body(
            &profile,
            "# {{date:yyyy-MM-dd}}\n猫: {{title}}{{cursor}}",
            sample_time(),
            &values,
        )
        .unwrap();
        assert_eq!(rendered.source, "# 2026-03-04\n猫: 猫");
        assert_eq!(rendered.cursor_byte_offset, "# 2026-03-04\n猫: 猫".len());
        assert!(rendered.missing_required_inputs.is_empty());
    }

    #[test]
    fn required_empty_default_is_valid_for_save_and_preview_but_blocks_creation() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();
        fs::write(
            fixture.0.join(".mdlite/templates/daily.md"),
            "{{input:title}}{{cursor}}",
        )
        .unwrap();
        let mut profile = default_profiles().remove(0);
        profile.inputs.push(ProfileInputDefinition {
            id: "title".to_owned(),
            label: "Title".to_owned(),
            default_value: String::new(),
            required: true,
        });
        validate_profile(&profile).unwrap();
        let preview =
            preview_profile_path(&workspace, &profile, sample_time(), &ProfileValues::new())
                .unwrap();
        assert_eq!(preview.missing_required_inputs, ["title"]);

        assert!(matches!(
            create_profile_note(&workspace, &profile, sample_time(), &ProfileValues::new()),
            Err(ProfileError::RequiredInputs(ids)) if ids == ["title"]
        ));
        assert!(!fixture.0.join("Dairy").exists());
    }

    #[test]
    fn broken_template_does_not_block_opening_an_existing_daily() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        let daily = default_profiles().remove(0);
        let preview =
            preview_profile_path(&workspace, &daily, sample_time(), &ProfileValues::new()).unwrap();
        fs::create_dir_all(preview.path.parent().unwrap()).unwrap();
        fs::write(&preview.path, "existing source").unwrap();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();
        fs::write(fixture.0.join(".mdlite/templates/daily.md"), "{{unknown}}").unwrap();

        let opened =
            create_profile_note(&workspace, &daily, sample_time(), &ProfileValues::new()).unwrap();
        assert!(!opened.created);
        assert_eq!(opened.path, preview.path);
        assert_eq!(fs::read(&opened.path).unwrap(), b"existing source");
    }

    #[test]
    fn create_validates_template_before_directories_and_sequences_collisions() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();
        let meeting = default_profiles().into_iter().nth(1).unwrap();
        fs::write(
            fixture.0.join(".mdlite/templates/meeting.md"),
            "# Meeting {{date:yyyy-MM-dd}}\n{{cursor}}",
        )
        .unwrap();
        let base = preview_profile_path(&workspace, &meeting, sample_time(), &ProfileValues::new())
            .unwrap()
            .path;
        fs::create_dir_all(base.parent().unwrap()).unwrap();
        fs::write(&base, "keep").unwrap();

        let first = create_profile_note(&workspace, &meeting, sample_time(), &ProfileValues::new())
            .unwrap();
        let second =
            create_profile_note(&workspace, &meeting, sample_time(), &ProfileValues::new())
                .unwrap();
        assert_eq!(first.path.file_name().unwrap(), "20260304_01.md");
        assert_eq!(second.path.file_name().unwrap(), "20260304_02.md");
        assert_eq!(fs::read(&base).unwrap(), b"keep");

        let daily = default_profiles().remove(0);
        fs::write(fixture.0.join(".mdlite/templates/daily.md"), "{{unknown}}").unwrap();
        assert!(matches!(
            create_profile_note(&workspace, &daily, sample_time(), &ProfileValues::new()),
            Err(ProfileError::UnknownToken(_))
        ));
        assert!(!fixture.0.join("Dairy").exists());
    }

    #[test]
    fn profile_layers_replace_whole_definitions_and_retain_peer_edits() {
        let fixture = Fixture::new();
        let common_path = fixture.0.join("common.toml");
        let loaded = load_profile_layer(&common_path).unwrap();
        let mut common = default_profiles();
        common[0].name = "Common Daily".to_owned();
        common[0].inputs.push(ProfileInputDefinition {
            id: "project".to_owned(),
            label: "Project".to_owned(),
            default_value: String::new(),
            required: true,
        });
        save_profile_layer(&common_path, &common, &loaded.snapshot).unwrap();
        let loaded_common = load_profile_layer(&common_path).unwrap();
        assert_eq!(loaded_common.profiles, common);

        let mut workspace = default_profiles();
        workspace[0].name = "Workspace Daily".to_owned();
        let resolved = resolve_profile_layers(&loaded_common.profiles, &workspace).unwrap();
        assert_eq!(resolved[0], workspace[0]);

        fs::write(&common_path, "peer change").unwrap();
        assert!(matches!(
            save_profile_layer(&common_path, &common, &loaded_common.snapshot),
            Err(ProfileError::ProfileConflict(_))
        ));
        assert_eq!(fs::read_to_string(common_path).unwrap(), "peer change");
    }

    #[test]
    fn default_templates_are_created_only_when_missing() {
        let fixture = Fixture::new();
        let workspace = fixture.workspace();
        fs::create_dir_all(fixture.0.join(".mdlite/templates")).unwrap();
        let custom = fixture.0.join(".mdlite/templates/daily.md");
        fs::write(&custom, "custom body").unwrap();
        let report = initialize_default_templates(&workspace).unwrap();
        assert!(report.is_complete());
        assert_eq!(report.created_files.len(), 2);
        assert_eq!(report.preserved_files, [fs::canonicalize(&custom).unwrap()]);
        assert_eq!(fs::read(custom).unwrap(), b"custom body");
    }

    #[cfg(unix)]
    #[test]
    fn template_symlink_escape_is_rejected_before_note_creation() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let outside = Fixture::new();
        let workspace = fixture.workspace();
        fs::create_dir_all(fixture.0.join(".mdlite")).unwrap();
        symlink(&outside.0, fixture.0.join(".mdlite/templates")).unwrap();
        let daily = default_profiles().remove(0);
        assert!(matches!(
            create_profile_note(&workspace, &daily, sample_time(), &ProfileValues::new()),
            Err(ProfileError::ReparsePoint(_))
        ));
        assert!(!fixture.0.join("Dairy").exists());
    }
}
