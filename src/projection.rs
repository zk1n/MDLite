//! A source-backed display projection for Markdown editing.
//!
//! The parser output is transient presentation data. Every run and block
//! retains UTF-8 source byte ranges, and the boundary map translates the
//! editor's UTF-16 display positions without serializing or normalizing source.

use std::ops::Range;

pub use crate::coordinates::BoundaryAffinity;
use crate::markdown::{gfm_autolinks, parser_options};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, HeadingLevel, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockKind {
    Paragraph,
    Heading {
        level: u8,
    },
    List {
        ordered: Option<u64>,
    },
    ListItem,
    Quote,
    Code {
        fenced: bool,
        language: Option<String>,
    },
    FrontMatter,
    Table,
    Image {
        destination: String,
        title: Option<String>,
        width: Option<u32>,
    },
    ThematicBreak,
    Raw,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionBlock {
    pub kind: BlockKind,
    pub source: Range<usize>,
    /// UTF-16 offsets into `DisplayProjection::text`.
    pub display_utf16: Range<usize>,
    /// Nesting depth among projected block elements.
    pub depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InlineKind {
    Text,
    Strong,
    Emphasis,
    Strikethrough,
    Superscript,
    Subscript,
    Code,
    Link {
        destination: String,
        title: Option<String>,
    },
    Image {
        destination: String,
        title: Option<String>,
    },
    Html,
    Math,
    FootnoteReference,
    TaskMarker {
        checked: bool,
    },
    TableSeparator,
    Break,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionRun {
    /// UTF-16 offsets into `DisplayProjection::text`.
    pub display_utf16: Range<usize>,
    /// Source bytes that produced this run. Escapes and entities map to their
    /// full raw spelling, including a backslash or entity delimiters.
    pub source: Range<usize>,
    /// Active inline formatting and semantic tags for this run.
    pub kinds: Vec<InlineKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceBoundary {
    pub before: usize,
    pub after: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayProjection {
    /// Rendered text with table cells separated by tabs and rows by newlines.
    pub text: String,
    pub blocks: Vec<ProjectionBlock>,
    pub runs: Vec<ProjectionRun>,
    /// Index `n` corresponds to UTF-16 display boundary `n`.
    pub boundaries: Vec<SourceBoundary>,
    pub source_len: usize,
    /// Source insertion points which can disappear from the rendered text,
    /// including physical line starts and projected block boundaries.
    pub editable_source_positions: Vec<usize>,
    source_boundary_bits: Vec<u64>,
    source_spans: Vec<SourceProjectionSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceProjectionSpan {
    source: Range<usize>,
    display_utf16: Range<usize>,
}

struct PendingText<'a> {
    text: CowStr<'a>,
    range: Range<usize>,
}

impl PendingText<'_> {
    fn append(&mut self, next: CowStr<'_>, range: Range<usize>) {
        let mut text = std::mem::replace(&mut self.text, CowStr::Borrowed("")).into_string();
        text.push_str(&next);
        self.text = text.into();
        self.range.end = range.end;
    }
}

impl DisplayProjection {
    pub fn parse(source: &str) -> Self {
        let mut builder = ProjectionBuilder::new(source);
        let parser = Parser::new_ext(source, parser_options()).into_offset_iter();
        let mut pending_text: Option<PendingText<'_>> = None;
        for (event, range) in parser {
            match event {
                Event::Text(text) => {
                    let source_start = include_escape_prefix(source, range.clone()).start;
                    match pending_text.take() {
                        Some(mut pending) if pending.range.end == source_start => {
                            pending.append(text, range);
                            pending_text = Some(pending);
                        }
                        Some(pending) => {
                            builder.process(Event::Text(pending.text), pending.range);
                            pending_text = Some(PendingText { text, range });
                        }
                        None => pending_text = Some(PendingText { text, range }),
                    }
                }
                event => {
                    if let Some(pending) = pending_text.take() {
                        builder.process(Event::Text(pending.text), pending.range);
                    }
                    builder.process(event, range);
                }
            }
        }
        if let Some(pending) = pending_text {
            builder.process(Event::Text(pending.text), pending.range);
        }
        builder.finish()
    }

    /// Convert a display UTF-16 boundary to its nearest source byte boundary.
    /// Hidden Markdown syntax creates a pair of valid choices; `affinity`
    /// selects the preceding or following side. An offset inside a surrogate
    /// pair resolves to the owning source character on the requested side.
    pub fn source_byte_at_utf16(
        &self,
        display_utf16: usize,
        affinity: BoundaryAffinity,
    ) -> Option<usize> {
        let boundary = self.boundaries.get(display_utf16)?;
        Some(match affinity {
            BoundaryAffinity::Before => boundary.before,
            BoundaryAffinity::After => boundary.after,
        })
    }

    /// Convert a source byte boundary to the nearest UTF-16 display boundary.
    /// Source positions inside hidden Markdown collapse to that syntax's
    /// display boundary. Invalid UTF-8 offsets are rejected.
    pub fn utf16_at_source_byte(
        &self,
        source_byte: usize,
        affinity: BoundaryAffinity,
    ) -> Option<usize> {
        if source_byte > self.source_len || !self.is_source_char_boundary(source_byte) {
            return None;
        }
        if let Some(span) = self
            .source_spans
            .iter()
            .find(|span| source_byte > span.source.start && source_byte < span.source.end)
        {
            return Some(match affinity {
                BoundaryAffinity::Before => span.display_utf16.start,
                BoundaryAffinity::After => span.display_utf16.end,
            });
        }
        let mut nearest = None;
        let mut nearest_distance = usize::MAX;
        for (display, boundary) in self.boundaries.iter().enumerate() {
            let selected = match affinity {
                BoundaryAffinity::Before => boundary.before,
                BoundaryAffinity::After => boundary.after,
            };
            if selected == source_byte {
                return Some(display);
            }
            if source_byte >= boundary.before.min(boundary.after)
                && source_byte <= boundary.before.max(boundary.after)
            {
                return Some(display);
            }
            let distance = selected.abs_diff(source_byte);
            if distance < nearest_distance {
                nearest = Some(display);
                nearest_distance = distance;
            }
        }
        nearest
    }

    fn is_source_char_boundary(&self, source_byte: usize) -> bool {
        self.source_boundary_bits
            .get(source_byte / 64)
            .is_some_and(|word| word & (1 << (source_byte % 64)) != 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EndTag {
    Paragraph,
    Heading,
    List,
    ListItem,
    Quote,
    Code,
    FrontMatter,
    Table,
    Image,
    Raw,
    Footnote,
}

#[derive(Clone, Debug)]
struct OpenBlock {
    kind: BlockKind,
    end_tag: EndTag,
    source_start: usize,
    display_start: usize,
    depth: usize,
}

struct ProjectionBuilder<'a> {
    source: &'a str,
    text: String,
    display_utf16: usize,
    source_cursor: usize,
    boundaries: Vec<SourceBoundary>,
    blocks: Vec<ProjectionBlock>,
    runs: Vec<ProjectionRun>,
    source_spans: Vec<SourceProjectionSpan>,
    open_blocks: Vec<OpenBlock>,
    inline_stack: Vec<InlineKind>,
    table_rows: usize,
    table_cells: usize,
    in_table: bool,
}

impl<'a> ProjectionBuilder<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            text: String::new(),
            display_utf16: 0,
            source_cursor: 0,
            boundaries: vec![SourceBoundary {
                before: 0,
                after: 0,
            }],
            blocks: Vec::new(),
            runs: Vec::new(),
            source_spans: Vec::new(),
            open_blocks: Vec::new(),
            inline_stack: Vec::new(),
            table_rows: 0,
            table_cells: 0,
            in_table: false,
        }
    }

    fn process(&mut self, event: Event<'_>, range: Range<usize>) {
        match event {
            Event::Start(tag) => self.start_tag(&tag, range),
            Event::End(tag) => self.end_tag(&tag, range),
            Event::Text(value) => {
                let kinds = self.current_kinds();
                if self.in_literal_context() {
                    self.append_literal(&value, range, kinds, '\n');
                } else {
                    self.append_text_with_autolinks(&value, range, kinds);
                }
            }
            Event::Code(value) => {
                let mut kinds = self.current_kinds();
                kinds.push(InlineKind::Code);
                self.append_inline_code(&value, range, kinds);
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                if !self.append_image_width_comment(&value, &range)
                    && !self.append_legacy_image(&value, &range)
                {
                    let mut kinds = self.current_kinds();
                    kinds.push(InlineKind::Html);
                    self.append_literal(&value, range, kinds, '\n');
                }
            }
            Event::InlineMath(value) | Event::DisplayMath(value) => {
                let mut kinds = self.current_kinds();
                kinds.push(InlineKind::Math);
                self.append_literal(&value, range, kinds, '\n');
            }
            Event::FootnoteReference(label) => {
                let mut kinds = self.current_kinds();
                kinds.push(InlineKind::FootnoteReference);
                self.append_visible(&label, range, kinds);
            }
            Event::SoftBreak => {
                self.append_visible(" ", range, vec![InlineKind::Break]);
            }
            Event::HardBreak => {
                self.append_visible("\n", range, vec![InlineKind::Break]);
            }
            Event::Rule => {
                let display_start = self.display_utf16;
                let rule_line = self.source.get(range.clone()).unwrap_or_default();
                let marker_end = range.start + rule_line.trim_end_matches(['\r', '\n']).len();
                self.append_visible("─", range.start..marker_end, vec![InlineKind::Text]);
                self.hidden(&range);
                self.blocks.push(ProjectionBlock {
                    kind: BlockKind::ThematicBreak,
                    source: range,
                    display_utf16: display_start..self.display_utf16,
                    depth: self.open_blocks.len(),
                });
            }
            Event::TaskListMarker(checked) => {
                let mut kinds = self.current_kinds();
                kinds.push(InlineKind::TaskMarker { checked });
                let visible = if checked { "☑" } else { "☐" };
                self.append_visible(visible, range, kinds);
            }
        }
    }

    fn start_tag(&mut self, tag: &Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.open_block(BlockKind::Paragraph, EndTag::Paragraph, &range),
            Tag::Heading { level, .. } => self.open_block(
                BlockKind::Heading {
                    level: heading_level(level),
                },
                EndTag::Heading,
                &range,
            ),
            Tag::List(start) => {
                self.open_block(BlockKind::List { ordered: *start }, EndTag::List, &range)
            }
            Tag::Item => self.open_block(BlockKind::ListItem, EndTag::ListItem, &range),
            Tag::BlockQuote(_) => self.open_block(BlockKind::Quote, EndTag::Quote, &range),
            Tag::CodeBlock(kind) => {
                let (fenced, language) = code_block_info(kind);
                self.open_block(BlockKind::Code { fenced, language }, EndTag::Code, &range);
                self.inline_stack.push(InlineKind::Code);
            }
            Tag::MetadataBlock(_) => {
                self.open_block(BlockKind::FrontMatter, EndTag::FrontMatter, &range)
            }
            Tag::Table(_) => {
                self.in_table = true;
                self.table_rows = 0;
                self.table_cells = 0;
                self.open_block(BlockKind::Table, EndTag::Table, &range);
            }
            Tag::TableHead | Tag::TableRow => self.start_table_row(&range),
            Tag::TableCell => self.start_table_cell(&range),
            Tag::HtmlBlock => self.open_block(BlockKind::Raw, EndTag::Raw, &range),
            Tag::FootnoteDefinition(_) => self.open_block(BlockKind::Raw, EndTag::Footnote, &range),
            Tag::Emphasis => self.push_inline(InlineKind::Emphasis),
            Tag::Strong => self.push_inline(InlineKind::Strong),
            Tag::Strikethrough => self.push_inline(InlineKind::Strikethrough),
            Tag::Superscript => self.push_inline(InlineKind::Superscript),
            Tag::Subscript => self.push_inline(InlineKind::Subscript),
            Tag::Link {
                dest_url, title, ..
            } => self.push_inline(InlineKind::Link {
                destination: dest_url.to_string(),
                title: nonempty(title.as_ref()),
            }),
            Tag::Image {
                dest_url, title, ..
            } => {
                let destination = dest_url.to_string();
                let title = nonempty(title.as_ref());
                self.open_block(
                    BlockKind::Image {
                        destination: destination.clone(),
                        title: title.clone(),
                        width: None,
                    },
                    EndTag::Image,
                    &range,
                );
                self.push_inline(InlineKind::Image { destination, title });
            }
            _ => self.hidden(&range),
        }
    }

    fn end_tag(&mut self, tag: &TagEnd, range: Range<usize>) {
        match tag {
            TagEnd::Emphasis => self.pop_inline(&InlineKind::Emphasis, &range),
            TagEnd::Strong => self.pop_inline(&InlineKind::Strong, &range),
            TagEnd::Strikethrough => self.pop_inline(&InlineKind::Strikethrough, &range),
            TagEnd::Superscript => self.pop_inline(&InlineKind::Superscript, &range),
            TagEnd::Subscript => self.pop_inline(&InlineKind::Subscript, &range),
            TagEnd::Link => {
                self.pop_inline_matching(|kind| matches!(kind, InlineKind::Link { .. }), &range)
            }
            TagEnd::Image => {
                self.pop_inline_matching(|kind| matches!(kind, InlineKind::Image { .. }), &range);
                self.close_block(&EndTag::Image, &range);
            }
            TagEnd::CodeBlock => {
                self.pop_inline(&InlineKind::Code, &range);
                self.close_block(&EndTag::Code, &range);
            }
            TagEnd::Table => {
                self.close_block(&EndTag::Table, &range);
                self.in_table = false;
            }
            TagEnd::TableCell => self.hidden(&range),
            TagEnd::HtmlBlock => self.close_block(&EndTag::Raw, &range),
            TagEnd::FootnoteDefinition => self.close_block(&EndTag::Footnote, &range),
            _ => {
                if let Some(end_tag) = end_tag_for_block(tag) {
                    self.close_block(&end_tag, &range);
                } else {
                    self.hidden(&range);
                }
            }
        }
    }

    fn open_block(&mut self, kind: BlockKind, end_tag: EndTag, range: &Range<usize>) {
        self.open_blocks.push(OpenBlock {
            kind,
            end_tag,
            source_start: range.start,
            display_start: self.display_utf16,
            depth: self.open_blocks.len(),
        });
    }

    fn close_block(&mut self, end_tag: &EndTag, range: &Range<usize>) {
        self.hidden(range);
        let Some(index) = self
            .open_blocks
            .iter()
            .rposition(|open| &open.end_tag == end_tag)
        else {
            return;
        };
        let open = self.open_blocks.remove(index);
        self.blocks.push(ProjectionBlock {
            kind: open.kind,
            source: open.source_start..range.end.max(open.source_start),
            display_utf16: open.display_start..self.display_utf16,
            depth: open.depth,
        });
    }

    fn push_inline(&mut self, kind: InlineKind) {
        self.inline_stack.push(kind);
    }

    fn pop_inline(&mut self, kind: &InlineKind, range: &Range<usize>) {
        self.pop_inline_matching(|current| current == kind, range);
    }

    fn pop_inline_matching(&mut self, matches: impl Fn(&InlineKind) -> bool, range: &Range<usize>) {
        self.hidden(range);
        if let Some(index) = self.inline_stack.iter().rposition(matches) {
            self.inline_stack.remove(index);
        }
    }

    fn start_table_row(&mut self, range: &Range<usize>) {
        if self.table_rows > 0 {
            self.append_synthetic("\n", range.start);
        }
        self.table_rows += 1;
        self.table_cells = 0;
    }

    fn start_table_cell(&mut self, range: &Range<usize>) {
        if self.table_cells > 0 {
            self.append_synthetic("\t", range.start);
        }
        self.table_cells += 1;
    }

    fn current_kinds(&self) -> Vec<InlineKind> {
        if self.inline_stack.is_empty() {
            vec![InlineKind::Text]
        } else {
            self.inline_stack.clone()
        }
    }

    fn in_literal_context(&self) -> bool {
        self.inline_stack.contains(&InlineKind::Code)
            || self.open_blocks.iter().any(|block| {
                matches!(
                    block.kind,
                    BlockKind::Code { .. } | BlockKind::FrontMatter | BlockKind::Raw
                )
            })
    }

    fn append_visible(&mut self, visible: &str, range: Range<usize>, kinds: Vec<InlineKind>) {
        if range.start > range.end || range.end > self.source.len() {
            self.append_synthetic(visible, self.source_cursor);
            return;
        }
        let range = include_escape_prefix(self.source, range);
        let raw = self.source.get(range.clone()).unwrap_or_default();
        let tokens = align_visible_tokens(raw, range.start, visible);
        self.append_tokens(visible, range, kinds, &tokens);
    }

    fn append_text_with_autolinks(
        &mut self,
        visible: &str,
        range: Range<usize>,
        kinds: Vec<InlineKind>,
    ) {
        if kinds
            .iter()
            .any(|kind| matches!(kind, InlineKind::Link { .. } | InlineKind::Image { .. }))
        {
            self.append_visible(visible, range, kinds);
            return;
        }

        let source_range = include_escape_prefix(self.source, range.clone());
        let Some(raw) = self.source.get(source_range.clone()) else {
            self.append_visible(visible, range, kinds);
            return;
        };
        let tokens = align_visible_tokens(raw, source_range.start, visible);
        if !tokens_align_with_visible(&tokens, visible) {
            self.append_visible(visible, range, kinds);
            return;
        }

        let mut links = gfm_autolinks(visible);
        links.retain(|link| {
            source_range_for_display_range(&tokens, &link.range).is_some_and(|source| {
                !link.needs_url_boundary
                    || source_preceded_by_gfm_url_delimiter(self.source, source.start)
            })
        });
        if links.is_empty() {
            self.append_tokens(visible, source_range, kinds, &tokens);
            return;
        }

        let mut display_cursor = 0;
        for link in links {
            if link.range.start < display_cursor {
                continue;
            }
            self.append_text_segment(
                visible,
                &tokens,
                display_cursor..link.range.start,
                kinds.clone(),
            );
            let mut link_kinds = kinds.clone();
            link_kinds.push(InlineKind::Link {
                destination: link.destination,
                title: None,
            });
            self.append_text_segment(visible, &tokens, link.range.clone(), link_kinds);
            display_cursor = link.range.end;
        }
        self.append_text_segment(visible, &tokens, display_cursor..visible.len(), kinds);
    }

    fn append_text_segment(
        &mut self,
        visible: &str,
        tokens: &[(String, Range<usize>)],
        display_range: Range<usize>,
        kinds: Vec<InlineKind>,
    ) {
        if display_range.is_empty() {
            return;
        }
        let Some(text) = visible.get(display_range.clone()) else {
            return;
        };
        let Some(tokens) = token_slice_for_display_range(tokens, &display_range) else {
            return;
        };
        let Some(source) = source_range_for_tokens(tokens) else {
            return;
        };
        self.append_tokens(text, source, kinds, tokens);
    }

    fn append_literal(
        &mut self,
        visible: &str,
        range: Range<usize>,
        kinds: Vec<InlineKind>,
        newline: char,
    ) {
        if range.start > range.end || range.end > self.source.len() {
            self.append_synthetic(visible, self.source_cursor);
            return;
        }
        let raw = self.source.get(range.clone()).unwrap_or_default();
        let tokens = align_literal_tokens(raw, range.start, visible, newline);
        self.append_tokens(visible, range, kinds, &tokens);
    }

    fn append_inline_code(&mut self, visible: &str, range: Range<usize>, kinds: Vec<InlineKind>) {
        if range.start > range.end || range.end > self.source.len() {
            self.append_synthetic(visible, self.source_cursor);
            return;
        }
        let tokens = inline_code_tokens(self.source, range.clone(), visible);
        self.append_tokens(visible, range.clone(), kinds, &tokens);
        // Inline code has no Start/End events, so collapse the closing
        // backtick delimiter here after mapping each literal content scalar.
        self.hidden(&range);
    }

    fn append_tokens(
        &mut self,
        visible: &str,
        range: Range<usize>,
        kinds: Vec<InlineKind>,
        tokens: &[(String, Range<usize>)],
    ) {
        let display_start = self.display_utf16;
        if tokens.is_empty() {
            if !visible.is_empty() {
                self.append_chunk(visible, range.clone());
            }
            if self.display_utf16 > display_start {
                self.push_run(display_start, range, kinds);
            } else {
                self.hidden(&range);
            }
            return;
        }

        let mut run_display_start = self.display_utf16;
        let mut run_source_start = None;
        let mut run_source_end = None;
        for (text, source_range) in tokens {
            let transformed = self.source.get(source_range.clone()) != Some(text.as_str());
            if transformed
                && self.display_utf16 > run_display_start
                && let (Some(source_start), Some(source_end)) =
                    (run_source_start.take(), run_source_end.take())
            {
                self.push_run(run_display_start, source_start..source_end, kinds.clone());
            }

            let token_display_start = self.display_utf16;
            self.append_chunk(text, source_range.clone());
            if transformed {
                self.push_run(token_display_start, source_range.clone(), kinds.clone());
                run_display_start = self.display_utf16;
            } else {
                run_source_start.get_or_insert(source_range.start);
                run_source_end = Some(source_range.end);
            }
        }

        if self.display_utf16 > run_display_start
            && let (Some(source_start), Some(source_end)) = (run_source_start, run_source_end)
        {
            self.push_run(run_display_start, source_start..source_end, kinds);
        }
        if self.display_utf16 == display_start {
            self.hidden(&range);
        }
    }

    fn push_run(&mut self, display_start: usize, source: Range<usize>, kinds: Vec<InlineKind>) {
        if self.display_utf16 > display_start {
            self.runs.push(ProjectionRun {
                display_utf16: display_start..self.display_utf16,
                source,
                kinds,
            });
        }
    }

    fn append_synthetic(&mut self, text: &str, source_anchor: usize) {
        if text.is_empty() {
            return;
        }
        let anchor = source_anchor.min(self.source.len());
        self.set_current_boundary(self.source_cursor, anchor);
        let display_start = self.display_utf16;
        self.text.push_str(text);
        let units = text.encode_utf16().count();
        for _ in 0..units {
            self.boundaries.push(SourceBoundary {
                before: anchor,
                after: anchor,
            });
        }
        self.display_utf16 += units;
        self.source_cursor = anchor;
        self.runs.push(ProjectionRun {
            display_utf16: display_start..self.display_utf16,
            source: self.source_cursor..self.source_cursor,
            kinds: vec![InlineKind::TableSeparator],
        });
    }

    fn append_chunk(&mut self, text: &str, source: Range<usize>) {
        if text.is_empty() {
            return;
        }
        let display_start = self.display_utf16;
        let start = source.start.min(self.source.len());
        let end = source.end.min(self.source.len()).max(start);
        self.set_current_boundary(
            self.source_cursor,
            visible_source_start(self.source.get(start..end).unwrap_or_default(), start),
        );
        let scalar_count = text.chars().count();
        for (scalar_index, character) in text.chars().enumerate() {
            self.text.push(character);
            for _ in 1..character.len_utf16() {
                self.boundaries.push(SourceBoundary {
                    before: start,
                    after: end,
                });
            }
            let is_final_scalar = scalar_index + 1 == scalar_count;
            self.boundaries.push(if is_final_scalar {
                SourceBoundary {
                    before: end,
                    after: end,
                }
            } else {
                SourceBoundary {
                    before: start,
                    after: end,
                }
            });
            self.display_utf16 += character.len_utf16();
        }
        if self.source.get(start..end) != Some(text) {
            self.source_spans.push(SourceProjectionSpan {
                source: start..end,
                display_utf16: display_start..self.display_utf16,
            });
        }
        self.source_cursor = end;
    }

    fn set_current_boundary(&mut self, before: usize, after: usize) {
        if let Some(boundary) = self.boundaries.last_mut() {
            boundary.before = boundary.before.min(before);
            boundary.after = boundary.after.max(after);
        }
    }

    fn hidden(&mut self, range: &Range<usize>) {
        if range.start > range.end || range.end > self.source.len() {
            return;
        }
        self.set_current_boundary(self.source_cursor, range.end);
    }

    fn append_image_width_comment(&mut self, html: &str, range: &Range<usize>) -> bool {
        let Some(width) = parse_image_width_comment(html) else {
            return false;
        };
        let Some(block) = self.blocks.iter_mut().rev().find(|block| {
            matches!(block.kind, BlockKind::Image { .. })
                && block.source.end <= range.start
                && self
                    .source
                    .get(block.source.end..range.start)
                    .is_some_and(|gap| gap.chars().all(char::is_whitespace))
        }) else {
            return false;
        };
        if let BlockKind::Image {
            width: image_width, ..
        } = &mut block.kind
        {
            *image_width = Some(width);
        }
        block.source.end = range.end;
        self.hidden(range);
        true
    }

    fn append_legacy_image(&mut self, html: &str, range: &Range<usize>) -> bool {
        if self.inline_stack.contains(&InlineKind::Code)
            || self.open_blocks.iter().any(|b| {
                matches!(b.kind, BlockKind::Code { .. } | BlockKind::FrontMatter)
                    || b.end_tag == EndTag::Footnote
            })
        {
            return false;
        }
        let Some(image) = crate::image_reference::parse_legacy(html) else {
            return false;
        };
        let kind = BlockKind::Image {
            destination: image.destination.clone(),
            title: image.title.clone(),
            width: image.width,
        };
        let raw = self
            .open_blocks
            .last_mut()
            .filter(|b| b.end_tag == EndTag::Raw && b.source_start == range.start);
        let standalone = raw.is_some();
        if let Some(raw) = raw {
            raw.kind = kind;
        } else {
            self.open_block(kind, EndTag::Image, range);
        }
        self.append_visible(
            &image.alt,
            range.clone(),
            vec![InlineKind::Image {
                destination: image.destination,
                title: image.title,
            }],
        );
        if !standalone {
            self.close_block(&EndTag::Image, range);
        }
        true
    }

    fn finish(mut self) -> DisplayProjection {
        for open in self.open_blocks.drain(..) {
            self.blocks.push(ProjectionBlock {
                kind: open.kind,
                source: open.source_start..self.source.len(),
                display_utf16: open.display_start..self.display_utf16,
                depth: open.depth,
            });
        }
        if let Some(boundary) = self.boundaries.last_mut() {
            boundary.before = boundary.before.min(self.source_cursor);
            boundary.after = boundary.after.max(self.source.len());
        }
        self.blocks.sort_by(|left, right| {
            left.source
                .start
                .cmp(&right.source.start)
                .then_with(|| right.source.end.cmp(&left.source.end))
        });
        let editable_source_positions = editable_source_positions(self.source, &self.blocks);
        DisplayProjection {
            text: self.text,
            blocks: self.blocks,
            runs: self.runs,
            boundaries: self.boundaries,
            source_len: self.source.len(),
            editable_source_positions,
            source_boundary_bits: source_boundary_bits(self.source),
            source_spans: self.source_spans,
        }
    }
}

fn editable_source_positions(source: &str, blocks: &[ProjectionBlock]) -> Vec<usize> {
    let bytes = source.as_bytes();
    let mut positions = vec![0, source.len()];
    let mut offset = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            b'\r' if bytes.get(offset + 1) == Some(&b'\n') => {
                offset += 2;
                positions.push(offset);
            }
            b'\r' | b'\n' => {
                offset += 1;
                positions.push(offset);
            }
            _ => {
                let character = source[offset..]
                    .chars()
                    .next()
                    .expect("valid source boundary");
                offset += character.len_utf8();
            }
        }
    }
    for block in blocks {
        positions.extend([block.source.start, block.source.end]);
    }
    positions.retain(|&position| {
        source.is_char_boundary(position)
            && !(position > 0
                && position < bytes.len()
                && bytes[position - 1] == b'\r'
                && bytes[position] == b'\n')
    });
    positions.sort_unstable();
    positions.dedup();
    positions
}

