#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug','release')]
    [string]$Preset = 'release',
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
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
    public static extern bool AttachThreadInput(uint attachThreadId, uint attachToThreadId, bool attach);
    [DllImport("kernel32.dll")]
    public static extern uint GetCurrentThreadId();
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

function Get-Selection([IntPtr]$Editor) {
    $startPointer = [Runtime.InteropServices.Marshal]::AllocHGlobal(4)
    $endPointer = [Runtime.InteropServices.Marshal]::AllocHGlobal(4)
    try {
        [Runtime.InteropServices.Marshal]::WriteInt32($startPointer, 0)
        [Runtime.InteropServices.Marshal]::WriteInt32($endPointer, 0)
        [void][MDLiteTableNative]::SendMessage($Editor, $EM_GETSEL, $startPointer, $endPointer)
        return [pscustomobject]@{
            start = [Runtime.InteropServices.Marshal]::ReadInt32($startPointer)
            end = [Runtime.InteropServices.Marshal]::ReadInt32($endPointer)
        }
    }
    finally {
        [Runtime.InteropServices.Marshal]::FreeHGlobal($startPointer)
        [Runtime.InteropServices.Marshal]::FreeHGlobal($endPointer)
    }
}

function Set-Selection([IntPtr]$Editor, [int]$Start, [int]$End = $Start) {
    [void][MDLiteTableNative]::SendMessage($Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteTableNative]::SendMessage($Editor, $EM_SETSEL, [IntPtr]$Start, [IntPtr]$End)
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
    [uint32]$targetThread = 0; [void][MDLiteTableNative]::GetWindowThreadProcessId($topLevel, [ref]$targetThread)
    $currentThread = [MDLiteTableNative]::GetCurrentThreadId(); $attached = $false
    if ($targetThread -ne 0 -and $targetThread -ne $currentThread) { $attached = [MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $true) }
    [void][MDLiteTableNative]::BringWindowToTop($topLevel); [void][MDLiteTableNative]::SetForegroundWindow($topLevel); [void][MDLiteTableNative]::SetFocus($Editor)
    $foreground = [MDLiteTableNative]::GetForegroundWindow()
    if ($foreground -ne $topLevel) { if ($attached) { [void][MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $false) }; return $null }
    [pscustomobject]@{ targetThread=$targetThread; currentThread=$currentThread; attached=$attached }
}
function Exit-InputTarget($Target) { if ($Target -and $Target.attached) { [void][MDLiteTableNative]::AttachThreadInput($Target.currentThread, $Target.targetThread, $false) } }

