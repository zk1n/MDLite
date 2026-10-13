[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$ProjectRoot = Split-Path -Parent $PSScriptRoot
$FixtureDirectory = Join-Path $ProjectRoot 'tests\fixtures\assets'
$Mise = (Get-Command mise -CommandType Application -ErrorAction Stop).Source
$ScratchParent = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
$ScratchDirectory = Join-Path $ScratchParent ('mdlite-rust-images-' + [guid]::NewGuid().ToString('N'))

$Generator = @'
use image::{
    codecs::gif::GifEncoder,
    Delay, DynamicImage, Frame, ImageBuffer, ImageFormat, Rgba, RgbaImage,
};
use std::{env, fs, io::{Cursor, Error, ErrorKind}, path::PathBuf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(env::args_os().nth(1).ok_or_else(|| Error::new(ErrorKind::InvalidInput, "expected output directory"))?);
    fs::create_dir_all(&output)?;
    let still = make_image(0, true);
    for (name, format) in [("raster.png", ImageFormat::Png), ("raster.jpg", ImageFormat::Jpeg), ("raster.gif", ImageFormat::Gif), ("raster.webp", ImageFormat::WebP)] {
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(still.clone()).write_to(&mut bytes, format)?;
        fs::write(output.join(name), bytes.into_inner())?;
    }
    let frames: Vec<_> = (0..3).map(|phase| make_image(phase, false)).collect();
    let mut gif = Vec::new();
    {
        let mut encoder = GifEncoder::new(&mut gif);
        encoder.encode_frames(frames.iter().cloned().map(|pixels| {
            Frame::from_parts(pixels, 0, 0, Delay::from_numer_denom_ms(100, 1))
        }))?;
    }
    fs::write(output.join("animated.gif"), gif)?;
    fs::write(output.join("animated.webp"), animated_webp(&frames, 100)?)?;
    Ok(())
}

fn make_image(phase: u8, translucent_corner: bool) -> RgbaImage {
    ImageBuffer::from_fn(32, 24, |x, y| {
        let checker = ((x / 4 + y / 3 + u32::from(phase)) % 2) as u8;
        let alpha = if translucent_corner && x < 5 && y < 5 { 96 } else { 255 };
        Rgba([
            28u8.wrapping_add((x as u8).wrapping_mul(5)).wrapping_add(phase * 47),
            48 + (y as u8).wrapping_mul(7),
            92 + checker * 72,
            alpha,
        ])
    })
}

fn append_chunk(output: &mut Vec<u8>, name: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(name);
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output.extend_from_slice(data);
    if data.len() % 2 != 0 { output.push(0); }
}

fn append_u24(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes()[..3]);
}

fn animated_webp(frames: &[RgbaImage], duration_ms: u32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let first = frames.first().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "animation needs a frame"))?;
    let (width, height) = first.dimensions();
    let mut chunks = Vec::new();
    let mut extended = vec![0x02, 0, 0, 0];
    append_u24(&mut extended, width - 1);
    append_u24(&mut extended, height - 1);
    append_chunk(&mut chunks, b"VP8X", &extended);
    append_chunk(&mut chunks, b"ANIM", &[0, 0, 0, 0, 0, 0]);

    for frame in frames {
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(frame.clone()).write_to(&mut encoded, ImageFormat::WebP)?;
        let bytes = encoded.into_inner();
        if bytes.get(8..12) != Some(&b"WEBP"[..]) { return Err(Error::new(ErrorKind::InvalidData, "static WebP encoder returned an invalid container").into()); }
        let mut image_data = Vec::new();
        let mut cursor = 12usize;
        while cursor + 8 <= bytes.len() {
            let size = u32::from_le_bytes(bytes[cursor + 4..cursor + 8].try_into()?) as usize;
            let end = cursor + 8 + size + (size % 2);
            let name = &bytes[cursor..cursor + 4];
            if end > bytes.len() || (name != b"VP8 " && name != b"VP8L" && name != b"ALPH") {
                return Err(Error::new(ErrorKind::InvalidData, "static WebP encoder returned unsupported frame chunks").into());
            }
            image_data.extend_from_slice(&bytes[cursor..end]);
            cursor = end;
        }
        let mut frame_data = Vec::new();
        append_u24(&mut frame_data, 0);
        append_u24(&mut frame_data, 0);
        append_u24(&mut frame_data, width - 1);
        append_u24(&mut frame_data, height - 1);
        append_u24(&mut frame_data, duration_ms);
        frame_data.push(0);
        frame_data.extend_from_slice(&image_data);
        append_chunk(&mut chunks, b"ANMF", &frame_data);
    }

    let mut riff = b"RIFF".to_vec();
    riff.extend_from_slice(&(4u32 + chunks.len() as u32).to_le_bytes());
    riff.extend_from_slice(b"WEBP");
    riff.extend_from_slice(&chunks);
    Ok(riff)
}
'@

