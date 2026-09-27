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
  std::vector<TableAlignment> alignments;
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

std::optional<TableAlignment> ParseDelimiterCell(std::wstring_view cell) {
  while (!cell.empty() && iswspace(cell.front())) cell.remove_prefix(1);
  while (!cell.empty() && iswspace(cell.back())) cell.remove_suffix(1);
  if (cell.empty()) return std::nullopt;
  const bool left = cell.front() == L':';
  if (left) cell.remove_prefix(1);
  const bool right = !cell.empty() && cell.back() == L':';
  if (right) cell.remove_suffix(1);
  if (cell.empty() ||
      !std::ranges::all_of(cell, [](wchar_t character) { return character == L'-'; }))
    return std::nullopt;
  if (left && right) return TableAlignment::Center;
  if (left) return TableAlignment::Left;
  if (right) return TableAlignment::Right;
  return TableAlignment::Unspecified;
}

std::optional<std::vector<TableAlignment>> ParseDelimiterAlignments(
    const Row& row, std::wstring_view source) {
  if (row.cells.empty()) return std::nullopt;
  std::vector<TableAlignment> alignments;
  alignments.reserve(row.cells.size());
  for (const auto& cell : row.cells) {
    const auto alignment = ParseDelimiterCell(
        source.substr(cell.begin, cell.end - cell.begin));
    if (!alignment) return std::nullopt;
    alignments.push_back(*alignment);
  }
  return alignments;
}

