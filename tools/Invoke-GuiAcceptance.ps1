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
if (-not $OutputPath) { $OutputPath = Join-Path $repoRoot "build\verification\gui-acceptance-$Preset.json" }
$previousCrashUiSetting = $env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;

public static class MDLiteNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int left; public int top; public int right; public int bottom; }
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
    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, string lparam);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, StringBuilder lparam);
    [DllImport("user32.dll")]
    public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")]
    public static extern bool GetClientRect(IntPtr window, out RECT rect);
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
}
'@

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_SETTEXT = 0x000C
$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$WM_GETFONT = 0x0031
$WM_CHAR = 0x0102
$WM_SETFOCUS = 0x0007
$WM_SIZE = 0x0005
$WM_UNDO = 0x0304
$EM_SETSEL = 0x00B1
$EM_GETSEL = 0x00B0
$EM_REDO = 0x0454
$BM_SETCHECK = 0x00F1
$BM_CLICK = 0x00F5
$LVM_GETITEMCOUNT = 0x1004
$TVM_GETCOUNT = 0x1105
$LB_GETCOUNT = 0x018B
$BST_UNCHECKED = 0
$BST_CHECKED = 1
$IDOK = 1
$IDCANCEL = 2

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
    throw "Control was not created: id=$Id class=$ClassName"
}

function Wait-VisibleControl([IntPtr]$Parent, [int]$Id, [string]$ClassName = '', [int]$TimeoutMs = 10000) {
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
    throw "Visible control was not created: id=$Id class=$ClassName"
}

function Get-WindowText([IntPtr]$Window) {
    $length = [MDLiteNative]::SendMessage($Window, $WM_GETTEXTLENGTH, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    $text = New-Object Text.StringBuilder ($length + 1)
    [void][MDLiteNative]::SendMessage($Window, $WM_GETTEXT, [IntPtr]($length + 1), $text)
    return $text.ToString()
}

function Send-Characters([IntPtr]$Editor, [string]$Text) {
    [void][MDLiteNative]::SendMessage($Editor, $WM_SETFOCUS, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($Editor, $EM_SETSEL, [IntPtr](-1), [IntPtr](-1))
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
[IO.File]::WriteAllBytes($pixel, [Convert]::FromBase64String(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+Q4j4WQAAAABJRU5ErkJggg=='))
$process = $null
$checks = [ordered]@{}
$typed = ' 日本語compact'
$firstInitialHash = Get-FileFingerprint $first

try {
    $process = Start-Process -FilePath $executable -ArgumentList @($first) -PassThru
    $main = Wait-ProcessWindow $process
    $editor = Wait-Control $main 102 'RICHEDIT50W'
    $checks.main_window = $true
    $checks.initial_view = ((Get-WindowText $editor) -replace "`r`n", "`n") -eq $initial
    $checks.runtime_environment = Get-NativeRuntimeSnapshot $process $main $editor $first $firstInitialHash $settingsFile
    # Re-enter native layout and paint synchronously; this is OS-native message evidence, not Human visual acceptance.
    $checks.paint_layout_reentry = Invoke-PaintLayoutReentryProbe $main $editor $process 64

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
    $checks.outline_tree_items = [MDLiteNative]::SendMessage(
        $outlineTree, $TVM_GETCOUNT, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32() -gt 0
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1066, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $checks.outline_pane_collapsed = -not [MDLiteNative]::IsWindowVisible($outlineTree)
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1066, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $checks.outline_pane_restored = [MDLiteNative]::IsWindowVisible($outlineTree)

    $calendar = Find-ChildClassWindow $main 'SysMonthCal32'
    $checks.calendar_control = $calendar -ne [IntPtr]::Zero
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1026, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $checks.calendar_visible = $calendar -ne [IntPtr]::Zero -and [MDLiteNative]::IsWindowVisible($calendar)
    [void][MDLiteNative]::PostMessage($main, $WM_COMMAND, [IntPtr]1026, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
    $checks.calendar_hidden = $calendar -eq [IntPtr]::Zero -or -not [MDLiteNative]::IsWindowVisible($calendar)

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
    Start-Sleep -Milliseconds 150
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $checks.quick_open_editor_after_selection = ((Get-WindowText $editor) -replace "`r`n", "`n")
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
    Start-Sleep -Milliseconds 150
    $editor = Wait-VisibleControl $main 102 'RICHEDIT50W'
    $checks.quick_open_returned_to_first = ((Get-WindowText $editor) -replace "`r`n", "`n").StartsWith('CaseToken')

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
    [void][MDLiteNative]::SendMessage($editor, $WM_UNDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.image_object_adjacent_edit_undo = [IO.File]::ReadAllText($first) -eq $imageSource
    [void][MDLiteNative]::SendMessage($editor, $EM_REDO, [IntPtr]::Zero, [IntPtr]::Zero)
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    $checks.image_object_adjacent_edit_redo = [IO.File]::ReadAllText($first) -eq $imageTailSource

    # Two same-line images have no text anchor that can identify their native
    # object markers after deletion. They must remain raw Markdown through the
    # real refresh-and-save path instead of replacing just their leading '!'.
    $adjacentImageMarkup = '![one](pixel.png)![two](pixel.png)'
    Send-Characters $editor $adjacentImageMarkup
    Start-Sleep -Milliseconds 900
    [void][MDLiteNative]::SendMessage($main, $WM_COMMAND, [IntPtr]1005, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 150
    $adjacentImageSource = [IO.File]::ReadAllText($first)
    $checks.adjacent_images_source_actual = $adjacentImageSource
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
    $failedChecks = @($checks.GetEnumerator() | Where-Object {
        $_.Value -is [bool] -and -not $_.Value
    })
    $checks['pass'] = $failedChecks.Count -eq 0
    $result = [pscustomobject]$checks
    $directory = Split-Path -Parent $OutputPath
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $result | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $result | ConvertTo-Json -Depth 4
    if (-not $result.pass) { exit 1 }
}
finally {
    if ($process -and -not $process.HasExited) {
        $main = [MDLiteNative]::FindWindow('MDLite.MainWindow', $null)
        if ($main -ne [IntPtr]::Zero) { [void][MDLiteNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero) }
        if (-not $process.WaitForExit(3000)) { $process.Kill() }
        $process.Dispose()
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
