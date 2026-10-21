//! Cached-projection styling and the bounded native editor status row.

use super::*;
use mdlite::{
    document::Document,
    projection::{BlockKind, DisplayProjection, InlineKind, ProjectionRun},
};
use std::ops::Range;

const STATUS_MAX_CHARS: usize = 260;
const MAX_INLINE_CODE_RECTS: usize = 128;
const LINE_HEIGHT_EM: f32 = 1.6;
const LINE_BASELINE_EM: f32 = 1.25;
const HEADING_GAP_EM: f32 = 0.12;

pub(super) struct StatusField {
    pub text: String,
    pub width: f32,
    priority: u8,
}

/// Fixed information owns its own cell; optional cells disappear before the
/// caret and save state. Positions never depend on a transient message length.
fn status_cells(fields: &[StatusField], width: f32) -> Vec<(usize, f32, f32)> {
    let mut remaining = width.max(0.0);
    let mut included = vec![false; fields.len()];
    for priority in 0..=7 {
        for (index, field) in fields.iter().enumerate() {
            if field.priority == priority && field.width <= remaining {
                included[index] = true;
                remaining -= field.width;
            }
        }
    }
    let mut right = width.max(0.0);
    let mut cells = Vec::new();
    for (index, field) in fields
        .iter()
        .enumerate()
        .rev()
        .filter(|(index, _)| *index >= 2)
    {
        if included[index] {
            right -= field.width;
            cells.push((index, right, field.width));
        }
    }
    cells.reverse();
    let mut left = 0.;
    for (index, field) in fields.iter().take(2).enumerate() {
        if included[index] {
            cells.insert(index.min(cells.len()), (index, left, field.width));
            left += field.width;
        }
    }
    cells
}

pub(super) fn line_height(size: f32) -> f32 {
    size * LINE_HEIGHT_EM
}

pub(super) fn line_spacing(size: f32) -> (f32, f32) {
    (line_height(size), size * LINE_BASELINE_EM)
}

pub(super) fn heading_scale(level: u8) -> f32 {
    [1.55, 1.4, 1.3, 1.2, 1.1, 1.05][level.saturating_sub(1).min(5) as usize]
}

pub(super) fn heading_gap(size: f32) -> f32 {
    size * HEADING_GAP_EM
}

pub(super) fn line_number_digits(line_count: usize) -> usize {
    line_count.max(1).to_string().len()
}

pub(super) fn source_line_number(line_starts: &[usize], byte: usize) -> usize {
    line_starts
        .partition_point(|start| *start <= byte)
        .saturating_sub(1)
        + 1
}

pub(super) fn is_source_line_start(line_starts: &[usize], byte: usize) -> bool {
    line_starts.binary_search(&byte).is_ok()
}

pub(super) fn source_row_shift_count(
    row_positions: &[usize],
    source_start: usize,
    include_equal: bool,
) -> usize {
    if include_equal {
        row_positions.partition_point(|position| *position <= source_start)
    } else {
        row_positions.partition_point(|position| *position < source_start)
    }
}

