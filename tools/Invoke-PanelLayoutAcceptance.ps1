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
$runRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot "panel-layout-$runId"))
$workspace = Join-Path $runRoot 'workspace'
$ownerMarker = Join-Path $runRoot '.panel-layout-acceptance-owner'
if (-not $OutputPath) { $OutputPath = Join-Path $verificationRoot "panel-layout-acceptance-$runId.json" }

$checks = [ordered]@{}
$process = $null
$main = [IntPtr]::Zero
$errorMessage = $null
$status = 'BLOCKED'
$beforeRestartLayout = $null
$executableHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;

public static class MDLitePanelNative {
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    public delegate bool EnumChildWindowsProc(IntPtr window, IntPtr parameter);
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    public struct POINT { public int X, Y; }
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumChildWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder name, int capacity);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowText(IntPtr window, StringBuilder text, int capacity);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr window, out RECT rect);
    [DllImport("user32.dll")] public static extern bool ScreenToClient(IntPtr window, ref POINT point);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", EntryPoint = "SendMessageW")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr window, IntPtr insertAfter, int x, int y, int width, int height, uint flags);
    public static IntPtr MakePoint(int x, int y) {
        uint packed = (uint)(ushort)(short)x | ((uint)(ushort)(short)y << 16);
        return new IntPtr(unchecked((int)packed));
    }
}
'@

$WM_COMMAND = 0x0111
$WM_SIZE = 0x0005
$WM_CLOSE = 0x0010
$WM_LBUTTONDOWN = 0x0201
$WM_MOUSEMOVE = 0x0200
$WM_LBUTTONUP = 0x0202
$SWP_NOZORDER = 0x0004
$SWP_NOACTIVATE = 0x0010
$kWorkspaceTree = 100
$kOutline = 103
$kCalendar = 118
$kPanelHeaderExplorer = 1072
$kPanelHeaderCalendar = 1073
$kPanelHeaderOutline = 1074
$kPanelHeaderGit = 1075
$kMoveCalendarRightTop = 1082
$kViewCalendar = 1026
$kViewWorkspacePane = 1065
$kViewResetPanels = 1067
$kViewGitPane = 1096

function Add-Check([string]$Name, [bool]$Pass, $Details = $null) {
    $checks[$Name] = [ordered]@{ status = if ($Pass) { 'PASS' } else { 'FAIL' }; details = $Details }
}

function Find-MainWindow([Diagnostics.Process]$Target, [int]$TimeoutMs = 15000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Target.Refresh()
        if ($Target.HasExited) { throw "MDLite exited with code $($Target.ExitCode) before its main window appeared." }
        $script:foundMain = [IntPtr]::Zero
        $callback = [MDLitePanelNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$parameter)
            [uint32]$ownerPid = 0
            [void][MDLitePanelNative]::GetWindowThreadProcessId($window, [ref]$ownerPid)
            if ($ownerPid -ne $script:targetPid) { return $true }
            $className = New-Object Text.StringBuilder 128
            [void][MDLitePanelNative]::GetClassName($window, $className, $className.Capacity)
            if ($className.ToString() -eq 'MDLite.MainWindow') {
                $script:foundMain = $window
                return $false
            }
            return $true
        }
        $script:targetPid = [uint32]$Target.Id
        [void][MDLitePanelNative]::EnumWindows($callback, [IntPtr]::Zero)
        if ($script:foundMain -ne [IntPtr]::Zero) { return $script:foundMain }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Main window was not created for process $($Target.Id)."
}

function Find-Control([IntPtr]$Parent, [int]$Id) {
    $script:foundControl = [IntPtr]::Zero
    $callback = [MDLitePanelNative+EnumChildWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLitePanelNative]::GetDlgCtrlID($window) -eq $script:targetControlId) {
            $script:foundControl = $window
            return $false
        }
        return $true
    }
    $script:targetControlId = $Id
    [void][MDLitePanelNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    return $script:foundControl
}

function Wait-Control([IntPtr]$Parent, [int]$Id, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $control = Find-Control $Parent $Id
        if ($control -ne [IntPtr]::Zero) { return $control }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Native child control was not created: id=$Id."
}

function Get-Bounds([IntPtr]$Window) {
    [MDLitePanelNative+RECT]$rect = New-Object MDLitePanelNative+RECT
    if (-not [MDLitePanelNative]::GetWindowRect($Window, [ref]$rect)) { throw 'GetWindowRect failed.' }
    return [pscustomobject]@{
        left = $rect.Left; top = $rect.Top; right = $rect.Right; bottom = $rect.Bottom
        width = $rect.Right - $rect.Left; height = $rect.Bottom - $rect.Top
        centerX = [int](($rect.Left + $rect.Right) / 2); centerY = [int](($rect.Top + $rect.Bottom) / 2)
        visible = [MDLitePanelNative]::IsWindowVisible($Window)
    }
}

