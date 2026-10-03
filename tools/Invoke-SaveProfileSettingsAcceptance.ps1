#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Preset = 'debug',
    [string]$OutputPath,
    [string[]]$OnlyCase = @()
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
$receiptPath = Join-Path $repoRoot "build\verification\build-receipt-$Preset.json"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Missing executable: $executable" }
if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) { throw "Missing build receipt: $receiptPath" }

function Get-SourceFingerprint([string]$Root) {
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd([char[]]@('\', '/')) + [IO.Path]::DirectorySeparatorChar
    [string[]]$paths = @(
        Get-ChildItem -LiteralPath (Join-Path $Root 'src') -File -Recurse -Force | Select-Object -ExpandProperty FullName
        Join-Path $Root 'CMakeLists.txt'
        Join-Path $Root 'CMakePresets.json'
    )
    [Array]::Sort($paths, [StringComparer]::Ordinal)
    $manifest = foreach ($path in $paths) {
        '{0}:{1}' -f ([IO.Path]::GetFullPath($path).Substring($prefix.Length).Replace('\', '/')),
            ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant())
    }
    $hasher = [Security.Cryptography.SHA256]::Create()
    try {
        $hash = [BitConverter]::ToString($hasher.ComputeHash(
            [Text.UTF8Encoding]::new($false).GetBytes([string]::Join([string][char]10, [string[]]$manifest)
        ))).Replace('-', '').ToLowerInvariant()
    } finally { $hasher.Dispose() }
    return [pscustomobject]@{ sha256 = $hash; file_count = $paths.Count }
}

$receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
$exeAtStart = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
$sourceAtStart = Get-SourceFingerprint $repoRoot
if ($receipt.build_status -ne 'PASS' -or
    $receipt.executable_path -ne [IO.Path]::GetFullPath($executable) -or
    $receipt.executable_sha256 -ne $exeAtStart -or
    $receipt.source_sha256 -ne $sourceAtStart.sha256) {
    throw 'Current source, executable, and build receipt do not match. Rebuild this preset before running GUI cases.'
}

$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0, 8)
$runRoot = Join-Path $repoRoot "build\verification\prehuman-save\focused-$Preset-$runId"
[IO.Directory]::CreateDirectory($runRoot) | Out-Null
if (-not $OutputPath) { $OutputPath = Join-Path $runRoot 'results.json' }
$oldSilent = $env:MDLITE_TEST_SILENT
$env:MDLITE_TEST_SILENT = '1'

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public sealed class SaveProbeWindow {
    public IntPtr Handle { get; set; }
    public int Id { get; set; }
    public string ClassName { get; set; }
    public string Text { get; set; }
    public uint ProcessId { get; set; }
    public uint ThreadId { get; set; }
    public IntPtr RootOwnerHandle { get; set; }
    public string RootOwnerClass { get; set; }
    public string RootOwnerCaption { get; set; }
    public bool IsWindow { get; set; }
}
public static class SaveProbeNative {
    [StructLayout(LayoutKind.Sequential)]
    private struct FILETIME {
        public uint Low;
        public uint High;
        public long Ticks { get { return ((long)High << 32) | Low; } }
    }
    public delegate bool EnumProc(IntPtr window, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumWindows(EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern bool EnumChildWindows(IntPtr parent, EnumProc callback, IntPtr parameter);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] public static extern bool IsWindow(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern IntPtr OpenThread(uint access, bool inherit, uint threadId);
    [DllImport("kernel32.dll", SetLastError=true)] private static extern bool GetThreadTimes(IntPtr thread, out FILETIME creation, out FILETIME exit, out FILETIME kernel, out FILETIME user);
    [DllImport("kernel32.dll")] private static extern bool CloseHandle(IntPtr handle);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetClassNameW(IntPtr window, StringBuilder value, int capacity);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowTextW(IntPtr window, StringBuilder value, int capacity);
    [DllImport("user32.dll")] private static extern int GetWindowTextLengthW(IntPtr window);
    [DllImport("user32.dll")] private static extern int GetDlgCtrlID(IntPtr window);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", SetLastError=true)]
    private static extern IntPtr SendMessageTimeout(IntPtr window, uint message, IntPtr wparam, IntPtr lparam,
        uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern IntPtr SendMessageTimeoutText(IntPtr window, uint message, IntPtr wparam,
        [MarshalAs(UnmanagedType.LPWStr)] string lparam, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll", EntryPoint="SendMessageTimeoutW", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern IntPtr SendMessageTimeoutBuffer(IntPtr window, uint message, IntPtr wparam,
        StringBuilder lparam, uint flags, uint timeout, out IntPtr result);
    [DllImport("user32.dll")] private static extern bool PostMessageW(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);

    private static string ClassOf(IntPtr window) {
        var value = new StringBuilder(128);
        GetClassNameW(window, value, value.Capacity);
        return value.ToString();
    }
    public static string CaptionOf(IntPtr window) {
        int length = Math.Max(0, GetWindowTextLengthW(window));
        var value = new StringBuilder(length + 1);
        GetWindowTextW(window, value, value.Capacity);
        return value.ToString();
    }
    public static string TextOf(IntPtr window) {
        IntPtr result;
        if (SendMessageTimeout(window, 0x000E, IntPtr.Zero, IntPtr.Zero, 2, 5000, out result) == IntPtr.Zero)
            throw new TimeoutException("WM_GETTEXTLENGTH failed: " + WindowContext(window) +
                ", configured_timeout_ms=5000, win32_error=" + Marshal.GetLastWin32Error());
        int length = Math.Max(0, result.ToInt32());
        var value = new StringBuilder(length + 1);
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        if (SendMessageTimeoutBuffer(window, 0x000D, new IntPtr(value.Capacity), value, 2, 5000, out result) == IntPtr.Zero)
            throw new TimeoutException("WM_GETTEXT failed: " + WindowContext(window) +
                ", configured_timeout_ms=5000, elapsed_ms=" + elapsed.Elapsed.TotalMilliseconds +
                ", win32_error=" + Marshal.GetLastWin32Error());
        return value.ToString();
    }
    private static SaveProbeWindow Describe(IntPtr window) {
        uint processId;
        uint threadId = GetWindowThreadProcessId(window, out processId);
        IntPtr rootOwner = GetAncestor(window, 3);
        return new SaveProbeWindow { Handle = window, Id = GetDlgCtrlID(window),
            ClassName = ClassOf(window), Text = CaptionOf(window), ProcessId = processId,
            ThreadId = threadId, RootOwnerHandle = rootOwner,
            RootOwnerClass = rootOwner == IntPtr.Zero ? "" : ClassOf(rootOwner),
            RootOwnerCaption = rootOwner == IntPtr.Zero ? "" : CaptionOf(rootOwner), IsWindow = IsWindow(window) };
    }
    private static string WindowContext(IntPtr window) {
        uint processId;
        uint threadId = GetWindowThreadProcessId(window, out processId);
        IntPtr rootOwner = GetAncestor(window, 3);
        string rootClass = rootOwner == IntPtr.Zero ? "" : ClassOf(rootOwner);
        string rootCaption = rootOwner == IntPtr.Zero ? "" : CaptionOf(rootOwner);
        return "hwnd=0x" + window.ToInt64().ToString("X") + ",class=" + ClassOf(window) +
            ",control_id=" + GetDlgCtrlID(window) + ",owner_pid=" + processId + ",thread_id=" + threadId +
            ",root_owner_hwnd=0x" + rootOwner.ToInt64().ToString("X") + ",root_owner_class=" + rootClass +
            ",root_owner_caption=" + rootCaption + ",is_window=" + IsWindow(window);
    }
    public static string GetWindowContext(IntPtr window) { return WindowContext(window); }
    public static uint GetWindowThreadId(IntPtr window) {
        uint processId;
        return GetWindowThreadProcessId(window, out processId);
    }
    public static long ThreadCpu100ns(uint threadId) {
        IntPtr thread = OpenThread(0x0040, false, threadId);
        if (thread == IntPtr.Zero) throw new InvalidOperationException("OpenThread failed (Win32 " + Marshal.GetLastWin32Error() + ").");
        try {
            FILETIME creation, exit, kernel, user;
            if (!GetThreadTimes(thread, out creation, out exit, out kernel, out user))
                throw new InvalidOperationException("GetThreadTimes failed (Win32 " + Marshal.GetLastWin32Error() + ").");
            return kernel.Ticks + user.Ticks;
        } finally { CloseHandle(thread); }
    }
    public static SaveProbeWindow[] TopLevel(uint processId) {
        var result = new List<SaveProbeWindow>();
        EnumWindows((window, unused) => {
            uint owner;
            GetWindowThreadProcessId(window, out owner);
            if (owner == processId) result.Add(Describe(window));
            return true;
        }, IntPtr.Zero);
        return result.ToArray();
    }
    public static SaveProbeWindow[] Children(IntPtr parent) {
        var result = new List<SaveProbeWindow>();
        EnumChildWindows(parent, (window, unused) => { result.Add(Describe(window)); return true; }, IntPtr.Zero);
        return result.ToArray();
    }
    public static long Send(IntPtr window, uint message, long wparam, long lparam) {
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        IntPtr result;
        if (SendMessageTimeout(window, message, new IntPtr(wparam), new IntPtr(lparam), 2, 5000, out result) == IntPtr.Zero)
            throw new TimeoutException("Native message 0x" + message.ToString("X") + " failed: " + WindowContext(window) +
                ", configured_timeout_ms=5000, elapsed_ms=" + elapsed.Elapsed.TotalMilliseconds +
                ", win32_error=" + Marshal.GetLastWin32Error());
        return result.ToInt64();
    }
    public static long SendText(IntPtr window, uint message, long wparam, string text) {
        var elapsed = System.Diagnostics.Stopwatch.StartNew();
        IntPtr result;
        if (SendMessageTimeoutText(window, message, new IntPtr(wparam), text, 2, 5000, out result) == IntPtr.Zero)
            throw new TimeoutException("Native text message 0x" + message.ToString("X") + " failed: " + WindowContext(window) +
                ", configured_timeout_ms=5000, elapsed_ms=" + elapsed.Elapsed.TotalMilliseconds +
                ", win32_error=" + Marshal.GetLastWin32Error());
        return result.ToInt64();
    }
    public static bool Post(IntPtr window, uint message, long wparam, long lparam) {
        return PostMessageW(window, message, new IntPtr(wparam), new IntPtr(lparam));
    }
}
'@

function Send([IntPtr]$Window, [uint32]$Message, [long]$WParam = 0, [long]$LParam = 0) {
    $script:lastNativeCallFailed = $false
    $script:lastNativeTarget = $null
    $script:lastNativeThreadId = [uint32]0
    $script:lastNativeThreadProbe = $null
    try { return [SaveProbeNative]::Send($Window, $Message, $WParam, $LParam) }
    catch {
        $script:lastNativeCallFailed = $true
        $script:lastNativeTarget = [SaveProbeNative]::GetWindowContext($Window)
        $script:lastNativeThreadId = [SaveProbeNative]::GetWindowThreadId($Window)
        throw
    }
}
function Send-Text([IntPtr]$Window, [uint32]$Message, [string]$Text, [long]$WParam = 0) {
    $script:lastNativeCallFailed = $false
    $script:lastNativeTarget = $null
    $script:lastNativeThreadId = [uint32]0
    $script:lastNativeThreadProbe = $null
    try {
        return [SaveProbeNative]::SendText($Window, $Message, $WParam, $Text)
    } catch {
        $script:lastNativeCallFailed = $true
        $script:lastNativeTarget = [SaveProbeNative]::GetWindowContext($Window)
        $script:lastNativeThreadId = [SaveProbeNative]::GetWindowThreadId($Window)
        throw
    }
}
function Find-Child([IntPtr]$Parent, [int]$Id, [string]$ClassName = '') {
    foreach ($child in [SaveProbeNative]::Children($Parent)) {
        if ($child.Id -eq $Id -and (-not $ClassName -or $child.ClassName -eq $ClassName)) { return $child }
    }
    return $null
}
function Wait-Child([IntPtr]$Parent, [int]$Id, [string]$ClassName, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $child = Find-Child $Parent $Id $ClassName
        if ($child) { return $child }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for child id=$Id class=$ClassName."
}
function Wait-VisibleChild([IntPtr]$Parent, [int]$Id, [string]$ClassName, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        foreach ($child in [SaveProbeNative]::Children($Parent)) {
            if ($child.Id -eq $Id -and $child.ClassName -eq $ClassName -and [SaveProbeNative]::IsWindowVisible($child.Handle)) {
                return $child
            }
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for visible child id=$Id class=$ClassName."
}
function Set-ComboIndex([IntPtr]$Parent, [int]$Id, [int]$Index) {
    $combo = Wait-Child $Parent $Id 'ComboBox'
    if ((Send $combo.Handle 0x014E $Index 0) -lt 0) { throw "ComboBox id=$Id rejected index=$Index." }
    return $combo
}
function Read-Utf8Fixture([string]$Path) {
    return [IO.File]::ReadAllText($Path, [Text.UTF8Encoding]::new($false, $true))
}
function Wait-Window([Diagnostics.Process]$Process, [string]$ClassName, [int]$TimeoutMs = 10000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $Process.Refresh()
        if ($Process.HasExited) { throw "MDLite PID $($Process.Id) exited with code $($Process.ExitCode)." }
        $found = @([SaveProbeNative]::TopLevel([uint32]$Process.Id) | Where-Object { $_.ClassName -eq $ClassName })
        if ($found.Count) { return $found[0] }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for window class $ClassName from PID $($Process.Id)."
}
function Wait-WindowGone([Diagnostics.Process]$Process, [string]$ClassName, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $found = @([SaveProbeNative]::TopLevel([uint32]$Process.Id) | Where-Object { $_.ClassName -eq $ClassName })
        if (-not $found.Count) { return }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Window class $ClassName did not close."
}
function Get-Modal([Diagnostics.Process]$Process) {
    $found = @([SaveProbeNative]::TopLevel([uint32]$Process.Id) | Where-Object { $_.ClassName -eq '#32770' })
    if ($found.Count) { return $found[0] }
    return $null
}
function Wait-Modal([Diagnostics.Process]$Process, [int]$TimeoutMs = 5000) {
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    do {
        $dialog = Get-Modal $Process
        if ($dialog) { return $dialog }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Timed out waiting for a native dialog from PID $($Process.Id)."
}
function Dismiss-Modal([Diagnostics.Process]$Process, [SaveProbeWindow]$Dialog, [int]$Command = 1) {
    [void](Send $Dialog.Handle 0x0111 $Command 0)
    Wait-WindowGone $Process $Dialog.ClassName
}
function Bytes-Evidence([string]$Name, [byte[]]$Bytes) {
    $hasher = [Security.Cryptography.SHA256]::Create()
    try { $sha256 = [BitConverter]::ToString($hasher.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $hasher.Dispose() }
    return [pscustomobject]@{ name = $Name; length = $Bytes.Length; sha256 = $sha256; hex = [BitConverter]::ToString($Bytes).Replace('-', '').ToLowerInvariant() }
}
function File-Evidence([string]$Path) {
    return Bytes-Evidence ([IO.Path]::GetFileName($Path)) ([IO.File]::ReadAllBytes($Path))
}
function Stream-Evidence([IO.FileStream]$Stream, [string]$Name) {
    [void]$Stream.Seek(0, [IO.SeekOrigin]::Begin)
    $bytes = [byte[]]::new([int]$Stream.Length)
    $offset = 0
    while ($offset -lt $bytes.Length) {
        $read = $Stream.Read($bytes, $offset, $bytes.Length - $offset)
        if ($read -le 0) { throw 'Locked fixture stream ended before the expected length.' }
        $offset += $read
    }
    return Bytes-Evidence $Name $bytes
}
function Probe-FileOpen([string]$Path) {
    $stream = $null
    try {
        $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        return [ordered]@{ opened = $true; length = $stream.Length; hresult = 0; outer_hresult = 0; exception_type = $null; win32_error = 0; error = $null }
    } catch {
        $outerHresult = $_.Exception.HResult
        $exception = $_.Exception.GetBaseException()
        $hresult = $exception.HResult
        return [ordered]@{ opened = $false; length = $null; hresult = $hresult; outer_hresult = $outerHresult; exception_type = $exception.GetType().FullName; win32_error = ($hresult -band 0xFFFF); error = $exception.Message }
    } finally {
        if ($stream) { $stream.Dispose() }
    }
}
function New-Workspace([string]$Name, [string]$FileName = 'README.md', [string]$Text = 'fixture') {
    $path = Join-Path $runRoot $Name
    [IO.Directory]::CreateDirectory((Join-Path $path '.mdlite')) | Out-Null
    $lf = [string][char]10
    $settings = 'schema_version = 1' + $lf + 'auto_save = false' + $lf + 'auto_save_delay_ms = 750' + $lf + 'theme = "dark"' + $lf + 'font_size_pt = 11' + $lf
    [IO.File]::WriteAllText((Join-Path $path '.mdlite\settings.toml'), $settings, [Text.UTF8Encoding]::new($false))
    $source = Join-Path $path $FileName
    [IO.File]::WriteAllText($source, $Text, [Text.UTF8Encoding]::new($false))
    return [pscustomobject]@{ root = $path; source = $source; settings = Join-Path $path '.mdlite\settings.toml' }
}
function Start-App([string]$Source) {
    $process = Start-Process -FilePath $executable -ArgumentList @(('"{0}"' -f $Source)) -WorkingDirectory $repoRoot -PassThru
    $app = [pscustomobject]@{ process = $process; main = [IntPtr]::Zero; editor = [IntPtr]::Zero; process_id = $process.Id; source = $Source }
    try {
        $main = Wait-Window $process 'MDLite.MainWindow'
        $app.main = $main.Handle
        $editor = Wait-VisibleChild $main.Handle 102 'RICHEDIT50W'
        $app.editor = $editor.Handle
        return $app
    } catch {
        $failure = $_.Exception.Message
        $diagnostics = Get-AppDiagnostics $app
        $cleanupError = $null
        try {
            Stop-App $app
            $process.Refresh()
            if (-not $process.HasExited) {
                Stop-Process -Id $process.Id -Force -ErrorAction Stop
                $cleanupError = 'Stop-App returned while the owned process was still running; forced it to exit.'
            }
        } catch {
            $cleanupError = $_.Exception.Message
            try { Stop-Process -Id $process.Id -Force -ErrorAction Stop } catch { $cleanupError += ' Force-stop also failed: ' + $_.Exception.Message }
        }
        $evidence = $diagnostics | ConvertTo-Json -Compress -Depth 7
        throw "MDLite startup readiness failed for source '$Source' PID $($process.Id): $failure; cleanup_error='$cleanupError'; diagnostics=$evidence"
    }
}
function App-State([IntPtr]$Main) {
    $editor = Wait-VisibleChild $Main 102 'RICHEDIT50W'
    return [pscustomobject]@{
        dirty = (Send $Main 0x8045 0 0) -ne 0
        undo = Send $Main 0x8045 1 0
        redo = Send $Main 0x8045 2 0
        revision = Send $Main 0x8045 3 0
        saved_revision = Send $Main 0x8045 6 0
        source_length = Send $Main 0x8045 4 0
        anchor = Send $Main 0x8033 0 0
        active = Send $Main 0x8034 0 0
        editor_text = [SaveProbeNative]::TextOf($editor.Handle)
    }
}
function Append-Editor([IntPtr]$Main, [string]$Text) {
    $editor = Wait-VisibleChild $Main 102 'RICHEDIT50W'
    $before = App-State $Main
    $expectedText = $before.editor_text + $Text
    [void](Send $editor.Handle 0x00B1 -1 -1)
    [void](Send-Text $editor.Handle 0x00C2 $Text 1)
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    $state = $before
    do {
        $state = App-State $Main
        if ($state.dirty -and $state.editor_text -ceq $expectedText -and
            $state.revision -gt $before.revision -and $state.undo -gt $before.undo -and
            $state.source_length -gt $before.source_length) {
            $state | Add-Member -NotePropertyName before_state -NotePropertyValue $before -Force
            return $state
        }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    throw ('Editor append did not synchronize. Before={0}; expected_text_length={1}; last={2}' -f
        ($before | ConvertTo-Json -Compress -Depth 4), $expectedText.Length,
        ($state | ConvertTo-Json -Compress -Depth 4))
}
function Save-As([pscustomobject]$App, [string]$Name) {
    [void][SaveProbeNative]::Post($App.main, 0x0111, 1005, 0)
    $dialog = Wait-Window $App.process '#32770' 5000
    $edit = Wait-Child $dialog.Handle 1001 'Edit' 5000
    if (-not $dialog.IsWindow -or -not $edit.IsWindow) { throw 'Save As dialog or filename Edit control is no longer a window.' }
    [void](Send-Text $edit.Handle 0x000C $Name)
    $readBack = [SaveProbeNative]::TextOf($edit.Handle)
    if ($readBack -cne $Name) { throw "Save As filename readback mismatch: expected '$Name', observed '$readBack'." }
    $button = Wait-Child $dialog.Handle 1 'Button' 5000
    if (-not $button.IsWindow) { throw 'Save As confirmation button is no longer a window.' }
    [void](Send $button.Handle 0x00F5 0 0)
    Wait-WindowGone $App.process '#32770' 5000
    return [pscustomobject]@{
        dialog = [ordered]@{ hwnd = $dialog.Handle.ToInt64(); class = $dialog.ClassName; caption = $dialog.Text; owner_pid = $dialog.ProcessId; owner_thread_id = $dialog.ThreadId }
        filename_edit = [ordered]@{ hwnd = $edit.Handle.ToInt64(); class = $edit.ClassName; control_id = $edit.Id; owner_pid = $edit.ProcessId; owner_thread_id = $edit.ThreadId; value = $readBack }
        save_button = [ordered]@{ hwnd = $button.Handle.ToInt64(); class = $button.ClassName; control_id = $button.Id; owner_pid = $button.ProcessId }
    }
}
function Wait-Recovery([string]$Workspace, [string]$Text) {
    $directory = Join-Path $Workspace '.mdlite\.state\recovery'
    $deadline = [DateTime]::UtcNow.AddSeconds(8)
    do {
        foreach ($file in @(Get-ChildItem -LiteralPath $directory -Filter '*.md' -File -ErrorAction SilentlyContinue)) {
            if ([IO.File]::ReadAllText($file.FullName).Contains($Text)) {
                return [pscustomobject]@{ file = $file.Name; sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $file.FullName).Hash.ToLowerInvariant(); contains_pending_source = $true }
            }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Recovery snapshot did not retain pending source.'
}
function Stop-App([pscustomobject]$App) {
    if (-not $App -or -not $App.process) { return }
    $process = $App.process
    try { $process.Refresh(); if ($process.HasExited) { return } } catch { return }
    try { [void][SaveProbeNative]::Post($App.main, 0x0010, 0, 0) } catch { }
    $deadline = [DateTime]::UtcNow.AddSeconds(2)
    do {
        try { $process.Refresh(); if ($process.HasExited) { return } } catch { break }
        try { $dialog = Get-Modal $process } catch { $dialog = $null }
        if ($dialog) { try { [void](Send $dialog.Handle 0x0111 7 0) } catch { }; break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $deadline)
    try {
        if (-not $process.WaitForExit(5000)) {
            Stop-Process -Id $process.Id -Force -ErrorAction Stop
            [void]$process.WaitForExit(3000)
        }
    } catch {
        try { Stop-Process -Id $process.Id -Force -ErrorAction SilentlyContinue } catch { }
    }
}
function Get-AppDiagnostics([pscustomobject]$App) {
    if (-not $App -or -not $App.process) { return [ordered]@{ available = $false } }
    $process = $App.process
    $process.Refresh()
    $hasExited = $process.HasExited
    $responding = $null
    $exitCode = $null
    if ($hasExited) { $exitCode = $process.ExitCode }
    else { try { $responding = $process.Responding } catch { $responding = 'ERROR: ' + $_.Exception.Message } }
    $windows = @()
    $windowError = $null
    $children = @()
    $childError = $null
    if (-not $hasExited) {
        try {
            $windows = @([SaveProbeNative]::TopLevel([uint32]$process.Id) | ForEach-Object {
                [ordered]@{
                    hwnd = $_.Handle.ToInt64()
                    class = $_.ClassName
                    caption = $_.Text
                    control_id = $_.Id
                    owner_pid = $_.ProcessId
                    owner_thread_id = $_.ThreadId
                    root_owner_hwnd = $_.RootOwnerHandle.ToInt64()
                    root_owner_class = $_.RootOwnerClass
                    root_owner_caption = $_.RootOwnerCaption
                    is_window = $_.IsWindow
                    visible = [SaveProbeNative]::IsWindowVisible($_.Handle)
                }
            })
        } catch { $windowError = $_.Exception.Message }
        if ($App.main -ne [IntPtr]::Zero) {
            try {
                $children = @([SaveProbeNative]::Children($App.main) | ForEach-Object {
                    [ordered]@{
                        hwnd = $_.Handle.ToInt64()
                        class = $_.ClassName
                        control_id = $_.Id
                        owner_pid = $_.ProcessId
                        owner_thread_id = $_.ThreadId
                        root_owner_hwnd = $_.RootOwnerHandle.ToInt64()
                        root_owner_class = $_.RootOwnerClass
                        is_window = $_.IsWindow
                        visible = [SaveProbeNative]::IsWindowVisible($_.Handle)
                    }
                })
            } catch { $childError = $_.Exception.Message }
        }
    }
    return [ordered]@{
        pid = $process.Id
        source = $App.source
        has_exited = $hasExited
        exit_code = $exitCode
        responding = $responding
        main_hwnd = $App.main.ToInt64()
        top_level_windows = $windows
        window_enumeration_error = $windowError
        main_children = $children
        child_enumeration_error = $childError
    }
}
function Get-ThreadCpuProbe([uint32]$ThreadId) {
    $before = $null
    $beforeError = $null
    try { $before = [SaveProbeNative]::ThreadCpu100ns($ThreadId) }
    catch { $beforeError = $_.Exception.Message }
    $started = [DateTime]::UtcNow
    Start-Sleep -Milliseconds 2000
    $elapsed = ([DateTime]::UtcNow - $started).TotalMilliseconds
    $after = $null
    $afterError = $null
    try { $after = [SaveProbeNative]::ThreadCpu100ns($ThreadId) }
    catch { $afterError = $_.Exception.Message }
    $cpuDeltaMs = $null
    if ($null -ne $before -and $null -ne $after) { $cpuDeltaMs = [Math]::Round(($after - $before) / 10000.0, 3) }
    return [ordered]@{
        thread_id = $ThreadId
        requested_probe_ms = 2000
        elapsed_ms = [Math]::Round($elapsed, 3)
        cpu_100ns_before = $before
        cpu_100ns_after = $after
        cpu_delta_ms = $cpuDeltaMs
        before_error = $beforeError
        after_error = $afterError
    }
}
function Record-BlockedCase([string]$Name, [string]$Expected, [string]$Fixture,
    [System.Management.Automation.ErrorRecord]$Failure, [pscustomobject]$App, [object]$Checkpoint = $null) {
    if (@($cases.ToArray() | Where-Object { $_.case -eq $Name }).Count -gt 0) { return }
    if ($script:lastNativeCallFailed -and $script:lastNativeThreadId -gt 0 -and $null -eq $script:lastNativeThreadProbe) {
        $script:lastNativeThreadProbe = Get-ThreadCpuProbe $script:lastNativeThreadId
    }
    $exception = if ($Failure) { $Failure.Exception.Message } else { 'Unspecified native UI helper failure.' }
    $line = if ($Failure) { $Failure.InvocationInfo.ScriptLineNumber } else { $null }
    $cases.Add([pscustomobject]@{
        case = $Name
        status = 'BLOCKED'
        expected = $Expected
        observed = [ordered]@{
            error = $exception
            script_line = $line
            last_native_target = $script:lastNativeTarget
            last_native_thread_id = $script:lastNativeThreadId
            last_native_call_failed = $script:lastNativeCallFailed
            current_thread_cpu_2s = $script:lastNativeThreadProbe
            checkpoint = $Checkpoint
            app = Get-AppDiagnostics $App
        }
        fixture = $Fixture
        input = 'Cross-process native messages; no physical keyboard or IME.'
    })
}

$cases = [Collections.Generic.List[object]]::new()
$apps = [Collections.Generic.List[object]]::new()
$script:allCaseNames = @(
    'empty_untitled_explicit_save',
    'daily_meeting_memo_date_repeat_no_overwrite',
    'settings_cancel_apply_persist_restart',
    'profile_form_apply_persists_workspace_definition',
    'settings_external_change_conflict',
    'native_share_lock_save_failure_and_retry',
    'native_readonly_attribute_save_failure_and_retry',
    'external_file_change_blocks_save',
    'cp932_conversion_failure'
)
$script:profileCaseNames = @(
    'daily_meeting_memo_date_repeat_no_overwrite',
    'settings_cancel_apply_persist_restart',
    'profile_form_apply_persists_workspace_definition',
    'settings_external_change_conflict'
)
$script:onlyCases = @($OnlyCase | ForEach-Object { $_ -split ',' } | ForEach-Object { $_.Trim() } | Where-Object { $_ })
$unknownCases = @($script:onlyCases | Where-Object { $script:allCaseNames -notcontains $_ })
if ($unknownCases.Count -gt 0) { throw ('Unknown case name(s): ' + ($unknownCases -join ', ')) }
$selectedCases = if ($script:onlyCases.Count -gt 0) { $script:onlyCases } else { $script:allCaseNames }
$notSelectedCases = @($script:allCaseNames | Where-Object { $selectedCases -notcontains $_ })
$allNotRunCases = @($notSelectedCases + 'Separate ACL access-denied injection' + 'Disk-full or physical media-write failure')
function Test-SelectedCase([string]$Name) {
    return ($script:onlyCases.Count -eq 0 -or $script:onlyCases -contains $Name)
}
$runProfileGroup = ($script:onlyCases.Count -eq 0 -or
    @($script:onlyCases | Where-Object { $script:profileCaseNames -contains $_ }).Count -gt 0)
$script:lastNativeTarget = $null
$script:lastNativeThreadId = [uint32]0
$script:lastNativeCallFailed = $false
$script:lastNativeThreadProbe = $null
try {
    if (Test-SelectedCase 'empty_untitled_explicit_save') {
    # Save an empty untitled document through the native Save As dialog.
    $app = $null
    $emptyFixture = $null
    $emptyBefore = $null
    $emptyPath = $null
    $tabCount = $null
    try {
        $emptyFixture = New-Workspace 'empty-untitled'
        $app = Start-App $emptyFixture.source
        $apps.Add($app)
        [void](Send $app.main 0x0111 1001 0)
        $tabs = Wait-Child $app.main 101 'SysTabControl32'
        $deadline = [DateTime]::UtcNow.AddSeconds(3)
        do { $tabCount = Send $tabs.Handle 0x1304 0 0; if ($tabCount -ge 2) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
        $emptyBefore = App-State $app.main
        $emptyPath = Join-Path $emptyFixture.root 'empty-save.md'
        $saveAsEvidence = Save-As $app 'empty-save.md'
        $settleDeadline = [DateTime]::UtcNow.AddSeconds(8)
        $settledCount = 0
        $lastFileHash = $null
        $emptyBytes = $null
        $emptyAfter = $null
        do {
            if (Test-Path -LiteralPath $emptyPath -PathType Leaf) {
                try {
                    $candidateBytes = File-Evidence $emptyPath
                    $candidateState = App-State $app.main
                    if ($candidateBytes.length -eq 0 -and -not $candidateState.dirty -and
                        $candidateState.saved_revision -eq $candidateState.revision) {
                        if ($lastFileHash -eq $candidateBytes.sha256) { $settledCount++ } else { $settledCount = 1 }
                        $lastFileHash = $candidateBytes.sha256
                        $emptyBytes = $candidateBytes
                        $emptyAfter = $candidateState
                        if ($settledCount -ge 2) { break }
                    } else {
                        $settledCount = 0
                        $lastFileHash = $candidateBytes.sha256
                    }
                } catch { $settledCount = 0 }
            }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $settleDeadline)
        if ($settledCount -lt 2) { throw 'Empty Save As file/source state did not settle within 8 seconds.' }
        $emptyPass = ($emptyBefore.source_length -eq 0 -and
            $emptyBytes.length -eq 0 -and -not $emptyAfter.dirty -and
            $emptyAfter.saved_revision -eq $emptyAfter.revision -and
            $emptyAfter.undo -eq $emptyBefore.undo -and $emptyAfter.redo -eq $emptyBefore.redo)
        $cases.Add([pscustomobject]@{ case='empty_untitled_explicit_save'; status=$(if($emptyPass){'PASS'}else{'FAIL'}); expected='Save As creates a zero-byte file and marks the empty source clean'; observed=[ordered]@{ state_before=$emptyBefore; bytes=$emptyBytes; state_after=$emptyAfter; tab_count=$tabCount; save_as=$saveAsEvidence }; fixture='empty-untitled' })
    } catch {
        $emptyCheckpoint = [ordered]@{
            state_before_save_as = $emptyBefore
            target = $emptyPath
            target_after_failure = if ($emptyPath -and (Test-Path -LiteralPath $emptyPath -PathType Leaf)) { File-Evidence $emptyPath } else { $null }
            tab_count = $tabCount
        }
        Record-BlockedCase 'empty_untitled_explicit_save' 'Save As creates a zero-byte file and marks the empty source clean' 'empty-untitled' $_ $app $emptyCheckpoint
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
    }
    }

    # Daily, Meeting and Memo use a workspace-only profile layer and sentinel files.
    if ($runProfileGroup) {
    $app = $null
    try {
    $profileFixture = New-Workspace 'profiles-builtins'
    $lf = [string][char]10
    $metadata = Join-Path $profileFixture.root '.mdlite'
    [IO.Directory]::CreateDirectory((Join-Path $metadata 'templates')) | Out-Null
    $profileToml = @'
schema_version = 1
[[profiles]]
id = "daily"
name = "Daily"
directory = "Dairy/{{date:yyyy}}/{{date:yyyyMM}}"
filename = "{{date:yyyyMMdd}}.md"
template = "templates/daily.md"
collision = "open-existing"
[[profiles]]
id = "meeting"
name = "Meeting"
directory = "Meeting/{{date:yyyy}}/{{date:yyyyMM}}"
filename = "{{date:yyyyMMdd}}.md"
template = "templates/meeting.md"
collision = "sequence"
sequence_format = "_%02d"
[[profiles]]
id = "memo"
name = "Memo"
directory = "Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}"
filename = "{{date:yyyyMMdd}}.md"
template = "templates/memo.md"
collision = "sequence"
sequence_format = "_%02d"
'@
    [IO.File]::WriteAllText((Join-Path $metadata 'profiles.toml'), $profileToml + $lf, [Text.UTF8Encoding]::new($false))
    foreach ($name in @('daily', 'meeting', 'memo')) {
        $body = '# ' + $name + ' {{date:yyyy-MM-dd}}' + $lf + '{{cursor}}' + $name + ' template' + $lf
        [IO.File]::WriteAllText((Join-Path $metadata "templates\$name.md"), $body, [Text.UTF8Encoding]::new($false))
    }
    $now = Get-Date
    $yyyy = $now.ToString('yyyy')
    $yyyyMM = $now.ToString('yyyyMM')
    $yyyyMMdd = $now.ToString('yyyyMMdd')
    $dailyPath = Join-Path $profileFixture.root "Dairy\$yyyy\$yyyyMM\$yyyyMMdd.md"
    $meetingDir = Join-Path $profileFixture.root "Meeting\$yyyy\$yyyyMM"
    $memoDir = Join-Path $profileFixture.root "Memo\$yyyy\$yyyyMM\$yyyyMMdd"
    if (Test-SelectedCase 'daily_meeting_memo_date_repeat_no_overwrite') {
        [IO.Directory]::CreateDirectory((Split-Path -Parent $dailyPath)) | Out-Null
        [IO.File]::WriteAllText($dailyPath, 'DAILY_SENTINEL_PRESERVE' + $lf, [Text.UTF8Encoding]::new($false))
        $dailyBefore = File-Evidence $dailyPath
    }
    $app = $null
    if ((Test-SelectedCase 'daily_meeting_memo_date_repeat_no_overwrite') -or
        (Test-SelectedCase 'settings_cancel_apply_persist_restart')) {
        $app = Start-App $profileFixture.source
        $apps.Add($app)
    }
    if (Test-SelectedCase 'daily_meeting_memo_date_repeat_no_overwrite') {
    [void](Send $app.main 0x0111 1001 0)
    $untitled = Append-Editor $app.main 'untitled-profile-draft'
    $tabCountBefore = Send (Wait-Child $app.main 101 'SysTabControl32').Handle 0x1304 0 0
    [void](Send $app.main 0x0111 1009 0)
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do { $dailyText = [SaveProbeNative]::TextOf((Wait-Child $app.main 102 'RICHEDIT50W').Handle); if ($dailyText.Contains('DAILY_SENTINEL_PRESERVE')) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    $dailyFirst = File-Evidence $dailyPath
    $dailyTabs = Send (Wait-Child $app.main 101 'SysTabControl32').Handle 0x1304 0 0
    [void](Send $app.main 0x0111 1009 0)
    Start-Sleep -Milliseconds 100
    $dailySecond = File-Evidence $dailyPath
    $dailyTabsRepeat = Send (Wait-Child $app.main 101 'SysTabControl32').Handle 0x1304 0 0
    [void](Send $app.main 0x0111 1010 0)
    $meetingOne = Join-Path $meetingDir "$yyyyMMdd.md"
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do { if (Test-Path -LiteralPath $meetingOne -PathType Leaf) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    $meetingFirst = File-Evidence $meetingOne
    $meetingSource = Read-Utf8Fixture $meetingOne
    $meetingState = App-State $app.main
    [void](Send $app.main 0x0111 1010 0)
    $meetingTwo = Join-Path $meetingDir ($yyyyMMdd + '_01.md')
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do { if (Test-Path -LiteralPath $meetingTwo -PathType Leaf) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    $meetingFirstAfter = File-Evidence $meetingOne
    $meetingSecond = if (Test-Path -LiteralPath $meetingTwo) { File-Evidence $meetingTwo } else { $null }
    [void](Send $app.main 0x0111 1011 0)
    $memoOne = Join-Path $memoDir "$yyyyMMdd.md"
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do { if (Test-Path -LiteralPath $memoOne -PathType Leaf) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    $memoFirst = File-Evidence $memoOne
    [void](Send $app.main 0x0111 1011 0)
    $memoTwo = Join-Path $memoDir ($yyyyMMdd + '_01.md')
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    do { if (Test-Path -LiteralPath $memoTwo -PathType Leaf) { break }; Start-Sleep -Milliseconds 50 } while ([DateTime]::UtcNow -lt $deadline)
    $memoFirstAfter = File-Evidence $memoOne
    $memoSecond = if (Test-Path -LiteralPath $memoTwo) { File-Evidence $memoTwo } else { $null }
    $tabCountAfter = Send (Wait-Child $app.main 101 'SysTabControl32').Handle 0x1304 0 0
    $tabs = Wait-Child $app.main 101 'SysTabControl32'
    $packed = Send $app.main 0x803A 1 0
    $x = $packed -band 0xFFFF
    $y = ($packed -shr 16) -band 0xFFFF
    $point = [IntPtr]([int]($x -bor ($y -shl 16)))
    [void](Send $tabs.Handle 0x0201 1 $point.ToInt64())
    [void](Send $tabs.Handle 0x0202 0 $point.ToInt64())
    $untitledAfter = App-State $app.main
    $profilePass = ($dailyBefore.sha256 -eq $dailyFirst.sha256 -and $dailyFirst.sha256 -eq $dailySecond.sha256 -and
        $dailyTabs -eq $dailyTabsRepeat -and $meetingFirst.sha256 -eq $meetingFirstAfter.sha256 -and
        $memoFirst.sha256 -eq $memoFirstAfter.sha256 -and $meetingSecond -and $memoSecond -and
        $meetingSource -match $now.ToString('yyyy-MM-dd') -and $meetingSource -notmatch '\{\{cursor\}\}' -and
        -not $meetingState.dirty -and $meetingState.saved_revision -eq $meetingState.revision -and
        $untitledAfter.editor_text -ceq $untitled.editor_text -and $untitledAfter.dirty -and
        $untitledAfter.source_length -eq $untitled.source_length -and
        $untitledAfter.revision -eq $untitled.revision -and $untitledAfter.undo -eq $untitled.undo -and
        $untitledAfter.redo -eq $untitled.redo -and $untitledAfter.saved_revision -eq $untitled.saved_revision -and
        $untitledAfter.anchor -eq $untitled.anchor -and $untitledAfter.active -eq $untitled.active)
    $cases.Add([pscustomobject]@{ case='daily_meeting_memo_date_repeat_no_overwrite'; status=$(if($profilePass){'PASS'}else{'FAIL'}); expected='Daily opens the existing date path without overwrite; Meeting/Memo repeat to _01; new profile documents preserve the pending untitled draft'; observed=[ordered]@{ local_date=$now.ToString('yyyy-MM-dd'); daily_before=$dailyBefore; daily_after_first=$dailyFirst; daily_after_repeat=$dailySecond; daily_tabs_first=$dailyTabs; daily_tabs_repeat=$dailyTabsRepeat; meeting_first=$meetingFirst; meeting_first_after_repeat=$meetingFirstAfter; meeting_second=$meetingSecond; meeting_source=$meetingSource; meeting_state=$meetingState; memo_first=$memoFirst; memo_first_after_repeat=$memoFirstAfter; memo_second=$memoSecond; tab_count_before=$tabCountBefore; tab_count_after=$tabCountAfter; untitled_before=$untitled; untitled_after=$untitledAfter }; fixture='profiles-builtins' })
    }

    # Cancel is checked after editing controls; Apply is checked across restart.
    if (Test-SelectedCase 'settings_cancel_apply_persist_restart') {
    $settingsBefore = File-Evidence $profileFixture.settings
    [void][SaveProbeNative]::Post($app.main, 0x0111, 1027, 0)
    $form = Wait-Window $app.process 'MDLite.NativeFormWindow'
    [void](Set-ComboIndex $form.Handle 1003 1)
    [void](Send-Text (Wait-Child $form.Handle 1005 'Edit').Handle 0x000C '1800')
    [void](Set-ComboIndex $form.Handle 1007 2)
    Dismiss-Modal $app.process $form 2
    $settingsAfterCancel = File-Evidence $profileFixture.settings
    $cancelPass = ($settingsBefore.sha256 -eq $settingsAfterCancel.sha256)
    [void][SaveProbeNative]::Post($app.main, 0x0111, 1027, 0)
    $form = Wait-Window $app.process 'MDLite.NativeFormWindow'
    [void](Set-ComboIndex $form.Handle 1003 2)
    [void](Send-Text (Wait-Child $form.Handle 1005 'Edit').Handle 0x000C '1250')
    [void](Set-ComboIndex $form.Handle 1007 2)
    Dismiss-Modal $app.process $form 1
    Wait-WindowGone $app.process 'MDLite.NativeFormWindow'
    $settingsText = ''
    $settingsAfterApply = $null
    $lastSettingsHash = $null
    $stableApplyReads = 0
    $applyDeadline = [DateTime]::UtcNow.AddSeconds(3)
    do {
        $settingsText = Read-Utf8Fixture $profileFixture.settings
        $settingsAfterApply = File-Evidence $profileFixture.settings
        $applyValuesVisible = ($settingsText -match '(?m)^auto_save = false$' -and
            $settingsText -match '(?m)^auto_save_delay_ms = 1250$' -and $settingsText -match '(?m)^theme = "light"$')
        if ($applyValuesVisible) {
            if ($settingsAfterApply.sha256 -eq $lastSettingsHash) { $stableApplyReads++ } else { $stableApplyReads = 1 }
        } else { $stableApplyReads = 0 }
        $lastSettingsHash = $settingsAfterApply.sha256
        if ($stableApplyReads -ge 2) { break }
        Start-Sleep -Milliseconds 50
    } while ([DateTime]::UtcNow -lt $applyDeadline)
    $applyPass = ($stableApplyReads -ge 2)
    Stop-App $app
    $apps.Remove($app)
    $app = Start-App $profileFixture.source
    $apps.Add($app)
    [void][SaveProbeNative]::Post($app.main, 0x0111, 1027, 0)
    $form = Wait-Window $app.process 'MDLite.NativeFormWindow'
    $restoredSettings = [ordered]@{
        auto_save = [SaveProbeNative]::TextOf((Wait-Child $form.Handle 1003 'ComboBox').Handle)
        delay = [SaveProbeNative]::TextOf((Wait-Child $form.Handle 1005 'Edit').Handle)
        theme = [SaveProbeNative]::TextOf((Wait-Child $form.Handle 1007 'ComboBox').Handle)
    }
    $settingsAfterRestart = File-Evidence $profileFixture.settings
    Dismiss-Modal $app.process $form 2
    $restorePass = ($restoredSettings.auto_save -eq 'off' -and $restoredSettings.delay -eq '1250' -and $restoredSettings.theme -eq 'light')
    $cases.Add([pscustomobject]@{ case='settings_cancel_apply_persist_restart'; status=$(if($cancelPass -and $applyPass -and $restorePass){'PASS'}else{'FAIL'}); expected='Cancel preserves bytes; Apply persists edited settings and restart restores them'; observed=[ordered]@{ before=$settingsBefore; after_cancel=$settingsAfterCancel; after_apply=$settingsAfterApply; settings_text=$settingsText; stable_apply_reads=$stableApplyReads; after_restart=$settingsAfterRestart; restored_form=$restoredSettings; cancel_preserves=$cancelPass; apply_persists=$applyPass; restart_restores=$restorePass }; fixture='profiles-builtins' })
    }

    # Profile Apply's MB_YESNO confirmation is intentionally unsilenced for this isolated fixture only.
    if (Test-SelectedCase 'profile_form_apply_persists_workspace_definition') {
    $profileForm = $null
    $profileFormValues = $null
    $profileFormValuesMatch = $false
    $confirm = $null
    $confirmText = ''
    $success = $null
    $successText = ''
    $profileCleanup = 'not-needed'
    $profileApplyFixture = $null
    $profileApplyApp = $null
    $profilesPath = $null
    try {
        $profileApplyFixture = New-Workspace 'profile-apply'
        $profileApplyMetadata = Join-Path $profileApplyFixture.root '.mdlite'
        [IO.Directory]::CreateDirectory((Join-Path $profileApplyMetadata 'templates')) | Out-Null
        $profilesPath = Join-Path $profileApplyMetadata 'profiles.toml'
        [IO.File]::WriteAllText($profilesPath, $profileToml + $lf, [Text.UTF8Encoding]::new($false))
        foreach ($name in @('daily', 'meeting', 'memo')) {
            $body = '# ' + $name + ' {{date:yyyy-MM-dd}}' + $lf + '{{cursor}}' + $name + ' template' + $lf
            [IO.File]::WriteAllText((Join-Path $profileApplyMetadata "templates\$name.md"), $body, [Text.UTF8Encoding]::new($false))
        }
        $profileSilentBefore = $env:MDLITE_TEST_SILENT
        try {
            $env:MDLITE_TEST_SILENT = '0'
            $profileApplyApp = Start-App $profileApplyFixture.source
            $apps.Add($profileApplyApp)
        } finally {
            $env:MDLITE_TEST_SILENT = $profileSilentBefore
        }
        $startupDialog = Get-Modal $profileApplyApp.process
        if ($startupDialog -and $startupDialog.Text -eq 'Workspace Trust') {
            throw 'Workspace Trust dialog appeared during the isolated profile-apply fixture; it was not accepted.'
        }
        [void][SaveProbeNative]::Post($profileApplyApp.main, 0x0111, 1029, 0)
        $profileForm = Wait-Window $profileApplyApp.process 'MDLite.NativeFormWindow'
        [void](Set-ComboIndex $profileForm.Handle 1001 1)
        [void](Set-ComboIndex $profileForm.Handle 1003 0)
        [void](Send-Text (Wait-Child $profileForm.Handle 1005 'Edit').Handle 0x000C 'daily')
        [void](Send-Text (Wait-Child $profileForm.Handle 1007 'Edit').Handle 0x000C 'qaProfile')
        [void](Send-Text (Wait-Child $profileForm.Handle 1009 'Edit').Handle 0x000C 'QA Profile')
        [void](Send-Text (Wait-Child $profileForm.Handle 1011 'Edit').Handle 0x000C 'QAProfiles/{{date:yyyy}}')
        [void](Send-Text (Wait-Child $profileForm.Handle 1013 'Edit').Handle 0x000C '{{date:yyyyMMdd}}.md')
        [void](Send-Text (Wait-Child $profileForm.Handle 1015 'Edit').Handle 0x000C 'templates/daily.md')
        [void](Set-ComboIndex $profileForm.Handle 1017 1)
        $profileFormValues = [ordered]@{
            scope = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1001 'ComboBox').Handle)
            action = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1003 'ComboBox').Handle)
            source_id = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1005 'Edit').Handle)
            id = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1007 'Edit').Handle)
            name = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1009 'Edit').Handle)
            directory = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1011 'Edit').Handle)
            filename = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1013 'Edit').Handle)
            template = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1015 'Edit').Handle)
            collision = [SaveProbeNative]::TextOf((Wait-Child $profileForm.Handle 1017 'ComboBox').Handle)
        }
        $profileFormValuesMatch = (
            $profileFormValues.scope -ceq 'workspace' -and $profileFormValues.action -ceq 'add' -and
            $profileFormValues.source_id -ceq 'daily' -and $profileFormValues.id -ceq 'qaProfile' -and
            $profileFormValues.name -ceq 'QA Profile' -and $profileFormValues.directory -ceq 'QAProfiles/{{date:yyyy}}' -and
            $profileFormValues.filename -ceq '{{date:yyyyMMdd}}.md' -and $profileFormValues.template -ceq 'templates/daily.md' -and
            $profileFormValues.collision -ceq 'sequence')
        if (-not $profileFormValuesMatch) {
            throw ('Profile form values did not match the isolated fixture contract: ' + ($profileFormValues | ConvertTo-Json -Compress))
        }
        [void][SaveProbeNative]::Post($profileForm.Handle, 0x0111, 1, 0)
        Wait-WindowGone $profileApplyApp.process 'MDLite.NativeFormWindow'
        $confirm = Wait-Modal $profileApplyApp.process
        $confirmText = @([SaveProbeNative]::Children($confirm.Handle) | ForEach-Object {
            try { [SaveProbeNative]::TextOf($_.Handle) } catch { '' }
        } | Where-Object { $_ }) -join ' | '
        # Application.cpp builds this MessageBox from the submitted scope, profile id, and preview.
        # The native child-text probe exposes only its buttons, so verify the exact form values and saved file.
        if ($confirm.Text -ne '作成プロファイル') {
            throw "Unexpected profile confirmation caption='$($confirm.Text)' buttons='$confirmText'."
        }
        [void](Send $confirm.Handle 0x0111 6 0)
        $success = Wait-Modal $profileApplyApp.process
        $successText = @([SaveProbeNative]::Children($success.Handle) | ForEach-Object {
            try { [SaveProbeNative]::TextOf($_.Handle) } catch { '' }
        } | Where-Object { $_ }) -join ' | '
        Dismiss-Modal $profileApplyApp.process $success 1
        $profilesText = Read-Utf8Fixture $profilesPath
        $profileApplyPass = ($profilesText -match '(?m)^id = "qaProfile"$' -and $profilesText -match '(?m)^collision = "sequence"$')
        $cases.Add([pscustomobject]@{ case='profile_form_apply_persists_workspace_definition'; status=$(if($profileApplyPass){'PASS'}else{'FAIL'}); expected='Profile Apply writes the definition in the isolated workspace layer'; observed=[ordered]@{ form_values=$profileFormValues; form_values_match=$profileFormValuesMatch; confirmation_caption=$confirm.Text; confirmation_buttons=$confirmText; success_dialog=$successText; file=File-Evidence $profilesPath; profile_text=$profilesText }; fixture='profile-apply' })
    } catch {
        $profileFailure = $_
        try {
            if ($success -and [SaveProbeNative]::IsWindow($success.Handle)) {
                Dismiss-Modal $profileApplyApp.process $success 1
                $profileCleanup = 'dismissed success dialog'
            } elseif ($confirm -and [SaveProbeNative]::IsWindow($confirm.Handle)) {
                Dismiss-Modal $profileApplyApp.process $confirm 7
                $profileCleanup = 'rejected confirmation with No'
            } elseif ($profileForm -and [SaveProbeNative]::IsWindow($profileForm.Handle)) {
                [void][SaveProbeNative]::Post($profileForm.Handle, 0x0111, 2, 0)
                Wait-WindowGone $profileApplyApp.process 'MDLite.NativeFormWindow'
                $profileCleanup = 'canceled form'
            } elseif ($profileApplyApp -and $profileApplyApp.process -and -not $profileApplyApp.process.HasExited) {
                $activeModal = Get-Modal $profileApplyApp.process
                if ($activeModal) {
                    Dismiss-Modal $profileApplyApp.process $activeModal 7
                    $profileCleanup = 'rejected active modal with No'
                }
            }
        } catch { $profileCleanup = 'dismissal failed: ' + $_.Exception.Message }
        $profileFileAfterFailure = if ($profilesPath -and (Test-Path -LiteralPath $profilesPath -PathType Leaf)) { File-Evidence $profilesPath } else { $null }
        $profileTextAfterFailure = if ($profilesPath -and (Test-Path -LiteralPath $profilesPath -PathType Leaf)) { Read-Utf8Fixture $profilesPath } else { $null }
        $profileCheckpoint = [ordered]@{
            form_values = $profileFormValues
            form_values_match = $profileFormValuesMatch
            confirmation_caption = if ($confirm) { $confirm.Text } else { $null }
            confirmation_buttons = $confirmText
            cleanup = $profileCleanup
            file_after_failure = $profileFileAfterFailure
            text_after_failure = $profileTextAfterFailure
        }
        Record-BlockedCase 'profile_form_apply_persists_workspace_definition' 'Profile Apply writes the definition in the isolated workspace layer.' 'profile-apply' $profileFailure $profileApplyApp $profileCheckpoint
    } finally {
        if ($profileApplyApp) { Stop-App $profileApplyApp; [void]$apps.Remove($profileApplyApp) }
    }
    }

    # External edits during the modal form must be preserved or reported as a conflict.
    if (Test-SelectedCase 'settings_external_change_conflict') {
    $settingsConflictFixture = $null
    $settingsConflictApp = $null
    try {
        $settingsConflictFixture = New-Workspace 'settings-external-change-conflict'
        $settingsConflictApp = Start-App $settingsConflictFixture.source
        $apps.Add($settingsConflictApp)
        [void][SaveProbeNative]::Post($settingsConflictApp.main, 0x0111, 1027, 0)
        $form = Wait-Window $settingsConflictApp.process 'MDLite.NativeFormWindow'
        [void](Set-ComboIndex $form.Handle 1007 3)
        $externalText = 'schema_version = 1' + $lf + 'auto_save = true' + $lf + 'auto_save_delay_ms = 1900' + $lf + 'theme = "dark"' + $lf + 'font_size_pt = 14' + $lf
        [IO.File]::WriteAllText($settingsConflictFixture.settings, $externalText, [Text.UTF8Encoding]::new($false))
        $externalBefore = File-Evidence $settingsConflictFixture.settings
        [void][SaveProbeNative]::Post($form.Handle, 0x0111, 1, 0)
        $conflictDialog = $null
        $conflictWaitError = $null
        try { $conflictDialog = Wait-Modal $settingsConflictApp.process 3000 } catch { $conflictWaitError = $_.Exception.Message }
        $conflictText = ''
        $settingsBytesAtConflict = $null
        if ($conflictDialog) {
            $conflictText = @([SaveProbeNative]::Children($conflictDialog.Handle) | ForEach-Object {
                try { [SaveProbeNative]::TextOf($_.Handle) } catch { '' }
            } | Where-Object { $_ }) -join ' | '
            $settingsBytesAtConflict = File-Evidence $settingsConflictFixture.settings
            Dismiss-Modal $settingsConflictApp.process $conflictDialog 1
        }
        $formStillOpen = $false
        try { Wait-WindowGone $settingsConflictApp.process 'MDLite.NativeFormWindow' 1500 }
        catch {
            $formStillOpen = $true
            [void][SaveProbeNative]::Post($form.Handle, 0x0111, 2, 0)
            Wait-WindowGone $settingsConflictApp.process 'MDLite.NativeFormWindow' 5000
        }
        $afterConflict = Read-Utf8Fixture $settingsConflictFixture.settings
        $externalPreserved = ($afterConflict -match '(?m)^auto_save = true$' -and
            $afterConflict -match '(?m)^auto_save_delay_ms = 1900$' -and $afterConflict -match '(?m)^font_size_pt = 14$')
        $conflictRejected = ($conflictDialog -and (File-Evidence $settingsConflictFixture.settings).sha256 -eq $externalBefore.sha256)
        $conflictPass = ($externalPreserved -or $conflictRejected)
        $cases.Add([pscustomobject]@{ case='settings_external_change_conflict'; status=$(if($conflictPass){'PASS'}else{'FAIL'}); expected='External settings are preserved or Apply reports a conflict without overwriting them'; observed=[ordered]@{ external_before_apply=$externalBefore; bytes_at_conflict=$settingsBytesAtConflict; after_apply=File-Evidence $settingsConflictFixture.settings; text_after_apply=$afterConflict; conflict_dialog_caption=if($conflictDialog){$conflictDialog.Text}else{$null}; conflict_dialog_text=$conflictText; conflict_wait_error=$conflictWaitError; form_still_open_after_apply=$formStillOpen; external_values_preserved=$externalPreserved; conflict_rejected=$conflictRejected }; fixture='settings-external-change-conflict' })
    } catch {
        Record-BlockedCase 'settings_external_change_conflict' 'External settings remain intact or Apply reports a conflict without overwriting them.' 'settings-external-change-conflict' $_ $settingsConflictApp
    } finally {
        if ($settingsConflictApp) { Stop-App $settingsConflictApp; [void]$apps.Remove($settingsConflictApp) }
    }
    }
    } catch {
        $profileFailure = $_
        if (Test-SelectedCase 'daily_meeting_memo_date_repeat_no_overwrite') { Record-BlockedCase 'daily_meeting_memo_date_repeat_no_overwrite' 'Daily opens existing date without overwrite; Meeting/Memo repeat without replacing prior files; pending draft is preserved.' 'profiles-builtins' $profileFailure $app }
        if (Test-SelectedCase 'settings_cancel_apply_persist_restart') { Record-BlockedCase 'settings_cancel_apply_persist_restart' 'Cancel preserves settings bytes; Apply persists values and restart restores them.' 'profiles-builtins' $profileFailure $app }
        if (Test-SelectedCase 'profile_form_apply_persists_workspace_definition') { Record-BlockedCase 'profile_form_apply_persists_workspace_definition' 'Profile Apply persists the definition in the isolated workspace layer.' 'profiles-builtins' $profileFailure $app }
        if (Test-SelectedCase 'settings_external_change_conflict') { Record-BlockedCase 'settings_external_change_conflict' 'External settings remain intact or Apply reports a conflict without overwriting them.' 'profiles-builtins' $profileFailure $app }
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
    }
    }

    if (Test-SelectedCase 'native_share_lock_save_failure_and_retry') {
    # Silent MessageBox errors produce no window; verify the file/state boundary instead.
    # FileShare.None must reject even an independent read handle during Save.
    $app = $null
    $lock = $null
    try {
    $saveFixture = New-Workspace 'share-lock-save' 'locked.md' ('original' + [string][char]10)
    $app = Start-App $saveFixture.source
    $apps.Add($app)
    $fileBefore = File-Evidence $saveFixture.source
    $stateBefore = App-State $app.main
    $edited = Append-Editor $app.main 'pending'
    $expectedText = 'original' + [string][char]10 + 'pending'
    $recovery = Wait-Recovery $saveFixture.root 'pending'
    $lock = [IO.File]::Open($saveFixture.source, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::None)
    $lockProofBefore = Probe-FileOpen $saveFixture.source
    if ($lockProofBefore.opened -or $lockProofBefore.win32_error -ne 32) {
        throw "FileShare.None did not prove an OS sharing violation: $($lockProofBefore | ConvertTo-Json -Compress)."
    }
    $saveDispatchResult = Send $app.main 0x0111 1005 0
    $stateAfterFailure = App-State $app.main
    $lockedHandleBytesAfterFailure = Stream-Evidence $lock 'locked.md'
    $pathOpenAfterFailure = Probe-FileOpen $saveFixture.source
    $pathBytesWhileLocked = if ($pathOpenAfterFailure.opened) { File-Evidence $saveFixture.source } else { $null }
    $failureStatePreserved = ($fileBefore.hex -ceq $lockedHandleBytesAfterFailure.hex -and
        -not $pathOpenAfterFailure.opened -and $stateAfterFailure.dirty -and
        $stateAfterFailure.editor_text -ceq $edited.editor_text -and
        $stateAfterFailure.source_length -eq $edited.source_length -and
        $stateAfterFailure.undo -eq $edited.undo -and $stateAfterFailure.redo -eq $edited.redo -and
        $stateAfterFailure.revision -eq $edited.revision -and
        $stateAfterFailure.saved_revision -eq $stateBefore.saved_revision -and
        $stateAfterFailure.anchor -eq $edited.anchor -and $stateAfterFailure.active -eq $edited.active -and
        $recovery.contains_pending_source)
    $lock.Dispose()
    $lock = $null
    $fileAfterReleaseBeforeRetry = File-Evidence $saveFixture.source
    $stateAfterRetry = $null
    $fileAfterRetry = $null
    $savedText = $null
    $retryDispatchResult = $null
    if ($failureStatePreserved -and $fileAfterReleaseBeforeRetry.hex -ceq $fileBefore.hex) {
        $retryDispatchResult = Send $app.main 0x0111 1005 0
        $deadline = [DateTime]::UtcNow.AddSeconds(8)
        do {
            $stateAfterRetry = App-State $app.main
            $fileAfterRetry = File-Evidence $saveFixture.source
            if ($fileAfterRetry.sha256 -ne $fileBefore.sha256 -and -not $stateAfterRetry.dirty) { break }
            Start-Sleep -Milliseconds 50
        } while ([DateTime]::UtcNow -lt $deadline)
        $savedText = Read-Utf8Fixture $saveFixture.source
    }
    $lockPass = ($failureStatePreserved -and $fileAfterReleaseBeforeRetry.hex -ceq $fileBefore.hex -and
        $savedText -ceq $expectedText -and $null -ne $stateAfterRetry -and -not $stateAfterRetry.dirty -and
        $stateAfterRetry.saved_revision -eq $stateAfterRetry.revision)
    $cases.Add([pscustomobject]@{ case='native_share_lock_save_failure_and_retry'; status=$(if($lockPass){'PASS'}else{'FAIL'}); expected='A proven FileShare.None denial preserves bytes, Dirty/source/history/selection and recovery; after releasing only that lock, retry writes exact source'; observed=[ordered]@{ bytes_before=$fileBefore; locked_handle_after_failed_save=$lockedHandleBytesAfterFailure; path_open_proof_before=$lockProofBefore; path_open_proof_after_save=$pathOpenAfterFailure; path_bytes_while_locked=$pathBytesWhileLocked; bytes_after_unlock_before_retry=$fileAfterReleaseBeforeRetry; bytes_after_retry=$fileAfterRetry; state_before=$stateBefore; state_after_edit=$edited; state_after_failure=$stateAfterFailure; state_after_retry=$stateAfterRetry; recovery=$recovery; save_dispatch_result=$saveDispatchResult; retry_dispatch_result=$retryDispatchResult; saved_text=$savedText }; fixture='share-lock-save'; input='Synchronous WM_COMMAND Save under MDLITE_TEST_SILENT=1; no modal expected.' })
    } catch {
        Record-BlockedCase 'native_share_lock_save_failure_and_retry' 'Sharing violation preserves original bytes and draft state; retry writes the exact source.' 'share-lock-save' $_ $app
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
        if ($lock) { $lock.Dispose(); $lock = $null }
    }
    }

    # A read-only file attribute must reject the native Save route without losing the draft.
    if (Test-SelectedCase 'native_readonly_attribute_save_failure_and_retry') {
    $app = $null
    try {
    $readOnlyFixture = New-Workspace 'readonly-attribute-save' 'readonly.md' ('baseline' + [string][char]10)
    $app = Start-App $readOnlyFixture.source
    $apps.Add($app)
    $readOnlyBefore = File-Evidence $readOnlyFixture.source
    $readOnlyAttributesBefore = [IO.File]::GetAttributes($readOnlyFixture.source)
    $readOnlyStateBefore = App-State $app.main
    $readOnlyEdited = Append-Editor $app.main ' permission-pending'
    $readOnlyExpectedText = 'baseline' + [string][char]10 + ' permission-pending'
    $readOnlyRecovery = Wait-Recovery $readOnlyFixture.root 'permission-pending'
    $readOnlyAttributesApplied = $null
    $readOnlyAttributesAfterRestore = $null
    $readOnlyAttributesAfterSave = $null
    $readOnlyFileAfterSave = $null
    $readOnlyStateAfterSave = $null
    $readOnlyStateAfterRetry = $null
    $readOnlyFileAfterRetry = $null
    $readOnlySavedText = $null
    $readOnlySaveDispatch = $null
    $readOnlyRetryDispatch = $null
    $readOnlyAttributesInjected = $false
    $readOnlyFailureSafe = $false
    $readOnlyRetryPass = $false
    try {
        $readOnlyMask = [IO.FileAttributes]([int]$readOnlyAttributesBefore -bor [int][IO.FileAttributes]::ReadOnly)
        [IO.File]::SetAttributes($readOnlyFixture.source, $readOnlyMask)
        $readOnlyAttributesInjected = $true
        $readOnlyAttributesApplied = [IO.File]::GetAttributes($readOnlyFixture.source)
        if (($readOnlyAttributesApplied -band [IO.FileAttributes]::ReadOnly) -eq 0) {
            throw 'The isolated source did not retain FILE_ATTRIBUTE_READONLY before Save.'
        }
        $readOnlySaveDispatch = Send $app.main 0x0111 1005 0
        $readOnlyFileAfterSave = File-Evidence $readOnlyFixture.source
        $readOnlyStateAfterSave = App-State $app.main
        $readOnlyAttributesAfterSave = [IO.File]::GetAttributes($readOnlyFixture.source)
        $readOnlyFailureSafe = (
            $readOnlyBefore.hex -ceq $readOnlyFileAfterSave.hex -and
            (($readOnlyAttributesAfterSave -band [IO.FileAttributes]::ReadOnly) -ne 0) -and
            $readOnlyStateAfterSave.dirty -and
            $readOnlyStateAfterSave.editor_text -ceq $readOnlyEdited.editor_text -and
            $readOnlyStateAfterSave.source_length -eq $readOnlyEdited.source_length -and
            $readOnlyStateAfterSave.undo -eq $readOnlyEdited.undo -and
            $readOnlyStateAfterSave.redo -eq $readOnlyEdited.redo -and
            $readOnlyStateAfterSave.revision -eq $readOnlyEdited.revision -and
            $readOnlyStateAfterSave.saved_revision -eq $readOnlyStateBefore.saved_revision -and
            $readOnlyStateAfterSave.anchor -eq $readOnlyEdited.anchor -and
            $readOnlyStateAfterSave.active -eq $readOnlyEdited.active -and
            $readOnlyRecovery.contains_pending_source)
        if ($readOnlyFailureSafe) {
            [IO.File]::SetAttributes($readOnlyFixture.source, $readOnlyAttributesBefore)
            $readOnlyAttributesAfterRestore = [IO.File]::GetAttributes($readOnlyFixture.source)
            $readOnlyAttributesInjected = $false
            $readOnlyRetryDispatch = Send $app.main 0x0111 1005 0
            $deadline = [DateTime]::UtcNow.AddSeconds(8)
            do {
                $readOnlyStateAfterRetry = App-State $app.main
                $readOnlyFileAfterRetry = File-Evidence $readOnlyFixture.source
                if ($readOnlyFileAfterRetry.sha256 -ne $readOnlyBefore.sha256 -and -not $readOnlyStateAfterRetry.dirty) { break }
                Start-Sleep -Milliseconds 50
            } while ([DateTime]::UtcNow -lt $deadline)
            $readOnlySavedText = Read-Utf8Fixture $readOnlyFixture.source
            $readOnlyRetryPass = ($readOnlySavedText -ceq $readOnlyExpectedText -and
                -not $readOnlyStateAfterRetry.dirty -and
                $readOnlyStateAfterRetry.saved_revision -eq $readOnlyStateAfterRetry.revision)
        } else {
            $readOnlySavedText = Read-Utf8Fixture $readOnlyFixture.source
        }
    }
    finally {
        if ($readOnlyAttributesInjected -and $app) {
            Stop-App $app
            [void]$apps.Remove($app)
        }
        if (Test-Path -LiteralPath $readOnlyFixture.source -PathType Leaf) {
            [IO.File]::SetAttributes($readOnlyFixture.source, $readOnlyAttributesBefore)
        }
    }
    $readOnlyAttributesFinal = [IO.File]::GetAttributes($readOnlyFixture.source)
    $readOnlyFailurePass = ($readOnlyFailureSafe -and $readOnlyRetryPass -and
        $readOnlyAttributesFinal -eq $readOnlyAttributesBefore)
    $cases.Add([pscustomobject]@{ case='native_readonly_attribute_save_failure_and_retry'; status=$(if($readOnlyFailurePass){'PASS'}else{'FAIL'}); expected='FILE_ATTRIBUTE_READONLY rejects native Save without changing original bytes or draft state; exact attributes are restored in finally and retry saves the exact source'; observed=[ordered]@{ bytes_before=$readOnlyBefore; bytes_while_readonly_after_save=$readOnlyFileAfterSave; bytes_after_retry=$readOnlyFileAfterRetry; attributes_before=[int]$readOnlyAttributesBefore; attributes_during_save=[int]$readOnlyAttributesApplied; attributes_after_save=[int]$readOnlyAttributesAfterSave; attributes_before_retry=[int]$readOnlyAttributesAfterRestore; attributes_final=[int]$readOnlyAttributesFinal; readonly_failure_safe=$readOnlyFailureSafe; retry_pass=$readOnlyRetryPass; state_before=$readOnlyStateBefore; state_after_edit=$readOnlyEdited; state_after_save=$readOnlyStateAfterSave; state_after_retry=$readOnlyStateAfterRetry; recovery=$readOnlyRecovery; save_dispatch_result=$readOnlySaveDispatch; retry_dispatch_result=$readOnlyRetryDispatch; message_box='MDLITE_TEST_SILENT=1 suppresses error popup; state and bytes are the oracle.'; saved_text=$readOnlySavedText }; fixture='readonly-attribute-save'; input='Synchronous WM_COMMAND Save under MDLITE_TEST_SILENT=1; no physical keyboard or ACL/security changes.' })
    } catch {
        Record-BlockedCase 'native_readonly_attribute_save_failure_and_retry' 'FILE_ATTRIBUTE_READONLY rejects native Save without changing bytes or draft state; attributes are restored exactly and retry succeeds.' 'readonly-attribute-save' $_ $app
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
    }
    }

    # An external rewrite after load must never be silently replaced.
    if (Test-SelectedCase 'external_file_change_blocks_save') {
    $app = $null
    try {
    $externalFixture = New-Workspace 'external-change' 'external.md' ('baseline' + [string][char]10)
    $app = Start-App $externalFixture.source
    $apps.Add($app)
    $baseline = File-Evidence $externalFixture.source
    [void](Append-Editor $app.main ' local-edit')
    $pendingState = App-State $app.main
    $externalText = 'external replacement' + [string][char]10
    [IO.File]::WriteAllText($externalFixture.source, $externalText, [Text.UTF8Encoding]::new($false))
    $externalBytes = File-Evidence $externalFixture.source
    $recovery = Wait-Recovery $externalFixture.root 'local-edit'
    $saveDispatchResult = Send $app.main 0x0111 1005 0
    $after = File-Evidence $externalFixture.source
    $stateAfter = App-State $app.main
    $externalPass = ($externalBytes.sha256 -eq $after.sha256 -and $stateAfter.dirty -and
        $stateAfter.editor_text -ceq $pendingState.editor_text -and $recovery.contains_pending_source -and
        $stateAfter.saved_revision -eq $pendingState.saved_revision -and
        $stateAfter.revision -eq $pendingState.revision -and $stateAfter.undo -eq $pendingState.undo -and
        $stateAfter.redo -eq $pendingState.redo -and $stateAfter.anchor -eq $pendingState.anchor -and
        $stateAfter.active -eq $pendingState.active)
    $cases.Add([pscustomobject]@{ case='external_file_change_blocks_save'; status=$(if($externalPass){'PASS'}else{'FAIL'}); expected='External bytes remain and local source stays Dirty and recoverable'; observed=[ordered]@{ baseline=$baseline; external_write=$externalBytes; after_attempt=$after; state_pending=$pendingState; state_after=$stateAfter; recovery=$recovery; save_dispatch_result=$saveDispatchResult; message_box='MDLITE_TEST_SILENT=1 suppresses popup; state and bytes determine the save result.' }; fixture='external-change'; input='Synchronous WM_COMMAND Save under MDLITE_TEST_SILENT=1; no modal expected.' })
    } catch {
        Record-BlockedCase 'external_file_change_blocks_save' 'External bytes remain and local source stays Dirty, unchanged in history/selection, and recoverable.' 'external-change' $_ $app
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
    }
    }

    # CP932 cannot represent emoji; the native Save route must retain the buffer.
    if (Test-SelectedCase 'cp932_conversion_failure') {
    $app = $null
    try {
    $cpFixture = New-Workspace 'cp932-conversion' 'cp932.md' 'placeholder'
    $japanese = ([char]0x65E5).ToString() + [char]0x672C + [char]0x8A9E
    $emoji = [char]::ConvertFromUtf32(0x1F600)
    [IO.File]::WriteAllBytes($cpFixture.source, [Text.Encoding]::GetEncoding(932).GetBytes($japanese))
    $app = Start-App $cpFixture.source
    $apps.Add($app)
    $cpBefore = File-Evidence $cpFixture.source
    $cpStateBefore = App-State $app.main
    $cpEdit = Append-Editor $app.main (' ' + $emoji)
    $cpRecovery = Wait-Recovery $cpFixture.root $emoji
    $cpSaveDispatch = Send $app.main 0x0111 1005 0
    $cpAfter = File-Evidence $cpFixture.source
    $cpState = App-State $app.main
    $cpPass = ($cpBefore.sha256 -eq $cpAfter.sha256 -and $cpState.dirty -and
        $cpState.editor_text.Contains($emoji) -and $cpRecovery.contains_pending_source -and
        $cpState.saved_revision -eq $cpStateBefore.saved_revision -and
        $cpState.revision -eq $cpEdit.revision -and $cpState.undo -eq $cpEdit.undo -and
        $cpState.redo -eq $cpEdit.redo -and $cpState.anchor -eq $cpEdit.anchor -and $cpState.active -eq $cpEdit.active)
    $cases.Add([pscustomobject]@{ case='cp932_conversion_failure'; status=$(if($cpPass){'PASS'}else{'FAIL'}); expected='Unrepresentable text is rejected while preserving original CP932 bytes, Dirty source and recovery'; observed=[ordered]@{ bytes_before=$cpBefore; bytes_after=$cpAfter; state_before=$cpStateBefore; state_after_edit=$cpEdit; state_after_attempt=$cpState; recovery=$cpRecovery; save_dispatch_result=$cpSaveDispatch; message_box='MDLITE_TEST_SILENT=1 suppresses popup; state and bytes determine the save result.' }; fixture='cp932-conversion'; input='Synchronous WM_COMMAND Save under MDLITE_TEST_SILENT=1; no modal expected.' })
    } catch {
        Record-BlockedCase 'cp932_conversion_failure' 'Unrepresentable Unicode is rejected while preserving exact CP932 bytes, Dirty source, and recovery.' 'cp932-conversion' $_ $app
    } finally {
        if ($app) { Stop-App $app; [void]$apps.Remove($app) }
    }
    }

    $sourceAtEnd = Get-SourceFingerprint $repoRoot
    $exeAtEnd = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    $identityMatches = ($sourceAtStart.sha256 -eq $sourceAtEnd.sha256 -and $receipt.executable_sha256 -eq $exeAtEnd)
    $failed = @($cases | Where-Object { $_.status -ne 'PASS' })
    $result = [pscustomobject]@{
        schema = 'mdlite-zk1-8-save-profile-settings-v1'
        run_id = $runId
        completed_utc = [DateTime]::UtcNow.ToString('o')
        preset = $Preset
        build_run_id = $receipt.build_run_id
        build_receipt_path = [IO.Path]::GetFullPath($receiptPath)
        build_receipt_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $receiptPath).Hash.ToLowerInvariant()
        source_sha256_at_start = $sourceAtStart.sha256
        source_sha256_at_end = $sourceAtEnd.sha256
        executable_path = [IO.Path]::GetFullPath($executable)
        executable_sha256_at_start = $receipt.executable_sha256
        executable_sha256_at_end = $exeAtEnd
        identity_matches = $identityMatches
        fixture_root = [IO.Path]::GetFullPath($runRoot)
        synthetic_input = 'Cross-process bounded Win32 messages; no physical keyboard or IME.'
        scope_status = if ($script:onlyCases.Count -gt 0) { 'Explicit case selection; inspect selected_cases and not_run_cases before interpreting pass.' } else { 'Focused acceptance subset only; inspect not_run_cases before interpreting pass.' }
        selected_cases = $selectedCases
        not_run_cases = $allNotRunCases
        cases = $cases.ToArray()
        pass = ($failed.Count -eq 0 -and $identityMatches)
    }
    [IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
    $result | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    $result | ConvertTo-Json -Depth 7
    if (-not $result.pass) { exit 1 }
} catch {
    $cases.Add([pscustomobject]@{ case='focused_harness'; status='FAIL'; error=$_.Exception.Message; phase='sequence stopped before normal result assembly' })
    try {
        $sourceAtEnd = Get-SourceFingerprint $repoRoot
        $exeAtEnd = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
    } catch {
        $sourceAtEnd = [pscustomobject]@{ sha256='UNAVAILABLE'; file_count=0 }
        $exeAtEnd = 'UNAVAILABLE'
    }
    $fallback = [pscustomobject]@{
        schema='mdlite-zk1-8-save-profile-settings-v1'
        run_id=$runId
        completed_utc=[DateTime]::UtcNow.ToString('o')
        preset=$Preset
        build_run_id=$receipt.build_run_id
        build_receipt_path=[IO.Path]::GetFullPath($receiptPath)
        source_sha256_at_start=$sourceAtStart.sha256
        source_sha256_at_end=$sourceAtEnd.sha256
        executable_sha256_at_start=$receipt.executable_sha256
        executable_sha256_at_end=$exeAtEnd
        fixture_root=[IO.Path]::GetFullPath($runRoot)
        selected_cases=$selectedCases
        not_run_cases=$allNotRunCases
        cases=$cases.ToArray()
        pass=$false
    }
    [IO.Directory]::CreateDirectory((Split-Path -Parent $OutputPath)) | Out-Null
    $fallback | ConvertTo-Json -Depth 9 | Set-Content -LiteralPath $OutputPath -Encoding utf8
    throw
} finally {
    foreach ($appItem in @($apps.ToArray())) { Stop-App $appItem }
    if ($null -eq $oldSilent) { Remove-Item Env:MDLITE_TEST_SILENT -ErrorAction SilentlyContinue }
    else { $env:MDLITE_TEST_SILENT = $oldSilent }
}
