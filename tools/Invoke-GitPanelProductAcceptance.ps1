#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Preset = 'debug',
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
$verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'build\verification'))
$evidenceRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot 'prehuman-calendar-git'))
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$runRoot = Join-Path $evidenceRoot "git-product-$runId"
$noGitWorkspace = Join-Path $runRoot 'no-git-workspace'
$gitWorkspace = Join-Path $runRoot 'git-workspace'
$emptyPath = Join-Path $runRoot 'empty-path'
$ownerMarker = Join-Path $runRoot '.git-product-owner'
$buildReceiptPath = Join-Path $verificationRoot "build-receipt-$Preset.json"
if (-not $OutputPath) { $OutputPath = Join-Path $evidenceRoot "git-product-acceptance-$runId.json" }
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }

$gitCommand = Get-Command git.exe -ErrorAction SilentlyContinue
$gitExe = if ($gitCommand) { $gitCommand.Source } else { 'C:\Program Files\Git\cmd\git.exe' }
if (-not (Test-Path -LiteralPath $gitExe -PathType Leaf)) { throw "git.exe was not found: $gitExe" }

$checks = [ordered]@{}
$errorMessage = $null
$trustUiEvidence = [ordered]@{ status = 'NOT_RUN'; stage = 'not_started'; details = [ordered]@{} }
$trustDialogFrames = [Collections.Generic.List[object]]::new()
$process = $null
$main = [IntPtr]::Zero
$noGitProcess = $null
$noGitMain = [IntPtr]::Zero
$stall = $null
$evidenceBoundary = 'Synthetic Win32 messages and OS SendInput. No physical keyboard/mouse, IME, or Human visual acceptance.'
$noGitPanelSnapshots = [Collections.Generic.List[object]]::new()
$gitPanelSnapshots = [Collections.Generic.List[object]]::new()
$paletteRuns = [Collections.Generic.List[object]]::new()
$noGitProcessId = 0
$noGitMainHandle = 0L
$noGitMainOwnerPid = 0
$gitProcessId = 0
$gitMainHandle = 0L
$gitMainOwnerPid = 0
$paletteScreenshot = $null
$headerScreenshot = $null
$mixedPanelScreenshot = $null
$gitRowsBefore = @()
$gitRowsAfterStage = @()
$gitRowsAfterUnstage = @()
$documentStates = [ordered]@{}
$recoveryEvidence = $null
$buildReceipt = $null
$sourceFingerprintBefore = $null
$sourceFingerprintAfter = $null
$executableHashBefore = ''
$executableHashAfter = ''
$status = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$previousGitDelaySetting = $env:MDLITE_TEST_GIT_ACTION_DELAY_MS
$previousTerminalPromptSetting = $env:GIT_TERMINAL_PROMPT
$env:MDLITE_TEST_SILENT = '1'
$env:MDLITE_TEST_GIT_ACTION_DELAY_MS = '7500'
$env:GIT_TERMINAL_PROMPT = '0'

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Net;
using System.Net.Sockets;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;

