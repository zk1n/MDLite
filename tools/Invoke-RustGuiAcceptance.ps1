[CmdletBinding()]
param(
    [ValidateSet('Debug', 'Release')][string]$Configuration = 'Release',
    [ValidateSet('Focused', 'Resource')][string]$Mode = 'Focused',
    [switch]$GuiOwnerAssigned,
    [switch]$ActivateOwnedWindowByClick,
    [switch]$SkipClipboard
)

$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$ExecutableRelativePath = if ($Configuration -eq 'Release') { 'target\release\mdlite.exe' } else { 'target\debug\mdlite.exe' }
$ExePath = Join-Path $ProjectRoot $ExecutableRelativePath
$CppExePath = Join-Path $ProjectRoot 'build\release\MDLite.exe'
$CppReceiptPath = Join-Path $ProjectRoot 'build\verification\build-receipt-release.json'
$CppExpectedSha256 = '06f192c57ef41d7cc8e70731cc76410b24580ceef7535597ca9be67f4ac0f303'
$RunParent = Join-Path $ProjectRoot '.codex-runtime\rust-foundation\gui'
$RuntimeFixture = Join-Path $ProjectRoot 'tests\fixtures'

$preflightErrors = [Collections.Generic.List[string]]::new()
$buildReceiptPath = Join-Path $ProjectRoot ('.codex-runtime\rust-foundation\build-{0}.json' -f $Configuration.ToLowerInvariant())
$RunPreflight = [ordered]@{
    gui_owner_assigned = [bool]$GuiOwnerAssigned.IsPresent
    mode = $Mode
    configuration = $Configuration
    resource_requires_release = ($Mode -ne 'Resource' -or $Configuration -eq 'Release')
    executable_path = [IO.Path]::GetFullPath($ExePath)
    executable_exists = (Test-Path -LiteralPath $ExePath -PathType Leaf)
    executable_sha256 = $null
    executable_hash_error = $null
    build_receipt_path = [IO.Path]::GetFullPath($buildReceiptPath)
    build_receipt_exists = (Test-Path -LiteralPath $buildReceiptPath -PathType Leaf)
    build_receipt_read_error = $null
    build_receipt_configuration = $null
    build_receipt_status = $null
    build_receipt_executable = $null
    build_receipt_executable_sha256 = $null
    build_receipt_source_sha256 = $null
    current_source_sha256 = $null
    source_identity_error = $null
    executable_matches_receipt = $false
    source_matches_receipt = $false
    apartment_state = [Threading.Thread]::CurrentThread.GetApartmentState().ToString()
    sta_required = $true
    process_inventory_status = 'NOT_RUN'
    blocking_processes = @()
    activate_owned_window_by_click = [bool]$ActivateOwnedWindowByClick.IsPresent
    skip_clipboard = [bool]$SkipClipboard.IsPresent
    errors = @()
    passed = $false
}

if (-not $RunPreflight.gui_owner_assigned) { $preflightErrors.Add('gui_owner_assignment_missing') }
if (-not $RunPreflight.resource_requires_release) { $preflightErrors.Add('resource_mode_requires_release_configuration') }
if (-not $RunPreflight.executable_exists) { $preflightErrors.Add('target_executable_missing') }
else {
    try { $RunPreflight.executable_sha256 = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant() }
    catch { $RunPreflight.executable_hash_error = $_.Exception.Message; $preflightErrors.Add('target_executable_hash_read_failed') }
}
if ($RunPreflight.apartment_state -ne 'STA') { $preflightErrors.Add('sta_apartment_required') }

$buildReceipt = $null
if ($RunPreflight.build_receipt_exists) {
    try {
        $buildReceipt = Get-Content -LiteralPath $buildReceiptPath -Raw | ConvertFrom-Json
        $RunPreflight.build_receipt_configuration = [string]$buildReceipt.configuration
        $RunPreflight.build_receipt_status = [string]$buildReceipt.build
        $RunPreflight.build_receipt_executable = [string]$buildReceipt.executable
        $RunPreflight.build_receipt_executable_sha256 = [string]$buildReceipt.executable_sha256
        $RunPreflight.build_receipt_source_sha256 = [string]$buildReceipt.source_sha256
    }
    catch {
        $RunPreflight.build_receipt_read_error = $_.Exception.Message
        $preflightErrors.Add('build_receipt_unreadable')
    }
}
else { $preflightErrors.Add('build_receipt_missing') }

if ($null -ne $buildReceipt) {
    if ($RunPreflight.build_receipt_configuration -ine $Configuration) { $preflightErrors.Add('build_receipt_configuration_mismatch') }
    if ($RunPreflight.build_receipt_status -cne 'PASS') { $preflightErrors.Add('build_receipt_status_not_pass') }
    try {
        $receiptExecutablePath = if ([string]::IsNullOrWhiteSpace($RunPreflight.build_receipt_executable)) { '' } else { [IO.Path]::GetFullPath($RunPreflight.build_receipt_executable) }
        if (-not [string]::Equals($receiptExecutablePath, $RunPreflight.executable_path, [StringComparison]::OrdinalIgnoreCase)) { $preflightErrors.Add('build_receipt_executable_path_mismatch') }
    }
    catch {
        $preflightErrors.Add('build_receipt_executable_path_invalid: ' + $_.Exception.Message)
    }
    $RunPreflight.executable_matches_receipt = $RunPreflight.executable_exists -and $RunPreflight.executable_sha256 -ceq $RunPreflight.build_receipt_executable_sha256
    if (-not $RunPreflight.executable_matches_receipt) { $preflightErrors.Add('target_executable_hash_mismatch') }
}

try {
    $sourcePaths = @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml') + @(Get-ChildItem -LiteralPath (Join-Path $ProjectRoot 'src') -File -Recurse | ForEach-Object { [IO.Path]::GetRelativePath($ProjectRoot, $_.FullName).Replace('\', '/') })
    $sourceManifest = foreach ($path in ($sourcePaths | Sort-Object)) { '{0}:{1}' -f $path, (Get-FileHash -LiteralPath (Join-Path $ProjectRoot $path) -Algorithm SHA256).Hash.ToLowerInvariant() }
    $RunPreflight.current_source_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes(($sourceManifest -join "`n")))).ToLowerInvariant()
    $RunPreflight.source_matches_receipt = ($null -ne $buildReceipt -and $RunPreflight.current_source_sha256 -ceq $RunPreflight.build_receipt_source_sha256)
    if ($null -ne $buildReceipt -and -not $RunPreflight.source_matches_receipt) { $preflightErrors.Add('product_source_hash_mismatch') }
}
catch {
    $RunPreflight.source_identity_error = $_.Exception.Message
    $preflightErrors.Add('product_source_identity_check_failed')
}

try {
    $blockingNames = @('Code', 'CodeLLDB', 'MDLite', 'mdlite')
    $RunPreflight.blocking_processes = @(Get-Process -ErrorAction Stop | Where-Object { $blockingNames -contains $_.ProcessName } | Select-Object Id, ProcessName)
    $RunPreflight.process_inventory_status = 'PASS'
    if ($RunPreflight.blocking_processes.Count -gt 0) { $preflightErrors.Add('blocking_app_or_ide_processes_active') }
}
catch {
    $RunPreflight.process_inventory_status = 'FAIL'
    $preflightErrors.Add('process_inventory_failed: ' + $_.Exception.Message)
}

$RunPreflight.errors = @($preflightErrors)
$RunPreflight.passed = ($preflightErrors.Count -eq 0)
if (-not $RunPreflight.passed) {
    Write-Output ($RunPreflight | ConvertTo-Json -Depth 8)
    throw ('GUI preflight failed: ' + ($preflightErrors -join '; '))
}

