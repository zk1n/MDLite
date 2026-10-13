//! Local image decoding and collision-safe workspace insertion.
use std::{
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, Take, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
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

/// Encoded raster files are read through a capped reader before decoder allocation.
const MAX_IMAGE_INPUT_BYTES: u64 = 512 * 1024 * 1024;
/// SVG must be held as UTF-8 while roxmltree and usvg parse it.
const MAX_SVG_INPUT_BYTES: u64 = 8 * 1024 * 1024;
/// A safety budget for one expanded image; it is not a document or cache limit.
const MAX_DECODED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 16_384;
const MAX_SVG_NODES: u32 = 16_384;
const MAX_SVG_DEPTH: usize = 256;
const MAX_SVG_USE_ELEMENTS: usize = 128;

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
        return decode_svg(&read_svg_input(path)?);
    }
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err(invalid("未対応の画像形式です"));
    }
    decode_raster_reader(image::ImageReader::new(bounded_file_reader(path)?))
}

/// Return dimensions only after checking the same input, decoder-output, and expanded-pixel
/// limits used by `decode`. This parses SVG structure but does not render pixels or load fonts.
pub fn image_dimensions(path: &Path) -> io::Result<(u32, u32)> {
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if extension == "svg" {
        let bytes = read_svg_input(path)?;
        let tree = parse_svg_tree(&bytes, false)?;
        return svg_dimensions(&tree);
    }
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp") {
        return Err(invalid("未対応の画像形式です"));
    }
    let reader = image::ImageReader::new(bounded_file_reader(path)?);
    let decoder = checked_raster_decoder(reader)?;
    Ok(decoder.dimensions())
}

/// Checked byte estimate for one RGBA8/BGRA8 output frame or Direct2D bitmap.
pub fn decoded_bytes(width: u32, height: u32) -> io::Result<u64> {
    check_decoded_size(width, height)
}

fn decoder_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    limits
}

fn bounded_file_reader(path: &Path) -> io::Result<BufReader<Take<File>>> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_IMAGE_INPUT_BYTES {
        return Err(invalid(
            "画像ファイルが入力サイズの安全な上限を超えています",
        ));
    }
    let read_limit = MAX_IMAGE_INPUT_BYTES
        .checked_add(1)
        .ok_or_else(|| invalid("画像入力サイズを確認できません"))?;
    Ok(BufReader::new(file.take(read_limit)))
}

fn read_svg_input(path: &Path) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let file_len = file.metadata()?.len();
    if file_len > MAX_SVG_INPUT_BYTES {
        return Err(invalid(
            "画像ファイルが入力サイズの安全な上限を超えています",
        ));
    }
    let initial_capacity =
        usize::try_from(file_len).map_err(|_| invalid("画像入力サイズを確認できません"))?;
    let read_limit = MAX_SVG_INPUT_BYTES
        .checked_add(1)
        .ok_or_else(|| invalid("画像入力サイズを確認できません"))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(initial_capacity)
        .map_err(|e| io::Error::other(e.to_string()))?;
    file.take(read_limit).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SVG_INPUT_BYTES {
        return Err(invalid(
            "画像ファイルが入力サイズの安全な上限を超えています",
        ));
    }
    Ok(bytes)
}

fn checked_raster_decoder<R: BufRead + Seek + 'static>(
    reader: image::ImageReader<R>,
) -> io::Result<impl ImageDecoder> {
    let mut reader = reader
        .with_guessed_format()
        .map_err(|e| invalid(e.to_string()))?;
    if !matches!(
        reader.format(),
        Some(
            image::ImageFormat::Png
                | image::ImageFormat::Jpeg
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
        )
    ) {
        return Err(invalid("PNG、JPEG、GIF、WebP以外の画像は表示できません"));
    }
    reader.limits(decoder_limits());
    let decoder = reader.into_decoder().map_err(|e| invalid(e.to_string()))?;
    check_decoder_size(&decoder)?;
    Ok(decoder)
}

