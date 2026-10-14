//! Manually refreshed, privacy-limited native diagnostics.

use super::*;
use std::{cell::Cell, collections::HashMap, rc::Rc, sync::OnceLock};
use windows::Win32::{
    Foundation::{COLORREF, ERROR_CLASS_ALREADY_EXISTS},
    Graphics::Gdi::{
        CreateSolidBrush, DeleteObject, FillRect, HBRUSH, HDC, SetBkColor, SetTextColor,
    },
};
use windows_sys::Win32::System::{
    ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX},
    Threading::GetCurrentProcess,
};

const DIAGNOSTICS_COMMAND_ID: usize = 167;
const REFRESH_BUTTON_ID: usize = 1;
const OUTPUT_EDIT_ID: usize = 2;
const WM_UPDATE_THEME: u32 = WM_APP + 97;
const MAX_DIRECT_DEPENDENCIES: usize = 24;
const MAX_DEPENDENCY_NAME: usize = 64;
const MAX_DEPENDENCY_VERSION: usize = 48;

static WINDOW_CLASS_REGISTERED: OnceLock<bool> = OnceLock::new();

#[derive(Default)]
pub(super) struct DiagnosticsPane {
    hwnd: HWND,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ProcessMemory {
    working_set: usize,
    private_bytes: usize,
    peak_working_set: usize,
}

fn current_process_memory() -> Option<ProcessMemory> {
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            (&mut counters as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
    };
    (ok != 0).then_some(ProcessMemory {
        working_set: counters.WorkingSetSize,
        private_bytes: counters.PrivateUsage,
        peak_working_set: counters.PeakWorkingSetSize,
    })
}

#[derive(Debug)]
struct DiagnosticsSnapshot {
    memory: Option<ProcessMemory>,
    document_count: usize,
    active_document: usize,
    encoding: &'static str,
    line_endings: &'static str,
    dirty: bool,
    source_bytes: usize,
    revision: u64,
    undo_count: usize,
    redo_count: usize,
    search: String,
}

impl DiagnosticsSnapshot {
    fn from_app(app: &App) -> Self {
        let active = app.tabs.get(app.active);
        let (encoding, line_endings, dirty, source_bytes, revision, undo_count, redo_count) =
            active.map_or(("none", "none", false, 0, 0, 0, 0), |tab| {
                let encoding = match tab.file.as_ref().map(|file| file.format()) {
                    Some(mdlite::storage::FileFormat::Utf8 { bom: true }) => "UTF-8 with BOM",
                    Some(mdlite::storage::FileFormat::Utf8 { bom: false }) => "UTF-8",
                    Some(mdlite::storage::FileFormat::Cp932) => "CP932",
                    None => "UTF-8 (unsaved)",
                };
                (
                    encoding,
                    line_ending_kind(tab.doc.source()),
                    tab.doc.is_dirty(),
                    tab.doc.byte_len(),
                    tab.doc.revision(),
                    tab.doc.undo_count(),
                    tab.doc.redo_count(),
                )
            });

        Self {
            memory: current_process_memory(),
            document_count: app.tabs.len(),
            active_document: active.map_or(0, |_| app.active + 1),
            encoding,
            line_endings,
            dirty,
            source_bytes,
            revision,
            undo_count,
            redo_count,
            search: app.search_status_for_diagnostics(),
        }
    }

