[CmdletBinding()]
param([int]$DelaySeconds = 8, [string]$OnlyCase = 'all')

Add-Type -TypeDefinition @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public static class MDLiteP4Probe {
    public delegate bool EnumChildrenProc(IntPtr window, IntPtr parameter);
    public delegate bool EnumWindowsProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern bool EnumChildWindows(IntPtr parent, EnumChildrenProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc callback, IntPtr parameter);
    [DllImport("user32.dll")]
    public static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)]
    public static extern int GetClassName(IntPtr window, StringBuilder className, int capacity);
    [DllImport("user32.dll", EntryPoint="SendMessageW")]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    public static IntPtr FindControl(IntPtr parent, int id) {
        IntPtr found = IntPtr.Zero;
        EnumChildWindows(parent, (window, unused) => {
            var name = new StringBuilder(128);
            GetClassName(window, name, name.Capacity);
            if (GetDlgCtrlID(window) == id || name.ToString() == "RICHEDIT50W") {
                found = window; return false;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
    public static IntPtr FindMainWindow(uint processId) {
        IntPtr found = IntPtr.Zero;
        EnumWindows((window, unused) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner != processId) return true;
            var name = new StringBuilder(128);
            GetClassName(window, name, name.Capacity);
            if (name.ToString() == "MDLite.MainWindow") { found = window; return false; }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
$root = Join-Path $repo 'build\verification\p4-crash-probe'
[IO.Directory]::CreateDirectory($root) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $root 'assets')) | Out-Null

Add-Type -AssemblyName System.Drawing
$bitmap = [Drawing.Bitmap]::new(8, 8)
try {
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try { $graphics.Clear([Drawing.Color]::SteelBlue) } finally { $graphics.Dispose() }
    $bitmap.Save((Join-Path $root 'assets\static.png'), [Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Save((Join-Path $root 'assets\static.jpg'), [Drawing.Imaging.ImageFormat]::Jpeg)
} finally { $bitmap.Dispose() }

$gifBytes = [byte[]]@(
    0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,0x00,0x00,0x00,0xFF,0xFF,0xFF,
    0x21,0xFF,0x0B,0x4E,0x45,0x54,0x53,0x43,0x41,0x50,0x45,0x32,0x2E,0x30,0x03,0x01,0x00,0x00,0x00,
    0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,
    0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x4C,0x01,0x00,0x3B)
[IO.File]::WriteAllBytes((Join-Path $root 'assets\animated-fast.gif'), $gifBytes)
$gifBytes[42] = 0x19
$gifBytes[65] = 0x19
[IO.File]::WriteAllBytes((Join-Path $root 'assets\animated-slow.gif'), $gifBytes)
[IO.File]::WriteAllBytes((Join-Path $root 'assets\static.webp'), [byte[]]@(
    0x52,0x49,0x46,0x46,0x1A,0x00,0x00,0x00,0x57,0x45,0x42,0x50,
    0x56,0x50,0x38,0x4C,0x0E,0x00,0x00,0x00,0x2F,0x00,0x00,0x00,
    0x10,0x07,0x10,0x11,0x11,0x88,0x88,0xFE,0x07,0x00))
[IO.File]::WriteAllBytes((Join-Path $root 'assets\animated.webp'),
    [Convert]::FromBase64String('UklGRooAAABXRUJQVlA4WAoAAAACAAAAAQAAAAAAQU5JTQYAAAD/////AABBTk1GKgAAAAAAAAAAAAEAAAAAAGQAAAJWUDhMEQAAAC8BAAAAD7D/8x/zHxUyov8BAEFOTUYsAAAAAAAAAAAAAQAAAAAA+gAAAFZQOEwTAAAALwEAAAAPMP/zP//zHzyoQET/AwA='))
[IO.File]::WriteAllText((Join-Path $root 'assets\safe.svg'),
    '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#4682b4"/></svg>',
    [Text.UTF8Encoding]::new($false))

$allAssets = @('static.png','static.jpg','static.webp','animated.webp',
               'animated-fast.gif','animated-slow.gif','safe.svg')
$cases = [ordered]@{
    one_static = @('static.png','static.jpg','static.webp','safe.svg')
    one_gif = @('animated-fast.gif','animated-slow.gif')
    one_webp = @('animated.webp')
    one_all = $allAssets
    one_all_compact = $allAssets
    three_all = $allAssets
    one_all_edit = $allAssets
    one_all_edit_compact = $allAssets
    three_all_edit = $allAssets
    three_all_edit_compact = $allAssets
}
$executable = Join-Path $repo 'build\release\MDLite.exe'
$oldCrashSetting = $env:MDLITE_TEST_NO_CRASH_UI
$oldSilentSetting = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_NO_CRASH_UI = '1'
$env:MDLITE_TEST_SILENT = '1'
$results = [Collections.Generic.List[object]]::new()
$newLine = [string]([char]13) + [char]10
try {
    foreach ($case in $cases.GetEnumerator()) {
        if ($OnlyCase -ne 'all' -and $case.Key -ne $OnlyCase) { continue }
        $workspace = Join-Path $root $case.Key
        [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
        $settings = 'schema_version = 1' + [char]10 + 'auto_save = false' + [char]10 + 'theme = "dark"' + [char]10
        [IO.File]::WriteAllText((Join-Path $workspace '.mdlite\settings.toml'), $settings,
            [Text.UTF8Encoding]::new($false))
        $count = if ($case.Key.StartsWith('three_')) { 3 } else { 1 }
        $targets = [Collections.Generic.List[string]]::new()
        for ($index = 1; $index -le $count; $index++) {
            $path = Join-Path $workspace ("probe-{0}.md" -f $index)
            $links = @($case.Value | ForEach-Object { "![probe](assets/$_)" }) -join $newLine
            $body = '# P4 probe ' + $index + $newLine + $newLine + $links + $newLine
            [IO.File]::WriteAllText($path, $body, [Text.UTF8Encoding]::new($false))
            $targets.Add($path)
        }
        $process = Start-Process -FilePath $executable -ArgumentList $targets.ToArray() -PassThru
        $failureStage = 'initial-window'
        $main = [IntPtr]::Zero
        $mainDeadline = [DateTime]::UtcNow.AddSeconds(10)
        do {
            $process.Refresh()
            $main = [MDLiteP4Probe]::FindMainWindow([uint32]$process.Id)
            if ($main -ne [IntPtr]::Zero) {
                break
            }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $mainDeadline -and -not $process.HasExited)
        $editSent = $false
        $editorFound = $false
        $compactCommandSent = $false
        if ($case.Key.Contains('_edit') -and -not $process.HasExited -and $main -ne [IntPtr]::Zero) {
            $editor = [IntPtr]::Zero
            $editorDeadline = [DateTime]::UtcNow.AddSeconds(20)
            do {
                $editor = [MDLiteP4Probe]::FindControl($main, 102)
                if ($editor -ne [IntPtr]::Zero -or $process.HasExited) { break }
                Start-Sleep -Milliseconds 50
            } while ([DateTime]::UtcNow -lt $editorDeadline)
            $editorFound = $editor -ne [IntPtr]::Zero
            if ($editor -ne [IntPtr]::Zero) {
                $failureStage = 'send-edit'
                [void][MDLiteP4Probe]::SendMessage($editor, 0x00B1, [IntPtr]::new(-1), [IntPtr]::new(-1))
                foreach ($character in ('input-latency-日本語-0123456789-abcdefghijklmnopqrstuvwxyz'.ToCharArray())) {
                    [void][MDLiteP4Probe]::SendMessage($editor, 0x0102, [IntPtr][int]$character, [IntPtr]::Zero)
                }
                $failureStage = 'save-edit'
                [void][MDLiteP4Probe]::SendMessage($main, 0x0111, [IntPtr]1005, [IntPtr]::Zero)
                $editSent = $true
                Start-Sleep -Milliseconds 250
            }
        }
        if ($case.Key.EndsWith('_compact') -and -not $process.HasExited -and $main -ne [IntPtr]::Zero) {
            $tabs = [MDLiteP4Probe]::FindControl($main, 101)
            for ($index = 0; $index -lt $count -and $tabs -ne [IntPtr]::Zero; $index++) {
                $failureStage = "query-tab-center-$index"
                $packed = [MDLiteP4Probe]::SendMessage($main, 0x803A, [IntPtr]$index,
                    [IntPtr]::Zero).ToInt64()
                $packed = $packed -band 0xffffffffL
                $process.Refresh()
                if ($process.HasExited) { break }
                if ($packed -ne 0xffffffffL) {
                    $failureStage = "click-tab-$index"
                    $x = [int][short]($packed -band 0xffff)
                    $y = [int][short](($packed -shr 16) -band 0xffff)
                    $point = [IntPtr]([long](($y -band 0xffff) -shl 16) -bor ($x -band 0xffff))
                    [void][MDLiteP4Probe]::SendMessage($tabs, 0x0201, [IntPtr]1, $point)
                    [void][MDLiteP4Probe]::SendMessage($tabs, 0x0202, [IntPtr]::Zero, $point)
                }
                $failureStage = "toggle-compact-$index"
                [void][MDLiteP4Probe]::SendMessage($main, 0x0111, [IntPtr]1030, [IntPtr]::Zero)
                Start-Sleep -Milliseconds 50
                $process.Refresh()
                if ($process.HasExited) { break }
            }
            $compactCommandSent = $true
            Start-Sleep -Milliseconds 250
        }
        Start-Sleep -Seconds $DelaySeconds
        $process.Refresh()
        if ($process.HasExited) {
            $results.Add([pscustomobject]@{
                case = $case.Key; status = 'EXITED'; exit_code = $process.ExitCode
                targets = $count; edit_sent = $editSent; editor_found = $editorFound; compact_command_sent = $compactCommandSent; main_window_found = ($main -ne [IntPtr]::Zero); delay_seconds = $DelaySeconds
                failure_stage = $failureStage
            })
        } else {
            $closeRequested = $process.CloseMainWindow()
            if (-not $process.WaitForExit(3000)) {
                $process.Kill()
                [void]$process.WaitForExit(3000)
            }
            $results.Add([pscustomobject]@{
                case = $case.Key; status = 'ALIVE_AFTER_DELAY'; exit_code = $null
                targets = $count; edit_sent = $editSent; editor_found = $editorFound; compact_command_sent = $compactCommandSent; main_window_found = ($main -ne [IntPtr]::Zero); delay_seconds = $DelaySeconds
                close_requested = $closeRequested
            })
        }
        $process.Dispose()
    }
} finally {
    if ($null -eq $oldCrashSetting) {
        Remove-Item Env:MDLITE_TEST_NO_CRASH_UI -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_NO_CRASH_UI = $oldCrashSetting
    }
    if ($null -eq $oldSilentSetting) {
        Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue
    } else {
        $env:MDLITE_TEST_SILENT = $oldSilentSetting
    }
}
$results | ConvertTo-Json -Depth 5