$NativeSource = @'
using System;
using System.ComponentModel;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class MdliteRustGuiNative {
    public const uint INPUT_KEYBOARD = 1;
    public const uint INPUT_MOUSE = 0;
    public const uint KEYEVENTF_KEYUP = 0x0002;
    public const uint KEYEVENTF_UNICODE = 0x0004;
    public const uint MOUSEEVENTF_LEFTDOWN = 0x0002;
    public const uint MOUSEEVENTF_LEFTUP = 0x0004;
    public const uint MOUSEEVENTF_RIGHTDOWN = 0x0008;
    public const uint MOUSEEVENTF_RIGHTUP = 0x0010;
    public const uint MOUSEEVENTF_WHEEL = 0x0800;
    public const uint SMTO_ABORTIFHUNG = 0x0002;
    public const uint SWP_NOZORDER = 0x0004;
    public const uint SWP_NOACTIVATE = 0x0010;
    public const int SW_RESTORE = 9;
    public const uint WM_NULL = 0x0000;
    public const uint WM_CLOSE = 0x0010;
    public const uint WM_NCHITTEST = 0x0084;
    public const ushort VK_LWIN = 0x5B;
    public const ushort VK_SPACE = 0x20;
    public const uint VK_IME_ON = 0x0016;
    public const uint VK_IME_OFF = 0x001A;
    public const uint TCM_GETITEMCOUNT = 0x1304;
    public const uint GW_OWNER = 4;
    public const uint GA_ROOT = 2;
    public const int HTCAPTION = 2;
    private static readonly IntPtr HWND_MESSAGE = new IntPtr(-3);
    private static IntPtr _clipboardOwnerWindow = IntPtr.Zero;
    private static string _clipboardOwnerWindowTitle;
    private static uint _clipboardOwnerProcessId;
    private static uint _clipboardOwnerThreadId;

    [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct CandidateForm { public uint Index, Style; public Point CurrentPos; public Rect Area; }
    [StructLayout(LayoutKind.Sequential)] public struct KeyboardInput {
        public ushort VirtualKey, ScanCode;
        public uint Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] public struct MouseInput {
        public int X, Y;
        public uint MouseData, Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Explicit)] public struct InputUnion {
        [FieldOffset(0)] public KeyboardInput Keyboard;
        [FieldOffset(0)] public MouseInput Mouse;
    }
    [StructLayout(LayoutKind.Sequential)] public struct Input { public uint Type; public InputUnion Union; }
    public static Input Key(ushort virtualKey, uint flags) {
        return new Input { Type = INPUT_KEYBOARD, Union = new InputUnion { Keyboard = new KeyboardInput { VirtualKey = virtualKey, Flags = flags } } };
    }
    public static Input Unicode(ushort codeUnit, uint flags) {
        return new Input { Type = INPUT_KEYBOARD, Union = new InputUnion { Keyboard = new KeyboardInput { ScanCode = codeUnit, Flags = flags | KEYEVENTF_UNICODE } } };
    }
    public static Input Wheel(int delta) {
        return new Input { Type = INPUT_MOUSE, Union = new InputUnion { Mouse = new MouseInput { MouseData = unchecked((uint)delta), Flags = MOUSEEVENTF_WHEEL } } };
    }
    public static Input MouseButton(uint flags) {
        return new Input { Type = INPUT_MOUSE, Union = new InputUnion { Mouse = new MouseInput { Flags = flags } } };
    }
    public static IntPtr ScreenPointLParam(int x, int y) {
        uint packed = ((uint)unchecked((ushort)y) << 16) | unchecked((ushort)x);
        return new IntPtr(unchecked((int)packed));
    }
    [DllImport("user32.dll", SetLastError = true)] public static extern uint SendInput(uint count, Input[] inputs, int inputSize);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll", SetLastError = true)] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsZoomed(IntPtr window);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool InvalidateRect(IntPtr window, IntPtr rect, bool erase);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool DestroyWindow(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern IntPtr CreateWindowExW(uint exStyle, string className, string windowName, uint style, int x, int y, int width, int height, IntPtr parent, IntPtr menu, IntPtr instance, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern IntPtr FindWindowExW(IntPtr parent, IntPtr childAfter, string className, string windowName);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassNameW(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageTimeoutW(IntPtr window, uint message, IntPtr wParam, IntPtr lParam, uint flags, uint timeoutMs, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr data);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr data);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool OpenClipboard(IntPtr owner);
    [DllImport("user32.dll", SetLastError = true)] public static extern bool CloseClipboard();
    [DllImport("user32.dll", SetLastError = true)] public static extern bool EmptyClipboard();
    [DllImport("user32.dll")] public static extern uint GetClipboardSequenceNumber();
    [DllImport("user32.dll", SetLastError = true)] public static extern uint EnumClipboardFormats(uint format);
    [DllImport("user32.dll", SetLastError = true)] public static extern IntPtr GetClipboardData(uint format);
    [DllImport("user32.dll", SetLastError = true)] public static extern IntPtr SetClipboardData(uint format, IntPtr memory);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern int GetClipboardFormatNameW(uint format, StringBuilder name, int capacity);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentProcessId();
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern IntPtr GetModuleHandleW(string moduleName);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr GlobalAlloc(uint flags, UIntPtr bytes);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr GlobalFree(IntPtr memory);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern UIntPtr GlobalSize(IntPtr memory);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern IntPtr GlobalLock(IntPtr memory);
    [DllImport("kernel32.dll", SetLastError = true)] public static extern bool GlobalUnlock(IntPtr memory);
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr data);
    [DllImport("imm32.dll")] public static extern IntPtr ImmGetContext(IntPtr window);
    [DllImport("imm32.dll")] public static extern bool ImmGetCandidateWindow(IntPtr context, uint index, out CandidateForm form);
    [DllImport("imm32.dll")] public static extern bool ImmReleaseContext(IntPtr window, IntPtr context);

    public static IntPtr GetClipboardOwnerWindow() {
        if (Thread.CurrentThread.GetApartmentState() != ApartmentState.STA) throw new InvalidOperationException("The clipboard owner window must be created and used on the harness STA thread.");
        uint processId = GetCurrentProcessId();
        uint threadId = GetCurrentThreadId();
        if (_clipboardOwnerWindow == IntPtr.Zero) {
            _clipboardOwnerWindowTitle = "MDLite clipboard owner " + processId + "-" + threadId + "-" + Guid.NewGuid().ToString("N");
            IntPtr window = CreateWindowExW(0, "STATIC", _clipboardOwnerWindowTitle, 0, 0, 0, 0, 0, HWND_MESSAGE, IntPtr.Zero, GetModuleHandleW(null), IntPtr.Zero);
            if (window == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not create the runner-owned message-only clipboard window.");
            _clipboardOwnerWindow = window;
            _clipboardOwnerProcessId = processId;
            _clipboardOwnerThreadId = threadId;
        }
        ValidateClipboardOwnerWindow(processId, threadId);
        return _clipboardOwnerWindow;
    }

    public static string GetClipboardOwnerWindowTitle() {
        ValidateClipboardOwnerWindow(GetCurrentProcessId(), GetCurrentThreadId());
        return _clipboardOwnerWindowTitle;
    }

    public static bool HasClipboardOwnerWindow() { return _clipboardOwnerWindow != IntPtr.Zero; }

    public static bool DestroyClipboardOwnerWindow() {
        if (_clipboardOwnerWindow == IntPtr.Zero) return true;
        ValidateClipboardOwnerWindow(GetCurrentProcessId(), GetCurrentThreadId());
        if (!DestroyWindow(_clipboardOwnerWindow)) throw new Win32Exception(Marshal.GetLastWin32Error(), "Could not destroy the runner-owned message-only clipboard window after clipboard cleanup.");
        _clipboardOwnerWindow = IntPtr.Zero;
        _clipboardOwnerProcessId = 0;
        _clipboardOwnerThreadId = 0;
        return true;
    }

    private static void ValidateClipboardOwnerWindow(uint processId, uint threadId) {
        if (processId != _clipboardOwnerProcessId || threadId != _clipboardOwnerThreadId) throw new InvalidOperationException("The clipboard owner window may only be used and destroyed by its creating process thread.");
        if (!IsWindow(_clipboardOwnerWindow)) throw new InvalidOperationException("The runner-owned clipboard HWND is no longer a live window.");
        uint windowProcessId = 0;
        uint windowThreadId = GetWindowThreadProcessId(_clipboardOwnerWindow, out windowProcessId);
        if (windowProcessId != _clipboardOwnerProcessId || windowThreadId != _clipboardOwnerThreadId) throw new InvalidOperationException("The clipboard HWND process/thread identity changed.");
        IntPtr messageOnlyMatch = FindWindowExW(HWND_MESSAGE, IntPtr.Zero, "Static", _clipboardOwnerWindowTitle);
        if (messageOnlyMatch != _clipboardOwnerWindow || FindWindowExW(HWND_MESSAGE, messageOnlyMatch, "Static", _clipboardOwnerWindowTitle) != IntPtr.Zero) throw new InvalidOperationException("The clipboard HWND is not the unique message-only STATIC window created by this runner.");
        if (IsWindowVisible(_clipboardOwnerWindow)) throw new InvalidOperationException("The clipboard HWND is visible; the runner-owned owner must remain hidden.");
        var className = new StringBuilder(80);
        if (GetClassNameW(_clipboardOwnerWindow, className, className.Capacity) == 0 || !string.Equals(className.ToString(), "Static", StringComparison.OrdinalIgnoreCase)) throw new InvalidOperationException("The clipboard HWND is not the expected Win32 STATIC window.");
    }

    public static string ClipboardFormatName(uint format) {
        var name = new StringBuilder(128);
        int length = GetClipboardFormatNameW(format, name, name.Capacity);
        return length > 0 ? name.ToString() : "format-" + format;
    }

    public static bool TryCaptureClipboard(out Dictionary<uint, byte[]> contents, out string reason) {
        const ulong MaxBytesPerFormat = 134217728;
        contents = new Dictionary<uint, byte[]>();
        reason = "";
        IntPtr owner = GetClipboardOwnerWindow();
        if (!OpenClipboard(owner)) { reason = "OpenClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
        try {
            uint format = 0;
            while ((format = EnumClipboardFormats(format)) != 0) {
                IntPtr handle = GetClipboardData(format);
                if (handle == IntPtr.Zero) { reason = "GetClipboardData failed for " + ClipboardFormatName(format) + "."; return false; }
                ulong size = GlobalSize(handle).ToUInt64();
                if (size == 0 || size > MaxBytesPerFormat) { reason = "Format cannot be safely snapshotted as bounded HGLOBAL data: " + ClipboardFormatName(format) + "."; return false; }
                IntPtr source = GlobalLock(handle);
                if (source == IntPtr.Zero) { reason = "GlobalLock failed for " + ClipboardFormatName(format) + "."; return false; }
                try {
                    var bytes = new byte[(int)size];
                    Marshal.Copy(source, bytes, 0, bytes.Length);
                    contents.Add(format, bytes);
                }
                finally { GlobalUnlock(handle); }
            }
            return true;
        }
        finally { CloseClipboard(); }
    }

    public static bool TryReadUnicodeClipboardText(uint expectedSequence, out string text, out uint currentSequence, out string reason) {
        const uint CF_UNICODETEXT = 13;
        text = "";
        currentSequence = GetClipboardSequenceNumber();
        reason = "";
        IntPtr owner = GetClipboardOwnerWindow();
        if (!OpenClipboard(owner)) { reason = "OpenClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
        try {
            currentSequence = GetClipboardSequenceNumber();
            if (currentSequence != expectedSequence) { reason = "Clipboard sequence changed before the OS text could be inspected."; return false; }
            IntPtr handle = GetClipboardData(CF_UNICODETEXT);
            if (handle == IntPtr.Zero) { reason = "The OS clipboard has no CF_UNICODETEXT format."; return false; }
            ulong size = GlobalSize(handle).ToUInt64();
            if (size < 2 || size > 134217728 || (size & 1) != 0) { reason = "CF_UNICODETEXT has an invalid or excessive HGLOBAL size."; return false; }
            IntPtr source = GlobalLock(handle);
            if (source == IntPtr.Zero) { reason = "GlobalLock failed for CF_UNICODETEXT."; return false; }
            try {
                var bytes = new byte[(int)size];
                Marshal.Copy(source, bytes, 0, bytes.Length);
                text = Encoding.Unicode.GetString(bytes).TrimEnd('\0');
            }
            finally { GlobalUnlock(handle); }
            return true;
        }
        finally { CloseClipboard(); }
    }

    public static bool TrySetUnicodeClipboardText(string text, uint expectedSequence, out uint currentSequence, out bool clipboardChanged, out string reason) {
        const uint CF_UNICODETEXT = 13;
        const uint GMEM_MOVEABLE_ZEROINIT = 0x0042;
        currentSequence = GetClipboardSequenceNumber();
        clipboardChanged = false;
        reason = "";
        byte[] bytes = Encoding.Unicode.GetBytes(text + "\0");
        IntPtr owner = GetClipboardOwnerWindow();
        IntPtr memory = GlobalAlloc(GMEM_MOVEABLE_ZEROINIT, new UIntPtr((uint)bytes.Length));
        if (memory == IntPtr.Zero) { reason = "GlobalAlloc failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
        try {
            IntPtr target = GlobalLock(memory);
            if (target == IntPtr.Zero) { reason = "GlobalLock failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
            try { Marshal.Copy(bytes, 0, target, bytes.Length); }
            finally { GlobalUnlock(memory); }
            if (!OpenClipboard(owner)) { reason = "OpenClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
            try {
                currentSequence = GetClipboardSequenceNumber();
                if (currentSequence != expectedSequence) { reason = "The clipboard sequence changed before the test text could be written; the current clipboard was left untouched."; return false; }
                if (!EmptyClipboard()) { reason = "EmptyClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                clipboardChanged = true;
                if (SetClipboardData(CF_UNICODETEXT, memory) == IntPtr.Zero) { currentSequence = GetClipboardSequenceNumber(); reason = "SetClipboardData failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                memory = IntPtr.Zero;
                currentSequence = GetClipboardSequenceNumber();
                return true;
            }
            finally { CloseClipboard(); }
        }
        finally { if (memory != IntPtr.Zero) GlobalFree(memory); }
    }

    public static bool TrySetDibClipboard(byte[] dib, uint expectedSequence, out uint currentSequence, out bool clipboardChanged, out string reason) {
        const uint CF_DIB = 8;
        const uint GMEM_MOVEABLE_ZEROINIT = 0x0042;
        currentSequence = GetClipboardSequenceNumber();
        clipboardChanged = false;
        reason = "";
        if (dib == null || dib.Length < 40 || dib.Length > 134217728) { reason = "The bounded CF_DIB fixture is invalid."; return false; }
        IntPtr owner = GetClipboardOwnerWindow();
        IntPtr memory = GlobalAlloc(GMEM_MOVEABLE_ZEROINIT, new UIntPtr((uint)dib.Length));
        if (memory == IntPtr.Zero) { reason = "GlobalAlloc failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
        try {
            IntPtr target = GlobalLock(memory);
            if (target == IntPtr.Zero) { reason = "GlobalLock failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
            try { Marshal.Copy(dib, 0, target, dib.Length); }
            finally { GlobalUnlock(memory); }
            if (!OpenClipboard(owner)) { reason = "OpenClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
            try {
                currentSequence = GetClipboardSequenceNumber();
                if (currentSequence != expectedSequence) { reason = "The clipboard sequence changed before the CF_DIB fixture could be written; the current clipboard was left untouched."; return false; }
                if (!EmptyClipboard()) { reason = "EmptyClipboard failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                clipboardChanged = true;
                if (SetClipboardData(CF_DIB, memory) == IntPtr.Zero) { currentSequence = GetClipboardSequenceNumber(); reason = "SetClipboardData(CF_DIB) failed (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                memory = IntPtr.Zero;
                currentSequence = GetClipboardSequenceNumber();
                return true;
            }
            finally { CloseClipboard(); }
        }
        finally { if (memory != IntPtr.Zero) GlobalFree(memory); }
    }

    public static bool TryRestoreClipboard(Dictionary<uint, byte[]> contents, uint expectedSequence, out uint restoredSequence, out bool sequenceChanged, out string reason) {
        const uint GMEM_MOVEABLE_ZEROINIT = 0x0042;
        var blocks = new Dictionary<uint, IntPtr>();
        var transferred = new HashSet<uint>();
        restoredSequence = GetClipboardSequenceNumber();
        sequenceChanged = false;
        reason = "";
        try {
            foreach (var item in contents) {
                IntPtr memory = GlobalAlloc(GMEM_MOVEABLE_ZEROINIT, new UIntPtr((uint)item.Value.Length));
                if (memory == IntPtr.Zero) { reason = "GlobalAlloc failed while restoring " + ClipboardFormatName(item.Key) + "."; return false; }
                blocks.Add(item.Key, memory);
                IntPtr target = GlobalLock(memory);
                if (target == IntPtr.Zero) { reason = "GlobalLock failed while restoring " + ClipboardFormatName(item.Key) + "."; return false; }
                try { Marshal.Copy(item.Value, 0, target, item.Value.Length); }
                finally { GlobalUnlock(memory); }
            }
            IntPtr owner = GetClipboardOwnerWindow();
            if (!OpenClipboard(owner)) { reason = "OpenClipboard failed while restoring (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
            try {
                restoredSequence = GetClipboardSequenceNumber();
                if (restoredSequence != expectedSequence) { sequenceChanged = true; reason = "The clipboard changed after the test paste; the new clipboard was left untouched."; return false; }
                if (!EmptyClipboard()) { reason = "EmptyClipboard failed while restoring (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                foreach (var item in blocks) {
                    if (SetClipboardData(item.Key, item.Value) == IntPtr.Zero) { reason = "SetClipboardData failed while restoring " + ClipboardFormatName(item.Key) + " (Win32 " + Marshal.GetLastWin32Error() + ")."; return false; }
                    transferred.Add(item.Key);
                }
                restoredSequence = GetClipboardSequenceNumber();
                return true;
            }
            finally { CloseClipboard(); }
        }
        finally { foreach (var item in blocks) if (!transferred.Contains(item.Key) && item.Value != IntPtr.Zero) GlobalFree(item.Value); }
    }
}
'@
Add-Type -TypeDefinition $NativeSource
Add-Type -AssemblyName System.Windows.Forms

$script:Report = [ordered]@{
    task = '20261012-mdlite-rust-editor-foundation / Rev2 GUI lane'
    mode = $Mode
    configuration = $Configuration
    preflight = $RunPreflight
    status = 'NOT_RUN'
    started_utc = [DateTime]::UtcNow.ToString('o')
    executable = [IO.Path]::GetFullPath($ExePath)
    executable_sha256 = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
    product_source_fingerprint_start = $null
    product_source_fingerprint_end = $null
    source_stable_during_run = $null
    executable_sha256_end = $null
    executable_stable_during_run = $null
    invalidations = @()
    cpp_baseline_executable = [IO.Path]::GetFullPath($CppExePath)
    cpp_baseline_expected_sha256 = $CppExpectedSha256
    cpp_baseline_source = [IO.Path]::GetFullPath((Join-Path $ProjectRoot '.local\cpp-baseline'))
    cpp_startup_readiness = @()
    input_delivery = 'Keyboard SendInput targets the launched app; Shell file drags use SendInput only while the fresh Explorer window or app owns foreground. SendMessageTimeout(WM_NULL) is a responsiveness probe.'
    physical_input = 'All keyboard and mouse input is OS-synthetic SendInput; no physical key or human drag is claimed.'
    gui_owner_assignment = 'Exclusive GUI use was assigned to this worker by Root message for this bounded run.'
    formal_runtime_lease = 'UNKNOWN/unavailable; no supported runtime lease command was present.'
    launch_focus = [ordered]@{ activate_by_caption_click = [bool]$ActivateOwnedWindowByClick.IsPresent; attempts = @() }
    actual_active_input_profile = $null
    input_profile = [ordered]@{ original = $null; target = $null; final = $null; trace = @(); selection_cycles = 0; restore_status = 'NOT_RUN'; profile_identity_restore_status = 'NOT_RUN'; restore_cycles = 0; original_ime_open_observation = $null; final_ime_open_observation = $null; ime_open_restore_status = 'NOT_RUN'; pending_composition = $null }
    ime = [ordered]@{ status = 'NOT_RUN'; note = 'Microsoft IME is attempted only after the native active profile identity and its language-profile description confirm Microsoft IME for Japanese.' }
    clipboard = [ordered]@{ status = 'NOT_RUN'; original_sequence = $null; test_sequence = $null; restored_sequence = $null; formats = @() }
    clipboard_owner = [ordered]@{ state = 'NOT_CREATED'; hwnd = $null; process_id = $null; thread_id = $null; window_title = $null; message_only_membership_verified = $null; class_name = $null; hidden = $null; destroyed = $null }
    fixture_fingerprint = $null
    process_runs = @()
    checks = @()
    screenshots = @()
    diagnostic_trace = @()
    r7_timeout_evidence = $null
    animation_trace = @()
    command_pathways = @()
    shell_drop_baseline_hwnds = @()
    shell_drop_target_folder = $null
    shell_drop_launch_executable = $null
    shell_drop_launch_arguments = $null
    shell_drop_cleanup = @()
    notes = @()
}
if ($SkipClipboard) {
    $script:Report.clipboard.status = 'SKIPPED_BY_SWITCH'
    $script:Report.notes = @($script:Report.notes) + 'Clipboard testing was disabled by -SkipClipboard; no clipboard snapshot, write, restore, or OS copy/cut/paste input will be attempted.'
}
$script:RunDirectory = $null
$script:Process = $null
$script:WindowHandle = [IntPtr]::Zero
$script:DiagnosticPath = $null
$script:OwnedExplorerWindows = @()
$script:ExplorerBaselineHandles = @()
$script:ExplorerDropFolder = $null
$script:ClipboardSnapshot = $null
$script:ClipboardCaptured = $false
$script:SkipClipboard = [bool]$SkipClipboard.IsPresent
$script:ActivateOwnedWindowByClick = [bool]$ActivateOwnedWindowByClick.IsPresent
$script:LastOwnedDiagnostic = $null
$script:ClipboardOwnerHandle = [IntPtr]::Zero
$script:ClipboardFormats = @()
$script:ClipboardOriginalSequence = [uint32]0
$script:ClipboardRestoreSequence = [uint32]0
$script:BaseSource = $null
$script:OriginalInputProfile = $null
$script:OriginalImeOpen = $null
$script:LastImeOpenObservation = $null
$script:LastOwnedDiagnosticRefreshUtc = $null
$script:InputProfileStateMayHaveChanged = $false
$script:InputProfileRestoreAttempted = $false

function Add-Check([string]$Id, [string]$Status, [string]$Detail) {
    $script:Report.checks = @($script:Report.checks) + [pscustomobject]@{
        id = $Id
        status = $Status
        detail = $Detail
    }
}

function Assert-ClipboardTestsEnabled([string]$Operation) {
    if ($script:SkipClipboard) {
        $script:Report.clipboard.status = 'SKIPPED_BY_SWITCH'
        throw "$Operation was blocked by -SkipClipboard; no clipboard API or OS copy/cut/paste input was sent."
    }
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

function Add-DiagnosticTrace([string]$Stage, $Diagnostic) {
    $sourceBytes = [Text.Encoding]::UTF8.GetBytes([string]$Diagnostic.source)
    $script:Report.diagnostic_trace = @($script:Report.diagnostic_trace) + [pscustomobject]@{
        stage = $Stage
        source = [string]$Diagnostic.source
        source_utf8_bytes = $sourceBytes.Length
        source_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($sourceBytes)).ToLowerInvariant()
        revision = $Diagnostic.revision
        dirty = $Diagnostic.dirty
        undo_count = $Diagnostic.undo_count
        redo_count = $Diagnostic.redo_count
        selection = $Diagnostic.selection
        raw = $Diagnostic.raw
        tabs = $Diagnostic.tabs
        scroll = $Diagnostic.scroll
        tables = $Diagnostic.tables
        caret = $Diagnostic.caret
        ime = $Diagnostic.ime
    }
}

function New-RunDirectory {
    $null = New-Item -ItemType Directory -Path $RunParent -Force
    do {
        $name = 'run-' + [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss-fff') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 6)
        $candidate = Join-Path $RunParent $name
    } while (Test-Path -LiteralPath $candidate)
    $null = New-Item -ItemType Directory -Path $candidate
    return $candidate
}

function Write-Utf8NoBom([string]$Path, [string]$Text) {
    [IO.File]::WriteAllText($Path, $Text, [Text.UTF8Encoding]::new($false))
}

function New-Fixture([string]$Directory, [string]$Name) {
    $docDir = Join-Path $Directory $Name
    $assetDir = Join-Path $docDir 'assets'
    $null = New-Item -ItemType Directory -Path $assetDir -Force
    $null = New-Item -ItemType Directory -Path (Join-Path $docDir '.mdlite') -Force
    foreach ($assetName in @('fixture.svg', 'raster.png', 'raster.jpg', 'raster.gif', 'raster.webp', 'animated.gif', 'animated.webp')) {
        $from = Join-Path (Join-Path $RuntimeFixture 'assets') $assetName
        if (-not (Test-Path -LiteralPath $from -PathType Leaf)) { throw "Required tracked fixture is missing: $from" }
        Copy-Item -LiteralPath $from -Destination (Join-Path $assetDir $assetName)
    }
    [IO.File]::WriteAllBytes((Join-Path $assetDir 'broken.png'), [byte[]](0x89, 0x50, 0x4e, 0x47, 0x00, 0x01))

    $source = @'
---
title: 架空のRust GUI受入試料
---

# 編集面の確認

日本語本文と **強調**、`code`、絵文字 👩‍💻 を含む短い試料です。

| 項目 | メモ |
| :--- | ---: |
| 表 | 同じ編集面で操作 |
| 画像 | 幅変更と前後の入力 |

### 二つ目の表

| 種別 | 値 |
| --- | --- |
| source | 不変性 |

### 表単位の横スクロール

| c01 | c02 | c03 | c04 | c05 | c06 | c07 | c08 | c09 | c10 | c11 | c12 | c13 | c14 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| a | b | c | d | e | f | g | h | i | j | k | l | m | n |

### Header-only boundary

| only1 | only2 |
| --- | --- |

## 5形式の画像

画像の直前にある文章です。

![PNG](assets/raster.png)

PNG直後の文章です。

![JPEG](assets/raster.jpg)

![GIF](assets/raster.gif)

![WebP](assets/raster.webp)

![Animated GIF](assets/animated.gif)

![Animated WebP](assets/animated.webp)

![SVG](assets/fixture.svg)

![不正PNG](assets/broken.png)

末尾の入力確認
'@
    $docPath = Join-Path $docDir ($Name + '.md')
    Write-Utf8NoBom $docPath $source
    [pscustomobject]@{ Path = $docPath; Directory = $docDir; WorkspaceStateDirectory = (Join-Path $docDir '.mdlite'); Source = $source }
}

function Get-FixtureFingerprint([string]$Directory) {
    $files = @(Get-ChildItem -LiteralPath $Directory -File -Recurse | Sort-Object FullName)
    $items = foreach ($file in $files) {
        [pscustomobject]@{
            path = $file.FullName.Substring($Directory.Length).TrimStart('\')
            bytes = $file.Length
            sha256 = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        }
    }
    $text = ($items | ForEach-Object { "$($_.path)`t$($_.bytes)`t$($_.sha256)" }) -join "`n"
    $sha = [Security.Cryptography.SHA256]::HashData([Text.Encoding]::UTF8.GetBytes($text))
    [pscustomobject]@{ sha256 = ([Convert]::ToHexString($sha).ToLowerInvariant()); files = @($items) }
}

function Get-ProductSourceFingerprint {
    [string[]]$paths = @(
        Get-ChildItem -LiteralPath (Join-Path $ProjectRoot 'src') -File -Recurse | Select-Object -ExpandProperty FullName
        Join-Path $ProjectRoot 'Cargo.toml'
        Join-Path $ProjectRoot 'Cargo.lock'
        Join-Path $ProjectRoot 'rust-toolchain.toml'
    )
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $lines = foreach ($path in $paths) {
        $relative = [IO.Path]::GetFullPath($path).Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
        "$relative`t$((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant())"
    }
    $content = [Text.Encoding]::UTF8.GetBytes(($lines -join "`n"))
    [pscustomobject]@{
        sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($content)).ToLowerInvariant()
        file_count = $paths.Count
    }
}

function Get-ForegroundSnapshot {
    $window = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$ownerPid = 0
    $processName = $null
    if ($window -ne [IntPtr]::Zero) {
        [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$ownerPid)
        if ($ownerPid -ne 0) { try { $processName = (Get-Process -Id ([int]$ownerPid) -ErrorAction Stop).ProcessName } catch { } }
    }
    [pscustomobject]@{
        window_handle = if ($window -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $window.ToInt64() }
        process_id = if ($ownerPid -eq 0) { $null } else { $ownerPid }
        process_name = $processName
        is_launched_window = ($null -ne $script:Process -and $window -eq $script:WindowHandle -and $ownerPid -eq [uint32]$script:Process.Id)
    }
}

function Get-R7WindowEvidence($Diagnostic) {
    $mainPid = $null
    $mainValid = $false
    $mainVisible = $false
    $mainEnabled = $false
    $evidenceErrors = [Collections.Generic.List[string]]::new()
    if ($null -ne $script:Process -and $script:WindowHandle -ne [IntPtr]::Zero) {
        try {
            $script:Process.Refresh()
            $mainValid = -not $script:Process.HasExited -and [MdliteRustGuiNative]::IsWindow($script:WindowHandle)
            [uint32]$ownerPid = 0
            [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$ownerPid)
            $mainPid = if ($ownerPid -eq 0) { $null } else { $ownerPid }
            $mainVisible = [MdliteRustGuiNative]::IsWindowVisible($script:WindowHandle)
            $mainEnabled = [MdliteRustGuiNative]::IsWindowEnabled($script:WindowHandle)
        }
        catch { $evidenceErrors.Add('launched_main_window: ' + $_.Exception.Message) }
    }
    $foreground = $null
    try { $foreground = Get-ForegroundSnapshot } catch { $evidenceErrors.Add('foreground: ' + $_.Exception.Message) }
    $visibleWindows = @()
    if ($null -ne $script:Process) {
        try {
            $script:OwnedVisibleWindows = [Collections.Generic.List[object]]::new()
            $callback = [MdliteRustGuiNative+EnumWindowsProc]{
                param([IntPtr]$window, [IntPtr]$data)
                [uint32]$windowPid = 0
                [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$windowPid)
                if ($windowPid -ne [uint32]$data.ToInt64() -or -not [MdliteRustGuiNative]::IsWindowVisible($window)) { return $true }
                $className = [Text.StringBuilder]::new(80)
                $classLength = [MdliteRustGuiNative]::GetClassNameW($window, $className, $className.Capacity)
                $ownerWindow = [MdliteRustGuiNative]::GetWindow($window, [MdliteRustGuiNative]::GW_OWNER)
                [uint32]$ownerPid = 0
                if ($ownerWindow -ne [IntPtr]::Zero) { [void][MdliteRustGuiNative]::GetWindowThreadProcessId($ownerWindow, [ref]$ownerPid) }
                $script:OwnedVisibleWindows.Add([pscustomobject]@{
                    hwnd = '0x{0:X}' -f $window.ToInt64()
                    process_id = $windowPid
                    class_name = if ($classLength -gt 0) { $className.ToString() } else { $null }
                    owner_hwnd = if ($ownerWindow -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $ownerWindow.ToInt64() }
                    owner_pid = if ($ownerPid -eq 0) { $null } else { $ownerPid }
                    visible = $true
                    enabled = [MdliteRustGuiNative]::IsWindowEnabled($window)
                    is_launched_main = ($window -eq $script:WindowHandle)
                })
                return $true
            }
            [void][MdliteRustGuiNative]::EnumWindows($callback, [IntPtr]$script:Process.Id)
            $visibleWindows = @($script:OwnedVisibleWindows.ToArray())
        }
        catch { $evidenceErrors.Add('owned_window_inventory: ' + $_.Exception.Message) }
    }
    $documentCounters = if ($null -eq $Diagnostic) { [pscustomobject]@{ available = $false; tabs = $null; dirty = $null; revision = $null; undo_count = $null; redo_count = $null } } else { [pscustomobject]@{ available = $true; tabs = $Diagnostic.tabs; dirty = $Diagnostic.dirty; revision = $Diagnostic.revision; undo_count = $Diagnostic.undo_count; redo_count = $Diagnostic.redo_count } }
    return [pscustomobject]@{
        captured_utc = [DateTime]::UtcNow.ToString('o')
        foreground = $foreground
        launched_main_window = [pscustomobject]@{
            hwnd = if ($script:WindowHandle -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $script:WindowHandle.ToInt64() }
            expected_pid = if ($null -eq $script:Process) { $null } else { $script:Process.Id }
            actual_pid = $mainPid
            valid = $mainValid
            visible = $mainVisible
            enabled = $mainEnabled
        }
        visible_owned_windows = $visibleWindows
        document_counters = $documentCounters
        evidence_errors = @($evidenceErrors)
    }
}

function Get-R7TimeoutEvidence($BeforeWindowEvidence, [int]$DialogWaitMs) {
    $afterDiagnostic = $null
    $diagnosticError = $null
    try {
        $null = Get-ActiveInputProfile
        $afterDiagnostic = $script:LastOwnedDiagnostic
        if ($null -eq $afterDiagnostic) { $diagnosticError = 'No fresh owned diagnostic was published after the Ctrl+W timeout.' }
    }
    catch { $diagnosticError = $_.Exception.Message; $afterDiagnostic = $script:LastOwnedDiagnostic }
    $afterWindowEvidence = $null
    try { $afterWindowEvidence = Get-R7WindowEvidence $afterDiagnostic }
    catch { $diagnosticError = if ($null -eq $diagnosticError) { $_.Exception.Message } else { $diagnosticError + '; ' + $_.Exception.Message } }
    return [pscustomobject]@{
        status = 'CAPTURED_AFTER_NO_OWNED_DIALOG'
        dialog_wait_ms = $DialogWaitMs
        pre_ctrl_w = $BeforeWindowEvidence
        post_ctrl_w = $afterWindowEvidence
        post_diagnostic_fresh = ($null -ne $afterDiagnostic)
        post_diagnostic_error = $diagnosticError
    }
}

function Assert-OwnedWindowIdentity([Diagnostics.Process]$Process, [IntPtr]$Window) {
    if ($null -eq $Process) { throw 'No launched process is available for owned-window activation.' }
    $Process.Refresh()
    if ($Process.HasExited) { throw "The launched app exited before owned-window activation with code $($Process.ExitCode)." }
    if ($Window -eq [IntPtr]::Zero -or $Process.MainWindowHandle -ne $Window -or -not [MdliteRustGuiNative]::IsWindow($Window)) { throw 'The exact launched main HWND is unavailable for owned-window activation.' }
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($Window, [ref]$ownerPid)
    if ($ownerPid -ne [uint32]$Process.Id) { throw "Launched HWND owner PID $ownerPid does not match PID $($Process.Id)." }
}

function Get-DiagnosticStateSummary($Diagnostic) {
    $sourceBytes = [Text.Encoding]::UTF8.GetBytes([string]$Diagnostic.source)
    return [pscustomobject]@{
        source_sha256 = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($sourceBytes)).ToLowerInvariant()
        revision = $Diagnostic.revision
        dirty = $Diagnostic.dirty
        undo_count = $Diagnostic.undo_count
        redo_count = $Diagnostic.redo_count
        selection = $Diagnostic.selection
    }
}

function Invoke-OwnedWindowCaptionClick($FocusRecord, [string]$DiagnosticPath) {
    $fallback = [ordered]@{
        status = 'PENDING'
        window_rect = $null
        caption_point = $null
        window_from_point = $null
        root_from_point = $null
        point_owner_pid = $null
        hit_test_result = $null
        mouse_down_attempted = $false
        mouse_down_sent = $false
        mouse_up_sent = $false
        foreground_after = $null
        source_before = $null
        source_after = $null
        source_invariants = if ([string]::IsNullOrWhiteSpace($DiagnosticPath)) { 'NOT_APPLICABLE_NO_DIAGNOSTIC' } else { 'NOT_RUN' }
        failure = $null
    }
    $FocusRecord.caption_fallback = $fallback
    $process = $script:Process
    $window = $script:WindowHandle
    $beforeDiagnostic = $null
    try {
        Assert-OwnedWindowIdentity $process $window
        if (-not [MdliteRustGuiNative]::IsWindowVisible($window) -or [MdliteRustGuiNative]::IsIconic($window)) { throw 'Caption activation refused: the exact launched window is hidden or minimized.' }
        if (-not [string]::IsNullOrWhiteSpace($DiagnosticPath)) {
            $null = Get-ActiveInputProfile
            $beforeDiagnostic = $script:LastOwnedDiagnostic
            if ($null -eq $beforeDiagnostic) { throw 'Caption activation refused because a fresh pre-click app diagnostic was unavailable.' }
            $fallback.source_before = Get-DiagnosticStateSummary $beforeDiagnostic
        }

        $rect = [MdliteRustGuiNative+Rect]::new()
        if (-not [MdliteRustGuiNative]::GetWindowRect($window, [ref]$rect)) { throw ('GetWindowRect failed before caption activation (Win32 ' + [Runtime.InteropServices.Marshal]::GetLastWin32Error() + ').') }
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        if ($width -lt 40 -or $height -lt 24) { throw 'Caption activation refused because the launched window rectangle is too small.' }
        $x = $rect.Left + [int][Math]::Floor($width / 2)
        $y = $rect.Top + 10
        $fallback.window_rect = [pscustomobject]@{ left = $rect.Left; top = $rect.Top; right = $rect.Right; bottom = $rect.Bottom; width = $width; height = $height }
        $fallback.caption_point = [pscustomobject]@{ x = $x; y = $y }
        if (-not [MdliteRustGuiNative]::SetCursorPos($x, $y)) { throw ('SetCursorPos failed before caption hit testing (Win32 ' + [Runtime.InteropServices.Marshal]::GetLastWin32Error() + ').') }

        $point = [MdliteRustGuiNative+Point]::new()
        $point.X = $x
        $point.Y = $y
        $pointWindow = [MdliteRustGuiNative]::WindowFromPoint($point)
        $pointRoot = if ($pointWindow -eq [IntPtr]::Zero) { [IntPtr]::Zero } else { [MdliteRustGuiNative]::GetAncestor($pointWindow, [MdliteRustGuiNative]::GA_ROOT) }
        [uint32]$pointOwnerPid = 0
        if ($pointRoot -ne [IntPtr]::Zero) { [void][MdliteRustGuiNative]::GetWindowThreadProcessId($pointRoot, [ref]$pointOwnerPid) }
        $fallback.window_from_point = if ($pointWindow -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $pointWindow.ToInt64() }
        $fallback.root_from_point = if ($pointRoot -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $pointRoot.ToInt64() }
        $fallback.point_owner_pid = if ($pointOwnerPid -eq 0) { $null } else { $pointOwnerPid }
        if ($pointRoot -ne $window -or $pointOwnerPid -ne [uint32]$process.Id) { throw 'Caption activation refused because another window covers the selected point or its PID differs.' }

        $hitResult = [IntPtr]::Zero
        $hitSent = [MdliteRustGuiNative]::SendMessageTimeoutW($window, [MdliteRustGuiNative]::WM_NCHITTEST, [IntPtr]::Zero, [MdliteRustGuiNative]::ScreenPointLParam($x, $y), [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, 250, [ref]$hitResult)
        $fallback.hit_test_result = if ($hitSent -eq [IntPtr]::Zero) { 'TIMEOUT_OR_ERROR' } else { $hitResult.ToInt32() }
        if ($hitSent -eq [IntPtr]::Zero -or $hitResult.ToInt32() -ne [MdliteRustGuiNative]::HTCAPTION) { throw 'Caption activation refused because the bounded WM_NCHITTEST did not return HTCAPTION.' }

        $mouseDown = [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTDOWN)
        $inputSize = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
        $fallback.mouse_down_attempted = $true
        $downSent = [MdliteRustGuiNative]::SendInput(1, [MdliteRustGuiNative+Input[]]@($mouseDown), $inputSize)
        $fallback.mouse_down_sent = ($downSent -eq 1)
        if ($downSent -ne 1) { throw ('Caption mouse-down delivered {0} of 1 events (Win32 {1}).' -f $downSent, [Runtime.InteropServices.Marshal]::GetLastWin32Error()) }
    }
    catch {
        $fallback.failure = $_.Exception.Message
        throw
    }
    finally {
        if ($fallback.mouse_down_attempted) {
            try {
                $mouseUp = [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTUP)
                $inputSize = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
                $upSent = [MdliteRustGuiNative]::SendInput(1, [MdliteRustGuiNative+Input[]]@($mouseUp), $inputSize)
                $fallback.mouse_up_sent = ($upSent -eq 1)
                if ($upSent -ne 1) { throw ('Caption mouse-up after a down attempt delivered {0} of 1 events (Win32 {1}).' -f $upSent, [Runtime.InteropServices.Marshal]::GetLastWin32Error()) }
            }
            catch {
                $fallback.mouse_up_sent = $false
                $fallback.failure = $_.Exception.Message
                $fallback.status = 'FAIL'
                throw
            }
        }
    }

    Assert-OwnedWindowIdentity $process $window
    $foreground = Get-ForegroundSnapshot
    $fallback.foreground_after = $foreground
    if (-not $foreground.is_launched_window) { throw 'Caption click did not make the exact launched HWND/PID foreground before any further input.' }
    if (-not [string]::IsNullOrWhiteSpace($DiagnosticPath)) {
        $null = Get-ActiveInputProfile
        $afterDiagnostic = $script:LastOwnedDiagnostic
        if ($null -eq $afterDiagnostic) { throw 'Caption click succeeded, but a fresh post-click app diagnostic was unavailable.' }
        $fallback.source_after = Get-DiagnosticStateSummary $afterDiagnostic
        Compare-SourceHistorySelection $beforeDiagnostic $afterDiagnostic 'Owned caption activation click'
        $fallback.source_invariants = 'PASS'
    }
    $fallback.status = 'PASS'
    return $foreground
}

function Start-ProductApp([string]$Executable, [string[]]$Documents, [string]$Diagnostic = '') {
    $focusRecord = [ordered]@{
        status = 'STARTING'
        executable = [IO.Path]::GetFullPath($Executable)
        executable_sha256 = $null
        diagnostic = $Diagnostic
        prelaunch_foreground = Get-ForegroundSnapshot
        launched_pid = $null
        launched_window_handle = $null
        activation = $null
        plain_focus_error = $null
        foreground_after_plain_failure = $null
        focus_status = 'NOT_RUN'
        foreground_after_acquisition = $null
        caption_fallback = if ($script:ActivateOwnedWindowByClick) { [ordered]@{ status = 'NOT_RUN'; failure = $null } } else { [ordered]@{ status = 'NOT_REQUESTED'; failure = $null } }
        failure = $null
        failing_foreground = $null
        process_still_running_at_failure = $null
    }
    $script:Report.launch_focus.attempts = @($script:Report.launch_focus.attempts) + $focusRecord
    $process = $null
    try {
        $executableHash = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        $focusRecord.executable_sha256 = $executableHash
        if ($Executable -eq $ExePath) { Assert-True ($executableHash -eq $script:Report.executable_sha256) 'The Rust executable changed after the run-start hash was recorded.' }
        if ($Executable -eq $CppExePath) { Assert-True ($executableHash -eq $CppExpectedSha256) 'The C++ executable no longer matches the known release baseline hash.' }
        $arguments = @()
        if (-not [string]::IsNullOrWhiteSpace($Diagnostic)) { $arguments += @('--diagnostic-path', $Diagnostic) }
        $arguments += $Documents
        $testEnvironmentOverrides = [ordered]@{}
        $info = [Diagnostics.ProcessStartInfo]::new()
        $info.FileName = $Executable
        $info.WorkingDirectory = $ProjectRoot
        foreach ($argument in $arguments) { $info.ArgumentList.Add([string]$argument) }
        $info.UseShellExecute = $false
        $info.CreateNoWindow = $false
        if ($Executable -eq $CppExePath) {
            $info.EnvironmentVariables['MDLITE_TEST_NO_CRASH_UI'] = '1'
            $info.EnvironmentVariables['MDLITE_TEST_SILENT'] = '1'
            $testEnvironmentOverrides['MDLITE_TEST_NO_CRASH_UI'] = '1'
            $testEnvironmentOverrides['MDLITE_TEST_SILENT'] = '1'
        }
        $process = [Diagnostics.Process]::new()
        $process.StartInfo = $info
        if (-not $process.Start()) { throw "Failed to start $Executable" }
        $focusRecord.launched_pid = $process.Id
        try { $process.WaitForInputIdle(5000) | Out-Null } catch { $focusRecord.input_idle_error = $_.Exception.Message }
        $until = [DateTime]::UtcNow.AddSeconds(10)
        do {
            $process.Refresh()
            if ($process.HasExited) { throw "MDLite exited during startup with code $($process.ExitCode)." }
            if ($process.MainWindowHandle -ne [IntPtr]::Zero) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $until)
        $process.Refresh()
        if ($process.MainWindowHandle -eq [IntPtr]::Zero) { throw 'MDLite did not expose its main window within 10 seconds.' }
        $script:Process = $process
        $script:WindowHandle = $process.MainWindowHandle
        $script:DiagnosticPath = if ([string]::IsNullOrWhiteSpace($Diagnostic)) { $null } else { $Diagnostic }
        $focusRecord.launched_window_handle = '0x{0:X}' -f $script:WindowHandle.ToInt64()
        Assert-OwnedWindowIdentity $process $script:WindowHandle
        $bringToTop = [MdliteRustGuiNative]::BringWindowToTop($script:WindowHandle)
        $bringToTopError = if ($bringToTop) { 0 } else { [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
        $setForeground = [MdliteRustGuiNative]::SetForegroundWindow($script:WindowHandle)
        $setForegroundError = if ($setForeground) { 0 } else { [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
        $focusRecord.activation = [ordered]@{
            bring_window_to_top = [bool]$bringToTop
            bring_window_to_top_win32_error = $bringToTopError
            set_foreground_window = [bool]$setForeground
            set_foreground_window_win32_error = $setForegroundError
            foreground_after_direct_calls = Get-ForegroundSnapshot
        }
        try {
            Wait-Foreground
            $focusRecord.focus_status = 'ACQUIRED_BY_PLAIN_FOREGROUND_API'
        }
        catch {
            $focusRecord.plain_focus_error = $_.Exception.Message
            $focusRecord.foreground_after_plain_failure = Get-ForegroundSnapshot
            if (-not $script:ActivateOwnedWindowByClick) { throw }
            try {
                $null = Invoke-OwnedWindowCaptionClick $focusRecord $script:DiagnosticPath
                $focusRecord.focus_status = 'ACQUIRED_BY_OWNED_CAPTION_CLICK'
            }
            catch {
                $focusRecord.caption_fallback.failure = $_.Exception.Message
                if ($focusRecord.caption_fallback.status -eq 'PENDING') { $focusRecord.caption_fallback.status = 'FAIL' }
                throw
            }
        }
        $focusRecord.foreground_after_acquisition = Get-ForegroundSnapshot
        if (-not $focusRecord.foreground_after_acquisition.is_launched_window) { throw 'The launched window was not the exact foreground HWND/PID after focus acquisition.' }
        if (-not [string]::IsNullOrWhiteSpace($script:DiagnosticPath)) {
            $deadline = [DateTime]::UtcNow.AddSeconds(10)
            $diagnosticState = $null
            do {
                try { $diagnosticState = Get-Diagnostic } catch { Start-Sleep -Milliseconds 100 }
                if ($null -ne $diagnosticState) { break }
                Start-Sleep -Milliseconds 100
            } while ([DateTime]::UtcNow -lt $deadline)
            if ($null -eq $diagnosticState) { throw 'The native app did not publish its diagnostic JSON within 10 seconds.' }
        }
        $focusRecord.status = 'STARTED'
        $focusRecord.started_utc = [DateTime]::UtcNow.ToString('o')
        [pscustomobject]@{
            pid = $process.Id
            window_handle = ('0x{0:X}' -f $script:WindowHandle.ToInt64())
            title = $process.MainWindowTitle
            executable = [IO.Path]::GetFullPath($process.Path)
            executable_sha256 = $executableHash
            diagnostic = $Diagnostic
            test_environment_overrides = $testEnvironmentOverrides
            font_feature_overrides = 'None set by the harness; application and system defaults apply.'
        }
    }
    catch {
        $focusRecord.status = 'FAIL'
        $focusRecord.failure = $_.Exception.Message
        $focusRecord.failing_foreground = Get-ForegroundSnapshot
        if ($null -ne $process) {
            try {
                $process.Refresh()
                $focusRecord.launched_pid = $process.Id
                $focusRecord.process_still_running_at_failure = -not $process.HasExited
                if ($script:WindowHandle -ne [IntPtr]::Zero) { $focusRecord.launched_window_handle = '0x{0:X}' -f $script:WindowHandle.ToInt64() }
            }
            catch { $focusRecord.process_state_error = $_.Exception.Message }
        }
        if ($focusRecord.caption_fallback.status -eq 'PENDING') { $focusRecord.caption_fallback.status = 'FAIL' }
        throw
    }
}

function Start-RustApp([string[]]$Documents, [string]$Diagnostic) {
    Start-ProductApp $ExePath $Documents $Diagnostic
}

function Get-Diagnostic {
    if (-not (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf)) { return $null }
    try { return Get-Content -LiteralPath $script:DiagnosticPath -Raw | ConvertFrom-Json } catch { return $null }
}

function Wait-Diagnostic([scriptblock]$Predicate, [int]$TimeoutMs = 5000, [string]$Description = 'diagnostic condition') {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-AppAlive
        $state = Get-Diagnostic
        if ($null -ne $state -and (& $Predicate $state)) { return $state }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Description."
}

function Assert-AppAlive {
    if ($null -eq $script:Process) { throw 'No launched MDLite process is registered.' }
    $script:Process.Refresh()
    if ($script:Process.HasExited) { throw "MDLite exited unexpectedly with code $($script:Process.ExitCode)." }
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$ownerPid)
    if ($ownerPid -ne [uint32]$script:Process.Id) { throw "Window handle ownership changed to PID $ownerPid; refusing cross-process UI access." }
    $result = [IntPtr]::Zero
    $sent = [MdliteRustGuiNative]::SendMessageTimeoutW($script:WindowHandle, [MdliteRustGuiNative]::WM_NULL, [IntPtr]::Zero, [IntPtr]::Zero, [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, 750, [ref]$result)
    if ($sent -eq [IntPtr]::Zero) { throw 'The launched MDLite window did not answer the bounded WM_NULL responsiveness probe.' }
}

function Wait-Foreground {
    $deadline = [DateTime]::UtcNow.AddSeconds(2)
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([MdliteRustGuiNative]::GetForegroundWindow() -eq $script:WindowHandle) { return }
        [void][MdliteRustGuiNative]::SetForegroundWindow($script:WindowHandle)
        Start-Sleep -Milliseconds 80
    }
    throw 'The launched MDLite window could not retain foreground focus; no input was sent.'
}

function Assert-Foreground {
    Assert-AppAlive
    $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$ownerPid)
    if ($foreground -eq [IntPtr]::Zero -or $ownerPid -ne [uint32]$script:Process.Id) { throw 'Foreground ownership changed; refusing to send input to another process.' }
}

function Send-Inputs([MdliteRustGuiNative+Input[]]$Inputs) {
    Assert-Foreground
    $size = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
    $sent = [MdliteRustGuiNative]::SendInput([uint32]$Inputs.Length, $Inputs, $size)
    if ($sent -ne $Inputs.Length) {
        $errorCode = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "SendInput delivered $sent of $($Inputs.Length) events (Win32 error $errorCode)."
    }
}

function Send-Key([int]$VirtualKey, [int[]]$Modifiers = @()) {
    $clipboardShortcut = ($Modifiers -contains 0x11 -and $VirtualKey -in @(0x43, 0x56, 0x58)) -or ($VirtualKey -eq 0x2D -and (($Modifiers -contains 0x11) -or ($Modifiers -contains 0x10))) -or ($VirtualKey -eq 0x2E -and $Modifiers -contains 0x10)
    if ($script:SkipClipboard -and $clipboardShortcut) { Assert-ClipboardTestsEnabled 'Clipboard shortcut input' }
    $events = [Collections.Generic.List[MdliteRustGuiNative+Input]]::new()
    foreach ($modifier in $Modifiers) { $events.Add([MdliteRustGuiNative]::Key([uint16]$modifier, 0)) }
    $events.Add([MdliteRustGuiNative]::Key([uint16]$VirtualKey, 0))
    $events.Add([MdliteRustGuiNative]::Key([uint16]$VirtualKey, [MdliteRustGuiNative]::KEYEVENTF_KEYUP))
    for ($i = $Modifiers.Length - 1; $i -ge 0; $i--) { $events.Add([MdliteRustGuiNative]::Key([uint16]$Modifiers[$i], [MdliteRustGuiNative]::KEYEVENTF_KEYUP)) }
    Send-Inputs $events.ToArray()
}

function Send-UnicodeText([string]$Text) {
    $events = [Collections.Generic.List[MdliteRustGuiNative+Input]]::new()
    foreach ($unit in $Text.ToCharArray()) {
        $code = [uint16][char]$unit
        $events.Add([MdliteRustGuiNative]::Unicode($code, 0))
        $events.Add([MdliteRustGuiNative]::Unicode($code, [MdliteRustGuiNative]::KEYEVENTF_KEYUP))
    }
    if ($events.Count -gt 0) { Send-Inputs $events.ToArray() }
}

function Hold-Key([int]$VirtualKey, [bool]$KeyUp) {
    $flags = if ($KeyUp) { [MdliteRustGuiNative]::KEYEVENTF_KEYUP } else { 0 }
    Send-Inputs ([MdliteRustGuiNative]::Key([uint16]$VirtualKey, [uint32]$flags))
}

function Send-Wheel([int]$Delta, [switch]$Horizontal) {
    Assert-Foreground
    $windowRect = [MdliteRustGuiNative+Rect]::new()
    if (-not [MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$windowRect)) { throw 'GetWindowRect failed before SendInput wheel delivery.' }
    $screenX = $windowRect.Left + [int](($windowRect.Right - $windowRect.Left) / 2)
    $screenY = $windowRect.Top + [int](($windowRect.Bottom - $windowRect.Top) / 2)
    if (-not [MdliteRustGuiNative]::SetCursorPos($screenX, $screenY)) { throw 'SetCursorPos failed before SendInput wheel delivery.' }
    if ($Horizontal) {
        Send-Inputs ([MdliteRustGuiNative]::Key(0x10, 0))
        Start-Sleep -Milliseconds 25
        Send-Inputs ([MdliteRustGuiNative]::Wheel($Delta))
        Start-Sleep -Milliseconds 25
        Send-Inputs ([MdliteRustGuiNative]::Key(0x10, [MdliteRustGuiNative]::KEYEVENTF_KEYUP))
    }
    else { Send-Inputs ([MdliteRustGuiNative]::Wheel($Delta)) }
}

function Click-ClientDip([double]$X, [double]$Y) {
    Assert-Foreground
    $scale = [double][MdliteRustGuiNative]::GetDpiForWindow($script:WindowHandle) / 96.0
    $point = [MdliteRustGuiNative+Point]::new()
    $point.X = [int][Math]::Round($X * $scale)
    $point.Y = [int][Math]::Round($Y * $scale)
    if (-not [MdliteRustGuiNative]::ClientToScreen($script:WindowHandle, [ref]$point)) { throw 'ClientToScreen failed for table cell geometry.' }
    if (-not [MdliteRustGuiNative]::SetCursorPos($point.X, $point.Y)) { throw 'SetCursorPos failed for the table cell hit-test.' }
    Send-Inputs @([MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTDOWN), [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTUP))
}

function RightClick-ClientDip([double]$X, [double]$Y) {
    Assert-Foreground
    $scale = [double][MdliteRustGuiNative]::GetDpiForWindow($script:WindowHandle) / 96.0
    $point = [MdliteRustGuiNative+Point]::new()
    $point.X = [int][Math]::Round($X * $scale)
    $point.Y = [int][Math]::Round($Y * $scale)
    if (-not [MdliteRustGuiNative]::ClientToScreen($script:WindowHandle, [ref]$point)) { throw 'ClientToScreen failed for a context-menu item.' }
    if (-not [MdliteRustGuiNative]::SetCursorPos($point.X, $point.Y)) { throw 'SetCursorPos failed for a context-menu item.' }
    Send-Inputs @([MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_RIGHTDOWN), [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_RIGHTUP))
}

function Wait-OwnedMenu([int]$TimeoutMs = 1000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $script:FoundMenu = [IntPtr]::Zero
        $callback = [MdliteRustGuiNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$data)
            [uint32]$ownerProcessId = 0
            [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
            if ($ownerProcessId -eq [uint32]$script:Process.Id -and [MdliteRustGuiNative]::IsWindowVisible($window)) {
                $className = [Text.StringBuilder]::new(80)
                [void][MdliteRustGuiNative]::GetClassNameW($window, $className, $className.Capacity)
                if ($className.ToString() -eq '#32768') { $script:FoundMenu = $window; return $false }
            }
            return $true
        }
        [void][MdliteRustGuiNative]::EnumWindows($callback, [IntPtr]::Zero)
        if ($script:FoundMenu -ne [IntPtr]::Zero) { return $script:FoundMenu }
        Start-Sleep -Milliseconds 30
    } while ([DateTime]::UtcNow -lt $deadline)
    return [IntPtr]::Zero
}

function Invoke-ContextMenuCommand([double]$X, [double]$Y, [int]$CommandId, [string]$Label) {
    $firstCommand = if ($CommandId -ge 27) { 27 } else { 20 }
    $offset = $CommandId - $firstCommand
    $itemCount = if ($CommandId -ge 27) { 2 } else { 7 }
    Assert-True ($offset -ge 0 -and $offset -lt $itemCount) "Unsupported native context-menu command $CommandId."
    RightClick-ClientDip $X $Y
    $menu = Wait-OwnedMenu 1000
    Assert-True ($menu -ne [IntPtr]::Zero) "The product did not expose its owned popup menu for $Label."
    [uint32]$menuOwnerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($menu, [ref]$menuOwnerPid)
    Assert-True ($menuOwnerPid -eq [uint32]$script:Process.Id) "The popup menu for $Label is no longer owned by the launched process."
    $menuRect = [MdliteRustGuiNative+Rect]::new()
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($menu, [ref]$menuRect)) "Could not read the popup menu geometry for $Label."
    $menuWidth = $menuRect.Right - $menuRect.Left
    $menuHeight = $menuRect.Bottom - $menuRect.Top
    Assert-True ($menuWidth -gt 0 -and $menuHeight -ge ($itemCount * 8)) "The popup menu geometry for $Label is invalid."
    $itemHeight = [double]$menuHeight / $itemCount
    $targetX = $menuRect.Left + [int][Math]::Floor($menuWidth / 2.0)
    $targetY = $menuRect.Top + [int][Math]::Floor(($offset + 0.5) * $itemHeight)
    Assert-True ($targetX -ge $menuRect.Left -and $targetX -lt $menuRect.Right -and $targetY -ge $menuRect.Top -and $targetY -lt $menuRect.Bottom) "The calculated menu item point for $Label is outside the owned popup."
    if (-not [MdliteRustGuiNative]::SetCursorPos($targetX, $targetY)) { throw "SetCursorPos failed for the popup item in $Label." }
    Start-Sleep -Milliseconds 40
    Send-Inputs @([MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTDOWN), [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTUP))
    $menuDeadline = [DateTime]::UtcNow.AddMilliseconds(1000)
    while ([MdliteRustGuiNative]::IsWindowVisible($menu) -and [DateTime]::UtcNow -lt $menuDeadline) { Start-Sleep -Milliseconds 25 }
    Assert-True (-not [MdliteRustGuiNative]::IsWindowVisible($menu)) "The popup for $Label remained visible after the target mouse click."
    $pathway = [pscustomobject]@{
        operation = $Label
        command_id = $CommandId
        popup_window = ('0x{0:X}' -f $menu.ToInt64())
        menu_bounds = [pscustomobject]@{ left = $menuRect.Left; top = $menuRect.Top; right = $menuRect.Right; bottom = $menuRect.Bottom }
        menu_item_count = $itemCount
        menu_item_index = $offset
        input_point = [pscustomobject]@{ x = $targetX; y = $targetY }
        dispatch = 'SendInput right-click + mouse click inside the owned native popup; the product TrackPopupMenu return is handled by table_command.'
    }
    $script:Report.command_pathways = @($script:Report.command_pathways) + $pathway
    return $pathway
}

function Insert-Utf8Byte([string]$Source, [int]$Offset, [byte]$Value) {
    $originalBytes = [Text.Encoding]::UTF8.GetBytes($Source)
    Assert-True ($Offset -ge 0 -and $Offset -le $originalBytes.Length) 'The diagnostic table-cell byte offset is outside the UTF-8 source.'
    $changed = [byte[]]::new($originalBytes.Length + 1)
    if ($Offset -gt 0) { [Array]::Copy($originalBytes, 0, $changed, 0, $Offset) }
    $changed[$Offset] = $Value
    if ($Offset -lt $originalBytes.Length) { [Array]::Copy($originalBytes, $Offset, $changed, $Offset + 1, $originalBytes.Length - $Offset) }
    return [Text.Encoding]::UTF8.GetString($changed)
}

function Get-Utf8Range([string]$Source, [int]$Start, [int]$End) {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Source)
    Assert-True ($Start -ge 0 -and $End -ge $Start -and $End -le $bytes.Length) 'The diagnostic source byte range is invalid.'
    return [Text.Encoding]::UTF8.GetString($bytes, $Start, $End - $Start)
}

function Get-Utf8ByteOffset([string]$Source, [string]$Token) {
    $index = $Source.IndexOf($Token, [StringComparison]::Ordinal)
    Assert-True ($index -ge 0) "Source token was not found: $Token"
    return [Text.Encoding]::UTF8.GetByteCount($Source.Substring(0, $index))
}

function Scroll-ViewToY([object]$TableIndex, [int]$SourceStart, [double]$TargetY = 240.0) {
    $before = Get-Diagnostic
    $view = @($before.views | Where-Object { ($null -eq $TableIndex -or $_.table -eq $TableIndex) -and $_.source_start -eq $SourceStart }) | Select-Object -First 1
    Assert-True ($null -ne $view) "No mapped source view exists for table '$TableIndex' byte $SourceStart."
    $documentY = [double]$view.y + [double]$before.scroll
    $targetScroll = [Math]::Max(0.0, $documentY - $TargetY)
    $steps = [int][Math]::Round(($targetScroll - $before.scroll) / 80.0)
    if ($steps -ne 0) {
        Send-Wheel -Delta ([int](-$steps * 120))
        $expectedScroll = $before.scroll + $steps * 80.0
        $visible = Wait-Diagnostic { param($d) [Math]::Abs($d.scroll - $expectedScroll) -lt 1.0 } 3000 "scrolling source view to y=$TargetY"
        return $visible
    }
    return $before
}

function Complete-EditTransaction($Before, $After, [string]$Label) {
    Assert-True ($After.source -cne $Before.source -and $After.dirty -and $After.undo_count -eq ($Before.undo_count + 1)) "$Label did not form one source transaction."
    $expected = [string]$After.source
    Send-Key 0x5A @(0x11)
    $undo = Wait-Diagnostic { param($d) $d.source -ceq $Before.source -and -not $d.dirty } 5000 "$Label Undo"
    Add-DiagnosticTrace "$Label-undo" $undo
    Send-Key 0x59 @(0x11)
    $redo = Wait-Diagnostic { param($d) $d.source -ceq $expected } 5000 "$Label Redo"
    Add-DiagnosticTrace "$Label-redo" $redo
    Send-Key 0x5A @(0x11)
    $rollback = Wait-Diagnostic { param($d) $d.source -ceq $Before.source -and -not $d.dirty } 5000 "$Label final rollback"
    Add-DiagnosticTrace "$Label-rollback" $rollback
    return $rollback
}

function Invoke-TableMenuTransaction($CellView, [int]$CommandId, [string]$Label) {
    Click-ClientDip ($CellView.x + $CellView.width / 2.0) ($CellView.y + $CellView.height / 2.0)
    $selected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $CellView.source_start -and $d.selection.head -le $CellView.source_end } 5000 "$Label table-cell selection"
    $before = Get-Diagnostic
    [void](Invoke-ContextMenuCommand ($CellView.x + $CellView.width / 2.0) ($CellView.y + $CellView.height / 2.0) $CommandId $Label)
    $after = Wait-Diagnostic { param($d) $d.source -cne $before.source } 5000 "$Label source edit"
    Add-DiagnosticTrace "$Label-applied" $after
    $rollback = Complete-EditTransaction $before $after $Label
    Add-Check "R5-$Label" 'PASS' "$Label used the real native context menu path and restored exact source through one-transaction Undo/Redo."
    return $rollback
}

function Invoke-TextViewTransaction($View, [string]$Label, [string]$Text) {
    Click-ClientDip ($View.x + [Math]::Min(320.0, $View.width / 2.0)) ($View.y + $View.height / 2.0)
    $selected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $View.source_start -and $d.selection.head -le $View.source_end } 5000 "$Label source view selection"
    $expectedSource = Insert-Utf8Byte $selected.source ([int]$selected.selection.head) ([byte][char]$Text)
    Send-UnicodeText $Text
    $after = Wait-Diagnostic { param($d) $d.source -ceq $expectedSource } 5000 "$Label text input"
    Add-DiagnosticTrace "$Label-typed" $after
    $rollback = Complete-EditTransaction $selected $after $Label
    Add-Check "R6-$Label" 'PASS' "$Label accepted a character next to the image source and restored exact source through Undo/Redo."
    return $rollback
}

function Invoke-OsTsvPaste($CellView, [string]$ClipboardText, [string]$ExpectedSource, [string]$Label) {
    Click-ClientDip ($CellView.x + $CellView.width / 2.0) ($CellView.y + $CellView.height / 2.0)
    $selected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $CellView.source_start -and $d.selection.head -le $CellView.source_end } 5000 "$Label target-cell selection"
    $clipboardReady = $true
    try { Save-ClipboardAndSetText $ClipboardText }
    catch { $clipboardReady = $false; Add-Check "R5-$Label" 'BLOCKED' $_.Exception.Message }
    if (-not $clipboardReady) { return }
    $before = Get-Diagnostic
    Wait-Foreground
    if (-not (Test-TestClipboardSequence "$Label Ctrl+V")) {
        Add-Check "R5-$Label" 'BLOCKED' 'The Windows clipboard sequence changed before Ctrl+V; no paste was sent.'
        return
    }
    Send-Key 0x56 @(0x11)
    $after = Wait-Diagnostic { param($d) $d.source -ceq $ExpectedSource } 5000 "$Label source and cell-range paste"
    Add-DiagnosticTrace "$Label-applied" $after
    $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{
        operation = $Label
        input = 'Windows CF_UNICODETEXT + SendInput Ctrl+V'
        dispatch = 'WindowProc WM_KEYDOWN Ctrl+V → paste_clipboard → Table::paste_tsv'
        selected_cell = $selected.selection
    }
    $rollback = Complete-EditTransaction $before $after $Label
    $restored = Restore-Clipboard
    if ($restored) { Add-Check "R5-$Label" 'PASS' "$Label applied the exact rectangular TSV in one edit transaction, round-tripped Undo/Redo, and restored the original clipboard formats." }
}

function Invoke-OsTsvBoundaryCheck($CellView, [string]$ClipboardText, [string]$Label) {
    Click-ClientDip ($CellView.x + $CellView.width / 2.0) ($CellView.y + $CellView.height / 2.0)
    $selected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $CellView.source_start -and $d.selection.head -le $CellView.source_end } 5000 "$Label target-cell selection"
    $ready = $true
    try { Save-ClipboardAndSetText $ClipboardText }
    catch { $ready = $false; Add-Check "R5-$Label" 'BLOCKED' $_.Exception.Message }
    if (-not $ready) { return }
    $before = Get-Diagnostic
    Wait-Foreground
    if (-not (Test-TestClipboardSequence "$Label Ctrl+V")) {
        Add-Check "R5-$Label" 'BLOCKED' 'The Windows clipboard sequence changed before the boundary paste; no paste was sent.'
        return
    }
    Send-Key 0x56 @(0x11)
    $rejected = Wait-Diagnostic { param($d) $d.status.Contains('表の貼り付け範囲') } 5000 "$Label rejection status"
    Add-DiagnosticTrace "$Label-rejected" $rejected
    Assert-True ($rejected.source -ceq $before.source -and $rejected.revision -eq $before.revision -and $rejected.dirty -eq $before.dirty -and $rejected.undo_count -eq $before.undo_count -and $rejected.redo_count -eq $before.redo_count -and $rejected.tables -eq $before.tables) "$Label changed source or history despite the table boundary rejection."
    $restored = Restore-Clipboard
    if ($restored) { Add-Check "R5-$Label" 'PASS' "$Label was rejected with status feedback and left exact source, table count and history unchanged; original clipboard formats were restored." }
}

function Capture-Desktop([string]$Name, $Diagnostic, [IntPtr]$TargetWindow = [IntPtr]::Zero) {
    if ($TargetWindow -eq [IntPtr]::Zero) { $TargetWindow = $script:WindowHandle }
    if ($TargetWindow -eq $script:WindowHandle) {
        Assert-Foreground
        Assert-True (-not [MdliteRustGuiNative]::IsIconic($script:WindowHandle)) 'The app is minimized; a real app frame cannot be captured.'
    }
    else {
        Assert-AppAlive
        Assert-True ([MdliteRustGuiNative]::IsWindowVisible($TargetWindow)) 'The owned candidate window is not visible.'
        Assert-True ([MdliteRustGuiNative]::GetWindow($TargetWindow, [MdliteRustGuiNative]::GW_OWNER) -eq $script:WindowHandle) 'The candidate screenshot target is not owned by the launched app window.'
    }
    $windowRect = [MdliteRustGuiNative+Rect]::new()
    if (-not [MdliteRustGuiNative]::GetWindowRect($TargetWindow, [ref]$windowRect)) { throw 'GetWindowRect failed.' }
    $appRect = [MdliteRustGuiNative+Rect]::new()
    if (-not [MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$appRect)) { throw 'GetWindowRect failed for the app window.' }
    $left, $top = $windowRect.Left, $windowRect.Top
    $width = $windowRect.Right - $windowRect.Left
    $height = $windowRect.Bottom - $windowRect.Top
    $captureRegion = 'window'
    if ($TargetWindow -eq $script:WindowHandle) {
        $clientRect = [MdliteRustGuiNative+Rect]::new()
        if (-not [MdliteRustGuiNative]::GetClientRect($TargetWindow, [ref]$clientRect)) { throw 'GetClientRect failed for the app screenshot.' }
        $clientOrigin = [MdliteRustGuiNative+Point]::new()
        $clientOrigin.X = $clientRect.Left
        $clientOrigin.Y = $clientRect.Top
        if (-not [MdliteRustGuiNative]::ClientToScreen($TargetWindow, [ref]$clientOrigin)) { throw 'ClientToScreen failed for the app screenshot.' }
        $left, $top = $clientOrigin.X, $clientOrigin.Y
        $width = $clientRect.Right - $clientRect.Left
        $height = $clientRect.Bottom - $clientRect.Top
        $captureRegion = 'client'
    }
    Assert-True ($width -gt 0 -and $height -gt 0) 'The target window has an empty desktop rectangle.'
    $path = Join-Path $script:RunDirectory $Name
    $bitmap = [Drawing.Bitmap]::new($width, $height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.CopyFromScreen($left, $top, 0, 0, [Drawing.Size]::new($width, $height), [Drawing.CopyPixelOperation]::SourceCopy)
        $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
    }
    finally { $graphics.Dispose(); $bitmap.Dispose() }
    [uint32]$windowProcessId = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($TargetWindow, [ref]$windowProcessId)
    $entry = [pscustomobject]@{
        name = $Name
        path = $path
        captured_utc = [DateTime]::UtcNow.ToString('o')
        sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        selection = $Diagnostic.selection
        caret = $Diagnostic.caret
        scroll = $Diagnostic.scroll
        window_rect = [pscustomobject]@{ left = $windowRect.Left; top = $windowRect.Top; right = $windowRect.Right; bottom = $windowRect.Bottom }
        app_window_rect = [pscustomobject]@{ left = $appRect.Left; top = $appRect.Top; right = $appRect.Right; bottom = $appRect.Bottom }
        capture_region = $captureRegion
        capture_bounds = [pscustomobject]@{ left = $left; top = $top; width = $width; height = $height; full_virtual_desktop = $false }
        captured_window = ('0x{0:X}' -f $TargetWindow.ToInt64())
        captured_window_pid = $windowProcessId
        foreground_window = ('0x{0:X}' -f [MdliteRustGuiNative]::GetForegroundWindow().ToInt64())
    }
    $script:Report.screenshots = @($script:Report.screenshots) + $entry
    return $entry
}

function Get-OwnedImePopupWindows {
    $script:OwnedImePopups = [Collections.Generic.List[object]]::new()
    $callback = [MdliteRustGuiNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$data)
        if ($window -eq $script:WindowHandle -or -not [MdliteRustGuiNative]::IsWindowVisible($window)) { return $true }
        if ([MdliteRustGuiNative]::GetWindow($window, [MdliteRustGuiNative]::GW_OWNER) -ne $script:WindowHandle) { return $true }
        $rect = [MdliteRustGuiNative+Rect]::new()
        if (-not [MdliteRustGuiNative]::GetWindowRect($window, [ref]$rect)) { return $true }
        [uint32]$ownerProcessId = 0
        [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
        $className = [Text.StringBuilder]::new(80)
        [void][MdliteRustGuiNative]::GetClassNameW($window, $className, $className.Capacity)
        $script:OwnedImePopups.Add([pscustomobject]@{
            handle = $window
            handle_hex = ('0x{0:X}' -f $window.ToInt64())
            process_id = $ownerProcessId
            class_name = $className.ToString()
            rect = [pscustomobject]@{ left = $rect.Left; top = $rect.Top; right = $rect.Right; bottom = $rect.Bottom }
            area = ($rect.Right - $rect.Left) * ($rect.Bottom - $rect.Top)
        })
        return $true
    }
    [void][MdliteRustGuiNative]::EnumWindows($callback, [IntPtr]::Zero)
    return @($script:OwnedImePopups | Sort-Object area -Descending)
}

function Compare-SourceHistorySelection($Before, $After, [string]$Label) {
    Assert-True ($Before.source -ceq $After.source) "$Label changed canonical source."
    Assert-True ($Before.revision -eq $After.revision) "$Label changed source revision."
    Assert-True ($Before.dirty -eq $After.dirty) "$Label changed dirty state."
    Assert-True ($Before.undo_count -eq $After.undo_count) "$Label changed undo history."
    Assert-True ($Before.redo_count -eq $After.redo_count) "$Label changed redo history."
    Assert-True ($Before.selection.anchor -eq $After.selection.anchor -and $Before.selection.head -eq $After.selection.head) "$Label changed selection."
}

function Wait-ImportedImageFile([string]$Path, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-AppAlive
        if (Test-Path -LiteralPath $Path -PathType Leaf) { return Get-Item -LiteralPath $Path }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The image import did not save its asset before the source transaction: $Path"
}

function Get-ShellFolderWindows([string]$FolderPath = '') {
    $targetPath = if ([string]::IsNullOrWhiteSpace($FolderPath)) { $null } else { [IO.Path]::GetFullPath($FolderPath).TrimEnd([char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)) }
    $shell = New-Object -ComObject Shell.Application
    $windows = $null
    $matches = [Collections.Generic.List[object]]::new()
    try {
        $windows = $shell.Windows()
        for ($index = 0; $index -lt $windows.Count; $index++) {
            $window = $null
            try {
                $window = $windows.Item($index)
                $handle = [IntPtr]::new([int64]$window.HWND)
                if ($handle -eq [IntPtr]::Zero) { continue }
                $className = [Text.StringBuilder]::new(80)
                [void][MdliteRustGuiNative]::GetClassNameW($handle, $className, $className.Capacity)
                if ($className.ToString() -notin @('CabinetWClass', 'ExploreWClass')) { continue }
                $locationUrl = [string]$window.LocationURL
                $locationPath = ''
                if (-not [string]::IsNullOrWhiteSpace($locationUrl)) {
                    try {
                        $locationUri = [Uri]::new($locationUrl)
                        if ($locationUri.IsFile) { $locationPath = [IO.Path]::GetFullPath($locationUri.LocalPath).TrimEnd([char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)) }
                    }
                    catch { }
                }
                if ($null -ne $targetPath -and -not [string]::Equals($locationPath, $targetPath, [StringComparison]::OrdinalIgnoreCase)) { continue }
                [uint32]$processId = 0
                [void][MdliteRustGuiNative]::GetWindowThreadProcessId($handle, [ref]$processId)
                $matches.Add([pscustomobject]@{ handle = $handle; process_id = $processId; class_name = $className.ToString(); folder_path = $locationPath })
            }
            catch { }
            finally {
                if ($null -ne $window -and [Runtime.InteropServices.Marshal]::IsComObject($window)) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($window) }
            }
        }
    }
    finally {
        if ($null -ne $windows -and [Runtime.InteropServices.Marshal]::IsComObject($windows)) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($windows) }
        if ([Runtime.InteropServices.Marshal]::IsComObject($shell)) { [void][Runtime.InteropServices.Marshal]::ReleaseComObject($shell) }
    }
    return @($matches)
}

function Open-OwnedExplorerWindow([string]$FolderPath) {
    $script:OwnedExplorerWindows = @()
    $script:ExplorerBaselineHandles = @((Get-ShellFolderWindows | ForEach-Object { $_.handle }))
    $script:Report.shell_drop_baseline_hwnds = @($script:ExplorerBaselineHandles | ForEach-Object { '0x{0:X}' -f $_.ToInt64() } | Sort-Object -Unique)
    $existing = @(Get-ShellFolderWindows $FolderPath)
    Assert-True ($existing.Count -eq 0) 'A Shell window already shows the fresh Explorer drop folder.'
    $script:ExplorerDropFolder = [IO.Path]::GetFullPath($FolderPath)
    $script:Report.shell_drop_target_folder = $script:ExplorerDropFolder
    $explorerExecutable = Join-Path $env:WINDIR 'explorer.exe'
    $script:Report.shell_drop_launch_executable = $explorerExecutable
    $folderArgument = $script:ExplorerDropFolder
    if ($folderArgument -match '\s') { $folderArgument = '"' + $folderArgument + '"' }
    $launchArguments = '/n,' + $folderArgument
    $script:Report.shell_drop_launch_arguments = $launchArguments
    Start-Process -FilePath $explorerExecutable -ArgumentList $launchArguments | Out-Null
    $deadline = [DateTime]::UtcNow.AddSeconds(12)
    do {
        $matches = @(Get-ShellFolderWindows $script:ExplorerDropFolder)
        if ($matches.Count -gt 0) {
            $newWindows = @($matches | Where-Object { $script:ExplorerBaselineHandles -notcontains $_.handle })
            $script:OwnedExplorerWindows = $newWindows
            Assert-True ($newWindows.Count -eq 1) 'Explorer did not expose exactly one new window for the fresh drop folder.'
            $window = $newWindows[0]
            [void][MdliteRustGuiNative]::SetForegroundWindow($window.handle)
            $foregroundDeadline = [DateTime]::UtcNow.AddSeconds(2)
            do {
                $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
                [uint32]$foregroundPid = 0
                [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
                if ($foreground -eq $window.handle -and $foregroundPid -eq [uint32]$window.process_id) { return $window }
                [void][MdliteRustGuiNative]::SetForegroundWindow($window.handle)
                Start-Sleep -Milliseconds 50
            } while ([DateTime]::UtcNow -lt $foregroundDeadline)
            throw 'The exact new Explorer HWND and PID did not obtain foreground ownership for the bounded synthetic drag.'
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Explorer did not expose the newly opened unique drop folder through Shell.Application.Windows.'
}

function Arrange-OwnedShellDropWindows($ExplorerWindow) {
    Assert-AppAlive
    [uint32]$appPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$appPid)
    Assert-True ($appPid -eq [uint32]$script:Process.Id) 'The drop target window is no longer owned by the launched app process.'
    [uint32]$shellPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($ExplorerWindow.handle, [ref]$shellPid)
    Assert-True ($shellPid -eq [uint32]$ExplorerWindow.process_id) 'The owned Explorer window process identity changed before positioning.'
    if ([MdliteRustGuiNative]::IsIconic($script:WindowHandle)) { [void][MdliteRustGuiNative]::ShowWindow($script:WindowHandle, [MdliteRustGuiNative]::SW_RESTORE) }
    if ([MdliteRustGuiNative]::IsZoomed($script:WindowHandle)) { [void][MdliteRustGuiNative]::ShowWindow($script:WindowHandle, [MdliteRustGuiNative]::SW_RESTORE) }
    if ([MdliteRustGuiNative]::IsIconic($ExplorerWindow.handle)) { [void][MdliteRustGuiNative]::ShowWindow($ExplorerWindow.handle, [MdliteRustGuiNative]::SW_RESTORE) }
    if ([MdliteRustGuiNative]::IsZoomed($ExplorerWindow.handle)) { [void][MdliteRustGuiNative]::ShowWindow($ExplorerWindow.handle, [MdliteRustGuiNative]::SW_RESTORE) }
    $screen = [Windows.Forms.SystemInformation]::VirtualScreen
    Assert-True ($screen.Width -ge 1120 -and $screen.Height -ge 520) 'The visible desktop is too small for a bounded side-by-side Explorer/app drag proof.'
    $margin = 8
    $gap = 12
    $minimumExplorerWidth = 480
    $minimumAppWidth = 600
    $maximumAppWidth = $screen.Width - 2 * $margin - $gap - $minimumExplorerWidth
    Assert-True ($maximumAppWidth -ge $minimumAppWidth) 'The visible desktop cannot fit the minimum Explorer/app drop windows.'
    $appBefore = [MdliteRustGuiNative+Rect]::new()
    $explorerBefore = [MdliteRustGuiNative+Rect]::new()
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$appBefore)) 'Could not read the app window rectangle before drop positioning.'
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($ExplorerWindow.handle, [ref]$explorerBefore)) 'Could not read the Explorer window rectangle before drop positioning.'
    $appWidth = [Math]::Min($appBefore.Right - $appBefore.Left, $maximumAppWidth)
    $appHeight = [Math]::Min($appBefore.Bottom - $appBefore.Top, $screen.Height - 2 * $margin)
    Assert-True ($appWidth -ge $minimumAppWidth -and $appHeight -ge 440) 'The app cannot remain large enough for the bounded drop target.'
    $explorerWidth = [Math]::Min($explorerBefore.Right - $explorerBefore.Left, $screen.Width - 2 * $margin - $gap - $appWidth)
    $explorerHeight = [Math]::Min($explorerBefore.Bottom - $explorerBefore.Top, $screen.Height - 2 * $margin)
    Assert-True ($explorerWidth -ge $minimumExplorerWidth -and $explorerHeight -ge 360) 'Explorer cannot remain large enough to show both synthetic PNG items.'
    $explorerX = $screen.Left + $margin
    $appX = $explorerX + $explorerWidth + $gap
    $top = $screen.Top + $margin
    $flags = [MdliteRustGuiNative]::SWP_NOZORDER -bor [MdliteRustGuiNative]::SWP_NOACTIVATE
    Assert-True ([MdliteRustGuiNative]::SetWindowPos($ExplorerWindow.handle, [IntPtr]::Zero, $explorerX, $top, $explorerWidth, $explorerHeight, $flags)) 'Could not position the owned Explorer window within the visible desktop.'
    Assert-True ([MdliteRustGuiNative]::SetWindowPos($script:WindowHandle, [IntPtr]::Zero, $appX, $top, $appWidth, $appHeight, $flags)) 'Could not position the app drop target within the visible desktop.'
    $appAfter = [MdliteRustGuiNative+Rect]::new()
    $explorerAfter = [MdliteRustGuiNative+Rect]::new()
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$appAfter)) 'Could not read the positioned app rectangle.'
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($ExplorerWindow.handle, [ref]$explorerAfter)) 'Could not read the positioned Explorer rectangle.'
    Assert-True ($appAfter.Left -ge $explorerAfter.Right -and $appAfter.Right -le ($screen.Right - $margin) -and $appAfter.Top -ge $screen.Top -and $appAfter.Bottom -le $screen.Bottom) 'The positioned app drop target is not fully visible beside Explorer.'
    Assert-True ($explorerAfter.Left -ge $screen.Left -and $explorerAfter.Right -le $screen.Right -and $explorerAfter.Top -ge $screen.Top -and $explorerAfter.Bottom -le $screen.Bottom) 'The positioned Explorer item source is not fully visible.'
    return [pscustomobject]@{
        screen = [pscustomobject]@{ left = $screen.Left; top = $screen.Top; width = $screen.Width; height = $screen.Height }
        app_before = [pscustomobject]@{ left = $appBefore.Left; top = $appBefore.Top; right = $appBefore.Right; bottom = $appBefore.Bottom }
        app_after = [pscustomobject]@{ left = $appAfter.Left; top = $appAfter.Top; right = $appAfter.Right; bottom = $appAfter.Bottom }
        explorer_before = [pscustomobject]@{ left = $explorerBefore.Left; top = $explorerBefore.Top; right = $explorerBefore.Right; bottom = $explorerBefore.Bottom }
        explorer_after = [pscustomobject]@{ left = $explorerAfter.Left; top = $explorerAfter.Top; right = $explorerAfter.Right; bottom = $explorerAfter.Bottom }
        owner_pids = [pscustomobject]@{ app = $appPid; explorer = $shellPid }
    }
}

function Get-ExplorerFileItemGeometry($ExplorerWindow, [string]$FileName, [int]$TimeoutMs = 8000) {
    Add-Type -AssemblyName UIAutomationClient
    Add-Type -AssemblyName UIAutomationTypes
    $displayNames = @(@($FileName, [IO.Path]::GetFileNameWithoutExtension($FileName)) | Select-Object -Unique)
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($ExplorerWindow.handle)
        $items = [Collections.Generic.List[object]]::new()
        foreach ($displayName in $displayNames) {
            $condition = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $displayName)
            $elements = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $condition)
            foreach ($element in $elements) {
                $current = $element.Current
                if ($current.IsOffscreen -or $current.ControlType -notin @([System.Windows.Automation.ControlType]::ListItem, [System.Windows.Automation.ControlType]::DataItem)) { continue }
                $bounds = $current.BoundingRectangle
                if ($bounds.IsEmpty -or $bounds.Width -le 0 -or $bounds.Height -le 0) { continue }
                $items.Add([pscustomobject]@{ name = $current.Name; control_type = $current.ControlType.ProgrammaticName; x = $bounds.X; y = $bounds.Y; width = $bounds.Width; height = $bounds.Height })
            }
        }
        if ($items.Count -eq 1) { return $items[0] }
        if ($items.Count -gt 1) { throw "UI Automation found multiple visible Explorer items matching '$FileName'." }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Fresh UI Automation did not expose visible item geometry for '$FileName'."
}

function Assert-OwnedShellDragWindowPairs($ExplorerWindow) {
    Assert-AppAlive
    Assert-True ($ExplorerWindow.handle -ne [IntPtr]::Zero -and $script:WindowHandle -ne [IntPtr]::Zero) 'A required owned drag HWND is missing.'
    Assert-True ($script:ExplorerBaselineHandles -notcontains $ExplorerWindow.handle) 'The Explorer drag source HWND belongs to the pre-launch baseline, not this run.'
    $trackedExplorer = @($script:OwnedExplorerWindows | Where-Object { $_.handle -eq $ExplorerWindow.handle -and $_.process_id -eq $ExplorerWindow.process_id -and [string]::Equals($_.folder_path, $ExplorerWindow.folder_path, [StringComparison]::OrdinalIgnoreCase) })
    Assert-True ($trackedExplorer.Count -gt 0) 'The Explorer source HWND/PID/folder tuple was not created by this run.'
    [uint32]$appPid = 0
    $appThread = [MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$appPid)
    Assert-True ($appThread -ne 0 -and $appPid -eq [uint32]$script:Process.Id) 'The app target HWND/PID pair changed before the Shell drag.'
    [uint32]$explorerPid = 0
    $explorerThread = [MdliteRustGuiNative]::GetWindowThreadProcessId($ExplorerWindow.handle, [ref]$explorerPid)
    Assert-True ($explorerThread -ne 0 -and $explorerPid -eq [uint32]$ExplorerWindow.process_id) 'The Explorer source HWND/PID pair changed before the Shell drag.'
    $className = [Text.StringBuilder]::new(80)
    [void][MdliteRustGuiNative]::GetClassNameW($ExplorerWindow.handle, $className, $className.Capacity)
    Assert-True ($className.ToString() -eq $ExplorerWindow.class_name -and $className.ToString() -in @('CabinetWClass', 'ExploreWClass')) 'The owned Explorer HWND changed class before the Shell drag.'
}

function Assert-OwnedShellDragFocus($ExplorerWindow, [switch]$ExplorerSourceOnly) {
    Assert-OwnedShellDragWindowPairs $ExplorerWindow
    $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$foregroundPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
    $foregroundIsExplorer = $foreground -eq $ExplorerWindow.handle -and $foregroundPid -eq [uint32]$ExplorerWindow.process_id
    $foregroundIsApp = $foreground -eq $script:WindowHandle -and $foregroundPid -eq [uint32]$script:Process.Id
    if ($ExplorerSourceOnly) {
        Assert-True $foregroundIsExplorer 'The exact newly owned Explorer HWND/PID lost foreground before reading the drag source.'
    }
    else {
        Assert-True ($foregroundIsExplorer -or $foregroundIsApp) 'Synthetic drag foreground is outside the exact owned Explorer/app HWND/PID pairs.'
    }
}

function Assert-OwnedExplorerSourceIdentity($ExplorerWindow, [string]$FileName, [int]$SourceX, [int]$SourceY) {
    Assert-OwnedShellDragWindowPairs $ExplorerWindow
    $targetPath = [IO.Path]::GetFullPath($ExplorerWindow.folder_path).TrimEnd([char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar))
    Assert-True ([string]::Equals($targetPath, $script:ExplorerDropFolder.TrimEnd([char[]]@([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)), [StringComparison]::OrdinalIgnoreCase)) 'The owned Explorer HWND no longer names the unique source folder for this run.'
    $shellMatches = @(Get-ShellFolderWindows $targetPath | Where-Object { $_.handle -eq $ExplorerWindow.handle -and $_.process_id -eq $ExplorerWindow.process_id -and $_.class_name -eq $ExplorerWindow.class_name -and [string]::Equals($_.folder_path, $targetPath, [StringComparison]::OrdinalIgnoreCase) })
    $matchingHandles = @($shellMatches | ForEach-Object { '{0}:{1}' -f $_.handle.ToInt64(), $_.process_id } | Sort-Object -Unique)
    Assert-True ($matchingHandles.Count -eq 1 -and $matchingHandles[0] -eq ('{0}:{1}' -f $ExplorerWindow.handle.ToInt64(), $ExplorerWindow.process_id)) 'Shell no longer maps the exact owned Explorer HWND/PID to the unique source folder.'
    $item = Get-ExplorerFileItemGeometry $ExplorerWindow $FileName
    Assert-True ($item.name -in @($FileName, [IO.Path]::GetFileNameWithoutExtension($FileName)) -and $item.control_type -in @('ControlType.ListItem', 'ControlType.DataItem')) 'Fresh UI Automation no longer identifies the requested file item in the owned source folder.'
    $freshX = [int][Math]::Round($item.x + ($item.width / 2.0))
    $freshY = [int][Math]::Round($item.y + ($item.height / 2.0))
    Assert-True ($freshX -eq $SourceX -and $freshY -eq $SourceY) 'The source file item moved after cursor positioning; refusing a stale mouse-down point.'
    Assert-OwnedShellDragWindowPairs $ExplorerWindow
    $shellMatches = @(Get-ShellFolderWindows $targetPath | Where-Object { $_.handle -eq $ExplorerWindow.handle -and $_.process_id -eq $ExplorerWindow.process_id -and $_.class_name -eq $ExplorerWindow.class_name -and [string]::Equals($_.folder_path, $targetPath, [StringComparison]::OrdinalIgnoreCase) })
    $matchingHandles = @($shellMatches | ForEach-Object { '{0}:{1}' -f $_.handle.ToInt64(), $_.process_id } | Sort-Object -Unique)
    Assert-True ($matchingHandles.Count -eq 1 -and $matchingHandles[0] -eq ('{0}:{1}' -f $ExplorerWindow.handle.ToInt64(), $ExplorerWindow.process_id)) 'The Shell source-folder identity changed during the fresh UIA read.'
    return $item
}

function Send-OwnedShellDragInput([MdliteRustGuiNative+Input[]]$Inputs, $ExplorerWindow, [switch]$ExplorerSourceOnly, [string]$SourceFileName, [int]$SourceX, [int]$SourceY) {
    if (-not [string]::IsNullOrWhiteSpace($SourceFileName)) { [void](Assert-OwnedExplorerSourceIdentity $ExplorerWindow $SourceFileName $SourceX $SourceY) }
    Assert-OwnedShellDragFocus $ExplorerWindow -ExplorerSourceOnly:$ExplorerSourceOnly
    $size = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
    $sent = [MdliteRustGuiNative]::SendInput([uint32]$Inputs.Length, $Inputs, $size)
    if ($sent -ne $Inputs.Length) { throw "SendInput delivered $sent of $($Inputs.Length) Shell drag events." }
}

function Set-OwnedShellDragCursorPos([int]$X, [int]$Y, $ExplorerWindow, [switch]$ExplorerSourceOnly) {
    Assert-OwnedShellDragFocus $ExplorerWindow -ExplorerSourceOnly:$ExplorerSourceOnly
    if (-not [MdliteRustGuiNative]::SetCursorPos($X, $Y)) { throw 'SetCursorPos failed during the ownership-guarded Shell file drag.' }
}

function Invoke-ShellExplorerFileDrag($ExplorerWindow, [string]$FileName) {
    Assert-AppAlive
    $item = Get-ExplorerFileItemGeometry $ExplorerWindow $FileName
    $explorerRect = [MdliteRustGuiNative+Rect]::new()
    $appRect = [MdliteRustGuiNative+Rect]::new()
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($ExplorerWindow.handle, [ref]$explorerRect)) 'Could not read the owned Explorer rectangle for the UIA source item.'
    Assert-True ([MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$appRect)) 'Could not read the app rectangle for the drag target.'
    $screen = [Windows.Forms.SystemInformation]::VirtualScreen
    $startX = [int][Math]::Round($item.x + ($item.width / 2.0))
    $startY = [int][Math]::Round($item.y + ($item.height / 2.0))
    Assert-True ($startX -ge $explorerRect.Left -and $startX -lt $explorerRect.Right -and $startY -ge $explorerRect.Top -and $startY -lt $explorerRect.Bottom -and $startX -ge $screen.Left -and $startX -lt $screen.Right -and $startY -ge $screen.Top -and $startY -lt $screen.Bottom) 'Fresh UIA item geometry is outside the visible Explorer source window or desktop.'
    $client = [MdliteRustGuiNative+Rect]::new()
    Assert-True ([MdliteRustGuiNative]::GetClientRect($script:WindowHandle, [ref]$client)) 'Could not read the app client rectangle for Explorer drop targeting.'
    $target = [MdliteRustGuiNative+Point]::new()
    $target.X = [int]($client.Right / 2)
    $target.Y = [int]($client.Bottom / 2)
    Assert-True ([MdliteRustGuiNative]::ClientToScreen($script:WindowHandle, [ref]$target)) 'Could not map the app drop point to screen coordinates.'
    Assert-True ($target.X -ge $appRect.Left -and $target.X -lt $appRect.Right -and $target.Y -ge $appRect.Top -and $target.Y -lt $appRect.Bottom -and $target.X -ge $screen.Left -and $target.X -lt $screen.Right -and $target.Y -ge $screen.Top -and $target.Y -lt $screen.Bottom) 'The app drop point is outside the visible owned app window or desktop.'
    Assert-True (-not ($target.X -ge $explorerRect.Left -and $target.X -lt $explorerRect.Right -and $target.Y -ge $explorerRect.Top -and $target.Y -lt $explorerRect.Bottom)) 'Explorer obscures the selected app drop point.'
    Assert-OwnedShellDragWindowPairs $ExplorerWindow
    [void][MdliteRustGuiNative]::SetForegroundWindow($ExplorerWindow.handle)
    $foregroundDeadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
        [uint32]$foregroundPid = 0
        [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid)
        if ($foreground -eq $ExplorerWindow.handle -and $foregroundPid -eq [uint32]$ExplorerWindow.process_id) { break }
        [void][MdliteRustGuiNative]::SetForegroundWindow($ExplorerWindow.handle)
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $foregroundDeadline)
    Assert-True ($foreground -eq $ExplorerWindow.handle -and $foregroundPid -eq [uint32]$ExplorerWindow.process_id) 'The exact newly owned Explorer HWND/PID lost foreground before the drag began.'
    Set-OwnedShellDragCursorPos $startX $startY $ExplorerWindow -ExplorerSourceOnly
    Start-Sleep -Milliseconds 50
    $buttonDown = $false
    try {
        Send-OwnedShellDragInput @([MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTDOWN)) $ExplorerWindow -ExplorerSourceOnly -SourceFileName $FileName -SourceX $startX -SourceY $startY
        $buttonDown = $true
        for ($step = 1; $step -le 4; $step++) {
            $x = $startX + [int][Math]::Round(($target.X - $startX) * $step / 4.0)
            $y = $startY + [int][Math]::Round(($target.Y - $startY) * $step / 4.0)
            Set-OwnedShellDragCursorPos $x $y $ExplorerWindow
            Start-Sleep -Milliseconds 60
        }
        Send-OwnedShellDragInput @([MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTUP)) $ExplorerWindow
        $buttonDown = $false
    }
    finally {
        if ($buttonDown) {
            $release = [MdliteRustGuiNative]::MouseButton([MdliteRustGuiNative]::MOUSEEVENTF_LEFTUP)
            $size = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
            $released = [MdliteRustGuiNative]::SendInput(1, [MdliteRustGuiNative+Input[]]@($release), $size)
            if ($released -ne 1) { throw 'SendInput could not release the held Explorer drag button during cleanup.' }
        }
    }
    return [pscustomobject]@{
        file_name = $FileName
        shell_window = ('0x{0:X}' -f $ExplorerWindow.handle.ToInt64())
        shell_process_id = $ExplorerWindow.process_id
        uia_name = $item.name
        uia_control_type = $item.control_type
        uia_item_bounds = [pscustomobject]@{ x = $item.x; y = $item.y; width = $item.width; height = $item.height }
        source_point = [pscustomobject]@{ x = $startX; y = $startY }
        drop_point = [pscustomobject]@{ x = $target.X; y = $target.Y }
        delivery = 'Synthetic SendInput mouse drag from a fresh UIA Explorer item; Shell supplies the file-drop payload to the owned app.'
        physical_dragdrop = $false
    }
}

