//! Source-backed outline and validated link targets. No shell execution here.
use crate::{
    coordinates,
    document::{Document, Selection},
    projection::{BlockKind, DisplayProjection},
};
use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub label: String,
    pub anchor: String,
    pub source: Range<usize>,
    pub section: Range<usize>,
}
#[derive(Clone, Debug)]
pub struct Outline {
    pub document_id: u64,
    pub revision: u64,
    pub headings: Vec<Heading>,
}
impl Outline {
    pub fn build(doc: &Document, projection: &DisplayProjection) -> Self {
        let mut emitted = HashSet::<String>::new();
        let mut headings: Vec<Heading> = projection
            .blocks
            .iter()
            .filter_map(|block| {
                let BlockKind::Heading { level } = block.kind else {
                    return None;
                };
                let start =
                    coordinates::utf16_to_byte(&projection.text, block.display_utf16.start).ok()?;
                let end =
                    coordinates::utf16_to_byte(&projection.text, block.display_utf16.end).ok()?;
                let label = projection.text[start..end].trim().to_owned();
                let base = slug(&label);
                let mut anchor = base.clone();
                let mut suffix = 1;
                while !emitted.insert(anchor.clone()) {
                    anchor = format!("{base}-{suffix}");
                    suffix += 1;
                }
                Some(Heading {
                    level,
                    label,
                    anchor,
                    source: block.source.clone(),
                    section: block.source.start..doc.byte_len(),
                })
            })
            .collect();
        for i in 0..headings.len() {
            if let Some(next) =
                (i + 1..headings.len()).find(|&j| headings[j].level <= headings[i].level)
            {
                headings[i].section.end = headings[next].source.start;
            }
        }
        Self {
            document_id: doc.id(),
            revision: doc.revision(),
            headings,
        }
    }
    pub fn is_current(&self, doc: &Document) -> bool {
        self.document_id == doc.id() && self.revision == doc.revision()
    }
    pub fn anchor(&self, value: &str) -> Result<usize, &'static str> {
        let value = percent_decode(value)?;
        self.headings
            .iter()
            .find(|h| h.anchor == value)
            .map(|h| h.source.start)
            .ok_or("anchor not found")
    }
    /// Move a section immediately before another heading in one source transaction.
    pub fn move_section(
        &self,
        doc: &mut Document,
        from: usize,
        before: usize,
        composing: bool,
    ) -> Result<(), &'static str> {
        if composing || !self.is_current(doc) {
            return Err("outline is stale or IME is active");
        }
        let source = &self.headings.get(from).ok_or("unknown section")?.section;
        let destination = self
            .headings
            .get(before)
            .ok_or("unknown destination")?
            .source
            .start;
        if (source.start..=source.end).contains(&destination) {
            return Err("destination is inside section or unchanged");
        }
        let moved = &doc.source()[source.clone()];
        // A final section without a trailing EOL needs one separator when moved
        // before a heading. It belongs to the same explicit structural edit.
        let separator = if moved.ends_with('\n') {
            ""
        } else if doc.source().contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let (range, replacement, caret) = if destination < source.start {
            (
                destination..source.end,
                format!(
                    "{moved}{separator}{}",
                    &doc.source()[destination..source.start]
                ),
                destination,
            )
        } else {
            (
                source.start..destination,
                format!(
                    "{}{}",
                    &doc.source()[source.end..destination],
                    &doc.source()[source.clone()]
                ),
                destination - (source.end - source.start),
            )
        };
        doc.replace_range_with_selection(range, &replacement, Selection::caret(caret))
            .map_err(|_| "section transaction failed")
    }
}
fn slug(label: &str) -> String {
    label
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_whitespace() {
                Some('-')
            } else if c.is_alphanumeric() || matches!(c, '_' | '-') {
                Some(c)
            } else {
                None
            }
        })
        .collect()
}
pub fn percent_decode(text: &str) -> Result<String, &'static str> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut i = 0;
    let raw = text.as_bytes();
    while i < raw.len() {
        if raw[i] == b'%' {
            let pair = raw.get(i + 1..i + 3).ok_or("incomplete percent escape")?;
            let hex = |b: u8| (b as char).to_digit(16).ok_or("invalid percent escape");
            bytes.push((hex(pair[0])? * 16 + hex(pair[1])?) as u8);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let result = String::from_utf8(bytes).map_err(|_| "escape is not UTF-8")?;
    if result.chars().any(|c| c.is_control()) {
        return Err("control character in target");
    }
    Ok(result)
}
#[derive(Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Anchor(String),
    Local {
        path: PathBuf,
        anchor: Option<String>,
    },
    External(String),
}
pub fn link_target(
    destination: &str,
    document_path: Option<&Path>,
) -> Result<LinkTarget, &'static str> {
    if let Some(anchor) = destination.strip_prefix('#') {
        percent_decode(anchor)?;
        return Ok(LinkTarget::Anchor(anchor.into()));
    }
    if let Some((scheme, _)) = destination.split_once(':') {
        if ["https", "http", "mailto"]
            .iter()
            .any(|s| scheme.eq_ignore_ascii_case(s))
        {
            if destination.chars().any(|c| c.is_control()) {
                return Err("control character in URL");
            }
            return Ok(LinkTarget::External(destination.into()));
        }
        // A native drive path is a local file, never a URI scheme.
        if scheme.len() != 1 || !scheme.as_bytes()[0].is_ascii_alphabetic() {
            return Err("unsupported link scheme");
        }
    }
    let (path, anchor) = destination
        .split_once('#')
        .map_or((destination, None), |(p, a)| (p, Some(a)));
    let path = PathBuf::from(percent_decode(path)?);
    let path = if path.is_absolute() {
        path
    } else {
        document_path
            .and_then(Path::parent)
            .ok_or("save document before opening relative links")?
            .join(path)
    };
    let path = std::fs::canonicalize(path).map_err(|_| "local target cannot be resolved")?;
    if !path.is_file() {
        return Err("local target is not a file");
    }
    // Keep the encoded fragment until Outline::anchor, its one decoding boundary.
    if let Some(value) = anchor {
        percent_decode(value)?;
    }
    let anchor = anchor.map(str::to_owned);
    Ok(LinkTarget::Local { path, anchor })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchor_collision_and_single_fragment_decode_are_unambiguous() {
        let d = Document::new(1, "# a\n# a\n# a-1\n# a\n# 100%\n".into());
        let o = Outline::build(&d, &DisplayProjection::parse(d.source()));
        let anchors: std::collections::HashSet<_> =
            o.headings.iter().map(|h| h.anchor.as_str()).collect();
        assert_eq!(anchors.len(), o.headings.len());
        assert_eq!(o.anchor("a-1-1").unwrap(), o.headings[2].source.start);
        assert!(o.anchor("%2561").is_err()); // decode once to literal %61, never to a
        let mut eof = Document::new(2, "# A\r\na\r\n# B\r\nb".into());
        let outline = Outline::build(&eof, &DisplayProjection::parse(eof.source()));
        outline.move_section(&mut eof, 1, 0, false).unwrap();
        assert_eq!(eof.source(), "# B\r\nb\r\n# A\r\na\r\n");
        eof.undo();
        assert_eq!(eof.source(), "# A\r\na\r\n# B\r\nb");
    }
    #[test]
    fn parser_outline_setext_frontmatter_code_duplicate_and_untitled_anchor() {
        let d = Document::new_unsaved(
            1,
            "---\nx: y\n---\n# 日本語\n\n```\n# hidden\n```\n\nSetext\n======\n\n# 日本語\n".into(),
        );
        let o = Outline::build(&d, &DisplayProjection::parse(d.source()));
        assert_eq!(o.headings.len(), 3);
        assert_eq!(o.headings[1].label, "Setext");
        assert_eq!(o.headings[2].anchor, "日本語-1");
        assert_eq!(
            o.anchor("%E6%97%A5%E6%9C%AC%E8%AA%9E-1").unwrap(),
            o.headings[2].source.start
        );
        assert!(matches!(
            link_target("#日本語", None).unwrap(),
            LinkTarget::Anchor(_)
        ));
        assert!(!d.is_dirty() || d.undo_count() == 0);
    }
    #[test]
    fn move_section_with_children_is_one_undo_and_rejects_stale_ime_and_self() {
        let text = "# A\r\na\r\n## child\r\nc\r\n# B\r\nb\r\n# C\r\nc\r\n";
        let mut d = Document::new(7, text.into());
        let o = Outline::build(&d, &DisplayProjection::parse(d.source()));
        assert!(o.move_section(&mut d, 0, 1, false).is_err());
        assert!(o.move_section(&mut d, 0, 3, true).is_err());
        o.move_section(&mut d, 0, 3, false).unwrap();
        assert_eq!(
            d.source(),
            "# B\r\nb\r\n# A\r\na\r\n## child\r\nc\r\n# C\r\nc\r\n"
        );
        assert_eq!(d.undo_count(), 1);
        assert!(o.move_section(&mut d, 0, 3, false).is_err());
        d.undo();
        assert_eq!(d.source(), text);
        assert!(!d.is_dirty());
    }
    #[test]
    fn links_reject_arbitrary_schemes_and_broken_escapes() {
        for value in [
            "javascript:alert(1)",
            "file:///private",
            "shell:run",
            "#%xx",
            "#%FF",
            "#%00",
        ] {
            assert!(link_target(value, None).is_err(), "{value}");
        }
        assert_eq!(
            link_target("https://example.test/a", None).unwrap(),
            LinkTarget::External("https://example.test/a".into())
        );
    }
}
