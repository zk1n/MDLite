#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug','release')]
    [string]$Preset = 'release',
    [string]$OutputPath,
    [switch]$OnlyE20
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }
if (-not $OutputPath) { $OutputPath = Join-Path $repoRoot "build\verification\gui-acceptance-$Preset.json" }
$previousCrashUiSetting = $env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Text;
using System.Runtime.InteropServices;

public static class MDLiteNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int left; public int top; public int right; public int bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int x; public int y; }
    [DllImport("user32.dll")]
    public static extern bool ScreenToClient(IntPtr window, ref POINT point);
    [StructLayout(LayoutKind.Sequential)]
    public struct SCROLLINFO {
        public uint cbSize;
        public uint fMask;
        public int nMin;
        public int nMax;
        public uint nPage;
        public int nPos;
        public int nTrackPos;
    }
    [StructLayout(LayoutKind.Sequential)]
    public struct GUITHREADINFO {
        public int cbSize;
        public uint flags;
        public IntPtr hwndActive;
        public IntPtr hwndFocus;
        public IntPtr hwndCapture;
        public IntPtr hwndMenuOwner;
        public IntPtr hwndMoveSize;
        public IntPtr hwndCaret;
        public RECT rcCaret;
    }
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    public struct LOGFONTW {
        public int lfHeight;
        public int lfWidth;
        public int lfEscapement;
        public int lfOrientation;
        public int lfWeight;
        public byte lfItalic;
        public byte lfUnderline;
        public byte lfStrikeOut;
        public byte lfCharSet;
        public byte lfOutPrecision;
        public byte lfClipPrecision;
        public byte lfQuality;
        public byte lfPitchAndFamily;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 32)] public string lfFaceName;
    }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string windowName);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool GetGUIThreadInfo(uint threadId, ref GUITHREADINFO info);
    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowTextLength(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
    public static IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam) {
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        IntPtr result;
        IntPtr api = SendMessageTimeoutW(window, message, wparam, lparam, 2, 5000, out result);
        return CheckMessageResult(api, result, window, message, elapsed);
    }
    public static IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, string lparam) {
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        IntPtr result;
        IntPtr api = SendMessageTimeoutText(window, message, wparam, lparam, 2, 5000, out result);
        return CheckMessageResult(api, result, window, message, elapsed);
    }
    public static IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, StringBuilder lparam) {
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        IntPtr result;
        IntPtr api = SendMessageTimeoutBuffer(window, message, wparam, lparam, 2, 5000, out result);
        return CheckMessageResult(api, result, window, message, elapsed);
    }
    private static IntPtr CheckMessageResult(IntPtr api, IntPtr result, IntPtr window,
                                            uint message, System.Diagnostics.Stopwatch elapsed) {
        if (api != IntPtr.Zero) return result;
        int error = Marshal.GetLastWin32Error();
        uint owner;
        uint thread = GetWindowThreadProcessId(window, out owner);
        throw new TimeoutException("Native message failed: hwnd=" + window + ", owner_pid=" + owner +
            ", thread=" + thread + ", message=0x" + message.ToString("X") +
            ", configured_timeout_ms=5000, elapsed_ms=" + elapsed.Elapsed.TotalMilliseconds +
            ", win32_error=" + error + ", is_window=" + IsWindow(window));
    }
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SendMessageTimeoutText(IntPtr window, uint message, IntPtr wparam,
        string lparam, uint flags, uint timeoutMilliseconds, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern IntPtr SendMessageTimeoutBuffer(IntPtr window, uint message, IntPtr wparam,
        StringBuilder lparam, uint flags, uint timeoutMilliseconds, out IntPtr result);
    [DllImport("user32.dll", EntryPoint = "SendMessageTimeoutW", SetLastError = true)]
    public static extern IntPtr SendMessageTimeoutW(
        IntPtr window, uint message, IntPtr wparam, IntPtr lparam,
        uint flags, uint timeoutMilliseconds, out IntPtr result);
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll", SetLastError = true)]
    public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")]
    private static extern bool GetScrollInfo(IntPtr window, int bar, ref SCROLLINFO info);
    public static int GetScrollPosition(IntPtr window, int bar) {
        var info = new SCROLLINFO();
        info.cbSize = (uint)Marshal.SizeOf(typeof(SCROLLINFO));
        info.fMask = 0x0004;
        return GetScrollInfo(window, bar, ref info) ? info.nPos : -1;
    }
    [DllImport("user32.dll")]
    public static extern uint GetDpiForWindow(IntPtr window);
    [DllImport("gdi32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetObject(IntPtr handle, int size, out LOGFONTW font);
    [DllImport("user32.dll")]
    public static extern bool InvalidateRect(IntPtr window, IntPtr rectangle, bool erase);
    [DllImport("user32.dll")]
    public static extern bool UpdateWindow(IntPtr window);
    [DllImport("user32.dll")]
    public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongPtrW")]
    public static extern IntPtr GetWindowLongPtr(IntPtr window, int index);
    [DllImport("user32.dll")]
    public static extern IntPtr SetFocus(IntPtr window);
    [DllImport("kernel32.dll")]
    private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    private static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")]
    private static extern bool AttachThreadInput(uint source, uint target, bool attach);
    public static bool FocusKeyboardControl(IntPtr main, IntPtr control) {
        uint processId;
        uint targetThread = GetWindowThreadProcessId(control, out processId);
        uint currentThread = GetCurrentThreadId();
        bool attached = false;
        try {
            if (targetThread != currentThread) attached = AttachThreadInput(currentThread, targetThread, true);
            if (targetThread != currentThread && !attached) return false;
            SetForegroundWindow(main);
            SetFocus(control);
        } finally {
            if (attached && !AttachThreadInput(currentThread, targetThread, false)) throw new InvalidOperationException("Could not detach keyboard focus setup.");
        }
        return GetFocusedWindow(control) == control;
    }
    public static IntPtr GetFocusedWindow(IntPtr window) {
        uint processId;
        uint threadId = GetWindowThreadProcessId(window, out processId);
        var info = new GUITHREADINFO();
        info.cbSize = Marshal.SizeOf(typeof(GUITHREADINFO));
        return GetGUIThreadInfo(threadId, ref info) ? info.hwndFocus : IntPtr.Zero;
    }
    public static string DescribeChildWindows(IntPtr parent) {
        var parts = new List<string>();
        EnumChildWindows(parent, (window, unused) => {
            var className = new StringBuilder(128);
            GetClassName(window, className, className.Capacity);
            int length = GetWindowTextLength(window);
            var text = new StringBuilder(length + 1);
            if (length > 0) GetWindowText(window, text, text.Capacity);
            parts.Add(GetDlgCtrlID(window) + ":" + className + ":" + text);
            return true;
        }, IntPtr.Zero);
        return String.Join(" | ", parts);
    }
    public static IntPtr FindChildWithText(IntPtr parent, string wantedText, string wantedClass) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (window, unused) => {
            var className = new StringBuilder(128);
            GetClassName(window, className, className.Capacity);
            if (className.ToString() == wantedClass) {
                int length = GetWindowTextLength(window);
                var text = new StringBuilder(length + 1);
                if (length > 0) GetWindowText(window, text, text.Capacity);
                if (text.ToString() == wantedText) {
                    found = window;
                    return false;
                }
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr[] GetChildWindowHandles(IntPtr parent) {
        var handles = new List<IntPtr>();
        EnumChildWindows(parent, (window, unused) => {
            handles.Add(window);
            return true;
        }, IntPtr.Zero);
        return handles.ToArray();
    }
}
'@

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_SETTEXT = 0x000C
$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$BM_CLICK = 0x00F5
$WM_GETFONT = 0x0031
$WM_CHAR = 0x0102
$WM_SETFOCUS = 0x0007
$WM_SIZE = 0x0005
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$WM_UNDO = 0x0304
$EM_SETSEL = 0x00B1
$EM_GETSEL = 0x00B0
$EM_LINEFROMCHAR = 0x00C9
$EM_REPLACESEL = 0x00C2
$EM_REDO = 0x0454
$EM_GETEVENTMASK = 0x043B
$EM_SETEVENTMASK = 0x0445
$ENM_SELCHANGE = 0x00080000
$VK_BACK = 0x08
$VK_DELETE = 0x2E
$VK_RIGHT = 0x27
$CFE_HIDDEN = 0x00000100
$CFM_HIDDEN = 0x00000100
$testFailNextEditorReadbackMessage = 0x802E
$testSetSelectionBySourceMessage = 0x8032
$testGetSourceAnchorMessage = 0x8033
$testGetSourceActiveMessage = 0x8034
$testGetTabItemCenterMessage = 0x803A
$testGetSuspendedViewAnchorLineMessage = 0x803B
$testFailNextMarkdownPresentationMessage = 0x803C
$testGetVisibleSourceOffsetMessage = 0x803D
$testGetVerticalSourceOffsetMessage = 0x803E
$testFailMarkdownPresentationAfterFirstTableMessage = 0x803F
$testSetNativeProjectionFailureStagesMessage = 0x8040
$testTrustWorkspaceForGitMessage = 0x8043
$testGetGitActionActiveMessage = 0x8044
# WM_APP+64 stage bits: programmatic write=1, restore=2, incremental=4,
# rebuild=8, and flat fallback=16.
$nativeProjectionFaultProgrammaticWrite = 0x01
$nativeProjectionFaultProgrammaticRestore = 0x02
$nativeProjectionFaultRebuildWrite = 0x08
$testGetEditorReadinessMessage = 0x8036
$testGetCharFormatAtSourceRangeMessage = 0x802F
$BM_SETCHECK = 0x00F1
$BM_CLICK = 0x00F5
$LVM_GETITEMCOUNT = 0x1004
$TVM_GETCOUNT = 0x1105
$TVM_GETNEXTITEM = 0x110A
$TVM_EXPAND = 0x1102
$TVM_SELECTITEM = 0x110B
$TVM_GETIMAGELIST = 0x1108
$TVGN_ROOT = 0
$TVGN_CHILD = 4
$TVGN_CARET = 9
$TVE_EXPAND = 2
$GWL_STYLE = -16
$ES_READONLY = 0x0800
$TCS_OWNERDRAWFIXED = 0x2000
$TVS_HASLINES = 0x0002
$TVS_LINESATROOT = 0x0004
$LB_GETCOUNT = 0x018B
$BST_UNCHECKED = 0
$BST_CHECKED = 1
$IDOK = 1
$IDCANCEL = 2
$script:workspaceDocumentReadinessTraces = [Collections.Generic.List[object]]::new()

function Wait-ProcessWindow([Diagnostics.Process]$Process, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Process.Refresh()
        $window = $Process.MainWindowHandle
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main window was not created for process $($Process.Id)"
}

function Wait-ProcessClassWindow([Diagnostics.Process]$Process, [string]$ClassName, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $script:foundWindow = [IntPtr]::Zero
        $callback = [MDLiteNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$parameter)
            [uint32]$processId = 0
            [void][MDLiteNative]::GetWindowThreadProcessId($window, [ref]$processId)
            if ($processId -ne [uint32]$Process.Id) { return $true }
            $name = New-Object Text.StringBuilder 128
            [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
            if ($name.ToString() -ne $ClassName) { return $true }
            $script:foundWindow = $window
            return $false
        }
        [void][MDLiteNative]::EnumWindows($callback, [IntPtr]::Zero)
        if ($script:foundWindow -ne [IntPtr]::Zero) { return $script:foundWindow }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window was not created for process $($Process.Id): $ClassName"
}

function Get-ProcessClassWindow([Diagnostics.Process]$Process, [string]$ClassName) {
    $script:foundWindow = [IntPtr]::Zero
    $callback = [MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        [uint32]$processId = 0
        [void][MDLiteNative]::GetWindowThreadProcessId($window, [ref]$processId)
        if ($processId -ne [uint32]$Process.Id) { return $true }
        $name = New-Object Text.StringBuilder 128
        [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
        if ($name.ToString() -ne $ClassName) { return $true }
        $script:foundWindow = $window
        return $false
    }
    [void][MDLiteNative]::EnumWindows($callback, [IntPtr]::Zero)
    return $script:foundWindow
}

function Wait-ProcessClassWindowGone([Diagnostics.Process]$Process, [string]$ClassName, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        if ((Get-ProcessClassWindow $Process $ClassName) -eq [IntPtr]::Zero) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window did not close for process $($Process.Id): $ClassName"
}

function Find-Control([IntPtr]$Parent, [int]$Id, [string]$ClassName = '') {
    $script:foundControl = [IntPtr]::Zero
    $callback = [MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLiteNative]::GetDlgCtrlID($window) -ne $Id) { return $true }
        if ($ClassName) {
            $name = New-Object Text.StringBuilder 128
            [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
            if ($name.ToString() -ne $ClassName) { return $true }
        }
        $script:foundControl = $window
        return $false
    }
    [void][MDLiteNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return $script:foundControl
}

function Find-ChildClassWindow([IntPtr]$Parent, [string]$ClassName) {
    $script:foundClassWindow = [IntPtr]::Zero
    $callback = [MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        $name = New-Object Text.StringBuilder 128
        [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
        if ($name.ToString() -eq $ClassName) {
            $script:foundClassWindow = $window
            return $false
        }
        return $true
    }
    [void][MDLiteNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return $script:foundClassWindow
}

function Wait-Control([IntPtr]$Parent, [int]$Id, [string]$ClassName = '', [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $control = Find-Control $Parent $Id $ClassName
        if ($control -ne [IntPtr]::Zero) { return $control }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    $children = [MDLiteNative]::DescribeChildWindows($Parent)
    throw "Control was not created: id=$Id class=$ClassName; children=$children"
}

function Wait-VisibleControl([IntPtr]$Parent, [int]$Id, [string]$ClassName = '',
                             [int]$TimeoutMs = 10000, [string]$Context = '') {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $script:foundVisibleControl = [IntPtr]::Zero
        $callback = [MDLiteNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$parameter)
            if (-not [MDLiteNative]::IsWindowVisible($window) -or
                [MDLiteNative]::GetDlgCtrlID($window) -ne $Id) { return $true }
            if ($ClassName) {
                $name = New-Object Text.StringBuilder 128
                [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
                if ($name.ToString() -ne $ClassName) { return $true }
            }
            $script:foundVisibleControl = $window
            return $false
        }
        [void][MDLiteNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
        if ($script:foundVisibleControl -ne [IntPtr]::Zero) { return $script:foundVisibleControl }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Visible control was not created: id=$Id class=$ClassName context=$Context"
}

function Get-WindowText([IntPtr]$Window) {
    $length = [MDLiteNative]::SendMessage($Window, $WM_GETTEXTLENGTH, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $text = New-Object Text.StringBuilder ($length + 1)
    [void][MDLiteNative]::SendMessage($Window, $WM_GETTEXT, [IntPtr]($length + 1), $text)
    return $text.ToString()
}

function Wait-WorkspaceDocumentReady([IntPtr]$Main, [Diagnostics.Process]$Target, [string]$DocumentName,
    [string]$Workspace, [int]$TimeoutMs = 10000) {
    $started = [DateTime]::UtcNow
    $deadline = $started.AddMilliseconds($TimeoutMs)
    $caption = 'MDLite — ' + $DocumentName + ' — ' + [IO.Path]::GetFileName($Workspace)
    $stable = 0; $lastEditor = [IntPtr]::Zero
    $samples = [Collections.Generic.List[object]]::new()
    $trace = [ordered]@{ document = $DocumentName; owner_pid = $Target.Id; main_hwnd = $Main.ToInt64(); started_utc = $started.ToString('o'); timeout_ms = $TimeoutMs; status = 'WAITING'; samples = $samples }
    $script:workspaceDocumentReadinessTraces.Add($trace)
    do {
        $sampleStarted = [DateTime]::UtcNow
        $Target.Refresh()
        if ($Target.HasExited) { $trace.status = 'PROCESS_EXITED'; throw 'Workspace process exited before final document readiness.' }
        $owner = [uint32]0
        [void][MDLiteNative]::GetWindowThreadProcessId($Main, [ref]$owner)
        $class = [Text.StringBuilder]::new(64)
        [void][MDLiteNative]::GetClassName($Main, $class, $class.Capacity)
        if ($owner -ne $Target.Id -or $class.ToString() -cne 'MDLite.MainWindow') {
            $trace.status = 'OWNERSHIP_MISMATCH'; $trace.observed_owner_pid = $owner
            throw 'Workspace caption observation requires the owned top-level MDLite window.'
        }
        # Foreign-process top-level caption reads use the OS caption cache,
        # avoiding two separately bounded WM_GETTEXT calls during startup.
        $captionText = [Text.StringBuilder]::new([Math]::Max(1024, $caption.Length + 2))
        [void][MDLiteNative]::GetWindowText($Main, $captionText, $captionText.Capacity)
        $captionMatches = $captionText.ToString() -ceq $caption
        $editors = @(Get-CurrentVisibleEditors $Main $Target.Id)
        $enabled = $editors.Count -eq 1 -and [MDLiteNative]::IsWindowEnabled($editors[0])
        $ready = [IntPtr]::Zero; $readyApi = [IntPtr]::Zero
        $remaining = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
        if ($captionMatches -and $enabled -and $remaining -gt 0) {
            $readyApi = [MDLiteNative]::SendMessageTimeoutW($Main, 0x8036, [IntPtr]::Zero, [IntPtr]::Zero, 2, [uint32][Math]::Min(250, $remaining), [ref]$ready)
        }
        $sourceReady = $readyApi -ne [IntPtr]::Zero -and ($ready.ToInt64() -band 5) -eq 5
        if ($captionMatches -and $enabled -and $sourceReady) {
            if ($editors[0] -eq $lastEditor) { $stable++ } else { $stable = 1; $lastEditor = $editors[0] }
        } else { $stable = 0 }
        $observed = [DateTime]::UtcNow
        if ($samples.Count -lt 256) { $samples.Add([ordered]@{
            started_elapsed_ms = [Math]::Round(($sampleStarted - $started).TotalMilliseconds, 3)
            finished_elapsed_ms = [Math]::Round(($observed - $started).TotalMilliseconds, 3)
            caption_matches = $captionMatches; editor_count = $editors.Count; editor_enabled = $enabled
            editor_hwnd = if ($editors.Count -eq 1) { $editors[0].ToInt64() } else { 0 }
            readiness_api_ok = $readyApi -ne [IntPtr]::Zero; readiness_flags = $ready.ToInt64()
            stable_reads = $stable; within_deadline = $observed -lt $deadline
        }) }
        if ($stable -ge 2 -and [DateTime]::UtcNow -lt $deadline) {
            $trace.status = 'READY_IN_BUDGET'; $trace.elapsed_ms = [Math]::Round(([DateTime]::UtcNow - $started).TotalMilliseconds, 3)
            if ([DateTime]::UtcNow -lt $deadline) { return $editors[0] }
        }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $trace.status = 'TIMEOUT'; $trace.elapsed_ms = [Math]::Round(([DateTime]::UtcNow - $started).TotalMilliseconds, 3)
    throw "Final workspace/document readiness did not settle within $TimeoutMs ms; see caption/editor/readiness observation trace."
}

function Invoke-CalendarToggleCheck([IntPtr]$Main, [int]$TimeoutMs = 5000) {
    $calendar = Find-ChildClassWindow $Main 'MDLite.CalendarView'
    if ($calendar -eq [IntPtr]::Zero) { throw 'Calendar control was not created.' }
    $initial = [MDLiteNative]::IsWindowVisible($calendar)
    $started = [DateTime]::UtcNow; $deadline = $started.AddMilliseconds($TimeoutMs)
    $observations = [Collections.Generic.List[object]]::new()
    foreach ($expected in @((-not $initial), $initial)) {
        $remaining = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
        if ($remaining -le 0) { throw 'Calendar toggle exhausted its shared command/observation deadline.' }
        $reply = [IntPtr]::Zero
        $api = [MDLiteNative]::SendMessageTimeoutW($Main, 0x0111, [IntPtr]1026, [IntPtr]::Zero, 2, [uint32]$remaining, [ref]$reply)
        if ($api -eq [IntPtr]::Zero) { throw 'Calendar menu toggle did not complete within its deadline.' }
        $matched = $false
        do {
            $visible = [MDLiteNative]::IsWindowVisible($calendar)
            $observed = [DateTime]::UtcNow
            if ($visible -eq $expected -and $observed -lt $deadline) { $matched = $true; break }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $deadline)
        if (-not $matched) { throw 'Calendar visibility did not reach the exact expected toggle state before deadline.' }
        $observations.Add([ordered]@{ expected_visible = $expected; observed_visible = $visible; elapsed_ms = [Math]::Round(($observed - $started).TotalMilliseconds, 3); command_completed = $true })
    }
    $result = [ordered]@{ calendar_control = $true; calendar_visible = $observations[0].observed_visible -ne $initial; calendar_hidden = $observations[1].observed_visible -eq $initial
        calendar_toggle_trace = [ordered]@{ initial_visible = $initial; after_first_command = $observations[0].observed_visible; after_second_command = $observations[1].observed_visible; observations = $observations.ToArray(); timeout_ms = $TimeoutMs } }
    if ([DateTime]::UtcNow -ge $deadline) { throw 'Calendar toggle result missed its shared command/observation deadline.' }
    return $result
}

function Test-WholeSourceSelection($Selection, [int]$SourceLength) {
    return $SourceLength -gt 0 -and [Math]::Min($Selection.anchor, $Selection.active) -eq 0 -and
        [Math]::Max($Selection.anchor, $Selection.active) -eq $SourceLength
}

function Get-WorkspaceSplitterPosition([IntPtr]$Main) {
    $header = Find-Control $Main 1072 'Button'
    if ($header -eq [IntPtr]::Zero -or -not [MDLiteNative]::IsWindowVisible($header)) { throw 'Explorer header is not visible for splitter geometry.' }
    $rect = [MDLiteNative+RECT]::new()
    if (-not [MDLiteNative]::GetWindowRect($header, [ref]$rect)) { throw 'Explorer header bounds are unavailable.' }
    $dpi = [int][MDLiteNative]::GetDpiForWindow($Main)
    if ($dpi -eq 0) { $dpi = 96 }
    $splitterWidth = [int][Math]::Floor((4 * $dpi + 48) / 96.0)
    $point = [MDLiteNative+POINT]::new()
    $point.x = $rect.right + [int][Math]::Floor($splitterWidth / 2)
    $point.y = $rect.bottom + [int][Math]::Floor((24 * $dpi + 48) / 96.0)
    if (-not [MDLiteNative]::ScreenToClient($Main, [ref]$point)) { throw 'Could not map splitter geometry into main-client coordinates.' }
    return [pscustomobject]@{ x = $point.x; y = $point.y; dpi = $dpi; splitter_width = $splitterWidth; header_hwnd = $header.ToInt64() }
}

function Wait-SaveAsControlsReady([Diagnostics.Process]$Target, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $remaining = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
        if ($remaining -le 0) { break }
        $dialog = Wait-ProcessClassWindow $Target '#32770' $remaining
        if ([MDLiteNative]::IsWindowVisible($dialog)) {
            $edit = Find-Control $dialog 1001 'Edit'
            $save = Find-Control $dialog 1 'Button'
            if ($edit -ne [IntPtr]::Zero -and $save -ne [IntPtr]::Zero -and
                [MDLiteNative]::IsWindowVisible($edit) -and [MDLiteNative]::IsWindowEnabled($edit) -and
                [MDLiteNative]::IsWindowVisible($save) -and [MDLiteNative]::IsWindowEnabled($save) -and
                (Get-WindowText $save) -match '^(保存|Save)') {
                if ([DateTime]::UtcNow -ge $deadline) { throw 'Save As controls matched only after the existing readiness deadline.' }
                return [pscustomobject]@{ dialog = $dialog; edit = $edit; save = $save }
            }
        }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Visible Save As filename host and semantic Save button were not ready within $TimeoutMs ms."
}

function Get-CurrentVisibleEditors([IntPtr]$Main, [int]$OwnerPid) {
    $script:quickOpenEditors=[Collections.Generic.List[IntPtr]]::new()
    $callback=[MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window,[IntPtr]$parameter)
        if(-not [MDLiteNative]::IsWindowVisible($window) -or [MDLiteNative]::GetDlgCtrlID($window) -ne 102){return $true}
        $owner=[uint32]0
        [void][MDLiteNative]::GetWindowThreadProcessId($window,[ref]$owner)
        $class=New-Object Text.StringBuilder 128
        [void][MDLiteNative]::GetClassName($window,$class,$class.Capacity)
        if($owner -eq $OwnerPid -and $class.ToString() -ceq 'RICHEDIT50W'){$script:quickOpenEditors.Add($window)}
        return $true
    }
    [void][MDLiteNative]::EnumChildWindows($Main,$callback,[IntPtr]::Zero)
    return @($script:quickOpenEditors.ToArray())
}
function Wait-QuickOpenDocumentEditor([IntPtr]$Main, $Target, [string]$DocumentName,
    [string]$ExpectedText, [switch]$Prefix, [int]$TimeoutMs=10000) {
    $started=[DateTime]::UtcNow;$deadline=$started.AddMilliseconds($TimeoutMs)
    $captionPattern='^MDLite — '+[regex]::Escape($DocumentName)+'(?: \*)? — .+$'
    $previousEditor=[IntPtr]::Zero;$previousText=$null;$stable=0;$invalidHandleRetries=0
    $last=[ordered]@{pid=$Target.Id;main_hwnd=$Main.ToInt64();expected_document=$DocumentName;caption=$null;editor_hwnd=$null;visible_editor_count=0;text_matches=$false;elapsed_ms=0;last_transient_error=$null}
    while([DateTime]::UtcNow -lt $deadline){
        $Target.Refresh()
        if($Target.HasExited){throw "Quick Open process exited before '$DocumentName' editor readiness (pid=$($Target.Id))."}
        $last.caption=Get-WindowText $Main
        $last.elapsed_ms=[Math]::Round(([DateTime]::UtcNow-$started).TotalMilliseconds,3)
        $current=@(Get-CurrentVisibleEditors $Main $Target.Id)
        $last.visible_editor_count=$current.Count
        if($last.caption -cmatch $captionPattern -and $current.Count -eq 1){
            $candidate=$current[0];$last.editor_hwnd=$candidate.ToInt64()
            try{$text=(Get-WindowText $candidate) -replace "`r`n","`n"}
            catch{
                $message=$_.Exception.GetBaseException().Message
                if($message -match 'win32_error=1400, is_window=False'){
                    $invalidHandleRetries++;$last.last_transient_error=$message
                    $stable=0;$previousEditor=[IntPtr]::Zero
                    Start-Sleep -Milliseconds 40
                    continue
                }
                throw
            }
            $confirmed=@(Get-CurrentVisibleEditors $Main $Target.Id)
            $confirmedCaption=Get-WindowText $Main
            $last.text_matches=if($Prefix){$text.StartsWith($ExpectedText,[StringComparison]::Ordinal)}else{$text -ceq $ExpectedText}
            if($confirmed.Count -eq 1 -and $confirmed[0] -eq $candidate -and [MDLiteNative]::IsWindow($candidate) -and
                $confirmedCaption -ceq $last.caption -and $last.text_matches){
                $stable=if($previousEditor -eq $candidate -and $previousText -ceq $text){$stable+1}else{1}
                $previousEditor=$candidate;$previousText=$text
                if($stable -ge 2 -and [DateTime]::UtcNow -lt $deadline){
                    return [pscustomobject]@{Editor=$candidate;Text=$text;Evidence=[ordered]@{pid=$Target.Id;main_hwnd=$Main.ToInt64();editor_hwnd=$candidate.ToInt64();document=$DocumentName;caption=$confirmedCaption;started_utc=$started.ToString('o');ready_utc=[DateTime]::UtcNow.ToString('o');elapsed_ms=[Math]::Round(([DateTime]::UtcNow-$started).TotalMilliseconds,3);invalid_handle_retries=$invalidHandleRetries;stable_reads=$stable;expected_text_matched=$true}}
                }
            }else{$stable=0;$previousEditor=[IntPtr]::Zero}
        }else{$stable=0;$previousEditor=[IntPtr]::Zero}
        Start-Sleep -Milliseconds 40
    }
    throw "Quick Open selected-document readiness timed out (configured_timeout_ms=$TimeoutMs, invalid_handle_retries=$invalidHandleRetries): $($last | ConvertTo-Json -Compress)."
}

function Save-WindowCapture([IntPtr]$Window, [string]$Path) {
    $rect = New-Object MDLiteNative+RECT
    if (-not [MDLiteNative]::GetWindowRect($Window, [ref]$rect)) {
        return [pscustomobject]@{ status='BLOCKED'; error='GetWindowRect failed'; path=$Path; sha256=$null }
    }
    $width = $rect.right - $rect.left
    $height = $rect.bottom - $rect.top
    if ($width -le 0 -or $height -le 0) {
        return [pscustomobject]@{ status='BLOCKED'; error='Window bounds are empty'; path=$Path; sha256=$null }
    }
    $bitmap = New-Object Drawing.Bitmap($width, $height)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $hdc = $graphics.GetHdc()
        try { $captured = [MDLiteNative]::PrintWindow($Window, $hdc, 2) }
        finally { $graphics.ReleaseHdc($hdc) }
        if (-not $captured) {
            return [pscustomobject]@{ status='BLOCKED'; error='PrintWindow failed'; path=$Path; sha256=$null }
        }
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    } finally {
        $graphics.Dispose()
        $bitmap.Dispose()
    }
    return [pscustomobject]@{
        status='CAPTURED_PRINTWINDOW'
        error=$null
        path=$Path
        sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    }
}

function Send-Characters([IntPtr]$Editor, [string]$Text) {
    [void][MDLiteNative]::SendMessage($Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    foreach ($character in $Text.ToCharArray()) {
        [void][MDLiteNative]::SendMessage($Editor, $WM_CHAR, [IntPtr][int]$character, [IntPtr]::Zero)
    }
}

function Send-SelectedCharacters([IntPtr]$Editor, [string]$Text) {
    [void][MDLiteNative]::SendMessage($Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    foreach ($character in $Text.ToCharArray()) {
        [void][MDLiteNative]::SendMessage($Editor, $WM_CHAR, [IntPtr][int]$character, [IntPtr]::Zero)
    }
}

function Wait-ListItemCount([IntPtr]$List, [int]$Minimum, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $count = [MDLiteNative]::SendMessage(
            $List, $LVM_GETITEMCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($count -ge $Minimum) { return $count }
        Start-Sleep -Milliseconds 20
    } while ([DateTime]::UtcNow -lt $deadline)
    return $count
}

function Get-WindowClass([IntPtr]$Window) {
    $name = New-Object Text.StringBuilder 128
    [void][MDLiteNative]::GetClassName($Window, $name, $name.Capacity)
    return $name.ToString()
}

function Get-ChildClassWindows([IntPtr]$Parent, [string]$ClassName) {
    $script:matchingClassWindows = [Collections.Generic.List[IntPtr]]::new()
    $callback = [MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        $name = New-Object Text.StringBuilder 128
        [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
        if ($name.ToString() -eq $ClassName) { $script:matchingClassWindows.Add($window) }
        return $true
    }
    [void][MDLiteNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return @($script:matchingClassWindows.ToArray())
}

function Get-TabItemCenter([IntPtr]$Main, [int]$Index) {
    $packed = [MDLiteNative]::SendMessage(
        $Main, $testGetTabItemCenterMessage, [IntPtr]$Index, [IntPtr]::Zero).ToInt64()
    if ($packed -eq -1) { throw "Tab center unavailable for index $Index" }
    return [pscustomobject]@{
        x = [int]($packed -band 0xFFFF)
        y = [int](($packed -shr 16) -band 0xFFFF)
    }
}

function Click-TabItem([IntPtr]$Main, [IntPtr]$Tabs, [int]$Index) {
    # Query immediately before each click: owner-draw layout and current selection
    # can change the tab rectangles. These are synthetic Win32 messages.
    $center = Get-TabItemCenter $Main $Index
    $point = [IntPtr]([int](($center.y -shl 16) -bor ($center.x -band 0xFFFF)))
    [void][MDLiteNative]::SendMessage($Tabs, $WM_LBUTTONDOWN, [IntPtr]1, $point)
    [void][MDLiteNative]::SendMessage($Tabs, $WM_LBUTTONUP, [IntPtr]0, $point)
}

function Get-FirstVisibleLine([IntPtr]$Editor) {
    return [MDLiteNative]::SendMessage($Editor, 0x00CE, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
}

function Get-SourceSelectionByMessage([IntPtr]$Main) {
    return [ordered]@{
        anchor = [MDLiteNative]::SendMessage(
            $Main, $testGetSourceAnchorMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        active = [MDLiteNative]::SendMessage(
            $Main, $testGetSourceActiveMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    }
}

function Invoke-SuspendedEditorLifecycle([string]$Directory) {
    [IO.Directory]::CreateDirectory((Join-Path $Directory '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $Directory '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $firstPath = Join-Path $Directory 'suspend-first.md'
    $secondPath = Join-Path $Directory 'suspend-second.md'
    $firstLines = @('# First lifecycle document') + @(
        1..180 | ForEach-Object { 'FIRST-LINE-{0:D3} distinct inactive document content' -f $_ })
    $lines = @('# Suspension lifecycle fixture') + @(
        1..180 | ForEach-Object { 'LIFE-LINE-{0:D3} payload for editor restoration' -f $_ })
    $firstSource = $firstLines -join "`n"
    $source = $lines -join "`n"
    [IO.File]::WriteAllText($firstPath, $firstSource, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($secondPath, $source, [Text.UTF8Encoding]::new($false))

    $child = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $firstPath) -PassThru
    $script:readbackProcesses.Add($child)
    $main = Wait-ProcessWindow $child
    $tabs = Wait-Control $main 101 'SysTabControl32'
    $quickOpenEditor = Wait-VisibleControl $main 102 'RICHEDIT50W'

    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1003, [IntPtr]::Zero)
    $picker = Wait-ProcessClassWindow $child 'MDLite.NativePickerWindow'
    $filter = Wait-Control $picker 100 'Edit'
    $list = Wait-Control $picker 101 'ListBox'
    [void][MDLiteNative]::SendMessage($filter, $WM_SETTEXT, [IntPtr]::Zero, 'suspend-second.md')
    Start-Sleep -Milliseconds 150
    $candidateCount = [MDLiteNative]::SendMessage(
        $list, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($candidateCount -ne 1) { throw "Expected one lifecycle quick-open candidate, got $candidateCount" }
    [void][MDLiteNative]::SendMessage($picker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $child 'MDLite.NativePickerWindow'
    $tabDeadline = [DateTime]::UtcNow.AddSeconds(3)
    $tabCount = 0
    do {
        $tabCount = [MDLiteNative]::SendMessage(
            $tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($tabCount -eq 2) { break }
        if ($tabCount -gt 2) { throw "Expected exactly two lifecycle tabs, got $tabCount" }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $tabDeadline)
    if ($tabCount -ne 2) { throw "Expected exactly two lifecycle tabs, got $tabCount" }
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $activeEditorText = (Get-WindowText $editor) -replace "`r`n", "`n"
    if ($activeEditorText -cne $source) {
        throw 'Quick Open did not activate the lifecycle fixture document.'
    }

    $anchor = $source.IndexOf('LIFE-LINE-112 payload', [StringComparison]::Ordinal) + 20
    $active = $source.IndexOf('LIFE-LINE-110 payload', [StringComparison]::Ordinal) + 5
    $pendingMarker = "`nSUSPEND-READBACK-PENDING"
    [void][MDLiteNative]::SendMessage($editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    [void][MDLiteNative]::SendMessage($editor, $EM_REPLACESEL, [IntPtr]1, $pendingMarker)
    $expectedBuffer = $source + $pendingMarker
    $expectedLines = @($expectedBuffer -split "`n")
    $selectionSet = [MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$anchor, [IntPtr]$active).ToInt32()
    # Move the viewport a few lines above the directional selection while
    # keeping its active endpoint visible through focus transitions.
    [void][MDLiteNative]::SendMessage($editor, 0x00B6, [IntPtr]::Zero, [IntPtr](-5))
    $initialSelection = Get-SourceSelectionByMessage $main
    $initialVisibleLine = Get-FirstVisibleLine $editor
    $initialText = (Get-WindowText $editor) -replace "`r`n", "`n"
    $expectedVisibleLine = if ($initialVisibleLine -ge 0 -and $initialVisibleLine -lt $expectedLines.Count) {
        $expectedLines[$initialVisibleLine]
    } else { '' }
    $initialStateValid = $selectionSet -ne 0 -and $initialSelection.anchor -eq $anchor -and
        $initialSelection.active -eq $active -and $anchor -gt $active -and
        $initialVisibleLine -ge 70 -and $expectedVisibleLine.Length -gt 0 -and
        $initialText -ceq $expectedBuffer

    # Force one readback failure while the second document is about to become
    # inactive. It should remain allocated and retain its complete native text.
    [void][MDLiteNative]::SendMessage($main, $testFailNextEditorReadbackMessage, [IntPtr]1, [IntPtr]::Zero)
    Click-TabItem $main $tabs 0
    $firstView = Wait-VisibleControl $main 102 'RICHEDIT50W'
    Start-Sleep -Milliseconds 150
    $editorsAfterFailure = @(Get-ChildClassWindows $main 'RICHEDIT50W')
    $residentText = $false
    foreach ($candidate in $editorsAfterFailure) {
        if (((Get-WindowText $candidate) -replace "`r`n", "`n") -ceq $expectedBuffer) { $residentText = $true }
    }
    $failureRetainedEditor = $editorsAfterFailure.Count -eq 2 -and $residentText

    # Return to the retained editor, then switch it away again to retry suspend.
    # A successful retry destroys that HWND; reactivation must recreate the view.
    Click-TabItem $main $tabs 1
    $retainedAgain = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $retrySourcePreserved = ((Get-WindowText $retainedAgain) -replace "`r`n", "`n") -ceq $expectedBuffer
    $retrySelectionSet = [MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$anchor, [IntPtr]$active).ToInt32()
    $retryCurrentVisibleLine = Get-FirstVisibleLine $retainedAgain
    [void][MDLiteNative]::SendMessage($retainedAgain, 0x00B6, [IntPtr]::Zero,
        [IntPtr](105 - $retryCurrentVisibleLine))
    $preSuspendSelection = Get-SourceSelectionByMessage $main
    $preSuspendVisibleLine = Get-FirstVisibleLine $retainedAgain
    $preSuspendVisibleText = if ($preSuspendVisibleLine -ge 0 -and $preSuspendVisibleLine -lt $expectedLines.Count) {
        $expectedLines[$preSuspendVisibleLine]
    } else { '' }
    $preSuspendStateValid = $retrySelectionSet -ne 0 -and
        $preSuspendSelection.anchor -eq $anchor -and $preSuspendSelection.active -eq $active -and
        $preSuspendVisibleLine -eq $initialVisibleLine -and
        $preSuspendVisibleText -ceq $expectedVisibleLine
    Click-TabItem $main $tabs 0
    $afterRetryCount = @(Get-ChildClassWindows $main 'RICHEDIT50W').Count
    Click-TabItem $main $tabs 1
    $restored = Wait-VisibleControl $main 102 'RICHEDIT50W'
    Start-Sleep -Milliseconds 200
    $restoredText = (Get-WindowText $restored) -replace "`r`n", "`n"
    $restoredSelection = Get-SourceSelectionByMessage $main
    $restoredVisibleLine = Get-FirstVisibleLine $restored
    $restoredScrollAnchorLine = [MDLiteNative]::SendMessage(
        $main, $testGetSuspendedViewAnchorLineMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $restoredVisibleText = if ($restoredVisibleLine -ge 0 -and $restoredVisibleLine -lt $expectedLines.Count) {
        $expectedLines[$restoredVisibleLine]
    } else { '' }
    $firstVisibleLinePreserved = $restoredVisibleLine -eq $preSuspendVisibleLine
    $restoredStateValid = $restoredText -ceq $expectedBuffer -and
        $restoredSelection.anchor -eq $anchor -and $restoredSelection.active -eq $active -and
        $firstVisibleLinePreserved -and $restoredVisibleText -ceq $preSuspendVisibleText

    # Presentation failure must not publish a tab switch or release the current editor.
    [void][MDLiteNative]::SendMessage(
        $main, $testFailNextMarkdownPresentationMessage, [IntPtr]0, [IntPtr]::Zero)
    Click-TabItem $main $tabs 0
    $tabAfterPresentationFailure = [MDLiteNative]::SendMessage(
        $tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $editorAfterPresentationFailure = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $bufferAfterPresentationFailure = (Get-WindowText $editorAfterPresentationFailure) -replace "`r`n", "`n"
    $selectionAfterPresentationFailure = Get-SourceSelectionByMessage $main
    $presentationFailureKeptPreviousDocument =
        $tabAfterPresentationFailure -eq 1 -and
        $bufferAfterPresentationFailure -ceq $expectedBuffer -and
        $selectionAfterPresentationFailure.anchor -eq $anchor -and
        $selectionAfterPresentationFailure.active -eq $active
    Click-TabItem $main $tabs 0
    $presentationRetryEditor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $presentationRetryText = (Get-WindowText $presentationRetryEditor) -replace "`r`n", "`n"
    $presentationRetryTab = [MDLiteNative]::SendMessage(
        $tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $presentationRetrySucceeded = $presentationRetryTab -eq 0 -and $presentationRetryText -ceq $firstSource

    return [pscustomobject][ordered]@{
        pass = $initialStateValid -and $failureRetainedEditor -and $retrySourcePreserved -and
            $preSuspendStateValid -and
            $afterRetryCount -eq 1 -and $restoredStateValid -and
            $presentationFailureKeptPreviousDocument -and $presentationRetrySucceeded
        tab_count = $tabCount
        initial_selection = $initialSelection
        expected_anchor = $anchor
        expected_active = $active
        pending_native_edit = $pendingMarker
        initial_first_visible_line = $initialVisibleLine
        initial_first_visible_source_line = $expectedVisibleLine
        editor_windows_after_injected_readback_failure = $editorsAfterFailure.Count
        hidden_editor_text_preserved_after_failure = $residentText
        retry_source_preserved = $retrySourcePreserved
        pre_suspend_selection = $preSuspendSelection
        pre_suspend_first_visible_line = $preSuspendVisibleLine
        pre_suspend_first_visible_source_line = $preSuspendVisibleText
        editor_windows_after_successful_retry = $afterRetryCount
        restored_selection = $restoredSelection
        restored_scroll_anchor_line = $restoredScrollAnchorLine
        restored_first_visible_line = $restoredVisibleLine
        first_visible_line_preserved = $firstVisibleLinePreserved
        restored_first_visible_source_line = $restoredVisibleText
        source_restored = $restoredText -ceq $expectedBuffer
        presentation_failure_kept_previous_document = $presentationFailureKeptPreviousDocument
        presentation_failure_selected_tab = $tabAfterPresentationFailure
        presentation_retry_succeeded = $presentationRetrySucceeded
        synthetic_input = 'Cross-process SendMessage(WM_APP test probes, PostMessage(WM_COMMAND Quick Open), WM_LBUTTONDOWN/UP to SysTabControl32); no physical input.'
    }
}

function Invoke-WorkspaceSessionViewRestore([string]$Directory) {
    $readinessTraceStart = $script:workspaceDocumentReadinessTraces.Count
    [IO.Directory]::CreateDirectory((Join-Path $Directory '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $Directory '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $longPath = Join-Path $Directory 'long-horizontal.md'
    $tablePath = Join-Path $Directory 'table-body.md'
    $longLine = '0123456789abcdefghijklmnopqrstuvwxyz' * 700
    $longSource = $longLine
    $tableLines = @('# Session table fixture', '', '| Row | Value |', '| --- | --- |')
    foreach ($index in 1..120) { $tableLines += ('| row-{0:D3} | BODY-VALUE-{0:D3} |' -f $index) }
    $tableSource = $tableLines -join "`n"
    [IO.File]::WriteAllText($longPath, $longSource, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($tablePath, $tableSource, [Text.UTF8Encoding]::new($false))

    $longLeftEdgeOffset = -1
    $longVerticalOffset = -1
    $tableVerticalOffset = -1
    $tableLeftEdgeOffset = -1
    $restoredLongLeftEdgeOffset = -1
    $restoredTableVerticalOffset = -1
    $restoredTableLeftEdgeOffset = -1
    $longHorizontalScrollPosition = -1
    $tableHorizontalScrollPosition = -1
    $longEditorSourceRestored = $false
    $tableEditorContentRestored = $false
    $sessionActiveIndex = -1
    $restoredActiveIndex = -1
    $sessionLongVerticalOffset = -1
    $sessionLongHorizontalOffset = -1
    $sessionTableVerticalOffset = -1
    $sessionTableHorizontalOffset = -1
    $sessionDocumentPaths = @()

    $child = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $longPath) -PassThru
    $script:readbackProcesses.Add($child)
    $main = Wait-ProcessWindow $child
    $tabs = Wait-Control $main 101 'SysTabControl32'
    $editor = Wait-WorkspaceDocumentReady $main $child 'long-horizontal.md' $Directory
    $longAnchor = $longLine.IndexOf('0123456789', [StringComparison]::Ordinal) + 12000
    [void][MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$longAnchor, [IntPtr]$longAnchor)
    [void][MDLiteNative]::SendMessage($editor, 0x00B7, [IntPtr]::Zero, [IntPtr]::Zero) # EM_SCROLLCARET
    $longHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    if ($longHorizontalScrollPosition -le 0) {
        throw "Long-line fixture did not establish horizontal scrolling: horizontal_scroll_position=$longHorizontalScrollPosition, first_visible_line=$(Get-FirstVisibleLine $editor)."
    }
    $longVerticalOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ((Get-FirstVisibleLine $editor) -ne 0 -or $longVerticalOffset -ne 0) {
        throw "Long-line fixture did not remain on its first logical line: first_visible_line=$(Get-FirstVisibleLine $editor), vertical_source_offset=$longVerticalOffset."
    }
    $longLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($longLeftEdgeOffset -le 0 -or $longLeftEdgeOffset -ge $longLine.Length) {
        $longSelectionAfterScroll = Get-SourceSelectionByMessage $main
        $longPositionPacked = [MDLiteNative]::SendMessage(
            $main, 0x8035, [IntPtr]$longAnchor, [IntPtr]::Zero).ToInt64()
        $longPosition = '{0}/{1}' -f ($longPositionPacked -band 0xFFFF),
            (($longPositionPacked -shr 16) -band 0xFFFF)
        throw "Long-line left-edge source anchor is outside the horizontally scrolled line: offset=$longLeftEdgeOffset, horizontal_scroll_position=$longHorizontalScrollPosition, first_visible_line=$(Get-FirstVisibleLine $editor), selection=$($longSelectionAfterScroll.anchor)/$($longSelectionAfterScroll.active), anchor_client_xy=$longPosition."
    }

    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1003, [IntPtr]::Zero)
    $picker = Wait-ProcessClassWindow $child 'MDLite.NativePickerWindow'
    $filter = Wait-Control $picker 100 'Edit'
    $list = Wait-Control $picker 101 'ListBox'
    [void][MDLiteNative]::SendMessage($filter, $WM_SETTEXT, [IntPtr]::Zero, 'table-body.md')
    Start-Sleep -Milliseconds 150
    $candidateCount = [MDLiteNative]::SendMessage(
        $list, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($candidateCount -ne 1) { throw "Expected one session-table Quick Open candidate, got $candidateCount." }
    [void][MDLiteNative]::SendMessage($picker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $child 'MDLite.NativePickerWindow'
    $tabDeadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $tabCount = [MDLiteNative]::SendMessage(
            $tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($tabCount -eq 2) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $tabDeadline)
    if ($tabCount -ne 2) { throw "Expected two session fixture tabs, got $tabCount." }

    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W' 10000 'session-table-open'
    $tableReadinessFlags = 0
    $tableReadinessDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $tableReadinessFlags = [MDLiteNative]::SendMessage(
            $main, $testGetEditorReadinessMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if (($tableReadinessFlags -band 0x107) -eq 0x107) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $tableReadinessDeadline)
    if (($tableReadinessFlags -band 0x107) -ne 0x107) {
        throw "GFM table projection did not initialize before the viewport assertion: readiness=0x$('{0:X}' -f $tableReadinessFlags)."
    }
    $tableAnchor = $tableSource.IndexOf('BODY-VALUE-080', [StringComparison]::Ordinal) + 8
    [void][MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$tableAnchor, [IntPtr]$tableAnchor)
    [void][MDLiteNative]::SendMessage($editor, 0x00B7, [IntPtr]::Zero, [IntPtr]::Zero) # EM_SCROLLCARET
    # The caret establishes a visible body row without scrolling the final empty
    # RichEdit paragraph to the top of the viewport.
    $tableHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    $tableVerticalOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($tableVerticalOffset -lt $tableSource.IndexOf('| row-001', [StringComparison]::Ordinal) -or
        $tableVerticalOffset -ge $tableSource.Length) {
        throw "Table vertical row anchor is outside a body row: offset=$tableVerticalOffset."
    }
    $tableLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($tableLeftEdgeOffset -lt $tableSource.IndexOf('| row-001', [StringComparison]::Ordinal) -or
        $tableLeftEdgeOffset -ge $tableSource.Length) {
        throw "Table left-edge anchor is outside a body row: offset=$tableLeftEdgeOffset."
    }

    Click-TabItem $main $tabs 0
    $suspendedEditorCount = @(Get-ChildClassWindows $main 'RICHEDIT50W').Count
    if ($suspendedEditorCount -ge 2) { throw "Expected at least one suspended editor, found $suspendedEditorCount resident editors." }
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W' 10000 'session-long-tab-restored'
    $longEditorSourceRestored = ((Get-WindowText $editor) -replace "`r`n", "`n") -ceq $longSource
    $returnedLongLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $returnedLongHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    $returnedLongVerticalOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($returnedLongHorizontalScrollPosition -le 0 -or
        $returnedLongLeftEdgeOffset -ne $longLeftEdgeOffset -or
        $returnedLongVerticalOffset -ne $longVerticalOffset) {
        throw "Long-line viewport changed across tab suspension: horizontal_scroll_position=$returnedLongHorizontalScrollPosition, left_edge_expected_actual=$longLeftEdgeOffset/$returnedLongLeftEdgeOffset, vertical_expected_actual=$longVerticalOffset/$returnedLongVerticalOffset."
    }
    Click-TabItem $main $tabs 1
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W' 10000 'session-table-tab-restored'
    $returnedTableVerticalOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $returnedTableLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $returnedTableHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    if ($returnedTableVerticalOffset -lt $tableSource.IndexOf('| row-001', [StringComparison]::Ordinal) -or
        $returnedTableVerticalOffset -ge $tableSource.Length) {
        throw "Table vertical anchor left the body rows across tab suspension: initial=$tableVerticalOffset actual=$returnedTableVerticalOffset."
    }
    if ($returnedTableLeftEdgeOffset -ne $tableLeftEdgeOffset) {
        throw "Table viewport-origin source character changed across tab suspension: expected=$tableLeftEdgeOffset actual=$returnedTableLeftEdgeOffset, vertical=$tableVerticalOffset/$returnedTableVerticalOffset, horizontal_scroll=$returnedTableHorizontalScrollPosition."
    }
    $tableEditorText = (Get-WindowText $editor) -replace "`r`n", "`n"
    $tableEditorContentRestored = $tableEditorText.Contains('BODY-VALUE-001') -and
        $tableEditorText.Contains('BODY-VALUE-120')

    [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $child.WaitForExit(10000)) { throw 'Workspace session fixture did not close cleanly.' }
    if ($child.ExitCode -ne 0) { throw "Workspace session fixture exited with code $($child.ExitCode)." }
    $sessionPath = Join-Path $Directory '.mdlite\.state\session.toml'
    if (-not (Test-Path -LiteralPath $sessionPath -PathType Leaf)) { throw 'Workspace session file was not written.' }
    # Read only this GUID-owned fixture session. Do not inspect any user workspace state.
    $sessionText = [IO.File]::ReadAllText($sessionPath)
    if ($sessionText -notmatch '(?m)^active_index = (\d+)$') { throw 'Fixture session has no active_index.' }
    $sessionActiveIndex = [int]$Matches[1]
    $sessionDocuments = @($sessionText -split '(?m)^\[\[document\]\]\s*$' | Select-Object -Skip 1)
    foreach ($documentText in $sessionDocuments) {
        if ($documentText -notmatch '(?m)^path = "([^"]+)"$') { continue }
        $relativePath = $Matches[1]
        $sessionDocumentPaths += $relativePath
        if ($documentText -notmatch '(?m)^first_visible_source_offset = (\d+)$') {
            throw "Fixture session is missing first_visible_source_offset for '$relativePath'."
        }
        $savedVerticalOffset = [int]$Matches[1]
        if ($relativePath -eq 'long-horizontal.md') {
            $sessionLongVerticalOffset = $savedVerticalOffset
            if ($documentText -notmatch '(?m)^horizontal_left_edge_source_offset = (\d+)$') {
                throw 'Fixture session is missing horizontal_left_edge_source_offset for long-horizontal.md.'
            }
            $sessionLongHorizontalOffset = [int]$Matches[1]
        }
        if ($relativePath -eq 'table-body.md') {
            $sessionTableVerticalOffset = $savedVerticalOffset
            if ($documentText -notmatch '(?m)^horizontal_left_edge_source_offset = (\d+)$') {
                throw 'Table fixture is missing its viewport-origin source character.'
            }
            $sessionTableHorizontalOffset = [int]$Matches[1]
        }
    }
    if ($sessionDocumentPaths.Count -ne 2 -or $sessionActiveIndex -ne 1 -or
        $sessionLongVerticalOffset -ne $longVerticalOffset -or
        $sessionLongHorizontalOffset -ne $longLeftEdgeOffset -or
        $sessionTableVerticalOffset -ne $tableVerticalOffset -or
        $sessionTableHorizontalOffset -ne $tableLeftEdgeOffset) {
        throw "Fixture session axis mismatch: docs=$($sessionDocumentPaths.Count), active=$sessionActiveIndex, long_vertical=$sessionLongVerticalOffset/$longVerticalOffset, long_horizontal=$sessionLongHorizontalOffset/$longLeftEdgeOffset, table_vertical=$sessionTableVerticalOffset/$tableVerticalOffset, table_horizontal=$sessionTableHorizontalOffset/$tableLeftEdgeOffset."
    }

    $child = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $Directory) -PassThru
    $script:readbackProcesses.Add($child)
    $main = Wait-ProcessWindow $child
    $tabs = Wait-Control $main 101 'SysTabControl32'
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $remaining = [int][Math]::Max(1, ($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    $editor = Wait-WorkspaceDocumentReady $main $child ([IO.Path]::GetFileName($sessionDocumentPaths[$sessionActiveIndex])) $Directory $remaining
    do {
        $restoredTabCount = [MDLiteNative]::SendMessage(
            $tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($restoredTabCount -eq 2) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($restoredTabCount -ne 2) { throw "Workspace restart restored $restoredTabCount fixture tabs, expected 2." }
    $restoredActiveIndex = [MDLiteNative]::SendMessage(
        $tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($restoredActiveIndex -ne $sessionActiveIndex) {
        throw "Workspace restart selected tab $restoredActiveIndex, expected saved active index $sessionActiveIndex."
    }
    $remaining = [int][Math]::Floor(($deadline - [DateTime]::UtcNow).TotalMilliseconds)
    if ($remaining -le 0) { throw 'Workspace restart exhausted its existing readiness budget before viewport observation.' }
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W' $remaining 'session-restart-table-active'
    $viewportMatchedInBudget = $false
    do {
        $restoredTableVerticalOffset = [MDLiteNative]::SendMessage(
            $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        $restoredTableHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
        $restoredTableLeftEdgeOffset = [MDLiteNative]::SendMessage(
            $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($restoredTableVerticalOffset -eq $sessionTableVerticalOffset -and $restoredTableLeftEdgeOffset -eq $sessionTableHorizontalOffset) {
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Saved session viewport matched only after the shared restart deadline.' }
            $viewportMatchedInBudget = $true
            break
        }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    if (-not $viewportMatchedInBudget) { throw 'Saved session viewport did not settle within the shared restart deadline.' }
    if ($restoredTableVerticalOffset -lt $tableSource.IndexOf('| row-001', [StringComparison]::Ordinal) -or
        $restoredTableVerticalOffset -ge $tableSource.Length) {
        throw "Restarted table vertical anchor left the body rows: offset=$restoredTableVerticalOffset."
    }
    if ($restoredTableVerticalOffset -ne $sessionTableVerticalOffset) { throw "Restarted table saved vertical source anchor did not settle: expected=$sessionTableVerticalOffset actual=$restoredTableVerticalOffset." }
    $restoredTableHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    $restoredTableLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($restoredTableLeftEdgeOffset -ne $sessionTableHorizontalOffset -or
        ($tableHorizontalScrollPosition -gt 0 -and $restoredTableHorizontalScrollPosition -le 0)) {
        throw "Restarted table viewport-origin mismatch: session=$sessionTableHorizontalOffset actual=$restoredTableLeftEdgeOffset, initial/restored horizontal_scroll=$tableHorizontalScrollPosition/$restoredTableHorizontalScrollPosition, vertical=$sessionTableVerticalOffset/$restoredTableVerticalOffset."
    }
    $tableEditorText = (Get-WindowText $editor) -replace "`r`n", "`n"
    $tableEditorContentRestored = $tableEditorContentRestored -and
        $tableEditorText.Contains('BODY-VALUE-001') -and $tableEditorText.Contains('BODY-VALUE-120')
    if (-not $tableEditorContentRestored) { throw 'Restored table editor is missing expected source body content.' }
    Click-TabItem $main $tabs 0
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W' 10000 'session-restart-long-tab'
    $restoredLongLeftEdgeOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVisibleSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $restoredLongVerticalOffset = [MDLiteNative]::SendMessage(
        $main, $testGetVerticalSourceOffsetMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $restoredLongHorizontalScrollPosition = [MDLiteNative]::GetScrollPosition($editor, 0)
    $longEditorSourceRestored = $longEditorSourceRestored -and
        ((Get-WindowText $editor) -replace "`r`n", "`n") -ceq $longSource
    if (-not $longEditorSourceRestored) { throw 'Restored long-line editor source does not match its fixture.' }
    if ($restoredLongHorizontalScrollPosition -le 0 -or
        $restoredLongLeftEdgeOffset -ne $sessionLongHorizontalOffset -or
        $restoredLongVerticalOffset -ne $sessionLongVerticalOffset) {
        throw "Restarted long-line viewport mismatch: horizontal_scroll_position=$restoredLongHorizontalScrollPosition, horizontal_session_actual=$sessionLongHorizontalOffset/$restoredLongLeftEdgeOffset, vertical_session_actual=$sessionLongVerticalOffset/$restoredLongVerticalOffset."
    }
    if ([IO.File]::ReadAllText($longPath) -cne $longSource -or
        [IO.File]::ReadAllText($tablePath) -cne $tableSource) {
        throw 'Workspace restart changed one or more fixture source files.'
    }
    [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $child.WaitForExit(10000)) { throw 'Restored workspace fixture did not close cleanly.' }

    # Selection restore can scroll RichEdit to a later caret before applying
    # the saved line-only viewport from a legacy session.
    $legacyDirectory = Join-Path (Split-Path -Parent $Directory) 'legacy-session-scroll'
    [IO.Directory]::CreateDirectory((Join-Path $legacyDirectory '.mdlite\.state')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $legacyDirectory '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $legacySourcePath = Join-Path $legacyDirectory 'legacy-scroll.md'
    $legacyLines = @('# Legacy session viewport fixture') + @(
        1..180 | ForEach-Object { 'LEGACY-LINE-{0:D3} viewport restoration' -f $_ })
    $legacySource = $legacyLines -join "`n"
    [IO.File]::WriteAllText($legacySourcePath, $legacySource, [Text.UTF8Encoding]::new($false))
    $legacyPathToml = 'legacy-scroll.md'
    $legacySelection = $legacySource.IndexOf('LEGACY-LINE-150', [StringComparison]::Ordinal) + 8
    $legacySession = @"
schema_version = 2
active_index = 0
main_x = 80
main_y = 80
main_width = 1100
main_height = 760

[[document]]
path = "$legacyPathToml"
selection_begin = $legacySelection
selection_end = $legacySelection
first_visible_line = 12
compact = false
x = 80
y = 80
width = 720
height = 520
"@
    [IO.File]::WriteAllText((Join-Path $legacyDirectory '.mdlite\.state\session.toml'),
        $legacySession.Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
    $legacyChild = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $legacyDirectory) -PassThru
    $script:readbackProcesses.Add($legacyChild)
    $legacyMain = Wait-ProcessWindow $legacyChild
    $legacyTabControl = Wait-Control $legacyMain 101 'SysTabControl32'
    try {
        $legacyEditor = Wait-VisibleControl $legacyMain 102 'RICHEDIT50W' 10000 'legacy-session-startup'
    } catch {
        $legacyChild.Refresh()
        $legacyTabCount = [MDLiteNative]::SendMessage(
            $legacyTabControl, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        $legacyEditors = @(Get-ChildClassWindows $legacyMain 'RICHEDIT50W')
        $legacyVisibleEditorCount = @($legacyEditors | Where-Object {
            [MDLiteNative]::IsWindowVisible($_)
        }).Count
        throw "Legacy session editor not visible: exited=$($legacyChild.HasExited), exit_code=$(if ($legacyChild.HasExited) { $legacyChild.ExitCode } else { 'running' }), tabs=$legacyTabCount, rich_edit_controls=$($legacyEditors.Count), visible_rich_edit_controls=$legacyVisibleEditorCount, main_title='$(Get-WindowText $legacyMain)', detail=$($_.Exception.Message)."
    }
    $legacyRestoredFirstLine = Get-FirstVisibleLine $legacyEditor
    $legacyRestoredSelection = Get-SourceSelectionByMessage $legacyMain
    $legacyRestorePass = $legacyRestoredFirstLine -eq 12 -and
        $legacyRestoredSelection.anchor -eq $legacySelection -and
        $legacyRestoredSelection.active -eq $legacySelection
    [void][MDLiteNative]::PostMessage($legacyMain, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $legacyChild.WaitForExit(10000)) { throw 'Legacy session fixture did not close cleanly.' }
    if (-not $legacyRestorePass) {
        throw "Legacy session viewport restore mismatch: visible=$legacyRestoredFirstLine expected=12, selection=$($legacyRestoredSelection.anchor)/$($legacyRestoredSelection.active) expected=$legacySelection."
    }

    return [ordered]@{
        pass = $true
        fixture = 'GUID-owned synthetic workspace with a long horizontal source line, GFM table body rows, and legacy line-only session state'
        saved_active_index = $sessionActiveIndex
        restored_active_index = $restoredActiveIndex
        saved_document_paths = $sessionDocumentPaths
        long_horizontal_scroll_position_before_suspend = $longHorizontalScrollPosition
        long_horizontal_left_edge_source_offset = $sessionLongHorizontalOffset
        restored_long_horizontal_left_edge_source_offset = $restoredLongLeftEdgeOffset
        long_vertical_source_offset = $sessionLongVerticalOffset
        restored_long_vertical_source_offset = $restoredLongVerticalOffset
        table_vertical_source_offset = $sessionTableVerticalOffset
        restored_table_vertical_source_offset = $restoredTableVerticalOffset
        table_horizontal_scroll_position_before_suspend = $tableHorizontalScrollPosition
        table_horizontal_left_edge_source_offset = $sessionTableHorizontalOffset
        restored_table_horizontal_left_edge_source_offset = $restoredTableLeftEdgeOffset
        table_projection_readiness_flags = ('0x{0:X}' -f $tableReadinessFlags)
        legacy_saved_first_visible_line = 12
        legacy_restored_first_visible_line = $legacyRestoredFirstLine
        legacy_restored_selection = $legacyRestoredSelection
        legacy_viewport_restore_pass = $legacyRestorePass
        long_source_restored = $longEditorSourceRestored
        table_editor_content_restored = $tableEditorContentRestored
        source_files_unchanged = $true
        workspace_document_readiness_traces = @($script:workspaceDocumentReadinessTraces.ToArray() | Select-Object -Skip $readinessTraceStart)
        suspended_editor_count_after_tab_switch = $suspendedEditorCount
        synthetic_input = 'Cross-process Win32 messages and a test-only visible-source-offset probe; no physical input.'
        evidence_boundary = 'Automated native observation only; Human visual, physical input, and IME acceptance remain unknown.'
    }
}

function Start-ReadbackFixture([string]$Name, [AllowEmptyString()][string]$InitialText = $null) {
    $directory = Join-Path $runRoot ("readback-" + $Name)
    [IO.Directory]::CreateDirectory((Join-Path $directory '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $directory '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $source = Join-Path $directory 'fixture.md'
    $initial = if ($null -ne $InitialText) { $InitialText } else { "E21 $Name baseline" }
    [IO.File]::WriteAllText($source, $initial, [Text.UTF8Encoding]::new($false))
    $fixtureProcess = Start-Process -FilePath $executable -ArgumentList @($source) -PassThru
    $script:readbackProcesses.Add($fixtureProcess)
    $main = Wait-ProcessWindow $fixtureProcess
    $editor = Wait-WorkspaceDocumentReady $main $fixtureProcess 'fixture.md' $directory
    return [pscustomobject]@{
        Process = $fixtureProcess
        Main = $main
        Editor = $editor
        Source = $source
        Initial = $initial
        Directory = $directory
    }
}

function Append-PendingEditorText($Fixture, [string]$Text) {
    [void][MDLiteNative]::SendMessage(
        $Fixture.Editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    [void][MDLiteNative]::SendMessage(
        $Fixture.Editor, $EM_REPLACESEL, [IntPtr]1, $Text)
}

function Inject-NextEditorReadbackFailure($Fixture) {
    [void][MDLiteNative]::SendMessage(
        $Fixture.Main, $testFailNextEditorReadbackMessage, [IntPtr]::Zero, [IntPtr]::Zero)
}

function Set-NativeProjectionFailureStages($Fixture, [int]$Mask) {
    return [MDLiteNative]::SendMessage(
        $Fixture.Main, $testSetNativeProjectionFailureStagesMessage,
        [IntPtr]$Mask, [IntPtr]::Zero).ToInt32() -eq 1
}

function Get-EditorReadinessFlags([IntPtr]$Main) {
    return [MDLiteNative]::SendMessage(
        $Main, $testGetEditorReadinessMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
}

function Get-CharFormatAtSourceRange([IntPtr]$Main, [int]$SourceBegin, [int]$SourceEnd) {
    $packed = [MDLiteNative]::SendMessage(
        $Main, $testGetCharFormatAtSourceRangeMessage,
        [IntPtr]$SourceBegin, [IntPtr]$SourceEnd).ToInt64()
    return [pscustomobject]@{
        mask = [uint32](($packed -shr 32) -band 0xFFFFFFFFL)
        effects = [uint32]($packed -band 0xFFFFFFFFL)
    }
}

function Stop-ReadbackFixture($Fixture) {
    if ($Fixture -and -not $Fixture.Process.HasExited) {
        $Fixture.Process.Kill()
        [void]$Fixture.Process.WaitForExit(3000)
    }
}

function Invoke-UntitledLifecycleAcceptance() {
    $fixture = Start-ReadbackFixture 'e01-multiple-untitled-save-as' ''
    $checks = [ordered]@{
        multiple_untitled_tabs_created = $false
        first_untitled_buffer_restored = $false
        second_untitled_buffer_restored = $false
        recovery_snapshots_preserve_both_buffers = $false
        untitled_documents_do_not_create_workspace_files = $false
        save_opens_save_as_for_untitled = $false
        save_as_cancel_keeps_buffer_and_recovery = $false
        editing_resumes_after_save_cancel = $false
        save_as_retry_writes_exact_pending_buffer = $false
        saved_tab_remains_independent_of_other_untitled = $false
        recovery_snapshot_removed_only_for_saved_tab = $false
    }
    $details = [ordered]@{}
    try {
        $tabs = Wait-Control $fixture.Main 101 'SysTabControl32'
        [void][MDLiteNative]::SendMessage($fixture.Main, $WM_COMMAND, [IntPtr]1001, [IntPtr]::Zero)
        $tabDeadline = [DateTime]::UtcNow.AddSeconds(3)
        $tabCount = 0
        do {
            $tabCount = [MDLiteNative]::SendMessage(
                $tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            if ($tabCount -eq 2) { break }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $tabDeadline)
        $firstNewEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $firstDraft = 'first untitled draft'
        Append-PendingEditorText ([pscustomobject]@{ Editor = $firstNewEditor }) $firstDraft
        Start-Sleep -Milliseconds 150
        $details.first_buffer_after_edit = (Get-WindowText $firstNewEditor) -replace "`r`n", "`n"
        $details.selected_tab_after_first_edit =
            [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()

        [void][MDLiteNative]::SendMessage($fixture.Main, $WM_COMMAND, [IntPtr]1001, [IntPtr]::Zero)
        $tabDeadline = [DateTime]::UtcNow.AddSeconds(3)
        do {
            $tabCount = [MDLiteNative]::SendMessage(
                $tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            if ($tabCount -eq 3) { break }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $tabDeadline)
        $checks.multiple_untitled_tabs_created = $tabCount -eq 3

        $editor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $details.second_buffer_after_edit = (Get-WindowText $editor) -replace "`r`n", "`n"
        $secondDraft = 'second untitled draft'
        $editor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        Append-PendingEditorText ([pscustomobject]@{ Editor = $editor }) $secondDraft
        Start-Sleep -Milliseconds 150
        $details.second_buffer_after_edit = (Get-WindowText $editor) -replace "`r`n", "`n"
        $details.selected_tab_after_second_edit =
            [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()

        Click-TabItem $fixture.Main $tabs 1
        $firstEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $details.first_buffer_after_switch = (Get-WindowText $firstEditor) -replace "`r`n", "`n"
        $details.selected_tab_after_first_switch =
            [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        $checks.first_untitled_buffer_restored =
            ((Get-WindowText $firstEditor) -replace "`r`n", "`n") -ceq $firstDraft
        Click-TabItem $fixture.Main $tabs 2
        $secondEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $details.second_buffer_after_switch = (Get-WindowText $secondEditor) -replace "`r`n", "`n"
        $details.selected_tab_after_second_switch =
            [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        $checks.second_untitled_buffer_restored =
            ((Get-WindowText $secondEditor) -replace "`r`n", "`n") -ceq $secondDraft

        $recoveryRoot = Join-Path $fixture.Directory '.mdlite\.state\recovery'
        $recoveryDeadline = [DateTime]::UtcNow.AddSeconds(9)
        $recoveryFiles = @()
        do {
            $recoveryFiles = @(Get-ChildItem -LiteralPath $recoveryRoot -File -ErrorAction SilentlyContinue)
            if ($recoveryFiles.Count -ge 2) { break }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $recoveryDeadline)
        $recoverySnapshotEvidence = @($recoveryFiles | ForEach-Object {
            [pscustomobject]@{ path = $_.FullName; content = [IO.File]::ReadAllText($_.FullName) }
        })
        $firstDraftSnapshots = @($recoverySnapshotEvidence | Where-Object { $_.content.Contains($firstDraft) })
        $secondDraftSnapshots = @($recoverySnapshotEvidence | Where-Object { $_.content.Contains($secondDraft) })
        $checks.recovery_snapshots_preserve_both_buffers =
            $recoverySnapshotEvidence.Count -eq 2 -and
            $firstDraftSnapshots.Count -eq 1 -and $secondDraftSnapshots.Count -eq 1 -and
            $firstDraftSnapshots[0].path -cne $secondDraftSnapshots[0].path
        $normalMarkdownFiles = @(Get-ChildItem -LiteralPath $fixture.Directory -Filter '*.md' -File -Recurse |
            Where-Object { -not $_.FullName.StartsWith((Join-Path $fixture.Directory '.mdlite'),
                [StringComparison]::OrdinalIgnoreCase) })
        $checks.untitled_documents_do_not_create_workspace_files =
            $normalMarkdownFiles.Count -eq 1 -and $normalMarkdownFiles[0].FullName -eq $fixture.Source
        $details.recovery_snapshots_before_save = $recoveryFiles.Count
        $details.recovery_snapshot_contents_before_save = @($recoverySnapshotEvidence | ForEach-Object {
            [ordered]@{ path = $_.path; contains_first_draft = $_.content.Contains($firstDraft); contains_second_draft = $_.content.Contains($secondDraft) }
        })
        $details.normal_markdown_files_before_save = @($normalMarkdownFiles | ForEach-Object FullName)
        $details.tab_count = $tabCount

        [void][MDLiteNative]::PostMessage($fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $cancelDialog = Wait-ProcessClassWindow $fixture.Process '#32770' 5000
        $checks.save_opens_save_as_for_untitled = $cancelDialog -ne [IntPtr]::Zero
        if ($cancelDialog -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage($cancelDialog, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
            Wait-ProcessClassWindowGone $fixture.Process '#32770' 5000
        }
        $secondEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $cancelledText = (Get-WindowText $secondEditor) -replace "`r`n", "`n"
        $details.buffer_after_save_cancel = $cancelledText
        $recoveryAfterCancel = @(Get-ChildItem -LiteralPath $recoveryRoot -File -ErrorAction SilentlyContinue)
        $recoveryAfterCancelEvidence = @($recoveryAfterCancel | ForEach-Object {
            [pscustomobject]@{ path = $_.FullName; content = [IO.File]::ReadAllText($_.FullName) }
        })
        $firstDraftAfterCancel = @($recoveryAfterCancelEvidence | Where-Object { $_.content.Contains($firstDraft) })
        $secondDraftAfterCancel = @($recoveryAfterCancelEvidence | Where-Object { $_.content.Contains($secondDraft) })
        $checks.save_as_cancel_keeps_buffer_and_recovery =
            $cancelDialog -ne [IntPtr]::Zero -and $cancelledText -ceq $secondDraft -and
            $recoveryAfterCancelEvidence.Count -eq 2 -and
            $firstDraftAfterCancel.Count -eq 1 -and $secondDraftAfterCancel.Count -eq 1
        $resumeText = ' resumed'
        Append-PendingEditorText ([pscustomobject]@{ Editor = $secondEditor }) $resumeText
        Start-Sleep -Milliseconds 150
        $expectedSavedText = $secondDraft + $resumeText
        $secondEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $checks.editing_resumes_after_save_cancel =
            ((Get-WindowText $secondEditor) -replace "`r`n", "`n") -ceq $expectedSavedText
        $details.buffer_after_resume = (Get-WindowText $secondEditor) -replace "`r`n", "`n"

        $target = Join-Path $fixture.Directory 'untitled.md'
        [void][MDLiteNative]::PostMessage($fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        # Preserve the previous 5s dialog + 5s filename budget as one readiness deadline.
        $saveReady = Wait-SaveAsControlsReady $fixture.Process 10000
        $retryDialog = $saveReady.dialog
        $fileNameEdit = $saveReady.edit
        $saveButton = $saveReady.save
        $requestedPath = [IO.Path]::GetFullPath($target)
        if (Test-Path -LiteralPath $requestedPath) { throw 'Save retry target already exists in this fresh fixture.' }
        $setName = [MDLiteNative]::SendMessage($fileNameEdit, $WM_SETTEXT, [IntPtr]::Zero, $requestedPath).ToInt64()
        $readName = Get-WindowText $fileNameEdit
        $details.save_as_file_name_control = '1001:Edit'
        $details.save_as_file_name_value = $readName
        $details.save_as_requested_path = $requestedPath
        $details.save_button_found = $true
        $details.save_as_host_visible = [MDLiteNative]::IsWindowVisible($retryDialog)
        if ($setName -eq 0 -or $readName -cne $requestedPath) { throw 'Save retry filename host did not accept/read back the exact absolute fixture path.' }
        [void][MDLiteNative]::SendMessage($saveButton, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
        Wait-ProcessClassWindowGone $fixture.Process '#32770' 5000
        $details.save_dialog_closed_after_click = -not [MDLiteNative]::IsWindow($retryDialog)
        $checks.save_as_retry_writes_exact_pending_buffer =
            $retryDialog -ne [IntPtr]::Zero -and $fileNameEdit -ne [IntPtr]::Zero -and
            (Test-Path -LiteralPath $target -PathType Leaf) -and
            [IO.File]::ReadAllText($target) -ceq $expectedSavedText
        $details.default_save_as_name = 'untitled.md'
        $details.saved_file_sha256 = if (Test-Path -LiteralPath $target -PathType Leaf) {
            (Get-FileHash -Algorithm SHA256 -LiteralPath $target).Hash.ToLowerInvariant()
        } else { $null }

        Click-TabItem $fixture.Main $tabs 1
        $firstEditor = Wait-VisibleControl $fixture.Main 102 'RICHEDIT50W'
        $checks.saved_tab_remains_independent_of_other_untitled =
            ((Get-WindowText $firstEditor) -replace "`r`n", "`n") -ceq $firstDraft
        $recoveryAfterSave = @(Get-ChildItem -LiteralPath $recoveryRoot -File -ErrorAction SilentlyContinue)
        $recoveryAfterSaveEvidence = @($recoveryAfterSave | ForEach-Object {
            [pscustomobject]@{ path = $_.FullName; content = [IO.File]::ReadAllText($_.FullName) }
        })
        $remainingFirstDraftSnapshots = @($recoveryAfterSaveEvidence | Where-Object { $_.content.Contains($firstDraft) })
        $remainingSecondDraftSnapshots = @($recoveryAfterSaveEvidence | Where-Object { $_.content.Contains($secondDraft) })
        $checks.recovery_snapshot_removed_only_for_saved_tab =
            $recoveryAfterSaveEvidence.Count -eq 1 -and
            $remainingFirstDraftSnapshots.Count -eq 1 -and $remainingSecondDraftSnapshots.Count -eq 0 -and
            $recoveryAfterSaveEvidence[0].path -ceq $firstDraftSnapshots[0].path
        $details.recovery_snapshots_after_save = $recoveryAfterSave.Count
        $details.recovery_snapshot_contents_after_save = @($recoveryAfterSaveEvidence | ForEach-Object {
            [ordered]@{ path = $_.path; contains_first_draft = $_.content.Contains($firstDraft); contains_second_draft = $_.content.Contains($secondDraft) }
        })
    } catch {
        $details.error = $_.Exception.Message
    } finally {
        Stop-ReadbackFixture $fixture
    }
    return [pscustomobject][ordered]@{
        pass = @($checks.Values | Where-Object { -not $_ }).Count -eq 0
        checks = $checks
        details = $details
        synthetic_input = 'Cross-process Win32 tab and Save commands plus WM_SETTEXT in the common Save dialog; no physical input.'
        evidence_boundary = 'Automated native route only; physical keyboard, IME, and Human interaction remain unobserved.'
    }
}

function Invoke-GitActionResponsiveness([string]$Directory) {
    $checks = [ordered]@{
        trusted_fixture_lists_untracked_file = $false
        selected_file_can_start_stage_action = $false
        stage_command_returns_before_git_work_finishes = $false
        editor_input_is_processed_during_git_work = $false
        compact_editor_remains_enabled_during_git_work = $false
        recovery_snapshot_is_written_during_git_work = $false
        competing_commit_is_blocked_during_stage = $false
        stage_finishes_and_updates_the_index = $false
    }
    $details = [ordered]@{}
    $child = $null
    $previousDelay = $env:MDLITE_TEST_GIT_ACTION_DELAY_MS
    try {
        $gitCommand = Get-Command git.exe -ErrorAction Stop
        $gitExe = $gitCommand.Source
        [IO.Directory]::CreateDirectory($Directory) | Out-Null
        [IO.Directory]::CreateDirectory((Join-Path $Directory '.mdlite')) | Out-Null
        $newLine = [Environment]::NewLine
        $settings = 'schema_version = 1' + $newLine + 'auto_save = false' + $newLine
        [IO.File]::WriteAllText((Join-Path $Directory '.mdlite\settings.toml'),
            $settings, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $Directory '.gitignore'), '.mdlite/' + $newLine, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $Directory 'README.md'), 'E20 Git fixture' + $newLine, [Text.UTF8Encoding]::new($false))
        $output = @(& $gitExe -C $Directory init -b main 2>&1)
        if ($LASTEXITCODE -ne 0) { throw "Temporary Git initialization failed: $($output -join ' ')" }
        $output = @(& $gitExe -C $Directory add -- .gitignore README.md 2>&1)
        if ($LASTEXITCODE -ne 0) { throw "Temporary Git seed stage failed: $($output -join ' ')" }
        $commitArgs = @('-c', 'user.name=MDLite E20 Test', '-c', 'user.email=e20@example.invalid', 'commit', '-m', 'seed')
        $output = @(& $gitExe -C $Directory @commitArgs 2>&1)
        if ($LASTEXITCODE -ne 0) { throw "Temporary Git seed commit failed: $($output -join ' ')" }
        $seedHead = @(& $gitExe -C $Directory rev-parse HEAD 2>&1)[0].ToString().Trim()
        $pendingFile = Join-Path $Directory 'pending.md'
        [IO.File]::WriteAllText($pendingFile, 'E20 delayed stage fixture' + $newLine, [Text.UTF8Encoding]::new($false))

        $env:MDLITE_TEST_GIT_ACTION_DELAY_MS = '7000'
        $fixtureReadme=Join-Path $Directory 'README.md'
        $child = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $fixtureReadme) -PassThru
        $env:MDLITE_TEST_GIT_ACTION_DELAY_MS = $previousDelay
        $main = Wait-ProcessWindow $child
        $gitFiles = Wait-Control $main 0 'SysListView32'
        # Created controls precede metadata initialization. The existing tracked
        # README makes normal positive document readiness observable before trust.
        $workspaceReadyStarted=[DateTime]::UtcNow
        $workspaceReadyDeadline=$workspaceReadyStarted.AddSeconds(10)
        $workspaceReady=$false;$readyFlags=0;$readySourceLength=0;$readyCaption='';$metadataReady=$false
        do {
            $readyFlags=[MDLiteNative]::SendMessage($main,$testGetEditorReadinessMessage,[IntPtr]::Zero,[IntPtr]::Zero).ToInt32()
            $readySourceLength=[MDLiteNative]::SendMessage($main,0x8045,[IntPtr]4,[IntPtr]::Zero).ToInt64()
            $readyCaption=Get-WindowText $main
            $metadataReady=(Test-Path -LiteralPath (Join-Path $Directory '.mdlite/workspace.toml') -PathType Leaf) -and
                (Test-Path -LiteralPath (Join-Path $Directory '.mdlite/profiles.toml') -PathType Leaf)
            if([DateTime]::UtcNow -lt $workspaceReadyDeadline -and ($readyFlags -band 5) -eq 5 -and $readySourceLength -gt 0 -and $metadataReady -and
                $readyCaption.StartsWith('MDLite — README.md — ',[StringComparison]::Ordinal)){$workspaceReady=$true;break}
            Start-Sleep -Milliseconds 40
        }while([DateTime]::UtcNow -lt $workspaceReadyDeadline)
        $details.workspace_ready_before_trust=[ordered]@{pid=$child.Id;main_hwnd=$main.ToInt64();ready=$workspaceReady;flags=$readyFlags;source_length=$readySourceLength;metadata_initialized=$metadataReady;caption=$readyCaption;elapsed_ms=[Math]::Round(([DateTime]::UtcNow-$workspaceReadyStarted).TotalMilliseconds,3);deadline_ms=10000;fixture_document=$fixtureReadme}
        if(-not $workspaceReady){throw 'E20 workspace/document did not reach positive native readiness before trust; trust was not dispatched.'}
        $trustResult = [MDLiteNative]::SendMessage(
            $main, $testTrustWorkspaceForGitMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($trustResult -eq 1) {
            $gitFiles = Wait-VisibleControl $main 0 'SysListView32' 10000 'E20-git-files'
        }
        $listDeadline = [DateTime]::UtcNow.AddSeconds(8)
        $itemCount = 0
        do {
            $itemCount = [MDLiteNative]::SendMessage(
                $gitFiles, 0x1004, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            if ($itemCount -eq 1) { break }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $listDeadline)
        $checks.trusted_fixture_lists_untracked_file = $trustResult -eq 1 -and $itemCount -eq 1
        $details.trust_test_route_accepted = $trustResult -eq 1
        $details.git_file_count_before_action = $itemCount

        [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1001, [IntPtr]::Zero)
        $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
        $draft = 'E20 editor input baseline'
        Send-Characters $editor $draft
        [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1030, [IntPtr]::Zero)
        $compactWindow = Wait-ProcessClassWindow $child 'MDLite.CompactWindow'
        $compactEditor = Wait-VisibleControl $compactWindow 102 'RICHEDIT50W' 5000 'E20-compact-editor'
        $rowPoint = [IntPtr]([long]((30 -band 0xffff) -shl 16) -bor (75 -band 0xffff))
        $rowDownPosted = [MDLiteNative]::PostMessage($gitFiles, 0x0201, [IntPtr]1, $rowPoint)
        $rowUpPosted = [MDLiteNative]::PostMessage($gitFiles, 0x0202, [IntPtr]::Zero, $rowPoint)
        $selectionDeadline = [DateTime]::UtcNow.AddSeconds(2)
        $selectedCount = 0
        do {
            Start-Sleep -Milliseconds 50
            $selectedCount = [MDLiteNative]::SendMessage(
                $gitFiles, 0x1032, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            if ($selectedCount -eq 1) { break }
        } while ([DateTime]::UtcNow -lt $selectionDeadline)
        $stageButton = Wait-Control $main 1036 'Button'
        $stageButtonEnabled = [MDLiteNative]::IsWindowEnabled($stageButton)
        $checks.selected_file_can_start_stage_action = $selectedCount -eq 1 -and $stageButtonEnabled
        $details.list_row_selected = $selectedCount -eq 1
        $details.list_row_mouse_messages_posted = $rowDownPosted -and $rowUpPosted
        $details.stage_button_enabled = $stageButtonEnabled

        $commandClock = [Diagnostics.Stopwatch]::StartNew()
        $stagePosted = [MDLiteNative]::PostMessage(
            $main, $WM_COMMAND, [IntPtr]1036, $stageButton)
        $commandClock.Stop()
        $details.stage_command_delivery_ms = [Math]::Round($commandClock.Elapsed.TotalMilliseconds, 3)
        $actionActive = 0
        $actionQueryResponded = $false
        $actionDeadline = [DateTime]::UtcNow.AddSeconds(5)
        do {
            $actionResult = [IntPtr]::Zero
            $queryCall = [MDLiteNative]::SendMessageTimeoutW(
                $main, $testGetGitActionActiveMessage, [IntPtr]::Zero, [IntPtr]::Zero,
                [uint32]0x0002, [uint32]750, [ref]$actionResult)
            if ($queryCall -ne [IntPtr]::Zero) {
                $actionQueryResponded = $true
                $actionActive = $actionResult.ToInt32()
                if ($actionActive -eq 1) { break }
            }
            Start-Sleep -Milliseconds 40
        } while ([DateTime]::UtcNow -lt $actionDeadline)
        $details.action_active_query_responded = $actionQueryResponded
        $details.action_active_during_delay = $actionActive -eq 1
        $compactEnabledDuringAction = [MDLiteNative]::IsWindowEnabled($compactWindow)
        $commitCommandPosted = [MDLiteNative]::PostMessage(
            $main, $WM_COMMAND, [IntPtr]1038, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 100
        $unexpectedCommitDialog = Get-ProcessClassWindow $child 'MDLite.NativeFormWindow'
        if ($unexpectedCommitDialog -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage(
                $unexpectedCommitDialog, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
            Wait-ProcessClassWindowGone $child 'MDLite.NativeFormWindow' 2000
        }
        $headDuringAction = @(& $gitExe -C $Directory rev-parse HEAD 2>&1)[0].ToString().Trim()
        $actionResult = [IntPtr]::Zero
        $actionQueryAfterCommit = [MDLiteNative]::SendMessageTimeoutW(
            $main, $testGetGitActionActiveMessage, [IntPtr]::Zero, [IntPtr]::Zero,
            [uint32]0x0002, [uint32]750, [ref]$actionResult)
        $actionStillActiveAfterCommit = $actionQueryAfterCommit -ne [IntPtr]::Zero -and $actionResult.ToInt32() -eq 1
        $checks.competing_commit_is_blocked_during_stage = $commitCommandPosted -and
            $unexpectedCommitDialog -eq [IntPtr]::Zero -and
            $headDuringAction -ceq $seedHead -and $actionStillActiveAfterCommit
        $details.competing_commit_message_posted = $commitCommandPosted
        $details.competing_commit_dialog_shown = $unexpectedCommitDialog -ne [IntPtr]::Zero
        $details.seed_head_during_stage = $headDuringAction
        $details.compact_window_enabled_during_action = $compactEnabledDuringAction
        Start-Sleep -Milliseconds 120
        $inputClock = [Diagnostics.Stopwatch]::StartNew()
        Send-Characters $compactEditor ' responsive'
        $inputClock.Stop()
        $editedBuffer = Get-WindowText $compactEditor
        $child.Refresh()
        $details.editor_message_return_ms = [Math]::Round($inputClock.Elapsed.TotalMilliseconds, 3)
        $details.process_responding_during_action = $child.Responding
        $details.editor_buffer_during_action = $editedBuffer

        $duringStatus = @(& $gitExe -C $Directory status --porcelain 2>&1)
        $duringExitCode = $LASTEXITCODE
        $duringUntracked = $duringExitCode -eq 0 -and
            @($duringStatus | Where-Object { $_ -match '^\?\? pending\.md$' }).Count -eq 1
        $checks.stage_command_returns_before_git_work_finishes =
            $stagePosted -and $commandClock.Elapsed.TotalMilliseconds -lt 750 -and
            $actionActive -eq 1 -and $duringUntracked
        $checks.editor_input_is_processed_during_git_work =
            $actionActive -eq 1 -and
            $inputClock.Elapsed.TotalMilliseconds -lt 750 -and
            $details.process_responding_during_action -and $editedBuffer.EndsWith(' responsive')
        $checks.compact_editor_remains_enabled_during_git_work =
            $compactEnabledDuringAction -and [MDLiteNative]::IsWindowEnabled($compactWindow)
        $details.stage_button_message_posted = $stagePosted
        $details.git_status_during_action = @($duringStatus)
        $details.untracked_entry_remained_during_worker_delay = $duringUntracked

        $recoveryRoot = Join-Path $Directory '.mdlite\.state\recovery'
        $recoveryDeadline = [DateTime]::UtcNow.AddSeconds(9)
        $recoverySnapshot = $null
        $recoveryObservedDuringAction = $false
        $recoveryNeedle = 'E20 editor input baseline responsive'
        do {
            $recoveryFiles = @(Get-ChildItem -LiteralPath $recoveryRoot -Filter '*.md' -File -ErrorAction SilentlyContinue)
            foreach ($candidate in $recoveryFiles) {
                if ([IO.File]::ReadAllText($candidate.FullName).Contains($recoveryNeedle)) {
                    $recoverySnapshot = $candidate.FullName
                    break
                }
            }
            if ($recoverySnapshot) {
                $actionResult = [IntPtr]::Zero
                $queryAtRecovery = [MDLiteNative]::SendMessageTimeoutW(
                    $main, $testGetGitActionActiveMessage, [IntPtr]::Zero, [IntPtr]::Zero,
                    [uint32]0x0002, [uint32]750, [ref]$actionResult)
                $recoveryObservedDuringAction = $queryAtRecovery -ne [IntPtr]::Zero -and
                    $actionResult.ToInt32() -eq 1
                break
            }
            Start-Sleep -Milliseconds 100
        } while ([DateTime]::UtcNow -lt $recoveryDeadline)
        $checks.recovery_snapshot_is_written_during_git_work =
            $recoverySnapshot -and $recoveryObservedDuringAction
        $details.recovery_snapshot_during_action = $recoverySnapshot
        $details.recovery_snapshot_observed_before_git_action_completed = $recoveryObservedDuringAction

        $indexDeadline = [DateTime]::UtcNow.AddSeconds(8)
        $stagedPaths = @()
        do {
            $stagedPaths = @(& $gitExe -C $Directory diff --cached --name-only 2>&1)
            if ($LASTEXITCODE -eq 0 -and $stagedPaths -contains 'pending.md') { break }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $indexDeadline)
        $checks.stage_finishes_and_updates_the_index = $LASTEXITCODE -eq 0 -and
            $stagedPaths -contains 'pending.md'
        $details.staged_paths_after_completion = $stagedPaths
        $details.input_method = 'Cross-process Win32 control messages; test-only worker delay is 7000 ms. No physical input.'
    } catch {
        $details.error = $_.Exception.Message
    } finally {
        $env:MDLITE_TEST_GIT_ACTION_DELAY_MS = $previousDelay
        if ($child) {
            try {
                $child.Refresh()
                if (-not $child.HasExited) {
                    $gitMain = Get-ProcessClassWindow $child 'MDLite.MainWindow'
                    if ($gitMain -ne [IntPtr]::Zero) {
                        [void][MDLiteNative]::PostMessage($gitMain, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
                    }
                    if (-not $child.WaitForExit(3000)) {
                        $child.Kill()
                        [void]$child.WaitForExit(3000)
                    }
                }
                $child.Dispose()
            } catch { }
        }
    }
    return [pscustomobject][ordered]@{
        pass = @($checks.Values | Where-Object { -not $_ }).Count -eq 0
        checks = $checks
        details = $details
        evidence_boundary = 'Automated native message delivery and local temporary Git index state; no physical keyboard or input-queue claim.'
    }
}

function Wait-TableTopologyReadback([IntPtr]$Main, [bool]$ExpectTable) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    $flags = 0
    do {
        $flags = Get-EditorReadinessFlags $Main
        $ready = if ($ExpectTable) {
            ($flags -band 0x107) -eq 0x107
        } else {
            ($flags -band 0xF5) -eq 0x05
        }
        if ($ready) { return [pscustomobject]@{ ready = $true; flags = $flags } }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $deadline)
    return [pscustomobject]@{ ready = $false; flags = $flags }
}

function Invoke-TableTopologyReplacement($Fixture, [string]$CaseName,
    [string]$Replacement, [int]$SelectionStart,
    [int]$SelectionEnd, [string]$ExpectedEditedSource,
    [string[]]$InitialProjectionSentinels, [string[]]$EditedProjectionSentinels,
    [ValidateSet('source-probe', 'select-all')][string]$SelectionMode,
    [ValidateSet('wm-char', 'em-replacesel')][string]$InputMode, [bool]$EditedHasTable) {
    $utf8 = [Text.UTF8Encoding]::new($false)
    $initialBytes = [IO.File]::ReadAllBytes($Fixture.Source)
    $initialBytesBase64 = [Convert]::ToBase64String($initialBytes)
    $editedBytesBase64 = [Convert]::ToBase64String($utf8.GetBytes($ExpectedEditedSource))
    $beforeEditor = (Get-WindowText $Fixture.Editor) -replace "`r`n", "`n"
    $beforeEnabled = [MDLiteNative]::IsWindowEnabled($Fixture.Editor)
    $beforeReadback = Wait-TableTopologyReadback $Fixture.Main $true
    $wholeSourceLength = [MDLiteNative]::SendMessage($Fixture.Main, 0x8045, [IntPtr]4, [IntPtr]::Zero).ToInt32()
    if ($SelectionMode -eq 'select-all') {
        [void][MDLiteNative]::SendMessage(
            $Fixture.Editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr](-1))
        $sourceSelection = Get-EditorSelection $Fixture.Editor
        $wholeSourceSelection = Get-SourceSelectionByMessage $Fixture.Main
        $selectionResult = if (Test-WholeSourceSelection $wholeSourceSelection $wholeSourceLength) { 1 } else { 0 }
    } else {
        $selectionResult = [MDLiteNative]::SendMessage(
            $Fixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]$SelectionStart, [IntPtr]$SelectionEnd).ToInt32()
        $sourceSelection = Get-EditorSelection $Fixture.Editor
        if ($selectionResult -ne 0) {
            [void][MDLiteNative]::SendMessage(
                $Fixture.Editor, $EM_SETSEL, [IntPtr]$sourceSelection.start, [IntPtr]$sourceSelection.end)
        }
    }
    $nativeSelection = Get-EditorSelection $Fixture.Editor
    $nativeSelectionSet = if ($SelectionMode -eq 'select-all') {
        $wholeSourceSelection = Get-SourceSelectionByMessage $Fixture.Main
        $selectionResult -eq 1 -and (Test-WholeSourceSelection $wholeSourceSelection $wholeSourceLength)
    } else {
        $selectionResult -eq 1 -and $nativeSelection.start -eq $sourceSelection.start -and
            $nativeSelection.end -eq $sourceSelection.end -and $nativeSelection.start -ne $nativeSelection.end
    }
    if ($selectionResult -ne 0) {
        # Selection is established by EM_SETSEL for Select All or by the
        # source probe plus EM_SETSEL for a Markdown range. Send through the
        # selected message route so product pre-mutation handling runs.
        if ($InputMode -eq 'wm-char') {
            Send-SelectedCharacters $Fixture.Editor $Replacement
        } else {
            [void][MDLiteNative]::SendMessage(
                $Fixture.Editor, $EM_REPLACESEL, [IntPtr]1, $Replacement)
        }
    }
    $editReadback = Wait-TableTopologyReadback $Fixture.Main $EditedHasTable
    $editEditor = Wait-VisibleControl $Fixture.Main 102 'RICHEDIT50W'
    $editedText = (Get-WindowText $editEditor) -replace "`r`n", "`n"
    $editedProjectionMatches = if ($EditedHasTable) {
        @($EditedProjectionSentinels | Where-Object { -not $editedText.Contains($_) }).Count -eq 0
    } else {
        $editedText -ceq ($ExpectedEditedSource -replace "`r`n", "`n")
    }
    $editedEnabled = [MDLiteNative]::IsWindowEnabled($editEditor)
    $bytesBeforeEditedSave = [Convert]::ToBase64String([IO.File]::ReadAllBytes($Fixture.Source))
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $editedSaveReadback = Wait-TableTopologyReadback $Fixture.Main $EditedHasTable
    $editedSavedBytes = [Convert]::ToBase64String([IO.File]::ReadAllBytes($Fixture.Source))

    [void][MDLiteNative]::SendMessage($editEditor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    $undoReadback = Wait-TableTopologyReadback $Fixture.Main $true
    $undoEditor = Wait-VisibleControl $Fixture.Main 102 'RICHEDIT50W'
    $undoText = (Get-WindowText $undoEditor) -replace "`r`n", "`n"
    $undoEnabled = [MDLiteNative]::IsWindowEnabled($undoEditor)
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $undoSaveReadback = Wait-TableTopologyReadback $Fixture.Main $true
    $undoSavedBytes = [Convert]::ToBase64String([IO.File]::ReadAllBytes($Fixture.Source))

    [void][MDLiteNative]::SendMessage($undoEditor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    $redoReadback = Wait-TableTopologyReadback $Fixture.Main $EditedHasTable
    $redoEditor = Wait-VisibleControl $Fixture.Main 102 'RICHEDIT50W'
    $redoText = (Get-WindowText $redoEditor) -replace "`r`n", "`n"
    $redoEnabled = [MDLiteNative]::IsWindowEnabled($redoEditor)
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $redoSaveReadback = Wait-TableTopologyReadback $Fixture.Main $EditedHasTable
    $redoSavedBytes = [Convert]::ToBase64String([IO.File]::ReadAllBytes($Fixture.Source))

    $checks = [ordered]@{
        initial_projection_present = @($InitialProjectionSentinels | Where-Object {
            -not $beforeEditor.Contains($_)
        }).Count -eq 0
        initial_editor_enabled = [bool]$beforeEnabled
        initial_readback_ready = $beforeReadback.ready
        selection_set = $selectionResult -eq 1
        native_selection_range_set = $nativeSelectionSet
        edited_editor_text_matches = $editedProjectionMatches -and $editReadback.ready
        edited_editor_enabled = [bool]$editedEnabled
        edited_readback_ready = $editReadback.ready
        source_bytes_unchanged_before_save = $bytesBeforeEditedSave -ceq $initialBytesBase64
        edited_save_exact_bytes = $editedSavedBytes -ceq $editedBytesBase64
        undo_editor_text_matches_original = $undoText -ceq $beforeEditor
        undo_editor_enabled = [bool]$undoEnabled
        undo_readback_ready = $undoReadback.ready -and $undoSaveReadback.ready
        undo_save_exact_original_bytes = $undoSavedBytes -ceq $initialBytesBase64
        redo_editor_text_matches_edited = $redoText -ceq $editedText
        redo_editor_enabled = [bool]$redoEnabled
        redo_readback_ready = $redoReadback.ready -and $redoSaveReadback.ready
        redo_save_exact_edited_bytes = $redoSavedBytes -ceq $editedBytesBase64
    }
    return [pscustomobject][ordered]@{
        pass = @($checks.Values | Where-Object { -not $_ }).Count -eq 0
        case = $CaseName
        input_route = if ($SelectionMode -eq 'select-all') {
            'Synthetic test-only silent route: direct RichEdit EM_SETSEL(0,-1) and one Japanese WM_CHAR; no physical keyboard or IME coverage.'
        } elseif ($InputMode -eq 'wm-char') {
            'Synthetic test-only silent route: source-selection probe, RichEdit EM_SETSEL, and WM_CHAR; one Japanese character; no physical keyboard or IME coverage.'
        } else {
            'Synthetic test-only silent route: source-selection probe, RichEdit EM_SETSEL, and one atomic EM_REPLACESEL; no physical paste, keyboard, or IME coverage.'
        }
        evidence_boundary = 'Automated native editor and file-byte observation only; physical keyboard and IME behavior remain unobserved.'
        selection_start = $SelectionStart
        selection_end = $SelectionEnd
        selection_result = $selectionResult
        native_selection_before_input = $nativeSelection
        source_selection_before_input = if ($SelectionMode -eq 'select-all') { $wholeSourceSelection } else { $null }
        whole_source_length = $wholeSourceLength
        editor_text_before = $beforeEditor
        editor_text_after_edit = $editedText
        editor_enabled_before = [bool]$beforeEnabled
        editor_enabled_after_edit = [bool]$editedEnabled
        readiness_flags = [ordered]@{
            before = ('0x{0:X}' -f $beforeReadback.flags)
            after_edit = ('0x{0:X}' -f $editReadback.flags)
            after_edited_save = ('0x{0:X}' -f $editedSaveReadback.flags)
            after_undo = ('0x{0:X}' -f $undoReadback.flags)
            after_undo_save = ('0x{0:X}' -f $undoSaveReadback.flags)
            after_redo = ('0x{0:X}' -f $redoReadback.flags)
            after_redo_save = ('0x{0:X}' -f $redoSaveReadback.flags)
        }
        source_bytes_base64 = [ordered]@{
            before = $initialBytesBase64
            after_edited_save = $editedSavedBytes
            after_undo_save = $undoSavedBytes
            after_redo_save = $redoSavedBytes
            expected_edited = $editedBytesBase64
        }
        checks = $checks
    }
}

function Get-EditorSelection([IntPtr]$Editor) {
    # Use EM_GETSEL's packed return value for this short cross-process fixture.
    # EM_EXGETSEL requires a CHARRANGE pointer, which is not marshalled across processes.
    $packed = [MDLiteNative]::SendMessage(
        $Editor, $EM_GETSEL, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    return [ordered]@{
        start = [int]($packed -band 0xFFFF)
        end = [int](($packed -shr 16) -band 0xFFFF)
    }
}

function Invoke-CollapsedImageIdentityDeletion([string]$Name, [ValidateSet('first-plus-X', 'X-plus-second')][string]$Target) {
    $fixtureDirectory = Join-Path $runRoot ("readback-" + $Name)
    [IO.Directory]::CreateDirectory($fixtureDirectory) | Out-Null
    [IO.File]::WriteAllBytes((Join-Path $fixtureDirectory 'pixel.png'), [Convert]::FromBase64String(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+Q4j4WQAAAABJRU5ErkJggg=='))
    $firstImage = '![first](pixel.png)'
    $secondImage = '![second](pixel.png)'
    $original = $firstImage + 'X' + $secondImage
    $fixture = Start-ReadbackFixture -Name $Name -InitialText $original
    try {
        $utf8 = [Text.UTF8Encoding]::new($false)
        $originalBytes = [IO.File]::ReadAllBytes($fixture.Source)
        $originalBytesBase64 = [Convert]::ToBase64String($originalBytes)
        $originalProjection = Get-WindowText $fixture.Editor
        $initialReadiness = Wait-TableTopologyReadback $fixture.Main $false
        $selectionStart = if ($Target -eq 'first-plus-X') { 0 } else { $firstImage.Length }
        $selectionEnd = if ($Target -eq 'first-plus-X') { $firstImage.Length + 1 } else { $original.Length }
        $selectionResult = [MDLiteNative]::SendMessage(
            $fixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]$selectionStart, [IntPtr]$selectionEnd).ToInt32()
        $nativeSelection = Get-EditorSelection $fixture.Editor
        $expectedNativeStart = if ($Target -eq 'first-plus-X') { 0 } else { 1 }
        $nativeSelectionMatches = $nativeSelection.start -eq $expectedNativeStart -and
            ($nativeSelection.end - $nativeSelection.start) -eq 2
        $bytesBeforeSaveBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture.Source))

        if ($selectionResult -ne 0) {
            [void][MDLiteNative]::SendMessage($fixture.Editor, $WM_KEYDOWN, [IntPtr]$VK_DELETE, [IntPtr]::Zero)
            [void][MDLiteNative]::SendMessage($fixture.Editor, $WM_KEYUP, [IntPtr]$VK_DELETE, [IntPtr]::Zero)
        }
        $editReadiness = Wait-TableTopologyReadback $fixture.Main $false
        [void][MDLiteNative]::SendMessage($fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $editSaveReadiness = Wait-TableTopologyReadback $fixture.Main $false
        $survivor = if ($Target -eq 'first-plus-X') { $secondImage } else { $firstImage }
        $editedBytesBase64 = [Convert]::ToBase64String($utf8.GetBytes($survivor))
        $editedSavedBytesBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture.Source))

        [void][MDLiteNative]::SendMessage($fixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $undoReadiness = Wait-TableTopologyReadback $fixture.Main $false
        [void][MDLiteNative]::SendMessage($fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $undoSaveReadiness = Wait-TableTopologyReadback $fixture.Main $false
        $undoSavedBytesBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture.Source))

        [void][MDLiteNative]::SendMessage($fixture.Editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redoReadiness = Wait-TableTopologyReadback $fixture.Main $false
        [void][MDLiteNative]::SendMessage($fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $redoSaveReadiness = Wait-TableTopologyReadback $fixture.Main $false
        $redoSavedBytesBase64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($fixture.Source))

        $checks = [ordered]@{
            initial_image_readiness = $initialReadiness.ready
            source_selection_set = $selectionResult -eq 1
            selected_native_object_and_X = $nativeSelectionMatches
            disk_unchanged_before_save = $bytesBeforeSaveBase64 -ceq $originalBytesBase64
            delete_save_exact_surviving_markdown = $editedSavedBytesBase64 -ceq $editedBytesBase64
            delete_save_readiness = $editReadiness.ready -and $editSaveReadiness.ready
            undo_save_exact_original_markdown = $undoSavedBytesBase64 -ceq $originalBytesBase64
            undo_save_readiness = $undoReadiness.ready -and $undoSaveReadiness.ready
            redo_save_exact_surviving_markdown = $redoSavedBytesBase64 -ceq $editedBytesBase64
            redo_save_readiness = $redoReadiness.ready -and $redoSaveReadiness.ready
        }
        return [pscustomobject][ordered]@{
            pass = @($checks.Values | Where-Object { -not $_ }).Count -eq 0
            case = $Target
            fixture = 'Synthetic local workspace with two collapsed images sharing pixel.png and distinct alt/source identity.'
            synthetic_input = 'Cross-process source-selection probe, WM_KEYDOWN/WM_KEYUP Delete, native Undo/Redo, and Save command; no physical input.'
            original_markdown = $original
            expected_surviving_markdown = $survivor
            initial_editor_projection = $originalProjection
            source_bytes_base64 = [ordered]@{
                before = $originalBytesBase64
                before_edit_save = $bytesBeforeSaveBase64
                after_delete_save = $editedSavedBytesBase64
                after_undo_save = $undoSavedBytesBase64
                after_redo_save = $redoSavedBytesBase64
                expected_original = $originalBytesBase64
                expected_survivor = $editedBytesBase64
            }
            readiness_flags = [ordered]@{
                initial = ('0x{0:X}' -f $initialReadiness.flags)
                after_delete = ('0x{0:X}' -f $editReadiness.flags)
                after_delete_save = ('0x{0:X}' -f $editSaveReadiness.flags)
                after_undo = ('0x{0:X}' -f $undoReadiness.flags)
                after_undo_save = ('0x{0:X}' -f $undoSaveReadiness.flags)
                after_redo = ('0x{0:X}' -f $redoReadiness.flags)
                after_redo_save = ('0x{0:X}' -f $redoSaveReadiness.flags)
            }
            checks = $checks
        }
    } finally {
        Stop-ReadbackFixture $fixture
    }
}

function Invoke-TransactionalTablePresentationFailure([string]$Directory) {
    [IO.Directory]::CreateDirectory((Join-Path $Directory '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $Directory '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $previousPath = Join-Path $Directory 'previous.md'
    $targetPath = Join-Path $Directory 'two-tables.md'
    $transactionStageLogPath = Join-Path $Directory 'transactional-table-presentation-stages.log'
    $transactionStageLogArtifactPath = [IO.Path]::ChangeExtension($OutputPath, '.transaction.log')
    $previousSource = "Previous editor remains intact`nDirectional selection anchor`nBuffer sentinel"
    $targetSource = @'
# Two table presentation failure fixture

| First header | First value |
| --- | --- |
| FIRST-CELL-ALPHA | FIRST-CELL-BETA |

Between tables.

| Second header | Second value |
| --- | --- |
| SECOND-CELL-GAMMA | SECOND-CELL-DELTA |
'@
    $targetSourceLf = $targetSource.Replace("`r`n", "`n").Replace("`r", "`n")
    $targetEditedSource = $targetSource.Replace('Between tables.', 'Between tables.X')
    $targetEditedSourceLf = $targetEditedSource.Replace("`r`n", "`n").Replace("`r", "`n")
    [IO.File]::WriteAllText($previousPath, $previousSource, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($targetPath, $targetSource, [Text.UTF8Encoding]::new($false))
    $child = Start-Process -FilePath $executable -ArgumentList ('"{0}"' -f $previousPath) -PassThru
    $script:readbackProcesses.Add($child)
    $main = Wait-ProcessWindow $child
    $tabs = Wait-Control $main 101 'SysTabControl32'
    [void](Wait-VisibleControl $main 102 'RICHEDIT50W')
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1003, [IntPtr]::Zero)
    $picker = Wait-ProcessClassWindow $child 'MDLite.NativePickerWindow'
    $filter = Wait-Control $picker 100 'Edit'
    $list = Wait-Control $picker 101 'ListBox'
    [void][MDLiteNative]::SendMessage($filter, $WM_SETTEXT, [IntPtr]::Zero, 'two-tables.md')
    Start-Sleep -Milliseconds 150
    $candidateCount = [MDLiteNative]::SendMessage(
        $list, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if ($candidateCount -ne 1) { throw "Expected one table failure Quick Open candidate, got $candidateCount." }
    [void][MDLiteNative]::SendMessage($picker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $child 'MDLite.NativePickerWindow'
    $targetReadyDeadline = [DateTime]::UtcNow.AddSeconds(10)
    $targetReadyFlags = 0
    do {
        $targetReadyFlags = [MDLiteNative]::SendMessage(
            $main, $testGetEditorReadinessMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if (($targetReadyFlags -band 0x107) -eq 0x107) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $targetReadyDeadline)
    if (($targetReadyFlags -band 0x107) -ne 0x107) { throw "Initial two-table projection was not ready: 0x$('{0:X}' -f $targetReadyFlags)." }
    $targetEditor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) target opened readiness=0x$('{0:X}' -f $targetReadyFlags)`r`n")
    $editPosition = $targetSource.IndexOf('Between tables.', [StringComparison]::Ordinal) + 'Between tables.'.Length
    $editSelectionSet = [MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage,
        [IntPtr]$editPosition, [IntPtr]$editPosition).ToInt32()
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) selection set result=$editSelectionSet source_position=$editPosition`r`n")
    [void][MDLiteNative]::SendMessage($targetEditor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) before WM_CHAR target_editor=$targetEditor selection_result=$editSelectionSet`r`n")
    $charMessageResult = [IntPtr]::Zero
    $charMessageCompleted = [MDLiteNative]::SendMessageTimeoutW(
        $targetEditor, $WM_CHAR, [IntPtr][int][char]'X', [IntPtr]::Zero,
        [uint32]3, [uint32]3000, [ref]$charMessageResult)
    $charMessageError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    if ($charMessageCompleted -eq [IntPtr]::Zero) {
        [IO.File]::AppendAllText($transactionStageLogPath,
            "$(Get-Date -Format o) after WM_CHAR timeout=true last_error=$charMessageError`r`n")
        $child.Kill()
        throw "Transactional WM_CHAR timed out (last error $charMessageError)."
    }
    $targetBufferAfterChar = (Get-WindowText $targetEditor) -replace "`r`n", "`n"
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) after WM_CHAR timeout=false result=$($charMessageResult.ToInt64())`r`n")
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) before Save command id=1005`r`n")
    $saveMessageResult = [IntPtr]::Zero
    $saveMessageCompleted = [MDLiteNative]::SendMessageTimeoutW(
        $main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero,
        [uint32]3, [uint32]3000, [ref]$saveMessageResult)
    $saveMessageError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
    $targetSaveCommandResult = if ($saveMessageCompleted -ne [IntPtr]::Zero) {
        $saveMessageResult.ToInt64()
    } else { "TIMEOUT:$saveMessageError" }
    if ($saveMessageCompleted -eq [IntPtr]::Zero) {
        [IO.File]::AppendAllText($transactionStageLogPath,
            "$(Get-Date -Format o) after Save timeout=true last_error=$saveMessageError`r`n")
        $child.Kill()
        throw "Transactional Save command timed out (last error $saveMessageError)."
    }
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) after Save timeout=false result=$targetSaveCommandResult`r`n")
    $targetEditedBufferAfterSave = (Get-WindowText $targetEditor) -replace "`r`n", "`n"
    $targetEditedDiskAfterSave = ([IO.File]::ReadAllText($targetPath) -replace "`r`n", "`n")
    $targetEditSavedBeforeFailure = $editSelectionSet -ne 0 -and
        $targetEditedBufferAfterSave.Contains('Between tables.X') -and
        $targetEditedDiskAfterSave -ceq $targetEditedSourceLf
    $targetBeforeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetPath).Hash.ToLowerInvariant()

    Click-TabItem $main $tabs 0
    $previousEditor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $previousHwnd = $previousEditor
    $anchor = $previousSource.IndexOf('selection anchor', [StringComparison]::Ordinal) + 8
    $active = $previousSource.IndexOf('remains intact', [StringComparison]::Ordinal) + 4
    $selectionSet = [MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$anchor, [IntPtr]$active).ToInt32()
    [void][MDLiteNative]::SetFocus($previousEditor)
    $failureHookAccepted = [MDLiteNative]::SendMessage(
        $main, $testFailMarkdownPresentationAfterFirstTableMessage,
        [IntPtr]1, [IntPtr]::Zero).ToInt32() -ne 0
    if (-not $failureHookAccepted) { throw 'Partial table presentation failure hook was rejected.' }
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) failure injected accepted=$failureHookAccepted`r`n")
    Click-TabItem $main $tabs 1
    $tabAfterFailure = [MDLiteNative]::SendMessage(
        $tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $focusedAfterFailure = [MDLiteNative]::GetFocusedWindow($main)
    $editorAfterFailure = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $bufferAfterFailure = (Get-WindowText $editorAfterFailure) -replace "`r`n", "`n"
    $selectionAfterFailure = Get-SourceSelectionByMessage $main
    $previousPreserved = $tabAfterFailure -eq 0 -and
        $editorAfterFailure -eq $previousHwnd -and
        $bufferAfterFailure -ceq $previousSource -and
        $selectionSet -ne 0 -and
        $selectionAfterFailure.anchor -eq $anchor -and
        $selectionAfterFailure.active -eq $active -and
        $focusedAfterFailure -eq $previousHwnd
    $targetAfterFailureHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetPath).Hash.ToLowerInvariant()

    Click-TabItem $main $tabs 1
    $retryEditor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $retryFlags = 0
    $retryDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $retryFlags = [MDLiteNative]::SendMessage(
            $main, $testGetEditorReadinessMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if (($retryFlags -band 0x107) -eq 0x107) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $retryDeadline)
    $retryBuffer = (Get-WindowText $retryEditor) -replace "`r`n", "`n"
    $retryFullyPresented = ($retryFlags -band 0x107) -eq 0x107 -and
        $retryBuffer.Contains('FIRST-CELL-ALPHA') -and $retryBuffer.Contains('SECOND-CELL-DELTA')
    $targetAfterRetryHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetPath).Hash.ToLowerInvariant()
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) retry ready flags=0x$('{0:X}' -f $retryFlags) full=$retryFullyPresented`r`n")

    [void][MDLiteNative]::SendMessage($retryEditor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    $diskStableAfterUndoBeforeSave =
        (Get-FileHash -Algorithm SHA256 -LiteralPath $targetPath).Hash.ToLowerInvariant() -eq $targetBeforeHash
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $undoBuffer = (Get-WindowText $retryEditor) -replace "`r`n", "`n"
    $undoDisk = ([IO.File]::ReadAllText($targetPath) -replace "`r`n", "`n")
    $undoSavedOriginalSource = $undoBuffer.Contains('Between tables.') -and
        -not $undoBuffer.Contains('Between tables.X') -and $undoDisk -ceq $targetSourceLf

    [void][MDLiteNative]::SendMessage($retryEditor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 600
    $diskStableAfterRedoBeforeSave =
        ([IO.File]::ReadAllText($targetPath) -replace "`r`n", "`n") -ceq $targetSourceLf
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $redoBuffer = (Get-WindowText $retryEditor) -replace "`r`n", "`n"
    $redoDisk = ([IO.File]::ReadAllText($targetPath) -replace "`r`n", "`n")
    $redoRestoredEditedSource = $redoBuffer.Contains('Between tables.X') -and
        $redoDisk -ceq $targetEditedSourceLf
    $targetAfterRedoSaveHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $targetPath).Hash.ToLowerInvariant()
    [IO.File]::AppendAllText($transactionStageLogPath,
        "$(Get-Date -Format o) undo/redo complete undo=$undoSavedOriginalSource redo=$redoRestoredEditedSource`r`n")
    [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $child.WaitForExit(10000)) { throw 'Transactional table failure fixture did not close cleanly.' }
    return [pscustomobject][ordered]@{
        pass = $targetEditSavedBeforeFailure -and $previousPreserved -and
            $targetBeforeHash -eq $targetAfterFailureHash -and
            $targetBeforeHash -eq $targetAfterRetryHash -and $retryFullyPresented -and
            $diskStableAfterUndoBeforeSave -and $undoSavedOriginalSource -and
            $diskStableAfterRedoBeforeSave -and $redoRestoredEditedSource -and
            $targetAfterRedoSaveHash -eq $targetBeforeHash
        partial_failure_hook_accepted = $failureHookAccepted
        edit_selection_set_result = $editSelectionSet
        target_text_after_wm_char = $targetBufferAfterChar
        target_text_after_save = $targetEditedBufferAfterSave
        target_disk_text_after_save = $targetEditedDiskAfterSave
        undo_disk_text_after_save = $undoDisk
        redo_disk_text_after_save = $redoDisk
        target_save_command_result = $targetSaveCommandResult
        transaction_stage_log = $transactionStageLogPath
        retained_transaction_stage_log = $transactionStageLogArtifactPath
        wm_char_buffer_contains_edit = $targetBufferAfterChar.Contains('Between tables.X')
        post_save_buffer_contains_edit = $targetEditedBufferAfterSave.Contains('Between tables.X')
        post_save_disk_equals_expected_source = $targetEditedDiskAfterSave -ceq $targetEditedSourceLf
        post_save_disk_contains_edit = $targetEditedDiskAfterSave.Contains('Between tables.X')
        target_edit_saved_before_failure = $targetEditSavedBeforeFailure
        target_edit_source = 'Between tables.X'
        previous_editor_hwnd_preserved = $editorAfterFailure -eq $previousHwnd
        previous_buffer_preserved = $bufferAfterFailure -ceq $previousSource
        previous_directional_selection_preserved = $selectionAfterFailure.anchor -eq $anchor -and
            $selectionAfterFailure.active -eq $active -and $anchor -gt $active
        previous_editor_focus_restored = $focusedAfterFailure -eq $previousHwnd
        previous_tab_selected_after_failure = $tabAfterFailure
        target_source_unchanged_after_failure = $targetBeforeHash -eq $targetAfterFailureHash
        target_source_unchanged_after_retry = $targetBeforeHash -eq $targetAfterRetryHash
        retry_editor_readiness_flags = '0x{0:X}' -f $retryFlags
        retry_fully_presented_two_table_content = $retryFullyPresented
        disk_baseline_stable_through_failure_and_retry =
            $targetBeforeHash -eq $targetAfterFailureHash -and $targetBeforeHash -eq $targetAfterRetryHash
        disk_stable_after_undo_before_save = $diskStableAfterUndoBeforeSave
        undo_restored_original_source_after_explicit_save = $undoSavedOriginalSource
        disk_stable_after_redo_before_save = $diskStableAfterRedoBeforeSave
        redo_restored_edited_source_after_explicit_save = $redoRestoredEditedSource
        disk_sha256_baseline = $targetBeforeHash
        disk_sha256_after_redo_save = $targetAfterRedoSaveHash
        synthetic_input = 'Quick Open and tab clicks use cross-process Win32 messages; test-only probe injects failure after streaming table one. No physical input.'
        evidence_boundary = 'Automated native observation only; Human visual, physical input, and IME acceptance remain unknown.'
    }
}

function Capture-E05State(
    $Fixture,
    [string]$Phase,
    [string]$ExpectedSource,
    [string]$ExpectedDiskSource,
    [int]$ExpectedCaret) {
    $editorSource = (Get-WindowText $Fixture.Editor) -replace "`r`n", "`n"
    $diskSource = ([IO.File]::ReadAllText($Fixture.Source)) -replace "`r`n", "`n"
    $selection = Get-EditorSelection $Fixture.Editor
    $sourceMatches = $editorSource -ceq $ExpectedSource
    $diskMatches = $diskSource -ceq $ExpectedDiskSource
    $caretMatches = $selection.start -eq $ExpectedCaret -and
        $selection.end -eq $ExpectedCaret
    return [pscustomobject][ordered]@{
        phase = $Phase
        expected_source = $ExpectedSource
        source_actual = $editorSource
        expected_disk_source = $ExpectedDiskSource
        disk_source_actual = $diskSource
        expected_caret = $ExpectedCaret
        caret_start = $selection.start
        caret_end = $selection.end
        source_matches = $sourceMatches
        disk_source_matches = $diskMatches
        caret_matches = $caretMatches
        pass = $sourceMatches -and $diskMatches -and $caretMatches
    }
}

function Invoke-E05EnglishTypingCase($Fixture, [int]$InterCharacterDelayMs) {
    $text = 'abc'
    $trace = [Collections.Generic.List[object]]::new()
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    $trace.Add((Capture-E05State $Fixture 'initial' '' '' 0))
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))

    $typed = ''
    foreach ($character in $text.ToCharArray()) {
        [void][MDLiteNative]::SendMessage(
            $Fixture.Editor, $WM_CHAR, [IntPtr][int]$character, [IntPtr]::Zero)
        $typed += [string]$character
        $trace.Add((Capture-E05State $Fixture ("type-{0}" -f $character) $typed '' $typed.Length))
        if ($InterCharacterDelayMs -gt 0 -and $typed.Length -lt $text.Length) {
            Start-Sleep -Milliseconds $InterCharacterDelayMs
        }
    }

    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $trace.Add((Capture-E05State $Fixture 'save-after-typing' $text $text $text.Length))

    for ($length = $text.Length - 1; $length -ge 0; $length--) {
        [void][MDLiteNative]::SendMessage($Fixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 100
        $expected = $text.Substring(0, $length)
        $trace.Add((Capture-E05State $Fixture ("undo-{0}" -f $expected) $expected $expected $length))
    }

    for ($length = 1; $length -le $text.Length; $length++) {
        [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 100
        $expected = $text.Substring(0, $length)
        $trace.Add((Capture-E05State $Fixture ("redo-{0}" -f $expected) $expected $expected $length))
    }

    $failedSteps = @($trace | Where-Object { -not $_.pass } | ForEach-Object { $_.phase })
    return [pscustomobject][ordered]@{
        case = 'E05 ordinary English abc typing'
        fixture = [IO.Path]::GetFileName($Fixture.Source)
        input_route = 'Synthetic Win32/editor-message input: SendMessage(WM_CHAR), WM_UNDO, EM_REDO, and WM_COMMAND Save'
        inter_character_delay_ms = $InterCharacterDelayMs
        status = if ($failedSteps.Count -eq 0) { 'PASS' } else { 'FAIL' }
        failed_steps = $failedSteps
        trace = @($trace)
    }
}

function Invoke-E05MiddleInsertCase($Fixture) {
    $trace = [Collections.Generic.List[object]]::new()
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    $trace.Add((Capture-E05State $Fixture 'initial' 'abcdef' 'abcdef' 0))
    [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_SETSEL, [IntPtr]3, [IntPtr]3)
    $trace.Add((Capture-E05State $Fixture 'caret-before-insert' 'abcdef' 'abcdef' 3))

    [void][MDLiteNative]::SendMessage($Fixture.Editor, $WM_CHAR, [IntPtr][int][char]'X', [IntPtr]::Zero)
    $trace.Add((Capture-E05State $Fixture 'insert-X-before-save' 'abcXdef' 'abcdef' 4))
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $trace.Add((Capture-E05State $Fixture 'save-after-insert' 'abcXdef' 'abcXdef' 4))

    [void][MDLiteNative]::SendMessage($Fixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $trace.Add((Capture-E05State $Fixture 'undo-after-save' 'abcdef' 'abcdef' 3))

    [void][MDLiteNative]::SendMessage($Fixture.Editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Fixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $trace.Add((Capture-E05State $Fixture 'redo-after-save' 'abcXdef' 'abcXdef' 4))

    $failedSteps = @($trace | Where-Object { -not $_.pass } | ForEach-Object { $_.phase })
    return [pscustomobject][ordered]@{
        case = 'E05 insertion between existing characters across normal Save'
        fixture = [IO.Path]::GetFileName($Fixture.Source)
        input_route = 'Synthetic Win32/editor-message input: SendMessage(WM_CHAR), WM_UNDO, EM_REDO, and WM_COMMAND Save'
        insertion = 'X at source/caret offset 3 in abcdef'
        status = if ($failedSteps.Count -eq 0) { 'PASS' } else { 'FAIL' }
        failed_steps = $failedSteps
        trace = @($trace)
    }
}

function New-E05BlockedResult([string]$CaseName, [string]$FixtureName, [string]$Reason) {
    return [pscustomobject][ordered]@{
        case = $CaseName
        fixture = $FixtureName
        input_route = 'Synthetic Win32/editor-message input'
        status = 'BLOCKED'
        reason = $Reason
        trace = @()
    }
}

function Get-FileFingerprint([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return 'missing' }
    return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
}

function Get-ResourceSnapshot([Diagnostics.Process]$Process) {
    try {
        $Process.Refresh()
        return [pscustomobject]@{
            status = 'PASS'
            gdi = [int][MDLiteNative]::GetGuiResources($Process.Handle, 0)
            user = [int][MDLiteNative]::GetGuiResources($Process.Handle, 1)
        }
    }
    catch {
        return [pscustomobject]@{ status = 'BLOCKED'; reason = $_.Exception.Message }
    }
}

function Invoke-PaintLayoutReentryProbe(
    [IntPtr]$Main,
    [IntPtr]$Editor,
    [Diagnostics.Process]$Process,
    [int]$Iterations = 64) {
    try {
        $before = Get-ResourceSnapshot $Process
        $beforeRect = New-Object MDLiteNative+RECT
        if ($before.status -ne 'PASS' -or -not [MDLiteNative]::GetClientRect($Main, [ref]$beforeRect)) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = if ($before.status -ne 'PASS') { $before.reason } else { 'GetClientRect failed' } }
        }
        $width = [int]($beforeRect.right - $beforeRect.left)
        $height = [int]($beforeRect.bottom - $beforeRect.top)
        $sizeParam = [IntPtr](($height -band 0xFFFF) -shl 16 -bor ($width -band 0xFFFF))
        $samples = [Collections.Generic.List[object]]::new()
        for ($index = 0; $index -lt $Iterations; $index++) {
            [void][MDLiteNative]::SendMessage($Main, $WM_SIZE, [IntPtr]1, $sizeParam)
            [void][MDLiteNative]::InvalidateRect($Main, [IntPtr]::Zero, $false)
            [void][MDLiteNative]::UpdateWindow($Main)
            [void][MDLiteNative]::InvalidateRect($Editor, [IntPtr]::Zero, $false)
            [void][MDLiteNative]::UpdateWindow($Editor)
            if (($index + 1) % 16 -eq 0) {
                $Process.Refresh()
                if ($Process.HasExited) { break }
                $samples.Add((Get-ResourceSnapshot $Process))
            }
        }
        $after = Get-ResourceSnapshot $Process
        $afterRect = New-Object MDLiteNative+RECT
        $rectOk = [MDLiteNative]::GetClientRect($Main, [ref]$afterRect) -and
            ($afterRect.right - $afterRect.left) -eq $width -and ($afterRect.bottom - $afterRect.top) -eq $height
        $validSamples = @($samples | Where-Object { $_.status -eq 'PASS' })
        if ($after.status -ne 'PASS' -or $validSamples.Count -eq 0) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; before = $before; after = $after; samples = @($samples); client_stable = $rectOk }
        }
        $maxGdi = (@($validSamples | ForEach-Object { $_.gdi }) | Measure-Object -Maximum).Maximum
        $maxUser = (@($validSamples | ForEach-Object { $_.user }) | Measure-Object -Maximum).Maximum
        $pass = -not $Process.HasExited -and $rectOk -and $after.gdi -le ($before.gdi + 24) -and
            $after.user -le ($before.user + 8) -and $maxGdi -le ($before.gdi + 24) -and
            $maxUser -le ($before.user + 8)
        return [pscustomobject]@{
            pass = $pass
            status = if ($pass) { 'PASS_OS_NATIVE_PAINT_LAYOUT' } else { 'FAIL_REENTRY_RESOURCE_GROWTH_OR_LAYOUT_DRIFT' }
            iterations = $Iterations
            client_width = $width
            client_height = $height
            client_stable = $rectOk
            before = $before
            after = $after
            samples = @($samples)
        }
    }
    catch {
        return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = $_.Exception.Message }
    }
}
function Get-NativeRuntimeSnapshot(
    [Diagnostics.Process]$Process,
    [IntPtr]$Main,
    [IntPtr]$Editor,
    [string]$SourcePath,
    [string]$InitialSourceHash,
    [string]$SettingsPath) {
    $windowRect = New-Object MDLiteNative+RECT
    $clientRect = New-Object MDLiteNative+RECT
    if (-not [MDLiteNative]::GetWindowRect($Main, [ref]$windowRect)) { throw 'GetWindowRect failed' }
    if (-not [MDLiteNative]::GetClientRect($Main, [ref]$clientRect)) { throw 'GetClientRect failed' }
    $dpi = [int][MDLiteNative]::GetDpiForWindow($Main)
    $fontHandle = [MDLiteNative]::SendMessage($Editor, $WM_GETFONT, [IntPtr]::Zero, [IntPtr]::Zero)
    $font = New-Object MDLiteNative+LOGFONTW
    $fontBytes = if ($fontHandle -ne [IntPtr]::Zero) {
        [MDLiteNative]::GetObject($fontHandle, [Runtime.InteropServices.Marshal]::SizeOf($font), [ref]$font)
    } else { 0 }
    $editorText = Get-WindowText $Editor
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        $editorTextHash = ([BitConverter]::ToString(
            $sha.ComputeHash([Text.Encoding]::Unicode.GetBytes($editorText))) -replace '-', '').ToLowerInvariant()
    } finally { $sha.Dispose() }
    $richEditPath = Join-Path $env:WINDIR 'System32\Msftedit.dll'
    $richEditVersion = if (Test-Path -LiteralPath $richEditPath -PathType Leaf) {
        (Get-Item -LiteralPath $richEditPath).VersionInfo.FileVersion
    } else { 'missing' }
    $settingsText = if (Test-Path -LiteralPath $SettingsPath -PathType Leaf) {
        [IO.File]::ReadAllText($SettingsPath)
    } else { '' }
    $configuredFontFace = if ($settingsText -match '(?m)^font_face\s*=\s*"([^"]*)"') { $Matches[1] } else { 'Segoe UI' }
    $configuredFontSize = if ($settingsText -match '(?m)^font_size_pt\s*=\s*([0-9]+)') {
        [int]$Matches[1]
    } else { 11 }
    $loadedRichEdit = @()
    $moduleError = $null
    try {
        $loadedRichEdit = @($Process.Modules | Where-Object {
            $_.ModuleName -match '(?i)^(msftedit|riched20)\.dll$'
        } | ForEach-Object {
            [ordered]@{ name = $_.ModuleName; path = $_.FileName; file_version = $_.FileVersionInfo.FileVersion }
        })
    } catch { $moduleError = $_.Exception.Message }
    $selectionPacked = [MDLiteNative]::SendMessage($Editor, $EM_GETSEL, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    return [ordered]@{
        process_id = $Process.Id
        os_version = [Environment]::OSVersion.Version.ToString()
        os_build = [Environment]::OSVersion.Version.Build
        main_class = Get-WindowClass $Main
        editor_class = Get-WindowClass $Editor
        rich_edit_dll = $richEditPath
        rich_edit_dll_file_version = $richEditVersion
        loaded_rich_edit_modules = $loadedRichEdit
        loaded_module_error = $moduleError
        dpi = $dpi
        window = [ordered]@{ left = $windowRect.left; top = $windowRect.top; right = $windowRect.right; bottom = $windowRect.bottom; width = $windowRect.right - $windowRect.left; height = $windowRect.bottom - $windowRect.top }
        client = [ordered]@{ left = $clientRect.left; top = $clientRect.top; right = $clientRect.right; bottom = $clientRect.bottom; width = $clientRect.right - $clientRect.left; height = $clientRect.bottom - $clientRect.top }
        font = [ordered]@{
            source = 'RichEdit character format configured by effective settings'
            configured_face = $configuredFontFace
            configured_size_pt = $configuredFontSize
            wm_getfont_handle = $fontHandle.ToInt64()
            control_logfont_available = $fontBytes -gt 0
            control_face = $font.lfFaceName
            control_height = $font.lfHeight
            control_weight = $font.lfWeight
            control_point_size = if ($dpi -gt 0 -and $fontBytes -gt 0) { [Math]::Round(([Math]::Abs($font.lfHeight) * 72.0 / $dpi), 2) } else { $null }
        }
        caret_selection_packed = $selectionPacked
        editor_text_utf16_sha256 = $editorTextHash
        source_initial_sha256 = $InitialSourceHash
        source_current_sha256 = Get-FileFingerprint $SourcePath
        settings_sha256 = Get-FileFingerprint $SettingsPath
    }
}

$runRoot = Join-Path ([IO.Path]::GetTempPath()) ("mdlite-gui-acceptance-" + [guid]::NewGuid().ToString('N'))
$workspace = Join-Path $runRoot 'workspace'
[IO.Directory]::CreateDirectory($workspace) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
$settingsFile = Join-Path $workspace '.mdlite\settings.toml'
$profilesFile = Join-Path $workspace '.mdlite\profiles.toml'
[IO.File]::WriteAllText($settingsFile,
    "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
$first = Join-Path $workspace 'first.md'
$second = Join-Path $workspace 'second.md'
$pixel = Join-Path $workspace 'pixel.png'
$initial = "CaseToken casetoken WorkspaceHit`n# Outline Heading"
[IO.File]::WriteAllText($first, $initial, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($second, 'second WorkspaceHit', [Text.UTF8Encoding]::new($false))
$treeFixtureFolder = Join-Path $workspace 'tree-folder'
[IO.Directory]::CreateDirectory($treeFixtureFolder) | Out-Null
[IO.File]::WriteAllText((Join-Path $treeFixtureFolder 'nested.md'), 'nested tree item', [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllBytes($pixel, [Convert]::FromBase64String(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+Q4j4WQAAAABJRU5ErkJggg=='))
$process = $null
$checks = [ordered]@{}
$readbackProcesses = [Collections.Generic.List[Diagnostics.Process]]::new()
$typed = ' 日本語compact'
$firstInitialHash = Get-FileFingerprint $first

try {
    $gitActionResponsiveness = Invoke-GitActionResponsiveness (Join-Path $runRoot 'git-action-responsiveness')
    $checks.e20_git_action_responsiveness = $gitActionResponsiveness
    $checks.e20_git_action_responsiveness_pass = [bool]$gitActionResponsiveness.pass
    if ($OnlyE20) {
        $result = [pscustomobject][ordered]@{
            preset = $Preset
            e20_git_action_responsiveness = $gitActionResponsiveness
            pass = [bool]$gitActionResponsiveness.pass
            executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
            timestamp_utc = [DateTime]::UtcNow.ToString('o')
        }
        $directory = Split-Path -Parent $OutputPath
        if ($directory) { [IO.Directory]::CreateDirectory($directory) | Out-Null }
        $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
        $result | ConvertTo-Json -Depth 8
        if (-not $result.pass) { exit 1 }
        return
    }

    $process = Start-Process -FilePath $executable -ArgumentList @($first) -PassThru
    $main = Wait-ProcessWindow $process
    $editor = Wait-Control $main 102 'RICHEDIT50W'
    $checks.main_window = $true
    $editor = Wait-WorkspaceDocumentReady $main $process 'first.md' $workspace
    $checks.initial_view = ((Get-WindowText $editor) -replace "`r`n", "`n") -eq $initial
    $checks.runtime_environment = Get-NativeRuntimeSnapshot $process $main $editor $first $firstInitialHash $settingsFile

    $suspendedEditorLifecycle = [ordered]@{}
    try {
        $suspendedEditorLifecycle = Invoke-SuspendedEditorLifecycle (Join-Path $runRoot 'suspended-editor-lifecycle')
    } catch {
        $suspendedEditorLifecycle = [ordered]@{
            pass = $false
            error = $_.Exception.Message
            synthetic_input = 'Cross-process Win32 messages only; no physical input.'
        }
    }
    $checks.suspended_editor_lifecycle = $suspendedEditorLifecycle
    $checks.suspended_editor_lifecycle_pass = [bool]$suspendedEditorLifecycle.pass

    $workspaceSessionViewRestore = [ordered]@{}
    try {
        $workspaceSessionViewRestore = Invoke-WorkspaceSessionViewRestore `
            (Join-Path $runRoot 'workspace-session-view-restore')
    } catch {
        $workspaceSessionViewRestore = [ordered]@{
            pass = $false
            error = $_.Exception.Message
            workspace_document_readiness_traces = $script:workspaceDocumentReadinessTraces.ToArray()
            synthetic_input = 'Cross-process Win32 messages and a test-only visible-source-offset probe; no physical input.'
            evidence_boundary = 'Automated native observation only; Human visual, physical input, and IME acceptance remain unknown.'
        }
    }
    $checks.workspace_session_view_restore = $workspaceSessionViewRestore
    $checks.workspace_session_view_restore_pass = [bool]$workspaceSessionViewRestore.pass

    $transactionalTableFailure = [ordered]@{}
    try {
        $transactionalTableFailure = Invoke-TransactionalTablePresentationFailure `
            (Join-Path $runRoot 'transactional-table-presentation-failure')
    } catch {
        $transactionalTableFailure = [ordered]@{
            pass = $false
            error = $_.Exception.Message
            synthetic_input = 'Cross-process Win32 messages and a test-only first-table presentation failure; no physical input.'
            evidence_boundary = 'Automated native observation only; Human visual, physical input, and IME acceptance remain unknown.'
        }
    }
    $checks.transactional_table_presentation_failure = $transactionalTableFailure
    $checks.transactional_table_presentation_failure_pass = [bool]$transactionalTableFailure.pass

    # Current E05 covers ordinary English typing. Each case uses synthetic
    # Win32/editor messages; physical-key behavior remains outside this route.
    $e05Results = [ordered]@{}
    foreach ($case in @(
        [pscustomobject]@{ key = 'fast'; name = 'e05-english-fast'; delay = 0 },
        [pscustomobject]@{ key = 'slow'; name = 'e05-english-slow'; delay = 250 })) {
        $e05Fixture = $null
        try {
            $e05Fixture = Start-ReadbackFixture -Name $case.name -InitialText ''
            $e05Results[$case.key] = Invoke-E05EnglishTypingCase $e05Fixture $case.delay
        }
        catch {
            $e05Results[$case.key] = New-E05BlockedResult `
                ("E05 ordinary English abc {0}" -f $case.key) $case.name $_.Exception.Message
        }
        finally {
            if ($e05Fixture) { Stop-ReadbackFixture $e05Fixture }
        }
    }

    $e05InsertFixture = $null
    try {
        $e05InsertFixture = Start-ReadbackFixture -Name 'e05-middle-insert' -InitialText 'abcdef'
        $e05Results.middle_insert = Invoke-E05MiddleInsertCase $e05InsertFixture
    }
    catch {
        $e05Results.middle_insert = New-E05BlockedResult `
            'E05 insertion between existing characters across normal Save' `
            'e05-middle-insert' $_.Exception.Message
    }
    finally {
        if ($e05InsertFixture) { Stop-ReadbackFixture $e05InsertFixture }
    }

    $checks.e05_input_route = 'Synthetic Win32/editor-message input: WM_CHAR, WM_UNDO, EM_REDO, EM_GETSEL, and normal Save command'
    $checks.e05_physical_key_evidence = 'UNKNOWN: this harness sends Win32/editor messages and does not establish physical-key behavior'
    $checks.e05_english_undo_redo = $e05Results
    $checks.e05_english_fast_pass = $e05Results.fast.status -eq 'PASS'
    $checks.e05_english_slow_pass = $e05Results.slow.status -eq 'PASS'
    $checks.e05_middle_insert_save_undo_redo_pass = $e05Results.middle_insert.status -eq 'PASS'

    foreach ($imageIdentityCase in @(
        [pscustomobject]@{ key = 'first_plus_X'; name = 'collapsed-image-first-plus-X'; target = 'first-plus-X' },
        [pscustomobject]@{ key = 'X_plus_second'; name = 'collapsed-image-X-plus-second'; target = 'X-plus-second' })) {
        $imageIdentityResult = $null
        try {
            $imageIdentityResult = Invoke-CollapsedImageIdentityDeletion `
                $imageIdentityCase.name $imageIdentityCase.target
        } catch {
            $imageIdentityResult = [pscustomobject][ordered]@{
                pass = $false
                case = $imageIdentityCase.target
                fixture = 'Synthetic local image-identity fixture'
                error = $_.Exception.Message
                synthetic_input = 'Cross-process Win32/editor messages; no physical input.'
                checks = [ordered]@{}
            }
        }
        $checks["collapsed_image_identity_$($imageIdentityCase.key)"] = $imageIdentityResult
        foreach ($checkName in $imageIdentityResult.checks.Keys) {
            $checks["collapsed_image_identity_$($imageIdentityCase.key)_$checkName"] =
                [bool]$imageIdentityResult.checks[$checkName]
        }
        $checks["collapsed_image_identity_$($imageIdentityCase.key)_pass"] = [bool]$imageIdentityResult.pass
    }

    $tableTopologyChecks = @(
        'initial_projection_present', 'initial_editor_enabled',
        'initial_readback_ready', 'selection_set', 'native_selection_range_set', 'edited_editor_text_matches',
        'edited_editor_enabled', 'edited_readback_ready', 'source_bytes_unchanged_before_save',
        'edited_save_exact_bytes', 'undo_editor_text_matches_original', 'undo_editor_enabled', 'undo_readback_ready',
        'undo_save_exact_original_bytes', 'redo_editor_text_matches_edited',
        'redo_editor_enabled', 'redo_readback_ready', 'redo_save_exact_edited_bytes')
    $tableTopologyReports = [ordered]@{}
    $topologyOriginal = @(
        '# 表の編集確認',
        '',
        '説明 日本語 CRLF',
        '',
        '| 名前 | 値 |',
        '| --- | --- |',
        '| 林檎 | 1 |',
        '| 茶 | 2 |',
        '',
        '末尾 日本語') -join "`r`n"
    $topologyOriginal += "`r`n"
    $topologyPlainReplacement = '置'
    $topologyPlainExpected = $topologyPlainReplacement
    $topologyFixture = $null
    try {
        $topologyFixture = Start-ReadbackFixture 'table-topology-full-document' $topologyOriginal
        $tableTopologyReports.full_document = Invoke-TableTopologyReplacement `
            $topologyFixture 'Full-document selection replaces rendered table with one Japanese character' `
            $topologyPlainReplacement 0 -1 `
            $topologyPlainExpected -InitialProjectionSentinels @(
                '説明 日本語 CRLF', '林檎', '茶', '末尾 日本語') `
            -EditedProjectionSentinels @() -SelectionMode 'select-all' `
            -InputMode 'wm-char' -EditedHasTable $false
    } catch {
        $tableTopologyReports.full_document = [pscustomobject][ordered]@{
            pass = $false
            error = $_.Exception.Message
            input_route = 'Synthetic test-only silent route: direct RichEdit EM_SETSEL(0,-1) and one Japanese WM_CHAR; no physical keyboard or IME coverage.'
            checks = [ordered]@{}
        }
    } finally {
        if ($topologyFixture) { Stop-ReadbackFixture $topologyFixture }
    }

    $topologyFixture = $null
    try {
        $topologyFixture = Start-ReadbackFixture 'table-topology-row-cell-replacement' $topologyOriginal
        $oldTable = @(
            '| 名前 | 値 |',
            '| --- | --- |',
            '| 林檎 | 1 |',
            '| 茶 | 2 |') -join "`r`n"
        $newTable = @(
            '| 名前 | 数 | 備考 |',
            '| --- | --- | --- |',
            '| 林檎 | 10 | 赤 |',
            '| 茶 | 20 | 茶色 |',
            '| 葡萄 | 30 | 紫 |') -join "`r`n"
        $tableStart = $topologyOriginal.IndexOf($oldTable, [StringComparison]::Ordinal)
        if ($tableStart -lt 0) { throw 'Could not locate the original table in the Japanese CRLF fixture.' }
        $tableEnd = $tableStart + $oldTable.Length
        $topologyChangedExpectedBytesSource = $topologyOriginal.Substring(0, $tableStart) +
            $newTable + $topologyOriginal.Substring($tableEnd)
        $tableTopologyReports.row_cell_topology = Invoke-TableTopologyReplacement `
            $topologyFixture 'Selection replacement changes rendered table row and cell topology' `
            $newTable $tableStart $tableEnd `
            $topologyChangedExpectedBytesSource -InitialProjectionSentinels @(
                '説明 日本語 CRLF', '林檎', '茶', '末尾 日本語') `
            -EditedProjectionSentinels @(
                '説明 日本語 CRLF', '林檎', '10', '赤', '茶', '20', '茶色',
                '葡萄', '30', '紫', '末尾 日本語') `
            -SelectionMode 'source-probe' -InputMode 'em-replacesel' -EditedHasTable $true
    } catch {
        $tableTopologyReports.row_cell_topology = [pscustomobject][ordered]@{
            pass = $false
            error = $_.Exception.Message
            input_route = 'Synthetic test-only silent route: source-selection probe, RichEdit EM_SETSEL, and one atomic EM_REPLACESEL; no physical paste, keyboard, or IME coverage.'
            checks = [ordered]@{}
        }
    } finally {
        if ($topologyFixture) { Stop-ReadbackFixture $topologyFixture }
    }
    foreach ($caseKey in @('full_document', 'row_cell_topology')) {
        $report = $tableTopologyReports[$caseKey]
        foreach ($checkName in $tableTopologyChecks) {
            $checkValue = if ($report.checks -and $report.checks.Contains($checkName)) {
                [bool]$report.checks[$checkName]
            } else { $false }
            $checks["table_topology_${caseKey}_${checkName}"] = $checkValue
        }
        $checks["table_topology_${caseKey}_pass"] = [bool]$report.pass
    }
    $checks.table_topology_regression_matrix = $tableTopologyReports
    $e05Statuses = @($e05Results.Values | ForEach-Object { $_.status })
    $checks.e05_english_undo_redo_status = if ($e05Statuses -contains 'BLOCKED') {
        'BLOCKED'
    } elseif ($e05Statuses -contains 'FAIL') {
        'FAIL'
    } else {
        'PASS'
    }

    # Re-enter native layout and paint synchronously; this is OS-native message evidence, not Human visual acceptance.
    $checks.paint_layout_reentry = Invoke-PaintLayoutReentryProbe $main $editor $process 64

    # Drag the workspace splitter through the real parent window route, then
    # restore its starting width so later checks keep the same layout.
    $tree = Wait-Control $main 100 'SysTreeView32'
    [MDLiteNative+RECT]$treeBefore = New-Object MDLiteNative+RECT
    [void][MDLiteNative]::GetWindowRect($tree, [ref]$treeBefore)
    $beforeTreeWidth = $treeBefore.right - $treeBefore.left
    $splitterBefore = Get-WorkspaceSplitterPosition $main
    $splitterX = $splitterBefore.x
    $splitterY = $splitterBefore.y
    $makePoint = {
        param([int]$x, [int]$y)
        return [IntPtr]([long](($y -band 0xffff) -shl 16) -bor ($x -band 0xffff))
    }
    [void][MDLiteNative]::SendMessage($main, 0x0201, [IntPtr]1, (& $makePoint $splitterX $splitterY))
    [void][MDLiteNative]::SendMessage($main, 0x0200, [IntPtr]1, (& $makePoint ($splitterX + 24) $splitterY))
    [void][MDLiteNative]::SendMessage($main, 0x0202, [IntPtr]::Zero, (& $makePoint ($splitterX + 24) $splitterY))
    [MDLiteNative+RECT]$treeDragged = New-Object MDLiteNative+RECT
    [void][MDLiteNative]::GetWindowRect($tree, [ref]$treeDragged)
    $draggedTreeWidth = $treeDragged.right - $treeDragged.left
    $splitterDragged = Get-WorkspaceSplitterPosition $main
    [void][MDLiteNative]::SendMessage($main, 0x0201, [IntPtr]1, (& $makePoint $splitterDragged.x $splitterDragged.y))
    [void][MDLiteNative]::SendMessage($main, 0x0200, [IntPtr]1, (& $makePoint $splitterX $splitterY))
    [void][MDLiteNative]::SendMessage($main, 0x0202, [IntPtr]::Zero, (& $makePoint $splitterX $splitterY))
    [MDLiteNative+RECT]$treeRestored = New-Object MDLiteNative+RECT
    [void][MDLiteNative]::GetWindowRect($tree, [ref]$treeRestored)
    $restoredTreeWidth = $treeRestored.right - $treeRestored.left
    $checks.workspace_splitter_drag = $draggedTreeWidth -ge ($beforeTreeWidth + 16)
    $checks.workspace_splitter_restore = [Math]::Abs($restoredTreeWidth - $beforeTreeWidth) -le 2
    $checks.workspace_splitter_trace = [ordered]@{
        before = $beforeTreeWidth
        dragged = $draggedTreeWidth
        restored = $restoredTreeWidth
        initial_geometry = $splitterBefore
        dragged_geometry = $splitterDragged
    }

    # View -> compact. Focus selection must follow the editor, and EN_CHANGE must route to its new parent.
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1030, [IntPtr]::Zero)
    $compact = Wait-ProcessClassWindow $process 'MDLite.CompactWindow'
    Send-Characters $editor $typed
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $savedAfterTyping = [IO.File]::ReadAllText($first)
    $checks.compact_edit_saved = $savedAfterTyping -eq ($initial + $typed)

    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    # RichEdit may coalesce a contiguous typing burst by build/optimization. Both a
    # whole-burst undo and a final-input-event undo are valid, but no-op/loss is not.
    $undoText = [IO.File]::ReadAllText($first)
    $checks.compact_undo_saved = $undoText -eq $initial -or
        $undoText -eq ($initial + $typed.Substring(0, $typed.Length - 1))
    $checks.compact_undo_granularity = if ($undoText -eq $initial) { 'typing-burst' } else { 'input-event' }

    [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $checks.compact_redo_saved = [IO.File]::ReadAllText($first) -eq ($initial + $typed)

    # Return the editor to the main window before exercising the in-window result list.
    [void][MDLiteNative]::SendMessage($compact, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    $findEdit = Wait-Control $main 105 'Edit'
    $caseCheck = Wait-Control $main 110 'Button'
    $regexCheck = Wait-Control $main 111 'Button'
    [void][MDLiteNative]::SendMessage($caseCheck, $BM_SETCHECK, [IntPtr]$BST_CHECKED, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($regexCheck, $BM_SETCHECK, [IntPtr]$BST_UNCHECKED, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($findEdit, $WM_SETTEXT, [IntPtr]::Zero, 'casetoken')
    [void][MDLiteNative]::SendMessage($editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]106, [IntPtr]::Zero)
    $selection = [MDLiteNative]::SendMessage($editor, $EM_GETSEL, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    $checks.find_match_case = (($selection -band 0xFFFF) -eq 10) -and ((($selection -shr 16) -band 0xFFFF) -eq 19)

    [void][MDLiteNative]::SendMessage($caseCheck, $BM_SETCHECK, [IntPtr]$BST_UNCHECKED, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($findEdit, $WM_SETTEXT, [IntPtr]::Zero, 'WorkspaceHit')
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]108, [IntPtr]::Zero)
    $results = Wait-Control $main 117 'SysListView32'
    $itemCount = Wait-ListItemCount $results 2
    $checks.workspace_result_list = $itemCount -eq 2

    # Search immediately after an edit, before the delayed input coalescer can
    # run.  Workspace search must synchronize the open buffer itself.
    Send-Characters $editor ' PendingHit'
    [void][MDLiteNative]::SendMessage($findEdit, $WM_SETTEXT, [IntPtr]::Zero, 'PendingHit')
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]108, [IntPtr]::Zero)
    $pendingCount = Wait-ListItemCount $results 1
    $checks.workspace_search_syncs_pending_input = $pendingCount -eq 1

    # Exercise the native workspace/outline/calendar presentation routes and
    # the searchable native Quick Open/command-palette pickers.  The picker
    # checks select real candidates, then restore the original document before
    # the remaining source/Undo/image assertions.
    $workspaceTree = Wait-Control $main 100 'SysTreeView32'
    $outlineTree = Wait-Control $main 103 'SysTreeView32'
    $checks.workspace_tree_items = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -gt 0
    $treeStyle = [int64][MDLiteNative]::GetWindowLongPtr($workspaceTree, $GWL_STYLE).ToInt64()
    $checks.explorer_tree_uses_icons = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_GETIMAGELIST, [IntPtr]::Zero, [IntPtr]::Zero) -ne [IntPtr]::Zero
    $checks.explorer_tree_hides_legacy_lines = (($treeStyle -band ($TVS_HASLINES -bor $TVS_LINESATROOT)) -eq 0)
    $treeRootItem = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_GETNEXTITEM, [IntPtr]$TVGN_ROOT, [IntPtr]::Zero)
    $treeExpandResult = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_EXPAND, [IntPtr]$TVE_EXPAND, $treeRootItem).ToInt32()
    $treeFirstChild = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_GETNEXTITEM, [IntPtr]$TVGN_CHILD, $treeRootItem)
    $treeSelectResult = [MDLiteNative]::SendMessage(
        $workspaceTree, $TVM_SELECTITEM, [IntPtr]$TVGN_CARET, $treeRootItem).ToInt32()
    $checks.explorer_tree_expand_select =
        $treeRootItem -ne [IntPtr]::Zero -and $treeExpandResult -ne 0 -and
        $treeFirstChild -ne [IntPtr]::Zero -and $treeSelectResult -ne 0
    $checks.outline_tree_items = [MDLiteNative]::SendMessage(
        $outlineTree, $TVM_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -gt 0
    $outlineBefore = [MDLiteNative]::IsWindowVisible($outlineTree)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1066, [IntPtr]::Zero)
    $outlineAfterCollapse = [MDLiteNative]::IsWindowVisible($outlineTree)
    $checks.outline_pane_collapsed = $outlineBefore -and -not $outlineAfterCollapse
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1066, [IntPtr]::Zero)
    $outlineAfterRestore = [MDLiteNative]::IsWindowVisible($outlineTree)
    $checks.outline_pane_restored = $outlineBefore -and -not $outlineAfterCollapse -and $outlineAfterRestore
    $checks.outline_toggle_trace = [ordered]@{ initial_visible = $outlineBefore; after_collapse_visible = $outlineAfterCollapse; after_restore_visible = $outlineAfterRestore; caption = Get-WindowText $main; status = if (-not $outlineBefore) { 'BLOCKED_INITIAL_OUTLINE_NOT_VISIBLE' } elseif ($checks.outline_pane_collapsed -and $checks.outline_pane_restored) { 'PASS_OBSERVED_COLLAPSE_RESTORE' } else { 'FAIL_VISIBILITY_TOGGLE' } }

    $calendarToggle = Invoke-CalendarToggleCheck $main
    foreach ($key in $calendarToggle.Keys) { $checks[$key] = $calendarToggle[$key] }

    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1003, [IntPtr]::Zero)
    $quickOpenPicker = Wait-ProcessClassWindow $process 'MDLite.NativePickerWindow'
    $quickOpenFilter = Wait-Control $quickOpenPicker 100 'Edit'
    $quickOpenList = Wait-Control $quickOpenPicker 101 'ListBox'
    $checks.quick_open_picker = $quickOpenPicker -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($quickOpenFilter, $WM_SETTEXT, [IntPtr]::Zero, 'second.md')
    Start-Sleep -Milliseconds 100
    $quickOpenCount = [MDLiteNative]::SendMessage(
        $quickOpenList, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.quick_open_filter_single = $quickOpenCount -eq 1
    [void][MDLiteNative]::SendMessage($quickOpenPicker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
    $quickOpenReady=Wait-QuickOpenDocumentEditor $main $process 'second.md' 'second WorkspaceHit'
    $editor=$quickOpenReady.Editor
    $checks.quick_open_selected_document_readiness=$quickOpenReady.Evidence
    $checks.quick_open_editor_after_selection=$quickOpenReady.Text
    $checks.quick_open_candidate_selected = $checks.quick_open_editor_after_selection -eq 'second WorkspaceHit'

    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1003, [IntPtr]::Zero)
    $quickOpenPicker = Wait-ProcessClassWindow $process 'MDLite.NativePickerWindow'
    $quickOpenFilter = Wait-Control $quickOpenPicker 100 'Edit'
    $quickOpenList = Wait-Control $quickOpenPicker 101 'ListBox'
    [void][MDLiteNative]::SendMessage($quickOpenFilter, $WM_SETTEXT, [IntPtr]::Zero, 'first.md')
    Start-Sleep -Milliseconds 100
    $quickOpenCount = [MDLiteNative]::SendMessage(
        $quickOpenList, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.quick_open_return_filter_single = $quickOpenCount -eq 1
    [void][MDLiteNative]::SendMessage($quickOpenPicker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
    $quickOpenReturnReady=Wait-QuickOpenDocumentEditor $main $process 'first.md' 'CaseToken' -Prefix
    $editor=$quickOpenReturnReady.Editor
    $checks.quick_open_return_document_readiness=$quickOpenReturnReady.Evidence
    $checks.quick_open_returned_to_first=$quickOpenReturnReady.Text.StartsWith('CaseToken')
    $tabs = Wait-Control $main 101 'SysTabControl32'
    $tabStyle = [int64][MDLiteNative]::GetWindowLongPtr($tabs, $GWL_STYLE).ToInt64()
    $tabCount = [MDLiteNative]::SendMessage($tabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.native_tab_owner_draw_style = ($tabStyle -band $TCS_OWNERDRAWFIXED) -ne 0
    $checks.native_tab_close_action = (Find-Control $main 129 'Button') -ne [IntPtr]::Zero
    $tabBefore = [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    if (-not [MDLiteNative]::FocusKeyboardControl($main, $tabs)) { throw 'Native tab keyboard focus could not be verified on the target GUI thread.' }
    [void][MDLiteNative]::SendMessage($tabs, 0x1330, [IntPtr]$tabBefore, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0100, [IntPtr]0x27, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0101, [IntPtr]0x27, [IntPtr]::Zero)
    $tabFocusAfterRight = [MDLiteNative]::SendMessage($tabs, 0x132F, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    [void][MDLiteNative]::SendMessage($tabs, 0x0100, [IntPtr]0x20, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0101, [IntPtr]0x20, [IntPtr]::Zero)
    $tabAfterRight = [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $tabRightReady = Wait-QuickOpenDocumentEditor $main $process 'second.md' 'second WorkspaceHit'
    if (-not [MDLiteNative]::FocusKeyboardControl($main, $tabs)) { throw 'Native tab return focus could not be verified on the target GUI thread.' }
    [void][MDLiteNative]::SendMessage($tabs, 0x1330, [IntPtr]$tabAfterRight, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0100, [IntPtr]0x25, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0101, [IntPtr]0x25, [IntPtr]::Zero)
    $tabFocusAfterLeft = [MDLiteNative]::SendMessage($tabs, 0x132F, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    [void][MDLiteNative]::SendMessage($tabs, 0x0100, [IntPtr]0x20, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($tabs, 0x0101, [IntPtr]0x20, [IntPtr]::Zero)
    $tabAfterLeft = [MDLiteNative]::SendMessage($tabs, 0x130B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.native_tab_keyboard_selection = $tabCount -ge 2 -and
        $tabBefore -eq 0 -and $tabFocusAfterRight -ne $tabBefore -and $tabAfterRight -eq $tabFocusAfterRight -and
        $tabFocusAfterLeft -eq $tabBefore -and $tabAfterLeft -eq $tabBefore
    $checks.native_tab_keyboard_trace = [ordered]@{ before_selected = $tabBefore; right_focused = $tabFocusAfterRight; right_selected_after_space = $tabAfterRight; left_focused = $tabFocusAfterLeft; left_selected_after_space = $tabAfterLeft; right_document_ready = $tabRightReady.Evidence }
    # Tab switching may destroy and recreate the RichEdit HWND. Refresh the
    # handle for the first document before sending subsequent editor messages.
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $editorTextAfterTabReturn = (Get-WindowText $editor) -replace "`r`n", "`n"
    if (-not $editorTextAfterTabReturn.StartsWith('CaseToken')) {
        throw 'Tab keyboard sequence did not return to the first document editor.'
    }
    [void][MDLiteNative]::SetFocus($editor)

    # E25 cancellation must return the directional source selection and editor
    # focus without changing the open buffer or saved bytes.
    $paletteAnchor = 12
    $paletteActive = 2
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1031, [IntPtr]::Zero)
    $commandPalettePicker = Wait-ProcessClassWindow $process 'MDLite.NativePickerWindow'
    $commandPaletteFilter = Wait-Control $commandPalettePicker 100 'Edit'
    $commandPaletteList = Wait-Control $commandPalettePicker 101 'ListBox'
    $checks.command_palette_picker = $commandPalettePicker -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($commandPaletteFilter, $WM_SETTEXT, [IntPtr]::Zero, '表示: Workspace設定')
    Start-Sleep -Milliseconds 100
    $commandPaletteCount = [MDLiteNative]::SendMessage(
        $commandPaletteList, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.command_palette_filter_single = $commandPaletteCount -eq 1
    [void][MDLiteNative]::SendMessage($commandPalettePicker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
    $commandPaletteForm = Wait-ProcessClassWindow $process 'MDLite.NativeFormWindow'
    $checks.command_palette_candidate_selected = $commandPaletteForm -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($commandPaletteForm, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativeFormWindow'

    [void][MDLiteNative]::SetFocus($editor)
    $escapeBeforeSelectionSet = [MDLiteNative]::SendMessage(
        $main, $testSetSelectionBySourceMessage, [IntPtr]$paletteAnchor,
        [IntPtr]$paletteActive).ToInt32()
    $escapeBeforeSelection = Get-SourceSelectionByMessage $main
    $escapeBeforeText = (Get-WindowText $editor) -replace "`r`n", "`n"
    $escapeBeforeDiskHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $first).Hash.ToLowerInvariant()
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1031, [IntPtr]::Zero)
    $escapePicker = Wait-ProcessClassWindow $process 'MDLite.NativePickerWindow'
    $escapeFilter = Wait-Control $escapePicker 100 'Edit'
    [void][MDLiteNative]::SetFocus($escapeFilter)
    [void][MDLiteNative]::PostMessage($escapeFilter, $WM_KEYDOWN, [IntPtr]0x1B, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $escapeAfterSelection = Get-SourceSelectionByMessage $main
    $escapeAfterText = (Get-WindowText $editor) -replace "`r`n", "`n"
    $escapeAfterDiskHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $first).Hash.ToLowerInvariant()
    $escapeFocusedWindow = [MDLiteNative]::GetFocusedWindow($main)
    $checks.command_palette_escape_closed = $escapePicker -ne [IntPtr]::Zero -and
        -not [MDLiteNative]::IsWindow($escapePicker)
    $checks.command_palette_escape_restored_directional_selection =
        $escapeBeforeSelectionSet -ne 0 -and
        $escapeBeforeSelection.anchor -eq $paletteAnchor -and
        $escapeBeforeSelection.active -eq $paletteActive -and
        $escapeAfterSelection.anchor -eq $paletteAnchor -and
        $escapeAfterSelection.active -eq $paletteActive
    $checks.command_palette_escape_restored_editor_focus = $escapeFocusedWindow -eq $editor
    $checks.command_palette_escape_selection_setup =
        $escapeBeforeSelectionSet -ne 0 -and
        $escapeBeforeSelection.anchor -eq $paletteAnchor -and
        $escapeBeforeSelection.active -eq $paletteActive
    $checks.command_palette_escape_preserved_buffer_and_disk =
        $escapeBeforeText -ceq $escapeAfterText -and
        $escapeBeforeDiskHash -ceq $escapeAfterDiskHash -and
        $escapeAfterDiskHash -ceq (Get-FileFingerprint $first)
    $checks.command_palette_escape_trace = [ordered]@{
        selection_before_escape = $escapeBeforeSelection
        selection_after_escape = $escapeAfterSelection
        editor_focus_restored = $escapeFocusedWindow -eq $editor
        buffer_unchanged = $escapeBeforeText -ceq $escapeAfterText
        disk_sha256_before = $escapeBeforeDiskHash
        disk_sha256_after = $escapeAfterDiskHash
        key_route = 'Posted WM_KEYDOWN(VK_ESCAPE) to the picker filter; no physical keyboard input.'
    }

    # R-16 diagnostics are available through the normal command palette. Check
    # the local snapshot fields, then exercise the explicit refresh button and
    # prove that the Task Dialog remains open with a newly recorded snapshot.
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1031, [IntPtr]::Zero)
    $diagnosticsPicker = Wait-ProcessClassWindow $process 'MDLite.NativePickerWindow'
    $diagnosticsFilter = Wait-Control $diagnosticsPicker 100 'Edit'
    $diagnosticsList = Wait-Control $diagnosticsPicker 101 'ListBox'
    [void][MDLiteNative]::SendMessage($diagnosticsFilter, $WM_SETTEXT, [IntPtr]::Zero, '表示: 診断情報')
    Start-Sleep -Milliseconds 100
    $diagnosticsCount = [MDLiteNative]::SendMessage(
        $diagnosticsList, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $checks.diagnostics_palette_filter_single = $diagnosticsCount -eq 1
    if ($diagnosticsCount -eq 1) {
        [void][MDLiteNative]::SendMessage($diagnosticsPicker, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
        Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
        $diagnosticsDialog = Wait-ProcessClassWindow $process '#32770'
        $diagnosticsTitle = Get-WindowText $diagnosticsDialog
        $diagnosticsStillOpen = [IntPtr]::Zero
        $checks.diagnostics_dialog_title = $diagnosticsTitle -eq '診断情報'
        $diagnosticsBeforeImagePath = [IO.Path]::ChangeExtension($OutputPath, '.diagnostics-before.png')
        $diagnosticsAfterImagePath = [IO.Path]::ChangeExtension($OutputPath, '.diagnostics-after.png')
        $diagnosticsBeforeCapture = Save-WindowCapture $diagnosticsDialog $diagnosticsBeforeImagePath
        $checks.diagnostics_view_capture = $diagnosticsBeforeCapture.status -eq 'CAPTURED_PRINTWINDOW'
        $checks.diagnostics_update_state = [ordered]@{
            app_update = 'SOURCE_UNSET_NO_AUTOMATIC_CHECK_OR_DOWNLOAD'
            dependency_update = 'MANUAL_PINNED_VERSION_URL_AND_SHA256'
            capture = $diagnosticsBeforeCapture
        }
        $diagnosticRefresh = [MDLiteNative]::FindChildWithText($diagnosticsDialog, '更新', 'Button')
        $checks.diagnostics_refresh_button = $diagnosticRefresh -ne [IntPtr]::Zero
        if ($diagnosticRefresh -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage($diagnosticRefresh, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 100
            $diagnosticsStillOpen = Get-ProcessClassWindow $process '#32770'
            $diagnosticsAfterCapture = if ($diagnosticsStillOpen -ne [IntPtr]::Zero) {
                Save-WindowCapture $diagnosticsStillOpen $diagnosticsAfterImagePath
            } else {
                [pscustomobject]@{ status='BLOCKED'; error='Task Dialog closed after refresh'; path=$diagnosticsAfterImagePath; sha256=$null }
            }
            $checks.diagnostics_refresh_keeps_dialog_open = $diagnosticsStillOpen -ne [IntPtr]::Zero
            $checks.diagnostics_refresh_updates_snapshot =
                $diagnosticsAfterCapture.status -eq 'CAPTURED_PRINTWINDOW' -and
                $diagnosticsAfterCapture.sha256 -ne $diagnosticsBeforeCapture.sha256
            $checks.diagnostics_before_capture = $diagnosticsBeforeCapture
            $checks.diagnostics_after_capture = $diagnosticsAfterCapture
        } else {
            $checks.diagnostics_refresh_keeps_dialog_open = $false
            $checks.diagnostics_refresh_updates_snapshot = $false
            $diagnosticsAfterCapture = [pscustomobject]@{
                status='BLOCKED'; error='Refresh button was not found'; path=$diagnosticsAfterImagePath; sha256=$null
            }
        }
        if ($diagnosticsStillOpen -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::PostMessage($diagnosticsStillOpen, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            Wait-ProcessClassWindowGone $process '#32770'
        }
    } else {
        $checks.diagnostics_dialog_title = $false
        $checks.diagnostics_view_capture = $false
        $checks.diagnostics_update_state = [ordered]@{ app_update='UNOBSERVED'; dependency_update='UNOBSERVED'; capture=$null }
        $checks.diagnostics_refresh_keeps_dialog_open = $false
        $checks.diagnostics_refresh_updates_snapshot = $false
        $checks.diagnostics_refresh_button = $false
        $checks.diagnostics_before_capture = $null
        $checks.diagnostics_after_capture = $null
        [void][MDLiteNative]::SendMessage($diagnosticsPicker, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
        Wait-ProcessClassWindowGone $process 'MDLite.NativePickerWindow'
    }

    # Settings and profile editing are native one-form dialogs. Exercise both
    # form-level Cancel and Apply paths. Profile Apply reaches the existing
    # silent-test confirmation boundary and is intentionally declined there;
    # this proves the form Apply/validation path without mutating the fixture.
    $settingsBeforeCancel = Get-FileFingerprint $settingsFile
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1027, [IntPtr]::Zero)
    $settingsCancelForm = Wait-ProcessClassWindow $process 'MDLite.NativeFormWindow'
    $checks.settings_native_form = $settingsCancelForm -ne [IntPtr]::Zero
    $checks.settings_native_cancel = $settingsCancelForm -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($settingsCancelForm, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativeFormWindow'
    $checks.settings_native_cancel_preserves = (Get-FileFingerprint $settingsFile) -eq $settingsBeforeCancel

    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1027, [IntPtr]::Zero)
    $settingsApplyForm = Wait-ProcessClassWindow $process 'MDLite.NativeFormWindow'
    $checks.settings_native_apply = $settingsApplyForm -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($settingsApplyForm, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativeFormWindow'

    $profilesBeforeCancel = Get-FileFingerprint $profilesFile
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1029, [IntPtr]::Zero)
    $profilesCancelForm = Wait-ProcessClassWindow $process 'MDLite.NativeFormWindow'
    $checks.profiles_native_form = $profilesCancelForm -ne [IntPtr]::Zero
    $checks.profiles_native_cancel = $profilesCancelForm -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($profilesCancelForm, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativeFormWindow'
    $checks.profiles_native_cancel_preserves = (Get-FileFingerprint $profilesFile) -eq $profilesBeforeCancel

    $profilesBeforeApply = Get-FileFingerprint $profilesFile
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1029, [IntPtr]::Zero)
    $profilesApplyForm = Wait-ProcessClassWindow $process 'MDLite.NativeFormWindow'
    $checks.profiles_native_apply = $profilesApplyForm -ne [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($profilesApplyForm, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    Wait-ProcessClassWindowGone $process 'MDLite.NativeFormWindow'
    $checks.profiles_native_apply_declined_preserves = (Get-FileFingerprint $profilesFile) -eq $profilesBeforeApply

    [void][MDLiteNative]::SendMessage($findEdit, $WM_SETTEXT, [IntPtr]::Zero, 'PendingHit')
    $replaceEdit = Wait-Control $main 107 'Edit'
    [void][MDLiteNative]::SendMessage($replaceEdit, $WM_SETTEXT, [IntPtr]::Zero, 'PendingDone')
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]115, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $replaceOneSource = [IO.File]::ReadAllText($first)
    $checks.current_replace_one_saved = $replaceOneSource.Contains('PendingDone') -and
        -not $replaceOneSource.Contains('PendingHit')
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.current_replace_one_undo = [IO.File]::ReadAllText($first).Contains('PendingHit')
    [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.current_replace_one_redo = [IO.File]::ReadAllText($first).Contains('PendingDone')

    # A failed source readback must abort Undo without replacing the live
    # RichEdit input or consuming the source-history entry.
    $readbackBaselineView = (Get-WindowText $editor) -replace "`r`n", "`n"
    $readbackBaselineSource = [IO.File]::ReadAllText($first)
    $readbackSuffix = ' ReadbackPending'
    [void][MDLiteNative]::SendMessage($editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    [void][MDLiteNative]::SendMessage(
        $editor, $EM_REPLACESEL, [IntPtr]1, $readbackSuffix)
    [void][MDLiteNative]::SendMessage(
        $main, $testFailNextEditorReadbackMessage, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    $readbackAfterFailedUndo = (Get-WindowText $editor) -replace "`r`n", "`n"
    $checks.readback_failure_undo_preserves_native_input =
        $readbackAfterFailedUndo -eq ($readbackBaselineView + $readbackSuffix)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $readbackSavedSource = [IO.File]::ReadAllText($first)
    $checks.readback_failure_pending_input_saved =
        $readbackSavedSource -eq ($readbackBaselineSource + $readbackSuffix)
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.readback_failure_history_retained = [IO.File]::ReadAllText($first) -eq $readbackBaselineSource

    # The same synchronization failure must also abort Redo while preserving
    # the live input and the previously saved source.
    $redoReadbackBaselineView = (Get-WindowText $editor) -replace "`r`n", "`n"
    $redoReadbackBaselineSource = [IO.File]::ReadAllText($first)
    $redoReadbackSuffix = ' RedoReadbackPending'
    [void][MDLiteNative]::SendMessage($editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    [void][MDLiteNative]::SendMessage($editor, $EM_REPLACESEL, [IntPtr]1, $redoReadbackSuffix)
    [void][MDLiteNative]::SendMessage(
        $main, $testFailNextEditorReadbackMessage, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    $redoReadbackAfterFailure = (Get-WindowText $editor) -replace "`r`n", "`n"
    $checks.readback_failure_redo_preserves_native_input =
        $redoReadbackAfterFailure -eq ($redoReadbackBaselineView + $redoReadbackSuffix)
    $checks.readback_failure_redo_preserves_saved_source =
        [IO.File]::ReadAllText($first) -eq $redoReadbackBaselineSource
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $redoReadbackSavedSource = [IO.File]::ReadAllText($first)
    $checks.readback_failure_redo_pending_input_saved =
        $redoReadbackSavedSource -eq ($redoReadbackBaselineSource + $redoReadbackSuffix)
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.readback_failure_redo_history_recovered =
        [IO.File]::ReadAllText($first) -eq $redoReadbackBaselineSource

    # E21: failed editor readback must fail closed across save and document-lifetime routes.
    $listInitial = "presentation anchor`n- bullet`n12. ordered`n- [ ] task"
    $listFixture = Start-ReadbackFixture 'list-markers' $listInitial
    Start-Sleep -Milliseconds 150
    $bulletStart = $listInitial.IndexOf('- bullet', [StringComparison]::Ordinal)
    $orderedStart = $listInitial.IndexOf('12. ordered', [StringComparison]::Ordinal)
    $taskStart = $listInitial.IndexOf('- [ ] task', [StringComparison]::Ordinal)
    $listNativeText = Get-WindowText $listFixture.Editor
    $bulletNativeStart = $listNativeText.IndexOf('- bullet', [StringComparison]::Ordinal)
    $orderedNativeStart = $listNativeText.IndexOf('12. ordered', [StringComparison]::Ordinal)
    $taskNativeStart = $listNativeText.IndexOf('- [ ] task', [StringComparison]::Ordinal)
    $checks.list_native_text = $listNativeText
    $checks.list_native_text_length = $listNativeText.Length
    $checks.list_bullet_native_start = $bulletNativeStart
    $checks.list_ordered_native_start = $orderedNativeStart
    $checks.list_task_native_start = $taskNativeStart
    $bulletInactiveEffects = Get-CharFormatAtSourceRange -Main $listFixture.Main `
        -SourceBegin $bulletStart -SourceEnd ($bulletStart + 1)
    $taskPrefixEffects = Get-CharFormatAtSourceRange -Main $listFixture.Main `
        -SourceBegin $taskStart -SourceEnd ($taskStart + 1)
    $taskCheckboxEffects = Get-CharFormatAtSourceRange -Main $listFixture.Main `
        -SourceBegin ($taskStart + 2) -SourceEnd ($taskStart + 3)
    $checks.list_inactive_bullet_prefix_hidden =
        (($bulletInactiveEffects.mask -band $CFM_HIDDEN) -ne 0) -and
        (($bulletInactiveEffects.effects -band $CFE_HIDDEN) -ne 0)
    $checks.list_inactive_bullet_format_mask = '0x{0:X8}' -f $bulletInactiveEffects.mask
    $checks.list_inactive_bullet_format_effects = '0x{0:X8}' -f $bulletInactiveEffects.effects
    $checks.list_inactive_task_prefix_hidden =
        (($taskPrefixEffects.mask -band $CFM_HIDDEN) -ne 0) -and
        (($taskPrefixEffects.effects -band $CFE_HIDDEN) -ne 0)
    $checks.list_inactive_task_format_mask = '0x{0:X8}' -f $taskPrefixEffects.mask
    $checks.list_inactive_task_format_effects = '0x{0:X8}' -f $taskPrefixEffects.effects
    $checks.list_task_checkbox_stays_visible =
        (($taskCheckboxEffects.mask -band $CFM_HIDDEN) -ne 0) -and
        (($taskCheckboxEffects.effects -band $CFE_HIDDEN) -eq 0)
    $checks.list_task_checkbox_format_mask = '0x{0:X8}' -f $taskCheckboxEffects.mask
    $checks.list_task_checkbox_format_effects = '0x{0:X8}' -f $taskCheckboxEffects.effects

    # Cross-process caret changes are not proof of keyboard focus or EN_SELCHANGE delivery.
    $checks.list_bullet_active_prefix_visible = 'UNKNOWN: physical/native focus route not observed'
    [void][MDLiteNative]::SendMessage($listFixture.Editor, $EM_SETSEL,
        [IntPtr]$orderedNativeStart, [IntPtr]$orderedNativeStart)
    [void][MDLiteNative]::SendMessage($listFixture.Editor, $WM_KEYDOWN,
        [IntPtr]$VK_RIGHT, [IntPtr]1)
    [void][MDLiteNative]::SendMessage($listFixture.Editor, $WM_KEYUP,
        [IntPtr]$VK_RIGHT, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $listCaretPacked = [MDLiteNative]::SendMessage(
        $listFixture.Editor, $EM_GETSEL, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    $listCaretNative = [int]($listCaretPacked -band 0xFFFF)
    $checks.list_active_native_start = $listCaretNative
    $checks.list_active_native_end = [int](($listCaretPacked -shr 16) -band 0xFFFF)
    $checks.list_active_line = [MDLiteNative]::SendMessage(
        $listFixture.Editor, $EM_LINEFROMCHAR, [IntPtr]$listCaretNative,
        [IntPtr]::Zero).ToInt32()
    $orderedActiveEffects = Get-CharFormatAtSourceRange -Main $listFixture.Main `
        -SourceBegin $orderedStart -SourceEnd ($orderedStart + 1)
    $checks.list_ordered_prefix_visible =
        (($orderedActiveEffects.mask -band $CFM_HIDDEN) -ne 0) -and
        (($orderedActiveEffects.effects -band $CFE_HIDDEN) -eq 0)
    $checks.list_ordered_active_format_mask = '0x{0:X8}' -f $orderedActiveEffects.mask
    $checks.list_ordered_active_format_effects = '0x{0:X8}' -f $orderedActiveEffects.effects
    $checks.list_bullet_after_caret_move = 'UNKNOWN: physical/native focus route not observed'
    $checks.list_presentation_preserves_source = [IO.File]::ReadAllText($listFixture.Source) -eq $listInitial
    Stop-ReadbackFixture $listFixture

    $replaceFixture = Start-ReadbackFixture 'replace-current'
    $replaceFind = Wait-Control $replaceFixture.Main 105 'Edit'
    $replaceWith = Wait-Control $replaceFixture.Main 107 'Edit'
    [void][MDLiteNative]::SendMessage($replaceFind, $WM_SETTEXT, [IntPtr]::Zero, 'baseline')
    [void][MDLiteNative]::SendMessage($replaceWith, $WM_SETTEXT, [IntPtr]::Zero, 'Complete')
    $replaceBeforeHash = Get-FileFingerprint $replaceFixture.Source
    $replacePending = ' pending-replace'
    Append-PendingEditorText $replaceFixture $replacePending
    Inject-NextEditorReadbackFailure $replaceFixture
    [void][MDLiteNative]::SendMessage($replaceFixture.Main, $WM_COMMAND, [IntPtr]115, [IntPtr]::Zero)
    $checks.e21_replace_failure_keeps_disk_baseline =
        (Get-FileFingerprint $replaceFixture.Source) -eq $replaceBeforeHash
    $checks.e21_replace_failure_keeps_native_buffer =
        (Get-WindowText $replaceFixture.Editor) -eq ($replaceFixture.Initial + $replacePending)
    $checks.e21_replace_failure_skips_success_dialog =
        (Get-ProcessClassWindow $replaceFixture.Process '#32770') -eq [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($replaceFixture.Main, $WM_COMMAND, [IntPtr]115, [IntPtr]::Zero)
    $replaceExpected = $replaceFixture.Initial.Replace('baseline', 'Complete') + $replacePending
    $checks.e21_replace_retry_updates_buffer = (Get-WindowText $replaceFixture.Editor) -eq $replaceExpected
    [void][MDLiteNative]::SendMessage($replaceFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.e21_replace_retry_saves_synced_source = [IO.File]::ReadAllText($replaceFixture.Source) -eq $replaceExpected
    Stop-ReadbackFixture $replaceFixture

    $workspaceReplaceFixture = Start-ReadbackFixture 'workspace-replace' 'Needle open baseline'
    $workspaceClosedSource = Join-Path $workspaceReplaceFixture.Directory 'closed.md'
    [IO.File]::WriteAllText($workspaceClosedSource, 'Needle closed baseline', [Text.UTF8Encoding]::new($false))
    $workspaceClosedHash = Get-FileFingerprint $workspaceClosedSource
    $workspaceReplaceFind = Wait-Control $workspaceReplaceFixture.Main 105 'Edit'
    $workspaceReplaceWith = Wait-Control $workspaceReplaceFixture.Main 107 'Edit'
    [void][MDLiteNative]::SendMessage($workspaceReplaceFind, $WM_SETTEXT, [IntPtr]::Zero, 'Needle')
    [void][MDLiteNative]::SendMessage($workspaceReplaceWith, $WM_SETTEXT, [IntPtr]::Zero, 'Done')
    $workspaceOpenHash = Get-FileFingerprint $workspaceReplaceFixture.Source
    $workspacePending = ' PendingNeedle'
    Append-PendingEditorText $workspaceReplaceFixture $workspacePending
    Inject-NextEditorReadbackFailure $workspaceReplaceFixture
    [void][MDLiteNative]::PostMessage($workspaceReplaceFixture.Main, $WM_COMMAND, [IntPtr]1025, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    $workspaceReplaceDialog = Get-ProcessClassWindow $workspaceReplaceFixture.Process '#32770'
    if ($workspaceReplaceDialog -ne [IntPtr]::Zero) {
        [void][MDLiteNative]::SendMessage($workspaceReplaceDialog, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
    }
    $checks.e21_workspace_replace_failure_keeps_open_disk =
        (Get-FileFingerprint $workspaceReplaceFixture.Source) -eq $workspaceOpenHash
    $checks.e21_workspace_replace_failure_keeps_closed_disk =
        (Get-FileFingerprint $workspaceClosedSource) -eq $workspaceClosedHash
    $checks.e21_workspace_replace_failure_keeps_native_buffer =
        (Get-WindowText $workspaceReplaceFixture.Editor) -eq ($workspaceReplaceFixture.Initial + $workspacePending)
    $checks.e21_workspace_replace_failure_skips_preview =
        (Get-ProcessClassWindow $workspaceReplaceFixture.Process '#32770') -eq [IntPtr]::Zero
    Stop-ReadbackFixture $workspaceReplaceFixture

    $resizeInitial = '![asset](https://example.invalid/pixel.png) image'
    $resizeFixture = Start-ReadbackFixture 'image-resize' $resizeInitial
    $resizeBeforeHash = Get-FileFingerprint $resizeFixture.Source
    $resizeNativeBaseline = Get-WindowText $resizeFixture.Editor
    $resizePending = 'x'
    Append-PendingEditorText $resizeFixture $resizePending
    # The image starts at source offset 0. Its RichEdit projection can use an
    # object marker, so do not pass a Markdown source offset as a native offset.
    [void][MDLiteNative]::SendMessage($resizeFixture.Editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    Inject-NextEditorReadbackFailure $resizeFixture
    [void][MDLiteNative]::SendMessage($resizeFixture.Main, $WM_COMMAND, [IntPtr]1054, [IntPtr]::Zero)
    $resizeDialog = Get-ProcessClassWindow $resizeFixture.Process '#32770'
    if ($resizeDialog -ne [IntPtr]::Zero) {
        [void][MDLiteNative]::SendMessage($resizeDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    }
    $checks.e21_resize_failure_keeps_disk_baseline =
        (Get-FileFingerprint $resizeFixture.Source) -eq $resizeBeforeHash
    $checks.e21_resize_failure_keeps_native_buffer =
        (Get-WindowText $resizeFixture.Editor) -eq ($resizeNativeBaseline + $resizePending)
    $checks.e21_resize_failure_skips_image_dialog =
        (Get-ProcessClassWindow $resizeFixture.Process '#32770') -eq [IntPtr]::Zero
    [void][MDLiteNative]::SendMessage($resizeFixture.Main, $WM_COMMAND, [IntPtr]1054, [IntPtr]::Zero)
    $resizeRetryDialog = Get-ProcessClassWindow $resizeFixture.Process '#32770'
    $checks.e21_resize_retry_dialog_text = if ($resizeRetryDialog -ne [IntPtr]::Zero) {
        Get-WindowText $resizeRetryDialog
    } else { '' }
    if ($resizeRetryDialog -ne [IntPtr]::Zero) {
        [void][MDLiteNative]::SendMessage($resizeRetryDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    }
    [void][MDLiteNative]::SendMessage($resizeFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.e21_resize_retry_updates_image_source = [IO.File]::ReadAllText($resizeFixture.Source).Contains('width="320"')
    Stop-ReadbackFixture $resizeFixture

    # Resizing one image must update source through one edit transaction and
    # preserve a neighboring image whose missing asset is shown as a collapsed
    # placeholder in the native projection.
    $resizeSelectionInitial = '![missing](missing-resize-neighbor.png)![target](https://example.invalid/target.png)'
    $resizeNeighbor = '![missing](missing-resize-neighbor.png)'
    $resizeSelectionFixture = Start-ReadbackFixture 'image-resize-selection' $resizeSelectionInitial
    try {
        $resizeNeighborBegin = $resizeSelectionInitial.IndexOf($resizeNeighbor, [StringComparison]::Ordinal)
        $resizePresentationReady = $false
        $resizeReadyDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $resizeReadinessFlags = Get-EditorReadinessFlags $resizeSelectionFixture.Main
            if (($resizeReadinessFlags -band 1) -ne 0) {
                $resizePresentationReady = $true
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $resizeReadyDeadline)
        $resizeOpeningBuffer = Get-WindowText $resizeSelectionFixture.Editor
        $resizeNeighborSlotIsSpace = $resizeNeighborBegin -ge 0 -and
            $resizeNeighborBegin -lt $resizeOpeningBuffer.Length -and
            $resizeOpeningBuffer[$resizeNeighborBegin] -eq ' ' -and
            -not $resizeOpeningBuffer.Contains($resizeNeighbor)
        $resizeTargetBegin = $resizeSelectionInitial.LastIndexOf('![target]', [StringComparison]::Ordinal)
        $resizeTargetStartsAtNeighborEnd =
            $resizeTargetBegin -eq ($resizeNeighborBegin + $resizeNeighbor.Length)
        $resizeSelectionSet = [MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]$resizeTargetBegin, [IntPtr]$resizeTargetBegin).ToInt32() -eq 1
        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Main, $WM_COMMAND, [IntPtr]1054, [IntPtr]::Zero)
        $resizeSelectionDialog = Get-ProcessClassWindow $resizeSelectionFixture.Process '#32770'
        if ($resizeSelectionDialog -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage(
                $resizeSelectionDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
        }
        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $resizeSelectionSaved = [IO.File]::ReadAllText($resizeSelectionFixture.Source)
        $resizeSelection = Get-SourceSelectionByMessage $resizeSelectionFixture.Main
        $checks.resize_selection_setup_succeeded = $resizeSelectionSet
        $checks.resize_selection_target_starts_at_neighbor_end = $resizeTargetStartsAtNeighborEnd
        $checks.resize_selection_neighbor_presentation_ready = $resizePresentationReady
        $checks.resize_selection_neighbor_is_single_space_slot = $resizeNeighborSlotIsSpace
        $checks.resize_selection_keeps_neighbor_image = $resizeSelectionSaved.Contains($resizeNeighbor)
        $checks.resize_selection_updates_target_image =
            $resizeSelectionSaved.Contains('<img src="https://example.invalid/target.png" alt="target" width="320">')
        $checks.resize_selection_end_caret_restored =
            $resizeSelection.anchor -eq $resizeSelectionSaved.Length -and
            $resizeSelection.active -eq $resizeSelectionSaved.Length

        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $resizeUndoSaved = [IO.File]::ReadAllText($resizeSelectionFixture.Source)
        $resizeUndoSelection = Get-SourceSelectionByMessage $resizeSelectionFixture.Main
        $checks.resize_selection_undo_restores_source_and_caret =
            $resizeUndoSaved -ceq $resizeSelectionInitial -and
            $resizeUndoSelection.anchor -eq $resizeTargetBegin -and
            $resizeUndoSelection.active -eq $resizeTargetBegin

        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        [void][MDLiteNative]::SendMessage(
            $resizeSelectionFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $resizeRedoSaved = [IO.File]::ReadAllText($resizeSelectionFixture.Source)
        $resizeRedoSelection = Get-SourceSelectionByMessage $resizeSelectionFixture.Main
        $checks.resize_selection_redo_restores_source_and_caret =
            $resizeRedoSaved -ceq $resizeSelectionSaved -and
            $resizeRedoSelection.anchor -eq $resizeSelectionSaved.Length -and
            $resizeRedoSelection.active -eq $resizeSelectionSaved.Length
    } finally {
        Stop-ReadbackFixture $resizeSelectionFixture
    }

    $resizeHtmlInitial = '<img data-src="fallback.png" src="https://example.invalid/a&#38;b.png" alt="A &amp; B" title="keep" width="120">'
    $resizeHtmlExpected = '<img data-src="fallback.png" src="https://example.invalid/a&#38;b.png" alt="A &amp; B" title="keep" width="320">'
    $resizeHtmlFixture = Start-ReadbackFixture 'image-resize-html-attributes' $resizeHtmlInitial
    try {
        $resizeHtmlSelectionSet = [MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1
        [void][MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $WM_COMMAND, [IntPtr]1054, [IntPtr]::Zero)
        $resizeHtmlDialog = Get-ProcessClassWindow $resizeHtmlFixture.Process '#32770'
        if ($resizeHtmlDialog -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage(
                $resizeHtmlDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
        }
        [void][MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $resizeHtmlFirst = [IO.File]::ReadAllText($resizeHtmlFixture.Source)
        $resizeHtmlSecondSelectionSet = [MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1
        [void][MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $WM_COMMAND, [IntPtr]1054, [IntPtr]::Zero)
        $resizeHtmlRetryDialog = Get-ProcessClassWindow $resizeHtmlFixture.Process '#32770'
        if ($resizeHtmlRetryDialog -ne [IntPtr]::Zero) {
            [void][MDLiteNative]::SendMessage(
                $resizeHtmlRetryDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
        }
        [void][MDLiteNative]::SendMessage(
            $resizeHtmlFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $resizeHtmlSecond = [IO.File]::ReadAllText($resizeHtmlFixture.Source)
        $checks.resize_html_source_selection_set = $resizeHtmlSelectionSet
        $checks.resize_html_first_resize_preserves_attributes = $resizeHtmlFirst -ceq $resizeHtmlExpected
        $checks.resize_html_second_resize_selection_set = $resizeHtmlSecondSelectionSet
        $checks.resize_html_second_resize_is_idempotent = $resizeHtmlSecond -ceq $resizeHtmlExpected
        $checks.resize_html_entity_not_double_escaped = -not $resizeHtmlSecond.Contains('&amp;#38;')
    } finally {
        Stop-ReadbackFixture $resizeHtmlFixture
    }

    $uploadFixture = Start-ReadbackFixture 'image-upload' '![asset](https://example.invalid/pixel.png) image'
    $uploadBeforeHash = Get-FileFingerprint $uploadFixture.Source
    $uploadNativeBaseline = Get-WindowText $uploadFixture.Editor
    $uploadPending = 'x'
    Append-PendingEditorText $uploadFixture $uploadPending
    [void][MDLiteNative]::SendMessage($uploadFixture.Editor, $EM_SETSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    Inject-NextEditorReadbackFailure $uploadFixture
    [void][MDLiteNative]::SendMessage($uploadFixture.Main, $WM_COMMAND, [IntPtr]1053, [IntPtr]::Zero)
    $uploadDialog = Get-ProcessClassWindow $uploadFixture.Process '#32770'
    if ($uploadDialog -ne [IntPtr]::Zero) {
        [void][MDLiteNative]::SendMessage($uploadDialog, $WM_COMMAND, [IntPtr]$IDOK, [IntPtr]::Zero)
    }
    $checks.e21_upload_failure_keeps_disk_baseline =
        (Get-FileFingerprint $uploadFixture.Source) -eq $uploadBeforeHash
    $checks.e21_upload_failure_keeps_native_buffer =
        (Get-WindowText $uploadFixture.Editor) -eq ($uploadNativeBaseline + $uploadPending)
    $checks.e21_upload_failure_skips_trust_or_adapter_dialog = $uploadDialog -eq [IntPtr]::Zero
    Stop-ReadbackFixture $uploadFixture

    $saveFixture = Start-ReadbackFixture 'save'
    $saveFixtureBeforeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $saveFixture.Source).Hash.ToLowerInvariant()
    $saveSuffix = ' pending-save'
    Append-PendingEditorText $saveFixture $saveSuffix
    Inject-NextEditorReadbackFailure $saveFixture
    [void][MDLiteNative]::SendMessage($saveFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $saveFixtureAfterFailureHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $saveFixture.Source).Hash.ToLowerInvariant()
    $checks.e21_save_failure_keeps_disk_baseline = $saveFixtureAfterFailureHash -eq $saveFixtureBeforeHash
    $checks.e21_save_failure_keeps_editor_buffer =
        (Get-WindowText $saveFixture.Editor) -eq ($saveFixture.Initial + $saveSuffix)
    [void][MDLiteNative]::SendMessage($saveFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.e21_save_retry_writes_pending_input =
        [IO.File]::ReadAllText($saveFixture.Source) -eq ($saveFixture.Initial + $saveSuffix)
    $checks.e21_save_baseline_sha256 = $saveFixtureBeforeHash
    $checks.e21_save_failed_sha256 = $saveFixtureAfterFailureHash
    Stop-ReadbackFixture $saveFixture

    $saveAsFixture = Start-ReadbackFixture 'save-as'
    $saveAsTarget = Join-Path $saveAsFixture.Directory 'must-not-be-created.md'
    $saveAsSuffix = ' pending-save-as'
    Append-PendingEditorText $saveAsFixture $saveAsSuffix
    Inject-NextEditorReadbackFailure $saveAsFixture
    [void][MDLiteNative]::PostMessage($saveAsFixture.Main, $WM_COMMAND, [IntPtr]1006, [IntPtr]::Zero)
    $saveAsDialog = [IntPtr]::Zero
    $saveAsDeadline = [DateTime]::UtcNow.AddMilliseconds(700)
    do {
        $saveAsDialog = Get-ProcessClassWindow $saveAsFixture.Process '#32770'
        if ($saveAsDialog -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $saveAsDeadline)
    if ($saveAsDialog -ne [IntPtr]::Zero) {
        [void][MDLiteNative]::SendMessage($saveAsDialog, $WM_COMMAND, [IntPtr]$IDCANCEL, [IntPtr]::Zero)
    }
    $checks.e21_save_as_failure_skips_dialog = $saveAsDialog -eq [IntPtr]::Zero
    $checks.e21_save_as_failure_skips_target = -not (Test-Path -LiteralPath $saveAsTarget)
    $checks.e21_save_as_failure_keeps_editor_buffer =
        (Get-WindowText $saveAsFixture.Editor) -eq ($saveAsFixture.Initial + $saveAsSuffix)
    Stop-ReadbackFixture $saveAsFixture

    $untitledLifecycle = Invoke-UntitledLifecycleAcceptance
    $checks.e01_e02_untitled_lifecycle = $untitledLifecycle
    foreach ($name in $untitledLifecycle.checks.Keys) {
        $checks[$name] = [bool]$untitledLifecycle.checks[$name]
    }
    $checks.e01_e02_untitled_lifecycle_pass = [bool]$untitledLifecycle.pass

    $closeFixture = Start-ReadbackFixture 'tab-close'
    $closeSuffix = ' pending-close'
    Append-PendingEditorText $closeFixture $closeSuffix
    Inject-NextEditorReadbackFailure $closeFixture
    [void][MDLiteNative]::SendMessage($closeFixture.Main, $WM_COMMAND, [IntPtr]1004, [IntPtr]::Zero)
    $closeTabs = Find-Control $closeFixture.Main 101 'SysTabControl32'
    $checks.e21_tab_close_keeps_tab =
        $closeTabs -ne [IntPtr]::Zero -and
        [MDLiteNative]::SendMessage($closeTabs, 0x1304, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1
    $checks.e21_tab_close_keeps_editor_buffer =
        (Get-WindowText $closeFixture.Editor) -eq ($closeFixture.Initial + $closeSuffix)
    Stop-ReadbackFixture $closeFixture

    $exitFixture = Start-ReadbackFixture 'clean-exit'
    $exitSuffix = ' pending-exit'
    Append-PendingEditorText $exitFixture $exitSuffix
    Inject-NextEditorReadbackFailure $exitFixture
    [void][MDLiteNative]::PostMessage($exitFixture.Main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    $exitFixture.Process.Refresh()
    $exitDialog = Get-ProcessClassWindow $exitFixture.Process '#32770'
    $exitRecovery = Join-Path $exitFixture.Directory '.mdlite\.state\recovery'
    $checks.e21_clean_exit_keeps_process = -not $exitFixture.Process.HasExited
    $checks.e21_clean_exit_keeps_editor_buffer =
        (Get-WindowText $exitFixture.Editor) -eq ($exitFixture.Initial + $exitSuffix)
    $checks.e21_clean_exit_skips_recovery_discard_prompt = $exitDialog -eq [IntPtr]::Zero
    $checks.e21_clean_exit_writes_no_stale_recovery =
        @(Get-ChildItem -LiteralPath $exitRecovery -File -ErrorAction SilentlyContinue).Count -eq 0
    Stop-ReadbackFixture $exitFixture

    $dirtyExitFixture = Start-ReadbackFixture 'dirty-exit'
    $modelEdit = ' model-dirty'
    $pendingEdit = ' pending-exit'
    Append-PendingEditorText $dirtyExitFixture $modelEdit
    Start-Sleep -Milliseconds 450
    Append-PendingEditorText $dirtyExitFixture $pendingEdit
    Inject-NextEditorReadbackFailure $dirtyExitFixture
    [void][MDLiteNative]::PostMessage($dirtyExitFixture.Main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250
    $dirtyExitFixture.Process.Refresh()
    $dirtyExitDialog = Get-ProcessClassWindow $dirtyExitFixture.Process '#32770'
    $dirtyExitRecovery = Join-Path $dirtyExitFixture.Directory '.mdlite\.state\recovery'
    $checks.e21_dirty_exit_keeps_process = -not $dirtyExitFixture.Process.HasExited
    $checks.e21_dirty_exit_keeps_latest_editor_buffer =
        (Get-WindowText $dirtyExitFixture.Editor) -eq ($dirtyExitFixture.Initial + $modelEdit + $pendingEdit)
    $checks.e21_dirty_exit_keeps_disk_baseline =
        [IO.File]::ReadAllText($dirtyExitFixture.Source) -eq $dirtyExitFixture.Initial
    $checks.e21_dirty_exit_skips_stale_recovery_discard_prompt = $dirtyExitDialog -eq [IntPtr]::Zero
    $checks.e21_dirty_exit_writes_no_stale_recovery =
        @(Get-ChildItem -LiteralPath $dirtyExitRecovery -File -ErrorAction SilentlyContinue).Count -eq 0
    Stop-ReadbackFixture $dirtyExitFixture

    # Native projection faults are injected through the test-only WM_APP+64
    # mask. These fixtures contain synthetic Markdown only.
    $projectionWriteFixture = Start-ReadbackFixture 'projection-programmatic-write'
    try {
        $projectionWriteSuffix = ' committed-edit'
        Append-PendingEditorText $projectionWriteFixture $projectionWriteSuffix
        Start-Sleep -Milliseconds 400
        [void][MDLiteNative]::SendMessage(
            $projectionWriteFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionWriteCommitted = $projectionWriteFixture.Initial + $projectionWriteSuffix
        $projectionWriteDiskBefore = [IO.File]::ReadAllText($projectionWriteFixture.Source)
        $projectionWriteMaskAccepted = Set-NativeProjectionFailureStages `
            $projectionWriteFixture $nativeProjectionFaultProgrammaticWrite
        [void][MDLiteNative]::SendMessage(
            $projectionWriteFixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 100
        $projectionWriteRestoredView = Get-WindowText $projectionWriteFixture.Editor
        [void][MDLiteNative]::SendMessage(
            $projectionWriteFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionWriteDiskAfterFailure = [IO.File]::ReadAllText($projectionWriteFixture.Source)
        $checks.native_projection_programmatic_fault_mask_accepted = $projectionWriteMaskAccepted
        $checks.native_projection_programmatic_failure_restores_raw_source =
            $projectionWriteRestoredView -eq $projectionWriteCommitted
        $checks.native_projection_programmatic_failure_keeps_source_and_history =
            $projectionWriteDiskBefore -eq $projectionWriteCommitted -and
            $projectionWriteDiskAfterFailure -eq $projectionWriteCommitted
        $projectionWriteRetryBufferBefore = Get-WindowText $projectionWriteFixture.Editor
        $projectionWriteRetryDiskBefore = [IO.File]::ReadAllText($projectionWriteFixture.Source)
        [void][MDLiteNative]::SendMessage(
            $projectionWriteFixture.Editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $projectionWriteRetryBufferAfter = Get-WindowText $projectionWriteFixture.Editor
        $projectionWriteRetryDiskAfter = [IO.File]::ReadAllText($projectionWriteFixture.Source)
        [void][MDLiteNative]::SendMessage(
            $projectionWriteFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionWriteRetryDiskAfterSave = [IO.File]::ReadAllText($projectionWriteFixture.Source)
        $checks.native_projection_programmatic_retry_buffer_before = $projectionWriteRetryBufferBefore
        $checks.native_projection_programmatic_retry_disk_before = $projectionWriteRetryDiskBefore
        $checks.native_projection_programmatic_retry_buffer_after = $projectionWriteRetryBufferAfter
        $checks.native_projection_programmatic_retry_disk_after_undo = $projectionWriteRetryDiskAfter
        $checks.native_projection_programmatic_retry_disk_after_save = $projectionWriteRetryDiskAfterSave
        $checks.native_projection_programmatic_failure_keeps_undo_entry =
            $projectionWriteRetryBufferAfter -eq $projectionWriteFixture.Initial -and
            $projectionWriteRetryDiskBefore -eq $projectionWriteCommitted -and
            $projectionWriteRetryDiskAfter -eq $projectionWriteCommitted -and
            $projectionWriteRetryDiskAfterSave -eq $projectionWriteFixture.Initial
    } finally {
        Stop-ReadbackFixture $projectionWriteFixture
    }

    $projectionRebuildFixture = Start-ReadbackFixture 'projection-rebuild-after-edit' `
        "| name | value |`n| --- | --- |`n| row | old |"
    try {
        $projectionTableInitial = $projectionRebuildFixture.Initial
        $projectionInitialTableReady = $false
        $projectionInitialReadyDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $projectionFallbackFlags = Get-EditorReadinessFlags $projectionRebuildFixture.Main
            if (($projectionFallbackFlags -band 0x101) -eq 0x101) {
                $projectionInitialTableReady = $true
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $projectionInitialReadyDeadline)
        $projectionEditOffset = $projectionTableInitial.IndexOf('old', [StringComparison]::Ordinal)
        $projectionSelectionSet = [MDLiteNative]::SendMessage(
            $projectionRebuildFixture.Main, $testSetSelectionBySourceMessage,
            [IntPtr]$projectionEditOffset, [IntPtr]$projectionEditOffset).ToInt32() -eq 1
        $projectionRebuildMaskAccepted = Set-NativeProjectionFailureStages `
            $projectionRebuildFixture $nativeProjectionFaultRebuildWrite
        [void][MDLiteNative]::SendMessage(
            $projectionRebuildFixture.Editor, $WM_CHAR, [IntPtr][int][char]'|', [IntPtr]::Zero)
        $projectionTableExpected = $projectionTableInitial.Replace('| row | old |', '| row | |old |')
        $projectionFallbackObserved = $false
        $projectionFallbackView = ''
        $projectionFallbackFlags = 0
        $projectionFallbackDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $projectionFallbackFlags = Get-EditorReadinessFlags $projectionRebuildFixture.Main
            if (($projectionFallbackFlags -band 4) -ne 0 -and
                ($projectionFallbackFlags -band 128) -ne 0) {
                $projectionFallbackView = Get-WindowText $projectionRebuildFixture.Editor
                $projectionFallbackObserved =
                    (($projectionFallbackView -replace "`r`n", "`n" -replace "`r", "`n") -ceq $projectionTableExpected)
                break
            }
            Start-Sleep -Milliseconds 10
        } while ([DateTime]::UtcNow -lt $projectionFallbackDeadline)

        [void][MDLiteNative]::SendMessage(
            $projectionRebuildFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionRebuildSaved = [IO.File]::ReadAllText($projectionRebuildFixture.Source)
        $projectionNativeReady = $false
        $projectionNativeReadyDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $projectionFallbackFlags = Get-EditorReadinessFlags $projectionRebuildFixture.Main
            if (($projectionFallbackFlags -band 0x101) -eq 0x101) {
                $projectionNativeReady = $true
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $projectionNativeReadyDeadline)
        $checks.native_projection_rebuild_fault_mask_accepted = $projectionRebuildMaskAccepted
        $checks.native_projection_rebuild_initial_table_ready = $projectionInitialTableReady
        $checks.native_projection_rebuild_selection_set = $projectionSelectionSet
        $checks.native_projection_rebuild_failure_reaches_verified_raw_fallback =
            $projectionFallbackObserved
        $checks.native_projection_rebuild_failure_keeps_committed_source =
            $projectionRebuildSaved -ceq $projectionTableExpected
        $checks.native_projection_rebuild_failure_deferred_reprojection_recovers =
            $projectionNativeReady
        $checks.native_projection_rebuild_final_readiness_flags = $projectionFallbackFlags
        $checks.native_projection_rebuild_final_buffer = Get-WindowText $projectionRebuildFixture.Editor
        $checks.native_projection_rebuild_fallback_buffer = $projectionFallbackView
    } finally {
        Stop-ReadbackFixture $projectionRebuildFixture
    }

    $projectionQuarantineFixture = Start-ReadbackFixture 'projection-quarantine-repair'
    try {
        $projectionQuarantineSuffix = ' current-source'
        Append-PendingEditorText $projectionQuarantineFixture $projectionQuarantineSuffix
        Start-Sleep -Milliseconds 400
        [void][MDLiteNative]::SendMessage(
            $projectionQuarantineFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionQuarantineExpected = $projectionQuarantineFixture.Initial + $projectionQuarantineSuffix
        $oldProjectionEditor = $projectionQuarantineFixture.Editor
        $projectionQuarantineMaskAccepted = Set-NativeProjectionFailureStages `
            $projectionQuarantineFixture (
                $nativeProjectionFaultProgrammaticWrite -bor $nativeProjectionFaultProgrammaticRestore)
        [void][MDLiteNative]::SendMessage(
            $oldProjectionEditor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $projectionRepairEditor = Wait-VisibleControl `
            $projectionQuarantineFixture.Main 102 'RICHEDIT50W' 5000 'native projection repair'
        $projectionRepairText = Get-WindowText $projectionRepairEditor
        [void][MDLiteNative]::SendMessage(
            $projectionQuarantineFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionRepairSaved = [IO.File]::ReadAllText($projectionQuarantineFixture.Source)
        $checks.native_projection_dual_fault_mask_accepted = $projectionQuarantineMaskAccepted
        $checks.native_projection_dual_failure_recreates_editor =
            $projectionRepairEditor -ne [IntPtr]::Zero -and
            [MDLiteNative]::IsWindow($projectionRepairEditor) -and
            [MDLiteNative]::IsWindowVisible($projectionRepairEditor)
        $checks.native_projection_dual_failure_discards_stale_text =
            $projectionRepairText -eq $projectionQuarantineExpected -and
            $projectionRepairSaved -eq $projectionQuarantineExpected
        $projectionQuarantineRetryBufferBefore = Get-WindowText $projectionRepairEditor
        $projectionQuarantineRetryDiskBefore = [IO.File]::ReadAllText($projectionQuarantineFixture.Source)
        [void][MDLiteNative]::SendMessage(
            $projectionRepairEditor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $projectionQuarantineRetryBufferAfter = Get-WindowText $projectionRepairEditor
        $projectionQuarantineRetryDiskAfter = [IO.File]::ReadAllText($projectionQuarantineFixture.Source)
        [void][MDLiteNative]::SendMessage(
            $projectionQuarantineFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $projectionQuarantineRetryDiskAfterSave = [IO.File]::ReadAllText($projectionQuarantineFixture.Source)
        $checks.native_projection_dual_retry_buffer_before = $projectionQuarantineRetryBufferBefore
        $checks.native_projection_dual_retry_disk_before = $projectionQuarantineRetryDiskBefore
        $checks.native_projection_dual_retry_buffer_after = $projectionQuarantineRetryBufferAfter
        $checks.native_projection_dual_retry_disk_after_undo = $projectionQuarantineRetryDiskAfter
        $checks.native_projection_dual_retry_disk_after_save = $projectionQuarantineRetryDiskAfterSave
        $checks.native_projection_dual_failure_preserves_source_history =
            $projectionQuarantineRetryBufferAfter -eq $projectionQuarantineFixture.Initial -and
            $projectionQuarantineRetryDiskBefore -eq $projectionQuarantineExpected -and
            $projectionQuarantineRetryDiskAfter -eq $projectionQuarantineExpected -and
            $projectionQuarantineRetryDiskAfterSave -eq $projectionQuarantineFixture.Initial
    } finally {
        Stop-ReadbackFixture $projectionQuarantineFixture
    }

    $missingImageFixture = Start-ReadbackFixture 'missing-local-image-backspace' `
        'before ![asset](missing.png) after'
    try {
        $missingImageSource = $missingImageFixture.Initial
        $missingImageBegin = $missingImageSource.IndexOf('![asset](missing.png)', [StringComparison]::Ordinal)
        $missingImageReady = $false
        $missingImageReadyDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $missingImageFlags = Get-EditorReadinessFlags $missingImageFixture.Main
            if (($missingImageFlags -band 1) -ne 0) {
                $missingImageReady = $true
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $missingImageReadyDeadline)
        $missingImageOpeningBuffer = Get-WindowText $missingImageFixture.Editor
        $missingImageSlotShowsSpace = $missingImageBegin -ge 0 -and
            $missingImageBegin -lt $missingImageOpeningBuffer.Length -and
            $missingImageOpeningBuffer[$missingImageBegin] -eq ' '
        $missingImageSelectionSet = $missingImageBegin -gt 0 -and
            [MDLiteNative]::SendMessage(
                $missingImageFixture.Main, $testSetSelectionBySourceMessage,
                [IntPtr]$missingImageBegin, [IntPtr]$missingImageBegin).ToInt32() -eq 1
        if ($missingImageSelectionSet) {
            [void][MDLiteNative]::SendMessage(
                $missingImageFixture.Editor, $WM_KEYDOWN, [IntPtr]$VK_BACK, [IntPtr]::Zero)
        }
        $missingImageExpected = if ($missingImageBegin -gt 0) {
            $missingImageSource.Remove($missingImageBegin - 1, 1)
        } else {
            $missingImageSource
        }
        $missingImageExpectedBuffer = if ($missingImageBegin -gt 0 -and
            $missingImageBegin -le $missingImageOpeningBuffer.Length) {
            $missingImageOpeningBuffer.Remove($missingImageBegin - 1, 1)
        } else {
            $missingImageOpeningBuffer
        }
        [void][MDLiteNative]::SendMessage(
            $missingImageFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $missingImageSaved = [IO.File]::ReadAllText($missingImageFixture.Source)
        $missingImageBufferAfter = Get-WindowText $missingImageFixture.Editor
        $checks.missing_local_image_presentation_ready = $missingImageReady
        $checks.missing_local_image_opening_buffer = $missingImageOpeningBuffer
        $checks.missing_local_image_shows_collapsed_space_slot = $missingImageSlotShowsSpace
        $checks.missing_local_image_source_selection_set = $missingImageSelectionSet
        $checks.missing_local_image_backspace_buffer_after = $missingImageBufferAfter
        $checks.missing_local_image_backspace_buffer_matches =
            $missingImageBufferAfter -eq $missingImageExpectedBuffer
        $checks.missing_local_image_backspace_saved_source = $missingImageSaved
        $checks.missing_local_image_backspace_deletes_only_preceding_space =
            $missingImageSaved -ceq $missingImageExpected -and
            $missingImageSaved.Contains('![asset](missing.png)') -and
            -not $missingImageSaved.Contains([char]0xFFFC)
    } finally {
        Stop-ReadbackFixture $missingImageFixture
    }

    $missingImageGateFixture = Start-ReadbackFixture 'missing-local-image-readback-gate' `
        'before ![asset](missing.png) after'
    try {
        $missingImageGateInitial = $missingImageGateFixture.Initial
        $missingImageGateBegin = $missingImageGateInitial.IndexOf(
            '![asset](missing.png)', [StringComparison]::Ordinal)
        $missingImageGateReady = $false
        $missingImageGateReadyDeadline = [DateTime]::UtcNow.AddMilliseconds(5000)
        do {
            $missingImageGateFlagsBeforeEdit = Get-EditorReadinessFlags $missingImageGateFixture.Main
            if (($missingImageGateFlagsBeforeEdit -band 1) -ne 0) {
                $missingImageGateReady = $true
                break
            }
            Start-Sleep -Milliseconds 25
        } while ([DateTime]::UtcNow -lt $missingImageGateReadyDeadline)
        $missingImageGateOpeningBuffer = Get-WindowText $missingImageGateFixture.Editor
        $missingImageGateSlotSpace = $missingImageGateBegin -ge 0 -and
            $missingImageGateBegin -lt $missingImageGateOpeningBuffer.Length -and
            $missingImageGateOpeningBuffer[$missingImageGateBegin] -eq ' '
        $missingImageGateEditorEnabledBeforeEdit =
            [MDLiteNative]::IsWindowEnabled($missingImageGateFixture.Editor)
        $missingImageGateStyleBeforeEdit = [MDLiteNative]::GetWindowLongPtr(
            $missingImageGateFixture.Editor, $GWL_STYLE).ToInt64()
        $missingImageGateReadOnlyBeforeEdit =
            ($missingImageGateStyleBeforeEdit -band $ES_READONLY) -ne 0
        $missingImageGateSelectionSet = $missingImageGateBegin -gt 0 -and
            [MDLiteNative]::SendMessage(
                $missingImageGateFixture.Main, $testSetSelectionBySourceMessage,
                [IntPtr]$missingImageGateBegin, [IntPtr]$missingImageGateBegin).ToInt32() -eq 1
        $missingImageGateExpected = if ($missingImageGateBegin -gt 0) {
            $missingImageGateInitial.Insert($missingImageGateBegin, ' ')
        } else {
            $missingImageGateInitial
        }
        $missingImageGateBufferBeforeDelete = if ($missingImageGateBegin -ge 0 -and
            $missingImageGateBegin -le $missingImageGateOpeningBuffer.Length) {
            $missingImageGateOpeningBuffer.Insert($missingImageGateBegin, ' ')
        } else {
            $missingImageGateOpeningBuffer
        }
        $missingImageGateDiskBeforeFailure = [IO.File]::ReadAllText($missingImageGateFixture.Source)
        Inject-NextEditorReadbackFailure $missingImageGateFixture
        if ($missingImageGateSelectionSet) {
            [void][MDLiteNative]::SendMessage(
                $missingImageGateFixture.Editor, $WM_CHAR, [IntPtr][int][char]' ', [IntPtr]::Zero)
        }
        # Send Delete immediately after the ordinary edit's failed readback;
        # do not sleep or poll before the retry timer's 500 ms deadline.
        [void][MDLiteNative]::SendMessage(
            $missingImageGateFixture.Editor, $WM_KEYDOWN, [IntPtr]$VK_DELETE, [IntPtr]::Zero)
        $missingImageGateEditorEnabledAfterFailure =
            [MDLiteNative]::IsWindowEnabled($missingImageGateFixture.Editor)
        $missingImageGateStyleAfterFailure = [MDLiteNative]::GetWindowLongPtr(
            $missingImageGateFixture.Editor, $GWL_STYLE).ToInt64()
        $missingImageGateReadOnlyAfterFailure =
            ($missingImageGateStyleAfterFailure -band $ES_READONLY) -ne 0
        $missingImageGateFlagsAfterDelete = Get-EditorReadinessFlags $missingImageGateFixture.Main
        $missingImageGateBufferAfterDelete = Get-WindowText $missingImageGateFixture.Editor
        $missingImageGateDiskAfterDelete = [IO.File]::ReadAllText($missingImageGateFixture.Source)
        $missingImageGateDeleteBlocked =
            $missingImageGateBufferAfterDelete -eq $missingImageGateBufferBeforeDelete -and
            $missingImageGateDiskAfterDelete -eq $missingImageGateDiskBeforeFailure -and
            ($missingImageGateFlagsAfterDelete -band 4) -eq 0 -and
            ($missingImageGateFlagsAfterDelete -band (16 -bor 32)) -ne 0

        [void][MDLiteNative]::SendMessage(
            $missingImageGateFixture.Main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
        $missingImageGateEditorEnabledAfterRetry =
            [MDLiteNative]::IsWindowEnabled($missingImageGateFixture.Editor)
        $missingImageGateStyleAfterRetry = [MDLiteNative]::GetWindowLongPtr(
            $missingImageGateFixture.Editor, $GWL_STYLE).ToInt64()
        $missingImageGateReadOnlyAfterRetry =
            ($missingImageGateStyleAfterRetry -band $ES_READONLY) -ne 0
        $missingImageGateSaved = [IO.File]::ReadAllText($missingImageGateFixture.Source)
        $missingImageGateFinalBuffer = Get-WindowText $missingImageGateFixture.Editor
        $missingImageGateFinalFlags = Get-EditorReadinessFlags $missingImageGateFixture.Main
        $checks.missing_image_readback_gate_ready = $missingImageGateReady
        $checks.missing_image_readback_gate_slot_is_space = $missingImageGateSlotSpace
        $checks.missing_image_readback_gate_selection_set = $missingImageGateSelectionSet
        $checks.missing_image_readback_gate_editor_enabled_before_edit = [int]$missingImageGateEditorEnabledBeforeEdit
        $checks.missing_image_readback_gate_editor_enabled_after_failure = [int]$missingImageGateEditorEnabledAfterFailure
        $checks.missing_image_readback_gate_editor_enabled_after_retry = [int]$missingImageGateEditorEnabledAfterRetry
        $checks.missing_image_readback_gate_style_before_edit = $missingImageGateStyleBeforeEdit
        $checks.missing_image_readback_gate_style_after_failure = $missingImageGateStyleAfterFailure
        $checks.missing_image_readback_gate_style_after_retry = $missingImageGateStyleAfterRetry
        $checks.missing_image_readback_gate_readonly_before_edit = [int]$missingImageGateReadOnlyBeforeEdit
        $checks.missing_image_readback_gate_readonly_after_failure = [int]$missingImageGateReadOnlyAfterFailure
        $checks.missing_image_readback_gate_readonly_after_retry = [int]$missingImageGateReadOnlyAfterRetry
        $checks.missing_image_readback_gate_editor_is_enabled_before_edit = $missingImageGateEditorEnabledBeforeEdit
        $checks.missing_image_readback_gate_editor_is_disabled_after_failure = -not $missingImageGateEditorEnabledAfterFailure
        $checks.missing_image_readback_gate_editor_is_enabled_after_retry = $missingImageGateEditorEnabledAfterRetry
        $checks.missing_image_readback_gate_readonly_is_clear_before_edit = -not $missingImageGateReadOnlyBeforeEdit
        $checks.missing_image_readback_gate_readonly_is_set_after_failure = $missingImageGateReadOnlyAfterFailure
        $checks.missing_image_readback_gate_readonly_is_clear_after_retry = -not $missingImageGateReadOnlyAfterRetry
        $checks.missing_image_readback_gate_flags_before_edit = $missingImageGateFlagsBeforeEdit
        $checks.missing_image_readback_gate_buffer_after_failed_insert = $missingImageGateBufferBeforeDelete
        $checks.missing_image_readback_gate_disk_before_failure = $missingImageGateDiskBeforeFailure
        $checks.missing_image_readback_gate_flags_after_delete = $missingImageGateFlagsAfterDelete
        $checks.missing_image_readback_gate_buffer_after_delete = $missingImageGateBufferAfterDelete
        $checks.missing_image_readback_gate_disk_after_delete = $missingImageGateDiskAfterDelete
        $checks.missing_image_readback_gate_delete_blocked = $missingImageGateDeleteBlocked
        $checks.missing_image_readback_gate_final_flags = $missingImageGateFinalFlags
        $checks.missing_image_readback_gate_final_buffer = $missingImageGateFinalBuffer
        $checks.missing_image_readback_gate_final_source = $missingImageGateSaved
        $checks.missing_image_readback_gate_retry_saves_inserted_space =
            $missingImageGateDeleteBlocked -and
            $missingImageGateEditorEnabledBeforeEdit -and
            -not $missingImageGateEditorEnabledAfterFailure -and
            $missingImageGateEditorEnabledAfterRetry -and
            -not $missingImageGateReadOnlyBeforeEdit -and
            $missingImageGateReadOnlyAfterFailure -and
            -not $missingImageGateReadOnlyAfterRetry -and
            $missingImageGateSaved -ceq $missingImageGateExpected -and
            $missingImageGateSaved.Contains('![asset](missing.png)')
    } finally {
        Stop-ReadbackFixture $missingImageGateFixture
    }

    # Completing an image construct changes the presentation to a native
    # object.  That display-only replacement must preserve the user's native
    # Undo/Redo history and the Markdown source saved to disk.
    $beforeImage = Get-WindowText $editor
    $imageMarkup = ' ![pixel](pixel.png)'
    Send-Characters $editor $imageMarkup
    Start-Sleep -Milliseconds 900
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $imageSource = [IO.File]::ReadAllText($first)
    $checks.image_source_actual = $imageSource
    $checks.image_markup_source_saved = $imageSource.EndsWith($imageMarkup)
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $imageUndoSource = [IO.File]::ReadAllText($first)
    $checks.image_undo_source_actual = $imageUndoSource
    $checks.image_presentation_undo_saved = $imageUndoSource -ne $imageSource -and
        (($imageUndoSource -replace "`r`n", "`n").StartsWith(($beforeImage -replace "`r`n", "`n")))
    [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $checks.image_presentation_redo_saved = [IO.File]::ReadAllText($first) -eq $imageSource

    # Edit source immediately after the native image object.  Reading the
    # RichEdit surface back must expand the object to its original Markdown
    # instead of persisting U+FFFC or dropping the image.
    $imageTail = ' tail-after-image'
    [void][MDLiteNative]::SendMessage($editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
    Send-Characters $editor $imageTail
    Start-Sleep -Milliseconds 300
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $imageTailSource = [IO.File]::ReadAllText($first)
    $checks.image_object_adjacent_edit_saved = $imageTailSource -eq ($imageSource + $imageTail)
    $checks.image_object_marker_not_persisted = -not $imageTailSource.Contains([char]0xFFFC)
    for ($undoIndex = 0; $undoIndex -lt $imageTail.Length; $undoIndex++) {
        [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.image_object_adjacent_edit_undo = [IO.File]::ReadAllText($first) -eq $imageSource
    for ($redoIndex = 0; $redoIndex -lt $imageTail.Length; $redoIndex++) {
        [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    }
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.image_object_adjacent_edit_redo = [IO.File]::ReadAllText($first) -eq $imageTailSource

    # Two same-line images have no text anchor that can identify their native
    # object markers after deletion. They must remain raw Markdown through the
    # real refresh-and-save path instead of replacing just their leading '!'.
    $adjacentImageMarkup = '![one](pixel.png)![two](pixel.png)'
    $adjacentImageTransitions = [Collections.Generic.List[string]]::new()
    $adjacentImageSourceTransitions = [Collections.Generic.List[string]]::new()
    $characters = $adjacentImageMarkup.ToCharArray()
    for ($characterIndex = 0; $characterIndex -lt $characters.Length; $characterIndex++) {
        $character = $characters[$characterIndex]
        Send-Characters $editor ([string]$character)
        if ($characterIndex -in @(15, 16, 17, 32, 33)) {
            [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
            Start-Sleep -Milliseconds 150
            $adjacentImageSourceTransitions.Add([IO.File]::ReadAllText($first))
            $adjacentImageTransitions.Add((Get-WindowText $editor))
        }
    }
    Start-Sleep -Milliseconds 900
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $adjacentImageSource = [IO.File]::ReadAllText($first)
    $checks.adjacent_images_source_actual = $adjacentImageSource
    $checks.adjacent_image_transition_trace = @($adjacentImageTransitions)
    $checks.adjacent_image_source_transition_trace = @($adjacentImageSourceTransitions)
    $checks.adjacent_images_remain_raw = $adjacentImageSource -eq ($imageTailSource + $adjacentImageMarkup)
    $checks.adjacent_images_no_object_marker = -not $adjacentImageSource.Contains([char]0xFFFC)

    if (Get-Command Get-NetTCPConnection -ErrorAction SilentlyContinue) {
        $tcpEndpoints = @(Get-NetTCPConnection -OwningProcess $process.Id -ErrorAction SilentlyContinue)
        $checks.tcp_endpoint_count = $tcpEndpoints.Count
        $checks.tcp_endpoints_absent = $tcpEndpoints.Count -eq 0
    } else {
        $checks.tcp_endpoints_absent = 'NOT RUN: Get-NetTCPConnection unavailable'
    }

    # Disable autosave, wait for the real recovery timer, terminate the process,
    # then accept the recovery prompt on a workspace-only restart.
    [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not $process.WaitForExit(3000)) { throw 'Primary GUI process did not close cleanly.' }
    $process.Dispose()
    $process = $null
    $recoveryWorkspace = Join-Path $runRoot 'recovery-workspace'
    [IO.Directory]::CreateDirectory((Join-Path $recoveryWorkspace '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $recoveryWorkspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $crashFile = Join-Path $recoveryWorkspace 'crash-recovery.md'
    $crashBase = 'recovery base'
    $crashEdit = ' crash edit'
    [IO.File]::WriteAllText($crashFile, $crashBase, [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath $executable -ArgumentList @($crashFile) -PassThru
    $crashMain = Wait-ProcessWindow $process
    $crashEditor = Wait-VisibleControl $crashMain 102 'RICHEDIT50W'
    Send-Characters $crashEditor $crashEdit
    Start-Sleep -Milliseconds 5600
    $recoveryDirectory = Join-Path $recoveryWorkspace '.mdlite\.state\recovery'
    $recoveryFiles = @(Get-ChildItem -LiteralPath $recoveryDirectory -Filter '*.md' -File -ErrorAction SilentlyContinue)
    $checks.recovery_snapshot_count = $recoveryFiles.Count
    $checks.recovery_snapshot_created = @($recoveryFiles | Where-Object {
        [IO.File]::ReadAllText($_.FullName) -like ('*' + $crashBase + $crashEdit)
    }).Count -eq 1
    $process.Kill()
    [void]$process.WaitForExit(3000)
    $process.Dispose()
    $process = $null
    $checks.crash_left_source_unchanged = [IO.File]::ReadAllText($crashFile) -eq $crashBase

    $process = Start-Process -FilePath $executable -ArgumentList @($recoveryWorkspace) -PassThru
    $recoveredMain = Wait-ProcessClassWindow $process 'MDLite.MainWindow'
    [void](Wait-VisibleControl $recoveredMain 102 'RICHEDIT50W')
    $recoveryCallback = [MDLiteNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        $name = New-Object Text.StringBuilder 128
        [void][MDLiteNative]::GetClassName($window, $name, $name.Capacity)
        if ($name.ToString() -eq 'RICHEDIT50W') {
            $editorText = Get-WindowText $window
            $script:recoveryEditorTexts.Add($editorText)
            if ($editorText -eq ($crashBase + $crashEdit)) {
                $script:recoveryTextFound = $true
            }
        }
        return $true
    }
    $recoveryDeadline = [DateTime]::UtcNow.AddSeconds(10)
    do {
        $script:recoveryTextFound = $false
        $script:recoveryEditorTexts = [Collections.Generic.List[string]]::new()
        [void][MDLiteNative]::EnumChildWindows($recoveredMain, $recoveryCallback, [IntPtr]::Zero)
        if ($script:recoveryTextFound) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $recoveryDeadline)
    $checks.crash_recovery_restored = $script:recoveryTextFound
    $checks.recovery_editor_texts = @($script:recoveryEditorTexts)

    $checks['process_responding'] = $process.Responding
    $checks['executable_sha256'] = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $checks['preset'] = $Preset
    $checks['timestamp_utc'] = [DateTime]::UtcNow.ToString('o')
    $checks['evidence'] = [ordered]@{
        legacy_gui_check_groups = [ordered]@{
            source_round_trip = @('compact_edit_saved','current_replace_one_saved','image_markup_source_saved')
            image_source_selection_transaction = @('resize_selection_setup_succeeded','resize_selection_target_starts_at_neighbor_end','resize_selection_neighbor_presentation_ready','resize_selection_neighbor_is_single_space_slot','resize_selection_keeps_neighbor_image','resize_selection_updates_target_image','resize_selection_end_caret_restored','resize_selection_undo_restores_source_and_caret','resize_selection_redo_restores_source_and_caret','resize_html_source_selection_set','resize_html_first_resize_preserves_attributes','resize_html_second_resize_selection_set','resize_html_second_resize_is_idempotent','resize_html_entity_not_double_escaped')
            save_boundary_and_recovery = @('compact_edit_saved','settings_native_cancel_preserves','profiles_native_cancel_preserves','recovery_snapshot_created','crash_recovery_restored')
            undo_redo_and_readback_recovery = @('compact_undo_saved','compact_redo_saved','current_replace_one_undo','current_replace_one_redo','image_presentation_undo_saved','image_presentation_redo_saved','readback_failure_undo_preserves_native_input','readback_failure_history_retained','readback_failure_redo_preserves_native_input','readback_failure_redo_history_recovered')
            paint_layout_reentry = @('paint_layout_reentry')
            workspace_navigation_and_picker = @('workspace_tree_items','explorer_tree_uses_icons','explorer_tree_hides_legacy_lines','explorer_tree_expand_select','outline_tree_items','calendar_visible','quick_open_picker','command_palette_picker','native_tab_owner_draw_style','native_tab_close_action','native_tab_keyboard_selection')
            suspended_editor_lifecycle = @('suspended_editor_lifecycle_pass')
            native_projection_failure_recovery = @('native_projection_programmatic_fault_mask_accepted','native_projection_programmatic_failure_restores_raw_source','native_projection_programmatic_failure_keeps_source_and_history','native_projection_programmatic_failure_keeps_undo_entry','native_projection_rebuild_fault_mask_accepted','native_projection_rebuild_selection_set','native_projection_rebuild_failure_reaches_verified_raw_fallback','native_projection_rebuild_failure_keeps_committed_source','native_projection_rebuild_failure_deferred_reprojection_recovers','native_projection_dual_fault_mask_accepted','native_projection_dual_failure_recreates_editor','native_projection_dual_failure_discards_stale_text','native_projection_dual_failure_preserves_source_history')
            negative_size_delta_missing_image = @('missing_local_image_presentation_ready','missing_local_image_shows_collapsed_space_slot','missing_local_image_source_selection_set','missing_local_image_backspace_buffer_matches','missing_local_image_backspace_deletes_only_preceding_space')
            missing_image_readback_failure_gate = @('missing_image_readback_gate_ready','missing_image_readback_gate_slot_is_space','missing_image_readback_gate_selection_set','missing_image_readback_gate_editor_is_enabled_before_edit','missing_image_readback_gate_editor_is_disabled_after_failure','missing_image_readback_gate_editor_is_enabled_after_retry','missing_image_readback_gate_readonly_is_clear_before_edit','missing_image_readback_gate_readonly_is_set_after_failure','missing_image_readback_gate_readonly_is_clear_after_retry','missing_image_readback_gate_delete_blocked','missing_image_readback_gate_retry_saves_inserted_space')
            workspace_session_view_restore = @('workspace_session_view_restore_pass')
            list_presentation = @('list_inactive_bullet_prefix_hidden','list_inactive_task_prefix_hidden','list_ordered_prefix_visible','list_task_checkbox_stays_visible','list_presentation_preserves_source','list_bullet_active_prefix_visible','list_bullet_after_caret_move')
            pending_editor_readback_save_boundary = @('e21_replace_failure_keeps_disk_baseline','e21_replace_failure_keeps_native_buffer','e21_replace_failure_skips_success_dialog','e21_replace_retry_updates_buffer','e21_replace_retry_saves_synced_source','e21_workspace_replace_failure_keeps_open_disk','e21_workspace_replace_failure_keeps_closed_disk','e21_workspace_replace_failure_keeps_native_buffer','e21_workspace_replace_failure_skips_preview','e21_resize_failure_keeps_disk_baseline','e21_resize_failure_keeps_native_buffer','e21_resize_failure_skips_image_dialog','e21_resize_retry_updates_image_source','e21_upload_failure_keeps_disk_baseline','e21_upload_failure_keeps_native_buffer','e21_upload_failure_skips_trust_or_adapter_dialog','e21_save_failure_keeps_disk_baseline','e21_save_failure_keeps_editor_buffer','e21_save_retry_writes_pending_input','e21_save_as_failure_skips_dialog','e21_save_as_failure_skips_target','e21_save_as_failure_keeps_editor_buffer','e21_tab_close_keeps_tab','e21_tab_close_keeps_editor_buffer','e21_clean_exit_keeps_process','e21_clean_exit_keeps_editor_buffer','e21_clean_exit_skips_recovery_discard_prompt','e21_clean_exit_writes_no_stale_recovery','e21_dirty_exit_keeps_process','e21_dirty_exit_keeps_latest_editor_buffer','e21_dirty_exit_keeps_disk_baseline','e21_dirty_exit_skips_stale_recovery_discard_prompt','e21_dirty_exit_writes_no_stale_recovery')
            untitled_recovery_and_save_retry = @('multiple_untitled_tabs_created','first_untitled_buffer_restored','second_untitled_buffer_restored','recovery_snapshots_preserve_both_buffers','untitled_documents_do_not_create_workspace_files','save_opens_save_as_for_untitled','save_as_cancel_keeps_buffer_and_recovery','editing_resumes_after_save_cancel','save_as_retry_writes_exact_pending_buffer','saved_tab_remains_independent_of_other_untitled','recovery_snapshot_removed_only_for_saved_tab')
        }
        current_task_crosswalk = [ordered]@{
            E01_multi_untitled_recovery = @('multiple_untitled_tabs_created','first_untitled_buffer_restored','second_untitled_buffer_restored','recovery_snapshots_preserve_both_buffers','untitled_documents_do_not_create_workspace_files')
            E02_save_cancel_resume_retry = @('save_opens_save_as_for_untitled','save_as_cancel_keeps_buffer_and_recovery','editing_resumes_after_save_cancel','save_as_retry_writes_exact_pending_buffer')
            E05_one_character_undo_redo = @('e05_english_fast_pass','e05_english_slow_pass','e05_middle_insert_save_undo_redo_pass')
            native_table_topology_replacement = @('table_topology_full_document_pass','table_topology_row_cell_topology_pass')
            E05_physical_keys = $checks.e05_physical_key_evidence
            suspended_editor_lifecycle = $checks.suspended_editor_lifecycle
            workspace_session_view_restore = $checks.workspace_session_view_restore
            E20_git_action_responsiveness = $checks.e20_git_action_responsiveness
            E25_palette_settings_route = 'Synthetic native checks: picker filter, settings-form route, Escape cancellation, directional source-selection restoration, editor focus, buffer and disk preservation. Physical keyboard and full customization remain open.'
            R16_diagnostics = @('diagnostics_palette_filter_single','diagnostics_dialog_title','diagnostics_view_capture','diagnostics_refresh_button','diagnostics_refresh_keeps_dialog_open','diagnostics_refresh_updates_snapshot')
            R16_general_update_source = 'BLOCKED_SOURCE_UNDEFINED: no product update channel is specified; this artifact does not claim a general updater.'
            R16_dependency_update = 'PARTIAL_MANUAL_PROCEDURE: libwebp version, official source URL, and SHA-256 are pinned in docs/dependencies.md; no automatic update is configured.'
        }
    }
    $checks['evidence_boundary'] = 'Automated native observation only; UNKNOWN and NOT_OBSERVED remain open.'
    $failedChecks = @($checks.GetEnumerator() | Where-Object {
        $_.Value -is [bool] -and -not $_.Value
    })
    $checks['pass'] = $failedChecks.Count -eq 0
    $result = [pscustomobject]$checks
    $directory = Split-Path -Parent $OutputPath
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $result | ConvertTo-Json -Depth 8
    if (-not $result.pass) { exit 1 }
}
finally {
    if ($process -and -not $process.HasExited) {
        $process.Refresh()
        $main = $process.MainWindowHandle
        if ($main -ne [IntPtr]::Zero) { [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
        if (-not $process.WaitForExit(3000)) { $process.Kill() }
        $process.Dispose()
    }
    foreach ($fixtureProcess in $readbackProcesses) {
        try {
            $fixtureProcess.Refresh()
            if (-not $fixtureProcess.HasExited) {
                $fixtureProcess.Kill()
                [void]$fixtureProcess.WaitForExit(3000)
            }
            $fixtureProcess.Dispose()
        } catch { }
    }
    $transactionStageLogPath = Join-Path $runRoot 'transactional-table-presentation-failure\transactional-table-presentation-stages.log'
    if (Test-Path -LiteralPath $transactionStageLogPath -PathType Leaf) {
        $transactionStageLogArtifactPath = [IO.Path]::ChangeExtension($OutputPath, '.transaction.log')
        if (-not [IO.Path]::IsPathRooted($transactionStageLogArtifactPath)) {
            $transactionStageLogArtifactPath = Join-Path $repoRoot $transactionStageLogArtifactPath
        }
        [IO.Directory]::CreateDirectory((Split-Path -Parent $transactionStageLogArtifactPath)) | Out-Null
        Copy-Item -LiteralPath $transactionStageLogPath -Destination $transactionStageLogArtifactPath -Force
    }
    if (Test-Path -LiteralPath $runRoot) { Remove-Item -LiteralPath $runRoot -Recurse -Force }
    if ($null -eq $previousCrashUiSetting) {
        Remove-Item Env:MDLITE_TEST_NO_CRASH_UI -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_NO_CRASH_UI = $previousCrashUiSetting
    }
    if ($null -eq $previousSilentSetting) {
        Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_SILENT = $previousSilentSetting
    }
}
