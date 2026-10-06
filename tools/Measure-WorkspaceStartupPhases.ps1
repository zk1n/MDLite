#Requires -Version 5.1
[CmdletBinding()]
param([ValidateRange(1,10000)][int]$FileCount=10000)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$repoRoot=Split-Path -Parent $PSScriptRoot
$runId=[guid]::NewGuid().ToString('N')
$artifactRoot=Join-Path $repoRoot ('.local/workspace-startup-phases-'+$runId)
$fixtureRoot=Join-Path ([IO.Path]::GetTempPath()) ('MDLite-startup-phases-'+$runId)
[IO.Directory]::CreateDirectory($artifactRoot)|Out-Null
[IO.Directory]::CreateDirectory($fixtureRoot)|Out-Null
$token=[guid]::NewGuid().ToString('N')
[IO.File]::WriteAllText((Join-Path $fixtureRoot '.owned-startup-probe'),$token)
$source=Join-Path $PSScriptRoot 'WorkspaceStartupPhaseProbe.cpp'
$app=Join-Path $repoRoot 'src/app/Application.cpp'
$appHash=(Get-FileHash $app).Hash
$vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
$installation=(& $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath).Trim()
if(-not $installation){throw 'Installed MSVC toolchain was not found.'}
Import-Module (Join-Path $installation 'Common7/Tools/Microsoft.VisualStudio.DevShell.dll')
Enter-VsDevShell -VsInstallPath $installation -SkipAutomaticLocation -DevCmdArguments '-arch=x64 -host_arch=x64'|Out-Null
$exe=Join-Path $artifactRoot 'workspace-startup-phases.exe'
$obj=Join-Path $artifactRoot 'workspace-startup-phases.obj'
& cl.exe /nologo /O2 /std:c++20 /EHsc /W4 /utf-8 /MT (('/Fo'+$obj)) (('/Fe'+$exe)) $source /link shell32.lib ole32.lib
if($LASTEXITCODE -ne 0){throw 'Standalone console diagnostic compile failed.'}
$process=$null
try{
    $start=[Diagnostics.ProcessStartInfo]::new()
    $start.FileName=$exe;$start.Arguments=('"{0}" {1}' -f $fixtureRoot,$FileCount)
    $start.UseShellExecute=$false;$start.CreateNoWindow=$true;$start.RedirectStandardOutput=$true;$start.RedirectStandardError=$true
    $process=[Diagnostics.Process]::Start($start)
    $outputTask=$process.StandardOutput.ReadToEndAsync();$errorTask=$process.StandardError.ReadToEndAsync()
    if(-not $process.WaitForExit(60000)){$process.Kill();[void]$process.WaitForExit(5000);throw 'Owned console phase probe exceeded60s; no app/UI controls created.'}
    if($process.ExitCode -ne 0){throw 'Owned console phase probe failed; see retained private fixture.'}
    $result=$outputTask.Result|ConvertFrom-Json
    $result|Add-Member NoteProperty method 'One warm generated metadata/header fixture; exact tree comparator,64 icon queries/mode,8192-byte binary scan and file-creation metadata queries; no TreeView/control creation'
    $result|Add-Member NoteProperty source_reference_sha256 $appHash.ToLowerInvariant()
    $result|Add-Member NoteProperty source_reference_unchanged ((Get-FileHash $app).Hash -eq $appHash)
    $result|Add-Member NoteProperty probe_executable_sha256 (Get-FileHash $exe).Hash.ToLowerInvariant()
    $result|ConvertTo-Json -Depth 4|Set-Content (Join-Path $artifactRoot 'result.json') -Encoding UTF8
    Write-Output ('Evidence: .local/workspace-startup-phases-'+$runId+'/result.json')
    $result|ConvertTo-Json -Depth 4 -Compress
}finally{
    if($null -ne $process){$process.Dispose()}
    $absolute=[IO.Path]::GetFullPath($fixtureRoot)
    $expectedParent=[IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd([char[]]@('\','/'))
    if([IO.Path]::GetDirectoryName($absolute) -cne $expectedParent -or [IO.Path]::GetFileName($absolute) -cne ('MDLite-startup-phases-'+$runId) -or [IO.File]::ReadAllText((Join-Path $absolute '.owned-startup-probe')) -cne $token){throw 'Owned diagnostic fixture cleanup containment check failed.'}
    if(((Get-Item -LiteralPath $absolute).Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Owned diagnostic fixture became a reparse point; preserved.'}
    if(@(Get-ChildItem -Force -LiteralPath $absolute|Where-Object {$_.PSIsContainer -or ($_.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0}).Count){throw 'Unexpected directory/reparse entry in flat owned fixture; preserved.'}
    Remove-Item -Recurse -Force -LiteralPath $absolute
}
