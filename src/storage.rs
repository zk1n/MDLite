//! Safe source-file decoding and atomic, encoding-preserving saves.

use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(not(windows))]
use encoding_rs::SHIFT_JIS;

#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;

#[cfg(windows)]
use windows_sys::Win32::{
    Globalization::{
        MB_ERR_INVALID_CHARS, MultiByteToWideChar, WC_NO_BEST_FIT_CHARS, WideCharToMultiByte,
    },
    Storage::FileSystem::{MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW},
};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";
#[cfg(windows)]
const WINDOWS_CP932: u32 = 932;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileFormat {
    Utf8 { bom: bool },
    Cp932,
}

impl Default for FileFormat {
    fn default() -> Self {
        Self::Utf8 { bom: false }
    }
}

#[derive(Debug)]
pub enum StorageError {
    Io(io::Error),
    InvalidUtf8,
    InvalidCp932,
    Cp932Unmappable,
    Cp932NotRoundTrippable,
    ExternalChange,
    DestinationExists,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(f),
            Self::InvalidUtf8 => f.write_str("file contains invalid UTF-8"),
            Self::InvalidCp932 => f.write_str("file contains invalid CP932 bytes"),
            Self::Cp932Unmappable => {
                f.write_str("source contains characters that cannot be represented in CP932")
            }
            Self::Cp932NotRoundTrippable => f.write_str(
                "this CP932 file contains byte aliases that cannot be safely re-encoded",
            ),
            Self::ExternalChange => {
                f.write_str("file bytes changed outside MDLite since they were opened")
            }
            Self::DestinationExists => f.write_str("save-as destination already exists"),
        }
    }
}

impl std::error::Error for StorageError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for StorageError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub struct LoadedFile {
    pub source: String,
    pub snapshot: FileSnapshot,
}

/// Disk identity and original bytes for one open document.
///
/// The byte snapshot is used to refuse stale saves and to preserve byte aliases
/// whenever the source has not changed.
pub struct FileSnapshot {
    path: PathBuf,
    original_bytes: Vec<u8>,
    format: FileFormat,
    cp932_reencodable: bool,
}

impl FileSnapshot {
    pub fn open(path: impl AsRef<Path>) -> Result<LoadedFile, StorageError> {
        let path = fs::canonicalize(path)?;
        let bytes = fs::read(&path)?;
        Self::from_bytes(path, bytes)
    }

    /// A background scanner must cap the actual read, including files that grow
    /// after metadata inspection. Normal document opening keeps its own contract.
    pub fn open_bounded(path: impl AsRef<Path>, limit: u64) -> Result<LoadedFile, StorageError> {
        let path = fs::canonicalize(path)?;
        let mut bytes = Vec::new();
        fs::File::open(&path)?
            .take(limit.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(StorageError::Io(io::Error::new(
                io::ErrorKind::FileTooLarge,
                "file exceeds scanner read limit",
            )));
        }
        Self::from_bytes(path, bytes)
    }

    fn from_bytes(path: PathBuf, bytes: Vec<u8>) -> Result<LoadedFile, StorageError> {
        let (source, format, cp932_reencodable) = decode_file(&bytes)?;
        Ok(LoadedFile {
            source,
            snapshot: Self {
                path,
                original_bytes: bytes,
                format,
                cp932_reencodable,
            },
        })
    }

