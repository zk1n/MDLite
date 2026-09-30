#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('release','debug')][string]$Preset = 'release',
    [ValidateSet('full','quick')][string]$Profile = 'full',
    [ValidateRange(1, 1000)][int]$P5Iterations = 100,
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot

function Get-SourceFingerprint([string]$Root) {
    $rootPrefix = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
    $paths = @(
        Get-ChildItem -LiteralPath (Join-Path $Root 'src') -File -Recurse -Force | Select-Object -ExpandProperty FullName
        Join-Path $Root 'CMakeLists.txt'
        Join-Path $Root 'CMakePresets.json'
    )
    [string[]]$paths = $paths
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $utf8 = [Text.UTF8Encoding]::new($false)
    $manifest = foreach ($path in $paths) {
        $relativePath = [IO.Path]::GetFullPath($path).Substring($rootPrefix.Length).Replace('\', '/')
        '{0}:{1}' -f $relativePath, ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant())
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $sha256 = [BitConverter]::ToString($hasher.ComputeHash($utf8.GetBytes([string]::Join("`n", [string[]]$manifest)))).Replace('-', '').ToLowerInvariant()
    } finally { $hasher.Dispose() }
    return [pscustomobject]@{ sha256 = $sha256; file_count = $paths.Count }
}

$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }
if (-not $OutputPath) { $OutputPath = Join-Path $repoRoot "build\verification\performance-$Preset-$Profile.json" }
$measurementRunId = [guid]::NewGuid().ToString('N')
$executableSha256AtStart = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
$buildReceiptPath = Join-Path $repoRoot "build\verification\build-receipt-$Preset.json"
if (-not (Test-Path -LiteralPath $buildReceiptPath -PathType Leaf)) {
    throw "Successful-build receipt is missing: $buildReceiptPath. Run .\tools\Invoke-Build.ps1 -Preset $Preset first."
}
$buildReceiptSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $buildReceiptPath).Hash.ToLowerInvariant()
$sourceAtMeasurementStart = Get-SourceFingerprint $repoRoot
$buildReceipt = Get-Content -Raw -LiteralPath $buildReceiptPath | ConvertFrom-Json
if ($buildReceipt.schema -ne 'mdlite-build-receipt-v2') { throw 'Build receipt schema is unsupported.' }
$buildRunId = [guid]::Empty
if (-not [guid]::TryParseExact([string]$buildReceipt.build_run_id, 'N', [ref]$buildRunId)) {
    throw 'Build receipt has an invalid build run ID.'
}
if ($buildReceipt.preset -ne $Preset) { throw 'Build receipt preset does not match the selected preset.' }
if ($buildReceipt.build_status -ne 'PASS' -or
    [int]$buildReceipt.configure_exit_code -ne 0 -or
    [int]$buildReceipt.build_exit_code -ne 0 -or
    ([bool]$buildReceipt.test_requested -and [int]$buildReceipt.test_exit_code -ne 0)) {
    throw 'Build receipt does not record a successful configure, build, and requested test run.'
}
if ([IO.Path]::GetFullPath([string]$buildReceipt.executable_path) -ne [IO.Path]::GetFullPath($executable)) {
    throw 'Build receipt executable path does not match this preset output.'
}
if ([string]$buildReceipt.executable_sha256 -notmatch '^[0-9a-fA-F]{64}$' -or
    [string]$buildReceipt.executable_sha256 -ine $executableSha256AtStart) {
    throw 'Build receipt executable SHA-256 does not match the executable being measured.'
}
if ($buildReceipt.source_file_count -ne $sourceAtMeasurementStart.file_count -or
    [string]$buildReceipt.source_sha256 -ine $sourceAtMeasurementStart.sha256) {
    throw 'Build receipt source fingerprint does not match the current source inputs.'
}
if ((Get-FileHash -Algorithm SHA256 -LiteralPath $buildReceiptPath).Hash.ToLowerInvariant() -ne $buildReceiptSha256) {
    throw 'Build receipt changed while it was being verified.'
}

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class MDLitePerfNative {
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left; public int Top; public int Right; public int Bottom; }
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr parent, EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll")] public static extern bool IsHungAppWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)] public static extern bool GetClientRect(IntPtr window, out RECT rectangle);
    [DllImport("user32.dll")] public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wparam, ref RECT lparam, uint flags, uint timeoutMs, out IntPtr result);
    [DllImport("user32.dll", SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wparam, IntPtr lparam, uint flags, uint timeoutMs, out IntPtr result);
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wparam, string lparam, uint flags, uint timeoutMs, out IntPtr result);
    [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    [DllImport("user32.dll")] public static extern uint GetGuiResources(IntPtr process, uint flags);
    [DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);
}
'@

# Test children inherit this mode, so a product crash is returned to the runner
# as a non-zero exit instead of blocking unattended P0-P5 runs with an OS dialog.
$previousErrorMode = [MDLitePerfNative]::SetErrorMode(0x0002) # SEM_NOGPFAULTERRORBOX
$previousCrashUiSetting = $env:MDLITE_TEST_NO_CRASH_UI
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$previousSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

$WM_COMMAND = 0x0111
$WM_CLOSE = 0x0010
$WM_CHAR = 0x0102
$WM_SETTEXT = 0x000C
$WM_GETTEXTLENGTH = 0x000E
$EM_SETSEL = 0x00B1
$LVM_GETITEMCOUNT = 0x1004
$BM_CLICK = 0x00F5
$WM_LBUTTONDOWN = 0x0201
$WM_LBUTTONUP = 0x0202
$WM_KEYDOWN = 0x0100
$WM_KEYUP = 0x0101
$VK_HOME = 0x24
$VK_RIGHT = 0x27
$TCM_GETITEMCOUNT = 0x1304 # TCM_FIRST + 4; scalar tab count
$TCM_GETCURSEL = 0x130B # TCM_FIRST + 11; scalar selected-index query
$WM_TEST_THEME_CHANGE = 0x802B # WM_APP + 43; enabled only under MDLITE_TEST_SILENT
$WM_TEST_TAB_ITEM_CENTER = 0x803A # WM_APP + 58; in-process tab rectangle query
$SMTO_FAIL_FAST = 0x0023 # BLOCK | ABORTIFHUNG | ERRORONEXIT
# Full fixtures intentionally exercise 10,000-file and 100 MiB paths.  Keep the
# quick-run watchdog strict, but allow a single synchronous GUI query enough time
# to return under the full profile so the measured latency is reported instead
# of being mistaken for an application crash.
$GuiMessageTimeoutMs = if ($Profile -eq 'full') { 60000 } else { 15000 }
$ControlTabs = 101
$ControlEditor = 102
$ControlFindEdit = 105
$ControlFindResults = 117

function Assert-ProcessRunning([Diagnostics.Process]$Process, [string]$Context) {
    $Process.Refresh()
    if ($Process.HasExited) {
        $exitCode = $Process.ExitCode
        $hex = '0x{0:X8}' -f ([uint64](4294967296L + [long]$exitCode) % 4294967296L)
        throw "$Context`: MDLite exited unexpectedly (code=$exitCode, $hex)."
    }
}

function Assert-WindowOwnedByProcess([Diagnostics.Process]$Process, [IntPtr]$Window,
                                     [string]$Context) {
    Assert-ProcessRunning $Process $Context
    if ($Window -eq [IntPtr]::Zero -or -not [MDLitePerfNative]::IsWindow($Window)) {
        throw "$Context`: target window is unavailable."
    }
    [uint32]$owner = 0
    [void][MDLitePerfNative]::GetWindowThreadProcessId($Window, [ref]$owner)
    if ($owner -ne [uint32]$Process.Id) {
        throw "$Context`: target window belongs to process $owner, expected $($Process.Id)."
    }
}

function Send-NativeMessage([Diagnostics.Process]$Process, [IntPtr]$Window, [uint32]$Message,
                            [IntPtr]$WParam, [IntPtr]$LParam, [string]$Context,
                            [int]$TimeoutMs = $GuiMessageTimeoutMs) {
    Write-Verbose "BEGIN GUI message: $Context"
    Assert-WindowOwnedByProcess $Process $Window $Context
    $messageResult = [IntPtr]::Zero
    $sent = [MDLitePerfNative]::SendMessageTimeout(
        $Window, $Message, $WParam, $LParam, $SMTO_FAIL_FAST, [uint32]$TimeoutMs, [ref]$messageResult)
    if ($sent -eq [IntPtr]::Zero) {
        Assert-ProcessRunning $Process $Context
        $nativeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        $Process.Refresh()
        $hung = [MDLitePerfNative]::IsHungAppWindow($Window)
        throw "$Context`: SendMessageTimeout failed or exceeded ${TimeoutMs}ms (Win32=$nativeError, pid=$($Process.Id), hung=$hung, cpu_ms=$([Math]::Round($Process.TotalProcessorTime.TotalMilliseconds, 3)), working_set=$($Process.WorkingSet64), private_bytes=$($Process.PrivateMemorySize64))."
    }
    Assert-ProcessRunning $Process $Context
    Write-Verbose "END GUI message: $Context"
    return $messageResult
}