function Close-OwnedExplorerWindows {
    if (@($script:OwnedExplorerWindows).Count -eq 0 -and -not [string]::IsNullOrWhiteSpace($script:ExplorerDropFolder)) {
        $script:OwnedExplorerWindows = @(Get-ShellFolderWindows $script:ExplorerDropFolder | Where-Object { $script:ExplorerBaselineHandles -notcontains $_.handle })
    }
    $results = @()
    foreach ($window in @($script:OwnedExplorerWindows)) {
        $closed = $false
        $reason = ''
        try {
            $matches = @(Get-ShellFolderWindows $window.folder_path | Where-Object { $_.handle -eq $window.handle })
            if ($matches.Count -eq 0 -or -not [MdliteRustGuiNative]::IsWindowVisible($window.handle)) { $closed = $true }
            elseif ($matches.Count -eq 1 -and $matches[0].process_id -eq $window.process_id) {
                $result = [IntPtr]::Zero
                [void][MdliteRustGuiNative]::SendMessageTimeoutW($window.handle, [MdliteRustGuiNative]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero, [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, 1000, [ref]$result)
                $deadline = [DateTime]::UtcNow.AddSeconds(2)
                do {
                    if (-not [MdliteRustGuiNative]::IsWindowVisible($window.handle)) { $closed = $true; break }
                    Start-Sleep -Milliseconds 50
                } while ([DateTime]::UtcNow -lt $deadline)
                if (-not $closed) { $reason = 'the owned Explorer window remained visible after bounded WM_CLOSE; it was not force-closed' }
            }
            else { $reason = 'the Shell window identity changed; it was left untouched' }
        }
        catch { $reason = $_.Exception.Message }
        $results += [pscustomobject]@{ handle = ('0x{0:X}' -f $window.handle.ToInt64()); process_id = $window.process_id; closed_normally = $closed; reason = $reason }
    }
    $script:OwnedExplorerWindows = @()
    return $results
}

function Save-ClipboardAndSetText([string]$Text) {
    Backup-Clipboard
    $testSequence = [uint32]0
    $clipboardChanged = $false
    $reason = ''
    if (-not [MdliteRustGuiNative]::TrySetUnicodeClipboardText($Text, $script:ClipboardOriginalSequence, [ref]$testSequence, [ref]$clipboardChanged, [ref]$reason)) {
        if ($clipboardChanged) {
            $script:ClipboardRestoreSequence = $testSequence
            $restoreReason = ''
            try { [void](Restore-Clipboard) } catch { $reason += "; immediate clipboard restore also failed: $($_.Exception.Message)" }
        }
        else {
            $script:ClipboardCaptured = $false
            $script:ClipboardSnapshot = $null
        }
        throw "Clipboard UTF-16 write failed: $reason"
    }
    $script:ClipboardRestoreSequence = $testSequence
    $script:Report.clipboard = [ordered]@{
        status = 'TEST_TEXT_SET'
        original_sequence = $script:ClipboardOriginalSequence
        test_sequence = $script:ClipboardRestoreSequence
        restored_sequence = $null
        formats = $script:ClipboardFormats
    }
}

function Save-ClipboardAndSetDib([byte[]]$Dib) {
    Backup-Clipboard
    $testSequence = [uint32]0
    $clipboardChanged = $false
    $reason = ''
    if (-not [MdliteRustGuiNative]::TrySetDibClipboard($Dib, $script:ClipboardOriginalSequence, [ref]$testSequence, [ref]$clipboardChanged, [ref]$reason)) {
        if ($clipboardChanged) {
            $script:ClipboardRestoreSequence = $testSequence
            try { [void](Restore-Clipboard) } catch { $reason += "; immediate clipboard restore also failed: $($_.Exception.Message)" }
        }
        else {
            $script:ClipboardCaptured = $false
            $script:ClipboardSnapshot = $null
            $script:Report.clipboard.status = 'UNCHANGED_DIB_WRITE_FAILED'
        }
        throw "Clipboard CF_DIB write failed: $reason"
    }
    $script:ClipboardRestoreSequence = $testSequence
    $script:Report.clipboard = [ordered]@{
        status = 'TEST_CF_DIB_SET'
        original_sequence = $script:ClipboardOriginalSequence
        test_sequence = $testSequence
        restored_sequence = $null
        formats = $script:ClipboardFormats
    }
}

function Backup-Clipboard {
    Assert-ClipboardTestsEnabled 'Clipboard snapshot'
    if ($script:ClipboardCaptured) { throw 'A clipboard backup is already active.' }
    $script:ClipboardOwnerHandle = [MdliteRustGuiNative]::GetClipboardOwnerWindow()
    [uint32]$ownerProcessId = 0
    $ownerThreadId = [MdliteRustGuiNative]::GetWindowThreadProcessId($script:ClipboardOwnerHandle, [ref]$ownerProcessId)
    $ownerClass = [Text.StringBuilder]::new(80)
    [void][MdliteRustGuiNative]::GetClassNameW($script:ClipboardOwnerHandle, $ownerClass, $ownerClass.Capacity)
    $script:Report.clipboard_owner = [ordered]@{
        state = 'ACTIVE'
        hwnd = ('0x{0:X}' -f $script:ClipboardOwnerHandle.ToInt64())
        process_id = $ownerProcessId
        process_matches_runner = ($ownerProcessId -eq [MdliteRustGuiNative]::GetCurrentProcessId())
        thread_id = $ownerThreadId
        thread_matches_runner = ($ownerThreadId -eq [MdliteRustGuiNative]::GetCurrentThreadId())
        window_title = [MdliteRustGuiNative]::GetClipboardOwnerWindowTitle()
        message_only_membership_verified = $true
        class_name = $ownerClass.ToString()
        hidden = -not [MdliteRustGuiNative]::IsWindowVisible($script:ClipboardOwnerHandle)
    }
    $sequenceBefore = [MdliteRustGuiNative]::GetClipboardSequenceNumber()
    $snapshot = $null
    $reason = ''
    if (-not [MdliteRustGuiNative]::TryCaptureClipboard([ref]$snapshot, [ref]$reason)) {
        throw "Cannot safely snapshot every current Windows clipboard format; clipboard tests were skipped without changing it: $reason"
    }
    if ([MdliteRustGuiNative]::GetClipboardSequenceNumber() -ne $sequenceBefore) {
        throw 'The clipboard changed during snapshot; clipboard tests were skipped without modifying the current contents.'
    }
    $script:ClipboardSnapshot = $snapshot
    $script:ClipboardCaptured = $true
    $script:ClipboardOriginalSequence = $sequenceBefore
    $script:ClipboardRestoreSequence = $sequenceBefore
    $script:ClipboardFormats = @($snapshot.Keys | ForEach-Object { [MdliteRustGuiNative]::ClipboardFormatName([uint32]$_) })
    $script:Report.clipboard = [ordered]@{
        status = 'BACKED_UP'
        original_sequence = $sequenceBefore
        test_sequence = $null
        restored_sequence = $null
        formats = $script:ClipboardFormats
    }
}

function Restore-Clipboard {
    if ($script:SkipClipboard) { return $false }
    if (-not $script:ClipboardCaptured) { return }
    $reason = ''
    $restoredSequence = [uint32]0
    $sequenceChanged = $false
    if (-not [MdliteRustGuiNative]::TryRestoreClipboard($script:ClipboardSnapshot, $script:ClipboardRestoreSequence, [ref]$restoredSequence, [ref]$sequenceChanged, [ref]$reason)) {
        if ($sequenceChanged) {
            $script:ClipboardCaptured = $false
            $script:ClipboardSnapshot = $null
            $script:Report.clipboard.status = 'BLOCKED_EXTERNAL_CLIPBOARD_CHANGE'
            Add-Check 'R4-clipboard-restore' 'BLOCKED' 'The clipboard sequence changed after the test paste; the helper preserved the newer clipboard contents.'
            return $false
        }
        throw "Native clipboard restore failed: $reason"
    }
    $script:ClipboardCaptured = $false
    $script:ClipboardSnapshot = $null
    $script:Report.clipboard.status = if ($script:ClipboardFormats.Count -eq 0) { 'RESTORED_EMPTY' } else { 'RESTORED' }
    $script:Report.clipboard.restored_sequence = $restoredSequence
    $script:Report.notes = @($script:Report.notes) + ("Restored the original Windows clipboard's " + $script:ClipboardFormats.Count + ' captured formats.')
    return $true
}

function Test-TestClipboardSequence([string]$Operation) {
    if ($script:SkipClipboard) { return $false }
    if (-not $script:ClipboardCaptured) { return $false }
    if ([MdliteRustGuiNative]::GetClipboardSequenceNumber() -eq $script:ClipboardRestoreSequence) { return $true }
    $script:Report.notes = @($script:Report.notes) + "$Operation skipped: the clipboard changed after the test marker was written."
    [void](Restore-Clipboard)
    return $false
}

function Wait-ClipboardSequenceChange([uint32]$PreviousSequence, [string]$Operation, [int]$TimeoutMs = 2500) {
    Assert-ClipboardTestsEnabled $Operation
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-AppAlive
        $sequence = [MdliteRustGuiNative]::GetClipboardSequenceNumber()
        if ($sequence -ne $PreviousSequence) { return $sequence }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "$Operation did not produce a Windows clipboard sequence change."
}

function Test-ClipboardTextValue([uint32]$Sequence, [string]$Expected, [string]$CheckId, [string]$Operation) {
    if ($script:SkipClipboard) {
        Add-Check $CheckId 'BLOCKED' "$Operation was blocked by -SkipClipboard; no clipboard payload was inspected."
        return $false
    }
    $actual = ''
    $current = [uint32]0
    $reason = ''
    if (-not [MdliteRustGuiNative]::TryReadUnicodeClipboardText($Sequence, [ref]$actual, [ref]$current, [ref]$reason)) {
        Add-Check $CheckId 'FAIL' "$Operation did not expose the expected Unicode clipboard format ($reason). Clipboard payload was not recorded."
        return $false
    }
    if ($actual -cne $Expected) {
        Add-Check $CheckId 'FAIL' "$Operation clipboard text did not match the expected fixture TSV; the observed payload was not recorded."
        return $false
    }
    $script:ClipboardRestoreSequence = $current
    $script:Report.clipboard.test_sequence = $current
    Add-Check $CheckId 'PASS' "$Operation published the exact expected fixture TSV through CF_UNICODETEXT."
    return $true
}

function Invoke-FocusedAcceptance {
    $fixture = New-Fixture $script:RunDirectory 'focused'
    $script:BaseSource = $fixture.Source
    $fingerprint = Get-FixtureFingerprint $fixture.Directory
    $script:Report.fixture_fingerprint = $fingerprint
    $diagPath = Join-Path $script:RunDirectory 'focused-diagnostic.json'
    $script:Report.process_runs = @((Start-RustApp @($fixture.Path) $diagPath))
    $initial = Wait-Diagnostic { param($d) $d.tabs -eq 1 -and $d.source -ceq $script:BaseSource } 8000 'the one-document source and first layout'
    Add-DiagnosticTrace 'initial-live-editor' $initial
    Capture-Desktop '00-live-editor-initial.png' $initial | Out-Null
    $expectedImageNames = @('raster.png', 'raster.jpg', 'raster.gif', 'raster.webp', 'animated.gif', 'animated.webp', 'fixture.svg', 'broken.png')
    $missingImageSources = @($expectedImageNames | Where-Object { -not $initial.source.Contains("assets/$_") })
    Assert-True ($missingImageSources.Count -eq 0) 'The live fixture does not reference every expected raster, SVG, and corrupt sample.'
    Assert-True ($initial.tables -ge 4 -and $initial.layout_count -gt 0 -and -not $initial.raw) 'The live editor did not report its content tables, wide table, header-only table and rendered layout.'
    Add-Check 'R1' 'PASS' 'One native Live Editor displays Japanese text, content tables, a wide table, a header-only table and raster/SVG fixture source; fingerprint and initial frame are attached.'

    $firstVisibleView = @($initial.views | Where-Object { $null -eq $_.table -and $_.source_end -gt $_.source_start -and $_.map.Count -gt 1 }) | Select-Object -First 1
    Assert-True ($null -ne $firstVisibleView) 'The initial Live Editor has no editable visible view map.'
    $liveHomeByte = [int]$firstVisibleView.map[0]
    Send-Key 0x24 @(0x11)
    $liveHome = Wait-Diagnostic { param($d) $d.selection.anchor -eq $liveHomeByte -and $d.selection.head -eq $liveHomeByte } 3000 'Ctrl+Home to the leading byte in the first visible view map'
    Assert-True ($initial.source -ceq $liveHome.source -and $initial.revision -eq $liveHome.revision -and $initial.dirty -eq $liveHome.dirty -and $initial.undo_count -eq $liveHome.undo_count -and $initial.redo_count -eq $liveHome.redo_count) 'Live Ctrl+Home changed source or edit history.'
    Add-DiagnosticTrace 'live-ctrl-home-visible-boundary' $liveHome
    Add-Check 'R3-live-home' 'PASS' "Live Ctrl+Home resolved the first visible view-map leading byte $liveHomeByte after hidden Markdown prefixes."
    $beforeRawHome = Get-Diagnostic
    Send-Key 0x45 @(0x11)
    $rawBeforeHome = Wait-Diagnostic { param($d) $d.raw } 3000 'entering raw mode for BOF navigation'
    Compare-SourceHistorySelection $beforeRawHome $rawBeforeHome 'Raw mode switch before Ctrl+Home'
    Send-Key 0x24 @(0x11)
    $rawHome = Wait-Diagnostic { param($d) $d.raw -and $d.selection.anchor -eq 0 -and $d.selection.head -eq 0 } 3000 'raw Ctrl+Home to byte zero'
    Assert-True ($rawBeforeHome.source -ceq $rawHome.source -and $rawBeforeHome.revision -eq $rawHome.revision -and $rawBeforeHome.undo_count -eq $rawHome.undo_count -and $rawBeforeHome.redo_count -eq $rawHome.redo_count) 'Raw Ctrl+Home changed source or edit history.'
    Add-DiagnosticTrace 'raw-ctrl-home-byte-zero' $rawHome
    Add-Check 'R3-raw-home' 'PASS' 'Raw Ctrl+Home resolved byte zero.'
    Send-Key 0x45 @(0x11)
    $liveAfterRawHome = Wait-Diagnostic { param($d) -not $d.raw } 3000 'returning to Live Editor after raw BOF navigation'
    Assert-True ($rawHome.source -ceq $liveAfterRawHome.source -and $rawHome.revision -eq $liveAfterRawHome.revision -and $rawHome.dirty -eq $liveAfterRawHome.dirty -and $rawHome.undo_count -eq $liveAfterRawHome.undo_count -and $rawHome.redo_count -eq $liveAfterRawHome.redo_count -and $liveAfterRawHome.selection.anchor -eq $liveHomeByte -and $liveAfterRawHome.selection.head -eq $liveHomeByte) 'Returning to Live after raw Ctrl+Home changed source/history or failed to map BOF to the first editable byte.'

    $beforeScroll = Get-Diagnostic
    Send-Wheel -Delta -960
    $scrolled = Wait-Diagnostic { param($d) $d.scroll -gt $beforeScroll.scroll } 5000 'scrolling the live image fixture'
    Add-DiagnosticTrace 'scroll-down' $scrolled
    Compare-SourceHistorySelection $beforeScroll $scrolled 'Wheel scrolling'
    $animatedGifStart = Get-Utf8ByteOffset $script:BaseSource '![Animated GIF](assets/animated.gif)'
    $animatedWebpStart = Get-Utf8ByteOffset $script:BaseSource '![Animated WebP](assets/animated.webp)'
    $animatedGifMarker = @($scrolled.views | Where-Object { $null -eq $_.table -and $_.source_start -eq $animatedGifStart -and $_.source_end -eq $animatedGifStart }) | Select-Object -First 1
    $animatedWebpMarker = @($scrolled.views | Where-Object { $null -eq $_.table -and $_.source_start -eq $animatedWebpStart -and $_.source_end -eq $animatedWebpStart }) | Select-Object -First 1
    Assert-True ($null -ne $animatedGifMarker -and $null -ne $animatedWebpMarker) 'The animated image source markers did not expose client-coordinate views after the initial scroll.'
    $animationClient = [MdliteRustGuiNative+Rect]::new()
    [void][MdliteRustGuiNative]::GetClientRect($script:WindowHandle, [ref]$animationClient)
    $animationScale = [double][MdliteRustGuiNative]::GetDpiForWindow($script:WindowHandle) / 96.0
    $animationViewportTop = 75.0
    $animationViewportBottom = $animationClient.Bottom / $animationScale - 32.0
    $imageTopInset = 17.0 * 1.6
    $webpImageTop = [double]$animatedWebpMarker.y + $imageTopInset
    $gifImageTop = [double]$animatedGifMarker.y + $imageTopInset
    $animationScrollSteps = [int][Math]::Max(0, [Math]::Ceiling(($webpImageTop - ($animationViewportBottom - 12.0)) / 80.0))
    $gifScrollStepsBeforeTop = [int][Math]::Floor(($gifImageTop - $animationViewportTop) / 80.0)
    Assert-True ($animationScrollSteps -le 4 -and $animationScrollSteps -le $gifScrollStepsBeforeTop) 'The bounded scroll needed to reveal animated WebP would move animated GIF above the content viewport.'
    if ($animationScrollSteps -gt 0) {
        $animationScrollBefore = $scrolled
        Send-Wheel -Delta (-120 * $animationScrollSteps)
        $expectedAnimationScroll = $animationScrollBefore.scroll + 80.0 * $animationScrollSteps
        $scrolled = Wait-Diagnostic { param($d) [Math]::Abs($d.scroll - $expectedAnimationScroll) -lt 1.0 } 5000 'bringing the animated WebP marker into the client viewport'
        Compare-SourceHistorySelection $animationScrollBefore $scrolled 'Animation-visibility scroll'
    }
    Add-DiagnosticTrace 'animation-visibility-scroll' $scrolled
    $animationsVisible = Wait-Diagnostic {
        param($d)
        $gif = @($d.animations | Where-Object { $_.path -match '(?i)\.gif$' -and $_.running })
        $webp = @($d.animations | Where-Object { $_.path -match '(?i)\.webp$' -and $_.running })
        $d.animation_timer_armed -and $gif.Count -gt 0 -and $webp.Count -gt 0
    } 5000 'the visible GIF/WebP animation timer and frame streams'
    $initialAnimationFrames = @{}
    foreach ($animation in $animationsVisible.animations) { $initialAnimationFrames[$animation.path] = $animation.frame }
    Start-Sleep -Milliseconds 260
    $animationTick = Wait-Diagnostic {
        param($d)
        $gif = @($d.animations | Where-Object { $_.path -match '(?i)\.gif$' -and $_.running })
        $webp = @($d.animations | Where-Object { $_.path -match '(?i)\.webp$' -and $_.running })
        $gifAdvanced = $gif.Count -gt 0 -and $gif[0].frame -ne $initialAnimationFrames[$gif[0].path]
        $webpAdvanced = $webp.Count -gt 0 -and $webp[0].frame -ne $initialAnimationFrames[$webp[0].path]
        $d.animation_timer_armed -and $gifAdvanced -and $webpAdvanced
    } 2500 'GIF and animated WebP frame advancement'
    $script:Report.animation_trace = @(
        [pscustomobject]@{ stage = 'visible-before-tick'; animation_timer_armed = $animationsVisible.animation_timer_armed; animations = $animationsVisible.animations },
        [pscustomobject]@{ stage = 'after-tick'; animation_timer_armed = $animationTick.animation_timer_armed; animations = $animationTick.animations }
    )
    Add-DiagnosticTrace 'visible-image-animation-tick' $animationTick
    Compare-SourceHistorySelection $beforeScroll $animationTick 'Animation frames'
    Capture-Desktop '01-images-and-animated-frames.png' $animationTick | Out-Null
    Add-Check 'R6-images' 'PASS' 'The same native document loaded PNG/JPEG/static GIF/static WebP/SVG and corrupt-PNG nodes; diagnostics proved visible animated GIF and WebP frame streams advanced while the timer was armed. Real desktop screenshots show the scrolled image viewport.'

    $beforeImageStart = Get-Utf8ByteOffset $script:BaseSource '画像の直前にある文章です。'
    $beforeImageView = @($animationTick.views | Where-Object { $null -eq $_.table -and $_.source_start -eq $beforeImageStart }) | Select-Object -First 1
    Assert-True ($null -ne $beforeImageView) 'The paragraph immediately before the first image has no diagnostic view.'
    $beforeImageTyped = Invoke-TextViewTransaction $beforeImageView 'before-image-text' 'P'
    Capture-Desktop 'before-image-text-edited.png' $beforeImageTyped | Out-Null
    $afterImageStart = Get-Utf8ByteOffset $script:BaseSource 'PNG直後の文章です。'
    $afterImageDiag = Get-Diagnostic
    $afterImageView = @($afterImageDiag.views | Where-Object { $null -eq $_.table -and $_.source_start -eq $afterImageStart }) | Select-Object -First 1
    Assert-True ($null -ne $afterImageView) 'The paragraph immediately after PNG has no diagnostic view.'
    $afterImageTyped = Invoke-TextViewTransaction $afterImageView 'after-image-text' 'A'
    Capture-Desktop 'after-image-text-edited.png' $afterImageTyped | Out-Null

    $pngToken = '![PNG](assets/raster.png)'
    $pngStart = Get-Utf8ByteOffset $script:BaseSource $pngToken
    $pngEnd = $pngStart + [Text.Encoding]::UTF8.GetByteCount($pngToken)
    $pngLayout = Scroll-ViewToY $null $pngStart 300.0
    $pngMarker = @($pngLayout.views | Where-Object { $null -eq $_.table -and $_.source_start -eq $pngStart -and $_.source_end -eq $pngStart }) | Select-Object -First 1
    Assert-True ($null -ne $pngMarker) 'The PNG image source marker has no native view geometry.'
    $pngClickX = 44.0 + 16.0
    $pngClickY = $pngMarker.y + (17.0 * 1.6) + 12.0
    Capture-Desktop 'image-before-resize.png' $pngLayout | Out-Null
    Click-ClientDip $pngClickX $pngClickY
    $pngSelected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $pngStart -and $d.selection.head -le $pngEnd } 5000 'selection on the PNG image block'
    $imageResizeBefore = Get-Diagnostic
    [void](Invoke-ContextMenuCommand $pngClickX $pngClickY 27 'image-resize')
    $imageResized = Wait-Diagnostic { param($d) $d.source.Contains('<!-- mdlite-width:26 -->') } 5000 'ratio-locked PNG image width metadata'
    Add-DiagnosticTrace 'image-width-resized' $imageResized
    Capture-Desktop 'image-after-resize.png' $imageResized | Out-Null
    $imageResizeRollback = Complete-EditTransaction $imageResizeBefore $imageResized 'image-width-resize'
    Capture-Desktop 'image-resize-undo.png' $imageResizeRollback | Out-Null
    Assert-True ($imageResizeRollback.source -ceq $script:BaseSource -and -not $imageResizeRollback.dirty) 'Image resize final Undo did not restore the original Markdown source.'
    Add-Check 'R6-image-resize' 'PASS' 'The native image context-menu shrink wrote width 26 from a 32×24 asset; after Undo/Redo/Undo the source returned exactly and screenshots show the before/after rendering.'

    $dropInputFolder = Join-Path $fixture.Directory 'shell-drop-input'
    $null = New-Item -ItemType Directory -Path $dropInputFolder
    $dropRunName = [IO.Path]::GetFileName($script:RunDirectory)
    $validDropName = "mdlite-valid-$dropRunName.png"
    $invalidDropName = "mdlite-invalid-$dropRunName.png"
    $validDropPath = Join-Path $dropInputFolder $validDropName
    $invalidDropPath = Join-Path $dropInputFolder $invalidDropName
    Copy-Item -LiteralPath (Join-Path $fixture.Directory 'assets\raster.png') -Destination $validDropPath
    Copy-Item -LiteralPath (Join-Path $fixture.Directory 'assets\broken.png') -Destination $invalidDropPath
    $validInputHash = (Get-FileHash -LiteralPath $validDropPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $invalidInputHash = (Get-FileHash -LiteralPath $invalidDropPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.shell_drop_fixture = [ordered]@{
        directory = $dropInputFolder
        files = @(
            [pscustomobject]@{ name = $validDropName; bytes = (Get-Item -LiteralPath $validDropPath).Length; sha256 = $validInputHash; expected = 'valid PNG' },
            [pscustomobject]@{ name = $invalidDropName; bytes = (Get-Item -LiteralPath $invalidDropPath).Length; sha256 = $invalidInputHash; expected = 'corrupt PNG' }
        )
    }
    $explorerWindow = $null
    try {
        $explorerWindow = Open-OwnedExplorerWindow $dropInputFolder
        $beforeShellPlacement = Get-Diagnostic
        $script:Report.shell_drop_layout = Arrange-OwnedShellDropWindows $explorerWindow
        Start-Sleep -Milliseconds 100
        $afterShellPlacement = Get-Diagnostic
        Compare-SourceHistorySelection $beforeShellPlacement $afterShellPlacement 'Shell drop window positioning'
        Add-DiagnosticTrace 'shell-drop-window-positioned' $afterShellPlacement
        $validImportedPath = Join-Path $fixture.Directory 'assets\image-00001.png'
        Assert-True (-not (Test-Path -LiteralPath $validImportedPath)) 'The successful Explorer import target already exists; refusing an ambiguous file-first assertion.'
        $beforeValidImport = Get-Diagnostic
        $validDrag = Invoke-ShellExplorerFileDrag $explorerWindow $validDropName
        $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{ operation = 'valid-image-shell-drag'; input_path = $validDropPath; input_sha256 = $validInputHash; physical_dragdrop = $false; shell_window = $validDrag.shell_window; shell_process_id = $validDrag.shell_process_id; uia_name = $validDrag.uia_name; uia_control_type = $validDrag.uia_control_type; uia_item_bounds = $validDrag.uia_item_bounds; drop_point = $validDrag.drop_point; dispatch = 'Fresh Explorer ListItem geometry → SendInput mouse drag → Shell file-drop delivery → import_image → assets::import_file' }
        $validImportedFile = Wait-ImportedImageFile $validImportedPath
        $validOutputHash = (Get-FileHash -LiteralPath $validImportedFile.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        Assert-True ($validImportedFile.Length -gt 0 -and $validOutputHash -eq $validInputHash) 'The file-first Shell drag import did not persist the exact valid PNG bytes before Markdown verification.'
        $validImportMarkdown = '![画像](assets/image-00001.png)'
        $validImport = Wait-Diagnostic { param($d) $d.source.Contains($validImportMarkdown) } 5000 'valid Shell image import Markdown after asset persistence'
        Add-DiagnosticTrace 'valid-image-shell-drop' $validImport
        Assert-True ($validImport.undo_count -eq ($beforeValidImport.undo_count + 1) -and $validImport.dirty) 'Valid Shell image import did not create one source transaction.'
        Wait-Foreground
        $validImportRollback = Complete-EditTransaction $beforeValidImport $validImport 'valid-image-shell-drop'
        Assert-True ($validImportRollback.source -ceq $beforeValidImport.source -and -not $validImportRollback.dirty) 'Valid Shell image import Undo/Redo did not return to its exact starting source.'
        $script:Report.command_pathways[-1] | Add-Member -NotePropertyName saved_sha256 -NotePropertyValue $validOutputHash -Force
        Add-Check 'R6-image-file-import' 'PASS' 'A valid local PNG arrived through a fresh Explorer item drag; the exact copied asset existed before source assertions, and source Undo/Redo/Undo restored the fixture. The drag input was synthetic OS input, not a physical human drag.'

        $dibFixture = [byte[]]::new(48)
        [Array]::Copy([BitConverter]::GetBytes([uint32]40), 0, $dibFixture, 0, 4)
        [Array]::Copy([BitConverter]::GetBytes([int32]1), 0, $dibFixture, 4, 4)
        [Array]::Copy([BitConverter]::GetBytes([int32]2), 0, $dibFixture, 8, 4)
        [Array]::Copy([BitConverter]::GetBytes([uint16]1), 0, $dibFixture, 12, 2)
        [Array]::Copy([BitConverter]::GetBytes([uint16]24), 0, $dibFixture, 14, 2)
        $dibFixture[40] = 0; $dibFixture[41] = 0; $dibFixture[42] = 255
        $dibFixture[44] = 255; $dibFixture[45] = 0; $dibFixture[46] = 0
        $dibImportedPath = Join-Path $fixture.Directory 'assets\image-00002.png'
        Assert-True (-not (Test-Path -LiteralPath $dibImportedPath)) 'The CF_DIB paste target already exists; refusing an ambiguous file-first assertion.'
        $beforeDibPaste = Get-Diagnostic
        $dibClipboardReady = $true
        try { Save-ClipboardAndSetDib $dibFixture }
        catch { $dibClipboardReady = $false; Add-Check 'R6-image-clipboard-paste' 'BLOCKED' $_.Exception.Message }
        if ($dibClipboardReady) {
            Wait-Foreground
            if (Test-TestClipboardSequence 'R6 CF_DIB Ctrl+V') {
                Send-Key 0x56 @(0x11)
                $dibImportedFile = Wait-ImportedImageFile $dibImportedPath
                $dibImage = [Drawing.Image]::FromFile($dibImportedFile.FullName)
                try {
                    $dibDimensions = [pscustomobject]@{ width = $dibImage.Width; height = $dibImage.Height }
                    Assert-True ($dibImage.Width -eq 1 -and $dibImage.Height -eq 2) 'The saved CF_DIB import did not preserve the 1×2 fixture dimensions.'
                }
                finally { $dibImage.Dispose() }
                $dibOutputHash = (Get-FileHash -LiteralPath $dibImportedFile.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
                Assert-True ($dibImportedFile.Length -gt 0) 'The CF_DIB paste created an empty PNG before the source transaction.'
                $dibImportMarkdown = '![画像](assets/image-00002.png)'
                $dibImported = Wait-Diagnostic { param($d) $d.source.Contains($dibImportMarkdown) } 5000 'CF_DIB paste Markdown after PNG persistence'
                Add-DiagnosticTrace 'cf-dib-image-paste' $dibImported
                Assert-True ($dibImported.undo_count -eq ($beforeDibPaste.undo_count + 1) -and $dibImported.dirty) 'CF_DIB Ctrl+V did not create one source transaction.'
                $dibPasteRollback = Complete-EditTransaction $beforeDibPaste $dibImported 'cf-dib-image-paste'
                Assert-True ($dibPasteRollback.source -ceq $beforeDibPaste.source -and -not $dibPasteRollback.dirty) 'CF_DIB paste Undo/Redo did not return to its exact starting source.'
                $dibClipboardRestored = Restore-Clipboard
                $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{ operation = 'cf-dib-image-paste'; input = 'Native HGLOBAL CF_DIB + sequence-guarded SendInput Ctrl+V'; physical_clipboard = $true; dispatch = 'WindowProc Ctrl+V → paste_clipboard → read_dib → assets::import_png → asset write completes before Markdown edit'; dib_fixture_bytes = $dibFixture.Length; saved_dimensions = $dibDimensions; saved_sha256 = $dibOutputHash; path = $dibImportedPath }
                if ($dibClipboardRestored) { Add-Check 'R6-image-clipboard-paste' 'PASS' 'The sequence-guarded CF_DIB clipboard image imported through Ctrl+V; the saved PNG existed and decoded to 1×2 before source assertions, and source/clipboard restoration completed.' }
                else { Add-Check 'R6-image-clipboard-paste' 'BLOCKED' 'The image paste and source rollback completed, but the original Windows clipboard was not restored.' }
            }
            else {
                Add-Check 'R6-image-clipboard-paste' 'BLOCKED' 'The Windows clipboard sequence changed before Ctrl+V; no image paste was sent.'
            }
        }

        $invalidImportedPath = Join-Path $fixture.Directory 'assets\image-00003.png'
        Assert-True (-not (Test-Path -LiteralPath $invalidImportedPath)) 'The corrupt Explorer import output path already exists.'
        $beforeDrop = Get-Diagnostic
        $invalidDrag = Invoke-ShellExplorerFileDrag $explorerWindow $invalidDropName
        $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{ operation = 'invalid-image-shell-drag'; input_path = $invalidDropPath; input_sha256 = $invalidInputHash; physical_dragdrop = $false; shell_window = $invalidDrag.shell_window; shell_process_id = $invalidDrag.shell_process_id; uia_name = $invalidDrag.uia_name; uia_control_type = $invalidDrag.uia_control_type; uia_item_bounds = $invalidDrag.uia_item_bounds; drop_point = $invalidDrag.drop_point; dispatch = 'Fresh Explorer ListItem geometry → SendInput mouse drag → Shell file-drop delivery → import_image → asset decode failure' }
        $dropRejected = Wait-Diagnostic { param($d) $d.status.Contains('画像の挿入に失敗') } 5000 'corrupt Explorer image insertion failure status'
        Add-DiagnosticTrace 'corrupt-image-shell-drop-rejected' $dropRejected
        Assert-True ($dropRejected.source -ceq $beforeDrop.source -and $dropRejected.revision -eq $beforeDrop.revision -and $dropRejected.dirty -eq $beforeDrop.dirty -and $dropRejected.undo_count -eq $beforeDrop.undo_count -and $dropRejected.redo_count -eq $beforeDrop.redo_count -and -not (Test-Path -LiteralPath $invalidImportedPath)) 'Corrupt Explorer image drop changed source/history or left an output asset.'
        Add-Check 'R6-image-import-failure' 'PASS' 'A corrupt local PNG arrived through a fresh Explorer item drag; failure status preserved source/history and created no asset. The drag input was synthetic OS input, not physical human drag-and-drop.'
    }
    finally {
        $explorerCleanup = @(Close-OwnedExplorerWindows)
        $script:Report.shell_drop_cleanup = $explorerCleanup
        if (@($explorerCleanup | Where-Object { -not $_.closed_normally }).Count -gt 0) {
            Add-Check 'R6-explorer-window-close' 'BLOCKED' 'The new Explorer window remained open after normal close; it was not force-closed.'
            throw 'The task-owned Explorer drop window did not close normally.'
        }
        if ($explorerCleanup.Count -gt 0) { Add-Check 'R6-explorer-window-close' 'PASS' 'The new Explorer window closed through bounded WM_CLOSE.' }
    }
    Wait-Foreground

    $beforeScrollReturn = Get-Diagnostic
    Assert-True ($beforeScrollReturn.source -ceq $script:BaseSource -and -not $beforeScrollReturn.dirty) 'R6 transactions did not restore the exact clean fixture source before returning the view to its initial scroll position.'
    Send-Wheel -Delta 960
    $top = Wait-Diagnostic { param($d) $d.scroll -eq $beforeScroll.scroll } 5000 'returning to the initial scroll position'
    Add-DiagnosticTrace 'scroll-return' $top
    Compare-SourceHistorySelection $beforeScrollReturn $top 'Scroll return wheel'
    Assert-True ($top.source -ceq $script:BaseSource -and -not $top.dirty) 'Returning the view to its initial scroll position changed the clean fixture source.'

    $cellToken = '| 表 |'
    $cellTokenIndex = $script:BaseSource.IndexOf($cellToken, [StringComparison]::Ordinal)
    Assert-True ($cellTokenIndex -ge 0) 'The target table cell is missing from the fixture source.'
    $cellStart = [Text.Encoding]::UTF8.GetByteCount($script:BaseSource.Substring(0, $cellTokenIndex)) + 2
    $cellView = @($top.views | Where-Object { $null -ne $_.table -and $_.table -eq 0 -and $_.source_start -eq $cellStart }) | Select-Object -First 1
    Assert-True ($null -ne $cellView -and $cellView.map -contains $cellStart) 'Table cell geometry/source map did not identify the expected source byte.'
    $script:Report.table_cell_geometry = [pscustomobject]@{
        table = $top.table_geometry[0]
        cell_view = $cellView
        expected_source_start = $cellStart
        first_table_source = Get-Utf8Range $script:BaseSource $top.table_geometry[0].source_start $top.table_geometry[0].source_end
    }
    Click-ClientDip ($cellView.x + $cellView.width / 2.0) ($cellView.y + $cellView.height / 2.0)
    $cellSelected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $cellView.source_start -and $d.selection.head -le $cellView.source_end } 5000 'the real mouse click to land in the mapped table cell'
    Add-DiagnosticTrace 'table-cell-click' $cellSelected
    Assert-True ($top.source -ceq $cellSelected.source -and $top.revision -eq $cellSelected.revision -and $top.dirty -eq $cellSelected.dirty -and $top.undo_count -eq $cellSelected.undo_count -and $top.redo_count -eq $cellSelected.redo_count) 'Table hit testing changed source or edit history.'
    Capture-Desktop '02-table-cell-hit-test.png' $cellSelected | Out-Null
    $expectedCellSource = Insert-Utf8Byte $script:BaseSource ([int]$cellSelected.selection.head) 0x51
    Send-UnicodeText 'Q'
    $cellTyped = Wait-Diagnostic { param($d) $d.source -ceq $expectedCellSource } 5000 'normal input inside the clicked table cell'
    Add-DiagnosticTrace 'table-cell-typed' $cellTyped
    Assert-True ($cellTyped.tables -eq 4 -and $cellTyped.undo_count -eq ($cellSelected.undo_count + 1) -and $cellTyped.dirty) 'The cell input changed table count or did not form one transaction.'
    Send-Key 0x5A @(0x11)
    $cellUndo = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource -and -not $d.dirty } 5000 'table cell input Undo'
    Add-DiagnosticTrace 'table-cell-undo' $cellUndo
    Assert-True ($cellUndo.tables -eq 4 -and $cellUndo.selection.anchor -eq $cellSelected.selection.anchor -and $cellUndo.selection.head -eq $cellSelected.selection.head) 'Table Undo did not restore the exact source and pre-edit selection.'
    Send-Key 0x59 @(0x11)
    $cellRedo = Wait-Diagnostic { param($d) $d.source -ceq $expectedCellSource } 5000 'table cell input Redo'
    Add-DiagnosticTrace 'table-cell-redo' $cellRedo
    Send-Key 0x5A @(0x11)
    $cellRollback = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource -and -not $d.dirty -and $d.tables -eq 4 } 5000 'table cell final rollback'
    Add-DiagnosticTrace 'table-cell-rollback' $cellRollback
    Add-Check 'R5-table' 'PASS' 'A SendInput mouse click used the native per-cell geometry/map to select the fixture source range; cell text input, exact table source, both table counts, and one-step Undo/Redo round-tripped.'

    foreach ($command in @(
        [pscustomobject]@{ id = 20; label = 'insert-row' },
        [pscustomobject]@{ id = 22; label = 'insert-column' },
        [pscustomobject]@{ id = 25; label = 'center-alignment' }
    )) {
        $beforeCommand = Get-Diagnostic
        $commandCell = @($beforeCommand.views | Where-Object { $null -ne $_.table -and $_.table -eq 0 -and $_.source_start -eq $cellStart }) | Select-Object -First 1
        Assert-True ($null -ne $commandCell) "The mapped table cell disappeared before $($command.label)."
        [void](Invoke-TableMenuTransaction $commandCell $command.id $command.label)
    }

    $headerCellStart = Get-Utf8ByteOffset $script:BaseSource 'only1'
    $headerCell = @((Get-Diagnostic).views | Where-Object { $_.table -eq 3 -and $_.source_start -le $headerCellStart -and $_.source_end -ge $headerCellStart }) | Select-Object -First 1
    Assert-True ($null -ne $headerCell) 'The header-only table cell has no source-mapped view.'
    [void](Invoke-TableMenuTransaction $headerCell 20 'header-only-row-insert')

    $headerCell = @((Get-Diagnostic).views | Where-Object { $_.table -eq 3 -and $_.source_start -le $headerCellStart -and $_.source_end -ge $headerCellStart }) | Select-Object -First 1
    Click-ClientDip ($headerCell.x + $headerCell.width / 2.0) ($headerCell.y + $headerCell.height / 2.0)
    Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $headerCell.source_start -and $d.selection.head -le $headerCell.source_end } 5000 'header-only table header-cell selection' | Out-Null
    $headerBeforeDelete = Get-Diagnostic
    [void](Invoke-ContextMenuCommand ($headerCell.x + $headerCell.width / 2.0) ($headerCell.y + $headerCell.height / 2.0) 21 'header-only-delete-header')
    $headerDeleteRejected = Wait-Diagnostic { param($d) $d.status.Contains('境界保護') } 5000 'header-only row-delete protection status'
    Add-DiagnosticTrace 'header-only-delete-protected' $headerDeleteRejected
    Compare-SourceHistorySelection $headerBeforeDelete $headerDeleteRejected 'Header-only row-delete protection'
    Assert-True ($headerDeleteRejected.tables -eq 4) 'Header-only row-delete protection changed the table count.'
    Add-Check 'R5-header-only-boundary' 'PASS' 'A one-row table accepted insertion of its first data row, then refused deleting the header through the native popup while source and history stayed unchanged.'

    $firstCellStart = $cellStart
    $topRightStart = Get-Utf8ByteOffset $script:BaseSource '同じ編集面で操作'
    $bottomRightStart = Get-Utf8ByteOffset $script:BaseSource '幅変更と前後の入力'
    $rectState = Get-Diagnostic
    $rectFirst = @($rectState.views | Where-Object { $_.table -eq 0 -and $_.source_start -eq $firstCellStart }) | Select-Object -First 1
    $rectTopRight = @($rectState.views | Where-Object { $_.table -eq 0 -and $_.source_start -eq $topRightStart }) | Select-Object -First 1
    $rectBottomRight = @($rectState.views | Where-Object { $_.table -eq 0 -and $_.source_start -eq $bottomRightStart }) | Select-Object -First 1
    Assert-True ($null -ne $rectFirst -and $null -ne $rectTopRight -and $null -ne $rectBottomRight) 'The 2x2 TSV source views were not mapped.'
    Click-ClientDip ($rectFirst.x + $rectFirst.width / 2.0) ($rectFirst.y + $rectFirst.height / 2.0)
    $rectCell = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $rectFirst.source_start -and $d.selection.head -le $rectFirst.source_end } 5000 'selecting the first TSV cell'
    Send-Key 0x24
    $rectStart = Wait-Diagnostic { param($d) $d.selection.anchor -eq $firstCellStart -and $d.selection.head -eq $firstCellStart } 3000 'cell Home before rectangular selection'
    Send-Key 0x27 @(0x10)
    $rectInsideFirst = Wait-Diagnostic { param($d) $d.selection.anchor -eq $firstCellStart -and $d.selection.head -gt $firstCellStart -and $d.selection.head -le $rectFirst.source_end } 3000 'extending across the first table-cell text'
    Send-Key 0x27 @(0x10)
    $rectRightCell = Wait-Diagnostic { param($d) $d.selection.anchor -eq $firstCellStart -and $d.selection.head -ge $rectTopRight.source_start -and $d.selection.head -le $rectTopRight.source_end } 3000 'extending from first to second cell'
    Send-Key 0x28 @(0x10)
    $rectSelection = Wait-Diagnostic { param($d) $d.selection.anchor -eq $firstCellStart -and $d.selection.head -ge $rectBottomRight.source_start -and $d.selection.head -le $rectBottomRight.source_end } 3000 'extending to a rectangular 2x2 cell range'
    Add-DiagnosticTrace 'rectangular-tsv-selection' $rectSelection
    $expectedTsv = "表`t同じ編集面で操作`n画像`t幅変更と前後の入力"
    $rectangleClipboardReady = $true
    try { Backup-Clipboard }
    catch { $rectangleClipboardReady = $false; Add-Check 'R5-rectangle-copy' 'BLOCKED' $_.Exception.Message }
    if ($rectangleClipboardReady -and (Test-TestClipboardSequence 'R5 rectangular copy')) {
        $beforeCopySequence = $script:ClipboardRestoreSequence
        Send-Key 0x43 @(0x11)
        $copySequence = Wait-ClipboardSequenceChange $beforeCopySequence 'Rectangular table Ctrl+C'
        if (Test-ClipboardTextValue $copySequence $expectedTsv 'R5-rectangle-copy' 'Rectangular table Ctrl+C') {
            $script:ClipboardRestoreSequence = $copySequence
            $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{ operation = 'rectangular-copy'; input = 'SendInput Ctrl+C'; dispatch = 'WindowProc WM_KEYDOWN → App::copy → Table::copy_tsv → native CF_UNICODETEXT' }
            if (Test-TestClipboardSequence 'R5 rectangular cut') {
                $beforeCut = Get-Diagnostic
                Send-Key 0x58 @(0x11)
                $cutSource = $script:BaseSource.Replace('| 表 | 同じ編集面で操作 |', '|  |  |').Replace('| 画像 | 幅変更と前後の入力 |', '|  |  |')
                $cut = Wait-Diagnostic { param($d) $d.source -ceq $cutSource } 5000 'rectangular table Ctrl+X source clearing'
                Add-DiagnosticTrace 'rectangular-cut' $cut
                Assert-True ($cut.undo_count -eq ($beforeCut.undo_count + 1) -and $cut.dirty -and $cut.tables -eq 4) 'Rectangular cut was not one bounded edit.'
                $cutSequence = Wait-ClipboardSequenceChange $copySequence 'Rectangular table Ctrl+X'
                if (Test-ClipboardTextValue $cutSequence $expectedTsv 'R5-rectangle-cut' 'Rectangular table Ctrl+X') {
                    $script:ClipboardRestoreSequence = $cutSequence
                    $cutBeforeUndo = $beforeCut
                    $cutAfter = $cut
                    [void](Complete-EditTransaction $cutBeforeUndo $cutAfter 'rectangular-cut')
                    $cutRestored = Restore-Clipboard
                    if ($cutRestored) { Add-Check 'R5-rectangle-cut' 'PASS' 'The rectangular 2x2 Ctrl+X copied exact OS TSV, cleared only those cells in one transaction, and restored source/clipboard through Undo/Redo.' }
                    $script:Report.command_pathways = @($script:Report.command_pathways) + [pscustomobject]@{ operation = 'rectangular-cut'; input = 'SendInput Ctrl+X'; dispatch = 'WindowProc WM_KEYDOWN → App::copy → Table::cut_tsv; clipboard write precedes one source transaction' }
                }
            }
        }
        else {
            [void](Restore-Clipboard)
        }
    }
    elseif ($rectangleClipboardReady) {
        Add-Check 'R5-rectangle-copy' 'BLOCKED' 'The clipboard sequence changed before Ctrl+C; no OS copy command was sent.'
    }
    if (@($script:Report.checks | Where-Object { $_.id -eq 'R5-rectangle-cut' }).Count -eq 0) {
        Add-Check 'R5-rectangle-cut' 'NOT_RUN' 'The rectangular cut was not attempted because its prerequisite copy and clipboard validation did not complete.'
    }

    $pasteCell = @((Get-Diagnostic).views | Where-Object { $_.table -eq 0 -and $_.source_start -eq $cellStart }) | Select-Object -First 1
    $pasteRow = '| 表 | 同じ編集面で操作 |'
    $pasteExpected = $script:BaseSource.Replace($pasteRow, '| TSV-L | TSV-R |')
    Invoke-OsTsvPaste $pasteCell "TSV-L`tTSV-R" $pasteExpected 'rectangular-tsv-paste'

    $boundaryCellStart = Get-Utf8ByteOffset $script:BaseSource '幅変更と前後の入力'
    $boundaryCell = @((Get-Diagnostic).views | Where-Object { $_.table -eq 0 -and $_.source_start -eq $boundaryCellStart }) | Select-Object -First 1
    Assert-True ($null -ne $boundaryCell) 'The bottom-right target cell for the overflow TSV was not mapped.'
    Invoke-OsTsvBoundaryCheck $boundaryCell "A`tB`nC`tD" 'overflow-tsv-rejection'

    $wideToken = '| c01 |'
    $wideStart = (Get-Utf8ByteOffset $script:BaseSource $wideToken) + 2
    $wideLayout = Scroll-ViewToY 2 $wideStart 260.0
    $wideCell = @($wideLayout.views | Where-Object { $_.table -eq 2 -and $_.source_start -eq $wideStart }) | Select-Object -First 1
    Assert-True ($null -ne $wideCell) 'The first cell of the wide table has no source-mapped view.'
    $clientRect = [MdliteRustGuiNative+Rect]::new()
    [void][MdliteRustGuiNative]::GetClientRect($script:WindowHandle, [ref]$clientRect)
    $dipScale = [double][MdliteRustGuiNative]::GetDpiForWindow($script:WindowHandle) / 96.0
    $availableTableWidth = $clientRect.Right / $dipScale - 44.0 * 2.0 - 12.0
    Assert-True ($wideLayout.table_geometry[2].width -gt $availableTableWidth) 'The fixture wide table does not exceed the available article width.'
    Click-ClientDip ($wideCell.x + $wideCell.width / 2.0) ($wideCell.y + $wideCell.height / 2.0)
    $wideSelected = Wait-Diagnostic { param($d) $d.selection.anchor -eq $d.selection.head -and $d.selection.head -ge $wideCell.source_start -and $d.selection.head -le $wideCell.source_end } 5000 'wide-table first-cell selection'
    $wideBeforeScroll = Get-Diagnostic
    $wideXBefore = [double](@($wideBeforeScroll.views | Where-Object { $_.table -eq 2 -and $_.source_start -eq $wideStart })[0].x)
    Send-Wheel -Delta 720 -Horizontal
    $wideScrolled = Wait-Diagnostic { param($d) @($d.views | Where-Object { $_.table -eq 2 -and $_.source_start -eq $wideStart }).Count -gt 0 -and [double](@($d.views | Where-Object { $_.table -eq 2 -and $_.source_start -eq $wideStart })[0].x) -lt ($wideXBefore - 100.0) } 5000 'table-local horizontal scroll'
    Compare-SourceHistorySelection $wideBeforeScroll $wideScrolled 'Wide-table local horizontal scroll'
    Assert-True ($wideScrolled.scroll -eq $wideBeforeScroll.scroll) 'Horizontal table scrolling moved the full document.'
    Send-Wheel -Delta -720 -Horizontal
    $wideReturned = Wait-Diagnostic { param($d) [Math]::Abs([double](@($d.views | Where-Object { $_.table -eq 2 -and $_.source_start -eq $wideStart })[0].x) - $wideXBefore) -lt 1.0 } 5000 'wide-table local scroll reset'
    Compare-SourceHistorySelection $wideBeforeScroll $wideReturned 'Wide-table scroll reset'
    Add-DiagnosticTrace 'table-local-horizontal-scroll' $wideReturned
    Add-Check 'R5-table-local-scroll' 'PASS' 'The 14-column table exceeded the available width; Shift+wheel changed its cell geometry while preserving source, selection and document scroll.'

    $beforeCtrlEnd = Get-Diagnostic
    $sourceLength = [Text.Encoding]::UTF8.GetByteCount($script:BaseSource)
    Send-Key 0x23 @(0x11)
    Start-Sleep -Milliseconds 100
    $ctrlEnd = Get-Diagnostic
    Add-DiagnosticTrace 'ctrl-end-navigation' $ctrlEnd
    Assert-True ($beforeCtrlEnd.source -ceq $ctrlEnd.source -and $beforeCtrlEnd.revision -eq $ctrlEnd.revision -and $beforeCtrlEnd.dirty -eq $ctrlEnd.dirty -and $beforeCtrlEnd.undo_count -eq $ctrlEnd.undo_count -and $beforeCtrlEnd.redo_count -eq $ctrlEnd.redo_count) 'Ctrl+End changed source or edit history.'
    if ($ctrlEnd.selection.anchor -eq $sourceLength -and $ctrlEnd.selection.head -eq $sourceLength) {
        Add-Check 'R4-ctrl-end-navigation' 'PASS' 'Ctrl+End moved the caret to the exact UTF-8 source end.'
    }
    else {
        Add-Check 'R4-ctrl-end-navigation' 'FAIL' "Ctrl+End did not reach source EOF $sourceLength; selection remained $($ctrlEnd.selection.anchor)..$($ctrlEnd.selection.head)."
    }

    $beforeEdit = Get-Diagnostic
    Send-Wheel -Delta -1200
    $lastParagraphVisible = Wait-Diagnostic { param($d) $d.scroll -gt $beforeEdit.scroll } 5000 'scrolling to the final paragraph'
    Compare-SourceHistorySelection $beforeEdit $lastParagraphVisible 'Scroll to the final paragraph'
    $endView = @($lastParagraphVisible.views | Where-Object { $null -eq $_.table -and $_.source_end -eq $sourceLength -and $_.map -contains $sourceLength }) | Select-Object -First 1
    Assert-True ($null -ne $endView) 'The final paragraph did not expose its exact source-end view map.'
    Click-ClientDip ($endView.x + [Math]::Min(320.0, $endView.width / 2.0)) ($endView.y + $endView.height / 2.0)
    $atEnd = Wait-Diagnostic { param($d) $d.selection.anchor -eq $sourceLength -and $d.selection.head -eq $sourceLength } 5000 'clicking the mapped final paragraph source end'
    Assert-True ($lastParagraphVisible.source -ceq $atEnd.source -and $lastParagraphVisible.revision -eq $atEnd.revision -and $lastParagraphVisible.dirty -eq $atEnd.dirty -and $lastParagraphVisible.undo_count -eq $atEnd.undo_count -and $lastParagraphVisible.redo_count -eq $atEnd.redo_count) 'Final paragraph hit testing changed source or edit history.'
    Add-DiagnosticTrace 'final-paragraph-caret' $atEnd
    Capture-Desktop '03-final-paragraph-caret.png' $atEnd | Out-Null
    Send-UnicodeText 'Z'
    $typed = Wait-Diagnostic { param($d) $d.source -ceq ($script:BaseSource + 'Z') } 5000 'normal WM_CHAR input'
    Add-DiagnosticTrace 'normal-input-typed' $typed
    Assert-True ($typed.undo_count -eq 1 -and $typed.redo_count -eq 0 -and $typed.dirty) 'One normal character was not one dirty undo transaction after branching history.'
    Send-Key 0x5A @(0x11)
    $undone = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource } 5000 'normal input Undo'
    Add-DiagnosticTrace 'normal-input-undo' $undone
    Assert-True (-not $undone.dirty -and $undone.undo_count -eq 0 -and $undone.redo_count -eq 1) 'Undo did not restore the clean fixture with exactly one redo entry.'
    Send-Key 0x59 @(0x11)
    $redone = Wait-Diagnostic { param($d) $d.source -ceq ($script:BaseSource + 'Z') } 5000 'normal input Redo'
    Add-DiagnosticTrace 'normal-input-redo' $redone
    Send-Key 0x5A @(0x11)
    $restored = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource -and -not $d.dirty } 5000 'normal input final rollback'
    Add-DiagnosticTrace 'normal-input-rollback' $restored
    Assert-True ($restored.undo_count -eq $atEnd.undo_count) 'Normal input rollback left an unexpected undo transaction.'
    Add-Check 'R4-normal-input' 'PASS' 'A normal SendInput character round-tripped through exact source, Undo, Redo and clean rollback.'

    $pasteToken = 'MDLITE_OS_CLIPBOARD'
    $clipboardReady = $true
    try { Save-ClipboardAndSetText $pasteToken }
    catch {
        $clipboardReady = $false
        Add-Check 'R4-clipboard' 'BLOCKED' $_.Exception.Message
    }
    if ($clipboardReady) {
        $script:Report.notes = @($script:Report.notes) + ("Clipboard formats backed up: " + ($script:ClipboardFormats -join ', '))
        Wait-Foreground
        if (Test-TestClipboardSequence 'R4-Ctrl+V') {
            Send-Key 0x56 @(0x11)
            $pasted = Wait-Diagnostic { param($d) $d.source -ceq ($script:BaseSource + $pasteToken) } 5000 'Ctrl+V from the Windows Unicode clipboard'
            Add-DiagnosticTrace 'clipboard-paste' $pasted
            Assert-True ($pasted.undo_count -eq 1 -and $pasted.redo_count -eq 0 -and $pasted.dirty) 'Ctrl+V did not create exactly one dirty transaction.'
            Send-Key 0x5A @(0x11)
            $pasteUndo = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource -and -not $d.dirty } 5000 'Ctrl+V Undo'
            Add-DiagnosticTrace 'clipboard-undo' $pasteUndo
            Send-Key 0x59 @(0x11)
            $pasteRedo = Wait-Diagnostic { param($d) $d.source -ceq ($script:BaseSource + $pasteToken) } 5000 'Ctrl+V Redo'
            Add-DiagnosticTrace 'clipboard-redo' $pasteRedo
            Send-Key 0x5A @(0x11)
            $clipboardRollback = Wait-Diagnostic { param($d) $d.source -ceq $script:BaseSource -and -not $d.dirty } 5000 'Ctrl+V final rollback'
            Add-DiagnosticTrace 'clipboard-rollback' $clipboardRollback
            $clipboardRestored = Restore-Clipboard
            Assert-True ($clipboardRollback.undo_count -eq $restored.undo_count) 'Ctrl+V rollback left an unexpected undo transaction.'
            if ($clipboardRestored) { Add-Check 'R4-clipboard' 'PASS' 'The Windows Unicode clipboard Ctrl+V transaction round-tripped exact source, Undo and Redo; every original clipboard format was restored.' }
        }
        else {
            Add-Check 'R4-clipboard' 'BLOCKED' 'The clipboard sequence no longer identifies the test-owned clipboard payload; no Ctrl+V was sent.'
        }
    }

    $ime = Select-MicrosoftImeProfile
    if (-not $ime.available) {
        $script:Report.ime = [ordered]@{ status = 'BLOCKED'; note = $ime.note; input_profile = $ime.profile; input_delivery = 'No IME composition keys were sent because the fresh native app profile identity/state gate did not pass.' }
        Add-Check 'R4-ime' 'BLOCKED' $ime.note
    }
    else {
        $script:Report.ime = Invoke-MicrosoftImeAttempt $ime
    }
    Restore-AppInputState
    if ($script:Report.ime.status -eq 'FAIL') { throw ('IME acceptance failed: ' + $script:Report.ime.note) }

    $stableBeforeRepeat = Get-Diagnostic
    $frames = @()
    try {
        Hold-Key 0x25 $false
        for ($index = 1; $index -le 4; $index++) {
            Start-Sleep -Milliseconds 240
            Hold-Key 0x25 $false
            $frameState = Get-Diagnostic
            if ($null -eq $frameState) { throw 'Repeat diagnostic JSON was unavailable after a timed synthetic Left key-down.' }
            Add-DiagnosticTrace ("synthetic-keydown-repeat-{0:D2}" -f $index) $frameState
            $frames += Capture-Desktop ("repeat-{0:D2}.png" -f $index) $frameState
        }
    }
    finally {
        $keyUp = [MdliteRustGuiNative]::Key(0x25, [MdliteRustGuiNative]::KEYEVENTF_KEYUP)
        $size = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
        $released = [MdliteRustGuiNative]::SendInput(1, [MdliteRustGuiNative+Input[]]@($keyUp), $size)
        if ($released -ne 1) { throw 'SendInput could not release the held repeat key during cleanup.' }
    }
    $repeatStates = @($frames | ForEach-Object { "$( $_.selection.anchor ):$( $_.selection.head )" } | Select-Object -Unique)
    $repeatFinal = Get-Diagnostic
    Assert-True ($repeatStates.Count -ge 3) "Timed synthetic SendInput Left KEYDOWN events produced only $($repeatStates.Count) distinct caret/selection samples."
    Assert-True ($repeatFinal.source -ceq $stableBeforeRepeat.source -and $repeatFinal.revision -eq $stableBeforeRepeat.revision -and $repeatFinal.undo_count -eq $stableBeforeRepeat.undo_count -and $repeatFinal.redo_count -eq $stableBeforeRepeat.redo_count -and -not $repeatFinal.dirty) 'Arrow repeat changed source, revision or edit history.'
    Add-Check 'R3' 'PASS' 'Four real desktop frames followed timed synthetic SendInput Left KEYDOWN events at 240 ms intervals; at least three distinct selections/caret positions were observed. Physical keyboard typematic remains unverified.'

    $beforeResize = Get-Diagnostic
    Add-DiagnosticTrace 'before-resize' $beforeResize
    $rectBefore = [MdliteRustGuiNative+Rect]::new()
    [void][MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$rectBefore)
    $currentWidth = $rectBefore.Right - $rectBefore.Left
    $screenWidth = [Windows.Forms.SystemInformation]::VirtualScreen.Width
    $delta = if ($rectBefore.Left + $currentWidth + 100 -lt $screenWidth) { 100 } else { -100 }
    $newWidth = [Math]::Max(500, $currentWidth + $delta)
    Assert-True ([MdliteRustGuiNative]::SetWindowPos($script:WindowHandle, [IntPtr]::Zero, $rectBefore.Left, $rectBefore.Top, $newWidth, ($rectBefore.Bottom - $rectBefore.Top), [MdliteRustGuiNative]::SWP_NOZORDER -bor [MdliteRustGuiNative]::SWP_NOACTIVATE)) 'SetWindowPos failed to trigger a real native resize.'
    Start-Sleep -Milliseconds 250
    $afterResize = Get-Diagnostic
    Add-DiagnosticTrace 'after-resize' $afterResize
    Compare-SourceHistorySelection $beforeResize $afterResize 'Resize'
    Capture-Desktop '02-after-resize.png' $afterResize | Out-Null

    Send-Key 0x45 @(0x11)
    $raw = Wait-Diagnostic { param($d) $d.raw } 5000 'raw/source display switch on'
    Add-DiagnosticTrace 'raw-source-view' $raw
    Compare-SourceHistorySelection $afterResize $raw 'Raw/source display switch on'
    Capture-Desktop '03-raw-source.png' $raw | Out-Null
    Send-Key 0x45 @(0x11)
    $liveAgain = Wait-Diagnostic { param($d) -not $d.raw } 5000 'raw/source display switch off'
    Add-DiagnosticTrace 'live-view-restored' $liveAgain
    Compare-SourceHistorySelection $raw $liveAgain 'Raw/source display switch off'
    Add-Check 'R2' 'PASS' 'Wheel scrolling, native window resize, and both raw/source display switches preserved exact source, revision, dirty state, undo/redo counts and selection.'

    Send-Key 0x4E @(0x11)
    $newTab = Wait-Diagnostic { param($d) $d.tabs -eq 2 -and $d.source -eq '' } 5000 'opening an untitled second document'
    Add-DiagnosticTrace 'new-unsaved-document' $newTab
    Assert-True $newTab.dirty 'A new untitled document did not require close protection.'
    $preCloseEvidence = Get-R7WindowEvidence $newTab
    Send-Key 0x57 @(0x11)
    $dialog = Wait-OwnedDialog 4000
    if ($dialog -eq [IntPtr]::Zero) { $script:Report.r7_timeout_evidence = Get-R7TimeoutEvidence $preCloseEvidence 4000 }
    Assert-True ($dialog -ne [IntPtr]::Zero) 'Ctrl+W did not show the dirty-document close confirmation.'
    [void][MdliteRustGuiNative]::SetForegroundWindow($dialog)
    Start-Sleep -Milliseconds 100
    if ([MdliteRustGuiNative]::GetForegroundWindow() -eq $dialog) { Send-Key 0x4E }
    $afterClose = Wait-Diagnostic { param($d) $d.tabs -eq 1 -and $d.source -ceq $script:BaseSource } 5000 'discarding only the test untitled document'
    Add-DiagnosticTrace 'unsaved-close-preserved-original' $afterClose
    Assert-AppAlive
    Add-Check 'R7' 'PASS' 'A second unsaved document prompted on close; declining save closed that document and retained the original source in the responsive app.'

    Capture-Desktop '04-final-live-editor.png' $afterClose | Out-Null
}

