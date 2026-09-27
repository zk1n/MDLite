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
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }

$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'build\verification'))
$runRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot "calendar-daily-$runId"))
$workspace = Join-Path $runRoot 'workspace'
$ownerMarker = Join-Path $runRoot '.calendar-daily-acceptance-owner'
if (-not $OutputPath) { $OutputPath = Join-Path $verificationRoot "calendar-daily-acceptance-$runId.json" }

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
$dailyCounts = [ordered]@{}
$openedFilePath = $null
$sessionDocuments = @()
$nativeStatusCapture = $null
$messageReceipts = [ordered]@{}
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
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll", EntryPoint = "SendMessageW")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll", EntryPoint = "SendMessageW", CharSet = CharSet.Unicode)] public static extern IntPtr SendMessageGetText(IntPtr window, uint message, IntPtr wparam, StringBuilder text);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    public static IntPtr MakePoint(int x, int y) {
        uint packed = (uint)(ushort)(short)x | ((uint)(ushort)(short)y << 16);
        return new IntPtr(unchecked((int)packed));
    }
}
'@

$WM_GETTEXT = 0x000D
$WM_GETTEXTLENGTH = 0x000E
$WM_CLOSE = 0x0010
$WM_COMMAND = 0x0111
$EM_SETSEL = 0x00B1
$WM_MOUSEMOVE = 0x0200
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$WM_SETFOCUS = 0x0007
$VK_LEFT = 0x25
$VK_RIGHT = 0x27
$VK_RETURN = 0x0D
$SWP_NOZORDER = 0x0004
$SWP_NOACTIVATE = 0x0010
$kCalendarView = 118
$kEditor = 102
$testRefreshWorkspaceTreeMessage = 0x8042
$expectedEditorId = $kEditor

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

    $renamedSource = Join-Path $workspace 'records\renamed-created-day.md'
    Rename-Item -LiteralPath $creationSource -NewName 'renamed-created-day.md'
    $renameRefreshResult = [MDLiteCalendarDailyNative]::SendMessage(
        $main, $testRefreshWorkspaceTreeMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    Start-Sleep -Milliseconds 100
    $renamedCreationTime = [IO.File]::GetCreationTime($renamedSource).ToString('yyyy-MM-dd HH:mm')
    $renameDetails = Get-CalendarDetailsText $main
    $renameProvenancePass = $renameRefreshResult -eq 1 -and $renamedCreationTime -ceq $creationStamp -and
        $renameDetails.Contains('records/renamed-created-day.md') -and
        $renameDetails.Contains('(作成 ' + $creationStamp + ')') -and
        -not $renameDetails.Contains('records/created-day.md')
    Add-Check 'calendar_creation_provenance_survives_rename' $renameProvenancePass ([ordered]@{
        file_system_rename = $true; application_refresh_message_accepted = ($renameRefreshResult -eq 1)
        before_path = 'records/created-day.md'; after_path = 'records/renamed-created-day.md'
        creation_timestamp_before = $creationStamp; creation_timestamp_after = $renamedCreationTime
        list_text = $renameDetails
    })

    $movedSource = Join-Path $workspace 'archive\renamed-created-day.md'
    Move-Item -LiteralPath $renamedSource -Destination $movedSource
    $moveRefreshResult = [MDLiteCalendarDailyNative]::SendMessage(
        $main, $testRefreshWorkspaceTreeMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
    Start-Sleep -Milliseconds 100
    $movedCreationTime = [IO.File]::GetCreationTime($movedSource).ToString('yyyy-MM-dd HH:mm')
    $moveDetails = Get-CalendarDetailsText $main
    $moveProvenancePass = $moveRefreshResult -eq 1 -and $movedCreationTime -ceq $creationStamp -and
        $moveDetails.Contains('archive/renamed-created-day.md') -and
        $moveDetails.Contains('(作成 ' + $creationStamp + ')') -and
        -not $moveDetails.Contains('records/renamed-created-day.md')
    Add-Check 'calendar_creation_provenance_survives_move' $moveProvenancePass ([ordered]@{
        file_system_move = $true; application_refresh_message_accepted = ($moveRefreshResult -eq 1)
        before_path = 'records/renamed-created-day.md'; after_path = 'archive/renamed-created-day.md'
        creation_timestamp_before = $creationStamp; creation_timestamp_after = $movedCreationTime
        list_text = $moveDetails
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
$result = [ordered]@{
    status = if ($failed -and $status -like 'PASS*') { 'FAIL_NATIVE_MESSAGE_ROUTE' } else { $status }
    pass = ($status -like 'PASS*' -and -not $failed)
    preset = $Preset
    executable = $executable
    executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    configured_daily_root = $dailyRoot
    test_date_local = $date.ToString('yyyy-MM-dd')
    created_file_path = $createdFilePath
    active_open_file_path = $openedFilePath
    daily_file_counts = $dailyCounts
    session_documents = $sessionDocuments
    native_status_capture = $nativeStatusCapture
    status_snapshots = @($statusSnapshots)
    checks = $checks
    message_receipts = $messageReceipts
    input_method = 'Synchronous cross-process SendMessage to the CalendarView HWND for WM_MOUSEMOVE, WM_LBUTTONDOWN/WM_LBUTTONUP and WM_KEYDOWN/WM_KEYUP; returns are recorded but are not independent input-receipt proof.'
    evidence_boundary = 'Automated native message-route, file-system, editor HWND/text, status-bar and workspace session observations only. No physical mouse/keyboard, OS input-queue delivery, pixel capture, DPI visual review, or Human acceptance was performed.'
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
