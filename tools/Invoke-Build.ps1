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
    & $cmake --preset $Preset
    $configureExitCode = $LASTEXITCODE
    if ($configureExitCode -ne 0) { throw 'CMake configure failed.' }
    & $cmake --build --preset $Preset
    $buildExitCode = $LASTEXITCODE
    if ($buildExitCode -ne 0) { throw 'CMake build failed.' }
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
