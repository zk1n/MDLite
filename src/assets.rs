//! Local image decoding and collision-safe workspace insertion.
use std::{
    fs::{self, File},
    io::BufReader,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use image::{AnimationDecoder, ImageDecoder};

#[derive(Debug)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// Premultiplied BGRA8, suitable for a Direct2D bitmap.
    pub pixels: Vec<u8>,
}

/// A safety budget for a single decoded bitmap; it is not a document limit.
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Remote destinations are never resolved or downloaded.
pub fn local_image_path(document_dir: &Path, destination: &str) -> io::Result<PathBuf> {
    if destination.contains("://")
        || destination.starts_with("//")
        || destination.starts_with("\\\\")
        || destination.starts_with("data:")
        || destination.starts_with("file:")
    {
        return Err(invalid("ローカル画像のみ表示できます"));
    }
    let path = Path::new(destination);
    if path.is_absolute() {
        return Err(invalid("画像には文書からの相対パスを使用してください"));
    }
    Ok(document_dir.join(path))
}

pub fn decode(path: &Path) -> io::Result<DecodedImage> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "svg" {
        return decode_svg(&fs::read(path)?);
    }
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err(invalid("未対応の画像形式です"));
    }
    let mut reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let rgba = reader
        .decode()
        .map_err(|e| invalid(e.to_string()))?
        .into_rgba8();
    let (width, height) = rgba.dimensions();
    let pixels = premultiplied_bgra(rgba.into_raw());
    Ok(DecodedImage {
        width,
        height,
        pixels,
    })
}

fn premultiplied_bgra(mut pixels: Vec<u8>) -> Vec<u8> {
    for p in pixels.as_chunks_mut::<4>().0 {
        let a = u16::from(p[3]);
        let r = (u16::from(p[0]) * a + 127) / 255;
        let g = (u16::from(p[1]) * a + 127) / 255;
        let b = (u16::from(p[2]) * a + 127) / 255;
        p[0] = b as u8;
        p[1] = g as u8;
        p[2] = r as u8;
    }
    pixels
}

#[derive(Clone, Copy)]
enum AnimatedFormat {
    Gif,
    WebP,
}

fn open_frame_stream(
    path: &Path,
    format: AnimatedFormat,
) -> io::Result<(image::Frames<'static>, Option<bool>)> {
    match format {
        AnimatedFormat::Gif => {
            let mut decoder =
                image::codecs::gif::GifDecoder::new(BufReader::new(File::open(path)?))
                    .map_err(|e| invalid(e.to_string()))?;
            let mut limits = image::Limits::default();
            limits.max_alloc = Some(MAX_DECODED_BYTES);
            decoder
                .set_limits(limits)
                .map_err(|e| invalid(e.to_string()))?;
            check_decoded_size(decoder.dimensions().0, decoder.dimensions().1)?;
            Ok((decoder.into_frames(), None))
        }
        AnimatedFormat::WebP => {
            let decoder = image::codecs::webp::WebPDecoder::new(BufReader::new(File::open(path)?))
                .map_err(|e| invalid(e.to_string()))?;
            check_decoded_size(decoder.dimensions().0, decoder.dimensions().1)?;
            let animated = decoder.has_animation();
            Ok((decoder.into_frames(), Some(animated)))
        }
    }
}

fn check_decoded_size(width: u32, height: u32) -> io::Result<()> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0 || height == 0 || bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES) {
        return Err(invalid("画像の展開サイズが安全な表示予算を超えています"));
    }
    Ok(())
}

fn decode_animation_frame(frame: image::Frame) -> io::Result<(DecodedImage, Duration)> {
    let delay = Duration::from(frame.delay()).max(Duration::from_millis(10));
    let rgba = frame.into_buffer();
    let (width, height) = rgba.dimensions();
    check_decoded_size(width, height)?;
    Ok((
        DecodedImage {
            width,
            height,
            pixels: premultiplied_bgra(rgba.into_raw()),
        },
        delay,
    ))
}

/// A streaming GIF or animated WebP decoder controlled by the owning UI thread.
///
/// It holds the current decoded frame and the decoder's disposal canvas. It has no
/// worker thread or accumulated frame cache; dropping it closes the backing file.
pub struct AnimatedImage {
    path: PathBuf,
    format: AnimatedFormat,
    current: DecodedImage,
    frame_delay: Duration,
    frames: Option<image::Frames<'static>>,
    visible: bool,
    remaining: Duration,
    next_due: Option<Instant>,
}