    fn render(&self) -> String {
        let mut text = format!(
            "MDLite diagnostics\r\nApp version: {}\r\n\r\n",
            env!("CARGO_PKG_VERSION")
        );
        match self.memory {
            Some(memory) => {
                text.push_str(&format!(
                    "Process memory\r\n  Working set: {}\r\n  Private bytes: {}\r\n  Peak working set: {}\r\n\r\n",
                    format_bytes(memory.working_set),
                    format_bytes(memory.private_bytes),
                    format_bytes(memory.peak_working_set),
                ));
            }
            None => text.push_str("Process memory: unavailable\r\n\r\n"),
        }
        text.push_str(&format!(
            "Documents: {} open\r\nActive document: {} of {}\r\nEncoding: {}\r\nLine endings: {}\r\nDirty: {}\r\nSource bytes: {}\r\nRevision: {}\r\nEdit history: {} undo / {} redo\r\n\r\n{}\r\n\r\n",
            self.document_count,
            self.active_document,
            self.document_count,
            self.encoding,
            self.line_endings,
            if self.dirty { "yes" } else { "no" },
            self.source_bytes,
            self.revision,
            self.undo_count,
            self.redo_count,
            self.search,
        ));
        text.push_str("Direct dependency versions (Cargo.lock)\r\n");
        text.push_str(&locked_dependency_summary(include_str!("../../Cargo.lock")));
        text
    }
}

fn line_ending_kind(source: &str) -> &'static str {
    let bytes = source.as_bytes();
    let (mut crlf, mut lf, mut cr) = (0usize, 0usize, 0usize);
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'\r' if bytes.get(index + 1) == Some(&b'\n') => {
                crlf += 1;
                index += 2;
            }
            b'\r' => {
                cr += 1;
                index += 1;
            }
            b'\n' => {
                lf += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    let kinds = (crlf != 0) as usize + (lf != 0) as usize + (cr != 0) as usize;
    if kinds == 0 {
        "none"
    } else if kinds > 1 {
        "mixed"
    } else if crlf != 0 {
        "CRLF"
    } else if lf != 0 {
        "LF"
    } else {
        "CR"
    }
}

fn format_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", value / (KIB * KIB * KIB))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

fn locked_dependency_summary(lock: &str) -> String {
    let blocks: Vec<_> = lock.split("[[package]]").skip(1).collect();
    let mut package_versions: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut direct_dependencies = Vec::new();

    for block in blocks {
        let name = toml_string_field(block, "name = ");
        let version = toml_string_field(block, "version = ");
        if let (Some(name), Some(version)) = (name, version) {
            package_versions.entry(name).or_default().push(version);
        }
        if name == Some("mdlite")
            && let Some((_, dependencies)) = block.split_once("dependencies = [")
        {
            for line in dependencies
                .lines()
                .take_while(|line| !line.trim().starts_with(']'))
            {
                if let Some(dependency) = toml_array_string(line) {
                    direct_dependencies.push(dependency);
                }
            }
        }
    }

    direct_dependencies.sort_unstable();
    direct_dependencies.dedup();
    let mut output = String::new();
    for dependency in direct_dependencies.iter().take(MAX_DIRECT_DEPENDENCIES) {
        let mut pieces = dependency.split_whitespace();
        let Some(name) = pieces.next() else { continue };
        if !valid_dependency_piece(name, MAX_DEPENDENCY_NAME) {
            continue;
        }
        let requested_version = pieces.next();
        let Some(version) = package_versions.get(name).and_then(|versions| {
            versions
                .iter()
                .copied()
                .find(|version| requested_version.is_none_or(|requested| requested == *version))
        }) else {
            continue;
        };
        if !valid_dependency_piece(version, MAX_DEPENDENCY_VERSION) {
            continue;
        }
        output.push_str(&format!("  {name} {version}\r\n"));
    }
    if output.is_empty() {
        output.push_str("  unavailable\r\n");
    }
    output
}

fn toml_string_field<'a>(block: &'a str, field: &str) -> Option<&'a str> {
    block
        .lines()
        .find_map(|line| line.trim().strip_prefix(field))
        .and_then(|value| value.strip_prefix('"'))
        .and_then(|value| value.split_once('"').map(|(value, _)| value))
}

fn toml_array_string(line: &str) -> Option<&str> {
    let line = line.trim();
    let line = line.strip_prefix('"')?;
    let (value, _) = line.split_once('"')?;
    Some(value)
}

fn valid_dependency_piece(value: &str, limit: usize) -> bool {
    !value.is_empty()
        && value.len() <= limit
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'+'))
}

fn color_ref(color: D2D1_COLOR_F) -> u32 {
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    channel(color.r) | (channel(color.g) << 8) | (channel(color.b) << 16)
}

struct DiagnosticsWindowState {
    owner: HWND,
    edit: HWND,
    refresh: HWND,
    text: String,
    background: COLORREF,
    foreground: COLORREF,
    brush: HBRUSH,
    owns_brush: bool,
    released: Rc<Cell<bool>>,
}

