$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$FixtureRoot = Join-Path $ProjectRoot 'tests/fixtures'
$DemoRoot = Join-Path $ProjectRoot 'samples/rust-editor'
foreach ($Path in @((Join-Path $ProjectRoot 'samples'), $DemoRoot, (Join-Path $DemoRoot 'assets'))) {
    if (Test-Path -LiteralPath $Path) {
        if ((Get-Item -LiteralPath $Path -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) {
            throw 'The generated demo must not traverse a link outside the project.'
        }
    } else { New-Item -ItemType Directory -Path $Path | Out-Null }
}
foreach ($File in (Get-ChildItem -LiteralPath $FixtureRoot -File -Recurse)) {
    $Relative = [IO.Path]::GetRelativePath($FixtureRoot, $File.FullName)
    $Destination = Join-Path $DemoRoot $Relative
    if (Test-Path -LiteralPath $Destination) { continue }
    $Directory = Split-Path -Parent $Destination
    New-Item -ItemType Directory -Path $Directory -Force | Out-Null
    # Existing demo text and images are always retained, including hand edits.
    $Source = [IO.File]::OpenRead($File.FullName)
    try {
        $Target = [IO.File]::Open($Destination, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write)
        try { $Source.CopyTo($Target) } finally { $Target.Dispose() }
    } finally { $Source.Dispose() }
}