pub(super) struct StatusEolSummary {
    summary: String,
    line_starts: Option<Vec<usize>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ViewSemantic {
    #[default]
    Plain,
    Heading,
    Code,
}

fn semantic_for_source(projection: &DisplayProjection, source: &Range<usize>) -> ViewSemantic {
    let end = projection
        .blocks
        .partition_point(|block| block.source.start <= source.start);
    let Some(block) = projection.blocks[..end]
        .iter()
        .rev()
        .find(|block| source.end <= block.source.end)
    else {
        return ViewSemantic::Plain;
    };
    match &block.kind {
        BlockKind::Heading { .. } => ViewSemantic::Heading,
        BlockKind::Code { .. } | BlockKind::FrontMatter => ViewSemantic::Code,
        _ => ViewSemantic::Plain,
    }
}

#[cfg(test)]
fn reference_semantic_for_source(
    projection: &DisplayProjection,
    source: &Range<usize>,
) -> ViewSemantic {
    let Some(block) = projection
        .blocks
        .iter()
        .filter(|block| block.source.start <= source.start && source.end <= block.source.end)
        .min_by_key(|block| block.source.len())
    else {
        return ViewSemantic::Plain;
    };
    match &block.kind {
        BlockKind::Heading { .. } => ViewSemantic::Heading,
        BlockKind::Code { .. } | BlockKind::FrontMatter => ViewSemantic::Code,
        _ => ViewSemantic::Plain,
    }
}

fn is_inline_code(run: &ProjectionRun) -> bool {
    run.kinds
        .iter()
        .any(|kind| matches!(kind, InlineKind::Code))
}

fn is_link_run(run: &ProjectionRun) -> bool {
    run.kinds
        .iter()
        .any(|kind| matches!(kind, InlineKind::Link { .. }))
}

fn current_projection(app: &App) -> Option<&DisplayProjection> {
    let (document, revision, projection) = app.projection_cache.as_ref()?;
    ((*document, *revision) == (app.doc().id(), app.doc().revision()))
        .then_some(projection.as_ref())
}

fn text_range_for_source(view: &TextView, source: &Range<usize>) -> Option<DWRITE_TEXT_RANGE> {
    let start = source.start.max(view.source.start);
    let end = source.end.min(view.source.end);
    if start >= end {
        return None;
    }
    let start = view.map.partition_point(|byte| *byte < start);
    let end = view.map.partition_point(|byte| *byte < end);
    if start >= end || end > view.text.len() {
        return None;
    }
    Some(DWRITE_TEXT_RANGE {
        startPosition: start as u32,
        length: (end - start) as u32,
    })
}

fn overlapping_runs<'a>(
    projection: &'a DisplayProjection,
    source: &Range<usize>,
) -> impl Iterator<Item = &'a ProjectionRun> + 'a {
    let source_start = source.start;
    let source_end = source.end;
    let first = projection
        .runs
        .partition_point(|run| run.source.end <= source_start);
    projection.runs[first..]
        .iter()
        .take_while(move |run| run.source.start < source_end)
}

fn eol_summary(rope: &crop::Rope) -> StatusEolSummary {
    let (mut cr, mut lf, mut crlf) = (0usize, 0usize, 0usize);
    let mut previous_cr = false;
    for byte in rope.bytes() {
        match byte {
            b'\r' => {
                cr += 1;
                previous_cr = true;
            }
            b'\n' => {
                lf += 1;
                if previous_cr {
                    crlf += 1;
                }
                previous_cr = false;
            }
            _ => previous_cr = false,
        }
    }
    let lone_cr = cr - crlf;
    let lone_lf = lf - crlf;
    let summary = if cr == 0 && lf == 0 {
        "none".to_owned()
    } else {
        format!("LF:{lone_lf} CRLF:{crlf} CR:{lone_cr}")
    };
    StatusEolSummary {
        summary,
        line_starts: (lone_cr != 0).then(|| physical_line_starts(rope)),
    }
}

fn physical_line_starts(rope: &crop::Rope) -> Vec<usize> {
    let mut starts = vec![0];
    let mut pending_cr = None;
    for (position, byte) in rope.bytes().enumerate() {
        match byte {
            b'\r' => {
                if let Some(previous_cr) = pending_cr.replace(position) {
                    starts.push(previous_cr + 1);
                }
            }
            b'\n' => {
                pending_cr = None;
                starts.push(position + 1);
            }
            _ => {
                if let Some(previous_cr) = pending_cr.take() {
                    starts.push(previous_cr + 1);
                }
            }
        }
    }
    if let Some(previous_cr) = pending_cr {
        starts.push(previous_cr + 1);
    }
    starts
}

fn line_column(document: &Document, eol: &StatusEolSummary) -> (usize, usize) {
    let rope = document.rope();
    let byte = document.selection().head.min(rope.byte_len());
    let (line, line_start) = if let Some(starts) = &eol.line_starts {
        let line = starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1);
        (line, starts[line])
    } else {
        let line = rope.line_of_byte(byte);
        (line, rope.byte_of_line(line).min(byte))
    };
    let column = document
        .slice(line_start..byte)
        .map(|slice| {
            slice
                .chars()
                .filter(|character| *character != '\r' && *character != '\n')
                .count()
                + 1
        })
        .unwrap_or(1);
    (line + 1, column)
}

