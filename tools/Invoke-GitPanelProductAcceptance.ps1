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

function Wait-WindowTitle([int]$ProcessId, [string]$Title, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $window = [MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, $Title)
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native dialog did not appear: $Title"
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
    $staticText = @($items | Where-Object { $_.class_name -eq 'Static' -and $_.wm_gettext })
    $buttons = @($items | Where-Object { $_.class_name -eq 'Button' })
    return [ordered]@{
        dialog_hwnd = $Window.ToInt64(); dialog_class = [MDLiteGitProductNative]::WindowClass($Window)
        dialog_caption = [MDLiteGitProductNative]::WindowText($Window)
        static_body_wm_gettext = @($staticText | ForEach-Object { $_.wm_gettext }) -join "`n"
        buttons = $buttons; controls = $items
    }
}

function Wait-WindowGone([int]$ProcessId, [string]$Title, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        if ([MDLiteGitProductNative]::FindTopLevelWindow($ProcessId, $Title) -eq [IntPtr]::Zero) { return $true }
        Start-Sleep -Milliseconds 35
    } while ([DateTime]::UtcNow -lt $deadline)
    return $false
}

function Wait-GitSummary([IntPtr]$Window, [string]$Needle, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
            if ([MDLiteGitProductNative]::WindowClass($child) -ne 'Static') { continue }
            if (-not [MDLiteGitProductNative]::IsWindowVisible($child)) { continue }
            $text = [MDLiteGitProductNative]::WindowText($child)
            if ($text.StartsWith('Git') -and $text.Contains($Needle)) { return $text }
        }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    return ''
}

