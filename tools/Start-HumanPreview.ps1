#Requires -Version 5.1
[CmdletBinding()]
param(
    [switch]$PrepareOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$receiptPath = Join-Path $repoRoot 'build\verification\build-receipt-release.json'
$executablePath = Join-Path $repoRoot 'build\release\MDLite.exe'

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
        $sha256 = [BitConverter]::ToString(
            $hasher.ComputeHash($utf8.GetBytes([string]::Join("`n", [string[]]$manifest)))
        ).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $hasher.Dispose()
    }

    return [pscustomobject]@{ sha256 = $sha256; file_count = $paths.Count }
}

function Assert-CurrentReleaseBuild {
    if (-not (Test-Path -LiteralPath $receiptPath -PathType Leaf)) {
        throw "Current Release PASS receipt is missing: $receiptPath"
    }
    if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
        throw "Current Release executable is missing: $executablePath"
    }

    $receipt = Get-Content -LiteralPath $receiptPath -Raw | ConvertFrom-Json
    if ($receipt.schema -ne 'mdlite-build-receipt-v2' -or
        $receipt.preset -ne 'release' -or
        $receipt.build_status -ne 'PASS' -or
        $receipt.configure_exit_code -ne 0 -or
        $receipt.build_exit_code -ne 0 -or
        -not $receipt.test_requested -or
        $receipt.test_exit_code -ne 0 -or
        -not $receipt.dependency_prefix_verified -or
        -not $receipt.dependency_probe_header_recorded -or
        -not $receipt.product_header_dependencies_verified) {
        throw 'Release receipt does not record the required passing build, tests, and dependency checks.'
    }

    $expectedExecutable = [IO.Path]::GetFullPath($executablePath)
    $receiptExecutable = [IO.Path]::GetFullPath([string]$receipt.executable_path)
    if (-not [string]::Equals($receiptExecutable, $expectedExecutable, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Release receipt points to a different executable.'
    }

    $currentSource = Get-SourceFingerprint $repoRoot
    if ($currentSource.sha256 -ne [string]$receipt.source_sha256 -or
        $currentSource.file_count -ne [int]$receipt.source_file_count) {
        throw 'Release receipt is stale for the current source. A fresh Release build/test is required.'
    }

    $executableHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $executablePath).Hash.ToLowerInvariant()
    if ($executableHash -ne [string]$receipt.executable_sha256) {
        throw 'Release executable SHA-256 does not match the PASS receipt.'
    }

    return [pscustomobject]@{
        receipt = $receipt
        receipt_sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $receiptPath).Hash.ToLowerInvariant()
        source_sha256 = $currentSource.sha256
        executable_sha256 = $executableHash
    }
}

$build = Assert-CurrentReleaseBuild
$previewParent = Join-Path $repoRoot '.local\human-preview'
New-Item -ItemType Directory -Path $previewParent -Force | Out-Null

$previewId = [DateTime]::UtcNow.ToString("yyyyMMdd'T'HHmmss'Z'") + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$workspacePath = Join-Path $previewParent $previewId
$workspaceRelativePath = '.local/human-preview/' + $previewId
& git -C $repoRoot check-ignore --quiet --no-index -- $workspaceRelativePath
if ($LASTEXITCODE -ne 0) {
    throw "Preview workspace is not covered by Git ignore rules: $workspaceRelativePath"
}
if (Test-Path -LiteralPath $workspacePath) {
    throw "Refusing to use an existing preview workspace: $workspacePath"
}

$demoWorkspaceHelper = Join-Path $repoRoot 'tools\New-F5DemoWorkspace.ps1'
if (-not (Test-Path -LiteralPath $demoWorkspaceHelper -PathType Leaf)) {
    throw "Existing demo workspace helper is missing: $demoWorkspaceHelper"
}
$helperOutput = & $demoWorkspaceHelper -OutputRoot $workspacePath
if ($helperOutput -notcontains $workspacePath -or
    -not (Test-Path -LiteralPath (Join-Path $workspacePath 'README.md') -PathType Leaf) -or
    -not (Test-Path -LiteralPath (Join-Path $workspacePath '.mdlite') -PathType Container)) {
    throw 'Demo workspace helper did not create the expected fresh workspace.'
}