fn clipped(text: &str, limit: usize) -> String {
    let mut chars = text.chars();
    let mut result: String = chars
        .by_ref()
        .take(limit)
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();
    if chars.next().is_some() {
        result.push('…');
    }
    result
}

fn format_encoding(format: FileFormat) -> &'static str {
    match format {
        FileFormat::Utf8 { bom: true } => "UTF-8 BOM",
        FileFormat::Utf8 { bom: false } => "UTF-8",
        FileFormat::Cp932 => "CP932",
    }
}

impl App {
    fn presentation_semantic(&self, view: &TextView) -> ViewSemantic {
        if !self.is_markdown_document() {
            return ViewSemantic::Plain;
        }
        current_projection(self)
            .map(|projection| semantic_for_source(projection, &view.source))
            .unwrap_or_default()
    }

    pub(super) fn view_is_heading(&self, view: &TextView) -> bool {
        self.presentation_semantic(view) == ViewSemantic::Heading
    }

    pub(super) fn view_has_code_background(&self, view: &TextView) -> bool {
        self.presentation_semantic(view) == ViewSemantic::Code
    }

    /// Fills the source-mapped inline-code glyph bounds with the caller's cached brush.
    pub(super) unsafe fn paint_inline_code_backgrounds(
        &self,
        target: &ID2D1RenderTarget,
        view: &TextView,
        x: f32,
        y: f32,
        brush: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        if !self.is_markdown_document() {
            return Ok(());
        }
        let Some(projection) = current_projection(self) else {
            return Ok(());
        };
        for run in overlapping_runs(projection, &view.source) {
            if !is_inline_code(run) {
                continue;
            }
            let Some(range) = text_range_for_source(view, &run.source) else {
                continue;
            };
            let mut count = 0;
            let _ = unsafe {
                view.layout.HitTestTextRange(
                    range.startPosition,
                    range.length,
                    x,
                    y,
                    None,
                    &mut count,
                )
            };
            let mut metrics = vec![
                DWRITE_HIT_TEST_METRICS::default();
                (count as usize).min(MAX_INLINE_CODE_RECTS)
            ];
            if metrics.is_empty() {
                continue;
            }
            let _ = unsafe {
                view.layout.HitTestTextRange(
                    range.startPosition,
                    range.length,
                    x,
                    y,
                    Some(&mut metrics),
                    &mut count,
                )
            };
            for metric in metrics
                .into_iter()
                .take((count as usize).min(MAX_INLINE_CODE_RECTS))
            {
                unsafe {
                    target.FillRectangle(
                        &rect(metric.left, metric.top, metric.width, metric.height),
                        brush,
                    );
                }
            }
        }
        Ok(())
    }

    /// Colors cached source-mapped link runs without fetching or parsing source text.
    pub(super) unsafe fn apply_link_color(
        &self,
        view: &TextView,
        brush: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        if !self.is_markdown_document() {
            return Ok(());
        }
        let Some(projection) = current_projection(self) else {
            return Ok(());
        };
        let effect = brush.cast::<windows::core::IUnknown>()?;
        for run in overlapping_runs(projection, &view.source) {
            if !is_link_run(run) {
                continue;
            }
            if let Some(range) = text_range_for_source(view, &run.source) {
                unsafe { view.layout.SetDrawingEffect(Some(&effect), range)? };
            }
        }
        Ok(())
    }