impl AnimatedImage {
    /// Opens GIF/WebP and decodes its first frame. Static files are returned with
    /// `is_animated() == false`; their first frame can still be used for display.
    pub fn open(path: &Path) -> io::Result<Self> {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let format = match extension.as_str() {
            "gif" => AnimatedFormat::Gif,
            "webp" => AnimatedFormat::WebP,
            _ => return Err(invalid("アニメーション画像はGIFまたはWebPに限られます")),
        };

        let (mut frames, animation_metadata) = open_frame_stream(path, format)?;
        if animation_metadata == Some(false) {
            return Ok(Self {
                path: path.to_owned(),
                format,
                current: decode(path)?,
                frame_delay: Duration::ZERO,
                frames: None,
                visible: false,
                remaining: Duration::ZERO,
                next_due: None,
            });
        }
        let first = frames
            .next()
            .transpose()
            .map_err(|e| invalid(e.to_string()))?
            .ok_or_else(|| invalid("画像に表示可能なフレームがありません"))?;
        let (current, frame_delay) = decode_animation_frame(first)?;
        let animated = match animation_metadata {
            Some(animated) => animated,
            None => frames
                .next()
                .transpose()
                .map_err(|e| invalid(e.to_string()))?
                .is_some(),
        };

        let frames = if animated && animation_metadata.is_none() {
            // GIF has no frame-count query. Probe one frame, then reopen the stream
            // so the first frame is current and the next frame is decoded on demand.
            drop(frames);
            let (mut restarted, _) = open_frame_stream(path, format)?;
            restarted
                .next()
                .transpose()
                .map_err(|e| invalid(e.to_string()))?
                .ok_or_else(|| invalid("画像に表示可能なフレームがありません"))?;
            Some(restarted)
        } else if animated {
            Some(frames)
        } else {
            None
        };

        Ok(Self {
            path: path.to_owned(),
            format,
            current,
            frame_delay,
            frames,
            visible: false,
            remaining: frame_delay,
            next_due: None,
        })
    }

    pub fn current(&self) -> &DecodedImage {
        &self.current
    }

    pub fn is_animated(&self) -> bool {
        self.frames.is_some()
    }

    pub fn next_due(&self) -> Option<Instant> {
        self.next_due
    }

    pub fn frame_delay(&self) -> Duration {
        self.frame_delay
    }

    /// Pauses offscreen playback and resumes from its remaining delay.
    pub fn set_visible(&mut self, visible: bool, now: Instant) {
        if !self.is_animated() || self.visible == visible {
            return;
        }
        self.visible = visible;
        if visible {
            self.next_due = Some(now.checked_add(self.remaining).unwrap_or(now));
        } else {
            self.remaining = self
                .next_due
                .map(|due| due.saturating_duration_since(now))
                .unwrap_or(self.frame_delay);
            self.next_due = None;
        }
    }

    /// Advances at most one frame when visible and due; late calls never catch up.
    pub fn advance_if_due(&mut self, now: Instant) -> io::Result<bool> {
        let Some(due) = self.next_due else {
            return Ok(false);
        };
        if now < due {
            return Ok(false);
        }

        let next = self.frames.as_mut().and_then(|frames| frames.next());
        let frame = match next {
            Some(Ok(frame)) => frame,
            Some(Err(error)) => {
                self.visible = false;
                self.next_due = None;
                return Err(invalid(error.to_string()));
            }
            None => {
                let (mut restarted, _) = match open_frame_stream(&self.path, self.format) {
                    Ok(restarted) => restarted,
                    Err(error) => {
                        self.visible = false;
                        self.next_due = None;
                        return Err(error);
                    }
                };
                let first = restarted.next().transpose();
                match first {
                    Ok(Some(frame)) => {
                        self.frames = Some(restarted);
                        frame
                    }
                    Ok(None) => {
                        self.visible = false;
                        self.next_due = None;
                        return Err(invalid("画像に表示可能なフレームがありません"));
                    }
                    Err(error) => {
                        self.visible = false;
                        self.next_due = None;
                        return Err(invalid(error.to_string()));
                    }
                }
            }
        };

        let (current, frame_delay) = match decode_animation_frame(frame) {
            Ok(frame) => frame,
            Err(error) => {
                self.visible = false;
                self.next_due = None;
                return Err(error);
            }
        };
        self.current = current;
        self.frame_delay = frame_delay;
        self.remaining = frame_delay;
        self.next_due = Some(now.checked_add(frame_delay).unwrap_or(now));
        Ok(true)
    }
}

