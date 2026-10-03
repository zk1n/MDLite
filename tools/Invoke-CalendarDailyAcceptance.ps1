#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Preset = 'debug',
    [string]$OutputPath,
    [switch]$PaletteFilterOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }

$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'build\verification'))
$runRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot "calendar-daily-$runId"))
$evidenceRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot 'prehuman-calendar-git'))
$workspace = Join-Path $runRoot 'workspace'
$ownerMarker = Join-Path $runRoot '.calendar-daily-acceptance-owner'
if (-not $OutputPath) {
    $outputName = if ($PaletteFilterOnly) { "calendar-palette-filter-$runId.json" } else { "calendar-daily-acceptance-$runId.json" }
    $OutputPath = Join-Path $evidenceRoot $outputName
}

$process = $null
$main = [IntPtr]::Zero
$checks = [ordered]@{}
$statusSnapshots = [Collections.Generic.List[object]]::new()
$errorMessage = $null
$status = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
$dailyRoot = Join-Path $workspace 'Dairy'
$date = [DateTime]::Now.Date
$relativeDailyPath = Join-Path 'Dairy\AcceptanceProbe' (Join-Path $date.ToString('yyyy') (Join-Path $date.ToString('yyyyMM') ($date.ToString('yyyyMMdd') + '.md')))
$createdFilePath = Join-Path $workspace $relativeDailyPath
$creationSource = Join-Path $workspace 'records\created-day.md'
$creationContentHash = ''
$renamedSource = Join-Path $workspace 'records\renamed-created-day.md'
$movedSource = Join-Path $workspace 'archive\renamed-created-day.md'
$dailyCounts = [ordered]@{}
$openedFilePath = $null
$sessionDocuments = @()
$nativeStatusCapture = $null
$messageReceipts = [ordered]@{}
$commandPaletteInteractions = [Collections.Generic.List[object]]::new()
$renameDialogMetadata = $null
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MDLiteCalendarDailyNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    public delegate bool EnumChildWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] private static extern IntPtr GetParent(IntPtr window);
    [DllImport("user32.dll")] private static extern IntPtr GetWindow(IntPtr window, uint command);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll", EntryPoint = "SendMessageW")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageSetText(IntPtr window, uint message, IntPtr wparam, string text);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageGetText(IntPtr window, uint message, IntPtr wparam, StringBuilder text);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    public static IntPtr ParentWindow(IntPtr window) { return GetParent(window); }
    public static IntPtr OwnerWindow(IntPtr window) { return GetWindow(window, 4); } // GW_OWNER
    public static IntPtr RootWindow(IntPtr window) { return GetAncestor(window, 2); } // GA_ROOT
    public static IntPtr RootOwnerWindow(IntPtr window) { return GetAncestor(window, 3); } // GA_ROOTOWNER
    public static IntPtr MakePoint(int x, int y) {
        uint packed = (uint)(ushort)(short)x | ((uint)(ushort)(short)y << 16);
        return new IntPtr(unchecked((int)packed));
    }

    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    private struct TVITEMW {
        public uint mask;
        public IntPtr item;
        public uint state;
        public uint stateMask;
        public IntPtr text;
        public int textCapacity;
        public int image;
        public int selectedImage;
        public int children;
        public IntPtr parameter;
    }

    [DllImport("kernel32.dll")] private static extern IntPtr OpenProcess(uint access, bool inherit, int processId);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern IntPtr VirtualAllocEx(IntPtr process, IntPtr address, UIntPtr size, uint allocationType, uint protection);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool VirtualFreeEx(IntPtr process, IntPtr address, UIntPtr size, uint freeType);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool WriteProcessMemory(IntPtr process, IntPtr address, IntPtr buffer, UIntPtr size, out UIntPtr written);
    [DllImport("kernel32.dll", SetLastError = true)] private static extern bool ReadProcessMemory(IntPtr process, IntPtr address, IntPtr buffer, UIntPtr size, out UIntPtr read);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);

    public static IntPtr GetTreeItem(IntPtr tree, uint relation, IntPtr item) {
        return SendMessage(tree, 0x110A, new IntPtr(unchecked((int)relation)), item); // TVM_GETNEXTITEM
    }

    public static bool SelectTreeItem(IntPtr tree, IntPtr item) {
        return SendMessage(tree, 0x110B, new IntPtr(9), item) != IntPtr.Zero; // TVM_SELECTITEM / TVGN_CARET
    }

    public static string GetTreeItemText(IntPtr tree, int processId, IntPtr item) {
        const uint processAccess = 0x0008 | 0x0020 | 0x0010; // VM_OPERATION | VM_READ | VM_WRITE
        const uint commit = 0x1000;
        const uint release = 0x8000;
        const uint readWrite = 0x04;
        const uint tvifText = 0x0001;
        const int capacity = 512;
        IntPtr process = OpenProcess(processAccess, false, processId);
        if (process == IntPtr.Zero) throw new InvalidOperationException("OpenProcess failed while reading the Workspace tree.");
        IntPtr remoteText = IntPtr.Zero, remoteItem = IntPtr.Zero, localText = IntPtr.Zero, localItem = IntPtr.Zero;
        try {
            int textBytes = capacity * 2;
            int itemBytes = Marshal.SizeOf(typeof(TVITEMW));
            remoteText = VirtualAllocEx(process, IntPtr.Zero, new UIntPtr((uint)textBytes), commit, readWrite);
            remoteItem = VirtualAllocEx(process, IntPtr.Zero, new UIntPtr((uint)itemBytes), commit, readWrite);
            localText = Marshal.AllocHGlobal(textBytes);
            localItem = Marshal.AllocHGlobal(itemBytes);
            if (remoteText == IntPtr.Zero || remoteItem == IntPtr.Zero) throw new InvalidOperationException("VirtualAllocEx failed while reading the Workspace tree.");
            TVITEMW value = new TVITEMW { mask = tvifText, item = item, text = remoteText, textCapacity = capacity };
            Marshal.StructureToPtr(value, localItem, false);
            UIntPtr written = UIntPtr.Zero;
            if (!WriteProcessMemory(process, remoteItem, localItem, new UIntPtr((uint)itemBytes), out written) || written.ToUInt64() != (ulong)itemBytes)
                throw new InvalidOperationException("WriteProcessMemory failed for the Workspace tree item.");
            if (SendMessage(tree, 0x113E, IntPtr.Zero, remoteItem) == IntPtr.Zero) return String.Empty; // TVM_GETITEMW
            UIntPtr read = UIntPtr.Zero;
            if (!ReadProcessMemory(process, remoteText, localText, new UIntPtr((uint)textBytes), out read) || read.ToUInt64() < 2)
                throw new InvalidOperationException("ReadProcessMemory failed for the Workspace tree text.");
            return Marshal.PtrToStringUni(localText) ?? String.Empty;
        } finally {
            if (localText != IntPtr.Zero) Marshal.FreeHGlobal(localText);
            if (localItem != IntPtr.Zero) Marshal.FreeHGlobal(localItem);
            if (remoteText != IntPtr.Zero) VirtualFreeEx(process, remoteText, UIntPtr.Zero, release);
            if (remoteItem != IntPtr.Zero) VirtualFreeEx(process, remoteItem, UIntPtr.Zero, release);
            CloseHandle(process);
        }
    }

}
'@
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$WM_SETTEXT = 0x000C
$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$EM_SETSEL = 0x00B1
$WM_MOUSEMOVE = 0x0200
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_SETFOCUS = 0x0007
$EN_CHANGE = 0x0300
$BM_CLICK = 0x00F5
$LB_SETCURSEL = 0x0186
$LB_GETCOUNT = 0x018B
$VK_LEFT = 0x25
$VK_RIGHT = 0x27
$VK_RETURN = 0x0D
$SWP_NOZORDER = 0x0004
$SWP_NOACTIVATE = 0x0010
$kCalendarView = 118
$kEditor = 102
$kCommandSearch = 122
$expectedEditorId = $kEditor
$buildReceiptPath = Join-Path $verificationRoot "build-receipt-$Preset.json"
$executableHashBefore = ''
$sourceFingerprintBefore = $null
$buildReceiptBefore = $null
$sourceFingerprintAfter = $null
$executableHashAfter = ''

