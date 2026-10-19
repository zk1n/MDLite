//! Layered settings parsing and snapshot-checked updates.
//!
//! Settings use a deliberately small subset of the legacy schema. Unknown
//! lines and sections stay in the source text; updates replace only selected
//! root assignments and use `FileSnapshot` for safe saves.

use crate::storage::{FileFormat, FileSnapshot, StorageError};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs, io,
    path::{Component, Path, PathBuf},
};

const SCHEMA_VERSION: &str = "1";
pub const COLOR_NAMES: &[&str] = &[
    "background",
    "foreground",
    "link",
    "heading",
    "marker",
    "code_background",
    "table_background",
];
pub const COMMAND_NAMES: &[&str] = &[
    "file.new",
    "file.open",
    "file.save",
    "file.quickOpen",
    "file.close",
    "edit.find",
    "edit.findNext",
    "edit.findPrevious",
    "edit.findWorkspace",
    "view.commandPalette",
];

/// A `None` field inherits its value from the lower settings layer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsLayer {
    pub auto_save: Option<bool>,
    pub auto_save_delay_ms: Option<u32>,
    pub theme: Option<ThemeMode>,
    pub font_face: Option<String>,
    pub font_size_pt: Option<u32>,
    pub default_memo_workspace: Option<PathBuf>,
    pub keybindings: BTreeMap<String, String>,
    pub colors: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
    Custom,
}

impl ThemeMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
            Self::Custom => "custom",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }
}

/// Concrete settings after standard, common, then Workspace overlays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveSettings {
    pub auto_save: bool,
    pub auto_save_delay_ms: u32,
    pub theme: ThemeMode,
    pub font_face: String,
    pub font_size_pt: u32,
    pub default_memo_workspace: PathBuf,
    pub keybindings: BTreeMap<String, String>,
    pub colors: BTreeMap<String, String>,
}

impl Default for EffectiveSettings {
    fn default() -> Self {
        Self {
            auto_save: true,
            auto_save_delay_ms: 750,
            theme: ThemeMode::Dark,
            font_face: "Segoe UI".to_owned(),
            font_size_pt: 11,
            default_memo_workspace: PathBuf::new(),
            keybindings: BTreeMap::from([
                ("file.new".to_owned(), "Ctrl+N".to_owned()),
                ("file.open".to_owned(), "Ctrl+O".to_owned()),
                ("file.save".to_owned(), "Ctrl+S".to_owned()),
                ("file.quickOpen".to_owned(), "Ctrl+P".to_owned()),
                ("file.close".to_owned(), "Ctrl+W".to_owned()),
                ("edit.find".to_owned(), "Ctrl+F".to_owned()),
                ("edit.findNext".to_owned(), "F3".to_owned()),
                ("edit.findPrevious".to_owned(), "Shift+F3".to_owned()),
                ("edit.findWorkspace".to_owned(), "Ctrl+Shift+F".to_owned()),
                ("view.commandPalette".to_owned(), "Ctrl+K".to_owned()),
            ]),
            colors: BTreeMap::new(),
        }
    }
}

impl EffectiveSettings {
    pub fn overlay(&mut self, layer: &SettingsLayer) {
        if let Some(value) = layer.auto_save {
            self.auto_save = value;
        }
        if let Some(value) = layer.auto_save_delay_ms {
            self.auto_save_delay_ms = value;
        }
        if let Some(value) = layer.theme {
            self.theme = value;
        }
        if let Some(value) = &layer.font_face {
            self.font_face.clone_from(value);
        }
        if let Some(value) = layer.font_size_pt {
            self.font_size_pt = value;
        }
        if let Some(value) = &layer.default_memo_workspace {
            self.default_memo_workspace.clone_from(value);
        }
        self.keybindings.extend(layer.keybindings.clone());
        self.colors.extend(layer.colors.clone());
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SettingValue {
    Bool(bool),
    Integer(u32),
    Theme(ThemeMode),
    Text(String),
    Path(PathBuf),
}

/// A present map entry is selected by the user. `Some(value)` sets an
/// override; `None` removes it so the next lower layer is inherited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SettingsPatch {
    pub changes: BTreeMap<String, Option<SettingValue>>,
}

impl SettingsPatch {
    pub fn set(&mut self, key: impl Into<String>, value: SettingValue) {
        self.changes.insert(key.into(), Some(value));
    }

