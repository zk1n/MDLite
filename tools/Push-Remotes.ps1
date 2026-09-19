#Requires -Version 5.1
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    [string]$Branch = '',
    [string[]]$Remotes = @('origin', 'github'),
    [switch]$AllowMain
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
# Native exit status is checked explicitly. Do not print credential data.
if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
    $PSNativeCommandUseErrorActionPreference = $false
}
function Read-Git([string[]]$GitArgs) {
    $result = @(& git @GitArgs)
    if ($LASTEXITCODE -ne 0) { throw "git command failed: $($GitArgs[0])" }
    return $result
}
$root = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $root
try {
    $gitRoot = ((Read-Git @('rev-parse', '--show-toplevel')) -join '').Trim()
    $expectedRoot = [IO.Path]::GetFullPath($root).TrimEnd([char[]]'\/')
    $actualRoot = [IO.Path]::GetFullPath($gitRoot).TrimEnd([char[]]'\/')
    if (-not [string]::Equals($expectedRoot, $actualRoot, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The tools directory must be inside the intended repository root.'
    }
    if ([string]::IsNullOrWhiteSpace($Branch)) {
        $Branch = ((Read-Git @('symbolic-ref', '--quiet', '--short', 'HEAD')) -join '').Trim()
    }
    if ([string]::IsNullOrWhiteSpace($Branch) -or $Branch.StartsWith('-')) {
        throw 'Specify a local branch, not a detached HEAD or an option.'
    }
    $ref = "refs/heads/$Branch"
    $null = Read-Git @('check-ref-format', $ref)
    if ($Branch -eq 'main' -and -not $AllowMain) {
        throw 'main is release-only. Use -AllowMain only for an authorized release or bootstrap.'
    }
    $oid = ((Read-Git @('rev-parse', '--verify', $ref)) -join '').Trim()
    if ($oid -notmatch '^[0-9a-f]{40,64}$') { throw 'Invalid commit ID.' }
    if ($Remotes.Count -lt 1 -or @($Remotes | Select-Object -Unique).Count -ne $Remotes.Count) {
        throw 'Use unique remote names.'
    }
    $targets = @()
    foreach ($remote in $Remotes) {
        if ($remote -notmatch '^[A-Za-z0-9][A-Za-z0-9._-]*$') { throw 'Invalid remote name.' }
        $fetchUrls = @(Read-Git @('remote', 'get-url', '--all', $remote))
        $pushUrls = @(Read-Git @('remote', 'get-url', '--push', '--all', $remote))
        if ($fetchUrls.Count -ne 1 -or $pushUrls.Count -ne 1 -or $fetchUrls[0] -cne $pushUrls[0]) {
            throw "Use a separate remote with one matching fetch/push URL: $remote"
        }
        $mirror = @(& git config --bool --get "remote.$remote.mirror")
        $rc = $LASTEXITCODE
        if ($rc -ne 0 -and $rc -ne 1) { throw 'Unable to check mirror configuration.' }
        if (($mirror -join '') -eq 'true') { throw "Mirror push is not allowed: $remote" }
        $targets += [pscustomobject]@{ Name = $remote; Url = [string]$pushUrls[0] }
    }
    # The same immutable object is pushed even if a local branch moves mid-command.
    $refspec = '{0}:{1}' -f $oid, $ref
    $failed = @()
    foreach ($target in $targets) {
        if (-not $PSCmdlet.ShouldProcess($target.Name, "Push $Branch at $oid")) { continue }
        try {
            & git -c push.followTags=false push --porcelain $target.Name $refspec
            if ($LASTEXITCODE -ne 0) { throw 'push failed' }
            $lines = @(Read-Git @('ls-remote', '--exit-code', '--refs', $target.Name, $ref))
            $match = @($lines | Where-Object { ($_ -split '\s+', 2)[1] -ceq $ref })
            if ($match.Count -ne 1 -or (($match[0] -split '\s+', 2)[0]) -cne $oid) {
                throw 'remote verification failed'
            }
            Write-Host ("OK: {0} {1} {2}" -f $target.Name, $Branch, $oid)
        }
        catch {
            $failed += $target.Name
            Write-Warning ("FAILED: {0}; {1}" -f $target.Name, $_.Exception.Message)
        }
    }
    if ($failed.Count -gt 0) {
        throw ('Some remotes are not synchronized: ' + ($failed -join ', ') + '. No rollback was performed.')
    }
}
finally { Pop-Location }
