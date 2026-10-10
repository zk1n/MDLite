#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug','release')]
    [string]$Preset = 'debug',
    [ValidateSet('full','search-panel-s1-s3')]
    [string]$Scenario = 'full',
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$verificationRoot = Join-Path $repoRoot 'build\verification'
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0,8)
$runRoot = Join-Path $verificationRoot "search-quality-$Preset-$runId"
$workspace = Join-Path $runRoot 'workspace'
$screenshots = Join-Path $runRoot 'screenshots'
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
$receiptPath = Join-Path $verificationRoot "build-receipt-$Preset.json"
$script:runProgressPath = Join-Path $verificationRoot "search-quality-progress-$Preset-$runId.txt"
$script:runStartedAt = [DateTime]::UtcNow
$script:nextRunBudgetCheckUtc = [DateTime]::MinValue
$script:runBudgetExceeded = $false
$script:maxRunSeconds = 120
$script:maxWorkingSetBytes = 512MB
$script:currentTestStage = 'preflight startup'
[IO.Directory]::CreateDirectory($verificationRoot) | Out-Null
[IO.File]::WriteAllText($script:runProgressPath, "RUN_START utc=$($script:runStartedAt.ToString('o')) scenario=$Scenario preset=$Preset`r`n", [Text.UTF8Encoding]::new($false))

function Write-RunBreadcrumb([string]$Message) {
    $safeMessage = $Message -replace '[\r\n]+', ' '
    [IO.File]::AppendAllText($script:runProgressPath, "utc=$([DateTime]::UtcNow.ToString('o')) $safeMessage`r`n", [Text.UTF8Encoding]::new($false))
}
function Assert-RunBudget([string]$Stage) {
    if ($script:runBudgetExceeded) { return }
    $now = [DateTime]::UtcNow
    if ($now -lt $script:nextRunBudgetCheckUtc) { return }
    $script:nextRunBudgetCheckUtc = $now.AddMilliseconds(250)
    $elapsed = ($now - $script:runStartedAt).TotalSeconds
    $currentProcess = [Diagnostics.Process]::GetCurrentProcess()
    try { $currentProcess.Refresh(); $workingSet = $currentProcess.WorkingSet64 }
    finally { $currentProcess.Dispose() }
    if ($elapsed -ge $script:maxRunSeconds -or $workingSet -ge $script:maxWorkingSetBytes) {
        $reason = if ($elapsed -ge $script:maxRunSeconds) { 'elapsed_seconds' } else { 'working_set_bytes' }
        $script:runBudgetExceeded = $true
        $script:currentTestStage = "RUN_BUDGET_STOP stage=$Stage reason=$reason elapsed_seconds=$([Math]::Round($elapsed,2)) working_set_bytes=$workingSet"
        Write-RunBreadcrumb $script:currentTestStage
        throw "Harness safety bound exceeded: $($script:currentTestStage)"
    }
}
function Get-FileSha256([string]$Path) {
    $hasher = [Security.Cryptography.SHA256]::Create()
    $stream = $null
    try {
        $stream = [IO.File]::OpenRead($Path)
        [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace('-','').ToLowerInvariant()
    } finally {
        try { if ($null -ne $stream) { $stream.Dispose() } }
        finally { $hasher.Dispose() }
    }
}

$script:currentTestStage = 'preflight STA check'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
if ([Threading.Thread]::CurrentThread.ApartmentState -ne 'STA') {
    Write-RunBreadcrumb "PREFLIGHT_FAILED stage=$script:currentTestStage reason=not_sta"
    throw 'Run with powershell.exe -STA -File.'
}
Write-RunBreadcrumb 'STAGE preflight loading drawing assemblies'
try {
    Add-Type -AssemblyName System.Drawing
    Add-Type -AssemblyName System.Windows.Forms
} catch {
    Write-RunBreadcrumb "PREFLIGHT_FAILED stage=load_drawing_assemblies message=$($_.Exception.Message)"
    throw
}
$script:currentTestStage = 'preflight required build files'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    Write-RunBreadcrumb "PREFLIGHT_FAILED stage=$script:currentTestStage missing_executable=$executable"
    throw "Build first: $executable"
}
if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
    Write-RunBreadcrumb "PREFLIGHT_FAILED stage=$script:currentTestStage missing_receipt=$receiptPath"
    throw "Build receipt missing: $receiptPath"
}

function Get-SourceHash {
    $prefix = [IO.Path]::GetFullPath($repoRoot).TrimEnd([char[]]@('\','/')) + [IO.Path]::DirectorySeparatorChar
    [string[]]$paths = @((Get-ChildItem (Join-Path $repoRoot 'src') -Recurse -File).FullName) + @(
        (Join-Path $repoRoot 'CMakeLists.txt'), (Join-Path $repoRoot 'CMakePresets.json'))
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $manifest = foreach ($path in $paths) {
        '{0}:{1}' -f $path.Substring($prefix.Length).Replace('\','/'),
            (Get-FileSha256 -Path $path)
    }
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        [BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes(
            [string]::Join("`n", [string[]]$manifest)))).Replace('-','').ToLowerInvariant()
    } finally { $hash.Dispose() }
}

$script:currentTestStage = 'preflight reading build receipt'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
$receipt = Get-Content -Raw -LiteralPath $receiptPath | ConvertFrom-Json
$script:currentTestStage = 'preflight executable SHA-256'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
$exeHash = Get-FileSha256 -Path $executable
$script:currentTestStage = 'preflight source SHA-256'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
$sourceHash = Get-SourceHash
$script:currentTestStage = 'preflight receipt and source validation'
Write-RunBreadcrumb "STAGE $script:currentTestStage"
if ($receipt.build_status -ne 'PASS' -or $receipt.executable_sha256 -ne $exeHash -or
    $receipt.source_sha256 -ne $sourceHash) {
    Write-RunBreadcrumb "PREFLIGHT_FAILED stage=$script:currentTestStage receipt_status=$($receipt.build_status) exe_match=$($receipt.executable_sha256 -ceq $exeHash) source_match=$($receipt.source_sha256 -ceq $sourceHash)"
    throw 'Build receipt, executable, and current source do not match.'
}

if (-not $OutputPath) { $OutputPath = Join-Path $runRoot 'search-quality-acceptance.json' }
[IO.Directory]::CreateDirectory($workspace) | Out-Null
[IO.Directory]::CreateDirectory($screenshots) | Out-Null

$previousSilent = $env:MDLITE_TEST_SILENT
$previousCrash = $env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_SILENT = '1'
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$checks = [Collections.Generic.List[object]]::new()
$frames = [Collections.Generic.List[object]]::new()
$keyInputEvidence = [Collections.Generic.List[object]]::new()
$script:ownedModalEvidence = [Collections.Generic.List[object]]::new()
$script:lastOwnedWindowWaitEvidence = $null
$process = $null
$main = [IntPtr]::Zero
$blocked = $null
$controlledFailure = $null
$missingControlId = $null
$ownedChildWindowsOnMissing = @()
$cleanup = 'NOT_STARTED'
$cleanupEvidence = $null
$currentTestStage = 'fixture startup'
$lastTextReadDiagnostic = $null
$scriptStableHash = $null

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Text;
using System.Runtime.InteropServices;
using System.Runtime.CompilerServices;

public sealed class MDLiteReferenceEqualityComparer : IEqualityComparer<object> {
    public new bool Equals(object left, object right) { return Object.ReferenceEquals(left, right); }
    public int GetHashCode(object value) { return RuntimeHelpers.GetHashCode(value); }
}

public static class MDLiteSearchNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    public delegate bool EnumChildWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] public struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] public struct GUIINFO {
        public int cbSize; public uint flags;
        public IntPtr active, focus, capture, menu, moveSize, caret;
        public RECT caretRect;
    }
    [StructLayout(LayoutKind.Sequential)] public struct KEYBDINPUT {
        public ushort wVk, wScan; public uint dwFlags, time; public IntPtr dwExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] public struct MOUSEINPUT {
        public int dx, dy; public uint mouseData, dwFlags, time; public IntPtr dwExtraInfo;
    }
    [StructLayout(LayoutKind.Explicit, Size=32)] public struct INPUT_UNION {
        [FieldOffset(0)] public KEYBDINPUT ki;
        [FieldOffset(0)] public MOUSEINPUT mi;
    }
    [StructLayout(LayoutKind.Sequential)] public struct INPUT {
        public uint type; public INPUT_UNION U;
    }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("kernel32.dll")] public static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("kernel32.dll", EntryPoint="SetLastError", SetLastError=true)] public static extern void SetLastError(uint error);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wp, IntPtr lp,
        uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="PostMessageW", SetLastError=true)]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wp, IntPtr lp);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr SendMessageTimeoutText(IntPtr window, uint message, IntPtr wp, string lp,
        uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
    public static extern IntPtr SendMessageTimeoutTextRead(IntPtr window, uint message, IntPtr wp,
        [Out] StringBuilder lp, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern IntPtr WindowFromPoint(POINT point);
    [DllImport("user32.dll")] public static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern bool GetCursorPos(out POINT point);
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr after,
        int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool BringWindowToTop(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] public static extern IntPtr SetFocus(IntPtr window);
    [DllImport("user32.dll")] public static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("user32.dll")] public static extern bool GetGUIThreadInfo(uint thread, ref GUIINFO info);
    [DllImport("user32.dll")] public static extern short GetAsyncKeyState(int key);
    [DllImport("user32.dll", SetLastError=true)] public static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetParent(IntPtr window);
    public static bool PostScalar(IntPtr window, uint message, long wp, long lp) {
        return PostMessage(window, message, new IntPtr(wp), new IntPtr(lp));
    }
    public static long SendScalar(IntPtr window, uint message, long wp, long lp, uint timeout=5000) {
        IntPtr result;
        if (SendMessageTimeout(window, message, new IntPtr(wp), new IntPtr(lp), 2, timeout, out result) == IntPtr.Zero)
            throw new InvalidOperationException("SendMessageTimeout failed: hwnd=" + window.ToInt64() +
                " message=0x" + message.ToString("X") + " error=" + Marshal.GetLastWin32Error());
        return result.ToInt64();
    }
    public static void SetText(IntPtr window, string value) {
        IntPtr result;
        if (SendMessageTimeoutText(window, 0x000C, IntPtr.Zero, value, 2, 5000, out result) == IntPtr.Zero)
            throw new InvalidOperationException("WM_SETTEXT timed out: " + Marshal.GetLastWin32Error());
    }
    public static string Text(IntPtr window) {
        // GetWindowText only exposes a foreign top-level caption; use bounded system text messages for child controls.
        const int maxTextLength = 16384;
        const uint timeoutMilliseconds = 1500;
        long length = SendScalar(window, 0x000E, 0, 0, timeoutMilliseconds);
        if (length < 0 || length > maxTextLength)
            throw new InvalidOperationException("WM_GETTEXTLENGTH exceeded the harness limit: hwnd=" +
                window.ToInt64() + " length=" + length);
        var buffer = new StringBuilder((int)length + 1);
        IntPtr result;
        if (SendMessageTimeoutTextRead(window, 0x000D, new IntPtr(length + 1), buffer, 2,
                                       timeoutMilliseconds, out result) == IntPtr.Zero)
            throw new InvalidOperationException("WM_GETTEXT timed out: hwnd=" + window.ToInt64() +
                " error=" + Marshal.GetLastWin32Error());
        return buffer.ToString();
    }
    public static IntPtr MakePoint(int x, int y) {
        uint packed = (uint)(ushort)(short)x | ((uint)(ushort)(short)y << 16);
        return new IntPtr(unchecked((int)packed));
    }
    public static bool FocusTarget(IntPtr target, out IntPtr topLevel, out IntPtr observedFocus) {
        topLevel = target;
        IntPtr parent;
        while ((parent = GetParent(topLevel)) != IntPtr.Zero) topLevel = parent;
        uint pid; uint targetThread = GetWindowThreadProcessId(topLevel, out pid);
        uint currentThread = GetCurrentThreadId(); bool attached = false;
        if (targetThread != 0 && targetThread != currentThread)
            attached = AttachThreadInput(currentThread, targetThread, true);
        BringWindowToTop(topLevel); SetForegroundWindow(topLevel);
        bool foreground = GetForegroundWindow() == topLevel;
        if (foreground && (attached || targetThread == currentThread)) SetFocus(target);
        if (attached && !AttachThreadInput(currentThread, targetThread, false))
            throw new InvalidOperationException("Could not detach the focus setup input queue.");
        var info = new GUIINFO(); info.cbSize = Marshal.SizeOf(info);
        bool observed = GetGUIThreadInfo(targetThread, ref info); observedFocus = info.focus;
        return foreground && observed && observedFocus == target;
    }
    public static INPUT Key(ushort vk, bool up) {
        return new INPUT { type=1, U=new INPUT_UNION { ki=new KEYBDINPUT { wVk=vk, dwFlags=up ? 2u : 0u } } };
    }
    public static INPUT MouseButton(bool down) {
        return new INPUT { type=0, U=new INPUT_UNION { mi=new MOUSEINPUT { dwFlags=down ? 2u : 4u } } };
    }
}

