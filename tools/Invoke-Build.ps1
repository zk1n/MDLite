#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('debug', 'release')]
    [string]$Preset = 'debug',
    [switch]$Test,
    [switch]$Run
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$env:VSLANG = '1033'
$repoRoot = Split-Path -Parent $PSScriptRoot
$buildRunId = [guid]::NewGuid().ToString('N')
$receiptPath = Join-Path $repoRoot "build\verification\build-receipt-$Preset.json"
# Invalidate the prior receipt before starting; a failed build cannot leave it current.
if (Test-Path -LiteralPath $receiptPath -PathType Leaf) {
    Remove-Item -LiteralPath $receiptPath -Force
    if (Test-Path -LiteralPath $receiptPath -PathType Leaf) {
        throw "Could not invalidate the previous build receipt: $receiptPath"
    }
}
$receiptDirectory = Split-Path -Parent $receiptPath
New-Item -ItemType Directory -Path $receiptDirectory -Force | Out-Null

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

function Get-NinjaProbeOutput([string]$Ninja, [string]$ProbeRoot, [string]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Ninja
    $start.Arguments = $Arguments
    $start.WorkingDirectory = $ProbeRoot
    $start.UseShellExecute = $false
    # Inherit the invoking console, as CMake's real Ninja build does. Detaching changes Ninja's output codepage.
    $start.CreateNoWindow = $false
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $stdout = [IO.MemoryStream]::new()
    $stderr = [IO.MemoryStream]::new()
    try {
        if (-not $process.Start()) { throw 'Ninja dependency prefix probe did not start.' }
        $stdoutCopy = $process.StandardOutput.BaseStream.CopyToAsync($stdout)
        $stderrCopy = $process.StandardError.BaseStream.CopyToAsync($stderr)
        if (-not $process.WaitForExit(15000)) {
            $process.Kill()
            $process.WaitForExit()
            throw 'Ninja dependency prefix probe exceeded 15000 ms.'
        }
        [Threading.Tasks.Task]::WaitAll([Threading.Tasks.Task[]]@($stdoutCopy, $stderrCopy))
        if ($process.ExitCode -ne 0) { throw "Ninja dependency prefix probe failed with exit code $($process.ExitCode)." }
        return ,([byte[]]($stdout.ToArray() + [byte[]]@(10) + $stderr.ToArray()))
    } finally { $process.Dispose(); $stdout.Dispose(); $stderr.Dispose() }
}

