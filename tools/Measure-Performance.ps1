#Requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('release','debug')]
    [string]$Preset = 'release'
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $repoRoot "build\$Preset\MDLite.exe"
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) { throw "Build first: $executable" }

$runRoot = Join-Path ([IO.Path]::GetTempPath()) ("mdlite-performance-" + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($runRoot) | Out-Null

function Measure-Workspace([string]$Name, [scriptblock]$Prepare) {
    $workspace = Join-Path $runRoot $Name
    [IO.Directory]::CreateDirectory($workspace) | Out-Null
    [IO.Directory]::CreateDirectory((Join-Path $workspace '.mdlite')) | Out-Null
    $launchTarget = & $Prepare $workspace
    if (-not $launchTarget) { $launchTarget = $workspace }
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $executable -ArgumentList @($launchTarget) -PassThru -WindowStyle Hidden
    try {
        Start-Sleep -Seconds 5
        $deadline = [DateTime]::UtcNow.AddSeconds(15)
        do {
            Start-Sleep -Milliseconds 100
            $process.Refresh()
        } while (-not $process.Responding -and [DateTime]::UtcNow -lt $deadline)
        $watch.Stop()
        [pscustomobject]@{
            scenario = $Name
            ready_ms = $watch.ElapsedMilliseconds
            responding = $process.Responding
            working_set_bytes = $process.WorkingSet64
            private_bytes = $process.PrivateMemorySize64
        }
    }
    finally {
        if (-not $process.HasExited) { $process.CloseMainWindow() | Out-Null; if (-not $process.WaitForExit(3000)) { $process.Kill() } }
        $process.Dispose()
    }
}

try {
    $results = @()
    $results += Measure-Workspace 'P0-empty' { param($workspace) }
    $results += Measure-Workspace 'P1-2000-files' {
        param($workspace)
        for ($index = 0; $index -lt 2000; $index++) {
            [IO.File]::WriteAllText((Join-Path $workspace ("note-{0:D4}.md" -f $index)), "# Note $index`n`n日本語 search token`n")
        }
    }
    foreach ($sizeMiB in 20, 100) {
        $results += Measure-Workspace ("P-large-{0}MiB" -f $sizeMiB) {
            param($workspace)
            $path = Join-Path $workspace 'large.md'
            $stream = [IO.File]::Create($path)
            try {
                $heading = [Text.Encoding]::UTF8.GetBytes("# Large document`n")
                $stream.Write($heading, 0, $heading.Length)
                # P3 includes a genuinely large single line without turning every
                # repeated block into a heading (which would measure outline-node
                # cardinality instead of large-document copy and mapping cost).
                $giant = [Text.Encoding]::UTF8.GetBytes(('x' * 1MB) + "`n")
                $stream.Write($giant, 0, $giant.Length)
                $block = [Text.Encoding]::UTF8.GetBytes("日本語 content line with plain markdown text`n")
                $target = $sizeMiB * 1MB
                while ($stream.Length -lt $target) { $stream.Write($block, 0, $block.Length) }
            } finally { $stream.Dispose() }
            $path
        }
    }
    $results | ConvertTo-Json -Depth 3
}
finally {
    if (Test-Path -LiteralPath $runRoot) { Remove-Item -LiteralPath $runRoot -Recurse -Force }
}
