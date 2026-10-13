[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$ProjectRoot = Split-Path -Parent $PSScriptRoot
$ExecutablePath = Join-Path $ProjectRoot 'target\release\mdlite.exe'
$RunRoot = Join-Path $ProjectRoot '.codex-runtime\editor-acceptance'
$TimeoutMs = 7000
$DiagnosticRefreshMessage = 0x8001
$WM_CLOSE = 0x0010
$SMTO_ABORTIFHUNG = 0x0002

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public static class MdliteRustAcceptanceNative
{
    private const uint INPUT_KEYBOARD = 1;
    private const uint KEYEVENTF_KEYUP = 0x0002;
    private const uint KEYEVENTF_UNICODE = 0x0004;

    [StructLayout(LayoutKind.Sequential)]
    private struct KEYBDINPUT
    {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct MOUSEINPUT
    {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint dwFlags;
        public uint time;
        public UIntPtr dwExtraInfo;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct HARDWAREINPUT
    {
        public uint uMsg;
        public ushort wParamL;
        public ushort wParamH;
    }

    [StructLayout(LayoutKind.Explicit)]
    private struct INPUTUNION
    {
        [FieldOffset(0)] public MOUSEINPUT mi;
        [FieldOffset(0)] public KEYBDINPUT ki;
        [FieldOffset(0)] public HARDWAREINPUT hi;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct INPUT
    {
        public uint type;
        public INPUTUNION data;
    }

    public static int AssertInputLayout()
    {
        int expected = IntPtr.Size == 8 ? 40 : 28;
        int actual = Marshal.SizeOf(typeof(INPUT));
        if (actual != expected)
            throw new InvalidOperationException(
                "Win32 INPUT layout mismatch: expected " + expected + " bytes, got " + actual + ".");
        return actual;
    }

    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr window);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetDpiForWindow(IntPtr window);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);

    [DllImport("user32.dll")]
    private static extern IntPtr GetKeyboardLayout(uint threadId);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, ExactSpelling = true)]
    private static extern short VkKeyScanExW(char character, IntPtr keyboardLayout);

    [DllImport("user32.dll")]
    private static extern short GetKeyState(int virtualKey);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetClassNameW(IntPtr window, StringBuilder className, int capacity);

    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int GetWindowTextW(IntPtr window, StringBuilder title, int capacity);

    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool SendMessageTimeoutW(
        IntPtr window, uint message, IntPtr wParam, IntPtr lParam,
        uint flags, uint timeoutMs, out IntPtr result);

    [DllImport("user32.dll", SetLastError = true)]
    private static extern uint SendInput(uint count, INPUT[] inputs, int inputSize);

    public static string WindowClass(IntPtr window)
    {
        StringBuilder value = new StringBuilder(256);
        GetClassNameW(window, value, value.Capacity);
        return value.ToString();
    }

    public static string WindowTitle(IntPtr window)
    {
        StringBuilder value = new StringBuilder(512);
        GetWindowTextW(window, value, value.Capacity);
        return value.ToString();
    }

    public static bool IsOwnedWindow(IntPtr window, uint expectedProcessId)
    {
        if (window == IntPtr.Zero || !IsWindow(window))
            return false;
        uint actualProcessId;
        GetWindowThreadProcessId(window, out actualProcessId);
        return actualProcessId == expectedProcessId;
    }

    public static IntPtr OwnedWindowKeyboardLayout(IntPtr window, uint expectedProcessId)
    {
        if (!IsOwnedWindow(window, expectedProcessId))
            return IntPtr.Zero;
        uint actualProcessId;
        uint threadId = GetWindowThreadProcessId(window, out actualProcessId);
        if (threadId == 0 || actualProcessId != expectedProcessId)
            return IntPtr.Zero;
        return GetKeyboardLayout(threadId);
    }

    public static short MapAsciiCharacter(char character, IntPtr keyboardLayout)
    {
        return VkKeyScanExW(character, keyboardLayout);
    }

    public static bool CapsLockIsOn()
    {
        return (GetKeyState(0x14) & 1) != 0;
    }

    private static bool IsOwnedForeground(IntPtr expectedWindow, uint expectedProcessId)
    {
        return GetForegroundWindow() == expectedWindow &&
               IsOwnedWindow(expectedWindow, expectedProcessId);
    }

    private static INPUT KeyInput(ushort virtualKey, ushort scanCode, uint flags)
    {
        INPUT input = new INPUT();
        input.type = INPUT_KEYBOARD;
        input.data.ki.wVk = virtualKey;
        input.data.ki.wScan = scanCode;
        input.data.ki.dwFlags = flags;
        input.data.ki.time = 0;
        input.data.ki.dwExtraInfo = UIntPtr.Zero;
        return input;
    }

    public static int SendVirtualKey(
        IntPtr expectedWindow, uint expectedProcessId,
        ushort virtualKey, ushort[] modifiers)
    {
        if (!IsOwnedForeground(expectedWindow, expectedProcessId))
            return -1;
        List<INPUT> events = new List<INPUT>();
        if (modifiers != null)
        {
            for (int i = 0; i < modifiers.Length; i++)
                events.Add(KeyInput(modifiers[i], 0, 0));
        }
        events.Add(KeyInput(virtualKey, 0, 0));
        events.Add(KeyInput(virtualKey, 0, KEYEVENTF_KEYUP));
        if (modifiers != null)
        {
            for (int i = modifiers.Length - 1; i >= 0; i--)
                events.Add(KeyInput(modifiers[i], 0, KEYEVENTF_KEYUP));
        }
        INPUT[] inputs = events.ToArray();
        return (int)SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(INPUT)));
    }

    public static int SendUnicodeText(
        IntPtr expectedWindow, uint expectedProcessId, string text)
    {
        if (!IsOwnedForeground(expectedWindow, expectedProcessId))
            return -1;
        List<INPUT> events = new List<INPUT>();
        foreach (char unit in text)
        {
            events.Add(KeyInput(0, unit, KEYEVENTF_UNICODE));
            events.Add(KeyInput(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
        }
        INPUT[] inputs = events.ToArray();
        return (int)SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(INPUT)));
    }
}
'@

