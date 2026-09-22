#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$OutputRoot = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputRoot) {
    $OutputRoot = Join-Path $repoRoot 'samples\demo-workspace'
}
$OutputRoot = [IO.Path]::GetFullPath($OutputRoot)
[IO.Directory]::CreateDirectory($OutputRoot) | Out-Null
[IO.Directory]::CreateDirectory((Join-Path $OutputRoot '.mdlite')) | Out-Null

# F5 data is disposable. Never overwrite an existing README or .mdlite state:
# this makes a local user's sample workspace safe to reuse between launches.
$readme = Join-Path $OutputRoot 'README.md'
if (-not (Test-Path -LiteralPath $readme -PathType Leaf)) {
    $defaultReadme = @"
# MDLite F5 demo workspace

This workspace is generated for local F5 and native GUI checks.

| Item | Value |
| --- | --- |
| Encoding | UTF-8 |
| Line ending | LF |
| Purpose | Disposable F5 smoke workspace |

Edit this file to exercise source, table, caret, and Undo/Redo behavior. The
workspace is intentionally outside the product source of truth.
"@
    [IO.File]::WriteAllText($readme, $defaultReadme, [Text.UTF8Encoding]::new($false))
}

Write-Output $OutputRoot