function Send-NativeTextMessage([Diagnostics.Process]$Process, [IntPtr]$Window, [uint32]$Message,
                                [IntPtr]$WParam, [string]$Text, [string]$Context,
                                [int]$TimeoutMs = $GuiMessageTimeoutMs) {
    Write-Verbose "BEGIN GUI text message: $Context"
    Assert-WindowOwnedByProcess $Process $Window $Context
    $messageResult = [IntPtr]::Zero
    $sent = [MDLitePerfNative]::SendMessageTimeout(
        $Window, $Message, $WParam, $Text, $SMTO_FAIL_FAST, [uint32]$TimeoutMs, [ref]$messageResult)
    if ($sent -eq [IntPtr]::Zero) {
        Assert-ProcessRunning $Process $Context
        $nativeError = [Runtime.InteropServices.Marshal]::GetLastWin32Error()
        throw "$Context`: SendMessageTimeout failed or exceeded ${TimeoutMs}ms (Win32=$nativeError)."
    }
    Assert-ProcessRunning $Process $Context
    Write-Verbose "END GUI text message: $Context"
    return $messageResult
}

function Write-RepeatedFile([string]$Path, [long]$Bytes, [Text.Encoding]$Encoding, [string]$Seed) {
    $header = $Encoding.GetBytes($Seed)
    # Keep the repeated/truncated tail ASCII so an exact byte target can never
    # cut a UTF-8 or CP932 multibyte sequence. The seed carries Japanese text.
    $block = $Encoding.GetBytes("content 0123456789 abcdefghijklmnopqrstuvwxyz`r`n")
    $stream = [IO.File]::Create($Path)
    try {
        $take = [Math]::Min([long]$header.Length, $Bytes)
        $stream.Write($header, 0, [int]$take)
        while ($stream.Length -lt $Bytes) {
            $take = [Math]::Min([long]$block.Length, $Bytes - $stream.Length)
            $stream.Write($block, 0, [int]$take)
        }
    } finally { $stream.Dispose() }
}

function New-Workspace([string]$Root, [string]$Name) {
    $path = Join-Path $Root $Name
    [IO.Directory]::CreateDirectory($path) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $path '.mdlite')) | Out-Null
    return $path
}

function New-DistributedFixture([string]$Workspace, [int]$Count, [long]$TotalBytes,
                                [bool]$MixedEncoding, [long[]]$OpenSizes = @()) {
    $reserved = [long](($OpenSizes | Measure-Object -Sum).Sum)
    if ($OpenSizes.Count -ge $Count -or $reserved -ge $TotalBytes) {
        throw "Open-document fixtures must leave at least one collection file and one byte."
    }
    $remainingCount = $Count - $OpenSizes.Count
    $remainingBytes = $TotalBytes - $reserved
    $base = [Math]::Floor($remainingBytes / $remainingCount)
    $remainder = $remainingBytes - ($base * $remainingCount)
    $utf8 = [Text.UTF8Encoding]::new($false)
    $cp932 = [Text.Encoding]::GetEncoding(932)
    $paths = New-Object Collections.Generic.List[string]
    for ($index = 0; $index -lt $Count; $index++) {
        if ($index -lt $OpenSizes.Count) {
            $size = $OpenSizes[$index]
        } else {
            $distributedIndex = $index - $OpenSizes.Count
            $size = [long]$base + $(if ($distributedIndex -lt $remainder) { 1 } else { 0 })
        }
        $path = Join-Path $Workspace ("note-{0:D5}.md" -f $index)
        $encoding = if ($MixedEncoding -and ($index % 2 -eq 1)) { $cp932 } else { $utf8 }
        Write-RepeatedFile $path $size $encoding ("# Note $index`r`nperformance token`r`n日本語`r`n")
        if ($index -lt $OpenSizes.Count) { $paths.Add($path) }
    }
    return $paths.ToArray()
}

function Get-ProcessClassWindows([Diagnostics.Process]$Process, [string]$ClassName) {
    Assert-ProcessRunning $Process "Enumerate $ClassName windows"
    $script:perfClassWindows = New-Object Collections.Generic.List[IntPtr]
    $callback = [MDLitePerfNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        [uint32]$processId = 0
        [void][MDLitePerfNative]::GetWindowThreadProcessId($window, [ref]$processId)
        if ($processId -ne [uint32]$Process.Id) { return $true }
        $name = New-Object Text.StringBuilder 128
        [void][MDLitePerfNative]::GetClassName($window, $name, $name.Capacity)
        if ($name.ToString() -eq $ClassName) { $script:perfClassWindows.Add($window) }
        return $true
    }
    [void][MDLitePerfNative]::EnumWindows($callback, [IntPtr]::Zero)
    Assert-ProcessRunning $Process "Enumerate $ClassName windows"
    return $script:perfClassWindows.ToArray()
}