$previewDocuments = @{
    'HumanPreview.md' = @'
# MDLite ヒューマンプレビュー

このWorkspaceと文書は動作確認用の架空データです。編集と保存はこの一時Workspace内だけで行ってください。

## 試す操作

1. このMarkdownソースの表を編集して保存し、本文が保持されることを確認します。
2. Calendarパネルを開き、今日の日付、件数、詳細表示、Daily操作を確認します。補助資料は[Calendarサンプル](Calendar-Sample.md)です。
3. Gitパネルを開き、新規Workspaceの未信頼状態と利用できないGit操作を確認します。信頼状態を変えないでください。説明は[Git Trustメモ](Git-Trust-Notes.md)にあります。
4. ウィンドウの標準タイトルバー、Activity名とアイコン、タブ、ステータス表示を確認します。

| 項目 | サンプル値 | 確認内容 |
| --- | --- | --- |
| 文書 | 架空の読書メモ | ソースと表示の一致 |
| 日付 | 今日 | Calendarの日付と件数 |
| Git Trust | 未信頼のまま | 信頼を追加せず状態表示を確認 |
| リンク | Calendar / Git Trust | このWorkspace内の相対リンク |

README.mdも既存のF5デモヘルパーが作成した架空サンプルです。
'@
    'Calendar-Sample.md' = @'
# Calendar サンプル

Calendarパネルで今日を選び、文書件数、詳細の表示／非表示、Daily操作を確認します。
Daily操作で作成する文書やメタデータは、この一時Workspace内だけに保存してください。

| 時刻 | 予定 | 状態 |
| --- | --- | --- |
| 09:00 | 架空のメモを読む | 未着手 |
| 13:30 | 表の行を編集する | 確認用 |
'@
    'Git-Trust-Notes.md' = @'
# Git Trust の確認

このスクリプトは一意な新規Workspaceを作るため、通常は既存のTrust登録がなく、Gitパネルに未信頼状態が表示されます。

- 未信頼の説明とTrust操作が表示されることを観察します。
- このプレビューでは「信頼する」「信頼を解除」は実行しません。TrustはユーザーのAppDataに保存されます。
- Stage、Commit、Fetch、PushなどのGit操作も実行しません。
'@
}

foreach ($entry in $previewDocuments.GetEnumerator()) {
    $documentPath = Join-Path $workspacePath $entry.Key
    if (Test-Path -LiteralPath $documentPath) {
        throw "Refusing to overwrite an existing preview document: $documentPath"
    }
    [IO.File]::WriteAllText($documentPath, [string]$entry.Value, [Text.UTF8Encoding]::new($false))
}

$gitCommit = (& git -C $repoRoot rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($gitCommit)) {
    throw 'Could not read the repository commit for the local preview receipt.'
}
$markdownFiles = @(Get-ChildItem -LiteralPath $workspacePath -File -Filter '*.md' | Select-Object -ExpandProperty FullName)
[string[]]$markdownFiles = $markdownFiles
[Array]::Sort($markdownFiles, [StringComparer]::Ordinal)
$contentManifest = foreach ($path in $markdownFiles) {
    $relativePath = [IO.Path]::GetFileName($path).Replace('\', '/')
    '{0}:{1}' -f $relativePath, ((Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLowerInvariant())
}
$manifestHasher = [Security.Cryptography.SHA256]::Create()
try {
    $previewContentHash = [BitConverter]::ToString(
        $manifestHasher.ComputeHash([Text.UTF8Encoding]::new($false).GetBytes([string]::Join("`n", [string[]]$contentManifest)))
    ).Replace('-', '').ToLowerInvariant()
}
finally {
    $manifestHasher.Dispose()
}

$currentBuild = Assert-CurrentReleaseBuild
if ($currentBuild.receipt.build_run_id -ne $build.receipt.build_run_id -or
    $currentBuild.source_sha256 -ne $build.source_sha256 -or
    $currentBuild.executable_sha256 -ne $build.executable_sha256) {
    throw 'Release build identity changed while preparing the preview. Run the script again after the build is stable.'
}

$previewInfoPath = Join-Path $workspacePath 'PreviewInfo.json'
if (Test-Path -LiteralPath $previewInfoPath) {
    throw "Refusing to overwrite preview version information: $previewInfoPath"
}
$previewInfo = [pscustomobject]@{
    schema = 'mdlite-human-preview-v1'
    prepared_utc = [DateTime]::UtcNow.ToString('o')
    git_commit = $gitCommit
    git_working_tree_dirty = @(& git -C $repoRoot status --porcelain).Count -ne 0
    workspace_relative_path = $workspaceRelativePath
    launch_document_relative_path = ($workspaceRelativePath + '/HumanPreview.md')
    release_receipt_relative_path = 'build/verification/build-receipt-release.json'
    release_receipt_sha256 = $currentBuild.receipt_sha256
    build_run_id = $currentBuild.receipt.build_run_id
    source_sha256 = $currentBuild.source_sha256
    executable_relative_path = 'build/release/MDLite.exe'
    executable_sha256 = $currentBuild.executable_sha256
    preview_content_sha256 = $previewContentHash
}
[IO.File]::WriteAllText($previewInfoPath, ($previewInfo | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))

$result = [ordered]@{
    mode = if ($PrepareOnly) { 'prepared' } else { 'launching' }
    workspace = $workspacePath
    launch_document = (Join-Path $workspacePath 'HumanPreview.md')
    executable = $executablePath
    source_sha256 = $currentBuild.source_sha256
    executable_sha256 = $currentBuild.executable_sha256
    preview_content_sha256 = $previewContentHash
}

if (-not $PrepareOnly) {
    $finalBuild = Assert-CurrentReleaseBuild
    if ($finalBuild.receipt.build_run_id -ne $currentBuild.receipt.build_run_id -or
        $finalBuild.source_sha256 -ne $currentBuild.source_sha256 -or
        $finalBuild.executable_sha256 -ne $currentBuild.executable_sha256) {
        throw 'Release build identity changed before launch. The preview was prepared but not launched.'
    }

    $launchArgument = '"' + (Join-Path $workspacePath 'HumanPreview.md') + '"'
    $process = Start-Process -FilePath $executablePath -WorkingDirectory $repoRoot -ArgumentList $launchArgument -WindowStyle Normal -PassThru
    $result.mode = 'launched'
    $result.process_id = $process.Id
}

[pscustomobject]$result
