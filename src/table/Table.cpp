#include "table/Table.h"

#include <algorithm>
#include <cwctype>
#include <optional>
#include <ranges>
#include <utility>

namespace mdlite {
namespace {

struct Cell {
  std::size_t raw_begin{};
  std::size_t raw_end{};
  std::size_t begin{};
  std::size_t end{};
};

struct Row {
  std::size_t begin{};
  std::size_t end{};
  std::vector<Cell> cells;
};

struct TableBlock {
  std::size_t begin{};
  std::size_t end{};
  std::size_t current_row{};
  std::vector<Row> row_ranges;
  std::vector<std::vector<std::wstring>> rows;
};

struct RowChange {
  std::size_t begin{};
  std::size_t end{};
  std::wstring replacement;
};

std::pair<std::size_t, std::size_t> LineRange(std::wstring_view source,
                                               std::size_t position) {
  position = std::min(position, source.size());
  const auto previous_newline = position == 0
      ? std::wstring_view::npos : source.rfind(L'\n', position - 1);
  const std::size_t begin = previous_newline == std::wstring_view::npos
      ? 0 : previous_newline + 1;
  std::size_t end = source.find(L'\n', position);
  if (end == std::wstring_view::npos) end = source.size();
  if (end > begin && source[end - 1] == L'\r') --end;
  return {begin, end};
}

std::size_t NextLineStart(std::wstring_view source, std::size_t content_end) {
  std::size_t position = std::min(content_end, source.size());
  if (position < source.size() && source[position] == L'\r') ++position;
  if (position < source.size() && source[position] == L'\n') ++position;
  return position;
}

bool IsEscaped(std::wstring_view line, std::size_t index) {
  std::size_t slashes{};
  for (std::size_t before = index; before > 0 && line[before - 1] == L'\\'; --before)
    ++slashes;
  return (slashes % 2) != 0;
}

std::size_t NextCellSeparator(std::wstring_view line, std::size_t cursor) {
  std::size_t code_ticks{};
  for (std::size_t index = cursor; index < line.size(); ++index) {
    if (line[index] != L'`' || IsEscaped(line, index)) {
      if (line[index] == L'|' && code_ticks == 0 && !IsEscaped(line, index)) return index;
      continue;
    }
    std::size_t run = 1;
    while (index + run < line.size() && line[index + run] == L'`') ++run;
    if (code_ticks == 0) code_ticks = run;
    else if (code_ticks == run) code_ticks = 0;
    index += run - 1;
  }
  return std::wstring_view::npos;
}

std::pair<std::size_t, std::size_t> TrimCell(std::wstring_view source,
                                              std::size_t begin,
                                              std::size_t end) {
  while (begin < end && iswspace(source[begin])) ++begin;
  while (end > begin && iswspace(source[end - 1])) --end;
  return {begin, end};
}

std::optional<Row> ParseRow(std::wstring_view source, std::size_t line_begin,
                            std::size_t line_end) {
  if (line_end < line_begin || line_end > source.size()) return std::nullopt;
  const auto line = source.substr(line_begin, line_end - line_begin);
  const std::size_t first = !line.empty() && line.front() == L'|' ? 1 : 0;
  std::size_t cursor = first;
  std::vector<Cell> cells;
  bool had_separator{};
  while (cursor <= line.size()) {
    std::size_t separator = NextCellSeparator(line, cursor);
    if (separator == std::wstring_view::npos) separator = line.size();
    const auto [begin, end] = TrimCell(line, cursor, separator);
    cells.push_back({line_begin + cursor, line_begin + separator,
                     line_begin + begin, line_begin + end});
    if (separator == line.size()) break;
    had_separator = true;
    cursor = separator + 1;
    bool only_trailing_whitespace = true;
    for (std::size_t index = cursor; index < line.size(); ++index) {
      if (!iswspace(line[index])) {
        only_trailing_whitespace = false;
        break;
      }
    }
    if (only_trailing_whitespace) break;
    if (cursor == line.size()) break;
  }
  std::size_t last_non_space = line.size();
  while (last_non_space > 0 && iswspace(line[last_non_space - 1])) --last_non_space;
  const bool explicit_one_cell = cells.size() == 1 && last_non_space >= 2 &&
      line.front() == L'|' && line[last_non_space - 1] == L'|' && had_separator;
  if (!had_separator || (cells.size() < 2 && !explicit_one_cell)) return std::nullopt;
  return Row{line_begin, line_end, std::move(cells)};
}

std::vector<std::wstring> CellValues(std::wstring_view source, const Row& row) {
  std::vector<std::wstring> values;
  values.reserve(row.cells.size());
  for (const auto& cell : row.cells)
    values.emplace_back(source.substr(cell.begin, cell.end - cell.begin));
  return values;
}

bool IsDelimiterCell(std::wstring_view cell) {
  while (!cell.empty() && iswspace(cell.front())) cell.remove_prefix(1);
  while (!cell.empty() && iswspace(cell.back())) cell.remove_suffix(1);
  if (cell.empty()) return false;
  if (cell.front() == L':') cell.remove_prefix(1);
  if (!cell.empty() && cell.back() == L':') cell.remove_suffix(1);
  if (cell.size() < 3) return false;
  return std::ranges::all_of(cell, [](wchar_t character) { return character == L'-'; });
}

bool IsDelimiter(const Row& row, std::wstring_view source) {
  if (row.cells.empty()) return false;
  return std::ranges::all_of(row.cells, [&](const Cell& cell) {
    return IsDelimiterCell(source.substr(cell.begin, cell.end - cell.begin));
  });
}

bool SameColumnCount(const Row& left, const Row& right) {
  return left.cells.size() == right.cells.size();
}

bool FenceMarker(std::wstring_view line, wchar_t& marker) {
  std::size_t indent{};
  while (indent < line.size() && indent < 4 && line[indent] == L' ') ++indent;
  if (indent > 3 || line.size() - indent < 3 ||
      (line[indent] != L'`' && line[indent] != L'~')) return false;
  marker = line[indent];
  return line[indent + 1] == marker && line[indent + 2] == marker;
}

std::wstring_view PreferredLineBreak(std::wstring_view source, std::size_t position) {
  auto newline = source.find(L'\n', position);
  if (newline == std::wstring_view::npos && position != 0)
    newline = source.rfind(L'\n', position - 1);
  return newline != std::wstring_view::npos && newline > 0 && source[newline - 1] == L'\r'
      ? std::wstring_view(L"\r\n") : std::wstring_view(L"\n");
}

std::size_t CellAt(const Row& row, std::size_t caret) {
  for (std::size_t index{}; index < row.cells.size(); ++index) {
    const auto& cell = row.cells[index];
    if (caret <= cell.end || caret <= cell.raw_end) return index;
  }
  return row.cells.size() - 1;
}

std::optional<TableBlock> BlockAt(std::wstring_view source, std::size_t caret) {
  caret = std::min(caret, source.size());
  const auto [target_begin, target_end] = LineRange(source, caret);
  std::size_t line_begin{};
  bool in_fence{};
  wchar_t fence_marker{};
  while (line_begin <= source.size()) {
    const auto [line_start, line_end] = LineRange(source, line_begin);
    const auto line = source.substr(line_start, line_end - line_start);
    wchar_t current_fence{};
    if (FenceMarker(line, current_fence)) {
      if (!in_fence) {
        in_fence = true;
        fence_marker = current_fence;
      } else if (current_fence == fence_marker) {
        in_fence = false;
      }
      const auto next = NextLineStart(source, line_end);
      if (next == line_end) break;
      line_begin = next;
      continue;
    }
    if (in_fence) {
      const auto next = NextLineStart(source, line_end);
      if (next == line_end) break;
      line_begin = next;
      continue;
    }
    const auto header = ParseRow(source, line_start, line_end);
    const std::size_t delimiter_begin = NextLineStart(source, line_end);
    if (header && delimiter_begin != line_end) {
      const auto [delimiter_start, delimiter_end] = LineRange(source, delimiter_begin);
      const auto delimiter = ParseRow(source, delimiter_start, delimiter_end);
      if (delimiter && SameColumnCount(*header, *delimiter) &&
          IsDelimiter(*delimiter, source)) {
        TableBlock block{header->begin, delimiter->end, 0, {}, {}};
        block.row_ranges.push_back(*header);
        block.rows.push_back(CellValues(source, *header));
        block.row_ranges.push_back(*delimiter);
        block.rows.push_back(CellValues(source, *delimiter));
        std::size_t next = NextLineStart(source, delimiter->end);
        while (next != delimiter->end && next <= source.size()) {
          const auto [body_start, body_end] = LineRange(source, next);
          const auto body = ParseRow(source, body_start, body_end);
          if (!body) break;
          block.row_ranges.push_back(*body);
          block.rows.push_back(CellValues(source, *body));
          block.end = body->end;
          const auto after = NextLineStart(source, body->end);
          if (after == body->end) break;
          next = after;
        }
        for (std::size_t index{}; index < block.row_ranges.size(); ++index) {
          if (block.row_ranges[index].begin == target_begin &&
              target_end >= block.row_ranges[index].begin) {
            block.current_row = index;
            return block;
          }
        }
        line_begin = block.end;
        const auto after = NextLineStart(source, block.end);
        if (after == block.end) break;
        line_begin = after;
        continue;
      }
    }
    const auto next = NextLineStart(source, line_end);
    if (next == line_end) break;
    line_begin = next;
  }
  return std::nullopt;
}

std::wstring RenderRow(std::size_t columns) {
  std::wstring row = L"|";
  for (std::size_t index{}; index < columns; ++index) row += L"  |";
  return row;
}

std::wstring InsertedColumnText(std::wstring_view value, bool before_existing_cell) {
  return before_existing_cell ? std::wstring(value) + L" | " : L" | " + std::wstring(value);
}

std::wstring ApplyRowChanges(std::wstring_view source, std::vector<RowChange> changes) {
  std::wstring result(source);
  std::ranges::sort(changes, {}, &RowChange::begin);
  for (auto change = changes.rbegin(); change != changes.rend(); ++change)
    result.replace(change->begin, change->end - change->begin, change->replacement);
  return result;
}

std::size_t RowCaretAfterChanges(std::wstring_view source, const TableBlock& block,
                                 std::size_t selected_column,
                                 const std::vector<RowChange>& changes) {
  std::size_t current_begin = block.row_ranges[block.current_row].begin;
  for (const auto& change : changes) {
    if (change.begin < current_begin) {
      current_begin += change.replacement.size();
      current_begin -= change.end - change.begin;
    }
  }
  const auto current_block = BlockAt(source, current_begin);
  if (!current_block) return current_begin;
  const auto& row = current_block->row_ranges[current_block->current_row];
  return row.cells[std::min(selected_column, row.cells.size() - 1)].begin;
}

std::optional<std::pair<std::size_t, std::size_t>> ColumnDeletionRange(
    std::wstring_view source, const Row& row, std::size_t column) {
  if (column >= row.cells.size()) return std::nullopt;
  if (column == 0) {
    const auto& cell = row.cells.front();
    const std::size_t end = cell.raw_end < row.end && source[cell.raw_end] == L'|'
        ? cell.raw_end + 1 : cell.raw_end;
    return std::pair{cell.raw_begin, end};
  }
  return std::pair{row.cells[column - 1].raw_end, row.cells[column].raw_end};
}

TableEditResult NoTableEdit(std::wstring_view source, std::size_t caret) {
  return {std::wstring(source), std::min(caret, source.size()), false};
}

}  // namespace

bool IsGfmTableDelimiter(std::wstring_view line) {
  if (!line.empty() && line.back() == L'\r') line.remove_suffix(1);
  const auto row = ParseRow(line, 0, line.size());
  return row && IsDelimiter(*row, line);
}

std::vector<TableVisualRow> ParseTableVisualRows(std::wstring_view source,
                                                 std::size_t begin,
                                                 std::size_t end) {
  std::vector<TableVisualRow> rows;
  begin = std::min(begin, source.size());
  end = std::min(end, source.size());
  if (begin >= end) return rows;
  auto block = BlockAt(source, begin);
  if (!block) block = BlockAt(source, end - 1);
  if (!block) return rows;
  for (const auto& row : block->row_ranges) {
    if (row.begin >= end || row.end < begin) continue;
    TableVisualRow visual{row.begin, row.end, {}};
    visual.cells.reserve(row.cells.size());
    for (const auto& cell : row.cells)
      visual.cells.push_back({cell.raw_begin, cell.raw_end});
    rows.push_back(std::move(visual));
  }
  return rows;
}

TableEditResult MoveToAdjacentTableCell(std::wstring_view source, std::size_t caret,
                                        bool backwards) {
  const auto block = BlockAt(source, caret);
  if (!block) return NoTableEdit(source, caret);
  const auto& row = block->row_ranges[block->current_row];
  const std::size_t cell = CellAt(row, caret);
  if (backwards && cell > 0)
    return {std::wstring(source), row.cells[cell - 1].begin, false};
  if (!backwards && cell + 1 < row.cells.size())
    return {std::wstring(source), row.cells[cell + 1].begin, false};
  if (backwards) {
    if (block->current_row == 0) return NoTableEdit(source, caret);
    const auto& previous = block->row_ranges[block->current_row - 1];
    return {std::wstring(source), previous.cells.back().begin, false};
  }
  if (block->current_row + 1 < block->row_ranges.size()) {
    const auto& next = block->row_ranges[block->current_row + 1];
    return {std::wstring(source), next.cells.front().begin, false};
  }
  if (IsDelimiter(row, source)) return NoTableEdit(source, caret);
  const std::wstring new_row = RenderRow(row.cells.size());
  const std::wstring line_break(PreferredLineBreak(source, row.begin));
  const std::size_t next = NextLineStart(source, row.end);
  const bool at_eof = next == row.end;
  const std::size_t insertion = at_eof ? row.end : next;
  const std::wstring inserted = at_eof ? line_break + new_row : new_row + line_break;
  std::wstring result(source);
  result.insert(insertion, inserted);
  const std::size_t selection = insertion + (at_eof ? line_break.size() : 0) + 2;
  return {std::move(result), selection, true};
}

std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source,
                                                    std::size_t caret,
                                                    TableCaretDirection direction) {
  const auto block = BlockAt(source, caret);
  if (!block) return std::nullopt;
  const auto& row = block->row_ranges[block->current_row];
  const std::size_t cell = CellAt(row, caret);
  if (direction == TableCaretDirection::Left) {
    if (caret > row.cells[cell].begin || cell == 0) return std::nullopt;
    return row.cells[cell - 1].end;
  }
  if (direction == TableCaretDirection::Right) {
    if (caret < row.cells[cell].end || cell + 1 >= row.cells.size()) return std::nullopt;
    return row.cells[cell + 1].begin;
  }
  const int step = direction == TableCaretDirection::Up ? -1 : 1;
  auto target = static_cast<long long>(block->current_row) + step;
  while (target >= 0 && target < static_cast<long long>(block->row_ranges.size()) &&
         static_cast<std::size_t>(target) == 1) {
    target += step;
  }
  if (target < 0 || target >= static_cast<long long>(block->row_ranges.size()))
    return std::nullopt;
  const auto& adjacent = block->row_ranges[static_cast<std::size_t>(target)];
  return adjacent.cells[std::min(cell, adjacent.cells.size() - 1)].begin;
}