function New-UnknownInputProfile([string]$Reason) {
    return [pscustomobject]@{ status = 'unknown'; active = $false; profile_type = 'unknown'; profile_type_value = $null; language_id = $null; clsid = $null; profile_guid = $null; category_guid = $null; hkl = $null; substitute_hkl = $null; description = $null; diagnostic_fresh = $false; diagnostic_refreshed_utc = $null; diagnostic_error = $Reason }
}

function Get-ActiveInputProfile {
    $script:LastOwnedDiagnostic = $null
    $script:LastOwnedDiagnosticRefreshUtc = $null
    if ($null -eq $script:Process -or $script:WindowHandle -eq [IntPtr]::Zero -or [string]::IsNullOrWhiteSpace([string]$script:DiagnosticPath)) { return (New-UnknownInputProfile 'The owned app diagnostic window/path is unavailable.') }
    $script:Process.Refresh()
    if ($script:Process.HasExited -or $script:Process.MainWindowHandle -ne $script:WindowHandle -or -not [MdliteRustGuiNative]::IsWindow($script:WindowHandle)) { return (New-UnknownInputProfile 'The exact launched app PID/HWND is unavailable for native profile observation.') }
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$ownerPid)
    if ($ownerPid -ne [uint32]$script:Process.Id) { return (New-UnknownInputProfile 'The owned app diagnostic HWND PID changed before native profile observation.') }
    if (-not (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf)) { return (New-UnknownInputProfile 'The owned app diagnostic file does not exist.') }
    $beforeWriteTicks = [IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath).Ticks
    if (-not [MdliteRustGuiNative]::InvalidateRect($script:WindowHandle, [IntPtr]::Zero, $false)) {
        return (New-UnknownInputProfile ('Normal owned-window repaint request failed (Win32 ' + [Runtime.InteropServices.Marshal]::GetLastWin32Error() + ').'))
    }
    $deadline = [DateTime]::UtcNow.AddMilliseconds(1500)
    do {
        if ([IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath).Ticks -gt $beforeWriteTicks) {
            $diagnostic = Get-Diagnostic
            if ($null -ne $diagnostic) {
                $script:LastOwnedDiagnostic = $diagnostic
                $script:LastOwnedDiagnosticRefreshUtc = [DateTime]::UtcNow.ToString('o')
            }
            $profileProperty = if ($null -eq $diagnostic) { $null } else { $diagnostic.PSObject.Properties['active_input_profile'] }
            if ($null -ne $profileProperty) {
                if ($null -eq $profileProperty.Value) { return (New-UnknownInputProfile 'The fresh native app diagnostic reported a null active_input_profile.') }
                $record = [ordered]@{}
                foreach ($property in $profileProperty.Value.PSObject.Properties) { $record[$property.Name] = $property.Value }
                $record.diagnostic_fresh = $true
                $record.diagnostic_refreshed_utc = $script:LastOwnedDiagnosticRefreshUtc
                $record.diagnostic_error = $null
                return [pscustomobject]$record
            }
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    return (New-UnknownInputProfile 'The normal owned-window repaint did not publish a fresh active_input_profile within 1500 ms.')
}

function Get-InputProfileKey($Profile) {
    if ($null -eq $Profile -or $Profile.diagnostic_fresh -ne $true -or $Profile.status -ne 'active' -or $Profile.active -ne $true) { return $null }
    $typeName = ([string]$Profile.profile_type).ToLowerInvariant()
    $typeValue = [string]$Profile.profile_type_value
    $language = ([string]$Profile.language_id).ToLowerInvariant()
    $clsid = ([string]$Profile.clsid).Replace('{', '').Replace('}', '').ToLowerInvariant()
    $profileGuid = ([string]$Profile.profile_guid).Replace('{', '').Replace('}', '').ToLowerInvariant()
    if ([string]::IsNullOrWhiteSpace($typeValue) -or [string]::IsNullOrWhiteSpace($language) -or [string]::IsNullOrWhiteSpace($clsid) -or [string]::IsNullOrWhiteSpace($profileGuid)) { return $null }
    $key = "$typeName|$typeValue|$clsid|$profileGuid|$language"
    if ($typeName -eq 'text_service' -and $typeValue -eq '1') { return $key }
    if ($typeName -eq 'keyboard_layout' -and $typeValue -eq '2' -and -not [string]::IsNullOrWhiteSpace([string]$Profile.hkl)) { return ($key + '|hkl=' + ([string]$Profile.hkl).ToLowerInvariant()) }
    return $null
}

function Test-MicrosoftImeProfile($Profile) {
    return ($null -ne $Profile -and $Profile.diagnostic_fresh -eq $true -and $Profile.status -eq 'active' -and $Profile.active -eq $true -and $Profile.profile_type -eq 'text_service' -and $Profile.profile_type_value -eq 1 -and $Profile.language_id -eq '0x0411' -and $null -eq $Profile.description_error_hresult -and [string]$Profile.description -match '(?i)^\s*Microsoft IME\s*$')
}

function Add-InputProfileTrace([string]$Stage, $Profile, [int]$Cycle = 0) {
    $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$foregroundPid = 0
    if ($foreground -ne [IntPtr]::Zero) { [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid) }
    $script:Report.input_profile.trace = @($script:Report.input_profile.trace) + [pscustomobject]@{
        stage = $Stage
        cycle = $Cycle
        target_window = if ($script:WindowHandle -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $script:WindowHandle.ToInt64() }
        target_pid = if ($null -eq $script:Process) { $null } else { $script:Process.Id }
        foreground_window = if ($foreground -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $foreground.ToInt64() }
        foreground_pid = if ($foreground -eq [IntPtr]::Zero) { $null } else { $foregroundPid }
        foreground_is_exact_owned_main = ($foreground -eq $script:WindowHandle -and $null -ne $script:Process -and $foregroundPid -eq [uint32]$script:Process.Id)
        available = ($Profile.diagnostic_fresh -eq $true -and $Profile.status -eq 'active' -and $Profile.active -eq $true)
        identity = Get-InputProfileKey $Profile
        profile_type = $Profile.profile_type
        profile_type_value = $Profile.profile_type_value
        clsid = $Profile.clsid
        profile_guid = $Profile.profile_guid
        language_id = $Profile.language_id
        hkl = $Profile.hkl
        substitute_hkl = $Profile.substitute_hkl
        description = $Profile.description
        category_guid = $Profile.category_guid
        diagnostic_fresh = $Profile.diagnostic_fresh
        diagnostic_refreshed_utc = $Profile.diagnostic_refreshed_utc
        diagnostic_error = $Profile.diagnostic_error
    }
}

function Get-AppImeOpenState($Profile) {
    $observation = [ordered]@{
        available = $false
        open = $null
        error = 'fresh_owned_app_diagnostic_unavailable'
        diagnostic_fresh = $false
        diagnostic_refreshed_utc = $null
        input_profile_identity = $null
    }
    $script:LastImeOpenObservation = [pscustomobject]$observation
    if ($null -eq $script:Process -or $script:WindowHandle -eq [IntPtr]::Zero -or $null -eq $Profile -or $Profile.diagnostic_fresh -ne $true -or [string]::IsNullOrWhiteSpace([string]$script:LastOwnedDiagnosticRefreshUtc) -or $Profile.diagnostic_refreshed_utc -cne $script:LastOwnedDiagnosticRefreshUtc) { return $null }
    $diagnostic = $script:LastOwnedDiagnostic
    if ($null -eq $diagnostic) { return $null }
    $observation.diagnostic_fresh = $true
    $observation.diagnostic_refreshed_utc = $Profile.diagnostic_refreshed_utc
    $observation.input_profile_identity = Get-InputProfileKey $Profile
    $diagnosticProfileProperty = $diagnostic.PSObject.Properties['active_input_profile']
    $imeProperty = $diagnostic.PSObject.Properties['ime']
    $openStatusProperty = if ($null -eq $imeProperty -or $null -eq $imeProperty.Value) { $null } else { $imeProperty.Value.PSObject.Properties['open_status'] }
    if ($null -eq $diagnosticProfileProperty -or $null -eq $diagnosticProfileProperty.Value -or $null -eq $openStatusProperty -or $null -eq $openStatusProperty.Value) {
        $observation.error = 'native_profile_or_open_status_missing_from_fresh_diagnostic'
        $script:LastImeOpenObservation = [pscustomobject]$observation
        return $null
    }
    $nativeOpenStatus = $openStatusProperty.Value
    $availableProperty = $nativeOpenStatus.PSObject.Properties['available']
    $openProperty = $nativeOpenStatus.PSObject.Properties['open']
    $errorProperty = $nativeOpenStatus.PSObject.Properties['error']
    if ($null -eq $availableProperty -or $null -eq $openProperty -or $null -eq $errorProperty) {
        $observation.error = 'native_open_status_contract_incomplete'
        $script:LastImeOpenObservation = [pscustomobject]$observation
        return $null
    }
    $observation.available = ($availableProperty.Value -is [bool] -and [bool]$availableProperty.Value)
    $observation.open = if ($openProperty.Value -is [bool]) { [bool]$openProperty.Value } else { $null }
    $observation.error = $errorProperty.Value
    $rawProfile = [ordered]@{}
    foreach ($property in $diagnosticProfileProperty.Value.PSObject.Properties) { $rawProfile[$property.Name] = $property.Value }
    $rawProfile.diagnostic_fresh = $true
    $rawIdentity = Get-InputProfileKey ([pscustomobject]$rawProfile)
    if ([string]::IsNullOrWhiteSpace([string]$observation.input_profile_identity) -or -not [string]::Equals($observation.input_profile_identity, $rawIdentity, [StringComparison]::OrdinalIgnoreCase)) {
        $observation.available = $false
        $observation.open = $null
        $observation.error = 'profile_identity_did_not_match_same_fresh_diagnostic'
    }
    $script:LastImeOpenObservation = [pscustomobject]$observation
    if ($observation.available -ne $true -or $observation.open -isnot [bool] -or $null -ne $observation.error) { return $null }
    return [bool]$observation.open
}

function Assert-OwnedAppMainForeground {
    if ($null -eq $script:Process) { throw 'No launched app process is registered for profile input.' }
    Assert-OwnedWindowIdentity $script:Process $script:WindowHandle
    $foreground = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$windowPid = 0
    [uint32]$foregroundPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$windowPid)
    if ($foreground -ne [IntPtr]::Zero) { [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foreground, [ref]$foregroundPid) }
    if ($windowPid -ne [uint32]$script:Process.Id -or $foreground -ne $script:WindowHandle -or $foregroundPid -ne [uint32]$script:Process.Id) { throw 'The profile input foreground HWND/PID is not the exact launched app main window.' }
}