    pub fn create_new(
        path: impl AsRef<Path>,
        source: &str,
        format: FileFormat,
    ) -> Result<Self, StorageError> {
        let requested = path.as_ref();
        let parent = path_parent(requested);
        let canonical_parent = fs::canonicalize(parent)?;
        let name = requested.file_name().ok_or_else(|| {
            StorageError::Io(io::Error::new(
                io::ErrorKind::InvalidInput,
                "missing file name",
            ))
        })?;
        let path = canonical_parent.join(name);
        if path.exists() {
            return Err(StorageError::DestinationExists);
        }
        let bytes = encode_file(source, format, true)?;
        let temp = write_temp_file(&path, &bytes)?;
        match move_temp(&temp, &path, false) {
            Ok(()) => Ok(Self {
                path,
                original_bytes: bytes,
                format,
                cp932_reencodable: format != FileFormat::Cp932 || cp932_round_trips(source),
            }),
            Err(error) => {
                let _ = fs::remove_file(&temp);
                if error.kind() == io::ErrorKind::AlreadyExists {
                    Err(StorageError::DestinationExists)
                } else {
                    Err(StorageError::Io(error))
                }
            }
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub const fn format(&self) -> FileFormat {
        self.format
    }

    /// Fingerprint of the originally opened bytes, never refreshed from disk.
    /// Recovery uses it to avoid adopting later external edits as a save baseline.
    pub fn baseline_sha256(&self) -> String {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(&self.original_bytes))
    }

    pub fn baseline_byte_len(&self) -> usize {
        self.original_bytes.len()
    }

    /// Save to the original path only if its current bytes still match the snapshot.
    pub fn save(&mut self, source: &str) -> Result<(), StorageError> {
        self.ensure_current()?;
        let (original_source, _, _) = decode_file(&self.original_bytes)?;
        let bytes = if source == original_source {
            self.original_bytes.clone()
        } else {
            encode_file(source, self.format, self.cp932_reencodable)?
        };

        self.replace_bytes(bytes, source, self.format)
    }

    /// An explicitly confirmed Save As retains the source document's format,
    /// rather than adopting the existing destination's encoding.
    pub fn save_with_format(
        &mut self,
        source: &str,
        format: FileFormat,
    ) -> Result<(), StorageError> {
        let bytes = encode_file(source, format, true)?;
        self.replace_bytes(bytes, source, format)
    }

    fn replace_bytes(
        &mut self,
        bytes: Vec<u8>,
        source: &str,
        format: FileFormat,
    ) -> Result<(), StorageError> {
        self.ensure_current()?;
        let temp = write_temp_file(&self.path, &bytes)?;
        if let Err(error) = self.ensure_current() {
            let _ = fs::remove_file(&temp);
            return Err(error);
        }
        if let Err(error) = move_temp(&temp, &self.path, true) {
            let _ = fs::remove_file(&temp);
            return Err(StorageError::Io(error));
        }

        self.original_bytes = bytes;
        self.format = format;
        self.cp932_reencodable = self.format != FileFormat::Cp932 || cp932_round_trips(source);
        Ok(())
    }

    fn ensure_current(&self) -> Result<(), StorageError> {
        match fs::read(&self.path) {
            Ok(bytes) if bytes == self.original_bytes => Ok(()),
            Ok(_) => Err(StorageError::ExternalChange),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(StorageError::ExternalChange)
            }
            Err(error) => Err(StorageError::Io(error)),
        }
    }
}

fn path_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn decode_file(bytes: &[u8]) -> Result<(String, FileFormat, bool), StorageError> {
    if let Some(body) = bytes.strip_prefix(UTF8_BOM) {
        return String::from_utf8(body.to_vec())
            .map(|source| (source, FileFormat::Utf8 { bom: true }, true))
            .map_err(|_| StorageError::InvalidUtf8);
    }
    if let Ok(source) = std::str::from_utf8(bytes) {
        return Ok((source.to_owned(), FileFormat::Utf8 { bom: false }, true));
    }

    decode_cp932(bytes).map(|(source, reencodable)| (source, FileFormat::Cp932, reencodable))
}

fn decode_cp932(bytes: &[u8]) -> Result<(String, bool), StorageError> {
    #[cfg(windows)]
    let source = decode_windows_cp932(bytes)?;
    #[cfg(not(windows))]
    let source = {
        let (decoded, had_errors) = SHIFT_JIS.decode_without_bom_handling(bytes);
        if had_errors {
            return Err(StorageError::InvalidCp932);
        }
        decoded.into_owned()
    };

    if source.contains('\u{FFFD}') {
        return Err(StorageError::InvalidCp932);
    }
    let reencodable = encode_cp932(&source).is_ok_and(|encoded| encoded == bytes);
    Ok((source, reencodable))
}

fn encode_file(
    source: &str,
    format: FileFormat,
    cp932_reencodable: bool,
) -> Result<Vec<u8>, StorageError> {
    match format {
        FileFormat::Utf8 { bom } => {
            let mut bytes = Vec::with_capacity(source.len() + usize::from(bom) * UTF8_BOM.len());
            if bom {
                bytes.extend_from_slice(UTF8_BOM);
            }
            bytes.extend_from_slice(source.as_bytes());
            Ok(bytes)
        }
        FileFormat::Cp932 => {
            if !cp932_reencodable {
                return Err(StorageError::Cp932NotRoundTrippable);
            }
            let bytes = encode_cp932(source)?;
            let (decoded, reencodable) = decode_cp932(&bytes)?;
            if !reencodable || decoded != source {
                return Err(StorageError::Cp932Unmappable);
            }
            Ok(bytes)
        }
    }
}

