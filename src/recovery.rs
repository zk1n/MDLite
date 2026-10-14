//! Workspace-local recovery. Reading never deletes candidates or changes a document.
use crate::{
    document::Document,
    storage::{FileFormat, FileSnapshot},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};
const LIMIT: u64 = 64 * 1024 * 1024;
static NEXT_STORE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
#[derive(Clone, Debug)]
pub struct Recovery {
    pub record: PathBuf,
    pub workspace: PathBuf,
    pub source_path: Option<PathBuf>,
    pub document_id: u64,
    pub revision: u64,
    pub source: String,
    pub legacy: bool,
    pub format: Option<FileFormat>,
    pub baseline_sha256: Option<String>,
    pub record_sha256: String,
}
pub struct Candidates {
    pub records: Vec<Recovery>,
    pub issues: Vec<String>,
}
pub struct RecoveryStore {
    root: PathBuf,
    directory: PathBuf,
    lifetime: String,
    owned: RefCell<HashMap<u64, (PathBuf, String)>>,
}
impl RecoveryStore {
    /// State initialization is an explicit separate user action. Do not create it here.
    pub fn open(workspace: &Path) -> io::Result<Self> {
        let root = fs::canonicalize(workspace)?;
        if !root.is_dir() {
            return Err(invalid("workspace is not a directory"));
        }
        let state = root.join(".mdlite/.state");
        check_bounded(&root, &state)?;
        if !state.is_dir() {
            return Err(invalid("workspace state is unavailable"));
        }
        Ok(Self {
            directory: state.join("recovery"),
            root,
            lifetime: format!(
                "{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(io::Error::other)?
                    .as_nanos(),
                NEXT_STORE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            owned: RefCell::new(HashMap::new()),
        })
    }
    pub fn write(&self, doc: &Document, source_path: Option<&Path>) -> io::Result<PathBuf> {
        self.write_with_file(doc, source_path, None)
    }
    pub fn write_with_file(
        &self,
        doc: &Document,
        source_path: Option<&Path>,
        file: Option<&FileSnapshot>,
    ) -> io::Result<PathBuf> {
        self.write_with_metadata(doc, source_path, file, None)
    }
    pub fn write_with_metadata(
        &self,
        doc: &Document,
        source_path: Option<&Path>,
        file: Option<&FileSnapshot>,
        fallback_format: Option<FileFormat>,
    ) -> io::Result<PathBuf> {
        check_bounded(&self.root, &self.directory)?;
        if !self.directory.exists() {
            fs::create_dir(&self.directory)?;
        }
        check_bounded(&self.root, &self.directory)?;
        let owned = self.owned.borrow().get(&doc.id()).cloned();
        let path = owned
            .as_ref()
            .map(|(path, _)| path.clone())
            .unwrap_or_else(|| {
                self.directory
                    .join(format!("rust-{}-{}.json", self.lifetime, doc.id()))
            });
        check_bounded(&self.root, &path)?;
        let format =
            file.map(FileSnapshot::format)
                .or(fallback_format)
                .map(|format| match format {
                    FileFormat::Utf8 { bom: false } => "utf8",
                    FileFormat::Utf8 { bom: true } => "utf8-bom",
                    FileFormat::Cp932 => "cp932",
                });
        let value = json!({"schema_version":1,"lifetime":self.lifetime,"workspace":self.root,"source_path":source_path,"document_id":doc.id(),"revision":doc.revision(),"source":doc.source(),"format":format,"baseline_sha256":file.map(FileSnapshot::baseline_sha256)});
        let text = serde_json::to_string(&value).map_err(io::Error::other)?;
        if text.len() as u64 > LIMIT {
            return Err(invalid(
                "recovery exceeds safety limit; source remains unsaved",
            ));
        }
        let fingerprint = if let Some((_, expected)) = owned {
            let mut loaded = FileSnapshot::open(&path).map_err(io::Error::other)?;
            if loaded.snapshot.baseline_sha256() != expected {
                return Err(invalid("recovery changed outside this document"));
            }
            let previous: Value = serde_json::from_str(&loaded.source).map_err(io::Error::other)?;
            if previous["lifetime"].as_str() != Some(self.lifetime.as_str())
                || previous["document_id"].as_u64() != Some(doc.id())
            {
                return Err(invalid("recovery record ownership changed"));
            }
            loaded
                .snapshot
                .save_with_format(&text, FileFormat::Utf8 { bom: false })
                .map_err(io::Error::other)?;
            loaded.snapshot.baseline_sha256()
        } else {
            FileSnapshot::create_new(&path, &text, FileFormat::Utf8 { bom: false })
                .map_err(io::Error::other)?
                .baseline_sha256()
        };
        self.owned
            .borrow_mut()
            .insert(doc.id(), (path.clone(), fingerprint));
        Ok(path)
    }
    pub fn list(&self) -> io::Result<Candidates> {
        check_bounded(&self.root, &self.directory)?;
        let mut result = Candidates {
            records: Vec::new(),
            issues: Vec::new(),
        };
        if !self.directory.try_exists()? {
            return Ok(result);
        }
        for item in fs::read_dir(&self.directory)? {
            let item = match item {
                Ok(item) => item,
                Err(e) => {
                    result.issues.push(e.to_string());
                    continue;
                }
            };
            let path = item.path();
            if !matches!(
                path.extension().and_then(|s| s.to_str()),
                Some("json" | "md")
            ) {
                continue;
            }
            match self.read(&path) {
                Ok(record) => result.records.push(record),
                Err(e) => result.issues.push(format!(
                    "{}: {e}; original retained for manual recovery",
                    item.file_name().to_string_lossy()
                )),
            }
        }
        result.records.sort_by(|a, b| a.record.cmp(&b.record));
        Ok(result)
    }
    pub fn read(&self, path: &Path) -> io::Result<Recovery> {
        check_bounded(&self.root, path)?;
        let canonical = fs::canonicalize(path)?;
        if canonical.parent() != Some(self.directory.as_path()) {
            return Err(invalid("record is outside recovery directory"));
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > LIMIT {
            return Err(invalid("recovery record too large"));
        }
        use sha2::{Digest, Sha256};
        let record_sha256 = format!("{:x}", Sha256::digest(&bytes));
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| invalid("invalid recovery UTF-8"))?
            .trim_start_matches('\u{feff}');
        if path.extension().is_some_and(|s| s == "md") {
            let prefix = "<!-- MDLite recovery\nsource: ";
            let normalized_header = text.replace("\r\n", "\n");
            if !normalized_header.starts_with(prefix) {
                return Err(invalid("unknown legacy recovery header"));
            }
            // Locate header in original bytes, preserving the exact body EOLs.
            let marker = text
                .find("-->")
                .ok_or_else(|| invalid("incomplete legacy recovery header"))?;
            let header = &text[..marker];
            let source_line = header
                .lines()
                .find_map(|s| s.strip_prefix("source: "))
                .ok_or_else(|| invalid("legacy source missing"))?;
            let source_path = PathBuf::from(source_line.trim_end_matches('\r'));
            if !source_path.starts_with(&self.root) {
                return Err(invalid("legacy source association is outside workspace"));
            }
            let suffix = &text[marker + 3..];
            let body = suffix
                .strip_prefix("\r\n")
                .or_else(|| suffix.strip_prefix('\n'))
                .ok_or_else(|| invalid("legacy header missing line ending"))?;
            return Ok(Recovery {
                record: canonical,
                workspace: self.root.clone(),
                source_path: Some(source_path),
                document_id: 0,
                revision: 0,
                source: body.to_owned(),
                legacy: true,
                format: None,
                baseline_sha256: None,
                record_sha256,
            });
        }
        let value: Value = serde_json::from_str(text).map_err(io::Error::other)?;
        if value["schema_version"].as_u64() != Some(1) {
            return Err(invalid("unsupported recovery schema"));
        }
        let workspace = PathBuf::from(
            value["workspace"]
                .as_str()
                .ok_or_else(|| invalid("missing recovery workspace"))?,
        );
        if workspace != self.root {
            return Err(invalid("recovery workspace mismatch"));
        }
        let source_path = match &value["source_path"] {
            Value::Null => None,
            Value::String(s) => Some(PathBuf::from(s)),
            _ => return Err(invalid("invalid source association")),
        };
        let format = match value["format"].as_str() {
            None => None,
            Some("utf8") => Some(FileFormat::Utf8 { bom: false }),
            Some("utf8-bom") => Some(FileFormat::Utf8 { bom: true }),
            Some("cp932") => Some(FileFormat::Cp932),
            _ => return Err(invalid("unsupported recovered format")),
        };
        let baseline_sha256 = match &value["baseline_sha256"] {
            Value::Null => None,
            Value::String(s) if s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()) => {
                Some(s.clone())
            }
            _ => return Err(invalid("invalid saved-file baseline")),
        };
        Ok(Recovery {
            record: canonical,
            workspace,
            source_path,
            document_id: value["document_id"]
                .as_u64()
                .ok_or_else(|| invalid("missing document id"))?,
            revision: value["revision"]
                .as_u64()
                .ok_or_else(|| invalid("missing revision"))?,
            source: value["source"]
                .as_str()
                .ok_or_else(|| invalid("missing recovery body"))?
                .to_owned(),
            legacy: false,
            format,
            baseline_sha256,
            record_sha256,
        })
    }
    /// Explicit discard only. A changed candidate must be presented again.
    pub fn discard(&self, record: &Recovery) -> io::Result<()> {
        let current = self.read(&record.record)?;
        if current.record_sha256 != record.record_sha256
            || current.source != record.source
            || current.revision != record.revision
            || current.source_path != record.source_path
        {
            return Err(invalid("recovery changed since it was presented"));
        }
        fs::remove_file(&record.record)
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn check_bounded(root: &Path, path: &Path) -> io::Result<()> {
    if !path.starts_with(root) {
        return Err(invalid("state path escapes workspace"));
    }
    let relative = path
        .strip_prefix(root)
        .map_err(|_| invalid("invalid state path"))?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return Err(invalid("invalid state component"));
        }
        current.push(part);
        match fs::symlink_metadata(&current) {
            Ok(meta) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err(invalid("reparse point in state path"));
                    }
                }
                if meta.file_type().is_symlink() {
                    return Err(invalid("symlink in state path"));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifetimes_keep_retained_records_and_preserve_opened_disk_baseline() {
        let f = Fixture::new();
        let source_path = f.0.join("note.md");
        fs::write(&source_path, b"\xef\xbb\xbforiginal\r\n").unwrap();
        let loaded = FileSnapshot::open(&source_path).unwrap();
        let mut doc = Document::new(3, loaded.source);
        doc.replace_range(0..0, "edited ").unwrap();
        let first = RecoveryStore::open(&f.0).unwrap();
        let a = first
            .write_with_file(&doc, Some(&source_path), Some(&loaded.snapshot))
            .unwrap();
        let second = RecoveryStore::open(&f.0).unwrap();
        let b = second
            .write_with_file(&doc, Some(&source_path), Some(&loaded.snapshot))
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(first.list().unwrap().records.len(), 2);
        let record = first.read(&a).unwrap();
        assert_eq!(record.format, Some(FileFormat::Utf8 { bom: true }));
        assert_eq!(
            record.baseline_sha256,
            Some(loaded.snapshot.baseline_sha256())
        );
        fs::write(&source_path, "external").unwrap();
        assert_ne!(
            record.baseline_sha256,
            Some(
                FileSnapshot::open(&source_path)
                    .unwrap()
                    .snapshot
                    .baseline_sha256()
            )
        );
        let before = fs::read(&a).unwrap();
        fs::write(&a, "peer changed candidate").unwrap();
        assert!(first.write(&doc, Some(&source_path)).is_err());
        assert_eq!(fs::read_to_string(&a).unwrap(), "peer changed candidate");
        fs::write(&a, before).unwrap();
    }
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "mdlite-recovery-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(p.join(".mdlite/.state")).unwrap();
            Self(fs::canonicalize(p).unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn recovery_roundtrip_untitled_never_marks_saved_and_requires_explicit_unchanged_discard() {
        let f = Fixture::new();
        let store = RecoveryStore::open(&f.0).unwrap();
        let mut d = Document::new_unsaved(2, "日本語\r\n😀".into());
        let p = store.write(&d, None).unwrap();
        assert!(d.is_dirty());
        assert_eq!(d.undo_count(), 0);
        let r = store.read(&p).unwrap();
        assert_eq!(r.source, d.source());
        assert!(r.source_path.is_none());
        d.replace_range(0..0, "new").unwrap();
        store.write(&d, None).unwrap();
        assert!(store.discard(&r).is_err());
        let current = store.read(&p).unwrap();
        store.discard(&current).unwrap();
        assert!(!p.exists());
    }
    #[test]
    fn legacy_body_and_unknown_schema_are_preserved() {
        let f = Fixture::new();
        let store = RecoveryStore::open(&f.0).unwrap();
        fs::create_dir(&store.directory).unwrap();
        let path = store.directory.join("0123456789abcdef.md");
        let text = format!(
            "<!-- MDLite recovery\r\nsource: {}\r\n-->\r\n日\r\n\r\n",
            f.0.join(".mdlite/.state/untitled/missing.md").display()
        );
        fs::write(&path, text).unwrap();
        assert_eq!(store.read(&path).unwrap().source, "日\r\n\r\n");
        let bad = store.directory.join("future.json");
        fs::write(&bad, "{\"schema_version\":999}").unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.records.len(), 1);
        assert_eq!(list.issues.len(), 1);
        assert_eq!(fs::read_to_string(bad).unwrap(), "{\"schema_version\":999}");
    }
}
