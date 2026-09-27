#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug','release')]
    [string]$Preset = 'release',
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }
if (-not $OutputPath) { $OutputPath = Join-Path $repoRoot "build\verification\table-edit-native-$Preset.json" }

$previousCrashUiSetting = $env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class MDLiteTableNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", EntryPoint = "SendMessageW")]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessageText(IntPtr window, uint message, IntPtr wparam, string lparam);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessageGetText(IntPtr window, uint message, IntPtr wparam, StringBuilder lparam);
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr SetFocus(IntPtr window);
    [DllImport("user32.dll")]
    public static extern IntPtr GetFocus();
    [DllImport("user32.dll")]
    public static extern bool AttachThreadInput(uint attachThreadId, uint attachToThreadId, bool attach);
    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();
    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentProcessId();
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern IntPtr OpenProcess(uint access, bool inheritHandle, uint processId);
    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr handle);
    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool OpenProcessToken(IntPtr process, uint access, out IntPtr token);
    [DllImport("advapi32.dll", SetLastError = true)]
    public static extern bool GetTokenInformation(IntPtr token, int informationClass,
        IntPtr information, int informationLength, out int returnLength);
    [DllImport("advapi32.dll")]
    public static extern IntPtr GetSidSubAuthorityCount(IntPtr sid);
    [DllImport("advapi32.dll")]
    public static extern IntPtr GetSidSubAuthority(IntPtr sid, uint subAuthority);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern IntPtr OpenInputDesktop(uint flags, bool inherit, uint access);
    [DllImport("user32.dll")]
    public static extern IntPtr GetThreadDesktop(uint threadId);
    [DllImport("user32.dll", EntryPoint = "GetUserObjectInformationW", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern bool GetUserObjectInformation(IntPtr handle, int index,
        StringBuilder information, int lengthBytes, out int needed);
    [DllImport("user32.dll")]
    public static extern bool CloseDesktop(IntPtr desktop);
    public static string GetDesktopName(IntPtr desktop) {
        if (desktop == IntPtr.Zero) return "UNAVAILABLE";
        var name = new StringBuilder(256);
        int needed;
        return GetUserObjectInformation(desktop, 2, name, name.Capacity * 2, out needed)
            ? name.ToString() : "UNAVAILABLE";
    }
    public static string GetIntegrityLevel(uint processId) {
        const uint processQueryLimitedInformation = 0x1000;
        const uint tokenQuery = 0x0008;
        const int tokenIntegrityLevel = 25;
        IntPtr process = OpenProcess(processQueryLimitedInformation, false, processId);
        if (process == IntPtr.Zero) return "UNAVAILABLE";
        IntPtr token = IntPtr.Zero;
        IntPtr buffer = IntPtr.Zero;
        try {
            if (!OpenProcessToken(process, tokenQuery, out token)) return "UNAVAILABLE";
            int required;
            GetTokenInformation(token, tokenIntegrityLevel, IntPtr.Zero, 0, out required);
            if (required <= 0) return "UNAVAILABLE";
            buffer = Marshal.AllocHGlobal(required);
            if (!GetTokenInformation(token, tokenIntegrityLevel, buffer, required, out required))
                return "UNAVAILABLE";
            IntPtr sid = Marshal.ReadIntPtr(buffer);
            IntPtr countPointer = GetSidSubAuthorityCount(sid);
            if (countPointer == IntPtr.Zero) return "UNAVAILABLE";
            byte count = Marshal.ReadByte(countPointer);
            if (count == 0) return "UNAVAILABLE";
            IntPtr ridPointer = GetSidSubAuthority(sid, (uint)(count - 1));
            if (ridPointer == IntPtr.Zero) return "UNAVAILABLE";
            int rid = Marshal.ReadInt32(ridPointer);
            if (rid >= 0x4000) return "SYSTEM";
            if (rid >= 0x3000) return "HIGH";
            if (rid >= 0x2000) return "MEDIUM";
            if (rid >= 0x1000) return "LOW";
            return "UNTRUSTED";
        } finally {
            if (buffer != IntPtr.Zero) Marshal.FreeHGlobal(buffer);
            if (token != IntPtr.Zero) CloseHandle(token);
            CloseHandle(process);
        }
    }
    [DllImport("user32.dll")]
    public static extern short GetKeyState(int key);
    [StructLayout(LayoutKind.Sequential)]
    public struct KEYBDINPUT {
        public ushort wVk;
        public ushort wScan;
        public uint dwFlags;
        public uint time;
        public IntPtr dwExtraInfo;
    }
    // INPUT's union is sized for the largest MOUSEINPUT member even when the
    // current event is keyboard input; on x64 this keeps INPUT at 40 bytes.
    [StructLayout(LayoutKind.Explicit, Size = 32)]
    public struct INPUT_UNION {
        [FieldOffset(0)] public KEYBDINPUT ki;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct INPUT {
        public uint type;
        public INPUT_UNION U;
    }
    [DllImport("user32.dll", SetLastError = true)]
    public static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")]
    public static extern bool InvalidateRect(IntPtr window, IntPtr rectangle, bool erase);
    [DllImport("user32.dll")]
    public static extern bool UpdateWindow(IntPtr window);
    [DllImport("user32.dll")]
    public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll")]
    public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter,
        int x, int y, int width, int height, uint flags);
}
'@

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$WM_CHAR = 0x0102
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_SETFOCUS = 0x0007
$WM_UNDO = 0x0304
$EM_SETSEL = 0x00B1
$EM_GETSEL = 0x00B0
$EM_POSFROMCHAR = 0x00D6
$EM_REDO = 0x0454
$VK_TAB = 0x09
$VK_SHIFT = 0x10
$VK_CONTROL = 0x11
$VK_Z = 0x5A
$VK_LEFT = 0x25
$VK_RIGHT = 0x27
$VK_UP = 0x26
$VK_DOWN = 0x28
$KEYEVENTF_KEYUP = 0x0002
$KEYEVENTF_UNICODE = 0x0004
$kTestSetSelectionBySourceMessage = 0x8032
$kTestGetSourceAnchorMessage = 0x8033
$kTestGetSourceActiveMessage = 0x8034
$kTestGetSourcePositionMessage = 0x8035
$kTestGetEditorReadinessMessage = 0x8036
$kTestGetSourceAtPointMessage = 0x8037
$kTestGetNativeAtPointMessage = 0x8038
$kTestGetLastHistoryBeforeMessage = 0x8039
$kEditor = 102
$kFileSave = 1005
$kTableRowBefore = 1057
$kTableRowAfter = 1058
$kTableRowDelete = 1059
$kTableColumnBefore = 1060
$kTableColumnAfter = 1061
$kTableColumnDelete = 1062

function Wait-ProcessWindow([Diagnostics.Process]$Process, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Process.Refresh()
        if ($Process.MainWindowHandle -ne [IntPtr]::Zero) { return $Process.MainWindowHandle }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main window was not created for process $($Process.Id)"
}

function Find-Control([IntPtr]$Parent, [int]$Id, [string]$ClassName = '') {
    $script:foundControl = [IntPtr]::Zero
    $callback = [MDLiteTableNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLiteTableNative]::GetDlgCtrlID($window) -ne $Id) { return $true }
        if ($ClassName) {
            $name = New-Object Text.StringBuilder 128
            [void][MDLiteTableNative]::GetClassName($window, $name, $name.Capacity)
            if ($name.ToString() -ne $ClassName) { return $true }
        }
        $script:foundControl = $window
        return $false
    }
    [void][MDLiteTableNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return $script:foundControl
}