function Invoke-ThemeChange([Diagnostics.Process]$Process, [IntPtr]$Main,
                            [string]$Workspace, [string]$Theme) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $themeValue = if ($Theme -eq 'light') { 1 } elseif ($Theme -eq 'dark') { 2 } else { 0 }
    $applied = (Send-NativeMessage $Process $Main $WM_TEST_THEME_CHANGE ([IntPtr]$themeValue) `
        ([IntPtr]::Zero) "Apply silent test theme '$Theme'").ToInt64() -ne 0
    $settingsPath = Join-Path $Workspace '.mdlite\settings.toml'
    $saved = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do {
        Assert-ProcessRunning $Process 'Wait for silent theme settings persistence'
        if ($applied -and (Test-Path -LiteralPath $settingsPath -PathType Leaf)) {
            $saved = [IO.File]::ReadAllText($settingsPath).Contains(('theme = "{0}"' -f $Theme))
        }
        if (-not $saved) { Start-Sleep -Milliseconds 10 }
    } while (-not $saved -and [DateTime]::UtcNow -lt $deadline)
    $watch.Stop()
    return [pscustomobject]@{ completed=$saved; elapsed_ms=[Math]::Round($watch.Elapsed.TotalMilliseconds, 3) }
}

function Invoke-SearchCancellation([Diagnostics.Process]$Process, [IntPtr]$Main) {
    $find = Find-Control $Process $Main $ControlFindEdit
    if ($find -eq [IntPtr]::Zero) {
        return [pscustomobject]@{ first_batch_seen=$false; stale_results_cleared=$false; completed=$false; elapsed_ms=$null }
    }
    [void](Send-NativeTextMessage $Process $find $WM_SETTEXT ([IntPtr]::Zero) 'performance token' 'Set cancellation query')
    $watch = [Diagnostics.Stopwatch]::StartNew()
    [void][MDLitePerfNative]::PostMessage($Main, $WM_COMMAND, [IntPtr]108, [IntPtr]::Zero)
    $firstBatchSeen = $false
    $staleResultsCleared = $false
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        Assert-ProcessRunning $Process 'Wait for cancellable search batch'
        $results = Find-Control $Process $Main $ControlFindResults
        if ($results -ne [IntPtr]::Zero) {
            $count = (Send-NativeMessage $Process $results $LVM_GETITEMCOUNT `
                ([IntPtr]::Zero) ([IntPtr]::Zero) 'Read cancellable search batch').ToInt32()
            if ($count -gt 0) { $firstBatchSeen = $true; break }
        }
        Start-Sleep -Milliseconds 10
    } while ([DateTime]::UtcNow -lt $deadline)
    if ($firstBatchSeen) {
        # SearchWorkspaceFromFindBar uses a generation token. Changing the query
        # requests stop, clears the old list, and schedules a replacement search.
        [void](Send-NativeTextMessage $Process $find $WM_SETTEXT ([IntPtr]::Zero) `
            'cancelled performance query' 'Cancel search by changing generation')
        Start-Sleep -Milliseconds 50
        $results = Find-Control $Process $Main $ControlFindResults
        if ($results -ne [IntPtr]::Zero) {
            $remaining = (Send-NativeMessage $Process $results $LVM_GETITEMCOUNT `
                ([IntPtr]::Zero) ([IntPtr]::Zero) 'Verify stale search results cleared').ToInt32()
            $staleResultsCleared = $remaining -eq 0
        }
        [void](Send-NativeTextMessage $Process $find $WM_SETTEXT ([IntPtr]::Zero) `
            'performance token' 'Restore search query after cancellation')
    }
    $watch.Stop()
    return [pscustomobject]@{
        first_batch_seen = $firstBatchSeen
        stale_results_cleared = $staleResultsCleared
        completed = $firstBatchSeen -and $staleResultsCleared -and
                    -not [MDLitePerfNative]::IsHungAppWindow($Main)
        elapsed_ms = [Math]::Round($watch.Elapsed.TotalMilliseconds, 3)
    }
}

function Open-CompactWindows([Diagnostics.Process]$Process, [IntPtr]$Main, [int]$Count) {
    $tabs = Find-Control $Process $Main $ControlTabs
    if ($tabs -eq [IntPtr]::Zero) { return 0 }
    # The native tab control may scroll the active item into view, leaving
    # earlier tabs clipped when the P4 fixture names exceed the available width.
    # Open in reverse visual order so selecting each earlier tab also scrolls
    # it into the visible client area before its item-center click.
    for ($index = $Count - 1; $index -ge 0; $index--) {
        # Ask the owning window to query the RECT in-process. TCM_GETITEMRECT
        # carries a RECT*; passing the caller's address directly across process
        # boundaries can crash the target control instead of returning geometry.
        $packed = (Send-NativeMessage $Process $Main $WM_TEST_TAB_ITEM_CENTER `
            ([IntPtr]$index) ([IntPtr]::Zero) "Get tab $index rectangle center").ToInt64()
        if ($packed -eq -1) { throw "Get tab $index rectangle center`: tab item is unavailable." }
        $packed = $packed -band 0xffffffffL
        $x = [int]($packed -band 0xffff)
        $y = [int](($packed -shr 16) -band 0xffff)
        if ($x -ge 0x8000) { $x -= 0x10000 }
        if ($y -ge 0x8000) { $y -= 0x10000 }
        $clientRect = [MDLitePerfNative+RECT]::new()
        if (-not [MDLitePerfNative]::GetClientRect($tabs, [ref]$clientRect)) {
            throw "GetClientRect failed for tab control $index."
        }
        if ($x -lt $clientRect.Left -or $x -ge $clientRect.Right -or
            $y -lt $clientRect.Top -or $y -ge $clientRect.Bottom) {
            # An open document can leave the tab row scrolled away from the
            # requested item. Use the control's normal keyboard navigation to
            # bring it into view, then click the freshly measured item center.
            [void](Send-NativeMessage $Process $tabs $WM_KEYDOWN ([IntPtr]$VK_HOME) ([IntPtr]1) "Scroll tabs to first item")
            [void](Send-NativeMessage $Process $tabs $WM_KEYUP ([IntPtr]$VK_HOME) ([IntPtr]0xC0000001) "Release Home key")
            for ($step = 0; $step -lt $index; $step++) {
                [void](Send-NativeMessage $Process $tabs $WM_KEYDOWN ([IntPtr]$VK_RIGHT) ([IntPtr]1) "Navigate to tab $index")
                [void](Send-NativeMessage $Process $tabs $WM_KEYUP ([IntPtr]$VK_RIGHT) ([IntPtr]0xC0000001) "Release Right key")
            }
            $packed = (Send-NativeMessage $Process $Main $WM_TEST_TAB_ITEM_CENTER `
                ([IntPtr]$index) ([IntPtr]::Zero) "Re-measure visible tab $index center").ToInt64()
            if ($packed -eq -1) { throw "Tab $index center is unavailable after keyboard navigation." }
            $packed = $packed -band 0xffffffffL
            $x = [int]($packed -band 0xffff)
            $y = [int](($packed -shr 16) -band 0xffff)
            if ($x -ge 0x8000) { $x -= 0x10000 }
            if ($y -ge 0x8000) { $y -= 0x10000 }
            if (-not [MDLitePerfNative]::GetClientRect($tabs, [ref]$clientRect)) {
                throw "GetClientRect failed for tab control $index after keyboard navigation."
            }
            if ($x -lt $clientRect.Left -or $x -ge $clientRect.Right -or
                $y -lt $clientRect.Top -or $y -ge $clientRect.Bottom) {
                throw "Tab $index remains outside tab client after keyboard navigation: center=($x,$y), client=($($clientRect.Left),$($clientRect.Top),$($clientRect.Right),$($clientRect.Bottom))."
            }
        }
        $lparam = [IntPtr](($x -band 0xFFFF) -bor (($y -band 0xFFFF) -shl 16))
        [void](Send-NativeMessage $Process $tabs $WM_LBUTTONDOWN ([IntPtr]1) $lparam "Select tab $index down")
        [void](Send-NativeMessage $Process $tabs $WM_LBUTTONUP ([IntPtr]::Zero) $lparam "Select tab $index up")
        $selected = (Send-NativeMessage $Process $tabs $TCM_GETCURSEL `
            ([IntPtr]::Zero) ([IntPtr]::Zero) "Read selected tab $index").ToInt64()
        if ($selected -ne $index) {
            throw "Select tab $index at ($x,$y) did not update selection; TCM_GETCURSEL returned $selected; tab client=($($clientRect.Left),$($clientRect.Top),$($clientRect.Right),$($clientRect.Bottom))."
        }
        [void](Send-NativeMessage $Process $Main $WM_COMMAND ([IntPtr]1030) ([IntPtr]::Zero) "Open compact window $index")
        Start-Sleep -Milliseconds 50
    }
    return @(Get-ProcessClassWindows $Process 'MDLite.CompactWindow').Count
}

function Find-Control([Diagnostics.Process]$Process, [IntPtr]$Parent, [int]$Id) {
    Write-Verbose "BEGIN control enumeration: $Id"
    Assert-WindowOwnedByProcess $Process $Parent "Find control $Id"
    $script:perfControl = [IntPtr]::Zero
    $callback = [MDLitePerfNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        if ([MDLitePerfNative]::GetDlgCtrlID($window) -eq $Id) {
            $script:perfControl = $window
            return $false
        }
        return $true
    }
    [void][MDLitePerfNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    Assert-ProcessRunning $Process "Find control $Id"
    Write-Verbose "END control enumeration: $Id handle=$script:perfControl"
    return $script:perfControl
}

function Count-EditorWindows([Diagnostics.Process]$Process, [IntPtr]$Parent) {
    Assert-WindowOwnedByProcess $Process $Parent 'Count RichEdit windows'
    $script:perfEditorCount = 0
    $callback = [MDLitePerfNative+EnumWindowsProc]{
        param([IntPtr]$window, [IntPtr]$parameter)
        $className = New-Object Text.StringBuilder 64
        [void][MDLitePerfNative]::GetClassName($window, $className, $className.Capacity)
        if ($className.ToString() -eq 'RICHEDIT50W') { $script:perfEditorCount++ }
        return $true
    }
    [void][MDLitePerfNative]::EnumChildWindows($Parent, $callback, [IntPtr]::Zero)
    Assert-ProcessRunning $Process 'Count RichEdit windows'
    return $script:perfEditorCount
}

function ConvertTo-HexString([byte[]]$Bytes, [int]$Offset, [int]$Count) {
    if ($null -eq $Bytes -or $Count -le 0) { return '' }
    return [BitConverter]::ToString($Bytes, $Offset, $Count).Replace('-', '')
}

function Test-InputSourceRoundTrip([string]$Path, [string]$Suffix) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
    try {
        # Fixtures put Japanese text in their short header, followed by ASCII blocks.
        $probe = New-Object byte[] ([int][Math]::Min(256, $stream.Length))
        $probeCount = $stream.Read($probe, 0, $probe.Length)
        $utf8 = [Text.UTF8Encoding]::new($false, $true)
        try {
            [void]$utf8.GetString($probe, 0, $probeCount)
            $encoding = $utf8
            $encodingName = 'UTF-8'
        } catch [Text.DecoderFallbackException] {
            $encoding = [Text.Encoding]::GetEncoding(932)
            $encodingName = 'Windows-932'
        }
        $expected = $encoding.GetBytes($Suffix)
        if ($stream.Length -lt $expected.Length) {
            return [pscustomobject]@{
                pass = $false
                encoding = $encodingName
                expected_suffix_bytes = $expected.Length
                actual_suffix_bytes = 0
                tail_probe_bytes = $probeCount
                suffix_position = 'file-shorter-than-input'
                actual_tail_hex = ConvertTo-HexString $probe 0 $probeCount
            }
        }

        # RichEdit keeps a terminal paragraph break. Saving text entered at the
        # end of the visible document can therefore emit one final CRLF/LF/CR
        # after the input. Require the exact input suffix at EOF or immediately
        # before that single document terminator; do not accept arbitrary
        # substring matches or extra content after the measured edit.
        $tailLength = [int][Math]::Min([long]($expected.Length + 2), $stream.Length)
        $tail = New-Object byte[] $tailLength
        [void]$stream.Seek(-$tailLength, [IO.SeekOrigin]::End)
        $tailCount = $stream.Read($tail, 0, $tail.Length)
        $suffixPosition = 'missing'
        $matches = $false
        $exactOffset = $tailCount - $expected.Length
        if ($exactOffset -ge 0) {
            $matches = $true
            for ($index = 0; $matches -and $index -lt $expected.Length; $index++) {
                if ($tail[$exactOffset + $index] -ne $expected[$index]) { $matches = $false }
            }
            if ($matches) { $suffixPosition = 'end-of-file' }
        }
        if (-not $matches) {
            foreach ($terminator in @(
                [byte[]]@(0x0D, 0x0A),
                [byte[]]@(0x0A),
                [byte[]]@(0x0D)
            )) {
                $offset = $tailCount - $expected.Length - $terminator.Length
                if ($offset -lt 0) { continue }
                $candidateMatches = $true
                for ($index = 0; $candidateMatches -and $index -lt $expected.Length; $index++) {
                    if ($tail[$offset + $index] -ne $expected[$index]) { $candidateMatches = $false }
                }
                for ($index = 0; $candidateMatches -and $index -lt $terminator.Length; $index++) {
                    if ($tail[$offset + $expected.Length + $index] -ne $terminator[$index]) { $candidateMatches = $false }
                }
                if ($candidateMatches) {
                    $matches = $true
                    $suffixPosition = switch ($terminator.Length) {
                        2 { 'before-terminal-crlf'; break }
                        1 { if ($terminator[0] -eq 0x0A) { 'before-terminal-lf' } else { 'before-terminal-cr' }; break }
                    }
                    break
                }
            }
        }
        return [pscustomobject]@{
            pass = $matches
            encoding = $encodingName
            expected_suffix_bytes = $expected.Length
            actual_suffix_bytes = if ($matches) { $expected.Length } else { 0 }
            tail_probe_bytes = $tailCount
            suffix_position = $suffixPosition
            actual_tail_hex = ConvertTo-HexString $tail 0 $tailCount
        }
    } finally {
        $stream.Dispose()
    }
}

