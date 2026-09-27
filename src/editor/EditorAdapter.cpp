#include "editor/EditorAdapter.h"
#include "markdown/Markdown.h"

#include <algorithm>
#include <cstdint>
#include <cwctype>
#include <utility>

namespace mdlite {
namespace {

bool PreferCrLf(std::wstring_view source, std::size_t position) {
  if (position > source.size()) position = source.size();
  const auto previous = source.rfind(L'\n', position == 0 ? 0 : position - 1);
  if (previous != std::wstring_view::npos) return previous > 0 && source[previous - 1] == L'\r';
  const auto next = source.find(L'\n', position);
  if (next != std::wstring_view::npos) return next > 0 && source[next - 1] == L'\r';
  return source.find(L"\r\n") != std::wstring_view::npos;
}

std::wstring NormalizeReplacement(std::wstring_view value, bool crlf) {
  std::wstring result;
  result.reserve(value.size());
  for (std::size_t index = 0; index < value.size(); ++index) {
    if (value[index] == L'\r') {
      if (crlf) result.push_back(L'\r');
      result.push_back(L'\n');
      if (index + 1 < value.size() && value[index + 1] == L'\n') ++index;
    } else {
      result.push_back(value[index]);
    }
  }
  return result;
}

bool IsLocalImage(const ImageReference& image) {
  return image.target.find(L"://") == std::wstring::npos;
}

bool ContainsOnlyWhitespace(std::wstring_view source, std::size_t begin, std::size_t end) {
  if (begin > end || end > source.size()) return false;
  return std::ranges::all_of(source.substr(begin, end - begin), [](wchar_t value) {
    // A line break is a stable textual anchor. Only same-line whitespace leaves
    // adjacent RichEdit object markers indistinguishable.
    return value != L'\r' && value != L'\n' && iswspace(value) != 0;
  });
}

bool HasAmbiguousImageNeighbour(const std::vector<ImageReference>& images,
                                std::wstring_view source, std::size_t index) {
  if (!IsLocalImage(images[index])) return false;
  for (std::size_t previous = index; previous > 0;) {
    --previous;
    if (!IsLocalImage(images[previous])) continue;
    if (ContainsOnlyWhitespace(source, images[previous].end, images[index].begin)) return true;
    break;
  }
  for (std::size_t next = index + 1; next < images.size(); ++next) {
    if (!IsLocalImage(images[next])) continue;
    if (ContainsOnlyWhitespace(source, images[index].end, images[next].begin)) return true;
    break;
  }
  return false;
}

std::optional<std::pair<std::size_t, std::size_t>> ResolveEditedViewRange(
    const EditorSnapshot& before, const EditorEditHint& hint) {
  if (!hint.selection_before) return std::nullopt;
  const auto selection = *hint.selection_before;
  const auto source_begin = std::min(selection.anchor, selection.active);
  const auto source_end = std::max(selection.anchor, selection.active);
  if (source_end > before.source_size) return std::nullopt;
  const auto view_begin = before.SourceToView(source_begin);
  const auto view_end = before.SourceToView(source_end);
  if (view_begin > view_end || view_end > before.view.size()) return std::nullopt;
  if (source_begin != source_end) return std::pair{view_begin, view_end};

  switch (hint.kind) {
    case EditorEditKind::Insert:
      return std::pair{view_begin, view_begin};
    case EditorEditKind::Backspace:
      if (view_begin == 0) return std::nullopt;
      return std::pair{view_begin - 1, view_begin};
    case EditorEditKind::Delete:
      if (view_begin >= before.view.size()) return std::nullopt;
      return std::pair{view_begin, view_begin + 1};
    case EditorEditKind::Unknown:
    case EditorEditKind::Replace:
      return std::nullopt;
  }
  return std::nullopt;
}

std::optional<std::wstring> RestoreCollapsedRanges(
    const EditorSnapshot& before, std::wstring_view source,
    std::size_t old_begin, std::size_t old_end, std::size_t new_begin,
    std::wstring_view replacement, std::wstring_view new_view,
    const EditorEditHint& hint, bool crlf) {
  std::vector<const CollapsedRange*> old_markers;
  for (const auto& marker : before.collapsed) {
    if (marker.view >= old_begin && marker.view < old_end) old_markers.push_back(&marker);
  }
  if (old_markers.empty()) {
    if (replacement.find(static_cast<wchar_t>(0xFFFC)) != std::wstring_view::npos)
      return std::nullopt;
    return NormalizeReplacement(replacement, crlf);
  }

  // RichEdit exposes every image as U+FFFC, which carries no source identity.
  // Resolve the old edit from its captured selection and kind, then locate
  // unchanged surrounding text. If either boundary is uncertain, do not guess.
  const auto edit_range = ResolveEditedViewRange(before, hint);
  if (!edit_range) return std::nullopt;
  const auto [edit_begin, edit_end] = *edit_range;
  if (edit_begin > edit_end || old_begin > edit_begin || edit_end > old_end ||
      new_begin > new_view.size() || replacement.size() > new_view.size() - new_begin)
    return std::nullopt;
  if (before.view.substr(0, edit_begin) != new_view.substr(0, edit_begin))
    return std::nullopt;
  const std::size_t suffix_length = before.view.size() - edit_end;
  if (suffix_length > new_view.size()) return std::nullopt;
  const std::size_t new_edit_end = new_view.size() - suffix_length;
  if (new_edit_end < edit_begin ||
      before.view.substr(edit_end) != new_view.substr(new_edit_end))
    return std::nullopt;

  std::vector<std::size_t> new_markers;
  for (std::size_t index = 0; index < replacement.size(); ++index) {
    if (replacement[index] == 0xFFFC) new_markers.push_back(index);
  }

  std::vector<const CollapsedRange*> mapped(new_markers.size());
  for (const auto* marker : old_markers) {
    if (marker->view >= edit_begin && marker->view < edit_end) continue;
    const std::size_t predicted_view = marker->view < edit_begin
        ? marker->view
        : new_edit_end + marker->view - edit_end;
    if (predicted_view < new_begin ||
        predicted_view >= new_begin + replacement.size() ||
        new_view[predicted_view] != 0xFFFC)
      return std::nullopt;
    const auto marker_position = predicted_view - new_begin;
    const auto found = std::ranges::lower_bound(new_markers, marker_position);
    if (found == new_markers.end() || *found != marker_position) return std::nullopt;
    const auto mapped_index = static_cast<std::size_t>(found - new_markers.begin());
    if (mapped[mapped_index]) return std::nullopt;
    mapped[mapped_index] = marker;
  }

  // A projected object with no old source range cannot safely become source
  // Markdown through this text-diff path.
  if (std::ranges::any_of(mapped, [](const CollapsedRange* marker) { return marker == nullptr; }))
    return std::nullopt;

  std::wstring restored;
  restored.reserve(replacement.size());
  std::size_t text_begin{};
  for (std::size_t marker_index = 0; marker_index < new_markers.size(); ++marker_index) {
    const std::size_t index = new_markers[marker_index];
    if (!mapped[marker_index]) continue;
    restored += NormalizeReplacement(replacement.substr(text_begin, index - text_begin), crlf);
    const auto& marker = *mapped[marker_index];
    restored.append(source.substr(marker.source_begin, marker.source_end - marker.source_begin));
    text_begin = index + 1;
  }
  restored += NormalizeReplacement(replacement.substr(text_begin), crlf);
  return restored;
}

std::ptrdiff_t OffsetAt(std::size_t coordinate, std::size_t source) noexcept {
  return static_cast<std::ptrdiff_t>(coordinate) - static_cast<std::ptrdiff_t>(source);
}

std::size_t ApplyOffset(std::size_t position, std::ptrdiff_t offset) noexcept {
  const auto mapped = static_cast<std::ptrdiff_t>(position) + offset;
  return mapped <= 0 ? 0 : static_cast<std::size_t>(mapped);
}

std::size_t TableCellCoordinate(const EditorTableCellMapping& cell,
                                std::size_t position, bool native) noexcept {
  const auto coordinate_begin = native ? cell.native_begin : cell.view_begin;
  const auto coordinate_end = native ? cell.native_end : cell.view_end;
  std::size_t collapsed_delta{};
  for (const auto& range : cell.collapsed) {
    if (position < range.source_begin) break;
    const auto collapsed_position = coordinate_begin + (range.view - cell.view_begin);
    if (position < range.source_end) return collapsed_position;
    collapsed_delta += range.source_end - range.source_begin - 1;
  }
  const auto source_offset = position - cell.source_begin;
  const auto offset = std::min(source_offset - std::min(source_offset, collapsed_delta),
                               coordinate_end - coordinate_begin);
  return coordinate_begin + offset;
}

std::size_t SourceToTableCoordinate(const EditorTableMapping& table,
                                    std::size_t position, bool native) noexcept {
  const auto coordinate_begin = native ? table.native_begin : table.view_begin;
  const auto coordinate_end = native ? table.native_end : table.view_end;
  for (const auto& row : table.visual_rows) {
    for (const auto& cell : row.cells) {
      if (cell.virtual_cell && position == cell.source_begin)
        return native ? cell.native_begin : cell.view_begin;
    }
  }
  for (const auto& cell : table.cells) {
    if (position >= cell.source_begin && position <= cell.source_end)
      return TableCellCoordinate(cell, position, native);
  }
  for (const auto& gap : table.gaps) {
    if (position >= gap.source_begin && position < gap.source_end) {
      const auto& left = table.cells[gap.left_cell];
      return native ? left.native_end : gap.view_begin;
    }
  }
  if (table.cells.empty()) return coordinate_begin;
  if (position < table.cells.front().source_begin)
    return native ? table.cells.front().native_begin : table.cells.front().view_begin;
  if (position > table.cells.back().source_end) return coordinate_end;
  return coordinate_begin;
}

std::size_t TableCoordinateToSource(const EditorTableMapping& table,
                                    std::size_t position, bool native) noexcept {
  if (!native && position == table.view_end) return table.source_end;
  for (const auto& row : table.visual_rows) {
    for (const auto& cell : row.cells) {
      const auto begin = native ? cell.native_begin : cell.view_begin;
      if (cell.virtual_cell && position == begin) return cell.source_begin;
    }
  }
  for (const auto& cell : table.cells) {
    const auto begin = native ? cell.native_begin : cell.view_begin;
    const auto end = native ? cell.native_end : cell.view_end;
    if (position >= begin && position <= end) {
      std::size_t collapsed_delta{};
      for (const auto& range : cell.collapsed) {
        const auto collapsed_position = begin + (range.view - cell.view_begin);
        if (position < collapsed_position) break;
        if (position < collapsed_position + 1) return range.source_begin;
        collapsed_delta += range.source_end - range.source_begin - 1;
      }
      const auto coordinate_offset = position - begin;
      return cell.source_begin + coordinate_offset +
             std::min(collapsed_delta, cell.source_end - cell.source_begin);
    }
  }
  for (const auto& gap : table.gaps) {
    const auto& left = table.cells[gap.left_cell];
    const auto& right = table.cells[gap.right_cell];
    const auto begin = native ? left.native_end : gap.view_begin;
    const auto end = native ? right.native_begin : gap.view_end;
    if (position >= begin && position < end) return gap.source_begin;
  }
  if (table.cells.empty()) return table.source_begin;
  const auto first_begin = native ? table.cells.front().native_begin
                                  : table.cells.front().view_begin;
  if (position < first_begin)
    return table.cells.front().source_begin;
  return table.cells.back().source_end;
}

std::size_t TableCoordinateToView(const EditorTableMapping& table,
                                  std::size_t position) noexcept {
  for (const auto& cell : table.cells) {
    if (position >= cell.native_begin && position <= cell.native_end) {
      const auto native_offset = position - cell.native_begin;
      return cell.view_begin + std::min(native_offset, cell.view_end - cell.view_begin);
    }
  }
  for (const auto& row : table.visual_rows) {
    for (const auto& cell : row.cells) {
      if (!cell.virtual_cell) continue;
      if (position == cell.native_begin) return cell.view_begin;
      if (position == cell.native_end) return cell.view_end;
    }
  }
  for (const auto& gap : table.gaps) {
    const auto& left = table.cells[gap.left_cell];
    const auto& right = table.cells[gap.right_cell];
    if (position >= left.native_end && position < right.native_begin) return gap.view_begin;
  }
  if (table.cells.empty()) return table.view_begin;
  if (position < table.cells.front().native_begin) return table.view_begin;
  return table.view_end;
}

std::size_t SourceToCoordinate(std::size_t position, std::size_t source_size,
                               const std::vector<NativeDiscontinuity>& discontinuities,
                               const std::vector<EditorTableMapping>& tables,
                               bool native) noexcept {
  position = std::min(position, source_size);
  for (const auto& table : tables) {
    if (position >= table.source_begin && position < table.source_end)
      return SourceToTableCoordinate(table, position, native);
  }
  std::ptrdiff_t offset{};
  for (const auto& range : discontinuities) {
    if (position < range.source_begin) break;
    if (position < range.source_end) return range.native_begin;
    offset = OffsetAt(range.native_end, range.source_end);
  }
  return ApplyOffset(position, offset);
}

std::size_t CoordinateToSource(std::size_t position, std::size_t source_size,
                               const std::vector<NativeDiscontinuity>& discontinuities,
                               const std::vector<EditorTableMapping>& tables,
                               bool native) noexcept {
  for (const auto& table : tables) {
    const auto begin = native ? table.native_begin : table.view_begin;
    const auto end = native ? table.native_end : table.view_end;
    if (position >= begin && (position < end || (!native && position == end)))
      return std::min(source_size, TableCoordinateToSource(table, position, native));
  }
  std::ptrdiff_t offset{};
  for (const auto& range : discontinuities) {
    if (position < range.native_begin) return ApplyOffset(position, offset);
    if (position < range.native_end) return std::min(source_size, range.source_begin);
    offset = OffsetAt(range.source_end, range.native_end);
  }
  return std::min(source_size, ApplyOffset(position, offset));
}

std::size_t CoordinateToView(std::size_t position,
                             const std::vector<EditorTableMapping>& tables) noexcept {
  std::ptrdiff_t view_offset{};
  for (const auto& table : tables) {
    if (position >= table.native_begin && position <= table.native_end)
      return TableCoordinateToView(table, position);
    if (position < table.native_begin) return ApplyOffset(position, view_offset);
    view_offset = OffsetAt(table.view_end, table.native_end);
  }
  return ApplyOffset(position, view_offset);
}

std::vector<NativeDiscontinuity> BuildProjectionDiscontinuities(
    std::wstring_view source, const std::vector<CollapsedRange>& collapsed,
    const std::vector<EditorTableMapping>& tables) {
  std::vector<NativeDiscontinuity> result;
  result.reserve(collapsed.size() + tables.size() + 8);
  std::size_t position{};
  std::size_t output{};
  std::size_t collapsed_index{};
  std::size_t table_index{};
  while (position < source.size()) {
    if (table_index < tables.size() && position == tables[table_index].source_begin) {
      const auto& table = tables[table_index++];
      result.push_back({table.source_begin, table.source_end, table.view_begin, table.view_end});
      output = table.view_end;
      position = table.source_end;
      // The table discontinuity subsumes any image ranges projected inside it.
      // Advance past them so a following image still maps at its own position.
      while (collapsed_index < collapsed.size() &&
             collapsed[collapsed_index].source_begin < table.source_end) {
        ++collapsed_index;
      }
      continue;
    }
    if (collapsed_index < collapsed.size() &&
        position == collapsed[collapsed_index].source_begin) {
      const auto& range = collapsed[collapsed_index++];
      result.push_back({range.source_begin, range.source_end, output, output + 1});
      ++output;
      position = range.source_end;
      continue;
    }
    if (source[position] == L'\r' && position + 1 < source.size() &&
        source[position + 1] == L'\n') {
      result.push_back({position, position + 2, output, output + 1});
      ++output;
      position += 2;
    } else {
      ++output;
      ++position;
    }
  }
  return result;
}

void RebuildNativeTableMappings(EditorSnapshot& snapshot) {
  std::ptrdiff_t view_offset{};
  for (auto& table : snapshot.tables) {
    if (table.native_coordinates_set) {
      view_offset = OffsetAt(table.native_end, table.view_end);
    } else {
      table.native_begin = ApplyOffset(table.view_begin, view_offset);
      table.native_end = ApplyOffset(table.view_end, view_offset);
      for (auto& cell : table.cells) {
        cell.native_begin = ApplyOffset(cell.view_begin, view_offset);
        cell.native_end = ApplyOffset(cell.view_end, view_offset);
      }
      for (auto& row : table.visual_rows) {
        row.native_begin = ApplyOffset(row.native_begin, view_offset);
        row.native_end = ApplyOffset(row.native_end, view_offset);
        for (auto& cell : row.cells) {
          cell.native_begin = ApplyOffset(cell.view_begin, view_offset);
          cell.native_end = ApplyOffset(cell.view_end, view_offset);
        }
      }
    }
  }
  snapshot.native_discontinuities.clear();
  snapshot.native_discontinuities.reserve(snapshot.view_discontinuities.size());
  view_offset = 0;
  for (const auto& range : snapshot.view_discontinuities) {
    const auto table = std::ranges::find_if(snapshot.tables, [&](const EditorTableMapping& candidate) {
      return candidate.source_begin == range.source_begin &&
             candidate.source_end == range.source_end;
    });
    if (table != snapshot.tables.end()) {
      snapshot.native_discontinuities.push_back(
          {range.source_begin, range.source_end, table->native_begin, table->native_end});
      if (table->native_coordinates_set)
        view_offset = OffsetAt(table->native_end, table->view_end);
    } else {
      snapshot.native_discontinuities.push_back(
          {range.source_begin, range.source_end,
           ApplyOffset(range.native_begin, view_offset),
           ApplyOffset(range.native_end, view_offset)});
    }
  }
}

}  // namespace

std::size_t EditorSnapshot::SourceToView(std::size_t position) const noexcept {
  if (!view_discontinuities.empty())
    return SourceToCoordinate(position, source_size, view_discontinuities, tables, false);
  if (native_coordinates) return SourceToNative(position);
  position = std::min(position, source_size);
  const auto collapsed_range = std::ranges::upper_bound(
      collapsed, position, {}, &CollapsedRange::source_begin);
  if (collapsed_range != collapsed.begin()) {
    const auto& range = *std::prev(collapsed_range);
    if (position < range.source_end) return range.view;
    const auto cr_begin = std::ranges::lower_bound(inserted_crs, range.source_end);
    const auto cr_end = std::ranges::lower_bound(inserted_crs, position);
    return range.view + 1 + position - range.source_end +
           static_cast<std::size_t>(cr_end - cr_begin);
  }
  const auto cr_end = std::ranges::lower_bound(inserted_crs, position);
  return position + static_cast<std::size_t>(cr_end - inserted_crs.begin());
}

std::size_t EditorSnapshot::ViewToSource(std::size_t position) const noexcept {
  if (!view_discontinuities.empty())
    return CoordinateToSource(position, source_size, view_discontinuities, tables, false);
  if (native_coordinates) return NativeToSource(position);
  const auto collapsed_range = std::ranges::lower_bound(collapsed, position, {}, &CollapsedRange::view);
  if (collapsed_range != collapsed.end() && collapsed_range->view == position)
    return collapsed_range->source_begin;
  std::size_t low{};
  std::size_t high = source_size;
  while (low < high) {
    const auto middle = low + (high - low + 1) / 2;
    if (SourceToView(middle) <= position) low = middle;
    else high = middle - 1;
  }
  return low;
}

std::size_t EditorSnapshot::SourceToNative(std::size_t position) const noexcept {
  return SourceToCoordinate(position, source_size, native_discontinuities, tables, true);
}

std::size_t EditorSnapshot::NativeToSource(std::size_t position) const noexcept {
  return CoordinateToSource(position, source_size, native_discontinuities, tables, true);
}

std::size_t EditorSnapshot::NativeToView(std::size_t position) const noexcept {
  return std::min(view.size(), CoordinateToView(position, tables));
}

bool EditorSnapshot::HasCollapsedSourceRange(std::size_t begin,
                                             std::size_t end) const noexcept {
  const auto range = std::ranges::lower_bound(collapsed, begin, {},
                                               &CollapsedRange::source_begin);
  return range != collapsed.end() && range->source_begin == begin &&
         range->source_end == end;
}

std::vector<NativeDiscontinuity> BuildNativeDiscontinuities(
    std::wstring_view source, const std::vector<CollapsedRange>& collapsed) {
  return BuildProjectionDiscontinuities(source, collapsed, {});
}

std::wstring BuildNativeView(std::wstring_view source,
                             const std::vector<CollapsedRange>& collapsed) {
  std::wstring result;
  result.reserve(source.size());
  std::size_t collapsed_index{};
  for (std::size_t source_position{}; source_position < source.size();) {
    if (collapsed_index < collapsed.size() &&
        source_position == collapsed[collapsed_index].source_begin) {
      result.push_back(0xFFFC);
      source_position = collapsed[collapsed_index++].source_end;
      continue;
    }
    if (source[source_position] == L'\r' && source_position + 1 < source.size() &&
        source[source_position + 1] == L'\n') {
      result.push_back(L'\r');
      source_position += 2;
    } else if (source[source_position] == L'\n') {
      result.push_back(L'\r');
      ++source_position;
    } else {
      result.push_back(source[source_position++]);
    }
  }
  return result;
}

std::wstring CanonicalizeNativeText(std::wstring_view native_text) {
  std::wstring result;
  result.reserve(native_text.size());
  for (std::size_t index{}; index < native_text.size(); ++index) {
    if (native_text[index] == L'\r') {
      result.push_back(L'\r');
      if (index + 1 < native_text.size() && native_text[index + 1] == L'\n') ++index;
    } else if (native_text[index] == L'\n') {
      // RichEdit's native paragraph boundary is CR. Treat a lone LF from an
      // alternate text/export path as the same boundary.
      result.push_back(L'\r');
    } else {
      result.push_back(native_text[index]);
    }
  }
  return result;
}

namespace {

void AppendNativeSourceRange(std::wstring_view source, std::size_t begin, std::size_t end,
                            const std::vector<ImageReference>& images,
                            std::size_t& image_index, EditorSnapshot& result) {
  for (std::size_t position = begin; position < end;) {
    while (image_index < images.size() && images[image_index].begin < position) ++image_index;
    if (image_index < images.size() && images[image_index].begin == position &&
        images[image_index].end <= end && IsLocalImage(images[image_index]) &&
        !HasAmbiguousImageNeighbour(images, source, image_index)) {
      const auto& image = images[image_index++];
      result.collapsed.push_back({image.begin, image.end, result.view.size()});
      result.view.push_back(0xFFFC);
      position = image.end;
    } else if (source[position] == L'\r' && position + 1 < end &&
               source[position + 1] == L'\n') {
      result.view.push_back(L'\r');
      position += 2;
    } else if (source[position] == L'\n') {
      result.view.push_back(L'\r');
      ++position;
    } else {
      result.view.push_back(source[position++]);
    }
  }
}

}  // namespace

EditorSnapshot BuildEditorSnapshot(std::wstring_view source) {
  EditorSnapshot result;
  result.source_size = source.size();
  result.view.reserve(source.size());
  for (std::size_t index = 0; index < source.size(); ++index) {
    if (source[index] == L'\n' && (index == 0 || source[index - 1] != L'\r')) {
      result.inserted_crs.push_back(static_cast<std::uint32_t>(index));
      result.view.push_back(L'\r');
    }
    result.view.push_back(source[index]);
  }
  result.native_discontinuities = BuildNativeDiscontinuities(source, result.collapsed);
  return result;
}

EditorSnapshot BuildMarkdownEditorSnapshot(std::wstring_view source) {
  const auto images = ParseMarkdownImages(source);
  if (images.empty()) return BuildEditorSnapshot(source);
  EditorSnapshot result;
  result.source_size = source.size();
  result.view.reserve(source.size());
  std::size_t image_index{};
  for (std::size_t index = 0; index < source.size();) {
    if (image_index < images.size() && index == images[image_index].begin) {
      const std::size_t current_image = image_index++;
      const auto& image = images[current_image];
      // RichEdit represents every inline image with the same U+FFFC marker.
      // Images separated only by whitespace therefore have no stable identity
      // after one object is deleted.  Keep those ambiguous groups as raw
      // Markdown rather than guessing which source range survived.
      if (IsLocalImage(image) &&
          !HasAmbiguousImageNeighbour(images, source, current_image)) {
        const std::size_t view_position = result.view.size();
        result.collapsed.push_back({image.begin, image.end, view_position});
        result.view.push_back(0xFFFC);
        index = image.end;
        continue;
      }
    }
    if (source[index] == L'\n' && (index == 0 || source[index - 1] != L'\r')) {
      result.inserted_crs.push_back(static_cast<std::uint32_t>(index));
      result.view.push_back(L'\r');
    }
    result.view.push_back(source[index]);
    ++index;
  }
  result.native_discontinuities = BuildNativeDiscontinuities(source, result.collapsed);
  return result;
}

EditorSnapshot BuildNativeEditorSnapshot(std::wstring_view source) {
  const auto parsed = ParseMarkdown(source);
  const auto& images = parsed.images;
  EditorSnapshot result;
  result.source_size = source.size();
  result.view.reserve(source.size());
  result.native_coordinates = true;
  std::size_t source_position{};
  std::size_t image_index{};
  for (const auto& parsed_table : parsed.tables) {
    if (parsed_table.begin < source_position || parsed_table.end > source.size() ||
        parsed_table.begin >= parsed_table.end || parsed_table.rows.empty()) continue;
    AppendNativeSourceRange(source, source_position, parsed_table.begin, images,
                            image_index, result);
    EditorTableMapping table;
    table.source_begin = parsed_table.begin;
    table.source_end = parsed_table.end;
    table.view_begin = result.view.size();
    table.native_begin = table.view_begin;
    table.alignments = parsed_table.alignments;
    std::vector<const TableVisualRow*> visible_rows;
    for (std::size_t row_index{}; row_index < parsed_table.rows.size(); ++row_index) {
      if (row_index != 1) visible_rows.push_back(&parsed_table.rows[row_index]);
    }
    std::size_t visual_column_count = parsed_table.alignments.size();
    for (const auto* row : visible_rows)
      visual_column_count = std::max(visual_column_count, row->cells.size());
    for (std::size_t row_index{}; row_index < visible_rows.size(); ++row_index) {
      const auto& row = *visible_rows[row_index];
      EditorTableVisualRowMapping visual_row{row.begin, row.end,
                                             result.view.size(), result.view.size(), {}};
      visual_row.cells.reserve(visual_column_count);
      if (row_index > 0 && !table.cells.empty() && !row.cells.empty()) {
        const auto left_index = table.cells.size() - 1;
        const std::size_t right_index = table.cells.size();
        EditorTableGapMapping gap{table.cells[left_index].source_end, row.cells.front().begin,
                                  result.view.size(), result.view.size() + 1,
                                  left_index, right_index};
        result.view.push_back(L'\r');
        table.gaps.push_back(gap);
      }
      for (std::size_t cell_index{}; cell_index < row.cells.size(); ++cell_index) {
        const auto& source_cell = row.cells[cell_index];
        if (source_cell.begin > source_cell.end || source_cell.end > source.size()) continue;
        if (cell_index > 0 && !table.cells.empty()) {
          const auto left_index = table.cells.size() - 1;
          EditorTableGapMapping gap{table.cells[left_index].source_end, source_cell.begin,
                                    result.view.size(), result.view.size() + 1,
                                    left_index, table.cells.size()};
          result.view.push_back(L'\t');
          table.gaps.push_back(gap);
        }
        const std::size_t view_begin = result.view.size();
        const auto collapsed_begin = result.collapsed.size();
        AppendNativeSourceRange(source, source_cell.begin, source_cell.end, images,
                                image_index, result);
        const std::size_t view_end = result.view.size();
        EditorTableCellMapping mapped_cell{source_cell.begin, source_cell.end,
                                           view_begin, view_end, view_begin, view_end};
        for (std::size_t collapsed = collapsed_begin; collapsed < result.collapsed.size(); ++collapsed)
          mapped_cell.collapsed.push_back(result.collapsed[collapsed]);
        visual_row.cells.push_back({source_cell.begin, source_cell.end,
                                    view_begin, view_end, view_begin, view_end, false});
        table.cells.push_back(std::move(mapped_cell));
      }
      const std::size_t virtual_anchor = row.cells.empty() ? row.begin : row.cells.back().end;
      while (visual_row.cells.size() < visual_column_count) {
        const std::size_t position = result.view.size();
        // Preserve a caret slot for each missing column; RTF projection omits this tab.
        result.view.push_back(L'\t');
        visual_row.cells.push_back({virtual_anchor, virtual_anchor,
                                    position, position + 1, position, position + 1, true});
      }
      visual_row.native_end = result.view.size();
      table.visual_rows.push_back(std::move(visual_row));
    }
    if (table.cells.empty()) continue;
    table.view_end = result.view.size();
    table.native_end = table.view_end;
    table.source_end = parsed_table.end;
    result.tables.push_back(std::move(table));
    source_position = parsed_table.end;
    while (image_index < images.size() && images[image_index].begin < source_position)
      ++image_index;
  }
  AppendNativeSourceRange(source, source_position, source.size(), images, image_index, result);
  result.view_discontinuities = BuildProjectionDiscontinuities(source, result.collapsed, result.tables);
  result.native_discontinuities = result.view_discontinuities;
  return result;
}

EditorSnapshot BuildNativeTextEditorSnapshot(std::wstring_view source) {
  auto result = BuildEditorSnapshot(source);
  result.native_coordinates = true;
  result.view = BuildNativeView(source, result.collapsed);
  result.inserted_crs.clear();
  result.view_discontinuities = result.native_discontinuities;
  return result;
}

SourceSelection NativeSelectionToSource(const EditorSnapshot& snapshot,
                                        std::size_t native_start,
                                        std::size_t native_end,
                                        bool start_active) noexcept {
  const auto first = std::min(native_start, native_end);
  const auto last = std::max(native_start, native_end);
  const auto source_first = snapshot.NativeToSource(first);
  const auto source_last = snapshot.NativeToSource(last);
  return start_active ? SourceSelection{source_last, source_first}
                      : SourceSelection{source_first, source_last};
}

SourceSelection NativeSelectionToView(const EditorSnapshot& snapshot,
                                      std::size_t native_start,
                                      std::size_t native_end,
                                      bool start_active) noexcept {
  const auto first = std::min(native_start, native_end);
  const auto last = std::max(native_start, native_end);
  const auto view_first = snapshot.NativeToView(first);
  const auto view_last = snapshot.NativeToView(last);
  return start_active ? SourceSelection{view_last, view_first}
                      : SourceSelection{view_first, view_last};
}

SourceSelection SourceSelectionToNative(const EditorSnapshot& snapshot,
                                        SourceSelection selection) noexcept {
  return {snapshot.SourceToNative(selection.anchor),
          snapshot.SourceToNative(selection.active)};
}

bool ReplaceNativeTableCoordinates(EditorSnapshot& snapshot, std::size_t table_index,
                                   const NativeTableCoordinates& coordinates) {
  if (table_index >= snapshot.tables.size() || coordinates.begin > coordinates.end) return false;
  auto& table = snapshot.tables[table_index];
  if (coordinates.cells.size() != table.cells.size()) return false;
  std::size_t previous = coordinates.begin;
  for (const auto& cell : coordinates.cells) {
    if (cell.begin < previous || cell.begin > cell.end || cell.end > coordinates.end) return false;
    previous = cell.end;
  }
  if (!coordinates.visual_rows.empty()) {
    if (coordinates.visual_rows.size() != table.visual_rows.size()) return false;
    if (coordinates.rows.size() != table.visual_rows.size()) return false;
    previous = coordinates.begin;
    for (const auto& row : coordinates.rows) {
      if (row.begin < previous || row.begin > row.end || row.end > coordinates.end) return false;
      previous = row.end;
    }
    previous = coordinates.begin;
    for (std::size_t row_index{}; row_index < coordinates.visual_rows.size(); ++row_index) {
      if (coordinates.visual_rows[row_index].size() != table.visual_rows[row_index].cells.size())
        return false;
      for (const auto& cell : coordinates.visual_rows[row_index]) {
        if (cell.begin < previous || cell.begin > cell.end || cell.end > coordinates.end) return false;
        previous = cell.end;
      }
    }
  }
  table.native_begin = coordinates.begin;
  table.native_end = coordinates.end;
  table.native_coordinates_set = true;
  for (std::size_t index{}; index < table.cells.size(); ++index) {
    table.cells[index].native_begin = coordinates.cells[index].begin;
    table.cells[index].native_end = coordinates.cells[index].end;
  }
  std::size_t source_cell_index{};
  for (std::size_t row_index{}; row_index < table.visual_rows.size(); ++row_index) {
    if (!coordinates.rows.empty()) {
      table.visual_rows[row_index].native_begin = coordinates.rows[row_index].begin;
      table.visual_rows[row_index].native_end = coordinates.rows[row_index].end;
    }
    for (std::size_t column{}; column < table.visual_rows[row_index].cells.size(); ++column) {
      auto& visual_cell = table.visual_rows[row_index].cells[column];
      if (!coordinates.visual_rows.empty()) {
        const auto& native_cell = coordinates.visual_rows[row_index][column];
        visual_cell.native_begin = native_cell.begin;
        visual_cell.native_end = native_cell.end;
      } else if (!visual_cell.virtual_cell && source_cell_index < coordinates.cells.size()) {
        visual_cell.native_begin = coordinates.cells[source_cell_index].begin;
        visual_cell.native_end = coordinates.cells[source_cell_index].end;
      }
      if (!visual_cell.virtual_cell) ++source_cell_index;
    }
  }
  if (source_cell_index != table.cells.size()) return false;
  RebuildNativeTableMappings(snapshot);
  return true;
}

SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view, EditorEditHint edit_hint) {
  std::wstring canonical_native;
  if (before.native_coordinates) {
    canonical_native = CanonicalizeNativeText(new_view);
    new_view = canonical_native;
  }
  SourceTransaction result;
  std::size_t prefix = 0;
  while (prefix < before.view.size() && prefix < new_view.size() &&
         before.view[prefix] == new_view[prefix]) {
    if (before.view[prefix] == 0xFFFC &&
        (prefix + 1 == before.view.size() || prefix + 1 == new_view.size() ||
         before.view[prefix + 1] != new_view[prefix + 1])) break;
    ++prefix;
  }
  if (prefix == before.view.size() && prefix == new_view.size()) {
    result.source = source;
    return result;
  }
  std::size_t old_suffix = before.view.size();
  std::size_t new_suffix = new_view.size();
  while (old_suffix > prefix && new_suffix > prefix &&
         before.view[old_suffix - 1] == new_view[new_suffix - 1]) {
    if (before.view[old_suffix - 1] == 0xFFFC &&
        (old_suffix <= prefix + 1 || new_suffix <= prefix + 1 ||
         before.view[old_suffix - 2] != new_view[new_suffix - 2])) break;
    --old_suffix;
    --new_suffix;
  }
  result.begin = before.ViewToSource(prefix);
  result.old_end = before.ViewToSource(old_suffix);
  if (old_suffix != prefix) {
    for (const auto& table : before.tables) {
      if (table.cells.empty() || old_suffix != table.view_end ||
          prefix < table.cells.back().view_begin ||
          prefix > table.cells.back().view_end) continue;
      // The closing GFM pipe has no projected character. When the final cell
      // edit reaches the end of its visible text, stop before that delimiter.
      result.old_end = table.cells.back().source_end;
      break;
    }
  }
  const bool crlf = PreferCrLf(source, result.begin);
  const auto replacement = RestoreCollapsedRanges(
      before, source, prefix, old_suffix, prefix,
      new_view.substr(prefix, new_suffix - prefix), new_view, edit_hint, crlf);
  if (!replacement) {
    result.source = source;
    result.identity_ambiguous = true;
    return result;
  }
  result.source = source;
  result.source.replace(result.begin, result.old_end - result.begin, *replacement);
  result.new_end = result.begin + replacement->size();
  result.changed = true;
  return result;
}

}  // namespace mdlite