function Invoke-Command([IntPtr]$Window, [int]$Command) {
    [void][MDLitePanelNative]::SendMessage($Window, $WM_COMMAND, [IntPtr]$Command, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 100
}

function Wait-LayoutFile([string]$Path) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        if (Test-Path -LiteralPath $Path -PathType Leaf) { return [IO.File]::ReadAllText($Path) }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Workspace panel layout was not persisted: $Path"
}

function Stop-TargetProcess {
    if ($script:process -and -not $script:process.HasExited) {
        if ($script:main -ne [IntPtr]::Zero) {
            [void][MDLitePanelNative]::PostMessage($script:main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
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
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
    [IO.File]::WriteAllText($ownerMarker, $runId, [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
        "schema_version = 1`nauto_save = false`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $workspace 'README.md'),
        "# Panel layout acceptance`n`nSynthetic workspace for native panel layout messages.`n",
        [Text.UTF8Encoding]::new($false))

    $process = Start-Process -FilePath $executable -ArgumentList @($workspace) -WorkingDirectory $repoRoot -PassThru
    $main = Find-MainWindow $process
    Start-Sleep -Milliseconds 400
    if (-not [MDLitePanelNative]::SetWindowPos($main, [IntPtr]::Zero, 80, 80, 1440, 960,
            $SWP_NOZORDER -bor $SWP_NOACTIVATE)) { throw 'Could not size the native test window.' }
    [void][MDLitePanelNative]::SendMessage($main, $WM_SIZE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 250

    $explorerHeader = Wait-Control $main $kPanelHeaderExplorer
    $calendarHeader = Wait-Control $main $kPanelHeaderCalendar
    $outlineHeader = Wait-Control $main $kPanelHeaderOutline
    $gitHeader = Wait-Control $main $kPanelHeaderGit
    $tree = Wait-Control $main $kWorkspaceTree
    $calendar = Wait-Control $main $kCalendar
    $outline = Wait-Control $main $kOutline
    $explorerBounds = Get-Bounds $explorerHeader
    $calendarBounds = Get-Bounds $calendarHeader
    $outlineBounds = Get-Bounds $outlineHeader
    $gitBounds = Get-Bounds $gitHeader
    $allDefaultVisible = $explorerBounds.visible -and $calendarBounds.visible -and
        $outlineBounds.visible -and $gitBounds.visible -and
        [MDLitePanelNative]::IsWindowVisible($tree) -and
        [MDLitePanelNative]::IsWindowVisible($calendar) -and
        [MDLitePanelNative]::IsWindowVisible($outline)
    $defaultSlots = $explorerBounds.centerX -lt $outlineBounds.centerX -and
        $calendarBounds.centerX -lt $gitBounds.centerX -and
        $explorerBounds.centerX -eq $calendarBounds.centerX -and
        $outlineBounds.centerX -eq $gitBounds.centerX -and
        $explorerBounds.centerY -lt $calendarBounds.centerY -and
        $outlineBounds.centerY -lt $gitBounds.centerY
    Add-Check 'four_panel_default_slots_and_controls' ($allDefaultVisible -and $defaultSlots) ([ordered]@{
        explorer = $explorerBounds; calendar = $calendarBounds; outline = $outlineBounds; git = $gitBounds
        controls = [ordered]@{ workspaceTree = [MDLitePanelNative]::IsWindowVisible($tree); calendar = [MDLitePanelNative]::IsWindowVisible($calendar); outline = [MDLitePanelNative]::IsWindowVisible($outline) }
    })

    Invoke-Command $main $kMoveCalendarRightTop
    $movedCalendar = Get-Bounds $calendarHeader
    $movedOutline = Get-Bounds $outlineHeader
    $movePass = $movedCalendar.centerX -gt $movedOutline.centerX -and
        $movedCalendar.centerY -lt $movedOutline.centerY
    Add-Check 'move_panel_command_swaps_slots' $movePass ([ordered]@{ calendar = $movedCalendar; outline = $movedOutline; command = $kMoveCalendarRightTop })

    $layoutPath = Join-Path $workspace '.mdlite\.state\panel-layout.toml'
    $layoutAfterMove = Wait-LayoutFile $layoutPath
    Add-Check 'move_persisted_inside_workspace' ($layoutAfterMove.Contains('id = "calendar"') -and $layoutAfterMove.Contains('slot = "right_top"')) ([ordered]@{ path = $layoutPath; content = $layoutAfterMove })

    Invoke-Command $main $kViewWorkspacePane
    $explorerCollapsed = -not [MDLitePanelNative]::IsWindowVisible($explorerHeader) -and -not [MDLitePanelNative]::IsWindowVisible($tree)
    Invoke-Command $main $kViewWorkspacePane
    $explorerRestored = [MDLitePanelNative]::IsWindowVisible($explorerHeader) -and [MDLitePanelNative]::IsWindowVisible($tree)
    Add-Check 'explorer_collapse_and_restore' ($explorerCollapsed -and $explorerRestored) ([ordered]@{ collapsed = $explorerCollapsed; restored = $explorerRestored })

    Invoke-Command $main 1066
    $outlineCollapsed = -not [MDLitePanelNative]::IsWindowVisible($outlineHeader) -and -not [MDLitePanelNative]::IsWindowVisible($outline)
    Invoke-Command $main 1066
    $outlineRestored = [MDLitePanelNative]::IsWindowVisible($outlineHeader) -and [MDLitePanelNative]::IsWindowVisible($outline)
    Add-Check 'outline_collapse_and_restore' ($outlineCollapsed -and $outlineRestored) ([ordered]@{ collapsed = $outlineCollapsed; restored = $outlineRestored })

    Invoke-Command $main $kViewCalendar
    $calendarHidden = -not [MDLitePanelNative]::IsWindowVisible($calendarHeader) -and -not [MDLitePanelNative]::IsWindowVisible($calendar)
    Invoke-Command $main $kViewCalendar
    $calendarRestored = [MDLitePanelNative]::IsWindowVisible($calendarHeader) -and [MDLitePanelNative]::IsWindowVisible($calendar)
    Add-Check 'calendar_hide_and_restore' ($calendarHidden -and $calendarRestored) ([ordered]@{ hidden = $calendarHidden; restored = $calendarRestored })

    Invoke-Command $main $kViewGitPane
    $gitHidden = -not [MDLitePanelNative]::IsWindowVisible($gitHeader)
    Invoke-Command $main $kViewGitPane
    $gitRestored = [MDLitePanelNative]::IsWindowVisible($gitHeader)
    Add-Check 'git_hide_and_restore' ($gitHidden -and $gitRestored) ([ordered]@{ hidden = $gitHidden; restored = $gitRestored })

    # Resize the main client, then route a synthetic splitter drag through the
    # same native window messages used by the interactive splitter handler.
    [MDLitePanelNative+RECT]$clientRect = New-Object MDLitePanelNative+RECT
    if (-not [MDLitePanelNative]::GetClientRect($main, [ref]$clientRect)) { throw 'GetClientRect failed.' }
    $clientWidthBefore = $clientRect.Right - $clientRect.Left
    if (-not [MDLitePanelNative]::SetWindowPos($main, [IntPtr]::Zero, 80, 80, 1540, 960,
            $SWP_NOZORDER -bor $SWP_NOACTIVATE)) { throw 'Could not resize the native test window.' }
    [void][MDLitePanelNative]::SendMessage($main, $WM_SIZE, [IntPtr]::Zero, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 200
    if (-not [MDLitePanelNative]::GetClientRect($main, [ref]$clientRect)) { throw 'GetClientRect failed after resize.' }
    $clientWidthAfter = $clientRect.Right - $clientRect.Left
    Add-Check 'window_resize_routes_wm_size' ($clientWidthAfter -gt $clientWidthBefore) ([ordered]@{ clientWidthBefore = $clientWidthBefore; clientWidthAfter = $clientWidthAfter })

    $explorerBoundsBeforeSplitter = Get-Bounds $explorerHeader
    $calendarBoundsBeforeSplitter = Get-Bounds $calendarHeader
    $splitterX = $explorerBoundsBeforeSplitter.right
    $splitterY = [int](($explorerBoundsBeforeSplitter.top + $calendarBoundsBeforeSplitter.bottom) / 2)
    [MDLitePanelNative+POINT]$splitterPoint = New-Object MDLitePanelNative+POINT
    $splitterPoint.X = $splitterX
    $splitterPoint.Y = $splitterY
    if (-not [MDLitePanelNative]::ScreenToClient($main, [ref]$splitterPoint)) { throw 'ScreenToClient failed for splitter.' }
    $splitterX = $splitterPoint.X
    $splitterY = $splitterPoint.Y
    $moveX = $splitterX + 72
    $mouseDown = [MDLitePanelNative]::SendMessage($main, $WM_LBUTTONDOWN, [IntPtr]1,
        [MDLitePanelNative]::MakePoint($splitterX, $splitterY)).ToInt64()
    [void][MDLitePanelNative]::SendMessage($main, $WM_MOUSEMOVE, [IntPtr]1,
        [MDLitePanelNative]::MakePoint($moveX, $splitterY))
    [void][MDLitePanelNative]::SendMessage($main, $WM_LBUTTONUP, [IntPtr]::Zero,
        [MDLitePanelNative]::MakePoint($moveX, $splitterY))
    Start-Sleep -Milliseconds 200
    $explorerBoundsAfterSplitter = Get-Bounds $explorerHeader
    $calendarBoundsAfterSplitter = Get-Bounds $calendarHeader
    $splitterPass = $explorerBoundsAfterSplitter.width -gt $explorerBoundsBeforeSplitter.width + 20 -and
        $calendarBoundsAfterSplitter.width -gt $calendarBoundsBeforeSplitter.width + 20
    Add-Check 'synthetic_splitter_drag_resizes_left_slots' $splitterPass ([ordered]@{
        mouseDownReturn = $mouseDown; beforeExplorer = $explorerBoundsBeforeSplitter; afterExplorer = $explorerBoundsAfterSplitter
        beforeCalendar = $calendarBoundsBeforeSplitter; afterCalendar = $calendarBoundsAfterSplitter
    })

    $beforeRestartLayout = Wait-LayoutFile $layoutPath
    Stop-TargetProcess
    $process = Start-Process -FilePath $executable -ArgumentList @($workspace) -WorkingDirectory $repoRoot -PassThru
    $main = Find-MainWindow $process
    Start-Sleep -Milliseconds 500
    $restartedCalendarHeader = Wait-Control $main $kPanelHeaderCalendar
    $restartedOutlineHeader = Wait-Control $main $kPanelHeaderOutline
    $restartedCalendar = Get-Bounds $restartedCalendarHeader
    $restartedOutline = Get-Bounds $restartedOutlineHeader
    $afterRestartLayout = Wait-LayoutFile $layoutPath
    $restartMovePass = $restartedCalendar.centerX -gt $restartedOutline.centerX -and
        $restartedCalendar.centerY -lt $restartedOutline.centerY
    Add-Check 'workspace_layout_persists_through_restart' ($restartMovePass -and $afterRestartLayout -ceq $beforeRestartLayout) ([ordered]@{
        calendar = $restartedCalendar; outline = $restartedOutline
        fileUnchanged = $afterRestartLayout -ceq $beforeRestartLayout; layoutPath = $layoutPath
    })
    $status = if (@($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -eq 0) { 'PASS_SYNTHETIC_NATIVE_MESSAGE_ROUTE' } else { 'FAIL_NATIVE_MESSAGE_ROUTE' }
} catch {
    $errorMessage = $_.Exception.Message
    $status = 'BLOCKED_HARNESS_OR_NATIVE_RUNTIME'
} finally {
    Stop-TargetProcess
}

$failed = @($checks.Values | Where-Object { $_.status -eq 'FAIL' }).Count -gt 0
$result = [ordered]@{
    status = if ($failed -and $status -like 'PASS*') { 'FAIL_NATIVE_MESSAGE_ROUTE' } else { $status }
    pass = ($status -like 'PASS*' -and -not $failed)
    preset = $Preset
    executable = $executable
    executable_sha256 = $executableHash
    checks = $checks
    input_method = 'Synchronous cross-process SendMessage(WM_COMMAND, WM_SIZE, WM_LBUTTONDOWN, WM_MOUSEMOVE, WM_LBUTTONUP) to the MDLite native window; no physical input was sent.'
    evidence_boundary = 'Automated native message-route and HWND visibility/geometry observation only. Physical mouse, OS input delivery, pixel capture, DPI visual review, and Human acceptance are UNKNOWN.'
    error = $errorMessage
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
}

$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) { [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null }
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
$result | ConvertTo-Json -Depth 8

# Remove only this run's uniquely named directory when its ownership marker
# and canonical parent both prove the target is the script-created workspace.
$canonicalRunRoot = [IO.Path]::GetFullPath($runRoot)
$canonicalVerificationRoot = [IO.Path]::GetFullPath($verificationRoot).TrimEnd('\') + '\'
if ((Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and
    [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
    $canonicalRunRoot.StartsWith($canonicalVerificationRoot, [StringComparison]::OrdinalIgnoreCase)) {
    Remove-Item -LiteralPath $canonicalRunRoot -Recurse -Force
}

if (-not $result.pass) { exit 1 }