impl DiagnosticsWindowState {
    fn new(owner: HWND, text: String, background: COLORREF, foreground: COLORREF) -> Self {
        let brush = unsafe { CreateSolidBrush(background) };
        let owns_brush = !brush.0.is_null();
        Self {
            owner,
            edit: HWND::default(),
            refresh: HWND::default(),
            text,
            background,
            foreground,
            brush: if owns_brush {
                brush
            } else {
                unsafe { windows::Win32::Graphics::Gdi::GetSysColorBrush(COLOR_WINDOW) }
            },
            owns_brush,
            released: Rc::new(Cell::new(false)),
        }
    }

    fn set_theme(&mut self, background: COLORREF, foreground: COLORREF) {
        let brush = unsafe { CreateSolidBrush(background) };
        let owns_brush = !brush.0.is_null();
        if self.owns_brush {
            unsafe {
                let _ = DeleteObject(self.brush.into());
            }
        }
        self.background = background;
        self.foreground = foreground;
        self.brush = if owns_brush {
            brush
        } else {
            unsafe { windows::Win32::Graphics::Gdi::GetSysColorBrush(COLOR_WINDOW) }
        };
        self.owns_brush = owns_brush;
    }
}

impl Drop for DiagnosticsWindowState {
    fn drop(&mut self) {
        if self.owns_brush {
            unsafe {
                let _ = DeleteObject(self.brush.into());
            }
        }
    }
}

unsafe extern "system" fn diagnostics_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut DiagnosticsWindowState;
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let result = DefWindowProcW(hwnd, message, wparam, lparam);
            if !pointer.is_null() {
                let state = Box::from_raw(pointer);
                state.released.set(true);
                drop(state);
            }
            return result;
        }
        if pointer.is_null() {
            return DefWindowProcW(hwnd, message, wparam, lparam);
        }
        let state = &mut *pointer;
        match message {
            WM_CREATE => {
                let instance = match GetModuleHandleW(None) {
                    Ok(module) => Some(module.into()),
                    Err(_) => return LRESULT(-1),
                };
                let edit = CreateWindowExW(
                    WINDOW_EX_STYLE(WS_EX_CLIENTEDGE.0),
                    w!("EDIT"),
                    PCWSTR::null(),
                    WINDOW_STYLE(
                        WS_CHILD.0
                            | WS_VISIBLE.0
                            | WS_TABSTOP.0
                            | ES_MULTILINE as u32
                            | ES_AUTOVSCROLL as u32
                            | ES_READONLY as u32
                            | WS_VSCROLL.0,
                    ),
                    12,
                    48,
                    720,
                    520,
                    Some(hwnd),
                    Some(HMENU(OUTPUT_EDIT_ID as *mut _)),
                    instance,
                    None,
                );
                let Ok(edit) = edit else { return LRESULT(-1) };
                state.edit = edit;
                let button = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("BUTTON"),
                    w!("更新"),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_PUSHBUTTON as u32),
                    12,
                    10,
                    92,
                    30,
                    Some(hwnd),
                    Some(HMENU(REFRESH_BUTTON_ID as *mut _)),
                    instance,
                    None,
                );
                let Ok(button) = button else {
                    return LRESULT(-1);
                };
                state.refresh = button;
                let text = super::wide(&state.text);
                let _ = SetWindowTextW(edit, PCWSTR(text.as_ptr()));
                state.text.clear();
                LRESULT(0)
            }
            WM_SIZE => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let width = client.right - client.left;
                let height = client.bottom - client.top;
                let _ = MoveWindow(state.refresh, 12, 10, 92, 30, true);
                let _ = MoveWindow(
                    state.edit,
                    12,
                    48,
                    (width - 24).max(80),
                    (height - 60).max(80),
                    true,
                );
                LRESULT(0)
            }
            WM_COMMAND if wparam.0 & 0xffff == REFRESH_BUTTON_ID => {
                let _ = PostMessageW(
                    Some(state.owner),
                    WM_COMMAND,
                    WPARAM(DIAGNOSTICS_COMMAND_ID),
                    LPARAM(0),
                );
                LRESULT(0)
            }
            WM_UPDATE_THEME => {
                state.set_theme(COLORREF(wparam.0 as u32), COLORREF(lparam.0 as u32));
                let _ = InvalidateRect(Some(hwnd), None, true);
                LRESULT(0)
            }
            WM_ERASEBKGND => {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let _ = FillRect(HDC(wparam.0 as *mut _), &client, state.brush);
                LRESULT(1)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC => {
                let dc = HDC(wparam.0 as *mut _);
                let _ = SetTextColor(dc, state.foreground);
                let _ = SetBkColor(dc, state.background);
                LRESULT(state.brush.0 as isize)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

fn ensure_window_class() -> bool {
    *WINDOW_CLASS_REGISTERED.get_or_init(|| unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return false;
        };
        let class = WNDCLASSW {
            lpfnWndProc: Some(diagnostics_window_proc),
            hInstance: instance.into(),
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: w!("MDLiteDiagnosticsWindow"),
            ..Default::default()
        };
        let atom = RegisterClassW(&class);
        atom != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS
    })
}