$InputSizeBytes = [MdliteRustAcceptanceNative]::AssertInputLayout()
$script:Process = $null
$script:WindowHandle = [IntPtr]::Zero
$script:ProcessId = [uint32]0
$script:DiagnosticPath = $null
$script:PromptOpen = $false
$script:ReportPath = $null
$script:ForegroundLogPath = $null
$script:ImeInitialOpen = $null
$script:ImeProfile = $null
$script:ImeRestorePending = $false
$script:ImeRestoreAttempted = $false
$script:Report = [ordered]@{
    schema_version = 1
    status = 'RUNNING'
    configuration = 'Release'
    executable_path = [IO.Path]::GetFullPath($ExecutablePath)
    executable_sha256_before = $null
    executable_sha256_after = $null
    source_tree_sha256_before = $null
    source_tree_sha256_after = $null
    git_head = $null
    harness_sha256 = (Get-FileHash -LiteralPath $PSCommandPath -Algorithm SHA256).Hash.ToLowerInvariant()
    input_size_bytes = $InputSizeBytes
    pid = $null
    hwnd = $null
    visible = $null
    dpi = $null
    foreground_log_path = $null
    foreground_acquired_utc = $null
    active_input_profile = $null
    ime_open_status = $null
    input_delivery = 'Visible Release app; ASCII uses current-layout virtual-key SendInput; Japanese packet text is a separate Unicode path; commands and modifiers use virtual keys through the normal window message loop.'
    ascii_keyboard_layout = $null
    ascii_caps_lock_on = $null
    ime_status = 'PENDING'
    ime_open_initial = $null
    ime_open_restored = $null
    physical_typematic_status = 'NOT_RUN: physical keyboard repeat was not exercised.'
    discrete_repeat_status = 'PENDING'
    tabs = $null
    fixtures = @()
    operations = @()
    unsaved_close_prompt = $null
    normal_close = $null
    process_still_running = $null
    failure = $null
    result_path = $null
}

function Get-SourceTreeSha256 {
    $files = @()
    foreach ($name in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml', 'build.rs')) {
        $path = Join-Path $ProjectRoot $name
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $files += Get-Item -LiteralPath $path
        }
    }
    $files += @(Get-ChildItem -LiteralPath (Join-Path $ProjectRoot 'src') -Filter '*.rs' -File -Recurse | Sort-Object FullName)
    $entries = foreach ($file in ($files | Sort-Object FullName -Unique)) {
        $relative = $file.FullName.Substring($ProjectRoot.Length).TrimStart('\').Replace('\', '/')
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        "$relative $hash"
    }
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]::Join(([char]10).ToString(), [string[]]$entries))
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return ([BitConverter]::ToString($sha.ComputeHash($bytes))).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
    }
}