fn cp932_round_trips(source: &str) -> bool {
    encode_cp932(source).is_ok_and(|bytes| {
        decode_cp932(&bytes).is_ok_and(|(decoded, reencodable)| reencodable && decoded == source)
    })
}

#[cfg(windows)]
fn decode_windows_cp932(bytes: &[u8]) -> Result<String, StorageError> {
    if bytes.is_empty() {
        return Ok(String::new());
    }
    let byte_len = i32::try_from(bytes.len()).map_err(|_| StorageError::InvalidCp932)?;
    // Windows decodes CP932 using the same mapping the native editor uses.
    let needed = unsafe {
        MultiByteToWideChar(
            WINDOWS_CP932,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            byte_len,
            std::ptr::null_mut(),
            0,
        )
    };
    if needed <= 0 {
        return Err(StorageError::InvalidCp932);
    }
    let mut wide = vec![0u16; needed as usize];
    let written = unsafe {
        MultiByteToWideChar(
            WINDOWS_CP932,
            MB_ERR_INVALID_CHARS,
            bytes.as_ptr(),
            byte_len,
            wide.as_mut_ptr(),
            needed,
        )
    };
    if written != needed {
        return Err(StorageError::InvalidCp932);
    }
    String::from_utf16(&wide).map_err(|_| StorageError::InvalidCp932)
}

#[cfg(windows)]
fn encode_cp932(source: &str) -> Result<Vec<u8>, StorageError> {
    let wide: Vec<u16> = source.encode_utf16().collect();
    if wide.is_empty() {
        return Ok(Vec::new());
    }
    let wide_len = i32::try_from(wide.len()).map_err(|_| StorageError::Cp932Unmappable)?;
    let mut used_default = 0;
    let needed = unsafe {
        WideCharToMultiByte(
            WINDOWS_CP932,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            wide_len,
            std::ptr::null_mut(),
            0,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if needed <= 0 || used_default != 0 {
        return Err(StorageError::Cp932Unmappable);
    }
    let mut bytes = vec![0u8; needed as usize];
    let written = unsafe {
        WideCharToMultiByte(
            WINDOWS_CP932,
            WC_NO_BEST_FIT_CHARS,
            wide.as_ptr(),
            wide_len,
            bytes.as_mut_ptr(),
            needed,
            std::ptr::null(),
            &mut used_default,
        )
    };
    if written != needed || used_default != 0 {
        return Err(StorageError::Cp932Unmappable);
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn encode_cp932(source: &str) -> Result<Vec<u8>, StorageError> {
    let (encoded, _, had_errors) = SHIFT_JIS.encode(source);
    if had_errors {
        return Err(StorageError::Cp932Unmappable);
    }
    Ok(encoded.into_owned())
}

fn write_temp_file(destination: &Path, bytes: &[u8]) -> io::Result<PathBuf> {
    let parent = path_parent(destination);
    for _ in 0..128 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let name = format!(".mdlite-{}-{id}.tmp", std::process::id());
        let temp = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&temp);
                    return Err(error);
                }
                return Ok(temp);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique temporary save file",
    ))
}

