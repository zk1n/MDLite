#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct TableEditResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
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

// Returns true only when line is a GFM delimiter row. The helper accepts
// either pipe style (with or without outer pipes) and ignores surrounding
// cell whitespace; it never normalizes or edits the supplied source.
bool IsGfmTableDelimiter(std::wstring_view line);

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret, bool backwards);
std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source, std::size_t caret,
                                                    TableCaretDirection direction);
TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret);
TableEditResult InsertTableColumn(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret);
std::vector<TableVisualRow> ParseTableVisualRows(std::wstring_view source,
                                                 std::size_t begin,
                                                 std::size_t end);

}  // namespace mdlite