fn check_decoder_size(decoder: &impl ImageDecoder) -> io::Result<u64> {
    let (width, height) = decoder.dimensions();
    let output_bytes = check_decoded_size(width, height)?;
    let raw_bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(u64::from(decoder.color_type().bytes_per_pixel())))
        .ok_or_else(|| invalid("画像のデコードサイズを確認できません"))?;
    if raw_bytes > MAX_DECODED_BYTES {
        return Err(invalid(
            "画像のデコード用バッファが安全な表示予算を超えています",
        ));
    }
    Ok(output_bytes)
}

fn decode_raster_reader<R: BufRead + Seek + 'static>(
    reader: image::ImageReader<R>,
) -> io::Result<DecodedImage> {
    let decoder = checked_raster_decoder(reader)?;
    decode_image_decoder(decoder)
}

fn decode_image_decoder(decoder: impl ImageDecoder) -> io::Result<DecodedImage> {
    let (width, height) = decoder.dimensions();
    let rgba = image::DynamicImage::from_decoder(decoder)
        .map_err(|e| invalid(e.to_string()))?
        .into_rgba8();
    debug_assert_eq!(rgba.dimensions(), (width, height));
    Ok(DecodedImage {
        width,
        height,
        pixels: premultiplied_bgra(rgba.into_raw()),
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

type WebPReader = BufReader<Take<File>>;

fn open_webp_decoder(path: &Path) -> io::Result<image::codecs::webp::WebPDecoder<WebPReader>> {
    let mut decoder = image::codecs::webp::WebPDecoder::new(bounded_file_reader(path)?)
        .map_err(|e| invalid(e.to_string()))?;
    check_decoder_size(&decoder)?;
    decoder
        .set_limits(decoder_limits())
        .map_err(|e| invalid(e.to_string()))?;
    Ok(decoder)
}

fn open_frame_stream(
    path: &Path,
    format: AnimatedFormat,
) -> io::Result<(image::Frames<'static>, Option<bool>)> {
    match format {
        AnimatedFormat::Gif => {
            let mut decoder = image::codecs::gif::GifDecoder::new(bounded_file_reader(path)?)
                .map_err(|e| invalid(e.to_string()))?;
            check_decoded_size(decoder.dimensions().0, decoder.dimensions().1)?;
            decoder
                .set_limits(decoder_limits())
                .map_err(|e| invalid(e.to_string()))?;
            Ok((decoder.into_frames(), None))
        }
        AnimatedFormat::WebP => {
            let decoder = open_webp_decoder(path)?;
            let animated = decoder.has_animation();
            Ok((decoder.into_frames(), Some(animated)))
        }
    }
}

fn check_decoded_size(width: u32, height: u32) -> io::Result<u64> {
    let bytes = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0
        || height == 0
        || width > MAX_IMAGE_DIMENSION
        || height > MAX_IMAGE_DIMENSION
        || bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
    {
        return Err(invalid("画像の展開サイズが安全な表示予算を超えています"));
    }
    Ok(bytes.unwrap())
}

fn decode_animation_frame(frame: image::Frame) -> io::Result<(DecodedImage, Duration)> {
    let delay = Duration::from(frame.delay()).max(Duration::from_millis(10));
    let (width, height) = frame.buffer().dimensions();
    check_decoded_size(width, height)?;
    let rgba = frame.into_buffer();
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

        let (mut frames, animation_metadata) = match format {
            AnimatedFormat::Gif => open_frame_stream(path, format)?,
            AnimatedFormat::WebP => {
                let decoder = open_webp_decoder(path)?;
                if !decoder.has_animation() {
                    return Ok(Self {
                        path: path.to_owned(),
                        format,
                        current: decode_image_decoder(decoder)?,
                        frame_delay: Duration::ZERO,
                        frames: None,
                        visible: false,
                        remaining: Duration::ZERO,
                        next_due: None,
                    });
                }
                (decoder.into_frames(), Some(true))
            }
        };
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

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn has_external_css_url(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor + 3 <= bytes.len() {
        if !bytes[cursor..cursor + 3].eq_ignore_ascii_case(b"url") {
            cursor += 1;
            continue;
        }
        let mut start = cursor + 3;
        while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
            start += 1;
        }
        if bytes.get(start) != Some(&b'(') {
            cursor += 1;
            continue;
        }
        start += 1;
        while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
            start += 1;
        }
        let quote = bytes
            .get(start)
            .copied()
            .filter(|b| matches!(b, b'\'' | b'"'));
        if quote.is_some() {
            start += 1;
        }
        let mut end = start;
        while end < bytes.len() && Some(bytes[end]) != quote && bytes[end] != b')' {
            end += 1;
        }
        if end == bytes.len() {
            return true;
        }
        let url = value[start..end].trim();
        if !url.starts_with('#') {
            return true;
        }
        if quote.is_some() {
            end += 1;
            while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
                end += 1;
            }
            if bytes.get(end) != Some(&b')') {
                return true;
            }
        }
        cursor = end.saturating_add(1);
    }
    false
}

fn svg_value_has_external_reference(value: &str) -> bool {
    contains_ascii_case_insensitive(value, "://")
        || contains_ascii_case_insensitive(value, "file:")
        || contains_ascii_case_insensitive(value, "data:")
        || contains_ascii_case_insensitive(value, "@import")
        || contains_ascii_case_insensitive(value, "@font-face")
        || has_external_css_url(value)
}

fn svg_system_fonts() -> Arc<resvg::usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = resvg::usvg::fontdb::Database::new();
            database.load_system_fonts();
            Arc::new(database)
        })
        .clone()
}