$null = New-Item -ItemType Directory -Path $ScratchDirectory
try {
    $ScratchDirectory = [IO.Path]::GetFullPath($ScratchDirectory)
    if (-not $ScratchDirectory.StartsWith($ScratchParent, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Scratch path escaped the temp directory: $ScratchDirectory"
    }
    Copy-Item -LiteralPath (Join-Path $ProjectRoot 'Cargo.toml') -Destination $ScratchDirectory
    Copy-Item -LiteralPath (Join-Path $ProjectRoot 'Cargo.lock') -Destination $ScratchDirectory
    $null = New-Item -ItemType Directory -Path (Join-Path $ScratchDirectory 'src\bin') -Force
    [IO.File]::WriteAllText((Join-Path $ScratchDirectory 'src\lib.rs'), "pub fn fixture_generator_package() {}`n", [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText((Join-Path $ScratchDirectory 'src\bin\new_rust_image_fixtures.rs'), $Generator, [Text.UTF8Encoding]::new($false))

    $StagingDirectory = Join-Path $ScratchDirectory 'out'
    & $Mise exec -- cargo run --manifest-path (Join-Path $ScratchDirectory 'Cargo.toml') --bin new_rust_image_fixtures --locked --offline -- $StagingDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Offline raster fixture generation failed.' }

    $Names = @('raster.png', 'raster.jpg', 'raster.gif', 'raster.webp', 'animated.gif', 'animated.webp')
    $Conflicts = foreach ($Name in $Names) {
        $Existing = Join-Path $FixtureDirectory $Name
        $Generated = Join-Path $StagingDirectory $Name
        if (Test-Path -LiteralPath $Existing) {
            $ExistingHash = (Get-FileHash -LiteralPath $Existing -Algorithm SHA256).Hash
            $GeneratedHash = (Get-FileHash -LiteralPath $Generated -Algorithm SHA256).Hash
            if ($ExistingHash -ne $GeneratedHash) { $Name }
        }
    }
    if ($Conflicts) { throw "Refusing to overwrite changed fixtures: $($Conflicts -join ', ')" }

    $null = New-Item -ItemType Directory -Path $FixtureDirectory -Force
    foreach ($Name in $Names) {
        $Destination = Join-Path $FixtureDirectory $Name
        if (-not (Test-Path -LiteralPath $Destination)) {
            Copy-Item -LiteralPath (Join-Path $StagingDirectory $Name) -Destination $Destination
        }
        $Item = Get-Item -LiteralPath $Destination
        $Hash = (Get-FileHash -LiteralPath $Destination -Algorithm SHA256).Hash.ToLowerInvariant()
        [pscustomobject]@{ Name = $Name; Bytes = $Item.Length; Sha256 = $Hash }
    }
}
finally {
    $ResolvedScratch = [IO.Path]::GetFullPath($ScratchDirectory)
    if (-not $ResolvedScratch.StartsWith($ScratchParent, [StringComparison]::OrdinalIgnoreCase) -or
        $ResolvedScratch.TrimEnd('\') -eq $ScratchParent.TrimEnd('\')) {
        throw "Refusing to remove an unchecked scratch path: $ResolvedScratch"
    }
    Remove-Item -LiteralPath $ResolvedScratch -Recurse -Force
}
