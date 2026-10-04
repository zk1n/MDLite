#Requires -Version 5.1
[CmdletBinding()]
param([string]$OutputPath, [string]$HarnessPath)

# Offline only: extract helpers without executing the acceptance script's app or fixture setup.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$harness = $HarnessPath
if (-not $harness) { $harness = Join-Path $PSScriptRoot 'Invoke-SaveProfileSettingsAcceptance.ps1' }
$tokens = $null
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($harness, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$checks = [Collections.Generic.List[string]]::new()
function Assert-Check([bool]$Condition, [string]$Name) {
    if (-not $Condition) { throw "Offline harness check failed: $Name" }
    $checks.Add($Name)
}
Assert-Check ([BitConverter]::ToString([IO.File]::ReadAllBytes($harness)[0..2]) -ceq 'EF-BB-BF') 'UTF-8 BOM preserved for Windows PowerShell 5.1'
$caption = -join ([char[]]@(0x4F5C, 0x6210, 0x30D7, 0x30ED, 0x30D5, 0x30A1, 0x30A4, 0x30EB))
$literals = $ast.FindAll({ param($node) $node -is [Management.Automation.Language.StringConstantExpressionAst] }, $true)
Assert-Check (@($literals | Where-Object { $_.Value -ceq $caption }).Count -ge 2) 'Japanese confirmation and success captions decode exactly'
foreach ($name in @('Wait-Window', 'Save-As', 'Test-ProfileDefinition', 'Select-ProfileSuccessOkButton')) {
    $fn = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq $name }, $true)
    if (-not $fn) { throw "Missing helper $name" }
    Invoke-Expression $fn.Extent.Text
}

# These fake handles are data only. There are no P/Invokes or native window operations in this test.
Add-Type -TypeDefinition @'
using System;
public sealed class SaveProbeWindow {
    public IntPtr Handle = new IntPtr(1);
    public int Id;
    public string ClassName = "#32770", Text = "fixture";
    public uint ProcessId = 1, ThreadId = 1;
    public bool IsWindow = true;
}
public static class SaveProbeNative {
    public static int Reads, Clicks;
    public static bool NeverVisible;
    public static string Filename = "untitled.md";
    public static SaveProbeWindow[] TopLevel(uint pid) { Reads++; return new[] { new SaveProbeWindow() }; }
    public static bool IsWindowVisible(IntPtr window) { return !NeverVisible && Reads >= 2; }
    public static bool Post(IntPtr window, uint message, long wparam, long lparam) { return true; }
    public static string TextOf(IntPtr window) { return Filename; }
}
'@
function Wait-VisibleChild([IntPtr]$Parent, [int]$Id, [string]$ClassName, [int]$TimeoutMs = 5000) {
    if (-not [SaveProbeNative]::IsWindowVisible($Parent)) { throw 'Attempt to write a hidden dialog' }
    return [SaveProbeWindow]@{ Handle=[IntPtr]$Id; Id=$Id; ClassName=$ClassName }
}
function Send-Text([IntPtr]$Window, [uint32]$Message, [string]$Text) {
    if (-not $script:ignoreInput) { [SaveProbeNative]::Filename = $Text }
    return $script:setResult
}
function Send([IntPtr]$Window, [uint32]$Message, [long]$WParam, [long]$LParam) { [SaveProbeNative]::Clicks++; return 0 }
function Wait-WindowGone([Diagnostics.Process]$Process, [string]$ClassName, [int]$TimeoutMs) { }
$app = [pscustomobject]@{ main=[IntPtr]1; process=[Diagnostics.Process]::GetCurrentProcess() }
$script:ignoreInput = $false
$script:setResult = 1
$saved = Save-As $app 'empty-save.md'
Assert-Check ([SaveProbeNative]::Reads -ge 2 -and $saved.filename_edit.before -ceq 'untitled.md') 'Hidden dialog skipped before filename input'
Assert-Check ($saved.filename_edit.value -ceq 'empty-save.md' -and [SaveProbeNative]::Clicks -eq 1) 'Exact requested filename accepted before confirmation'
foreach ($failure in @('readback', 'rejected')) {
    [SaveProbeNative]::Clicks = 0
    [SaveProbeNative]::Filename = 'untitled.md'
    $script:ignoreInput = ($failure -eq 'readback')
    $script:setResult = if ($failure -eq 'rejected') { 0 } else { 1 }
    $message = ''
    try { [void](Save-As $app 'empty-save.md') } catch { $message = $_.Exception.Message }
    $expected = if ($failure -eq 'readback') { 'readback mismatch' } else { 'rejected WM_SETTEXT' }
    Assert-Check ($message.Contains($expected) -and [SaveProbeNative]::Clicks -eq 0) "Filename $failure blocks confirmation"
}
[SaveProbeNative]::NeverVisible = $true
$message = ''
try { [void](Wait-Window $app.process '#32770' 0 -VisibleOnly) } catch { $message = $_.Exception.Message }
Assert-Check ($message.Contains('configured_timeout_ms=0')) 'Hidden dialog remains bounded by timeout'