pub fn decode_svg(bytes: &[u8]) -> io::Result<DecodedImage> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("SVGはUTF-8で指定してください"))?;
    let xml = roxmltree::Document::parse(text).map_err(|e| invalid(e.to_string()))?;
    for node in xml.descendants().filter(|n| n.is_element()) {
        if matches!(
            node.tag_name().name(),
            "script" | "foreignObject" | "image" | "iframe" | "audio" | "video"
        ) {
            return Err(invalid("SVGのscript、埋込みHTML、外部画像は表示できません"));
        }
        for attr in node.attributes() {
            let name = attr.name().to_ascii_lowercase();
            let value = attr.value().trim();
            if name.starts_with("on")
                || (name == "href" && !value.starts_with('#'))
                || (name == "base")
                || value.contains("://")
                || value.to_ascii_lowercase().contains("@import")
            {
                return Err(invalid("SVGの外部参照や実行要素は許可されていません"));
            }
        }
        if node.tag_name().name() == "style" {
            let style = node.text().unwrap_or_default().to_ascii_lowercase();
            if style.contains("@import") || style.contains("@font-face") || style.contains("://") {
                return Err(invalid("SVGの外部スタイルやフォントは許可されていません"));
            }
        }
    }
    let has_text = xml
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "text");
    let mut options = resvg::usvg::Options {
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    if has_text {
        options.font_family = "Yu Gothic UI".to_owned();
        options.fontdb_mut().load_system_fonts();
    }
    let tree = resvg::usvg::Tree::from_data(bytes, &options).map_err(|e| invalid(e.to_string()))?;
    let size = tree.size().to_int_size();
    let width = size.width();
    let height = size.height();
    check_decoded_size(width, height)
        .map_err(|_| invalid("SVGの展開サイズが安全な表示予算を超えています"))?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)
        .ok_or_else(|| invalid("SVG表示領域を確保できません"))?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    let mut pixels = pixmap.take();
    for p in pixels.as_chunks_mut::<4>().0 {
        p.swap(0, 2);
    }
    Ok(DecodedImage {
        width,
        height,
        pixels,
    })
}

#[derive(Debug)]
pub struct ImportedImage {
    pub path: PathBuf,
    pub markdown: String,
}

/// Validate before allocating a destination. Successful creation precedes source insertion.
pub fn import_file(
    workspace: &Path,
    document_dir: &Path,
    source_path: &Path,
) -> io::Result<ImportedImage> {
    let _validated = decode(source_path)?;
    let bytes = fs::read(source_path)?;
    let extension = source_path
        .extension()
        .and_then(|v| v.to_str())
        .ok_or_else(|| invalid("画像拡張子が必要です"))?
        .to_ascii_lowercase();
    import_bytes(workspace, document_dir, &extension, &bytes)
}

pub fn import_png(
    workspace: &Path,
    document_dir: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> io::Result<ImportedImage> {
    use image::ImageEncoder;
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| invalid(e.to_string()))?;
    import_bytes(workspace, document_dir, "png", &bytes)
}

