//! Revision-bound edits to the language token of a recognized fenced block.
use crate::{
    document::{Document, Selection},
    projection::{BlockKind, DisplayProjection},
};
use std::ops::Range;

#[derive(Clone, Debug)]
pub struct CodeLanguage {
    pub document_id: u64,
    pub revision: u64,
    pub block: Range<usize>,
    pub opening: Range<usize>,
    pub token: Range<usize>,
    pub language: String,
}

impl CodeLanguage {
    /// Clearing the first token cannot denote an unspecified language while
    /// another info token remains: that token would become the language.
    pub fn can_clear(&self, doc: &Document) -> bool {
        doc.id() == self.document_id
            && doc.revision() == self.revision
            && doc
                .source()
                .get(self.token.end..self.opening.end)
                .is_some_and(|info| info.trim().is_empty())
    }
    pub fn all(doc: &Document, projection: &DisplayProjection) -> Vec<Self> {
        projection
            .blocks
            .iter()
            .filter_map(|block| {
                if !matches!(block.kind, BlockKind::Code { fenced: true, .. }) {
                    return None;
                }
                let line = projection.source_delimiter_line_at(block.source.start)?;
                let source = doc
                    .source()
                    .get(line.clone())?
                    .trim_end_matches(['\r', '\n']);
                let marker = source.find(['`', '~'])?;
                let byte = source.as_bytes()[marker];
                let length = source.as_bytes()[marker..]
                    .iter()
                    .take_while(|c| **c == byte)
                    .count();
                if length < 3 {
                    return None;
                }
                let info = marker + length;
                let remainder = &source[info..];
                let start = info + remainder.len()
                    - remainder.trim_start_matches(char::is_whitespace).len();
                let end = start
                    + source[start..]
                        .find(char::is_whitespace)
                        .unwrap_or(source.len() - start);
                Some(Self {
                    document_id: doc.id(),
                    revision: doc.revision(),
                    block: block.source.clone(),
                    opening: line.clone(),
                    token: line.start + start..line.start + end,
                    language: source[start..end].to_owned(),
                })
            })
            .collect()
    }

    /// Update only the first info token. The marker, indent and remaining info
    /// bytes belong to the source and are never normalized by this operation.
    pub fn set(&self, doc: &mut Document, language: &str) -> Result<bool, &'static str> {
        if doc.id() != self.document_id || doc.revision() != self.revision {
            return Err("コードブロックが変更されています");
        }
        if language
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '`')
        {
            return Err("言語名には空白・改行・backtickを使えません");
        }
        if language == self.language {
            return Ok(false);
        }
        if language.is_empty() && !self.can_clear(doc) {
            return Err("追加のinfoを保持するため言語を削除できません。原文から編集できます");
        }
        if doc.source().get(self.token.clone()) != Some(self.language.as_str()) {
            return Err("コードブロックが変更されています");
        }
        doc.set_selection(self.token.start, self.token.start)
            .map_err(|_| "言語位置が無効です")?;
        doc.replace_range_with_selection(
            self.token.clone(),
            language,
            Selection::caret(self.token.start + language.len()),
        )
        .map_err(|_| "言語を変更できません")?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_only_language_and_preserves_markers_info_body_and_history() {
        for eol in ["\n", "\r\n"] {
            for marker in ["```", "~~~~~"] {
                let source =
                    format!("{marker}rust  title=\"a\"{eol}## literal{eol}{marker}{eol}body");
                let mut doc = Document::new(1, source.clone());
                let fence =
                    CodeLanguage::all(&doc, &DisplayProjection::parse(doc.source())).remove(0);
                assert!(!fence.set(&mut doc, "rust").unwrap());
                assert_eq!(doc.undo_count(), 0);
                assert!(fence.set(&mut doc, "python").unwrap());
                assert_eq!(doc.source(), source.replacen("rust", "python", 1));
                assert_eq!(doc.undo_count(), 1);
                assert!(doc.undo());
                assert_eq!(doc.source(), source);
                assert!(doc.redo());
                assert_eq!(doc.source(), source.replacen("rust", "python", 1));
            }
        }
    }
    #[test]
    fn unspecified_unknown_and_stale_are_source_backed() {
        let mut doc = Document::new(1, "```\nx\n```".into());
        let fence = CodeLanguage::all(&doc, &DisplayProjection::parse(doc.source())).remove(0);
        assert_eq!(fence.language, "");
        assert!(fence.set(&mut doc, "my-language").unwrap());
        assert_eq!(doc.source(), "```my-language\nx\n```");
        assert!(fence.set(&mut doc, "rust").is_err());
        let fence = CodeLanguage::all(&doc, &DisplayProjection::parse(doc.source())).remove(0);
        assert!(fence.set(&mut doc, "").unwrap());
        assert_eq!(doc.source(), "```\nx\n```");
    }

    #[test]
    fn language_removal_with_remaining_info_is_explicit_non_destructive_refusal() {
        let source = "```python extra=kept\r\nbody\r\n```\r\n";
        let mut doc = Document::new(1, source.into());
        let fence = CodeLanguage::all(&doc, &DisplayProjection::parse(doc.source())).remove(0);
        let before = (
            doc.revision(),
            doc.selection(),
            doc.undo_count(),
            doc.is_dirty(),
        );
        assert!(!fence.can_clear(&doc));
        assert!(fence.set(&mut doc, "").is_err());
        assert_eq!(doc.source(), source);
        assert_eq!(
            (
                doc.revision(),
                doc.selection(),
                doc.undo_count(),
                doc.is_dirty()
            ),
            before
        );
        for space in [" ", "\t", "\u{a0}"] {
            let spaced = Document::new(2, format!("```{space}rust\nx\n```"));
            assert_eq!(
                CodeLanguage::all(&spaced, &DisplayProjection::parse(spaced.source()))[0].language,
                "rust"
            );
        }
    }
}