    pub fn inherit(&mut self, key: impl Into<String>) {
        self.changes.insert(key.into(), None);
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiagnosticKind {
    MissingSchema,
    UnsupportedSchema,
    MalformedLine,
    MalformedValue,
    DuplicateManagedKey,
    UnknownManagedKey,
    InvalidShortcut,
    BindingConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsDiagnostic {
    pub kind: DiagnosticKind,
    pub line: Option<usize>,
    pub key: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedSettings {
    pub layer: SettingsLayer,
    pub diagnostics: Vec<SettingsDiagnostic>,
    schema_supported: bool,
    duplicate_managed_keys: BTreeSet<String>,
}

impl ParsedSettings {
    pub fn schema_supported(&self) -> bool {
        self.schema_supported
    }

    pub fn has_duplicate_managed_keys(&self) -> bool {
        !self.duplicate_managed_keys.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsResolution {
    pub settings: EffectiveSettings,
    pub diagnostics: Vec<SettingsDiagnostic>,
}

impl SettingsResolution {
    pub fn is_valid(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Resolve the default, common, and Workspace layers in priority order.
/// Invalid individual values have already been omitted by the parser. If the
/// combined bindings conflict, the defaults remain active as a safe fallback.
pub fn resolve_settings(
    common: Option<&ParsedSettings>,
    workspace: Option<&ParsedSettings>,
) -> SettingsResolution {
    let mut settings = EffectiveSettings::default();
    let mut diagnostics = Vec::new();
    for parsed in [common, workspace].into_iter().flatten() {
        diagnostics.extend(parsed.diagnostics.clone());
        settings.overlay(&parsed.layer);
    }
    if let Err(mut binding_diagnostics) = validate_bindings(&settings.keybindings) {
        diagnostics.append(&mut binding_diagnostics);
        settings.keybindings = EffectiveSettings::default().keybindings;
    }
    SettingsResolution {
        settings,
        diagnostics,
    }
}

/// Parse the supported root-level settings. A UTF-8 BOM is accepted, matching
/// the UTF-8 BOM handling used by `FileSnapshot`.
pub fn parse_settings(source: &str) -> ParsedSettings {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let document = scan_document(source);
    let mut parsed = ParsedSettings {
        diagnostics: document.diagnostics.clone(),
        ..ParsedSettings::default()
    };
    let mut schema: Option<&Assignment> = None;
    let mut counts = BTreeMap::<String, usize>::new();
    let mut duplicate_lines = BTreeMap::<String, usize>::new();

    for assignment in &document.assignments {
        if assignment.key == "schema_version" {
            schema = Some(assignment);
        }
        if is_managed_key(&assignment.key) {
            let count = counts.entry(assignment.key.clone()).or_default();
            *count += 1;
            if *count == 2 {
                duplicate_lines.insert(assignment.key.clone(), assignment.line);
            }
        }
    }
    for (key, line) in duplicate_lines {
        parsed.duplicate_managed_keys.insert(key.clone());
        parsed.diagnostics.push(diagnostic(
            DiagnosticKind::DuplicateManagedKey,
            Some(line),
            Some(key.clone()),
            format!("duplicate managed setting `{key}`; neither value is used"),
        ));
    }

    let schema_count = counts.get("schema_version").copied().unwrap_or(0);
    match (schema, schema_count) {
        (None, _) => parsed.diagnostics.push(diagnostic(
            DiagnosticKind::MissingSchema,
            None,
            Some("schema_version".to_owned()),
            "settings file has no root schema_version = 1".to_owned(),
        )),
        (Some(_), count) if count != 1 => {
            // The duplicate-key diagnostic above explains why this schema is unsafe.
        }
        (Some(assignment), 1) => {
            if scalar_value(source, assignment) == Some(SCHEMA_VERSION) {
                parsed.schema_supported = true;
            } else {
                parsed.diagnostics.push(diagnostic(
                    DiagnosticKind::UnsupportedSchema,
                    Some(assignment.line),
                    Some("schema_version".to_owned()),
                    "settings schema is not supported; defaults are active".to_owned(),
                ));
            }
        }
        _ => {}
    }

    if !parsed.schema_supported {
        return parsed;
    }

    for assignment in &document.assignments {
        if assignment.key == "schema_version"
            || parsed.duplicate_managed_keys.contains(&assignment.key)
        {
            continue;
        }
        let value = match scalar_value(source, assignment) {
            Some(value) => value,
            None => {
                if is_managed_key(&assignment.key) {
                    parsed.diagnostics.push(diagnostic(
                        DiagnosticKind::MalformedValue,
                        Some(assignment.line),
                        Some(assignment.key.clone()),
                        "managed setting has an empty value".to_owned(),
                    ));
                }
                continue;
            }
        };
        if let Err(error) = parse_setting(&assignment.key, value, &mut parsed.layer) {
            let kind = if assignment.key.starts_with("bind.")
                && matches!(&error, SettingParseError::Shortcut(_))
            {
                DiagnosticKind::InvalidShortcut
            } else if matches!(&error, SettingParseError::UnknownKey) {
                DiagnosticKind::UnknownManagedKey
            } else {
                DiagnosticKind::MalformedValue
            };
            parsed.diagnostics.push(diagnostic(
                kind,
                Some(assignment.line),
                Some(assignment.key.clone()),
                error.to_string(),
            ));
        }
    }

    parsed
}

#[derive(Debug)]
pub enum SettingsError {
    Io(io::Error),
    Storage(StorageError),
    InvalidPath(&'static str),
    UnsupportedEncoding,
    UnsupportedSchema,
    DuplicateManagedKeys(Vec<String>),
    InvalidPatch { key: String, message: String },
    BindingConflicts(Vec<SettingsDiagnostic>),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::Storage(error) => error.fmt(f),
            Self::InvalidPath(message) => f.write_str(message),
            Self::UnsupportedEncoding => f.write_str("settings files must be UTF-8"),
            Self::UnsupportedSchema => {
                f.write_str("settings schema is missing or unsupported; file was left unchanged")
            }
            Self::DuplicateManagedKeys(keys) => write!(
                f,
                "duplicate managed settings must be resolved before Apply: {}",
                keys.join(", ")
            ),
            Self::InvalidPatch { key, message } => {
                write!(f, "invalid setting `{key}`: {message}")
            }
            Self::BindingConflicts(diagnostics) => {
                let messages = diagnostics
                    .iter()
                    .map(|item| item.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "keybinding conflicts prevent Apply: {messages}")
            }
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Storage(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for SettingsError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<StorageError> for SettingsError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

enum CapturedFile {
    Missing,
    Existing(FileSnapshot),
}

/// A captured settings form. Its target is canonicalized and private, so an
/// Apply cannot be redirected by changing the selected scope after capture.
pub struct CapturedSettings {
    target: PathBuf,
    file: CapturedFile,
    source: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApplyOutcome {
    pub wrote_file: bool,
}

impl CapturedSettings {
    pub fn capture(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let path = path.as_ref();
        match fs::symlink_metadata(path) {
            Ok(_) => {
                let loaded = FileSnapshot::open(path)?;
                if loaded.snapshot.format() == FileFormat::Cp932 {
                    return Err(SettingsError::UnsupportedEncoding);
                }
                let target = loaded.snapshot.path().to_path_buf();
                Ok(Self {
                    target,
                    file: CapturedFile::Existing(loaded.snapshot),
                    source: loaded.source,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self {
                target: canonical_missing_target(path)?,
                file: CapturedFile::Missing,
                source: String::new(),
            }),
            Err(error) => Err(SettingsError::Io(error)),
        }
    }

    pub fn target(&self) -> &Path {
        &self.target
    }

    /// A fresh parse is returned for display. Dropping this capture, including
    /// on Cancel, performs no write.
    pub fn parsed(&self) -> ParsedSettings {
        match &self.file {
            CapturedFile::Missing => ParsedSettings {
                schema_supported: true,
                ..ParsedSettings::default()
            },
            CapturedFile::Existing(_) => parse_settings(&self.source),
        }
    }

    pub fn apply(&mut self, patch: &SettingsPatch) -> Result<ApplyOutcome, SettingsError> {
        self.apply_with_layers(patch, &[], &[])
    }

    /// Apply with effective layers below and above this file. The final merged
    /// binding set is checked before any bytes are written.
    pub fn apply_with_layers(
        &mut self,
        patch: &SettingsPatch,
        layers_below: &[&SettingsLayer],
        layers_above: &[&SettingsLayer],
    ) -> Result<ApplyOutcome, SettingsError> {
        if patch.is_empty() {
            return Ok(ApplyOutcome { wrote_file: false });
        }

        let current = self.parsed();
        if !current.schema_supported {
            return Err(SettingsError::UnsupportedSchema);
        }
        if current.has_duplicate_managed_keys() {
            return Err(SettingsError::DuplicateManagedKeys(
                current.duplicate_managed_keys.into_iter().collect(),
            ));
        }
        for (key, change) in &patch.changes {
            if let Some(value) = change {
                validate_patch_value(key, value).map_err(|message| {
                    SettingsError::InvalidPatch {
                        key: key.clone(),
                        message,
                    }
                })?;
            } else if !is_supported_key(key) {
                return Err(SettingsError::InvalidPatch {
                    key: key.clone(),
                    message: "unknown setting key".to_owned(),
                });
            }
        }

        let updated = patch_source(
            &self.source,
            &patch.changes,
            matches!(&self.file, CapturedFile::Missing),
        );
        if updated == self.source {
            return Ok(ApplyOutcome { wrote_file: false });
        }

        let parsed_updated = parse_settings(&updated);
        if !parsed_updated.schema_supported {
            return Err(SettingsError::UnsupportedSchema);
        }
        if parsed_updated.has_duplicate_managed_keys() {
            return Err(SettingsError::DuplicateManagedKeys(
                parsed_updated.duplicate_managed_keys.into_iter().collect(),
            ));
        }
        let mut effective = EffectiveSettings::default();
        for layer in layers_below {
            effective.overlay(layer);
        }
        effective.overlay(&parsed_updated.layer);
        for layer in layers_above {
            effective.overlay(layer);
        }
        if let Err(diagnostics) = validate_bindings(&effective.keybindings) {
            return Err(SettingsError::BindingConflicts(diagnostics));
        }

        if matches!(&self.file, CapturedFile::Missing) {
            match fs::symlink_metadata(&self.target) {
                Ok(_) => return Err(SettingsError::Storage(StorageError::DestinationExists)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(SettingsError::Io(error)),
            }
            let parent = self
                .target
                .parent()
                .ok_or(SettingsError::InvalidPath("settings target has no parent"))?;
            fs::create_dir_all(parent)?;
            let file_name = self
                .target
                .file_name()
                .ok_or(SettingsError::InvalidPath("settings path has no file name"))?;
            if fs::canonicalize(parent)?.join(file_name) != self.target {
                return Err(SettingsError::InvalidPath(
                    "settings target parent changed after capture",
                ));
            }
            let snapshot =
                FileSnapshot::create_new(&self.target, &updated, FileFormat::Utf8 { bom: false })?;
            self.file = CapturedFile::Existing(snapshot);
        } else if let CapturedFile::Existing(snapshot) = &mut self.file {
            snapshot.save(&updated)?;
        }
        self.source = updated;
        Ok(ApplyOutcome { wrote_file: true })
    }
}

#[derive(Debug)]
struct Assignment {
    key: String,
    line: usize,
    line_start: usize,
    value_start: usize,
    value_end: usize,
    line_end: usize,
    comment_start: Option<usize>,
}

#[derive(Debug, Default)]
struct ScannedDocument {
    assignments: Vec<Assignment>,
    diagnostics: Vec<SettingsDiagnostic>,
    first_section: Option<usize>,
    preferred_newline: Option<String>,
}

fn scan_document(source: &str) -> ScannedDocument {
    let mut document = ScannedDocument::default();
    let mut offset = 0;
    let mut line_number = 1;
    let mut in_section = false;

    while offset < source.len() {
        let rest = &source[offset..];
        let newline_index = rest.find(['\r', '\n']);
        let content_end = newline_index.map_or(source.len(), |index| offset + index);
        let line_end = match newline_index {
            Some(index) => {
                let newline_at = offset + index;
                if source.as_bytes()[newline_at] == b'\r'
                    && source.as_bytes().get(newline_at + 1) == Some(&b'\n')
                {
                    newline_at + 2
                } else {
                    newline_at + 1
                }
            }
            None => source.len(),
        };
        if document.preferred_newline.is_none() && content_end < line_end {
            document.preferred_newline = Some(source[content_end..line_end].to_owned());
        }
        let body = &source[offset..content_end];
        let trimmed = body.trim_start();
        if trimmed.starts_with('[') {
            in_section = true;
            document.first_section.get_or_insert(offset);
        } else if !in_section && !trimmed.is_empty() && !trimmed.starts_with('#') {
            match parse_assignment(body, offset, line_end, line_number) {
                Some(assignment) => document.assignments.push(assignment),
                None => document.diagnostics.push(diagnostic(
                    DiagnosticKind::MalformedLine,
                    Some(line_number),
                    None,
                    "settings line is not a root key/value assignment".to_owned(),
                )),
            }
        }
        offset = line_end;
        line_number += 1;
    }
    document
}

fn parse_assignment(
    body: &str,
    line_start: usize,
    line_end: usize,
    line: usize,
) -> Option<Assignment> {
    let equals = body.find('=')?;
    let key = body[..equals].trim();
    if key.is_empty() {
        return None;
    }
    let after_equals = equals + 1;
    let raw_tail = &body[after_equals..];
    let leading = raw_tail.len() - raw_tail.trim_start().len();
    let value_start_in_line = after_equals + leading;
    let value_tail = &body[value_start_in_line..];
    let comment_in_tail = find_comment(value_tail);
    let before_comment = comment_in_tail.map_or(value_tail, |index| &value_tail[..index]);
    let value_content = before_comment.trim_end();
    let value_end_in_line = value_start_in_line + value_content.len();
    Some(Assignment {
        key: key.to_owned(),
        line,
        line_start,
        value_start: line_start + value_start_in_line,
        value_end: line_start + value_end_in_line,
        line_end,
        comment_start: comment_in_tail.map(|index| line_start + value_start_in_line + index),
    })
}

fn find_comment(value: &str) -> Option<usize> {
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if character == '"' && !escaped {
            quoted = !quoted;
        } else if character == '#' && !quoted {
            return Some(index);
        }
        escaped = character == '\\' && !escaped;
    }
    None
}

fn scalar_value<'a>(source: &'a str, assignment: &Assignment) -> Option<&'a str> {
    let value = source
        .get(assignment.value_start..assignment.value_end)?
        .trim();
    (!value.is_empty()).then_some(value)
}

fn is_managed_key(key: &str) -> bool {
    matches!(
        key,
        "schema_version"
            | "auto_save"
            | "auto_save_delay_ms"
            | "theme"
            | "font_face"
            | "font_size_pt"
            | "default_memo_workspace"
    ) || key.starts_with("bind.")
        || key.starts_with("color.")
}

fn is_supported_key(key: &str) -> bool {
    matches!(
        key,
        "auto_save"
            | "auto_save_delay_ms"
            | "theme"
            | "font_face"
            | "font_size_pt"
            | "default_memo_workspace"
    ) || key
        .strip_prefix("bind.")
        .is_some_and(|name| COMMAND_NAMES.contains(&name))
        || key
            .strip_prefix("color.")
            .is_some_and(|name| COLOR_NAMES.contains(&name))
}

fn parse_setting(key: &str, raw: &str, layer: &mut SettingsLayer) -> Result<(), SettingParseError> {
    match key {
        "auto_save" => match raw {
            "true" => layer.auto_save = Some(true),
            "false" => layer.auto_save = Some(false),
            _ => return Err(SettingParseError::Invalid("expected true or false")),
        },
        "auto_save_delay_ms" => {
            let value = raw
                .parse::<u32>()
                .map_err(|_| SettingParseError::Invalid("expected an integer"))?;
            if !(100..=60_000).contains(&value) {
                return Err(SettingParseError::Invalid("expected 100..=60000 ms"));
            }
            layer.auto_save_delay_ms = Some(value);
        }
        "theme" => {
            let value =
                parse_quoted(raw).ok_or(SettingParseError::Invalid("expected a quoted theme"))?;
            layer.theme = Some(ThemeMode::parse(value).ok_or(SettingParseError::Invalid(
                "expected system/light/dark/custom",
            ))?);
        }
        "font_face" => {
            let value =
                parse_quoted(raw).ok_or(SettingParseError::Invalid("expected quoted text"))?;
            validate_font_face(value).map_err(SettingParseError::Invalid)?;
            layer.font_face = Some(value.to_owned());
        }
        "font_size_pt" => {
            let value = raw
                .parse::<u32>()
                .map_err(|_| SettingParseError::Invalid("expected an integer"))?;
            if !(6..=96).contains(&value) {
                return Err(SettingParseError::Invalid("expected 6..=96 pt"));
            }
            layer.font_size_pt = Some(value);
        }
        "default_memo_workspace" => {
            let value =
                parse_quoted(raw).ok_or(SettingParseError::Invalid("expected a quoted path"))?;
            let path = PathBuf::from(value);
            if !value.is_empty() && !path.is_absolute() {
                return Err(SettingParseError::Invalid("path must be absolute or empty"));
            }
            layer.default_memo_workspace = Some(path);
        }
        _ if key.starts_with("bind.") => {
            let command = &key[5..];
            if !COMMAND_NAMES.contains(&command) {
                return Err(SettingParseError::UnknownKey);
            }
            let shortcut = parse_quoted(raw)
                .ok_or(SettingParseError::Shortcut("expected a quoted shortcut"))?;
            validate_shortcut(shortcut).map_err(SettingParseError::Shortcut)?;
            layer
                .keybindings
                .insert(command.to_owned(), shortcut.to_owned());
        }
        _ if key.starts_with("color.") => {
            let name = &key[6..];
            if !COLOR_NAMES.contains(&name) {
                return Err(SettingParseError::UnknownKey);
            }
            let color =
                parse_quoted(raw).ok_or(SettingParseError::Invalid("expected a quoted color"))?;
            if !valid_color(color) {
                return Err(SettingParseError::Invalid("expected #RRGGBB"));
            }
            layer.colors.insert(name.to_owned(), color.to_owned());
        }
        _ => {}
    }
    Ok(())
}

#[derive(Debug)]
enum SettingParseError {
    Invalid(&'static str),
    Shortcut(&'static str),
    UnknownKey,
}

impl fmt::Display for SettingParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => f.write_str(message),
            Self::Shortcut(message) => write!(f, "invalid shortcut: {message}"),
            Self::UnknownKey => f.write_str("unknown managed setting key"),
        }
    }
}

fn parse_quoted(value: &str) -> Option<&str> {
    let value = value.trim();
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    if inner.contains('"') || inner.contains(['\r', '\n', '\0']) {
        None
    } else {
        Some(inner)
    }
}

fn validate_font_face(value: &str) -> Result<(), &'static str> {
    if value.is_empty()
        || value.encode_utf16().count() > 31
        || value.contains(['"', '\r', '\n', '\0'])
    {
        Err("font face must contain 1..=31 UTF-16 units")
    } else {
        Ok(())
    }
}

fn valid_color(value: &str) -> bool {
    let Some(hex) = value.strip_prefix('#') else {
        return false;
    };
    hex.len() == 6 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn normalize_shortcut(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

fn validate_shortcut(value: &str) -> Result<String, &'static str> {
    if value.contains(['"', '\r', '\n', '\0']) {
        return Err("shortcut contains an unsupported character");
    }
    let normalized = normalize_shortcut(value);
    if normalized.is_empty() || normalized == "NONE" {
        return Ok(normalized);
    }
    let tokens = normalized.split('+').collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err("missing key");
    }
    let mut modifiers = BTreeSet::new();
    for token in &tokens[..tokens.len() - 1] {
        if !matches!(*token, "CTRL" | "SHIFT" | "ALT") || !modifiers.insert(*token) {
            return Err("modifier list is invalid");
        }
    }
    let key = tokens[tokens.len() - 1];
    if key.is_empty() {
        return Err("missing key");
    }
    let valid_letter_or_digit = key.len() == 1
        && key
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric());
    let valid_function = key
        .strip_prefix('F')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number));
    if valid_letter_or_digit && modifiers.is_empty() {
        Err("printable keys require a modifier")
    } else if valid_letter_or_digit || valid_function {
        Ok(normalized)
    } else {
        Err("supported keys are A-Z, 0-9, and F1-F24")
    }
}

fn validate_patch_value(key: &str, value: &SettingValue) -> Result<(), String> {
    let invalid_type = || "value type does not match this setting".to_owned();
    match (key, value) {
        ("auto_save", SettingValue::Bool(_)) => Ok(()),
        ("auto_save_delay_ms", SettingValue::Integer(value)) if (100..=60_000).contains(value) => {
            Ok(())
        }
        ("auto_save_delay_ms", SettingValue::Integer(_)) => {
            Err("expected 100..=60000 ms".to_owned())
        }
        ("theme", SettingValue::Theme(_)) => Ok(()),
        ("font_face", SettingValue::Text(value)) => {
            validate_font_face(value).map_err(str::to_owned)
        }
        ("font_size_pt", SettingValue::Integer(value)) if (6..=96).contains(value) => Ok(()),
        ("font_size_pt", SettingValue::Integer(_)) => Err("expected 6..=96 pt".to_owned()),
        ("default_memo_workspace", SettingValue::Path(value)) => {
            let text = value.to_string_lossy();
            if text.contains(['"', '\r', '\n', '\0']) {
                Err("path contains a character that cannot be saved".to_owned())
            } else if value.as_os_str().is_empty() || value.is_absolute() {
                Ok(())
            } else {
                Err("path must be absolute or empty".to_owned())
            }
        }
        (key, SettingValue::Text(value)) if key.starts_with("bind.") => {
            let command = &key[5..];
            if !COMMAND_NAMES.contains(&command) {
                return Err("unknown binding command".to_owned());
            }
            validate_shortcut(value).map(|_| ()).map_err(str::to_owned)
        }
        (key, SettingValue::Text(value)) if key.starts_with("color.") => {
            let name = &key[6..];
            if !COLOR_NAMES.contains(&name) {
                return Err("unknown custom color".to_owned());
            }
            if valid_color(value) {
                Ok(())
            } else {
                Err("expected #RRGGBB".to_owned())
            }
        }
        _ if !is_supported_key(key) => Err("unknown setting key".to_owned()),
        _ => Err(invalid_type()),
    }
}

fn validate_bindings(bindings: &BTreeMap<String, String>) -> Result<(), Vec<SettingsDiagnostic>> {
    let mut owners = BTreeMap::<String, String>::new();
    let mut diagnostics = Vec::new();
    for (command, shortcut) in bindings {
        if !COMMAND_NAMES.contains(&command.as_str()) {
            diagnostics.push(diagnostic(
                DiagnosticKind::UnknownManagedKey,
                None,
                Some(format!("bind.{command}")),
                format!("binding command `{command}` is not implemented"),
            ));
            continue;
        }
        let normalized = match validate_shortcut(shortcut) {
            Ok(value) => value,
            Err(message) => {
                diagnostics.push(diagnostic(
                    DiagnosticKind::InvalidShortcut,
                    None,
                    Some(format!("bind.{command}")),
                    format!("invalid shortcut `{shortcut}`: {message}"),
                ));
                continue;
            }
        };
        if normalized.is_empty() || normalized == "NONE" {
            continue;
        }
        if let Some(owner) = FIXED_SHORTCUTS
            .iter()
            .find_map(|(shortcut, owner)| (*shortcut == normalized.as_str()).then_some(*owner))
            && owner != command
        {
            diagnostics.push(diagnostic(
                DiagnosticKind::BindingConflict,
                None,
                Some(format!("bind.{command}")),
                format!("{shortcut} is reserved for {owner}"),
            ));
            continue;
        }
        if let Some(owner) = owners.insert(normalized, command.clone()) {
            diagnostics.push(diagnostic(
                DiagnosticKind::BindingConflict,
                None,
                Some(format!("bind.{command}")),
                format!("{shortcut} conflicts with binding for {owner}"),
            ));
        }
    }
    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

const FIXED_SHORTCUTS: &[(&str, &str)] = &[
    ("CTRL+N", "file.new"),
    ("CTRL+O", "file.open"),
    ("CTRL+S", "file.save"),
    ("CTRL+SHIFT+S", "file.saveAs"),
    ("CTRL+P", "file.quickOpen"),
    ("CTRL+W", "file.close"),
    ("CTRL+F", "edit.find"),
    ("CTRL+SHIFT+F", "edit.findWorkspace"),
    ("F3", "edit.findNext"),
    ("SHIFT+F3", "edit.findPrevious"),
    ("CTRL+K", "view.commandPalette"),
    ("CTRL+Z", "edit.undo"),
    ("CTRL+Y", "edit.redo"),
    ("CTRL+SHIFT+Z", "edit.redo"),
    ("CTRL+A", "edit.selectAll"),
    ("CTRL+C", "edit.copy"),
    ("CTRL+X", "edit.cut"),
    ("CTRL+V", "edit.paste"),
    ("CTRL+E", "view.raw"),
    ("CTRL+TAB", "tab.next"),
    ("CTRL+SHIFT+TAB", "tab.previous"),
    ("CTRL+SHIFT+T", "table.source"),
    ("CTRL+ALT+T", "table.toolbar"),
    ("CTRL+ALT+UP", "panel.resize.grow"),
    ("CTRL+ALT+DOWN", "panel.resize.shrink"),
    ("CTRL+ALT+1", "panel.move.left_top"),
    ("CTRL+ALT+2", "panel.move.left_bottom"),
    ("CTRL+ALT+3", "panel.move.right_top"),
    ("CTRL+ALT+4", "panel.move.right_bottom"),
];

fn patch_source(
    source: &str,
    changes: &BTreeMap<String, Option<SettingValue>>,
    initialize: bool,
) -> String {
    let document = scan_document(source);
    let mut replacements = Vec::<(usize, usize, String)>::new();
    let mut existing = BTreeMap::<String, &Assignment>::new();
    for assignment in &document.assignments {
        if is_supported_key(&assignment.key) {
            existing.insert(assignment.key.clone(), assignment);
        }
    }

    let mut additions = Vec::<(String, String)>::new();
    for (key, value) in changes {
        match (value, existing.get(key).copied()) {
            (Some(value), Some(assignment)) => {
                replacements.push((
                    assignment.value_start,
                    assignment.value_end,
                    format_setting_value(key, value),
                ));
            }
            (None, Some(assignment)) => {
                if let Some(comment_start) = assignment.comment_start {
                    let line = &source[assignment.line_start..assignment.line_end];
                    let indent = line.len() - line.trim_start().len();
                    let ending_start = line.trim_end_matches(['\r', '\n']).len();
                    let preserved_comment =
                        &source[comment_start..assignment.line_start + ending_start];
                    replacements.push((
                        assignment.line_start,
                        assignment.line_end,
                        format!(
                            "{}{}{}",
                            &line[..indent],
                            preserved_comment,
                            &source[assignment.line_start + ending_start..assignment.line_end]
                        ),
                    ));
                } else {
                    replacements.push((assignment.line_start, assignment.line_end, String::new()));
                }
            }
            (Some(value), None) => additions.push((key.clone(), format_setting_value(key, value))),
            (None, None) => {}
        }
    }

    let mut result = source.to_owned();
    replacements.sort_by_key(|(start, _, _)| std::cmp::Reverse(*start));
    for (start, end, replacement) in replacements {
        result.replace_range(start..end, &replacement);
    }

    if initialize && !additions.is_empty() {
        additions.insert(0, ("schema_version".to_owned(), SCHEMA_VERSION.to_owned()));
    }
    if !additions.is_empty() {
        let newline = document
            .preferred_newline
            .unwrap_or_else(|| "\n".to_owned());
        additions.sort_by(|left, right| left.0.cmp(&right.0));
        if initialize {
            additions.sort_by(|left, right| {
                if left.0 == "schema_version" {
                    std::cmp::Ordering::Less
                } else if right.0 == "schema_version" {
                    std::cmp::Ordering::Greater
                } else {
                    left.0.cmp(&right.0)
                }
            });
        }
        let mut insert = additions
            .into_iter()
            .map(|(key, value)| format!("{key} = {value}"))
            .collect::<Vec<_>>()
            .join(&newline);
        insert.push_str(&newline);
        let scan_after = scan_document(&result);
        if let Some(position) = scan_after.first_section {
            result.insert_str(position, &insert);
        } else {
            if !result.is_empty() && !result.ends_with(['\r', '\n']) {
                result.push_str(&newline);
            }
            result.push_str(&insert);
        }
    }
    result
}

fn format_setting_value(key: &str, value: &SettingValue) -> String {
    match (key, value) {
        ("auto_save", SettingValue::Bool(value)) => value.to_string(),
        ("auto_save_delay_ms" | "font_size_pt", SettingValue::Integer(value)) => value.to_string(),
        ("theme", SettingValue::Theme(value)) => format!("\"{}\"", value.as_str()),
        ("font_face", SettingValue::Text(value)) => format!("\"{value}\""),
        ("default_memo_workspace", SettingValue::Path(value)) => {
            let portable = value.to_string_lossy().replace('\\', "/");
            format!("\"{portable}\"")
        }
        (key, SettingValue::Text(value))
            if key.starts_with("bind.") || key.starts_with("color.") =>
        {
            format!("\"{value}\"")
        }
        _ => String::new(),
    }
}

fn canonical_missing_target(path: &Path) -> Result<PathBuf, SettingsError> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    let file_name = normalized
        .file_name()
        .ok_or(SettingsError::InvalidPath("settings path has no file name"))?;
    let mut parent = normalized
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(SettingsError::InvalidPath("settings path has no parent"))?
        .to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(&parent) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                canonical.push(file_name);
                return Ok(canonical);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = parent
                    .file_name()
                    .ok_or(SettingsError::InvalidPath("settings parent has no name"))?;
                missing.push(name.to_os_string());
                parent = parent
                    .parent()
                    .ok_or(SettingsError::InvalidPath(
                        "settings parent has no ancestor",
                    ))?
                    .to_path_buf();
            }
            Err(error) => return Err(SettingsError::Io(error)),
        }
    }
}

fn diagnostic(
    kind: DiagnosticKind,
    line: Option<usize>,
    key: Option<String>,
    message: impl Into<String>,
) -> SettingsDiagnostic {
    SettingsDiagnostic {
        kind,
        line,
        key,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("mdlite-settings-{}-{id}", std::process::id()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn setting_patch(key: &str, value: SettingValue) -> SettingsPatch {
        let mut patch = SettingsPatch::default();
        patch.set(key, value);
        patch
    }

    #[test]
    fn layers_are_per_key_and_defaults_match_the_product_contract() {
        let common = parse_settings(
            "schema_version = 1\nauto_save = false\nauto_save_delay_ms = 1200\ntheme = \"light\"\n",
        );
        let workspace = parse_settings("schema_version = 1\nauto_save_delay_ms = 2400\n");
        let resolved = resolve_settings(Some(&common), Some(&workspace));
        assert!(resolved.is_valid(), "{:?}", resolved.diagnostics);
        assert!(!resolved.settings.auto_save);
        assert_eq!(resolved.settings.auto_save_delay_ms, 2400);
        assert_eq!(resolved.settings.theme, ThemeMode::Light);
        assert_eq!(resolved.settings.font_face, "Segoe UI");
        assert_eq!(resolved.settings.font_size_pt, 11);
        assert_eq!(resolved.settings.keybindings["file.new"], "Ctrl+N");

        let defaults = EffectiveSettings::default();
        assert!(defaults.auto_save);
        assert_eq!(defaults.auto_save_delay_ms, 750);
    }

    #[test]
    fn bom_utf8_crlf_and_unknown_sections_survive_a_selected_patch() {
        let directory = TempDir::new();
        let path = directory.0.join("settings.toml");
        let original = concat!(
            "\u{feff}# keep this comment\r\n",
            "schema_version = 1\r\n",
            "auto_save = false  # keep inline note\r\n",
            "future_root = \"keep\"\r\n",
            "\r\n[calendar]\r\n",
            "week_start = \"sunday\"\r\n",
        );
        fs::write(&path, original.as_bytes()).unwrap();
        let mut captured = CapturedSettings::capture(&path).unwrap();
        assert!(captured.parsed().schema_supported());
        let result = captured
            .apply(&setting_patch("auto_save", SettingValue::Bool(true)))
            .unwrap();
        assert!(result.wrote_file);
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\xef\xbb\xbf"));
        let saved = String::from_utf8(bytes[3..].to_vec()).unwrap();
        assert!(saved.contains(
            "# keep this comment\r\nschema_version = 1\r\nauto_save = true  # keep inline note\r\n"
        ));
        assert!(
            saved.contains(
                "future_root = \"keep\"\r\n\r\n[calendar]\r\nweek_start = \"sunday\"\r\n"
            )
        );
        assert!(!saved.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn added_root_override_is_inserted_before_first_section_and_inherit_removes_it() {
        let directory = TempDir::new();
        let path = directory.0.join("settings.toml");
        fs::write(
            &path,
            "schema_version = 1\n\n[calendar]\nweek_start = \"sunday\"\n",
        )
        .unwrap();
        let mut captured = CapturedSettings::capture(&path).unwrap();
        let added = captured
            .apply(&setting_patch(
                "theme",
                SettingValue::Theme(ThemeMode::Custom),
            ))
            .unwrap();
        assert!(added.wrote_file);
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.find("theme = \"custom\"").unwrap() < saved.find("[calendar]").unwrap());

        let mut clear = SettingsPatch::default();
        clear.inherit("theme");
        captured.apply(&clear).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("theme ="));
        assert!(saved.contains("[calendar]\nweek_start = \"sunday\"\n"));
    }

    #[test]
    fn malformed_values_fall_back_per_key_and_schema_or_duplicates_block_apply() {
        let parsed = parse_settings(
            "schema_version = 1\nauto_save = maybe\ntheme = \"dark\"\ncolor.unknown = \"#FFFFFF\"\n",
        );
        assert_eq!(parsed.layer.auto_save, None);
        assert_eq!(parsed.layer.theme, Some(ThemeMode::Dark));
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|item| item.key.as_deref() == Some("auto_save"))
        );
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|item| item.kind == DiagnosticKind::UnknownManagedKey)
        );

        let directory = TempDir::new();
        let path = directory.0.join("bad.toml");
        fs::write(&path, "schema_version = 2\nauto_save = false\n").unwrap();
        let mut captured = CapturedSettings::capture(&path).unwrap();
        assert!(!captured.parsed().schema_supported());
        assert!(
            captured
                .apply(&setting_patch("auto_save", SettingValue::Bool(true)))
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "schema_version = 2\nauto_save = false\n"
        );

        fs::write(
            &path,
            "schema_version = 1\nauto_save = true\nauto_save = false\n",
        )
        .unwrap();
        let mut duplicated = CapturedSettings::capture(&path).unwrap();
        assert!(duplicated.parsed().has_duplicate_managed_keys());
        assert!(
            duplicated
                .apply(&setting_patch(
                    "theme",
                    SettingValue::Theme(ThemeMode::Dark)
                ))
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "schema_version = 1\nauto_save = true\nauto_save = false\n"
        );
    }

    #[test]
    fn binding_validation_covers_editor_and_panel_reservations() {
        let common = parse_settings("schema_version = 1\nbind.file.save = \"Ctrl+O\"\n");
        let resolution = resolve_settings(Some(&common), None);
        assert!(!resolution.is_valid());
        assert_eq!(resolution.settings.keybindings["file.save"], "Ctrl+S");

        for binding in ["Ctrl+Alt+T", "Ctrl+Alt+Up", "Ctrl+Alt+Down"] {
            let parsed = parse_settings(&format!(
                "schema_version = 1\nbind.file.save = \"{binding}\"\n"
            ));
            assert!(
                !resolve_settings(Some(&parsed), None).is_valid(),
                "{binding}"
            );
        }
        let panel_collision =
            parse_settings("schema_version = 1\nbind.file.new = \"Ctrl+Alt+1\"\n");
        let panel_resolution = resolve_settings(Some(&panel_collision), None);
        assert!(
            panel_resolution
                .diagnostics
                .iter()
                .any(|item| item.kind == DiagnosticKind::BindingConflict)
        );

        let editor_collision =
            parse_settings("schema_version = 1\nbind.file.quickOpen = \"Ctrl+Z\"\n");
        let resolution = resolve_settings(Some(&editor_collision), None);
        assert!(!resolution.is_valid());
        assert!(
            resolution
                .diagnostics
                .iter()
                .any(|item| item.kind == DiagnosticKind::BindingConflict)
        );

        let text_collision = parse_settings("schema_version = 1\nbind.file.new = \"A\"\n");
        assert!(
            text_collision
                .diagnostics
                .iter()
                .any(|item| item.kind == DiagnosticKind::InvalidShortcut)
        );
    }

    #[test]
    fn absent_target_is_not_created_on_cancel_and_create_new_detects_peer_creation() {
        let directory = TempDir::new();
        let path = directory.0.join("settings.toml");
        {
            let captured = CapturedSettings::capture(&path).unwrap();
            assert_eq!(
                captured.target(),
                fs::canonicalize(path.parent().unwrap())
                    .unwrap()
                    .join(path.file_name().unwrap())
            );
            // Dropping without Apply is the Cancel path.
        }
        assert!(!path.exists());

        let mut reset = CapturedSettings::capture(&path).unwrap();
        let mut inherit = SettingsPatch::default();
        inherit.inherit("theme");
        assert!(!reset.apply(&inherit).unwrap().wrote_file);
        assert!(!path.exists());

        let mut captured = CapturedSettings::capture(&path).unwrap();
        fs::write(&path, "peer data\n").unwrap();
        assert!(
            captured
                .apply(&setting_patch("auto_save", SettingValue::Bool(false)))
                .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "peer data\n");
    }

    #[test]
    fn absent_target_in_missing_parent_is_deferred_until_apply() {
        let directory = TempDir::new();
        let path = directory
            .0
            .join("local")
            .join("MDLite")
            .join("settings.toml");
        let expected = fs::canonicalize(&directory.0)
            .unwrap()
            .join("local/MDLite/settings.toml");
        {
            let captured = CapturedSettings::capture(&path).unwrap();
            assert_eq!(captured.target(), expected);
            assert!(!path.parent().unwrap().exists());
            // Cancel leaves the absent target and its parent absent.
        }
        assert!(!path.parent().unwrap().exists());

        let mut captured = CapturedSettings::capture(&path).unwrap();
        captured
            .apply(&setting_patch("auto_save", SettingValue::Bool(false)))
            .unwrap();
        assert!(path.parent().unwrap().is_dir());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "schema_version = 1\nauto_save = false\n"
        );
    }

    #[test]
    fn captured_existing_file_refuses_external_replacement_or_deletion() {
        let directory = TempDir::new();
        let path = directory.0.join("settings.toml");
        fs::write(&path, "schema_version = 1\n").unwrap();
        let mut changed = CapturedSettings::capture(&path).unwrap();
        fs::write(&path, "schema_version = 1\nauto_save = false\n").unwrap();
        assert!(
            changed
                .apply(&setting_patch(
                    "theme",
                    SettingValue::Theme(ThemeMode::Light)
                ))
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "schema_version = 1\nauto_save = false\n"
        );

        let mut deleted = CapturedSettings::capture(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert!(
            deleted
                .apply(&setting_patch(
                    "theme",
                    SettingValue::Theme(ThemeMode::Dark)
                ))
                .is_err()
        );
        assert!(!path.exists());
    }

    #[test]
    fn patch_rejects_shortcut_conflicts_without_writing() {
        let directory = TempDir::new();
        let path = directory.0.join("settings.toml");
        fs::write(&path, "schema_version = 1\n").unwrap();
        let mut captured = CapturedSettings::capture(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let result = captured.apply(&setting_patch(
            "bind.file.quickOpen",
            SettingValue::Text("Ctrl+Z".to_owned()),
        ));
        assert!(matches!(result, Err(SettingsError::BindingConflicts(_))));
        assert_eq!(fs::read(&path).unwrap(), before);

        let common = parse_settings("schema_version = 1\nbind.file.new = \"Ctrl+Q\"\n");
        let mut workspace = CapturedSettings::capture(&path).unwrap();
        let before = fs::read(&path).unwrap();
        let result = workspace.apply_with_layers(
            &setting_patch("bind.file.open", SettingValue::Text("Ctrl+Q".to_owned())),
            &[&common.layer],
            &[],
        );
        assert!(matches!(result, Err(SettingsError::BindingConflicts(_))));
        assert_eq!(fs::read(&path).unwrap(), before);
    }
}