function Open-ConfirmedMicrosoftIme($Profile) {
    $profileKey = Get-InputProfileKey $Profile
    if ([string]::IsNullOrWhiteSpace($profileKey) -or -not (Test-MicrosoftImeProfile $Profile)) { throw 'The native IME-open key was refused because exact Japanese Microsoft IME identity was not confirmed.' }
    $openState = Get-AppImeOpenState $Profile
    if ($null -eq $openState) { return $false }
    if (-not $openState) {
        Assert-OwnedAppMainForeground
        $null = Assert-InputProfileIdentity $profileKey 'Confirmed Microsoft IME before native IME-on'
        $script:InputProfileStateMayHaveChanged = $true
        Send-Key ([int][MdliteRustGuiNative]::VK_IME_ON)
        Start-Sleep -Milliseconds 100
        $profileAfterOpen = Assert-InputProfileIdentity $profileKey 'Confirmed Microsoft IME after native IME-on'
        $openState = Get-AppImeOpenState $profileAfterOpen
    }
    return ($openState -eq $true)
}

function Assert-InputProfileIdentity([string]$ExpectedIdentity, [string]$Stage) {
    $profile = Get-ActiveInputProfile
    Add-InputProfileTrace $Stage $profile
    $actualIdentity = Get-InputProfileKey $profile
    if ([string]::IsNullOrWhiteSpace($actualIdentity) -or -not [string]::Equals($actualIdentity, $ExpectedIdentity, [StringComparison]::OrdinalIgnoreCase)) {
        throw "$Stage did not retain the confirmed native app profile identity; expected $ExpectedIdentity, got '$actualIdentity'."
    }
    return $profile
}

