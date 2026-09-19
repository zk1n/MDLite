#pragma once

#include <cstddef>
#include <optional>
#include <string>

namespace mdlite {

struct TableEditResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

enum class TableCaretDirection { Left, Right, Up, Down };

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret, bool backwards);
std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source, std::size_t caret,
                                                    TableCaretDirection direction);
TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret);
TableEditResult InsertTableColumn(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret);

}  // namespace mdlite
