#Requires -Version 5.1
[CmdletBinding()]
param(
    [string]$OutputRoot = ''
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$sourceRoot = $PSScriptRoot
if (-not $OutputRoot) {
    $OutputRoot = Join-Path ([IO.Path]::GetTempPath()) ('mdlite-acceptance-' + [guid]::NewGuid().ToString('N'))
}
[IO.Directory]::CreateDirectory($OutputRoot) | Out-Null

foreach ($name in @('notes', 'assets')) {
    Copy-Item -LiteralPath (Join-Path $sourceRoot $name) -Destination $OutputRoot -Recurse
}
Copy-Item -LiteralPath (Join-Path $sourceRoot 'README.md') -Destination $OutputRoot

$fixtures = Join-Path $OutputRoot 'assets\local-fixtures'
[IO.Directory]::CreateDirectory($fixtures) | Out-Null
Add-Type -AssemblyName System.Drawing
$bitmap = [Drawing.Bitmap]::new(96, 54)
try {
    $graphics = [Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.Clear([Drawing.Color]::MidnightBlue)
        $graphics.FillEllipse([Drawing.Brushes]::Gold, 18, 8, 38, 38)
        $font = [Drawing.Font]::new('Segoe UI', 10)
        try { $graphics.DrawString('MDLite', $font, [Drawing.Brushes]::White, 56, 18) }
        finally { $font.Dispose() }
    } finally { $graphics.Dispose() }
    $bitmap.Save((Join-Path $fixtures 'fixture.png'), [Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Save((Join-Path $fixtures 'fixture.jpg'), [Drawing.Imaging.ImageFormat]::Jpeg)
} finally { $bitmap.Dispose() }

# Two complete 1x1 frames with a 100 ms delay. It is deliberately tiny and
# synthetic: visual acceptance checks timing/repaint without using a real note.
[IO.File]::WriteAllBytes((Join-Path $fixtures 'fixture-animated.gif'), [byte[]]@(
    0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,
    0x00,0x00,0x00,0xFF,0xFF,0xFF,0x21,0xFF,0x0B,0x4E,0x45,0x54,0x53,
    0x43,0x41,0x50,0x45,0x32,0x2E,0x30,0x03,0x01,0x00,0x00,0x00,
    0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,
    0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,
    0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,0x2C,0x00,0x00,0x00,
    0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x4C,0x01,0x00,0x3B))

# A one-pixel lossless WebP fixture, decoded by MDLite's pinned libwebp path.
[IO.File]::WriteAllBytes((Join-Path $fixtures 'fixture.webp'), [byte[]]@(
    0x52,0x49,0x46,0x46,0x1A,0x00,0x00,0x00,0x57,0x45,0x42,0x50,
    0x56,0x50,0x38,0x4C,0x0E,0x00,0x00,0x00,0x2F,0x00,0x00,0x00,
    0x10,0x07,0x10,0x11,0x11,0x88,0x88,0xFE,0x07,0x00))

# Two lossless 2x1 frames encoded by the pinned libwebp 1.6.0 fixture builder:
# red/green for 100 ms, then blue/white for 250 ms.
[IO.File]::WriteAllBytes((Join-Path $fixtures 'fixture-animated.webp'),
    [Convert]::FromBase64String(
        'UklGRooAAABXRUJQVlA4WAoAAAACAAAAAQAAAAAAQU5JTQYAAAD/////AABBTk1GKgAAAAAAAAAAAAEAAAAAAGQAAAJWUDhMEQAAAC8BAAAAD7D/8x/zHxUyov8BAEFOTUYsAAAAAAAAAAAAAQAAAAAA+gAAAFZQOEwTAAAALwEAAAAPMP/zP//zHzyoQET/AwA='))

$imageNote = @'
# 画像

![PNG](../assets/local-fixtures/fixture.png)

![JPEG](../assets/local-fixtures/fixture.jpg)

![GIF animation](../assets/local-fixtures/fixture-animated.gif)

![WebP](../assets/local-fixtures/fixture.webp)

![WebP animation](../assets/local-fixtures/fixture-animated.webp)

![safe SVG](../assets/safe-gradient.svg)
'@
[IO.File]::WriteAllText((Join-Path $OutputRoot 'notes\画像確認.md'), $imageNote,
    [Text.UTF8Encoding]::new($false))

$manifest = Get-ChildItem -LiteralPath $OutputRoot -File -Recurse | ForEach-Object {
    [pscustomobject]@{
        relative_path = $_.FullName.Substring($OutputRoot.Length).TrimStart('\')
        bytes = $_.Length
        sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $_.FullName).Hash.ToLowerInvariant()
    }
}
$manifest | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (
    Join-Path $OutputRoot 'FIXTURE-MANIFEST.json') -Encoding utf8

Write-Output $OutputRoot