function Wait-MainWindow([Diagnostics.Process]$Process, [int]$TimeoutMs = 120000) {
    $watch = [Diagnostics.Stopwatch]::StartNew()
    do {
        if ($Process.HasExited) { throw "MDLite exited during startup with code $($Process.ExitCode)" }
        $script:perfMain = [IntPtr]::Zero
        $callback = [MDLitePerfNative+EnumWindowsProc]{
            param([IntPtr]$window, [IntPtr]$parameter)
            [uint32]$processId = 0
            [void][MDLitePerfNative]::GetWindowThreadProcessId($window, [ref]$processId)
            if ($processId -ne [uint32]$Process.Id) { return $true }
            $name = New-Object Text.StringBuilder 128
            [void][MDLitePerfNative]::GetClassName($window, $name, $name.Capacity)
            if ($name.ToString() -ne 'MDLite.MainWindow') { return $true }
            $script:perfMain = $window
            return $false
        }
        [void][MDLitePerfNative]::EnumWindows($callback, [IntPtr]::Zero)
        if ($script:perfMain -ne [IntPtr]::Zero -and -not [MDLitePerfNative]::IsHungAppWindow($script:perfMain)) {
            return [pscustomobject]@{ handle = $script:perfMain; elapsed_ms = $watch.Elapsed.TotalMilliseconds }
        }
        Start-Sleep -Milliseconds 10
    } while ($watch.ElapsedMilliseconds -lt $TimeoutMs)
    throw "MDLite did not create a responding main window within ${TimeoutMs}ms (exited=$($Process.HasExited), handle=$script:perfMain)"
}

function Get-Percentile([double[]]$Values, [double]$Fraction) {
    if ($Values.Count -eq 0) { return $null }
    $sorted = @($Values | Sort-Object)
    $index = [Math]::Ceiling($Fraction * $sorted.Count) - 1
    return [Math]::Round($sorted[[Math]::Max(0, $index)], 3)
}

function Get-ResourceSample([Diagnostics.Process]$Process, [Diagnostics.Stopwatch]$Watch, [IntPtr]$Window) {
    Assert-ProcessRunning $Process 'Collect resource sample'
    return [pscustomobject]@{
        elapsed_ms = $Watch.ElapsedMilliseconds
        working_set_bytes = $Process.WorkingSet64
        private_bytes = $Process.PrivateMemorySize64
        process_cpu_ms = [Math]::Round($Process.TotalProcessorTime.TotalMilliseconds, 3)
        handles = $Process.HandleCount
        gdi_objects = [MDLitePerfNative]::GetGuiResources($Process.Handle, 0)
        user_objects = [MDLitePerfNative]::GetGuiResources($Process.Handle, 1)
        main_pane_editor_windows = Count-EditorWindows $Process $Window
        responding = -not [MDLitePerfNative]::IsHungAppWindow($Window)
    }
}

function Wait-InputDebounces([Diagnostics.Process]$Process, [IntPtr]$Window,
                             [Diagnostics.Stopwatch]$InputWatch, [double]$CpuStartMs,
                             [string]$Context) {
    # Product input handling intentionally defers source synchronization for
    # 500ms, then presentation for another 250ms. Wait through both deadlines
    # while checking for a crash/hung window so the observation includes the
    # deferred hot path rather than only the synchronous WM_CHAR return.
    $minimumSettleMs = 750
    do {
        Assert-ProcessRunning $Process "$Context settled input"
        Start-Sleep -Milliseconds 25
    } while ($InputWatch.Elapsed.TotalMilliseconds -lt $minimumSettleMs)

    $responsiveDeadline = [DateTime]::UtcNow.AddSeconds(5)
    while ([MDLitePerfNative]::IsHungAppWindow($Window) -and [DateTime]::UtcNow -lt $responsiveDeadline) {
        Assert-ProcessRunning $Process "$Context settled input"
        Start-Sleep -Milliseconds 25
    }
    Assert-ProcessRunning $Process "$Context settled input"
    if ([MDLitePerfNative]::IsHungAppWindow($Window)) {
        throw "$Context settled input did not return to a responsive window after both debounce deadlines."
    }

    $Process.Refresh()
    return [pscustomobject]@{
        elapsed_ms = [Math]::Round($InputWatch.Elapsed.TotalMilliseconds, 3)
        process_cpu_ms = [Math]::Round([Math]::Max(0, $Process.TotalProcessorTime.TotalMilliseconds - $CpuStartMs), 3)
    }
}