function Get-GitSummary([IntPtr]$Window) {
    foreach ($child in [MDLiteGitProductNative]::Children($Window)) {
        if ([MDLiteGitProductNative]::WindowClass($child) -ne 'Static') { continue }
        if (-not [MDLiteGitProductNative]::IsWindowVisible($child)) { continue }
        $text = [MDLiteGitProductNative]::WindowText($child)
        if ($text.StartsWith('Git')) { return $text }
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

function Find-ProductButton([IntPtr]$Window, [string]$Text) {
    return Find-ChildByText $Window 'Button' $Text
}

function Get-ButtonEvidence([IntPtr]$Window, [string]$Text) {
    $button = Find-ProductButton $Window $Text
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
    $control = Get-ButtonEvidence $Window '信頼する'
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

function Complete-TrustPrompt([int]$ProcessId, [bool]$Allow) {
    $dialog = Wait-WindowTitle $ProcessId 'Workspace Trust'
    $dialogEvidence = Get-DialogControlsEvidence $dialog
    $promptKind = if ($Allow) { 'accept' } else { 'deny' }
    $dialogEvidence['screenshot'] = Save-WindowFrame $dialog (Join-Path $evidenceRoot "trust-prompt-$promptKind-$runId.png")
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
    if (-not (Wait-WindowGone $ProcessId 'Workspace Trust')) { throw 'Workspace Trust confirmation stayed open.' }
    if (-not $Allow) {
        return [pscustomobject]@{
            decision = 'NO'; decision_button_id = $decisionId; decision_button = $decisionCaption
            confirmation = $body; confirmation_controls = $dialogEvidence; result = ''; result_controls = $null
        }
    }

    $receipt = Wait-WindowTitle $ProcessId 'Workspace Trust'
    $receiptEvidence = Get-DialogControlsEvidence $receipt
    $receiptEvidence['screenshot'] = Save-WindowFrame $receipt (Join-Path $evidenceRoot "trust-result-accept-$runId.png")
    $script:trustDialogFrames.Add($receiptEvidence.screenshot)
    $receiptText = $receiptEvidence.static_body_wm_gettext
    $ackButton = Get-DialogButton $receipt 1
    $ackCaption = [MDLiteGitProductNative]::WindowText($ackButton)
    Click-ProductButton $ackButton 'Workspace Trust acknowledgement'
    if (-not (Wait-WindowGone $ProcessId 'Workspace Trust')) { throw 'Workspace Trust result dialog stayed open.' }
    return [pscustomobject]@{
        decision = 'YES'; decision_button_id = $decisionId; decision_button = $decisionCaption
        confirmation = $body; confirmation_controls = $dialogEvidence
        result = $receiptText; result_contains_trusted_message = $receiptText.Contains('信頼しました')
        result_controls = $receiptEvidence; acknowledgement_button_id = 1; acknowledgement_button = $ackCaption
    }
}

function Complete-TrustRevocation([IntPtr]$Window, [Diagnostics.Process]$Target) {
    $command = Invoke-ProductCommandFromPalette $Window $Target 'Workspace: 信頼を解除'
    $dialog = Wait-WindowTitle $Target.Id 'Workspace Trust'
    $dialogEvidence = Get-DialogControlsEvidence $dialog
    $body = $dialogEvidence.static_body_wm_gettext
    $ackButton = Get-DialogButton $dialog 1
    $ackCaption = [MDLiteGitProductNative]::WindowText($ackButton)
    Click-ProductButton $ackButton 'Workspace Trust revocation acknowledgement'
    if (-not (Wait-WindowGone $Target.Id 'Workspace Trust')) { throw 'Workspace Trust revoke result stayed open.' }
    return [pscustomobject]@{
        command = $command; result = $body; result_contains_revoked_message = $body.Contains('信頼を解除しました')
        result_controls = $dialogEvidence; acknowledgement_button_id = 1; acknowledgement_button = $ackCaption
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

function Start-MDLite([string]$DocumentPath, [bool]$WithoutGit) {
    $oldPath = $env:PATH
    $oldHttpProxy = $env:HTTP_PROXY
    $oldHttpsProxy = $env:HTTPS_PROXY
    $oldAllProxy = $env:ALL_PROXY
    $oldNoProxy = $env:NO_PROXY
    $oldGitCeiling = $env:GIT_CEILING_DIRECTORIES
    try {
        if ($WithoutGit) { $env:PATH = $emptyPath }
        $env:HTTP_PROXY = ''
        $env:HTTPS_PROXY = ''
        $env:ALL_PROXY = ''
        $env:NO_PROXY = '127.0.0.1,localhost'
        $env:GIT_CEILING_DIRECTORIES = $runRoot
        return Start-Process -FilePath $executable -ArgumentList @($DocumentPath) -WorkingDirectory $repoRoot -PassThru
    } finally {
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
                                [string]$ExpectedSummary, [bool]$RequireList = $false) {
    $summary = Get-GitSummary $Window
    if (-not $summary) {
        [void](Invoke-ProductCommandFromPalette $Window $Target '表示: Gitを表示/非表示')
        $summary = Wait-GitSummary $Window $ExpectedSummary 5000
    } elseif (-not $summary.Contains($ExpectedSummary)) {
        $summary = Wait-GitSummary $Window $ExpectedSummary 3000
    }
    if (-not $summary -or -not $summary.Contains($ExpectedSummary)) {
        throw "Visible Git panel summary did not match '$ExpectedSummary': $summary"
    }
    $list = Get-GitList $Window
    if ($RequireList -and $list -eq [IntPtr]::Zero) {
        throw "Git panel ListView is absent in required ready state '$ExpectedSummary'."
    }
    return $list
}

function Update-GitStatus([IntPtr]$Window) {
    $refresh = Find-ProductButton $Window '更新'
    Click-ProductButton $refresh 'Git status refresh'
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
        $initialTrustButton = Get-ButtonEvidence $noGitMain '信頼する'
        $initialTrustSummary = Get-GitSummary $noGitMain
        $initialTrustList = Get-GitList $noGitMain
        $trustUiEvidence.details.mode1_store_selection = [ordered]@{
            hook_wparam = 1; hook_result = $trustStoreHook; trust_store_inventory = $trustInventoryBefore
            current_rendered_trust_state = if ($initialTrustSummary -match 'Workspace未信頼') { 'UNTRUSTED' } elseif ($initialTrustButton.visible) { 'UNTRUSTED' } else { 'NOT_OBSERVED' }
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
            trust_button = Get-ButtonEvidence $noGitMain '信頼する'
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
            trust_button = Get-ButtonEvidence $noGitMain '信頼する'
            git_listview_present = (Get-GitList $noGitMain) -ne [IntPtr]::Zero
            listview_required_for_no_git_state = $false
        }
        $noGitPanelSnapshots.Add([pscustomobject]@{ phase = 'after_ui_accept'; text = $noGitText })
        Add-Check 'trust_ui_accept_writes_only_fixture_store' (
            $trustAccepted.decision -eq 'YES' -and $trustAccepted.acknowledgement_button_id -eq 1 -and
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
            trust_store_inventory = $trustInventoryAfterRevoke; trust_button = Get-ButtonEvidence $noGitMain '信頼する'
            git_listview_present = (Get-GitList $noGitMain) -ne [IntPtr]::Zero
        }
        $noGitPanelSnapshots.Add([pscustomobject]@{ phase = 'after_ui_revoke'; text = $afterRevoke })
        Add-Check 'trust_ui_revoke_removes_fixture_grant' (
            $trustRevoked.acknowledgement_button_id -eq 1 -and $trustInventoryAfterRevoke.files_matching_current_workspace_path -eq 0
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
        Click-ProductButton (Find-ProductButton $main 'Commit') 'Git Commit'
        $commitTaskDialog = Wait-WindowTitle $process.Id 'ステージ済み変更をコミット'
        $commitProgressEvidence = Get-DialogControlsEvidence $commitTaskDialog
        $commitWaitDeadline = [DateTime]::UtcNow.AddSeconds(15)
        while ([MDLiteGitProductNative]::IsWindow($commitTaskDialog) -and [DateTime]::UtcNow -lt $commitWaitDeadline) { Start-Sleep -Milliseconds 40 }
        if ([MDLiteGitProductNative]::IsWindow($commitTaskDialog)) { throw 'Commit progress dialog did not close within 15 seconds.' }
        if ($env:MDLITE_TEST_SILENT -eq '1') {
            $commitResultText = 'RunProcessWithCancel dialog closed; the subsequent MessageBoxW result is intentionally suppressed in MDLITE_TEST_SILENT mode.'
        } else {
            $commitResultDialog = Wait-WindowTitle $process.Id 'ステージ済み変更をコミット'
            $commitResultText = Get-DialogChildText $commitResultDialog
            [void][MDLiteGitProductNative]::PostMessage($commitResultDialog, 0x0111, [IntPtr]1, [IntPtr]::Zero)
            if (-not (Wait-WindowGone $process.Id 'ステージ済み変更をコミット')) { throw 'Commit result message did not close.' }
        }
    $committedTracked = Get-GitOutput $gitWorkspace @('show', 'HEAD:tracked.md')
    $workingTracked = [IO.File]::ReadAllText($trackedPath)
    $headFiles = Get-GitOutput $gitWorkspace @('ls-tree', '--name-only', 'HEAD')
    $headPaths = @($headFiles.output -split "`n" | Where-Object { $_ })
    $statusAfterCommit = Get-GitOutput $gitWorkspace @('status', '--short')
    $documentStates.disk_hash_after_commit = (Get-FileHash -Algorithm SHA256 -LiteralPath $gitDocument).Hash.ToLowerInvariant()
    $headContainsPendingAsStaged = @($headPaths | Where-Object { $_ -ceq 'pending.md' }).Count -gt 0
    $pendingRemainsInWorktreeStatus = $statusAfterCommit.output -match '(?m)^\?\? pending\.md$'
    $allCachedPathsCommitted = @($cachedPathsBeforeCommit | Where-Object { $_ -notin $headPaths }).Count -eq 0
    Add-Check 'product_commit_consumes_index_without_worktree_pathspec' ($committedTracked.exit_code -eq 0 -and
        $committedTracked.output -eq 'staged-version' -and $workingTracked -eq 'unstaged-version' -and
        $headFiles.output -match '(?m)^tracked\.md$' -and $headContainsPendingAsStaged -eq $pendingStagedAtCommit -and
        $allCachedPathsCommitted -and
        $headFiles.output -notmatch '(?m)^other\.md$' -and $statusAfterCommit.exit_code -eq 0 -and
        $documentStates.disk_hash_after_commit -ceq $diskHashBefore -and
        $statusAfterCommit.output.Contains('tracked.md') -and $statusAfterCommit.output.Contains('other.md') -and
        $pendingRemainsInWorktreeStatus -eq (-not $pendingStagedAtCommit)) ([ordered]@{
        commit_message = $commitMessage; commit_result = $commitResultText; committed_tracked_version = $committedTracked.output
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

    # Fetch is a distinct lane. In silent mode its non-Trust YES/NO confirmation is IDNO, so record that guard rather than waiting for a dialog that cannot appear.
    try {
        $stall = [MDLiteLocalStallServer]::new()
        $stall.Start()
        $remoteUrl = "http://127.0.0.1:$($stall.Port)/fixture.git"
        $remoteAdd = Get-GitOutput $gitWorkspace @('remote', 'add', 'origin', $remoteUrl)
        if ($remoteAdd.exit_code -ne 0) { throw "Could not configure fixture loopback remote: $($remoteAdd.output)" }
        Update-GitStatus $main
        $remotePanel = Wait-GitSummary $main 'unstaged 1'
        $remoteReady = [bool]$remotePanel -and -not $remotePanel.Contains('remoteなし')
        $fetchPaletteRoute = Invoke-ProductCommandFromPalette $main $process 'Git: Fetch'
        $fetchHead = Join-Path $gitWorkspace '.git\FETCH_HEAD'
        if ($env:MDLITE_TEST_SILENT -eq '1') {
            Start-Sleep -Milliseconds 250
            $fetchConfirmation = [MDLiteGitProductNative]::FindTopLevelWindow($process.Id, 'Git 明示操作')
            $fetchTaskDialog = [MDLiteGitProductNative]::FindTopLevelWindow($process.Id, 'Fetch')
            $loopbackConnected = $stall.WaitForConnection(750)
            $statusAfterFetchCancel = Get-GitOutput $gitWorkspace @('status', '--short')
            $checks['loopback_fetch_cancel_uses_product_taskdialog_and_preserves_index'] = [ordered]@{
                status = 'BLOCKED'
                details = [ordered]@{
                    reason = 'MDLITE_TEST_SILENT maps non-Workspace-Trust MB_YESNO confirmations to IDNO; Fetch never reaches RunProcessWithCancel.'
                    palette_route = $fetchPaletteRoute; confirmation_dialog_hwnd = $fetchConfirmation.ToInt64()
                    fetch_taskdialog_hwnd = $fetchTaskDialog.ToInt64(); loopback_connection_accepted = $loopbackConnected
                    url = $remoteUrl; panel = $remotePanel; remote_ready = $remoteReady
                    status_before = $statusAfterCommit.output; status_after = $statusAfterFetchCancel.output
                    fetch_head_created = (Test-Path -LiteralPath $fetchHead)
                }
            }
        } else {
            $confirmation = Wait-WindowTitle $process.Id 'Git 明示操作'
            $fetchQuestion = Get-DialogChildText $confirmation
            [void][MDLiteGitProductNative]::PostMessage($confirmation, 0x0111, [IntPtr]6, [IntPtr]::Zero)
            if (-not (Wait-WindowGone $process.Id 'Git 明示操作')) { throw 'Fetch confirmation stayed open.' }
            $taskDialog = Wait-WindowTitle $process.Id 'Fetch'
            $cancelButton = [IntPtr]::Zero
            foreach ($candidate in [MDLiteGitProductNative]::Children($taskDialog)) {
                if ([MDLiteGitProductNative]::WindowClass($candidate) -ne 'Button') { continue }
                $caption = [MDLiteGitProductNative]::WindowText($candidate)
                if ($caption -match '^(Cancel|キャンセル)$') { $cancelButton = $candidate; break }
            }
            $cancelCaption = if ($cancelButton -ne [IntPtr]::Zero) { [MDLiteGitProductNative]::WindowText($cancelButton) } else { '' }
            if ($cancelButton -eq [IntPtr]::Zero) { throw 'The actual Fetch TaskDialog Cancel button was not found.' }
            $loopbackConnected = $stall.WaitForConnection(2500)
            Click-ProductButton $cancelButton 'Fetch cancellation'
            if (-not (Wait-WindowGone $process.Id 'Fetch' 10000)) { throw 'Cancellable Fetch TaskDialog did not close.' }
            $fetchResult = Wait-WindowTitle $process.Id 'Fetch'
            $fetchResultText = Get-DialogChildText $fetchResult
            [void][MDLiteGitProductNative]::PostMessage($fetchResult, 0x0111, [IntPtr]1, [IntPtr]::Zero)
            if (-not (Wait-WindowGone $process.Id 'Fetch')) { throw 'Fetch result message did not close.' }
            $statusAfterFetchCancel = Get-GitOutput $gitWorkspace @('status', '--short')
            Add-Check 'loopback_fetch_cancel_uses_product_taskdialog_and_preserves_index' ($remoteReady -and $fetchQuestion.Contains('fetch --prune') -and
                $loopbackConnected -and [bool]$cancelCaption -and $fetchResultText.Contains('キャンセル') -and $statusAfterFetchCancel.exit_code -eq 0 -and
                $statusAfterFetchCancel.output -ceq $statusAfterCommit.output -and -not (Test-Path -LiteralPath $fetchHead)) ([ordered]@{
                url = $remoteUrl; panel = $remotePanel; loopback_connection_accepted = $loopbackConnected
                confirmation = $fetchQuestion; cancel_button = $cancelCaption; result = $fetchResultText
                status_before = $statusAfterCommit.output; status_after = $statusAfterFetchCancel.output; fetch_head_created = (Test-Path -LiteralPath $fetchHead)
            })
        }
    } catch {
        $checks['loopback_fetch_cancel_uses_product_taskdialog_and_preserves_index'] = [ordered]@{
            status = 'BLOCKED'; details = [ordered]@{ error = $_.Exception.Message; process_id = $process.Id; main_window_hwnd = $main.ToInt64() }
        }
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
    trust_fixture_route = 'noGit fixture uses WM_APP+67 wParam=1 to select only its .mdlite/.state/test-trust store, then real native Workspace Trust dialogs for deny, accept, and revoke; main Git fixture uses explicitly labeled wParam=0 direct fixture grant for product Git checks.'
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
