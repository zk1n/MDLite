#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug','release')]
    [string]$Preset = 'release',
    [ValidateRange(1, 120)]
    [int]$DurationSeconds = 30,
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$runRoot = Join-Path $repoRoot "build\verification\calendar-hover-$runId"
$workspace = Join-Path $runRoot 'workspace'
$outputRoot = Join-Path $runRoot 'frames'
if (-not $OutputPath) { $OutputPath = Join-Path $runRoot 'calendar-hover-acceptance.json' }
$testGetCalendarHoverCellMessage = 0x8030
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'
$previousSyntheticMouseSetting = $env:MDLITE_TEST_SYNTHETIC_MOUSE_NO_LEAVE
$env:MDLITE_TEST_SYNTHETIC_MOUSE_NO_LEAVE = '1'
[IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
[IO.Directory]::CreateDirectory($outputRoot) | Out-Null
[IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'),
    "schema_version = 1`nauto_save = false`ntheme = `"dark`"`nfont_face = `"Segoe UI`"`nfont_size_pt = 11`n",
    [Text.UTF8Encoding]::new($false))
[IO.File]::WriteAllText((Join-Path $workspace '.mdlite\workspace.toml'),
    "schema_version = 1`n`n[calendar]`nweek_start = `"sunday`"`nholiday_region = `"JP`"`n",
    [Text.UTF8Encoding]::new($false))
$document = Join-Path $workspace 'README.md'
[IO.File]::WriteAllText($document,
    "# Calendar hover probe`n`nHover messages must update one date cell without creating a Daily note.`n",
    [Text.UTF8Encoding]::new($false))

Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class MDLiteCalendarNative {
  public delegate bool EnumChildProc(IntPtr hwnd, IntPtr parameter);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumChildProc callback, IntPtr parameter);
  [DllImport("user32.dll")] public static extern int GetClassName(IntPtr hwnd, StringBuilder text, int capacity);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr hwnd, out RECT rect);
  [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hwnd, IntPtr after, int x, int y, int cx, int cy, uint flags);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint processId);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint message, IntPtr wparam, IntPtr lparam);
  [DllImport("user32.dll")] public static extern bool UpdateWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr hwnd, int command);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
  [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
  public static IntPtr FindChildClass(IntPtr parent, string name) {
    IntPtr found = IntPtr.Zero;
    EnumChildWindows(parent, (hwnd, unused) => {
      var value = new StringBuilder(128);
      GetClassName(hwnd, value, value.Capacity);
      if (value.ToString() == name) { found = hwnd; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  public static IntPtr MakePoint(int x, int y) {
    uint packed = (uint)(ushort)(short)x | ((uint)(ushort)(short)y << 16);
    return new IntPtr(unchecked((int)packed));
  }
}
'@

function Get-DailyFileCount([string]$Root) {
    $daily = Join-Path $Root 'Dairy'
    if (-not (Test-Path -LiteralPath $daily -PathType Container)) { return 0 }
    return @(Get-ChildItem -LiteralPath $daily -File -Recurse -ErrorAction SilentlyContinue).Count
}

function Save-PrintWindowFrame([IntPtr]$Window, [string]$Path,
                               [int]$SampleX, [int]$SampleY,
                               [int]$CalendarLeft, [int]$CalendarTop,
                               [int]$CalendarWidth, [int]$CalendarHeight,
                               [int]$HoveredCell) {
    [MDLiteCalendarNative+RECT]$rect = New-Object MDLiteCalendarNative+RECT
    if (-not [MDLiteCalendarNative]::GetWindowRect($Window, [ref]$rect)) { throw 'GetWindowRect failed.' }
    $width = $rect.Right - $rect.Left
    $height = $rect.Bottom - $rect.Top
    $bitmap = [Drawing.Bitmap]::new($width, $height, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    $hdc = $graphics.GetHdc()
    try {
        $captured = [MDLiteCalendarNative]::PrintWindow($Window, $hdc, 2)
    } finally {
        $graphics.ReleaseHdc($hdc)
        $graphics.Dispose()
    }
    if (-not $captured) { $bitmap.Dispose(); throw 'PrintWindow failed.' }
    $pixelX = [Math]::Clamp($SampleX, 0, $width - 1)
    $pixelY = [Math]::Clamp($SampleY, 0, $height - 1)
    $pixel = $bitmap.GetPixel($pixelX, $pixelY)
    $brightPixels = 0
    $calendarRight = [Math]::Min($width, $CalendarLeft + $CalendarWidth)
    $calendarBottom = [Math]::Min($height, $CalendarTop + $CalendarHeight)
    for ($y = [Math]::Max(0, $CalendarTop); $y -lt $calendarBottom; $y++) {
        for ($x = [Math]::Max(0, $CalendarLeft); $x -lt $calendarRight; $x++) {
            $current = $bitmap.GetPixel($x, $y)
            if ($current.R -ge 240 -and $current.G -ge 240 -and $current.B -ge 240) {
                $brightPixels++
            }
        }
    }
    $calendarArea = [Math]::Max(1, ($calendarRight - $CalendarLeft) *
        ($calendarBottom - $CalendarTop))
    $bitmap.Save($Path, [Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    return [pscustomobject]@{
        path = $Path
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
        capture_method = 'PrintWindow of the native application; not desktop screen capture'
        width = $width
        height = $height
        hovered_cell = $HoveredCell
        hover_sample = @{ x = $pixelX; y = $pixelY;
            rgb = ('#{0:X2}{1:X2}{2:X2}' -f $pixel.R, $pixel.G, $pixel.B) }
        calendar_bright_pixel_fraction = [Math]::Round($brightPixels / $calendarArea, 5)
    }
}

$frames = [Collections.Generic.List[object]]::new()
$calendarControlFrames = [Collections.Generic.List[object]]::new()
$process = $null
$pass = $false
$operationalPass = $false
$visualPass = $false
$status = 'FAIL_OR_BLOCKED_OPERATIONAL'
$errorMessage = $null
$resourceBefore = $null
$resourceAfter = $null
$dailyBefore = Get-DailyFileCount $workspace
$dailyAfter = $dailyBefore
$hoverVisualMatches = $false
$mouseMessagesSent = 0
$mouseMessagesDelivered = 0
$hoverStateChecks = 0
$hoverStateMatches = 0
$hoverStateAvailable = $false
$hoverStatePass = $false
$hoverStateSamples = [Collections.Generic.List[object]]::new()
$mainOwnerPid = 0
$calendarOwnerPid = 0
$targetProcessId = 0
$targetWindowVerified = $false
$elapsedSeconds = 0.0
$calendarBounds = $null
$sampleClient = $null

try {
    $process = Start-Process -FilePath $executable -ArgumentList @($document) -PassThru
    $targetProcessId = [int]$process.Id
    $main = [IntPtr]::Zero
    for ($attempt = 0; $attempt -lt 100 -and $main -eq [IntPtr]::Zero; $attempt++) {
        $process.Refresh()
        $main = $process.MainWindowHandle
        if ($main -eq [IntPtr]::Zero) { Start-Sleep -Milliseconds 100 }
    }
    if ($main -eq [IntPtr]::Zero) { throw 'MDLite main window was not found.' }
    [void][MDLiteCalendarNative]::GetWindowThreadProcessId($main, [ref]$mainOwnerPid)
    if ($mainOwnerPid -ne [uint32]$process.Id) {
        throw "Main window PID $mainOwnerPid does not match launched process $($process.Id)."
    }
    [void][MDLiteCalendarNative]::SetWindowPos($main, [IntPtr]::Zero, 0, 0, 1672, 941, 0x0004 -bor 0x0040)
    [void][MDLiteCalendarNative]::ShowWindow($main, 9)
    [void][MDLiteCalendarNative]::SetForegroundWindow($main)
    $calendar = [MDLiteCalendarNative]::FindChildClass($main, 'MDLite.CalendarView')
    if ($calendar -eq [IntPtr]::Zero -or -not [MDLiteCalendarNative]::IsWindowVisible($calendar)) {
        throw 'Visible MDLite.CalendarView control was not found.'
    }
    [void][MDLiteCalendarNative]::GetWindowThreadProcessId($calendar, [ref]$calendarOwnerPid)
    if ($calendarOwnerPid -ne [uint32]$process.Id) {
        throw "Calendar control PID $calendarOwnerPid does not match launched process $($process.Id)."
    }
    $targetWindowVerified = $true
    [MDLiteCalendarNative+RECT]$calendarRect = New-Object MDLiteCalendarNative+RECT
    [MDLiteCalendarNative+RECT]$calendarClient = New-Object MDLiteCalendarNative+RECT
    [MDLiteCalendarNative+RECT]$mainRect = New-Object MDLiteCalendarNative+RECT
    [void][MDLiteCalendarNative]::GetWindowRect($calendar, [ref]$calendarRect)
    [void][MDLiteCalendarNative]::GetClientRect($calendar, [ref]$calendarClient)
    [void][MDLiteCalendarNative]::GetWindowRect($main, [ref]$mainRect)
    $calendarBounds = [ordered]@{
        left = $calendarRect.left; top = $calendarRect.top
        right = $calendarRect.right; bottom = $calendarRect.bottom
        width = $calendarClient.right; height = $calendarClient.bottom
    }
    $height = $calendarClient.bottom
    $header = [Math]::Min($height, [Math]::Max(24, [Math]::Min(36, [Math]::Floor($height / 5))))
    $weekday = [Math]::Min($height - $header,
        [Math]::Max(20, [Math]::Min(24, [Math]::Floor(($height - $header) / 8))))
    $gridTop = [int]($header + $weekday)
    $footer = [Math]::Min([Math]::Max(0, $height - $gridTop),
        [Math]::Max(24, [Math]::Min(32, [Math]::Floor($height / 9))))
    $gridBottom = [Math]::Max($gridTop, $height - $footer)
    $gridHeight = $gridBottom - $gridTop
    $gridWidth = $calendarClient.right
    $today = [DateTime]::Today
    $firstOfMonth = [DateTime]::new($today.Year, $today.Month, 1)
    $leadingDays = ([int]$firstOfMonth.DayOfWeek - [int][DayOfWeek]::Sunday + 7) % 7
    $todayCell = $leadingDays + $today.Day - 1
    $dailyBefore = Get-DailyFileCount $workspace
    $resourceBefore = [pscustomobject]@{
        gdi = [int][MDLiteCalendarNative]::GetGuiResources($process.Handle, 0)
        user = [int][MDLiteCalendarNative]::GetGuiResources($process.Handle, 1)
        cpu_ms = [int64]$process.TotalProcessorTime.TotalMilliseconds
    }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $index = 0
    $nextCapture = 0
    while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $cell = $index % 42
        $row = [Math]::Floor($cell / 7)
        $column = $cell % 7
        $x = [int]([Math]::Floor($gridWidth * ($column + 0.5) / 7))
        $y = [int]($gridTop + [Math]::Floor($gridHeight * ($row + 0.5) / 6))
        # These synthetic WM_MOUSEMOVE messages must not create TrackMouseEvent leave
        # tracking: the physical cursor is elsewhere, so Windows would immediately
        # clear the synthetic hovered cell with WM_MOUSELEAVE.
        [void][MDLiteCalendarNative]::SendMessage($calendar, 0x0200, [IntPtr]::Zero,
            [MDLiteCalendarNative]::MakePoint($x, $y))
        $mouseMessagesDelivered++
        [void][MDLiteCalendarNative]::UpdateWindow($calendar)
        [void][MDLiteCalendarNative]::UpdateWindow($main)
        $mouseMessagesSent++
        Start-Sleep -Milliseconds 25
        $hoveredCellPlusOne = [MDLiteCalendarNative]::SendMessage(
            $main, $testGetCalendarHoverCellMessage, [IntPtr]::Zero, [IntPtr]::Zero).ToInt32()
        if ($hoveredCellPlusOne -ge 0) {
            $hoverStateAvailable = $true
            $hoverStateChecks++
            $hoverStateMatchesCurrent = $hoveredCellPlusOne -eq ($cell + 1)
            if ($hoverStateMatchesCurrent) { $hoverStateMatches++ }
        }
        $wholeSecond = [int][Math]::Floor($watch.Elapsed.TotalSeconds)
        if ($wholeSecond -ge $nextCapture -and $frames.Count -lt 7) {
            if ($cell -eq $todayCell) {
                $nextCapture = $wholeSecond + 1
                Start-Sleep -Milliseconds 25
                $index++
                continue
            }
            $framePath = Join-Path $outputRoot ('hover-frame-{0:D2}s.png' -f $wholeSecond)
            $cellLeft = [int][Math]::Floor($gridWidth * $column / 7)
            $cellTop = [int]($gridTop + [Math]::Floor($gridHeight * $row / 6))
            $calendarOffsetX = $calendarRect.left - $mainRect.left
            $calendarOffsetY = $calendarRect.top - $mainRect.top
            $sampleX = $calendarOffsetX + $cellLeft + 3
            $sampleY = $calendarOffsetY + $cellTop + 3
            $frame = Save-PrintWindowFrame $main $framePath $sampleX $sampleY `
                $calendarOffsetX $calendarOffsetY $gridWidth $height $cell
            $frame | Add-Member -NotePropertyName observed_hovered_cell -NotePropertyValue `
                $(if ($hoveredCellPlusOne -gt 0) { $hoveredCellPlusOne - 1 } else { $null })
            $frame | Add-Member -NotePropertyName hover_state_matches -NotePropertyValue `
                ($hoveredCellPlusOne -eq ($cell + 1))
            $frames.Add($frame)
            $hoverStateSamples.Add([pscustomobject]@{
                elapsed_seconds = $wholeSecond
                expected_cell = $cell
                observed_cell = if ($hoveredCellPlusOne -gt 0) { $hoveredCellPlusOne - 1 } else { $null }
                hover_state_matches = ($hoveredCellPlusOne -eq ($cell + 1))
            })
            $controlFramePath = Join-Path $outputRoot ('calendar-control-frame-{0:D2}s.png' -f $wholeSecond)
            $controlFrame = Save-PrintWindowFrame $calendar $controlFramePath `
                ($cellLeft + 3) ($cellTop + 3) 0 0 $gridWidth $height $cell
            $controlFrame | Add-Member -NotePropertyName observed_hovered_cell -NotePropertyValue `
                $(if ($hoveredCellPlusOne -gt 0) { $hoveredCellPlusOne - 1 } else { $null })
            $controlFrame | Add-Member -NotePropertyName hover_state_matches -NotePropertyValue `
                ($hoveredCellPlusOne -eq ($cell + 1))
            $calendarControlFrames.Add($controlFrame)
            $nextCapture = $wholeSecond + 5
        }
        Start-Sleep -Milliseconds 25
        $index++
    }
    $elapsedSeconds = $watch.Elapsed.TotalSeconds
    [void][MDLiteCalendarNative]::SendMessage($calendar, 0x02A3, [IntPtr]::Zero, [IntPtr]::Zero) # WM_MOUSELEAVE
    [void][MDLiteCalendarNative]::UpdateWindow($calendar)
    $process.Refresh()
    $resourceAfter = [pscustomobject]@{
        gdi = [int][MDLiteCalendarNative]::GetGuiResources($process.Handle, 0)
        user = [int][MDLiteCalendarNative]::GetGuiResources($process.Handle, 1)
        cpu_ms = [int64]$process.TotalProcessorTime.TotalMilliseconds
    }
    $dailyAfter = Get-DailyFileCount $workspace
    $hoverSampleColors = @($calendarControlFrames | ForEach-Object { $_.hover_sample.rgb } | Select-Object -Unique)
    $expectedHoverColor = '#323740'
    $hoverVisualMatches = $hoverSampleColors.Count -eq 1 -and
        $hoverSampleColors[0] -eq $expectedHoverColor
    $hoverStatePass = $hoverStateAvailable -and $hoverStateChecks -gt 0 -and
        $hoverStateMatches -eq $hoverStateChecks
    $frameVariations = @($calendarControlFrames | ForEach-Object { $_.sha256 } | Select-Object -Unique).Count
    $maxBrightFraction = [Math]::Max(0, [double](($calendarControlFrames | Measure-Object `
        -Property calendar_bright_pixel_fraction -Maximum).Maximum))
    $operationalPass = $elapsedSeconds -ge $DurationSeconds -and $frames.Count -ge 6 -and
        $mouseMessagesSent -gt 0 -and
        $mouseMessagesDelivered -eq $mouseMessagesSent -and $targetWindowVerified -and
        $dailyAfter -eq $dailyBefore -and
        $resourceAfter.gdi -le ($resourceBefore.gdi + 24) -and
        $resourceAfter.user -le ($resourceBefore.user + 8) -and
        $process.Responding
    $visualPass = $hoverStatePass -and $hoverVisualMatches -and $frameVariations -ge 2 -and
        $maxBrightFraction -lt 0.10
    $pass = $operationalPass -and $visualPass
    $status = if (-not $operationalPass) {
        'FAIL_OR_BLOCKED_OPERATIONAL'
    } elseif (-not $hoverStateAvailable) {
        'UNKNOWN_HOVER_STATE_QUERY_UNAVAILABLE'
    } elseif (-not $hoverStatePass) {
        'FAIL_HOVER_STATE_NOT_UPDATED'
    } elseif (-not $hoverVisualMatches) {
        'BLOCKED_PRINTWINDOW_CAPTURE_UNVERIFIED'
    } elseif (-not $pass) {
        'FAIL_VISUAL_RENDER_OR_FLICKER'
    } else {
        'PASS_SYNCHRONOUS_NATIVE_MESSAGE_ROUTE'
    }
} catch {
    $errorMessage = $_.Exception.Message
} finally {
    if ($process -and -not $process.HasExited) {
        [void][MDLiteCalendarNative]::PostMessage($main, 0x0010, [IntPtr]::Zero, [IntPtr]::Zero)
        if (-not $process.WaitForExit(5000)) { $process.Kill(); [void]$process.WaitForExit(3000) }
        $process.Dispose()
    }
    if ($null -eq $previousSyntheticMouseSetting) {
        Remove-Item Env:MDLITE_TEST_SYNTHETIC_MOUSE_NO_LEAVE -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_SYNTHETIC_MOUSE_NO_LEAVE = $previousSyntheticMouseSetting
    }
    if ($null -eq $previousSilentSetting) {
        Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_SILENT = $previousSilentSetting
    }
}

$result = [ordered]@{
    pass = [bool]$pass
    status = $status
    operational_pass = [bool]$operationalPass
    visual_pass = [bool]$visualPass
    input_method = 'Synthetic synchronous cross-process WM_MOUSEMOVE with test-only TrackMouseEvent leave suppression; in-process hovered-cell state is checked. This is not Human physical input.'
    message_receipt_status = if ($targetWindowVerified -and $mouseMessagesDelivered -eq $mouseMessagesSent -and $mouseMessagesSent -gt 0) {
        'SYNCHRONOUS_WINDOW_PROC_RETURN_CONFIRMED'
    } else { 'NOT_CONFIRMED' }
    target_process_id = $targetProcessId
    main_window_process_id = [int]$mainOwnerPid
    calendar_control_process_id = [int]$calendarOwnerPid
    mouse_messages_sent = $mouseMessagesSent
    mouse_messages_delivered = $mouseMessagesDelivered
    hover_state_query_status = if ($hoverStatePass) { 'PASS_INTERNAL_HOVERED_CELL_STATE' } elseif ($hoverStateAvailable) { 'FAIL_STATE_MISMATCH' } else { 'UNKNOWN_QUERY_UNAVAILABLE' }
    hover_state_checks = $hoverStateChecks
    hover_state_matches = $hoverStateMatches
    hover_state_samples = @($hoverStateSamples)
    hover_state_verified = [bool]$hoverStatePass
    physical_cursor_or_mouse_status = 'UNKNOWN_NOT_TESTED_BY_HUMAN_PHYSICAL_MOUSE_INPUT'
    duration_seconds = [Math]::Round($elapsedSeconds, 2)
    frame_count = $frames.Count
    frames = @($frames)
    calendar_control_frames = @($calendarControlFrames)
    calendar_bounds = $calendarBounds
    resource_before = $resourceBefore
    resource_after = $resourceAfter
    daily_file_count_before = $dailyBefore
    daily_file_count_after = $dailyAfter
    hover_sample_colors = if ($frames.Count -gt 0) {
        @($calendarControlFrames | ForEach-Object { $_.hover_sample.rgb } | Select-Object -Unique)
    } else { @() }
    expected_hover_background_rgb = '#323740'
    hover_visual_verified = [bool]$hoverVisualMatches
    hover_visual_status = if ($hoverVisualMatches) { 'PASS_PRINTWINDOW_CELL_SAMPLE' } elseif ($hoverStatePass) {
        'UNKNOWN_CAPTURE_PATH: internal hovered-cell state changed correctly, but PrintWindow did not show the expected cell color.'
    } else {
        'UNKNOWN_CAPTURE_PATH: PrintWindow did not show the expected hover cell color.'
    }
    frame_variation_count = if ($frames.Count -gt 0) {
        @($frames | ForEach-Object { $_.sha256 } | Select-Object -Unique).Count
    } else { 0 }
    max_calendar_bright_pixel_fraction = if ($frames.Count -gt 0) {
        [Math]::Max(0, [double](($frames | Measure-Object `
            -Property calendar_bright_pixel_fraction -Maximum).Maximum))
    } else { $null }
    readme_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $document).Hash.ToLowerInvariant()
    executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    error = $errorMessage
    timestamp_utc = [DateTime]::UtcNow.ToString('o')
}
[IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
$result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
Get-Content -LiteralPath $OutputPath
if (-not $pass) { exit 1 }