fn parse_svg_tree(bytes: &[u8], use_system_fonts: bool) -> io::Result<resvg::usvg::Tree> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SVG_INPUT_BYTES {
        return Err(invalid("SVGがXML入力の安全な上限を超えています"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| invalid("SVGはUTF-8で指定してください"))?;
    let xml = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_SVG_NODES,
        },
    )
    .map_err(|e| invalid(e.to_string()))?;
    let mut has_text = false;
    let mut use_elements = 0;
    for node in xml.descendants() {
        if node.ancestors().take(MAX_SVG_DEPTH + 1).count() > MAX_SVG_DEPTH {
            return Err(invalid("SVGのXML階層が安全な上限を超えています"));
        }
        if !node.is_element() {
            continue;
        }
        let name = node.tag_name().name();
        if [
            "script",
            "foreignObject",
            "image",
            "iframe",
            "audio",
            "video",
            "feImage",
        ]
        .iter()
        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
        {
            return Err(invalid("SVGのscript、埋込みHTML、外部画像は表示できません"));
        }
        if name.eq_ignore_ascii_case("font-face-uri") || name.eq_ignore_ascii_case("font-face-src")
        {
            return Err(invalid("SVGの外部フォントは表示できません"));
        }
        if name.eq_ignore_ascii_case("use") {
            use_elements += 1;
            if use_elements > MAX_SVG_USE_ELEMENTS {
                return Err(invalid("SVGの参照要素が安全な上限を超えています"));
            }
        }
        has_text |= name.eq_ignore_ascii_case("text");
        for attr in node.attributes() {
            let name = attr.name();
            let value = attr.value().trim();
            let is_event = name
                .as_bytes()
                .get(..2)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"on"));
            if is_event
                || name.eq_ignore_ascii_case("base")
                || (name.eq_ignore_ascii_case("href") && !value.starts_with('#'))
                || svg_value_has_external_reference(value)
            {
                return Err(invalid("SVGの外部参照や実行要素は許可されていません"));
            }
        }
        if node.text().is_some_and(svg_value_has_external_reference) {
            return Err(invalid("SVGの外部スタイルやフォントは許可されていません"));
        }
    }

    let mut options = resvg::usvg::Options {
        image_href_resolver: resvg::usvg::ImageHrefResolver {
            resolve_data: Box::new(|_, _, _| None),
            resolve_string: Box::new(|_, _| None),
        },
        ..Default::default()
    };
    if has_text {
        options.font_family = "Yu Gothic UI".to_owned();
        if use_system_fonts {
            options.fontdb = svg_system_fonts();
        }
    }
    resvg::usvg::Tree::from_xmltree(&xml, &options).map_err(|e| invalid(e.to_string()))
}