function Measure-Scenario([string]$Name, [string[]]$Targets, [bool]$MeasureInput, [bool]$MeasureSearch,
                          [int]$StableMs = 1500, [bool]$MeasureSearchCancel = $false,
                          [int]$CompactCount = 0, [string]$ThemeChange = '',
                          [bool]$MeasureSettledInput = $false) {
    Write-Host "Measuring $Name"
    if (-not $Name.StartsWith('P5-')) { Start-Sleep -Milliseconds 500 }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $openedDocumentBytes = [long](($Targets | ForEach-Object {
        (Get-Item -LiteralPath $_).Length
    } | Measure-Object -Sum).Sum)
    if ($Targets.Count -gt 0 -and $Name -match '^P[123]-') {
        $targetWorkspace = [IO.Path]::GetFullPath((Split-Path -Parent $Targets[0]))
        $performanceRoot = [IO.Path]::GetFullPath($runRoot) + [IO.Path]::DirectorySeparatorChar
        if (-not $targetWorkspace.StartsWith($performanceRoot, [StringComparison]::OrdinalIgnoreCase)) {
            throw "Refusing to reset session outside the generated performance fixture: $targetWorkspace"
        }
        $previousSession = Join-Path $targetWorkspace '.mdlite\.state\session.toml'
        if (Test-Path -LiteralPath $previousSession -PathType Leaf) {
            # Each P1/P2/P3 scenario is an independent workload. The app's
            # normal clean exit persists its last session, so remove only that
            # generated fixture state before the next isolated measurement.
            Remove-Item -LiteralPath $previousSession -Force
        }
    }
    if ($null -eq $Targets -or $Targets.Count -eq 0) {
        $process = Start-Process -FilePath $executable -PassThru
    } else {
        $process = Start-Process -FilePath $executable -ArgumentList $Targets -PassThru
    }
    $main = [IntPtr]::Zero
    $samples = New-Object Collections.Generic.List[object]
    try {
        $ready = Wait-MainWindow $process
        $main = $ready.handle
        $documentReadyMs = $null
        $documentReadyTabCount = $null
        if ($Targets.Count -gt 0) {
            $documentTimeoutSeconds = if ($Profile -eq 'full') { 300 } else { 60 }
            $documentDeadline = [DateTime]::UtcNow.AddSeconds($documentTimeoutSeconds)
            $initialTabs = Find-Control $process $main $ControlTabs
            $initialEditor = [IntPtr]::Zero
            $initialLength = 0L
            do {
                Assert-ProcessRunning $process "$Name document load"
                if ([MDLitePerfNative]::IsHungAppWindow($main)) {
                    Start-Sleep -Milliseconds 25
                    continue
                }
                if ($initialTabs -eq [IntPtr]::Zero) {
                    $initialTabs = Find-Control $process $main $ControlTabs
                }
                if ($initialTabs -eq [IntPtr]::Zero) {
                    Start-Sleep -Milliseconds 10
                    continue
                }
                $documentReadyTabCount = (Send-NativeMessage $process $initialTabs $TCM_GETITEMCOUNT `
                    ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name startup tab count").ToInt32()
                # Opening multiple command-line paths creates each tab in sequence,
                # suspending the previous editor as it goes. Wait for the final tab
                # notification before retaining an editor HWND for text probing.
                if ($documentReadyTabCount -ne $Targets.Count) {
                    Start-Sleep -Milliseconds 10
                    continue
                }
                $initialEditor = Find-Control $process $main $ControlEditor
                if ($initialEditor -ne [IntPtr]::Zero) {
                    $initialLength = (Send-NativeMessage $process $initialEditor $WM_GETTEXTLENGTH `
                        ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name document length").ToInt64()
                    if ($initialLength -gt 0) { break }
                }
                Start-Sleep -Milliseconds 10
            } while ([DateTime]::UtcNow -lt $documentDeadline)
            if ($initialEditor -eq [IntPtr]::Zero -or $initialLength -eq 0) {
                throw "The initial document did not finish loading $($Targets.Count) tabs within $documentTimeoutSeconds seconds (observed tabs=$documentReadyTabCount)."
            }
            $documentReadyMs = [Math]::Round($watch.Elapsed.TotalMilliseconds, 3)
        }
        $deadline = [DateTime]::UtcNow.AddMilliseconds($StableMs)
        do {
            $samples.Add((Get-ResourceSample $process $watch $main))
            if ($StableMs -gt 0) { Start-Sleep -Milliseconds 250 }
        } while ([DateTime]::UtcNow -lt $deadline)
        $editor = Find-Control $process $main $ControlEditor
        $latencies = New-Object Collections.Generic.List[double]
        $settledInputMs = $null
        $settledInputCpuMs = $null
        $saveMs = $null
        $inputSourceRoundTrip = $null
        $inputEditorLengthBefore = $null
        $inputEditorLengthAfter = $null
        $inputEditorLengthAfterSave = $null
        $inputExpectedCharacters = $null
        if ($MeasureInput -and $editor -ne [IntPtr]::Zero) {
            $inputEditorLengthBefore = (Send-NativeMessage $process $editor $WM_GETTEXTLENGTH `
                ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name input text length before").ToInt64()
            [void](Send-NativeMessage $process $editor $EM_SETSEL `
                ([IntPtr]$inputEditorLengthBefore) ([IntPtr]$inputEditorLengthBefore) "$Name select input position")
            $process.Refresh()
            $settledCpuStartMs = $process.TotalProcessorTime.TotalMilliseconds
            $settledWatch = [Diagnostics.Stopwatch]::StartNew()
            # Cross-process WM_CHAR is not a faithful Unicode/IME injector for
            # RichEdit on these Japanese Windows fixtures; send ASCII here and
            # keep Japanese composition evidence in the separate GUI/native
            # acceptance routes.
            $inputText = 'input-latency-0123456789-abcdefghijklmnopqrstuvwxyz'
            $inputExpectedCharacters = $inputText.Length
            foreach ($character in $inputText.ToCharArray()) {
                $inputWatch = [Diagnostics.Stopwatch]::StartNew()
                [void](Send-NativeMessage $process $editor $WM_CHAR ([IntPtr][int]$character) ([IntPtr]::Zero) "$Name input character")
                $inputWatch.Stop()
                $latencies.Add($inputWatch.Elapsed.TotalMilliseconds)
            }
            if ($MeasureSettledInput) {
                $settled = Wait-InputDebounces $process $main $settledWatch $settledCpuStartMs $Name
                $settledInputMs = $settled.elapsed_ms
                $settledInputCpuMs = $settled.process_cpu_ms
                $samples.Add((Get-ResourceSample $process $watch $main))
            }
            $inputEditorLengthAfter = (Send-NativeMessage $process $editor $WM_GETTEXTLENGTH `
                ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name input text length after").ToInt64()
            $saveWatch = [Diagnostics.Stopwatch]::StartNew()
            [void](Send-NativeMessage $process $main $WM_COMMAND ([IntPtr]1005) ([IntPtr]::Zero) "$Name save")
            $saveWatch.Stop()
            $saveMs = [Math]::Round($saveWatch.Elapsed.TotalMilliseconds, 3)
            $inputEditorLengthAfterSave = (Send-NativeMessage $process $editor $WM_GETTEXTLENGTH `
                ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name input text length after save").ToInt64()
            Start-Sleep -Milliseconds 250
            $samples.Add((Get-ResourceSample $process $watch $main))
            $tabs = Find-Control $process $main $ControlTabs
            if ($tabs -eq [IntPtr]::Zero) { throw "$Name input source round-trip: tab control is unavailable." }
            $activeTabIndex = (Send-NativeMessage $process $tabs $TCM_GETCURSEL `
                ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name active input tab index").ToInt32()
            if ($activeTabIndex -lt 0 -or $activeTabIndex -ge $Targets.Count) {
                throw "$Name input source round-trip: active tab index $activeTabIndex is outside $($Targets.Count) targets."
            }
            $inputSourceRoundTrip = Test-InputSourceRoundTrip $Targets[$activeTabIndex] $inputText
            $inputSourceRoundTrip | Add-Member -NotePropertyName target_tab_index -NotePropertyValue $activeTabIndex
            $inputSourceRoundTrip | Add-Member -NotePropertyName editor_length_before -NotePropertyValue $inputEditorLengthBefore
            $inputSourceRoundTrip | Add-Member -NotePropertyName editor_length_after -NotePropertyValue $inputEditorLengthAfter
            $inputSourceRoundTrip | Add-Member -NotePropertyName editor_length_after_save -NotePropertyValue $inputEditorLengthAfterSave
            $inputSourceRoundTrip | Add-Member -NotePropertyName expected_input_characters -NotePropertyValue $inputExpectedCharacters
            if (-not $inputSourceRoundTrip.pass) {
                throw "$Name input source round-trip failed for active tab $activeTabIndex ($($inputSourceRoundTrip.encoding)): emitted input suffix is absent at the saved document boundary (position=$($inputSourceRoundTrip.suffix_position), editor_length=$inputEditorLengthBefore->$inputEditorLengthAfter->$inputEditorLengthAfterSave, expected_input_characters=$inputExpectedCharacters, tail_hex=$($inputSourceRoundTrip.actual_tail_hex))."
            }
            if ($Name.StartsWith('P5-') -and ($inputEditorLengthAfter - $inputEditorLengthBefore) -ne $inputExpectedCharacters) {
                throw "$Name editor input length delta was $($inputEditorLengthAfter - $inputEditorLengthBefore); expected $inputExpectedCharacters characters."
            }
        }
        $cancelResult = [pscustomobject]@{ first_batch_seen=$false; stale_results_cleared=$false; completed=$false; elapsed_ms=$null }
        if ($MeasureSearchCancel) {
            $cancelResult = Invoke-SearchCancellation $process $main
            $samples.Add((Get-ResourceSample $process $watch $main))
        }
        $searchFirstResultMs = $null
        $searchSettledMs = $null
        $resultCount = $null
        if ($MeasureSearch) {
            $find = Find-Control $process $main $ControlFindEdit
            if ($find -ne [IntPtr]::Zero) {
                [void](Send-NativeTextMessage $process $find $WM_SETTEXT ([IntPtr]::Zero) 'performance token' "$Name set search query")
                $searchWatch = [Diagnostics.Stopwatch]::StartNew()
                # A real button command is queued. Cross-process synchronous
                # SendMessage would execute the modal progress dialog as a
                # nonqueued message and is not representative of user input.
                [void][MDLitePerfNative]::PostMessage($main, $WM_COMMAND, [IntPtr]108, [IntPtr]::Zero)
                $searchTimeoutSeconds = if ($Profile -eq 'full') { 300 } else { 60 }
                $searchDeadline = [DateTime]::UtcNow.AddSeconds($searchTimeoutSeconds)
                $lastCount = -1
                $lastChange = [DateTime]::UtcNow
                do {
                    Assert-ProcessRunning $process "$Name workspace search"
                    Start-Sleep -Milliseconds 20
                    $results = Find-Control $process $main $ControlFindResults
                    if ($results -ne [IntPtr]::Zero) {
                        $resultCount = (Send-NativeMessage $process $results $LVM_GETITEMCOUNT `
                            ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name result count").ToInt32()
                        if ($resultCount -gt 0 -and $null -eq $searchFirstResultMs) {
                            $searchFirstResultMs = [Math]::Round($searchWatch.Elapsed.TotalMilliseconds, 3)
                        }
                        if ($resultCount -ne $lastCount) {
                            $lastCount = $resultCount
                            $lastChange = [DateTime]::UtcNow
                        }
                    }
                } while (($null -eq $searchFirstResultMs -or
                          ([DateTime]::UtcNow - $lastChange).TotalMilliseconds -lt 750) -and
                         [DateTime]::UtcNow -lt $searchDeadline)
                $searchWatch.Stop()
                if ($null -eq $searchFirstResultMs) {
                    throw "Workspace search did not publish a first result within $searchTimeoutSeconds seconds."
                }
                $searchSettledMs = [Math]::Round($searchWatch.Elapsed.TotalMilliseconds, 3)
                $samples.Add((Get-ResourceSample $process $watch $main))
            }
        }
        $compactWindows = 0
        if ($CompactCount -gt 0) {
            $compactWindows = Open-CompactWindows $process $main ([Math]::Min($CompactCount, $Targets.Count))
            $samples.Add((Get-ResourceSample $process $watch $main))
        }
        $themeResult = [pscustomobject]@{ completed=$false; elapsed_ms=$null }
        if ($ThemeChange) {
            $workspace = Split-Path -Parent $Targets[0]
            $themeResult = Invoke-ThemeChange $process $main $workspace $ThemeChange
            $samples.Add((Get-ResourceSample $process $watch $main))
        }
        if ($MeasureSearchCancel -and (-not $cancelResult.first_batch_seen -or -not $cancelResult.completed)) {
            throw 'GUI search cancellation was requested but did not complete responsively.'
        }
        if ($CompactCount -gt 0 -and $compactWindows -lt $CompactCount) {
            throw "Requested $CompactCount compact windows but observed $compactWindows."
        }
        if ($ThemeChange -and -not $themeResult.completed) {
            throw "GUI theme change to '$ThemeChange' did not persist."
        }
        $last = $samples[$samples.Count - 1]
        $tabs = Find-Control $process $main $ControlTabs
        $mainTabCount = if ($tabs -ne [IntPtr]::Zero) {
            (Send-NativeMessage $process $tabs $TCM_GETITEMCOUNT `
                ([IntPtr]::Zero) ([IntPtr]::Zero) "$Name tab count").ToInt32()
        } else { $null }
        $mainEditorCount = Count-EditorWindows $process $main
        $expectedMainTabCount = if ($Name -eq 'P1-one-document') { 1 } `
            elseif ($Name -eq 'P1-six-documents') { 6 } else { $null }
        $expectedMainEditorCount = if ($null -ne $expectedMainTabCount) { 1 } else { $null }
        if ($null -ne $expectedMainTabCount -and
            ($mainTabCount -ne $expectedMainTabCount -or
             $mainEditorCount -ne $expectedMainEditorCount)) {
            throw "$Name`: expected $expectedMainTabCount main tabs and $expectedMainEditorCount main-pane RichEdit window, observed tabs=$mainTabCount, editors=$mainEditorCount."
        }
        return [pscustomobject]@{
            scenario = $Name
            status = 'PASS'
            opened_document_count = $Targets.Count
            main_tab_count = $mainTabCount
            main_pane_editor_windows = $mainEditorCount
            expected_main_tab_count = $expectedMainTabCount
            expected_main_pane_editor_count = $expectedMainEditorCount
            opened_document_bytes = $openedDocumentBytes
            ready_ms = [Math]::Round([double]$ready.elapsed_ms, 3)
            document_ready_ms = $documentReadyMs
            document_ready_tab_count = $documentReadyTabCount
            input_definition = if ($MeasureInput) { 'wall time until cross-process synchronous WM_CHAR returns after RichEdit processing and synchronous EN_CHANGE; excludes deferred source synchronization and presentation' } else { $null }
            input_count = $latencies.Count
            input_p50_ms = Get-Percentile $latencies.ToArray() 0.50
            input_p95_ms = Get-Percentile $latencies.ToArray() 0.95
            input_max_ms = if ($latencies.Count) { [Math]::Round(($latencies | Measure-Object -Maximum).Maximum, 3) } else { $null }
            input_settled_definition = if ($MeasureSettledInput) { 'wall time and process CPU from before the measured input burst through the 500ms source-sync plus 250ms presentation debounce deadlines and a responsive-window check' } else { $null }
            input_settled_ms = $settledInputMs
            input_settled_cpu_ms = $settledInputCpuMs
            save_command_ms = $saveMs
            input_editor_length_before = $inputEditorLengthBefore
            input_editor_length_after = $inputEditorLengthAfter
            input_editor_length_after_save = $inputEditorLengthAfterSave
            input_expected_characters = $inputExpectedCharacters
            input_source_round_trip = $inputSourceRoundTrip
            save_definition = if ($MeasureInput) { 'wall time until synchronous Save command returns, including immediate editor-to-source synchronization and disk write' } else { $null }
            search_publication_mode = if ($MeasureSearch) { 'progressive batches; first transition observed directly, settled is a 750ms no-count-change heuristic' } else { $null }
            search_first_result_ms = $searchFirstResultMs
            search_settled_ms = $searchSettledMs
            search_complete_ms = $null
            search_result_count = $resultCount
            search_cancel_requested = $MeasureSearchCancel
            search_cancel_trigger = if ($MeasureSearchCancel) { 'query-change generation after first progressive batch' } else { $null }
            search_cancel_first_batch_seen = $cancelResult.first_batch_seen
            search_cancel_stale_results_cleared = $cancelResult.stale_results_cleared
            search_cancel_completed = $cancelResult.completed
            search_cancel_ms = $cancelResult.elapsed_ms
            requested_compact_windows = $CompactCount
            observed_compact_windows = $compactWindows
            theme_change_requested = $ThemeChange
            theme_change_completed = $themeResult.completed
            theme_change_ms = $themeResult.elapsed_ms
            peak_working_set_bytes = ($samples | Measure-Object working_set_bytes -Maximum).Maximum
            peak_private_bytes = ($samples | Measure-Object private_bytes -Maximum).Maximum
            final_handles = $last.handles
            final_gdi_objects = $last.gdi_objects
            final_user_objects = $last.user_objects
            samples = $samples.ToArray()
        }
    }
    catch {
        throw "$Name`: $($_.Exception.Message)"
    }
    finally {
        if (-not $process.HasExited) {
            if ($main -ne [IntPtr]::Zero) {
                [void][MDLitePerfNative]::PostMessage($main, $WM_CLOSE, [IntPtr]::Zero, [IntPtr]::Zero)
            }
            if (-not $process.WaitForExit(5000)) { $process.Kill() }
        }
        $process.Dispose()
    }
}

function Add-ScenarioResult([string]$Name, [scriptblock]$Measurement) {
    $script:activePerformanceScenario = $Name
    $startedUtc = [DateTime]::UtcNow
    try {
        $script:scenarios.Add((& $Measurement)) | Out-Null
    } catch {
        $script:scenarios.Add([pscustomobject]@{
            scenario = $Name
            status = 'FAILED'
            elapsed_seconds = [Math]::Round(([DateTime]::UtcNow - $startedUtc).TotalSeconds, 3)
            error = $_.Exception.Message
        }) | Out-Null
    } finally {
        $script:activePerformanceScenario = $null
    }
}

$tempRootPath = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$tempRootPrefix = $tempRootPath.TrimEnd([char[]]@(
    [IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)) +
    [IO.Path]::DirectorySeparatorChar
$runRoot = [IO.Path]::GetFullPath((Join-Path $tempRootPrefix `
    ("mdlite-performance-" + [guid]::NewGuid().ToString('N'))))
if (-not $runRoot.StartsWith($tempRootPrefix, [StringComparison]::OrdinalIgnoreCase) -or
    (Split-Path -Leaf $runRoot) -notmatch '^mdlite-performance-[0-9a-f]{32}$') {
    throw "Generated performance fixture escaped its temp root: $runRoot"
}
[IO.Directory]::CreateDirectory($runRoot) | Out-Null
$scenarios = New-Object Collections.Generic.List[object]
$p5Rows = New-Object Collections.Generic.List[object]
$p5Failures = 0
try {
    $full = $Profile -eq 'full'
    $p1 = New-Workspace $runRoot 'P1'
    $p1Bytes = if ($full) { 20MB } else { 2MB }
    $p1Count = if ($full) { 2000 } else { 200 }
    $p1OpenBytes = if ($full) { 1MB } else { 256KB }
    $p1SixSizes = @([long]($p1OpenBytes / 6)) * 6
    $p1SixSizes[0] += $p1OpenBytes - [long](($p1SixSizes | Measure-Object -Sum).Sum)
    $p1Targets = New-DistributedFixture $p1 $p1Count $p1Bytes $true (@($p1OpenBytes) + $p1SixSizes)
    # Keep one CP932 note last so the active six-document edit/save round-trip
    # exercises both fixture encodings across P1/P2 single- and multi-tab cases.
    $p1SixTargets = @($p1Targets[2..6]) + @($p1Targets[1])

    $p2 = New-Workspace $runRoot 'P2'
    $p2Bytes = if ($full) { 200MB } else { 10MB }
    $p2Count = if ($full) { 10000 } else { 500 }
    $p2OpenBytes = if ($full) { 1MB } else { 256KB }
    $p2SixSizes = @([long]($p2OpenBytes / 6)) * 6
    $p2SixSizes[0] += $p2OpenBytes - [long](($p2SixSizes | Measure-Object -Sum).Sum)
    $p2Targets = New-DistributedFixture $p2 $p2Count $p2Bytes $true (@($p2OpenBytes) + $p2SixSizes)
    $p2SixTargets = @($p2Targets[2..6]) + @($p2Targets[1])
    $p1OneFixtureBytes = (Get-Item -LiteralPath $p1Targets[0]).Length
    $p1SixFixtureBytes = [long](($p1SixTargets | ForEach-Object {
        (Get-Item -LiteralPath $_).Length
    } | Measure-Object -Sum).Sum)
    $p2OneFixtureBytes = (Get-Item -LiteralPath $p2Targets[0]).Length
    $p2SixFixtureBytes = [long](($p2SixTargets | ForEach-Object {
        (Get-Item -LiteralPath $_).Length
    } | Measure-Object -Sum).Sum)

    $p3 = New-Workspace $runRoot 'P3'
    $p3Sizes = if ($full) { @(20MB, 100MB) } else { @(5MB, 10MB) }
    $p3Targets = @()
    foreach ($size in $p3Sizes) {
        $path = Join-Path $p3 ("large-{0}MiB.md" -f [int]($size / 1MB))
        Write-RepeatedFile $path $size ([Text.UTF8Encoding]::new($false)) ("# Large`nperformance token`n" + ('x' * [Math]::Min(1MB, [int]$size)) + "`n")
        $p3Targets += $path
    }

    $p4 = New-Workspace $runRoot 'P4'
    [IO.Directory]::CreateDirectory((Join-Path $p4 'assets')) | Out-Null
    Add-Type -AssemblyName System.Drawing
    $bitmap = [Drawing.Bitmap]::new(8, 8)
    try {
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        try { $graphics.Clear([Drawing.Color]::SteelBlue) } finally { $graphics.Dispose() }
        $bitmap.Save((Join-Path $p4 'assets\static.png'), [Drawing.Imaging.ImageFormat]::Png)
        $bitmap.Save((Join-Path $p4 'assets\static.jpg'), [Drawing.Imaging.ImageFormat]::Jpeg)
    } finally { $bitmap.Dispose() }
    $gifBytes = [byte[]]@(
        0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,0x00,0x00,0x00,0xFF,0xFF,0xFF,
        0x21,0xFF,0x0B,0x4E,0x45,0x54,0x53,0x43,0x41,0x50,0x45,0x32,0x2E,0x30,0x03,0x01,0x00,0x00,0x00,
        0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,
        0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x4C,0x01,0x00,0x3B)
    [IO.File]::WriteAllBytes((Join-Path $p4 'assets\animated-fast.gif'), $gifBytes)
    $gifBytes[42] = 0x19
    $gifBytes[65] = 0x19
    [IO.File]::WriteAllBytes((Join-Path $p4 'assets\animated-slow.gif'), $gifBytes)
    [IO.File]::WriteAllBytes((Join-Path $p4 'assets\static.webp'), [byte[]]@(
        0x52,0x49,0x46,0x46,0x1A,0x00,0x00,0x00,0x57,0x45,0x42,0x50,
        0x56,0x50,0x38,0x4C,0x0E,0x00,0x00,0x00,0x2F,0x00,0x00,0x00,
        0x10,0x07,0x10,0x11,0x11,0x88,0x88,0xFE,0x07,0x00))
    [IO.File]::WriteAllBytes(
        (Join-Path $p4 'assets\animated.webp'),
        [Convert]::FromBase64String('UklGRooAAABXRUJQVlA4WAoAAAACAAAAAQAAAAAAQU5JTQYAAAD/////AABBTk1GKgAAAAAAAAAAAAEAAAAAAGQAAAJWUDhMEQAAAC8BAAAAD7D/8x/zHxUyov8BAEFOTUYsAAAAAAAAAAAAAQAAAAAA+gAAAFZQOEwTAAAALwEAAAAPMP/zP//zHzyoQET/AwA='))
    [IO.File]::WriteAllText((Join-Path $p4 'assets\safe.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#4682b4"/></svg>', [Text.UTF8Encoding]::new($false))
    $p4Docs = @()
    foreach ($index in 1..3) {
        $p4Doc = Join-Path $p4 ("images-{0}.md" -f $index)
        [IO.File]::WriteAllText($p4Doc,
            "# Images $index`r`n`r`n![png](assets/static.png)`r`n`r`n![jpeg](assets/static.jpg)`r`n`r`n![webp](assets/static.webp)`r`n`r`n![animated-webp](assets/animated.webp)`r`n`r`n![fast](assets/animated-fast.gif)`r`n`r`n![slow](assets/animated-slow.gif)`r`n`r`n![vector](assets/safe.svg)`r`n",
            [Text.UTF8Encoding]::new($false))
        $p4Docs += $p4Doc
    }

    $p5 = New-Workspace $runRoot 'P5'
    $p5Doc = Join-Path $p5 'cycle.md'
    $p5Seed = "# Cycle`r`nperformance token`r`n"
    $p5ExpectedInputCharacters = 'input-latency-0123456789-abcdefghijklmnopqrstuvwxyz'.Length
    [IO.File]::WriteAllText($p5Doc, $p5Seed, [Text.UTF8Encoding]::new($false))

    Add-ScenarioResult 'P0-first-run-approximation' { Measure-Scenario 'P0-first-run-approximation' @() $false $false 1500 }
    Add-ScenarioResult 'P0-warm' { Measure-Scenario 'P0-warm' @() $false $false 1500 }
    # P1 isolates document editing memory; workspace search has dedicated P2 scenarios.
    Add-ScenarioResult 'P1-one-document' { Measure-Scenario -Name 'P1-one-document' -Targets @($p1Targets[0]) -MeasureInput $true -MeasureSearch $false -StableMs 1500 -MeasureSettledInput $true }
    Add-ScenarioResult 'P1-six-documents' { Measure-Scenario -Name 'P1-six-documents' -Targets $p1SixTargets -MeasureInput $true -MeasureSearch $false -StableMs 1500 -MeasureSettledInput $true }
    Add-ScenarioResult 'P2-one-document-search' { Measure-Scenario 'P2-one-document-search' @($p2Targets[0]) $true $true 1000 $full }
    Add-ScenarioResult 'P2-six-documents-search' { Measure-Scenario 'P2-six-documents-search' $p2SixTargets $true $true 1000 $false }
    foreach ($target in $p3Targets) {
        $size = [int]((Get-Item -LiteralPath $target).Length / 1MB)
        Add-ScenarioResult "P3-${size}MiB" { Measure-Scenario "P3-${size}MiB" @($target) $true $true 1000 }
    }
    Add-ScenarioResult 'P4-images-and-compacts' { Measure-Scenario -Name 'P4-images-and-compacts' -Targets $p4Docs -MeasureInput $true -MeasureSearch $false -StableMs 2500 -CompactCount 3 -MeasureSettledInput $true }

    for ($iteration = 1; $iteration -le $P5Iterations; $iteration++) {
        try {
            [IO.File]::WriteAllText($p5Doc, $p5Seed, [Text.UTF8Encoding]::new($false))
            $theme = if ($iteration % 2 -eq 0) { 'light' } else { 'dark' }
            $row = Measure-Scenario ("P5-{0:D3}" -f $iteration) @($p5Doc) $true $true 0 $false 0 $theme
            $p5Rows.Add([pscustomobject]@{
                iteration=$iteration
                input_editor_length_before=$row.input_editor_length_before
                input_editor_length_after=$row.input_editor_length_after
                input_editor_length_after_save=$row.input_editor_length_after_save
                input_editor_length_delta=($row.input_editor_length_after - $row.input_editor_length_before)
                input_expected_characters=$row.input_expected_characters
                input_source_round_trip_pass=$row.input_source_round_trip.pass
                ready_ms=$row.ready_ms
                input_p95_ms=$row.input_p95_ms
                search_first_result_ms=$row.search_first_result_ms
                search_settled_ms=$row.search_settled_ms
                theme_change_requested=$row.theme_change_requested
                theme_change_completed=$row.theme_change_completed
                theme_change_ms=$row.theme_change_ms
                peak_private_bytes=$row.peak_private_bytes
                final_handles=$row.final_handles
                final_gdi_objects=$row.final_gdi_objects
            })
        } catch {
            $p5Failures++
            $p5Rows.Add([pscustomobject]@{
                iteration=$iteration
                input_editor_length_before=$null
                input_editor_length_after=$null
                input_editor_length_after_save=$null
                input_editor_length_delta=$null
                input_expected_characters=$p5ExpectedInputCharacters
                input_source_round_trip_pass=$false
                ready_ms=$null
                input_p95_ms=$null
                search_first_result_ms=$null
                search_settled_ms=$null
                theme_change_requested=$theme
                theme_change_completed=$false
                theme_change_ms=$null
                peak_private_bytes=$null
                final_handles=$null
                final_gdi_objects=$null
                error=$_.Exception.Message
            })
        }
    }

    $fixtureFiles = Get-ChildItem -LiteralPath $runRoot -File -Recurse
    $gitExe = @((Get-Command git -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
                'C:\Program Files\Git\cmd\git.exe') | Where-Object { $_ -and (Test-Path -LiteralPath $_ -PathType Leaf) } | Select-Object -First 1
    $buildSha = $null
    $worktreeDirty = $null
    if ($gitExe) {
        $buildSha = (& $gitExe -C $repoRoot rev-parse HEAD 2>$null | Select-Object -First 1)
        $worktreeDirty = @(& $gitExe -C $repoRoot status --porcelain=v1 2>$null).Count -gt 0
    }
    $compilerPath = $null
    $compilerVersion = $null
    $cachePath = Join-Path $repoRoot "build\$Preset\CMakeCache.txt"
    if (Test-Path -LiteralPath $cachePath -PathType Leaf) {
        $compilerLine = Get-Content -LiteralPath $cachePath | Where-Object { $_ -match '^CMAKE_CXX_COMPILER:FILEPATH=' } | Select-Object -First 1
        if ($compilerLine) {
            $compilerPath = $compilerLine.Substring($compilerLine.IndexOf('=') + 1)
            if (Test-Path -LiteralPath $compilerPath -PathType Leaf) {
                $compilerVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($compilerPath).FileVersion
            }
        }
    }
    $environmentSource = 'CIM'
    try {
        $computer = Get-CimInstance Win32_ComputerSystem -ErrorAction Stop
        $processor = Get-CimInstance Win32_Processor -ErrorAction Stop | Select-Object -First 1
        $os = Get-CimInstance Win32_OperatingSystem -ErrorAction Stop
        $osName = $os.Caption
        $osBuild = $os.BuildNumber
        $cpuName = $processor.Name.Trim()
        $logicalProcessors = $computer.NumberOfLogicalProcessors
        $ramBytes = [long]$computer.TotalPhysicalMemory
    } catch {
        # Performance evidence must remain usable in restricted runners where
        # WMI/CIM is denied. These values are metadata, not scenario results.
        $environmentSource = 'dotnet-registry-fallback'
        $osName = [Environment]::OSVersion.VersionString
        $osBuild = [Environment]::OSVersion.Version.Build.ToString()
        $logicalProcessors = [Environment]::ProcessorCount
        $cpuName = $env:PROCESSOR_IDENTIFIER
        try {
            $cpuName = (Get-ItemProperty -LiteralPath 'HKLM:\HARDWARE\DESCRIPTION\System\CentralProcessor\0' -ErrorAction Stop).ProcessorNameString.Trim()
        } catch {}
        $ramBytes = $null
        try {
            Add-Type -AssemblyName Microsoft.VisualBasic -ErrorAction Stop
            $ramBytes = [long]([Microsoft.VisualBasic.Devices.ComputerInfo]::new().TotalPhysicalMemory)
        } catch {}
    }
    $scenarioFailureCount = @($scenarios | Where-Object { $_.status -eq 'FAILED' }).Count
    $executableSha256AtEnd = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $executableUnchangedDuringRun = $executableSha256AtStart -eq $executableSha256AtEnd
    $sourceAtMeasurementEnd = Get-SourceFingerprint $repoRoot
    $sourceUnchangedDuringRun = $sourceAtMeasurementStart.sha256 -eq $sourceAtMeasurementEnd.sha256
    $buildReceiptSha256AtEnd = (Get-FileHash -Algorithm SHA256 -LiteralPath $buildReceiptPath).Hash.ToLowerInvariant()
    $buildReceiptUnchangedDuringRun = $buildReceiptSha256 -eq $buildReceiptSha256AtEnd
    $result = [pscustomobject]@{
        schema = 'mdlite-performance-v5'
        run_id = $measurementRunId
        timestamp_utc = [DateTime]::UtcNow.ToString('o')
        measurement_status = if ($scenarioFailureCount -eq 0 -and $p5Failures -eq 0 -and
            $executableUnchangedDuringRun -and $sourceUnchangedDuringRun -and
            $buildReceiptUnchangedDuringRun) { 'PASS' } else { 'FAILED_OR_INCOMPLETE' }
        profile = $Profile
        preset = $Preset
        executable_sha256 = $executableSha256AtStart
        source_sha256_at_start = $sourceAtMeasurementStart.sha256
        source_sha256_at_end = $sourceAtMeasurementEnd.sha256
        source_unchanged_during_run = $sourceUnchangedDuringRun
        build = [pscustomobject]@{
            build_run_id = $buildReceipt.build_run_id
            receipt_path = [IO.Path]::GetFullPath($buildReceiptPath)
            receipt_sha256 = $buildReceiptSha256
            receipt_sha256_at_end = $buildReceiptSha256AtEnd
            receipt_unchanged_during_run = $buildReceiptUnchangedDuringRun
            completed_utc = $buildReceipt.completed_utc
            configure_exit_code = $buildReceipt.configure_exit_code
            build_exit_code = $buildReceipt.build_exit_code
            test_requested = $buildReceipt.test_requested
            test_exit_code = $buildReceipt.test_exit_code
            executable_path = [IO.Path]::GetFullPath($executable)
            executable_sha256 = $buildReceipt.executable_sha256.ToLowerInvariant()
            executable_sha256_at_end = $executableSha256AtEnd
            executable_unchanged_during_run = $executableUnchangedDuringRun
            source_sha256 = $buildReceipt.source_sha256
            source_file_count = $buildReceipt.source_file_count
            git_head = $buildSha
            worktree_dirty = $worktreeDirty
            compiler_path = $compilerPath
            compiler_version = $compilerVersion
        }
        environment = [pscustomobject]@{
            source = $environmentSource
            os = $osName
            os_build = $osBuild
            cpu = $cpuName
            logical_processors = $logicalProcessors
            ram_bytes = $ramBytes
            powershell = $PSVersionTable.PSVersion.ToString()
        }
        fixture = [pscustomobject]@{
            files = $fixtureFiles.Count
            bytes = ($fixtureFiles | Measure-Object Length -Sum).Sum
            p1_files = $p1Count
            p1_bytes = $p1Bytes
            p1_one_document_bytes = $p1OneFixtureBytes
            p1_six_documents_bytes = $p1SixFixtureBytes
            p2_files = $p2Count
            p2_bytes = $p2Bytes
            p2_one_document_bytes = $p2OneFixtureBytes
            p2_six_documents_bytes = $p2SixFixtureBytes
            p3_bytes = @($p3Sizes)
            p4_documents = $p4Docs.Count
            p4_assets = 7
            encoding = 'P1/P2 alternate UTF-8 no BOM and Windows-932'
        }
        scenarios = $scenarios.ToArray()
        scenario_failures = $scenarioFailureCount
        p5 = [pscustomobject]@{
            iterations = $P5Iterations
            failures = $p5Failures
            private_bytes_first = if ($p5Rows.Count) { $p5Rows[0].peak_private_bytes } else { $null }
            private_bytes_last = if ($p5Rows.Count) { $p5Rows[$p5Rows.Count - 1].peak_private_bytes } else { $null }
            rows = $p5Rows.ToArray()
        }
        limitations = @(
            'P0-first-run-approximation is the first launch in this run; the script does not flush the Windows filesystem cache and therefore does not call it a controlled cold start.',
            'Per-character input timing ends when synchronous WM_CHAR returns. P1/P4 additionally record wall time and process CPU through the 500ms source-sync plus 250ms presentation debounce deadlines; save_command_ms separately includes forced synchronization plus save.',
            'Workspace search first-result timing is the first observed ListView count transition. The external runner cannot read the private completion-generation message safely, so search_complete_ms remains null and search_settled_ms is explicitly a 750ms no-count-change heuristic.',
            $(if ($full) { 'P2 cancellation changes the live query after the first progressive batch, exercising generation cancellation/stale-result discard and recording return to a responsive window.' } else { 'Quick profile skips the P2 GUI cancellation injection; full profile is required for that evidence.' }),
            'P4 opens three image documents and requests three compact windows with PNG/JPEG/static and animated WebP/two GIF periods/SVG. Viewport-offscreen decode suppression and subjective animation quality remain separately unmeasured.',
            'P3 records load, visible input dispatch, forced source-sync/save, and search completion. Resources after process exit cannot be sampled from the exited process; P5 compares repeated final in-process samples instead.',
            'Performance input uses ASCII WM_CHAR messages; cross-process WM_CHAR does not model Unicode IME composition. Japanese input and composition remain separate GUI/native acceptance evidence.',
            'P5 performs launch, input/save, workspace search, persisted theme application, and clean exit in every iteration. The theme step uses a test-only silent window message that calls the same workspace settings save/reload/application path without opening prompt or result dialogs.'
        )
    }
    $directory = Split-Path -Parent $OutputPath
    [IO.Directory]::CreateDirectory($directory) | Out-Null
    $result | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $result | ConvertTo-Json -Depth 5
    if ($scenarioFailureCount -ne 0 -or $p5Failures -ne 0 -or
        -not $executableUnchangedDuringRun -or -not $sourceUnchangedDuringRun -or
        -not $buildReceiptUnchangedDuringRun) { exit 1 }
}
finally {
    if (Test-Path -LiteralPath $runRoot) {
        $resolvedRunRoot = [IO.Path]::GetFullPath($runRoot)
        $runRootLeaf = Split-Path -Leaf $resolvedRunRoot
        if ($resolvedRunRoot.StartsWith($tempRootPrefix, [StringComparison]::OrdinalIgnoreCase) -and
            $runRootLeaf -match '^mdlite-performance-[0-9a-f]{32}$') {
            Remove-Item -LiteralPath $resolvedRunRoot -Recurse -Force
        } else {
            Write-Warning "Refusing to remove an unexpected performance fixture path: $resolvedRunRoot"
        }
    }
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
    [void][MDLitePerfNative]::SetErrorMode($previousErrorMode)
}