function ConvertTo-WindowsArgument([string]$Value) {
    if ($Value -notmatch '[\s"]') { return $Value }
    $builder = New-Object System.Text.StringBuilder
    [void]$builder.Append('"')
    $slashes = 0
    foreach ($character in $Value.ToCharArray()) {
        if ($character -eq [char]'\') {
            $slashes++
        } elseif ($character -eq [char]'"') {
            [void]$builder.Append([char]'\', (2 * $slashes + 1))
            [void]$builder.Append([char]'"')
            $slashes = 0
        } else {
            if ($slashes -gt 0) { [void]$builder.Append([char]'\', $slashes) }
            [void]$builder.Append($character)
            $slashes = 0
        }
    }
    if ($slashes -gt 0) { [void]$builder.Append([char]'\', (2 * $slashes)) }
    [void]$builder.Append('"')
    return $builder.ToString()
}

function Assert-OwnedWindow([IntPtr]$Window) {
    if (-not [MdliteRustAcceptanceNative]::IsOwnedWindow($Window, $script:ProcessId)) {
        throw 'The target HWND is no longer owned by the launched Release process.'
    }
}

function Set-OwnedForeground(
    [IntPtr]$Window = $script:WindowHandle,
    [int]$WaitMs = 5000
) {
    Assert-OwnedWindow $Window
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $activationLogged = $false
    do {
        Assert-OwnedWindow $Window
        $foreground = [MdliteRustAcceptanceNative]::GetForegroundWindow()
        [uint32]$foregroundProcessId = 0
        if ($foreground -ne [IntPtr]::Zero) {
            [void][MdliteRustAcceptanceNative]::GetWindowThreadProcessId(
                $foreground, [ref]$foregroundProcessId)
        }
        if ($foreground -eq $Window -and $foregroundProcessId -eq $script:ProcessId) {
            if ($activationLogged) {
                $confirmed = 'FOREGROUND_CONFIRMED pid={0} hwnd=0x{1:X} foreground_pid={2} foreground_hwnd=0x{3:X}' -f
                    $script:ProcessId, $Window.ToInt64(), $foregroundProcessId, $foreground.ToInt64()
                Write-Host $confirmed
                if (-not [string]::IsNullOrWhiteSpace($script:ForegroundLogPath)) {
                    [IO.File]::AppendAllText(
                        $script:ForegroundLogPath, $confirmed + [Environment]::NewLine,
                        [Text.UTF8Encoding]::new($false))
                }
            }
            if ($null -eq $script:Report.foreground_acquired_utc) {
                $script:Report.foreground_acquired_utc = [DateTime]::UtcNow.ToString('o')
            }
            return
        }
        if (-not $activationLogged) {
            $required = 'FOREGROUND_REQUIRED pid={0} hwnd=0x{1:X}' -f
                $script:ProcessId, $Window.ToInt64()
            Write-Host $required
            if (-not [string]::IsNullOrWhiteSpace($script:ForegroundLogPath)) {
                [IO.File]::AppendAllText(
                    $script:ForegroundLogPath, $required + [Environment]::NewLine,
                    [Text.UTF8Encoding]::new($false))
            }
            $activationLogged = $true
        }
        [void][MdliteRustAcceptanceNative]::SetForegroundWindow($Window)
        if ([DateTime]::UtcNow -ge $deadline) {
            throw "The verified owned HWND did not become foreground within $WaitMs ms."
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "The verified owned HWND did not become foreground within $WaitMs ms."
}

function Send-Key(
    [IntPtr]$TargetWindow,
    [int]$VirtualKey,
    [uint16[]]$Modifiers = @()
) {
    Set-OwnedForeground -Window $TargetWindow
    $expected = 2 + (2 * $Modifiers.Length)
    $sent = [MdliteRustAcceptanceNative]::SendVirtualKey(
        $TargetWindow, $script:ProcessId, [uint16]$VirtualKey, $Modifiers)
    if ($sent -ne $expected) {
        throw "SendInput delivered $sent of $expected key events to the verified owned foreground window."
    }
}

function Send-UnicodeText([string]$Text) {
    Set-OwnedForeground
    $expected = 2 * $Text.Length
    $sent = [MdliteRustAcceptanceNative]::SendUnicodeText(
        $script:WindowHandle, $script:ProcessId, $Text)
    if ($sent -ne $expected) {
        throw "SendInput delivered $sent of $expected Unicode key events to the verified owned foreground window."
    }
}

function Send-Ascii([string]$Text) {
    if ([string]::IsNullOrEmpty($Text)) { return }
    Set-OwnedForeground
    $keyboardLayout = [MdliteRustAcceptanceNative]::OwnedWindowKeyboardLayout(
        $script:WindowHandle, $script:ProcessId)
    if ($keyboardLayout -eq [IntPtr]::Zero) {
        throw 'The exact owned window did not expose its current keyboard layout.'
    }
    $capsLockOn = [MdliteRustAcceptanceNative]::CapsLockIsOn()
    if ($null -eq $script:Report.ascii_caps_lock_on) {
        $script:Report.ascii_caps_lock_on = $capsLockOn
        $script:Report.ascii_keyboard_layout = '0x{0:X}' -f $keyboardLayout.ToInt64()
    } elseif ([bool]$script:Report.ascii_caps_lock_on -ne $capsLockOn -or
              [string]$script:Report.ascii_keyboard_layout -cne ('0x{0:X}' -f $keyboardLayout.ToInt64())) {
        throw 'The observed CapsLock state or owned-window keyboard layout changed during ASCII input.'
    }

    $mappedKeys = @()
    foreach ($character in $Text.ToCharArray()) {
        $code = [int]$character
        if ($code -lt 0x20 -or $code -gt 0x7e) {
            throw 'Send-Ascii accepts printable ASCII characters only.'
        }
        $mapped = [MdliteRustAcceptanceNative]::MapAsciiCharacter($character, $keyboardLayout)
        if ($mapped -eq -1) {
            throw ('The current keyboard layout cannot produce ASCII U+{0:X4}.' -f $code)
        }
        $shiftState = ([int]$mapped -shr 8) -band 0xff
        if (($shiftState -band 0xfe) -ne 0) {
            throw ('ASCII U+{0:X4} requires Ctrl/Alt in the current layout; refusing that mapping.' -f $code)
        }
        $isLetter = ($code -ge 0x41 -and $code -le 0x5a) -or
                    ($code -ge 0x61 -and $code -le 0x7a)
        if ($capsLockOn -and $isLetter) { $shiftState = $shiftState -bxor 1 }
        [uint16[]]$modifiers = @()
        if ($shiftState -band 1) { $modifiers = [uint16[]]@(0x10) }
        $mappedKeys += [pscustomobject]@{
            virtual_key = [int]([int]$mapped -band 0xff)
            modifiers = $modifiers
        }
    }

    foreach ($key in $mappedKeys) {
        $currentLayout = [MdliteRustAcceptanceNative]::OwnedWindowKeyboardLayout(
            $script:WindowHandle, $script:ProcessId)
        if ($currentLayout -ne $keyboardLayout) {
            throw 'The owned-window keyboard layout changed during ASCII input; no profile switch was requested.'
        }
        if ([MdliteRustAcceptanceNative]::CapsLockIsOn() -ne $capsLockOn) {
            throw 'CapsLock changed during ASCII input; the harness did not modify it.'
        }
        Send-Key $script:WindowHandle $key.virtual_key $key.modifiers
    }
}

function Assert-SelectionBoundary($State) {
    $bytes = [Text.Encoding]::UTF8.GetBytes([string]$State.source)
    foreach ($offset in @([int]$State.selection.anchor, [int]$State.selection.head)) {
        if ($offset -lt 0 -or $offset -gt $bytes.Length) {
            throw "Diagnostic selection offset $offset is outside the UTF-8 source."
        }
        if ($offset -lt $bytes.Length -and (($bytes[$offset] -band 0xC0) -eq 0x80)) {
            throw "Diagnostic selection offset $offset splits a UTF-8 scalar."
        }
    }
}

function Read-Diagnostic([int]$MessageTimeoutMs = 1000) {
    Assert-OwnedWindow $script:WindowHandle
    $previousWriteUtc = $null
    try {
        if (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf) {
            $previousWriteUtc = [IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath)
        }
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot stat: ' + $_.Exception.Message), $_.Exception)
    }
    $messageResult = [IntPtr]::Zero
    $sent = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$DiagnosticRefreshMessage,
        [IntPtr]::Zero, [IntPtr]::Zero, [uint32]$SMTO_ABORTIFHUNG,
        [uint32]$MessageTimeoutMs, [ref]$messageResult)
    Assert-OwnedWindow $script:WindowHandle
    if (-not $sent) {
        throw [TimeoutException]::new('SendMessageTimeout did not complete.')
    }
    if ($messageResult.ToInt64() -ne 1) {
        throw [TimeoutException]::new('WM_APP+1 returned LRESULT 0; the owner-thread refresh was deferred.')
    }
    $writeUtc = $null
    try {
        if (-not (Test-Path -LiteralPath $script:DiagnosticPath -PathType Leaf)) {
            throw [TimeoutException]::new('WM_APP+1 returned 1 but no diagnostic snapshot file exists yet.')
        }
        $writeUtc = [IO.File]::GetLastWriteTimeUtc($script:DiagnosticPath)
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot stat: ' + $_.Exception.Message), $_.Exception)
    }
    if ($null -ne $previousWriteUtc -and $writeUtc -le $previousWriteUtc) {
        throw [TimeoutException]::new('WM_APP+1 returned 1 but the diagnostic snapshot timestamp did not advance.')
    }
    try {
        $json = [IO.File]::ReadAllText($script:DiagnosticPath, [Text.Encoding]::UTF8)
    } catch [IO.IOException] {
        throw [TimeoutException]::new(('Transient diagnostic snapshot I/O: ' + $_.Exception.Message), $_.Exception)
    }
    try {
        $state = $json | ConvertFrom-Json -ErrorAction Stop
    } catch {
        throw [TimeoutException]::new(('Diagnostic snapshot JSON is not complete yet: ' + $_.Exception.Message), $_.Exception)
    }
    if ($null -eq $state) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; object is unavailable.')
    }
    $sourceProperty = $state.PSObject.Properties['source']
    $selectionProperty = $state.PSObject.Properties['selection']
    if ($null -eq $sourceProperty -or $null -eq $selectionProperty -or
        $null -eq $sourceProperty.Value -or $null -eq $selectionProperty.Value) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; source or selection is unavailable.')
    }
    $selection = $selectionProperty.Value
    if ($sourceProperty.Value -isnot [string]) {
        throw [TimeoutException]::new('Diagnostic snapshot JSON is incomplete; source is not a string.')
    }
    foreach ($name in @('source', 'selection', 'revision', 'dirty', 'undo_count', 'redo_count', 'raw', 'tables', 'ime', 'caret', 'native_caret')) {
        $property = $state.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic snapshot is incomplete; missing '$name'.")
        }
    }
    foreach ($name in @('anchor', 'head')) {
        $property = $selection.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic selection is incomplete; missing '$name'.")
        }
        if ($property.Value -isnot [int] -and $property.Value -isnot [long]) {
            throw [TimeoutException]::new("Diagnostic selection is incomplete; '$name' is not an integer.")
        }
    }
    foreach ($name in @('active', 'preedit', 'open_status')) {
        $property = $state.ime.PSObject.Properties[$name]
        if ($null -eq $property -or $null -eq $property.Value) {
            throw [TimeoutException]::new("Diagnostic IME state is incomplete; missing '$name'.")
        }
    }
    Assert-SelectionBoundary $state
    return $state
}

function Wait-Diagnostic(
    [scriptblock]$Condition,
    [string]$Description,
    [int]$WaitMs = $TimeoutMs
) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    $attempts = 0
    $lastResponse = 'no diagnostic request completed'
    do {
        $attempts++
        $remainingMs = [Math]::Max(1, [Math]::Min(1000, [int]($deadline - [DateTime]::UtcNow).TotalMilliseconds))
        try {
            $state = Read-Diagnostic $remainingMs
            $lastResponse = 'WM_APP+1 returned 1; fresh valid snapshot revision={0}, selection={1}..{2}, dirty={3}; condition false' -f
                $state.revision, $state.selection.anchor, $state.selection.head, $state.dirty
            if (& $Condition $state) { return $state }
        } catch [TimeoutException] {
            $lastResponse = $_.Exception.Message
        }
        if ([DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 60 }
    } while ([DateTime]::UtcNow -lt $deadline)
    throw ('Timed out after ' + $WaitMs + ' ms waiting for ' + $Description +
        ' (' + $attempts + ' attempts; last observer response: ' + $lastResponse + ').')
}

function Wait-Document(
    [string]$ExpectedSource,
    [int]$ExpectedAnchor,
    [int]$ExpectedHead,
    [bool]$ExpectedDirty
) {
    return Wait-Diagnostic {
        param($state)
        $state.source -ceq $ExpectedSource -and
        [int]$state.selection.anchor -eq $ExpectedAnchor -and
        [int]$state.selection.head -eq $ExpectedHead -and
        [bool]$state.dirty -eq $ExpectedDirty
    } "source/selection/dirty state for '$ExpectedSource'"
}

function Add-Observation([string]$Name, $State) {
    $script:Report.operations += [pscustomobject]@{
        name = $Name
        revision = [int]$State.revision
        selection = $State.selection
        dirty = [bool]$State.dirty
        undo_count = [int]$State.undo_count
        redo_count = [int]$State.redo_count
        raw = [bool]$State.raw
        table_count = [int]$State.tables
        ime_active = [bool]$State.ime.active
        preedit_length = ([string]$State.ime.preedit).Length
        caret = $State.caret
        native_caret = $State.native_caret
    }
}

function Test-SameInputProfile($State) {
    if ($null -eq $script:ImeProfile -or $null -eq $State.active_input_profile) { return $false }
    foreach ($name in @('status', 'profile_type', 'language_id', 'clsid', 'profile_guid', 'hkl')) {
        $expected = $script:ImeProfile.PSObject.Properties[$name]
        $actual = $State.active_input_profile.PSObject.Properties[$name]
        if ($null -eq $expected -or $null -eq $actual -or [string]$expected.Value -cne [string]$actual.Value) {
            return $false
        }
    }
    return $true
}

function Wait-ImeOpenState([bool]$ExpectedOpen, [string]$ExpectedSource) {
    return Wait-Diagnostic {
        param($state)
        (Test-SameInputProfile $state) -and
        [bool]$state.ime.open_status.available -and
        [bool]$state.ime.open_status.open -eq $ExpectedOpen -and
        $state.source -ceq $ExpectedSource
    } ('app-local IME open state ' + $ExpectedOpen)
}

function Restore-ImeOpenState {
    if (-not $script:ImeRestorePending -or $script:ImeRestoreAttempted) { return }
    $script:ImeRestoreAttempted = $true
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic before IME restoration'
    if (-not (Test-SameInputProfile $state)) {
        throw 'The observed input profile changed during the app-local IME check; no profile-switching key was sent.'
    }
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x1B
        $state = Wait-Diagnostic {
            param($value)
            (Test-SameInputProfile $value) -and
            -not [bool]$value.ime.active -and
            [string]::IsNullOrEmpty([string]$value.ime.preedit)
        } 'cancellation of any remaining owned composition'
    }
    if (-not [bool]$state.ime.open_status.available) {
        throw 'The owned app-local IME open state became unavailable before restoration.'
    }
    $currentOpen = [bool]$state.ime.open_status.open
    if ($currentOpen -ne [bool]$script:ImeInitialOpen) {
        Set-OwnedForeground
        $key = if ($script:ImeInitialOpen) { 0x16 } else { 0x1A }
        Send-Key $script:WindowHandle $key
        $state = Wait-ImeOpenState ([bool]$script:ImeInitialOpen) ([string]$state.source)
    }
    $script:Report.ime_open_restored = [bool]$state.ime.open_status.open
    $script:ImeRestorePending = $false
    if ([bool]$state.ime.open_status.open -ne [bool]$script:ImeInitialOpen) {
        throw 'The owned app-local IME did not return to its original open state.'
    }
}

function Invoke-ConfiguredImeAcceptance([string]$ExpectedSource, [int]$ExpectedHead) {
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic before IME acceptance'
    $profile = $state.active_input_profile
    $open = $state.ime.open_status
    $profileDescription = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.description }
    $profileType = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.profile_type }
    $profileLanguage = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.language_id }
    $profileStatus = if ($null -eq $profile) { 'unavailable' } else { [string]$profile.status }
    if ($null -eq $profile -or [string]$profile.status -cne 'active' -or
        $profileType -cne 'text_service' -or $profileLanguage -cne '0x0411') {
        $script:Report.ime_status = "BLOCKED: the current owner-thread input profile is not an active Japanese text service (description='$profileDescription', language=$profileLanguage, type=$profileType, status=$profileStatus); no input or profile-selection keys were sent."
        return $state
    }
    if ($null -eq $open -or -not [bool]$open.available -or $null -eq $open.open) {
        $reason = if ($null -eq $open) { 'diagnostic field unavailable' } else { [string]$open.error }
        $script:Report.ime_status = "BLOCKED: app-local IME open state is not observable ($reason); no IME keys were sent."
        return $state
    }
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'The owned editor was already composing before the scoped IME acceptance sequence.'
    }

    $script:ImeProfile = $profile
    $script:ImeInitialOpen = [bool]$open.open
    $script:ImeRestorePending = $true
    $script:Report.ime_open_initial = [bool]$script:ImeInitialOpen
    $script:Report.ime_profile_description = $profileDescription
    $script:Report.ime_profile_type = [string]$profile.profile_type
    $script:Report.ime_profile_language = [string]$profile.language_id

    try {
        if ([bool]$script:ImeInitialOpen) {
            Set-OwnedForeground
            Send-Key $script:WindowHandle 0x1A
            $state = Wait-ImeOpenState $false $ExpectedSource
            Add-Observation 'app-local IME OFF' $state
        }
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x16
        $state = Wait-ImeOpenState $true $ExpectedSource
        Add-Observation 'app-local IME ON' $state

        $before = $state
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x4B
        Send-Key $script:WindowHandle 0x41
        try {
            $state = Wait-Diagnostic {
                param($value)
                (Test-SameInputProfile $value) -and
                [bool]$value.ime.active -and
                -not [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
                $value.source -ceq $ExpectedSource -and
                [int]$value.selection.anchor -eq $ExpectedHead -and
                [int]$value.selection.head -eq $ExpectedHead
            } 'real configured IME composition after virtual-key romaji'
        } catch {
            $observed = Wait-Diagnostic { param($value) $true } 'fresh diagnostic after IME composition attempt'
            if ($observed.source -ceq $ExpectedSource -and -not [bool]$observed.ime.active) {
                $script:Report.ime_status = "BLOCKED: active profile '$profileDescription' and app-local IME ON were observed, but virtual-key 'ka' produced no native composition/preedit; no other profile was selected."
                return $observed
            }
            throw
        }
        Add-Observation 'configured IME romaji composition' $state
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x20
        $state = Wait-Diagnostic {
            param($value)
            (Test-SameInputProfile $value) -and
            [bool]$value.ime.active -and
            $value.source -ceq $ExpectedSource -and
            [int]$value.selection.head -eq $ExpectedHead
        } 'Space conversion without source mutation'
        Add-Observation 'configured IME Space candidate conversion' $state

        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x1B
        $state = Wait-Diagnostic {
            param($value)
            (Test-SameInputProfile $value) -and
            -not [bool]$value.ime.active -and
            [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
            $value.source -ceq $ExpectedSource -and
            [int]$value.selection.anchor -eq $ExpectedHead -and
            [int]$value.selection.head -eq $ExpectedHead -and
            [int]$value.undo_count -eq [int]$before.undo_count
        } 'Escape cancellation without source or history mutation'
        Add-Observation 'configured IME Escape cancellation' $state

        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x4E
        Send-Key $script:WindowHandle 0x41
        $state = Wait-Diagnostic {
            param($value)
            (Test-SameInputProfile $value) -and
            [bool]$value.ime.active -and
            -not [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
            $value.source -ceq $ExpectedSource
        } 'second configured IME composition before commit'
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x20
        $state = Wait-Diagnostic {
            param($value)
            [bool]$value.ime.active -and $value.source -ceq $ExpectedSource
        } 'second Space conversion before commit'
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x0D
        $baseUndoCount = [int]$state.undo_count
        $state = Wait-Diagnostic {
            param($value)
            (Test-SameInputProfile $value) -and
            -not [bool]$value.ime.active -and
            [string]::IsNullOrEmpty([string]$value.ime.preedit) -and
            $value.source.StartsWith($ExpectedSource, [StringComparison]::Ordinal) -and
            $value.source.Length -gt $ExpectedSource.Length -and
            -not $value.source.Substring($ExpectedSource.Length).Contains([char]10)
        } 'Enter commit of the configured IME conversion'
        if ([int]$state.undo_count -ne ($baseUndoCount + 1)) {
            throw 'Configured IME commit did not create exactly one undo unit.'
        }
        $committedSource = [string]$state.source
        $committedEnd = [Text.Encoding]::UTF8.GetByteCount($committedSource)
        if ([int]$state.selection.anchor -ne $committedEnd -or [int]$state.selection.head -ne $committedEnd) {
            throw 'Configured IME commit left selection/caret away from the committed source end.'
        }
        $script:Report.ime_committed_suffix = $committedSource.Substring($ExpectedSource.Length)
        Add-Observation 'configured IME Enter commit' $state

        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
        $state = Wait-Document $ExpectedSource $ExpectedHead $ExpectedHead ([bool]$before.dirty)
        Add-Observation 'Undo configured IME commit' $state
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
        $state = Wait-Document $committedSource $committedEnd $committedEnd $true
        Add-Observation 'Redo configured IME commit' $state
        $script:Report.ime_status = "PASS: active configured profile '$profileDescription' (language $($profile.language_id)); app-local open state was restored; virtual-key romaji, Space, Escape cancellation, Enter commit, and one-unit Undo/Redo were observed."
    } finally {
        Restore-ImeOpenState
    }
    return $state
}

function Switch-ToSource([string]$ExpectedSource) {
    for ($attempt = 0; $attempt -lt 2; $attempt++) {
        $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic while selecting a fixture tab'
        if ($state.source -ceq $ExpectedSource) { return $state }
        if ([int]$state.tabs -ne 2) { throw 'The owned fixture process no longer has exactly two tabs.' }
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x09 ([uint16[]]@(0x11))
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $ExpectedSource
        } 'Ctrl+Tab switch to the requested synthetic fixture'
    }
    throw 'The requested synthetic fixture was not active after two bounded tab switches.'
}

function Wait-OwnedConfirmation([int]$WaitMs = 2500) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($WaitMs)
    do {
        $window = [MdliteRustAcceptanceNative]::GetForegroundWindow()
        if ($window -ne [IntPtr]::Zero -and
            [MdliteRustAcceptanceNative]::IsOwnedWindow($window, $script:ProcessId) -and
            [MdliteRustAcceptanceNative]::WindowClass($window) -eq '#32770' -and
            [MdliteRustAcceptanceNative]::WindowTitle($window) -eq 'MDLite') {
            return $window
        }
        Start-Sleep -Milliseconds 50
        $script:Process.Refresh()
        if ($script:Process.HasExited) { return [IntPtr]::Zero }
    } while ([DateTime]::UtcNow -lt $deadline)
    return [IntPtr]::Zero
}