    pub(super) fn editor_status_fields(&mut self) -> Vec<StatusField> {
        let document_id = self.doc().id();
        let revision = self.doc().revision();
        let cache_matches =
            self.shell
                .status_eol_cache
                .as_ref()
                .is_some_and(|(cached_id, cached_revision, _)| {
                    (*cached_id, *cached_revision) == (document_id, revision)
                });
        if !cache_matches {
            self.shell.status_eol_cache =
                Some((document_id, revision, eol_summary(self.doc().rope())));
        }
        let tab = &self.tabs[self.active];
        let encoding = tab
            .file
            .as_ref()
            .map(|file| file.format())
            .or(tab.persistence.format)
            .unwrap_or_default();
        let workspace = self
            .persistence
            .workspace_root()
            .and_then(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| "未設定".to_owned());
        let branch = self.git_branch_label();
        let eol = &self.shell.status_eol_cache.as_ref().unwrap().2;
        let (line, column) = line_column(&tab.doc, eol);
        let save = if tab.doc.is_dirty() {
            "未保存"
        } else {
            "保存済み"
        };
        let external = if self.shell.external_change_docs.contains(&document_id) {
            "外部変更:検出"
        } else {
            "外部変更:未確認"
        };
        let document_type = if self.is_markdown_document() {
            "Markdown"
        } else {
            "Plain text"
        };
        let mode = if tab.raw { "Source" } else { "Live" };
        vec![
            StatusField {
                text: format!("Workspace {workspace}"),
                width: 180.,
                priority: 2,
            },
            StatusField {
                text: format!("Git {branch}"),
                width: 150.,
                priority: 3,
            },
            StatusField {
                text: format!("Ln {line}, Col {column}"),
                width: 112.,
                priority: 0,
            },
            StatusField {
                text: format_encoding(encoding).into(),
                width: 76.,
                priority: 4,
            },
            StatusField {
                text: eol.summary.clone(),
                width: 126.,
                priority: 6,
            },
            StatusField {
                text: format!("{mode} · {document_type}"),
                width: 132.,
                priority: 5,
            },
            StatusField {
                text: save.into(),
                width: 76.,
                priority: 1,
            },
            StatusField {
                text: external.into(),
                width: 112.,
                priority: 7,
            },
        ]
    }

    #[cfg(test)]
    pub(super) fn editor_status_text(&mut self) -> String {
        self.editor_status_fields()
            .into_iter()
            .map(|field| field.text)
            .collect::<Vec<_>>()
            .join(" · ")
    }

