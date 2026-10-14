//! Machine-local consent tied to canonical path AND physical directory identity.
//! Trust is a permission check, not an operating-system sandbox.
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceIdentity {
    pub canonical: PathBuf,
    fingerprint: String,
}
impl WorkspaceIdentity {
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    pub fn read(path: &Path) -> Result<Self, String> {
        let canonical = fs::canonicalize(path).map_err(|e| e.to_string())?;
        if !canonical.is_dir() {
            return Err("Workspace must be a directory".into());
        }
        let mut hash = Sha256::new();
        #[cfg(windows)]
        {
            use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
            use windows_sys::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, GetFileInformationByHandle,
            };
            let file = fs::OpenOptions::new()
                .access_mode(0)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(&canonical)
                .map_err(|e| e.to_string())?;
            let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            // ASCII case folding avoids changing non-ASCII path spellings through lossy UTF-8.
            for unit in canonical.as_os_str().encode_wide() {
                let unit = if (65..=90).contains(&unit) {
                    unit + 32
                } else {
                    unit
                };
                hash.update(unit.to_le_bytes());
            }
            hash.update(info.dwVolumeSerialNumber.to_le_bytes());
            hash.update(info.nFileIndexHigh.to_le_bytes());
            hash.update(info.nFileIndexLow.to_le_bytes());
        }
        #[cfg(unix)]
        {
            use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};
            let metadata = fs::metadata(&canonical).map_err(|e| e.to_string())?;
            hash.update(canonical.as_os_str().as_bytes());
            hash.update(metadata.dev().to_le_bytes());
            hash.update(metadata.ino().to_le_bytes());
        }
        #[cfg(not(any(unix, windows)))]
        {
            return Err("Physical directory identity unavailable".into());
        }
        Ok(Self {
            canonical,
            fingerprint: format!("{:x}", hash.finalize()),
        })
    }
}
#[derive(Clone, Debug)]
pub struct TrustToken {
    identity: WorkspaceIdentity,
}
impl TrustToken {
    pub fn identity(&self) -> &WorkspaceIdentity {
        &self.identity
    }
}