fn svg_dimensions(tree: &resvg::usvg::Tree) -> io::Result<(u32, u32)> {
    let size = tree.size().to_int_size();
    let (width, height) = size.dimensions();
    decoded_bytes(width, height)?;
    Ok((width, height))
}

pub fn decode_svg(bytes: &[u8]) -> io::Result<DecodedImage> {
    let tree = parse_svg_tree(bytes, true)?;
    let (width, height) = svg_dimensions(&tree)
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
    let extension = source_path
        .extension()
        .and_then(|v| v.to_str())
        .ok_or_else(|| invalid("画像拡張子が必要です"))?
        .to_ascii_lowercase();
    let max_input_bytes = import_input_limit(&extension)?;
    drop(decode(source_path)?);
    let source = File::open(source_path)?;
    let source_len = source.metadata()?.len();
    if source_len > max_input_bytes {
        return Err(invalid(
            "画像ファイルが入力サイズの安全な上限を超えています",
        ));
    }
    let read_limit = max_input_bytes
        .checked_add(1)
        .ok_or_else(|| invalid("画像入力サイズを確認できません"))?;
    import_reader(
        workspace,
        document_dir,
        &extension,
        source.take(read_limit),
        source_len,
    )
}

pub fn import_png(
    workspace: &Path,
    document_dir: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> io::Result<ImportedImage> {
    use image::ImageEncoder;
    let expected = usize::try_from(decoded_bytes(width, height)?)
        .map_err(|_| invalid("画像の展開サイズを確認できません"))?;
    if rgba.len() != expected {
        return Err(invalid("画像データのサイズが寸法と一致しません"));
    }
    let mut writer = BoundedVecWriter::new();
    image::codecs::png::PngEncoder::new(&mut writer)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| invalid(e.to_string()))?;
    import_bytes(workspace, document_dir, "png", &writer.into_inner())
}

struct BoundedVecWriter {
    bytes: Vec<u8>,
}

impl BoundedVecWriter {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn into_inner(self) -> Vec<u8> {
        self.bytes
    }
}

impl Write for BoundedVecWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let new_len = u64::try_from(self.bytes.len())
            .unwrap_or(u64::MAX)
            .checked_add(u64::try_from(buf.len()).unwrap_or(u64::MAX))
            .ok_or_else(|| invalid("PNGの入力サイズを確認できません"))?;
        if new_len > MAX_IMAGE_INPUT_BYTES {
            return Err(invalid("PNGが入力サイズの安全な上限を超えています"));
        }
        self.bytes
            .try_reserve(buf.len())
            .map_err(|e| io::Error::other(e.to_string()))?;
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn import_input_limit(extension: &str) -> io::Result<u64> {
    match extension {
        "png" | "jpg" | "jpeg" | "gif" | "webp" => Ok(MAX_IMAGE_INPUT_BYTES),
        "svg" => Ok(MAX_SVG_INPUT_BYTES),
        _ => Err(invalid("未対応の画像形式です")),
    }
}

fn import_bytes(
    workspace: &Path,
    document_dir: &Path,
    extension: &str,
    bytes: &[u8],
) -> io::Result<ImportedImage> {
    let limit = import_input_limit(extension)?;
    let input_len =
        u64::try_from(bytes.len()).map_err(|_| invalid("画像入力サイズを確認できません"))?;
    if input_len > limit {
        return Err(invalid(
            "画像ファイルが入力サイズの安全な上限を超えています",
        ));
    }
    import_reader(workspace, document_dir, extension, bytes, input_len)
}