fn end_tag_for_block(tag: &TagEnd) -> Option<EndTag> {
    match tag {
        TagEnd::Paragraph => Some(EndTag::Paragraph),
        TagEnd::Heading(_) => Some(EndTag::Heading),
        TagEnd::List(_) => Some(EndTag::List),
        TagEnd::Item => Some(EndTag::ListItem),
        TagEnd::BlockQuote(_) => Some(EndTag::Quote),
        TagEnd::MetadataBlock(_) => Some(EndTag::FrontMatter),
        _ => None,
    }
}

fn heading_level(level: &HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn code_block_info(kind: &CodeBlockKind<'_>) -> (bool, Option<String>) {
    match kind {
        CodeBlockKind::Fenced(info) => {
            let language = info
                .split_whitespace()
                .next()
                .filter(|value| !value.is_empty())
                .map(str::to_owned);
            (true, language)
        }
        CodeBlockKind::Indented => (false, None),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_image_width_comment(html: &str) -> Option<u32> {
    let html = html.trim();
    let body = html.strip_prefix("<!--")?.strip_suffix("-->")?.trim();
    let width = body.strip_prefix("mdlite-width:")?.trim().parse().ok()?;
    (width > 0).then_some(width)
}

fn source_boundary_bits(source: &str) -> Vec<u64> {
    let mut bits = vec![0; source.len() / 64 + 1];
    for (byte, _) in source
        .char_indices()
        .chain(std::iter::once((source.len(), '\0')))
    {
        bits[byte / 64] |= 1 << (byte % 64);
    }
    bits
}

fn align_visible_tokens(raw: &str, base: usize, visible: &str) -> Vec<(String, Range<usize>)> {
    if visible.is_empty() {
        return Vec::new();
    }
    align_tokens(source_tokens(raw, base), base..base + raw.len(), visible)
}

fn align_literal_tokens(
    raw: &str,
    base: usize,
    visible: &str,
    newline: char,
) -> Vec<(String, Range<usize>)> {
    if visible.is_empty() {
        return Vec::new();
    }
    align_tokens(
        literal_source_tokens(raw, base, newline),
        base..base + raw.len(),
        visible,
    )
}

fn align_tokens(
    raw_tokens: Vec<(String, Range<usize>)>,
    fallback_range: Range<usize>,
    visible: &str,
) -> Vec<(String, Range<usize>)> {
    let mut visible_offset = 0;
    let mut mapped = Vec::new();
    for (token, range) in raw_tokens {
        if visible[visible_offset..].starts_with(&token) {
            visible_offset += token.len();
            mapped.push((token, range));
            if visible_offset == visible.len() {
                break;
            }
        }
    }
    if visible_offset < visible.len() {
        // Parser-normalized whitespace and uncommon entity forms still get a
        // deterministic source anchor. Common characters, escapes and HTML
        // entities above retain their exact individual source spans.
        mapped.push((visible[visible_offset..].to_owned(), fallback_range));
    }
    mapped
}

fn tokens_align_with_visible(tokens: &[(String, Range<usize>)], visible: &str) -> bool {
    let mut offset = 0;
    for (token, _) in tokens {
        let Some(rest) = visible.get(offset..) else {
            return false;
        };
        if !rest.starts_with(token) {
            return false;
        }
        offset += token.len();
    }
    offset == visible.len()
}

fn token_slice_for_display_range<'a>(
    tokens: &'a [(String, Range<usize>)],
    display_range: &Range<usize>,
) -> Option<&'a [(String, Range<usize>)]> {
    if display_range.start > display_range.end {
        return None;
    }
    let mut display_offset = 0usize;
    let mut covered_end = display_range.start;
    let mut selected_start = None;
    let mut selected_end = None;
    for (index, token) in tokens.iter().enumerate() {
        let token_end = display_offset.checked_add(token.0.len())?;
        if token_end <= display_range.start {
            display_offset = token_end;
            continue;
        }
        if display_offset >= display_range.end {
            selected_end = Some(index);
            break;
        }
        if display_offset < display_range.start || token_end > display_range.end {
            return None;
        }
        selected_start.get_or_insert(index);
        covered_end = token_end;
        display_offset = token_end;
    }
    if covered_end != display_range.end {
        return None;
    }
    let start = selected_start?;
    let end = selected_end.unwrap_or(tokens.len());
    Some(&tokens[start..end])
}

fn source_range_for_tokens(tokens: &[(String, Range<usize>)]) -> Option<Range<usize>> {
    Some(tokens.first()?.1.start..tokens.last()?.1.end)
}

fn source_range_for_display_range(
    tokens: &[(String, Range<usize>)],
    display_range: &Range<usize>,
) -> Option<Range<usize>> {
    source_range_for_tokens(token_slice_for_display_range(tokens, display_range)?)
}

fn source_preceded_by_gfm_url_delimiter(source: &str, source_byte: usize) -> bool {
    let Some(prefix) = source.get(..source_byte) else {
        return false;
    };
    let Some(previous) = prefix.chars().next_back() else {
        return true;
    };
    if previous.is_whitespace() || matches!(previous, '*' | '_' | '~' | '(') {
        return true;
    }

    // A blockquote marker is syntax rather than visible text, so a URL at the
    // first text position in `>https://...` is still at the start of its line.
    let line_prefix = prefix.rsplit(['\r', '\n']).next().unwrap_or_default();
    let mut markers = line_prefix.trim_start();
    let mut saw_marker = false;
    while let Some(rest) = markers.strip_prefix('>') {
        saw_marker = true;
        markers = rest.trim_start_matches([' ', '\t']);
    }
    saw_marker && markers.is_empty()
}

fn include_escape_prefix(source: &str, range: Range<usize>) -> Range<usize> {
    if range.start > range.end || range.end > source.len() {
        return range;
    }
    let Some(first) = source
        .get(range.start..range.end)
        .and_then(|text| text.chars().next())
    else {
        return range;
    };
    if !first.is_ascii_punctuation() || first == '\\' {
        return range;
    }
    let mut start = range.start;
    while start > 0 && source.as_bytes().get(start - 1) == Some(&b'\\') {
        start -= 1;
    }
    let slash_count = range.start - start;
    if slash_count.is_multiple_of(2) {
        range
    } else {
        start..range.end
    }
}

fn visible_source_start(raw: &str, source_start: usize) -> usize {
    let Some(escaped) = raw.strip_prefix('\\') else {
        return source_start;
    };
    if escaped
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_punctuation())
    {
        source_start + 1
    } else {
        source_start
    }
}