#[derive(Default)]
struct State {
    mutations: HashSet<String>,
}
#[derive(Clone)]
pub struct TrustStore {
    root: PathBuf,
    state: Arc<Mutex<State>>,
}
impl TrustStore {
    pub fn local() -> Result<Self, String> {
        let base = std::env::var_os("LOCALAPPDATA")
            .filter(|v| !v.is_empty())
            .ok_or("Local AppData unavailable")?;
        Ok(Self::at(PathBuf::from(base).join("MDLite/trust/v2")))
    }
    /// Explicit registry root, also used by synthetic fixtures. Never derive this from a workspace file.
    pub fn at(root: PathBuf) -> Self {
        // All callers in this app share mutation exclusion, even with separately constructed stores.
        static STATE: OnceLock<Arc<Mutex<State>>> = OnceLock::new();
        Self {
            root,
            state: STATE.get_or_init(Arc::default).clone(),
        }
    }
    fn path(&self, identity: &WorkspaceIdentity) -> PathBuf {
        self.root.join(format!("{}.trust", identity.fingerprint))
    }
    fn lock_registry(&self, identity: &WorkspaceIdentity) -> Result<fs::File, String> {
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // File sharing exclusion also protects against another MDLite process.
            options.share_mode(0);
        }
        options
            .open(self.root.join(format!("{}.lock", identity.fingerprint)))
            .map_err(|e| format!("Trust registry busy or unavailable: {e}"))
    }
    fn trusted(&self, identity: &WorkspaceIdentity) -> bool {
        let Ok(file) = fs::File::open(self.path(identity)) else {
            return false;
        };
        let mut bytes = Vec::new();
        file.take(256).read_to_end(&mut bytes).is_ok()
            && bytes == format!("MDLITE_TRUST_V2\n{}\n", identity.fingerprint).as_bytes()
    }
    pub fn token(&self, workspace: &Path) -> Result<TrustToken, String> {
        let _state = self.state.lock().map_err(|_| "Trust state unavailable")?;
        let identity = WorkspaceIdentity::read(workspace)?;
        if !self.trusted(&identity) {
            return Err("Workspace is not trusted on this machine".into());
        }
        Ok(TrustToken { identity })
    }
    pub fn check(&self, token: &TrustToken, workspace: &Path) -> Result<(), String> {
        let _state = self.state.lock().map_err(|_| "Trust state unavailable")?;
        self.check_locked(token, workspace)
    }
    fn check_locked(&self, token: &TrustToken, workspace: &Path) -> Result<(), String> {
        if WorkspaceIdentity::read(workspace)? != token.identity || !self.trusted(&token.identity) {
            return Err("Trust revoked or workspace identity changed".into());
        }
        Ok(())
    }
    pub fn set_trusted(&self, workspace: &Path, trusted: bool) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "Trust state unavailable")?;
        let identity = WorkspaceIdentity::read(workspace)?;
        if state.mutations.contains(&identity.fingerprint) {
            return Err("Trust cannot change during an external mutation".into());
        }
        let _registry_lock = self.lock_registry(&identity)?;
        let path = self.path(&identity);
        if !trusted {
            return match fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            };
        }
        fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        // create_new ensures an incomplete write is never accepted as another grant.
        if self.trusted(&identity) {
            return Ok(());
        }
        if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        if let Err(e) = (|| -> std::io::Result<()> {
            file.write_all(format!("MDLITE_TRUST_V2\n{}\n", identity.fingerprint).as_bytes())?;
            file.sync_all()
        })() {
            let _ = fs::remove_file(path);
            return Err(e.to_string());
        }
        Ok(())
    }
    pub fn begin_mutation(
        &self,
        token: &TrustToken,
        workspace: &Path,
    ) -> Result<MutationLease, String> {
        let mut state = self.state.lock().map_err(|_| "Trust state unavailable")?;
        let registry_lock = self.lock_registry(&token.identity)?;
        self.check_locked(token, workspace)?;
        if state.mutations.contains(&token.identity.fingerprint) {
            return Err("Workspace mutation already in progress".into());
        }
        state.mutations.insert(token.identity.fingerprint.clone());
        Ok(MutationLease {
            state: self.state.clone(),
            key: token.identity.fingerprint.clone(),
            _registry_lock: registry_lock,
        })
    }
}
pub struct MutationLease {
    state: Arc<Mutex<State>>,
    key: String,
    _registry_lock: fs::File,
}
impl Drop for MutationLease {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.mutations.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_identity_revocation_and_busy() {
        let root = std::env::temp_dir().join(format!(
            "mdlite-trust-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        fs::create_dir_all(&workspace).unwrap();
        let store = TrustStore::at(root.join("registry"));
        assert!(store.token(&workspace).is_err());
        store.set_trusted(&workspace, true).unwrap();
        let token = store.token(&workspace).unwrap();
        let lease = store.begin_mutation(&token, &workspace).unwrap();
        assert!(store.set_trusted(&workspace, false).is_err());
        let other = TrustStore::at(root.join("registry"));
        assert!(other.set_trusted(&workspace, false).is_err());
        #[cfg(windows)]
        {
            let independent_process_state = TrustStore {
                root: root.join("registry"),
                state: Arc::default(),
            };
            assert!(
                independent_process_state
                    .set_trusted(&workspace, false)
                    .is_err()
            );
        }
        assert!(store.begin_mutation(&token, &workspace).is_err());
        drop(lease);
        fs::rename(&workspace, root.join("old")).unwrap();
        fs::create_dir(&workspace).unwrap();
        assert!(store.check(&token, &workspace).is_err());
        assert!(store.token(&workspace).is_err());
        store.set_trusted(&workspace, true).unwrap();
        let token = store.token(&workspace).unwrap();
        store.set_trusted(&workspace, false).unwrap();
        assert!(store.check(&token, &workspace).is_err());
        let identity = WorkspaceIdentity::read(&workspace).unwrap();
        fs::write(store.path(&identity), "MDLITE_TRUST_V2\ncorrupt\n").unwrap();
        assert!(store.token(&workspace).is_err());
        store.set_trusted(&workspace, true).unwrap();
        assert!(store.token(&workspace).is_ok());
        assert!(store.token(&root.join("missing-directory")).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
