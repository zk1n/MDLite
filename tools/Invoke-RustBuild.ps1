param(
    [ValidateSet('Debug','Release')][string]$Configuration = 'Debug',
    [switch]$Verify,
    [switch]$PrepareDemo,
    [string]$TargetDirectory = 'target',
    [string]$EvidenceDirectory = '.codex-runtime/rust-foundation'
)
$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $ProjectRoot
try {
    $Mise = Get-Command mise -CommandType Application -ErrorAction Stop
    $RustVersion = & $Mise exec -- rustc --version
    if ($LASTEXITCODE -ne 0) { throw 'Rust toolchain unavailable through mise.' }
    Write-Output $RustVersion
    if ($RustVersion -notmatch '^rustc 1\.99\.0\s') {
        throw 'MDLite requires the recorded Rust 1.99.0 toolchain. Do not update or reinstall automatically.'
    }
    $BuildArgs = @('exec','--','cargo','build','--locked','--target-dir',$TargetDirectory)
    if ($Configuration -eq 'Release') { $BuildArgs += '--release' }
    & $Mise @BuildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    if ($Verify) {
        $TestArgs = @('exec','--','cargo','test','--locked','--all-targets','--target-dir',$TargetDirectory)
        if ($Configuration -eq 'Release') { $TestArgs += '--release' }
        & $Mise @TestArgs
        if ($LASTEXITCODE -ne 0) { throw 'Cargo tests failed.' }
        & $Mise @('exec','--','cargo','clippy','--locked','--all-targets','--target-dir',$TargetDirectory,'--','-D','warnings')
        if ($LASTEXITCODE -ne 0) { throw 'Clippy failed.' }
        & $Mise exec -- cargo fmt --check
        if ($LASTEXITCODE -ne 0) { throw 'Rust formatting differs.' }
    }
    $ProfileDirectory = $Configuration.ToLowerInvariant()
    $Executable = Join-Path (Join-Path $ProjectRoot $TargetDirectory) "$ProfileDirectory/mdlite.exe"
    $SourcePaths = @('Cargo.toml','Cargo.lock','rust-toolchain.toml') +
        @(Get-ChildItem (Join-Path $ProjectRoot 'src') -File -Recurse | ForEach-Object {
            [IO.Path]::GetRelativePath($ProjectRoot, $_.FullName).Replace('\','/')
        })
    $SourceManifest = foreach ($Path in ($SourcePaths | Sort-Object)) {
        '{0}:{1}' -f $Path, (Get-FileHash (Join-Path $ProjectRoot $Path) -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $SourceHash = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData(
        [Text.Encoding]::UTF8.GetBytes(($SourceManifest -join "`n")))).ToLowerInvariant()
    $EvidenceDirectory = Join-Path $ProjectRoot $EvidenceDirectory
    New-Item -ItemType Directory -Path $EvidenceDirectory -Force | Out-Null
    @{
        configuration=$Configuration; rust_version=$RustVersion; build='PASS'
        source_sha256=$SourceHash; executable=$Executable
        executable_sha256=(Get-FileHash $Executable -Algorithm SHA256).Hash.ToLowerInvariant()
        test=$(if ($Verify) { 'PASS' } else { 'NOT_RUN' })
        clippy=$(if ($Verify) { 'PASS' } else { 'NOT_RUN' })
        fmt=$(if ($Verify) { 'PASS' } else { 'NOT_RUN' })
        utc=[DateTime]::UtcNow.ToString('o')
    } | ConvertTo-Json | Set-Content (Join-Path $EvidenceDirectory "build-$ProfileDirectory.json") -Encoding utf8
    if ($PrepareDemo) { & (Join-Path $PSScriptRoot 'New-RustDemoWorkspace.ps1') }
} finally { Pop-Location }