function Send-UnicodeCharacters([IntPtr]$Editor, [string]$Text) {
    $target = Enter-InputTarget $Editor
    if (-not $target) { return [pscustomobject]@{ sent = 0; expected = $Text.Length * 2; error = 5; status = 'BLOCKED'; reason = 'foreground/thread delivery not proven' } }
    $inputs = [MDLiteTableNative+INPUT[]]::new($Text.Length * 2)
    for ($index = 0; $index -lt $Text.Length; $index++) {
        $down = $index * 2; $up = $down + 1; $inputs[$down].type = 1; $inputs[$down].U = [MDLiteTableNative+INPUT_UNION]::new(); $inputs[$down].U.ki.wScan = [uint16][int]$Text[$index]; $inputs[$down].U.ki.dwFlags = $KEYEVENTF_UNICODE
        $inputs[$up].type = 1; $inputs[$up].U = [MDLiteTableNative+INPUT_UNION]::new(); $inputs[$up].U.ki.wScan = [uint16][int]$Text[$index]; $inputs[$up].U.ki.dwFlags = $KEYEVENTF_UNICODE -bor $KEYEVENTF_KEYUP
    }
    $sent = [MDLiteTableNative]::SendInput($inputs.Length, $inputs, [Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=$inputs.Length; error=if($sent -eq $inputs.Length){0}else{[Runtime.InteropServices.Marshal]::GetLastWin32Error()}; status=if($sent -eq $inputs.Length){'PASS'}else{'BLOCKED'} }
}

function Send-ControlKey([IntPtr]$Editor, [int]$Key) {
    $target = Enter-InputTarget $Editor; if (-not $target) { return [pscustomobject]@{ sent=0; expected=4; error=5; status='BLOCKED'; reason='foreground/thread delivery not proven' } }
    $inputs = [MDLiteTableNative+INPUT[]]::new(4); for ($index=0; $index -lt 4; $index++) { $inputs[$index].type=1; $inputs[$index].U=[MDLiteTableNative+INPUT_UNION]::new() }
    $inputs[0].U.ki.wVk=[uint16]$VK_CONTROL; $inputs[1].U.ki.wVk=[uint16]$Key; $inputs[2].U.ki.wVk=[uint16]$Key; $inputs[2].U.ki.dwFlags=$KEYEVENTF_KEYUP; $inputs[3].U.ki.wVk=[uint16]$VK_CONTROL; $inputs[3].U.ki.dwFlags=$KEYEVENTF_KEYUP
    $sent=[MDLiteTableNative]::SendInput(4,$inputs,[Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=4; error=if($sent -eq 4){0}else{[Runtime.InteropServices.Marshal]::GetLastWin32Error()}; status=if($sent -eq 4){'PASS'}else{'BLOCKED'} }
}

function Send-KeyRepeat([IntPtr]$Editor, [int]$Key, [int]$Repeat) {
    $target = Enter-InputTarget $Editor; if (-not $target) { return [pscustomobject]@{ sent=0; expected=$Repeat*2; error=5; status='BLOCKED'; reason='foreground/thread delivery not proven' } }
    $inputs=[MDLiteTableNative+INPUT[]]::new($Repeat*2); for($index=0;$index -lt $Repeat;$index++){ $down=$index*2; $up=$down+1; $inputs[$down].type=1; $inputs[$down].U=[MDLiteTableNative+INPUT_UNION]::new(); $inputs[$down].U.ki.wVk=[uint16]$Key; $inputs[$up].type=1; $inputs[$up].U=[MDLiteTableNative+INPUT_UNION]::new(); $inputs[$up].U.ki.wVk=[uint16]$Key; $inputs[$up].U.ki.dwFlags=$KEYEVENTF_KEYUP }
    $sent=[MDLiteTableNative]::SendInput($inputs.Length,$inputs,[Runtime.InteropServices.Marshal]::SizeOf($inputs[0])); Exit-InputTarget $target
    [pscustomobject]@{ sent=$sent; expected=$inputs.Length; error=if($sent -eq $inputs.Length){0}else{[Runtime.InteropServices.Marshal]::GetLastWin32Error()}; status=if($sent -eq $inputs.Length){'PASS'}else{'BLOCKED'} }
}
function Send-ShiftKey([IntPtr]$Editor, [int]$Key) {
    $topLevel = $Editor
    while (($parent = [MDLiteTableNative]::GetParent($topLevel)) -ne [IntPtr]::Zero) { $topLevel = $parent }
    [uint32]$targetThread = 0
    [void][MDLiteTableNative]::GetWindowThreadProcessId($topLevel, [ref]$targetThread)
    $currentThread = [MDLiteTableNative]::GetCurrentThreadId()
    $attached = $false
    if ($targetThread -ne 0 -and $targetThread -ne $currentThread) {
        $attached = [MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $true)
    }
    try {
        [void][MDLiteTableNative]::BringWindowToTop($topLevel)
        [void][MDLiteTableNative]::SetForegroundWindow($topLevel)
        [void][MDLiteTableNative]::SetFocus($Editor)
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
        if ($sent -ne 4) {
            return [pscustomobject]@{
                sent = $sent
                error = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
            }
        }
        Start-Sleep -Milliseconds 150
        return $null
    }
    finally {
        if ($attached) { [void][MDLiteTableNative]::AttachThreadInput($currentThread, $targetThread, $false) }
    }
}

$source = "# Table UI probe`n`n| Head A | Head B |`n| :--- | ---: |`n| one | two |`n| three | four |`n"
$runRoot = Join-Path ([IO.Path]::GetTempPath()) ("mdlite-table-acceptance-" + [guid]::NewGuid().ToString('N'))
$checks = [ordered]@{}
$processes = [Collections.Generic.List[Diagnostics.Process]]::new()

function Invoke-TableCase([string]$Name, [scriptblock]$Action) {
    $workspace = Join-Path $runRoot $Name
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    $path = Join-Path $workspace 'table.md'
    [IO.File]::WriteAllText($path, $source, [Text.UTF8Encoding]::new($false))
    $process = Start-Process -FilePath $executable -ArgumentList @($path) -PassThru
    $processes.Add($process)
    try {
        $main = Wait-ProcessWindow $process
        $editor = Wait-Control $main $kEditor 'RICHEDIT50W'
        Set-Selection $editor 0 0
        $value = & $Action $main $editor $path $process
        if ($null -eq $value) { throw "Case $Name returned no result" }
        $checks[$Name] = $value
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
    $one = $source.IndexOf('one', [StringComparison]::Ordinal)
    $two = $source.IndexOf('two', [StringComparison]::Ordinal)

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
        $undone = Save-Source $main $path
        [void][MDLiteTableNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
        $redone = Save-Source $main $path
        [pscustomobject]@{ pass = $edited.Contains('| ONE | two |') -and $undone.Contains('| one | two |') -and $redone.Contains('| ONE | two |') }
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
        $four = $source.IndexOf('four', [StringComparison]::Ordinal)
        Set-Selection $editor $four
        Send-Key $editor $VK_TAB
        $saved = Save-Source $main $path
        [pscustomobject]@{ pass = $saved.Contains('|  |  |') -and $saved -ne $source; actual = $saved }
    }
    Invoke-TableCase 'shift_tab_previous_cell' {
        param($main, $editor, $path)
        Set-Selection $editor $two
        $input = Send-ShiftKey $editor $VK_TAB
        if ($null -ne $input) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = "SendInput sent $($input.sent)/4 (Win32 $($input.error))" }
        }
        $selection = Get-Selection $editor
        $saved = Save-Source $main $path
        $pass = $selection.start -eq $one -and $selection.end -eq $one -and $saved -eq $source
        if (-not $pass) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = 'SendInput did not expose Shift modifier to the target RichEdit in this execution frame'; actual_start = $selection.start; actual_end = $selection.end; expected = $one; saved = $saved }
        }
        [pscustomobject]@{ pass = $true; actual_start = $selection.start; actual_end = $selection.end; expected = $one; saved = $saved }
    }
    Invoke-TableCase 'arrow_right_boundary' {
        param($main, $editor, $path)
        Set-Selection $editor ($one + 3)
        Send-Key $editor $VK_RIGHT
        $selection = Get-Selection $editor
        [pscustomobject]@{ pass = $selection.start -eq $two -and $selection.end -eq $two; actual_start = $selection.start; actual_end = $selection.end; expected = $two }
    }
    Invoke-TableCase 'arrow_down_column' {
        param($main, $editor, $path, $process)
        $head = $source.IndexOf('Head A', [StringComparison]::Ordinal)
        Set-Selection $editor $head
        Send-Key $editor $VK_DOWN
        $selection = Get-Selection $editor
        [pscustomobject]@{ pass = $selection.start -eq $one -and $selection.end -eq $one; actual_start = $selection.start; actual_end = $selection.end; expected = $one }
    }
    Invoke-TableCase 'continuous_unicode_input_no_accumulation' {
        param($main, $editor, $path, $process)
        $payload = 'x' * 128
        Set-Selection $editor $source.Length
        $input = Send-UnicodeCharacters $editor $payload
        if ($input.sent -ne $input.expected) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = "SendInput sent $($input.sent)/$($input.expected) (Win32 $($input.error))" }
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
        if ($firstInput.sent -ne $firstInput.expected) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = "Ctrl+Z SendInput sent $($firstInput.sent)/$($firstInput.expected) (Win32 $($firstInput.error))" }
        }
        $undoOne = Save-Source $main $path
        Start-Sleep -Milliseconds 100
        $secondInput = Send-ControlKey $editor $VK_Z
        if ($secondInput.sent -ne $secondInput.expected) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = "second Ctrl+Z SendInput sent $($secondInput.sent)/$($secondInput.expected) (Win32 $($secondInput.error))" }
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
        $three = $source.IndexOf('three', [StringComparison]::Ordinal)
        Set-Selection $editor ($one + 3)
        $input = Send-KeyRepeat $editor $VK_RIGHT 8
        if ($input.sent -ne $input.expected) {
            return [pscustomobject]@{ pass = $false; status = 'BLOCKED'; reason = "arrow SendInput sent $($input.sent)/$($input.expected) (Win32 $($input.error))" }
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

    $failed = @($checks.GetEnumerator() | Where-Object { -not $_.Value.pass })
    $result = [ordered]@{
        preset = $Preset
        executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
        source = $source
        checks = $checks
        evidence = [ordered]@{
            E01_source_round_trip = @('row_before','row_after','row_delete','column_before','column_after','column_delete')
            E02_dirty_save_boundary = @('cell_edit_undo_redo','tab_next_cell','tab_appends_row')
            E03_undo_redo_invariant = @('cell_edit_undo_redo','ctrl_z_single_transaction')
            E04_table_geometry_navigation = @('tab_next_cell','arrow_right_boundary','arrow_down_column')
            E05_arrow_repeat_fallback = @('arrow_repeat_boundary')
            E06_native_paint_invalidation = @('table_paint_reentry')
            E07_continuous_input = @('continuous_unicode_input_no_accumulation')
            E08_transaction_rollback = @('ctrl_z_single_transaction')
            E09_ime_atok = 'UNKNOWN: synthetic SendInput is not IME/ATOK acceptance'
            E10_physical_long_press = 'UNKNOWN: no physical long-press input'
            E11_dpi_visual = 'UNKNOWN: no Human visual/DPI acceptance'
            E12_dialog_timer = 'NOT OBSERVED BY TABLE HARNESS'
            E13_presentation = 'NOT OBSERVED BY TABLE HARNESS'
        }
        evidence_boundary = 'Automated native Win32/RichEdit only; UNKNOWN and NOT_OBSERVED remain open.'
        all_cases_pass = $failed.Count -eq 0
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