function Wait-Control([IntPtr]$Parent, [int]$Id, [string]$ClassName = '', [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $control = Find-Control $Parent $Id $ClassName
        if ($control -ne [IntPtr]::Zero) { return $control }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Control was not created: id=$Id class=$ClassName"
}

function Get-WindowText([IntPtr]$Window) {
    $length = [MDLiteTableNative]::SendMessage($Window, $WM_GETTEXTLENGTH, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $text = New-Object Text.StringBuilder ($length + 1)
    [void][MDLiteTableNative]::SendMessageGetText($Window, $WM_GETTEXT, [IntPtr]($length + 1), $text)
    return $text.ToString()
}

function Get-MainWindow([IntPtr]$Window) {
    $main = $Window
    while (($parent = [MDLiteTableNative]::GetParent($main)) -ne [IntPtr]::Zero) {
        $main = $parent
    }
    return $main
}

function Get-Selection([IntPtr]$Editor) {
    $main = Get-MainWindow $Editor
    return [pscustomobject]@{
        start = [MDLiteTableNative]::SendMessage($main, $kTestGetSourceAnchorMessage,
            [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        end = [MDLiteTableNative]::SendMessage($main, $kTestGetSourceActiveMessage,
            [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    }
}

function Set-Selection([IntPtr]$Editor, [int]$Start, [int]$End = $Start) {
    $main = Get-MainWindow $Editor
    $result = [MDLiteTableNative]::SendMessage($main, $kTestSetSelectionBySourceMessage,
        [IntPtr]$Start, [IntPtr]$End)
    if ($result.ToInt64() -eq 0) { throw "Could not set source selection: $Start..$End" }
}

function Get-PositionForSource([IntPtr]$Editor, [int]$Index) {
    $main = Get-MainWindow $Editor
    $packed = [MDLiteTableNative]::SendMessage(
        $main, $kTestGetSourcePositionMessage, [IntPtr]$Index, [IntPtr]::Zero).ToInt64()
    if ($packed -eq -1) { throw "Could not get native position for source offset $Index" }
    $packed = $packed -band 0xffffffffL
    return [pscustomobject]@{
        x = [int][short]($packed -band 0xffff)
        y = [int][short](($packed -shr 16) -band 0xffff)
    }
}

function Get-EditorReadiness([IntPtr]$Editor) {
    $main = Get-MainWindow $Editor
    return [MDLiteTableNative]::SendMessage(
        $main, $kTestGetEditorReadinessMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
}

function Get-SourceAtPoint([IntPtr]$Editor, [int]$X, [int]$Y) {
    $main = Get-MainWindow $Editor
    $packed = ([long]($Y -band 0xffff) -shl 16) -bor ($X -band 0xffff)
    return [MDLiteTableNative]::SendMessage(
        $main, $kTestGetSourceAtPointMessage, [IntPtr]::Zero, [IntPtr]$packed).ToInt32()
}

function Get-NativeAtPoint([IntPtr]$Editor, [int]$X, [int]$Y) {
    $main = Get-MainWindow $Editor
    $packed = ([long]($Y -band 0xffff) -shl 16) -bor ($X -band 0xffff)
    return [MDLiteTableNative]::SendMessage(
        $main, $kTestGetNativeAtPointMessage, [IntPtr]::Zero, [IntPtr]$packed).ToInt32()
}

function Wait-EditorProjection([IntPtr]$Editor, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $state = Get-EditorReadiness $Editor
        if (($state -band 7) -eq 7) { return $state }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Editor projection did not become ready (state=$(Get-EditorReadiness $Editor))."
}

function Send-Characters([IntPtr]$Editor, [string]$Text) {
    [void][MDLiteTableNative]::SendMessage($Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    foreach ($character in $Text.ToCharArray()) {
        [void][MDLiteTableNative]::SendMessage($Editor, $WM_CHAR, [IntPtr][int]$character, [IntPtr]::Zero)
    }
}

function Save-Source([IntPtr]$Main, [string]$Path) {
    [void][MDLiteTableNative]::SendMessage($Main, $WM_COMMAND, [IntPtr]$kFileSave, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    return [IO.File]::ReadAllText($Path)
}

function Invoke-Command([IntPtr]$Main, [int]$Command) {
    [void][MDLiteTableNative]::SendMessage($Main, $WM_COMMAND, [IntPtr]$Command, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
}

function Send-Key([IntPtr]$Editor, [int]$Key) {
    [void][MDLiteTableNative]::SendMessage($Editor, $WM_KEYDOWN, [IntPtr]$Key, [IntPtr]::Zero)
    [void][MDLiteTableNative]::SendMessage($Editor, $WM_KEYUP, [IntPtr]$Key, [IntPtr]::Zero)
}

function Get-ResourceSnapshot([Diagnostics.Process]$Process) {
    try {
        $Process.Refresh()
        return [pscustomobject]@{
            status = 'PASS'
            gdi = [int][MDLiteTableNative]::GetGuiResources($Process.Handle, 0)
            user = [int][MDLiteTableNative]::GetGuiResources($Process.Handle, 1)
        }
    }
    catch {
        return [pscustomobject]@{ status = 'BLOCKED'; reason = $_.Exception.Message }
    }
}

function Invoke-PaintReentryProbe([IntPtr]$Editor, [Diagnostics.Process]$Process, [int]$Iterations = 96) {
    try {
        $before = Get-ResourceSnapshot $Process
        if ($before.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = $before.reason }
        }
        $samples = [Collections.Generic.List[object]]::new()
        for ($index = 0; $index -lt $Iterations; $index++) {
            [void][MDLiteTableNative]::InvalidateRect($Editor, [IntPtr]::Zero, $false)
            [void][MDLiteTableNative]::UpdateWindow($Editor)
            if (($index + 1) % 16 -eq 0) {
                $Process.Refresh()
                if ($Process.HasExited) { break }
                $samples.Add((Get-ResourceSnapshot $Process))
            }
        }
        $after = Get-ResourceSnapshot $Process
        $validSamples = @($samples | Where-Object { $_.status -eq 'PASS' })
        if ($after.status -ne 'PASS' -or $validSamples.Count -eq 0) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; before = $before; after = $after; samples = @($samples) }
        }
        $maxGdi = (@($validSamples | ForEach-Object { $_.gdi }) | Measure-Object -Maximum).Maximum
        $maxUser = (@($validSamples | ForEach-Object { $_.user }) | Measure-Object -Maximum).Maximum
        $pass = -not $Process.HasExited -and $after.gdi -le ($before.gdi + 24) -and
            $after.user -le ($before.user + 8) -and $maxGdi -le ($before.gdi + 24) -and
            $maxUser -le ($before.user + 8)
        return [pscustomobject]@{
            pass = $pass
            status = if ($pass) { 'PASS_OS_NATIVE_PAINT' } else { 'FAIL_RESOURCE_GROWTH_OR_EXIT' }
            iterations = $Iterations
            before = $before
            after = $after
            samples = @($samples)
        }
    }
    catch {
        return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = $_.Exception.Message }
    }
}

function Enter-InputTarget([IntPtr]$Editor) {
    $topLevel = $Editor
    while (($parent = [MDLiteTableNative]::GetParent($topLevel)) -ne [IntPtr]::Zero) { $topLevel = $parent }
    [uint32]$targetProcessId = 0
    [uint32]$targetThread = [MDLiteTableNative]::GetWindowThreadProcessId($topLevel, [ref]$targetProcessId)
    $currentThread = [MDLiteTableNative]::GetCurrentThreadId()
    $currentProcessId = [MDLiteTableNative]::GetCurrentProcessId()
    $foregroundBefore = [MDLiteTableNative]::GetForegroundWindow()
    [uint32]$foregroundBeforePid = 0
    [uint32]$foregroundBeforeThread = [MDLiteTableNative]::GetWindowThreadProcessId($foregroundBefore, [ref]$foregroundBeforePid)
    $attached = $false
    if ($targetThread -ne 0 -and $targetThread -ne $currentThread) {
        $attached = [MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $true)
    }
    [void][MDLiteTableNative]::BringWindowToTop($topLevel)
    $setForeground = [MDLiteTableNative]::SetForegroundWindow($topLevel)
    $foregroundAfter = [MDLiteTableNative]::GetForegroundWindow()
    [uint32]$foregroundAfterPid = 0
    [uint32]$foregroundAfterThread = [MDLiteTableNative]::GetWindowThreadProcessId($foregroundAfter, [ref]$foregroundAfterPid)
    $foregroundTarget = $foregroundAfter -eq $topLevel
    if ($foregroundTarget) { [void][MDLiteTableNative]::SetFocus($Editor) }
    $focusAfter = [MDLiteTableNative]::GetFocus()
    $canSend = $foregroundTarget -and $focusAfter -eq $Editor

    $targetProcess = Get-Process -Id $targetProcessId -ErrorAction SilentlyContinue
    $foregroundProcess = if ($foregroundAfterPid -ne 0) { Get-Process -Id $foregroundAfterPid -ErrorAction SilentlyContinue } else { $null }
    $inputDesktop = [MDLiteTableNative]::OpenInputDesktop(0, $false, 0x0001)
    $inputDesktopName = [MDLiteTableNative]::GetDesktopName($inputDesktop)
    $targetDesktopName = [MDLiteTableNative]::GetDesktopName(
        [MDLiteTableNative]::GetThreadDesktop($targetThread))
    $currentDesktopName = [MDLiteTableNative]::GetDesktopName(
        [MDLiteTableNative]::GetThreadDesktop($currentThread))
    if ($inputDesktop -ne [IntPtr]::Zero) { [void][MDLiteTableNative]::CloseDesktop($inputDesktop) }

    $context = [pscustomobject]@{
        can_send = $canSend
        attached = $attached
        target_window = ('0x{0:X}' -f $topLevel.ToInt64())
        target_pid = $targetProcessId
        target_thread = $targetThread
        target_session = if ($targetProcess) { $targetProcess.SessionId } else { $null }
        target_path = if ($targetProcess) { $targetProcess.Path } else { $null }
        target_integrity = [MDLiteTableNative]::GetIntegrityLevel($targetProcessId)
        current_pid = $currentProcessId
        current_thread = $currentThread
        current_session = [Diagnostics.Process]::GetCurrentProcess().SessionId
        current_integrity = [MDLiteTableNative]::GetIntegrityLevel($currentProcessId)
        foreground_before_pid = $foregroundBeforePid
        foreground_before_thread = $foregroundBeforeThread
        foreground_after_pid = $foregroundAfterPid
        foreground_after_thread = $foregroundAfterThread
        foreground_after_window = ('0x{0:X}' -f $foregroundAfter.ToInt64())
        focus_after_window = ('0x{0:X}' -f $focusAfter.ToInt64())
        set_foreground_returned_true = $setForeground
        input_desktop = $inputDesktopName
        target_thread_desktop = $targetDesktopName
        current_thread_desktop = $currentDesktopName
    }
    if (-not $canSend -and $attached) {
        [void][MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $false)
        $context.attached = $false
    }
    return $context
}
function Exit-InputTarget($Target) {
    if ($Target -and $Target.attached) {
        [void][MDLiteTableNative]::AttachThreadInput($Target.current_thread, $Target.target_thread, $false)
    }
}

function Send-UnicodeCharacters([IntPtr]$Editor, [string]$Text) {
    $target = Enter-InputTarget $Editor
    if (-not $target.can_send) {
        return [pscustomobject]@{ sent = 0; expected = $Text.Length * 2; sendinput_called = $false; status = 'BLOCKED_FOREGROUND_NOT_ESTABLISHED'; target_context = $target }
    }
    $inputs = [MDLiteTableNative+INPUT[]]::new($Text.Length * 2)
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $down = $index * 2; $up = $down + 1; $inputs[$down].type = 1; $inputs[$down].U = [MDLiteTableNative+INPUT_UNION]::new(); $inputs[$down].U.ki.wScan = [uint16][int]$Text[$index]; $inputs[$down].U.ki.dwFlags = $KEYEVENTF_UNICODE
        $inputs[$up].type = 1; $inputs[$up].U = [MDLiteTableNative+INPUT_UNION]::new(); $inputs[$up].U.ki.wScan = [uint16][int]$Text[$index]; $inputs[$up].U.ki.dwFlags = $KEYEVENTF_UNICODE -bor $KEYEVENTF_KEYUP
    }
    $sent = [MDLiteTableNative]::SendInput($inputs.Length, $inputs, [Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); $errorCode = if ($sent -eq $inputs.Length) { 0 } else { [Runtime.InteropServices.Marshal]::GetLastWin32Error() }; Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=$inputs.Length; sendinput_called=$true; error=$errorCode; status=if($sent -eq $inputs.Length){'PASS'}else{'BLOCKED_SENDINPUT'}; target_context=$target }
}

function Send-ControlKey([IntPtr]$Editor, [int]$Key) {
    $target = Enter-InputTarget $Editor; if (-not $target.can_send) { return [pscustomobject]@{ sent=0; expected=4; sendinput_called=$false; status='BLOCKED_FOREGROUND_NOT_ESTABLISHED'; target_context=$target } }
    $inputs = [MDLiteTableNative+INPUT[]]::new(4); for ($index=0; $index -lt 4; $index++) { $inputs[$index].type=1; $inputs[$index].U=[MDLiteTableNative+INPUT_UNION]::new() }
    $inputs[0].U.ki.wVk=[uint16]$VK_CONTROL; $inputs[1].U.ki.wVk=[uint16]$Key; $inputs[2].U.ki.wVk=[uint16]$Key; $inputs[2].U.ki.dwFlags=$KEYEVENTF_KEYUP; $inputs[3].U.ki.wVk=[uint16]$VK_CONTROL; $inputs[3].U.ki.dwFlags=$KEYEVENTF_KEYUP
    $sent=[MDLiteTableNative]::SendInput(4,$inputs,[Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); $errorCode=if($sent -eq 4){0}else{[Runtime.InteropServices.Marshal]::GetLastWin32Error()}; Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=4; sendinput_called=$true; error=$errorCode; status=if($sent -eq 4){'PASS'}else{'BLOCKED_SENDINPUT'}; target_context=$target }
}

function Send-KeyRepeat([IntPtr]$Editor, [int]$Key, [int]$Repeat) {
    $target = Enter-InputTarget $Editor; if (-not $target.can_send) { return [pscustomobject]@{ sent=0; expected=$Repeat*2; sendinput_called=$false; status='BLOCKED_FOREGROUND_NOT_ESTABLISHED'; target_context=$target } }
    $inputs=[MDLiteTableNative+INPUT[]]::new($Repeat*2); for($index=0;$index -lt $Repeat;$index++){ $down=$index*2; $up=$down+1; $inputs[$down].type=1; $inputs[$down].U=[MDLiteTableNative+INPUT_UNION]::new(); $inputs[$down].U.ki.wVk=[uint16]$Key; $inputs[$up].type=1; $inputs[$up].U=[MDLiteTableNative+INPUT_UNION]::new(); $inputs[$up].U.ki.wVk=[uint16]$Key; $inputs[$up].U.ki.dwFlags=$KEYEVENTF_KEYUP }
    $sent=[MDLiteTableNative]::SendInput($inputs.Length,$inputs,[Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); $errorCode=if($sent -eq $inputs.Length){0}else{[Runtime.InteropServices.Marshal]::GetLastWin32Error()}; Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=$inputs.Length; sendinput_called=$true; error=$errorCode; status=if($sent -eq $inputs.Length){'PASS'}else{'BLOCKED_SENDINPUT'}; target_context=$target }
}
function Send-ShiftKey([IntPtr]$Editor, [int]$Key) {
    $target = Enter-InputTarget $Editor
    if (-not $target.can_send) {
        return [pscustomobject]@{ sent = 0; expected = 4; sendinput_called = $false; status = 'BLOCKED_FOREGROUND_NOT_ESTABLISHED'; target_context = $target }
    }
    try {
        $inputs = [MDLiteTableNative+INPUT[]]::new(4)
        for ($index = 0; $index -lt $inputs.Length; $index++) {
            $inputs[$index].type = 1
            $inputs[$index].U = [MDLiteTableNative+INPUT_UNION]::new()
        }
        $inputs[0].U.ki.wVk = [uint16]$VK_SHIFT
        $inputs[1].U.ki.wVk = [uint16]$Key
        $inputs[2].U.ki.wVk = [uint16]$Key
        $inputs[2].U.ki.dwFlags = 0x0002
        $inputs[3].U.ki.wVk = [uint16]$VK_SHIFT
        $inputs[3].U.ki.dwFlags = 0x0002
        $sent = [MDLiteTableNative]::SendInput(4, $inputs, [Runtime.InteropServices.Marshal]::SizeOf($inputs[0]))
        $errorCode = if ($sent -eq 4) { 0 } else { [Runtime.InteropServices.Marshal]::GetLastWin32Error() }
        Start-Sleep -Milliseconds 150
        return [pscustomobject]@{ sent = $sent; expected = 4; sendinput_called = $true; error = $errorCode; status = if ($sent -eq 4) { 'PASS' } else { 'BLOCKED_SENDINPUT' }; target_context = $target }
    }
    finally {
        Exit-InputTarget $target
    }
}

$source = "# Table UI probe`n`n| Head A | Head B |`n| :--- | ---: |`n| one | two |`n| three | four |`n"
$nativeView = "# Table UI probe`r`r Head A `t Head B `r one `t two `r three `t four `r"
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$runRoot = Join-Path ([IO.Path]::GetTempPath()) ("mdlite-table-acceptance-" + [guid]::NewGuid().ToString('N'))
$captureDirectory = Join-Path $repoRoot "build\verification\native-table-frames-$Preset-$runId"
$captureRows = [Collections.Generic.List[object]]::new()
$checks = [ordered]@{}
$nativeFindOffsets = [ordered]@{}
$processes = [Collections.Generic.List[Diagnostics.Process]]::new()
$firstNativeEditorText = $null

function Save-NativeTableFrame([IntPtr]$Window, [string]$Name) {
    $path = Join-Path $script:captureDirectory ($Name + '.png')
    [MDLiteTableNative+RECT]$rect = New-Object MDLiteTableNative+RECT
    if (-not [MDLiteTableNative]::GetWindowRect($Window, [ref]$rect)) {
        return [pscustomobject]@{ name=$Name; status='BLOCKED'; reason='GetWindowRect failed.' }
    }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    if ($width -le 0 -or $height -le 0) {
        return [pscustomobject]@{ name=$Name; status='BLOCKED'; reason='Window bounds are empty.' }
    }
    $bitmap = [Drawing.Bitmap]::new($width, $height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    try { $captured = [MDLiteTableNative]::PrintWindow($Window, $hdc, 2) }
    finally {
        $graphics.ReleaseHdc($hdc)
        $graphics.Dispose()
    }
    if (-not $captured) {
        $bitmap.Dispose()
        return [pscustomobject]@{ name=$Name; status='BLOCKED'; reason='PrintWindow failed.' }
    }
    try { $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png) }
    finally { $bitmap.Dispose() }
    return [pscustomobject]@{
        name = $Name
        status = 'CAPTURED_NATIVE_WINDOW'
        path = $path
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant()
        width = $width
        height = $height
        capture_method = 'PrintWindow of the active native MDLite window'
    }
}

function Invoke-TableCase([string]$Name, [scriptblock]$Action) {
    $workspace = Join-Path $runRoot $Name
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`ntheme = `"dark`"`n", [Text.UTF8Encoding]::new($false))
    $path = Join-Path $workspace 'table.md'
    $caseSource = if ($Name -like 'ragged_virtual_cell_*') {
        "# Table UI probe`n`n| Head A | Head B | Head C |`n| :--- | ---: | --- |`n| one | two |`n"
    } else { $source }
    [IO.File]::WriteAllText($path, $caseSource, [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath $executable -ArgumentList @($path) -PassThru
    $processes.Add($process)
    try {
        $main = Wait-ProcessWindow $process
        if (-not [MDLiteTableNative]::SetWindowPos($main, [IntPtr]::Zero, 0, 0,
                1672, 941, 0x0004)) {
            throw 'Could not set the native table fixture to the approved 1672x941 view.'
        }
        $foregroundActivated = [MDLiteTableNative]::SetForegroundWindow($main)
        [void][MDLiteTableNative]::BringWindowToTop($main)
        Start-Sleep -Milliseconds 40
        $editor = Wait-Control $main $kEditor 'RICHEDIT50W'
        $projectionState = Wait-EditorProjection $editor
        $script:one = $caseSource.IndexOf('one', [StringComparison]::Ordinal)
        $script:two = $caseSource.IndexOf('two', [StringComparison]::Ordinal)
        $script:three = $caseSource.IndexOf('three', [StringComparison]::Ordinal)
        $script:four = $caseSource.IndexOf('four', [StringComparison]::Ordinal)
        $script:head = $caseSource.IndexOf('Head A', [StringComparison]::Ordinal)
        $flatEditorText = Get-WindowText $editor
        $nativeFindOffsets[$Name] = [ordered]@{
            one = $script:one
            two = $script:two
            three = $script:three
            four = $script:four
            head = $script:head
            flattened_one = $flatEditorText.IndexOf('one', [StringComparison]::Ordinal)
            flattened_two = $flatEditorText.IndexOf('two', [StringComparison]::Ordinal)
            projection_state = $projectionState
            foreground_activated = $foregroundActivated
        }
        if ($null -eq $script:firstNativeEditorText) {
            $script:firstNativeEditorText = Get-WindowText $editor
        }
        $captureCase = $Name -in @('row_before','cell_edit_undo_redo','tab_next_cell',
                                   'ragged_virtual_cell_edit_undo_redo',
                                   'ragged_virtual_cell_tab_edit_undo_redo','table_paint_reentry')
        if ($captureCase) { $script:captureRows.Add((Save-NativeTableFrame $main ($Name + '-before'))) }
        Set-Selection $editor 0 0
        $value = & $Action $main $editor $path $process
        if ($null -eq $value) { throw "Case $Name returned no result" }
        $checks[$Name] = $value
        if ($captureCase) { $script:captureRows.Add((Save-NativeTableFrame $main ($Name + '-after'))) }
    }
    finally {
        if (-not $process.HasExited) {
            [void][MDLiteTableNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            if (-not $process.WaitForExit(3000)) { $process.Kill(); [void]$process.WaitForExit(3000) }
        }
        $process.Dispose()
        [void]$processes.Remove($process)
    }
}

try {
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    [IO.Directory]::CreateDirectory($captureDirectory) | Out-Null
    $one = $source.IndexOf('one', [StringComparison]::Ordinal)
    $two = $source.IndexOf('two', [StringComparison]::Ordinal)
    $three = $source.IndexOf('three', [StringComparison]::Ordinal)
    $four = $source.IndexOf('four', [StringComparison]::Ordinal)
    $head = $source.IndexOf('Head A', [StringComparison]::Ordinal)

    Invoke-TableCase 'row_before' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableRowBefore
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains("|  |  |`n| one | two |") }
    }
    Invoke-TableCase 'row_after' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableRowAfter
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains("| one | two |`n|  |  |`n| three | four |") }
    }
    Invoke-TableCase 'row_delete' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableRowDelete
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = (-not $saved.Contains('| one | two |')) -and $saved.Contains('| three | four |') }
    }
    Invoke-TableCase 'column_before' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableColumnBefore
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains('|  | Head A | Head B |') -and $saved.Contains('|  | one | two |') }
    }
    Invoke-TableCase 'column_after' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableColumnAfter
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains('| Head A |  | Head B |') -and $saved.Contains('| one |  | two |') }
    }
    Invoke-TableCase 'column_delete' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Invoke-Command $main $kTableColumnDelete
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains('| Head B |') -and (-not $saved.Contains('| one | two |')) }
    }
    Invoke-TableCase 'cell_edit_undo_redo' {
        param($main, $editor, $path)
        Set-Selection $editor $one ($one + 3)
        Send-Characters $editor 'ONE'
        $edited = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undo_one = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undo_two = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undo_three = Save-Source $main $path
        $undo_selection = Get-Selection $editor
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redo_one = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redo_two = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redo_three = Save-Source $main $path
        $redo_selection = Get-Selection $editor
        [pscustomobject]@{
            pass = $edited.Contains('| ONE | two |') -and $undo_one.Contains('| ON | two |') -and
                $undo_two.Contains('| O | two |') -and $undo_three.Contains('| one | two |') -and
                $redo_one.Contains('| O | two |') -and $redo_two.Contains('| ON | two |') -and
                $redo_three.Contains('| ONE | two |') -and
                $undo_selection.start -eq $one -and $undo_selection.end -eq ($one + 3) -and
                $redo_selection.start -eq ($one + 3) -and $redo_selection.end -eq ($one + 3)
            status = 'PASS_WINDOW_MESSAGE_PATH'
            input_method = 'SendMessage WM_CHAR, WM_UNDO, EM_REDO; one transaction per WM_CHAR'
            edited = $edited
            undo_one = $undo_one
            undo_two = $undo_two
            undo_three = $undo_three
            redo_one = $redo_one
            redo_two = $redo_two
            redo_three = $redo_three
            undo_selection = $undo_selection
            redo_selection = $redo_selection
        }
    }
    Invoke-TableCase 'ragged_virtual_cell_edit_undo_redo' {
        param($main, $editor, $path)
        Start-Sleep -Milliseconds 350
        $targetIndex = $caseSource.IndexOf('Head C', [StringComparison]::Ordinal)
        $bodyIndex = $caseSource.IndexOf('one', [StringComparison]::Ordinal)
        $targetPosition = Get-PositionForSource $editor $targetIndex
        $bodyPosition = Get-PositionForSource $editor $bodyIndex
        $pointX = $targetPosition.x + 2
        $pointY = $bodyPosition.y + 2
        $point = [IntPtr]([long](($pointY -band 0xffff) -shl 16) -bor ($pointX -band 0xffff))
        $stateBeforeClick = Get-EditorReadiness $editor
        $sourceHitBeforeClick = Get-SourceAtPoint $editor $pointX $pointY
        $nativeHitBeforeClick = Get-NativeAtPoint $editor $pointX $pointY
        [void][MDLiteTableNative]::SendMessage($editor, 0x0200, [IntPtr]::Zero, $point)
        [void][MDLiteTableNative]::SendMessage($editor, 0x0201, [IntPtr]1, $point)
        $selectionAfterDown = Get-Selection $editor
        $stateAfterDown = Get-EditorReadiness $editor
        [void][MDLiteTableNative]::SendMessage($editor, 0x0202, [IntPtr]::Zero, $point)
        $selectionAfterClick = Get-Selection $editor
        $stateAfterClick = Get-EditorReadiness $editor
        Send-Characters $editor 'Q'
        $edited = Save-Source $main $path
        $historyBefore = [MDLiteTableNative]::SendMessage(
            $main, $kTestGetLastHistoryBeforeMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        $selectionAfterEdit = Get-Selection $editor
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $selectionAfterUndoBeforeSave = Get-Selection $editor
        $undone = Save-Source $main $path
        $selectionAfterUndo = Get-Selection $editor
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redone = Save-Source $main $path
        $selectionAfterRedo = Get-Selection $editor
        $expectedCaret = $edited.IndexOf('Q', [StringComparison]::Ordinal) + 1
        $expectedUndoCaret = $caseSource.IndexOf('two', [StringComparison]::Ordinal) + 4
        [pscustomobject]@{
            pass = $edited.Contains('| one | two | Q |') -and
                $undone -eq $caseSource -and $redone -eq $edited -and
                $selectionAfterEdit.start -eq $expectedCaret -and
                $selectionAfterEdit.end -eq $expectedCaret -and
                $selectionAfterUndo.start -eq $expectedUndoCaret -and
                $selectionAfterUndo.end -eq $selectionAfterUndo.start -and
                $selectionAfterRedo.start -eq $expectedCaret -and
                $selectionAfterRedo.end -eq $expectedCaret
            status = 'PASS_WINDOW_MESSAGE_PATH'
            input_method = 'Synthetic cross-process SendMessage(WM_LBUTTONDOWN/WM_CHAR/WM_UNDO/EM_REDO); observed through source and caret'
            clicked_column = 2
            target_index = $targetIndex
            body_index = $bodyIndex
            editor_view = (Get-WindowText $editor) -replace "`r`n", "`r"
            target_position = $targetPosition
            body_position = $bodyPosition
            state_before_click = $stateBeforeClick
            source_hit_before_click = $sourceHitBeforeClick
            native_hit_before_click = $nativeHitBeforeClick
            selection_after_down = $selectionAfterDown
            selection_after_click = $selectionAfterClick
            state_after_down = $stateAfterDown
            state_after_click = $stateAfterClick
            edited = $edited
            history_before = $historyBefore
            undone = $undone
            redone = $redone
            selection_after_edit = $selectionAfterEdit
            selection_after_undo = $selectionAfterUndo
            selection_after_undo_before_save = $selectionAfterUndoBeforeSave
            selection_after_redo = $selectionAfterRedo
        }
    }
    Invoke-TableCase 'ragged_virtual_cell_tab_edit_undo_redo' {
        param($main, $editor, $path)
        Start-Sleep -Milliseconds 350
        $raggedNativeView = "# Table UI probe`r`r Head A `t Head B `t Head C `r one `t two `r"
        $twoIndex = $caseSource.IndexOf('two', [StringComparison]::Ordinal)
        Set-Selection $editor $twoIndex
        Send-Key $editor $VK_TAB
        $selectionAfterTab = Get-Selection $editor
        $afterTab = Save-Source $main $path
        Send-Characters $editor 'Q'
        $edited = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undone = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redone = Save-Source $main $path
        [pscustomobject]@{
            pass = $selectionAfterTab.start -eq ($twoIndex + 4) -and
                $selectionAfterTab.end -eq $selectionAfterTab.start -and
                $afterTab -eq $caseSource -and $edited.Contains('| one | two | Q |') -and
                $undone -eq $caseSource -and $redone -eq $edited
            status = 'PASS_WINDOW_MESSAGE_PATH'
            input_method = 'Synthetic cross-process SendMessage(WM_KEYDOWN/VK_TAB, WM_CHAR, WM_UNDO, EM_REDO); saved source observed'
            selection_after_tab = $selectionAfterTab
            after_tab = $afterTab
            edited = $edited
            undone = $undone
            redone = $redone
        }
    }
    Invoke-TableCase 'undo_selection_message_path' {
        param($main, $editor, $path)
        Set-Selection $editor $one ($one + 3)
        $selection_before = Get-Selection $editor
        Send-Characters $editor 'X'
        $edited = Save-Source $main $path
        $selection_after_edit = Get-Selection $editor
        [void][MDLiteTableNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undone = Save-Source $main $path
        $selection_after_undo = Get-Selection $editor
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redone = Save-Source $main $path
        $selection_after_redo = Get-Selection $editor
        $expected_caret = $one + 1
        [pscustomobject]@{
            pass = $edited.Contains('| X | two |') -and $undone.Contains('| one | two |') -and
                $redone.Contains('| X | two |') -and
                $selection_after_undo.start -eq $selection_before.start -and
                $selection_after_undo.end -eq ($one + 3) -and
                $selection_after_edit.start -eq $expected_caret -and
                $selection_after_edit.end -eq $expected_caret -and
                $selection_after_redo.start -eq $expected_caret -and
                $selection_after_redo.end -eq $expected_caret
            status = 'PASS_WINDOW_MESSAGE_PATH'
            input_method = 'SendMessage WM_CHAR, WM_UNDO, EM_REDO; not SendInput or physical keyboard'
            selection_before = $selection_before
            selection_after_edit = $selection_after_edit
            selection_after_undo = $selection_after_undo
            selection_after_redo = $selection_after_redo
            edited = $edited
            undone = $undone
            redone = $redone
        }
    }
    Invoke-TableCase 'tab_next_cell' {
        param($main, $editor, $path)
        Set-Selection $editor $one
        Send-Key $editor $VK_TAB
        $selection = Get-Selection $editor
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $selection.start -eq $two -and $selection.end -eq $two -and $saved -eq $source; actual_start = $selection.start; actual_end = $selection.end; expected = $two; saved = $saved }
    }
    Invoke-TableCase 'tab_appends_row' {
        param($main, $editor, $path)
        Set-Selection $editor $four
        Send-Key $editor $VK_TAB
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains('|  |  |') -and $saved -ne $source; actual = $saved }
    }
    Invoke-TableCase 'shift_tab_previous_cell' {
        param($main, $editor, $path)
        Set-Selection $editor $two
        $input = Send-ShiftKey $editor $VK_TAB
        if ($input.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = $input.status; delivery = $input; reason = 'OS Shift+Tab was not delivered to the target RichEdit.' }
        }
        $selection = Get-Selection $editor
        $saved = Save-Source $main $path
        $pass = $selection.start -eq $one -and $selection.end -eq $one -and $saved -eq $source
        if (-not $pass) {
            return [pscustomobject]@{ pass = $false; status = 'FAIL'; reason = 'OS Shift+Tab was delivered, but the selection/source result did not match the previous cell'; actual_start = $selection.start; actual_end = $selection.end; expected = $one; saved = $saved }
        }
        [pscustomobject]@{ pass = $true; actual_start = $selection.start; actual_end = $selection.end; expected = $one; saved = $saved }
    }
    Invoke-TableCase 'arrow_right_boundary' {
        param($main, $editor, $path)
        Set-Selection $editor ($one + 3)
        $before = Get-Selection $editor
        Send-Key $editor $VK_RIGHT
        $selection = Get-Selection $editor
        [pscustomobject]@{ pass = $selection.start -eq $two -and $selection.end -eq $two; before = $before; actual_start = $selection.start; actual_end = $selection.end; expected = $two }
    }
    Invoke-TableCase 'arrow_down_column' {
        param($main, $editor, $path, $process)
        Set-Selection $editor $head
        Send-Key $editor $VK_DOWN
        $selection = Get-Selection $editor
        $saved = Save-Source $main $path
        $cellBegin = $one - 1
        $cellEnd = $one + 4
        [pscustomobject]@{
            pass = $selection.start -ge $cellBegin -and $selection.start -le $cellEnd -and
                $selection.end -eq $selection.start -and $saved -eq $source
            actual_start = $selection.start
            actual_end = $selection.end
            target_cell_begin = $cellBegin
            target_cell_end = $cellEnd
            saved = $saved
        }
    }
    Invoke-TableCase 'continuous_unicode_input_no_accumulation' {
        param($main, $editor, $path, $process)
        $payload = 'x' * 128
        Set-Selection $editor $nativeView.Length
        $input = Send-UnicodeCharacters $editor $payload
        if ($input.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = $input.status; delivery = $input; reason = 'OS Unicode input was not delivered to the target RichEdit.' }
        }
        Start-Sleep -Milliseconds 250
        $saved = Save-Source $main $path
        [pscustomobject]@{
            pass = $saved -eq ($source + $payload)
            status = if ($saved -eq ($source + $payload)) { 'PASS_OS_INPUT_NO_IME' } else { 'BLOCKED_INPUT_DELIVERY_UNPROVEN' }
            expected_length = ($source + $payload).Length
            actual_length = $saved.Length
        }
    }
    Invoke-TableCase 'ctrl_z_single_transaction' {
        param($main, $editor, $path, $process)
        Set-Selection $editor $one
        Invoke-Command $main $kTableRowBefore
        $afterRow = Save-Source $main $path
        Set-Selection $editor $one
        Invoke-Command $main $kTableColumnAfter
        $afterTwo = Save-Source $main $path
        $firstInput = Send-ControlKey $editor $VK_Z
        if ($firstInput.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = $firstInput.status; delivery = $firstInput; reason = 'OS Ctrl+Z was not delivered to the target RichEdit.' }
        }
        $undoOne = Save-Source $main $path
        Start-Sleep -Milliseconds 100
        $secondInput = Send-ControlKey $editor $VK_Z
        if ($secondInput.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = $secondInput.status; delivery = $secondInput; reason = 'Second OS Ctrl+Z was not delivered to the target RichEdit.' }
        }
        $undoTwo = Save-Source $main $path
        [pscustomobject]@{
            pass = $afterTwo -ne $afterRow -and $undoOne -eq $afterRow -and $undoTwo -eq $source
            status = if ($undoOne -eq $afterRow -and $undoTwo -eq $source) { 'PASS_OS_INPUT_ONE_TRANSACTION' } else { 'BLOCKED_INPUT_DELIVERY_UNPROVEN' }
            after_row = $afterRow
            after_two = $afterTwo
            undo_one = $undoOne
            undo_two = $undoTwo
        }
    }
    Invoke-TableCase 'arrow_repeat_boundary' {
        param($main, $editor, $path, $process)
        Set-Selection $editor ($one + 3)
        $input = Send-KeyRepeat $editor $VK_RIGHT 8
        if ($input.status -ne 'PASS') {
            return [pscustomobject]@{ pass = $false; status = $input.status; delivery = $input; reason = 'OS arrow repeat was not delivered to the target RichEdit.' }
        }
        $selection = Get-Selection $editor
        $saved = Save-Source $main $path
        $pass = $saved -eq $source -and $selection.start -ge $two -and $selection.end -eq $selection.start -and $selection.start -lt $three
        [pscustomobject]@{ pass = $pass; status = if ($pass) { 'PASS_OS_INPUT_BOUNDARY' } else { 'BLOCKED_INPUT_DELIVERY_UNPROVEN' }; actual_start = $selection.start; actual_end = $selection.end; first_cell = $two; next_row = $three; saved = $saved }
    }
    Invoke-TableCase 'table_paint_reentry' {
        param($main, $editor, $path, $process)
        Invoke-PaintReentryProbe $editor $process 96
    }

    $blocked = [Collections.Generic.List[object]]::new()
    $failed = [Collections.Generic.List[object]]::new()
    $resultShapeErrors = [Collections.Generic.List[object]]::new()
    foreach ($entry in $checks.GetEnumerator()) {
        $passProperty = $entry.Value.PSObject.Properties['pass']
        $statusProperty = $entry.Value.PSObject.Properties['status']
        if ($null -eq $passProperty) {
            $resultShapeErrors.Add([pscustomobject]@{
                name = $entry.Key
                result_type = if ($null -eq $entry.Value) { 'null' } else { $entry.Value.GetType().FullName }
                has_pass = $false
                has_status = $null -ne $statusProperty
            })
            continue
        }
        if (-not [bool]$passProperty.Value) {
            if ($null -ne $statusProperty -and [string]$statusProperty.Value -match '^BLOCKED') { $blocked.Add($entry) }
            else { $failed.Add($entry) }
        }
    }
    $result = [ordered]@{
        preset = $Preset
        executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
        source = $source
        native_editor_text_before_first_action = $firstNativeEditorText
        native_find_offsets = $nativeFindOffsets
        checks = $checks
        captures = $captureRows.ToArray()
        evidence = [ordered]@{
            semantic_check_groups = [ordered]@{
                table_source_round_trip = @('row_before','row_after','row_delete','column_before','column_after','column_delete','ragged_virtual_cell_edit_undo_redo','ragged_virtual_cell_tab_edit_undo_redo')
                edit_source_observation = @('cell_edit_undo_redo','tab_next_cell','tab_appends_row','ragged_virtual_cell_edit_undo_redo','ragged_virtual_cell_tab_edit_undo_redo')
                undo_redo_and_selection = @('cell_edit_undo_redo','undo_selection_message_path','ctrl_z_single_transaction','ragged_virtual_cell_edit_undo_redo','ragged_virtual_cell_tab_edit_undo_redo')
                cell_and_boundary_navigation = @('tab_next_cell','shift_tab_previous_cell','ragged_virtual_cell_tab_edit_undo_redo','arrow_right_boundary','arrow_down_column')
                synthetic_arrow_repeat = @('arrow_repeat_boundary')
                native_paint_resource_probe = @('table_paint_reentry')
                continuous_unicode_input_probe = @('continuous_unicode_input_no_accumulation')
                table_transaction_rollback = @('undo_selection_message_path','ctrl_z_single_transaction','ragged_virtual_cell_edit_undo_redo','ragged_virtual_cell_tab_edit_undo_redo')
            }
            current_task_acceptance_crosswalk = [ordered]@{
                task_snapshot = '20260925-mdlite-approved-ui-live-editor-rebuild / ACCEPTANCE_TESTS.md Revision 1'
                E07 = [ordered]@{
                    coverage = 'PARTIAL'
                    supporting_checks = @('row_before','row_after','row_delete','column_before','column_after','column_delete','cell_edit_undo_redo','undo_selection_message_path','ctrl_z_single_transaction','ragged_virtual_cell_edit_undo_redo','ragged_virtual_cell_tab_edit_undo_redo')
                    supports_only = @('selected table row/column source edits','selected table-cell edit and undo/redo probes')
                    uncovered_aspects = @('paste and cut','general deletion','forward and reverse selection replacement contracts','template operations','section movement','the complete E07 one-operation history and caret/selection-direction contract')
                    input_route = 'Synthetic cross-process Win32 messages: WM_COMMAND for table commands, WM_CHAR for text, and WM_UNDO/EM_REDO or synthetic Ctrl+Z for history. Per-check input_method/status remain authoritative; this is not physical-input evidence.'
                }
                E09 = [ordered]@{
                    coverage = 'PARTIAL'
                    supporting_checks = @('tab_next_cell','shift_tab_previous_cell','ragged_virtual_cell_tab_edit_undo_redo','arrow_right_boundary','arrow_down_column')
                    supports_only = @('selected cell, Tab/Shift+Tab, and arrow boundary-navigation probes in the harness fixtures')
                    uncovered_aspects = @('all-cell and wrapped-cell coverage','surrounding body transitions','caret-to-hit-test geometry agreement','unabsorbed input across the full E09 matrix')
                    input_route = 'Synthetic cross-process WM_KEYDOWN/WM_KEYUP messages for Tab/arrows; Shift+Tab attempts OS SendInput only after foreground/focus is verified. Its delivery object records whether SendInput was called plus window, session, integrity, desktop and foreground facts. No physical keyboard evidence.'
                }
                E10 = [ordered]@{
                    coverage = 'NOT_COVERED'
                    related_check = 'arrow_repeat_boundary'
                    reason = 'The harness sends a short synthetic repeat sequence and checks a final boundary offset. It does not provide physical 3-5 second arrow holds, timed caret/selection/scroll observation, or the required video/continuous frames.'
                }
                E12 = [ordered]@{
                    coverage = 'NOT_COVERED'
                    related_but_insufficient_checks = @('continuous_unicode_input_no_accumulation','table_paint_reentry')
                    uncovered_aspects = @('IME input','resize','scroll','DPI changes','combined table input and undo/redo under those conditions','continuous visual paint/frame correctness')
                    reason = 'The Unicode input and resource-count paint probes are isolated checks; neither establishes E12 visual/source invariants across its combined input, history, and viewport conditions.'
                }
                other_acceptance_ids = 'No crosswalk is asserted for current-task IDs other than the partial E07/E09 and uncovered E10/E12 entries above.'
                status_rule = 'This crosswalk is traceability only. Individual check pass flags do not make an acceptance ID PASS or imply overall current-task acceptance.'
            }
        }
        evidence_boundary = 'Harness checks use synthetic native Win32/RichEdit routes. Semantic check groups are harness-local, not current-task E01-E25 IDs. The current-task crosswalk is conservative and partial; UNKNOWN and NOT_COVERED conditions remain open.'
        all_cases_pass = $failed.Count -eq 0 -and $blocked.Count -eq 0 -and $resultShapeErrors.Count -eq 0
        automated_product_checks_pass = $failed.Count -eq 0 -and $resultShapeErrors.Count -eq 0
        blocked_cases = @($blocked | ForEach-Object { $_.Key })
        failed_cases = @($failed | ForEach-Object { $_.Key })
        invalid_result_shape_cases = @($resultShapeErrors)
        caveat = 'Automated native Win32/RichEdit path; not Human IME/DPI/subjective acceptance.'
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
    }
    $directory = Split-Path -Parent $OutputPath
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $result | ConvertTo-Json -Depth 8
    if (-not $result.all_cases_pass) { exit 1 }
}
finally {
    foreach ($process in @($processes)) {
        if (-not $process.HasExited) { $process.Kill(); [void]$process.WaitForExit(3000) }
        $process.Dispose()
    }
    if (Test-Path -LiteralPath $runRoot) { Remove-Item -LiteralPath $runRoot -Recurse -Force }
    if ($null -eq $previousCrashUiSetting) {
        Remove-Item Env:MDLITE_TEST_NO_CRASH_UI -ErrorAction SilentlyContinue
    } else { $env:MDLITE_TEST_NO_CRASH_UI = $previousCrashUiSetting }
    if ($null -eq $previousSilentSetting) {
        Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue
    } else { $env:MDLITE_TEST_SILENT = $previousSilentSetting }
}
