#include "table/Table.h"

#include <algorithm>
#include <cwctype>
#include <optional>
#include <tuple>
#include <vector>

namespace mdlite {
namespace {

struct Row {
  std::size_t begin{};
  std::size_t end{};
  std::vector<std::pair<std::size_t, std::size_t>> cells;
};

struct TableBlock {
  std::size_t begin{};
  std::size_t end{};
  std::size_t current_row{};
  std::vector<std::vector<std::wstring>> rows;
};

std::pair<std::size_t, std::size_t> LineRange(std::wstring_view source, std::size_t position) {
  position = std::min(position, source.size());
  const std::size_t begin = position == 0 ? 0 : source.rfind(L'\n', position - 1) + 1;
  std::size_t end = source.find(L'\n', position);
  if (end == std::wstring_view::npos) end = source.size();
  if (end > begin && source[end - 1] == L'\r') --end;
  return {begin, end};
}

std::wstring_view PreferredLineBreak(std::wstring_view source, std::size_t position) {
  auto newline = source.find(L'\n', position);
  if (newline == std::wstring_view::npos && position != 0)
    newline = source.rfind(L'\n', position - 1);
  return newline != std::wstring_view::npos && newline > 0 && source[newline - 1] == L'\r'
      ? std::wstring_view(L"\r\n")
      : std::wstring_view(L"\n");
}

std::size_t NextLineStart(std::wstring_view source, std::size_t content_end) {
  std::size_t position = std::min(content_end, source.size());
  if (position < source.size() && source[position] == L'\r') ++position;
  if (position < source.size() && source[position] == L'\n') ++position;
  return position;
}

std::size_t NextCellSeparator(std::wstring_view line, std::size_t cursor) {
  bool code{};
  for (std::size_t index = cursor; index < line.size(); ++index) {
    std::size_t slashes{};
    for (std::size_t before = index; before > 0 && line[before - 1] == L'\\'; --before) ++slashes;
    const bool escaped = slashes % 2 != 0;
    if (line[index] == L'`' && !escaped) code = !code;
    else if (line[index] == L'|' && !escaped && !code) return index;
  }
  return std::wstring_view::npos;
}

std::optional<Row> ParseRow(std::wstring_view source, std::size_t line_begin, std::size_t line_end) {
  const auto line = source.substr(line_begin, line_end - line_begin);
  if (line.find(L'|') == std::wstring_view::npos) return std::nullopt;
  Row row{line_begin, line_end, {}};
  std::size_t cursor{};
  if (!line.empty() && line.front() == L'|') cursor = 1;
  while (cursor <= line.size()) {
    std::size_t separator = NextCellSeparator(line, cursor);
    if (separator == std::wstring_view::npos) separator = line.size();
    std::size_t begin = cursor;
    std::size_t end = separator;
    while (begin < end && iswspace(line[begin])) ++begin;
    while (end > begin && iswspace(line[end - 1])) --end;
    row.cells.emplace_back(line_begin + begin, line_begin + end);
    if (separator == line.size()) break;
    cursor = separator + 1;
    if (cursor == line.size()) break;
  }
  return row.cells.size() >= 2 ? std::optional<Row>(std::move(row)) : std::nullopt;
}

std::optional<Row> RowAt(std::wstring_view source, std::size_t caret) {
  const auto [begin, end] = LineRange(source, caret);
  return ParseRow(source, begin, end);
}

std::size_t CellAt(const Row& row, std::size_t caret) {
  for (std::size_t index = 0; index < row.cells.size(); ++index) {
    if (caret <= row.cells[index].second) return index;
  }
  return row.cells.size() - 1;
}

std::vector<std::wstring> CellValues(std::wstring_view source, const Row& row) {
  std::vector<std::wstring> values;
  for (const auto [begin, end] : row.cells) values.emplace_back(source.substr(begin, end - begin));
  return values;
}

std::wstring RenderRow(const std::vector<std::wstring>& cells) {
  std::wstring row = L"|";
  for (const auto& cell : cells) row += L" " + cell + L" |";
  return row;
}

bool IsDelimiter(const std::vector<std::wstring>& cells) {
  for (const auto& cell : cells) {
    std::size_t dashes{};
    for (wchar_t character : cell) {
      if (character == L'-') ++dashes;
      else if (character != L':' && !iswspace(character)) return false;
    }
    if (dashes < 3) return false;
  }
  return !cells.empty();
}

std::optional<TableBlock> BlockAt(std::wstring_view source, std::size_t caret) {
  const auto current = RowAt(source, caret);
  if (!current) return std::nullopt;
  std::size_t block_begin = current->begin;
  while (block_begin > 0) {
    const auto [previous_begin, previous_end] = LineRange(source, block_begin - 1);
    if (!ParseRow(source, previous_begin, previous_end)) break;
    block_begin = previous_begin;
  }
  TableBlock block{block_begin, block_begin, 0, {}};
  std::size_t line_begin = block_begin;
  while (line_begin <= source.size()) {
    const auto [begin, end] = LineRange(source, line_begin);
    const auto row = ParseRow(source, begin, end);
    if (!row) break;
    if (begin == current->begin) block.current_row = block.rows.size();
    block.rows.push_back(CellValues(source, *row));
    block.end = end;
    const std::size_t next = NextLineStart(source, end);
    if (next == end) break;
    line_begin = next;
  }
  return block.rows.size() >= 2 ? std::optional<TableBlock>(std::move(block)) : std::nullopt;
}

TableEditResult RenderBlock(std::wstring_view source, const TableBlock& block,
                            std::size_t selected_column) {
  std::wstring rendered;
  const std::wstring line_break(PreferredLineBreak(source, block.begin));
  std::size_t selection_in_block{};
  for (std::size_t row_index = 0; row_index < block.rows.size(); ++row_index) {
    if (row_index != 0) rendered += line_break;
    const std::size_t row_begin = rendered.size();
    const std::wstring row_text = RenderRow(block.rows[row_index]);
    if (row_index == block.current_row) {
      const auto row = ParseRow(row_text, 0, row_text.size());
      if (row) selection_in_block = row_begin + row->cells[std::min(selected_column, row->cells.size() - 1)].first;
    }
    rendered += row_text;
  }
  std::wstring result(source);
  result.replace(block.begin, block.end - block.begin, rendered);
  return {std::move(result), block.begin + selection_in_block, true};
}

TableEditResult ReplaceLine(std::wstring_view source, const Row& row, std::wstring replacement,
                            std::size_t selection) {
  std::wstring result(source);
  result.replace(row.begin, row.end - row.begin, replacement);
  return {std::move(result), selection, true};
}

}  // namespace

std::vector<TableVisualRow> ParseTableVisualRows(std::wstring_view source,
                                                 std::size_t begin,
                                                 std::size_t end) {
  std::vector<TableVisualRow> rows;
  begin = std::min(begin, source.size());
  end = std::min(end, source.size());
  for (std::size_t line_begin = begin; line_begin < end;) {
    std::size_t line_end = source.find(L'\n', line_begin);
    if (line_end == std::wstring_view::npos || line_end > end) line_end = end;
    std::size_t content_end = line_end;
    if (content_end > line_begin && source[content_end - 1] == L'\r') --content_end;
    const auto line = source.substr(line_begin, content_end - line_begin);
    TableVisualRow row{line_begin, content_end, {}};
    std::size_t cursor = !line.empty() && line.front() == L'|' ? 1 : 0;
    while (cursor <= line.size()) {
      std::size_t separator = NextCellSeparator(line, cursor);
      if (separator == std::wstring_view::npos) separator = line.size();
      row.cells.push_back({line_begin + cursor, line_begin + separator});
      if (separator == line.size()) break;
      cursor = separator + 1;
      if (cursor == line.size()) break;
    }
    if (row.cells.size() >= 2) rows.push_back(std::move(row));
    if (line_end >= end) break;
    line_begin = line_end + 1;
  }
  return rows;
}

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret, bool backwards) {
  const auto row = RowAt(source, caret);
  if (!row) return {std::wstring(source), caret, false};
  const std::size_t cell = CellAt(*row, caret);
  if (backwards && cell > 0) return {std::wstring(source), row->cells[cell - 1].first, false};
  if (!backwards && cell + 1 < row->cells.size()) return {std::wstring(source), row->cells[cell + 1].first, false};
  std::size_t adjacent_position{};
  if (backwards) {
    if (row->begin == 0) return {std::wstring(source), caret, false};
    adjacent_position = row->begin - 1;
  } else {
    adjacent_position = NextLineStart(source, row->end);
  }
  const auto adjacent = (backwards || adjacent_position > row->end)
      ? RowAt(source, adjacent_position)
      : std::nullopt;
  if (adjacent) return {std::wstring(source), backwards ? adjacent->cells.back().first : adjacent->cells.front().first, false};
  if (backwards) return {std::wstring(source), caret, false};

  const auto values = CellValues(source, *row);
  if (IsDelimiter(values)) return {std::wstring(source), caret, false};
  const std::wstring new_row = RenderRow(std::vector<std::wstring>(values.size(), L""));
  std::wstring result(source);
  const std::wstring line_break(PreferredLineBreak(source, row->begin));
  const std::size_t next = NextLineStart(source, row->end);
  const bool at_eof = next == row->end;
  const std::size_t insertion = at_eof ? row->end : next;
  const std::wstring inserted = at_eof ? line_break + new_row : new_row + line_break;
  result.insert(insertion, inserted);
  const std::size_t selection = insertion + (at_eof ? line_break.size() : 0) + 2;
  return {std::move(result), selection, true};
}