fn literal_source_tokens(raw: &str, base: usize, newline: char) -> Vec<(String, Range<usize>)> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let Some(rest) = raw.get(offset..) else {
            break;
        };
        if rest.starts_with("\r\n") {
            tokens.push((newline.to_string(), base + offset..base + offset + 2));
            offset += 2;
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        let end = offset + character.len_utf8();
        if matches!(character, '\r' | '\n') {
            tokens.push((newline.to_string(), base + offset..base + end));
        } else {
            tokens.push((character.to_string(), base + offset..base + end));
        }
        offset = end;
    }
    tokens
}

// pulldown-cmark emits one space for each CR or LF scalar in inline code,
// including both bytes of a CRLF pair; keep that event text and map each space
// to its corresponding source scalar.
fn inline_code_source_tokens(raw: &str, base: usize) -> Vec<(String, Range<usize>)> {
    raw.char_indices()
        .map(|(offset, character)| {
            let end = offset + character.len_utf8();
            let value = if matches!(character, '\r' | '\n') {
                " ".to_owned()
            } else {
                character.to_string()
            };
            (value, base + offset..base + end)
        })
        .collect()
}

fn inline_code_tokens(
    source: &str,
    range: Range<usize>,
    visible: &str,
) -> Vec<(String, Range<usize>)> {
    let Some(raw) = source.get(range.clone()) else {
        return Vec::new();
    };
    let opening = raw.bytes().take_while(|byte| *byte == b'`').count();
    let closing = raw.bytes().rev().take_while(|byte| *byte == b'`').count();
    let has_delimiters = opening > 0 && opening == closing && opening + closing <= raw.len();
    let content_start = range.start + usize::from(has_delimiters) * opening;
    let content_end = range.end - usize::from(has_delimiters) * closing;
    let content = source.get(content_start..content_end).unwrap_or_default();
    let mut tokens = inline_code_source_tokens(content, content_start);
    let normalized: String = tokens.iter().map(|(text, _)| text.as_str()).collect();
    if normalized.starts_with(' ')
        && normalized.ends_with(' ')
        && normalized.chars().any(|character| character != ' ')
    {
        if tokens.first().is_some_and(|(text, _)| text == " ") {
            tokens.remove(0);
        }
        if tokens.last().is_some_and(|(text, _)| text == " ") {
            tokens.pop();
        }
    }
    let normalized: String = tokens.iter().map(|(text, _)| text.as_str()).collect();
    if normalized == visible {
        tokens
    } else {
        align_tokens(tokens, content_start..content_end, visible)
    }
}