impl DiagnosticsPane {
    unsafe fn show_or_refresh(
        &mut self,
        owner: HWND,
        text: &str,
        background: COLORREF,
        foreground: COLORREF,
        dpi: f32,
    ) -> bool {
        unsafe {
            if self.hwnd != HWND::default() && IsWindow(Some(self.hwnd)).as_bool() {
                if let Ok(edit) = GetDlgItem(Some(self.hwnd), OUTPUT_EDIT_ID as i32) {
                    let text = super::wide(text);
                    let _ = SetWindowTextW(edit, PCWSTR(text.as_ptr()));
                }
                let _ = SendMessageW(
                    self.hwnd,
                    WM_UPDATE_THEME,
                    Some(WPARAM(background.0 as usize)),
                    Some(LPARAM(foreground.0 as isize)),
                );
                let _ = ShowWindow(self.hwnd, SW_SHOW);
                let _ = SetForegroundWindow(self.hwnd);
                return true;
            }
            self.hwnd = HWND::default();
            if !ensure_window_class() {
                return false;
            }
            let scale = dpi.max(1.0);
            let width = (760.0 * scale).round() as i32;
            let height = (620.0 * scale).round() as i32;
            let state = Box::new(DiagnosticsWindowState::new(
                owner,
                text.to_owned(),
                background,
                foreground,
            ));
            let released = Rc::clone(&state.released);
            let pointer = Box::into_raw(state);
            let window = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                w!("MDLiteDiagnosticsWindow"),
                w!("MDLite — 診断情報"),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                width,
                height,
                Some(owner),
                None,
                GetModuleHandleW(None).ok().map(Into::into),
                Some(pointer.cast()),
            );
            match window {
                Ok(hwnd) => {
                    self.hwnd = hwnd;
                    let _ = ShowWindow(hwnd, SW_SHOW);
                    let _ = SetForegroundWindow(hwnd);
                    true
                }
                Err(_) => {
                    if !released.get() {
                        drop(Box::from_raw(pointer));
                    }
                    false
                }
            }
        }
    }
}

impl App {
    pub(super) fn diagnostics_command(&mut self) -> bool {
        let snapshot = DiagnosticsSnapshot::from_app(self).render();
        let owner = if self.shell.main_window != HWND::default() {
            self.shell.main_window
        } else {
            self.hwnd
        };
        unsafe {
            self.diagnostics.show_or_refresh(
                owner,
                &snapshot,
                COLORREF(color_ref(self.shell.tokens.panel)),
                COLORREF(color_ref(self.shell.tokens.foreground)),
                self.dpi,
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_omits_synthetic_note_query_and_absolute_path_canaries() {
        const NOTE: &str = "NOTE_BODY_CANARY_6f2d";
        const QUERY: &str = "PRIVATE_QUERY_CANARY_9b13";
        const PATH: &str = r"C:\PRIVATE_PATH_CANARY\note.md";

        let mut app = super::super::tests::composing_test_app(NOTE);
        app.search_ui
            .seed_diagnostics_privacy_test_state(QUERY, PATH);
        let output = DiagnosticsSnapshot::from_app(&app).render();

        assert!(!output.contains(NOTE));
        assert!(!output.contains(QUERY));
        assert!(!output.contains(PATH));
    }

    #[test]
    fn process_status_reports_nonzero_finite_values_for_this_process() {
        let memory = current_process_memory().expect("GetProcessMemoryInfo for current process");
        assert!(memory.working_set > 0);
        assert!(memory.private_bytes > 0);
        assert!(memory.peak_working_set >= memory.working_set);
    }
}