public static class MDLiteGitProductNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    public delegate bool EnumChildWindowsProc(IntPtr window, IntPtr parameter);

    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)] private struct LVITEMW {
        public uint mask;
        public int iItem;
        public int iSubItem;
        public uint state;
        public uint stateMask;
        public IntPtr text;
        public int textCapacity;
        public int image;
        public IntPtr parameter;
    }
    [StructLayout(LayoutKind.Sequential)] private struct GUITHREADINFO {
        public uint cbSize;
        public uint flags;
        public IntPtr active;
        public IntPtr focus;
        public IntPtr capture;
        public IntPtr menuOwner;
        public IntPtr moveSize;
        public IntPtr caret;
        public RECT caretRect;
    }
    [StructLayout(LayoutKind.Sequential)] private struct KEYBDINPUT {
        public ushort virtualKey;
        public ushort scanCode;
        public uint flags;
        public uint time;
        public IntPtr extraInfo;
    }
    [StructLayout(LayoutKind.Sequential)] private struct MOUSEINPUT {
        public int dx;
        public int dy;
        public uint mouseData;
        public uint flags;
        public uint time;
        public IntPtr extraInfo;
    }
    [StructLayout(LayoutKind.Explicit)] private struct INPUTUNION {
        [FieldOffset(0)] public KEYBDINPUT keyboard;
        [FieldOffset(0)] public MOUSEINPUT mouse;
    }
    [StructLayout(LayoutKind.Sequential)] private struct INPUT {
        public uint type;
        public INPUTUNION data;
    }

    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] private static extern int GetClassName(IntPtr window, StringBuilder value, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "GetWindowTextW")] private static extern int GetWindowTextNative(IntPtr window, StringBuilder value, int capacity);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)] private static extern IntPtr SendMessageGetText(IntPtr window, uint message, IntPtr wparam, StringBuilder value);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageSetText(IntPtr window, uint message, IntPtr wparam, string text);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", EntryPoint = "GetWindowLongW")] private static extern int GetWindowLong(IntPtr window, int index);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", EntryPoint = "SendMessageW")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr SetFocus(IntPtr window);
    [DllImport("kernel32.dll")] private static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] private static extern bool AttachThreadInput(uint currentThread, uint targetThread, bool attach);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint threadId, ref GUITHREADINFO info);
    [DllImport("user32.dll")] private static extern uint SendInput(uint count, INPUT[] inputs, int size);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr window, IntPtr dc, uint flags);

    [DllImport("kernel32.dll")] private static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr VirtualAllocEx(IntPtr process, IntPtr address, UIntPtr size, uint allocationType, uint protection);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool VirtualFreeEx(IntPtr process, IntPtr address, UIntPtr size, uint freeType);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool WriteProcessMemory(IntPtr process, IntPtr address, IntPtr buffer, UIntPtr size, out UIntPtr written);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool ReadProcessMemory(IntPtr process, IntPtr address, IntPtr buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);

    private static string TextOf(IntPtr window) {
        int length = SendMessage(window, 0x000E, IntPtr.Zero, IntPtr.Zero).ToInt32();
        if (length <= 0) return String.Empty;
        StringBuilder text = new StringBuilder(Math.Min(8192, length + 1));
        SendMessageGetText(window, 0x000D, new IntPtr(text.Capacity), text);
        return text.ToString();
    }
    private static string ClassOf(IntPtr window) {
        StringBuilder value = new StringBuilder(256);
        GetClassName(window, value, value.Capacity);
        return value.ToString();
    }
    private static string TitleOf(IntPtr window) {
        StringBuilder value = new StringBuilder(2048);
        GetWindowTextNative(window, value, value.Capacity);
        return value.ToString();
    }
    public static string WindowText(IntPtr window) { return TextOf(window); }
    public static string WindowClass(IntPtr window) { return ClassOf(window); }
    public static int ControlId(IntPtr window) { return GetDlgCtrlID(window); }
    public static int WindowStyle(IntPtr window) { return GetWindowLong(window, -16); }

    public static IntPtr FindMainWindow(int processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr unused) {
            uint owner = 0;
            GetWindowThreadProcessId(window, out owner);
            if (owner == (uint)processId && ClassOf(window) == "MDLite.MainWindow") {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr FindTopLevelWindow(int processId, string title) {
        IntPtr found = IntPtr.Zero;
        EnumWindows(delegate(IntPtr window, IntPtr unused) {
            uint owner = 0;
            GetWindowThreadProcessId(window, out owner);
            if (owner == (uint)processId && TitleOf(window) == title) {
                found = window;
                return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr[] Children(IntPtr parent) {
        List<IntPtr> items = new List<IntPtr>();
        EnumChildWindows(parent, delegate(IntPtr window, IntPtr unused) {
            items.Add(window);
            return true;
        }, IntPtr.Zero);
        return items.ToArray();
    }
    public static IntPtr FindChild(IntPtr parent, int id, string className) {
        foreach (IntPtr child in Children(parent)) {
            if (id >= 0 && GetDlgCtrlID(child) != id) continue;
            if (!String.IsNullOrEmpty(className) && ClassOf(child) != className) continue;
            return child;
        }
        return IntPtr.Zero;
    }
    public static IntPtr FindChildByText(IntPtr parent, string className, string text) {
        foreach (IntPtr child in Children(parent)) {
            if (!String.IsNullOrEmpty(className) && ClassOf(child) != className) continue;
            if (TextOf(child) == text) return child;
        }
        return IntPtr.Zero;
    }
    public static IntPtr FocusedWindow(IntPtr main) {
        uint pid = 0;
        uint thread = GetWindowThreadProcessId(main, out pid);
        GUITHREADINFO info = new GUITHREADINFO();
        info.cbSize = (uint)Marshal.SizeOf(typeof(GUITHREADINFO));
        return GetGUIThreadInfo(thread, ref info) ? info.focus : IntPtr.Zero;
    }
    public static bool FocusControl(IntPtr main, IntPtr control) {
        uint pid = 0;
        uint targetThread = GetWindowThreadProcessId(main, out pid);
        uint currentThread = GetCurrentThreadId();
        bool attached = AttachThreadInput(currentThread, targetThread, true);
        try {
            SetForegroundWindow(main);
            SetFocus(control);
        } finally {
            if (attached) AttachThreadInput(currentThread, targetThread, false);
        }
        return FocusedWindow(main) == control;
    }
    public static bool SendKeyChord(ushort[] keys) {
        if (keys == null || keys.Length == 0) return false;
        INPUT[] inputs = new INPUT[keys.Length * 2];
        for (int index = 0; index < keys.Length; ++index) {
            inputs[index].type = 1;
            inputs[index].data.keyboard.virtualKey = keys[index];
            inputs[index].data.keyboard.flags = 0;
        }
        for (int index = 0; index < keys.Length; ++index) {
            int target = keys.Length + index;
            inputs[target].type = 1;
            inputs[target].data.keyboard.virtualKey = keys[keys.Length - 1 - index];
            inputs[target].data.keyboard.flags = 2;
        }
        return SendInput((uint)inputs.Length, inputs, Marshal.SizeOf(typeof(INPUT))) == (uint)inputs.Length;
    }
    public static int ListViewCount(IntPtr list) {
        return SendMessage(list, 0x1004, IntPtr.Zero, IntPtr.Zero).ToInt32();
    }
    private static IntPtr AllocateRemote(IntPtr process, int bytes) {
        return VirtualAllocEx(process, IntPtr.Zero, new UIntPtr((uint)bytes), 0x1000, 0x04);
    }
    public static string ListViewText(IntPtr list, int processId, int row, int column) {
        const uint access = 0x0008 | 0x0020 | 0x0010;
        const int capacity = 2048;
        IntPtr process = OpenProcess(access, false, processId);
        if (process == IntPtr.Zero) throw new InvalidOperationException("OpenProcess failed while reading the Git panel.");
        IntPtr remoteText = IntPtr.Zero, remoteItem = IntPtr.Zero, localText = IntPtr.Zero, localItem = IntPtr.Zero;
        try {
            int textBytes = capacity * 2;
            int itemBytes = Marshal.SizeOf(typeof(LVITEMW));
            remoteText = AllocateRemote(process, textBytes);
            remoteItem = AllocateRemote(process, itemBytes);
            localText = Marshal.AllocHGlobal(textBytes);
            localItem = Marshal.AllocHGlobal(itemBytes);
            if (remoteText == IntPtr.Zero || remoteItem == IntPtr.Zero) throw new InvalidOperationException("VirtualAllocEx failed while reading the Git panel.");
            LVITEMW item = new LVITEMW();
            item.mask = 1;
            item.iItem = row;
            item.iSubItem = column;
            item.text = remoteText;
            item.textCapacity = capacity;
            Marshal.StructureToPtr(item, localItem, false);
            UIntPtr written = UIntPtr.Zero;
            if (!WriteProcessMemory(process, remoteItem, localItem, new UIntPtr((uint)itemBytes), out written) || written.ToUInt64() != (ulong)itemBytes)
                throw new InvalidOperationException("WriteProcessMemory failed while reading the Git panel.");
            SendMessage(list, 0x1073, new IntPtr(row), remoteItem); // LVM_GETITEMTEXTW
            UIntPtr read = UIntPtr.Zero;
            if (!ReadProcessMemory(process, remoteText, localText, new UIntPtr((uint)textBytes), out read) || read.ToUInt64() < 2)
                throw new InvalidOperationException("ReadProcessMemory failed while reading Git panel text.");
            return Marshal.PtrToStringUni(localText) ?? String.Empty;
        } finally {
            if (localText != IntPtr.Zero) Marshal.FreeHGlobal(localText);
            if (localItem != IntPtr.Zero) Marshal.FreeHGlobal(localItem);
            if (remoteText != IntPtr.Zero) VirtualFreeEx(process, remoteText, UIntPtr.Zero, 0x8000);
            if (remoteItem != IntPtr.Zero) VirtualFreeEx(process, remoteItem, UIntPtr.Zero, 0x8000);
            CloseHandle(process);
        }
    }
    public static bool SelectListViewRow(IntPtr list, int processId, int row) {
        const uint access = 0x0008 | 0x0020 | 0x0010;
        IntPtr process = OpenProcess(access, false, processId);
        if (process == IntPtr.Zero) return false;
        IntPtr remoteItem = IntPtr.Zero, localItem = IntPtr.Zero;
        try {
            int itemBytes = Marshal.SizeOf(typeof(LVITEMW));
            remoteItem = AllocateRemote(process, itemBytes);
            localItem = Marshal.AllocHGlobal(itemBytes);
            if (remoteItem == IntPtr.Zero) return false;
            LVITEMW item = new LVITEMW();
            item.mask = 8;
            item.iItem = row;
            item.state = 3;
            item.stateMask = 3;
            Marshal.StructureToPtr(item, localItem, false);
            UIntPtr written = UIntPtr.Zero;
            if (!WriteProcessMemory(process, remoteItem, localItem, new UIntPtr((uint)itemBytes), out written) || written.ToUInt64() != (ulong)itemBytes)
                return false;
            return SendMessage(list, 0x102B, new IntPtr(row), remoteItem) != IntPtr.Zero; // LVM_SETITEMSTATE
        } finally {
            if (localItem != IntPtr.Zero) Marshal.FreeHGlobal(localItem);
            if (remoteItem != IntPtr.Zero) VirtualFreeEx(process, remoteItem, UIntPtr.Zero, 0x8000);
            CloseHandle(process);
        }
    }
}

public sealed class MDLiteLocalStallServer : IDisposable {
    private readonly TcpListener listener;
    private readonly ManualResetEvent stopping = new ManualResetEvent(false);
    private readonly ManualResetEvent accepted = new ManualResetEvent(false);
    private Thread thread;
    public int Port { get; private set; }
    public bool WaitForConnection(int milliseconds) { return accepted.WaitOne(milliseconds); }
    public MDLiteLocalStallServer() { listener = new TcpListener(IPAddress.Loopback, 0); }
    public void Start() {
        listener.Start();
        Port = ((IPEndPoint)listener.LocalEndpoint).Port;
        thread = new Thread(Run);
        thread.IsBackground = true;
        thread.Start();
    }
    private void Run() {
        TcpClient client = null;
        try {
            client = listener.AcceptTcpClient();
            accepted.Set();
            stopping.WaitOne(120000);
        } catch { }
        finally { if (client != null) client.Close(); }
    }
    public void Dispose() {
        stopping.Set();
        listener.Stop();
        if (thread != null) thread.Join(1500);
        stopping.Dispose();
        accepted.Dispose();
    }
}
'@

function Add-Check([string]$Name, [bool]$Pass, $Details = $null) {
    $checks[$Name] = [ordered]@{ status = if ($Pass) { 'PASS' } else { 'FAIL' }; details = $Details }
}

function Get-SourceFingerprint {
    $rootPrefix = [IO.Path]::GetFullPath($repoRoot).TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
    $paths = @(
        Get-ChildItem -LiteralPath (Join-Path $repoRoot 'src') -File -Recurse -Force | Select-Object -ExpandProperty FullName
        Join-Path $repoRoot 'CMakeLists.txt'
        Join-Path $repoRoot 'CMakePresets.json'
    )
    [string[]]$paths = $paths
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $utf8 = [Text.UTF8Encoding]::new($false)
    $manifest = foreach ($path in $paths) {
        $relativePath = [IO.Path]::GetFullPath($path).Substring($rootPrefix.Length).Replace('\', '/')
        '{0}:{1}' -f $relativePath, ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant())
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { $hash = [BitConverter]::ToString($hasher.ComputeHash($utf8.GetBytes([string]::Join("`n", [string[]]$manifest)))).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
    return [pscustomobject]@{ sha256 = $hash; file_count = $paths.Count }
}

function Get-GitOutput([string]$Root, [string[]]$Arguments) {
    $previousPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        $lines = @(& $gitExe -C $Root @Arguments 2>&1 | ForEach-Object { $_.ToString() })
        $exitCode = [int]$LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    [pscustomobject]@{ exit_code = $exitCode; output = ($lines -join "`n").Trim() }
}

function Find-MainWindow([Diagnostics.Process]$Target, [int]$TimeoutMs = 15000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Target.Refresh()
        if ($Target.HasExited) { throw "MDLite exited with code $($Target.ExitCode) before its main window appeared." }
        $window = [MDLiteGitProductNative]::FindMainWindow($Target.Id)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main window was not created for process $($Target.Id)."
}

function Find-Child([IntPtr]$Parent, [int]$Id = -1, [string]$ClassName = '') {
    return [MDLiteGitProductNative]::FindChild($Parent, $Id, $ClassName)
}

function Find-ChildByText([IntPtr]$Parent, [string]$ClassName, [string]$Text) {
    return [MDLiteGitProductNative]::FindChildByText($Parent, $ClassName, $Text)
}

function Wait-WindowTitle([int]$ProcessId, [string]$Title, [int]$TimeoutMs = 10000,
    [int[]]$RequiredButtonIds = @(), [string]$RequiredBody = '', [switch]$TrustAcknowledgement) {
    if ($TrustAcknowledgement -and ($Title -cne 'Workspace Trust' -or -not $RequiredBody -or $RequiredButtonIds.Count -gt 0)) {
        throw 'Trust acknowledgement requires an exact receipt body and cannot use Yes/No button requirements.'
    }
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $lastWindow = [IntPtr]::Zero
    $lastEvidence = $null
    do {
        $window = [MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, $Title)
        $lastWindow = $window
        if ($window -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($window)) {
            $ready = $true
            if ($Title -ceq 'MDLite コマンドパレット') {
                $ready = [MDLiteGitProductNative]::WindowClass($window) -ceq 'MDLite.NativePickerWindow'
                $filter = Find-Child $window 100 'Edit'
                $list = Find-Child $window 101 'ListBox'
                foreach ($control in @($filter, $list)) {
                    if ($control -eq [IntPtr]::Zero -or -not [MDLiteGitProductNative]::IsWindowVisible($control) -or
                        -not [MDLiteGitProductNative]::IsWindowEnabled($control)) { $ready = $false }
                }
                if ($ready) { $ready = [MDLiteGitProductNative]::SendMessage($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -gt 0 }
            }
            if ($RequiredButtonIds.Count -gt 0 -or $RequiredBody) {
                $lastEvidence = Get-DialogControlsEvidence $window
                if ($TrustAcknowledgement) {
                    if ($null -eq (Select-TrustAcknowledgement $lastEvidence $RequiredBody)) { $ready = $false }
                } elseif ($lastEvidence.dialog_class -cne '#32770' -or -not $lastEvidence.static_body_wm_gettext.Contains($RequiredBody)) { $ready = $false }
                foreach ($id in $RequiredButtonIds) {
                    $buttons = @($lastEvidence.buttons | Where-Object { $_.control_id -eq $id -and $_.visible -and $_.enabled })
                    if ($buttons.Count -ne 1) { $ready = $false }
                }
            }
            if ($ready) { return $window }
        }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native dialog readiness timed out: title='$Title', owner_pid=$ProcessId, hwnd=$($lastWindow.ToInt64()), configured_timeout_ms=$TimeoutMs, elapsed_ms=$($watch.ElapsedMilliseconds), evidence=$($lastEvidence | ConvertTo-Json -Depth 5 -Compress)"
}

function Select-TrustAcknowledgement([System.Collections.IDictionary]$Evidence, [string]$ExpectedBody) {
    if ($Evidence.dialog_class -cne '#32770' -or $Evidence.dialog_caption -cne 'Workspace Trust' -or
        -not $Evidence.dialog_visible -or $Evidence.static_body_wm_gettext -cne $ExpectedBody) { return $null }
    $buttons = @($Evidence.buttons)
    if ($buttons.Count -ne 1 -or $buttons[0].class_name -cne 'Button' -or
        $buttons[0].wm_gettext -cne 'OK' -or $buttons[0].control_id -notin @(1, 2) -or
        -not $buttons[0].visible -or -not $buttons[0].enabled) { return $null }
    return $buttons[0]
}

function Get-DialogChildText([IntPtr]$Window) {
    $values = foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
        $text = [MDLiteGitProductNative]::WindowText($child)
        if ($text) { $text }
    }
    return (@($values) -join "`n")
}

function Get-DialogControlsEvidence([IntPtr]$Window) {
    $controls = foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
        $className = [MDLiteGitProductNative]::WindowClass($child)
        $text = [MDLiteGitProductNative]::WindowText($child) # WM_GETTEXT / WM_GETTEXTLENGTH
        [ordered]@{
            hwnd = $child.ToInt64(); class_name = $className; control_id = [MDLiteGitProductNative]::ControlId($child)
            wm_gettext = $text; visible = [MDLiteGitProductNative]::IsWindowVisible($child)
            enabled = [MDLiteGitProductNative]::IsWindowEnabled($child)
        }
    }
    $items = @($controls)
    $staticText = @($items | Where-Object { $_.class_name -eq 'Static' -and $_.visible -and $_.wm_gettext })
    $buttons = @($items | Where-Object { $_.class_name -eq 'Button' })
    return [ordered]@{
        dialog_hwnd = $Window.ToInt64(); dialog_class = [MDLiteGitProductNative]::WindowClass($Window)
        dialog_caption = [MDLiteGitProductNative]::WindowText($Window)
        dialog_visible = [MDLiteGitProductNative]::IsWindowVisible($Window)
        static_body_wm_gettext = @($staticText | ForEach-Object { $_.wm_gettext }) -join "`n"
        buttons = $buttons; controls = $items
    }
}

function Wait-WindowGone([int]$ProcessId, [string]$Title, [int]$TimeoutMs = 5000, [IntPtr]$WindowHandle = [IntPtr]::Zero) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        if ($WindowHandle -ne [IntPtr]::Zero) {
            if (-not [MDLiteGitProductNative]::IsWindow($WindowHandle)) { return $true }
        } elseif ([MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, $Title) -eq [IntPtr]::Zero) { return $true }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Wait-GitSummary([IntPtr]$Window, [string]$Needle, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $text = Get-GitSummary $Window
        if ($text -and $text.Contains($Needle)) { return $text }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    return ''
}

function Get-GitSummary([IntPtr]$Window) {
    foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
        # git_panel_ currently has ID 0: require its readonly/multiline/tabstop styles and exact state line.
        # The diff EDIT has no WS_TABSTOP, and Calendar details starts with its selected date.
        if ([MDLiteGitProductNative]::WindowClass($child) -ne 'Edit' -or
            [MDLiteGitProductNative]::ControlId($child) -ne 0 -or
            ([MDLiteGitProductNative]::WindowStyle($child) -band 0x10804) -ne 0x10804) { continue }
        if (-not [MDLiteGitProductNative]::IsWindowVisible($child)) { continue }
        $text = [MDLiteGitProductNative]::WindowText($child)
        $stateLine = ($text -split '[\r\n]', 2)[0]
        if (@('Workspace未選択', 'Workspaceは未信頼です', 'Git未導入', 'repositoryなし',
              '準備完了', 'remoteなし', '操作中', 'エラー', '不明') -ccontains $stateLine) { return $text }
    }
    return ''
}

function Get-GitList([IntPtr]$Window) {
    foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
        if ([MDLiteGitProductNative]::WindowClass($child) -eq 'SysListView32' -and
            [MDLiteGitProductNative]::IsWindowVisible($child)) { return $child }
    }
    return [IntPtr]::Zero
}

function Get-GitRows([IntPtr]$Window, [int]$ProcessId) {
    $list = Get-GitList $Window
    if ($list -eq [IntPtr]::Zero) { return ,@() }
    $count = [MDLiteGitProductNative]::ListViewCount($list)
    $rows = [Collections.Generic.List[object]]::new()
    for ($index = 0; $index -lt $count; $index++) {
        $rows.Add([pscustomobject]@{
            status = [MDLiteGitProductNative]::ListViewText($list, $ProcessId, $index, 0)
            path = [MDLiteGitProductNative]::ListViewText($list, $ProcessId, $index, 1)
        })
    }
    return ,@($rows.ToArray())
}

function Wait-GitActionIdle([IntPtr]$Window, [int]$TimeoutMs = 15000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        if ([MDLiteGitProductNative]::SendMessage($Window, 0x8044, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 0) { return $true }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Get-DocumentState([IntPtr]$Window) {
    $values = @()
    for ($key = 0; $key -le 4; $key++) {
        $values += [MDLiteGitProductNative]::SendMessage($Window, 0x8045, [IntPtr]$key, [IntPtr]::Zero).ToInt64()
    }
    $raw = [MDLiteGitProductNative]::SendMessage($Window, 0x8045, [IntPtr]5, [IntPtr]([Math]::Max(0, $values[4] - 1))).ToInt64()
    $savedRevision = [MDLiteGitProductNative]::SendMessage($Window, 0x8045, [IntPtr]6, [IntPtr]::Zero).ToInt64()
    return [pscustomobject]@{
        dirty = [bool]$values[0]; undo_count = $values[1]; redo_count = $values[2]
        revision = $values[3]; source_length = $values[4]; final_code_unit = $raw; saved_revision = $savedRevision
    }
}

function Get-SourceSelection([IntPtr]$Window) {
    # WM_APP+51/+52 expose full source anchor/active offsets; EM_GETSEL only packs 16-bit native offsets.
    $anchor = [MDLiteGitProductNative]::SendMessage($Window, 0x8033, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    $active = [MDLiteGitProductNative]::SendMessage($Window, 0x8034, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64()
    return [pscustomobject]@{ anchor = $anchor; active = $active }
}

function Get-DocumentSource([IntPtr]$Window) {
    $length = [MDLiteGitProductNative]::SendMessage($Window, 0x8045, [IntPtr]4, [IntPtr]::Zero).ToInt32()
    if ($length -lt 0) { throw 'Main fixture source inspection hook is unavailable.' }
    $text = [Text.StringBuilder]::new($length)
    for ($index = 0; $index -lt $length; $index++) {
        [void]$text.Append([char][MDLiteGitProductNative]::SendMessage($Window, 0x8045, [IntPtr]5, [IntPtr]$index).ToInt32())
    }
    return $text.ToString()
}

function Invoke-ProductCommandFromPalette([IntPtr]$Window, [Diagnostics.Process]$Target, [string]$Query) {
    $search = Find-Child $Window 122 'Button'
    if ($search -eq [IntPtr]::Zero) { throw 'The Command Search product control was not found.' }
    if (-not [MDLiteGitProductNative]::PostMessage($search, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not post the Command Search product button.'
    }
    $picker = Wait-WindowTitle $Target.Id 'MDLite コマンドパレット'
    $filter = Find-Child $picker 100 'Edit'
    $list = Find-Child $picker 101 'ListBox'
    if ($filter -eq [IntPtr]::Zero -or $list -eq [IntPtr]::Zero) { throw 'Command palette controls were not found.' }
    $countBefore = [MDLiteGitProductNative]::SendMessage($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $setTextResult = [MDLiteGitProductNative]::SendMessageSetText($filter, 0x000C, [IntPtr]::Zero, $Query).ToInt32()
    Start-Sleep -Milliseconds 150
    $filterText = [MDLiteGitProductNative]::WindowText($filter)
    $countAfterSetText = [MDLiteGitProductNative]::SendMessage($list, 0x018B, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $notificationWParam = [IntPtr]([int64]((0x0300 -shl 16) -bor 100))
    # WM_SETTEXT changed the filtered ListBox; the native EDIT EN_CHANGE values are derived from its ID/HWND.
    $count = $countAfterSetText
    $probe = [ordered]@{
        record_type = 'filter_probe'; query = $Query; filter_hwnd = $filter.ToInt64()
        filter_class = [MDLiteGitProductNative]::WindowClass($filter); filter_control_id = [MDLiteGitProductNative]::ControlId($filter)
        filter_text = $filterText; wm_settext_lresult = $setTextResult; list_count_before = $countBefore
        list_count_after_settext = $countAfterSetText
        parent_wm_command_wparam_from_control_id_and_EN_CHANGE = ('0x{0:X8}' -f $notificationWParam.ToInt64())
        parent_wm_command_lparam_filter_hwnd = $filter.ToInt64(); list_count_after_native_EN_CHANGE = $count
    }
    $script:paletteRuns.Add($probe)
    if ($count -ne 1) { throw "Command palette WM_SETTEXT '$filterText' matched $count rows (before=$countBefore after_settext=$countAfterSetText after_notification=$count); expected one." }
    [void][MDLiteGitProductNative]::SendMessage($list, 0x0186, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not [MDLiteGitProductNative]::PostMessage($picker, 0x0111, [IntPtr]1, [IntPtr]::Zero)) { throw 'Could not accept the command palette row.' }
    if (-not (Wait-WindowGone $Target.Id 'MDLite コマンドパレット')) { throw 'Command palette did not close after command acceptance.' }
    $interaction = [ordered]@{
        query = $Query; filter_hwnd = $filter.ToInt64(); filter_class = [MDLiteGitProductNative]::WindowClass($filter)
        filter_control_id = [MDLiteGitProductNative]::ControlId($filter); filter_text = $filterText
        wm_settext_lresult = $setTextResult; list_count_before = $countBefore; list_count_after_settext = $countAfterSetText
        parent_wm_command_wparam_from_control_id_and_EN_CHANGE = ('0x{0:X8}' -f $notificationWParam.ToInt64())
        parent_wm_command_lparam_filter_hwnd = $filter.ToInt64()
        list_count_after_native_EN_CHANGE = $count; route = 'Command Search control -> native palette -> Edit WM_SETTEXT -> native EN_CHANGE -> filtered row -> IDOK'
    }
    $script:paletteRuns.Add($interaction)
    return $interaction
}

function Invoke-PaletteEscapeFromFocus([IntPtr]$Window, [Diagnostics.Process]$Target, [IntPtr]$Control, [string]$Name) {
    $focused = [MDLiteGitProductNative]::FocusControl($Window, $Control)
    $beforeFocus = [MDLiteGitProductNative]::FocusedWindow($Window)
    $beforeState = Get-DocumentState $Window
    $selectionBefore = Get-SourceSelection $Window
    $diskHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $keySent = [MDLiteGitProductNative]::SendKeyChord([System.UInt16[]]@(0x11, 0x10, 0x50))
    $picker = Wait-WindowTitle $Target.Id 'MDLite コマンドパレット'
    if ($Name -eq 'editor') { $script:paletteScreenshot = Save-WindowFrame $picker (Join-Path $evidenceRoot "palette-custom-hints-$runId.png") }
    $escapeSent = [MDLiteGitProductNative]::SendKeyChord([System.UInt16[]]@(0x1B))
    $closed = Wait-WindowGone $Target.Id 'MDLite コマンドパレット'
    $afterState = Get-DocumentState $Window
    $selectionAfter = Get-SourceSelection $Window
    $diskHashAfter = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $afterFocus = [MDLiteGitProductNative]::FocusedWindow($Window)
    $record = [ordered]@{
        focus_name = $Name; focus_set = $focused; focused_hwnd_before = $beforeFocus.ToInt64()
        custom_binding_sendinput = $keySent; picker_opened = ($picker -ne [IntPtr]::Zero)
        escape_sendinput = $escapeSent; picker_closed = $closed; main_reenabled = [MDLiteGitProductNative]::IsWindowEnabled($Window)
        focused_hwnd_after = $afterFocus.ToInt64(); source_before = $beforeState; source_after = $afterState
        source_selection_before = $selectionBefore; source_selection_after = $selectionAfter
        selection_preserved = ($selectionBefore.anchor -eq $selectionAfter.anchor -and $selectionBefore.active -eq $selectionAfter.active)
        saved_revision = $beforeState.saved_revision
        disk_sha256_before = $diskHashBefore; disk_sha256_after = $diskHashAfter; disk_bytes_unchanged = ($diskHashBefore -ceq $diskHashAfter)
        source_unchanged = ($beforeState.dirty -eq $afterState.dirty -and $beforeState.undo_count -eq $afterState.undo_count -and
            $beforeState.redo_count -eq $afterState.redo_count -and $beforeState.revision -eq $afterState.revision -and
            $beforeState.source_length -eq $afterState.source_length -and $beforeState.final_code_unit -eq $afterState.final_code_unit -and
            $beforeState.saved_revision -eq $afterState.saved_revision)
    }
    $paletteRuns.Add($record)
    Add-Check "command_palette_custom_binding_escape_$Name" ($focused -and $keySent -and $picker -ne [IntPtr]::Zero -and
        $escapeSent -and $closed -and $record.main_reenabled -and $record.source_unchanged -and $record.selection_preserved -and $record.disk_bytes_unchanged) $record
}

function Find-ProductButton([IntPtr]$Window, [string]$Text, [int]$ControlId = -1) {
    if ($ControlId -ge 0) {
        $button = Find-Child $Window $ControlId 'Button'
        if ($button -eq [IntPtr]::Zero -or [MDLiteGitProductNative]::WindowText($button) -cne $Text) { return [IntPtr]::Zero }
        return $button
    }
    return Find-ChildByText $Window 'Button' $Text
}

function Get-ButtonEvidence([IntPtr]$Window, [string]$Text, [int]$ControlId = -1) {
    $button = Find-ProductButton $Window $Text $ControlId
    if ($button -eq [IntPtr]::Zero) {
        return [ordered]@{ exists = $false; hwnd = 0L; control_id = -1; name = $Text; enabled = $false; visible = $false }
    }
    return [ordered]@{
        exists = $true; hwnd = $button.ToInt64(); control_id = [MDLiteGitProductNative]::ControlId($button)
        name = [MDLiteGitProductNative]::WindowText($button)
        enabled = [MDLiteGitProductNative]::IsWindowEnabled($button)
        visible = [MDLiteGitProductNative]::IsWindowVisible($button)
    }
}

function Invoke-WorkspaceTrustPrompt([IntPtr]$Window, [Diagnostics.Process]$Target) {
    $control = Get-ButtonEvidence $Window '信頼を確認...' 1032 # kWorkspaceTrust
    if ($control.exists -and $control.enabled -and $control.visible) {
        Click-ProductButton ([IntPtr]$control.hwnd) 'Workspace Trust confirmation'
        return [ordered]@{ route = 'Git panel Trust button BM_CLICK via PostMessage'; control = $control; palette = $null }
    }
    $palette = Invoke-ProductCommandFromPalette $Window $Target 'Workspace: 信頼する'
    return [ordered]@{ route = 'Command Search: Workspace: 信頼する'; control = $control; palette = $palette }
}

function Ensure-FocusTargetVisible([IntPtr]$Window, [Diagnostics.Process]$Target,
                                   [int]$ControlId, [string]$ClassName, [string]$Command) {
    $control = Find-Child $Window $ControlId $ClassName
    if ($control -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($control)) { return $control }
    [void](Invoke-ProductCommandFromPalette $Window $Target $Command)
    Start-Sleep -Milliseconds 100
    $control = Find-Child $Window $ControlId $ClassName
    if ($control -eq [IntPtr]::Zero -or -not [MDLiteGitProductNative]::IsWindowVisible($control)) { return [IntPtr]::Zero }
    return $control
}

function Click-ProductButton([IntPtr]$Button, [string]$Name) {
    if ($Button -eq [IntPtr]::Zero) { throw "Product button was not found: $Name" }
    if (-not [MDLiteGitProductNative]::PostMessage($Button, 0x00F5, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw "Product button click could not be posted: $Name"
    }
}

function Get-DialogButton([IntPtr]$Dialog, [int]$ControlId) {
    $button = Find-Child $Dialog $ControlId 'Button'
    if ($button -eq [IntPtr]::Zero) { throw "Native dialog button id $ControlId was not found." }
    return $button
}

function Complete-TrustPrompt([int]$ProcessId, [bool]$Allow, [string]$EvidenceLabel = '') {
    $dialog = Wait-WindowTitle $ProcessId 'Workspace Trust' -RequiredButtonIds @(6, 7) -RequiredBody '信頼しますか'
    $dialogEvidence = Get-DialogControlsEvidence $dialog
    $promptKind = if ($Allow) { 'accept' } else { 'deny' }
    $dialogEvidence['screenshot'] = Save-WindowFrame $dialog (Join-Path $evidenceRoot "trust-prompt-$promptKind-$EvidenceLabel-$runId.png")
    $script:trustDialogFrames.Add($dialogEvidence.screenshot)
    $body = $dialogEvidence.static_body_wm_gettext
    if (-not $body.Contains('信頼しますか')) {
        throw "Workspace Trust Static body was not readable through WM_GETTEXT: $($dialogEvidence | ConvertTo-Json -Depth 6 -Compress)"
    }
    $decisionId = if ($Allow) { 6 } else { 7 } # IDYES / IDNO
    $decisionButton = Get-DialogButton $dialog $decisionId
    $decisionCaption = [MDLiteGitProductNative]::WindowText($decisionButton)
    $captionMatchesDecision = if ($Allow) { $decisionCaption -match '^(はい|Yes)' } else { $decisionCaption -match '^(いいえ|No)' }
    if (-not $captionMatchesDecision) { throw "Workspace Trust button id $decisionId has unexpected caption '$decisionCaption'." }
    Click-ProductButton $decisionButton 'Workspace Trust decision'
    if (-not (Wait-WindowGone $ProcessId 'Workspace Trust' -WindowHandle $dialog)) { throw 'Workspace Trust confirmation stayed open.' }
    if (-not $Allow) {
        return [pscustomobject]@{
            decision = 'NO'; decision_button_id = $decisionId; decision_button = $decisionCaption
            confirmation = $body; confirmation_controls = $dialogEvidence; result = ''; result_controls = $null
        }
    }

    $receipt = Wait-WindowTitle $ProcessId 'Workspace Trust' -RequiredBody 'このWorkspaceを信頼しました。' -TrustAcknowledgement
    $receiptEvidence = Get-DialogControlsEvidence $receipt
    $receiptEvidence['screenshot'] = Save-WindowFrame $receipt (Join-Path $evidenceRoot "trust-result-accept-$EvidenceLabel-$runId.png")
    $script:trustDialogFrames.Add($receiptEvidence.screenshot)
    $receiptText = $receiptEvidence.static_body_wm_gettext
    $ack = Select-TrustAcknowledgement $receiptEvidence 'このWorkspaceを信頼しました。'
    if ($null -eq $ack) { throw 'Workspace Trust success receipt lost its exact semantic OK contract.' }
    Click-ProductButton ([IntPtr]$ack.hwnd) 'Workspace Trust acknowledgement'
    if (-not (Wait-WindowGone $ProcessId 'Workspace Trust' -WindowHandle $receipt)) { throw 'Workspace Trust result dialog stayed open.' }
    return [pscustomobject]@{
        decision = 'YES'; decision_button_id = $decisionId; decision_button = $decisionCaption
        confirmation = $body; confirmation_controls = $dialogEvidence
        result = $receiptText; result_contains_trusted_message = $receiptText.Contains('信頼しました')
        result_controls = $receiptEvidence; acknowledgement_button_id = $ack.control_id; acknowledgement_button = $ack.wm_gettext; acknowledgement_semantic_ok = $true
    }
}

function Complete-TrustRevocation([IntPtr]$Window, [Diagnostics.Process]$Target) {
    $command = Invoke-ProductCommandFromPalette $Window $Target 'Workspace: 信頼を解除'
    $dialog = Wait-WindowTitle $Target.Id 'Workspace Trust' -RequiredBody 'Workspaceの信頼を解除しました。' -TrustAcknowledgement
    $dialogEvidence = Get-DialogControlsEvidence $dialog
    $body = $dialogEvidence.static_body_wm_gettext
    $ack = Select-TrustAcknowledgement $dialogEvidence 'Workspaceの信頼を解除しました。'
    if ($null -eq $ack) { throw 'Workspace Trust revoke receipt lost its exact semantic OK contract.' }
    Click-ProductButton ([IntPtr]$ack.hwnd) 'Workspace Trust revocation acknowledgement'
    if (-not (Wait-WindowGone $Target.Id 'Workspace Trust' -WindowHandle $dialog)) { throw 'Workspace Trust revoke result stayed open.' }
    return [pscustomobject]@{
        command = $command; result = $body; result_contains_revoked_message = $body.Contains('信頼を解除しました')
        result_controls = $dialogEvidence; acknowledgement_button_id = $ack.control_id; acknowledgement_button = $ack.wm_gettext; acknowledgement_semantic_ok = $true
    }
}

function Get-TestTrustFiles([string]$Workspace) {
    $root = Join-Path $Workspace '.mdlite\.state\test-trust'
    if (-not (Test-Path -LiteralPath $root -PathType Container)) { return ,@() }
    return ,@(Get-ChildItem -LiteralPath $root -Filter '*.trust' -File -ErrorAction SilentlyContinue)
}

function Get-TestTrustInventory([string]$Workspace) {
    $currentPath = [IO.Path]::GetFullPath($Workspace).ToLowerInvariant()
    $magic = "MDLITE_TRUST_V1`n"
    $magicBytes = [Text.Encoding]::ASCII.GetBytes($magic)
    $files = foreach ($file in (Get-TestTrustFiles $Workspace | Sort-Object Name)) {
        $bytes = [IO.File]::ReadAllBytes($file.FullName)
        $hasHeader = $bytes.Length -ge $magicBytes.Length -and
            [Text.Encoding]::ASCII.GetString($bytes, 0, $magicBytes.Length) -ceq $magic
        $storedIdentity = ''
        if ($hasHeader -and (($bytes.Length - $magicBytes.Length) % 2 -eq 0)) {
            $storedIdentity = [Text.Encoding]::Unicode.GetString($bytes, $magicBytes.Length, $bytes.Length - $magicBytes.Length)
        }
        $storedPath = if ($storedIdentity) { ($storedIdentity -split "`n", 2)[0] } else { '' }
        [ordered]@{
            name = $file.Name; length = $file.Length; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant()
            valid_header = $hasHeader; stored_identity = $storedIdentity; stored_path = $storedPath
            stored_path_matches_current_workspace = ($storedPath -and $storedPath.Equals($currentPath, [StringComparison]::OrdinalIgnoreCase))
        }
    }
    $items = @($files)
    return [ordered]@{
        current_workspace_path = $currentPath; file_count = $items.Count
        files_matching_current_workspace_path = @($items | Where-Object { $_.stored_path_matches_current_workspace }).Count
        files = $items
    }
}

function Test-ProcessAlive([int]$ProcessId) {
    if ($ProcessId -le 0) { return $false }
    try {
        $target = [Diagnostics.Process]::GetProcessById($ProcessId)
        try { return -not $target.HasExited }
        finally { $target.Dispose() }
    } catch {
        return $false
    }
}

function New-WorkspaceFixture([string]$Workspace) {
    [IO.Directory]::CreateDirectory((Join-Path $Workspace '.mdlite')) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $Workspace 'records')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $Workspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`nbind.view.commandPalette = `"Ctrl+Shift+P`"`nbind.file.new = `"Ctrl+Shift+N`"`n",
        [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $Workspace 'README.md'), "# Git product route $runId`n`nKeep this source unchanged during Git work.`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $Workspace 'records\created-day.md'), "# Calendar rename fixture $runId`n", [Text.UTF8Encoding]::new($false))
}

function Start-MDLite([string]$DocumentPath, [bool]$WithoutGit, [bool]$Silent = $true) {
    $oldPath = $env:PATH
    $oldHttpProxy = $env:HTTP_PROXY
    $oldHttpsProxy = $env:HTTPS_PROXY
    $oldAllProxy = $env:ALL_PROXY
    $oldNoProxy = $env:NO_PROXY
    $oldGitCeiling = $env:GIT_CEILING_DIRECTORIES
    $oldSilent = $env:MDLITE_TEST_SILENT
    try {
        $env:MDLITE_TEST_SILENT = if ($Silent) { '1' } else { '0' }
        if ($WithoutGit) { $env:PATH = $emptyPath }
        $env:HTTP_PROXY = ''
        $env:HTTPS_PROXY = ''
        $env:ALL_PROXY = ''
        $env:NO_PROXY = '127.0.0.1,localhost'
        $env:GIT_CEILING_DIRECTORIES = $runRoot
        return Start-Process -FilePath $executable -ArgumentList @($DocumentPath) -WorkingDirectory $repoRoot -PassThru
    } finally {
        if ($null -eq $oldSilent) { Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_SILENT = $oldSilent }
        $env:PATH = $oldPath
        if ($null -eq $oldHttpProxy) { Remove-Item Env:HTTP_PROXY -ErrorAction SilentlyContinue } else { $env:HTTP_PROXY = $oldHttpProxy }
        if ($null -eq $oldHttpsProxy) { Remove-Item Env:HTTPS_PROXY -ErrorAction SilentlyContinue } else { $env:HTTPS_PROXY = $oldHttpsProxy }
        if ($null -eq $oldAllProxy) { Remove-Item Env:ALL_PROXY -ErrorAction SilentlyContinue } else { $env:ALL_PROXY = $oldAllProxy }
        if ($null -eq $oldNoProxy) { Remove-Item Env:NO_PROXY -ErrorAction SilentlyContinue } else { $env:NO_PROXY = $oldNoProxy }
        if ($null -eq $oldGitCeiling) { Remove-Item Env:GIT_CEILING_DIRECTORIES -ErrorAction SilentlyContinue } else { $env:GIT_CEILING_DIRECTORIES = $oldGitCeiling }
    }
}

function Stop-MDLite([Diagnostics.Process]$Target, [IntPtr]$Window) {
    if (-not $Target) { return }
    $Target.Refresh()
    if ($Target.HasExited) { $Target.Dispose(); return }
    if ($Window -ne [IntPtr]::Zero) { [void][MDLiteGitProductNative]::PostMessage($Window, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero) }
    if (-not $Target.WaitForExit(2500)) { $Target.Kill(); [void]$Target.WaitForExit(3000) }
    $Target.Dispose()
}

function Save-WindowFrame([IntPtr]$Window, [string]$Path) {
    $bitmap = $null
    $graphics = $null
    try {
        [MDLiteGitProductNative+RECT]$rect = New-Object MDLiteGitProductNative+RECT
        if (-not [MDLiteGitProductNative]::GetWindowRect($Window, [ref]$rect)) {
            return [pscustomobject]@{ status = 'BLOCKED_CAPTURE'; path = $Path; error = 'GetWindowRect failed.' }
        }
        $width = $rect.Right - $rect.Left
        $height = $rect.Bottom - $rect.Top
        $bitmap = [Drawing.Bitmap]::new($width, $height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $dc = $graphics.GetHdc()
        try { $captured = [MDLiteGitProductNative]::PrintWindow($Window, $dc, 2) }
        finally { $graphics.ReleaseHdc($dc) }
        if (-not $captured) { return [pscustomobject]@{ status = 'BLOCKED_CAPTURE'; path = $Path; error = 'PrintWindow returned false.' } }
        $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
        return [pscustomobject]@{ status = 'CAPTURED_PRINTWINDOW'; path = $Path; width = $width; height = $height }
    } catch {
        return [pscustomobject]@{ status = 'BLOCKED_CAPTURE'; path = $Path; error = $_.Exception.Message }
    } finally {
        if ($graphics) { $graphics.Dispose() }
        if ($bitmap) { $bitmap.Dispose() }
    }
}

function Ensure-GitPanelVisible([IntPtr]$Window, [Diagnostics.Process]$Target,
                                [string]$ExpectedSummary, [bool]$RequireList = $false, [int]$TimeoutMs = 5000) {
    # Activity activation shows/expands/focuses; a visibility toggle can hide an
    # already open panel during transient status publication or child layout.
    $activity = Find-ProductButton $Window 'Git（ソース管理）' 125
    Click-ProductButton $activity 'Git activity activation'
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $summary = Get-GitSummary $Window
        $list = Get-GitList $Window
        $listReady = -not $RequireList
        if ($RequireList -and $list -ne [IntPtr]::Zero) {
            $bounds = New-Object MDLiteGitProductNative+RECT
            $listReady = [MDLiteGitProductNative]::GetWindowRect($list, [ref]$bounds) -and
                $bounds.Right -gt $bounds.Left -and $bounds.Bottom -gt $bounds.Top
        }
        if ($summary -and $summary.Contains($ExpectedSummary) -and $listReady) { return $list }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $children = @([MDLiteGitProductNative]::Children($Window) | ForEach-Object {
        [pscustomobject]@{ id = [MDLiteGitProductNative]::ControlId($_)
            class = [MDLiteGitProductNative]::WindowClass($_)
            visible = [MDLiteGitProductNative]::IsWindowVisible($_) }
    })
    throw "Git panel readiness timed out for '$ExpectedSummary', require_list=$RequireList, summary='$summary', pid=$($Target.Id), children=$($children | ConvertTo-Json -Compress)."
}

function Wait-FreshFetchStartup([Diagnostics.Process]$Target, [string]$DocumentPath, [DateTime]$StartedUtc,
    [System.Collections.IDictionary]$Trace) {
    $mainDeadline = $StartedUtc.AddMilliseconds(10000)
    $deadline = $StartedUtc.AddMilliseconds(15000)
    $expectedCaption = 'MDLite — ' + [IO.Path]::GetFileName($DocumentPath) + ' — ' + [IO.Path]::GetFileName([IO.Path]::GetDirectoryName($DocumentPath))
    $Trace.process_started_utc = $StartedUtc.ToString('o'); $Trace.total_budget_ms = 15000
    $Trace.expected_caption = $expectedCaption; $Trace.initial = $null; $Trace.first_visible_utc = $null; $Trace.ready_utc = $null; $Trace.last_controls = @()
    do {
        $Target.Refresh()
        if ($Target.HasExited) { throw "Fresh Fetch process exited before startup readiness (exit=$($Target.ExitCode))." }
        if ([DateTime]::UtcNow -ge $deadline) { break }
        $window = [MDLiteGitProductNative]::FindMainWindow($Target.Id)
        $visible = $window -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($window)
        if ($null -eq $Trace.initial) { $Trace.initial = [ordered]@{ utc = [DateTime]::UtcNow.ToString('o'); hwnd = $window.ToInt64(); visible = $visible } }
        if ($visible) {
            if ($null -eq $Trace.first_visible_utc) { $Trace.first_visible_utc = [DateTime]::UtcNow.ToString('o') }
            $activity = Get-ButtonEvidence $window 'Git（ソース管理）' 125
            $editor = Find-Child $window 102 'RICHEDIT50W'
            $editorVisible = $editor -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($editor)
            $editorEnabled = $editor -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowEnabled($editor)
            $caption = [MDLiteGitProductNative]::WindowText($window)
            $Trace.last_controls = @($activity, [ordered]@{ hwnd = $editor.ToInt64(); control_id = 102; class_name = 'RICHEDIT50W'; visible = $editorVisible; enabled = $editorEnabled })
            $Trace.last_caption = $caption
            if ($caption -ceq $expectedCaption -and $activity.exists -and $activity.visible -and $activity.enabled -and
                $editorVisible -and $editorEnabled -and [MDLiteGitProductNative]::IsWindowEnabled($window) -and [DateTime]::UtcNow -lt $deadline) {
                $Trace.ready_utc = [DateTime]::UtcNow.ToString('o')
                $Trace.ready_elapsed_ms = [Math]::Round(([DateTime]::UtcNow - $StartedUtc).TotalMilliseconds)
                return $window
            }
        } elseif ([DateTime]::UtcNow -ge $mainDeadline) {
            throw "Fresh Fetch main window was not visible within its existing 10000 ms budget (pid=$($Target.Id))."
        }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    $Trace.timeout_utc = [DateTime]::UtcNow.ToString('o')
    throw "Fresh Fetch requested-document/activity/editor readiness exhausted the shared 15000 ms startup budget (pid=$($Target.Id), trace=$($Trace | ConvertTo-Json -Depth 4 -Compress))."
}

function Update-GitStatus([IntPtr]$Window) {
    $refresh = Find-ProductButton $Window '更新'
    Click-ProductButton $refresh 'Git status refresh'
}

function Get-RealTrustInventory([string]$Workspace) {
    $root = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'MDLite\trust\v1'
    $identityPath = [IO.Path]::GetFullPath($Workspace).ToLowerInvariant()
    $magic = "MDLITE_TRUST_V1`n"
    $files = @(if (Test-Path -LiteralPath $root -PathType Container) {
        foreach ($file in (Get-ChildItem -LiteralPath $root -File | Sort-Object Name)) {
            $bytes = [IO.File]::ReadAllBytes($file.FullName)
            $matchesFixture = $false
            if ($bytes.Length -gt $magic.Length -and [Text.Encoding]::ASCII.GetString($bytes, 0, $magic.Length) -ceq $magic) {
                $identity = [Text.Encoding]::Unicode.GetString($bytes, $magic.Length, $bytes.Length - $magic.Length)
                $matchesFixture = ($identity -split "`n", 2)[0] -ceq $identityPath
            }
            [pscustomobject]@{ name = $file.Name; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant(); matches_fixture = $matchesFixture }
        }
    })
    return [pscustomobject]@{ files = $files; fixture_entries = @($files | Where-Object matches_fixture).Count }
}

function Test-OtherTrustEntriesUnchanged($Before, $After) {
    $left = @($Before.files | Where-Object { -not $_.matches_fixture } | Sort-Object name | ForEach-Object { $_.name + ':' + $_.sha256 }) -join "`n"
    $right = @($After.files | Where-Object { -not $_.matches_fixture } | Sort-Object name | ForEach-Object { $_.name + ':' + $_.sha256 }) -join "`n"
    return $left -ceq $right
}

function Get-FetchFixtureState([string]$Workspace, [IntPtr]$Editor) {
    $head = Get-GitOutput $Workspace @('rev-parse', 'HEAD')
    $status = Get-GitOutput $Workspace @('status', '--short')
    $refs = Get-GitOutput $Workspace @('for-each-ref', '--sort=refname', '--format=%(refname) %(objectname)')
    $objects = Get-GitOutput $Workspace @('cat-file', '--batch-all-objects', '--batch-check=%(objectname)')
    if ($head.exit_code -ne 0 -or $status.exit_code -ne 0 -or $refs.exit_code -ne 0 -or $objects.exit_code -ne 0) { throw 'Could not read isolated Fetch fixture Git state.' }
    $files = @('README.md', 'tracked.md', 'other.md', '.gitignore', 'records\created-day.md') | ForEach-Object {
        $_ + ':' + (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Workspace $_)).Hash.ToLowerInvariant()
    }
    $objectFiles = @(Get-ChildItem -LiteralPath (Join-Path $Workspace '.git\objects') -File -Recurse | Sort-Object FullName | ForEach-Object {
        $_.FullName.Substring($Workspace.Length) + ':' + (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
    }) -join "`n"
    $fetchHeadPath = Join-Path $Workspace '.git\FETCH_HEAD'
    $fetchHead = if (Test-Path -LiteralPath $fetchHeadPath -PathType Leaf) {
        [ordered]@{ exists = $true; length = [IO.FileInfo]::new($fetchHeadPath).Length; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $fetchHeadPath).Hash.ToLowerInvariant() }
    } else { [ordered]@{ exists = $false; length = 0; sha256 = $null } }
    return [ordered]@{ head = $head.output; status = $status.output; index_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $Workspace '.git\index')).Hash.ToLowerInvariant(); worktree = $files -join "`n"; editor_text = [MDLiteGitProductNative]::WindowText($Editor); refs = $refs.output; object_ids = @($objects.output -split "`n" | Where-Object { $_ } | Sort-Object) -join "`n"; object_files = $objectFiles; fetch_head = $fetchHead }
}

function Test-FetchCancellationState([System.Collections.IDictionary]$Before, [System.Collections.IDictionary]$After,
    [string]$ResultBody, [bool]$LoopbackConnected) {
    # Git opens/truncates FETCH_HEAD before contacting the remote; empty metadata is not a fetched ref.
    $emptyFetchHead = -not $Before.fetch_head.exists -and (-not $After.fetch_head.exists -or
        ($After.fetch_head.length -eq 0 -and $After.fetch_head.sha256 -ceq 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'))
    return $LoopbackConnected -and $ResultBody -match '(?m)^終了コード: 1223\r?$' -and
        $ResultBody.Contains('(ユーザーがキャンセルしました)') -and $emptyFetchHead -and
        $Before.head -ceq $After.head -and $Before.index_sha256 -ceq $After.index_sha256 -and
        $Before.worktree -ceq $After.worktree -and $Before.status -ceq $After.status -and $Before.editor_text -ceq $After.editor_text -and
        $Before.refs -ceq $After.refs -and $Before.object_ids -ceq $After.object_ids -and $Before.object_files -ceq $After.object_files
}

function Select-FetchTaskCancel([System.Collections.IDictionary]$Evidence) {
    if ($Evidence.dialog_class -cne '#32770' -or $Evidence.dialog_caption -cne 'Fetch' -or -not $Evidence.dialog_visible) { return $null }
    $directUi = @($Evidence.controls | Where-Object { $_.class_name -ceq 'DirectUIHWND' -and $_.visible })
    $cancel = @($Evidence.buttons | Where-Object { $_.class_name -ceq 'Button' -and $_.wm_gettext -cmatch '^(Cancel|キャンセル)$' -and $_.visible -and $_.enabled })
    $done = @($Evidence.buttons | Where-Object { $_.class_name -ceq 'Button' -and $_.wm_gettext -ceq '完了' -and $_.visible -and -not $_.enabled })
    if ($directUi.Count -eq 0 -or $cancel.Count -ne 1 -or $done.Count -ne 1) { return $null }
    # DirectUI native HWND IDs can be 0; virtual TaskDialog IDCANCEL is not a native child-control ID.
    return $cancel[0]
}

function Wait-FetchTaskCancel([int]$ProcessId, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $lastEvidence = $null
    do {
        $dialog = [MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, 'Fetch')
        if ($dialog -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($dialog)) {
            $lastEvidence = Get-DialogControlsEvidence $dialog
            $cancel = Select-FetchTaskCancel $lastEvidence
            if ($null -ne $cancel) { return [pscustomobject]@{ dialog = $dialog; cancel = $cancel; evidence = $lastEvidence } }
        }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Running Fetch TaskDialog semantic Cancel was not ready within $TimeoutMs ms: $($lastEvidence | ConvertTo-Json -Depth 5 -Compress)"
}

function Complete-FetchResult([int]$ProcessId, [string]$Workspace) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds(10000)
    do {
        $dialog = [MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, 'Fetch')
        if ($dialog -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($dialog)) {
            $evidence = Get-DialogControlsEvidence $dialog
            $body = $evidence.static_body_wm_gettext
            $buttons = @($evidence.buttons)
            if ($evidence.dialog_class -ceq '#32770' -and $body.Contains('作業ディレクトリ: ' + $Workspace) -and
                $body.Contains('コマンド: git fetch --prune') -and $body.Contains('(ユーザーがキャンセルしました)') -and
                $buttons.Count -eq 1 -and $buttons[0].wm_gettext -ceq 'OK' -and $buttons[0].control_id -in @(1, 2) -and
                $buttons[0].visible -and $buttons[0].enabled) {
                Click-ProductButton ([IntPtr]$buttons[0].hwnd) 'Fetch cancelled-result acknowledgement'
                if (-not (Wait-WindowGone $ProcessId 'Fetch' -WindowHandle $dialog)) { throw 'Fetch result acknowledgement did not close.' }
                return $evidence
            }
        }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Exact cancelled Fetch result and sole semantic OK were not ready within 10000 ms.'
}

function Invoke-FreshLoopbackFetch {
    $workspace = Join-Path $runRoot 'fetch-fresh-workspace'
    $target = $null; $window = [IntPtr]::Zero; $server = $null
    $evidence = [ordered]@{ fixture = $workspace; silent = $false; test_hooks_used = $false; stage = 'preparing_fixture'; grant_created = $false; cleanup_status = 'NOT_STARTED' }
    $beforeTrust = $null; $pass = $false; $grantAttempted = $false; $revocationConfirmed = $false
    try {
        if (Test-Path -LiteralPath $workspace) { throw 'Fresh Fetch fixture already exists; no draft workspace will be restarted.' }
        New-WorkspaceFixture $workspace
        [IO.File]::WriteAllText((Join-Path $workspace '.gitignore'), ".mdlite/`n", [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $workspace 'tracked.md'), 'baseline', [Text.UTF8Encoding]::new($false))
        foreach ($setupArguments in @(@('init', '--initial-branch=main'), @('config', '--local', 'user.name', 'MDLite acceptance'),
            @('config', '--local', 'user.email', 'mdlite-acceptance@example.invalid'), @('add', '-A'), @('commit', '-m', 'fresh Fetch fixture'))) {
            $setup = Get-GitOutput $workspace $setupArguments
            if ($setup.exit_code -ne 0) { throw 'Fresh Fetch fixture Git setup failed.' }
        }
        [IO.File]::WriteAllText((Join-Path $workspace 'tracked.md'), 'staged', [Text.UTF8Encoding]::new($false))
        $setup = Get-GitOutput $workspace @('add', '--', 'tracked.md')
        if ($setup.exit_code -ne 0) { throw 'Fresh Fetch staged fixture setup failed.' }
        [IO.File]::WriteAllText((Join-Path $workspace 'tracked.md'), 'unstaged', [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $workspace 'other.md'), 'untracked', [Text.UTF8Encoding]::new($false))
        $server = [MDLiteLocalStallServer]::new(); $server.Start()
        $url = "http://127.0.0.1:$($server.Port)/fixture.git"
        $setup = Get-GitOutput $workspace @('remote', 'add', 'origin', $url)
        if ($setup.exit_code -ne 0) { throw 'Fresh Fetch loopback remote setup failed.' }
        $beforeTrust = Get-RealTrustInventory $workspace
        if ($beforeTrust.fixture_entries -ne 0 -or (Test-Path -LiteralPath (Join-Path $workspace '.mdlite\.state\recovery'))) { throw 'Fresh Fetch fixture already has Trust or recovery data.' }
        $evidence.stage = 'starting_non_silent'
        $document = Join-Path $workspace 'README.md'
        $target = Start-MDLite $document $false $false
        $startedUtc = $target.StartTime.ToUniversalTime()
        $evidence.process_id = $target.Id; $evidence.startup = [ordered]@{}
        $evidence.stage = 'waiting_startup'
        $window = Wait-FreshFetchStartup $target $document $startedUtc $evidence.startup
        $remainingMs = [Math]::Min(5000, [int][Math]::Floor(($startedUtc.AddMilliseconds(15000) - [DateTime]::UtcNow).TotalMilliseconds))
        if ($remainingMs -le 0) { throw 'Fresh Fetch startup budget exhausted before Git activity activation.' }
        $evidence.stage = 'showing_untrusted_git'
        [void](Ensure-GitPanelVisible $window $target 'Workspaceは未信頼です' $false $remainingMs)
        $evidence.startup.panel_ready_utc = [DateTime]::UtcNow.ToString('o')
        if ([DateTime]::UtcNow -ge $startedUtc.AddMilliseconds(15000)) { throw 'Fresh Fetch shared startup budget expired before Git panel readiness.' }
        $evidence.stage = 'granting_trust'
        $grantAttempted = $true
        $evidence.trust_route = Invoke-WorkspaceTrustPrompt $window $target
        $evidence.trust_grant = Complete-TrustPrompt $target.Id $true 'fetch-fresh'
        $granted = Get-RealTrustInventory $workspace
        $evidence.grant_created = $granted.fixture_entries -gt 0
        $evidence.trust_before = $beforeTrust; $evidence.trust_granted = $granted
        if ($granted.fixture_entries -ne 1 -or -not (Test-OtherTrustEntriesUnchanged $beforeTrust $granted)) { throw 'Ordinary Trust grant did not affect only the fresh Fetch fixture.' }
        $evidence.stage = 'waiting_trusted_git'
        [void](Ensure-GitPanelVisible $window $target 'unstaged 1' $true)
        $editor = Find-Child $window 102 'RICHEDIT50W'
        if ($editor -eq [IntPtr]::Zero -or -not [MDLiteGitProductNative]::IsWindowVisible($editor)) { throw 'Fresh Fetch editor is not visible.' }
        $before = Get-FetchFixtureState $workspace $editor
        $fetchHead = Join-Path $workspace '.git\FETCH_HEAD'
        if (Test-Path -LiteralPath $fetchHead) { throw 'Fresh Fetch fixture unexpectedly has FETCH_HEAD.' }
        $evidence.stage = 'fetch_confirmation'
        $evidence.palette = Invoke-ProductCommandFromPalette $window $target 'Git: Fetch'
        $confirmation = Wait-WindowTitle $target.Id 'Git 明示操作' -RequiredButtonIds @(6, 7) -RequiredBody 'コマンド: git fetch --prune'
        $question = Get-DialogChildText $confirmation
        $yes = Get-DialogButton $confirmation 6; $no = Get-DialogButton $confirmation 7
        if (-not $question.Contains('Fetchを実行しますか？') -or -not $question.Contains('作業ディレクトリ: ' + $workspace) -or
            [MDLiteGitProductNative]::WindowText($yes) -notmatch '^(はい|Yes)' -or [MDLiteGitProductNative]::WindowText($no) -notmatch '^(いいえ|No)') { throw 'Fetch confirmation did not match the isolated command/workspace and Yes/No contract.' }
        $evidence.confirmation = Get-DialogControlsEvidence $confirmation
        Click-ProductButton $yes 'Fetch explicit Yes'
        if (-not (Wait-WindowGone $target.Id 'Git 明示操作' -WindowHandle $confirmation)) { throw 'Fetch confirmation did not close.' }
        $evidence.stage = 'fetch_running'
        $taskReady = Wait-FetchTaskCancel $target.Id
        $task = $taskReady.dialog
        $cancel = [IntPtr]$taskReady.cancel.hwnd
        $connected = $server.WaitForConnection(2500)
        $evidence.task_dialog = $taskReady.evidence
        $evidence.cancel_dispatch = [ordered]@{ native_hwnd = $cancel.ToInt64(); native_control_id = $taskReady.cancel.control_id; caption = $taskReady.cancel.wm_gettext; method = 'Captured native Button HWND BM_CLICK via PostMessage; no WM_COMMAND with native ID0' }
        Click-ProductButton $cancel 'Fetch cancellation'
        if (-not (Wait-WindowGone $target.Id 'Fetch' 10000 -WindowHandle $task)) { throw 'Fetch TaskDialog did not close after Cancel.' }
        $evidence.stage = 'fetch_result'
        $evidence.result = Complete-FetchResult $target.Id $workspace
        $evidence.stage = 'verifying_baseline'
        $after = Get-FetchFixtureState $workspace $editor
        $evidence.before = $before; $evidence.after = $after; $evidence.loopback_connection_accepted = $connected; $evidence.fetch_head_created = Test-Path -LiteralPath $fetchHead
        $pass = Test-FetchCancellationState $before $after $evidence.result.static_body_wm_gettext $connected
    } catch { $evidence.error = $_.Exception.Message; $evidence.failed_stage = $evidence.stage }
    finally {
        if ($beforeTrust) {
            try {
                $current = Get-RealTrustInventory $workspace
                if ($grantAttempted -and $current.fixture_entries -gt 0) {
                    $evidence.grant_created = $true
                    if (-not $target -or $target.HasExited) { throw 'Fresh Fetch process is unavailable for ordinary Trust revocation.' }
                    $pending = [MDLiteGitProductNative]::FindTopLevelWindow($target.Id, 'Workspace Trust')
                    if ($pending -ne [IntPtr]::Zero) {
                        $ack = Select-TrustAcknowledgement (Get-DialogControlsEvidence $pending) 'このWorkspaceを信頼しました。'
                        if ($null -eq $ack) { throw 'Unexpected modal prevents safe ordinary Trust cleanup.' }
                        Click-ProductButton ([IntPtr]$ack.hwnd) 'Pending fresh Trust acknowledgement'
                        if (-not (Wait-WindowGone $target.Id 'Workspace Trust' -WindowHandle $pending)) { throw 'Pending Trust acknowledgement did not close.' }
                    }
                    if (-not [MDLiteGitProductNative]::IsWindowEnabled($window)) { throw 'A remaining modal prevents safe ordinary Trust cleanup.' }
                    $evidence.trust_revoke = Complete-TrustRevocation $window $target
                    if (-not (Get-GitSummary $window).Contains('Workspaceは未信頼です')) { throw 'Fresh fixture UI did not return to untrusted after revocation.' }
                    $revocationConfirmed = $true
                }
                $finalTrust = Get-RealTrustInventory $workspace
                $evidence.trust_after = $finalTrust
                $cleanupOk = $finalTrust.fixture_entries -eq 0 -and (Test-OtherTrustEntriesUnchanged $beforeTrust $finalTrust) -and (-not $grantAttempted -or $revocationConfirmed)
                $evidence.cleanup_status = if (-not $cleanupOk) { 'UNKNOWN_PRESERVED_STORE' } elseif (-not $grantAttempted) { 'NO_GRANT_CREATED_OTHER_ENTRIES_UNCHANGED' } else { 'CONFIRMED_FIXTURE_REVOKED_OTHER_ENTRIES_UNCHANGED' }
                $pass = $pass -and $cleanupOk
            } catch { $evidence.cleanup_status = 'UNKNOWN_PRESERVED_STORE'; $evidence.cleanup_error = $_.Exception.Message; $pass = $false }
        }
        try { if ($target) { Stop-MDLite $target $window } }
        finally { if ($server) { $server.Dispose() } }
    }
    return [pscustomobject]@{ pass = $pass; evidence = $evidence }
}

try {
    [IO.Directory]::CreateDirectory($evidenceRoot) | Out-Null
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    [IO.Directory]::CreateDirectory($emptyPath) | Out-Null
    [IO.File]::WriteAllText($ownerMarker, $runId, [Text.UTF8Encoding]::new($false))
    if (-not (Test-Path -LiteralPath $buildReceiptPath -PathType Leaf)) { throw "Build receipt is missing: $buildReceiptPath" }
    $buildReceipt = Get-Content -Raw -LiteralPath $buildReceiptPath | ConvertFrom-Json
    $sourceFingerprintBefore = Get-SourceFingerprint
    $executableHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    if ($buildReceipt.build_status -ne 'PASS' -or $buildReceipt.executable_sha256 -ne $executableHashBefore -or
        $buildReceipt.source_sha256 -ne $sourceFingerprintBefore.sha256) {
        throw 'The executable/source fingerprint does not match the fresh build receipt.'
    }

    # Trust UI is a separate diagnostic lane; failures here never suppress the Git machine cases.
    try {
        New-WorkspaceFixture $noGitWorkspace
        $noGitDocument = Join-Path $noGitWorkspace 'README.md'
        $noGitProcess = Start-MDLite $noGitDocument $true
        $noGitMain = Find-MainWindow $noGitProcess
        $noGitProcessId = $noGitProcess.Id
        $noGitMainHandle = $noGitMain.ToInt64()
        [uint32]$noGitOwnerPid = 0
        [void][MDLiteGitProductNative]::GetWindowThreadProcessId($noGitMain, [ref]$noGitOwnerPid)
        $noGitMainOwnerPid = [int]$noGitOwnerPid
        [void][MDLiteGitProductNative]::SetForegroundWindow($noGitMain)
        [void][MDLiteGitProductNative]::SetWindowPos($noGitMain, [IntPtr]::Zero, 80, 80, 1440, 960, 0x0004)

        $trustUiEvidence.stage = 'mode1_store_selection'
        $trustStoreHook = [MDLiteGitProductNative]::SendMessage($noGitMain, 0x8043, [IntPtr]1, [IntPtr]::Zero).ToInt32()
        $trustInventoryBefore = Get-TestTrustInventory $noGitWorkspace
        $initialTrustButton = Get-ButtonEvidence $noGitMain '信頼を確認...' 1032
        $initialTrustSummary = Get-GitSummary $noGitMain
        $initialTrustList = Get-GitList $noGitMain
        $trustUiEvidence.details.mode1_store_selection = [ordered]@{
            hook_wparam = 1; hook_result = $trustStoreHook; trust_store_inventory = $trustInventoryBefore
            current_rendered_trust_state = if ($initialTrustSummary -match 'Workspaceは未信頼です') { 'UNTRUSTED' } elseif ($initialTrustButton.visible) { 'UNTRUSTED' } else { 'NOT_OBSERVED' }
            visible_summary = $initialTrustSummary; trust_button = $initialTrustButton
            git_listview_present = ($initialTrustList -ne [IntPtr]::Zero)
            listview_absence_expected_while_untrusted = $true
            store_root = (Join-Path $noGitWorkspace '.mdlite\.state\test-trust')
        }
        $mode1SelectionStatus = if ($trustStoreHook -eq 1) { 'PASS' } else { 'BLOCKED' }
        $checks['trust_test_mode_selects_fixture_store_without_grant'] = [ordered]@{
            status = $mode1SelectionStatus; details = $trustUiEvidence.details.mode1_store_selection
        }
        if ($trustStoreHook -ne 1) { throw 'WM_APP+67 wParam=1 did not select the fixture-only Trust store.' }

        $trustUiEvidence.stage = 'deny'
        $denyRoute = Invoke-WorkspaceTrustPrompt $noGitMain $noGitProcess
        $trustDenied = Complete-TrustPrompt $noGitProcess.Id $false
        $afterDeny = Get-GitSummary $noGitMain
        $trustInventoryAfterDeny = Get-TestTrustInventory $noGitWorkspace
        $trustStoreBeforeDenyFingerprint = (@($trustInventoryBefore.files | Sort-Object name | ForEach-Object { '{0}:{1}' -f $_.name, $_.sha256 }) -join "`n")
        $trustStoreAfterDenyFingerprint = (@($trustInventoryAfterDeny.files | Sort-Object name | ForEach-Object { '{0}:{1}' -f $_.name, $_.sha256 }) -join "`n")
        $trustStoreUnchangedByDeny = $trustStoreBeforeDenyFingerprint -ceq $trustStoreAfterDenyFingerprint
        $trustUiEvidence.details.deny = [ordered]@{
            activation_route = $denyRoute; decision = $trustDenied.decision; confirmation = $trustDenied.confirmation
            confirmation_controls = $trustDenied.confirmation_controls
            visible_summary = $afterDeny; trust_store_inventory = $trustInventoryAfterDeny
            trust_store_unchanged_by_deny = $trustStoreUnchangedByDeny
            trust_button = Get-ButtonEvidence $noGitMain '信頼を確認...' 1032
            git_listview_present = (Get-GitList $noGitMain) -ne [IntPtr]::Zero
        }
        $noGitPanelSnapshots.Add([pscustomobject]@{ phase = 'after_ui_deny'; text = $afterDeny })
        Add-Check 'trust_ui_deny_keeps_workspace_untrusted' (
            $trustDenied.decision -eq 'NO' -and $trustDenied.confirmation.Contains('信頼しますか') -and $trustStoreUnchangedByDeny
        ) $trustUiEvidence.details.deny

        $trustUiEvidence.stage = 'accept'
        $acceptRoute = Invoke-WorkspaceTrustPrompt $noGitMain $noGitProcess
        $trustAccepted = Complete-TrustPrompt $noGitProcess.Id $true
        $noGitPanelEnsureError = $null
        try { [void](Ensure-GitPanelVisible $noGitMain $noGitProcess 'Git未導入' $false); $noGitText = Get-GitSummary $noGitMain }
        catch { $noGitPanelEnsureError = $_.Exception.Message; $noGitText = Get-GitSummary $noGitMain }
        $trustInventoryAfterAccept = Get-TestTrustInventory $noGitWorkspace
        $trustUiEvidence.details.accept = [ordered]@{
            activation_route = $acceptRoute; decision = $trustAccepted.decision; confirmation = $trustAccepted.confirmation
            confirmation_controls = $trustAccepted.confirmation_controls
            result = $trustAccepted.result; result_contains_trusted_message = $trustAccepted.result_contains_trusted_message
            result_controls = $trustAccepted.result_controls; visible_summary = $noGitText
            trust_store_inventory = $trustInventoryAfterAccept
            panel_visibility_error = $noGitPanelEnsureError
            trust_button = Get-ButtonEvidence $noGitMain '信頼を確認...' 1032
            git_listview_present = (Get-GitList $noGitMain) -ne [IntPtr]::Zero
            listview_required_for_no_git_state = $false
        }
        $noGitPanelSnapshots.Add([pscustomobject]@{ phase = 'after_ui_accept'; text = $noGitText })
        Add-Check 'trust_ui_accept_writes_only_fixture_store' (
            $trustAccepted.decision -eq 'YES' -and $trustAccepted.acknowledgement_semantic_ok -and $trustAccepted.result_contains_trusted_message -and
            $trustInventoryAfterAccept.files_matching_current_workspace_path -ge 1
        ) $trustUiEvidence.details.accept
        $noGitStateStatus = if ($noGitText -match 'Git未導入') { 'PASS' } else { 'BLOCKED' }
        $checks['nogit_state_after_actual_trust_accept'] = [ordered]@{
            status = $noGitStateStatus; details = $trustUiEvidence.details.accept
        }

        $trustUiEvidence.stage = 'revoke'
        $trustRevoked = Complete-TrustRevocation $noGitMain $noGitProcess
        $afterRevoke = Get-GitSummary $noGitMain
        $trustInventoryAfterRevoke = Get-TestTrustInventory $noGitWorkspace
        $trustUiEvidence.details.revoke = [ordered]@{
            command = $trustRevoked.command; result = $trustRevoked.result; result_contains_revoked_message = $trustRevoked.result_contains_revoked_message
            result_controls = $trustRevoked.result_controls; visible_summary = $afterRevoke
            trust_store_inventory = $trustInventoryAfterRevoke; trust_button = Get-ButtonEvidence $noGitMain '信頼を確認...' 1032
            git_listview_present = (Get-GitList $noGitMain) -ne [IntPtr]::Zero
        }
        $noGitPanelSnapshots.Add([pscustomobject]@{ phase = 'after_ui_revoke'; text = $afterRevoke })
        Add-Check 'trust_ui_revoke_removes_fixture_grant' (
            $trustRevoked.acknowledgement_semantic_ok -and $trustRevoked.result_contains_revoked_message -and $trustInventoryAfterRevoke.files_matching_current_workspace_path -eq 0
        ) $trustUiEvidence.details.revoke
        $trustUiEvidence.status = 'PASS'
        $trustUiEvidence.stage = 'complete'
    } catch {
        $trustUiEvidence.status = 'BLOCKED'
        $trustUiEvidence.error = $_.Exception.Message
        $trustUiEvidence.failed_stage = $trustUiEvidence.stage
        $checks['workspace_trust_ui_lane'] = [ordered]@{ status = 'BLOCKED'; details = $trustUiEvidence }
    } finally {
        $trustCleanupError = $null
        if ($noGitProcess) {
            try { Stop-MDLite $noGitProcess $noGitMain }
            catch { $trustCleanupError = $_.Exception.Message }
        }
        if (Test-ProcessAlive $noGitProcessId) {
            try {
                $fallbackTrustProcess = [Diagnostics.Process]::GetProcessById($noGitProcessId)
                try { $fallbackTrustProcess.Kill(); [void]$fallbackTrustProcess.WaitForExit(3000) }
                finally { $fallbackTrustProcess.Dispose() }
            } catch {
                if ($trustCleanupError) { $trustCleanupError += ' | ' }
                $trustCleanupError += $_.Exception.Message
            }
        }
        $trustUiEvidence.process_cleanup = [ordered]@{
            process_id = $noGitProcessId; process_still_running = (Test-ProcessAlive $noGitProcessId)
            error = $trustCleanupError
        }
        $noGitProcess = $null; $noGitMain = [IntPtr]::Zero
        if ($trustUiEvidence.process_cleanup.process_still_running) {
            throw "The Trust fixture process $noGitProcessId could not be closed; refusing to overlap another product window."
        }
    }
    if ($trustUiEvidence.status -eq 'PASS') {
        $checks['workspace_trust_ui_lane'] = [ordered]@{ status = 'PASS'; details = $trustUiEvidence }
    }

    # The main fixture begins without .git, so the product's NoRepository state is observed first.
    New-WorkspaceFixture $gitWorkspace
    $gitDocument = Join-Path $gitWorkspace 'README.md'
    $process = Start-MDLite $gitDocument $false
    $main = Find-MainWindow $process
    $gitProcessId = $process.Id
    $gitMainHandle = $main.ToInt64()
    [uint32]$gitOwnerPid = 0
    [void][MDLiteGitProductNative]::GetWindowThreadProcessId($main, [ref]$gitOwnerPid)
    $gitMainOwnerPid = [int]$gitOwnerPid
    [void][MDLiteGitProductNative]::SetForegroundWindow($main)
    [void][MDLiteGitProductNative]::SetWindowPos($main, [IntPtr]::Zero, 80, 80, 1440, 960, 0x0004)
    $hookResult = [MDLiteGitProductNative]::SendMessage($main, 0x8043, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $noRepoPanelError = $null
    try { [void](Ensure-GitPanelVisible $main $process 'repositoryなし' $false); $noRepoPanelSummary = Get-GitSummary $main }
    catch { $noRepoPanelError = $_.Exception.Message; $noRepoPanelSummary = Get-GitSummary $main }
    $noRepoList = Get-GitList $main
    $noRepoText = $noRepoPanelSummary
    $gitPanelSnapshots.Add([pscustomobject]@{ phase = 'no_repository'; text = $noRepoText })
    $noRepoCheckStatus = if ($hookResult -ne 1) { 'FAIL' } elseif ($noRepoText -and $noRepoText.Contains('repositoryなし')) { 'PASS' } else { 'BLOCKED' }
    $checks['no_repository_state_rendered_in_git_panel'] = [ordered]@{ status = $noRepoCheckStatus; details = [ordered]@{
        hook_wparam = 0; test_only_direct_fixture_grant = $true; hook_result = $hookResult; panel = $noRepoText
        list_view_present = ($noRepoList -ne [IntPtr]::Zero); list_view_required_for_no_repository_state = $false
        panel_visibility_error = $noRepoPanelError
    }}
    # Baseline creation and mixed index/worktree state are fixture setup only; product buttons perform Stage/Unstage/Commit.
    $init = Get-GitOutput $gitWorkspace @('init', '--initial-branch=main')
    if ($init.exit_code -ne 0) { throw "git init failed: $($init.output)" }
    foreach ($pair in @(@('user.name', 'MDLite acceptance'), @('user.email', 'mdlite-acceptance@example.invalid'))) {
        $configured = Get-GitOutput $gitWorkspace @('config', '--local', $pair[0], $pair[1])
        if ($configured.exit_code -ne 0) { throw "git config failed: $($configured.output)" }
    }
    $trackedPath = Join-Path $gitWorkspace 'tracked.md'
    [IO.File]::WriteAllText($trackedPath, 'initial-version', [Text.UTF8Encoding]::new($false))
    $addInitial = Get-GitOutput $gitWorkspace @('add', '-A')
    $commitInitial = Get-GitOutput $gitWorkspace @('commit', '-m', 'initial fixture')
    if ($addInitial.exit_code -ne 0 -or $commitInitial.exit_code -ne 0) { throw "initial fixture commit failed: $($addInitial.output) $($commitInitial.output)" }
    [IO.File]::WriteAllText($trackedPath, 'staged-version', [Text.UTF8Encoding]::new($false))
    $stageSetup = Get-GitOutput $gitWorkspace @('add', '--', 'tracked.md')
    [IO.File]::WriteAllText($trackedPath, 'unstaged-version', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $gitWorkspace 'pending.md'), 'pending-stage-route', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $gitWorkspace 'other.md'), 'remains-untracked', [Text.UTF8Encoding]::new($false))
    if ($stageSetup.exit_code -ne 0) { throw "staged fixture setup failed: $($stageSetup.output)" }
    $racePath = Join-Path $gitWorkspace 'status-race-temporary.md'
    [IO.File]::WriteAllText($racePath, 'status generation probe', [Text.UTF8Encoding]::new($false))
    Update-GitStatus $main
    Start-Sleep -Milliseconds 25
    Remove-Item -LiteralPath $racePath -Force
    Update-GitStatus $main
    $list = Ensure-GitPanelVisible $main $process 'remoteなし' $true
    $readyText = Get-GitSummary $main
    $gitRowsBefore = Get-GitRows $main $process.Id
    $staleRowDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (@($gitRowsBefore | Where-Object { $_.path -eq 'status-race-temporary.md' }).Count -gt 0 -and
        [DateTime]::UtcNow -lt $staleRowDeadline) {
        Start-Sleep -Milliseconds 40
        $gitRowsBefore = Get-GitRows $main $process.Id
    }
    Add-Check 'git_status_second_ui_refresh_discards_stale_file_result' (
        $readyText -and @($gitRowsBefore | Where-Object { $_.path -eq 'status-race-temporary.md' }).Count -eq 0
    ) ([ordered]@{
        first_refresh_started_with_temporary_file = $true; temporary_file_removed_before_second_refresh = $true
        panel = $readyText; rendered_rows_after_latest_refresh = $gitRowsBefore
    })
    $mixedPass = $readyText.Contains('staged 1') -and $readyText.Contains('unstaged 1') -and $readyText.Contains('untracked 2') -and
        (@($gitRowsBefore | Where-Object { $_.path -eq 'pending.md' }).Count -eq 1) -and
        (@($gitRowsBefore | Where-Object { $_.path -eq 'other.md' }).Count -eq 1) -and
        (@($gitRowsBefore | Where-Object { $_.path -eq 'tracked.md' }).Count -eq 1)
    Add-Check 'mixed_status_rows_rendered_without_remote' $mixedPass ([ordered]@{ panel = $readyText; rows = $gitRowsBefore })
    $mixedPanelScreenshot = Save-WindowFrame $main (Join-Path $evidenceRoot "git-panel-mixed-$runId.png")

    $editor = Find-Child $main 102 'RICHEDIT50W'
    if ($editor -eq [IntPtr]::Zero) { throw 'The active source editor was not found.' }
    $pendingIndex = -1
    for ($rowIndex = 0; $rowIndex -lt $gitRowsBefore.Count; $rowIndex++) {
        if ($gitRowsBefore[$rowIndex].path -eq 'pending.md') { $pendingIndex = $rowIndex; break }
    }
    if ($pendingIndex -lt 0) { throw 'The pending fixture row was not rendered.' }
    $list = Get-GitList $main
    if (-not [MDLiteGitProductNative]::SelectListViewRow($list, $process.Id, $pendingIndex)) { throw 'Could not select the pending Git row.' }
    $stageButton = Find-ProductButton $main 'Stage'
    $diskHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $documentStates.before_stage = Get-DocumentState $main
    Click-ProductButton $stageButton 'Git Stage'
    $activeDeadline = [DateTime]::UtcNow.AddSeconds(3)
    while ([DateTime]::UtcNow -lt $activeDeadline -and
        [MDLiteGitProductNative]::SendMessage($main, 0x8044, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 0) { Start-Sleep -Milliseconds 30 }
    $stageActive = [MDLiteGitProductNative]::SendMessage($main, 0x8044, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -eq 1
    [void][MDLiteGitProductNative]::SendMessage($editor, 0x00B1, [IntPtr](-1), [IntPtr](-1))
    [void][MDLiteGitProductNative]::SendMessage($editor, 0x0102, [IntPtr]0x7E, [IntPtr]::Zero) # WM_CHAR '~'
    $documentStates.during_stage = Get-DocumentState $main
    $selectionPacked = [MDLiteGitProductNative]::SendMessage($editor, 0x00B0, [IntPtr]::Zero, [IntPtr]::Zero).ToInt64() # EM_GETSEL
    $documentStates.caret_after_input = [ordered]@{ packed = $selectionPacked; start = ($selectionPacked -band 0xFFFF); end = (($selectionPacked -shr 16) -band 0xFFFF) }
    $caretCollapsed = $documentStates.caret_after_input.start -eq $documentStates.caret_after_input.end
    $recoveryRoot = Join-Path $gitWorkspace '.mdlite\.state\recovery'
    $recoveryDeadline = [DateTime]::UtcNow.AddSeconds(7)
    $recoverySnapshotPath = $null
    $recoverySnapshotText = ''
    do {
        $snapshots = @(Get-ChildItem -LiteralPath $recoveryRoot -Filter '*.md' -File -ErrorAction SilentlyContinue)
        foreach ($snapshot in $snapshots) {
            $text = [IO.File]::ReadAllText($snapshot.FullName)
            if ($text.Contains('~')) { $recoverySnapshotPath = $snapshot.FullName; $recoverySnapshotText = $text; break }
        }
        if ($recoverySnapshotPath) { break }
        Start-Sleep -Milliseconds 60
    } while ([DateTime]::UtcNow -lt $recoveryDeadline)
    $recoveryEvidence = [ordered]@{
        path = $recoverySnapshotPath; exists = [bool]$recoverySnapshotPath
        source_matches_fixture = [bool]($recoverySnapshotText.Contains($gitDocument))
        contains_concurrent_input = $recoverySnapshotText.Contains('~')
        sha256 = if ($recoverySnapshotPath) { (Get-FileHash -Algorithm SHA256 -LiteralPath $recoverySnapshotPath).Hash.ToLowerInvariant() } else { '' }
    }
    $documentStates.disk_hash_during_stage = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $waitStage = Wait-GitActionIdle $main 15000
    $stageRows = Get-GitOutput $gitWorkspace @('diff', '--cached', '--name-only')
    $gitRowsAfterStage = Get-GitRows $main $process.Id
    $stageRefreshSent = $false
    $stageRefreshError = $null
    $stagedRowDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while (@($gitRowsAfterStage | Where-Object { $_.path -eq 'pending.md' }).Count -eq 0 -and
        [DateTime]::UtcNow -lt $stagedRowDeadline) {
        if (-not $stageRefreshSent) {
            try { Update-GitStatus $main; $stageRefreshSent = $true }
            catch { $stageRefreshError = $_.Exception.Message; $stageRefreshSent = $true }
        }
        Start-Sleep -Milliseconds 45
        $gitRowsAfterStage = Get-GitRows $main $process.Id
    }
    $documentStates.after_stage = Get-DocumentState $main
    $documentStates.disk_hash_after_stage = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $stageConcurrentPass = $stageActive -and $waitStage -and $documentStates.during_stage.dirty -and
        $documentStates.during_stage.source_length -eq ($documentStates.before_stage.source_length + 1) -and
        $documentStates.during_stage.undo_count -gt $documentStates.before_stage.undo_count -and
        $documentStates.during_stage.revision -gt $documentStates.before_stage.revision -and
        $documentStates.during_stage.final_code_unit -eq 0x7E -and
        $caretCollapsed -and
        $documentStates.disk_hash_during_stage -ceq $diskHashBefore -and
        $documentStates.disk_hash_after_stage -ceq $diskHashBefore -and
        $recoveryEvidence.exists -and $recoveryEvidence.source_matches_fixture -and $recoveryEvidence.contains_concurrent_input -and
        $stageRows.exit_code -eq 0 -and $stageRows.output -match '(?m)^pending\.md$'
    Add-Check 'stage_ui_keeps_editing_caret_history_and_recovery_live' $stageConcurrentPass ([ordered]@{
        active_when_input_sent = $stageActive; action_finished = $waitStage; document_states = $documentStates
        caret_collapsed_after_input = $caretCollapsed
        saved_revision = $documentStates.during_stage.saved_revision
        recovery = $recoveryEvidence; disk_sha256_before = $diskHashBefore; disk_sha256_during = $documentStates.disk_hash_during_stage
        cached_paths = $stageRows.output; rows_after_stage = $gitRowsAfterStage
        panel_summary_after_stage = (Get-GitSummary $main); refresh_attempted = $stageRefreshSent; refresh_error = $stageRefreshError
        git_list_hwnd_after_stage = (Get-GitList $main).ToInt64()
    })

    $pendingIndex = -1
    for ($rowIndex = 0; $rowIndex -lt $gitRowsAfterStage.Count; $rowIndex++) {
        if ($gitRowsAfterStage[$rowIndex].path -eq 'pending.md') { $pendingIndex = $rowIndex; break }
    }
    $unstageStatus = 'BLOCKED'
    $unstageError = $null
    $waitUnstage = $false
    if ($pendingIndex -ge 0) {
        try {
            if (-not [MDLiteGitProductNative]::SelectListViewRow((Get-GitList $main), $process.Id, $pendingIndex)) {
                throw 'Could not select the staged pending row.'
            }
            Click-ProductButton (Find-ProductButton $main 'Unstage') 'Git Unstage'
            $waitUnstage = Wait-GitActionIdle $main 15000
            $cachedAfterUnstage = Get-GitOutput $gitWorkspace @('diff', '--cached', '--name-only')
            $gitRowsAfterUnstage = Get-GitRows $main $process.Id
            $unstageStatus = if ($waitUnstage -and $cachedAfterUnstage.exit_code -eq 0 -and
                $cachedAfterUnstage.output -notmatch '(?m)^pending\.md$' -and $cachedAfterUnstage.output -match '(?m)^tracked\.md$') { 'PASS' } else { 'FAIL' }
        } catch {
            $unstageError = $_.Exception.Message
            $cachedAfterUnstage = Get-GitOutput $gitWorkspace @('diff', '--cached', '--name-only')
            $gitRowsAfterUnstage = Get-GitRows $main $process.Id
        }
    } else {
        $unstageError = 'Staged pending.md was absent from rendered rows after a bounded status refresh; unstage UI was not activated.'
        $cachedAfterUnstage = Get-GitOutput $gitWorkspace @('diff', '--cached', '--name-only')
        $gitRowsAfterUnstage = $gitRowsAfterStage
    }
    $checks['product_unstage_uses_only_selected_path'] = [ordered]@{
        status = $unstageStatus
        details = [ordered]@{
            action_finished = $waitUnstage; selected_row_index = $pendingIndex; error = $unstageError
            cache_after = $cachedAfterUnstage.output; rows_after_unstage = $gitRowsAfterUnstage
            rows_after_stage = $gitRowsAfterStage; panel_summary = (Get-GitSummary $main)
            stage_list_hwnd = (Get-GitList $main).ToInt64(); stage_refresh_attempted = $stageRefreshSent
        }
    }

    $commitMessage = "product-route-$runId"
    $headBeforeCommit = Get-GitOutput $gitWorkspace @('rev-parse', 'HEAD')
    if ($headBeforeCommit.exit_code -ne 0) { throw 'Could not read fixture HEAD before product Commit.' }
    $cachedBeforeCommit = Get-GitOutput $gitWorkspace @('diff', '--cached', '--name-only')
    $cachedPathsBeforeCommit = @($cachedBeforeCommit.output -split "`n" | Where-Object { $_ })
    $pendingStagedAtCommit = @($cachedPathsBeforeCommit | Where-Object { $_ -ceq 'pending.md' }).Count -gt 0
    $expectedCommitSummary = 'staged {0}' -f $cachedPathsBeforeCommit.Count
    $commitReadinessRefreshAttempted = $false
    $commitReadinessRefreshError = $null
    $commitReadySummary = Get-GitSummary $main
    $commitReadyDeadline = [DateTime]::UtcNow.AddSeconds(8)
    while ($cachedPathsBeforeCommit.Count -gt 0 -and
        (-not $commitReadySummary.Contains($expectedCommitSummary) -or $commitReadySummary.Contains('操作中')) -and
        [DateTime]::UtcNow -lt $commitReadyDeadline) {
        if (-not $commitReadinessRefreshAttempted) {
            try { Update-GitStatus $main; $commitReadinessRefreshAttempted = $true }
            catch { $commitReadinessRefreshError = $_.Exception.Message; $commitReadinessRefreshAttempted = $true }
        }
        Start-Sleep -Milliseconds 45
        $commitReadySummary = Get-GitSummary $main
    }
    $commitButtonBeforeAction = Get-ButtonEvidence $main 'Commit'
    $commitReady = $cachedPathsBeforeCommit.Count -gt 0 -and $commitReadySummary.Contains($expectedCommitSummary) -and
        -not $commitReadySummary.Contains('操作中') -and $commitButtonBeforeAction.enabled -and $commitButtonBeforeAction.visible
    $commitTextSetResult = 0
    $commitTextReadback = ''
    $commitResultText = ''
    $statusAfterCommit = Get-GitOutput $gitWorkspace @('status', '--short')
    try {
    if (-not $commitReady) {
        throw "Commit action was not enabled after a bounded status refresh (summary='$commitReadySummary'; expected='$expectedCommitSummary'; button=$($commitButtonBeforeAction | ConvertTo-Json -Compress))."
    }
    $commitEdit = [IntPtr]::Zero
    foreach ($candidate in [MDLiteGitProductNative]::Children($main)) {
        if ([MDLiteGitProductNative]::WindowClass($candidate) -eq 'Edit' -and
            [MDLiteGitProductNative]::IsWindowVisible($candidate) -and
            [string]::IsNullOrEmpty([MDLiteGitProductNative]::WindowText($candidate))) { $commitEdit = $candidate; break }
    }
    if ($commitEdit -eq [IntPtr]::Zero) { throw 'Git commit message edit control was not found.' }
    $commitTextSetResult = [MDLiteGitProductNative]::SendMessageSetText($commitEdit, 0x000C, [IntPtr]::Zero, $commitMessage).ToInt32()
    $commitTextReadback = [MDLiteGitProductNative]::WindowText($commitEdit)
    if ($commitTextSetResult -eq 0 -or $commitTextReadback -cne $commitMessage) { throw 'Commit message field did not accept the fixture text.' }
        $commitDocumentBefore = Get-DocumentState $main
        $commitSourceBefore = Get-DocumentSource $main
        Click-ProductButton (Find-ProductButton $main 'Commit') 'Git Commit'
        $commitTaskDialog = [IntPtr]::Zero
        $commitProgressEvidence = $null
        $commitHead = $null
        $commitSubject = $null
        $commitWaitDeadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            $progress = [MDLiteGitProductNative]::FindTopLevelWindow($process.Id, 'ステージ済み変更をコミット')
            if ($progress -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($progress)) {
                $commitTaskDialog = $progress
                if (-not $commitProgressEvidence) { $commitProgressEvidence = Get-DialogControlsEvidence $progress }
            }
            $commitHead = Get-GitOutput $gitWorkspace @('rev-parse', 'HEAD')
            $commitSubject = Get-GitOutput $gitWorkspace @('log', '-1', '--format=%s')
            $commitBusy = [MDLiteGitProductNative]::SendMessage($main, 0x8044, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
            if ($commitHead.exit_code -eq 0 -and $commitHead.output -cne $headBeforeCommit.output -and
                $commitSubject.exit_code -eq 0 -and $commitSubject.output -ceq $commitMessage -and
                $commitBusy -eq 0 -and [MDLiteGitProductNative]::IsWindowEnabled($main)) { break }
            Start-Sleep -Milliseconds 40
        } while ([DateTime]::UtcNow -lt $commitWaitDeadline)
        $commitParent = Get-GitOutput $gitWorkspace @('rev-parse', 'HEAD^')
        $commitResultText = 'Commit completion observed through changed HEAD, exact subject, cleared busy state and re-enabled main window; silent result popup is not required.'
    $committedTracked = Get-GitOutput $gitWorkspace @('show', 'HEAD:tracked.md')
    $workingTracked = [IO.File]::ReadAllText($trackedPath)
    $headFiles = Get-GitOutput $gitWorkspace @('ls-tree', '--name-only', 'HEAD')
    $headPaths = @($headFiles.output -split "`n" | Where-Object { $_ })
    $statusAfterCommit = Get-GitOutput $gitWorkspace @('status', '--short')
    $commitDocumentAfter = Get-DocumentState $main
    $commitSourceAfter = Get-DocumentSource $main
    $documentStates.disk_hash_after_commit = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $headContainsPendingAsStaged = @($headPaths | Where-Object { $_ -ceq 'pending.md' }).Count -gt 0
    $pendingRemainsInWorktreeStatus = $statusAfterCommit.output -match '(?m)^\?\? pending\.md$'
    $allCachedPathsCommitted = @($cachedPathsBeforeCommit | Where-Object { $_ -notin $headPaths }).Count -eq 0
    Add-Check 'product_commit_consumes_index_without_worktree_pathspec' ($commitHead.exit_code -eq 0 -and $commitHead.output -cne $headBeforeCommit.output -and
        $commitSubject.exit_code -eq 0 -and $commitSubject.output -ceq $commitMessage -and
        $commitParent.exit_code -eq 0 -and $commitParent.output -ceq $headBeforeCommit.output -and $commitBusy -eq 0 -and
        [MDLiteGitProductNative]::IsWindowEnabled($main) -and $committedTracked.exit_code -eq 0 -and
        $committedTracked.output -eq 'staged-version' -and $workingTracked -eq 'unstaged-version' -and
        $headFiles.output -match '(?m)^tracked\.md$' -and $headContainsPendingAsStaged -eq $pendingStagedAtCommit -and
        $allCachedPathsCommitted -and
        $commitDocumentBefore.dirty -and $commitDocumentAfter.dirty -and $commitSourceBefore -ceq $commitSourceAfter -and
        ($commitDocumentBefore | ConvertTo-Json -Compress) -ceq ($commitDocumentAfter | ConvertTo-Json -Compress) -and
        $headFiles.output -notmatch '(?m)^other\.md$' -and $statusAfterCommit.exit_code -eq 0 -and
        $documentStates.disk_hash_after_commit -ceq $diskHashBefore -and
        $statusAfterCommit.output.Contains('tracked.md') -and $statusAfterCommit.output.Contains('other.md') -and
        $pendingRemainsInWorktreeStatus -eq (-not $pendingStagedAtCommit)) ([ordered]@{
        commit_message = $commitMessage; commit_result = $commitResultText; committed_tracked_version = $committedTracked.output
        head_before = $headBeforeCommit.output; head_after = $commitHead.output; parent_after = $commitParent.output; actual_subject = $commitSubject.output; busy_after = $commitBusy
        source_before = $commitSourceBefore; source_after = $commitSourceAfter; document_before = $commitDocumentBefore; document_after = $commitDocumentAfter
        commit_input_set_lresult = $commitTextSetResult; commit_input_readback = $commitTextReadback
        cached_paths_before_product_commit = $cachedPathsBeforeCommit; pending_staged_at_commit = $pendingStagedAtCommit
        readiness_summary = $commitReadySummary; expected_summary_token = $expectedCommitSummary
        commit_button_before_action = $commitButtonBeforeAction; status_refresh_attempted = $commitReadinessRefreshAttempted
        status_refresh_error = $commitReadinessRefreshError; progress_dialog = $commitProgressEvidence
        unsaved_document_disk_bytes_preserved = ($documentStates.disk_hash_after_commit -ceq $diskHashBefore)
        worktree_tracked_version = $workingTracked; HEAD_files = $headFiles.output; remaining_status = $statusAfterCommit.output
    })

    } catch {
        $commitLaneError = $_.Exception.Message
        try { $statusAfterCommit = Get-GitOutput $gitWorkspace @('status', '--short') } catch {}
        $commitModal = [MDLiteGitProductNative]::FindTopLevelWindow($process.Id, 'ステージ済み変更をコミット')
        $commitModalHandle = $commitModal.ToInt64()
        if ($commitModal -ne [IntPtr]::Zero) {
            $okButton = Find-Child $commitModal 1 'Button'
            $cancelButton = Find-Child $commitModal 2 'Button'
            $dismissButton = if ($okButton -ne [IntPtr]::Zero) { $okButton } else { $cancelButton }
            if ($dismissButton -ne [IntPtr]::Zero) {
                try { Click-ProductButton $dismissButton 'Commit modal cleanup' } catch {}
                [void](Wait-WindowGone $process.Id 'ステージ済み変更をコミット' 2000)
            }
        }
        $checks['product_commit_consumes_index_without_worktree_pathspec'] = [ordered]@{
            status = 'BLOCKED'
            details = [ordered]@{
                error = $commitLaneError; process_id = $process.Id; main_window_hwnd = $main.ToInt64()
                commit_modal_hwnd_at_failure = $commitModalHandle; commit_message = $commitMessage
                commit_text_set_lresult = $commitTextSetResult; commit_text_readback = $commitTextReadback
                cached_paths_before_product_commit = $cachedBeforeCommit.output; pending_staged_at_commit = $pendingStagedAtCommit
                readiness_summary = $commitReadySummary; expected_summary_token = $expectedCommitSummary
                commit_button_before_action = $commitButtonBeforeAction; status_refresh_attempted = $commitReadinessRefreshAttempted
                status_refresh_error = $commitReadinessRefreshError; status_after = $statusAfterCommit.output
            }
        }
    }

    # Exercise the custom command binding and Escape from editor, Explorer, Calendar, and Git focus.
    $headerImage = Join-Path $evidenceRoot "git-panel-header-hint-$runId.png"
    $headerScreenshot = Save-WindowFrame $main $headerImage
    $focusTargets = [Collections.Generic.List[object]]::new()
    $focusTargets.Add([pscustomobject]@{ name = 'editor'; hwnd = (Find-Child $main 102 'RICHEDIT50W') })
    $focusTargets.Add([pscustomobject]@{ name = 'git'; hwnd = (Get-GitList $main) })
    foreach ($focusSpec in @(
        [pscustomobject]@{ name = 'explorer'; id = 100; class_name = 'SysTreeView32'; command = '表示: Explorerを折り畳む/表示' },
        [pscustomobject]@{ name = 'calendar'; id = 118; class_name = 'MDLite.CalendarView'; command = '表示: Calendarを表示/非表示' }
    )) {
        try {
            $focusTargets.Add([pscustomobject]@{
                name = $focusSpec.name
                hwnd = (Ensure-FocusTargetVisible $main $process $focusSpec.id $focusSpec.class_name $focusSpec.command)
            })
        } catch {
            $checks["palette_focus_target_$($focusSpec.name)"] = [ordered]@{
                status = 'BLOCKED'; details = [ordered]@{ command = $focusSpec.command; error = $_.Exception.Message }
            }
        }
    }
    foreach ($target in $focusTargets) {
        try {
            if ($target.hwnd -ne [IntPtr]::Zero -and [MDLiteGitProductNative]::IsWindowVisible($target.hwnd)) {
                Invoke-PaletteEscapeFromFocus $main $process $target.hwnd $target.name
            } else {
                $checks["command_palette_custom_binding_escape_$($target.name)"] = [ordered]@{
                    status = 'BLOCKED'; details = [ordered]@{ target_visible = $false; hwnd = $target.hwnd.ToInt64() }
                }
            }
        } catch {
            $checks["command_palette_custom_binding_escape_$($target.name)"] = [ordered]@{
                status = 'BLOCKED'; details = [ordered]@{ target_visible = $true; hwnd = $target.hwnd.ToInt64(); error = $_.Exception.Message }
            }
        }
    }
    $editorCountBefore = @( [MDLiteGitProductNative]::Children($main) | Where-Object {
        [MDLiteGitProductNative]::WindowClass($_) -eq 'RICHEDIT50W'
    }).Count
    $editorCountAfter = $editorCountBefore
    try {
        $gitList = Get-GitList $main
        if ($gitList -eq [IntPtr]::Zero) { throw 'The Git ListView was not visible for the custom binding test.' }
        $newBindingFocused = [MDLiteGitProductNative]::FocusControl($main, $gitList)
        $newBindingKey = [MDLiteGitProductNative]::SendKeyChord([System.UInt16[]]@(0x11, 0x10, 0x4E))
        $newBindingDeadline = [DateTime]::UtcNow.AddSeconds(3)
        do {
            $editorCountAfter = @( [MDLiteGitProductNative]::Children($main) | Where-Object {
                [MDLiteGitProductNative]::WindowClass($_) -eq 'RICHEDIT50W'
            }).Count
            if ($editorCountAfter -gt $editorCountBefore) { break }
            Start-Sleep -Milliseconds 40
        } while ([DateTime]::UtcNow -lt $newBindingDeadline)
        Add-Check 'custom_file_new_binding_runs_command_from_git_focus' ($newBindingFocused -and $newBindingKey -and $editorCountAfter -gt $editorCountBefore) ([ordered]@{
            focused_git_list = $newBindingFocused; sendinput = $newBindingKey; editor_count_before = $editorCountBefore; editor_count_after = $editorCountAfter
            config_line = 'bind.file.new = "Ctrl+Shift+N"'
        })
        if ($editorCountAfter -gt $editorCountBefore) { [void](Invoke-ProductCommandFromPalette $main $process 'ファイル: タブを閉じる') }
    } catch {
        $checks['custom_file_new_binding_runs_command_from_git_focus'] = [ordered]@{
            status = 'BLOCKED'; details = [ordered]@{ focused_git_list_hwnd = if ($gitList) { $gitList.ToInt64() } else { 0L }; error = $_.Exception.Message }
        }
    }

    # Fetch uses a separate fresh non-silent process; main fixture scalar/isolation hooks stay enabled.
    $freshFetch = Invoke-FreshLoopbackFetch
    $checks['loopback_fetch_cancel_uses_product_taskdialog_and_preserves_index'] = [ordered]@{
        status = if ($freshFetch.pass) { 'PASS' } else { 'BLOCKED' }; details = $freshFetch.evidence
    }
    $status = 'PASS_PRODUCT_UI_SYNTHETIC_NATIVE_ROUTE'
} catch {
    $errorMessage = $_.Exception.Message
    $status = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
} finally {
    if ($stall) { $stall.Dispose(); $stall = $null }
    if ($noGitProcess) { Stop-MDLite $noGitProcess $noGitMain }
    if ($process) { Stop-MDLite $process $main }
    if ($null -eq $previousSilentSetting) { Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_SILENT = $previousSilentSetting }
    if ($null -eq $previousGitDelaySetting) { Remove-Item Env:MDLITE_TEST_GIT_ACTION_DELAY_MS -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_GIT_ACTION_DELAY_MS = $previousGitDelaySetting }
    if ($null -eq $previousTerminalPromptSetting) { Remove-Item Env:GIT_TERMINAL_PROMPT -ErrorAction SilentlyContinue } else { $env:GIT_TERMINAL_PROMPT = $previousTerminalPromptSetting }
}

$sourceFingerprintAfter = Get-SourceFingerprint
$executableHashAfter = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
$sourceStable = $sourceFingerprintBefore -and $sourceFingerprintAfter -and $sourceFingerprintBefore.sha256 -ceq $sourceFingerprintAfter.sha256 -and
    $executableHashBefore -ceq $executableHashAfter
Add-Check 'source_and_executable_unchanged_during_ui_run' ([bool]$sourceStable) ([ordered]@{
    source_before = if ($sourceFingerprintBefore) { $sourceFingerprintBefore.sha256 } else { '' }
    source_after = if ($sourceFingerprintAfter) { $sourceFingerprintAfter.sha256 } else { '' }
    executable_before = $executableHashBefore; executable_after = $executableHashAfter
})
$failed = @($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0
$blocked = @($checks.Values | Where-Object { $_.status -eq 'BLOCKED' }).Count -gt 0
$appProcessEvidence = [ordered]@{
    no_git = [ordered]@{
        process_id = $noGitProcessId; main_window_hwnd = ('0x{0:X}' -f $noGitMainHandle); owner_pid = $noGitMainOwnerPid
        window_owner_matches_process = ($noGitProcessId -gt 0 -and $noGitMainOwnerPid -eq $noGitProcessId)
        process_still_running = (Test-ProcessAlive $noGitProcessId)
        main_window_still_alive = ($noGitMainHandle -gt 0 -and [MDLiteGitProductNative]::IsWindow([IntPtr]$noGitMainHandle))
    }
    git_fixture = [ordered]@{
        process_id = $gitProcessId; main_window_hwnd = ('0x{0:X}' -f $gitMainHandle); owner_pid = $gitMainOwnerPid
        window_owner_matches_process = ($gitProcessId -gt 0 -and $gitMainOwnerPid -eq $gitProcessId)
        process_still_running = (Test-ProcessAlive $gitProcessId)
        main_window_still_alive = ($gitMainHandle -gt 0 -and [MDLiteGitProductNative]::IsWindow([IntPtr]$gitMainHandle))
    }
}
$result = [ordered]@{
    status = if ($failed -and $status -like 'PASS*') { 'FAIL_PRODUCT_UI_ROUTE' } elseif ($blocked -and $status -like 'PASS*') { 'PASS_PRODUCT_UI_WITH_BLOCKED_SUBCASES' } else { $status }
    pass = ($status -like 'PASS*' -and -not $failed -and -not $blocked)
    preset = $Preset
    executable = $executable
    executable_sha256_before = $executableHashBefore
    executable_sha256_after = $executableHashAfter
    build_receipt_path = $buildReceiptPath
    build_receipt = $buildReceipt
    source_fingerprint_before = $sourceFingerprintBefore
    source_fingerprint_after = $sourceFingerprintAfter
    fixture_root = $runRoot
    product_processes = $appProcessEvidence
    trust_ui_evidence = $trustUiEvidence
    trust_dialog_screenshots = @($trustDialogFrames)
    no_git_panel_snapshots = @($noGitPanelSnapshots)
    git_panel_snapshots = @($gitPanelSnapshots)
    app_git_discovery_ceiling = $runRoot
    mixed_rows = $gitRowsBefore
    rows_after_stage = $gitRowsAfterStage
    rows_after_unstage = $gitRowsAfterUnstage
    document_states = $documentStates
    recovery = $recoveryEvidence
    palette_runs = @($paletteRuns)
    checks = $checks
    input_method = 'Product buttons and command palette use synthetic Win32 messages. Custom accelerator and Escape use OS SendInput; this is synthetic OS queue input, not physical keyboard input.'
    evidence_boundary = $evidenceBoundary
    screenshots = @($mixedPanelScreenshot, $paletteScreenshot, $headerScreenshot)
    local_http_fetch_fixture = '127.0.0.1 ephemeral TcpListener accepts one connection and sends no response; no external network or credentials.'
    trust_fixture_route = 'noGit uses fixture-store mode1 and main Git uses isolated mode0; fresh non-silent Fetch uses ordinary Trust grant/revoke with real-store inventory comparison and no test hooks.'
    error = $errorMessage
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
}
$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) { [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null }
$result | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $OutputPath -Encoding utf8
$result | ConvertTo-Json -Depth 10

$canonicalRunRoot = [IO.Path]::GetFullPath($runRoot)
$canonicalEvidenceRoot = [IO.Path]::GetFullPath($evidenceRoot).TrimEnd('\') + '\'
if (-not $result.pass -and (Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and
    [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
    $canonicalRunRoot.StartsWith($canonicalEvidenceRoot, [StringComparison]::OrdinalIgnoreCase)) {
    # Retain a failed isolated fixture for diagnosis; successful fixtures are removed below.
} elseif ((Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and
    [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
    $canonicalRunRoot.StartsWith($canonicalEvidenceRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $canonicalRunRoot -Recurse -Force
}
if (-not $result.pass) { exit 1 }