fn source_tokens(raw: &str, base: usize) -> Vec<(String, Range<usize>)> {
    let mut tokens = Vec::new();
    let mut offset = 0;
    while offset < raw.len() {
        let Some(rest) = raw.get(offset..) else {
            break;
        };
        if rest.starts_with('\\') {
            let after_slash = offset + 1;
            if let Some(character) = raw.get(after_slash..).and_then(|tail| tail.chars().next())
                && character.is_ascii_punctuation()
            {
                let end = after_slash + character.len_utf8();
                tokens.push((character.to_string(), base + offset..base + end));
                offset = end;
                continue;
            }
        }
        if rest.starts_with('&')
            && let Some((decoded, end)) = entity_at(raw, offset)
        {
            tokens.push((decoded, base + offset..base + end));
            offset = end;
            continue;
        }
        let Some(character) = rest.chars().next() else {
            break;
        };
        let end = offset + character.len_utf8();
        tokens.push((character.to_string(), base + offset..base + end));
        offset = end;
    }
    tokens
}

fn entity_at(raw: &str, start: usize) -> Option<(String, usize)> {
    let rest = raw.get(start..)?;
    let mut end = start + 1;
    for character in rest[1..].chars().take(40) {
        if !(character.is_ascii_alphanumeric() || matches!(character, '#' | ';')) {
            break;
        }
        end += character.len_utf8();
        if character == ';' {
            break;
        }
    }
    if end == start + 1 {
        return None;
    }
    let candidate = raw.get(start..end)?;
    let decoded = Parser::new(candidate).find_map(|event| match event {
        Event::Text(text) => Some(text.to_string()),
        _ => None,
    })?;
    (decoded != candidate).then_some((decoded, end))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pulldown_cmark::Options;

    const DIALECT_FIXTURE: &str = include_str!("../tests/fixtures/markdown_dialect.md");

    fn project_with_options(source: &str, options: Options) -> DisplayProjection {
        let mut builder = ProjectionBuilder::new(source);
        for (event, range) in Parser::new_ext(source, options).into_offset_iter() {
            builder.process(event, range);
        }
        builder.finish()
    }

    #[test]
    fn editable_source_positions_keep_blank_lines_soft_breaks_and_fence_edges() {
        let source = "before\r\nsecond\r\n\r\n```rs\r\nlet x = 1;\r\n```\r\n\r\n## ";
        let projection = DisplayProjection::parse(source);
        let positions = &projection.editable_source_positions;
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(positions.iter().all(|&position| {
            source.is_char_boundary(position)
                && !(position > 0
                    && position < source.len()
                    && source.as_bytes()[position - 1] == b'\r'
                    && source.as_bytes()[position] == b'\n')
        }));
        for (start, ending) in source.match_indices("\r\n") {
            assert!(!positions.contains(&(start + 1)));
            assert!(positions.contains(&(start + ending.len())));
        }

        let first_break_end = source.find("\r\n").unwrap() + 2;
        assert!(positions.contains(&first_break_end));
        assert!(projection.text.contains("before second"));
        let code = projection
            .blocks
            .iter()
            .find(|block| matches!(block.kind, BlockKind::Code { fenced: true, .. }))
            .expect("fenced code block");
        assert!(positions.contains(&code.source.start));
        assert!(positions.contains(&code.source.end));
        assert!(positions.contains(&source.len()));
        assert_eq!(projection.source_len, source.len());
    }

    #[test]
    fn projects_blocks_inline_formatting_and_image_metadata_without_changing_source() {
        let source = "---\ntitle: 日本語\n---\n\n# **見出し**\n\n> 引用\n\n- 項目\n\n```rust\nlet x = 1;\n```\n\n![猫](cat.png)<!-- mdlite-width:320 -->";
        let projection = DisplayProjection::parse(source);
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::FrontMatter)
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Heading { level: 1 })
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Quote)
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::List { ordered: None })
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| matches!(block.kind, BlockKind::Code { fenced: true, .. }))
        );
        let image = projection
            .blocks
            .iter()
            .find(|block| matches!(block.kind, BlockKind::Image { .. }))
            .expect("image block");
        assert!(
            matches!(&image.kind, BlockKind::Image { destination, width: Some(320), .. } if destination == "cat.png")
        );
        assert_eq!(
            &source[image.source.clone()],
            "![猫](cat.png)<!-- mdlite-width:320 -->"
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Strong))
        );
        assert!(projection.text.contains("見出し"));
        assert!(!projection.text.contains("mdlite-width"));
        assert_eq!(
            source,
            "---\ntitle: 日本語\n---\n\n# **見出し**\n\n> 引用\n\n- 項目\n\n```rust\nlet x = 1;\n```\n\n![猫](cat.png)<!-- mdlite-width:320 -->"
        );
    }

    #[test]
    fn thematic_breaks_keep_one_visible_atomic_source_mapped_marker() {
        for syntax in ["***", "---", "___"] {
            let source = format!("before\n\n{syntax}\n\nafter");
            let projection = DisplayProjection::parse(&source);
            let block = projection
                .blocks
                .iter()
                .find(|block| matches!(block.kind, BlockKind::ThematicBreak))
                .expect("thematic-break block");
            assert!(source[block.source.clone()].starts_with(syntax));
            let text_before = projection.text[..]
                .split_once('─')
                .map(|(before, _)| before.encode_utf16().count())
                .expect("visible thematic-break marker");
            assert_eq!(block.display_utf16, text_before..text_before + 1);
            assert_eq!(projection.text.matches('─').count(), 1);
            let run = projection
                .runs
                .iter()
                .find(|run| run.display_utf16 == block.display_utf16)
                .expect("source-backed marker run");
            assert_eq!(&source[run.source.clone()], syntax);
        }
    }

    #[test]
    fn thematic_break_projection_leaves_literal_contexts_and_table_delimiters_alone() {
        let source = concat!(
            "---\ntitle: ---\n---\n\n",
            "```md\n---\n***\n```\n\n",
            "`---` and \\***\n\n",
            "| A | B |\n| --- | --- |\n| x --- y | `---` |\n",
        );
        let projection = DisplayProjection::parse(source);
        assert!(
            projection
                .blocks
                .iter()
                .all(|block| !matches!(block.kind, BlockKind::ThematicBreak))
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| matches!(block.kind, BlockKind::FrontMatter))
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| matches!(block.kind, BlockKind::Table))
        );
        assert!(projection.text.contains("title: ---"));
        assert!(projection.text.contains("---\n***"));
        assert!(projection.text.contains("x --- y"));
    }

    #[test]
    fn maps_utf16_unicode_escapes_entities_and_hidden_markdown_with_affinity() {
        let source = "# **A😺** &amp; [日本](docs/a.md)";
        let projection = DisplayProjection::parse(source);
        assert_eq!(projection.text, "A😺 & 日本");
        assert_eq!(
            projection.boundaries.len(),
            projection.text.encode_utf16().count() + 1
        );
        let a_start = source.find('A').expect("A");
        assert_eq!(
            projection.source_byte_at_utf16(0, BoundaryAffinity::Before),
            Some(0)
        );
        assert_eq!(
            projection.source_byte_at_utf16(0, BoundaryAffinity::After),
            Some(a_start)
        );
        assert_eq!(
            projection.utf16_at_source_byte(a_start, BoundaryAffinity::After),
            Some(0)
        );
        let emoji_start = source.find("😺").expect("emoji");
        assert!(
            projection
                .utf16_at_source_byte(emoji_start + 1, BoundaryAffinity::After)
                .is_none()
        );

        let amp_display_end = 5;
        let entity_start = source.find("&amp;").expect("entity");
        assert_eq!(
            projection.source_byte_at_utf16(amp_display_end, BoundaryAffinity::After),
            Some(entity_start + "&amp;".len())
        );
        assert_eq!(
            projection.utf16_at_source_byte(entity_start + 2, BoundaryAffinity::Before),
            Some(4)
        );
        assert_eq!(
            projection.utf16_at_source_byte(entity_start + 2, BoundaryAffinity::After),
            Some(5)
        );
        assert!(projection.runs.iter().any(|run| {
            run.source == (entity_start..entity_start + "&amp;".len())
                && run.display_utf16 == (4..5)
        }));
        let label_start = source.find("日本").expect("label");
        assert_eq!(
            projection.utf16_at_source_byte(label_start, BoundaryAffinity::After),
            Some(6)
        );
        assert_eq!(
            projection.source_byte_at_utf16(3, BoundaryAffinity::Before),
            Some(source.find("😺").expect("emoji") + "😺".len())
        );
        assert_eq!(
            projection.source_byte_at_utf16(3, BoundaryAffinity::After),
            Some(source.find("** &amp;").expect("closing strong") + 2)
        );
        let label_end = label_start + "日本".len();
        assert_eq!(
            projection.source_byte_at_utf16(8, BoundaryAffinity::Before),
            Some(label_end)
        );
        assert_eq!(
            projection.source_byte_at_utf16(8, BoundaryAffinity::After),
            Some(source.len())
        );
    }

    #[test]
    fn projects_table_cells_with_source_affine_separators_and_keeps_malformed_source_literal() {
        let source = "before\n| **h** | b |\n| --- | --- |\n| a\\|b | &amp; |\n\nafter";
        let projection = DisplayProjection::parse(source);
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Table)
        );
        assert!(projection.text.contains("h\tb\na|b\t&"));
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Strong))
        );
        let displayed_pipe = projection.text.find("a|b").expect("visible pipe") + 1;
        let source_escape = source.find("a\\|b").expect("escaped pipe") + 1;
        assert_eq!(
            projection.source_byte_at_utf16(displayed_pipe, BoundaryAffinity::Before),
            Some(source_escape)
        );
        assert_eq!(
            projection.source_byte_at_utf16(displayed_pipe, BoundaryAffinity::After),
            Some(source_escape + 1)
        );
        let displayed_amp_end = projection.text.find('&').expect("entity") + 1;
        let source_amp_end = source.find("&amp;").expect("source entity") + "&amp;".len();
        assert_eq!(
            projection.source_byte_at_utf16(displayed_amp_end, BoundaryAffinity::Before),
            Some(source_amp_end)
        );
        let escape_start = source.find("a\\|b").expect("escaped cell") + 1;
        let escaped_run = projection
            .runs
            .iter()
            .find(|run| run.display_utf16.start == displayed_pipe)
            .expect("escaped text run");
        assert_eq!(escaped_run.source, escape_start..escape_start + 2);
        let malformed = "| heading | other |\n| --- |\n| value | other |";
        let malformed_projection = DisplayProjection::parse(malformed);
        assert!(malformed_projection.text.contains("heading"));
        assert!(
            !malformed_projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Table)
        );
        assert_eq!(malformed_projection.source_len, malformed.len());
    }

    #[test]
    fn inline_and_block_code_and_front_matter_keep_escape_and_entity_spelling_literal() {
        let inline_source = "`a\\*b &amp;`";
        let inline = DisplayProjection::parse(inline_source);
        assert_eq!(inline.text, "a\\*b &amp;");
        let inline_run = inline
            .runs
            .iter()
            .find(|run| run.kinds.contains(&InlineKind::Code))
            .expect("inline code run");
        assert_eq!(inline_run.source, 1..inline_source.len() - 1);
        let inline_star = inline.text.find('*').expect("literal star");
        let source_star = inline_source.find('*').expect("source star");
        assert_eq!(
            inline.source_byte_at_utf16(inline_star, BoundaryAffinity::After),
            Some(source_star)
        );
        assert_eq!(
            inline.utf16_at_source_byte(source_star, BoundaryAffinity::After),
            Some(inline_star)
        );

        let fenced_source = "```txt\na\\*b &amp;\n```";
        let fenced = DisplayProjection::parse(fenced_source);
        assert!(fenced.text.contains("a\\*b &amp;"));
        let fenced_star = fenced.text.find('*').expect("fenced literal star");
        assert_eq!(
            fenced.source_byte_at_utf16(fenced_star, BoundaryAffinity::After),
            Some(fenced_source.find('*').expect("fenced source star"))
        );

        let front_matter_source = "---\nvalue: a\\* &amp;\n---\n# body";
        let front_matter = DisplayProjection::parse(front_matter_source);
        assert!(front_matter.text.contains("a\\* &amp;"));
        let front_matter_amp = front_matter.text.find("&amp;").expect("literal entity");
        assert_eq!(
            front_matter.source_byte_at_utf16(front_matter_amp, BoundaryAffinity::After),
            Some(front_matter_source.find("&amp;").expect("source entity"))
        );
    }

    #[test]
    fn inline_code_crlf_spaces_map_to_their_source_scalars() {
        let source = "`a\r\nb`";
        let projection = DisplayProjection::parse(source);
        assert_eq!(projection.text, "a  b");
        let space = 1;
        let newline_start = source.find("\r\n").expect("CRLF");
        assert_eq!(
            projection.source_byte_at_utf16(space, BoundaryAffinity::Before),
            Some(newline_start)
        );
        assert_eq!(
            projection.source_byte_at_utf16(space + 1, BoundaryAffinity::Before),
            Some(newline_start + 1)
        );
        assert_eq!(
            projection.source_byte_at_utf16(space + 2, BoundaryAffinity::Before),
            Some(newline_start + 2)
        );
        assert_eq!(
            projection.utf16_at_source_byte(newline_start, BoundaryAffinity::Before),
            Some(space)
        );
        assert_eq!(
            projection.utf16_at_source_byte(newline_start + 1, BoundaryAffinity::Before),
            Some(space + 1)
        );
    }

    #[test]
    fn adopted_commonmark_and_gfm_fixture_keeps_source_ranges_for_native_editing() {
        let projection = DisplayProjection::parse(DIALECT_FIXTURE);
        assert_eq!(projection.source_len, DIALECT_FIXTURE.len());
        assert!(projection.blocks.iter().any(|block| {
            block.kind == BlockKind::FrontMatter
                && DIALECT_FIXTURE
                    .get(block.source.clone())
                    .is_some_and(|source| {
                        source.starts_with("---\n") && source.trim_end().ends_with("...")
                    })
        }));
        for level in 1..=6 {
            assert!(
                projection
                    .blocks
                    .iter()
                    .any(|block| block.kind == BlockKind::Heading { level })
            );
        }
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| { matches!(block.kind, BlockKind::List { ordered: None }) })
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| { matches!(block.kind, BlockKind::List { ordered: Some(1) }) })
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Quote)
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| { matches!(block.kind, BlockKind::Code { fenced: true, .. }) })
        );
        assert!(projection.text.contains("title: `literal` &amp; source"));
        assert!(projection.text.contains("reference link.\nThe next line"));
        assert!(projection.text.contains("code span\ta|b\tbold"));
        assert!(
            projection
                .text
                .contains("[!NOTE] This remains ordinary quote text.")
        );
        assert!(
            projection
                .text
                .contains("<script>window.alert(\"literal\")</script>")
        );
        assert!(projection.text.contains("[unfinished **nested markup"));
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Strong))
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Emphasis))
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Code))
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::Strikethrough))
        );
        let tasks = projection
            .runs
            .iter()
            .filter_map(|run| {
                run.kinds.iter().find_map(|kind| match kind {
                    InlineKind::TaskMarker { checked } => Some((run, *checked)),
                    _ => None,
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(tasks.len(), 2);
        assert!(!tasks[0].1);
        assert!(tasks[1].1);
        assert_eq!(DIALECT_FIXTURE.get(tasks[0].0.source.clone()), Some("[ ]"));
        assert_eq!(DIALECT_FIXTURE.get(tasks[1].0.source.clone()), Some("[x]"));

        let destinations = projection
            .runs
            .iter()
            .flat_map(|run| run.kinds.iter())
            .filter_map(|kind| match kind {
                InlineKind::Link { destination, .. } => Some(destination.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert!(destinations.contains(&"https://example.org/inline"));
        assert!(destinations.contains(&"https://example.org/reference"));
        assert!(destinations.contains(&"https://example.org/path_(part)"));
        assert!(destinations.contains(&"http://www.example.org/path"));
        assert!(destinations.contains(&"mailto:help+tag@example.org"));
        for url in [
            "https://example.org/inline-code",
            "https://example.org/fenced-code",
        ] {
            let source_byte = DIALECT_FIXTURE.find(url).expect("code URL");
            assert!(!projection.runs.iter().any(|run| {
                run.source.start <= source_byte
                    && source_byte < run.source.end
                    && run
                        .kinds
                        .iter()
                        .any(|kind| matches!(kind, InlineKind::Link { .. }))
            }));
        }

        let tables = crate::table::find_tables(DIALECT_FIXTURE);
        assert_eq!(tables.len(), 1);
        assert_eq!(
            tables[0].alignments,
            [
                crate::table::TableAlignment::Left,
                crate::table::TableAlignment::Center,
                crate::table::TableAlignment::Right,
            ]
        );
        assert_eq!(
            DIALECT_FIXTURE.get(tables[0].rows[1].cells[0].content_range.clone()),
            Some("`code span`")
        );
        assert_eq!(
            DIALECT_FIXTURE.get(tables[0].rows[1].cells[1].content_range.clone()),
            Some("a\\|b")
        );
    }

    #[test]
    fn gfm_autolinks_keep_entity_and_escape_source_spelling_and_offsets() {
        let source =
            "URL https://example.org/a&amp;b, www.example.org/path. and user+tag@example.org.";
        let projection = DisplayProjection::parse(source);
        assert!(projection.text.contains("https://example.org/a&b"));

        let raw_url = "https://example.org/a&amp;b";
        let raw_start = source.find(raw_url).expect("raw URL");
        let link_runs = projection
            .runs
            .iter()
            .filter(|run| {
                run.kinds.iter().any(|kind| {
                    matches!(kind, InlineKind::Link { destination, .. } if destination == "https://example.org/a&b")
                })
            })
            .collect::<Vec<_>>();
        let first = link_runs.first().expect("source-mapped GFM URL");
        let last = link_runs.last().expect("source-mapped GFM URL");
        let source_range = first.source.start..last.source.end;
        let display_range = first.display_utf16.start..last.display_utf16.end;
        assert_eq!(source_range, raw_start..raw_start + raw_url.len());
        assert_eq!(
            projection.source_byte_at_utf16(display_range.start, BoundaryAffinity::After),
            Some(source_range.start)
        );
        assert_eq!(
            projection.source_byte_at_utf16(display_range.end, BoundaryAffinity::Before),
            Some(source_range.end)
        );
        assert_eq!(
            projection.utf16_at_source_byte(source_range.start, BoundaryAffinity::After),
            Some(display_range.start)
        );

        let no_match = concat!(
            "prefixhttps://example.org, ftp://example.org, example.org, ",
            ")https://example.org, ,www.example.org"
        );
        let projection = DisplayProjection::parse(no_match);
        assert!(!projection.runs.iter().any(|run| {
            run.kinds
                .iter()
                .any(|kind| matches!(kind, InlineKind::Link { .. }))
        }));

        for source in ["(https://example.org", ">https://example.org"] {
            let projection = DisplayProjection::parse(source);
            assert!(projection.runs.iter().any(|run| {
                run.kinds.iter().any(|kind| {
                    matches!(kind, InlineKind::Link { destination, .. } if destination == "https://example.org")
                })
            }));
        }
    }

    #[test]
    fn gfm_autolink_source_range_includes_markdown_escape_spelling() {
        let source = r"Escaped https://example.org/a\*b.";
        let projection = DisplayProjection::parse(source);
        let raw_url = r"https://example.org/a\*b";
        let start = source.find(raw_url).expect("escaped URL source");
        let link_runs = projection
            .runs
            .iter()
            .filter(|run| {
                run.kinds.iter().any(|kind| {
                    matches!(kind, InlineKind::Link { destination, .. } if destination == "https://example.org/a*b")
                })
            })
            .collect::<Vec<_>>();
        let first = link_runs.first().expect("escaped URL link run");
        let last = link_runs.last().expect("escaped URL link run");
        assert_eq!(
            first.source.start..last.source.end,
            start..start + raw_url.len()
        );
        assert!(projection.text.contains("https://example.org/a*b"));
    }

    #[test]
    fn front_matter_needs_a_complete_boundary_and_uses_literal_source_text() {
        for (closing, closing_text) in [("---", "---"), ("...", "...")] {
            let source = format!("---\r\ntitle: `raw` &amp;\r\n{closing}\r\n# body");
            let projection = DisplayProjection::parse(&source);
            let block = projection
                .blocks
                .iter()
                .find(|block| block.kind == BlockKind::FrontMatter)
                .expect("closed YAML front matter");
            let raw = source
                .get(block.source.clone())
                .expect("front matter source");
            assert!(raw.starts_with("---\r\n"));
            assert!(raw.trim_end().ends_with(closing_text));
            assert!(projection.text.contains("title: `raw` &amp;"));
            assert!(!projection.runs.iter().any(|run| {
                run.kinds
                    .iter()
                    .any(|kind| matches!(kind, InlineKind::Link { .. }))
            }));
        }

        for source in [
            "---\ntitle: not closed\n# body\n",
            "---\ntitle: ambiguous\n--- trailing text\n# body\n",
            "body\n---\ntitle: not at document start\n---\n",
        ] {
            let projection = DisplayProjection::parse(source);
            assert!(
                !projection
                    .blocks
                    .iter()
                    .any(|block| block.kind == BlockKind::FrontMatter)
            );
            assert_eq!(projection.source_len, source.len());
        }
    }

    #[test]
    fn deferred_extensions_and_smart_punctuation_stay_literal() {
        let punctuation = "\"straight\" 'quotes' ... -- remain literal";
        assert_eq!(DisplayProjection::parse(punctuation).text, punctuation);

        let source = concat!(
            "[^note] and [^note]: definition\n\n",
            "x^2^ and H~2O~ and [[Wiki Page]]\n\n",
            "$$x^2$$\n\n",
            "+++\nkey: value\n+++\n\n",
            "# Heading {#id .class}\n\n",
            "> [!NOTE] ordinary quote text"
        );
        let projection = DisplayProjection::parse(source);
        assert!(projection.text.contains("[^note]"));
        assert!(projection.text.contains("x^2^ and H~2O~ and [[Wiki Page]]"));
        assert!(projection.text.contains("$$x^2$$"));
        assert!(projection.text.contains("+++"));
        assert!(projection.text.contains("Heading {#id .class}"));
        assert!(projection.text.contains("[!NOTE] ordinary quote text"));
        assert!(
            !projection
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::FrontMatter)
        );
        assert!(!projection.runs.iter().any(|run| {
            run.kinds.iter().any(|kind| {
                matches!(
                    kind,
                    InlineKind::Math
                        | InlineKind::FootnoteReference
                        | InlineKind::Superscript
                        | InlineKind::Subscript
                )
            })
        }));
    }

    #[test]
    fn footnotes_and_alerts_are_deferred_until_their_markers_are_source_safe() {
        let footnotes = "A reference[^1].\n\n[^1]: Footnote body";
        // The current projection displays only the parsed label and treats the
        // definition as a raw block, hiding the Markdown marker spellings.
        let enabled = project_with_options(footnotes, Options::ENABLE_FOOTNOTES);
        assert!(
            enabled
                .runs
                .iter()
                .any(|run| run.kinds.contains(&InlineKind::FootnoteReference))
        );
        assert!(!enabled.text.contains("[^1]"));
        assert!(!enabled.text.contains("[^1]:"));
        assert!(
            enabled
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Raw)
        );
        let deferred = DisplayProjection::parse(footnotes);
        assert!(deferred.text.contains("[^1]"));
        assert!(deferred.text.contains("[^1]: Footnote body"));
        assert_eq!(deferred.source_len, footnotes.len());

        let alert = "> [!NOTE]\n> Alert body\n";
        // ENABLE_GFM turns [!NOTE] into BlockQuoteKind metadata, but the
        // current projection only retains the generic Quote block kind.
        let enabled = project_with_options(alert, Options::ENABLE_GFM);
        assert!(!enabled.text.contains("[!NOTE]"));
        let deferred = DisplayProjection::parse(alert);
        assert!(deferred.text.contains("[!NOTE]"));
        assert!(
            deferred
                .blocks
                .iter()
                .any(|block| block.kind == BlockKind::Quote)
        );
        assert_eq!(deferred.source_len, alert.len());
    }

    #[test]
    fn table_discovery_uses_the_same_front_matter_boundaries_as_projection() {
        let source = concat!(
            "---\n",
            "| YAML key | YAML value |\n",
            "| --- | --- |\n",
            "...\n\n",
            "| Header | Value |\n",
            "| --- | --- |\n",
            "| body | table |\n"
        );
        let projection = DisplayProjection::parse(source);
        let front_matter = projection
            .blocks
            .iter()
            .find(|block| block.kind == BlockKind::FrontMatter)
            .expect("front matter");
        let tables = crate::table::find_tables(source);
        assert_eq!(tables.len(), 1);
        assert!(tables[0].range.start >= front_matter.source.end);
        let table_blocks = projection
            .blocks
            .iter()
            .filter(|block| block.kind == BlockKind::Table)
            .collect::<Vec<_>>();
        assert_eq!(table_blocks.len(), 1);
        assert!(table_blocks[0].source.start >= front_matter.source.end);
        assert!(
            source
                .get(front_matter.source.clone())
                .is_some_and(|raw| raw.contains("YAML key"))
        );
    }
}