function Send-NormalWinSpaceCycle($Before, [string]$Label) {
    Assert-OwnedAppMainForeground
    $script:InputProfileStateMayHaveChanged = $true
    $events = [MdliteRustGuiNative+Input[]]@(
        [MdliteRustGuiNative]::Key([MdliteRustGuiNative]::VK_LWIN, 0),
        [MdliteRustGuiNative]::Key([MdliteRustGuiNative]::VK_SPACE, 0),
        [MdliteRustGuiNative]::Key([MdliteRustGuiNative]::VK_SPACE, [MdliteRustGuiNative]::KEYEVENTF_KEYUP),
        [MdliteRustGuiNative]::Key([MdliteRustGuiNative]::VK_LWIN, [MdliteRustGuiNative]::KEYEVENTF_KEYUP)
    )
    try {
        Send-Inputs $events
        Start-Sleep -Milliseconds 120
    }
    finally {
        $releaseErrors = [Collections.Generic.List[string]]::new()
        $size = [Runtime.InteropServices.Marshal]::SizeOf([type][MdliteRustGuiNative+Input])
        foreach ($virtualKey in @([int][MdliteRustGuiNative]::VK_SPACE, [int][MdliteRustGuiNative]::VK_LWIN)) {
            try {
                $keyUp = [MdliteRustGuiNative]::Key([uint16]$virtualKey, [MdliteRustGuiNative]::KEYEVENTF_KEYUP)
                $released = [MdliteRustGuiNative]::SendInput(1, [MdliteRustGuiNative+Input[]]@($keyUp), $size)
                if ($released -ne 1) { $releaseErrors.Add(('VK 0x{0:X2} key-up delivered {1} of 1 events.' -f $virtualKey, $released)) }
            }
            catch { $releaseErrors.Add(('VK 0x{0:X2} key-up failed: {1}' -f $virtualKey, $_.Exception.Message)) }
        }
        if ($releaseErrors.Count -gt 0) { throw ('Unconditional Win+Space key release failed: ' + ($releaseErrors -join '; ')) }
    }
    Assert-OwnedAppMainForeground
    $after = Get-Diagnostic
    Compare-SourceHistorySelection $Before $after $Label
    return $after
}

function Select-MicrosoftImeProfile {
    $original = Get-ActiveInputProfile
    $script:OriginalInputProfile = $original
    $script:OriginalImeOpen = Get-AppImeOpenState $original
    $script:Report.actual_active_input_profile = $original
    $script:Report.input_profile.original = $original
    $script:Report.input_profile.original_ime_open = $script:OriginalImeOpen
    $script:Report.input_profile.original_ime_open_observation = $script:LastImeOpenObservation
    Add-InputProfileTrace 'original' $original
    $originalKey = Get-InputProfileKey $original
    if ([string]::IsNullOrWhiteSpace($originalKey)) {
        return [pscustomobject]@{ available = $false; note = 'A fresh owned-app active_input_profile diagnostic could not establish the original profile identity; no Win+Space cycles or IME keys were sent.'; profile = $null }
    }
    if ($null -eq $script:OriginalImeOpen) {
        return [pscustomobject]@{ available = $false; note = 'The app IME-open state could not be read, so the original profile/open-state pair was not changed.'; profile = $original }
    }
    if (Test-MicrosoftImeProfile $original) {
        $script:Report.input_profile.target = $original
        if (-not (Open-ConfirmedMicrosoftIme $original)) {
            return [pscustomobject]@{ available = $false; note = 'The active Microsoft IME identity was confirmed, but its existing app IME context could not be read or opened; exact restoration will be checked before close.'; profile = $original }
        }
        return [pscustomobject]@{ available = $true; note = 'The fresh owned-app diagnostic confirms an input-processor profile described as Microsoft IME for language 0x0411 is already active.'; profile = $original }
    }

    $visited = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    [void]$visited.Add($originalKey)
    $note = 'Microsoft IME with language 0x0411 was not found in the bounded active-profile cycle.'
    $cycleBoundReached = $true
    for ($cycle = 1; $cycle -le 8; $cycle++) {
        $beforeCycle = Get-Diagnostic
        $null = Send-NormalWinSpaceCycle $beforeCycle "Win+Space Microsoft IME selection cycle $cycle"
        $current = Get-ActiveInputProfile
        $script:Report.input_profile.selection_cycles = $cycle
        Add-InputProfileTrace 'select-cycle' $current $cycle
        if (-not (Get-InputProfileKey $current)) {
            $note = 'The fresh owned-app profile identity became unavailable during Win+Space selection; exact restoration is required before close.'
            $cycleBoundReached = $false
            break
        }
        if (Test-MicrosoftImeProfile $current) {
            $script:Report.input_profile.target = $current
            if (-not (Open-ConfirmedMicrosoftIme $current)) {
                $note = 'Microsoft IME identity was confirmed, but its existing app IME context could not be read or opened; restoring the original profile.'
                Restore-AppInputState
                return [pscustomobject]@{ available = $false; note = $note; profile = $current }
            }
            return [pscustomobject]@{ available = $true; note = 'The fresh owned-app diagnostic confirms a Microsoft IME input-processor profile for language 0x0411 after normal Win+Space selection.'; profile = $current }
        }
        $key = Get-InputProfileKey $current
        if (-not $visited.Add($key)) {
            $note = 'The active-profile identity repeated before Microsoft IME was found; the original identity will be restored through normal Win+Space cycles.'
            $cycleBoundReached = $false
            break
        }
    }

    if ($cycleBoundReached) {
        $note = 'The active-profile cycle reached its eight-cycle bound before Microsoft IME was confirmed; the original identity will be restored through normal Win+Space cycles.'
    }
    Restore-AppInputState
    return [pscustomobject]@{ available = $false; note = $note; profile = $original }
}

function Get-ImeCandidateGeometry {
    $context = [MdliteRustGuiNative]::ImmGetContext($script:WindowHandle)
    if ($context -eq [IntPtr]::Zero) { return [pscustomobject]@{ available = $false; reason = 'ImmGetContext returned null.' } }
    try {
        $form = [MdliteRustGuiNative+CandidateForm]::new()
        $available = [MdliteRustGuiNative]::ImmGetCandidateWindow($context, 0, [ref]$form)
        [pscustomobject]@{
            available = $available
            index = $form.Index
            style = $form.Style
            current_position = [pscustomobject]@{ x = $form.CurrentPos.X; y = $form.CurrentPos.Y }
            area = [pscustomobject]@{ left = $form.Area.Left; top = $form.Area.Top; right = $form.Area.Right; bottom = $form.Area.Bottom }
            coordinate_note = 'IMM CANDIDATEFORM values are logged as returned for index 0; task-local app and owned-candidate window crops are separate visual evidence.'
        }
    }
    finally { [void][MdliteRustGuiNative]::ImmReleaseContext($script:WindowHandle, $context) }
}