function Repair-MsvcIncludePrefix([string]$BuildRoot, [string]$ProbeRoot, [byte[]]$PreviousPrefixBytes, [string]$Ninja) {
    # Ninja normalizes compiler output before parsing dependencies. Probe through that same execution path.
    $byteEncoding = [Text.Encoding]::GetEncoding(28591)
    $cache = [IO.File]::ReadAllText((Join-Path $BuildRoot 'CMakeCache.txt'))
    $compilerMatch = [regex]::Match($cache, '(?m)^CMAKE_CXX_COMPILER:FILEPATH=([^\r\n]+)')
    if (-not $compilerMatch.Success) { throw 'Configured C++ compiler was not recorded in CMakeCache.txt.' }
    $compiler = $compilerMatch.Groups[1].Value.Replace('$', '$$')
    [IO.Directory]::CreateDirectory($ProbeRoot) | Out-Null
    $header = Join-Path $ProbeRoot 'mdlite-prefix-probe.h'
    $source = Join-Path $ProbeRoot 'mdlite-prefix-probe.cpp'
    [IO.File]::WriteAllText($header, "// Synthetic dependency prefix probe.`n", [Text.Encoding]::ASCII)
    [IO.File]::WriteAllText($source, "#include `"mdlite-prefix-probe.h`"`n", [Text.Encoding]::ASCII)
    $probeNinja = Join-Path $ProbeRoot 'prefix-probe.ninja'
    # /EP uses a different output encoding from /c on this MSVC; mirror the real compilation command.
    $discoveryRule = 'rule prefix_discovery' + "`n" + '  command = "' + $compiler +
        '" /nologo /showIncludes /c mdlite-prefix-probe.cpp /Fomdlite-prefix-discovery.obj' + "`n" +
        'build mdlite-prefix-discovery.obj: prefix_discovery mdlite-prefix-probe.cpp' + "`n"
    [IO.File]::WriteAllText($probeNinja, $discoveryRule, [Text.UTF8Encoding]::new($false))
    $output = $byteEncoding.GetString((Get-NinjaProbeOutput $Ninja $ProbeRoot '-f prefix-probe.ninja mdlite-prefix-discovery.obj'))
    $includeLines = @([regex]::Matches($output,
        '(?m)^(?<prefix>[^\r\n]*?)(?<path>[A-Za-z]:[\\/][^\r\n]*mdlite-prefix-probe\.h)\r?$') | Where-Object {
        # Latin1 above is a reversible byte mapping, not the encoding of the native path.
        # Ninja inherits the invoking console's encoding; decode only the path before Unicode comparison.
        $nativePath = [Console]::OutputEncoding.GetString($byteEncoding.GetBytes($_.Groups['path'].Value))
        [IO.Path]::GetFullPath($nativePath).Equals([IO.Path]::GetFullPath($header), [StringComparison]::OrdinalIgnoreCase)
    })
    if ($includeLines.Count -ne 1) { throw 'MSVC dependency prefix probe did not produce one exact fixture-header include line.' }
    $prefix = $includeLines[0].Groups['prefix'].Value
    if (-not $prefix) { throw 'Ninja compiler include-prefix discovery returned an empty prefix.' }
    # Byte equality alone is insufficient: prove Ninja actually records the fixture header.
    $compileRule = 'rule prefix_compile' + "`n" + '  deps = msvc' + "`n" + '  command = "' + $compiler +
        '" /nologo /showIncludes /c mdlite-prefix-probe.cpp /Fomdlite-prefix-probe.obj' + "`n" +
        'build mdlite-prefix-probe.obj: prefix_compile mdlite-prefix-probe.cpp' + "`n"
    $probeRules = $byteEncoding.GetBytes('msvc_deps_prefix = ' + $prefix + "`n") + [Text.UTF8Encoding]::new($false).GetBytes($compileRule)
    [IO.File]::WriteAllBytes($probeNinja, [byte[]]$probeRules)
    [void](Get-NinjaProbeOutput $Ninja $ProbeRoot '-f prefix-probe.ninja mdlite-prefix-probe.obj')
    $dependencies = $byteEncoding.GetString((Get-NinjaProbeOutput $Ninja $ProbeRoot '-f prefix-probe.ninja -t deps mdlite-prefix-probe.obj'))
    if ($dependencies -notmatch '#deps [1-9][0-9]*,' -or
        $dependencies -notmatch '(?m)^\s+(?:[^\r\n]*[\\/])?mdlite-prefix-probe\.h\r?$') {
        throw 'Ninja dependency prefix probe did not record the actual fixture header.'
    }
    # Keep CMake's compiler metadata UTF-8. Raw localized bytes belong only in Ninja's byte parser.
    $rulesPath = Join-Path $BuildRoot 'CMakeFiles\rules.ninja'
    $rules = $byteEncoding.GetString([IO.File]::ReadAllBytes($rulesPath))
    $line = [regex]::Match($rules, '(?m)^msvc_deps_prefix = (?<prefix>[^\r\n]*)(?=\r?$)')
    if (-not $line.Success) { throw 'Generated Ninja dependency prefix was not found.' }
    if ($line.Groups['prefix'].Value -cne $prefix) {
        $replacement = 'msvc_deps_prefix = ' + $prefix
        $rules = $rules.Substring(0, $line.Index) + $replacement + $rules.Substring($line.Index + $line.Length)
        [IO.File]::WriteAllBytes($rulesPath, $byteEncoding.GetBytes($rules))
    }
    $changed = (-not $PreviousPrefixBytes -or $byteEncoding.GetString($PreviousPrefixBytes) -cne $prefix)
    return [pscustomobject]@{ changed = $changed; prefix_bytes = $byteEncoding.GetBytes($prefix); header_dependency_recorded = $true }
}

function Get-NinjaIncludePrefix([string]$BuildRoot) {
    $path = Join-Path $BuildRoot 'CMakeFiles\rules.ninja'
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { return $null }
    $byteEncoding = [Text.Encoding]::GetEncoding(28591)
    $rules = $byteEncoding.GetString([IO.File]::ReadAllBytes($path))
    $line = [regex]::Match($rules, '(?m)^msvc_deps_prefix = (?<prefix>[^\r\n]*)\r?$')
    if (-not $line.Success) { throw 'Prior Ninja dependency prefix was not found.' }
    return ,($byteEncoding.GetBytes($line.Groups['prefix'].Value))
}

function Assert-NinjaIncludePrefix([string]$BuildRoot, [byte[]]$PrefixBytes) {
    $byteEncoding = [Text.Encoding]::GetEncoding(28591)
    $rules = $byteEncoding.GetString([IO.File]::ReadAllBytes((Join-Path $BuildRoot 'CMakeFiles\rules.ninja')))
    $line = [regex]::Match($rules, '(?m)^msvc_deps_prefix = (?<prefix>[^\r\n]*)\r?$')
    if (-not $line.Success -or $line.Groups['prefix'].Value -cne $byteEncoding.GetString($PrefixBytes)) {
        throw 'Generated Ninja dependency prefix does not byte-match the actual MSVC include output.'
    }
}

function Assert-ProjectHeaderDependencies([string]$BuildRoot, [string]$Ninja) {
    $byteEncoding = [Text.Encoding]::GetEncoding(28591)
    foreach ($check in @(
        @{ object = 'CMakeFiles/MDLite.dir/src/app/Application.cpp.obj'; header = 'Application.h' },
        @{ object = 'CMakeFiles/mdlite_calendar_view_tests.dir/tests/CalendarViewTests.cpp.obj'; header = 'CalendarView.h' }
    )) {
        $dependencies = $byteEncoding.GetString((Get-NinjaProbeOutput $Ninja $BuildRoot ('-t deps ' + $check.object)))
        $headerPattern = '(?m)^\s+(?:[^\r\n]*[\\/])?' + [regex]::Escape($check.header) + '\r?$'
        if ($dependencies -notmatch '#deps [1-9][0-9]*,' -or $dependencies -notmatch $headerPattern) {
            throw "Ninja did not record $($check.header) for $($check.object); no build receipt will be written."
        }
    }
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw 'vswhere.exe was not found. Install the MSVC x64 build tools.'
}

$installation = (& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath).Trim()
if (-not $installation) { throw 'A compatible MSVC installation was not found.' }

$cmakeRoot = Join-Path $installation 'Common7\IDE\CommonExtensions\Microsoft\CMake'
$cmake = Get-ChildItem -LiteralPath $cmakeRoot -Filter cmake.exe -File -Recurse | Select-Object -First 1 -ExpandProperty FullName
$ninja = Get-ChildItem -LiteralPath $cmakeRoot -Filter ninja.exe -File -Recurse | Select-Object -First 1 -ExpandProperty FullName
$devShell = Join-Path $installation 'Common7\Tools\Microsoft.VisualStudio.DevShell.dll'
if (-not $cmake -or -not $ninja -or -not (Test-Path -LiteralPath $devShell)) {
    throw 'The Visual Studio CMake, Ninja, or developer shell component was not found.'
}

Import-Module $devShell
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64' | Out-Null
$env:VSLANG = '1033'
$env:PATH = (Split-Path -Parent $ninja) + [IO.Path]::PathSeparator + $env:PATH

Push-Location -LiteralPath $repoRoot
try {
    $sourceAtBuildStart = Get-SourceFingerprint $repoRoot
    $buildRoot = Join-Path $repoRoot "build\$Preset"
    $previousPrefixBytes = Get-NinjaIncludePrefix $buildRoot
    & $cmake --preset $Preset
    $configureExitCode = $LASTEXITCODE
    if ($configureExitCode -ne 0) { throw 'CMake configure failed.' }
    $prefixRepair = Repair-MsvcIncludePrefix $buildRoot (Join-Path $receiptDirectory "msvc-prefix-$buildRunId") $previousPrefixBytes $ninja
    Assert-NinjaIncludePrefix $buildRoot $prefixRepair.prefix_bytes
    # Header dependencies already lost under a bad prefix require one clean build to repopulate .ninja_deps.
    $buildArguments = @('--build', '--preset', $Preset)
    if ($prefixRepair.changed) { $buildArguments += '--clean-first' }
    & $cmake @buildArguments
    $buildExitCode = $LASTEXITCODE
    if ($buildExitCode -ne 0) { throw 'CMake build failed.' }
    Assert-ProjectHeaderDependencies $buildRoot $ninja
    $testExitCode = $null
    if ($Test) {
        & (Join-Path (Split-Path -Parent $cmake) 'ctest.exe') --preset $Preset
        $testExitCode = $LASTEXITCODE
        if ($testExitCode -ne 0) { throw 'CTest failed.' }
    }

    $executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "Successful build did not produce the expected executable: $executable"
    }
    $sourceAtBuildEnd = Get-SourceFingerprint $repoRoot
    if ($sourceAtBuildStart.sha256 -ne $sourceAtBuildEnd.sha256) {
        throw 'Build source inputs changed during the build; no receipt was written.'
    }
    $receipt = [pscustomobject]@{
        schema = 'mdlite-build-receipt-v2'
        build_run_id = $buildRunId
        completed_utc = [DateTime]::UtcNow.ToString('o')
        preset = $Preset
        build_status = 'PASS'
        configure_exit_code = $configureExitCode
        build_exit_code = $buildExitCode
        test_requested = [bool]$Test
        test_exit_code = $testExitCode
        executable_path = [IO.Path]::GetFullPath($executable)
        executable_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $executable).Hash.ToLowerInvariant()
        source_sha256 = $sourceAtBuildStart.sha256
        source_file_count = $sourceAtBuildStart.file_count
        dependency_prefix_verified = $true
        dependency_probe_header_recorded = $prefixRepair.header_dependency_recorded
        product_header_dependencies_verified = $true
        dependency_repair_clean_first = $prefixRepair.changed
    }
    [IO.File]::WriteAllText($receiptPath, ($receipt | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
    Write-Host "Build receipt: $receiptPath"

    if ($Run) {
        & $executable
    }
}
finally {
    Pop-Location
}