fn move_temp(temp: &Path, destination: &Path, replace: bool) -> io::Result<()> {
    #[cfg(windows)]
    {
        let temp_wide = wide_path(temp);
        let destination_wide = wide_path(destination);
        let mut flags = MOVEFILE_WRITE_THROUGH;
        if replace {
            flags |= MOVEFILE_REPLACE_EXISTING;
        }
        let moved = unsafe { MoveFileExW(temp_wide.as_ptr(), destination_wide.as_ptr(), flags) };
        if moved == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    #[cfg(not(windows))]
    {
        if replace {
            fs::rename(temp, destination)
        } else {
            fs::hard_link(temp, destination)?;
            fs::remove_file(temp)
        }
    }
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIR_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            loop {
                let id = TEST_DIR_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir()
                    .join(format!("mdlite-storage-test-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("failed to create test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn utf8_bom_and_mixed_eol_bytes_are_preserved() {
        let dir = TestDir::new();
        let path = dir.0.join("mixed.md");
        let original = [UTF8_BOM, "# 日本語\r\nline two\nlast\r".as_bytes()].concat();
        fs::write(&path, &original).unwrap();

        let mut loaded = FileSnapshot::open(&path).unwrap();
        assert_eq!(loaded.source, "# 日本語\r\nline two\nlast\r");
        assert_eq!(loaded.snapshot.format(), FileFormat::Utf8 { bom: true });
        loaded.snapshot.save(&loaded.source).unwrap();
        assert_eq!(fs::read(&path).unwrap(), original);

        loaded.snapshot.save("# 更新\r\nline two\nlast\r").unwrap();
        let updated = [UTF8_BOM, "# 更新\r\nline two\nlast\r".as_bytes()].concat();
        assert_eq!(fs::read(&path).unwrap(), updated);
    }

    #[test]
    fn stale_file_bytes_are_refused_without_overwriting_external_content() {
        let dir = TestDir::new();
        let path = dir.0.join("stale.md");
        fs::write(&path, b"before\r\nafter").unwrap();
        let mut loaded = FileSnapshot::open(&path).unwrap();
        fs::write(&path, b"external edit").unwrap();

        assert!(matches!(
            loaded.snapshot.save("agent edit"),
            Err(StorageError::ExternalChange)
        ));
        assert_eq!(fs::read(path).unwrap(), b"external edit");
    }

    #[cfg(windows)]
    #[test]
    fn save_as_existing_retains_source_format_and_rejects_loss() {
        let dir = TestDir::new();
        let path = dir.0.join("destination.md");
        fs::write(&path, "existing UTF-8").unwrap();
        let mut loaded = FileSnapshot::open(&path).unwrap();
        loaded
            .snapshot
            .save_with_format("日本語\r\n", FileFormat::Cp932)
            .unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            encode_cp932("日本語\r\n").unwrap()
        );
        let before = fs::read(&path).unwrap();
        assert!(matches!(
            loaded.snapshot.save_with_format("😀", FileFormat::Cp932),
            Err(StorageError::Cp932Unmappable)
        ));
        assert_eq!(fs::read(&path).unwrap(), before);
        loaded
            .snapshot
            .save_with_format("BOM\n", FileFormat::Utf8 { bom: true })
            .unwrap();
        assert_eq!(fs::read(path).unwrap(), [UTF8_BOM, b"BOM\n"].concat());
    }

    #[test]
    fn save_as_refuses_an_existing_path_and_writes_a_new_utf8_file() {
        let dir = TestDir::new();
        let path = dir.0.join("new.md");
        let snapshot =
            FileSnapshot::create_new(&path, "日本語\r\n", FileFormat::default()).unwrap();
        assert_eq!(fs::read(snapshot.path()).unwrap(), "日本語\r\n".as_bytes());
        assert!(matches!(
            FileSnapshot::create_new(&path, "overwrite", FileFormat::default()),
            Err(StorageError::DestinationExists)
        ));
        assert_eq!(fs::read(path).unwrap(), "日本語\r\n".as_bytes());
    }

    #[cfg(windows)]
    #[test]
    fn cp932_round_trips_supported_text_and_rejects_unmappable_edits() {
        let dir = TestDir::new();
        let path = dir.0.join("cp932.md");
        let source = "日本語\r\nEnglish\n";
        let mut snapshot = FileSnapshot::create_new(&path, source, FileFormat::Cp932).unwrap();
        let original = fs::read(&path).unwrap();
        let loaded = FileSnapshot::open(&path).unwrap();
        assert_eq!(loaded.source, source);
        assert_eq!(loaded.snapshot.format(), FileFormat::Cp932);
        snapshot.save("日本語更新\r\nEnglish\n").unwrap();

        let after_save = fs::read(&path).unwrap();
        let mut reopened = FileSnapshot::open(&path).unwrap();
        assert!(matches!(
            reopened.snapshot.save("絵文字 😀"),
            Err(StorageError::Cp932Unmappable)
        ));
        assert_eq!(fs::read(&path).unwrap(), after_save);
        assert_ne!(original, after_save);
    }

    #[cfg(windows)]
    #[test]
    fn invalid_cp932_bytes_are_rejected() {
        let dir = TestDir::new();
        let path = dir.0.join("invalid.md");
        fs::write(&path, [0x82]).unwrap();
        assert!(matches!(
            FileSnapshot::open(path),
            Err(StorageError::InvalidCp932)
        ));
    }
}