TableEditResult InsertTableRow(std::wstring_view source, std::size_t caret, bool after) {
  const auto block = BlockAt(source, caret);
  if (!block) return NoTableEdit(source, caret);
  const auto& row = block->row_ranges[block->current_row];
  if (IsDelimiter(row, source)) return NoTableEdit(source, caret);
  const std::wstring new_row = RenderRow(row.cells.size());
  const std::wstring line_break(PreferredLineBreak(source, row.begin));
  std::wstring result(source);
  if (!after) {
    result.insert(row.begin, new_row + line_break);
    return {std::move(result), row.begin + 2, true};
  }
  const auto next = NextLineStart(source, row.end);
  if (next == row.end) {
    result.insert(row.end, line_break + new_row);
    return {std::move(result), row.end + line_break.size() + 2, true};
  }
  result.insert(next, new_row + line_break);
  return {std::move(result), next + 2, true};
}

TableEditResult DeleteTableRow(std::wstring_view source, std::size_t caret) {
  const auto block = BlockAt(source, caret);
  if (!block) return NoTableEdit(source, caret);
  const auto& row = block->row_ranges[block->current_row];
  if (IsDelimiter(row, source)) return NoTableEdit(source, caret);
  std::size_t erase_begin = row.begin;
  std::size_t erase_end = row.end;
  const auto next = NextLineStart(source, row.end);
  if (next > row.end) erase_end = next;
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
  const auto block = BlockAt(source, caret);
  if (!block) return NoTableEdit(source, caret);
  const auto& current = block->row_ranges[block->current_row];
  const std::size_t target = CellAt(current, caret) + (after ? 1 : 0);
  std::vector<RowChange> changes;
  changes.reserve(block->row_ranges.size());
  for (std::size_t index{}; index < block->row_ranges.size(); ++index) {
    const auto& row = block->row_ranges[index];
    const auto& cells = block->rows[index];
    const std::size_t insertion_cell = std::min(target, cells.size());
    const bool before_existing_cell = insertion_cell < cells.size();
    const std::size_t insertion = before_existing_cell
        ? row.cells[insertion_cell].begin : row.cells.back().end;
    const std::wstring value = IsDelimiter(row, source) ? L"---" : L"";
    changes.push_back({insertion, insertion,
                       InsertedColumnText(value, before_existing_cell)});
  }
  const std::wstring result = ApplyRowChanges(source, changes);
  return {result, RowCaretAfterChanges(result, *block, target, changes), true};
}

TableEditResult DeleteTableColumn(std::wstring_view source, std::size_t caret) {
  const auto block = BlockAt(source, caret);
  if (!block) return NoTableEdit(source, caret);
  const auto& current = block->row_ranges[block->current_row];
  if (current.cells.size() <= 1) return NoTableEdit(source, caret);
  const std::size_t target = CellAt(current, caret);
  std::vector<RowChange> changes;
  changes.reserve(block->row_ranges.size());
  for (const auto& row : block->row_ranges) {
    if (row.cells.size() <= 1 || target >= row.cells.size()) continue;
    const auto deletion = ColumnDeletionRange(source, row, target);
    if (deletion) changes.push_back({deletion->first, deletion->second, L""});
  }
  if (changes.empty()) return NoTableEdit(source, caret);
  const std::wstring result = ApplyRowChanges(source, changes);
  const std::size_t selected_column = target == 0 ? 0 : target - 1;
  return {result, RowCaretAfterChanges(result, *block, selected_column, changes), true};
}

}  // namespace mdlite
