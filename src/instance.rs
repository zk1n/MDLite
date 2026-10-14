//! One local editor process per physical Workspace. No Workspace files are written.
#[cfg(windows)]
mod platform {
    use std::{ffi::c_void, path::Path};
    use windows_sys::Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*};
    pub struct WorkspaceLease {
        handle: HANDLE,
        pub message: u32,
    }
    impl Drop for WorkspaceLease {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
    pub enum Claim {
        Owned(WorkspaceLease),
        Existing { message: u32 },
    }
    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }
    pub fn claim(root: &Path) -> Result<Claim, String> {
        let identity = crate::trust::WorkspaceIdentity::read(root)?;
        let key = identity.fingerprint();
        unsafe {
            let message = RegisterWindowMessageW(wide(&format!("MDLite.Workspace.{key}")).as_ptr());
            if message == 0 {
                return Err(std::io::Error::last_os_error().to_string());
            }
            let handle = CreateMutexW(
                std::ptr::null(),
                0,
                wide(&format!("Local\\MDLite.Workspace.{key}")).as_ptr(),
            );
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().to_string());
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                Ok(Claim::Existing { message })
            } else {
                Ok(Claim::Owned(WorkspaceLease { handle, message }))
            }
        }
    }
    pub fn notify_existing(message: u32) -> bool {
        struct Search {
            message: u32,
            exe: std::path::PathBuf,
            found: bool,
        }
        unsafe extern "system" fn window(hwnd: HWND, data: LPARAM) -> i32 {
            unsafe {
                let search = &mut *(data as *mut Search);
                let mut name = [0u16; 128];
                let count = GetClassNameW(hwnd, name.as_mut_ptr(), name.len() as i32);
                if count <= 0
                    || String::from_utf16_lossy(&name[..count as usize]) != "MDLiteRustEditor"
                {
                    return 1;
                }
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, &mut pid);
                if pid == GetCurrentProcessId() {
                    return 1;
                }
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if process.is_null() {
                    return 1;
                }
                let mut path = vec![0u16; 32768];
                let mut length = path.len() as u32;
                let ok = QueryFullProcessImageNameW(
                    process,
                    PROCESS_NAME_WIN32,
                    path.as_mut_ptr(),
                    &mut length,
                );
                CloseHandle(process);
                if ok == 0 {
                    return 1;
                }
                let candidate =
                    std::path::PathBuf::from(String::from_utf16_lossy(&path[..length as usize]));
                if std::fs::canonicalize(candidate).ok() != Some(search.exe.clone()) {
                    return 1;
                }
                let mut response = 0;
                AllowSetForegroundWindow(pid);
                if SendMessageTimeoutW(
                    hwnd,
                    search.message,
                    0,
                    0,
                    SMTO_ABORTIFHUNG,
                    300,
                    &mut response,
                ) != 0
                    && response == 1
                {
                    search.found = true;
                    return 0;
                }
                1
            }
        }
        let Ok(exe) = std::env::current_exe().and_then(std::fs::canonicalize) else {
            return false;
        };
        let mut search = Search {
            message,
            exe,
            found: false,
        };
        unsafe {
            EnumWindows(
                Some(window),
                (&mut search as *mut Search as *mut c_void) as isize,
            );
        }
        search.found
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn duplicate_lease_releases_without_writing_workspace() {
            let path = std::env::temp_dir().join(format!(
                "mdlite-instance-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir(&path).unwrap();
            let Claim::Owned(first) = claim(&path).unwrap() else {
                panic!("first lease")
            };
            assert!(matches!(claim(&path).unwrap(), Claim::Existing { .. }));
            assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
            drop(first);
            assert!(matches!(claim(&path).unwrap(), Claim::Owned(_)));
            std::fs::remove_dir(path).unwrap();
        }
    }
}
#[cfg(windows)]
pub use platform::*;