$values = [ordered]@{ id='qaProfile'; name='QA Profile'; directory='QAProfiles/{{date:yyyy}}'; filename='{{date:yyyyMMdd}}.md'; template='templates/daily.md'; collision='sequence' }
$definition = "[[profiles]]`n" + (($values.Keys | ForEach-Object { '{0} = "{1}"' -f $_, $values[$_] }) -join "`n") + "`n"
Assert-Check (Test-ProfileDefinition $definition $values) 'Exact persisted profile definition accepted'
foreach ($key in $values.Keys) {
    $wrong = $definition.Replace(('{0} = "{1}"' -f $key, $values[$key]), ('{0} = "wrong"' -f $key))
    Assert-Check (-not (Test-ProfileDefinition $wrong $values)) "Wrong persisted $key rejected"
}
Assert-Check (-not (Test-ProfileDefinition ($definition + $definition) $values)) 'Duplicate profile id rejected'
Assert-Check (-not (Test-ProfileDefinition ($definition + 'collision = "wrong"') $values)) 'Conflicting duplicate field rejected'
$otherProfile = "[[profiles]]`nid = `"daily`"`ncollision = `"sequence`"`n"
Assert-Check (-not (Test-ProfileDefinition ($definition.Replace('collision = "sequence"', 'collision = "open-existing"') + $otherProfile) $values)) 'Sibling sequence field cannot satisfy target profile'
# A single native MB_OK can expose actual IDCANCEL=2; select its verified semantics,
# never an arbitrary Cancel or any button merely sharing that numeric ID.
$okInventory = @([pscustomobject]@{ id=2; class='Button'; text='OK'; visible=$true; enabled=$true })
Assert-Check ((Select-ProfileSuccessOkButton $okInventory).id -eq 2) 'Observed sole semantic OK accepts actual native ID2'
$okId1 = @([pscustomobject]@{ id=1; class='Button'; text='OK'; visible=$true; enabled=$true })
Assert-Check ((Select-ProfileSuccessOkButton $okId1).id -eq 1) 'Semantic OK selection preserves actual native ID1'
foreach ($unready in @('visible', 'enabled')) {
    $button = [pscustomobject]@{ id=2; class='Button'; text='OK'; visible=$true; enabled=$true }
    $button.$unready = $false
    Assert-Check ($null -eq (Select-ProfileSuccessOkButton @($button))) "OK button $unready readiness required"
}
foreach ($unsafe in @('cancel', 'multiple')) {
    $inventory = if ($unsafe -eq 'cancel') {
        @([pscustomobject]@{ id=2; class='Button'; text='Cancel'; visible=$true; enabled=$true })
    } else { @($okInventory[0], $okId1[0]) }
    $rejected = $false
    try { $null = Select-ProfileSuccessOkButton $inventory } catch { $rejected = $true }
    Assert-Check $rejected "Unsafe $unsafe buttons cannot satisfy semantic OK"
}
$result = [pscustomobject]@{ schema='mdlite-save-harness-offline-v1'; powershell_version=$PSVersionTable.PSVersion.ToString(); harness_sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $harness).Hash.ToLowerInvariant(); evidence='Synthetic helper checks only; no product, HWND, physical input, clipboard, or IME operations'; checks=$checks.ToArray(); pass=$true }
$json = $result | ConvertTo-Json -Depth 3
if ($OutputPath) {
    [IO.Directory]::CreateDirectory((Split-Path -Parent ([IO.Path]::GetFullPath($OutputPath)))) | Out-Null
    [IO.File]::WriteAllText([IO.Path]::GetFullPath($OutputPath), $json, [Text.UTF8Encoding]::new($false))
}
$json