fn import_bytes(
    workspace: &Path,
    document_dir: &Path,
    extension: &str,
    bytes: &[u8],
) -> io::Result<ImportedImage> {
    let workspace = workspace.canonicalize()?;
    let document_dir = document_dir.canonicalize()?;
    if !document_dir.starts_with(&workspace) {
        return Err(invalid("文書はWorkspace内に保存してください"));
    }
    let directory = workspace.join("assets");
    fs::create_dir_all(&directory)?;
    let directory = directory.canonicalize()?;
    if !directory.starts_with(&workspace) {
        return Err(invalid("画像保存先がWorkspace外へ接続されています"));
    }
    for number in 1..=100_000 {
        let path = directory.join(format!("image-{number:05}.{extension}"));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        let mut relative = PathBuf::new();
        let remainder = document_dir
            .strip_prefix(&workspace)
            .map_err(|_| invalid("文書位置が変更されました"))?;
        for _ in remainder.components() {
            relative.push("..");
        }
        relative.push("assets");
        relative.push(path.file_name().unwrap());
        let link = relative.to_string_lossy().replace('\\', "/");
        return Ok(ImportedImage {
            path,
            markdown: format!("![画像]({link})"),
        });
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "画像名を確保できません",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageEncoder;

    fn test_image_path(extension: &str) -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "mdlite-animation-{}-{nonce}.{extension}",
            std::process::id()
        ))
    }

    fn gif_frame(color: [u8; 4], delay_ms: u32) -> image::Frame {
        image::Frame::from_parts(
            image::RgbaImage::from_pixel(1, 1, image::Rgba(color)),
            0,
            0,
            image::Delay::from_numer_denom_ms(delay_ms, 1),
        )
    }

    fn append_webp_chunk(output: &mut Vec<u8>, kind: &[u8; 4], payload: &[u8]) {
        output.extend_from_slice(kind);
        output.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        output.extend_from_slice(payload);
        if !payload.len().is_multiple_of(2) {
            output.push(0);
        }
    }

    fn webp_lossless_chunk(color: [u8; 4]) -> Vec<u8> {
        let mut encoded = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
            .write_image(&color, 1, 1, image::ExtendedColorType::Rgba8)
            .unwrap();
        assert_eq!(&encoded[12..16], b"VP8L");
        let payload_len = u32::from_le_bytes(encoded[16..20].try_into().unwrap()) as usize;
        encoded[12..20 + payload_len + (payload_len % 2)].to_vec()
    }

    fn two_frame_webp() -> Vec<u8> {
        // Compose a minimal RIFF animation around real, lossless one-pixel VP8L frames.
        let mut chunks = Vec::new();
        let mut extended_header = vec![0x02, 0, 0, 0]; // animation feature flag
        extended_header.extend_from_slice(&[0; 6]); // one-pixel canvas
        append_webp_chunk(&mut chunks, b"VP8X", &extended_header);
        append_webp_chunk(&mut chunks, b"ANIM", &[0; 6]); // transparent background, infinite loop
        for color in [[255, 0, 0, 255], [0, 0, 255, 255]] {
            let mut frame = vec![0; 12]; // origin and one-pixel frame size
            frame.extend_from_slice(&[25, 0, 0, 0x02]); // 25ms, no alpha blending
            frame.extend_from_slice(&webp_lossless_chunk(color));
            append_webp_chunk(&mut chunks, b"ANMF", &frame);
        }
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(&chunks);
        let mut encoded = b"RIFF".to_vec();
        encoded.extend_from_slice(&(body.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&body);
        encoded
    }

    #[test]
    fn animated_gif_streams_pauses_and_loops_without_catching_up() {
        let path = test_image_path("gif");
        let mut bytes = Vec::new();
        let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
        encoder
            .set_repeat(image::codecs::gif::Repeat::Infinite)
            .unwrap();
        encoder
            .encode_frames([
                gif_frame([255, 0, 0, 255], 20),
                gif_frame([0, 0, 255, 255], 30),
            ])
            .unwrap();
        drop(encoder);
        fs::write(&path, bytes).unwrap();

        let mut animation = AnimatedImage::open(&path).unwrap();
        assert!(animation.is_animated());
        assert_eq!(
            (animation.current().width, animation.current().height),
            (1, 1)
        );
        assert_eq!(&animation.current().pixels, &[0, 0, 255, 255]);
        assert_eq!(animation.frame_delay(), Duration::from_millis(20));
        assert_eq!(animation.next_due(), None);

        let start = Instant::now();
        animation.set_visible(true, start);
        assert_eq!(
            animation.next_due(),
            Some(start + Duration::from_millis(20))
        );
        assert!(
            !animation
                .advance_if_due(start + Duration::from_millis(19))
                .unwrap()
        );
        assert!(
            animation
                .advance_if_due(start + Duration::from_millis(20))
                .unwrap()
        );
        assert_eq!(&animation.current().pixels, &[255, 0, 0, 255]);
        assert_eq!(animation.frame_delay(), Duration::from_millis(30));

        animation.set_visible(false, start + Duration::from_millis(22));
        assert_eq!(animation.next_due(), None);
        let resumed = start + Duration::from_secs(60);
        animation.set_visible(true, resumed);
        let due = resumed + Duration::from_millis(28);
        assert_eq!(animation.next_due(), Some(due));
        assert!(
            !animation
                .advance_if_due(due - Duration::from_millis(1))
                .unwrap()
        );
        assert!(animation.advance_if_due(due).unwrap());
        assert_eq!(&animation.current().pixels, &[0, 0, 255, 255]);
        assert_eq!(animation.next_due(), Some(due + Duration::from_millis(20)));
        assert!(!animation.advance_if_due(due).unwrap());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn animated_webp_streams_lossless_frames() {
        let path = test_image_path("webp");
        fs::write(&path, two_frame_webp()).unwrap();

        let mut animation = AnimatedImage::open(&path).unwrap();
        assert!(animation.is_animated());
        assert_eq!(
            (animation.current().width, animation.current().height),
            (1, 1)
        );
        assert_eq!(&animation.current().pixels, &[0, 0, 255, 255]);
        assert_eq!(animation.frame_delay(), Duration::from_millis(25));
        let start = Instant::now();
        animation.set_visible(true, start);
        assert!(
            animation
                .advance_if_due(start + Duration::from_millis(25))
                .unwrap()
        );
        assert_eq!(&animation.current().pixels, &[255, 0, 0, 255]);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn static_webp_has_a_frame_but_no_animation_timer() {
        let path = test_image_path("webp");
        let bytes = webp_lossless_chunk([0, 255, 0, 255]);
        assert!(bytes.len() > 8);
        let mut encoded = b"RIFF".to_vec();
        let mut body = b"WEBP".to_vec();
        body.extend_from_slice(&bytes);
        encoded.extend_from_slice(&(body.len() as u32).to_le_bytes());
        encoded.extend_from_slice(&body);
        fs::write(&path, encoded).unwrap();

        let mut image = AnimatedImage::open(&path).unwrap();
        assert!(!image.is_animated());
        assert_eq!(&image.current().pixels, &[0, 255, 0, 255]);
        image.set_visible(true, Instant::now());
        assert_eq!(image.next_due(), None);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn animated_frames_follow_the_existing_bitmap_budget() {
        assert!(check_decoded_size(8191, 8192).is_ok());
        assert!(check_decoded_size(8192, 8192).is_ok());
        assert!(check_decoded_size(8193, 8192).is_err());
    }

    #[test]
    fn svg_system_text_renders_without_external_fonts() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="32"><rect width="96" height="32" fill="#fff"/><text x="2" y="24" font-size="20" fill="#000">日本語 MDLite</text></svg>"##;
        let image = decode_svg(svg.as_bytes()).unwrap();
        assert_eq!((image.width, image.height), (96, 32));
        assert!(
            image
                .pixels
                .as_chunks::<4>()
                .0
                .iter()
                .any(|p| { p[3] > 0 && (p[0] < 255 || p[1] < 255 || p[2] < 255) })
        );

        let external_font = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><style>@font-face { font-family: x; src: url(https://example.test/font.woff); }</style></svg>"##;
        assert!(decode_svg(external_font).is_err());
    }

    #[test]
    fn svg_is_local_and_passive() {
        let safe = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><rect width="8" height="4" fill="#ff0000"/></svg>"##;
        let image = decode_svg(safe).unwrap();
        assert_eq!((image.width, image.height), (8, 4));
        assert_eq!(&image.pixels[..4], &[0, 0, 255, 255]);
        for dangerous in [
            "<script/>",
            "<foreignObject/>",
            "<image href='private.png'/>",
            "<rect onclick='run()'/>",
        ] {
            let data = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='4'>{dangerous}</svg>"
            );
            assert!(decode_svg(data.as_bytes()).is_err());
        }
        assert!(local_image_path(Path::new("."), "https://example.test/a.png").is_err());
    }
    #[test]
    fn insert_creates_before_link_and_avoids_collisions() {
        let root = std::env::temp_dir().join(format!("mdlite-image-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let a = import_png(&root, &root, 1, 1, &[255, 0, 0, 255]).unwrap();
        let b = import_png(&root, &root, 1, 1, &[0, 255, 0, 255]).unwrap();
        assert_ne!(a.path, b.path);
        assert!(a.path.is_file());
        assert!(a.markdown.contains("assets/"));
        assert_eq!(decode(&a.path).unwrap().pixels, vec![0, 0, 255, 255]);
        fs::remove_file(a.path).unwrap();
        fs::remove_file(b.path).unwrap();
        let _ = fs::remove_dir(root.join("assets"));
        let _ = fs::remove_dir(root);
    }
}