bool IsDelimiter(const Row& row, std::wstring_view source) {
  return ParseDelimiterAlignments(row, source).has_value();
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

bool IsIndentedCodeLine(std::wstring_view line) {
  std::size_t indent{};
  for (const auto character : line) {
    if (character == L' ') {
      ++indent;
    } else if (character == L'\t') {
      indent += 4 - indent % 4;
    } else {
      break;
    }
    if (indent >= 4) return true;
  }
  return false;
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

std::optional<TableBlock> ParseTableAtHeader(std::wstring_view source,
                                             std::size_t header_begin) {
  if (header_begin > source.size()) return std::nullopt;
  const auto [header_start, header_end] = LineRange(source, header_begin);
  if (header_start != header_begin) return std::nullopt;
  const auto header = ParseRow(source, header_start, header_end);
  const std::size_t delimiter_begin = NextLineStart(source, header_end);
  if (!header || delimiter_begin == header_end || delimiter_begin > source.size())
    return std::nullopt;

  const auto [delimiter_start, delimiter_end] = LineRange(source, delimiter_begin);
  const auto delimiter = ParseRow(source, delimiter_start, delimiter_end);
  const auto alignments = delimiter
      ? ParseDelimiterAlignments(*delimiter, source) : std::nullopt;
  if (!delimiter || !SameColumnCount(*header, *delimiter) || !alignments)
    return std::nullopt;

  TableBlock block{header->begin, delimiter->end, 0, {}, {}, *alignments};
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
  return block;
}

GfmTable PublicTable(const TableBlock& block) {
  GfmTable table{block.begin, block.end, block.alignments, {}};
  table.rows.reserve(block.row_ranges.size());
  for (const auto& row : block.row_ranges) {
    TableVisualRow visual{row.begin, row.end, {}};
    visual.cells.reserve(row.cells.size());
    for (const auto& cell : row.cells)
      visual.cells.push_back({cell.raw_begin, cell.raw_end});
    table.rows.push_back(std::move(visual));
  }
  return table;
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
    if (IsIndentedCodeLine(line)) {
      const auto next = NextLineStart(source, line_end);
      if (next == line_end) break;
      line_begin = next;
      continue;
    }
    const std::size_t delimiter_begin = NextLineStart(source, line_end);
    if (delimiter_begin != line_end) {
      auto block = ParseTableAtHeader(source, line_start);
      if (block) {
        for (std::size_t index{}; index < block->row_ranges.size(); ++index) {
          if (block->row_ranges[index].begin == target_begin &&
              target_end >= block->row_ranges[index].begin) {
            block->current_row = index;
            return block;
          }
        }
        line_begin = block->end;
        const auto after = NextLineStart(source, block->end);
        if (after == block->end) break;
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

std::optional<TableCellIntent> IntentAtColumn(const TableBlock& block,
                                              std::size_t row_index,
                                              std::size_t column) {
  if (row_index >= block.row_ranges.size() || row_index == 1 ||
      column >= block.alignments.size()) return std::nullopt;
  const auto& row = block.row_ranges[row_index];
  if (row.cells.empty()) return std::nullopt;
  const bool virtual_cell = column >= row.cells.size();
  const std::size_t source_position = virtual_cell
      ? row.cells.back().raw_end : row.cells[column].begin;
  return TableCellIntent{row.begin, column, source_position, virtual_cell};
}

std::optional<std::size_t> NextVisibleRow(const TableBlock& block,
                                          std::size_t row_index) {
  for (std::size_t next = row_index + 1; next < block.row_ranges.size(); ++next) {
    if (next != 1) return next;
  }
  return std::nullopt;
}

std::optional<std::size_t> PreviousVisibleRow(std::size_t row_index) {
  while (row_index > 0) {
    --row_index;
    if (row_index != 1) return row_index;
  }
  return std::nullopt;
}

std::optional<std::size_t> GfmTableRowAt(const GfmTable& table,
                                         std::wstring_view source,
                                         std::size_t caret) {
  if (table.rows.empty() || caret < table.begin || caret > table.end) return std::nullopt;
  const std::size_t line_begin = LineRange(source, caret).first;
  const auto row = std::lower_bound(table.rows.begin(), table.rows.end(), line_begin,
      [](const TableVisualRow& candidate, std::size_t begin) {
        return candidate.begin < begin;
      });
  if (row == table.rows.end() || row->begin != line_begin) return std::nullopt;
  return static_cast<std::size_t>(row - table.rows.begin());
}

std::size_t GfmCellAt(const TableVisualRow& row, std::wstring_view source,
                     std::size_t caret) {
  for (std::size_t index{}; index < row.cells.size(); ++index) {
    const auto& cell = row.cells[index];
    const auto end = TrimCell(source, cell.begin, cell.end).second;
    if (caret <= end || caret <= cell.end) return index;
  }
  return row.cells.size() - 1;
}

std::optional<TableCellIntent> GfmIntentAtColumn(const GfmTable& table,
                                                 std::wstring_view source,
                                                 std::size_t row_index,
                                                 std::size_t column) {
  if (row_index >= table.rows.size() || row_index == 1 ||
      column >= table.alignments.size()) return std::nullopt;
  const auto& row = table.rows[row_index];
  if (row.cells.empty()) return std::nullopt;
  const bool virtual_cell = column >= row.cells.size();
  const auto source_position = virtual_cell
      ? row.cells.back().end
      : TrimCell(source, row.cells[column].begin, row.cells[column].end).first;
  return TableCellIntent{row.begin, column, source_position, virtual_cell};
}

std::optional<std::size_t> NextVisibleRow(const GfmTable& table,
                                          std::size_t row_index) {
  for (std::size_t next = row_index + 1; next < table.rows.size(); ++next) {
    if (next != 1) return next;
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

std::optional<GfmTable> ParseGfmTableAt(std::wstring_view source, std::size_t position) {
  const auto block = BlockAt(source, position);
  if (!block) return std::nullopt;
  return PublicTable(*block);
}

std::optional<GfmTable> ParseGfmTableAtHeader(std::wstring_view source,
                                              std::size_t header_begin) {
  const auto block = ParseTableAtHeader(source, header_begin);
  if (!block) return std::nullopt;
  return PublicTable(*block);
}

std::optional<TableCellIntent> ResolveTableCellIntent(std::wstring_view source,
                                                      std::size_t caret) {
  const auto block = BlockAt(source, caret);
  if (!block || block->row_ranges[block->current_row].cells.empty() ||
      block->alignments.empty() || block->current_row == 1) return std::nullopt;
  const auto& row = block->row_ranges[block->current_row];
  const std::size_t column = std::min(CellAt(row, caret), block->alignments.size() - 1);
  return IntentAtColumn(*block, block->current_row, column);
}

std::optional<TableCellIntent> ResolveTableCellIntent(const GfmTable& table,
                                                      std::wstring_view source,
                                                      std::size_t caret) {
  const auto row_index = GfmTableRowAt(table, source, caret);
  if (!row_index || *row_index == 1 || table.alignments.empty()) return std::nullopt;
  const auto& row = table.rows[*row_index];
  if (row.cells.empty()) return std::nullopt;
  const std::size_t column = std::min(GfmCellAt(row, source, caret),
                                      table.alignments.size() - 1);
  return GfmIntentAtColumn(table, source, *row_index, column);
}

bool IsGfmTableDelimiter(std::wstring_view line) {
  if (!line.empty() && line.back() == L'\r') line.remove_suffix(1);
  const auto row = ParseRow(line, 0, line.size());
  return row && IsDelimiter(*row, line);
}

TableEditResult InsertTextIntoMissingTableCell(std::wstring_view source,
                                               std::size_t row_begin,
                                               std::size_t target_column,
                                               std::wstring_view inserted_text) {
  for (std::size_t index{}; index < inserted_text.size(); ++index) {
    if (inserted_text[index] == L'\r' || inserted_text[index] == L'\n')
      return NoTableEdit(source, row_begin);
    if (inserted_text[index] != L'|') continue;
    std::size_t backslashes{};
    for (std::size_t previous = index; previous > 0 &&
         inserted_text[previous - 1] == L'\\'; --previous) ++backslashes;
    if (backslashes % 2 == 0) return NoTableEdit(source, row_begin);
  }

  const auto block = BlockAt(source, row_begin);
  if (!block || block->current_row < 2) return NoTableEdit(source, row_begin);
  const auto& row = block->row_ranges[block->current_row];
  const std::size_t column_count = block->alignments.size();
  if (row.begin != row_begin || row.cells.empty() || target_column >= column_count ||
      target_column < row.cells.size() || IsDelimiter(row, source) || inserted_text.empty())
    return NoTableEdit(source, row_begin);

  const auto& last_cell = row.cells.back();
  const bool has_trailing_pipe = last_cell.raw_end < row.end &&
      source[last_cell.raw_end] == L'|' &&
      std::ranges::all_of(source.substr(last_cell.raw_end + 1,
                                        row.end - last_cell.raw_end - 1),
                          [](wchar_t character) { return iswspace(character) != 0; });
  const std::size_t insertion = last_cell.raw_end;
  std::wstring inserted;
  std::size_t caret_offset{};
  for (std::size_t column = row.cells.size(); column <= target_column; ++column) {
    if (has_trailing_pipe) {
      inserted.push_back(L'|');
      if (column == target_column) {
        inserted.push_back(L' ');
        inserted.append(inserted_text);
        caret_offset = inserted.size();
        inserted.push_back(L' ');
      } else {
        inserted += L"  ";
      }
    } else {
      inserted += L" | ";
      if (column == target_column) {
        inserted.append(inserted_text);
        caret_offset = inserted.size();
      }
    }
  }

  std::wstring result(source);
  result.insert(insertion, inserted);
  return {std::move(result), insertion + caret_offset, true};
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
  const auto current_cell = ResolveTableCellIntent(source, caret);
  return current_cell ? MoveToAdjacentTableCell(source, *current_cell, backwards)
                      : NoTableEdit(source, caret);
}

TableEditResult MoveToAdjacentTableCell(std::wstring_view source,
                                        const TableCellIntent& current_cell,
                                        bool backwards) {
  const auto block = BlockAt(source, current_cell.row_begin);
  if (!block || block->row_ranges[block->current_row].begin != current_cell.row_begin ||
      block->current_row == 1 || current_cell.column >= block->alignments.size())
    return NoTableEdit(source, current_cell.source_position);

  const auto move_to = [&](std::size_t row_index, std::size_t column) {
    const auto target = IntentAtColumn(*block, row_index, column);
    return target ? TableEditResult{std::wstring(source), target->source_position,
                                    false, target}
                  : NoTableEdit(source, current_cell.source_position);
  };
  const std::size_t column_count = block->alignments.size();
  if (backwards) {
    if (current_cell.column > 0) return move_to(block->current_row, current_cell.column - 1);
    const auto previous_row = PreviousVisibleRow(block->current_row);
    return previous_row ? move_to(*previous_row, column_count - 1)
                        : NoTableEdit(source, current_cell.source_position);
  }
  if (current_cell.column + 1 < column_count)
    return move_to(block->current_row, current_cell.column + 1);
  if (const auto next_row = NextVisibleRow(*block, block->current_row))
    return move_to(*next_row, 0);

  const auto& last_row = block->row_ranges.back();
  const std::wstring new_row = RenderRow(column_count);
  const std::wstring line_break(PreferredLineBreak(source, last_row.begin));
  const std::size_t next = NextLineStart(source, last_row.end);
  const bool at_eof = next == last_row.end;
  const std::size_t insertion = at_eof ? last_row.end : next;
  const std::wstring inserted = at_eof ? line_break + new_row : new_row + line_break;
  std::wstring result(source);
  result.insert(insertion, inserted);
  const std::size_t new_row_begin = insertion + (at_eof ? line_break.size() : 0);
  const auto updated_block = BlockAt(result, new_row_begin);
  const auto target = updated_block
      ? IntentAtColumn(*updated_block, updated_block->current_row, 0) : std::nullopt;
  if (!target) return NoTableEdit(source, current_cell.source_position);
  return {std::move(result), target->source_position, true, target};
}

std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    std::wstring_view source, std::size_t caret, TableCaretDirection direction) {
  const auto current_cell = ResolveTableCellIntent(source, caret);
  return current_cell
      ? MoveTableCaretTargetAtBoundary(source, *current_cell, direction) : std::nullopt;
}

std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    std::wstring_view source, const TableCellIntent& current_cell,
    TableCaretDirection direction) {
  const auto block = BlockAt(source, current_cell.row_begin);
  if (!block || block->row_ranges[block->current_row].begin != current_cell.row_begin ||
      block->current_row == 1 || current_cell.column >= block->alignments.size())
    return std::nullopt;

  switch (direction) {
    case TableCaretDirection::Left:
      if (current_cell.column == 0) return std::nullopt;
      return IntentAtColumn(*block, block->current_row, current_cell.column - 1);
    case TableCaretDirection::Right:
      if (current_cell.column + 1 >= block->alignments.size()) return std::nullopt;
      return IntentAtColumn(*block, block->current_row, current_cell.column + 1);
    case TableCaretDirection::Up: {
      const auto previous_row = PreviousVisibleRow(block->current_row);
      return previous_row ? IntentAtColumn(*block, *previous_row, current_cell.column)
                          : std::nullopt;
    }
    case TableCaretDirection::Down: {
      const auto next_row = NextVisibleRow(*block, block->current_row);
      return next_row ? IntentAtColumn(*block, *next_row, current_cell.column)
                      : std::nullopt;
    }
  }
  return std::nullopt;
}

std::optional<TableCellIntent> MoveTableCaretTargetAtBoundary(
    const GfmTable& table, std::wstring_view source,
    const TableCellIntent& current_cell, TableCaretDirection direction) {
  const auto row = std::lower_bound(table.rows.begin(), table.rows.end(), current_cell.row_begin,
      [](const TableVisualRow& candidate, std::size_t begin) {
        return candidate.begin < begin;
      });
  if (row == table.rows.end() || row->begin != current_cell.row_begin) return std::nullopt;
  const auto row_index = static_cast<std::size_t>(row - table.rows.begin());
  if (row_index == 1 || current_cell.column >= table.alignments.size()) return std::nullopt;

  switch (direction) {
    case TableCaretDirection::Left:
      if (current_cell.column == 0) return std::nullopt;
      return GfmIntentAtColumn(table, source, row_index, current_cell.column - 1);
    case TableCaretDirection::Right:
      if (current_cell.column + 1 >= table.alignments.size()) return std::nullopt;
      return GfmIntentAtColumn(table, source, row_index, current_cell.column + 1);
    case TableCaretDirection::Up: {
      const auto previous = PreviousVisibleRow(row_index);
      return previous ? GfmIntentAtColumn(table, source, *previous, current_cell.column)
                      : std::nullopt;
    }
    case TableCaretDirection::Down: {
      const auto next = NextVisibleRow(table, row_index);
      return next ? GfmIntentAtColumn(table, source, *next, current_cell.column)
                  : std::nullopt;
    }
  }
  return std::nullopt;
}

std::optional<std::size_t> MoveTableCaretAtBoundary(std::wstring_view source,
                                                    std::size_t caret,
                                                    TableCaretDirection direction) {
  const auto block = BlockAt(source, caret);
  if (!block || block->current_row == 1 || block->alignments.empty()) return std::nullopt;
  const auto current_cell = ResolveTableCellIntent(source, caret);
  if (!current_cell || current_cell->column >=
          block->row_ranges[block->current_row].cells.size()) return std::nullopt;
  const auto& row = block->row_ranges[block->current_row];
  const std::size_t cell = current_cell->column;
  if (direction == TableCaretDirection::Left) {
    if (caret > row.cells[cell].begin || cell == 0) return std::nullopt;
  } else if (direction == TableCaretDirection::Right) {
    if (caret < row.cells[cell].end || cell + 1 >= block->alignments.size())
      return std::nullopt;
  }
  const auto target = MoveTableCaretTargetAtBoundary(source, *current_cell, direction);
  return target ? std::optional<std::size_t>(target->source_position) : std::nullopt;
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