fn import_reader(
    workspace: &Path,
    document_dir: &Path,
    extension: &str,
    mut bytes: impl Read,
    expected_len: u64,
) -> io::Result<ImportedImage> {
    let max_input_bytes = import_input_limit(extension)?;
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
        let copy_result = io::copy(
            &mut bytes.by_ref().take(max_input_bytes.saturating_add(1)),
            &mut file,
        );
        let copied = match copy_result {
            Ok(copied) => copied,
            Err(error) => {
                drop(file);
                let _ = fs::remove_file(&path);
                return Err(error);
            }
        };
        if copied > max_input_bytes || copied != expected_len {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(invalid("画像ファイルのサイズが読み込み中に変化しました"));
        }
        if let Err(error) = file.sync_all() {
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

    fn oversized_raster_fixtures() -> Vec<(&'static str, Vec<u8>)> {
        let width = MAX_IMAGE_DIMENSION + 1;
        let rgba = vec![0; width as usize * 4];
        let rgb = vec![0; width as usize * 3];

        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(&rgba, width, 1, image::ExtendedColorType::Rgba8)
            .unwrap();

        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .write_image(&rgb, width, 1, image::ExtendedColorType::Rgb8)
            .unwrap();

        let mut gif = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut gif)
            .encode_frame(image::Frame::from_parts(
                image::RgbaImage::new(width, 1),
                0,
                0,
                image::Delay::from_numer_denom_ms(40, 1),
            ))
            .unwrap();

        let mut webp_chunks = Vec::new();
        let mut extended_header = vec![0; 4];
        for dimension in [width - 1, 0] {
            extended_header.extend_from_slice(&[
                dimension as u8,
                (dimension >> 8) as u8,
                (dimension >> 16) as u8,
            ]);
        }
        append_webp_chunk(&mut webp_chunks, b"VP8X", &extended_header);
        webp_chunks.extend_from_slice(&webp_lossless_chunk([0, 0, 0, 255]));
        let mut webp_body = b"WEBP".to_vec();
        webp_body.extend_from_slice(&webp_chunks);
        let mut webp = b"RIFF".to_vec();
        webp.extend_from_slice(&(webp_body.len() as u32).to_le_bytes());
        webp.extend_from_slice(&webp_body);

        let svg = format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='{width}' height='1'><rect width='1' height='1'/></svg>"
        )
        .into_bytes();

        vec![
            ("png", png),
            ("jpg", jpeg),
            ("gif", gif),
            ("webp", webp),
            ("svg", svg),
        ]
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
        two_frame_webp_canvas(1, 1)
    }

    fn two_frame_webp_canvas(width: u32, height: u32) -> Vec<u8> {
        // Compose a minimal RIFF animation around real, lossless one-pixel VP8L frames.
        let mut chunks = Vec::new();
        let mut extended_header = vec![0x02, 0, 0, 0]; // animation feature flag
        for dimension in [width - 1, height - 1] {
            extended_header.extend_from_slice(&[
                dimension as u8,
                (dimension >> 8) as u8,
                (dimension >> 16) as u8,
            ]);
        }
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
    fn normal_fixture_formats_decode_with_the_checked_dimensions() {
        let assets = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/assets");
        for name in [
            "raster.png",
            "raster.jpg",
            "raster.gif",
            "raster.webp",
            "fixture.svg",
        ] {
            let path = assets.join(name);
            let image = decode(&path).unwrap_or_else(|error| panic!("{name}: {error}"));
            let dimensions = (image.width, image.height);
            assert!(dimensions.0 > 0 && dimensions.1 > 0, "{name}");
            assert_eq!(image_dimensions(&path).unwrap(), dimensions, "{name}");
            assert_eq!(
                u64::try_from(image.pixels.len()).unwrap(),
                decoded_bytes(image.width, image.height).unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn truncated_inputs_fail_for_each_supported_format() {
        for (extension, bytes) in [
            ("png", &b"\x89PNG\r\n\x1a\n"[..]),
            ("jpg", &b"\xff\xd8\xff"[..]),
            ("gif", &b"GIF89a\x01"[..]),
            ("webp", &b"RIFF\0\0\0\0WEBP"[..]),
            ("svg", &b"<svg xmlns='http://www.w3.org/2000/svg'"[..]),
        ] {
            let path = test_image_path(extension);
            fs::write(&path, bytes).unwrap();
            assert!(decode(&path).is_err(), "{extension} unexpectedly decoded");
            let _ = fs::remove_file(path);
        }
    }

    #[test]
    fn oversized_sparse_inputs_are_rejected_before_reading_bodies() {
        let raster_path = test_image_path("png");
        let raster_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&raster_path)
            .unwrap();
        raster_file.set_len(MAX_IMAGE_INPUT_BYTES + 1).unwrap();
        drop(raster_file);
        assert!(decode(&raster_path).is_err());
        assert!(image_dimensions(&raster_path).is_err());
        let _ = fs::remove_file(raster_path);

        let svg_path = test_image_path("svg");
        let svg_file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&svg_path)
            .unwrap();
        svg_file.set_len(MAX_SVG_INPUT_BYTES + 1).unwrap();
        drop(svg_file);
        assert!(decode(&svg_path).is_err());
        assert!(image_dimensions(&svg_path).is_err());
        let _ = fs::remove_file(svg_path);
    }

    #[test]
    fn every_format_rejects_oversized_dimensions_before_expansion() {
        assert_eq!(decoded_bytes(8192, 8192).unwrap(), MAX_DECODED_BYTES);
        assert!(decoded_bytes(8193, 8192).is_err());
        assert!(decoded_bytes(u32::MAX, u32::MAX).is_err());

        for (extension, bytes) in oversized_raster_fixtures() {
            let path = test_image_path(extension);
            fs::write(&path, bytes).unwrap();
            assert!(
                image_dimensions(&path).is_err(),
                "{extension} dimensions passed"
            );
            assert!(decode(&path).is_err(), "{extension} pixels were expanded");
            if matches!(extension, "gif" | "webp") {
                assert!(
                    AnimatedImage::open(&path).is_err(),
                    "{extension} canvas passed"
                );
            }
            let _ = fs::remove_file(path);
        }

        let animation_path = test_image_path("webp");
        fs::write(
            &animation_path,
            two_frame_webp_canvas(MAX_IMAGE_DIMENSION + 1, 1),
        )
        .unwrap();
        assert!(AnimatedImage::open(&animation_path).is_err());
        let _ = fs::remove_file(animation_path);
    }

    #[test]
    fn rejected_import_keeps_the_document_and_source_file_unchanged() {
        let root = std::env::temp_dir().join(format!(
            "mdlite-image-import-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let document = root.join("note.md");
        let source = root.join("broken.png");
        let source_bytes = b"\x89PNG\r\n\x1a\n";
        fs::write(&document, b"# Unsaved work\n").unwrap();
        fs::write(&source, source_bytes).unwrap();

        assert!(import_file(&root, &root, &source).is_err());
        assert_eq!(fs::read(&document).unwrap(), b"# Unsaved work\n");
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert!(!root.join("assets").exists());
        let _ = fs::remove_file(source);
        let _ = fs::remove_file(document);
        let _ = fs::remove_dir(root);
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
            "<use href='https://example.test/image.svg#shape'/>",
            "<style>rect { fill: url(file:private.svg) }</style>",
            "<style>@import url('//example.test/private.css');</style>",
            "<feImage href='private.png'/>",
            "<font-face-uri href='private.woff'/>",
        ] {
            let data = format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='8' height='4'>{dangerous}</svg>"
            );
            assert!(decode_svg(data.as_bytes()).is_err());
        }
        assert!(local_image_path(Path::new("."), "https://example.test/a.png").is_err());

        let local_reference = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="4"><defs><linearGradient id="g"><stop offset="0" stop-color="#f00"/></linearGradient></defs><rect width="8" height="4" fill="url(#g)"/></svg>"##;
        assert!(decode_svg(local_reference).is_ok());
    }

    #[test]
    fn svg_xml_nodes_and_nesting_are_bounded() {
        let mut too_many_nodes =
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>".to_owned();
        too_many_nodes.extend(std::iter::repeat_n("<g/>", MAX_SVG_NODES as usize + 1));
        too_many_nodes.push_str("</svg>");
        assert!(decode_svg(too_many_nodes.as_bytes()).is_err());

        let mut too_deep =
            "<svg xmlns='http://www.w3.org/2000/svg' width='1' height='1'>".to_owned();
        too_deep.extend(std::iter::repeat_n("<g>", MAX_SVG_DEPTH + 1));
        too_deep.push_str("<rect width='1' height='1'/>");
        too_deep.extend(std::iter::repeat_n("</g>", MAX_SVG_DEPTH + 1));
        too_deep.push_str("</svg>");
        assert!(decode_svg(too_deep.as_bytes()).is_err());
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