std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source, std::size_t caret,
                                                    TableCaretDirection direction) {
  const auto row = RowAt(source, caret);
  if (!row) return std::nullopt;
  const std::size_t cell = CellAt(*row, caret);
  const auto [begin, end] = row->cells[cell];
  if (direction == TableCaretDirection::Left) {
    if (caret > begin || cell == 0) return std::nullopt;
    return row->cells[cell - 1].second;
  }
  if (direction == TableCaretDirection::Right) {
    if (caret < end || cell + 1 >= row->cells.size()) return std::nullopt;
    return row->cells[cell + 1].first;
  }
  std::size_t adjacent_position{};
  if (direction == TableCaretDirection::Up) {
    if (row->begin == 0) return std::nullopt;
    adjacent_position = row->begin - 1;
  } else {
    adjacent_position = NextLineStart(source, row->end);
    if (adjacent_position == row->end) return std::nullopt;
  }
  auto adjacent = RowAt(source, adjacent_position);
  if (adjacent && IsDelimiter(CellValues(source, *adjacent))) {
    if (direction == TableCaretDirection::Up) {
      if (adjacent->begin == 0) return std::nullopt;
      adjacent = RowAt(source, adjacent->begin - 1);
    } else {
      const auto next = NextLineStart(source, adjacent->end);
      if (next == adjacent->end) return std::nullopt;
      adjacent = RowAt(source, next);
    }
  }
  if (!adjacent) return std::nullopt;
  return adjacent->cells[std::min(cell, adjacent->cells.size() - 1)].first;
}

TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after) {
  const auto row = RowAt(source, caret);
  if (!row) return {std::wstring(source), caret, false};
  const std::wstring new_row = RenderRow(std::vector<std::wstring>(row->cells.size(), L""));
  std::wstring result(source);
  const std::wstring line_break(PreferredLineBreak(source, row->begin));
  if (!after) {
    result.insert(row->begin, new_row + line_break);
    return {std::move(result), row->begin + 2, true};
  }
  const auto next = NextLineStart(source, row->end);
  if (next == row->end) {
    result.insert(row->end, line_break + new_row);
    return {std::move(result), row->end + line_break.size() + 2, true};
  }
  result.insert(next, new_row + line_break);
  return {std::move(result), next + 2, true};
}

TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret) {
  const auto row = RowAt(source, caret);
  if (!row || IsDelimiter(CellValues(source, *row))) return {std::wstring(source), caret, false};
  std::size_t erase_begin = row->begin;
  std::size_t erase_end = row->end;
  const auto next = NextLineStart(source, erase_end);
  if (next > erase_end) erase_end = next;
  else if (erase_begin > 0) {
    --erase_begin;
    if (erase_begin > 0 && source[erase_begin] == L'\n' && source[erase_begin - 1] == L'\r')
      --erase_begin;
  }
  std::wstring result(source);
  result.erase(erase_begin, erase_end - erase_begin);
  return {std::move(result), erase_begin, true};
}

TableEditResult InsertTableColumn(std::wstring_view source, std::size_t caret, bool after) {
  const auto current = RowAt(source, caret);
  if (!current) return {std::wstring(source), caret, false};
  const std::size_t target = CellAt(*current, caret) + (after ? 1 : 0);
  auto block = BlockAt(source, caret);
  if (!block) return {std::wstring(source), caret, false};
  for (auto& cells : block->rows) {
    const bool delimiter = IsDelimiter(cells);
    cells.insert(cells.begin() + static_cast<std::ptrdiff_t>(std::min(target, cells.size())),
                 delimiter ? L"---" : L"");
  }
  return RenderBlock(source, *block, target);
}

TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret) {
  const auto current = RowAt(source, caret);
  if (!current || current->cells.size() <= 1) return {std::wstring(source), caret, false};
  const std::size_t target = CellAt(*current, caret);
  auto block = BlockAt(source, caret);
  if (!block) return {std::wstring(source), caret, false};
  for (auto& cells : block->rows) {
    if (target >= cells.size() || cells.size() <= 1) return {std::wstring(source), caret, false};
    cells.erase(cells.begin() + static_cast<std::ptrdiff_t>(target));
  }
  return RenderBlock(source, *block, target);
}

}  // namespace mdlite
