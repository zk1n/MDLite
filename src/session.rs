//! Versioned session coordinates. Sessions contain views and paths, never bodies.
use crate::{
    coordinates::BoundaryAffinity,
    document::{Document, Selection},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Component, PathBuf},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Default for WindowPlacement {
    fn default() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 1280,
            height: 800,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SessionDocument {
    /// Empty only for an untitled recovery view, never a disk document.
    pub path: PathBuf,
    pub source_sha256: Option<String>,
    pub recovery_record: Option<PathBuf>,
    pub anchor: usize,
    pub head: usize,
    pub affinity: BoundaryAffinity,
    pub first_visible_line: usize,
    pub view_anchor: Option<usize>,
    pub horizontal_anchor: Option<usize>,
    pub compact: bool,
    pub placement: WindowPlacement,
}
#[derive(Clone, Debug)]
pub struct Session {
    pub version: u8,
    pub active: usize,
    pub recent: Vec<PathBuf>,
    pub documents: Vec<SessionDocument>,
    /// Views waiting for an explicitly restored, matching recovery source.
    pub recovery_views: Vec<SessionDocument>,
    pub main: WindowPlacement,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            version: 4,
            active: 0,
            recent: Vec::new(),
            documents: Vec::new(),
            recovery_views: Vec::new(),
            main: WindowPlacement::default(),
        }
    }
}
#[derive(Debug, PartialEq)]
pub struct View {
    pub selection: Selection,
    pub affinity: BoundaryAffinity,
    pub first_visible_line: usize,
    pub anchor: Option<usize>,
    pub horizontal_anchor: Option<usize>,
}
impl Session {
    pub fn hydrate(&self, record: &SessionDocument, doc: &Document) -> Result<View, &'static str> {
        if record
            .source_sha256
            .as_ref()
            .is_some_and(|expected| *expected != source_sha256(doc.source()))
        {
            return Err("session view belongs to different source; recovery view retained");
        }
        let convert = |offset| {
            if self.version < 3 {
                doc.utf16_to_byte(offset)
                    .map_err(|_| "invalid legacy UTF-16 session coordinate")
            } else {
                doc.byte_to_utf16(offset)
                    .map(|_| offset)
                    .map_err(|_| "invalid UTF-8 session coordinate")
            }
        };
        Ok(View {
            selection: Selection {
                anchor: convert(record.anchor)?,
                head: convert(record.head)?,
            },
            affinity: record.affinity,
            first_visible_line: record.first_visible_line,
            anchor: record.view_anchor.map(convert).transpose()?,
            horizontal_anchor: record.horizontal_anchor.map(convert).transpose()?,
        })
    }
    pub fn parse(text: &str) -> Result<Self, &'static str> {
        if text.len() > 4 * 1024 * 1024 {
            return Err("session too large");
        }
        let mut root = BTreeMap::new();
        let mut records: Vec<(bool, BTreeMap<&str, &str>)> = Vec::new();
        let mut recent = Vec::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line == "[[document]]" || line == "[[recovery_view]]" {
                records.push((line == "[[recovery_view]]", BTreeMap::new()));
                continue;
            }
            let (key, value) = line.split_once('=').ok_or("invalid session assignment")?;
            let (key, value) = (key.trim(), value.trim());
            if key == "recent" {
                recent.push(parse_path(value)?);
                continue;
            }
            if key == "open" {
                records.push((false, BTreeMap::from([("path", value)])));
                continue;
            }
            let fields = if let Some((_, record)) = records.last_mut() {
                record
            } else {
                &mut root
            };
            if fields.insert(key, value).is_some() {
                return Err("duplicate session key");
            }
        }
        let version: u8 = number(&root, "schema_version", 1)?;
        if !(1..=4).contains(&version) {
            return Err("unsupported session schema; original retained");
        }
        let active = number(&root, "active_index", 0)?;
        let main = placement(&root, "main_")?;
        let mut documents = Vec::new();
        let mut recovery_views = Vec::new();
        for (recovery_view, fields) in records {
            if recovery_view && version < 4 {
                return Err("recovery views require session schema 4");
            }
            let path = fields
                .get("path")
                .map(|p| parse_path(p))
                .transpose()?
                .unwrap_or_default();
            let source_sha256 = fields
                .get("source_sha256")
                .map(|value| parse_hash(value))
                .transpose()?;
            let recovery_record = fields
                .get("recovery_record")
                .map(|value| parse_recovery_path(value))
                .transpose()?;
            if version == 4 && source_sha256.is_none() {
                return Err("session source identity missing");
            }
            if path.as_os_str().is_empty() && (!recovery_view || recovery_record.is_none()) {
                return Err("session path missing");
            }
            if !recovery_view && documents.iter().any(|d: &SessionDocument| d.path == path) {
                return Err("duplicate session document");
            }
            let optional = |key| {
                fields
                    .get(key)
                    .map(|s| s.parse::<usize>().map_err(|_| "invalid view offset"))
                    .transpose()
            };
            let affinity = match fields.get("affinity").copied().unwrap_or("\"after\"") {
                "\"before\"" => BoundaryAffinity::Before,
                "\"after\"" => BoundaryAffinity::After,
                _ => return Err("invalid session affinity"),
            };
            let record = SessionDocument {
                path,
                source_sha256,
                recovery_record,
                anchor: number(&fields, "selection_begin", 0)?,
                head: number(&fields, "selection_end", 0)?,
                affinity,
                first_visible_line: number(&fields, "first_visible_line", 0)?,
                view_anchor: optional("first_visible_source_offset")?,
                horizontal_anchor: optional("horizontal_left_edge_source_offset")?,
                compact: fields
                    .get("compact")
                    .map_or(Ok(false), |v| v.parse().map_err(|_| "invalid compact flag"))?,
                placement: placement(&fields, "")?,
            };
            if recovery_view {
                recovery_views.push(record);
            } else {
                documents.push(record);
            }
        }
        if !documents.is_empty() && active >= documents.len() {
            return Err("active session index out of range");
        }
        Ok(Self {
            version,
            active,
            recent,
            documents,
            recovery_views,
            main,
        })
    }
    /// Writer accepts source-bound UTF-8 views only. Legacy records are hydrated
    /// against their source before the caller creates a new version-4 session.
    pub fn serialize(&self) -> Result<String, &'static str> {
        if self.version != 4 {
            return Err("hydrate legacy session before writing");
        }
        let mut text = format!("schema_version = 4\nactive_index = {}\n", self.active);
        write_placement(&mut text, &self.main, "main_");
        for path in &self.recent {
            text.push_str(&format!("recent = {}\n", quote_path(path)?));
        }
        for (recovery_view, d) in self
            .documents
            .iter()
            .map(|d| (false, d))
            .chain(self.recovery_views.iter().map(|d| (true, d)))
        {
            let fingerprint = d
                .source_sha256
                .as_ref()
                .ok_or("session source identity missing")?;
            parse_hash(&format!("\"{fingerprint}\""))?;
            text.push_str(if recovery_view {
                "\n[[recovery_view]]\n"
            } else {
                "\n[[document]]\n"
            });
            if !d.path.as_os_str().is_empty() {
                text.push_str(&format!("path = {}\n", quote_path(&d.path)?));
            } else if !recovery_view || d.recovery_record.is_none() {
                return Err("session path missing");
            }
            text.push_str(&format!("source_sha256 = \"{fingerprint}\"\n"));
            if let Some(path) = &d.recovery_record {
                let quoted = quote_path(path)?;
                parse_recovery_path(&quoted)?;
                text.push_str(&format!("recovery_record = {quoted}\n"));
            }
            text.push_str(&format!("selection_begin = {}\nselection_end = {}\naffinity = \"{}\"\nfirst_visible_line = {}\ncompact = {}\n", d.anchor,d.head, if d.affinity == BoundaryAffinity::Before { "before" } else { "after" },d.first_visible_line,d.compact));
            write_placement(&mut text, &d.placement, "");
            if let Some(n) = d.view_anchor {
                text.push_str(&format!("first_visible_source_offset = {n}\n"));
            }
            if let Some(n) = d.horizontal_anchor {
                text.push_str(&format!("horizontal_left_edge_source_offset = {n}\n"));
            }
        }
        Ok(text)
    }
}
pub fn source_sha256(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}
fn parse_hash(value: &str) -> Result<String, &'static str> {
    let value = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("invalid quoted source identity")?;
    if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("invalid source identity");
    }
    Ok(value.to_ascii_lowercase())
}
fn parse_recovery_path(value: &str) -> Result<PathBuf, &'static str> {
    let path = parse_path(value)?;
    if path.parent() != Some(std::path::Path::new(".mdlite/.state/recovery")) {
        return Err("session recovery reference is outside recovery directory");
    }
    Ok(path)
}
fn number<T: std::str::FromStr>(
    fields: &BTreeMap<&str, &str>,
    key: &str,
    default: T,
) -> Result<T, &'static str> {
    fields.get(key).map_or(Ok(default), |v| {
        v.parse().map_err(|_| "invalid session number")
    })
}
fn parse_path(value: &str) -> Result<PathBuf, &'static str> {
    let raw = value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .ok_or("invalid quoted session path")?;
    if raw.contains(['"', '\r', '\n', '\0']) {
        return Err("unsafe session path");
    }
    let path = PathBuf::from(raw);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|p| !matches!(p, Component::Normal(_)))
    {
        return Err("session path must be workspace relative");
    }
    Ok(path)
}
fn quote_path(path: &std::path::Path) -> Result<String, &'static str> {
    let value = path
        .to_str()
        .ok_or("session path is not Unicode")?
        .replace('\\', "/");
    parse_path(&format!("\"{value}\""))?;
    Ok(format!("\"{value}\""))
}
fn placement(fields: &BTreeMap<&str, &str>, prefix: &str) -> Result<WindowPlacement, &'static str> {
    let p = WindowPlacement {
        x: number(fields, &format!("{prefix}x"), 100)?,
        y: number(fields, &format!("{prefix}y"), 100)?,
        width: number(fields, &format!("{prefix}width"), 1280)?,
        height: number(fields, &format!("{prefix}height"), 800)?,
    };
    if !(160..=32768).contains(&p.width) || !(100..=32768).contains(&p.height) {
        return Err("invalid window dimensions");
    }
    Ok(p)
}
fn write_placement(text: &mut String, p: &WindowPlacement, prefix: &str) {
    text.push_str(&format!(
        "{prefix}x = {}\n{prefix}y = {}\n{prefix}width = {}\n{prefix}height = {}\n",
        p.x, p.y, p.width, p.height
    ));
}

