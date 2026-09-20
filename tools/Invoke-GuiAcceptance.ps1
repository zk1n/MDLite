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
}
'@

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_SETTEXT = 0x000C
$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$WM_CHAR = 0x0102
$WM_SETFOCUS = 0x0007
$WM_UNDO = 0x0304
$EM_SETSEL = 0x00B1
$EM_GETSEL = 0x00B0
$EM_REDO = 0x0454
$BM_SETCHECK = 0x00F1
$BM_CLICK = 0x00F5
$LVM_GETITEMCOUNT = 0x1004
$BST_UNCHECKED = 0
$BST_CHECKED = 1

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

$runRoot = Join-Path ([IO.Path]::GetTempPath()) ("mdlite-gui-acceptance-" + [guid]::NewGuid().ToString('N'))
$workspace = Join-Path $runRoot 'workspace'
[IO.Directory]::CreateDirectory($workspace) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
[IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
    "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
$first = Join-Path $workspace 'first.md'
$second = Join-Path $workspace 'second.md'
$pixel = Join-Path $workspace 'pixel.png'
$initial = 'CaseToken casetoken WorkspaceHit'
[IO.File]::WriteAllText($first, $initial, [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText($second, 'second WorkspaceHit', [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllBytes($pixel, [Convert]::FromBase64String(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+Q4j4WQAAAABJRU5ErkJggg=='))
$process = $null
$checks = [ordered]@{}
$typed = ' 日本語compact'

try {
    $process = Start-Process -FilePath $executable -ArgumentList @($first) -PassThru
    $main = Wait-ProcessWindow $process
    $editor = Wait-Control $main 102 'RICHEDIT50W'
    $checks.main_window = $true
    $checks.initial_view = (Get-WindowText $editor) -eq $initial

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
        $imageUndoSource.StartsWith($beforeImage)
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