    pub(super) unsafe fn draw_editor_status(
        &mut self,
        target: &ID2D1RenderTarget,
        top: f32,
        foreground: &ID2D1SolidColorBrush,
        border: &ID2D1SolidColorBrush,
    ) -> windows::core::Result<()> {
        unsafe {
            let fields = self.editor_status_fields();
            let left = if self.shell.enabled && self.hwnd == self.shell.main_window {
                52.
            } else {
                8.
            };
            let available = (self.width - left - 8.).max(0.);
            let cells = status_cells(&fields, available);
            let tooltip = fields
                .iter()
                .map(|field| field.text.as_str())
                .chain(std::iter::once(self.status.as_str()))
                .collect::<Vec<_>>()
                .join("\n");
            self.status_tooltip(&tooltip, left, top + 2., available);
            for &(index, offset, width) in &cells {
                let x = left + offset;
                target.DrawLine(point(x, top + 7.), point(x, top + 23.), border, 1., None);
                let layout =
                    self.ui_text_layout(&fields[index].text, (width - 16.).max(1.), 12.)?;
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let ellipsis = self.graphics.write.CreateEllipsisTrimmingSign(&layout)?;
                layout.SetTrimming(
                    &DWRITE_TRIMMING {
                        granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                        ..Default::default()
                    },
                    &ellipsis,
                )?;
                target.DrawTextLayout(
                    point(x + 8., top + 5.),
                    &layout,
                    foreground,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            // The fixed cells stay in place while a message uses the spare
            // space at the left. Its full value remains in the tooltip.
            let left_end = cells
                .iter()
                .filter(|(index, _, _)| *index < 2)
                .map(|(_, x, width)| x + width)
                .fold(0., f32::max);
            let right_start = cells
                .iter()
                .filter(|(index, _, _)| *index >= 2)
                .map(|(_, x, _)| *x)
                .fold(available, f32::min);
            let message_width = right_start - left_end;
            if !self.status.is_empty() && message_width >= 60. {
                let layout = self.ui_text_layout(
                    &clipped(&self.status, STATUS_MAX_CHARS),
                    (message_width - 12.).max(1.),
                    12.,
                )?;
                layout.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                target.DrawTextLayout(
                    point(left + left_end + 6., top + 5.),
                    &layout,
                    foreground,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            Ok(())
        }
    }

    pub(super) fn mark_external_change(&mut self, document_id: u64) {
        self.shell.external_change_docs.insert(document_id);
    }

    pub(super) fn clear_external_change(&mut self, document_id: u64) {
        self.shell.external_change_docs.remove(&document_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eol_summary_counts_lf_crlf_and_cr_without_materializing_source() {
        let source = crop::Rope::from("a\r\nb\nc\rd");
        assert_eq!(eol_summary(&source).summary, "LF:1 CRLF:1 CR:1");
        assert_eq!(eol_summary(&crop::Rope::from("plain")).summary, "none");
    }

    #[test]
    fn physical_line_starts_keep_cr_only_and_mixed_cursor_columns_accurate() {
        let mut document = Document::new(1, "one\rtwo\r\nthree\nend".to_owned());
        let cursor = "one\rtwo\r\nthree".len();
        document.set_selection(cursor, cursor).unwrap();
        let eol = eol_summary(document.rope());
        assert_eq!(line_column(&document, &eol), (3, 6));
        assert_eq!(eol.line_starts.as_deref(), Some(&[0, 4, 9, 15][..]));
    }

    #[test]
    fn line_numbers_use_physical_mixed_eol_starts_and_scale_to_six_digits() {
        let source = "a\r\nb\rc\n";
        let starts = super::super::physical_line_starts(source);
        assert_eq!(starts, [0, 3, 5, 7]);
        assert_eq!(source_line_number(&starts, 0), 1);
        assert_eq!(source_line_number(&starts, 4), 2);
        assert_eq!(source_line_number(&starts, 6), 3);
        assert_eq!(source_line_number(&starts, source.len()), 4);
        assert!(is_source_line_start(&starts, 7));
        assert!(!is_source_line_start(&starts, 6));

        let many = format!("{}x", "x\n".repeat(99_999));
        let starts = super::super::physical_line_starts(&many);
        assert_eq!(starts.len(), 100_000);
        assert_eq!(line_number_digits(starts.len()), 6);
        assert_eq!(
            source_line_number(&starts, *starts.last().unwrap()),
            100_000
        );
    }

    #[test]
    fn batched_source_rows_match_sequential_equal_position_anchor_order() {
        let height = 27.2;
        let candidates = [10, 20, 30, 40, 50];
        let existing_empty_anchors = [20, 30, 40]; // literal, image start, image end
        let row_positions: Vec<_> = candidates
            .into_iter()
            .filter(|position| existing_empty_anchors.binary_search(position).is_err())
            .collect();
        assert_eq!(row_positions, [10, 50]);

        let original = [
            ("text", 10),
            ("literal", 20),
            ("image-start", 30),
            ("image-end", 40),
            ("text", 50),
        ];
        let mut sequential: Vec<_> = original
            .iter()
            .map(|(kind, source_start)| (*kind, *source_start, 100.0))
            .collect();
        for position in candidates {
            if existing_empty_anchors.binary_search(&position).is_ok() {
                continue;
            }
            for (_, source_start, y) in &mut sequential {
                if *source_start >= position {
                    *y += height;
                }
            }
        }
        let batched: Vec<_> = original
            .iter()
            .map(|(kind, source_start)| {
                (
                    *kind,
                    *source_start,
                    100.0
                        + source_row_shift_count(&row_positions, *source_start, true) as f32
                            * height,
                )
            })
            .collect();
        assert_eq!(batched, sequential);

        for (position, expected_y) in [(10, 100.0), (50, 127.2)] {
            assert_eq!(
                100.0 + source_row_shift_count(&row_positions, position, false) as f32 * height,
                expected_y,
                "a new empty row anchor excludes its own source position"
            );
        }

        let many_blank_rows: Vec<_> = (0..100_000).map(|index| index * 2).collect();
        assert_eq!(
            source_row_shift_count(&many_blank_rows, 199_998, true),
            100_000
        );
        assert_eq!(
            source_row_shift_count(&many_blank_rows, 199_998, false),
            99_999
        );
    }

    #[test]
    fn cached_projection_classifies_heading_fenced_and_indented_code_views() {
        let source = "# Heading\n\n```rust\nlet x = 1;\n```\n\n    indented\n";
        let projection = DisplayProjection::parse(source);
        let heading = projection
            .blocks
            .iter()
            .find(|block| matches!(&block.kind, BlockKind::Heading { .. }))
            .unwrap();
        assert_eq!(
            semantic_for_source(&projection, &heading.source),
            ViewSemantic::Heading
        );
        let code: Vec<_> = projection
            .blocks
            .iter()
            .filter(|block| matches!(&block.kind, BlockKind::Code { .. }))
            .collect();
        assert_eq!(code.len(), 2);
        assert!(
            code.iter()
                .all(|block| semantic_for_source(&projection, &block.source) == ViewSemantic::Code)
        );
    }

    #[test]
    fn optimized_semantics_match_reference_for_nested_list_heading_and_code_ranges() {
        let source =
            "# Heading\n\n- parent\n  - nested `inline`\n\n    ```rust\n    let x = 1;\n    ```\n";
        let projection = DisplayProjection::parse(source);
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| matches!(&block.kind, BlockKind::Heading { .. }))
        );
        assert!(
            projection
                .blocks
                .iter()
                .filter(|block| matches!(&block.kind, BlockKind::ListItem))
                .count()
                >= 2
        );
        assert!(
            projection
                .blocks
                .iter()
                .any(|block| matches!(&block.kind, BlockKind::Code { .. }))
        );

        for range in projection
            .blocks
            .iter()
            .map(|block| block.source.clone())
            .chain(projection.runs.iter().map(|run| run.source.clone()))
        {
            assert_eq!(
                semantic_for_source(&projection, &range),
                reference_semantic_for_source(&projection, &range),
                "source range {range:?}"
            );
        }
    }

    #[test]
    fn projection_exposes_inline_code_and_link_runs_for_native_styling() {
        let source = "`inline` and [link](https://example.invalid)";
        let projection = DisplayProjection::parse(source);
        let code = projection
            .runs
            .iter()
            .find(|run| is_inline_code(run))
            .unwrap();
        let link = projection.runs.iter().find(|run| is_link_run(run)).unwrap();
        assert_eq!(&source[code.source.clone()], "inline");
        assert!(source[link.source.clone()].contains("link"));
    }

    #[test]
    fn editor_status_uses_recovery_format_and_known_external_state() {
        let source = "a\r\nb\ncat\rd";
        let mut app = super::super::tests::composing_test_app(source);
        let caret = "a\r\nb\nca".len();
        app.doc_mut().set_selection(source.len(), caret).unwrap();
        app.tabs[0].persistence.format = Some(FileFormat::Cp932);
        let document_id = app.doc().id();
        app.mark_external_change(document_id);

        let status = app.editor_status_text();
        assert!(status.contains("CP932"));
        assert!(status.contains("外部変更:検出"));
        assert!(status.contains("Ln 3, Col 3"));
        assert!(status.contains("LF:1 CRLF:1 CR:1"));
        assert!(status.contains("Markdown"));
    }

    #[test]
    fn status_cells_keep_caret_and_save_at_narrow_width_without_overlap() {
        let mut app = super::super::tests::composing_test_app("line\n");
        let fields = app.editor_status_fields();
        for width in [0., 80., 188., 300., 640., 1024., 1600.] {
            let mut cells = status_cells(&fields, width);
            cells.sort_by(|a, b| a.1.total_cmp(&b.1));
            for &(_, x, cell_width) in &cells {
                assert!(x >= 0. && x + cell_width <= width);
            }
            assert!(
                cells
                    .windows(2)
                    .all(|pair| pair[0].1 + pair[0].2 <= pair[1].1)
            );
            if width >= 188. {
                assert!(cells.iter().any(|(index, _, _)| *index == 2));
                assert!(cells.iter().any(|(index, _, _)| *index == 6));
            }
        }
        let cells = status_cells(&fields, 1024.);
        assert_eq!(
            cells.iter().find(|(index, _, _)| *index == 0).unwrap().1,
            0.
        );
        assert_eq!(
            cells.iter().find(|(index, _, _)| *index == 1).unwrap().1,
            180.
        );
        assert!(fields[0].text.contains("未設定"));
        assert!(fields[1].text.contains("Workspace未設定"));
    }

    #[test]
    fn status_clipping_is_bounded_and_removes_line_breaks() {
        assert_eq!(clipped("123456789", 5), "12345…");
        assert_eq!(clipped("short", 5), "short");
        assert_eq!(clipped("line\nbreak", 32), "line break");
    }
}