function Add-Check([string]$Name, [bool]$Pass, $Details = $null) {
    $checks[$Name] = [ordered]@{ status = if ($Pass) { 'PASS' } else { 'FAIL' }; details = $Details }
}

function Find-MainWindow([Diagnostics.Process]$Target, [int]$TimeoutMs = 15000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Target.Refresh()
        if ($Target.HasExited) { throw "MDLite exited with code $($Target.ExitCode) before its main window appeared." }
        $script:foundMain = [IntPtr]::Zero
        $script:targetPid = [uint32]$Target.Id
        $callback = [MDLiteCalendarDailyNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$parameter)
            [uint32]$ownerPid = 0
            [void][MDLiteCalendarDailyNative]::GetWindowThreadProcessId($window, [ref]$ownerPid)
            if ($ownerPid -ne $script:targetPid) { return $true }
            $className = New-Object Text.StringBuilder 128
            [void][MDLiteCalendarDailyNative]::GetClassName($window, $className, $className.Capacity)
            if ($className.ToString() -eq 'MDLite.MainWindow') {
                $script:foundMain = $window
                return $false
            }
            return $true
        }
        [void][MDLiteCalendarDailyNative]::EnumWindows($callback, [IntPtr]::Zero)
        if ($script:foundMain -ne [IntPtr]::Zero) { return $script:foundMain }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main window was not created for process $($Target.Id)."
}

function Find-Child([IntPtr]$Parent, [int]$Id = -1, [string]$ClassName = '') {
    $children = @(Find-Children $Parent $Id $ClassName)
    if ($children.Count -gt 0) { return $children[0] }
    return [IntPtr]::Zero
}

