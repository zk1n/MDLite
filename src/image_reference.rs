//! Narrow, non-executing legacy `<img>` compatibility and source-local edits.
use std::ops::Range;
#[derive(Clone, Debug)]
pub struct LegacyImage {
    pub destination: String,
    pub alt: String,
    pub title: Option<String>,
    pub width: Option<u32>,
    pub destination_range: Range<usize>,
    pub width_range: Option<Range<usize>>,
    pub insert_width: usize,
}
pub fn parse_legacy(source: &str) -> Option<LegacyImage> {
    if source.len() > 16 * 1024 {
        return None;
    }
    let start = source.find('<')?;
    if !source[..start].trim().is_empty() {
        return None;
    }
    let trimmed = source[start..].trim_end();
    if !trimmed.starts_with("<img ")
        && !trimmed.starts_with("<img\n")
        && !trimmed.starts_with("<img\t")
    {
        return None;
    }
    let end = trimmed.strip_suffix('>')?;
    let xml = if end.ends_with('/') {
        trimmed.to_owned()
    } else {
        format!("{end}/>")
    };
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: 4,
    };
    let doc = roxmltree::Document::parse_with_options(&xml, options).ok()?;
    let node = doc.root_element();
    if node.tag_name().name() != "img"
        || node.tag_name().namespace().is_some()
        || node.children().next().is_some()
    {
        return None;
    }
    if node
        .attributes()
        .any(|a| a.namespace().is_some() || !matches!(a.name(), "src" | "alt" | "title" | "width"))
    {
        return None;
    }
    let src = node.attribute_node("src")?;
    let destination = src.value().to_owned();
    if destination.is_empty() || destination.chars().any(char::is_control) {
        return None;
    }
    let width = node
        .attribute("width")
        .map(str::parse::<u32>)
        .transpose()
        .ok()?;
    if width.is_some_and(|w| !(24..=16384).contains(&w)) {
        return None;
    }
    Some(LegacyImage {
        destination,
        alt: node.attribute("alt").unwrap_or("").into(),
        title: node.attribute("title").map(str::to_owned),
        width,
        destination_range: src.range_value().start + start..src.range_value().end + start,
        width_range: node
            .attribute_node("width")
            .map(|a| a.range_value().start + start..a.range_value().end + start),
        insert_width: start + end.trim_end_matches('/').len(),
    })
}
pub fn set_width(source: &str, width: u32) -> Result<String, &'static str> {
    if !(24..=16384).contains(&width) {
        return Err("image width is outside supported range");
    }
    if let Some(image) = parse_legacy(source) {
        if image.width == Some(width) {
            return Ok(source.into());
        }
        let mut output = source.to_owned();
        if let Some(range) = image.width_range {
            output.replace_range(range, &width.to_string());
        } else {
            output.insert_str(image.insert_width, &format!(" width=\"{width}\""));
        }
        return Ok(output);
    }
    if !source.trim_start().starts_with("![") {
        return Err("not a supported image reference");
    }
    if let Some(start) = source.find("<!-- mdlite-width:") {
        let digits = start + "<!-- mdlite-width:".len();
        let end = source[digits..]
            .find("-->")
            .ok_or("invalid image width comment")?
            + digits;
        let original = &source[digits..end];
        let leading = original.len() - original.trim_start().len();
        let trailing = original.trim_end().len();
        original
            .trim()
            .parse::<u32>()
            .map_err(|_| "invalid image width value")?;
        let mut output = source.to_owned();
        output.replace_range(digits + leading..digits + trailing, &width.to_string());
        Ok(output)
    } else {
        Ok(format!("{source} <!-- mdlite-width:{width} -->"))
    }
}
/// Encode path punctuation without changing Unicode spelling. The matching
/// local resolver percent-decodes exactly once before filesystem resolution.
pub fn encode_path(path: &str) -> String {
    let mut result = String::new();
    for c in path.chars() {
        if c.is_control()
            || matches!(
                c,
                ' ' | '(' | ')' | '<' | '>' | '"' | '\'' | '#' | '?' | '%'
            )
        {
            let mut buffer = [0; 4];
            for b in c.encode_utf8(&mut buffer).bytes() {
                result.push_str(&format!("%{b:02X}"));
            }
        } else {
            result.push(c);
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_width_keeps_alt_title_quotes_and_other_attributes_literal() {
        let source = "<img src='画像 (1).png' alt='a &amp; b' title=\"keep\" width='320'>";
        let parsed = parse_legacy(source).unwrap();
        assert_eq!(parsed.alt, "a & b");
        assert_eq!(parsed.destination, "画像 (1).png");
        assert_eq!(
            set_width(source, 480).unwrap(),
            source.replace("'320'", "'480'")
        );
        assert_eq!(set_width(source, 320).unwrap(), source);
        let added = set_width("<img src=\"a.png\" alt=\"x\" />", 640).unwrap();
        assert_eq!(parse_legacy(&added).unwrap().width, Some(640));
        for bad in [
            "<img src='x' onerror='run()'>",
            "<script>x</script>",
            "<img src='x' style='color:red'>",
        ] {
            assert!(parse_legacy(bad).is_none());
        }
    }
    #[test]
    fn markdown_width_changes_only_digits_and_path_roundtrips() {
        let source = "![alt](<a (1).png> \"title\") <!-- mdlite-width:320 -->";
        assert_eq!(
            set_width(source, 480).unwrap(),
            source.replace("320", "480")
        );
        let path = "画像 (1)#%.png";
        assert_eq!(
            crate::navigation::percent_decode(&encode_path(path)).unwrap(),
            path
        );
    }
    #[test]
    fn legacy_projection_is_narrow_and_source_preserving() {
        let source = "<img src='画像.png' alt='shown' width='320'>\n\n<script>alert(1)</script>\n\n```\n<img src='literal.png'>\n```\n";
        let p = crate::projection::DisplayProjection::parse(source);
        let images: Vec<_> = p
            .blocks
            .iter()
            .filter(|b| matches!(b.kind, crate::projection::BlockKind::Image { .. }))
            .collect();
        assert_eq!(images.len(), 1);
        assert_eq!(
            &source[images[0].source.clone()],
            "<img src='画像.png' alt='shown' width='320'>\n"
        );
        assert!(p.text.contains("<script>alert(1)</script>"));
        assert!(p.text.contains("<img src='literal.png'>"));
    }
    #[test]
    fn encoded_local_paths_resolve_once_and_document_svg_is_refused() {
        let root = std::env::temp_dir();
        let destination = encode_path("画像 (1)#%.png");
        assert_eq!(
            crate::assets::local_image_path(&root, &destination).unwrap(),
            root.join("画像 (1)#%.png")
        );
        assert!(crate::assets::local_image_path(&root, "unsafe.svg").is_err());
        assert!(crate::assets::local_image_path(&root, "https%3A//example.test/a.png").is_err());
    }
}
