//! Source-preserving operations for GFM tables.
//!
//! Cell offsets are UTF-8 byte offsets into the original Markdown source. The
//! structural pipes and delimiter row stay outside cell content ranges, so a
//! normal cell edit cannot consume table structure.

use std::{cmp::Reverse, ops::Range};

use crate::markdown::parser_options;
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCell {
    /// The raw cell, including all padding but excluding neighboring pipes.
    pub range: Range<usize>,
    /// The editable cell value after removing at most one ASCII padding space
    /// from each edge. Any additional whitespace remains source content.
    pub content_range: Range<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableRow {
    /// The row source without its line ending.
    pub range: Range<usize>,
    pub cells: Vec<TableCell>,
    /// Absolute byte offsets of unescaped structural pipes.
    pub pipes: Vec<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TablePosition {
    /// Header is row zero. The delimiter row is not an editable row.
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TableRect {
    pub start: TablePosition,
    pub end: TablePosition,
}

impl TableRect {
    pub fn normalized(self) -> Self {
        Self {
            start: TablePosition {
                row: self.start.row.min(self.end.row),
                column: self.start.column.min(self.end.column),
            },
            end: TablePosition {
                row: self.start.row.max(self.end.row),
                column: self.start.column.max(self.end.column),
            },
        }
    }

    pub fn width(self) -> usize {
        self.start.column.abs_diff(self.end.column) + 1
    }

    pub fn height(self) -> usize {
        self.start.row.abs_diff(self.end.row) + 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableEdit {
    /// A single bounded source replacement. Apply this once through the
    /// document model so the operation is one undo transaction.
    pub range: Range<usize>,
    pub replacement: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableCut {
    pub tsv: String,
    pub edit: TableEdit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    /// The exact source span occupied by the header, delimiter and data rows,
    /// excluding a trailing line ending and any surrounding blank lines.
    pub range: Range<usize>,
    /// Header and data rows. The delimiter row is stored separately.
    pub rows: Vec<TableRow>,
    pub delimiter: TableRow,
    pub columns: usize,
    pub alignments: Vec<TableAlignment>,
}

impl Table {
    /// Parse a parser-recognized GFM table within `range`.
    pub fn parse(source: &str, range: Range<usize>) -> Option<Self> {
        if range.start > range.end || range.end > source.len() {
            return None;
        }
        let table_source = source.get(range.clone())?;
        if !is_gfm_table(table_source) {
            return None;
        }

        let parsed_lines = physical_lines(table_source, range.start);
        if parsed_lines.len() < 2 {
            return None;
        }
        let mut parsed_rows: Vec<TableRow> = parsed_lines
            .into_iter()
            .map(|(row_range, row)| parse_table_row(row, row_range.start))
            .collect();

        let header = parsed_rows.first()?;
        let delimiter = parsed_rows.get(1)?;
        let columns = header.cells.len();
        if columns == 0 || delimiter.cells.len() != columns {
            return None;
        }
        if parsed_rows.iter().any(|row| row.cells.len() != columns) {
            return None;
        }
        let alignments = delimiter
            .cells
            .iter()
            .map(|cell| parse_alignment(source.get(cell.content_range.clone())?))
            .collect::<Option<Vec<_>>>()?;

        let delimiter = parsed_rows.remove(1);
        let table_start = parsed_rows.first()?.range.start;
        let table_end = parsed_rows.last()?.range.end.max(delimiter.range.end);
        Some(Self {
            range: table_start..table_end,
            rows: parsed_rows,
            delimiter,
            columns,
            alignments,
        })
    }

    /// Find the table containing a byte position. Cell padding belongs to the
    /// cell; delimiter pipes themselves do not.
    pub fn cell_at(&self, source_byte: usize) -> Option<TablePosition> {
        for (row_index, row) in self.rows.iter().enumerate() {
            for (column, cell) in row.cells.iter().enumerate() {
                if source_byte >= cell.range.start && source_byte <= cell.range.end {
                    return Some(TablePosition {
                        row: row_index,
                        column,
                    });
                }
            }
        }
        None
    }

    /// Move to an adjacent cell. Returning `None` at a table edge lets the
    /// editor hand the caret back to normal document navigation.
    pub fn move_cell(
        &self,
        position: TablePosition,
        direction: CellDirection,
    ) -> Option<TablePosition> {
        if position.row >= self.rows.len() || position.column >= self.columns {
            return None;
        }
        let target = match direction {
            CellDirection::Left if position.column > 0 => TablePosition {
                column: position.column - 1,
                ..position
            },
            CellDirection::Right if position.column + 1 < self.columns => TablePosition {
                column: position.column + 1,
                ..position
            },
            CellDirection::Up if position.row > 0 => TablePosition {
                row: position.row - 1,
                ..position
            },
            CellDirection::Down if position.row + 1 < self.rows.len() => TablePosition {
                row: position.row + 1,
                ..position
            },
            _ => return None,
        };
        Some(target)
    }

    /// Insert a blank data row before `before_row`. Row zero is the header and
    /// cannot be displaced; pass `rows.len()` to append.
    pub fn insert_row(&self, source: &str, before_row: usize) -> Option<TableEdit> {
        if before_row == 0 || before_row > self.rows.len() || self.rows.is_empty() {
            return None;
        }
        let template = self.rows.get(before_row.saturating_sub(1))?;
        let row_text = blank_row_like(source, template, self.columns)?;
        let newline = self.line_ending_for_insert(source, before_row);
        if let Some(next) = self.rows.get(before_row) {
            Some(TableEdit {
                range: next.range.start..next.range.start,
                replacement: format!("{row_text}{newline}"),
            })
        } else {
            Some(TableEdit {
                range: self.range.end..self.range.end,
                replacement: format!("{newline}{row_text}"),
            })
        }
    }

    fn line_ending_for_insert(&self, source: &str, before_row: usize) -> &'static str {
        if before_row < self.rows.len() {
            let next = &self.rows[before_row];
            let previous_end = if before_row == 1 {
                self.delimiter.range.end
            } else {
                self.rows[before_row - 1].range.end
            };
            return line_ending_in(source, previous_end..next.range.start)
                .unwrap_or_else(|| table_line_ending(source, &self.range));
        }

        let table_separator = (|| {
            let last_data = self.rows.len().checked_sub(1)?;
            let previous_end = if last_data == 0 {
                self.rows[0].range.end
            } else if last_data == 1 {
                self.delimiter.range.end
            } else {
                self.rows[last_data - 1].range.end
            };
            line_ending_in(source, previous_end..self.rows[last_data].range.start)
        })()
        .or_else(|| {
            line_ending_in(
                source,
                self.rows.first()?.range.end..self.delimiter.range.start,
            )
        });
        table_separator
            .or_else(|| line_ending_after(source, self.range.end))
            .unwrap_or_else(|| table_line_ending(source, &self.range))
    }

    /// Delete a data row. The header and the last remaining data row are
    /// protected, so every accepted result remains a useful table.
    pub fn delete_row(&self, row: usize) -> Option<TableEdit> {
        if row == 0 || row >= self.rows.len() || self.rows.len() <= 2 {
            return None;
        }
        let current = self.rows.get(row)?;
        if let Some(next) = self.rows.get(row + 1) {
            Some(TableEdit {
                range: current.range.start..next.range.start,
                replacement: String::new(),
            })
        } else {
            let previous = self.rows.get(row - 1)?;
            Some(TableEdit {
                range: previous.range.end..current.range.end,
                replacement: String::new(),
            })
        }
    }

    /// Insert a blank column before `before_column`; `columns` appends it.
    /// Untouched cells and their exact spelling, whitespace and inline
    /// formatting are copied byte-for-byte into one table-bounded edit.
    pub fn insert_column(&self, source: &str, before_column: usize) -> Option<TableEdit> {
        if before_column > self.columns || self.columns == 0 {
            return None;
        }
        let mut replacements = Vec::with_capacity(self.rows.len() + 1);
        for row in &self.rows {
            let replacement = change_row_columns(source, row, before_column, Some("  "))?;
            replacements.push((row.range.clone(), replacement));
        }
        let delimiter = change_row_columns(source, &self.delimiter, before_column, Some(" --- "))?;
        replacements.push((self.delimiter.range.clone(), delimiter));
        apply_table_replacements(source, self.range.clone(), replacements)
    }

    /// Delete a column. The final column is protected.
    pub fn delete_column(&self, source: &str, column: usize) -> Option<TableEdit> {
        if self.columns <= 1 || column >= self.columns {
            return None;
        }
        let mut replacements = Vec::with_capacity(self.rows.len() + 1);
        for row in &self.rows {
            let replacement = change_row_columns(source, row, column, None)?;
            replacements.push((row.range.clone(), replacement));
        }
        let delimiter = change_row_columns(source, &self.delimiter, column, None)?;
        replacements.push((self.delimiter.range.clone(), delimiter));
        apply_table_replacements(source, self.range.clone(), replacements)
    }

    /// Change one column's delimiter alignment without changing cell padding.
    pub fn set_alignment(&self, column: usize, alignment: TableAlignment) -> Option<TableEdit> {
        let cell = self.delimiter.cells.get(column)?;
        Some(TableEdit {
            range: cell.content_range.clone(),
            replacement: match alignment {
                TableAlignment::Left => "---",
                TableAlignment::Center => ":---:",
                TableAlignment::Right => "---:",
            }
            .to_owned(),
        })
    }

    /// Copy a rectangular set of cells in row-major TSV form. Markdown inline
    /// spelling is retained as cell text.
    pub fn copy_tsv(&self, source: &str, rect: TableRect) -> Option<String> {
        let rect = rect.normalized();
        self.validate_rect(rect)?;
        let mut lines = Vec::with_capacity(rect.height());
        for row_index in rect.start.row..=rect.end.row {
            let row = self.rows.get(row_index)?;
            let mut values = Vec::with_capacity(rect.width());
            for column in rect.start.column..=rect.end.column {
                let cell = row.cells.get(column)?;
                values.push(source.get(cell.content_range.clone())?);
            }
            lines.push(values.join("\t"));
        }
        Some(lines.join("\n"))
    }

    /// Return copied TSV and a single edit that clears only those cells' text.
    /// The caller should write the clipboard first and apply `edit` only if
    /// that write succeeds.
    pub fn cut_tsv(&self, source: &str, rect: TableRect) -> Option<TableCut> {
        let rect = rect.normalized();
        let tsv = self.copy_tsv(source, rect)?;
        let mut replacements = Vec::with_capacity(rect.width() * rect.height());
        for row_index in rect.start.row..=rect.end.row {
            let row = self.rows.get(row_index)?;
            for column in rect.start.column..=rect.end.column {
                let cell = row.cells.get(column)?;
                replacements.push((cell.content_range.clone(), String::new()));
            }
        }
        let edit = apply_table_replacements(source, self.range.clone(), replacements)?;
        Some(TableCut { tsv, edit })
    }

    /// Paste a rectangular TSV block into existing cells. Ragged input,
    /// overflow, and plain single-cell text are rejected without edits.
    pub fn paste_tsv(&self, source: &str, target: TablePosition, tsv: &str) -> Option<TableEdit> {
        let values = parse_tsv(tsv)?;
        let height = values.len();
        let width = values.first()?.len();
        if target.row.checked_add(height)? > self.rows.len()
            || target.column.checked_add(width)? > self.columns
        {
            return None;
        }
        let mut replacements = Vec::with_capacity(height * width);
        for (row_offset, values_row) in values.iter().enumerate() {
            let row = self.rows.get(target.row + row_offset)?;
            for (column_offset, value) in values_row.iter().enumerate() {
                let cell = row.cells.get(target.column + column_offset)?;
                replacements.push((cell.content_range.clone(), escape_table_pipes(value)));
            }
        }
        apply_table_replacements(source, self.range.clone(), replacements)
    }

    fn validate_rect(&self, rect: TableRect) -> Option<()> {
        if rect.start.row > rect.end.row
            || rect.start.column > rect.end.column
            || rect.end.row >= self.rows.len()
            || rect.end.column >= self.columns
        {
            return None;
        }
        Some(())
    }
}

/// Find all well-formed GFM tables in document order.
pub fn find_tables(source: &str) -> Vec<Table> {
    let parser = Parser::new_ext(source, parser_options()).into_offset_iter();
    let mut start = None;
    let mut tables = Vec::new();
    for (event, range) in parser {
        match event {
            Event::Start(Tag::Table(_)) => start = Some(range.start),
            Event::End(TagEnd::Table) => {
                if let Some(table_start) = start.take()
                    && let Some(table) = Table::parse(source, table_start..range.end)
                {
                    tables.push(table);
                }
            }
            _ => {}
        }
    }
    tables
}

/// Parse a single table row while respecting escaped pipes. `from` is the
/// absolute UTF-8 source byte offset of the row.
pub fn parse_table_row(text: &str, from: usize) -> TableRow {
    let pipes: Vec<usize> = unescaped_pipe_offsets(text)
        .into_iter()
        .map(|offset| from + offset)
        .collect();
    let has_leading_outer_pipe = pipes.first().is_some_and(|pipe| {
        text.get(..pipe - from)
            .is_some_and(|prefix| prefix.trim().is_empty())
    });
    let start = if has_leading_outer_pipe {
        pipes[0] - from + 1
    } else {
        0
    };
    let has_trailing_outer_pipe = pipes.last().is_some_and(|pipe| {
        text.get(pipe - from + 1..)
            .is_some_and(|suffix| suffix.trim().is_empty())
    });
    let end = if has_trailing_outer_pipe {
        pipes.last().map_or(text.len(), |pipe| pipe - from)
    } else {
        text.len()
    };

    let mut boundaries = vec![start];
    boundaries.extend(
        pipes
            .iter()
            .map(|pipe| pipe - from)
            .filter(|pipe| *pipe >= start && *pipe < end),
    );
    boundaries.push(end);

    let mut cells = Vec::with_capacity(boundaries.len().saturating_sub(1));
    let mut cell_start = start;
    for right in boundaries.iter().skip(1).copied() {
        let leading = usize::from(text.as_bytes().get(cell_start) == Some(&b' '));
        let raw = &text[cell_start..right];
        let trailing = usize::from(raw.len() > leading && raw.as_bytes().last() == Some(&b' '));
        let content_start = cell_start + leading;
        let content_end = right.saturating_sub(trailing);
        cells.push(TableCell {
            range: from + cell_start..from + right,
            content_range: from + content_start..from + content_end.max(content_start),
        });
        cell_start = right.saturating_add(1);
    }
    TableRow {
        range: from..from + text.len(),
        cells,
        pipes,
    }
}

fn is_gfm_table(source: &str) -> bool {
    Parser::new_ext(source, parser_options())
        .any(|event| matches!(event, Event::Start(Tag::Table(_))))
}

fn physical_lines(source: &str, base: usize) -> Vec<(Range<usize>, &str)> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in source.bytes().enumerate() {
        if byte != b'\n' {
            continue;
        }
        let mut end = index;
        if end > start && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        if let Some(line) = source.get(start..end) {
            lines.push((base + start..base + end, line));
        }
        start = index + 1;
    }
    if start < source.len() {
        let mut end = source.len();
        if end > start && source.as_bytes()[end - 1] == b'\r' {
            end -= 1;
        }
        if let Some(line) = source.get(start..end) {
            lines.push((base + start..base + end, line));
        }
    }
    lines
}

fn unescaped_pipe_offsets(text: &str) -> Vec<usize> {
    let mut pipes = Vec::new();
    let mut backslashes = 0usize;
    for (offset, character) in text.char_indices() {
        if character == '|' && backslashes.is_multiple_of(2) {
            pipes.push(offset);
        }
        if character == '\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
    }
    pipes
}

fn parse_alignment(value: &str) -> Option<TableAlignment> {
    let value = value.trim();
    let leading = value.starts_with(':');
    let trailing = value.ends_with(':');
    let body = value.trim_matches(':');
    if body.len() < 3 || !body.bytes().all(|byte| byte == b'-') {
        return None;
    }
    Some(match (leading, trailing) {
        (true, true) => TableAlignment::Center,
        (false, true) => TableAlignment::Right,
        _ => TableAlignment::Left,
    })
}

fn blank_row_like(source: &str, template: &TableRow, columns: usize) -> Option<String> {
    let first = template.cells.first()?;
    let last = template.cells.last()?;
    let prefix = source.get(template.range.start..first.range.start)?;
    let suffix = source.get(last.range.end..template.range.end)?;
    let blank = "  ";
    let cells = std::iter::repeat_n(blank, columns).collect::<Vec<_>>();
    let mut prefix = prefix.to_owned();
    let mut suffix = suffix.to_owned();
    if columns == 1 && !prefix.contains('|') && !suffix.contains('|') {
        prefix.insert(0, '|');
        suffix.push('|');
    }
    Some(format!("{prefix}{}{suffix}", cells.join("|")))
}

fn change_row_columns(
    source: &str,
    row: &TableRow,
    column: usize,
    delimiter_value: Option<&str>,
) -> Option<String> {
    let first = row.cells.first()?;
    let last = row.cells.last()?;
    let prefix = source.get(row.range.start..first.range.start)?;
    let suffix = source.get(last.range.end..row.range.end)?;
    let mut cells = row
        .cells
        .iter()
        .map(|cell| source.get(cell.range.clone()).map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    if let Some(value) = delimiter_value {
        cells.insert(column, value.to_owned());
    } else if column >= cells.len() {
        return None;
    } else {
        cells.remove(column);
    }
    let mut prefix = prefix.to_owned();
    let mut suffix = suffix.to_owned();
    if cells.len() == 1 && !prefix.contains('|') && !suffix.contains('|') {
        prefix.insert(0, '|');
        suffix.push('|');
    }
    Some(format!("{prefix}{}{suffix}", cells.join("|")))
}

fn apply_table_replacements(
    source: &str,
    table_range: Range<usize>,
    mut replacements: Vec<(Range<usize>, String)>,
) -> Option<TableEdit> {
    let mut replacement = source.get(table_range.clone())?.to_owned();
    replacements.sort_by_key(|replacement| Reverse(replacement.0.start));
    let mut last_start = usize::MAX;
    for (range, value) in replacements {
        if range.start < table_range.start
            || range.end > table_range.end
            || range.start > range.end
            || range.end > last_start
        {
            return None;
        }
        let local = range.start - table_range.start..range.end - table_range.start;
        replacement.replace_range(local, &value);
        last_start = range.start;
    }
    Some(TableEdit {
        range: table_range,
        replacement,
    })
}

fn line_ending_after(source: &str, position: usize) -> Option<&'static str> {
    let suffix = source.get(position..)?;
    if suffix.starts_with("\r\n") {
        Some("\r\n")
    } else if suffix.starts_with('\n') {
        Some("\n")
    } else {
        None
    }
}

fn line_ending_in(source: &str, range: Range<usize>) -> Option<&'static str> {
    let gap = source.get(range)?;
    let newline = gap.rfind('\n')?;
    Some(
        if newline > 0 && gap.as_bytes().get(newline - 1) == Some(&b'\r') {
            "\r\n"
        } else {
            "\n"
        },
    )
}

fn table_line_ending(source: &str, table_range: &Range<usize>) -> &'static str {
    source
        .get(table_range.clone())
        .and_then(|table| {
            let newline = table.rfind('\n')?;
            Some(
                if newline > 0 && table.as_bytes().get(newline - 1) == Some(&b'\r') {
                    "\r\n"
                } else {
                    "\n"
                },
            )
        })
        .unwrap_or("\n")
}

fn parse_tsv(input: &str) -> Option<Vec<Vec<String>>> {
    if !input.contains('\t') && !input.contains('\n') && !input.contains('\r') {
        return None;
    }
    let normalized = input.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.strip_suffix('\n').unwrap_or(&normalized);
    if normalized.is_empty() {
        return None;
    }
    let rows: Vec<Vec<String>> = normalized
        .split('\n')
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect();
    let width = rows.first()?.len();
    if width == 0 || rows.iter().any(|row| row.len() != width) {
        return None;
    }
    Some(rows)
}

fn escape_table_pipes(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    let mut backslashes = 0usize;
    for character in value.chars() {
        if character == '|' && backslashes.is_multiple_of(2) {
            escaped.push('\\');
        }
        escaped.push(character);
        if character == '\\' {
            backslashes += 1;
        } else {
            backslashes = 0;
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(source: &str, edit: TableEdit) -> String {
        let mut result = source.to_owned();
        result.replace_range(edit.range, &edit.replacement);
        result
    }

    fn assert_unchanged_outside(source: &str, edited: &str, edit: &TableEdit) {
        assert_eq!(&edited[..edit.range.start], &source[..edit.range.start]);
        assert!(edited.ends_with(&source[edit.range.end..]));
    }

    fn first_table(source: &str) -> Table {
        find_tables(source).into_iter().next().expect("table")
    }

    #[test]
    fn parses_exact_cells_and_preserves_only_one_ascii_padding_edge() {
        let source =
            "|  前  |　日本　|  | a\\|b |\r\n| :--- | :---: | ---: | --- |\r\n| x | y | z | w |";
        let table = first_table(source);
        assert_eq!(table.columns, 4);
        assert_eq!(
            table.alignments,
            [
                TableAlignment::Left,
                TableAlignment::Center,
                TableAlignment::Right,
                TableAlignment::Left
            ]
        );
        let header = &table.rows[0];
        let ascii = &header.cells[0];
        let full_width = &header.cells[1];
        let empty = &header.cells[2];
        let escaped = &header.cells[3];
        assert_eq!(&source[ascii.content_range.clone()], " 前 ");
        assert_eq!(&source[full_width.content_range.clone()], "　日本　");
        assert_eq!(&source[empty.content_range.clone()], "");
        assert_eq!(&source[escaped.range.clone()], " a\\|b ");
        assert_eq!(
            table.cell_at(escaped.content_range.start),
            Some(TablePosition { row: 0, column: 3 })
        );
        assert_eq!(
            table.move_cell(TablePosition { row: 0, column: 1 }, CellDirection::Right),
            Some(TablePosition { row: 0, column: 2 })
        );
        assert_eq!(
            table.move_cell(TablePosition { row: 1, column: 3 }, CellDirection::Down),
            None
        );
    }

    #[test]
    fn structural_edits_are_one_atomic_bounded_replacement_and_protect_minimum_shape() {
        let source = "before\r\n| **h** | link |\r\n| --- | --- |\r\n| a\\|b | `code` |\r\n| keep | untouched |\r\n\r\nafter";
        let table = first_table(source);
        let insertion = table.insert_column(source, 1).expect("insert column");
        let inserted = apply(source, insertion.clone());
        assert_unchanged_outside(source, &inserted, &insertion);
        assert!(inserted.contains("| **h** |  | link |"));
        assert!(inserted.contains("| a\\|b |  | `code` |"));
        assert!(inserted.starts_with("before\r\n"));
        assert!(inserted.ends_with("\r\n\r\nafter"));

        let source = inserted;
        let table = first_table(&source);
        let removed = apply(
            &source,
            table.delete_column(&source, 1).expect("delete column"),
        );
        assert!(removed.contains("| **h** | link |"));
        assert!(table.delete_column(&source, 0).is_some());
        let one_column = first_table("| h | v |\n| --- | --- |\n| a | b |");
        let once = apply(
            "| h | v |\n| --- | --- |\n| a | b |",
            one_column
                .delete_column("| h | v |\n| --- | --- |\n| a | b |", 1)
                .expect("one column"),
        );
        let one_column = first_table(&once);
        assert_eq!(one_column.columns, 1);
        assert!(one_column.delete_column(&once, 0).is_none());
        assert!(one_column.delete_row(0).is_none());
        assert!(one_column.delete_row(1).is_none());

        let many_rows = first_table("| h |\n| --- |\n| a |\n| b |");
        assert!(many_rows.delete_row(1).is_some());
        assert!(many_rows.delete_row(2).is_some());
    }

    #[test]
    fn row_insert_delete_and_alignment_keep_neighboring_source_literal() {
        let source = "before\n| h | v |\n| --- | --- |\n| a | b |\n\nafter";
        let table = first_table(source);
        let insertion = table.insert_row(source, table.rows.len()).expect("append");
        let added = apply(source, insertion.clone());
        assert_unchanged_outside(source, &added, &insertion);
        assert!(added.contains("| a | b |\n|  |  |\n\nafter"));
        let table = first_table(&added);
        let deleted = apply(
            &added,
            table
                .delete_row(table.rows.len() - 1)
                .expect("delete last of several"),
        );
        assert!(deleted.contains("| a | b |\n\nafter"));
        assert!(deleted.starts_with("before\n"));

        let table = first_table(source);
        let alignment = table
            .set_alignment(1, TableAlignment::Center)
            .expect("align");
        let aligned = apply(source, alignment.clone());
        assert_unchanged_outside(source, &aligned, &alignment);
        assert!(aligned.contains("| --- | :---: |"));
        assert!(aligned.ends_with("\n\nafter"));
    }

    #[test]
    fn header_only_tables_include_delimiter_and_insert_after_it_with_table_eol() {
        let lf_source = "before\n| H |\n| --- |\n\nafter";
        let lf_table = first_table(lf_source);
        let delimiter_end = lf_source.find("| --- |").expect("delimiter") + "| --- |".len();
        assert_eq!(lf_table.range.end, delimiter_end);
        assert!(lf_table.delete_row(0).is_none());
        assert!(lf_table.delete_row(1).is_none());
        assert!(lf_table.delete_column(lf_source, 0).is_none());

        let row_edit = lf_table.insert_row(lf_source, 1).expect("first data row");
        let lf_with_row = apply(lf_source, row_edit.clone());
        assert_eq!(lf_with_row, "before\n| H |\n| --- |\n|  |\n\nafter");
        assert_unchanged_outside(lf_source, &lf_with_row, &row_edit);
        let reparsed = first_table(&lf_with_row);
        assert_eq!(reparsed.rows.len(), 2);
        assert_eq!(
            &lf_with_row[reparsed.rows[0].cells[0].content_range.clone()],
            "H"
        );

        let column_edit = lf_table.insert_column(lf_source, 1).expect("column");
        let lf_with_column = apply(lf_source, column_edit.clone());
        assert_eq!(lf_with_column, "before\n| H |  |\n| --- | --- |\n\nafter");
        assert_unchanged_outside(lf_source, &lf_with_column, &column_edit);

        let alignment_edit = lf_table
            .set_alignment(0, TableAlignment::Right)
            .expect("alignment");
        let lf_aligned = apply(lf_source, alignment_edit.clone());
        assert_eq!(lf_aligned, "before\n| H |\n| ---: |\n\nafter");
        assert_unchanged_outside(lf_source, &lf_aligned, &alignment_edit);

        let crlf_source = "before\n| H |\r\n| --- |\r\n\r\nafter";
        let crlf_table = first_table(crlf_source);
        let delimiter_end = crlf_source.find("| --- |").expect("delimiter") + "| --- |".len();
        assert_eq!(crlf_table.range.end, delimiter_end);

        let row_edit = crlf_table
            .insert_row(crlf_source, 1)
            .expect("CRLF first data row");
        let crlf_with_row = apply(crlf_source, row_edit.clone());
        assert_eq!(
            crlf_with_row,
            "before\n| H |\r\n| --- |\r\n|  |\r\n\r\nafter"
        );
        assert_unchanged_outside(crlf_source, &crlf_with_row, &row_edit);

        let column_edit = crlf_table
            .insert_column(crlf_source, 1)
            .expect("CRLF column");
        let crlf_with_column = apply(crlf_source, column_edit.clone());
        assert_eq!(
            crlf_with_column,
            "before\n| H |  |\r\n| --- | --- |\r\n\r\nafter"
        );
        assert_unchanged_outside(crlf_source, &crlf_with_column, &column_edit);

        let alignment_edit = crlf_table
            .set_alignment(0, TableAlignment::Right)
            .expect("CRLF alignment");
        let crlf_aligned = apply(crlf_source, alignment_edit.clone());
        assert_eq!(crlf_aligned, "before\n| H |\r\n| ---: |\r\n\r\nafter");
        assert_unchanged_outside(crlf_source, &crlf_aligned, &alignment_edit);

        let mixed_source = "before\n| H |\r\n| --- |\n\nafter";
        let mixed_table = first_table(mixed_source);
        let mixed_edit = mixed_table
            .insert_row(mixed_source, 1)
            .expect("table EOL wins over following paragraph EOL");
        let mixed_result = apply(mixed_source, mixed_edit.clone());
        assert_eq!(mixed_result, "before\n| H |\r\n| --- |\r\n|  |\n\nafter");
        assert_unchanged_outside(mixed_source, &mixed_result, &mixed_edit);
    }

    #[test]
    fn rectangular_clipboard_is_bounded_ragged_input_is_rejected_and_pipes_are_escaped() {
        let source =
            "before\n| h1 | h2 | h3 |\n| --- | --- | --- |\n| a | b | c |\n| d | e | f |\n\nafter";
        let table = first_table(source);
        let rect = TableRect {
            start: TablePosition { row: 1, column: 0 },
            end: TablePosition { row: 2, column: 1 },
        };
        assert_eq!(table.copy_tsv(source, rect), Some("a\tb\nd\te".to_owned()));
        let cut = table.cut_tsv(source, rect).expect("cut");
        assert_eq!(cut.tsv, "a\tb\nd\te");
        let cut_edit = cut.edit.clone();
        let cut_source = apply(source, cut.edit);
        assert_unchanged_outside(source, &cut_source, &cut_edit);
        assert!(cut_source.contains("|  |  | c |\n|  |  | f |"));
        assert!(cut_source.starts_with("before\n"));
        assert!(cut_source.ends_with("\n\nafter"));

        let pasted = table
            .paste_tsv(
                source,
                TablePosition { row: 1, column: 0 },
                "one|part\ttwo\r\nthree\tfour\r\n",
            )
            .expect("bounded paste");
        let paste_edit = pasted.clone();
        let pasted_source = apply(source, pasted);
        assert_unchanged_outside(source, &pasted_source, &paste_edit);
        assert!(pasted_source.contains("| one\\|part | two | c |\n| three | four | f |"));
        assert!(
            table
                .paste_tsv(source, TablePosition { row: 2, column: 2 }, "x\ty")
                .is_none()
        );
        assert!(
            table
                .paste_tsv(source, TablePosition { row: 1, column: 0 }, "x\ty\nz")
                .is_none()
        );
        assert!(
            table
                .paste_tsv(source, TablePosition { row: 1, column: 0 }, "plain text")
                .is_none()
        );
    }

    #[test]
    fn escaped_structural_pipes_and_malformed_tables_are_not_reinterpreted() {
        let row = parse_table_row("| a\\|b | c |", 5);
        assert_eq!(row.cells.len(), 2);
        assert_eq!(&row.cells[0].range.start, &6);
        assert!(find_tables("| heading | other |\n| --- |\n| value | other |").is_empty());
        assert!(find_tables("heading | ordinary text\n--- | ---\nnot a table").is_empty());
    }
}