function Find-Children([IntPtr]$Parent, [int]$Id = -1, [string]$ClassName = '') {
    $script:foundChildren = [Collections.Generic.List[IntPtr]]::new()
    $script:targetChildId = $Id
    $script:targetChildClass = $ClassName
    $callback = [MDLiteCalendarDailyNative+EnumChildWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ($script:targetChildId -ge 0 -and
            [MDLiteCalendarDailyNative]::GetDlgCtrlID($window) -ne $script:targetChildId) { return $true }
        if ($script:targetChildClass) {
            $className = New-Object Text.StringBuilder 128
            [void][MDLiteCalendarDailyNative]::GetClassName($window, $className, $className.Capacity)
            if ($className.ToString() -ne $script:targetChildClass) { return $true }
        }
        $script:foundChildren.Add($window)
        return $true
    }
    [void][MDLiteCalendarDailyNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return @($script:foundChildren.ToArray())
}

function Wait-Child([IntPtr]$Parent, [int]$Id = -1, [string]$ClassName = '', [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $child = Find-Child $Parent $Id $ClassName
        if ($child -ne [IntPtr]::Zero) { return $child }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native child was not created: id=$Id class=$ClassName."
}

function Get-WindowTextValue([IntPtr]$Window) {
    $length = [MDLiteCalendarDailyNative]::SendMessage($Window, $WM_GETTEXTLENGTH, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $buffer = New-Object Text.StringBuilder ([Math]::Max(2, $length + 1))
    [void][MDLiteCalendarDailyNative]::SendMessageGetText($Window, $WM_GETTEXT, [IntPtr]($length + 1), $buffer)
    return $buffer.ToString()
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

function Find-TreeItemByText([IntPtr]$Tree, [int]$ProcessId, [string]$Text) {
    $item = [MDLiteCalendarDailyNative]::GetTreeItem($Tree, 0, [IntPtr]::Zero) # TVGN_ROOT
    while ($item -ne [IntPtr]::Zero) {
        if ([MDLiteCalendarDailyNative]::GetTreeItemText($Tree, $ProcessId, $item) -ceq $Text) { return $item }
        [void][MDLiteCalendarDailyNative]::SendMessage($Tree, 0x1102, [IntPtr]2, $item) # TVM_EXPAND / TVE_EXPAND; loads placeholder children.
        $child = [MDLiteCalendarDailyNative]::GetTreeItem($Tree, 4, $item) # TVGN_CHILD
        if ($child -ne [IntPtr]::Zero) { $item = $child; continue }
        while ($item -ne [IntPtr]::Zero) {
            $sibling = [MDLiteCalendarDailyNative]::GetTreeItem($Tree, 1, $item) # TVGN_NEXT
            if ($sibling -ne [IntPtr]::Zero) { $item = $sibling; break }
            $item = [MDLiteCalendarDailyNative]::GetTreeItem($Tree, 3, $item) # TVGN_PARENT
        }
    }
    return [IntPtr]::Zero
}

function Find-TopLevelWindowByTitle([int]$ProcessId, [string]$Title) {
    $script:foundDialog = [IntPtr]::Zero
    $script:dialogPid = [uint32]$ProcessId
    $script:dialogTitle = $Title
    $callback = [MDLiteCalendarDailyNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        [uint32]$ownerPid = 0
        [void][MDLiteCalendarDailyNative]::GetWindowThreadProcessId($window, [ref]$ownerPid)
        if ($ownerPid -ne $script:dialogPid) { return $true }
        $value = New-Object Text.StringBuilder 512
        [void][MDLiteCalendarDailyNative]::GetWindowText($window, $value, $value.Capacity)
        if ($value.ToString() -ceq $script:dialogTitle) {
            $script:foundDialog = $window
            return $false
        }
        return $true
    }
    [void][MDLiteCalendarDailyNative]::EnumWindows($callback, [IntPtr]::Zero)
    return $script:foundDialog
}

function Wait-TopLevelWindowByTitle([int]$ProcessId, [string]$Title, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $window = Find-TopLevelWindowByTitle $ProcessId $Title
        if ($window -ne [IntPtr]::Zero) { return $window }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native dialog did not appear: $Title"
}

function Open-ProductPaletteFilter([string]$Query) {
    $search = Find-Child $main $kCommandSearch 'Button'
    if ($search -eq [IntPtr]::Zero) { throw 'The Command Search product control was not found.' }
    if (-not [MDLiteCalendarDailyNative]::PostMessage($search, $BM_CLICK, [IntPtr]::Zero, [IntPtr]::Zero)) {
        throw 'Could not post the Command Search button activation.'
    }
    $picker = Wait-TopLevelWindowByTitle $process.Id 'MDLite コマンドパレット'
    $controlsWatch = [Diagnostics.Stopwatch]::StartNew()
    $filter = [IntPtr]::Zero
    $list = [IntPtr]::Zero
    $controlsDeadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $filter = Find-Child $picker 100 'Edit'
        $list = Find-Child $picker 101 'ListBox'
        if ($filter -ne [IntPtr]::Zero -and $list -ne [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 25
    } while ([DateTime]::UtcNow -lt $controlsDeadline)
    $controlsWatch.Stop()
    if ($filter -eq [IntPtr]::Zero -or $list -eq [IntPtr]::Zero) {
        $children = foreach ($child in (Find-Children $picker)) {
            $class = New-Object Text.StringBuilder 128
            [void][MDLiteCalendarDailyNative]::GetClassName($child, $class, $class.Capacity)
            $text = Get-WindowTextValue $child
            '{0}:id={1}:text={2}' -f $class.ToString(), [MDLiteCalendarDailyNative]::GetDlgCtrlID($child), $text
        }
        throw "Command palette controls were not ready in 3000ms (pid=$($process.Id), picker=$($picker.ToInt64()), children=$($children -join '; '))."
    }
    $countBefore = [MDLiteCalendarDailyNative]::SendMessage($list, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $setTextResult = [MDLiteCalendarDailyNative]::SendMessageSetText($filter, $WM_SETTEXT, [IntPtr]::Zero, $Query).ToInt32()
    Start-Sleep -Milliseconds 150
    $filterText = Get-WindowTextValue $filter
    $countAfterSetText = [MDLiteCalendarDailyNative]::SendMessage($list, $LB_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $filterClassBuffer = New-Object Text.StringBuilder 128
    [void][MDLiteCalendarDailyNative]::GetClassName($filter, $filterClassBuffer, $filterClassBuffer.Capacity)
    $filterClass = $filterClassBuffer.ToString()
    $notificationWParam = [IntPtr]([int64](($EN_CHANGE -shl 16) -bor 100))
    $notificationLParam = $filter
    # The Edit control's native WM_SETTEXT delivers EN_CHANGE to the picker.
    $count = $countAfterSetText
    $probe = [pscustomobject]@{
        record_type = 'filter_probe'
        picker = $picker; list = $list; filter = $filter; query = $Query
        control_wait_ms = $controlsWatch.ElapsedMilliseconds
        filter_hwnd = $filter.ToInt64(); filter_class = $filterClass
        filter_control_id = [MDLiteCalendarDailyNative]::GetDlgCtrlID($filter); filter_text = $filterText
        wm_settext_lresult = $setTextResult; list_count_before = $countBefore; list_count_after_settext = $countAfterSetText
        parent_wm_command_wparam_from_control_id_and_EN_CHANGE = ('0x{0:X8}' -f $notificationWParam.ToInt64())
        parent_wm_command_lparam_filter_hwnd = $notificationLParam.ToInt64(); list_count_after_native_EN_CHANGE = $count
    }
    $script:commandPaletteInteractions.Add([ordered]@{
        record_type = $probe.record_type; query = $probe.query; filter_hwnd = $probe.filter_hwnd
        filter_class = $probe.filter_class; filter_control_id = $probe.filter_control_id
        control_wait_ms = $probe.control_wait_ms
        filter_text = $probe.filter_text; wm_settext_lresult = $probe.wm_settext_lresult
        list_count_before = $probe.list_count_before; list_count_after_settext = $probe.list_count_after_settext
        parent_wm_command_wparam_from_control_id_and_EN_CHANGE = $probe.parent_wm_command_wparam_from_control_id_and_EN_CHANGE
        parent_wm_command_lparam_filter_hwnd = $probe.parent_wm_command_lparam_filter_hwnd
        list_count_after_native_EN_CHANGE = $probe.list_count_after_native_EN_CHANGE
    })
    return $probe
}

function Invoke-ProductCommandFromPalette([string]$Query) {
    $filtered = Open-ProductPaletteFilter $Query
    if ($filtered.list_count_after_native_EN_CHANGE -ne 1) {
        throw "Command palette WM_SETTEXT '$($filtered.filter_text)' matched $($filtered.list_count_after_native_EN_CHANGE) rows (before=$($filtered.list_count_before) after_settext=$($filtered.list_count_after_settext)); expected exactly one."
    }
    $picker = $filtered.picker
    $list = $filtered.list
    [void][MDLiteCalendarDailyNative]::SendMessage($list, $LB_SETCURSEL, [IntPtr]::Zero, [IntPtr]::Zero)
    if (-not [MDLiteCalendarDailyNative]::PostMessage($picker, $WM_COMMAND, [IntPtr]1, [IntPtr]::Zero)) {
        throw 'Could not accept the selected command palette row.'
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if ((Find-TopLevelWindowByTitle $process.Id 'MDLite コマンドパレット') -eq [IntPtr]::Zero) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    $interaction = [ordered]@{
        invocation = 'Command Search control -> native command palette -> WM_SETTEXT -> EN_CHANGE -> filtered list -> IDOK'
        query = $Query; filter_hwnd = $filtered.filter_hwnd; filter_class = $filtered.filter_class
        filter_control_id = $filtered.filter_control_id; control_wait_ms = $filtered.control_wait_ms; filter_text = $filtered.filter_text
        wm_settext_lresult = $filtered.wm_settext_lresult; list_count_before = $filtered.list_count_before
        list_count_after_settext = $filtered.list_count_after_settext
        parent_wm_command_wparam_from_control_id_and_EN_CHANGE = $filtered.parent_wm_command_wparam_from_control_id_and_EN_CHANGE
        parent_wm_command_lparam_filter_hwnd = $filtered.parent_wm_command_lparam_filter_hwnd
        list_count_after_native_EN_CHANGE = $filtered.list_count_after_native_EN_CHANGE
        accepted = ((Find-TopLevelWindowByTitle $process.Id 'MDLite コマンドパレット') -eq [IntPtr]::Zero)
    }
    $script:commandPaletteInteractions.Add($interaction)
    if (-not $interaction.accepted) { throw 'The command palette remained open after accepting a command.' }
    return $interaction
}

function Set-CommonFileDialogDestination([IntPtr]$Dialog, [string]$SourceName, [string]$SourcePath, [string]$TargetPath) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($Dialog)
    if ($null -eq $root) { throw 'UI Automation could not attach to the real common file dialog.' }
    [uint32]$dialogOwnerPid = 0
    [void][MDLiteCalendarDailyNative]::GetWindowThreadProcessId($Dialog, [ref]$dialogOwnerPid)
    $dialogClassBuffer = New-Object Text.StringBuilder 128
    [void][MDLiteCalendarDailyNative]::GetClassName($Dialog, $dialogClassBuffer, $dialogClassBuffer.Capacity)
    $rootCurrent = $root.Current
    $script:renameDialogMetadata = [ordered]@{
        dialog_hwnd = $Dialog.ToInt64(); dialog_caption = Get-WindowTextValue $Dialog
        dialog_class = $dialogClassBuffer.ToString(); dialog_owner_pid = $dialogOwnerPid
        dialog_parent_hwnd = [MDLiteCalendarDailyNative]::ParentWindow($Dialog).ToInt64()
        dialog_owner_hwnd = [MDLiteCalendarDailyNative]::OwnerWindow($Dialog).ToInt64()
        dialog_root_hwnd = [MDLiteCalendarDailyNative]::RootWindow($Dialog).ToInt64()
        dialog_root_owner_hwnd = [MDLiteCalendarDailyNative]::RootOwnerWindow($Dialog).ToInt64()
        ui_automation_process_id = $rootCurrent.ProcessId; ui_automation_native_hwnd = $rootCurrent.NativeWindowHandle
        selected_tree_item_text = $SourceName
        selected_tree_source_path = $SourcePath
        selected_tree_source_sha256_before = if (Test-Path -LiteralPath $SourcePath -PathType Leaf) { (Get-FileHash -Algorithm SHA256 -LiteralPath $SourcePath).Hash.ToLowerInvariant() } else { '' }
    }
    $all = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,
        [System.Windows.Automation.Condition]::TrueCondition)
    $valueCandidates = [Collections.Generic.List[object]]::new()
    $uiaControls = [Collections.Generic.List[object]]::new()
    $uiaLabels = [Collections.Generic.List[string]]::new()
    foreach ($element in $all) {
        $current = $element.Current
        if ($current.ControlType.ProgrammaticName -eq 'ControlType.Text' -and $current.Name) { $uiaLabels.Add($current.Name) }
        $pattern = $null
        $hasValuePattern = $element.TryGetCurrentPattern([System.Windows.Automation.ValuePattern]::Pattern, [ref]$pattern)
        $value = ''
        $isReadOnly = $true
        if ($hasValuePattern) {
            $valuePattern = [System.Windows.Automation.ValuePattern]$pattern
            $value = $valuePattern.Current.Value
            $isReadOnly = $valuePattern.Current.IsReadOnly
        }
        if ($current.ControlType.ProgrammaticName -match 'ControlType\.(Edit|ComboBox)' -or $hasValuePattern) {
            $uiaControls.Add([pscustomobject]@{
                name = $current.Name; automation_id = $current.AutomationId
                control_type = $current.ControlType.ProgrammaticName; process_id = $current.ProcessId
                native_hwnd = $current.NativeWindowHandle; value = $value
                has_value_pattern = $hasValuePattern; is_read_only = $isReadOnly
            })
        }
        if (-not $hasValuePattern) { continue }
        $valuePattern = [System.Windows.Automation.ValuePattern]$pattern
        $candidate = [pscustomobject]@{
            element = $element; pattern = $valuePattern; name = $current.Name; automation_id = $current.AutomationId
            control_type = $current.ControlType.ProgrammaticName; process_id = $current.ProcessId
            value = $value; is_read_only = $isReadOnly
        }
        $valueCandidates.Add($candidate)
    }
    $script:renameDialogMetadata.uia_text_labels = @($uiaLabels.ToArray())
    $script:renameDialogMetadata.uia_edit_controls = @($uiaControls.ToArray())
    $filenameCandidates = @($valueCandidates | Where-Object {
        $_.name -match '(?i)file.?name|ファイル名' -or $_.automation_id -match '(?i)file.?name|edt1|1152|0480' -or
        (-not $_.is_read_only -and $_.value.EndsWith($SourceName, [StringComparison]::OrdinalIgnoreCase))
    })
    if ($filenameCandidates.Count -eq 0 -and $valueCandidates.Count -eq 1 -and -not $valueCandidates[0].is_read_only) {
        $filenameCandidates = @($valueCandidates[0])
    }
    if ($filenameCandidates.Count -ne 1) {
        $summary = $uiaControls.ToArray() | ConvertTo-Json -Depth 5 -Compress
        throw "The Save dialog filename ValuePattern was not unique for source '$SourceName' (dialog=$($Dialog.ToInt64()), pid=$dialogOwnerPid, labels=$($uiaLabels -join '|'), controls=$summary)."
    }

    $filename = $filenameCandidates[0]
    $before = $filename.pattern.Current.Value
    $filename.pattern.SetValue($TargetPath)
    $deadline = [DateTime]::UtcNow.AddSeconds(3)
    $readback = ''
    do {
        $readback = $filename.pattern.Current.Value
        if ($readback -ceq $TargetPath) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($readback -cne $TargetPath) {
        throw "The Save dialog filename did not read back the requested target (before='$before', after='$readback')."
    }

    $buttonCondition = New-Object System.Windows.Automation.PropertyCondition(
        [System.Windows.Automation.AutomationElement]::ControlTypeProperty,
        [System.Windows.Automation.ControlType]::Button)
    $buttons = $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $buttonCondition)
    $saveButton = $null
    foreach ($button in $buttons) {
        $current = $button.Current
        if ($current.Name -match '(?i)save|保存' -or $current.AutomationId -eq '1') { $saveButton = $button; break }
    }
    if ($null -eq $saveButton) { throw 'The common file dialog Save button was not exposed through UI Automation.' }
    $savePattern = $null
    if (-not $saveButton.TryGetCurrentPattern([System.Windows.Automation.InvokePattern]::Pattern, [ref]$savePattern)) {
        throw "The common file dialog Save button has no InvokePattern (name='$($saveButton.Current.Name)')."
    }
    $savePattern.Invoke()
    return [ordered]@{
        route = 'UI Automation ValuePattern on the actual filename field, readback, and InvokePattern on Save'
        source_name_from_selected_tree_item = $SourceName
        filename_control = [ordered]@{
            name = $filename.name; automation_id = $filename.automation_id; control_type = $filename.control_type
            process_id = $filename.process_id; value_before = $before; value_after = $readback
        }
        save_button = [ordered]@{ name = $saveButton.Current.Name; automation_id = $saveButton.Current.AutomationId; process_id = $saveButton.Current.ProcessId }
    }
}

function Invoke-ProductRename([string]$OldName, [string]$SourcePath, [string]$TargetPath) {
    if (-not (Test-Path -LiteralPath $SourcePath -PathType Leaf)) { throw "Rename source file was not present in the fixture: $SourcePath" }
    if ([IO.Path]::GetFileName($SourcePath) -cne $OldName) { throw "Tree item label does not match the requested source file: $OldName / $SourcePath" }
    $tree = Find-Child $main -1 'SysTreeView32'
    if ($tree -eq [IntPtr]::Zero) { throw 'Workspace tree was not found.' }
    $item = Find-TreeItemByText $tree $process.Id $OldName
    if ($item -eq [IntPtr]::Zero) { throw "Workspace tree item was not found: $OldName" }
    if (-not [MDLiteCalendarDailyNative]::SelectTreeItem($tree, $item)) { throw "Workspace tree selection failed: $OldName" }
    $paletteResult = Invoke-ProductCommandFromPalette 'Workspace: 名前変更・移動'
    $dialog = Wait-TopLevelWindowByTitle $process.Id '新しい名前または移動先'
    $dialogReadyWatch = [Diagnostics.Stopwatch]::StartNew()
    $dialogResult = Set-CommonFileDialogDestination $dialog $OldName $SourcePath $TargetPath
    $dialogReadyWatch.Stop()
    $paletteResult.destination_path = $TargetPath
    $paletteResult.common_dialog = $dialogResult
    $paletteResult.common_dialog_ready_wait_ms = $dialogReadyWatch.ElapsedMilliseconds
    return $paletteResult
}

function Get-CalendarDetailsControl([IntPtr]$MainWindow) {
    foreach ($candidate in (Find-Children $MainWindow -1 'Edit')) {
        $text = Get-WindowTextValue $candidate
        if ($text.StartsWith('選択日:')) { return $candidate }
    }
    return [IntPtr]::Zero
}

function Get-CalendarDetailsText([IntPtr]$MainWindow) {
    $control = Get-CalendarDetailsControl $MainWindow
    if ($control -eq [IntPtr]::Zero) { return '' }
    return Get-WindowTextValue $control
}

function Get-NativeStatus([IntPtr]$Window) {
    $statusBar = Find-Child $Window -1 'msctls_statusbar32'
    if ($statusBar -eq [IntPtr]::Zero) { return [pscustomobject]@{ status = 'NOT_FOUND'; text = '' } }
    $text = Get-WindowTextValue $statusBar
    return [pscustomobject]@{
        status = if ($text) { 'CAPTURED_WM_GETTEXT' } else { 'OWNERDRAW_TEXT_UNAVAILABLE_CROSS_PROCESS' }
        text = $text
    }
}

function Get-DailyFiles {
    if (-not (Test-Path -LiteralPath $dailyRoot -PathType Container)) { return ,@() }
    $files = @(Get-ChildItem -LiteralPath $dailyRoot -File -Recurse -ErrorAction SilentlyContinue |
        Where-Object { $_.Extension -eq '.md' })
    return ,$files
}

function Get-CalendarGeometry([IntPtr]$Calendar) {
    [MDLiteCalendarDailyNative+RECT]$client = New-Object MDLiteCalendarDailyNative+RECT
    if (-not [MDLiteCalendarDailyNative]::GetClientRect($Calendar, [ref]$client)) { throw 'GetClientRect failed for CalendarView.' }
    $width = $client.Right - $client.Left
    $height = $client.Bottom - $client.Top
    $header = [Math]::Min($height, [Math]::Max(24, [Math]::Min(36, [int][Math]::Floor($height / 5))))
    $weekday = [Math]::Min($height - $header,
        [Math]::Max(20, [Math]::Min(24, [int][Math]::Floor(($height - $header) / 8))))
    $gridTop = $header + $weekday
    $footer = [Math]::Min([Math]::Max(0, $height - $gridTop),
        [Math]::Max(24, [Math]::Min(32, [int][Math]::Floor($height / 9))))
    $gridBottom = [Math]::Max($gridTop, $height - $footer)
    return [pscustomobject]@{ width = $width; height = $height; header = $header; gridTop = $gridTop; gridBottom = $gridBottom; navWidth = [Math]::Min(44, [int][Math]::Floor($width / 3)) }
}

function Get-DatePoint([object]$Geometry, [DateTime]$Day) {
    $monthStart = [DateTime]::new($Day.Year, $Day.Month, 1)
    $cell = ([int]$monthStart.DayOfWeek + $Day.Day - 1)
    $row = [int][Math]::Floor($cell / 7)
    $column = $cell % 7
    $gridHeight = $Geometry.gridBottom - $Geometry.gridTop
    $left = [int][Math]::Floor($Geometry.width * $column / 7)
    $right = [int][Math]::Floor($Geometry.width * ($column + 1) / 7)
    $top = $Geometry.gridTop + [int][Math]::Floor($gridHeight * $row / 6)
    $bottom = $Geometry.gridTop + [int][Math]::Floor($gridHeight * ($row + 1) / 6)
    return [pscustomobject]@{ x = [int](($left + $right) / 2); y = [int](($top + $bottom) / 2); row = $row; column = $column; date = $Day.ToString('yyyy-MM-dd') }
}

function Send-Click([IntPtr]$Window, [int]$X, [int]$Y) {
    $point = [MDLiteCalendarDailyNative]::MakePoint($X, $Y)
    $down = [MDLiteCalendarDailyNative]::SendMessage($Window, $WM_LBUTTONDOWN, [IntPtr]1, $point).ToInt64()
    $up = [MDLiteCalendarDailyNative]::SendMessage($Window, $WM_LBUTTONUP, [IntPtr]::Zero, $point).ToInt64()
    return [pscustomobject]@{ down = $down; up = $up }
}

function Capture-Status([string]$Phase) {
    $captured = Get-NativeStatus $main
    $statusSnapshots.Add([pscustomobject]@{ phase = $Phase; status = $captured.status; text = $captured.text })
    return $captured
}

function Stop-TargetProcess {
    if ($script:process -and -not $script:process.HasExited) {
        if ($script:main -ne [IntPtr]::Zero) {
            [void][MDLiteCalendarDailyNative]::PostMessage($script:main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
        }
        if (-not $script:process.WaitForExit(5000)) {
            $script:process.Kill()
            [void]$script:process.WaitForExit(3000)
        }
    }
    if ($script:process) { $script:process.Dispose(); $script:process = $null }
    $script:main = [IntPtr]::Zero
}

if ($PaletteFilterOnly) {
    $probeWorkspace = Join-Path $runRoot 'palette-workspace'
    $probeDocument = Join-Path $probeWorkspace 'README.md'
    $probeSourceBefore = $null
    $probeSourceAfter = $null
    $probeReceipt = $null
    $probeExeBefore = ''
    $probeExeAfter = ''
    $probe = $null
    $probeEvidence = $null
    $probeError = $null
    $probeStatus = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
    try {
        [IO.Directory]::CreateDirectory((Join-Path $probeWorkspace '.mdlite')) | Out-Null
        [IO.File]::WriteAllText($ownerMarker, $runId, [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText((Join-Path $probeWorkspace '.mdlite\settings.toml'),
            "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
        [IO.File]::WriteAllText($probeDocument, "# Palette filter probe $runId`n", [Text.UTF8Encoding]::new($false))
        if (-not (Test-Path -LiteralPath $buildReceiptPath -PathType Leaf)) { throw "Build receipt is missing: $buildReceiptPath" }
        $probeReceipt = Get-Content -Raw -LiteralPath $buildReceiptPath | ConvertFrom-Json
        $probeExeBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
        $probeSourceBefore = Get-SourceFingerprint
        if ($probeReceipt.build_status -ne 'PASS' -or $probeReceipt.executable_sha256 -ne $probeExeBefore -or
            $probeReceipt.source_sha256 -ne $probeSourceBefore.sha256) { throw 'Build receipt does not match this probe source/executable.' }
        $process = Start-Process -FilePath $executable -ArgumentList @($probeDocument) -WorkingDirectory $repoRoot -PassThru
        $main = Find-MainWindow $process
        [void][MDLiteCalendarDailyNative]::SetWindowPos($main, [IntPtr]::Zero, 80, 80, 1440, 960, $SWP_NOZORDER)
        [void][MDLiteCalendarDailyNative]::ShowWindow($main, 9)
        $probe = Open-ProductPaletteFilter 'Workspace: 名前変更・移動'
        $probeEvidence = [ordered]@{
            query = $probe.query; filter_hwnd = $probe.filter_hwnd; filter_class = $probe.filter_class
            filter_control_id = $probe.filter_control_id; control_wait_ms = $probe.control_wait_ms; filter_text = $probe.filter_text
            wm_settext_lresult = $probe.wm_settext_lresult; list_count_before = $probe.list_count_before
            list_count_after_settext = $probe.list_count_after_settext
            parent_wm_command_wparam_from_control_id_and_EN_CHANGE = $probe.parent_wm_command_wparam_from_control_id_and_EN_CHANGE
            parent_wm_command_lparam_filter_hwnd = $probe.parent_wm_command_lparam_filter_hwnd
            list_count_after_native_EN_CHANGE = $probe.list_count_after_native_EN_CHANGE
        }
        $singleRow = $probe.wm_settext_lresult -ne 0 -and $probe.list_count_after_native_EN_CHANGE -eq 1 -and
            $probe.filter_text -ceq 'Workspace: 名前変更・移動'
        Add-Check 'command_palette_filter_single_row' $singleRow $probeEvidence
        [void][MDLiteCalendarDailyNative]::PostMessage($probe.picker, $WM_COMMAND, [IntPtr]2, [IntPtr]::Zero)
        $closeDeadline = [DateTime]::UtcNow.AddSeconds(5)
        do {
            if ((Find-TopLevelWindowByTitle $process.Id 'MDLite コマンドパレット') -eq [IntPtr]::Zero) { break }
            Start-Sleep -Milliseconds 35
        } while ([DateTime]::UtcNow -lt $closeDeadline)
        if ((Find-TopLevelWindowByTitle $process.Id 'MDLite コマンドパレット') -ne [IntPtr]::Zero) { throw 'Palette filter probe could not close its picker.' }
        Stop-TargetProcess
        $probeStatus = if ($singleRow) { 'PASS_PALETTE_FILTER_MICROPROBE' } else { 'FAIL_PALETTE_FILTER_MICROPROBE' }
    } catch {
        $probeError = $_.Exception.Message
    } finally {
        Stop-TargetProcess
        if ($null -eq $previousSilentSetting) { Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue } else { $env:MDLITE_TEST_SILENT = $previousSilentSetting }
    }
    $probeExeAfter = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $probeSourceAfter = Get-SourceFingerprint
    $stable = $probeSourceBefore -and $probeSourceAfter -and $probeSourceBefore.sha256 -ceq $probeSourceAfter.sha256 -and $probeExeBefore -ceq $probeExeAfter
    Add-Check 'source_and_executable_unchanged_during_palette_probe' ([bool]$stable) ([ordered]@{
        source_before = if ($probeSourceBefore) { $probeSourceBefore.sha256 } else { '' }
        source_after = if ($probeSourceAfter) { $probeSourceAfter.sha256 } else { '' }
        exe_before = $probeExeBefore; exe_after = $probeExeAfter
    })
    $probeFailed = @($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0
    $probeResult = [ordered]@{
        status = if ($probeFailed -and $probeStatus -like 'PASS*') { 'FAIL_PALETTE_FILTER_MICROPROBE' } else { $probeStatus }
        pass = ($probeStatus -like 'PASS*' -and -not $probeFailed)
        preset = $Preset; executable = $executable; executable_sha256_before = $probeExeBefore; executable_sha256_after = $probeExeAfter
        build_receipt_path = $buildReceiptPath; build_receipt = $probeReceipt
        source_fingerprint_before = $probeSourceBefore; source_fingerprint_after = $probeSourceAfter
        filter_probe = $probeEvidence; checks = $checks; error = $probeError
        evidence_boundary = 'Synthetic cross-process WM_SETTEXT and EN_CHANGE on the product NativePicker. No physical input or Human visual acceptance.'
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
    }
    $probeOutputDirectory = Split-Path -Parent $OutputPath
    if ($probeOutputDirectory) { [IO.Directory]::CreateDirectory($probeOutputDirectory) | Out-Null }
    $probeResult | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $probeResult | ConvertTo-Json -Depth 10
    if ($probeResult.pass) {
        $canonicalRunRoot = [IO.Path]::GetFullPath($runRoot)
        $canonicalVerificationRoot = [IO.Path]::GetFullPath($verificationRoot).TrimEnd('\') + '\'
        if ((Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
            $canonicalRunRoot.StartsWith($canonicalVerificationRoot, [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $canonicalRunRoot -Recurse -Force
        }
        exit 0
    }
    exit 1
}

try {
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite\templates')) | Out-Null
    [IO.File]::WriteAllText($ownerMarker, $runId, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\profiles.toml'), @'
schema_version = 1

[[profiles]]
id = "daily"
name = "Acceptance Daily"
directory = "Dairy/AcceptanceProbe/{{date:yyyy}}/{{date:yyyyMM}}"
filename = "{{date:yyyyMMdd}}.md"
template = "templates/calendar-daily.md"
collision = "open-existing"
'@, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\templates\calendar-daily.md'),
        "# {{date:yyyy-MM-dd}}`n`n{{cursor}}`n", [Text.UTF8Encoding]::new($false))
    $starterDocument = Join-Path $workspace 'README.md'
    [IO.File]::WriteAllText($starterDocument, "# Calendar Daily acceptance`n", [Text.UTF8Encoding]::new($false))
    $creationSource = Join-Path $workspace 'records\created-day.md'
    [IO.Directory]::CreateDirectory((Split-Path -Parent $creationSource)) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $workspace 'archive')) | Out-Null
    [IO.File]::WriteAllText($creationSource, "# Created on selected day`n", [Text.UTF8Encoding]::new($false))
    $creationTime = $date.AddHours(9).AddMinutes(17)
    [IO.File]::SetCreationTime($creationSource, $creationTime)
    $creationDocumentSentinel = "Calendar details open target: $runId"
    $creationSourceText = "# Created on selected day" + [Environment]::NewLine +
        [Environment]::NewLine + $creationDocumentSentinel + [Environment]::NewLine
    [IO.File]::WriteAllText($creationSource, $creationSourceText, [Text.UTF8Encoding]::new($false))
    [IO.File]::SetCreationTime($creationSource, $creationTime)
    $creationStamp = [IO.File]::GetCreationTime($creationSource).ToString('yyyy-MM-dd HH:mm')
    $creationContentHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $creationSource).Hash.ToLowerInvariant()

    if (-not (Test-Path -LiteralPath $buildReceiptPath -PathType Leaf)) {
        throw "Build receipt is missing: $buildReceiptPath"
    }
    $buildReceiptBefore = Get-Content -Raw -LiteralPath $buildReceiptPath | ConvertFrom-Json
    $executableHashBefore = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $sourceFingerprintBefore = Get-SourceFingerprint
    if ($buildReceiptBefore.build_status -ne 'PASS' -or
        $buildReceiptBefore.executable_sha256 -ne $executableHashBefore -or
        $buildReceiptBefore.source_sha256 -ne $sourceFingerprintBefore.sha256) {
        throw 'The executable/source fingerprint does not match the fresh build receipt.'
    }

    $process = Start-Process -FilePath $executable -ArgumentList @($starterDocument) -WorkingDirectory $repoRoot -PassThru
    $main = Find-MainWindow $process
    Start-Sleep -Milliseconds 350
    [void][MDLiteCalendarDailyNative]::SetWindowPos($main, [IntPtr]::Zero, 80, 80, 1440, 960,
        $SWP_NOZORDER -bor $SWP_NOACTIVATE)
    [void][MDLiteCalendarDailyNative]::ShowWindow($main, 9)
    Start-Sleep -Milliseconds 250

    $calendar = Wait-Child $main -1 'MDLite.CalendarView'
    if (-not [MDLiteCalendarDailyNative]::IsWindowVisible($calendar)) { throw 'CalendarView is not visible in the test workspace.' }
    $geometry = Get-CalendarGeometry $calendar
    $point = Get-DatePoint $geometry $date
    $dailyCounts.before = (Get-DailyFiles).Count

    $hoverReceipt = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_MOUSEMOVE, [IntPtr]::Zero,
        [MDLiteCalendarDailyNative]::MakePoint($point.x, $point.y)).ToInt64()
    $messageReceipts.hover_mousemove = $hoverReceipt
    Start-Sleep -Milliseconds 100
    $dailyCounts.after_hover = (Get-DailyFiles).Count
    [void](Capture-Status 'after_hover')

    $navY = [int][Math]::Floor($geometry.header / 2)
    $previousClick = Send-Click $calendar ([int][Math]::Floor($geometry.navWidth / 2)) $navY
    $nextClick = Send-Click $calendar ($geometry.width - [int][Math]::Floor($geometry.navWidth / 2)) $navY
    $messageReceipts.month_previous = $previousClick
    $messageReceipts.month_next = $nextClick
    Start-Sleep -Milliseconds 100
    $dailyCounts.after_month_navigation = (Get-DailyFiles).Count
    [void](Capture-Status 'after_month_navigation')

    $rightDown = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYDOWN, [IntPtr]$VK_RIGHT, [IntPtr]::Zero).ToInt64()
    $rightUp = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYUP, [IntPtr]$VK_RIGHT, [IntPtr]::Zero).ToInt64()
    $leftDown = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYDOWN, [IntPtr]$VK_LEFT, [IntPtr]::Zero).ToInt64()
    $leftUp = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYUP, [IntPtr]$VK_LEFT, [IntPtr]::Zero).ToInt64()
    $messageReceipts.focus_right_then_left = [ordered]@{ rightDown = $rightDown; rightUp = $rightUp; leftDown = $leftDown; leftUp = $leftUp }
    Start-Sleep -Milliseconds 100
    $dailyCounts.after_focus_movement = (Get-DailyFiles).Count
    [void](Capture-Status 'after_focus_movement')
    Add-Check 'hover_month_navigation_and_focus_do_not_create_daily' (
        $dailyCounts.before -eq 0 -and $dailyCounts.after_hover -eq 0 -and
        $dailyCounts.after_month_navigation -eq 0 -and $dailyCounts.after_focus_movement -eq 0) $dailyCounts

    # Re-click today's cell explicitly so Enter targets the same selected date.
    # Activation is observed through the resulting configured workspace path,
    # editor HWND/text, and native session record rather than the discarded
    # return value of a cross-process SendMessage call.
    $mouseActivation = Send-Click $calendar $point.x $point.y
    $messageReceipts.date_mouse_up = $mouseActivation
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    while ([DateTime]::UtcNow -lt $deadline -and -not (Test-Path -LiteralPath $createdFilePath -PathType Leaf)) {
        Start-Sleep -Milliseconds 50
    }
    Start-Sleep -Milliseconds 150
    $dailyFilesAfterMouse = Get-DailyFiles
    $dailyCounts.after_mouse_activation = $dailyFilesAfterMouse.Count
    $mouseContent = if (Test-Path -LiteralPath $createdFilePath -PathType Leaf) { [IO.File]::ReadAllText($createdFilePath) } else { '' }
    $editors = @(Find-Children $main $kEditor 'RICHEDIT50W')
    $visibleEditors = @($editors | Where-Object { [MDLiteCalendarDailyNative]::IsWindowVisible($_) })
    $visibleEditorText = if ($visibleEditors.Count -eq 1) { Get-WindowTextValue $visibleEditors[0] } else { '' }
    $mouseOpenPass = $dailyCounts.after_mouse_activation -eq 1 -and
        (Test-Path -LiteralPath $createdFilePath -PathType Leaf) -and
        $visibleEditors.Count -eq 1 -and $visibleEditorText.Contains($date.ToString('yyyy-MM-dd'))
    Add-Check 'mouse_up_opens_or_creates_configured_daily_path_once' $mouseOpenPass ([ordered]@{
        configured_path = $createdFilePath; daily_file_count = $dailyCounts.after_mouse_activation
        visible_editor_count = $visibleEditors.Count; editor_text_contains_date = $visibleEditorText.Contains($date.ToString('yyyy-MM-dd'))
        file_content = $mouseContent; activation_messages = $mouseActivation
    })
    [void](Capture-Status 'after_mouse_activation')

    $creationDetails = Get-CalendarDetailsText $main
    $creationListPass = $creationDetails.Contains('records/created-day.md') -and
        $creationDetails.Contains('(作成 ' + $creationStamp + ')')
    Add-Check 'calendar_creation_date_list_reports_path_and_filesystem_timestamp' $creationListPass ([ordered]@{
        expected_relative_path = 'records/created-day.md'; expected_creation_timestamp = $creationStamp
        list_text = $creationDetails
    })

    if (-not (Test-Path -LiteralPath $createdFilePath -PathType Leaf)) { throw "Mouse activation did not create the expected configured path: $createdFilePath" }
    $preservationSentinel = "Preserve existing Daily content on repeated activation: $runId"
    $preservationContent = $mouseContent.TrimEnd("`r", "`n") + "`n`n" + $preservationSentinel + "`n"
    [IO.File]::WriteAllText($createdFilePath, $preservationContent, [Text.UTF8Encoding]::new($false))

    $detailsControl = Get-CalendarDetailsControl $main
    $detailsTextForOpen = if ($detailsControl -ne [IntPtr]::Zero) { Get-WindowTextValue $detailsControl } else { '' }
    $listedPathOffset = $detailsTextForOpen.IndexOf('records/created-day.md', [StringComparison]::Ordinal)
    if ($detailsControl -ne [IntPtr]::Zero -and $listedPathOffset -ge 0) {
        [void][MDLiteCalendarDailyNative]::SendMessage(
            $detailsControl, $EM_SETSEL, [IntPtr]$listedPathOffset, [IntPtr]$listedPathOffset)
        $openListedKeyDown = [MDLiteCalendarDailyNative]::SendMessage(
            $detailsControl, $WM_KEYDOWN, [IntPtr]$VK_RETURN, [IntPtr]::Zero).ToInt64()
        [void][MDLiteCalendarDailyNative]::SendMessage(
            $detailsControl, $WM_KEYUP, [IntPtr]$VK_RETURN, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 200
    } else {
        $openListedKeyDown = -1
    }
    $openedListedEditors = @(Find-Children $main $kEditor 'RICHEDIT50W' | Where-Object {
        [MDLiteCalendarDailyNative]::IsWindowVisible($_)
    })
    $openedListedText = if ($openedListedEditors.Count -eq 1) {
        Get-WindowTextValue $openedListedEditors[0]
    } else { '' }
    $openListedPass = $openListedKeyDown -ge 0 -and
        $openedListedText.Contains($creationDocumentSentinel)
    Add-Check 'calendar_creation_list_enter_opens_the_selected_document' $openListedPass ([ordered]@{
        details_control_found = $detailsControl -ne [IntPtr]::Zero
        listed_relative_path = 'records/created-day.md'
        source_offset = $listedPathOffset
        visible_editor_count = $openedListedEditors.Count
        editor_contains_document_sentinel = $openedListedText.Contains($creationDocumentSentinel)
        document_text = $openedListedText
    })
    if ($openListedPass) {
        [void][MDLiteCalendarDailyNative]::SendMessage(
            $main, $WM_COMMAND, [IntPtr]1004, [IntPtr]::Zero)
        Start-Sleep -Milliseconds 100
    }

    [void][MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    $enterDown = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYDOWN, [IntPtr]$VK_RETURN, [IntPtr]::Zero).ToInt64()
    $enterUp = [MDLiteCalendarDailyNative]::SendMessage($calendar, $WM_KEYUP, [IntPtr]$VK_RETURN, [IntPtr]::Zero).ToInt64()
    $messageReceipts.enter_activation = [ordered]@{ keyDown = $enterDown; keyUp = $enterUp }
    Start-Sleep -Milliseconds 250
    $dailyCounts.after_enter_repeat = (Get-DailyFiles).Count
    $afterEnterContent = [IO.File]::ReadAllText($createdFilePath)
    $editorsAfterEnter = @(Find-Children $main $kEditor 'RICHEDIT50W')
    $visibleEditorsAfterEnter = @($editorsAfterEnter | Where-Object { [MDLiteCalendarDailyNative]::IsWindowVisible($_) })
    $enterPreserved = $dailyCounts.after_enter_repeat -eq 1 -and
        $afterEnterContent -ceq $preservationContent -and $visibleEditorsAfterEnter.Count -eq 1
    Add-Check 'enter_reactivation_preserves_existing_content_and_single_file' $enterPreserved ([ordered]@{
        configured_path = $createdFilePath; daily_file_count = $dailyCounts.after_enter_repeat
        content_unchanged = $afterEnterContent -ceq $preservationContent
        preservation_sentinel_found = $afterEnterContent.Contains($preservationSentinel)
        visible_editor_count = $visibleEditorsAfterEnter.Count; activation_messages = $messageReceipts.enter_activation
    })
    [void](Capture-Status 'after_enter_repeat')

    $closeInteraction = Invoke-ProductCommandFromPalette 'ファイル: タブを閉じる'
    $messageReceipts.close_source_from_palette = $closeInteraction
    Start-Sleep -Milliseconds 120

    $renamedSource = Join-Path $workspace 'records\renamed-created-day.md'
    $renameInteraction = Invoke-ProductRename 'created-day.md' $creationSource $renamedSource
    $renameDeadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $renameDetails = Get-CalendarDetailsText $main
        if ((Test-Path -LiteralPath $renamedSource -PathType Leaf) -and $renameDetails.Contains('records/renamed-created-day.md')) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $renameDeadline)
    $renameExists = Test-Path -LiteralPath $renamedSource -PathType Leaf
    $renamedCreationTime = if ($renameExists) { [IO.File]::GetCreationTime($renamedSource).ToString('yyyy-MM-dd HH:mm') } else { '' }
    $renameContentHash = if ($renameExists) { (Get-FileHash -Algorithm SHA256 -LiteralPath $renamedSource).Hash.ToLowerInvariant() } else { '' }
    $renameProvenancePass = $renameExists -and $renamedCreationTime -ceq $creationStamp -and
        $renameContentHash -ceq $creationContentHash -and
        $renameDetails.Contains('records/renamed-created-day.md') -and
        $renameDetails.Contains('(作成 ' + $creationStamp + ')') -and
        -not $renameDetails.Contains('records/created-day.md')
    Add-Check 'calendar_product_rename_updates_creation_date_list' $renameProvenancePass ([ordered]@{
        route = 'Command Search product control -> Workspace: 名前変更・移動 -> selected TreeView item -> common file dialog -> PopulateWorkspaceTree'
        command_palette = $renameInteraction
        application_refresh_message_used = $false
        before_path = 'records/created-day.md'; after_path = 'records/renamed-created-day.md'
        creation_timestamp_before = $creationStamp; creation_timestamp_after = $renamedCreationTime
        source_bytes_preserved = ($renameContentHash -ceq $creationContentHash)
        sha256_after = $renameContentHash; list_text = $renameDetails
    })

    $movedSource = Join-Path $workspace 'archive\renamed-created-day.md'
    if ($renameExists) {
        $moveInteraction = Invoke-ProductRename 'renamed-created-day.md' $renamedSource $movedSource
    } else {
        $moveInteraction = $null
    }
    $moveDeadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        $moveDetails = Get-CalendarDetailsText $main
        if ((Test-Path -LiteralPath $movedSource -PathType Leaf) -and $moveDetails.Contains('archive/renamed-created-day.md')) { break }
        Start-Sleep -Milliseconds 40
    } while ([DateTime]::UtcNow -lt $moveDeadline)
    $moveExists = Test-Path -LiteralPath $movedSource -PathType Leaf
    $movedCreationTime = if ($moveExists) { [IO.File]::GetCreationTime($movedSource).ToString('yyyy-MM-dd HH:mm') } else { '' }
    $movedContentHash = if ($moveExists) { (Get-FileHash -Algorithm SHA256 -LiteralPath $movedSource).Hash.ToLowerInvariant() } else { '' }
    $moveProvenancePass = $renameExists -and $moveExists -and $movedCreationTime -ceq $creationStamp -and
        $movedContentHash -ceq $renameContentHash -and
        $moveDetails.Contains('archive/renamed-created-day.md') -and
        $moveDetails.Contains('(作成 ' + $creationStamp + ')') -and
        -not $moveDetails.Contains('records/renamed-created-day.md')
    Add-Check 'calendar_product_move_updates_creation_date_list' $moveProvenancePass ([ordered]@{
        route = 'Command Search product control -> Workspace: 名前変更・移動 -> selected TreeView item -> common file dialog -> PopulateWorkspaceTree'
        command_palette = $moveInteraction
        application_refresh_message_used = $false
        before_path = 'records/renamed-created-day.md'; after_path = 'archive/renamed-created-day.md'
        creation_timestamp_before = $creationStamp; creation_timestamp_after = $movedCreationTime
        source_bytes_preserved = ($movedContentHash -ceq $renameContentHash)
        sha256_after = $movedContentHash; list_text = $moveDetails
    })

    $nativeStatusCapture = Capture-Status 'before_close'
    Stop-TargetProcess
    $sessionPath = Join-Path $workspace '.mdlite\.state\session.toml'
    $sessionText = if (Test-Path -LiteralPath $sessionPath -PathType Leaf) { [IO.File]::ReadAllText($sessionPath) } else { '' }
    $sessionDocuments = @([regex]::Matches($sessionText, '(?m)^path = "([^"]+)"$') | ForEach-Object { $_.Groups[1].Value })
    $activeMatch = [regex]::Match($sessionText, '(?m)^active_index = (\d+)$')
    $activeIndex = if ($activeMatch.Success) { [int]$activeMatch.Groups[1].Value } else { -1 }
    if ($activeIndex -ge 0 -and $activeIndex -lt $sessionDocuments.Count) {
        $openedFilePath = Join-Path $workspace ($sessionDocuments[$activeIndex] -replace '/', '\')
    }
    $openedPathPass = $openedFilePath -and [IO.Path]::GetFullPath($openedFilePath) -ieq [IO.Path]::GetFullPath($createdFilePath)
    Add-Check 'daily_file_is_active_document_after_repeat_activation' $openedPathPass ([ordered]@{
        session_path = $sessionPath; active_index = $activeIndex; session_documents = $sessionDocuments
        active_document_path = $openedFilePath; expected_path = $createdFilePath
    })
    $status = if (@($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -eq 0) { 'PASS_SYNTHETIC_NATIVE_MESSAGE_ROUTE' } else { 'FAIL_NATIVE_MESSAGE_ROUTE' }
} catch {
    $errorMessage = $_.Exception.Message
    $status = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
} finally {
    Stop-TargetProcess
    if ($null -eq $previousSilentSetting) {
        Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_SILENT = $previousSilentSetting
    }
}

$failed = @($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0
$executableHashAfter = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
$sourceFingerprintAfter = Get-SourceFingerprint
$sourceStable = $sourceFingerprintBefore -and $sourceFingerprintAfter -and
    $sourceFingerprintBefore.sha256 -ceq $sourceFingerprintAfter.sha256 -and
    $executableHashBefore -ceq $executableHashAfter
Add-Check 'source_and_executable_unchanged_during_ui_run' ([bool]$sourceStable) ([ordered]@{
    source_before = if ($sourceFingerprintBefore) { $sourceFingerprintBefore.sha256 } else { '' }
    source_after = if ($sourceFingerprintAfter) { $sourceFingerprintAfter.sha256 } else { '' }
    executable_before = $executableHashBefore; executable_after = $executableHashAfter
})
$creationSourceExistsAfter = Test-Path -LiteralPath $creationSource -PathType Leaf
$creationSourceHashAfter = if ($creationSourceExistsAfter) { (Get-FileHash -Algorithm SHA256 -LiteralPath $creationSource).Hash.ToLowerInvariant() } else { '' }
$renamedSourceExistsAfter = Test-Path -LiteralPath $renamedSource -PathType Leaf
$renamedSourceHashAfter = if ($renamedSourceExistsAfter) { (Get-FileHash -Algorithm SHA256 -LiteralPath $renamedSource).Hash.ToLowerInvariant() } else { '' }
$movedSourceExistsAfter = Test-Path -LiteralPath $movedSource -PathType Leaf
$movedSourceHashAfter = if ($movedSourceExistsAfter) { (Get-FileHash -Algorithm SHA256 -LiteralPath $movedSource).Hash.ToLowerInvariant() } else { '' }
$failed = @($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0
$result = [ordered]@{
    status = if ($failed -and $status -like 'PASS*') { 'FAIL_NATIVE_MESSAGE_ROUTE' } else { $status }
    pass = ($status -like 'PASS*' -and -not $failed)
    preset = $Preset
    executable = $executable
    executable_sha256_before = $executableHashBefore
    executable_sha256_after = $executableHashAfter
    build_receipt_path = $buildReceiptPath
    build_receipt = $buildReceiptBefore
    source_fingerprint_before = $sourceFingerprintBefore
    source_fingerprint_after = $sourceFingerprintAfter
    harness_prelaunch_incidents = @('Initial C# helper compile on Windows PowerShell 5.1 rejected inline out-variable declarations; MDLite was not started. Locals were predeclared and the helper compiled successfully under PowerShell 5.1 before this run.')
    configured_daily_root = $dailyRoot
    test_date_local = $date.ToString('yyyy-MM-dd')
    created_file_path = $createdFilePath
    calendar_rename_source_path = $creationSource
    calendar_rename_source_sha256_before = $creationContentHash
    calendar_rename_source_exists_after = $creationSourceExistsAfter
    calendar_rename_source_sha256_after = $creationSourceHashAfter
    calendar_rename_source_bytes_unchanged = [bool]($creationContentHash -and $creationContentHash -ceq $creationSourceHashAfter)
    calendar_rename_target_path = $renamedSource
    calendar_rename_target_exists_after = $renamedSourceExistsAfter
    calendar_rename_target_sha256_after = $renamedSourceHashAfter
    calendar_move_target_path = $movedSource
    calendar_move_target_exists_after = $movedSourceExistsAfter
    calendar_move_target_sha256_after = $movedSourceHashAfter
    active_open_file_path = $openedFilePath
    daily_file_counts = $dailyCounts
    session_documents = $sessionDocuments
    native_status_capture = $nativeStatusCapture
    status_snapshots = @($statusSnapshots)
    checks = $checks
    message_receipts = $messageReceipts
    command_palette_interactions = @($commandPaletteInteractions.ToArray())
    rename_dialog_metadata = $renameDialogMetadata
    input_method = 'Synthetic cross-process Win32 messages; Calendar activation via WM_MOUSEMOVE/WM_LBUTTONDOWN/UP and WM_KEYDOWN/UP. Rename/move via the product Command Search button, native command palette, TreeView selection, and real common Save dialog.'
    evidence_boundary = 'Automated native-message product route, file-system, editor HWND/text, status-bar and workspace session observations only. Physical input, OS keyboard queue, pixel/DPI review, and Human acceptance were not performed.'
    error = $errorMessage
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
}

$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) { [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null }
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
$result | ConvertTo-Json -Depth 8

$canonicalRunRoot = [IO.Path]::GetFullPath($runRoot)
$canonicalVerificationRoot = [IO.Path]::GetFullPath($verificationRoot).TrimEnd('\') + '\'
if ((Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and
    [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
    $canonicalRunRoot.StartsWith($canonicalVerificationRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $canonicalRunRoot -Recurse -Force
}

if (-not $result.pass) { exit 1 }
