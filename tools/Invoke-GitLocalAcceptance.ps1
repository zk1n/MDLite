#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$OutputPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$runId = [DateTime]::UtcNow.ToString('yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$verificationRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'build\verification'))
$evidenceRoot = [IO.Path]::GetFullPath((Join-Path $verificationRoot 'prehuman-calendar-git'))
if (-not $OutputPath) { $OutputPath = Join-Path $evidenceRoot "git-local-acceptance-$runId.json" }

$gitCommand = Get-Command git.exe -ErrorAction SilentlyContinue
$gitExe = if ($gitCommand) { $gitCommand.Source } else { 'C:\Program Files\Git\cmd\git.exe' }
if (-not (Test-Path -LiteralPath $gitExe -PathType Leaf)) { throw "git.exe was not found: $gitExe" }

function Invoke-Git([string]$Root, [string[]]$Arguments) {
    $lines = @(& $gitExe -C $Root @Arguments 2>&1 | ForEach-Object { $_.ToString() })
    [pscustomobject]@{
        exit_code = [int]$LASTEXITCODE
        output = ($lines -join "`n").Trim()
    }
}

$outputDirectory = Split-Path -Parent $OutputPath
if ($outputDirectory) { New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null }
$probeRoot = Join-Path $evidenceRoot ("git-local-model-$runId")
$ownerMarker = Join-Path $probeRoot '.git-local-model-owner'
$checks = [ordered]@{}
$result = $null

try {
    New-Item -ItemType Directory -Path $probeRoot | Out-Null
    [IO.File]::WriteAllText($ownerMarker, $runId, [Text.UTF8Encoding]::new($false))
    $init = Invoke-Git $probeRoot @('init', '--initial-branch=main')
    if ($init.exit_code -ne 0) { throw "git init failed: $($init.output)" }
    foreach ($pair in @(
        @('user.name', 'MDLite acceptance'),
        @('user.email', 'mdlite-acceptance@example.invalid')
    )) {
        $configured = Invoke-Git $probeRoot @('config', '--local', $pair[0], $pair[1])
        if ($configured.exit_code -ne 0) { throw "git config failed: $($configured.output)" }
    }

    $tracked = Join-Path $probeRoot 'tracked.md'
    $untracked = Join-Path $probeRoot 'untracked.md'
    $other = Join-Path $probeRoot 'other.md'
    [IO.File]::WriteAllText($tracked, 'initial')
    $addInitial = Invoke-Git $probeRoot @('add', '--', 'tracked.md')
    $commitInitial = Invoke-Git $probeRoot @('commit', '-m', 'initial')
    if ($addInitial.exit_code -ne 0 -or $commitInitial.exit_code -ne 0) {
        throw "initial commit failed: $($addInitial.output) $($commitInitial.output)"
    }

    # The index contains staged-version while the worktree contains a later
    # unstaged edit and two untracked files.  This is the former `-- .` trap.
    [IO.File]::WriteAllText($tracked, 'staged-version')
    $addStaged = Invoke-Git $probeRoot @('add', '--', 'tracked.md')
    [IO.File]::WriteAllText($tracked, 'unstaged-version')
    [IO.File]::WriteAllText($untracked, 'untracked-version')
    [IO.File]::WriteAllText($other, 'other-version')
    if ($addStaged.exit_code -ne 0) { throw "staging failed: $($addStaged.output)" }

    $commit = Invoke-Git $probeRoot @('commit', '-m', 'index-only')
    $headTracked = Invoke-Git $probeRoot @('show', 'HEAD:tracked.md')
    $status = Invoke-Git $probeRoot @('status', '--short')
    $headFiles = Invoke-Git $probeRoot @('ls-tree', '--name-only', 'HEAD')
    $remotes = Invoke-Git $probeRoot @('remote')

    $source = Get-Content -Raw (Join-Path $repoRoot 'src\app\Application.cpp')
    $commitCase = [regex]::Match($source, 'case kGitCommit:(?<body>.*?)(?=case kGitBranchCreate:)', [Text.RegularExpressions.RegexOptions]::Singleline)
    $sourceIndexOnly = $commitCase.Success -and
        $commitCase.Groups['body'].Value -match 'arguments\.insert\(arguments\.end\(\), \{L"commit", L"-m", value\}\)' -and
        $commitCase.Groups['body'].Value -notmatch 'L"--", L"\."'

    $checks.git_init = $init.exit_code -eq 0
    $checks.no_remote_configured = $remotes.exit_code -eq 0 -and [string]::IsNullOrWhiteSpace($remotes.output)
    $checks.commit_exit_code_zero = $commit.exit_code -eq 0
    $checks.index_version_committed = $headTracked.exit_code -eq 0 -and $headTracked.output -eq 'staged-version'
    $checks.unstaged_version_not_committed = (Get-Content -Raw $tracked) -eq 'unstaged-version'
    $checks.untracked_not_committed = $headFiles.exit_code -eq 0 -and $headFiles.output -notmatch '(^|`n)untracked\.md($|`n)'
    $checks.other_untracked_not_committed = $headFiles.exit_code -eq 0 -and $headFiles.output -notmatch '(^|`n)other\.md($|`n)'
    $checks.worktree_keeps_unstaged_and_untracked = $status.exit_code -eq 0 -and
        $status.output -match 'tracked\.md' -and $status.output -match 'untracked\.md' -and $status.output -match 'other\.md'
    $checks.application_commit_is_index_only = $sourceIndexOnly
    $checks.local_commit_requires_no_remote = $checks.no_remote_configured -and $checks.commit_exit_code_zero
    $checks.all_checks_pass = [bool]($checks.Values | Where-Object { -not $_ } | Measure-Object).Count -eq 0

    $result = [ordered]@{
        schema = 1
        generated_at_utc = [DateTime]::UtcNow.ToString('o')
        git_executable = $gitExe
        git_version = (Invoke-Git $probeRoot @('--version')).output
        evidence_lane = 'MODEL_AND_DIRECT_GIT_FIXTURE_ONLY'
        product_gui_route = 'NOT_RUN; covered by Invoke-GitPanelProductAcceptance.ps1'
        probe = 'isolated repository beneath build/verification; no user Workspace or remote changed'
        checks = $checks
        status = if ($checks.all_checks_pass) { 'PASS' } else { 'FAIL' }
        caveat = 'This proves the local Git/index boundary and command shape; it does not replace Human GUI, hook, credential, or offline-network acceptance.'
    }
    $result | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath $OutputPath -Encoding UTF8
    if (-not $checks.all_checks_pass) { throw "Git local acceptance failed; see $OutputPath" }
}
finally {
    $canonicalProbeRoot = [IO.Path]::GetFullPath($probeRoot)
    $canonicalEvidenceRoot = [IO.Path]::GetFullPath($evidenceRoot).TrimEnd('\') + '\'
    if ((Test-Path -LiteralPath $ownerMarker -PathType Leaf) -and
        [IO.File]::ReadAllText($ownerMarker) -ceq $runId -and
        $canonicalProbeRoot.StartsWith($canonicalEvidenceRoot, [StringComparison]::OrdinalIgnoreCase)) {
        Remove-Item -LiteralPath $probeRoot -Recurse -Force
    }
}

Write-Output (Get-Content -Raw $OutputPath)