/// Separate trailing autosave and non-starving periodic recovery clocks.
#[derive(Default)]
pub struct SaveSchedule {
    autosave: Option<Instant>,
    recovery: Option<Instant>,
    revision: Option<u64>,
}
impl SaveSchedule {
    pub fn changed(&mut self, revision: u64, dirty: bool, now: Instant, delay: Duration) {
        if !dirty {
            self.autosave = None;
            self.recovery = None;
            self.revision = Some(revision);
            return;
        }
        if self.revision != Some(revision) {
            self.autosave = Some(now + delay);
            self.revision = Some(revision);
        }
        self.recovery.get_or_insert(now + Duration::from_secs(5));
    }
    pub fn due(
        &self,
        now: Instant,
        blocked: bool,
        existing_file: bool,
        autosave: bool,
    ) -> (bool, bool) {
        if blocked {
            return (false, false);
        }
        (
            existing_file && autosave && self.autosave.is_some_and(|t| now >= t),
            self.recovery.is_some_and(|t| now >= t),
        )
    }
    pub fn autosave_attempted(&mut self, now: Instant, delay: Duration) {
        self.autosave = Some(now + delay);
    }
    pub fn recovery_attempted(&mut self, now: Instant) {
        self.recovery = Some(now + Duration::from_secs(5));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reversed_legacy_utf16_and_view_offsets_convert_without_clamping() {
        let text = "schema_version = 2\nactive_index = 0\nrecent = \"note.md\"\nrecent = \"other.md\"\n[[document]]\npath = \"note.md\"\nselection_begin = 4\nselection_end = 1\nfirst_visible_source_offset = 3\nhorizontal_left_edge_source_offset = 1\n";
        let session = Session::parse(text).unwrap();
        let doc = Document::new(1, "A😀日\r\nZ".into());
        let v = session.hydrate(&session.documents[0], &doc).unwrap();
        assert_eq!(v.selection, Selection { anchor: 8, head: 1 });
        assert_eq!(v.anchor, Some(5));
        assert_eq!(v.horizontal_anchor, Some(1));
        let bad =
            Session::parse(&text.replace("selection_begin = 4", "selection_begin = 2")).unwrap();
        assert!(bad.hydrate(&bad.documents[0], &doc).is_err());
        assert!(session.serialize().is_err());
        for invalid in [
            "schema_version = 99",
            "schema_version = 3\n[[document]]\npath = \"../secret.md\"",
        ] {
            assert!(Session::parse(invalid).is_err());
        }
    }
    #[test]
    fn current_session_roundtrip_contains_no_body() {
        let s = Session {
            documents: vec![SessionDocument {
                path: "a.md".into(),
                source_sha256: Some(source_sha256("A😀日")),
                recovery_record: None,
                anchor: 4,
                head: 1,
                affinity: BoundaryAffinity::Before,
                first_visible_line: 2,
                view_anchor: Some(1),
                horizontal_anchor: None,
                compact: true,
                placement: WindowPlacement::default(),
            }],
            ..Default::default()
        };
        let text = s.serialize().unwrap();
        let parsed = Session::parse(&text).unwrap();
        assert_eq!(parsed.documents[0].anchor, 4);
        assert_eq!(parsed.documents[0].affinity, BoundaryAffinity::Before);
    }
    #[test]
    fn source_bound_views_reject_different_text_even_with_valid_offsets() {
        let source = "A😀日";
        let record = SessionDocument {
            path: PathBuf::from("a.md"),
            source_sha256: Some(source_sha256(source)),
            recovery_record: Some(PathBuf::from(".mdlite/.state/recovery/example.json")),
            anchor: 8,
            head: 1,
            affinity: BoundaryAffinity::Before,
            first_visible_line: 0,
            view_anchor: Some(5),
            horizontal_anchor: None,
            compact: false,
            placement: WindowPlacement::default(),
        };
        let session = Session {
            documents: vec![record.clone()],
            ..Session::default()
        };
        let serialized = session.serialize().unwrap();
        assert!(!serialized.contains(source));
        let parsed = Session::parse(&serialized).unwrap();
        assert_eq!(parsed.version, 4);
        assert_eq!(
            parsed
                .hydrate(&parsed.documents[0], &Document::new(9, source.into()))
                .unwrap()
                .selection,
            Selection { anchor: 8, head: 1 }
        );
        assert!(
            parsed
                .hydrate(&parsed.documents[0], &Document::new(9, "B😀月".into()))
                .is_err()
        );
        assert!(
            Session::parse(&serialized.replace("schema_version = 4", "schema_version = 5"))
                .is_err()
        );
        assert!(
            Session::parse(
                &serialized.replace(".mdlite/.state/recovery/example.json", "other/example.json")
            )
            .is_err()
        );
        let legacy=Session::parse("schema_version = 3\n[[document]]\npath = \"a.md\"\nselection_begin = 8\nselection_end = 1\n").unwrap();
        assert_eq!(
            legacy
                .hydrate(&legacy.documents[0], &Document::new(9, source.into()))
                .unwrap()
                .selection,
            Selection { anchor: 8, head: 1 }
        );
    }
    #[test]
    fn continuous_edits_do_not_starve_recovery_and_ime_defers_both() {
        let start = Instant::now();
        let delay = Duration::from_millis(750);
        let mut s = SaveSchedule::default();
        for i in 0..=60 {
            s.changed(i, true, start + Duration::from_millis(i * 100), delay);
        }
        let now = start + Duration::from_secs(6);
        assert_eq!(s.due(now, false, true, true), (false, true));
        assert_eq!(s.due(now, true, true, true), (false, false));
        assert_eq!(
            s.due(now + Duration::from_secs(1), false, false, true),
            (false, true)
        );
        s.recovery_attempted(now);
        assert_eq!(s.due(now, false, true, false), (false, false));
    }
}