public static class SearchOwnedSelection {
    [StructLayout(LayoutKind.Sequential)] struct TreeItem {
        public uint Mask; public IntPtr Item; public uint State, StateMask;
        public IntPtr Text; public int TextMax, Image, SelectedImage, Children; public IntPtr Parameter;
    }
    public sealed class Result { public int Begin, End; public string Text; public uint OwnerPid; public int RemoteBytes; }
    [DllImport("user32.dll", SetLastError=true)] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] static extern int GetClassName(IntPtr h, StringBuilder text, int size);
    [DllImport("user32.dll")] static extern bool IsWindow(IntPtr h);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)]
    static extern IntPtr Send(IntPtr h, uint message, IntPtr w, IntPtr l, uint flags, uint timeout, out IntPtr reply);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint rights, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr VirtualAllocEx(IntPtr process, IntPtr address, UIntPtr bytes, uint allocation, uint protection);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool VirtualFreeEx(IntPtr process, IntPtr address, UIntPtr bytes, uint type);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool ReadProcessMemory(IntPtr process, IntPtr address, byte[] bytes, UIntPtr count, out UIntPtr read);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool WriteProcessMemory(IntPtr process, IntPtr address, byte[] bytes, UIntPtr count, out UIntPtr written);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr process);
    static void Verify(IntPtr h, uint expected) {
        uint actual; var name = new StringBuilder(64);
        if (expected == 0 || !IsWindow(h) || GetWindowThreadProcessId(h, out actual) == 0 || actual != expected ||
            GetClassName(h, name, 64) == 0 || !name.ToString().StartsWith("RICHEDIT", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Owned RichEdit identity mismatch");
    }
    static IntPtr Scalar(IntPtr h, uint message) {
        IntPtr reply;
        if (Send(h, message, IntPtr.Zero, IntPtr.Zero, 2, 1500, out reply) == IntPtr.Zero)
            throw new InvalidOperationException("Owned RichEdit scalar read failed: " + Marshal.GetLastWin32Error());
        return reply;
    }
    static void VerifyTree(IntPtr h, uint expectedPid) {
        uint actual; var name = new StringBuilder(64);
        if (expectedPid == 0 || !IsWindow(h) || GetWindowThreadProcessId(h, out actual) == 0 || actual != expectedPid ||
            GetClassName(h, name, 64) == 0 || !String.Equals(name.ToString(), "SysTreeView32", StringComparison.OrdinalIgnoreCase))
            throw new InvalidOperationException("Owned TreeView identity mismatch");
    }
    public static long ReadTreeParameter(IntPtr h, IntPtr item, uint expectedPid) {
        VerifyTree(h, expectedPid);
        if (IntPtr.Size != 8 || item == IntPtr.Zero) throw new InvalidOperationException("Tree fixture requires an x64 item");
        int size = Marshal.SizeOf(typeof(TreeItem));
        int itemOffset = Marshal.OffsetOf(typeof(TreeItem), "Item").ToInt32();
        int parameterOffset = Marshal.OffsetOf(typeof(TreeItem), "Parameter").ToInt32();
        if (size != 56 || itemOffset != 8 || parameterOffset != 48) throw new InvalidOperationException("Unexpected TVITEMW layout");
        var bytes = new byte[size];
        Array.Copy(BitConverter.GetBytes(4u), 0, bytes, 0, 4); // TVIF_PARAM only; no text pointer.
        Array.Copy(BitConverter.GetBytes(item.ToInt64()), 0, bytes, itemOffset, 8);
        IntPtr process = OpenProcess(0x38, false, expectedPid), remote = IntPtr.Zero;
        bool started = false, completed = false;
        if (process == IntPtr.Zero) throw new InvalidOperationException("OpenProcess owned TreeView failed");
        try {
            remote = VirtualAllocEx(process, IntPtr.Zero, (UIntPtr)size, 0x3000, 4);
            if (remote == IntPtr.Zero) throw new InvalidOperationException("Owned TreeView allocation failed");
            UIntPtr transferred;
            if (!WriteProcessMemory(process, remote, bytes, (UIntPtr)size, out transferred) || transferred.ToUInt64() != (ulong)size)
                throw new InvalidOperationException("Owned TreeView request write failed");
            VerifyTree(h, expectedPid);
            IntPtr reply; started = true;
            if (Send(h, 0x113e, IntPtr.Zero, remote, 2, 1500, out reply) == IntPtr.Zero)
                throw new InvalidOperationException("Owned TreeView read uncertain; remote allocation retained until fixture exit");
            completed = true;
            VerifyTree(h, expectedPid);
            if (reply == IntPtr.Zero) throw new InvalidOperationException("Owned TreeView item no longer exists");
            if (!ReadProcessMemory(process, remote, bytes, (UIntPtr)size, out transferred) || transferred.ToUInt64() != (ulong)size)
                throw new InvalidOperationException("Owned TreeView parameter readback failed");
            return BitConverter.ToInt64(bytes, parameterOffset);
        } finally {
            if (remote != IntPtr.Zero && (!started || completed)) VirtualFreeEx(process, remote, UIntPtr.Zero, 0x8000);
            CloseHandle(process);
        }
    }
    public static Result Read(IntPtr h, uint expectedPid) {
        Verify(h, expectedPid);
        long length = Scalar(h, 0x000e).ToInt64();
        if (length < 0 || length > 16384) throw new InvalidOperationException("Selection fixture exceeds small-text bound");
        long packed = Scalar(h, 0x00b0).ToInt64();
        if (packed == -1 || unchecked((uint)packed) == uint.MaxValue)
            throw new InvalidOperationException("EM_GETSEL range cannot be represented");
        int begin = (int)(packed & 0xffff), end = (int)((packed >> 16) & 0xffff);
        if (end < begin || end - begin > 16383) throw new InvalidOperationException("Selection exceeds bounded remote buffer");
        var result = new Result { Begin = begin, End = end, OwnerPid = expectedPid, Text = "", RemoteBytes = 0 };
        if (begin == end) return result;
        // EM_GETSELTEXT has no size parameter; reserve the full bound so a queued
        // selection change within this small fixture cannot overrun a word buffer.
        const int capacityChars = 32768, bytes = 65536;
        IntPtr process = OpenProcess(0x38, false, expectedPid), remote = IntPtr.Zero;
        bool pointerMessageStarted = false, pointerMessageCompleted = false;
        if (process == IntPtr.Zero) throw new InvalidOperationException("OpenProcess owned selection failed");
        try {
            remote = VirtualAllocEx(process, IntPtr.Zero, (UIntPtr)bytes, 0x3000, 4);
            if (remote == IntPtr.Zero) throw new InvalidOperationException("Owned selection allocation failed");
            Verify(h, expectedPid);
            if (Scalar(h, 0x00b0).ToInt64() != packed)
                throw new InvalidOperationException("Owned selection changed during read");
            IntPtr reply;
            IntPtr sendResult;
            pointerMessageStarted = true;
            try { sendResult = Send(h, 0x043e, IntPtr.Zero, remote, 2, 1500, out reply); }
            catch (Exception error) {
                throw new InvalidOperationException("Owned selected-text send outcome is uncertain; remote buffer retained until owner process exit. " + error.Message, error);
            }
            pointerMessageCompleted = sendResult != IntPtr.Zero;
            if (!pointerMessageCompleted)
                throw new InvalidOperationException("Owned selected-text send timed out; remote buffer retained until owner process exit. error=" + Marshal.GetLastWin32Error());
            Verify(h, expectedPid);
            if (Scalar(h, 0x00b0).ToInt64() != packed)
                throw new InvalidOperationException("Owned selection changed after text read");
            long count = reply.ToInt64();
            if (count < 0 || count >= capacityChars)
                throw new InvalidOperationException("Owned selected-text count exceeds allocation");
            var data = new byte[(int)count * 2]; UIntPtr read;
            if (data.Length > 0 && (!ReadProcessMemory(process, remote, data, (UIntPtr)data.Length, out read) ||
                                    read.ToUInt64() != (ulong)data.Length))
                throw new InvalidOperationException("Owned selected-text readback failed");
            result.Text = Encoding.Unicode.GetString(data); result.RemoteBytes = bytes;
            return result;
        } finally {
            if (remote != IntPtr.Zero && (!pointerMessageStarted || pointerMessageCompleted))
                VirtualFreeEx(process, remote, UIntPtr.Zero, 0x8000);
            CloseHandle(process);
        }
    }
}
'@

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_SIZE = 0x0005
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_IME_ESCAPE = 0x0000
$WM_USER = 0x0400
$EN_CHANGE = 0x0300
$BN_CLICKED = 0
$CBN_SELCHANGE = 1
$TVM_GETCOUNT = 0x1105
$TVM_GETNEXTITEM = 0x110A
$TVM_SELECTITEM = 0x110B
$TVGN_ROOT = 0
$TVGN_NEXT = 1
$TVGN_CHILD = 4
$TVGN_CARET = 9
$LB_GETCOUNT = 0x018B
$LB_SETCURSEL = 0x0186
$kWorkspaceTree = 100
$kTabs = 101
$kEditor = 102
$kFindEdit = 105
$kFindNext = 106
$kReplaceEdit = 107
$kFindRegex = 111
$kFindInclude = 113
$kFindExclude = 114
$kReplaceOne = 115
$kReplaceDocument = 116
$kFindResults = 117
$kActivityExplorer = 123
$kActivitySearch = 124
$kPanelHeaderExplorer = 1072
$kSearchScope = 135
$kSearchPrevious = 136
$kSearchReplaceToggle = 137
$kSearchDetailsToggle = 138
$kSearchIgnore = 139
$kSearchHidden = 140
$kSearchClose = 141
$kSearchTarget = 142
$kSearchCount = 143
$kSearchStatus = 144
$kRollbackWorkspaceReplace = 145
$kEditFind = 1022
$kEditFindNext = 1023
$kEditFindWorkspace = 1024
$kReplaceWorkspace = 109
$kFileSave = 1005
$kTestSetSelectionBySourceMessage = 0x8032
$kTestGetSourceAnchorMessage = 0x8033
$kTestGetSourceActiveMessage = 0x8034
$kTestGetSourcePositionMessage = 0x8035
$kTestGetEditorReadinessMessage = 0x8036
$kTestGetTabItemCenterMessage = 0x803A
$kTestGetDocumentStateMessage = 0x8045
$kTestSetReplaceConfirmationUiMessage = 0x8047

function Add-Check([string]$Name, [bool]$Pass, $Details = $null) {
    $checks.Add([ordered]@{ name=$Name; status=if ($Pass) { 'PASS' } else { 'FAIL' }; details=$Details })
}
function Find-Control([IntPtr]$Parent, [int]$Id) {
    $script:foundControl = [IntPtr]::Zero
    $script:targetControlId = $Id
    $callback = [MDLiteSearchNative+EnumChildWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLiteSearchNative]::GetDlgCtrlID($window) -eq $script:targetControlId) {
            $script:foundControl = $window; return $false
        }
        return $true
    }
    [void][MDLiteSearchNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    $script:foundControl
}
function Find-Controls([IntPtr]$Parent, [int]$Id) {
    $script:matchingControls = [Collections.Generic.List[IntPtr]]::new()
    $script:targetControlId = $Id
    $callback = [MDLiteSearchNative+EnumChildWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLiteSearchNative]::GetDlgCtrlID($window) -eq $script:targetControlId) {
            $script:matchingControls.Add($window)
        }
        return $true
    }
    [void][MDLiteSearchNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    @($script:matchingControls.ToArray())
}
function Get-OwnedChildWindowInventory([IntPtr]$Parent) {
    $script:childWindowInventory = [Collections.Generic.List[object]]::new()
    $script:childWindowInventoryProcessId = if ($script:process) { $script:process.Id } else { 0 }
    $callback = [MDLiteSearchNative+EnumChildWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        [uint32]$ownerProcessId = 0
        $ownerThreadId = [MDLiteSearchNative]::GetWindowThreadProcessId($window, [ref]$ownerProcessId)
        if ($ownerProcessId -eq $script:childWindowInventoryProcessId) {
            $className = [Text.StringBuilder]::new(128)
            [void][MDLiteSearchNative]::GetClassName($window, $className, $className.Capacity)
            $script:childWindowInventory.Add([ordered]@{
                hwnd=$window.ToInt64(); id=[MDLiteSearchNative]::GetDlgCtrlID($window)
                class=$className.ToString(); owner_pid=$ownerProcessId; owner_thread=$ownerThreadId
                visible=[MDLiteSearchNative]::IsWindowVisible($window)
                enabled=[MDLiteSearchNative]::IsWindowEnabled($window)
            })
        }
        return $true
    }
    [void][MDLiteSearchNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    @($script:childWindowInventory.ToArray())
}
function Get-OwnedTopWindowInventory([int]$ProcessId) {
    $script:topWindowInventory=[Collections.Generic.List[object]]::new()
    $script:topWindowInventoryProcessId=$ProcessId
    $callback=[MDLiteSearchNative+EnumWindowsProc]{
        param([IntPtr]$window,[IntPtr]$parameter)
        [uint32]$ownerProcessId=0
        $ownerThreadId=[MDLiteSearchNative]::GetWindowThreadProcessId($window,[ref]$ownerProcessId)
        if ($ownerProcessId -eq $script:topWindowInventoryProcessId) {
            $className=[Text.StringBuilder]::new(128)
            [void][MDLiteSearchNative]::GetClassName($window,$className,$className.Capacity)
            try { $caption=[MDLiteSearchNative]::Text($window) } catch { $caption='READ_ERROR: '+$_.Exception.Message }
            $script:topWindowInventory.Add([ordered]@{
                hwnd=$window.ToInt64();process_id=$ownerProcessId;thread_id=$ownerThreadId
                class=$className.ToString();caption=$caption
                visible=[MDLiteSearchNative]::IsWindowVisible($window)
                enabled=[MDLiteSearchNative]::IsWindowEnabled($window)
            })
        }
        return $true
    }
    [void][MDLiteSearchNative]::EnumWindows($callback,[IntPtr]::Zero)
    @($script:topWindowInventory.ToArray())
}
function Capture-OwnedModalInventory([int]$ProcessId,[string]$Stage) {
    $script:modalInventoryProcessId=$ProcessId
    $script:modalInventoryDialogs=[Collections.Generic.List[object]]::new()
    $script:modalInventoryMainWindows=[Collections.Generic.List[object]]::new()
    $callback=[MDLiteSearchNative+EnumWindowsProc]{
        param([IntPtr]$window,[IntPtr]$parameter)
        [uint32]$ownerProcessId=0
        $ownerThreadId=[MDLiteSearchNative]::GetWindowThreadProcessId($window,[ref]$ownerProcessId)
        if($ownerProcessId -ne $script:modalInventoryProcessId){return $true}
        $classBuilder=[Text.StringBuilder]::new(128)
        [void][MDLiteSearchNative]::GetClassName($window,$classBuilder,$classBuilder.Capacity)
        $className=$classBuilder.ToString()
        if($className -ne '#32770' -and $className -cne 'MDLite.ReplaceReviewDialog' -and $className -cne 'MDLite.MainWindow'){return $true}
        try{$caption=[MDLiteSearchNative]::Text($window)}catch{$caption='READ_ERROR: '+$_.Exception.Message}
        $snapshot=[ordered]@{
            hwnd=$window.ToInt64();process_id=$ownerProcessId;thread_id=$ownerThreadId;class=$className;caption=$caption
            visible=[MDLiteSearchNative]::IsWindowVisible($window);enabled=[MDLiteSearchNative]::IsWindowEnabled($window)
        }
        if($className -ceq 'MDLite.MainWindow'){$script:modalInventoryMainWindows.Add($snapshot)}
        else{$script:modalInventoryDialogs.Add($snapshot)}
        return $true
    }
    [void][MDLiteSearchNative]::EnumWindows($callback,[IntPtr]::Zero)
    $dialogs=@($script:modalInventoryDialogs.ToArray())
    $mainWindows=@($script:modalInventoryMainWindows.ToArray())
    $children=[Collections.Generic.List[object]]::new()
    foreach($dialog in $dialogs) {
        $controls=@(Get-OwnedChildWindowInventory ([IntPtr]$dialog.hwnd))
        $controlText=[Collections.Generic.List[object]]::new()
        foreach($control in $controls) {
            if($control.class -in @('Static','Button')) {
                try{$text=[MDLiteSearchNative]::Text([IntPtr]$control.hwnd)}catch{$text='READ_ERROR: '+$_.Exception.Message}
                $controlText.Add([ordered]@{hwnd=$control.hwnd;id=$control.id;class=$control.class;text=$text})
            }
        }
        $children.Add([ordered]@{dialog_hwnd=$dialog.hwnd;dialog_class=$dialog.class;dialog_caption=$dialog.caption;controls=$controls;control_text=@($controlText.ToArray())})
    }
    $snapshot=[ordered]@{stage=$Stage;process_id=$ProcessId;main_windows=$mainWindows;dialogs=$dialogs;dialog_controls=@($children.ToArray())}
    [void]$script:ownedModalEvidence.Add($snapshot)
    $snapshot
}
function Wait-Control([IntPtr]$Parent, [int]$Id, [int]$TimeoutMs=10000) {
    $script:currentTestStage="Wait-Control id=$Id caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do { Assert-RunBudget $script:currentTestStage; $control=Find-Control $Parent $Id; if ($control -ne [IntPtr]::Zero) { return $control }; Start-Sleep -Milliseconds 40 }
    while ([DateTime]::UtcNow -lt $deadline)
    $script:missingControlId = $Id
    $script:ownedChildWindowsOnMissing = Get-OwnedChildWindowInventory $Parent
    throw "Native search control missing: id=$Id"
}
function Find-MainWindow([Diagnostics.Process]$Target, [int]$TimeoutMs=15000) {
    $script:currentTestStage="Find-MainWindow pid=$($Target.Id) caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-RunBudget $script:currentTestStage
        $Target.Refresh(); if ($Target.HasExited) { throw "MDLite exited with code $($Target.ExitCode)." }
        $script:foundMain=[IntPtr]::Zero; $script:targetPid=[uint32]$Target.Id
        $callback=[MDLiteSearchNative+EnumWindowsProc]{
            param([IntPtr]$window,[IntPtr]$parameter)
            [uint32]$windowProcessId=0; [void][MDLiteSearchNative]::GetWindowThreadProcessId($window,[ref]$windowProcessId)
            if ($windowProcessId -ne $script:targetPid) { return $true }
            $name=[Text.StringBuilder]::new(128); [void][MDLiteSearchNative]::GetClassName($window,$name,$name.Capacity)
            if ($name.ToString() -eq 'MDLite.MainWindow') { $script:foundMain=$window; return $false }
            return $true
        }
        [void][MDLiteSearchNative]::EnumWindows($callback,[IntPtr]::Zero)
        if ($script:foundMain -ne [IntPtr]::Zero) { return $script:foundMain }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main MDLite window was not created for pid $($Target.Id)."
}
function Send([IntPtr]$Window,[uint32]$Message,[long]$Wp=0,[long]$Lp=0) {
    [MDLiteSearchNative]::SendScalar($Window,$Message,$Wp,$Lp)
}
function Send-Command([IntPtr]$Window,[int]$Command,[int]$Code=0,[IntPtr]$Control=[IntPtr]::Zero) {
    $wparam=([long]$Code -shl 16) -bor ($Command -band 0xffff)
    [void](Send $Window $WM_COMMAND $wparam $Control.ToInt64())
}
function Post-Scalar([IntPtr]$Window,[uint32]$Message,[long]$Wp=0,[long]$Lp=0) {
    if (-not [MDLiteSearchNative]::PostScalar($Window,$Message,$Wp,$Lp)) {
        throw "PostMessage failed: hwnd=$($Window.ToInt64()) message=0x$('{0:X}' -f $Message) error=$([Runtime.InteropServices.Marshal]::GetLastWin32Error())"
    }
}
function Post-Command([IntPtr]$Window,[int]$Command,[int]$Code=0,[IntPtr]$Control=[IntPtr]::Zero) {
    $wparam=([long]$Code -shl 16) -bor ($Command -band 0xffff)
    Post-Scalar $Window $WM_COMMAND $wparam $Control.ToInt64()
}
function Find-OwnedWindow([int]$ProcessId,[string]$ClassName,[string]$Caption) {
    $script:dialogProcessId=$ProcessId
    $script:dialogClassName=$ClassName
    $script:dialogCaption=$Caption
    $script:ownedDialog=[IntPtr]::Zero
    $script:ownedDialogReadError=$null
    $callback=[MDLiteSearchNative+EnumWindowsProc]{
        param([IntPtr]$window,[IntPtr]$parameter)
        if (-not (Test-OwnedWindowIdentity $window $script:dialogProcessId $script:dialogClassName)) { return $true }
        if ($script:dialogCaption) {
            try { $observedCaption=[MDLiteSearchNative]::Text($window) }
            catch {
                $readError=$_.Exception.Message
                if (-not (Test-OwnedWindowIdentity $window $script:dialogProcessId $script:dialogClassName)) { return $true }
                $script:ownedDialogReadError=$readError
                return $false
            }
            if (-not (Test-OwnedWindowIdentity $window $script:dialogProcessId $script:dialogClassName)) { return $true }
            if ($observedCaption -cne $script:dialogCaption) { return $true }
        }
        $script:ownedDialog=$window
        return $false
    }
    [void][MDLiteSearchNative]::EnumWindows($callback,[IntPtr]::Zero)
    if ($script:ownedDialogReadError) {
        throw "Owned window caption read failed while its PID/class identity remained valid: pid=$ProcessId class=$ClassName error=$($script:ownedDialogReadError)"
    }
    $script:ownedDialog
}
function Wait-OwnedWindow([int]$ProcessId,[string]$ClassName,[string]$Caption,[int]$TimeoutMs=6000) {
    $script:currentTestStage="Wait-OwnedWindow pid=$ProcessId class=$ClassName caption=$Caption caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-RunBudget $script:currentTestStage
        $window=Find-OwnedWindow $ProcessId $ClassName $Caption
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $inventory=$null;$inventoryError=$null
    try { $inventory=Capture-OwnedModalInventory $ProcessId 'Wait-OwnedWindow timeout' }
    catch { $inventoryError=$_.Exception.Message }
    $script:lastOwnedWindowWaitEvidence=[ordered]@{
        stage=$script:currentTestStage;process_id=$ProcessId;expected_class=$ClassName;expected_caption=$Caption
        timeout_ms=$TimeoutMs;inventory=$inventory;inventory_error=$inventoryError
    }
    $visibleDialogCaptions=if($inventory){@($inventory.dialogs|Where-Object visible|ForEach-Object { '{0}:{1}' -f $_.class,$_.caption })}else{@()}
    Write-RunBreadcrumb "WAIT_FAIL stage=$script:currentTestStage visible_owned_dialogs=$([string]::Join('|',[string[]]$visibleDialogCaptions)) inventory_error=$inventoryError"
    throw "Expected owned modal window not found: class=$ClassName caption=$Caption pid=$ProcessId"
}
function Wait-OwnedWindowClosed([int]$ProcessId,[string]$ClassName,[string]$Caption,[int]$TimeoutMs=6000) {
    $script:currentTestStage="Wait-OwnedWindowClosed pid=$ProcessId class=$ClassName caption=$Caption caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-RunBudget $script:currentTestStage
        if ((Find-OwnedWindow $ProcessId $ClassName $Caption) -eq [IntPtr]::Zero) { return $true }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $inventory=$null;$inventoryError=$null
    try { $inventory=Capture-OwnedModalInventory $ProcessId 'Wait-OwnedWindowClosed timeout' }
    catch { $inventoryError=$_.Exception.Message }
    $script:lastOwnedWindowWaitEvidence=[ordered]@{
        stage=$script:currentTestStage;process_id=$ProcessId;expected_class=$ClassName;expected_caption=$Caption
        timeout_ms=$TimeoutMs;inventory=$inventory;inventory_error=$inventoryError
    }
    $visibleDialogCaptions=if($inventory){@($inventory.dialogs|Where-Object visible|ForEach-Object { '{0}:{1}' -f $_.class,$_.caption })}else{@()}
    Write-RunBreadcrumb "WAIT_FAIL stage=$script:currentTestStage visible_owned_dialogs=$([string]::Join('|',[string[]]$visibleDialogCaptions)) inventory_error=$inventoryError"
    throw "Owned modal window did not close: class=$ClassName caption=$Caption pid=$ProcessId"
}
function Read-ReplaceReview([int]$ProcessId,[int]$ExpectedCount,[IntPtr]$StatusControl) {
    $script:currentTestStage="Read-ReplaceReview pid=$ProcessId expected=$ExpectedCount caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "STAGE $script:currentTestStage"
    Assert-RunBudget $script:currentTestStage
    $dialog=Wait-OwnedWindow $ProcessId 'MDLite.ReplaceReviewDialog' '置換対象の確認'
    $list=Wait-Control $dialog 4101
    $pathControl=Wait-Control $dialog 4102
    $beforeControl=Wait-Control $dialog 4103
    $afterControl=Wait-Control $dialog 4104
    $count=Send $list $LB_GETCOUNT
    if ($count -ne $ExpectedCount) { throw "Replace review target count mismatch: expected=$ExpectedCount observed=$count" }
    $reviewDialogSnapshot=Get-WindowDiagnosticSnapshot $dialog
    $reviewControls=Get-OwnedChildWindowInventory $dialog
    $entries=[Collections.Generic.List[object]]::new()
    for($index=0;$index -lt $count;$index++) {
        Assert-RunBudget "Read-ReplaceReview item=$index"
        $script:currentTestStage="Read-ReplaceReview path text item=$index"
        if($index -gt 0) {
            [void](Send $list $LB_SETCURSEL $index 0)
            Post-Command $dialog 4101 1 $list
            $deadline=[DateTime]::UtcNow.AddSeconds(3)
            Write-RunBreadcrumb "WAIT_START stage=Read-ReplaceReview target=$index timeout_ms=3000"
            do { Assert-RunBudget "Read-ReplaceReview target=$index"; $pathText=[MDLiteSearchNative]::Text($pathControl); if($pathText -match ('対象\s+'+($index+1)+'\s+/')) { break }; Start-Sleep -Milliseconds 30 }
            while([DateTime]::UtcNow -lt $deadline)
            if($pathText -notmatch ('対象\s+'+($index+1)+'\s+/')) { throw "Replace review did not select target $($index+1): $pathText" }
        } else {
            $script:currentTestStage="Read-ReplaceReview path text item=$index"
            $pathText=[MDLiteSearchNative]::Text($pathControl)
        }
        $script:currentTestStage="Read-ReplaceReview before/after text item=$index"
        $entries.Add([ordered]@{path=$pathText;before=[MDLiteSearchNative]::Text($beforeControl);after=[MDLiteSearchNative]::Text($afterControl)})
    }
    $statusDuringReview=[MDLiteSearchNative]::Text($StatusControl)
    $closeButton=Wait-Control $dialog 1
    $closeButtonText=[MDLiteSearchNative]::Text($closeButton)
    Post-Command $dialog 1 $BN_CLICKED $closeButton
    Wait-OwnedWindowClosed $ProcessId 'MDLite.ReplaceReviewDialog' '置換対象の確認' | Out-Null
    [pscustomobject]@{
        count=$count;entries=@($entries.ToArray());status_during_review=$statusDuringReview
        dialog_before=$reviewDialogSnapshot;dialog_controls=$reviewControls
        close_action=@{control_id=1;caption=$closeButtonText;dialog_closed=(-not [MDLiteSearchNative]::IsWindow($dialog))}
    }
}
function Answer-OwnedConfirmation([int]$ProcessId,[string]$Caption,[int]$ButtonId,[switch]$CaptureInventory) {
    if ($ButtonId -notin @(6,7)) { throw "Confirmation button is outside the fixture Yes/No set: $ButtonId" }
    $dialog=Wait-OwnedWindow $ProcessId '#32770' $Caption
    $button=Wait-Control $dialog $ButtonId
    $dialogBefore=Get-WindowDiagnosticSnapshot $dialog
    $buttonText=[MDLiteSearchNative]::Text($button)
    $inventoryBefore=if($CaptureInventory){Capture-OwnedModalInventory $ProcessId "Confirmation before $Caption button=$ButtonId"}else{$null}
    Post-Command $dialog $ButtonId $BN_CLICKED $button
    Wait-OwnedWindowClosed $ProcessId '#32770' $Caption | Out-Null
    if ($CaptureInventory) {
        $inventoryAfter=Capture-OwnedModalInventory $ProcessId "Confirmation after $Caption button=$ButtonId"
        return [pscustomobject]@{
            process_id=$ProcessId;caption=$Caption;dialog_hwnd=$dialog.ToInt64();dialog_before=$dialogBefore
            button_id=$ButtonId;button_text=$buttonText;closed=(-not [MDLiteSearchNative]::IsWindow($dialog))
            inventory_before=$inventoryBefore;inventory_after=$inventoryAfter
        }
    }
}
function Get-Bounds([IntPtr]$Window) {
    $rect=[MDLiteSearchNative+RECT]::new()
    if (-not [MDLiteSearchNative]::GetWindowRect($Window,[ref]$rect)) { throw 'GetWindowRect failed.' }
    [pscustomobject]@{left=$rect.Left;top=$rect.Top;right=$rect.Right;bottom=$rect.Bottom;width=$rect.Right-$rect.Left;height=$rect.Bottom-$rect.Top;visible=[MDLiteSearchNative]::IsWindowVisible($Window)}
}
function Get-Count([IntPtr]$CountControl) {
    $text=[MDLiteSearchNative]::Text($CountControl)
    if ($text -match '/\s*(\d+)') { return [int]$Matches[1] }
    if ($text -match '(\d+)\s*件') { return [int]$Matches[1] }
    0
}
function Wait-Search([IntPtr]$CountControl,[IntPtr]$StatusControl,[int]$Expected,[int]$TimeoutMs=8000) {
    $script:currentTestStage="Wait-Search expected=$Expected caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        Assert-RunBudget $script:currentTestStage
        $status=[MDLiteSearchNative]::Text($StatusControl)
        $count=Get-Count $CountControl
        if ($status -notmatch '検索中' -and $count -eq $Expected) { return [pscustomobject]@{status=$status;count=$count} }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $status=[MDLiteSearchNative]::Text($StatusControl); $count=Get-Count $CountControl
    throw "Search did not settle: expected=$Expected count=$count status=$status"
}
function Set-Query([IntPtr]$Main,[IntPtr]$Edit,[string]$Text) {
    [MDLiteSearchNative]::SetText($Edit,$Text)
    # Exercise the native Edit EN_CHANGE parent route even on Windows builds
    # where WM_SETTEXT does not send a change notification.
    Send-Command $Main $kFindEdit $EN_CHANGE $Edit
}
function Replace-Selection([IntPtr]$Control,[string]$Text) {
    $result=[IntPtr]::Zero
    if ([MDLiteSearchNative]::SendMessageTimeoutText($Control,0x00C2,[IntPtr]1,$Text,2,5000,[ref]$result) -eq [IntPtr]::Zero) {
        throw 'EM_REPLACESEL timed out.'
    }
}
function Set-Scope([IntPtr]$Main,[IntPtr]$Combo,[int]$Scope) {
    [void](Send $Combo 0x014E $Scope 0)
    Send-Command $Main $kSearchScope $CBN_SELCHANGE $Combo
}
function Set-Checkbox([IntPtr]$Main,[IntPtr]$Checkbox,[int]$Id,[bool]$Checked) {
    [void](Send $Checkbox 0x00F1 $(if($Checked){1}else{0}) 0)
    Send-Command $Main $Id $BN_CLICKED $Checkbox
}
function Wait-Ready([IntPtr]$Main) {
    $script:currentTestStage="Wait-Ready main=$($Main.ToInt64()) caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=12000"
    $deadline=[DateTime]::UtcNow.AddSeconds(12)
    do { Assert-RunBudget $script:currentTestStage; $ready=Send $Main $kTestGetEditorReadinessMessage 0 0; if (($ready -band 5) -eq 5) { return }; Start-Sleep -Milliseconds 40 }
    while ([DateTime]::UtcNow -lt $deadline)
    throw "Editor did not reach source/presentation ready state: $ready"
}
function Get-DocumentText([IntPtr]$Main) {
    $script:currentTestStage="Get-DocumentText main=$($Main.ToInt64()) caller_line=$($MyInvocation.ScriptLineNumber)"
    Write-RunBreadcrumb "STAGE $script:currentTestStage"
    Assert-RunBudget $script:currentTestStage
    $length=Send $Main $kTestGetDocumentStateMessage 4 0
    if ($length -lt 0 -or $length -gt 2000000) { throw "Document source length invalid: $length" }
    $builder=[Text.StringBuilder]::new([int]$length)
    for ($index=0;$index -lt $length;$index++) {
        if (($index -band 63) -eq 0) { Assert-RunBudget "$($script:currentTestStage) index=$index length=$length" }
        [void]$builder.Append([char](Send $Main $kTestGetDocumentStateMessage 5 $index))
    }
    $builder.ToString()
}
function Get-TextSha256([string]$Text) {
    $hasher=[Security.Cryptography.SHA256]::Create()
    try { [BitConverter]::ToString($hasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($Text))).Replace('-','').ToLowerInvariant() }
    finally { $hasher.Dispose() }
}
function Get-WindowDiagnosticSnapshot([IntPtr]$Window) {
    if ($Window -eq [IntPtr]::Zero) { return [ordered]@{hwnd=0;is_window=$false} }
    [uint32]$ownerPid=0
    $ownerThread=[MDLiteSearchNative]::GetWindowThreadProcessId($Window,[ref]$ownerPid)
    $className=[Text.StringBuilder]::new(128)
    [void][MDLiteSearchNative]::GetClassName($Window,$className,$className.Capacity)
    $guiInfo=[MDLiteSearchNative+GUIINFO]::new()
    $guiInfo.cbSize=[Runtime.InteropServices.Marshal]::SizeOf($guiInfo)
    $guiInfoRead=$ownerThread -ne 0 -and [MDLiteSearchNative]::GetGUIThreadInfo($ownerThread,[ref]$guiInfo)
    [ordered]@{
        hwnd=$Window.ToInt64();is_window=[MDLiteSearchNative]::IsWindow($Window)
        process_id=$ownerPid;thread_id=$ownerThread;class=$className.ToString()
        control_id=[MDLiteSearchNative]::GetDlgCtrlID($Window)
        visible=[MDLiteSearchNative]::IsWindowVisible($Window);enabled=[MDLiteSearchNative]::IsWindowEnabled($Window)
        gui_thread_info_read=$guiInfoRead;gui_flags=if($guiInfoRead){$guiInfo.flags}else{0}
        gui_focus_hwnd=if($guiInfoRead){$guiInfo.focus.ToInt64()}else{0}
    }
}
function New-RangePair([int]$Begin,[int]$Length) {
    $pair=[int[]]@($Begin,($Begin+$Length))
    if ($pair.Length -ne 2) { throw "Range pair must contain exactly two elements; observed=$($pair.Length)." }
    return ,$pair
}
function Get-SourceRangePair([IntPtr]$Main) {
    $anchor=[int](Send $Main $kTestGetSourceAnchorMessage 0 0)
    $active=[int](Send $Main $kTestGetSourceActiveMessage 0 0)
    $pair=[int[]]@($anchor,$active)
    if ($pair.Length -ne 2) { throw "Observed source range must contain exactly two elements; observed=$($pair.Length)." }
    return ,$pair
}
function Test-RangePair($Expected,$Observed) {
    $expectedPair=@($Expected)
    $observedPair=@($Observed)
    if ($expectedPair.Count -ne 2 -or $observedPair.Count -ne 2) { return $false }
    return ([int]$expectedPair[0] -eq [int]$observedPair[0] -and
            [int]$expectedPair[1] -eq [int]$observedPair[1])
}
function Get-RichEditSelectionEvidence([IntPtr]$Window,[int]$ProcessId) {
    $selection=[SearchOwnedSelection]::Read($Window,[uint32]$ProcessId)
    $range=[int[]]@($selection.Begin,$selection.End)
    if ($range.Length -ne 2) { throw "Owned RichEdit selection must contain exactly two elements; observed=$($range.Length)." }
    [pscustomobject]@{
        window=Get-WindowDiagnosticSnapshot $Window;range=@($range);selected_text=$selection.Text
        owner_pid=$selection.OwnerPid;remote_buffer_bytes=$selection.RemoteBytes
    }
}
function Capture-InputSettleBaseline([IntPtr]$Main,[IntPtr]$Target,[int]$ProcessId) {
    $range=Get-SourceRangePair $Main
    [ordered]@{
        process_id=$ProcessId;main_hwnd=$Main.ToInt64();target_control_hwnd=$Target.ToInt64()
        windows_before=[ordered]@{main=Get-WindowDiagnosticSnapshot $Main;target=Get-WindowDiagnosticSnapshot $Target}
        range_before=@($range)
    }
}
function Wait-WorkspaceApplySettled([IntPtr]$Main,[IntPtr]$StatusControl,[int]$ProcessId,[string[]]$Paths,[string[]]$ExpectedContents,
                                    [string]$JournalDirectory,[string[]]$JournalNamesBefore,[string]$Stage,
                                    [int]$TimeoutMs=8000,[bool]$RequireNewJournal=$true) {
    if ($Paths.Count -eq 0 -or $Paths.Count -ne $ExpectedContents.Count) {
        throw "Workspace apply settle paths and expected contents must be nonempty and aligned; paths=$($Paths.Count) expected=$($ExpectedContents.Count)."
    }
    $script:currentTestStage=$Stage
    Write-RunBreadcrumb "WAIT_START stage=$Stage timeout_ms=$TimeoutMs require_new_journal=$RequireNewJournal"
    $inventoryBefore=Capture-OwnedModalInventory $ProcessId "$Stage before wait"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $samples=[Collections.Generic.List[object]]::new()
    $observations=0
    $contents=[string[]]::new($Paths.Count)
    $journalNames=@()
    $newJournalNames=@()
    $filesMatch=$false
    $mainWindow=Get-WindowDiagnosticSnapshot $Main
    $inventoryAfter=$null
    $uiProbe=$null
    $visibleModals=@()
    $ready=$false
    do {
        Assert-RunBudget $Stage
        $observations++
        for($index=0;$index -lt $Paths.Count;$index++) { $contents[$index]=[IO.File]::ReadAllText($Paths[$index]) }
        $filesMatch=$true
        for($index=0;$index -lt $Paths.Count;$index++) {
            if ($contents[$index] -cne $ExpectedContents[$index]) { $filesMatch=$false; break }
        }
        $journalNames=if(Test-Path -LiteralPath $JournalDirectory -PathType Container) {
            @(Get-ChildItem -LiteralPath $JournalDirectory -File -Filter '*.journal' | ForEach-Object Name)
        } else { @() }
        $newJournalNames=@($journalNames|Where-Object {$JournalNamesBefore -notcontains $_})
        $mainWindow=Get-WindowDiagnosticSnapshot $Main
        if ($filesMatch -and (-not $RequireNewJournal -or $newJournalNames.Count -gt 0) -and $mainWindow.enabled) {
            # This synchronous, read-only message is handled only after the modal workflow returns to the main window.
            $uiProbe=Send $Main $kTestGetDocumentStateMessage 0 0
            $mainWindow=Get-WindowDiagnosticSnapshot $Main
            $inventoryAfter=Capture-OwnedModalInventory $ProcessId "$Stage after owner returned"
            $visibleModals=@($inventoryAfter.dialogs|Where-Object {
                $_.visible -and ($_.class -eq '#32770' -or $_.class -eq 'MDLite.ReplaceReviewDialog')
            })
            $ready=$uiProbe -ne -1 -and $mainWindow.is_window -and $mainWindow.enabled -and $visibleModals.Count -eq 0
            if($samples.Count -lt 40){$samples.Add([ordered]@{elapsed_ms=$timer.ElapsedMilliseconds;files_match=$filesMatch;new_journal_count=$newJournalNames.Count;main_enabled=$mainWindow.enabled;ui_probe_succeeded=($uiProbe -ne -1);visible_modal_count=$visibleModals.Count;ready=$ready})}
            if ($ready) { break }
        } else {
            if($samples.Count -lt 40){$samples.Add([ordered]@{elapsed_ms=$timer.ElapsedMilliseconds;files_match=$filesMatch;new_journal_count=$newJournalNames.Count;main_enabled=$mainWindow.enabled;visible_modal_count=$null;ready=$false})}
        }
        if ([DateTime]::UtcNow -ge $deadline) { break }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    $timer.Stop()
    $statusText=if($StatusControl -ne [IntPtr]::Zero){[MDLiteSearchNative]::Text($StatusControl)}else{$null}
    [pscustomobject]@{
        stage=$Stage;process_id=$ProcessId;main_hwnd=$Main.ToInt64();ready=$ready;elapsed_ms=$timer.ElapsedMilliseconds;timeout_ms=$TimeoutMs
        paths=@($Paths);expected_contents=@($ExpectedContents);observed_contents=@($contents);files_match=$filesMatch
        journal_directory=$JournalDirectory;journal_names_before=@($JournalNamesBefore);journal_names_after=@($journalNames);new_journal_names=@($newJournalNames)
        main_window=$mainWindow;ui_probe=$uiProbe;status_after=$statusText
        completion_proof='Expected closed-file contents and journal are present; main window is enabled, synchronous test-state probe returned, and no visible owned task/review dialog remains.'
        visible_modals_after=@($visibleModals);inventory_before=$inventoryBefore;inventory_after=$inventoryAfter
        observation_count=$observations;sample_count=$samples.Count;sample_limit=40;samples=@($samples.ToArray())
    }
}
function Wait-SourceRangeAfterInput([IntPtr]$Main,[IntPtr]$Target,[int]$ProcessId,[int[]]$Expected,[string]$InputName,$Baseline,[int]$TimeoutMs=2000) {
    $expectedCount=@($Expected).Count
    $beforeCount=@($Baseline.range_before).Count
    if ($expectedCount -ne 2 -or $beforeCount -ne 2) {
        throw "Semantic settle requires two-element source ranges; input=$InputName expected=$expectedCount before=$beforeCount."
    }
    $script:currentTestStage="Semantic settle $InputName pid=$ProcessId control=$($Target.ToInt64())"
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=$TimeoutMs"
    $deadline=[DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $timer=[Diagnostics.Stopwatch]::StartNew()
    $samples=[Collections.Generic.List[object]]::new()
    $observed=[int[]]@($Baseline.range_before)
    $matched=Test-RangePair $Expected $observed
    do {
        Assert-RunBudget $script:currentTestStage
        $observed=Get-SourceRangePair $Main
        $matched=Test-RangePair $Expected $observed
        $samples.Add([ordered]@{elapsed_ms=$timer.ElapsedMilliseconds;range=@($observed);matches_expected=$matched})
        if ($matched) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $timer.Stop()
    $windowsAfter=[ordered]@{main=Get-WindowDiagnosticSnapshot $Main;target=Get-WindowDiagnosticSnapshot $Target}
    [pscustomobject]@{
        input=$InputName;process_id=$ProcessId;target_control_hwnd=$Target.ToInt64();main_hwnd=$Main.ToInt64()
        expected_range=@($Expected);range_before=@($Baseline.range_before);observed_range=@($observed)
        windows_before=$Baseline.windows_before;windows_after=$windowsAfter
        elapsed_ms=$timer.ElapsedMilliseconds;timeout_ms=$TimeoutMs;sample_count=$samples.Count;samples=@($samples.ToArray());matched=$matched
    }
}
function Test-OwnedWindowIdentity([IntPtr]$Window,[int]$ProcessId,[string]$ClassName) {
    if ($Window -eq [IntPtr]::Zero -or -not [MDLiteSearchNative]::IsWindow($Window)) { return $false }
    [uint32]$ownerProcessId=0
    [void][MDLiteSearchNative]::GetWindowThreadProcessId($Window,[ref]$ownerProcessId)
    if ($ownerProcessId -ne $ProcessId) { return $false }
    $class=[Text.StringBuilder]::new(128)
    [void][MDLiteSearchNative]::GetClassName($Window,$class,$class.Capacity)
    return $class.ToString() -ceq $ClassName
}
function Invoke-WindowMessageDiagnostic([IntPtr]$Window,[uint32]$Message,[int]$TimeoutMs) {
    $reply=[IntPtr]::Zero
    [MDLiteSearchNative]::SetLastError(0)
    $sent=[MDLiteSearchNative]::SendMessageTimeout(
        $Window,$Message,[IntPtr]::Zero,[IntPtr]::Zero,2,[uint32]$TimeoutMs,[ref]$reply)
    $lastError=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
    [ordered]@{
        window_hwnd=$Window.ToInt64();message=('0x{0:X}' -f $Message)
        flags=2;timeout_ms=$TimeoutMs;last_error_reset_before_send=0
        send_message_completed=($sent -ne [IntPtr]::Zero);message_result=$reply.ToInt64();last_error=$lastError
    }
}
function Read-TextWithDiagnostics([IntPtr]$Window,[string]$Stage) {
    $script:currentTestStage=$Stage
    $ctrlBefore=[int][MDLiteSearchNative]::GetAsyncKeyState(0x11)
    $diagnostic=[ordered]@{
        stage=$Stage;target_before=Get-WindowDiagnosticSnapshot $Window
        main_before=Get-WindowDiagnosticSnapshot $main
        fixture_process_id=if($process){$process.Id}else{0}
        active_document_identity=$documentIdentity;active_document_source_sha256=$documentSourceHash
        ctrl_async_state_before=$ctrlBefore;ctrl_down_before=(($ctrlBefore -band 0x8000) -ne 0)
        send_timeout_flags=2;timeout_ms=1500;last_error_reset_before_each_send=0
    }
    try {
        $lengthCall=Invoke-WindowMessageDiagnostic $Window 0x000E 1500
        $diagnostic['wm_gettextlength_call']=$lengthCall
        if (-not $lengthCall.send_message_completed) { throw "WM_GETTEXTLENGTH timed out; last_error=$($lengthCall.last_error)." }
        $length=[long]$lengthCall.message_result
        $diagnostic['text_length']=$length
        if ($length -lt 0 -or $length -gt 16384) { throw "WM_GETTEXTLENGTH exceeded the diagnostic limit: $length." }
        $buffer=[Text.StringBuilder]::new([int]$length+1)
        $reply=[IntPtr]::Zero
        [MDLiteSearchNative]::SetLastError(0)
        $sent=[MDLiteSearchNative]::SendMessageTimeoutTextRead(
            $Window,0x000D,[IntPtr]($length+1),$buffer,2,1500,[ref]$reply)
        $lastError=[Runtime.InteropServices.Marshal]::GetLastWin32Error()
        $textCall=[ordered]@{
            window_hwnd=$Window.ToInt64();message='0x000D';flags=2;timeout_ms=1500
            last_error_reset_before_send=0;send_message_completed=($sent -ne [IntPtr]::Zero)
            message_result=$reply.ToInt64();last_error=$lastError;buffer_capacity=$length+1
        }
        $diagnostic['wm_gettext_call']=$textCall
        if ($sent -eq [IntPtr]::Zero) { throw "WM_GETTEXT timed out; last_error=$lastError." }
        $text=$buffer.ToString()
        $diagnostic['read_text_length']=$text.Length
        $diagnostic['target_after']=Get-WindowDiagnosticSnapshot $Window
        $diagnostic['main_after']=Get-WindowDiagnosticSnapshot $main
        $diagnostic['ctrl_async_state_after']=[int][MDLiteSearchNative]::GetAsyncKeyState(0x11)
        $script:lastTextReadDiagnostic=$diagnostic
        return $text
    } catch {
        $diagnostic['exception']=$_.Exception.Message
        $diagnostic['target_after']=Get-WindowDiagnosticSnapshot $Window
        $diagnostic['main_after']=Get-WindowDiagnosticSnapshot $main
        $diagnostic['main_readiness_after']=Invoke-WindowMessageDiagnostic $main $kTestGetEditorReadinessMessage 500
        $diagnostic['main_thread_wm_null_after']=Invoke-WindowMessageDiagnostic $main 0x0000 250
        $diagnostic['target_thread_wm_null_after']=Invoke-WindowMessageDiagnostic $Window 0x0000 250
        $ctrlAfter=[int][MDLiteSearchNative]::GetAsyncKeyState(0x11)
        $diagnostic['ctrl_async_state_after']=$ctrlAfter
        $diagnostic['ctrl_down_after']=(($ctrlAfter -band 0x8000) -ne 0)
        $script:lastTextReadDiagnostic=$diagnostic
        throw
    }
}
function Get-TreeCounts([IntPtr]$Tree) {
    $script:currentTestStage="Get-TreeCounts tree=$($Tree.ToInt64())"
    Assert-RunBudget $script:currentTestStage
    $total=Send $Tree $TVM_GETCOUNT 0 0
    $roots=0; $item=Send $Tree $TVM_GETNEXTITEM $TVGN_ROOT 0
    while ($item -ne 0) { Assert-RunBudget "$($script:currentTestStage) roots=$roots"; $roots++; $item=Send $Tree $TVM_GETNEXTITEM $TVGN_NEXT $item }
    [pscustomobject]@{total=$total;roots=$roots}
}
function ConvertTo-JsonPlainSnapshot($Root,[int]$MaxNodes=10000,[int]$MaxDepth=64,[long]$MaxStringChars=8388608) {
    $script:currentTestStage='JSON plain snapshot'
    Write-RunBreadcrumb "STAGE $script:currentTestStage max_nodes=$MaxNodes max_depth=$MaxDepth max_string_chars=$MaxStringChars"
    $active=[Collections.Generic.HashSet[object]]::new([MDLiteReferenceEqualityComparer]::new())
    $pending=[Collections.Generic.Stack[object]]::new()
    $rootSlot=[object[]]::new(1)
    $pending.Push([pscustomobject]@{exit=$false;value=$Root;parent=$rootSlot;slot='root';key=$null;index=0;path='$';depth=0})
    $scalarHistogram=[Collections.Generic.Dictionary[string,int]]::new([StringComparer]::Ordinal)
    $expanded=0; $scalarCount=0; $dictionaryCount=0; $arrayCount=0; $noteSnapshotCount=0
    $extendedScalarCount=0; $stringChars=0L
    while ($pending.Count -gt 0) {
        Assert-RunBudget 'JSON plain snapshot'
        $frame=$pending.Pop()
        $node=$frame.value
        if ($frame.exit) { [void]$active.Remove($node); continue }
        $expanded++
        if ($expanded -gt $MaxNodes) {
            Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) reason=node_limit expanded=$expanded max_nodes=$MaxNodes"
            throw "JSON snapshot exceeded $MaxNodes expanded nodes at $($frame.path)."
        }
        if ($frame.depth -gt $MaxDepth) {
            Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) reason=depth_limit depth=$($frame.depth) max_depth=$MaxDepth"
            throw "JSON snapshot exceeded depth $MaxDepth at $($frame.path)."
        }

        $type=$null; $typeName='null'; $snapshotValue=$null; $scalarKind=$null
        if ($null -ne $node) {
            $type=$node.GetType()
            $typeName=$type.FullName
            if ($type -eq [string]) { $scalarKind='string'; $snapshotValue=[string]$node }
            elseif ($type -eq [bool]) { $scalarKind='boolean'; $snapshotValue=[bool]$node }
            else {
                switch -CaseSensitive ($typeName) {
                    'System.Byte' { $scalarKind='number'; $snapshotValue=[byte]$node; break }
                    'System.SByte' { $scalarKind='number'; $snapshotValue=[sbyte]$node; break }
                    'System.Int16' { $scalarKind='number'; $snapshotValue=[int16]$node; break }
                    'System.UInt16' { $scalarKind='number'; $snapshotValue=[uint16]$node; break }
                    'System.Int32' { $scalarKind='number'; $snapshotValue=[int32]$node; break }
                    'System.UInt32' { $scalarKind='number'; $snapshotValue=[uint32]$node; break }
                    'System.Int64' { $scalarKind='number'; $snapshotValue=[int64]$node; break }
                    'System.UInt64' { $scalarKind='number'; $snapshotValue=[uint64]$node; break }
                    'System.Single' { $scalarKind='number'; $snapshotValue=[single]$node; break }
                    'System.Double' { $scalarKind='number'; $snapshotValue=[double]$node; break }
                    'System.Decimal' { $scalarKind='number'; $snapshotValue=[decimal]$node; break }
                }
            }
        } else { $scalarKind='null' }

        if ($null -ne $scalarKind) {
            $instanceMembers=[Collections.Generic.List[string]]::new()
            if ($null -ne $node) {
                foreach ($member in $node.PSObject.Properties) {
                    if($member.MemberType -in @(
                        [Management.Automation.PSMemberTypes]::NoteProperty,
                        [Management.Automation.PSMemberTypes]::ScriptProperty,
                        [Management.Automation.PSMemberTypes]::CodeProperty,
                        [Management.Automation.PSMemberTypes]::AliasProperty)){
                        $instanceMembers.Add(('{0}:{1}' -f $member.Name,$member.MemberType))
                    }
                }
            }
            $memberSignature=if($instanceMembers.Count -gt 0){[string]::Join(',',([string[]]$instanceMembers.ToArray()))}else{'-'}
            $histogramPath=[regex]::Replace($frame.path,'\[\d+\]','[*]')
            $histogramKey="type=$typeName path=$histogramPath members=$memberSignature"
            if($scalarHistogram.ContainsKey($histogramKey)){$scalarHistogram[$histogramKey]++}else{$scalarHistogram.Add($histogramKey,1)}
            $scalarCount++
            if($instanceMembers.Count -gt 0){$extendedScalarCount++}
            if($scalarKind -eq 'string'){
                $stringChars += $snapshotValue.Length
                if($stringChars -gt $MaxStringChars){
                    Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=string_budget chars=$stringChars max_chars=$MaxStringChars"
                    throw "JSON snapshot exceeded $MaxStringChars string characters at $($frame.path)."
                }
            }
            if($frame.slot -eq 'root'){[void]($rootSlot[0]=$snapshotValue)}
            elseif($frame.slot -eq 'dictionary'){[void]($frame.parent[$frame.key]=$snapshotValue)}
            else{[void]($frame.parent[$frame.index]=$snapshotValue)}
            continue
        }

        if($type.IsValueType){
            Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=unsupported_value_type"
            throw "JSON snapshot rejected value type $typeName at $($frame.path)."
        }
        if($active.Contains($node)){
            Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=cycle"
            throw "JSON snapshot found a cycle at $($frame.path) ($typeName)."
        }

        $isDictionary=($type -eq [Collections.Hashtable] -or $type -eq [Collections.Specialized.OrderedDictionary])
        if(-not $isDictionary -and $node -is [Collections.IDictionary] -and $type.IsGenericType){
            $genericArguments=$type.GetGenericArguments()
            $isDictionary=($type.GetGenericTypeDefinition().FullName -eq 'System.Collections.Generic.Dictionary`2' -and $genericArguments[0] -eq [string])
        }
        $isArray=($type.IsArray -and $type.GetArrayRank() -eq 1 -and $node.GetLowerBound(0) -eq 0)
        $isArrayList=($type -eq [Collections.ArrayList])
        $isGenericList=($type.IsGenericType -and $type.GetGenericTypeDefinition().FullName -eq 'System.Collections.Generic.List`1')

        if($isDictionary){
            if($node.Count -gt ($MaxNodes-$expanded)){
                Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=container_budget count=$($node.Count)"
                throw "JSON snapshot dictionary exceeds the node budget at $($frame.path)."
            }
            $keys=[Collections.Generic.List[string]]::new()
            foreach($key in $node.Keys){
                if($key -isnot [string]){
                    $keyType=if($null -eq $key){'null'}else{$key.GetType().FullName}
                    Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=non_string_dictionary_key key_type=$keyType"
                    throw "JSON snapshot dictionary has a non-string key at $($frame.path)."
                }
                $freshKey=[string]$key
                $stringChars += $freshKey.Length
                if($stringChars -gt $MaxStringChars){
                    Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) reason=string_budget dictionary_key=true chars=$stringChars max_chars=$MaxStringChars"
                    throw "JSON snapshot exceeded $MaxStringChars string characters at $($frame.path)."
                }
                $keys.Add($freshKey)
            }
            $snapshot=[Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
            foreach($key in $keys){$snapshot.Add($key,$null)}
            [void]$active.Add($node)
            if($frame.slot -eq 'root'){[void]($rootSlot[0]=$snapshot)}
            elseif($frame.slot -eq 'dictionary'){[void]($frame.parent[$frame.key]=$snapshot)}
            else{[void]($frame.parent[$frame.index]=$snapshot)}
            $dictionaryCount++
            $pending.Push([pscustomobject]@{exit=$true;value=$node})
            for($index=$keys.Count-1;$index -ge 0;$index--){
                $key=$keys[$index]
                $pending.Push([pscustomobject]@{exit=$false;value=$node[$key];parent=$snapshot;slot='dictionary';key=$key;index=0;path=('{0}.{1}' -f $frame.path,$key);depth=($frame.depth+1)})
            }
            continue
        }
        if($isArray -or $isArrayList -or $isGenericList){
            $count=$node.Count
            if($count -gt ($MaxNodes-$expanded)){
                Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=container_budget count=$count"
                throw "JSON snapshot list exceeds the node budget at $($frame.path)."
            }
            $snapshot=[object[]]::new($count)
            [void]$active.Add($node)
            if($frame.slot -eq 'root'){[void]($rootSlot[0]=$snapshot)}
            elseif($frame.slot -eq 'dictionary'){[void]($frame.parent[$frame.key]=$snapshot)}
            else{[void]($frame.parent[$frame.index]=$snapshot)}
            $arrayCount++
            $pending.Push([pscustomobject]@{exit=$true;value=$node})
            for($index=$count-1;$index -ge 0;$index--){
                $pending.Push([pscustomobject]@{exit=$false;value=$node[$index];parent=$snapshot;slot='array';key=$null;index=$index;path=('{0}[{1}]' -f $frame.path,$index);depth=($frame.depth+1)})
            }
            continue
        }
        if($type.FullName -ceq 'System.Management.Automation.PSCustomObject'){
            $properties=@($node.PSObject.Properties)
            if($properties.Count -gt ($MaxNodes-$expanded)){
                Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=container_budget count=$($properties.Count)"
                throw "JSON snapshot object exceeds the node budget at $($frame.path)."
            }
            $snapshot=[Collections.Specialized.OrderedDictionary]::new([StringComparer]::Ordinal)
            foreach($property in $properties){
                if($property.MemberType -ne [Management.Automation.PSMemberTypes]::NoteProperty){
                    $propertyPath='{0}.{1}' -f $frame.path,$property.Name
                    Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$propertyPath type=$typeName member_type=$($property.MemberType) reason=non_note_property"
                    throw "JSON snapshot rejected non-note property $($property.Name) at $($frame.path)."
                }
                $propertyName=[string]$property.Name
                $stringChars += $propertyName.Length
                if($stringChars -gt $MaxStringChars){
                    Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=string_budget property_name=true chars=$stringChars max_chars=$MaxStringChars"
                    throw "JSON snapshot exceeded $MaxStringChars string characters at $($frame.path)."
                }
                $snapshot.Add($propertyName,$null)
            }
            [void]$active.Add($node)
            if($frame.slot -eq 'root'){[void]($rootSlot[0]=$snapshot)}
            elseif($frame.slot -eq 'dictionary'){[void]($frame.parent[$frame.key]=$snapshot)}
            else{[void]($frame.parent[$frame.index]=$snapshot)}
            $noteSnapshotCount++
            $pending.Push([pscustomobject]@{exit=$true;value=$node})
            for($index=$properties.Count-1;$index -ge 0;$index--){
                $property=$properties[$index]; $propertyName=[string]$property.Name
                $pending.Push([pscustomobject]@{exit=$false;value=$property.Value;parent=$snapshot;slot='dictionary';key=$propertyName;index=0;path=('{0}.{1}' -f $frame.path,$propertyName);depth=($frame.depth+1)})
            }
            continue
        }
        $kind=if($node -is [Collections.IDictionary]){'unsupported_dictionary'}elseif($node -is [Collections.IList]){'unsupported_list'}else{'unsupported_live_object'}
        Write-RunBreadcrumb "JSON_SNAPSHOT_REJECT path=$($frame.path) type=$typeName reason=$kind"
        throw "JSON snapshot rejected $typeName at $($frame.path)."
    }
    [string[]]$histogramKeys=@($scalarHistogram.Keys)
    [Array]::Sort($histogramKeys,[StringComparer]::Ordinal)
    Write-RunBreadcrumb "JSON_SCALAR_HISTOGRAM_BEGIN entries=$($histogramKeys.Count) extended_scalars=$extendedScalarCount scalar_nodes=$scalarCount string_chars=$stringChars"
    foreach($key in $histogramKeys){Write-RunBreadcrumb "JSON_SCALAR_HISTOGRAM count=$($scalarHistogram[$key]) $key"}
    Write-RunBreadcrumb 'JSON_SCALAR_HISTOGRAM_END'
    [pscustomobject]@{
        snapshot=$rootSlot[0];expanded_nodes=$expanded;scalar_nodes=$scalarCount
        dictionary_nodes=$dictionaryCount;array_nodes=$arrayCount;note_snapshot_nodes=$noteSnapshotCount
        extended_scalar_nodes=$extendedScalarCount;string_chars=$stringChars;max_nodes=$MaxNodes;max_depth=$MaxDepth;max_string_chars=$MaxStringChars
    }
}
function Get-FirstResult([IntPtr]$Tree,[int]$RootIndex,[int]$FixtureProcessId) {
    $root=Send $Tree $TVM_GETNEXTITEM $TVGN_ROOT 0
    $matchGroupIndex=0;$visited=0;$rootsVisited=0;$skipped=[Collections.Generic.List[object]]::new()
    while($root -ne 0) {
        if(++$rootsVisited -gt 256){throw 'Small-fixture TreeView scan exceeded256 groups.'}
        $child=Send $Tree $TVM_GETNEXTITEM $TVGN_CHILD $root
        while($child -ne 0) {
            if(++$visited -gt 256){throw 'Small-fixture TreeView scan exceeded256 nodes.'}
            $data=[SearchOwnedSelection]::ReadTreeParameter($Tree,[IntPtr]$child,[uint32]$FixtureProcessId)
            if($data -ge 0) {
                if($data -ge 20000){throw 'TreeView result index exceeds the product match bound.'}
                if($matchGroupIndex -eq $RootIndex){return [pscustomobject]@{root=$root;child=$child;match_index=$data;skipped_nonmatch_nodes=@($skipped.ToArray())}}
                $matchGroupIndex++;break
            }
            $skipped.Add([ordered]@{root=$root;child=$child;parameter=$data})
            $child=Send $Tree $TVM_GETNEXTITEM $TVGN_NEXT $child
        }
        $root=Send $Tree $TVM_GETNEXTITEM $TVGN_NEXT $root
    }
    [pscustomobject]@{root=0;child=0;match_index=-1;skipped_nonmatch_nodes=@($skipped.ToArray())}
}
function Select-TreeItem([IntPtr]$Tree,[long]$Item) {
    if ($Item -eq 0) { throw 'Search result TreeView item missing.' }
    [void](Send $Tree $TVM_SELECTITEM $TVGN_CARET $Item)
    # Programmatic selection can leave an already-selected item without a new TVN_SELCHANGED route.
    # The existing result-tree VK_RETURN handler explicitly opens the selected result.
    [void](Send $Tree $WM_KEYDOWN 0x0D 0)
}
function Capture-Frame([IntPtr]$Window,[string]$Name) {
    $bounds=Get-Bounds $Window
    if ($bounds.width -le 0 -or $bounds.height -le 0) { throw 'Screenshot bounds are empty.' }
    $path=Join-Path $screenshots ($Name+'.png')
    $bitmap=[Drawing.Bitmap]::new($bounds.width,$bounds.height,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics=[Drawing.Graphics]::FromImage($bitmap);$hdc=$graphics.GetHdc()
    try { $captured=[MDLiteSearchNative]::PrintWindow($Window,$hdc,2) }
    finally { $graphics.ReleaseHdc($hdc);$graphics.Dispose() }
    if (-not $captured) { $bitmap.Dispose(); throw 'PrintWindow screenshot failed.' }
    try { $bitmap.Save($path,[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
    $frames.Add([ordered]@{name=$Name;path=$path;sha256=(Get-FileSha256 -Path $path);width=$bounds.width;height=$bounds.height;method='PrintWindow native window capture'})
    $path
}
function Capture-ForegroundFrame([IntPtr]$Window,[string]$Name) {
    if ([MDLiteSearchNative]::GetForegroundWindow() -ne $Window) { throw 'Foreground screenshot requires the owned fixture to be foreground.' }
    $bounds=Get-Bounds $Window
    if (-not $bounds.visible -or $bounds.width -le 0 -or $bounds.height -le 0) { throw 'Foreground screenshot bounds are invalid.' }
    $path=Join-Path $screenshots ($Name+'.png')
    $bitmap=[Drawing.Bitmap]::new($bounds.width,$bounds.height,[Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics=[Drawing.Graphics]::FromImage($bitmap)
    try { $graphics.CopyFromScreen($bounds.left,$bounds.top,0,0,[Drawing.Size]::new($bounds.width,$bounds.height)) }
    finally { $graphics.Dispose() }
    try { $bitmap.Save($path,[Drawing.Imaging.ImageFormat]::Png) } finally { $bitmap.Dispose() }
    $frames.Add([ordered]@{name=$Name;path=$path;sha256=(Get-FileSha256 -Path $path);width=$bounds.width;height=$bounds.height;method='CopyFromScreen owned fixture rect after verified foreground'})
    $path
}
function Send-KeyCombo([IntPtr]$Control,[int[]]$Keys) {
    $top=[IntPtr]::Zero; $observedFocus=[IntPtr]::Zero
    $focus=[MDLiteSearchNative]::FocusTarget($Control,[ref]$top,[ref]$observedFocus)
    $foreground=[MDLiteSearchNative]::GetForegroundWindow()
    [uint32]$targetProcessId=0
    $targetThreadId=[MDLiteSearchNative]::GetWindowThreadProcessId($top,[ref]$targetProcessId)
    $guiInfo=[MDLiteSearchNative+GUIINFO]::new(); $guiInfo.cbSize=[Runtime.InteropServices.Marshal]::SizeOf($guiInfo)
    $guiInfoRead=$targetThreadId -ne 0 -and [MDLiteSearchNative]::GetGUIThreadInfo($targetThreadId,[ref]$guiInfo)
    $threadFocus=if($guiInfoRead){$guiInfo.focus}else{[IntPtr]::Zero}
    $focusReady=$focus -and $foreground -eq $top -and $guiInfoRead -and $threadFocus -eq $Control
    if (-not $focusReady) {
        return [pscustomobject]@{status='BLOCKED_FOREGROUND_OR_FOCUS';focus_ready=$false;target=$Control.ToInt64();top=$top.ToInt64();observed_focus=$observedFocus.ToInt64();foreground=$foreground.ToInt64();target_process_id=$targetProcessId;target_thread_id=$targetThreadId;target_gui_thread_focus=$threadFocus.ToInt64();target_gui_thread_info_read=$guiInfoRead;sent=0;expected=0;method='SendInput synthetic OS keyboard route; focus guard prevented delivery'}
    }
    $inputs=[Collections.Generic.List[MDLiteSearchNative+INPUT]]::new()
    foreach($key in $Keys){$inputs.Add([MDLiteSearchNative]::Key([uint16]$key,$false))}
    for($index=$Keys.Length-1;$index -ge 0;$index--){$inputs.Add([MDLiteSearchNative]::Key([uint16]$Keys[$index],$true))}
    $array=$inputs.ToArray()
    $sent=[MDLiteSearchNative]::SendInput([uint32]$array.Length,$array,[Runtime.InteropServices.Marshal]::SizeOf([type][MDLiteSearchNative+INPUT]))
    [pscustomobject]@{status=if($sent -eq $array.Length){'SENDINPUT_QUEUED'}else{'BLOCKED_SENDINPUT'};focus_ready=$true;target=$Control.ToInt64();top=$top.ToInt64();observed_focus=$observedFocus.ToInt64();foreground=$foreground.ToInt64();target_process_id=$targetProcessId;target_thread_id=$targetThreadId;target_gui_thread_focus=$threadFocus.ToInt64();target_gui_thread_info_read=$guiInfoRead;sent=$sent;expected=$array.Length;method='SendInput synthetic OS keyboard route; not physical input'}
}
function Get-SearchInputState([IntPtr]$Main,[IntPtr]$Control,[IntPtr]$Scope,[IntPtr]$TargetLabel,[int]$FixtureProcessId,[string]$DocumentIdentity,[string]$DocumentSourceHash) {
    $script:currentTestStage="Get-SearchInputState control_id=$([MDLiteSearchNative]::GetDlgCtrlID($Control)) caller_line=$($MyInvocation.ScriptLineNumber)"
    $top=$Control
    while (($parent=[MDLiteSearchNative]::GetParent($top)) -ne [IntPtr]::Zero) { $top=$parent }
    [uint32]$windowProcessId=0
    $threadId=[MDLiteSearchNative]::GetWindowThreadProcessId($Main,[ref]$windowProcessId)
    $guiInfo=[MDLiteSearchNative+GUIINFO]::new(); $guiInfo.cbSize=[Runtime.InteropServices.Marshal]::SizeOf($guiInfo)
    $guiInfoRead=$threadId -ne 0 -and [MDLiteSearchNative]::GetGUIThreadInfo($threadId,[ref]$guiInfo)
    $focusWindow=if($guiInfoRead){$guiInfo.focus}else{[IntPtr]::Zero}
    [uint32]$focusProcessId=0
    $focusThreadId=if($focusWindow -ne [IntPtr]::Zero){[MDLiteSearchNative]::GetWindowThreadProcessId($focusWindow,[ref]$focusProcessId)}else{0}
    $foreground=[MDLiteSearchNative]::GetForegroundWindow()
    [ordered]@{
        fixture_process_id=$FixtureProcessId
        main_hwnd=$Main.ToInt64()
        main_process_id=$windowProcessId
        main_thread_id=$threadId
        main_process_matches_fixture=($windowProcessId -eq $FixtureProcessId)
        main_visible=[MDLiteSearchNative]::IsWindowVisible($Main)
        main_enabled=[MDLiteSearchNative]::IsWindowEnabled($Main)
        target_control_hwnd=$Control.ToInt64()
        target_control_id=[MDLiteSearchNative]::GetDlgCtrlID($Control)
        target_control_visible=[MDLiteSearchNative]::IsWindowVisible($Control)
        target_control_enabled=[MDLiteSearchNative]::IsWindowEnabled($Control)
        target_top_hwnd=$top.ToInt64()
        foreground_hwnd=$foreground.ToInt64()
        foreground_is_main=($foreground -eq $Main)
        target_gui_thread_info_read=$guiInfoRead
        target_gui_thread_focus_hwnd=$focusWindow.ToInt64()
        target_gui_thread_focus_control_id=if($focusWindow -ne [IntPtr]::Zero){[MDLiteSearchNative]::GetDlgCtrlID($focusWindow)}else{-1}
        target_gui_thread_focus_process_id=$focusProcessId
        target_gui_thread_focus_thread_id=$focusThreadId
        target_gui_thread_focus_is_control=($focusWindow -eq $Control)
        selected_scope_index=(Send $Scope 0x0147 0 0)
        search_target_label=[MDLiteSearchNative]::Text($TargetLabel)
        active_document_identity=$DocumentIdentity
        active_document_source_sha256=$DocumentSourceHash
    }
}
function Activate-OwnedFixtureTitlebar([IntPtr]$Main,[int]$FixtureProcessId) {
    $script:currentTestStage='Activate-OwnedFixtureTitlebar foreground wait'
    [uint32]$mainProcessId=0
    [void][MDLiteSearchNative]::GetWindowThreadProcessId($Main,[ref]$mainProcessId)
    $mainRoot=[MDLiteSearchNative]::GetAncestor($Main,2)
    if ($mainRoot -ne $Main -or $mainProcessId -ne $FixtureProcessId -or
        -not [MDLiteSearchNative]::IsWindowVisible($Main) -or -not [MDLiteSearchNative]::IsWindowEnabled($Main)) {
        return [ordered]@{status='BLOCKED_NOT_OWNED_VISIBLE_ENABLED_TOPLEVEL';main_hwnd=$Main.ToInt64();root_hwnd=$mainRoot.ToInt64();main_process_id=$mainProcessId;fixture_process_id=$FixtureProcessId;pointer_restored=$true}
    }
    $rect=[MDLiteSearchNative+RECT]::new()
    if (-not [MDLiteSearchNative]::GetWindowRect($Main,[ref]$rect) -or $rect.Right -le $rect.Left -or $rect.Bottom -le $rect.Top) {
        return [ordered]@{status='BLOCKED_INVALID_WINDOW_RECT';main_hwnd=$Main.ToInt64();pointer_restored=$true}
    }
    $point=[MDLiteSearchNative+POINT]::new()
    $point.X=$rect.Left+[Math]::Min(80,[Math]::Max(20,($rect.Right-$rect.Left)/4))
    $point.Y=$rect.Top+12
    $underBefore=[MDLiteSearchNative]::WindowFromPoint($point)
    $rootBefore=[MDLiteSearchNative]::GetAncestor($underBefore,2)
    if ($underBefore -eq [IntPtr]::Zero -or $rootBefore -ne $Main) {
        return [ordered]@{status='BLOCKED_TITLEBAR_OCCLUDED_OR_NOT_OWNED';main_hwnd=$Main.ToInt64();point=@($point.X,$point.Y);window_under_point=$underBefore.ToInt64();root_under_point=$rootBefore.ToInt64();pointer_restored=$true}
    }
    $originalCursor=[MDLiteSearchNative+POINT]::new()
    if (-not [MDLiteSearchNative]::GetCursorPos([ref]$originalCursor)) {
        return [ordered]@{status='BLOCKED_CURSOR_STATE_UNAVAILABLE';main_hwnd=$Main.ToInt64();point=@($point.X,$point.Y);window_under_point=$underBefore.ToInt64();pointer_restored=$true}
    }
    $sent=0; $status='BLOCKED_TITLEBAR_CLICK'; $errorText=$null; $pointerRestored=$false
    $underNow=[IntPtr]::Zero; $rootNow=[IntPtr]::Zero
    try {
        if (-not [MDLiteSearchNative]::SetCursorPos($point.X,$point.Y)) {
            $status='BLOCKED_CURSOR_MOVE'
        } else {
            $underNow=[MDLiteSearchNative]::WindowFromPoint($point)
            $rootNow=[MDLiteSearchNative]::GetAncestor($underNow,2)
            if ($underNow -eq [IntPtr]::Zero -or $rootNow -ne $Main) {
                $status='BLOCKED_TITLEBAR_TARGET_CHANGED'
            } else {
                $mouseInputs=[Collections.Generic.List[MDLiteSearchNative+INPUT]]::new()
                $mouseInputs.Add([MDLiteSearchNative]::MouseButton($true))
                $mouseInputs.Add([MDLiteSearchNative]::MouseButton($false))
                $mouseArray=$mouseInputs.ToArray()
                $sent=[MDLiteSearchNative]::SendInput([uint32]$mouseArray.Length,$mouseArray,[Runtime.InteropServices.Marshal]::SizeOf([type][MDLiteSearchNative+INPUT]))
                $status=if($sent -eq $mouseArray.Length){'TITLEBAR_CLICK_SENT'}else{'BLOCKED_MOUSE_INPUT'}
            }
        }
    } catch { $status='BLOCKED_TITLEBAR_EXCEPTION'; $errorText=$_.Exception.Message }
    finally { $pointerRestored=[MDLiteSearchNative]::SetCursorPos($originalCursor.X,$originalCursor.Y) }
    $deadline=[DateTime]::UtcNow.AddMilliseconds(500)
    $foreground=[MDLiteSearchNative]::GetForegroundWindow()
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=500"
    while ($sent -eq 2 -and $foreground -ne $Main -and [DateTime]::UtcNow -lt $deadline) {
        Assert-RunBudget $script:currentTestStage
        Start-Sleep -Milliseconds 25
        $foreground=[MDLiteSearchNative]::GetForegroundWindow()
    }
    if ($sent -eq 2 -and $foreground -eq $Main) {
        $status=if($pointerRestored){'TITLEBAR_CLICK_ACTIVATED'}else{'BLOCKED_CURSOR_RESTORE_FAILED'}
    }
    [ordered]@{status=$status;main_hwnd=$Main.ToInt64();fixture_process_id=$FixtureProcessId;main_process_id=$mainProcessId;point=@($point.X,$point.Y);window_under_point=$underBefore.ToInt64();root_under_point=$rootBefore.ToInt64();window_under_point_after=$underNow.ToInt64();root_under_point_after=$rootNow.ToInt64();sent=$sent;expected=2;foreground_after=$foreground.ToInt64();pointer_before=@($originalCursor.X,$originalCursor.Y);pointer_restored=$pointerRestored;error=$errorText;method='Guarded visible titlebar click to owned fixture via SendInput mouse; pointer restored'}
}
function Click-Tab([IntPtr]$Main,[IntPtr]$Tabs,[int]$Index) {
    $packed=Send $Main 0x803A $Index 0
    if ($packed -lt 0) { throw "Tab center unavailable for index $Index" }
    $value=$packed -band 0xffffffffL
    $x=[int]($value -band 0xffff); $y=[int](($value -shr 16) -band 0xffff)
    [void](Send $Tabs 0x0201 1 ([MDLiteSearchNative]::MakePoint($x,$y).ToInt64()))
    [void](Send $Tabs 0x0202 0 ([MDLiteSearchNative]::MakePoint($x,$y).ToInt64()))
}
function Click-SearchHeaderClose([IntPtr]$Header) {
    $bounds=Get-Bounds $Header
    $x=[Math]::Max(1,$bounds.width-14); $y=[int]($bounds.height/2)
    [void](Send $Header 0x0202 0 ([MDLiteSearchNative]::MakePoint($x,$y).ToInt64()))
}

$workspaceName = 'search-fixture-' + $runId
$notes = Join-Path $workspace 'notes'
[IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite\.state')) | Out-Null
[IO.Directory]::CreateDirectory($notes) | Out-Null
$alpha = Join-Path $notes 'alpha.md'
$beta = Join-Path $notes 'beta.txt'
$ignored = Join-Path $notes 'ignored.md'
$hidden = Join-Path $notes 'hidden.md'
$gamma = Join-Path $notes 'gamma.txt'
$image = Join-Path $notes 'illustration.png'
$recoveryA = Join-Path $notes 'recovery-a.md'
$recoveryB = Join-Path $notes 'recovery-b.md'
$dirtyRecoveryFile = Join-Path $notes 'recovery-dirty.md'
$staleRecoveryFile = Join-Path $notes 'recovery-stale.md'
$multiline = Join-Path $notes 'multiline.txt'
# The panel-only route isolates sidebar search from table projection and navigation.
if ($Scenario -eq 'search-panel-s1-s3') {
    $alphaSource = "# Search Fixture`r`ntoken alpha token`r`n日本語 😀 end`r`n"
} else {
    $alphaSource = "# Search Fixture`r`ntoken alpha token`r`n| Item | Value |`r`n| --- | --- |`r`n| cellmark | 3 |`r`n日本語 😀 end`r`n"
}
$betaSource = "token beta`r`n"
$gammaSource = "externalword`r`n"
$recoveryASource = "recoverymark alpha`r`n"
$recoveryBSource = "recoverymark beta`r`n"
$dirtyRecoverySource = "dirtyundo source`r`n"
$staleRecoverySource = "staleundo source`r`n"
$multilineSource = "before left`r`nright after`r`n"
[IO.File]::WriteAllText($alpha,$alphaSource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($beta,$betaSource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($ignored,"ignoredword`r`n",[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($hidden,"hiddenword`r`n",[Text.UTF8Encoding]::new($false))
[IO.File]::SetAttributes($hidden,[IO.FileAttributes]::Hidden)
[IO.File]::WriteAllText($gamma,$gammaSource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($recoveryA,$recoveryASource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($recoveryB,$recoveryBSource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($dirtyRecoveryFile,$dirtyRecoverySource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($staleRecoveryFile,$staleRecoverySource,[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($multiline,$multilineSource,[Text.UTF8Encoding]::new($false))
$pngBytes = [byte[]]@(0x89,0x50,0x4E,0x47,0x0D,0x0A,0x1A,0x0A,0x00,0x00,0x00,0x0D,0x49,0x48,0x44,0x52)
[IO.File]::WriteAllBytes($image,$pngBytes)
[IO.File]::WriteAllText((Join-Path $workspace '.gitignore'),"notes/ignored.md`r`n",[Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
    "schema_version = 1`nauto_save = false`ntheme = `"dark`"`n",[Text.UTF8Encoding]::new($false))
$session = @"
schema_version = 2
active_index = 0
main_width = 1440
main_height = 920

[[document]]
path = "notes/alpha.md"

[[document]]
path = "notes/beta.txt"
"@
[IO.File]::WriteAllText((Join-Path $workspace '.mdlite\.state\session.toml'),$session,[Text.UTF8Encoding]::new($false))

$initialFiles = @{
    alpha = Get-FileSha256 -Path $alpha
    beta = Get-FileSha256 -Path $beta
    ignored = Get-FileSha256 -Path $ignored
    gamma = Get-FileSha256 -Path $gamma
    image = Get-FileSha256 -Path $image
    recovery_a = Get-FileSha256 -Path $recoveryA
    recovery_b = Get-FileSha256 -Path $recoveryB
    recovery_dirty = Get-FileSha256 -Path $dirtyRecoveryFile
    recovery_stale = Get-FileSha256 -Path $staleRecoveryFile
    multiline = Get-FileSha256 -Path $multiline
}

try {
    $process=Start-Process -FilePath $executable -ArgumentList @($workspace) -WorkingDirectory $repoRoot -WindowStyle Normal -PassThru
    $main=Find-MainWindow $process
    [void][MDLiteSearchNative]::SetWindowPos($main,[IntPtr]::Zero,80,80,1440,920,0x0004 -bor 0x0010)
    [void](Send $main $WM_SIZE 0 0)
    Wait-Ready $main
    $editor=Wait-Control $main $kEditor
    $tabs=Wait-Control $main $kTabs
    $tree=Wait-Control $main $kWorkspaceTree
    $centerEditorBefore=Get-Bounds $editor
    $tabsBefore=Get-Bounds $tabs

    Send-Command $main $kActivitySearch
    $scope=Wait-Control $main $kSearchScope
    $find=Wait-Control $main $kFindEdit
    $replace=Wait-Control $main $kReplaceEdit
    $resultTree=Wait-Control $main $kFindResults
    $count=Wait-Control $main $kSearchCount
    $status=Wait-Control $main $kSearchStatus
    $target=Wait-Control $main $kSearchTarget
    Start-Sleep -Milliseconds 100
    $searchBounds=Get-Bounds $find
    $editorBounds=Get-Bounds $editor
    $tabsBounds=Get-Bounds $tabs
    $sidePanelPass=$searchBounds.visible -and $searchBounds.left -lt $editorBounds.left -and
        $editorBounds.top -le ($tabsBounds.bottom+1) -and
        -not [MDLiteSearchNative]::IsWindowVisible($tree)
    Add-Check 'S1_sidebar_scope_layout_no_editor_padding' $sidePanelPass ([ordered]@{
        scope=(Send $scope 0x0147 0 0); target=[MDLiteSearchNative]::Text($target)
        search_input=$searchBounds; tabs=$tabsBounds; editor=$editorBounds
        explorer_tree_visible=[MDLiteSearchNative]::IsWindowVisible($tree)
    })
    [void](Capture-Frame $main '01-search-sidebar-empty')

    Set-Query $main $find 'token'
    $workspaceToken=Wait-Search $count $status 3
    $workspaceTreeState=Get-TreeCounts $resultTree
    $workspaceStatus=[MDLiteSearchNative]::Text($status)
    Add-Check 'S2_workspace_realtime_three_hits' ($workspaceToken.count -eq 3 -and $workspaceTreeState.roots -eq 3) ([ordered]@{search=$workspaceToken;tree=$workspaceTreeState;status=$workspaceStatus})
    Add-Check 'S2_binary_exclusion_is_informational' ($workspaceStatus -match '対象外 1件' -and $workspaceStatus -notmatch '部分結果|未処理') ([ordered]@{status=$workspaceStatus;image=$image})
    [void](Capture-Frame $main '02-workspace-grouped-results')
    $editBefore=Get-DocumentText $main
    $documentIdentity=if($editBefore -ceq $alphaSource){'notes/alpha.md'}elseif($editBefore -ceq $betaSource){'notes/beta.txt'}else{'unknown fixture document'}
    $documentHasher=[Security.Cryptography.SHA256]::Create()
    try { $documentSourceHash=[BitConverter]::ToString($documentHasher.ComputeHash([Text.Encoding]::UTF8.GetBytes($editBefore))).Replace('-','').ToLowerInvariant() }
    finally { $documentHasher.Dispose() }

    $multilineClipboardText="left`r`nright"
    $priorClipboard=[Windows.Forms.Clipboard]::GetDataObject()
    $clipboardRestored=$false
    try {
        [Windows.Forms.Clipboard]::SetText($multilineClipboardText,[Windows.Forms.TextDataFormat]::UnicodeText)
        [void](Send $find 0x00B1 0 -1)
        $queryBeforePaste=[MDLiteSearchNative]::Text($find)
        $editorSourceBeforePaste=Get-DocumentText $main
        $editorHashBeforePaste=Get-TextSha256 $editorSourceBeforePaste
        $pasteFocusBefore=Get-SearchInputState $main $find $scope $target $process.Id $documentIdentity $editorHashBeforePaste
        $pasteInput=Send-KeyCombo $find @(17,0x56)
        $pastedQuery=[MDLiteSearchNative]::Text($find)
        $editorSourceAfterPaste=Get-DocumentText $main
        $editorHashAfterPaste=Get-TextSha256 $editorSourceAfterPaste
        $pasteFocusAfter=Get-SearchInputState $main $find $scope $target $process.Id $documentIdentity $editorHashAfterPaste
        $pasteEvidence=[ordered]@{
            event='Ctrl+V multiline search query';method='SendInput synthetic OS keyboard route'
            input_delivery_first=$pasteInput;query_before=$queryBeforePaste;query_after_first_send=$pastedQuery
            editor_source_sha256_before=$editorHashBeforePaste;editor_source_sha256_after_first_send=$editorHashAfterPaste
            editor_source_unchanged=($editorSourceBeforePaste -ceq $editorSourceAfterPaste)
            focus_before=$pasteFocusBefore;focus_after_first_send=$pasteFocusAfter
        }
        [void]$keyInputEvidence.Add($pasteEvidence)
        $focusBlockProven=$pasteInput.status -eq 'BLOCKED_FOREGROUND_OR_FOCUS' -and
            -not $pasteInput.focus_ready -and $pasteInput.sent -eq 0 -and
            $pasteInput.target -eq $find.ToInt64() -and
            $pasteFocusAfter.main_process_matches_fixture -and
            $pasteFocusAfter.target_control_id -eq $kFindEdit -and
            $pasteFocusAfter.target_gui_thread_focus_is_control -and
            -not $pasteFocusAfter.foreground_is_main
        if ($focusBlockProven) {
            $titlebarActivation=Activate-OwnedFixtureTitlebar $main $process.Id
            $pasteEvidence['titlebar_activation_fallback']=$titlebarActivation
            if ($titlebarActivation.status -eq 'TITLEBAR_CLICK_ACTIVATED') {
                $focusAfterTitlebar=Get-SearchInputState $main $find $scope $target $process.Id $documentIdentity $editorHashAfterPaste
                $pasteEvidence['focus_after_titlebar']=$focusAfterTitlebar
                $retryAllowed=$focusAfterTitlebar.main_process_matches_fixture -and
                    $focusAfterTitlebar.foreground_is_main -and
                    $focusAfterTitlebar.target_control_id -eq $kFindEdit -and
                    $focusAfterTitlebar.target_gui_thread_focus_is_control
                if ($retryAllowed) {
                    [void](Capture-ForegroundFrame $main '02-workspace-grouped-results-foreground')
                    $pasteInputRetry=Send-KeyCombo $find @(17,0x56)
                    $pastedQueryAfterRetry=[MDLiteSearchNative]::Text($find)
                    $editorSourceAfterRetry=Get-DocumentText $main
                    $editorHashAfterRetry=Get-TextSha256 $editorSourceAfterRetry
                    $pasteFocusAfterRetry=Get-SearchInputState $main $find $scope $target $process.Id $documentIdentity $editorHashAfterRetry
                    $pasteEvidence['input_delivery_retry']=$pasteInputRetry
                    $pasteEvidence['query_after_retry']=$pastedQueryAfterRetry
                    $pasteEvidence['editor_source_sha256_after_retry']=$editorHashAfterRetry
                    $pasteEvidence['editor_source_unchanged_after_retry']=($editorSourceBeforePaste -ceq $editorSourceAfterRetry)
                    $pasteEvidence['focus_after_retry']=$pasteFocusAfterRetry
                    $pasteInput=$pasteInputRetry
                    $pastedQuery=$pastedQueryAfterRetry
                    $editorSourceAfterPaste=$editorSourceAfterRetry
                    $editorHashAfterPaste=$editorHashAfterRetry
                    $pasteFocusAfter=$pasteFocusAfterRetry
                } else {
                    $pasteEvidence['retry_blocked_reason']='Foreground or exact Find Edit focus was not verified after the titlebar activation.'
                }
            }
        }
        $pasteEvidence['input_delivery_final']=$pasteInput
        $pasteEvidence['query_after_final_send']=$pastedQuery
        $pasteEvidence['editor_source_sha256_after_final_send']=$editorHashAfterPaste
        $pasteEvidence['focus_after_final_send']=$pasteFocusAfter
        $pasteWasQueuedToFind=$pasteInput.status -eq 'SENDINPUT_QUEUED' -and $pasteInput.focus_ready -and
            $pasteInput.target -eq $find.ToInt64() -and $pasteFocusAfter.target_control_id -eq $kFindEdit -and
            $pasteFocusAfter.main_process_matches_fixture -and $pasteFocusAfter.foreground_is_main -and
            $pasteFocusAfter.target_gui_thread_focus_is_control
        if (-not $pasteWasQueuedToFind) {
            throw "Multiline Ctrl+V was not queued to the verified Find Edit; status=$($pasteInput.status) sent=$($pasteInput.sent)/$($pasteInput.expected) focus_ready=$($pasteInput.focus_ready) target=$($pasteInput.target) find=$($find.ToInt64()) focus_id=$($pasteFocusAfter.target_control_id) focus_is_find=$($pasteFocusAfter.target_gui_thread_focus_is_control)."
        }
        $pastedSearch=Wait-Search $count $status 1
        $pastedQueryAfterSearch=[MDLiteSearchNative]::Text($find)
        $pasteFocusAfterSearch=Get-SearchInputState $main $find $scope $target $process.Id $documentIdentity $editorHashAfterPaste
        $pasteEvidence['query_after_search']=$pastedQueryAfterSearch
        $pasteEvidence['focus_after_search']=$pasteFocusAfterSearch
        $normalizedClipboardQuery=$multilineClipboardText.Replace("`r`n","`n")
        $normalizedPastedQueryAfterSearch=$pastedQueryAfterSearch.Replace("`r`n","`n").Replace("`r","`n")
        $multilinePastePass=$normalizedPastedQueryAfterSearch -ceq $normalizedClipboardQuery -and
            $editorHashBeforePaste -ceq $editorHashAfterPaste -and $pastedSearch.count -eq 1 -and
            $pastedSearch.status -notmatch '部分結果|未処理'
    } finally {
        if ($null -ne $priorClipboard) { [Windows.Forms.Clipboard]::SetDataObject($priorClipboard,$true) }
        else { [Windows.Forms.Clipboard]::Clear() }
        $clipboardRestored=$true
    }
    Add-Check 'S2_pasted_multiline_query_preserves_newline_and_matches_across_lines' ($multilinePastePass -and $clipboardRestored) ([ordered]@{query_after_send=$pastedQuery;query_after_search=$pastedQueryAfterSearch;expected=$multilineClipboardText;search=$pastedSearch;input=$pasteEvidence;clipboard_restored=$clipboardRestored;fixture_content=$multilineSource})
    $currentTestStage='after multiline pass: restore token query'
    Set-Query $main $find 'token'
    $currentTestStage='after multiline pass: wait for restored Workspace token results'
    $workspaceToken=Wait-Search $count $status 3

    $currentTestStage='S1 Ctrl+F: capture pre-input editor focus state'
    $focusBeforeCtrlF=Get-SearchInputState $main $editor $scope $target $process.Id $documentIdentity $documentSourceHash
    $ctrlFBeforeQueueWindows=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
    $ctrlFAttempts=[Collections.Generic.List[object]]::new()
    $currentTestStage='S1 Ctrl+F: SendInput to verified editor'
    Write-RunBreadcrumb "INPUT_QUEUE stage=$currentTestStage scope_before=$(Send $scope 0x0147 0 0)"
    $ctrlFDeadline=[DateTime]::UtcNow.AddSeconds(2)
    $ctrlFTransitionTimer=[Diagnostics.Stopwatch]::StartNew()
    $ctrlF=Send-KeyCombo $editor @(17,0x46)
    $ctrlFAttempts.Add($ctrlF)
    $ctrlFTrace=[ordered]@{
        event='Ctrl+F';method='SendInput synthetic OS keyboard route'
        input_delivery=$ctrlF;input_attempts=@($ctrlFAttempts.ToArray())
        focus_before=$focusBeforeCtrlF;query_before='token'
        windows_before_queue=$ctrlFBeforeQueueWindows
        windows_after_queue=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
        active_document_identity=$documentIdentity;active_document_source_sha256=$documentSourceHash
        ctrl_async_state_after_send=[int][MDLiteSearchNative]::GetAsyncKeyState(0x11)
    }
    [void]$keyInputEvidence.Add($ctrlFTrace)
    $ctrlFEvidence=$ctrlFTrace
    $ctrlFEvidence['accelerator']=@{name='edit.find';command_id=$kEditFind;route='RebuildAccelerators -> TranslateAcceleratorW -> HandleMessage(kEditFind) -> ShowFindBar(CurrentFile)'}
    if ($ctrlF.status -eq 'BLOCKED_FOREGROUND_OR_FOCUS') {
        $titlebarActivation=Activate-OwnedFixtureTitlebar $main $process.Id
        $ctrlFEvidence['titlebar_activation_fallback']=$titlebarActivation
        if ($titlebarActivation.status -eq 'TITLEBAR_CLICK_ACTIVATED') {
            $ctrlF=Send-KeyCombo $editor @(17,0x46)
            $ctrlFAttempts.Add($ctrlF)
            $ctrlFEvidence['input_attempts']=@($ctrlFAttempts.ToArray())
            $ctrlFEvidence['input_delivery']=$ctrlF
            $ctrlFEvidence['windows_after_titlebar_retry']=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
        }
    }
    if ($ctrlF.status -ne 'SENDINPUT_QUEUED' -or -not $ctrlF.focus_ready) {
        throw "Ctrl+F input/focus was not proven; status=$($ctrlF.status)."
    }
    $script:currentTestStage='S1 Ctrl+F scope and Find focus transition wait'
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=2000"
    $ctrlFScopeAfterKeyboard=-1; $ctrlFFindFocusAfterKeyboard=$false; $ctrlFTransitionSamples=0
    $ctrlFTransitionAfterWindows=$null
    do {
        Assert-RunBudget $script:currentTestStage
        $ctrlFScopeAfterKeyboard=Send $scope 0x0147 0 0
        $ctrlFTransitionAfterWindows=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
        $ctrlFFindFocusAfterKeyboard=$ctrlFTransitionAfterWindows.main.gui_thread_info_read -and
            $ctrlFTransitionAfterWindows.main.gui_focus_hwnd -eq $find.ToInt64() -and
            $ctrlFTransitionAfterWindows.main.process_id -eq $process.Id -and
            $ctrlFTransitionAfterWindows.find.process_id -eq $process.Id -and
            $ctrlFTransitionAfterWindows.find.control_id -eq $kFindEdit -and
            $ctrlFTransitionAfterWindows.find.visible -and $ctrlFTransitionAfterWindows.find.enabled
        $ctrlFTransitionSamples++
        if ($ctrlFScopeAfterKeyboard -eq 0 -and $ctrlFFindFocusAfterKeyboard) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $ctrlFDeadline)
    $ctrlFTransitionTimer.Stop()
    $ctrlFTransitionElapsedMs=$ctrlFTransitionTimer.ElapsedMilliseconds
    $ctrlFEvidence['selected_scope_after_keyboard']=$ctrlFScopeAfterKeyboard
    $ctrlFEvidence['transition_elapsed_ms']=$ctrlFTransitionElapsedMs
    $ctrlFEvidence['transition_samples']=$ctrlFTransitionSamples
    $ctrlFEvidence['transition_after_windows']=$ctrlFTransitionAfterWindows
    if ($ctrlFScopeAfterKeyboard -ne 0 -or -not $ctrlFFindFocusAfterKeyboard) {
        $ctrlFDetails=[ordered]@{input=$ctrlF;focus_before=$focusBeforeCtrlF;scope_after_keyboard=$ctrlFScopeAfterKeyboard;find_focus_after_keyboard=$ctrlFFindFocusAfterKeyboard;transition_elapsed_ms=$ctrlFTransitionElapsedMs;transition_samples=$ctrlFTransitionSamples;windows_before_queue=$ctrlFBeforeQueueWindows;windows_after_transition=$ctrlFTransitionAfterWindows}
        Add-Check 'S1_ctrl_f_current_dirty_buffer_scope' $false $ctrlFDetails
        $controlledFailure="Ctrl+F semantic transition was not achieved within 2000 ms; scope=$ctrlFScopeAfterKeyboard find_focus=$ctrlFFindFocusAfterKeyboard elapsed_ms=$ctrlFTransitionElapsedMs."
        throw $controlledFailure
    }
    $fileScope=$ctrlFScopeAfterKeyboard -eq 0
    $currentTestStage='S1 Ctrl+F: read Find Edit after semantic scope transition'
    $queryAfterCtrlF=Read-TextWithDiagnostics $find $currentTestStage
    $sameQuery=$queryAfterCtrlF -ceq 'token'
    $ctrlFEvidence['query_after_scope_transition']=$queryAfterCtrlF
    $ctrlFEvidence['query_read_after_scope_transition']=$lastTextReadDiagnostic
    $fileToken=Wait-Search $count $status 2
    Add-Check 'S1_ctrl_f_current_dirty_buffer_scope' ($fileScope -and $ctrlFFindFocusAfterKeyboard -and $sameQuery -and $fileToken.count -eq 2) ([ordered]@{input=$ctrlF;delivery_evidence=$ctrlFEvidence;scope=$fileScope;find_focus=$ctrlFFindFocusAfterKeyboard;query_preserved=$sameQuery;search=$fileToken})

    $scopeBeforeCtrlShiftF=Send $scope 0x0147 0 0
    $focusBeforeCtrlShiftF=Get-SearchInputState $main $editor $scope $target $process.Id $documentIdentity $documentSourceHash
    $ctrlShiftFBeforeQueueWindows=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
    $script:currentTestStage='S1 Ctrl+Shift+F: SendInput to verified editor'
    Write-RunBreadcrumb "INPUT_QUEUE stage=$script:currentTestStage scope_before=$scopeBeforeCtrlShiftF"
    $ctrlShiftFDeadline=[DateTime]::UtcNow.AddSeconds(2)
    $ctrlShiftFTransitionTimer=[Diagnostics.Stopwatch]::StartNew()
    $ctrlShiftF=Send-KeyCombo $editor @(17,16,0x46)
    $ctrlShiftFAfterQueueWindows=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
    $ctrlShiftFEvidence=[ordered]@{event='Ctrl+Shift+F';method='SendInput synthetic OS keyboard route';accelerator=@{name='edit.findWorkspace';command_id=$kEditFindWorkspace;route='RebuildAccelerators -> TranslateAcceleratorW -> HandleMessage(kEditFindWorkspace)'};scope_before=$scopeBeforeCtrlShiftF;focus_before=$focusBeforeCtrlShiftF;windows_before_queue=$ctrlShiftFBeforeQueueWindows;input_delivery=$ctrlShiftF;windows_after_queue=$ctrlShiftFAfterQueueWindows}
    [void]$keyInputEvidence.Add($ctrlShiftFEvidence)
    if ($ctrlShiftF.status -ne 'SENDINPUT_QUEUED' -or -not $ctrlShiftF.focus_ready) { throw "Ctrl+Shift+F input/focus was not proven; status=$($ctrlShiftF.status)." }
    $script:currentTestStage='S1 Ctrl+Shift+F scope and Find focus transition wait'
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=2000"
    $ctrlShiftFScopeAfterKeyboard=-1; $ctrlShiftFFindFocusAfterKeyboard=$false; $ctrlShiftFTransitionSamples=0
    $ctrlShiftFTransitionAfterWindows=$null
    do {
        Assert-RunBudget $script:currentTestStage
        $ctrlShiftFScopeAfterKeyboard=Send $scope 0x0147 0 0
        $ctrlShiftFTransitionAfterWindows=[ordered]@{main=Get-WindowDiagnosticSnapshot $main;find=Get-WindowDiagnosticSnapshot $find}
        $ctrlShiftFFindFocusAfterKeyboard=$ctrlShiftFTransitionAfterWindows.main.gui_thread_info_read -and
            $ctrlShiftFTransitionAfterWindows.main.gui_focus_hwnd -eq $find.ToInt64() -and
            $ctrlShiftFTransitionAfterWindows.main.process_id -eq $process.Id -and
            $ctrlShiftFTransitionAfterWindows.find.process_id -eq $process.Id -and
            $ctrlShiftFTransitionAfterWindows.find.control_id -eq $kFindEdit -and
            $ctrlShiftFTransitionAfterWindows.find.visible -and $ctrlShiftFTransitionAfterWindows.find.enabled
        $ctrlShiftFTransitionSamples++
        if($ctrlShiftFScopeAfterKeyboard -eq 1 -and $ctrlShiftFFindFocusAfterKeyboard){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $ctrlShiftFDeadline)
    $ctrlShiftFTransitionTimer.Stop()
    $ctrlShiftFTransitionElapsedMs=$ctrlShiftFTransitionTimer.ElapsedMilliseconds
    $workspaceScope=$ctrlShiftFScopeAfterKeyboard -eq 1
    $ctrlShiftFEvidence['scope_after_keyboard']=$ctrlShiftFScopeAfterKeyboard
    $ctrlShiftFEvidence['find_focus_after_keyboard']=$ctrlShiftFFindFocusAfterKeyboard
    $ctrlShiftFEvidence['transition_elapsed_ms']=$ctrlShiftFTransitionElapsedMs
    $ctrlShiftFEvidence['transition_samples']=$ctrlShiftFTransitionSamples
    $ctrlShiftFEvidence['transition_after_windows']=$ctrlShiftFTransitionAfterWindows
    if(-not $workspaceScope -or -not $ctrlShiftFFindFocusAfterKeyboard){
        $details=[ordered]@{input=$ctrlShiftF;scope_before=$scopeBeforeCtrlShiftF;scope_after=$ctrlShiftFScopeAfterKeyboard;find_focus_after_keyboard=$ctrlShiftFFindFocusAfterKeyboard;transition_elapsed_ms=$ctrlShiftFTransitionElapsedMs;transition_samples=$ctrlShiftFTransitionSamples;windows_before_queue=$ctrlShiftFBeforeQueueWindows;windows_after_transition=$ctrlShiftFTransitionAfterWindows}
        Add-Check 'S1_ctrl_shift_f_workspace_scope' $false $details
        $controlledFailure="Ctrl+Shift+F semantic transition was not achieved within 2000 ms; scope=$ctrlShiftFScopeAfterKeyboard find_focus=$ctrlShiftFFindFocusAfterKeyboard elapsed_ms=$ctrlShiftFTransitionElapsedMs."
        throw $controlledFailure
    }
    $currentTestStage='S1 Ctrl+Shift+F: read Find Edit after semantic scope transition'
    $ctrlShiftQuery=Read-TextWithDiagnostics $find $currentTestStage
    $ctrlShiftQueryPreserved=$ctrlShiftQuery -ceq 'token'
    $ctrlShiftFEvidence['query_after_scope_transition']=$ctrlShiftQuery
    $ctrlShiftFEvidence['query_read_after_scope_transition']=$lastTextReadDiagnostic
    $workspaceAgain=Wait-Search $count $status 3
    Add-Check 'S1_ctrl_shift_f_workspace_scope' ($ctrlShiftF.status -eq 'SENDINPUT_QUEUED' -and $workspaceScope -and $ctrlShiftFFindFocusAfterKeyboard -and $ctrlShiftQueryPreserved -and $workspaceAgain.count -eq 3) ([ordered]@{input=$ctrlShiftF;scope=$workspaceScope;find_focus=$ctrlShiftFFindFocusAfterKeyboard;query_preserved=$ctrlShiftQueryPreserved;search=$workspaceAgain;delivery_evidence=$ctrlShiftFEvidence})

    Set-Query $main $find 'alpha'
    $alphaSearch=Wait-Search $count $status 2
    Send-Command $main $kActivityExplorer
    $explorerReturned=-not [MDLiteSearchNative]::IsWindowVisible($find)
    Send-Command $main $kActivitySearch
    $currentTestStage='S1 Explorer return: verify Find Edit query preservation'
    $queryAfterExplorer=Read-TextWithDiagnostics $find $currentTestStage
    $queryPreserved=$queryAfterExplorer -ceq 'alpha'
    $ctrlFEvidence['query_read_after_explorer_return']=$lastTextReadDiagnostic
    $restoredSearch=Wait-Search $count $status 2
    Add-Check 'S1_explorer_switch_preserves_search_input' ($explorerReturned -and $queryPreserved -and $restoredSearch.count -eq 2) ([ordered]@{explorer_hidden_search=$explorerReturned;query_preserved=$queryPreserved;search=$restoredSearch;expected_fixture_hits=@('notes/alpha.md','notes/recovery-a.md')})

    [void](Send $find $WM_KEYDOWN 0x1B 0)
    Start-Sleep -Milliseconds 80
    $escapeClosed=-not [MDLiteSearchNative]::IsWindowVisible($find) -and [MDLiteSearchNative]::IsWindowVisible($tree)
    $currentTestStage='S1 Escape close: verify Find Edit query preservation'
    $escapeQuery=Read-TextWithDiagnostics $find $currentTestStage
    $ctrlFEvidence['query_read_after_escape']=$lastTextReadDiagnostic
    Send-Command $main $kActivitySearch
    $searchHeader=Wait-Control $main $kPanelHeaderExplorer
    Click-SearchHeaderClose $searchHeader
    Start-Sleep -Milliseconds 80
    $xClosed=-not [MDLiteSearchNative]::IsWindowVisible($find) -and [MDLiteSearchNative]::IsWindowVisible($tree)
    Add-Check 'S1_escape_and_header_x_close_without_losing_query' ($escapeClosed -and $escapeQuery -ceq 'alpha' -and $xClosed) ([ordered]@{escape_closed=$escapeClosed;x_closed=$xClosed;query=$escapeQuery})
    Send-Command $main $kActivitySearch

    Set-Query $main $find 'token'
    [void](Wait-Search $count $status 3)
    Set-Query $main $find 'alpha'
    Set-Query $main $find 'not-present'
    $latest=Wait-Search $count $status 0
    $currentTestStage='S2 latest-generation zero-match count and exclusion-only tree inspection'
    $latestStatus=[MDLiteSearchNative]::Text($status)
    $latestTree=Get-TreeCounts $resultTree
    $latestFrame=Capture-Frame $main '03-zero-match-exclusion-only-results'
    $noMatchCount=$latest.count -eq 0 -and $latestStatus -match '検索結果なし'
    $exclusionOnlyTree=$latestStatus -match '対象外 1件' -and $latestTree.roots -eq 1 -and $latestTree.total -eq 2
    Add-Check 'S2_latest_generation_zero_match_nodes_count_and_highlight_frame' ($noMatchCount -and $exclusionOnlyTree) ([ordered]@{search=$latest;status=$latestStatus;tree=$latestTree;zero_match_count=$noMatchCount;binary_exclusion_issue_only_tree=$exclusionOnlyTree;expected_excluded_fixture=$image;no_match_screenshot=$latestFrame})

    Set-Checkbox $main (Wait-Control $main $kFindRegex) $kFindRegex $true
    Set-Query $main $find '['
    $regexDeadline=[DateTime]::UtcNow.AddSeconds(8)
    $script:currentTestStage='S2 invalid regex status wait'
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=8000"
    do { Assert-RunBudget $script:currentTestStage; $regexStatus=[MDLiteSearchNative]::Text($status); if ($regexStatus -notmatch '検索中') { break }; Start-Sleep -Milliseconds 40 }
    while ([DateTime]::UtcNow -lt $regexDeadline)
    $regexInvalid=$regexStatus -match '正規表現|regex|無効'
    $regexTree=Get-TreeCounts $resultTree
    Add-Check 'S2_invalid_regex_is_inline_and_clears_old_results' ($regexInvalid -and $regexTree.total -eq 0) ([ordered]@{status=$regexStatus;tree=$regexTree})
    Set-Checkbox $main (Wait-Control $main $kFindRegex) $kFindRegex $false
    Set-Query $main $find ''
    $emptyStatus=[MDLiteSearchNative]::Text($status)
    Add-Check 'S2_empty_query_clears_results' ($emptyStatus -match '検索語を入力' -and (Get-TreeCounts $resultTree).total -eq 0) ([ordered]@{status=$emptyStatus})

    Send-Command $main $kSearchDetailsToggle $BN_CLICKED (Wait-Control $main $kSearchDetailsToggle)
    $include=Wait-Control $main $kFindInclude
    $exclude=Wait-Control $main $kFindExclude
    $ignore=Wait-Control $main $kSearchIgnore
    $hiddenToggle=Wait-Control $main $kSearchHidden
    $detailsVisible=[MDLiteSearchNative]::IsWindowVisible($include) -and [MDLiteSearchNative]::IsWindowVisible($ignore)
    Add-Check 'S3_collapsible_details_expose_optional_globs_and_ignore' $detailsVisible ([ordered]@{include=[MDLiteSearchNative]::IsWindowVisible($include);exclude=[MDLiteSearchNative]::IsWindowVisible($exclude);ignore=[MDLiteSearchNative]::IsWindowVisible($ignore)})

    Set-Scope $main $scope 1
    Set-Query $main $find 'token'
    [MDLiteSearchNative]::SetText($include,'')
    Send-Command $main $kFindInclude $EN_CHANGE $include
    $unrestricted=Wait-Search $count $status 3
    Add-Check 'S3_empty_include_is_unrestricted_including_text_files' ($unrestricted.count -eq 3) ([ordered]@{search=$unrestricted;include=[MDLiteSearchNative]::Text($include)})
    [MDLiteSearchNative]::SetText($include,'notes/*.txt'); Send-Command $main $kFindInclude $EN_CHANGE $include
    $includedTextOnly=Wait-Search $count $status 1
    Add-Check 'S3_include_glob_limits_results' ($includedTextOnly.count -eq 1) ([ordered]@{search=$includedTextOnly;include=[MDLiteSearchNative]::Text($include)})
    [MDLiteSearchNative]::SetText($include,''); Send-Command $main $kFindInclude $EN_CHANGE $include
    [MDLiteSearchNative]::SetText($exclude,'notes/*.txt'); Send-Command $main $kFindExclude $EN_CHANGE $exclude
    $excludedTextOnly=Wait-Search $count $status 2
    Add-Check 'S3_exclude_glob_filters_text_file' ($excludedTextOnly.count -eq 2) ([ordered]@{search=$excludedTextOnly;exclude=[MDLiteSearchNative]::Text($exclude)})
    [MDLiteSearchNative]::SetText($exclude,''); Send-Command $main $kFindExclude $EN_CHANGE $exclude

    Set-Query $main $find 'ignoredword'
    $ignoredByDefault=Wait-Search $count $status 0
    Set-Checkbox $main $ignore $kSearchIgnore $false
    $ignoreDisabled=Wait-Search $count $status 1
    Add-Check 'S3_gitignore_default_and_visible_toggle' ($ignoredByDefault.count -eq 0 -and $ignoreDisabled.count -eq 1) ([ordered]@{default=$ignoredByDefault;disabled=$ignoreDisabled})
    Set-Checkbox $main $ignore $kSearchIgnore $true

    Set-Query $main $find 'hiddenword'
    $hiddenDefault=Wait-Search $count $status 0
    Set-Checkbox $main $hiddenToggle $kSearchHidden $true
    $hiddenIncluded=Wait-Search $count $status 1
    Add-Check 'S3_hidden_files_are_opt_in' ($hiddenDefault.count -eq 0 -and $hiddenIncluded.count -eq 1) ([ordered]@{default=$hiddenDefault;included=$hiddenIncluded})
    Set-Checkbox $main $hiddenToggle $kSearchHidden $false

    if ($Scenario -eq 'search-panel-s1-s3') {
        Set-Scope $main $scope 0
        Set-Query $main $find 'beta'
        [void](Wait-Search $count $status 0)
        Click-Tab $main $tabs 1
        $tabDeadline=[DateTime]::UtcNow.AddSeconds(8)
        $script:currentTestStage='search-panel beta tab target wait'
        Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=8000"
        do { Assert-RunBudget $script:currentTestStage; $targetText=[MDLiteSearchNative]::Text($target); if ($targetText -match 'beta') { break }; Start-Sleep -Milliseconds 40 }
        while ([DateTime]::UtcNow -lt $tabDeadline)
        $betaEditor=Wait-Control $main $kEditor
        [void](Send $betaEditor 0x00B1 0 -1)
        [void](Send $betaEditor 0x00B2 1 0)
        Replace-Selection $betaEditor 'dirtymark token token'
        Set-Query $main $find 'dirtymark'
        $dirtyCurrent=Wait-Search $count $status 1
        Set-Scope $main $scope 1
        $dirtyWorkspace=Wait-Search $count $status 1
        $dirtyState=[ordered]@{dirty=(Send $main $kTestGetDocumentStateMessage 0 0);source=(Get-DocumentText $main)}
        Add-Check 'S3_unsaved_buffer_wins_workspace_disk' ($dirtyState.dirty -and $dirtyState.source -ceq 'dirtymark token token' -and $dirtyCurrent.count -eq 1 -and $dirtyWorkspace.count -eq 1 -and (Get-Content -Raw -LiteralPath $beta) -ceq $betaSource) ([ordered]@{current=$dirtyCurrent;workspace=$dirtyWorkspace;document=$dirtyState;disk=(Get-Content -Raw -LiteralPath $beta)})
    }

    # The full route continues through table navigation, Unicode offsets, and replacement.
    if ($Scenario -eq 'full') {
        Set-Scope $main $scope 0
    Set-Query $main $find 'token'
    [void](Wait-Search $count $status 2)
    [void](Send $main $kTestSetSelectionBySourceMessage 0 0)
    $firstToken=$alphaSource.IndexOf('token')
    $secondToken=$alphaSource.LastIndexOf('token')
    $expectedFirst=New-RangePair $firstToken 5
    $expectedSecond=New-RangePair $secondToken 5
    if ($expectedFirst.Count -ne 2 -or $expectedSecond.Count -ne 2) { throw 'S4 expected source ranges must each contain exactly two elements.' }
    # The fixture's leading source CRLF at [16,18) projects to one native paragraph terminator [16,17).
    # Both token hits follow that prefix, so native CP is one code unit lower than the strict source pair.
    $expectedFirstNative=New-RangePair ($firstToken-1) 5
    $expectedSecondNative=New-RangePair ($secondToken-1) 5
    if ($expectedFirstNative.Count -ne 2 -or $expectedSecondNative.Count -ne 2) { throw 'S4 expected native ranges must each contain exactly two elements.' }
    $enterQueryBefore=Read-TextWithDiagnostics $find 'S4 OS Enter query before'
    $enterBaseline=Capture-InputSettleBaseline $main $find $process.Id
    $enterInput=Send-KeyCombo $find @(0x0D)
    $enterSettle=Wait-SourceRangeAfterInput $main $find $process.Id $expectedSecond 'Enter' $enterBaseline
    $enterRange=[int[]]@($enterSettle.observed_range)
    $enterNative=Get-RichEditSelectionEvidence $editor $process.Id
    $enterNativePass=(Test-RangePair $expectedSecondNative $enterNative.range) -and $enterNative.selected_text -ceq 'token'
    $enterSearchDeadline=[DateTime]::UtcNow.AddMilliseconds(2000)
    $enterSearchStatus='';$enterSearchCount=0
    do {
        Assert-RunBudget 'S4 OS Enter query/results settle'
        $enterSearchStatus=[MDLiteSearchNative]::Text($status);$enterSearchCount=Get-Count $count
        if($enterSearchStatus -notmatch '検索中'){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $enterSearchDeadline)
    $enterSearchSettled=$enterSearchStatus -notmatch '検索中'
    $enterQueryAfter=Read-TextWithDiagnostics $find 'S4 OS Enter query after'
    $enterQueryPass=$enterQueryBefore -ceq 'token' -and $enterQueryAfter -ceq 'token'
    $enterSearchPass=$enterSearchSettled -and $enterSearchCount -eq 2
    $enterPass=$enterInput.status -eq 'SENDINPUT_QUEUED' -and $enterSettle.matched -and
        (Test-RangePair $expectedSecond $enterRange) -and $enterNativePass -and $enterQueryPass -and $enterSearchPass
    Add-Check 'S4_os_enter_preserves_token_query_and_two_hits' $enterPass ([ordered]@{
        input=$enterInput;settle=$enterSettle;expected_range=@($expectedSecond);observed_range=@($enterRange)
        native=$enterNative;native_match=$enterNativePass;query_before=$enterQueryBefore;query_after=$enterQueryAfter
        query_after_read=$lastTextReadDiagnostic;status=$enterSearchStatus;count=$enterSearchCount;search_settled=$enterSearchSettled
    })
    if(-not $enterPass){throw 'OS Enter did not reach the next hit while preserving query token and the settled two-hit result.'}

    $previousQueryBefore=Read-TextWithDiagnostics $find 'S4 Previous button query before'
    Send-Command $main $kSearchPrevious $BN_CLICKED (Wait-Control $main $kSearchPrevious)
    $previousRange=Get-SourceRangePair $main
    $previousNative=Get-RichEditSelectionEvidence $editor $process.Id
    $previousNativePass=(Test-RangePair $expectedFirstNative $previousNative.range) -and $previousNative.selected_text -ceq 'token'
    $previousSearchDeadline=[DateTime]::UtcNow.AddMilliseconds(2000)
    $previousSearchStatus='';$previousSearchCount=0
    do {
        Assert-RunBudget 'S4 Previous button query/results settle'
        $previousSearchStatus=[MDLiteSearchNative]::Text($status);$previousSearchCount=Get-Count $count
        if($previousSearchStatus -notmatch '検索中'){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $previousSearchDeadline)
    $previousSearchSettled=$previousSearchStatus -notmatch '検索中'
    $previousQueryAfter=Read-TextWithDiagnostics $find 'S4 Previous button query after'
    $previousPass=(Test-RangePair $expectedFirst $previousRange) -and $previousNativePass -and
        $previousQueryBefore -ceq 'token' -and $previousQueryAfter -ceq 'token' -and
        $previousSearchSettled -and $previousSearchCount -eq 2
    Add-Check 'S4_previous_button_establishes_first_hit_baseline' $previousPass ([ordered]@{
        expected_range=@($expectedFirst);observed_range=@($previousRange);native=$previousNative;native_match=$previousNativePass
        query_before=$previousQueryBefore;query_after=$previousQueryAfter;status=$previousSearchStatus
        count=$previousSearchCount;search_settled=$previousSearchSettled
    })
    if(-not $previousPass){throw 'Previous button did not establish the first-hit 18..23 / 17..22 baseline with query token and two settled hits.'}

    $shiftEnterQueryBefore=Read-TextWithDiagnostics $find 'S4 OS Shift+Enter query before'
    $shiftEnterBaseline=Capture-InputSettleBaseline $main $find $process.Id
    $shiftEnter=Send-KeyCombo $find @(16,0x0D)
    $shiftEnterSettle=Wait-SourceRangeAfterInput $main $find $process.Id $expectedSecond 'Shift+Enter' $shiftEnterBaseline
    $shiftRange=[int[]]@($shiftEnterSettle.observed_range)
    $shiftNative=Get-RichEditSelectionEvidence $editor $process.Id
    $shiftNativePass=(Test-RangePair $expectedSecondNative $shiftNative.range) -and $shiftNative.selected_text -ceq 'token'
    $shiftSearchDeadline=[DateTime]::UtcNow.AddMilliseconds(2000)
    $shiftSearchStatus='';$shiftSearchCount=0
    do {
        Assert-RunBudget 'S4 OS Shift+Enter query/results settle'
        $shiftSearchStatus=[MDLiteSearchNative]::Text($status);$shiftSearchCount=Get-Count $count
        if($shiftSearchStatus -notmatch '検索中'){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $shiftSearchDeadline)
    $shiftSearchSettled=$shiftSearchStatus -notmatch '検索中'
    $shiftEnterQueryAfter=Read-TextWithDiagnostics $find 'S4 OS Shift+Enter query after'
    $shiftQueryPass=$shiftEnterQueryBefore -ceq 'token' -and $shiftEnterQueryAfter -ceq 'token'
    $shiftSearchPass=$shiftSearchSettled -and $shiftSearchCount -eq 2
    $shiftPass=$shiftEnter.status -eq 'SENDINPUT_QUEUED' -and $shiftEnterSettle.matched -and
        (Test-RangePair $expectedSecond $shiftRange) -and $shiftNativePass -and $shiftQueryPass -and $shiftSearchPass
    Add-Check 'S4_os_shift_enter_preserves_token_query_and_two_hits' $shiftPass ([ordered]@{
        input=$shiftEnter;settle=$shiftEnterSettle;expected_range=@($expectedSecond);observed_range=@($shiftRange)
        native=$shiftNative;native_match=$shiftNativePass;query_before=$shiftEnterQueryBefore;query_after=$shiftEnterQueryAfter
        query_after_read=$lastTextReadDiagnostic;status=$shiftSearchStatus;count=$shiftSearchCount;search_settled=$shiftSearchSettled
    })
    if(-not $shiftPass){throw 'OS Shift+Enter did not wrap to the second hit while preserving query token and the settled two-hit result.'}
    # Previous from the first hit wraps Shift+Enter to the second; F3 then wraps to first, and Shift+F3 returns to second.
    $f3Baseline=Capture-InputSettleBaseline $main $editor $process.Id
    $f3=Send-KeyCombo $editor @(0x72)
    $f3Settle=Wait-SourceRangeAfterInput $main $editor $process.Id $expectedFirst 'F3' $f3Baseline
    $f3Range=[int[]]@($f3Settle.observed_range)
    $f3Native=Get-RichEditSelectionEvidence $editor $process.Id
    $f3NativePass=(Test-RangePair $expectedFirstNative $f3Native.range) -and $f3Native.selected_text -ceq 'token'
    $f3Pass=$f3.status -eq 'SENDINPUT_QUEUED' -and $f3Settle.matched -and
        (Test-RangePair $expectedFirst $f3Range) -and $f3NativePass
    $shiftF3Baseline=Capture-InputSettleBaseline $main $editor $process.Id
    $shiftF3=Send-KeyCombo $editor @(16,0x72)
    $shiftF3Settle=Wait-SourceRangeAfterInput $main $editor $process.Id $expectedSecond 'Shift+F3' $shiftF3Baseline
    $shiftF3Range=[int[]]@($shiftF3Settle.observed_range)
    $shiftF3Native=Get-RichEditSelectionEvidence $editor $process.Id
    $shiftF3NativePass=(Test-RangePair $expectedSecondNative $shiftF3Native.range) -and $shiftF3Native.selected_text -ceq 'token'
    $shiftF3Pass=$shiftF3.status -eq 'SENDINPUT_QUEUED' -and $shiftF3Settle.matched -and
        (Test-RangePair $expectedSecond $shiftF3Range) -and $shiftF3NativePass
    Add-Check 'S4_enter_shift_enter_f3_and_shift_f3_wrap' ($enterPass -and $previousPass -and $shiftPass -and $f3Pass -and $shiftF3Pass) ([ordered]@{
        expected_first=@($expectedFirst);expected_second=@($expectedSecond)
        expected_first_native=@($expectedFirstNative);expected_second_native=@($expectedSecondNative)
        enter=@($enterRange);previous=@($previousRange);shift_enter=@($shiftRange)
        f3=@($f3Range);shift_f3=@($shiftF3Range)
        native=@{enter=$enterNative;previous=$previousNative;shift_enter=$shiftNative;f3=$f3Native;shift_f3=$shiftF3Native}
        native_match=@{enter=$enterNativePass;previous=$previousNativePass;shift_enter=$shiftNativePass;f3=$f3NativePass;shift_f3=$shiftF3NativePass}
        input=@($enterInput,$shiftEnter,$f3,$shiftF3)
        settle=@($enterSettle,$shiftEnterSettle,$f3Settle,$shiftF3Settle)
    })

    Set-Scope $main $scope 1
    Set-Query $main $find 'cellmark'
    [void](Wait-Search $count $status 1)
    $cellRoot=Get-FirstResult $resultTree 0 $process.Id
    Select-TreeItem $resultTree $cellRoot.child
    Start-Sleep -Milliseconds 120
    $cellOffset=$alphaSource.IndexOf('cellmark')
    $cellSourceExpected=New-RangePair $cellOffset 8
    if ($cellSourceExpected.Count -ne 2) { throw 'S4 expected table source range must contain exactly two elements.' }
    $cellParentRange=Get-SourceRangePair $main
    $cellViewportEvidence=[Collections.Generic.List[object]]::new()
    $cellViewport=[IntPtr]::Zero
    $cellViewportText=''
    $cellViewportRange=[int[]]@()
    $cellViewportSelectedText=''
    $cellViewportOwnerPid=0
    foreach($candidate in (Find-Controls $editor 0x7100)) {
        try {
            $candidateText=[MDLiteSearchNative]::Text($candidate)
            $candidateSelection=Get-RichEditSelectionEvidence $candidate $process.Id
            $candidateRange=[int[]]@($candidateSelection.range)
            $candidateOffset=$candidateText.IndexOf('cellmark',[StringComparison]::Ordinal)
            $candidateMatch=$candidateSelection.selected_text -ceq 'cellmark'
            $cellViewportEvidence.Add([ordered]@{
                window=Get-WindowDiagnosticSnapshot $candidate;text=$candidateText;match_offset=$candidateOffset
                observed_range=@($candidateRange);selected_text=$candidateSelection.selected_text
                owner_pid=$candidateSelection.owner_pid;remote_buffer_bytes=$candidateSelection.remote_buffer_bytes
                selected_text_matches_query=$candidateMatch
            })
            if ($candidateMatch -and $candidateSelection.owner_pid -eq $process.Id -and $cellViewport -eq [IntPtr]::Zero) {
                $cellViewport=$candidate;$cellViewportText=$candidateText;$cellViewportRange=$candidateRange
                $cellViewportSelectedText=$candidateSelection.selected_text;$cellViewportOwnerPid=$candidateSelection.owner_pid
            }
        } catch {
            $cellViewportEvidence.Add([ordered]@{hwnd=$candidate.ToInt64();error=$_.Exception.Message})
            if ($_.Exception.Message -match 'remote buffer retained until owner process exit') { throw }
        }
    }
    $cellViewportPass=$cellRoot.child -ne 0 -and $cellViewport -ne [IntPtr]::Zero -and
        $cellViewportOwnerPid -eq $process.Id -and $cellViewportRange.Count -eq 2 -and
        $cellViewportSelectedText -ceq 'cellmark' -and (Test-RangePair $cellSourceExpected $cellParentRange)
    Add-Check 'S4_workspace_tree_cell_result_selects_source_range' $cellViewportPass ([ordered]@{
        result=$cellRoot;source_expected=@($cellSourceExpected);parent_source_query=@($cellParentRange)
        viewport_control_id=0x7100;selected_viewport=if($cellViewport -ne [IntPtr]::Zero){Get-WindowDiagnosticSnapshot $cellViewport}else{$null}
        viewport_owner_pid=$cellViewportOwnerPid;viewport_text=$cellViewportText
        viewport_selected_text_expected='cellmark';viewport_selected_text=$cellViewportSelectedText
        viewport_observed=@($cellViewportRange);activation='Select-TreeItem sends TVM_SELECTITEM then WM_KEYDOWN VK_RETURN through the existing result-tree route'
        viewport_candidates=@($cellViewportEvidence.ToArray())
    })
    [void](Capture-Frame $main '03-table-search-result-selected')

    Set-Scope $main $scope 0
    Set-Query $main $find '😀'
    [void](Wait-Search $count $status 1)
    $emojiOffset=$alphaSource.IndexOf('😀')
    [void](Send $main $kTestSetSelectionBySourceMessage $emojiOffset $emojiOffset)
    [void](Send $find $WM_KEYDOWN 0x0D 0)
    [void](Send $find $WM_KEYUP 0x0D 0)
    Start-Sleep -Milliseconds 80
    $emojiExpected=New-RangePair $emojiOffset 2
    $emojiRange=Get-SourceRangePair $main
    if ($emojiExpected.Count -ne 2 -or $emojiRange.Count -ne 2) { throw 'S5 Unicode source range must contain exactly two elements.' }
    Add-Check 'S5_unicode_source_offsets_preserve_utf16_selection' (Test-RangePair $emojiExpected $emojiRange) ([ordered]@{expected=@($emojiExpected);observed=@($emojiRange)})

    Set-Scope $main $scope 0
    Set-Query $main $find 'beta'
    [void](Wait-Search $count $status 0)
    Click-Tab $main $tabs 1
    $tabTargetDeadline=[DateTime]::UtcNow.AddSeconds(8)
        $script:currentTestStage='search-panel beta query refresh wait'
        Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=8000"
        do { Assert-RunBudget $script:currentTestStage; $targetText=[MDLiteSearchNative]::Text($target); $tabScopeCount=Get-Count $count; if ($targetText -match 'beta' -and $tabScopeCount -eq 1) { break }; Start-Sleep -Milliseconds 40 }
    while ([DateTime]::UtcNow -lt $tabTargetDeadline)
    Add-Check 'S5_current_file_scope_follows_active_tab' ($targetText -match 'beta' -and $tabScopeCount -eq 1) ([ordered]@{target=$targetText;count=$tabScopeCount})

    $betaEditor=Wait-Control $main $kEditor
    [void](Send $betaEditor 0x00B1 0 -1)
    [void](Send $betaEditor 0x00B2 1 0)
    Replace-Selection $betaEditor 'dirtymark token token'
    Start-Sleep -Milliseconds 320
    Set-Query $main $find 'dirtymark'
    $dirtyCurrent=Wait-Search $count $status 1
    Set-Scope $main $scope 1
    $dirtyWorkspace=Wait-Search $count $status 1
    $dirtyState=[ordered]@{dirty=(Send $main $kTestGetDocumentStateMessage 0 0);source=(Get-DocumentText $main)}
    Add-Check 'S3_S5_unsaved_buffer_wins_workspace_disk' ($dirtyState.dirty -and $dirtyState.source -ceq 'dirtymark token token' -and $dirtyCurrent.count -eq 1 -and $dirtyWorkspace.count -eq 1) ([ordered]@{current=$dirtyCurrent;workspace=$dirtyWorkspace;document=$dirtyState;disk=(Get-Content -Raw -LiteralPath $beta)})
    [void](Capture-Frame $main '04-unsaved-workspace-results')

    Set-Query $main $find 'token'
    [void](Wait-Search $count $status 4)
    $treeBefore=Get-TreeCounts $resultTree
    Set-Query $main $find ''
    $emptyCleared=(Get-TreeCounts $resultTree).total -eq 0
    Set-Query $main $find 'token'
    [void](Wait-Search $count $status 4)
    [MDLiteSearchNative]::SetText($replace,'SWAP')
    Send-Command $main $kSearchReplaceToggle $BN_CLICKED (Wait-Control $main $kSearchReplaceToggle)
    $rollbackAction=Wait-Control $main $kRollbackWorkspaceReplace
    $rollbackActionVisible=[MDLiteSearchNative]::IsWindowVisible($rollbackAction) -and [MDLiteSearchNative]::IsWindowEnabled($rollbackAction)
    Add-Check 'S6_closed_file_rollback_action_is_discoverable_in_replace_panel' $rollbackActionVisible ([ordered]@{visible=[MDLiteSearchNative]::IsWindowVisible($rollbackAction);enabled=[MDLiteSearchNative]::IsWindowEnabled($rollbackAction)})
    # Document buttons dispatch a selected Workspace result while that scope is
    # active. Exercise the current-file route explicitly and wait after Undo.
    Set-Scope $main $scope 0
    $currentReplaceSearch=Wait-Search $count $status 2
    $dirtyExpected='dirtymark token token'
    $currentReplaceBefore=Get-DocumentText $main
    $currentReplaceUndoBefore=Send $main $kTestGetDocumentStateMessage 1 0
    $currentReplaceScope=Send $scope 0x0147 0 0
    $currentReplaceReady=$currentReplaceScope -eq 0 -and $currentReplaceBefore -ceq $dirtyExpected -and $currentReplaceSearch.count -eq 2
    Add-Check 'S6_current_document_scope_and_source_precondition' $currentReplaceReady ([ordered]@{scope=$currentReplaceScope;source=$currentReplaceBefore;search=$currentReplaceSearch;undo=$currentReplaceUndoBefore})
    if(-not $currentReplaceReady){throw 'Current-document replacement precondition failed; no replacement attempted.'}
    [void](Send $main $kTestSetSelectionBySourceMessage 0 0)
    Send-Command $main $kReplaceOne $BN_CLICKED (Wait-Control $main $kReplaceOne)
    Start-Sleep -Milliseconds 160
    $oneSource=Get-DocumentText $main
    $oneUndo=Send $main $kTestGetDocumentStateMessage 1 0
    $oneApplied=$oneSource -ceq 'dirtymark SWAP token' -and $oneUndo -eq ($currentReplaceUndoBefore+1)
    Add-Check 'S6_current_document_one_replace_is_undoable' $oneApplied ([ordered]@{source=$oneSource;undo=$oneUndo;undo_before=$currentReplaceUndoBefore})
    if(-not $oneApplied){throw 'One replacement was not applied as one transaction; Undo was not sent.'}
    [void](Send $betaEditor 0x0304 0 0)
    Start-Sleep -Milliseconds 120
    $oneUndoRestored=Get-DocumentText $main
    Add-Check 'S6_current_one_replace_undo_restores_source' ($oneUndoRestored -ceq $dirtyExpected) ([ordered]@{expected=$dirtyExpected;observed=$oneUndoRestored})
    if($oneUndoRestored -cne $dirtyExpected){throw 'One-replace Undo did not restore the fixture; Replace All was not sent.'}
    $currentReplaceSearchAfterUndo=Wait-Search $count $status 2
    $allUndoBefore=Send $main $kTestGetDocumentStateMessage 1 0
    [void](Send $main $kTestSetSelectionBySourceMessage 0 0)
    Send-Command $main $kReplaceDocument $BN_CLICKED (Wait-Control $main $kReplaceDocument)
    Start-Sleep -Milliseconds 160
    $allSource=Get-DocumentText $main
    $allUndo=Send $main $kTestGetDocumentStateMessage 1 0
    $allApplied=$allSource -ceq 'dirtymark SWAP SWAP' -and $allUndo -eq ($allUndoBefore+1)
    Add-Check 'S6_current_document_replace_all_is_single_transaction' $allApplied ([ordered]@{source=$allSource;undo=$allUndo;undo_before=$allUndoBefore;search_before=$currentReplaceSearchAfterUndo})
    if(-not $allApplied){throw 'Replace All was not applied as one transaction; Undo was not sent.'}
    [void](Send $betaEditor 0x0304 0 0)
    Start-Sleep -Milliseconds 100
    $allUndoRestored=Get-DocumentText $main
    Add-Check 'S6_current_all_replace_undo_restores_source' ($allUndoRestored -ceq $dirtyExpected) ([ordered]@{expected=$dirtyExpected;observed=$allUndoRestored})

    Set-Scope $main $scope 1
    Set-Query $main $find 'token'
    [void](Wait-Search $count $status 4)
    [MDLiteSearchNative]::SetText($replace,'SHOULD_NOT_WRITE')
    $betaBeforePreview=[IO.File]::ReadAllText($beta)
    $gammaBeforePreview=[IO.File]::ReadAllText($gamma)
    $imageBeforePreview=Get-FileSha256 -Path $image
    if ((Send $main $kTestSetReplaceConfirmationUiMessage 1 0) -ne 1) { throw 'Could not enable owned replacement confirmation for the fixture.' }
    Post-Command $main $kReplaceWorkspace $BN_CLICKED (Wait-Control $main $kReplaceWorkspace)
    $previewReview=Read-ReplaceReview $process.Id 2 $status
    $reviewTargets=(@($previewReview.entries|Where-Object {$_.path -match 'alpha\.md' -and $_.before -match 'token' -and $_.after -match 'SHOULD_NOT_WRITE'}).Count -eq 1) -and
        (@($previewReview.entries|Where-Object {$_.path -match 'beta\.txt' -and $_.before -match 'token' -and $_.after -match 'SHOULD_NOT_WRITE'}).Count -eq 1)
    $previewConfirmation=Answer-OwnedConfirmation $process.Id 'Workspace置換' 7 -CaptureInventory
    $betaAfterPreview=[IO.File]::ReadAllText($beta)
    $gammaAfterPreview=[IO.File]::ReadAllText($gamma)
    $imageAfterPreview=Get-FileSha256 -Path $image
    $previewStatusAfterCancel=[MDLiteSearchNative]::Text($status)
    $previewReached=$previewReview.count -eq 2 -and $reviewTargets -and $previewConfirmation.button_id -eq 7 -and $previewConfirmation.closed
    $previewDiskUnchanged=$betaAfterPreview -ceq $betaBeforePreview -and $gammaAfterPreview -ceq $gammaBeforePreview
    $imageUnchanged=$imageAfterPreview -ceq $imageBeforePreview -and $imageBeforePreview -ceq $initialFiles.image
    [void](Send $main $kTestSetReplaceConfirmationUiMessage 0 0)
    Add-Check 'S6_workspace_replace_preview_requires_explicit_confirmation' ($previewReached -and $previewDiskUnchanged -and $imageUnchanged) ([ordered]@{
        preview_reached=$previewReached;review_target_count=$previewReview.count;review_targets=$previewReview.entries
        reviewed_expected_targets=$reviewTargets;status_during_review=$previewReview.status_during_review;status_after_cancel=$previewStatusAfterCancel
        disk_unchanged=$previewDiskUnchanged;beta=$betaAfterPreview;gamma=$gammaAfterPreview;image_unchanged=$imageUnchanged
        image_sha256_before=$imageBeforePreview;image_sha256_after=$imageAfterPreview;confirmation=$previewConfirmation
    })

    Set-Query $main $find 'recoverymark'
    $recoverySearch=Wait-Search $count $status 2
    [MDLiteSearchNative]::SetText($replace,'recoveredmark')
    [void](Send $main $kTestSetReplaceConfirmationUiMessage 1 0)
    $journalDirectory=Join-Path $workspace '.mdlite\.state\replace'
    $journalNamesBefore=@()
    if(Test-Path -LiteralPath $journalDirectory -PathType Container){$journalNamesBefore=@(Get-ChildItem -LiteralPath $journalDirectory -File -Filter '*.journal'|ForEach-Object Name)}
    Post-Command $main $kReplaceWorkspace $BN_CLICKED (Wait-Control $main $kReplaceWorkspace)
    $recoveryApplyReview=Read-ReplaceReview $process.Id 2 $status
    $recoveryApplyReviewPass=(@($recoveryApplyReview.entries|Where-Object {$_.path -match 'recovery-a\.md' -and $_.before -match 'recoverymark alpha' -and $_.after -match 'recoveredmark alpha'}).Count -eq 1) -and
        (@($recoveryApplyReview.entries|Where-Object {$_.path -match 'recovery-b\.md' -and $_.before -match 'recoverymark beta' -and $_.after -match 'recoveredmark beta'}).Count -eq 1)
    $recoveryApplyConfirmation=Answer-OwnedConfirmation $process.Id 'Workspace置換' 6 -CaptureInventory
    $recoveryApplyCompletion=Wait-WorkspaceApplySettled -Main $main -StatusControl $status -ProcessId $process.Id `
        -Paths @($recoveryA,$recoveryB) -ExpectedContents @("recoveredmark alpha`r`n","recoveredmark beta`r`n") `
        -JournalDirectory $journalDirectory -JournalNamesBefore $journalNamesBefore -Stage 'S6 closed file replacement workflow completion' -TimeoutMs 8000 -RequireNewJournal $true
    $recoveryApplySearchAfterApply=Wait-Search $count $status 0
    $recoveryAAfterApply=[string]$recoveryApplyCompletion.observed_contents[0]
    $recoveryBAfterApply=[string]$recoveryApplyCompletion.observed_contents[1]
    $recoveryApplied=$recoveryAAfterApply -ceq "recoveredmark alpha`r`n" -and $recoveryBAfterApply -ceq "recoveredmark beta`r`n"
    Add-Check 'S6_closed_workspace_replace_applies_reviewed_files' ($recoverySearch.count -eq 2 -and $recoveryApplyReviewPass -and $recoveryApplied -and $recoveryApplyCompletion.ready -and $recoveryApplySearchAfterApply.count -eq 0) ([ordered]@{
        search=$recoverySearch;review=$recoveryApplyReview.entries;reviewed_expected_diffs=$recoveryApplyReviewPass
        apply_confirmation=$recoveryApplyConfirmation;apply_completion=$recoveryApplyCompletion;search_after_apply=$recoveryApplySearchAfterApply;applied=$recoveryApplied
        file_a=$recoveryAAfterApply;file_b=$recoveryBAfterApply
    })
    if (-not $recoveryApplyCompletion.ready) { throw 'Closed-file Workspace apply did not reach the owned-window completion barrier; rollback was not started.' }

    $rollbackWindowsBefore=Capture-OwnedModalInventory $process.Id 'S6 closed-file rollback before command'
    Post-Command $main $kRollbackWorkspaceReplace $BN_CLICKED (Wait-Control $main $kRollbackWorkspaceReplace)
    $recoveryUndoReview=Read-ReplaceReview $process.Id 2 $status
    $rollbackWindowsAfter=Capture-OwnedModalInventory $process.Id 'S6 closed-file rollback review opened'
    $recoveryUndoReviewPass=(@($recoveryUndoReview.entries|Where-Object {$_.path -match 'recovery-a\.md' -and $_.before -match 'recoveredmark alpha' -and $_.after -match 'recoverymark alpha'}).Count -eq 1) -and
        (@($recoveryUndoReview.entries|Where-Object {$_.path -match 'recovery-b\.md' -and $_.before -match 'recoveredmark beta' -and $_.after -match 'recoverymark beta'}).Count -eq 1)
    $recoveryUndoConfirmation=Answer-OwnedConfirmation $process.Id '閉じたファイルの置換を戻す' 6 -CaptureInventory
    $recoveryUndoCompletion=Wait-WorkspaceApplySettled -Main $main -StatusControl $status -ProcessId $process.Id `
        -Paths @($recoveryA,$recoveryB) -ExpectedContents @($recoveryASource,$recoveryBSource) `
        -JournalDirectory $journalDirectory -JournalNamesBefore $journalNamesBefore -Stage 'S6 closed file rollback workflow completion' -TimeoutMs 8000 -RequireNewJournal $false
    $recoveryUndoSearchAfterRollback=Wait-Search $count $status 2
    $recoveryAAfterUndo=[string]$recoveryUndoCompletion.observed_contents[0]
    $recoveryBAfterUndo=[string]$recoveryUndoCompletion.observed_contents[1]
    $recoveryUndone=$recoveryAAfterUndo -ceq $recoveryASource -and $recoveryBAfterUndo -ceq $recoveryBSource -and
        (Get-FileSha256 -Path $recoveryA) -ceq $initialFiles.recovery_a -and
        (Get-FileSha256 -Path $recoveryB) -ceq $initialFiles.recovery_b
    Add-Check 'S6_closed_workspace_replace_rollback_restores_reviewed_files' ($recoveryUndoReviewPass -and $recoveryUndone -and $recoveryUndoCompletion.ready -and $recoveryUndoSearchAfterRollback.count -eq 2) ([ordered]@{
        review=$recoveryUndoReview.entries;reviewed_expected_restore=$recoveryUndoReviewPass;confirmation=$recoveryUndoConfirmation
        modal_windows_before=$rollbackWindowsBefore;modal_windows_after=$rollbackWindowsAfter;rollback_completion=$recoveryUndoCompletion
        search_after_rollback=$recoveryUndoSearchAfterRollback;restored=$recoveryUndone;file_a=$recoveryAAfterUndo;file_b=$recoveryBAfterUndo
    })

    Set-Query $main $find 'dirtyundo'
    [void](Wait-Search $count $status 1)
    [MDLiteSearchNative]::SetText($replace,'dirtydone')
    $dirtyJournalNamesBefore=@()
    if(Test-Path -LiteralPath $journalDirectory -PathType Container){$dirtyJournalNamesBefore=@(Get-ChildItem -LiteralPath $journalDirectory -File -Filter '*.journal'|ForEach-Object Name)}
    Post-Command $main $kReplaceWorkspace $BN_CLICKED (Wait-Control $main $kReplaceWorkspace)
    $dirtyApplyReview=Read-ReplaceReview $process.Id 1 $status
    $dirtyApplyReviewPass=$dirtyApplyReview.entries[0].path -match 'recovery-dirty\.md' -and
        $dirtyApplyReview.entries[0].before -match 'dirtyundo source' -and $dirtyApplyReview.entries[0].after -match 'dirtydone source'
    $dirtyApplyConfirmation=Answer-OwnedConfirmation $process.Id 'Workspace置換' 6 -CaptureInventory
    $dirtyApplyCompletion=Wait-WorkspaceApplySettled -Main $main -StatusControl $status -ProcessId $process.Id `
        -Paths @($dirtyRecoveryFile) -ExpectedContents @("dirtydone source`r`n") -JournalDirectory $journalDirectory `
        -JournalNamesBefore $dirtyJournalNamesBefore -Stage 'S6 dirty target workflow completion' -TimeoutMs 8000 -RequireNewJournal $true
    $dirtyApplySearchAfter=Wait-Search $count $status 0
    $dirtyRecoveryAfterApply=[string]$dirtyApplyCompletion.observed_contents[0]
    Add-Check 'S6_dirty_target_apply_workflow_completed' ($dirtyApplyCompletion.ready -and $dirtyApplySearchAfter.count -eq 0 -and $dirtyRecoveryAfterApply -ceq "dirtydone source`r`n") ([ordered]@{review=$dirtyApplyReview.entries;confirmation=$dirtyApplyConfirmation;completion=$dirtyApplyCompletion;search_after_apply=$dirtyApplySearchAfter;file=$dirtyRecoveryAfterApply})
    if (-not $dirtyApplyCompletion.ready) { throw 'Closed-file dirty-target apply did not reach the owned-window completion barrier.' }
    Set-Query $main $find 'dirtydone'
    [void](Wait-Search $count $status 1)
    $dirtyRecoveryResult=Get-FirstResult $resultTree 0 $process.Id
    Select-TreeItem $resultTree $dirtyRecoveryResult.child
    $dirtyActivationDeadline=[DateTime]::UtcNow.AddSeconds(5)
    $script:currentTestStage='S6 dirty recovery result activation wait'
    Write-RunBreadcrumb "WAIT_START stage=$script:currentTestStage timeout_ms=5000"
    do {
        Assert-RunBudget $script:currentTestStage
        $dirtyRecoveryActiveSource=Get-DocumentText $main
        if($dirtyRecoveryActiveSource -ceq "dirtydone source`r`n"){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $dirtyActivationDeadline)
    $dirtyRecoveryActivated=$dirtyRecoveryActiveSource -ceq "dirtydone source`r`n"
    Add-Check 'S6_dirty_rejection_opens_the_closed_target_document' ($dirtyRecoveryResult.child -ne 0 -and $dirtyRecoveryActivated) ([ordered]@{result=$dirtyRecoveryResult;expected_source="dirtydone source`r`n";observed_source=$dirtyRecoveryActiveSource;activated=$dirtyRecoveryActivated;activation_route='Select-TreeItem sends the source-backed result-tree VK_RETURN route'})
    if(-not $dirtyRecoveryActivated){throw 'The dirty recovery search result did not activate the target document; rollback refusal was not attempted.'}
    $dirtyRecoveryEditor=Wait-Control $main $kEditor
    [void](Send $dirtyRecoveryEditor 0x00B1 0 -1)
    Replace-Selection $dirtyRecoveryEditor 'unsaved recovery edit'
    Start-Sleep -Milliseconds 550
    $dirtyRecoverySourceBeforeReject=Get-DocumentText $main
    $dirtyBufferState=Send $main $kTestGetDocumentStateMessage 0 0
    $dirtyRollbackWindowsBefore=Capture-OwnedModalInventory $process.Id 'S6 dirty-open rollback before command'
    Post-Command $main $kRollbackWorkspaceReplace $BN_CLICKED (Wait-Control $main $kRollbackWorkspaceReplace)
    $dirtyRollbackDeadline=[DateTime]::UtcNow.AddSeconds(3)
    $dirtyRollbackStatus=''
    do {
        Assert-RunBudget 'S6 dirty-open rollback refusal status wait'
        $dirtyRollbackStatus=[MDLiteSearchNative]::Text($status)
        if($dirtyRollbackStatus -match '未保存|タブを閉じ' -or $dirtyRollbackStatus -match '戻すpreview'){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $dirtyRollbackDeadline)
    $dirtyRecoveryDiskAfterReject=[IO.File]::ReadAllText($dirtyRecoveryFile)
    $dirtyRollbackWindowsAfter=Capture-OwnedModalInventory $process.Id 'S6 dirty-open rollback after command'
    $dirtyUnexpectedReview=@($dirtyRollbackWindowsAfter.dialogs|Where-Object {$_.visible -and $_.class -eq 'MDLite.ReplaceReviewDialog'}).Count -gt 0
    $dirtyRejected=$dirtyBufferState -ne 0 -and $dirtyRollbackStatus -match '未保存|タブを閉じ' -and
        $dirtyRecoverySourceBeforeReject -ceq 'unsaved recovery edit' -and
        $dirtyRecoveryDiskAfterReject -ceq "dirtydone source`r`n" -and -not $dirtyUnexpectedReview
    Add-Check 'S6_closed_replace_rollback_rejects_dirty_open_buffer' ($dirtyApplyReviewPass -and $dirtyRecoveryActivated -and $dirtyRejected) ([ordered]@{
        review=$dirtyApplyReview.entries;buffer_dirty=$dirtyBufferState;source_before_reject=$dirtyRecoverySourceBeforeReject
        status=$dirtyRollbackStatus;disk=$dirtyRecoveryDiskAfterReject;unexpected_review=$dirtyUnexpectedReview
        modal_windows_before=$dirtyRollbackWindowsBefore;modal_windows_after=$dirtyRollbackWindowsAfter
    })
    if(-not $dirtyRejected){throw 'Dirty-open rollback refusal was not observed; stale-file workflow was skipped.'}
    [void](Send $dirtyRecoveryEditor 0x0304 0 0)
    $dirtyUndoDeadline=[DateTime]::UtcNow.AddSeconds(3)
    do {
        Assert-RunBudget 'S6 dirty rejection undo restoration wait'
        $dirtyRecoveryAfterUndo=Get-DocumentText $main
        $dirtyRecoveryDirtyAfterUndo=Send $main $kTestGetDocumentStateMessage 0 0
        if($dirtyRecoveryDirtyAfterUndo -eq 0 -and $dirtyRecoveryAfterUndo -ceq "dirtydone source`r`n"){break}
        Start-Sleep -Milliseconds 40
    } while([DateTime]::UtcNow -lt $dirtyUndoDeadline)
    $dirtyRecoveryTestClean=$dirtyRecoveryDirtyAfterUndo -eq 0 -and $dirtyRecoveryAfterUndo -ceq "dirtydone source`r`n"
    Add-Check 'S6_dirty_rejection_fixture_undo_restores_buffer' $dirtyRecoveryTestClean ([ordered]@{clean=$dirtyRecoveryTestClean;source=(Get-DocumentText $main)})

    Set-Query $main $find 'staleundo'
    [void](Wait-Search $count $status 1)
    [MDLiteSearchNative]::SetText($replace,'staledone')
    $staleJournalNamesBefore=@()
    if(Test-Path -LiteralPath $journalDirectory -PathType Container){$staleJournalNamesBefore=@(Get-ChildItem -LiteralPath $journalDirectory -File -Filter '*.journal'|ForEach-Object Name)}
    Post-Command $main $kReplaceWorkspace $BN_CLICKED (Wait-Control $main $kReplaceWorkspace)
    $staleApplyReview=Read-ReplaceReview $process.Id 1 $status
    $staleApplyReviewPass=$staleApplyReview.entries[0].path -match 'recovery-stale\.md' -and
        $staleApplyReview.entries[0].before -match 'staleundo source' -and $staleApplyReview.entries[0].after -match 'staledone source'
    $staleApplyConfirmation=Answer-OwnedConfirmation $process.Id 'Workspace置換' 6 -CaptureInventory
    $staleApplyCompletion=Wait-WorkspaceApplySettled -Main $main -StatusControl $status -ProcessId $process.Id `
        -Paths @($staleRecoveryFile) -ExpectedContents @("staledone source`r`n") -JournalDirectory $journalDirectory `
        -JournalNamesBefore $staleJournalNamesBefore -Stage 'S6 stale-target workflow completion' -TimeoutMs 8000 -RequireNewJournal $true
    $staleApplySearchAfter=Wait-Search $count $status 0
    $staleRecoveryAfterApply=[string]$staleApplyCompletion.observed_contents[0]
    Add-Check 'S6_stale_target_apply_workflow_completed' ($staleApplyCompletion.ready -and $staleApplySearchAfter.count -eq 0 -and $staleRecoveryAfterApply -ceq "staledone source`r`n") ([ordered]@{review=$staleApplyReview.entries;confirmation=$staleApplyConfirmation;completion=$staleApplyCompletion;search_after_apply=$staleApplySearchAfter;file=$staleRecoveryAfterApply})
    if (-not $staleApplyCompletion.ready) { throw 'Closed-file stale-target apply did not reach the owned-window completion barrier.' }
    $staleExternalSource="external stale edit`r`n"
    [IO.File]::WriteAllText($staleRecoveryFile,$staleExternalSource,[Text.UTF8Encoding]::new($false))
    Post-Command $main $kRollbackWorkspaceReplace $BN_CLICKED (Wait-Control $main $kRollbackWorkspaceReplace)
    Start-Sleep -Milliseconds 150
    $staleRollbackStatus=[MDLiteSearchNative]::Text($status)
    $staleRecoveryDiskAfterReject=[IO.File]::ReadAllText($staleRecoveryFile)
    $staleRejected=$staleRollbackStatus -match '変更|journal|外部' -and $staleRecoveryDiskAfterReject -ceq $staleExternalSource
    Add-Check 'S6_closed_replace_rollback_rejects_external_change' ($staleApplyReviewPass -and $staleRejected) ([ordered]@{review=$staleApplyReview.entries;status=$staleRollbackStatus;disk=$staleRecoveryDiskAfterReject})
    [void](Send $main $kTestSetReplaceConfirmationUiMessage 0 0)

    $replaceVisible=[MDLiteSearchNative]::IsWindowVisible((Wait-Control $main $kReplaceOne))
    Send-Command $main $kSearchClose $BN_CLICKED
    $closedExplorer=[MDLiteSearchNative]::IsWindowVisible($tree) -and -not [MDLiteSearchNative]::IsWindowVisible($find)
    Add-Check 'S1_close_command_returns_to_explorer' ($closedExplorer -and $replaceVisible) ([ordered]@{closed=$closedExplorer;replace_was_expanded=$replaceVisible})
    [void](Capture-Frame $main '05-explorer-restored')
    }

    $scriptStableHash=Get-SourceHash
    $statusName=if (@($checks|Where-Object status -eq 'FAIL').Count -ne 0) {'FAIL'} elseif ($Scenario -eq 'search-panel-s1-s3') {'PASS_SEARCH_PANEL_S1_S3'} else {'PASS_SYNTHETIC_NATIVE'}
} catch {
    if ($controlledFailure) {
        $statusName='FAIL'
    } else {
        $blocked=$_.Exception.Message
        $statusName='BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
    }
} finally {
    if ($process -and -not $process.HasExited) {
        $closeReply=[IntPtr]::Zero
        try {
            $closeSend=[MDLiteSearchNative]::SendMessageTimeout($main,0x0010,[IntPtr]::Zero,[IntPtr]::Zero,2,1000,[ref]$closeReply)
            $closeMessageEvidence=[ordered]@{send_message_completed=($closeSend -ne [IntPtr]::Zero);reply=$closeReply.ToInt64();last_error=if($closeSend -eq [IntPtr]::Zero){[Runtime.InteropServices.Marshal]::GetLastWin32Error()}else{0}}
        } catch {
            $closeMessageEvidence=[ordered]@{send_message_completed=$false;reply=$closeReply.ToInt64();error=$_.Exception.Message}
        }
        $process.Refresh()
        if (-not $process.HasExited) {
            $ownedTopWindows=@()
            try { $ownedTopWindows=Get-OwnedTopWindowInventory $process.Id }
            catch { $ownedTopWindows=@([ordered]@{inventory_error=$_.Exception.Message}) }
            $cleanupEvidence=[ordered]@{
                stage='WM_CLOSE_TIMEOUT_BEFORE_PROCESS_KILL'
                fixture_process_id=$process.Id;main_hwnd=$main.ToInt64()
                main_window_visible=if($main -ne [IntPtr]::Zero){[MDLiteSearchNative]::IsWindowVisible($main)}else{$false}
                close_message=$closeMessageEvidence;owned_top_windows=$ownedTopWindows
            }
            if ($script:runBudgetExceeded) {
                $cleanupEvidence['document_read_error']='Skipped after harness safety bound.'
            } elseif ($main -ne [IntPtr]::Zero -and [MDLiteSearchNative]::IsWindowVisible($main)) {
                try {
                    $dirtyStateRaw=Send $main $kTestGetDocumentStateMessage 0 0
                    $sourceAtCloseTimeout=Get-DocumentText $main
                    $cleanupEvidence['document_dirty_state_raw']=$dirtyStateRaw
                    $cleanupEvidence['document_dirty']=($dirtyStateRaw -ne 0)
                    $cleanupEvidence['document_source_text']=$sourceAtCloseTimeout
                    $cleanupEvidence['document_source_sha256']=Get-TextSha256 $sourceAtCloseTimeout
                } catch { $cleanupEvidence['document_read_error']=$_.Exception.Message }
            } else {
                $cleanupEvidence['document_read_error']='Main window was unavailable at close timeout.'
            }
            try {
                $process.Refresh()
                $cleanupEvidence['process_exited_before_kill']=$process.HasExited
                if ($process.HasExited) {
                    $cleanup='PROCESS_EXITED_AFTER_CLOSE_OBSERVATION'
                } else {
                    $process.Kill()
                    [void]$process.WaitForExit(3000)
                    $process.Refresh()
                    $cleanup=if($process.HasExited){'FIXTURE_PROCESS_TERMINATED_AFTER_CLOSE_TIMEOUT'}else{'PROCESS_KILL_WAIT_TIMEOUT'}
                }
            } catch {
                try { $process.Refresh() } catch {}
                if ($process.HasExited) { $cleanup='PROCESS_EXITED_DURING_CLEANUP' }
                else { $cleanup='PROCESS_CLEANUP_BLOCKED: '+$_.Exception.Message }
            }
        } else { $cleanup='WM_CLOSE_COMPLETED' }
        $process.Dispose()
    } else { $cleanup='PROCESS_EXITED' }
    if ($null -eq $previousSilent) { Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_SILENT=$previousSilent }
    if ($null -eq $previousCrash) { Remove-Item Env:MDLITE_TEST_NO_CRASH_UI -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_NO_CRASH_UI=$previousCrash }
}

Write-RunBreadcrumb 'STAGE final source hash and evidence assembly'
if (-not $scriptStableHash) { $scriptStableHash=Get-SourceHash }
$finalSourceHash=Get-SourceHash
$sourceStable=$sourceHash -eq $finalSourceHash
$failed=@($checks|Where-Object status -eq 'FAIL'|ForEach-Object name)
$result=[ordered]@{
    schema='mdlite-search-quality-v1'
    status=if ($blocked) { $statusName } elseif ($failed.Count -gt 0) { 'FAIL' } elseif ($sourceStable) { $statusName } else { 'FAIL_SOURCE_CHANGED_DURING_RUN' }
    preset=$Preset; scenario=$Scenario; commit=(& git -C $repoRoot rev-parse HEAD)
    build_receipt=$receiptPath; executable_path=$executable; executable_sha256=$exeHash
    source_sha256=$sourceHash; source_sha256_after=$finalSourceHash; source_stable=$sourceStable
    workspace=$workspace; screenshot_directory=$screenshots; progress_path=$script:runProgressPath; screenshots=@($frames)
    checks=@($checks); failed=$failed; blocked_reason=$blocked; failure_reason=$controlledFailure; cleanup=$cleanup; cleanup_evidence=$cleanupEvidence
    failure_stage=$currentTestStage;text_read_diagnostic=$lastTextReadDiagnostic
    fixture_process_id=if($process){$process.Id}else{$null}; key_input_evidence=@($keyInputEvidence)
    missing_control_id=$missingControlId; owned_child_windows_on_missing=@($ownedChildWindowsOnMissing)
    owned_modal_evidence=@($script:ownedModalEvidence.ToArray());owned_window_wait_failure=$script:lastOwnedWindowWaitEvidence
    input_method='Synthetic native Edit/TreeView messages and SendInput keyboard shortcuts; if focus acquisition fails, at most one guarded titlebar click on the owned fixture with pointer restoration; not physical input.'
    human_lane='NOT_RUN: real IME confirmation/Escape, physical keyboard/mouse, and subjective visual inspection remain human-only.'
    timestamp_utc=[DateTime]::UtcNow.ToString('o')
}
[IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
try { $jsonSnapshot=ConvertTo-JsonPlainSnapshot $result 10000 64 8388608 }
catch {
    Write-RunBreadcrumb "JSON_SNAPSHOT_FAILED stage=$script:currentTestStage message=$($_.Exception.Message)"
    throw
}
Assert-RunBudget 'before JSON serialization'
Write-RunBreadcrumb "JSON_BEFORE stage=$script:currentTestStage expanded_nodes=$($jsonSnapshot.expanded_nodes) scalar_nodes=$($jsonSnapshot.scalar_nodes) dictionary_nodes=$($jsonSnapshot.dictionary_nodes) array_nodes=$($jsonSnapshot.array_nodes) note_snapshot_nodes=$($jsonSnapshot.note_snapshot_nodes) extended_scalar_nodes=$($jsonSnapshot.extended_scalar_nodes) string_chars=$($jsonSnapshot.string_chars) max_nodes=$($jsonSnapshot.max_nodes) max_depth=$($jsonSnapshot.max_depth) max_string_chars=$($jsonSnapshot.max_string_chars)"
try { $json=ConvertTo-Json -InputObject $jsonSnapshot.snapshot -Depth 12 }
catch {
    Write-RunBreadcrumb "JSON_SERIALIZATION_FAILED stage=$script:currentTestStage message=$($_.Exception.Message)"
    throw
}
Write-RunBreadcrumb "JSON_AFTER stage=$script:currentTestStage expanded_nodes=$($jsonSnapshot.expanded_nodes) scalar_nodes=$($jsonSnapshot.scalar_nodes) serialized_chars=$($json.Length)"
Assert-RunBudget 'after JSON serialization'
$json | Set-Content -LiteralPath $OutputPath -Encoding UTF8
Write-RunBreadcrumb "RESULT_WRITTEN path=$OutputPath status=$($result.status)"
Write-Output "Evidence: $OutputPath; status=$($result.status); PASS=$(@($checks|Where-Object status -eq 'PASS').Count); FAIL=$($failed.Count); screenshots=$($frames.Count); source_stable=$sourceStable"
if ($result.status -notlike 'PASS*') { exit 1 }