function Restore-AppInputState {
    if ($null -eq $script:OriginalInputProfile) { return }
    if ($script:Report.input_profile.ime_open_restore_status -in @('BLOCKED_PENDING_COMPOSITION', 'BLOCKED_COMPOSITION_STATE_UNKNOWN')) {
        throw 'Input-profile restoration remains blocked by pending or unknown IME composition state; normal close is withheld.'
    }
    $guardProfile = $null
    $guardDiagnostic = $null
    $guardReadError = $null
    try {
        $guardProfile = Get-ActiveInputProfile
        $guardDiagnostic = $script:LastOwnedDiagnostic
    }
    catch { $guardReadError = $_.Exception.Message }
    $guardIme = if ($null -eq $guardDiagnostic) { $null } else { $guardDiagnostic.PSObject.Properties['ime'] }
    $guardActive = if ($null -eq $guardIme -or $null -eq $guardIme.Value) { $null } else { $guardIme.Value.PSObject.Properties['active'] }
    $guardPreedit = if ($null -eq $guardIme -or $null -eq $guardIme.Value) { $null } else { $guardIme.Value.PSObject.Properties['preedit'] }
    $guardFresh = ($null -ne $guardDiagnostic -and $null -ne $guardProfile -and $guardProfile.diagnostic_fresh -eq $true -and -not [string]::IsNullOrWhiteSpace([string]$script:LastOwnedDiagnosticRefreshUtc) -and $guardProfile.diagnostic_refreshed_utc -ceq $script:LastOwnedDiagnosticRefreshUtc)
    $guardActiveKnown = ($null -ne $guardActive -and $guardActive.Value -is [bool])
    $guardPreeditKnown = ($null -ne $guardPreedit -and $guardPreedit.Value -is [string])
    $guardCompositionActive = if ($guardActiveKnown) { [bool]$guardActive.Value } else { $null }
    $guardPreeditText = if ($guardPreeditKnown) { [string]$guardPreedit.Value } else { $null }
    $guardStatus = 'BLOCKED_COMPOSITION_STATE_UNKNOWN'
    if ($guardFresh -and $guardActiveKnown -and $guardPreeditKnown) {
        $guardStatus = if ($guardCompositionActive -or $guardPreeditText.Length -gt 0) { 'BLOCKED_PENDING_COMPOSITION' } else { 'CLEAR' }
    }
    $script:Report.input_profile.pending_composition = [pscustomobject]@{
        status = $guardStatus
        diagnostic_fresh = [bool]$guardFresh
        diagnostic_refreshed_utc = if ($null -eq $guardProfile) { $null } else { $guardProfile.diagnostic_refreshed_utc }
        profile_identity = Get-InputProfileKey $guardProfile
        process_id = if ($null -eq $script:Process) { $null } else { $script:Process.Id }
        window_handle = if ($script:WindowHandle -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $script:WindowHandle.ToInt64() }
        ime_active = $guardCompositionActive
        preedit_length = if ($guardPreeditKnown) { $guardPreeditText.Length } else { $null }
        document_state = if ($null -eq $guardDiagnostic) { $null } else { Get-DiagnosticStateSummary $guardDiagnostic }
        diagnostic_error = $guardReadError
    }
    if ($guardStatus -ne 'CLEAR') {
        $script:Report.input_profile.restore_status = 'BLOCKED'
        $script:Report.input_profile.profile_identity_restore_status = 'BLOCKED'
        $script:Report.input_profile.ime_open_restore_status = $guardStatus
        $failure = if ($guardStatus -eq 'BLOCKED_PENDING_COMPOSITION') { 'A fresh owned diagnostic shows active IME composition or nonempty preedit; no profile cycle or IME on/off restore key was sent and normal close is withheld.' } else { 'A fresh owned diagnostic could not establish native IME composition/preedit state; no profile cycle or IME on/off restore key was sent and normal close is withheld.' }
        Add-Check 'R4-input-profile-restore' 'BLOCKED' $failure
        $script:Report.notes = @($script:Report.notes) + $failure
        throw $failure
    }
    if ($script:InputProfileRestoreAttempted) {
        if ($script:Report.input_profile.restore_status -eq 'NOT_REQUIRED_ORIGINAL_IDENTITY_UNKNOWN') { return }
        if ($script:Report.input_profile.restore_status -in @('PASS', 'PARTIAL_OPEN_STATE_UNKNOWN')) {
            $finalProfile = $null
            $revalidationFailure = $null
            try {
                $finalProfile = Get-ActiveInputProfile
                $finalKey = Get-InputProfileKey $finalProfile
                $originalKey = Get-InputProfileKey $script:OriginalInputProfile
                $finalOpen = Get-AppImeOpenState $finalProfile
                $script:Report.input_profile.final_ime_open_observation = $script:LastImeOpenObservation
                if (-not [string]::Equals($finalKey, $originalKey, [StringComparison]::OrdinalIgnoreCase) -or ($null -ne $script:OriginalImeOpen -and $finalOpen -ne $script:OriginalImeOpen) -or ($null -eq $script:OriginalImeOpen -and $script:InputProfileStateMayHaveChanged)) {
                    $revalidationFailure = 'The previously restored active input profile or IME-open state changed before close.'
                }
            }
            catch { $revalidationFailure = $_.Exception.Message }
            if ($null -ne $revalidationFailure) {
                $script:Report.input_profile.restore_status = 'BLOCKED'
                $script:Report.input_profile.profile_identity_restore_status = 'BLOCKED'
                $script:Report.input_profile.ime_open_restore_status = 'BLOCKED'
                $failure = "$revalidationFailure No further profile cycles were sent and normal close is withheld."
                Add-Check 'R4-input-profile-restore' 'BLOCKED' $failure
                $script:Report.notes = @($script:Report.notes) + $failure
                throw $failure
            }
            $script:Report.input_profile.final = $finalProfile
            $script:Report.input_profile.profile_identity_restore_status = 'PASS'
            if ($null -eq $script:OriginalImeOpen) {
                $script:Report.input_profile.restore_status = 'PARTIAL_OPEN_STATE_UNKNOWN'
                $script:Report.input_profile.ime_open_restore_status = 'NOT_REQUIRED_ORIGINAL_UNKNOWN_NO_STATE_INPUT_SENT'
            }
            Add-InputProfileTrace 'final-readback' $finalProfile
            return
        }
        throw 'Input-profile restoration was already attempted and blocked; no further profile cycles were sent.'
    }
    $script:InputProfileRestoreAttempted = $true
    $originalKey = Get-InputProfileKey $script:OriginalInputProfile
    if ([string]::IsNullOrWhiteSpace($originalKey)) {
        if (-not $script:InputProfileStateMayHaveChanged) {
            $script:Report.input_profile.restore_status = 'NOT_REQUIRED_ORIGINAL_IDENTITY_UNKNOWN'
            $script:Report.input_profile.profile_identity_restore_status = 'NOT_REQUIRED_ORIGINAL_IDENTITY_UNKNOWN'
            $script:Report.input_profile.ime_open_restore_status = 'NOT_REQUIRED_ORIGINAL_UNKNOWN_NO_STATE_INPUT_SENT'
            return
        }
        $script:Report.input_profile.restore_status = 'BLOCKED'
        $script:Report.input_profile.profile_identity_restore_status = 'BLOCKED'
        $script:Report.input_profile.ime_open_restore_status = 'BLOCKED_ORIGINAL_IDENTITY_UNKNOWN_AFTER_STATE_CHANGE'
        $failure = 'The original active input profile identity is unknown after profile state may have changed; no unverified profile cycles were sent and normal close is withheld.'
        Add-Check 'R4-input-profile-restore' 'BLOCKED' $failure
        throw $failure
    }

    try {
        $current = Get-ActiveInputProfile
        Add-InputProfileTrace 'before-restore' $current
        $currentKey = Get-InputProfileKey $current
        $currentIsOriginal = [string]::Equals($currentKey, $originalKey, [StringComparison]::OrdinalIgnoreCase)
        if ($null -eq $script:OriginalImeOpen -and (-not $currentIsOriginal -or $script:InputProfileStateMayHaveChanged)) {
            $script:Report.input_profile.restore_status = 'BLOCKED'
            $script:Report.input_profile.profile_identity_restore_status = if ($currentIsOriginal) { 'PASS' } else { 'BLOCKED' }
            $script:Report.input_profile.ime_open_restore_status = 'BLOCKED_ORIGINAL_UNKNOWN_AFTER_STATE_CHANGE'
            throw 'The original app IME-open state is unknown and the current profile differs or state may have changed; no further profile or IME input was sent and normal close is withheld.'
        }
        $visited = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        if ($currentKey) { [void]$visited.Add($currentKey) }
        $cycles = 0
        while (-not [string]::Equals($currentKey, $originalKey, [StringComparison]::OrdinalIgnoreCase) -and $cycles -lt 8) {
            $beforeCycle = Get-Diagnostic
            $cycles++
            $null = Send-NormalWinSpaceCycle $beforeCycle "Win+Space original-profile restoration cycle $cycles"
            $current = Get-ActiveInputProfile
            Add-InputProfileTrace 'restore-cycle' $current $cycles
            $currentKey = Get-InputProfileKey $current
            if ($currentKey -and [string]::Equals($currentKey, $originalKey, [StringComparison]::OrdinalIgnoreCase)) { break }
            if ($currentKey -and -not $visited.Add($currentKey)) { break }
        }
        $script:Report.input_profile.restore_cycles = $cycles
        if (-not [string]::Equals($currentKey, $originalKey, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'The original CLSID/profile GUID/language identity was not reached within eight guarded Win+Space cycles.'
        }

        $openState = Get-AppImeOpenState $current
        $script:Report.input_profile.final_ime_open_observation = $script:LastImeOpenObservation
        if ($null -ne $script:OriginalImeOpen) {
            if ($null -eq $openState) { throw 'The restored profile IME-open state could not be read back.' }
            if ($openState -ne $script:OriginalImeOpen) {
                Assert-OwnedAppMainForeground
                $null = Assert-InputProfileIdentity $originalKey 'Original profile before IME-open restoration'
                $script:InputProfileStateMayHaveChanged = $true
                $key = if ($script:OriginalImeOpen) { [int][MdliteRustGuiNative]::VK_IME_ON } else { [int][MdliteRustGuiNative]::VK_IME_OFF }
                Send-Key $key
                Start-Sleep -Milliseconds 100
                $restoredProfile = Assert-InputProfileIdentity $originalKey 'Original profile after IME-open restoration'
                $openState = Get-AppImeOpenState $restoredProfile
                $script:Report.input_profile.final_ime_open_observation = $script:LastImeOpenObservation
                if ($openState -ne $script:OriginalImeOpen) { throw 'The original IME-open state did not match after the native restore key.' }
            }
        }
        $finalProfile = Get-ActiveInputProfile
        $finalKey = Get-InputProfileKey $finalProfile
        $finalOpen = Get-AppImeOpenState $finalProfile
        $script:Report.input_profile.final_ime_open_observation = $script:LastImeOpenObservation
        if (-not [string]::Equals($finalKey, $originalKey, [StringComparison]::OrdinalIgnoreCase)) { throw 'The final fresh owned-app profile readback did not match the original identity.' }
        if ($null -ne $script:OriginalImeOpen -and $finalOpen -ne $script:OriginalImeOpen) { throw 'The final app IME-open readback did not match the original state.' }
        $script:Report.input_profile.final = $finalProfile
        Add-InputProfileTrace 'final-readback' $finalProfile
        if ($null -eq $script:OriginalImeOpen) {
            $script:Report.input_profile.restore_status = 'PARTIAL_OPEN_STATE_UNKNOWN'
            $script:Report.input_profile.profile_identity_restore_status = 'PASS'
            $script:Report.input_profile.ime_open_restore_status = 'NOT_REQUIRED_ORIGINAL_UNKNOWN_NO_STATE_INPUT_SENT'
            $script:Report.notes = @($script:Report.notes) + 'Restored the exact original native input profile identity. The app IME-open state remained UNKNOWN; the harness sent no profile-switch or IME input, so exact open-state restoration is not claimed.'
        }
        else {
            $script:Report.input_profile.restore_status = 'PASS'
            $script:Report.input_profile.profile_identity_restore_status = 'PASS'
            $script:Report.input_profile.ime_open_restore_status = 'PASS'
            $script:Report.notes = @($script:Report.notes) + 'Restored the exact original native profile type/CLSID/profile GUID/language/keyboard-layout HKL identity and app IME-open state through guarded Win+Space/native IME keys before close.'
        }
    }
    catch {
        $script:Report.input_profile.restore_status = 'BLOCKED'
        if ($script:Report.input_profile.ime_open_restore_status -ne 'BLOCKED_ORIGINAL_UNKNOWN_AFTER_STATE_CHANGE') {
            $script:Report.input_profile.profile_identity_restore_status = 'BLOCKED'
            $script:Report.input_profile.ime_open_restore_status = 'BLOCKED'
        }
        Add-Check 'R4-input-profile-restore' 'BLOCKED' $_.Exception.Message
        throw
    }
}

function Send-RomanLetters([string]$Letters) {
    $events = [Collections.Generic.List[MdliteRustGuiNative+Input]]::new()
    foreach ($letter in $Letters.ToUpperInvariant().ToCharArray()) {
        $key = [int][char]$letter
        $events.Add([MdliteRustGuiNative]::Key([uint16]$key, 0))
        $events.Add([MdliteRustGuiNative]::Key([uint16]$key, [MdliteRustGuiNative]::KEYEVENTF_KEYUP))
    }
    if ($events.Count -gt 0) { Send-Inputs $events.ToArray() }
}

function Wait-GuardedImeState([string]$ExpectedIdentity, $Original, [scriptblock]$Predicate, [string]$Label) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds(3000)
    do {
        $null = Assert-InputProfileIdentity $ExpectedIdentity "$Label profile"
        $state = $script:LastOwnedDiagnostic
        Assert-True ($null -ne $state) "$Label had no fresh owned-app diagnostic."
        Compare-SourceHistorySelection $Original $state $Label
        if (& $Predicate $state) { return $state }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for $Label."
}

function Invoke-GuardedImeEscapeCancellation([string]$ExpectedIdentity, $Original, $StateBeforeCancel, [string]$Label, [string]$Reading) {
    $beforeProfile = Assert-InputProfileIdentity $ExpectedIdentity "$Label profile before Escape"
    $before = $script:LastOwnedDiagnostic
    Assert-True ($null -ne $before) "$Label had no fresh owned-app diagnostic before Escape."
    Compare-SourceHistorySelection $StateBeforeCancel $before "$Label pre-Escape state refresh"
    Compare-SourceHistorySelection $Original $before "$Label before Escape"
    Assert-True ($before.ime.active -and -not [string]::IsNullOrEmpty($before.ime.preedit)) "$Label had no active nonempty preedit; no Escape was sent."

    Send-Key 0x1B
    try {
        $null = Wait-GuardedImeState $ExpectedIdentity $Original { param($d) (-not $d.ime.active -and [string]::IsNullOrEmpty($d.ime.preedit)) -or ($d.ime.active -and $d.ime.preedit -ceq $Reading) } "$Label first Escape completion or return to reading"
    }
    catch {
        if ($_.Exception.Message -notlike 'Timed out waiting for *') { throw }
        return [pscustomobject]@{ status = 'BLOCKED'; escapes_sent = 1; note = $_.Exception.Message + ' No second Escape was sent.' }
    }
    $firstEscapeProfile = Assert-InputProfileIdentity $ExpectedIdentity "$Label profile after first Escape"
    $firstEscape = $script:LastOwnedDiagnostic
    Assert-True ($null -ne $firstEscape) "$Label had no fresh owned-app diagnostic after first Escape."
    Compare-SourceHistorySelection $Original $firstEscape "$Label first Escape"
    Add-DiagnosticTrace "$Label-first-escape" $firstEscape
    if (-not $firstEscape.ime.active -and [string]::IsNullOrEmpty($firstEscape.ime.preedit)) {
        return [pscustomobject]@{ status = 'PASS'; escapes_sent = 1; profile_before = $beforeProfile; profile_after_first = $firstEscapeProfile; state_after_first = Get-DiagnosticStateSummary $firstEscape; state_after_second = $null; note = 'The first guarded Escape cleared the IME composition.' }
    }
    if (-not $firstEscape.ime.active -or [string]::IsNullOrEmpty($firstEscape.ime.preedit)) {
        return [pscustomobject]@{ status = 'BLOCKED'; escapes_sent = 1; profile_before = $beforeProfile; profile_after_first = $firstEscapeProfile; state_after_first = Get-DiagnosticStateSummary $firstEscape; state_after_second = $null; note = 'The first Escape left an incomplete IME state; no second Escape was sent.' }
    }

    $secondGuardProfile = Assert-InputProfileIdentity $ExpectedIdentity "$Label profile before the single second Escape"
    $secondGuard = $script:LastOwnedDiagnostic
    Assert-True ($null -ne $secondGuard) "$Label had no fresh owned-app diagnostic before the single second Escape."
    Compare-SourceHistorySelection $Original $secondGuard "$Label before the single second Escape"
    Assert-True ([string]::Equals((Get-InputProfileKey $firstEscapeProfile), (Get-InputProfileKey $secondGuardProfile), [StringComparison]::OrdinalIgnoreCase)) "$Label native input profile changed between Escape stages."
    Assert-True ($secondGuard.ime.active -and $secondGuard.ime.preedit -ceq $Reading) "$Label did not retain the complete unconverted reading before the single second Escape; no further input was sent."
    Send-Key 0x1B
    try {
        $null = Wait-GuardedImeState $ExpectedIdentity $Original { param($d) -not $d.ime.active -and [string]::IsNullOrEmpty($d.ime.preedit) } "$Label second Escape completion"
    }
    catch {
        if ($_.Exception.Message -notlike 'Timed out waiting for *') { throw }
        return [pscustomobject]@{ status = 'BLOCKED'; escapes_sent = 2; note = $_.Exception.Message + ' No further input was sent.' }
    }
    $finalProfile = Assert-InputProfileIdentity $ExpectedIdentity "$Label profile after the single second Escape"
    $finalState = $script:LastOwnedDiagnostic
    Assert-True ($null -ne $finalState) "$Label had no fresh owned-app diagnostic after the single second Escape."
    Compare-SourceHistorySelection $Original $finalState "$Label second Escape"
    Add-DiagnosticTrace "$Label-second-escape" $finalState
    if ($finalState.ime.active -or -not [string]::IsNullOrEmpty($finalState.ime.preedit)) {
        return [pscustomobject]@{ status = 'BLOCKED'; escapes_sent = 2; profile_before = $beforeProfile; profile_after_first = $firstEscapeProfile; profile_before_second = $secondGuardProfile; profile_after_second = $finalProfile; state_after_first = Get-DiagnosticStateSummary $firstEscape; state_after_second = Get-DiagnosticStateSummary $finalState; note = 'The single guarded second Escape did not clear the IME composition; no further input was sent.' }
    }
    [pscustomobject]@{ status = 'PASS'; escapes_sent = 2; profile_before = $beforeProfile; profile_after_first = $firstEscapeProfile; profile_before_second = $secondGuardProfile; profile_after_second = $finalProfile; state_after_first = Get-DiagnosticStateSummary $firstEscape; state_after_second = Get-DiagnosticStateSummary $finalState; note = 'The first Escape returned to preedit; one guarded second Escape cleared it.' }
}

function Invoke-MicrosoftImeAttempt($ImeState) {
    $targetKey = Get-InputProfileKey $ImeState.profile
    $reading = 'にほんご'
    $result = [ordered]@{ status = 'BLOCKED'; note = $ImeState.note; input_profile = $ImeState.profile; original_profile = $script:OriginalInputProfile; original_ime_open = $script:OriginalImeOpen; commit_status = 'NOT_RUN'; undo_redo_status = 'NOT_RUN'; input_delivery = 'The confirmed Microsoft IME profile was selected through normal Win+Space input; roman keys reached the owned app through Windows SendInput and native IME messages. No synthetic WM_IME_COMPOSITION was sent; this is not physical keyboard input.' }
    $original = Get-Diagnostic
    try {
        if ([string]::IsNullOrWhiteSpace($targetKey) -or -not (Test-MicrosoftImeProfile $ImeState.profile)) { throw 'The IME composition test was refused because exact Japanese Microsoft IME identity was not confirmed.' }
        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME before composition input'
        Send-Key 0x23
        Send-RomanLetters 'kana'
        $composition = $null
        try { $composition = Wait-GuardedImeState $targetKey $original { param($d) $d.ime.active -and $d.ime.preedit -ceq 'かな' } 'complete Microsoft IME kana reading' }
        catch {
            if ($_.Exception.Message -cne 'Timed out waiting for complete Microsoft IME kana reading.') { throw }
            $result.note = 'The confirmed Microsoft IME did not deliver native preedit text to the editor within 3 seconds.'
            $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile before preedit cancellation'
            $state = $script:LastOwnedDiagnostic
            Compare-SourceHistorySelection $original $state 'Before cancelling unobserved Microsoft IME preedit'
            if ($state.ime.active -and -not [string]::IsNullOrEmpty($state.ime.preedit)) {
                Send-Key 0x1B
                $cancelled = Wait-GuardedImeState $targetKey $original { param($d) -not $d.ime.active -and [string]::IsNullOrEmpty($d.ime.preedit) } 'cancelling unobserved Microsoft IME preedit'
                Add-DiagnosticTrace 'ime-unobserved-preedit-cancelled' $cancelled
            }
            Add-Check 'R4-ime' 'BLOCKED' $result.note
            return $result
        }
        $result.preedit_active_input_profile = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile at first preedit'
        $result.preedit_observed = $composition.ime.preedit
        $result.preedit_screenshot = (Capture-Desktop 'ime-preedit-app.png' $composition).path
        Send-Key 0x1B
        $cancelled = Wait-GuardedImeState $targetKey $original { param($d) -not $d.ime.active -and [string]::IsNullOrEmpty($d.ime.preedit) } 'Microsoft IME cancellation'
        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile after Escape cancellation'
        Add-DiagnosticTrace 'ime-cancelled' $cancelled
        Send-RomanLetters 'nihongo'
        $secondComposition = Wait-GuardedImeState $targetKey $original { param($d) $d.ime.active -and $d.ime.preedit -ceq $reading } 'complete Microsoft IME nihongo reading'
        $result.second_preedit_active_input_profile = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile at second preedit'
        $secondComposition = $script:LastOwnedDiagnostic
        Assert-True ($secondComposition.ime.active -and $secondComposition.ime.preedit -ceq $reading) 'The complete nihongo reading was not retained before conversion.'
        $result.second_preedit = $secondComposition.ime.preedit
        Compare-SourceHistorySelection $original $secondComposition 'Before Microsoft IME conversion'
        Add-DiagnosticTrace 'ime-second-preedit-ready' $secondComposition

        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile before first Space conversion'
        $beforeConversion = $script:LastOwnedDiagnostic
        Compare-SourceHistorySelection $original $beforeConversion 'Before first Space conversion'
        Assert-True ($beforeConversion.ime.active -and $beforeConversion.ime.preedit -ceq $reading) 'The complete nihongo reading changed before first Space; no conversion input was sent.'
        Send-Key 0x20
        $null = Wait-GuardedImeState $targetKey $original { param($d) $d.ime.active -and -not [string]::IsNullOrEmpty($d.ime.preedit) -and $d.ime.preedit -cne $reading -and $d.ime.preedit -notmatch '[A-Za-zＡ-Ｚａ-ｚ]' } 'Microsoft IME conversion of the complete reading'
        $result.conversion_active_input_profile = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile after first Space conversion'
        $conversionState = $script:LastOwnedDiagnostic
        Assert-True ($conversionState.ime.active -and -not [string]::IsNullOrEmpty($conversionState.ime.preedit)) 'The first Space conversion did not leave a fresh active nonempty IME preedit.'
        Assert-True ($conversionState.ime.preedit -cne $reading -and $conversionState.ime.preedit -notmatch '[A-Za-zＡ-Ｚａ-ｚ]') 'The first Space did not expose a converted complete reading; candidate input was not sent.'
        Compare-SourceHistorySelection $original $conversionState 'First Space conversion'
        Add-DiagnosticTrace 'ime-first-space-conversion' $conversionState
        $result.converted_preedit = $conversionState.ime.preedit

        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile before second Space candidate request'
        $beforeCandidateRequest = $script:LastOwnedDiagnostic
        Compare-SourceHistorySelection $original $beforeCandidateRequest 'Before second Space candidate request'
        Assert-True ($beforeCandidateRequest.ime.active -and -not [string]::IsNullOrEmpty($beforeCandidateRequest.ime.preedit)) 'The converted preedit was not active before candidate input.'
        Send-Key 0x20
        Start-Sleep -Milliseconds 400
        $result.candidate_active_input_profile = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile after second Space candidate request'
        $candidateState = $script:LastOwnedDiagnostic
        Assert-True ($candidateState.ime.active -and -not [string]::IsNullOrEmpty($candidateState.ime.preedit)) 'The second Space candidate request did not retain active nonempty preedit.'
        Compare-SourceHistorySelection $original $candidateState 'Second Space candidate request'
        $candidateGeometry = Get-ImeCandidateGeometry
        $result.candidate_geometry = $candidateGeometry
        $candidatePopups = @(Get-OwnedImePopupWindows)
        $result.owned_candidate_windows = @($candidatePopups | Select-Object handle_hex, process_id, class_name, rect)
        if ($candidatePopups.Count -eq 0) {
            $result.candidate_popup_status = 'BLOCKED_NO_VISIBLE_OWNED_WINDOW'
            $result.note = 'Two guarded Space stages retained pure source/history, but no visible candidate popup owned by the app was observed; candidate UI acceptance is BLOCKED.'
            Add-Check 'R4-ime-candidate' 'BLOCKED' $result.note
        }
        else {
            $result.candidate_popup_status = 'PASS'
            $candidatePopup = $candidatePopups[0]
            $result.candidate_app_screenshot = (Capture-Desktop 'ime-candidate-app.png' $candidateState).path
            $result.candidate_screenshot = (Capture-Desktop 'ime-candidate-window.png' $candidateState $candidatePopup.handle).path
        }
        $result.candidate_cancel = Invoke-GuardedImeEscapeCancellation $targetKey $original $candidateState 'ime-candidate-cancel' $reading
        if ($result.candidate_cancel.status -ne 'PASS') {
            $result.note = $result.candidate_cancel.note
            Add-Check 'R4-ime' 'BLOCKED' $result.note
            return $result
        }

        Send-RomanLetters 'nihongo'
        $null = Wait-GuardedImeState $targetKey $original { param($d) $d.ime.active -and $d.ime.preedit -ceq $reading } 'complete Microsoft IME reading before commit conversion'
        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile before commit conversion'
        $beforeCommitConversion = $script:LastOwnedDiagnostic
        Compare-SourceHistorySelection $original $beforeCommitConversion 'Before commit conversion'
        Assert-True ($beforeCommitConversion.ime.active -and $beforeCommitConversion.ime.preedit -ceq $reading) 'The complete nihongo reading changed before commit conversion; no input was sent.'
        Send-Key 0x20
        $null = Wait-GuardedImeState $targetKey $original { param($d) $d.ime.active -and -not [string]::IsNullOrEmpty($d.ime.preedit) -and $d.ime.preedit -cne $reading -and $d.ime.preedit -notmatch '[A-Za-zＡ-Ｚａ-ｚ]' } 'Microsoft IME commit conversion of the complete reading'
        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile after commit conversion'
        $commitConversion = $script:LastOwnedDiagnostic
        Assert-True ($commitConversion.ime.active -and -not [string]::IsNullOrEmpty($commitConversion.ime.preedit)) 'The commit conversion did not leave active nonempty preedit.'
        Assert-True ($commitConversion.ime.preedit -cne $reading -and $commitConversion.ime.preedit -notmatch '[A-Za-zＡ-Ｚａ-ｚ]') 'The commit conversion did not convert the complete reading.'
        Compare-SourceHistorySelection $original $commitConversion 'Commit conversion'
        $null = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile before Enter commit'
        $commitReady = $script:LastOwnedDiagnostic
        Compare-SourceHistorySelection $original $commitReady 'Before Enter commit'
        Assert-True ($commitReady.ime.active -and -not [string]::IsNullOrEmpty($commitReady.ime.preedit)) 'No active converted preedit remained before Enter commit.'
        Send-Key 0x0D
        $committed = Wait-Diagnostic { param($d) -not $d.ime.active -and $d.source -cne $original.source } 5000 'Microsoft IME result commit'
        $result.commit_active_input_profile = Assert-InputProfileIdentity $targetKey 'Microsoft IME profile at commit'
        Add-DiagnosticTrace 'ime-committed' $committed
        $result.committed_suffix = Get-Utf8Range $committed.source ([Text.Encoding]::UTF8.GetByteCount($original.source)) ([Text.Encoding]::UTF8.GetByteCount($committed.source))
        $result.commit_undo_count = $committed.undo_count
        Assert-True ($committed.undo_count -eq ($original.undo_count + 1)) 'Microsoft IME commit was not one edit transaction.'
        Send-Key 0x5A @(0x11)
        $undo = Wait-Diagnostic { param($d) $d.source -ceq $original.source -and -not $d.dirty } 5000 'Microsoft IME commit Undo'
        Add-DiagnosticTrace 'ime-undo' $undo
        Send-Key 0x59 @(0x11)
        $redo = Wait-Diagnostic { param($d) $d.source -ceq $committed.source } 5000 'Microsoft IME commit Redo'
        Add-DiagnosticTrace 'ime-redo' $redo
        Send-Key 0x5A @(0x11)
        $imeRollback = Wait-Diagnostic { param($d) $d.source -ceq $original.source -and -not $d.dirty } 5000 'Microsoft IME final rollback'
        Add-DiagnosticTrace 'ime-rollback' $imeRollback
        $result.commit_status = 'PASS'
        $result.undo_redo_status = 'PASS'
        Add-Check 'R4-ime-commit-undo-redo' 'PASS' 'The complete nihongo reading was converted, committed as one source transaction, undone, redone and rolled back through the real Microsoft IME independently of candidate popup association.'
        $result.status = if ($result.candidate_popup_status -eq 'PASS') { 'PASS' } else { 'BLOCKED' }
        $result.cancelled_source_unchanged = $cancelled.source -ceq $original.source
        $result.undo_restored_exact_source = $undo.source -ceq $original.source
        $result.redo_restored_exact_commit = $redo.source -ceq $committed.source
        $result.note = if ($result.status -eq 'PASS') { 'The installed Microsoft IME delivered composition, conversion, candidate UI, staged Escape cancellation, Enter commit, Undo and Redo through the actual app window.' } else { 'Microsoft IME composition, conversion, cancellation, Enter commit and Undo/Redo passed; candidate UI remains BLOCKED because no visible popup owned by the app was associated.' }
        Add-Check 'R4-ime' $result.status $result.note
        return $result
    }
    catch {
        $result.status = 'FAIL'
        $result.note = $_.Exception.Message
        Add-Check 'R4-ime' 'FAIL' $result.note
        return $result
    }
}

function Wait-OwnedDialog([int]$TimeoutMs) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $found = [IntPtr]::Zero
        $callback = [MdliteRustGuiNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$data)
            [uint32]$ownerProcessId = 0
            [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
            if ($ownerProcessId -eq [uint32]$data.ToInt64()) {
                $className = [Text.StringBuilder]::new(80)
                [void][MdliteRustGuiNative]::GetClassNameW($window, $className, $className.Capacity)
                if ($className.ToString() -eq '#32770') { $script:FoundDialog = $window; return $false }
            }
            return $true
        }
        $script:FoundDialog = [IntPtr]::Zero
        [void][MdliteRustGuiNative]::EnumWindows($callback, [IntPtr]$script:Process.Id)
        if ($script:FoundDialog -ne [IntPtr]::Zero) { return $script:FoundDialog }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    return [IntPtr]::Zero
}