function Save-ActiveFixture([string]$ExpectedSource, [string]$Path) {
    $expectedEnd = [Text.Encoding]::UTF8.GetByteCount($ExpectedSource)
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x53 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $ExpectedSource -and
        [int]$value.selection.anchor -eq $expectedEnd -and
        [int]$value.selection.head -eq $expectedEnd -and
        -not [bool]$value.dirty
    } 'save of the active synthetic fixture'
    if ([IO.File]::ReadAllText($Path, [Text.Encoding]::UTF8) -cne $ExpectedSource) {
        throw "The owned fixture did not save with the exact expected source: $Path"
    }
    $bytes = [IO.File]::ReadAllBytes($Path)
    if ($bytes.Length -ge 3 -and $bytes[0] -eq 0xEF -and $bytes[1] -eq 0xBB -and $bytes[2] -eq 0xBF) {
        throw "The UTF-8 no-BOM synthetic fixture acquired an unexpected BOM: $Path"
    }
    Add-Observation ('save synthetic fixture ' + [IO.Path]::GetFileName($Path)) $state
}

try {
    if (-not (Test-Path -LiteralPath $ExecutablePath -PathType Leaf)) {
        throw "Release executable is missing: $ExecutablePath"
    }
    if ($ExecutablePath -notmatch '[\\/]target[\\/]release[\\/]mdlite\.exe$') {
        throw 'Only target/release/mdlite.exe is accepted by this harness.'
    }

    $script:Report.git_head = (& git -C $ProjectRoot rev-parse HEAD).Trim()
    if ($LASTEXITCODE -ne 0) { throw 'Could not pin the current Git HEAD.' }
    $script:Report.executable_sha256_before = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.source_tree_sha256_before = Get-SourceTreeSha256

    $runId = '{0}-{1}' -f [DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ'), [guid]::NewGuid().ToString('N')
    $runDirectory = Join-Path $RunRoot $runId
    [void][IO.Directory]::CreateDirectory($runDirectory)
    $script:ReportPath = Join-Path $runDirectory 'result.json'
    $script:DiagnosticPath = Join-Path $runDirectory 'diagnostic.json'
    $script:ForegroundLogPath = Join-Path $runDirectory 'foreground.log'
    $script:Report.foreground_log_path = $script:ForegroundLogPath
    $blankPath = Join-Path $runDirectory 'synthetic-empty.md'
    $structuredPath = Join-Path $runDirectory 'synthetic-editor.md'

    $newline = [string][char]10
    $fence = ([string][char]96) * 3
    $structuredSource = '| left | right |' + $newline +
        '| --- | --- |' + $newline +
        '| cell | value |' + $newline + $newline +
        $fence + 'rust' + $newline +
        'let source = "Markdown";' + $newline +
        '## asdkl' + $newline +
        $fence + $newline + $newline

    [IO.File]::WriteAllText($blankPath, '', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($structuredPath, $structuredSource, [Text.UTF8Encoding]::new($false))
    $script:Report.fixtures = @(
        @{ path = $blankPath; initial_sha256 = (Get-FileHash -LiteralPath $blankPath -Algorithm SHA256).Hash.ToLowerInvariant() },
        @{ path = $structuredPath; initial_sha256 = (Get-FileHash -LiteralPath $structuredPath -Algorithm SHA256).Hash.ToLowerInvariant() }
    )

    $startInfo = New-Object System.Diagnostics.ProcessStartInfo
    $startInfo.FileName = $ExecutablePath
    $startInfo.WorkingDirectory = $ProjectRoot
    $startInfo.UseShellExecute = $false
    $startInfo.Arguments = (ConvertTo-WindowsArgument $blankPath) + ' ' +
        (ConvertTo-WindowsArgument $structuredPath) + ' --diagnostic-path ' +
        (ConvertTo-WindowsArgument $script:DiagnosticPath)
    $script:Process = [Diagnostics.Process]::Start($startInfo)
    if ($null -eq $script:Process) { throw 'Could not start the pinned Release executable.' }
    $script:ProcessId = [uint32]$script:Process.Id
    $script:Report.pid = [int]$script:ProcessId

    $windowDeadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $script:Process.Refresh()
        if ($script:Process.HasExited) { throw 'The owned Release process exited before creating its main window.' }
        $script:WindowHandle = $script:Process.MainWindowHandle
        if ($script:WindowHandle -ne [IntPtr]::Zero -and
            [MdliteRustAcceptanceNative]::WindowClass($script:WindowHandle) -eq 'MDLiteRustEditor') {
            break
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $windowDeadline)
    if ($script:WindowHandle -eq [IntPtr]::Zero -or
        [MdliteRustAcceptanceNative]::WindowClass($script:WindowHandle) -ne 'MDLiteRustEditor') {
        throw 'The launched process did not expose its expected native editor HWND before timeout.'
    }
    Assert-OwnedWindow $script:WindowHandle
    $script:Report.hwnd = '0x{0:X}' -f $script:WindowHandle.ToInt64()
    $script:Report.visible = [MdliteRustAcceptanceNative]::IsWindowVisible($script:WindowHandle)
    $script:Report.dpi = [int][MdliteRustAcceptanceNative]::GetDpiForWindow($script:WindowHandle)
    if (-not $script:Report.visible -or $script:Report.dpi -le 0) {
        throw 'The exact owned native window was not visible or did not report a valid DPI.'
    }
    Set-OwnedForeground -WaitMs 60000

    $initial = Wait-Diagnostic {
        param($state)
        [int]$state.tabs -eq 2 -and
        ($state.source -ceq '' -or $state.source -ceq $structuredSource)
    } 'startup with both owned synthetic fixtures'
    $script:Report.tabs = [int]$initial.tabs
    $script:Report.active_input_profile = $initial.active_input_profile
    $script:Report.ime_open_status = $initial.ime.open_status
    Add-Observation 'visible Release startup' $initial
    if ([bool]$initial.raw) { throw 'The synthetic fixtures did not open in Live mode.' }
    if ([bool]$initial.ime.active -or -not [string]::IsNullOrEmpty([string]$initial.ime.preedit)) {
        throw 'The owned editor began with active composition; no keyboard input was sent.'
    }

    $state = Switch-ToSource $structuredSource
    if ([int]$state.tables -ne 1) { throw 'The structured fixture did not expose its single Markdown table.' }
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x24 ([uint16[]]@(0x11))
    $tableStart = $structuredSource.IndexOf('left', [StringComparison]::Ordinal)
    $tableEnd = $tableStart + 4
    $state = Wait-Diagnostic { param($value) $true } 'fresh table-caret diagnostic'
    for ($step = 0; $step -lt 8 -and
        -not ([int]$state.selection.anchor -eq [int]$state.selection.head -and
              [int]$state.selection.head -ge $tableStart -and
              [int]$state.selection.head -le $tableEnd); $step++) {
        $oldHead = [int]$state.selection.head
        Set-OwnedForeground
        Send-Key $script:WindowHandle 0x27
        $state = Wait-Diagnostic {
            param($value)
            $value.source -ceq $structuredSource -and [int]$value.selection.head -ne $oldHead
        } 'caret movement into the first table cell'
    }
    if (-not ([int]$state.selection.anchor -eq [int]$state.selection.head -and
              [int]$state.selection.head -ge $tableStart -and
              [int]$state.selection.head -le $tableEnd)) {
        throw 'Keyboard navigation did not reach the first table cell within the bounded steps.'
    }
    $rightStart = $structuredSource.IndexOf('right', [StringComparison]::Ordinal)
    $rightEnd = $rightStart + 'right'.Length
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x09
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $structuredSource -and
        [int]$value.selection.anchor -eq $rightStart -and
        [int]$value.selection.head -eq $rightEnd
    } 'Tab navigation to the next table cell'
    if ([int]$state.tables -ne 1) { throw 'Tab navigation changed the table count.' }
    Add-Observation 'Tab selects the whole destination cell' $state

    $editedTableSource = $structuredSource.Remove($rightStart, $rightEnd - $rightStart).Insert($rightStart, 'X')
    Send-Ascii 'X'
    $editedEnd = $rightStart + 1
    $state = Wait-Document $editedTableSource $editedEnd $editedEnd $true
    if ([int]$state.tables -ne 1) { throw 'Typing in a table cell changed the table count.' }
    Add-Observation 'replace selected table cell through normal input' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document $structuredSource $rightStart $rightEnd $false
    if ([int]$state.tables -ne 1) { throw 'Undo did not restore the original table structure.' }
    Add-Observation 'table cell Undo restores source and selected range' $state
    Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
    $state = Wait-Document $editedTableSource $editedEnd $editedEnd $true
    Add-Observation 'table cell Redo restores edit' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document $structuredSource $rightStart $rightEnd $false

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($structuredSource)
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $structuredSource -and
        [int]$value.selection.anchor -eq $sourceEnd -and
        [int]$value.selection.head -eq $sourceEnd
    } 'Ctrl+End at the final blank paragraph'
    Add-Observation 'Ctrl+End reaches the EOF blank paragraph' $state

    Send-Key $script:WindowHandle 0x0D
    $expectedStructured = $structuredSource + $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    Add-Observation 'Enter appends a single EOF line break' $state
    Send-Key $script:WindowHandle 0x0D
    $expectedStructured += $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    Add-Observation 'repeated Enter preserves the next blank line' $state
    Send-Ascii '## asdkl'
    $expectedStructured += '## asdkl'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedStructured)
    $state = Wait-Document $expectedStructured $sourceEnd $sourceEnd $true
    if ([bool]$state.raw) { throw 'Markdown heading input did not remain in Live mode.' }
    if ($state.source.IndexOf('## asdkl', [StringComparison]::Ordinal) -lt 0 -or
        -not $state.source.EndsWith('## asdkl', [StringComparison]::Ordinal)) {
        throw 'The same Markdown spelling was not preserved inside code and at the document tail.'
    }
    Add-Observation 'write ## heading outside fenced literal at EOF' $state
    Save-ActiveFixture $expectedStructured $structuredPath
    $script:Report.fixtures[1].saved_source_sha256 = (Get-FileHash -LiteralPath $structuredPath -Algorithm SHA256).Hash.ToLowerInvariant()

    $state = Switch-ToSource ''
    if ([bool]$state.dirty -or [int]$state.selection.anchor -ne 0 -or [int]$state.selection.head -ne 0) {
        throw 'The empty synthetic fixture did not start clean at source offset zero.'
    }
    Add-Observation 'empty document starts at source offset zero' $state
    Send-Ascii 'alpha beta'
    $state = Wait-Document 'alpha beta' 10 10 $true
    if ([int]$state.undo_count -le 0) { throw 'Normal text input did not create document history.' }
    Add-Observation 'normal text input in empty document' $state

    Set-OwnedForeground
    $expectedHead = 10
    foreach ($move in 1..2) {
        Send-Key $script:WindowHandle 0x25
        $expectedHead--
        $state = Wait-Document 'alpha beta' $expectedHead $expectedHead $true
        Add-Observation ('repeated Left key ' + $move) $state
    }
    Send-Key $script:WindowHandle 0x27 ([uint16[]]@(0x10))
    $state = Wait-Document 'alpha beta' 8 9 $true
    Add-Observation 'Shift+Right extends selection' $state
    Send-Key $script:WindowHandle 0x27 ([uint16[]]@(0x10))
    $state = Wait-Document 'alpha beta' 8 10 $true
    Add-Observation 'repeated Shift+Right extends selection' $state
    Send-Key $script:WindowHandle 0x25
    $state = Wait-Document 'alpha beta' 8 8 $true
    Add-Observation 'Left collapses the selection to its start' $state
    Send-Key $script:WindowHandle 0x23 ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 10 $true
    $script:Report.discrete_repeat_status = 'PASS: discrete repeated Left and Shift+Right key events ran through the visible window; physical held-key typematic remains NOT_RUN.'

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x25 ([uint16[]]@(0x11, 0x10))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq 'alpha beta' -and
        [int]$value.selection.anchor -eq 10 -and
        [int]$value.selection.head -lt 10
    } 'Ctrl+Shift+Left reverse word selection'
    $selected = $state.source.Substring([int]$state.selection.head, [int]($state.selection.anchor - $state.selection.head))
    if ($selected -cne 'beta') { throw "Ctrl+Shift+Left selected '$selected' instead of the expected final word." }
    Add-Observation 'Ctrl+Shift+Left with virtual-key modifiers' $state

    Send-Ascii 'gamma'
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'replace selected word' $state
    $beforeUndoCount = [int]$state.undo_count
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 6 $true
    if ([int]$state.undo_count -ge $beforeUndoCount -or [int]$state.redo_count -le 0) {
        throw 'Ctrl+Z did not restore source, reverse selection, and redo history.'
    }
    Add-Observation 'Ctrl+Z restores source and reverse selection' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11, 0x10))
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'Ctrl+Shift+Z restores replacement' $state
    Send-Key $script:WindowHandle 0x5A ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha beta' 10 6 $true
    Send-Key $script:WindowHandle 0x59 ([uint16[]]@(0x11))
    $state = Wait-Document 'alpha gamma' 11 11 $true
    Add-Observation 'Ctrl+Y restores replacement' $state

    Send-UnicodeText 'かな'
    $state = Wait-Document 'alpha gammaかな' 17 17 $true
    Add-Observation 'Japanese Unicode packet text (separate from IME composition)' $state
    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x0D
    $expectedBlank = 'alpha gammaかな' + $newline
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'Enter in empty-document scenario' $state
    Send-Ascii '## heading'
    $expectedBlank += '## heading'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'write Markdown after Enter in Live mode' $state

    Set-OwnedForeground
    Send-Key $script:WindowHandle 0x45 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and [bool]$value.raw
    } 'Ctrl+E Source mode switch'
    Add-Observation 'Ctrl+E enters Source mode' $state
    Send-Ascii '!'
    $expectedBlank += '!'
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and
        [int]$value.selection.anchor -eq $sourceEnd -and
        [int]$value.selection.head -eq $sourceEnd -and
        [bool]$value.dirty -and [bool]$value.raw
    } 'Source mode input'
    Add-Observation 'write in Source mode' $state
    Send-Key $script:WindowHandle 0x45 ([uint16[]]@(0x11))
    $state = Wait-Diagnostic {
        param($value)
        $value.source -ceq $expectedBlank -and -not [bool]$value.raw
    } 'Ctrl+E Live mode switch'
    if ([bool]$state.ime.active -or -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'The synthetic text path left composition active.'
    }
    Add-Observation 'Ctrl+E returns to Live mode' $state

    $state = Invoke-ConfiguredImeAcceptance $expectedBlank $sourceEnd
    if ([string]$script:Report.ime_status -eq 'PENDING') {
        throw 'The configured IME acceptance path returned without PASS or a concrete BLOCKED reason.'
    }
    $imeOriginalSource = $expectedBlank
    $state = Wait-Diagnostic { param($value) $true } 'fresh diagnostic after IME acceptance'
    $imePassed = ([string]$script:Report.ime_status).StartsWith('PASS:', [StringComparison]::Ordinal)
    if ($imePassed) {
        if (-not $state.source.StartsWith($imeOriginalSource, [StringComparison]::Ordinal) -or
            $state.source.Length -le $imeOriginalSource.Length) {
            throw 'The configured IME PASS result did not leave a committed suffix in the active source.'
        }
    } elseif ($state.source -cne $imeOriginalSource) {
        throw 'The IME BLOCKED path changed the source unexpectedly.'
    }
    $expectedBlank = [string]$state.source
    $sourceEnd = [Text.Encoding]::UTF8.GetByteCount($expectedBlank)
    if ([int]$state.selection.anchor -ne $sourceEnd -or
        [int]$state.selection.head -ne $sourceEnd -or
        -not [bool]$state.dirty -or [bool]$state.ime.active -or
        -not [string]::IsNullOrEmpty([string]$state.ime.preedit)) {
        throw 'Post-IME source, caret, dirty, or composition state is not ready for the close/save check.'
    }
    Add-Observation 'post-IME source and caret baseline for close/save' $state

    Assert-OwnedWindow $script:WindowHandle
    $closeResult = [IntPtr]::Zero
    [void][MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
        [uint32]$SMTO_ABORTIFHUNG, [uint32]800, [ref]$closeResult)
    $dialog = Wait-OwnedConfirmation
    if ($dialog -eq [IntPtr]::Zero) {
        throw 'Dirty WM_CLOSE did not present the owned MDLite save confirmation.'
    }
    $script:PromptOpen = $true
    $script:Report.unsaved_close_prompt = @{
        hwnd = '0x{0:X}' -f $dialog.ToInt64()
        pid = [int]$script:ProcessId
        class = [MdliteRustAcceptanceNative]::WindowClass($dialog)
        title = [MdliteRustAcceptanceNative]::WindowTitle($dialog)
        response = 'Escape cancels close and preserves the dirty synthetic document.'
    }
    Send-Key $dialog 0x1B
    $script:PromptOpen = $false
    Set-OwnedForeground
    $state = Wait-Document $expectedBlank $sourceEnd $sourceEnd $true
    Add-Observation 'cancel dirty close preserves unsaved source' $state

    Save-ActiveFixture $expectedBlank $blankPath
    $script:Report.fixtures[0].saved_source_sha256 = (Get-FileHash -LiteralPath $blankPath -Algorithm SHA256).Hash.ToLowerInvariant()

    Assert-OwnedWindow $script:WindowHandle
    $closeResult = [IntPtr]::Zero
    $closedMessage = [MdliteRustAcceptanceNative]::SendMessageTimeoutW(
        $script:WindowHandle, [uint32]$WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero,
        [uint32]$SMTO_ABORTIFHUNG, [uint32]1000, [ref]$closeResult)
    if (-not $closedMessage -or -not $script:Process.WaitForExit(2500)) {
        throw 'The clean owned Release window did not close normally before timeout.'
    }
    $script:Report.normal_close = 'PASS after dirty-close cancellation and saving both synthetic fixtures.'

    $script:Report.executable_sha256_after = (Get-FileHash -LiteralPath $ExecutablePath -Algorithm SHA256).Hash.ToLowerInvariant()
    $script:Report.source_tree_sha256_after = Get-SourceTreeSha256
    if ($script:Report.executable_sha256_after -cne $script:Report.executable_sha256_before) {
        throw 'The executable changed while this acceptance run was in progress.'
    }
    if ($script:Report.source_tree_sha256_after -cne $script:Report.source_tree_sha256_before) {
        throw 'Rust source or build manifests changed while this acceptance run was in progress.'
    }
    $script:Report.status = 'PASS'
} catch {
    $script:Report.status = 'FAIL'
    $script:Report.failure = $_.Exception.Message
} finally {
    if ($script:PromptOpen -and $script:WindowHandle -ne [IntPtr]::Zero) {
        try {
            $dialog = Wait-OwnedConfirmation 500
            if ($dialog -ne [IntPtr]::Zero) {
                Send-Key $dialog 0x1B
                $script:PromptOpen = $false
            }
        } catch {
            $script:Report.cleanup_note = 'An owned confirmation could not be dismissed safely; the process was left running.'
        }
    }
    if ($null -ne $script:Process) {
        try {
            $script:Process.Refresh()
            $script:Report.process_still_running = -not $script:Process.HasExited
            if ($script:Report.status -ne 'PASS' -and -not $script:Process.HasExited) {
                $script:Report.cleanup_note = 'Failed runs leave the exact owned process open so unsaved synthetic text remains available for inspection.'
            }
        } catch {
            $script:Report.process_still_running = $null
        }
        $script:Process.Dispose()
    } else {
        $script:Report.process_still_running = $false
    }
    if (-not [string]::IsNullOrWhiteSpace($script:ReportPath)) {
        $script:Report.ended_utc = [DateTime]::UtcNow.ToString('o')
        $script:Report.result_path = $script:ReportPath
        $json = $script:Report | ConvertTo-Json -Depth 12
        [IO.File]::WriteAllText($script:ReportPath, $json, [Text.UTF8Encoding]::new($false))
    }
}

if ($script:Report.status -ne 'PASS') {
    throw "Rust GUI acceptance failed: $($script:Report.failure) Evidence: $($script:ReportPath)"
}
Write-Output "PASS: Release native window acceptance. Evidence: $($script:ReportPath)"
