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
    & $cmake --preset $Preset
    if ($LASTEXITCODE -ne 0) { throw 'CMake configure failed.' }
    & $cmake --build --preset $Preset
    if ($LASTEXITCODE -ne 0) { throw 'CMake build failed.' }
    if ($Test) {
        & (Join-Path (Split-Path -Parent $cmake) 'ctest.exe') --preset $Preset
        if ($LASTEXITCODE -ne 0) { throw 'CTest failed.' }
    }
    if ($Run) {
        & (Join-Path $repoRoot "build\$Preset\MDLite.exe")
    }
}
finally {
    Pop-Location
}