function Get-ResourceWindowSnapshot([Diagnostics.Process]$Process, [string]$ExpectedExecutable) {
    Assert-OwnedWindowIdentity $Process $script:WindowHandle
    $Process.Refresh()
    if ($Process.HasExited) { throw "The resource process $($Process.Id) exited before its geometry readback." }
    $expectedPath = [IO.Path]::GetFullPath($ExpectedExecutable)
    $actualPath = [IO.Path]::GetFullPath($Process.MainModule.FileName)
    if (-not $actualPath.Equals($expectedPath, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Resource process path '$actualPath' does not match expected executable '$expectedPath'."
    }
    $windowRect = [MdliteRustGuiNative+Rect]::new()
    $clientRect = [MdliteRustGuiNative+Rect]::new()
    if (-not [MdliteRustGuiNative]::GetWindowRect($script:WindowHandle, [ref]$windowRect)) { throw 'GetWindowRect failed for the owned resource window.' }
    if (-not [MdliteRustGuiNative]::GetClientRect($script:WindowHandle, [ref]$clientRect)) { throw 'GetClientRect failed for the owned resource window.' }
    $foregroundWindow = [MdliteRustGuiNative]::GetForegroundWindow()
    [uint32]$foregroundPid = 0
    if ($foregroundWindow -ne [IntPtr]::Zero) { [void][MdliteRustGuiNative]::GetWindowThreadProcessId($foregroundWindow, [ref]$foregroundPid) }
    [pscustomobject]@{
        hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
        process_id = $Process.Id
        expected_executable = $expectedPath
        process_executable = $actualPath
        window_rect = [ordered]@{ left = $windowRect.Left; top = $windowRect.Top; right = $windowRect.Right; bottom = $windowRect.Bottom; width = $windowRect.Right - $windowRect.Left; height = $windowRect.Bottom - $windowRect.Top }
        client_rect = [ordered]@{ left = $clientRect.Left; top = $clientRect.Top; right = $clientRect.Right; bottom = $clientRect.Bottom; width = $clientRect.Right - $clientRect.Left; height = $clientRect.Bottom - $clientRect.Top }
        dpi = [MdliteRustGuiNative]::GetDpiForWindow($script:WindowHandle)
        visible = [MdliteRustGuiNative]::IsWindowVisible($script:WindowHandle)
        is_minimized = [MdliteRustGuiNative]::IsIconic($script:WindowHandle)
        is_maximized = [MdliteRustGuiNative]::IsZoomed($script:WindowHandle)
        foreground_hwnd = if ($foregroundWindow -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $foregroundWindow.ToInt64() }
        foreground_process_id = if ($foregroundWindow -eq [IntPtr]::Zero) { $null } else { $foregroundPid }
        foreground_is_exact_window = ($foregroundWindow -eq $script:WindowHandle)
        foreground_is_owned_process = ($foregroundPid -eq [uint32]$Process.Id)
    }
}

function Set-ResourceWindowGeometry([Diagnostics.Process]$Process, [string]$ExpectedExecutable) {
    $targetWidth = 1084; $targetHeight = 761; $targetDpi = 96
    $before = Get-ResourceWindowSnapshot $Process $ExpectedExecutable
    $after = $before
    $diagnosticBefore = if ([string]::IsNullOrWhiteSpace([string]$script:DiagnosticPath)) { $null } else { Get-Diagnostic }
    $failure = if ($null -ne $script:DiagnosticPath -and $null -eq $diagnosticBefore) { 'A fresh Rust diagnostic was unavailable before positioning.' } elseif ($before.dpi -ne $targetDpi) { "Resource comparison requires $targetDpi DPI; the window reports $($before.dpi)." } elseif (-not $before.visible) { 'The exact owned resource window is not visible.' } elseif (-not ($before.is_minimized -or $before.is_maximized) -and (-not $before.foreground_is_exact_window -or -not $before.foreground_is_owned_process)) { 'The exact owned resource HWND/PID is not foreground.' } else { $null }
    $restoreAttempted = $false
    $setWindowPosRequest = $null
    if ($null -eq $failure -and ($before.is_minimized -or $before.is_maximized)) {
        $restoreAttempted = $true
        [void][MdliteRustGuiNative]::ShowWindow($script:WindowHandle, [MdliteRustGuiNative]::SW_RESTORE)
        $after = Get-ResourceWindowSnapshot $Process $ExpectedExecutable
        if (-not $after.visible -or $after.is_minimized -or $after.is_maximized -or -not $after.foreground_is_exact_window -or -not $after.foreground_is_owned_process) { $failure = 'The owned window did not restore to a normal, visible foreground state.' }
    }
    if ($null -eq $failure -and ($after.client_rect.width -ne $targetWidth -or $after.client_rect.height -ne $targetHeight)) {
        $outerWidth = $after.window_rect.width + $targetWidth - $after.client_rect.width
        $outerHeight = $after.window_rect.height + $targetHeight - $after.client_rect.height
        $setWindowPosRequest = [ordered]@{ left = $after.window_rect.left; top = $after.window_rect.top; width = $outerWidth; height = $outerHeight; flags = 'SWP_NOZORDER|SWP_NOACTIVATE' }
        if (-not [MdliteRustGuiNative]::SetWindowPos($script:WindowHandle, [IntPtr]::Zero, $setWindowPosRequest.left, $setWindowPosRequest.top, $outerWidth, $outerHeight, [MdliteRustGuiNative]::SWP_NOZORDER -bor [MdliteRustGuiNative]::SWP_NOACTIVATE)) { $failure = 'SetWindowPos failed for the owned resource window (Win32 {0}).' -f [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
    }
    if ($null -eq $failure) {
        $deadline = [DateTime]::UtcNow.AddMilliseconds(750)
        do {
            Start-Sleep -Milliseconds 50
            $after = Get-ResourceWindowSnapshot $Process $ExpectedExecutable
        } while (($after.dpi -ne $targetDpi -or $after.client_rect.width -ne $targetWidth -or $after.client_rect.height -ne $targetHeight) -and [DateTime]::UtcNow -lt $deadline)
        if ($after.dpi -ne $targetDpi -or $after.client_rect.width -ne $targetWidth -or $after.client_rect.height -ne $targetHeight -or -not $after.visible -or $after.is_minimized -or $after.is_maximized -or -not $after.foreground_is_exact_window -or -not $after.foreground_is_owned_process) { $failure = 'The owned window did not settle at the requested geometry with exact foreground ownership.' }
    }
    else { $after = Get-ResourceWindowSnapshot $Process $ExpectedExecutable }
    $diagnosticAfter = if ([string]::IsNullOrWhiteSpace([string]$script:DiagnosticPath)) { $null } else { Get-Diagnostic }
    if ($null -ne $diagnosticBefore -and $null -eq $diagnosticAfter) { if ($null -eq $failure) { $failure = 'A fresh Rust diagnostic was unavailable after positioning.' } }
    elseif ($null -ne $diagnosticBefore) { try { Compare-SourceHistorySelection $diagnosticBefore $diagnosticAfter 'Resource window positioning' } catch { if ($null -eq $failure) { $failure = $_.Exception.Message } } }
    $result = [pscustomobject]@{
        status = if ($null -eq $failure) { 'PASS' } else { 'BLOCKED' }
        reason = $failure
        requested = [ordered]@{ client_width = $targetWidth; client_height = $targetHeight; dpi = $targetDpi; normal_window = $true; foreground_owned = $true }
        restore_attempted = $restoreAttempted
        set_window_pos_request = $setWindowPosRequest
        before = $before
        after = $after
        rust_document_state_before = if ($null -ne $diagnosticBefore) { Get-DiagnosticStateSummary $diagnosticBefore } else { $null }
        rust_document_state_after = if ($null -ne $diagnosticAfter) { Get-DiagnosticStateSummary $diagnosticAfter } else { $null }
    }
    return $result
}

function Get-ResourceSample([Diagnostics.Process]$Process, [int]$DocumentCount, [string]$ExpectedExecutable) {
    $geometrySetup = Set-ResourceWindowGeometry $Process $ExpectedExecutable
    if ($geometrySetup.status -ne 'PASS') {
        return [pscustomobject]@{ status = 'BLOCKED'; documents = $DocumentCount; process_id = $Process.Id; geometry_setup = $geometrySetup; reason = $geometrySetup.reason }
    }
    $Process.Refresh()
    if ($Process.HasExited) { throw "The resource process $($Process.Id) exited before sampling." }
    $startCpu = $Process.TotalProcessorTime
    $startUtc = [DateTime]::UtcNow
    Start-Sleep -Seconds 5
    $Process.Refresh()
    $endUtc = [DateTime]::UtcNow
    if ($Process.HasExited) { throw "The resource process $($Process.Id) exited during sampling." }
    $cpuMs = ($Process.TotalProcessorTime - $startCpu).TotalMilliseconds
    $wallMs = ($endUtc - $startUtc).TotalMilliseconds
    $window = Get-ResourceWindowSnapshot $Process $ExpectedExecutable
    $sampleFailure = $null
    if ($window.dpi -ne $geometrySetup.requested.dpi -or $window.client_rect.width -ne $geometrySetup.requested.client_width -or $window.client_rect.height -ne $geometrySetup.requested.client_height) { $sampleFailure = 'The owned window changed from the requested client size or DPI during sampling.' }
    elseif (-not $window.visible -or $window.is_minimized -or $window.is_maximized) { $sampleFailure = 'The resource window was not normal and visible at sample end.' }
    elseif (-not $window.foreground_is_exact_window -or -not $window.foreground_is_owned_process) { $sampleFailure = 'The resource window lacked exact foreground HWND/PID ownership at sample end.' }
    $inputProfile = [ordered]@{ status = 'UNKNOWN'; reason = 'No native input-profile observer is available for this process.' }
    if (-not [string]::IsNullOrWhiteSpace([string]$script:DiagnosticPath)) {
        $inputProfile = Get-ActiveInputProfile
    }
    [pscustomobject]@{
        status = if ($null -eq $sampleFailure) { 'MEASURED' } else { 'BLOCKED' }
        reason = $sampleFailure
        documents = $DocumentCount
        sampled_utc = $endUtc.ToString('o')
        working_set_bytes = $Process.WorkingSet64
        private_usage_bytes = $Process.PrivateMemorySize64
        peak_working_set_bytes = $Process.PeakWorkingSet64
        idle_cpu_percent_one_core = [Math]::Round(100.0 * $cpuMs / $wallMs, 3)
        idle_cpu_percent_machine = [Math]::Round(100.0 * $cpuMs / ($wallMs * [Environment]::ProcessorCount), 3)
        cpu_window_ms = [Math]::Round($wallMs, 1)
        process_id = $Process.Id
        geometry_setup = $geometrySetup
        window_geometry = $window
        active_input_profile = $inputProfile
    }
}

function Compare-ResourceSampleGeometry($RustSample, $CppSample, [int]$DocumentCount) {
    $reasons = @()
    $target = $RustSample.geometry_setup.requested
    foreach ($entry in @(@{ label = 'Rust'; sample = $RustSample }, @{ label = 'C++'; sample = $CppSample })) {
        $sample = $entry.sample
        $label = $entry.label
        if ($sample.status -ne 'MEASURED') { $reasons += "$label $DocumentCount-document sample was not measured."; continue }
        if ($sample.geometry_setup.status -ne 'PASS') { $reasons += "$label $DocumentCount-document geometry setup did not pass."; continue }
        $window = $sample.window_geometry
        if ($sample.geometry_setup.requested.client_width -ne $target.client_width -or $sample.geometry_setup.requested.client_height -ne $target.client_height -or $sample.geometry_setup.requested.dpi -ne $target.dpi) { $reasons += "$label $DocumentCount-document requested geometry differed from the Rust target." }
        if ($window.dpi -ne $sample.geometry_setup.requested.dpi) { $reasons += "$label $DocumentCount-document sample DPI did not match its request." }
        if ($window.client_rect.width -ne $sample.geometry_setup.requested.client_width -or $window.client_rect.height -ne $sample.geometry_setup.requested.client_height) { $reasons += "$label $DocumentCount-document client size did not match its request." }
        if (-not $window.visible -or $window.is_minimized -or $window.is_maximized) { $reasons += "$label $DocumentCount-document window was not normal and visible." }
        if (-not $window.foreground_is_exact_window -or -not $window.foreground_is_owned_process) { $reasons += "$label $DocumentCount-document sample lacked exact foreground HWND/PID ownership." }
    }
    if ($RustSample.status -eq 'MEASURED' -and $CppSample.status -eq 'MEASURED') {
        foreach ($dimension in @('width','height')) {
            if ($RustSample.window_geometry.window_rect[$dimension] -ne $CppSample.window_geometry.window_rect[$dimension]) { $reasons += "The Rust and C++ $DocumentCount-document outer window $dimension differed." }
            if ($RustSample.window_geometry.client_rect[$dimension] -ne $CppSample.window_geometry.client_rect[$dimension]) { $reasons += "The Rust and C++ $DocumentCount-document client $dimension differed." }
        }
        if ($RustSample.window_geometry.dpi -ne $CppSample.window_geometry.dpi) { $reasons += "The Rust and C++ $DocumentCount-document DPI differed." }
    }
    [pscustomobject]@{
        documents = $DocumentCount
        status = if ($reasons.Count -eq 0) { 'PASS' } else { 'BLOCKED' }
        target = $target
        reasons = @($reasons)
    }
}

function Find-CppTabControl {
    $script:FoundTabControl = [IntPtr]::Zero
    $callback = [MdliteRustGuiNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$data)
        [uint32]$ownerProcessId = 0
        [void][MdliteRustGuiNative]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
        if ($ownerProcessId -eq [uint32]$script:Process.Id) {
            $className = [Text.StringBuilder]::new(80)
            [void][MdliteRustGuiNative]::GetClassNameW($window, $className, $className.Capacity)
            if ($className.ToString() -eq 'SysTabControl32') { $script:FoundTabControl = $window; return $false }
        }
        return $true
    }
    [void][MdliteRustGuiNative]::EnumChildWindows($script:WindowHandle, $callback, [IntPtr]::Zero)
    return $script:FoundTabControl
}

function Assert-CppLaunchIdentity([int]$ExpectedProcessId, [IntPtr]$ExpectedWindow) {
    if ($null -eq $script:Process) { throw 'No launched C++ process is registered during tab startup.' }
    $script:Process.Refresh()
    if ($script:Process.HasExited) { throw "The C++ baseline exited during startup with code $($script:Process.ExitCode)." }
    if ($script:Process.Id -ne $ExpectedProcessId -or $script:WindowHandle -ne $ExpectedWindow -or $script:Process.MainWindowHandle -ne $ExpectedWindow) { throw 'The registered C++ PID/HWND changed during startup.' }
    if ($ExpectedWindow -eq [IntPtr]::Zero -or -not [MdliteRustGuiNative]::IsWindow($ExpectedWindow)) { throw 'The captured C++ main HWND is no longer a live window.' }
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($ExpectedWindow, [ref]$ownerPid)
    if ($ownerPid -ne [uint32]$ExpectedProcessId) { throw "C++ main HWND owner PID $ownerPid does not match launched PID $ExpectedProcessId." }
}

function Get-CppTabCount([IntPtr]$TabControl, [uint32]$ExpectedProcessId, [int]$TimeoutMs = 250) {
    if ($TabControl -eq [IntPtr]::Zero -or -not [MdliteRustGuiNative]::IsWindow($TabControl)) { throw 'The captured C++ tab-control HWND is no longer a live window.' }
    [uint32]$ownerPid = 0
    [void][MdliteRustGuiNative]::GetWindowThreadProcessId($TabControl, [ref]$ownerPid)
    if ($ownerPid -ne $ExpectedProcessId) { throw "C++ tab-control HWND owner PID $ownerPid does not match launched PID $ExpectedProcessId." }
    $result = [IntPtr]::Zero
    $sent = [MdliteRustGuiNative]::SendMessageTimeoutW($TabControl, [MdliteRustGuiNative]::TCM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero, [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, [uint32]$TimeoutMs, [ref]$result)
    if ($sent -eq [IntPtr]::Zero) { return [pscustomobject]@{ responsive = $false; count = $null; win32_error = [Runtime.InteropServices.Marshal]::GetLastWin32Error() } }
    return [pscustomobject]@{ responsive = $true; count = $result.ToInt32(); win32_error = 0 }
}

function Wait-CppTabCount([int]$Expected, [int]$TimeoutMs = 15000) {
    if ($Expected -le 0) { throw 'The expected C++ startup tab count must be positive.' }
    if ($TimeoutMs -le 0) { throw 'The C++ startup deadline must be positive.' }
    $expectedProcessId = [int]$script:Process.Id
    $expectedWindow = $script:WindowHandle
    $deadline = [DateTime]::UtcNow.AddMilliseconds([Math]::Min(15000, $TimeoutMs))
    $status = 'TIMEOUT'
    $failure = $null
    $tab = [IntPtr]::Zero
    $lastCount = $null
    $tabQueryTimeouts = 0
    $lastTabQueryError = $null
    $responsivenessChecks = 0
    $transientResponsivenessFailures = 0
    $firstResponsivenessFailure = $null
    $lastResponsivenessFailure = $null
    $lastResponsiveUtc = $null
    try {
        do {
            Assert-CppLaunchIdentity $expectedProcessId $expectedWindow
            $remainingMs = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
            if ($remainingMs -le 0) { break }
            $responseResult = [IntPtr]::Zero
            $responsivenessChecks++
            $responseBudget = [uint32][Math]::Max(1, [Math]::Min(250, $remainingMs))
            $responseSent = [MdliteRustGuiNative]::SendMessageTimeoutW($expectedWindow, [MdliteRustGuiNative]::WM_NULL, [IntPtr]::Zero, [IntPtr]::Zero, [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, $responseBudget, [ref]$responseResult)
            if ($responseSent -eq [IntPtr]::Zero) {
                $transientResponsivenessFailures++
                $lastResponsivenessFailure = [pscustomobject]@{ at_utc = [DateTime]::UtcNow.ToString('o'); win32_error = [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
                if ($null -eq $firstResponsivenessFailure) { $firstResponsivenessFailure = $lastResponsivenessFailure }
            }
            else { $lastResponsiveUtc = [DateTime]::UtcNow.ToString('o') }

            $tab = Find-CppTabControl
            if ($tab -ne [IntPtr]::Zero) {
                $remainingMs = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
                if ($remainingMs -le 0) { break }
                $tabBudget = [int][Math]::Max(1, [Math]::Min(250, $remainingMs))
                $tabResult = Get-CppTabCount $tab ([uint32]$expectedProcessId) $tabBudget
                if ($tabResult.responsive) { $lastCount = [int]$tabResult.count }
                else { $tabQueryTimeouts++; $lastTabQueryError = [int]$tabResult.win32_error }
                if ($tabResult.responsive -and $tabResult.count -eq $Expected -and $responseSent -ne [IntPtr]::Zero) {
                    Assert-CppLaunchIdentity $expectedProcessId $expectedWindow
                    $status = 'READY'
                    return [pscustomobject]@{
                        tab_control = ('0x{0:X}' -f $tab.ToInt64())
                        count = $lastCount
                        pid = $expectedProcessId
                        window_handle = ('0x{0:X}' -f $expectedWindow.ToInt64())
                        responsiveness_checks = $responsivenessChecks
                        transient_responsiveness_failures = $transientResponsivenessFailures
                        first_responsiveness_failure = $firstResponsivenessFailure
                        last_responsiveness_failure = $lastResponsivenessFailure
                        last_responsive_utc = $lastResponsiveUtc
                        tab_query_timeouts = $tabQueryTimeouts
                    }
                }
            }
            $remainingMs = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
            if ($remainingMs -gt 0) { Start-Sleep -Milliseconds ([Math]::Min(100, $remainingMs)) }
        } while ([DateTime]::UtcNow -lt $deadline)
        $failure = "Timed out after $([Math]::Min(15000, $TimeoutMs)) ms waiting for C++ startup tab count $Expected; last_count=$lastCount, main_window_handle=$('0x{0:X}' -f $expectedWindow.ToInt64()), tab_control_handle=$('0x{0:X}' -f $tab.ToInt64()), transient_responsiveness_failures=$transientResponsivenessFailures, tab_query_timeouts=$tabQueryTimeouts, last_tab_query_win32_error=$lastTabQueryError."
        throw $failure
    }
    catch {
        if ($null -eq $failure) { $failure = $_.Exception.Message; $status = 'FAIL' }
        throw
    }
    finally {
        $script:Report.cpp_startup_readiness = @($script:Report.cpp_startup_readiness) + [pscustomobject]@{
            expected_count = $Expected
            status = $status
            pid = $expectedProcessId
            window_handle = if ($expectedWindow -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $expectedWindow.ToInt64() }
            tab_control_handle = if ($tab -eq [IntPtr]::Zero) { $null } else { '0x{0:X}' -f $tab.ToInt64() }
            last_count = $lastCount
            responsiveness_checks = $responsivenessChecks
            transient_responsiveness_failures = $transientResponsivenessFailures
            first_responsiveness_failure = $firstResponsivenessFailure
            last_responsiveness_failure = $lastResponsivenessFailure
            last_responsive_utc = $lastResponsiveUtc
            tab_query_timeouts = $tabQueryTimeouts
            last_tab_query_win32_error = $lastTabQueryError
            failure = $failure
        }
    }
}

function Get-CppBaselineIdentity {
    Assert-True (Test-Path -LiteralPath $CppExePath -PathType Leaf) "The C++ baseline executable is missing: $CppExePath"
    Assert-True (Test-Path -LiteralPath $CppReceiptPath -PathType Leaf) "The C++ release build receipt is missing: $CppReceiptPath"
    $receipt = Get-Content -LiteralPath $CppReceiptPath -Raw | ConvertFrom-Json
    $hash = (Get-FileHash -LiteralPath $CppExePath -Algorithm SHA256).Hash.ToLowerInvariant()
    Assert-True ($hash -eq $CppExpectedSha256 -and $receipt.executable_sha256 -eq $CppExpectedSha256) 'The C++ executable or receipt no longer matches the recorded e852ee7 release baseline.'
    $head = $null
    $gitResult = & git -C (Join-Path $ProjectRoot '.local\cpp-baseline') rev-parse HEAD 2>$null
    if ($LASTEXITCODE -eq 0) { $head = [string]$gitResult }
    [pscustomobject]@{
        executable = [IO.Path]::GetFullPath($CppExePath)
        executable_sha256 = $hash
        receipt = [IO.Path]::GetFullPath($CppReceiptPath)
        receipt_sha256 = (Get-FileHash -LiteralPath $CppReceiptPath -Algorithm SHA256).Hash.ToLowerInvariant()
        recorded_source_sha256 = [string]$receipt.source_sha256
        source_checkout = [IO.Path]::GetFullPath((Join-Path $ProjectRoot '.local\cpp-baseline'))
        source_head = $head
    }
}

function Measure-CppDocuments([string[]]$Documents, [int]$ExpectedTabs) {
    $workspaceState = Join-Path (Split-Path -Parent $Documents[0]) '.mdlite'
    $sessionState = Join-Path $workspaceState '.state\session.toml'
    Assert-True (Test-Path -LiteralPath $workspaceState -PathType Container) "C++ workspace consent directory is missing: $workspaceState"
    Assert-True (-not (Test-Path -LiteralPath $sessionState)) "C++ fixture session state already exists; refusing an ambiguous startup: $sessionState"
    $processInfo = Start-ProductApp $CppExePath $Documents ''
    $script:Report.process_runs = @($script:Report.process_runs) + $processInfo
    try {
        $tabs = Wait-CppTabCount $ExpectedTabs
        if ($tabs.count -ne $ExpectedTabs) {
            return [pscustomobject]@{
                status = 'NOT_COMPARED'
                documents_requested = $ExpectedTabs
                documents_established = $tabs.count
                tab_control = $tabs.tab_control
                pid = $script:Process.Id
                executable_sha256 = $processInfo.executable_sha256
                reason = 'The C++ startup tab count did not match the requested same-fixture count.'
            }
        }
        $sample = Get-ResourceSample $script:Process $ExpectedTabs $CppExePath
        if ($sample.status -ne 'MEASURED') {
            return [pscustomobject]@{ status = 'BLOCKED'; startup_tab_count = $tabs.count; tab_control = $tabs.tab_control; executable_sha256 = $processInfo.executable_sha256; sample = $sample; reason = $sample.reason }
        }
        [pscustomobject]@{ status = 'MEASURED'; startup_tab_count = $tabs.count; tab_control = $tabs.tab_control; executable_sha256 = $processInfo.executable_sha256; sample = $sample }
    }
    finally {
        Close-LaunchedApp
        $script:Process.Dispose()
        $script:Process = $null
        $script:WindowHandle = [IntPtr]::Zero
        $script:DiagnosticPath = $null
    }
}

function Close-LaunchedApp {
    if ($null -eq $script:Process) { return }
    try { $script:Process.Refresh() } catch { return }
    if ($script:Process.HasExited) { return }
    try {
        [uint32]$ownerPid = 0
        [void][MdliteRustGuiNative]::GetWindowThreadProcessId($script:WindowHandle, [ref]$ownerPid)
        if ($ownerPid -ne [uint32]$script:Process.Id) { throw "Refusing to send close to a window owned by PID $ownerPid." }
        Restore-AppInputState
        $closeResult = [IntPtr]::Zero
        [void][MdliteRustGuiNative]::SendMessageTimeoutW($script:WindowHandle, [MdliteRustGuiNative]::WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero, [MdliteRustGuiNative]::SMTO_ABORTIFHUNG, 750, [ref]$closeResult)
        if ($script:Process.WaitForExit(1500)) { return }
        $dialog = Wait-OwnedDialog 500
        if ($dialog -ne [IntPtr]::Zero) {
            [void][MdliteRustGuiNative]::SetForegroundWindow($dialog)
            Start-Sleep -Milliseconds 80
            if ([MdliteRustGuiNative]::GetForegroundWindow() -eq $dialog) { Send-Key 0x4E }
            [void]$script:Process.WaitForExit(1500)
        }
        $script:Process.Refresh()
        if (-not $script:Process.HasExited) {
            $script:Report.notes = @($script:Report.notes) + "Launched test PID $($script:Process.Id) remained open after a graceful WM_CLOSE; it was left running rather than force-stopped."
        }
    }
    catch { $script:Report.notes = @($script:Report.notes) + "Graceful close evidence: $($_.Exception.Message)" }
}

function Invoke-ResourceAcceptance {
    $script:Report.resource_measurements = [ordered]@{ collection_status = 'IN_PROGRESS'; rust_one = $null; rust_six = $null }
    $resourceRoot = Join-Path $script:RunDirectory 'resource-fixtures'
    $single = New-Fixture (Join-Path $resourceRoot 'one') 'single'
    $sixWorkspace = New-Fixture (Join-Path $resourceRoot 'six') 'doc-01'
    $sixPaths = @($sixWorkspace.Path)
    for ($index = 2; $index -le 6; $index++) {
        $docPath = Join-Path $sixWorkspace.Directory ("doc-{0:D2}.md" -f $index)
        Write-Utf8NoBom $docPath $sixWorkspace.Source
        $sixPaths += $docPath
    }
    $singleRun = Join-Path $script:RunDirectory 'resource-one.json'
    $script:Report.process_runs = @((Start-RustApp @($single.Path) $singleRun))
    $oneState = Wait-Diagnostic { param($d) $d.tabs -eq 1 } 8000 'one-document Release layout'
    Assert-True (-not $oneState.dirty) 'Opening the one-document resource fixture dirtied its source.'
    $one = Get-ResourceSample $script:Process 1 $ExePath
    $script:Report.resource_measurements['rust_one'] = $one
    $script:Report.resource_measurements['collection_status'] = 'PARTIAL'
    Close-LaunchedApp
    $script:Process.Dispose()
    $script:Process = $null
    $script:WindowHandle = [IntPtr]::Zero

    $sixRun = Join-Path $script:RunDirectory 'resource-six.json'
    $script:Report.process_runs = @($script:Report.process_runs) + (Start-RustApp $sixPaths $sixRun)
    $sixState = Wait-Diagnostic { param($d) $d.tabs -eq 6 } 10000 'six-document Release layout'
    Assert-True (-not $sixState.dirty) 'Opening the six-document resource fixtures dirtied the active source.'
    $six = Get-ResourceSample $script:Process 6 $ExePath
    $script:Report.resource_measurements['rust_six'] = $six
    $script:Report.resource_measurements['collection_status'] = 'COMPLETE'
    Close-LaunchedApp
    $script:Process.Dispose()
    $script:Process = $null
    $script:WindowHandle = [IntPtr]::Zero
    $script:DiagnosticPath = $null
    $script:Report.fixture_fingerprint = Get-FixtureFingerprint $resourceRoot
    if ($one.status -eq 'MEASURED' -and $six.status -eq 'MEASURED') {
        Add-Check 'R9-resource' 'PASS' 'Fresh Release executable measurements recorded WorkingSet, private usage, peak WorkingSet and five-second idle CPU for one and six open documents after matching geometry setup.'
    }
    else {
        $rustResourceReasons = @()
        if ($one.status -ne 'MEASURED') { $rustResourceReasons += "one_document: $($one.reason)" }
        if ($six.status -ne 'MEASURED') { $rustResourceReasons += "six_documents: $($six.reason)" }
        Add-Check 'R9-resource' 'BLOCKED' ('A controlled Rust resource sample was not established: ' + ($rustResourceReasons -join ' | '))
    }
    if ($one.status -ne 'MEASURED' -or $six.status -ne 'MEASURED') {
        Add-Check 'R9-budget' 'BLOCKED' 'The 50,000,000-byte budget cannot be evaluated without both controlled one- and six-document samples.'
    }
    elseif ($one.working_set_bytes -le 50000000 -and $six.working_set_bytes -le 50000000) {
        Add-Check 'R9-budget' 'PASS' 'Both one- and six-document WorkingSet samples were at or below 50,000,000 bytes.'
    }
    else {
        Add-Check 'R9-budget' 'FAIL' 'At least one WorkingSet sample exceeded 50,000,000 bytes.'
    }

    $cppOne = [pscustomobject]@{ status = 'NOT_COMPARED'; documents_requested = 1; reason = 'The C++ one-document measurement was not started.' }
    $cppSix = [pscustomobject]@{ status = 'NOT_COMPARED'; documents_requested = 6; reason = 'The C++ six-document measurement was not started.' }
    $cppBaselineFailure = $null
    try { $script:Report.cpp_baseline_identity = Get-CppBaselineIdentity }
    catch { $cppBaselineFailure = $_.Exception.Message }
    if ($null -ne $cppBaselineFailure) {
        $cppOne = [pscustomobject]@{ status = 'NOT_COMPARED'; documents_requested = 1; reason = "C++ baseline identity failed: $cppBaselineFailure" }
        $cppSix = [pscustomobject]@{ status = 'NOT_COMPARED'; documents_requested = 6; reason = 'The six-document comparison was not attempted because the baseline identity failed.' }
    }
    else {
        try { $cppOne = Measure-CppDocuments @($single.Path) 1 }
        catch { $cppOne = [pscustomobject]@{ status = 'BLOCKED'; documents_requested = 1; reason = $_.Exception.Message } }
        if ($cppOne.status -eq 'MEASURED') {
            try { $cppSix = Measure-CppDocuments $sixPaths 6 }
            catch { $cppSix = [pscustomobject]@{ status = 'BLOCKED'; documents_requested = 6; reason = $_.Exception.Message } }
        }
        else {
            $cppSix = [pscustomobject]@{ status = 'NOT_COMPARED'; documents_requested = 6; reason = 'The one-document C++ startup count/measurement was not established.' }
        }
    }
    $cppMeasurementStatus = if ($cppOne.status -eq 'MEASURED' -and $cppSix.status -eq 'MEASURED') { 'MEASURED' } elseif ($cppOne.status -eq 'BLOCKED' -or $cppSix.status -eq 'BLOCKED') { 'BLOCKED' } elseif ($cppOne.status -eq 'MEASURED') { 'PARTIAL' } else { 'NOT_COMPARED' }
    $geometryComparisons = @()
    $cppComparisonStatus = 'BLOCKED'
    $cppReasons = @()
    if ($one.status -ne 'MEASURED') { $cppReasons += "Rust one_document: $($one.reason)" }
    if ($six.status -ne 'MEASURED') { $cppReasons += "Rust six_documents: $($six.reason)" }
    if ($cppOne.status -ne 'MEASURED') { $cppReasons += "C++ one_document: $($cppOne.reason)" }
    if ($cppSix.status -ne 'MEASURED') { $cppReasons += "C++ six_documents: $($cppSix.reason)" }
    if ($one.status -eq 'MEASURED' -and $six.status -eq 'MEASURED' -and $cppOne.status -eq 'MEASURED' -and $cppSix.status -eq 'MEASURED') {
        $geometryComparisons = @(
            (Compare-ResourceSampleGeometry $one $cppOne.sample 1)
            (Compare-ResourceSampleGeometry $six $cppSix.sample 6)
        )
        foreach ($comparison in $geometryComparisons) {
            if ($comparison.status -ne 'PASS') { $cppReasons += @($comparison.reasons | ForEach-Object { "$($comparison.documents)-document geometry: $_" }) }
        }
        if ($geometryComparisons.status -notcontains 'BLOCKED') { $cppComparisonStatus = 'PASS' }
    }
    $script:Report.cpp_resource_measurements = [ordered]@{ status = $cppComparisonStatus; measurement_status = $cppMeasurementStatus; one_document = $cppOne; six_documents = $cppSix; geometry_comparison = $geometryComparisons; fixture_fingerprint = $script:Report.fixture_fingerprint }
    if ($cppComparisonStatus -eq 'PASS') {
        Add-Check 'R9-cpp-comparison' 'PASS' 'Rust and C++ one- and six-document Release samples matched normal visible window and client dimensions, 96 DPI, and exact foreground HWND/PID ownership; each CPU sample used five seconds.'
    }
    else {
        Add-Check 'R9-cpp-comparison' 'BLOCKED' ('Rust/C++ comparison requires measured 1/6-document samples with matching normal window/client dimensions, 96 DPI, and exact foreground ownership: ' + ($cppReasons -join ' | '))
    }
}

function Write-Report {
    if ($null -eq $script:RunDirectory) { return }
    $script:Report.product_source_fingerprint_end = Get-ProductSourceFingerprint
    $script:Report.source_stable_during_run = $script:Report.product_source_fingerprint_start.sha256 -eq $script:Report.product_source_fingerprint_end.sha256
    $script:Report.executable_sha256_end = (Get-FileHash -LiteralPath $ExePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.executable_stable_during_run = $script:Report.executable_sha256 -eq $script:Report.executable_sha256_end
    if (-not $script:Report.source_stable_during_run) { $script:Report.invalidations = @($script:Report.invalidations) + 'Product source changed during this run; interpret GUI evidence as bound to the recorded executable hash, not the final source tree.' }
    if (-not $script:Report.executable_stable_during_run) { $script:Report.invalidations = @($script:Report.invalidations) + 'The target executable path changed during this run; process evidence remains bound to the per-process executable SHA-256.' }
    $statuses = @($script:Report.checks | ForEach-Object status)
    if ($statuses -contains 'FAIL') { $script:Report.status = 'FAIL' }
    elseif ($statuses -contains 'BLOCKED') { $script:Report.status = 'BLOCKED' }
    elseif ($statuses.Count -gt 0 -and $statuses -notcontains 'NOT_RUN') { $script:Report.status = 'PASS' }
    else { $script:Report.status = 'NOT_RUN' }
    $script:Report.finished_utc = [DateTime]::UtcNow.ToString('o')
    $script:Report.process_still_running = $false
    if ($null -ne $script:Process) {
        try { $script:Process.Refresh(); $script:Report.process_still_running = -not $script:Process.HasExited } catch { }
    }
    $path = Join-Path $script:RunDirectory 'acceptance-report.json'
    $json = $script:Report | ConvertTo-Json -Depth 14
    Write-Utf8NoBom $path $json
    Write-Output $path
}

try {
    $script:Report.product_source_fingerprint_start = Get-ProductSourceFingerprint
    $script:RunDirectory = New-RunDirectory
    if ($Mode -eq 'Focused') { Invoke-FocusedAcceptance }
    else { Invoke-ResourceAcceptance }
}
catch {
    Add-Check 'runner' 'FAIL' $_.Exception.Message
    $script:Report.notes = @($script:Report.notes) + $_.ScriptStackTrace
}
finally {
    try {
        try { Restore-Clipboard } catch { Add-Check 'clipboard-restore' 'FAIL' $_.Exception.Message }
        if (@($script:OwnedExplorerWindows).Count -gt 0) {
            $script:Report.shell_drop_cleanup = @(Close-OwnedExplorerWindows)
            if (@($script:Report.shell_drop_cleanup | Where-Object { -not $_.closed_normally }).Count -gt 0) {
                Add-Check 'R6-explorer-window-close' 'BLOCKED' 'An Explorer window created for the synthetic file-drop test remained open after normal close; it was not force-closed.'
                throw 'The task-owned Explorer drop window did not close normally.'
            }
            if ($script:Report.shell_drop_cleanup.Count -gt 0) { Add-Check 'R6-explorer-window-close' 'PASS' 'The new Explorer window closed through bounded WM_CLOSE.' }
        }
        Close-LaunchedApp
    }
    finally {
        try {
            if ([MdliteRustGuiNative]::HasClipboardOwnerWindow()) {
                $script:Report.clipboard_owner.destroyed = [MdliteRustGuiNative]::DestroyClipboardOwnerWindow()
                $script:Report.clipboard_owner.state = 'DESTROYED'
                $script:ClipboardOwnerHandle = [IntPtr]::Zero
            }
        }
        catch {
            $script:Report.clipboard_owner.state = 'DESTROY_FAILED'
            Add-Check 'R4-clipboard-owner-window' 'FAIL' $_.Exception.Message
        }
        try { Write-Report } catch { Write-Error "Could not write acceptance report: $($_.Exception.Message)" }
    }
}

if ($script:Report.status -eq 'FAIL') { exit 1 }
if ($script:Report.status -eq 'BLOCKED') { exit 2 }
