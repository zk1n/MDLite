#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct TableCellIntent {
  std::size_t row_begin{};
  std::size_t column{};
  std::size_t source_position{};
  bool virtual_cell{};
};

struct TableEditResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
  std::optional<TableCellIntent> target_cell;
};

enum class TableCaretDirection { Left, Right, Up, Down };

struct TableVisualCell {
  std::size_t begin{};
  std::size_t end{};
};

struct TableVisualRow {
  std::size_t begin{};
  std::size_t end{};
  std::vector<TableVisualCell> cells;
};

enum class TableAlignment { Unspecified, Left, Center, Right };

struct GfmTable {
  std::size_t begin{};
  std::size_t end{};
  std::vector<TableAlignment> alignments;
  std::vector<TableVisualRow> rows;
};

std::optional<TableVisualCell> TableCellContentRange(std::wstring_view source,
                                                   const TableCellIntent& cell);
TableEditResult MoveTableCellCommand(std::wstring_view source, const TableCellIntent& cell,
                                    bool backwards, bool vertical);
TableEditResult LeaveTable(std::wstring_view source, std::size_t caret);
TableEditResult SetTableColumnAlignment(std::wstring_view source, std::size_t caret,
                                       TableAlignment alignment);
// These content operations preserve every structural separator and untouched
// cell's original padding. The endpoints identify a rectangle of visible cells.
std::wstring CopyTableRectangle(std::wstring_view source, const TableCellIntent& first,
                                const TableCellIntent& last);
TableEditResult ReplaceTableRectangle(std::wstring_view source, const TableCellIntent& first,
                                      const TableCellIntent& last, std::wstring_view tsv);

// Parses the GFM table containing position, if any. The result includes raw
// source ranges for every row and cell and the delimiter's per-column alignment.
// It recognizes the same fenced-code, delimiter, and ragged-body rules used by
// table editing; it never normalizes or edits source text.
std::optional<GfmTable> ParseGfmTableAt(std::wstring_view source, std::size_t position);

// Parses a table from a candidate header at a known line start without scanning
// the source prefix. The caller is responsible for candidate context such as
// fenced-code state; the row, delimiter, alignment, and body rules are shared
// with ParseGfmTableAt.
std::optional<GfmTable> ParseGfmTableAtHeader(std::wstring_view source,
                                             std::size_t header_begin);

// Returns true only when line is a GFM delimiter row. The helper accepts
// either pipe style (with or without outer pipes) and ignores surrounding
// cell whitespace; it never normalizes or edits the supplied source.
bool IsGfmTableDelimiter(std::wstring_view line);

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret, bool backwards);
TableEditResult MoveToAdjacentTableCell(std::wstring_view source,
                                        const TableCellIntent& current_cell,
                                        bool backwards);
std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source, std::size_t caret,
                                                    TableCaretDirection direction);
std::optional<TableCellIntent> ResolveTableCellIntent(std::wstring_view source,
                                                      std::size_t caret);
std::optional<TableCellIntent> ResolveTableCellIntent(const GfmTable& table,
                                                      std::wstring_view source,
                                                      std::size_t caret);
std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    std::wstring_view source, std::size_t caret, TableCaretDirection direction);
std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    std::wstring_view source, const TableCellIntent& current_cell,
    TableCaretDirection direction);
std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    const GfmTable& table, std::wstring_view source,
    const TableCellIntent& current_cell, TableCaretDirection direction);
TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret);
TableEditResult InsertTableColumn(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret);
// Inserts text into a missing trailing cell in a body row, adding any skipped
// empty cells and pipe separators. row_begin must identify the row's source start.
// The returned selection is immediately after inserted_text; unsafe or invalid
// targets return the unchanged source with changed=false.
TableEditResult InsertTextIntoMissingTableCell(std::wstring_view source,
                                               std::size_t row_begin,
                                               std::size_t target_column,
                                               std::wstring_view inserted_text);
std::vector<TableVisualRow> ParseTableVisualRows(std::wstring_view source,
                                                 std::size_t begin,
                                                 std::size_t end);

}  // namespace mdlite
