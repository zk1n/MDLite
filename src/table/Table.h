#pragma once

#include <cstddef>
#include <string>

namespace mdlite {

struct TableEditResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret, bool backwards);
TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret);
TableEditResult InsertTableColumn(std::wstring_view source, std::size_t caret, bool after);
TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret);

}  // namespace mdlite
