#include "editor/EditorAdapter.h"
#include "markdown/Markdown.h"

#include <algorithm>
#include <cstdint>
#include <cwctype>

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

std::wstring RestoreCollapsedRanges(const EditorSnapshot& before, std::wstring_view source,
                                    std::size_t old_begin, std::size_t old_end,
                                    std::wstring_view replacement, bool crlf) {
  std::vector<const CollapsedRange*> old_markers;
  for (const auto& marker : before.collapsed) {
    if (marker.view >= old_begin && marker.view < old_end) old_markers.push_back(&marker);
  }
  if (old_markers.empty()) return NormalizeReplacement(replacement, crlf);

  std::vector<std::size_t> new_markers;
  for (std::size_t index = 0; index < replacement.size(); ++index) {
    if (replacement[index] == 0xFFFC) new_markers.push_back(index);
  }

  // RichEdit exposes every derived image as the same U+FFFC character.  Match
  // surviving objects monotonically by their adjacent text, not just ordinal:
  // deleting the first of two objects must not turn the second image into the
  // first image's Markdown.  The dynamic program is bounded by image count;
  // unusually object-dense edits fall back to stable ordinal pairing.
  std::vector<const CollapsedRange*> mapped(new_markers.size());
  constexpr std::size_t maximum_alignment_cells = 1'000'000;
  if (!new_markers.empty() && old_markers.size() <= maximum_alignment_cells / new_markers.size()) {
    const std::size_t columns = new_markers.size() + 1;
    std::vector<std::int64_t> scores((old_markers.size() + 1) * columns);
    std::vector<unsigned char> actions(scores.size());  // 1=skip old, 2=skip new, 3=match
    const auto context_score = [&](const CollapsedRange& old_marker, std::size_t new_marker) {
      std::size_t left{};
      std::size_t old_cursor = old_marker.view;
      std::size_t new_cursor = new_marker;
      while (left < 64 && old_cursor > old_begin && new_cursor > 0) {
        const wchar_t old_character = before.view[old_cursor - 1];
        const wchar_t new_character = replacement[new_cursor - 1];
        if (old_character == 0xFFFC || new_character == 0xFFFC ||
            towlower(old_character) != towlower(new_character)) break;
        ++left;
        --old_cursor;
        --new_cursor;
      }
      std::size_t right{};
      old_cursor = old_marker.view + 1;
      new_cursor = new_marker + 1;
      while (right < 64 && old_cursor < old_end && new_cursor < replacement.size()) {
        const wchar_t old_character = before.view[old_cursor];
        const wchar_t new_character = replacement[new_cursor];
        if (old_character == 0xFFFC || new_character == 0xFFFC ||
            towlower(old_character) != towlower(new_character)) break;
        ++right;
        ++old_cursor;
        ++new_cursor;
      }
      const std::size_t old_offset = old_marker.view - old_begin;
      const std::size_t distance = old_offset > new_marker ? old_offset - new_marker
                                                            : new_marker - old_offset;
      return static_cast<std::int64_t>(1'000'000 + 32 * (left + right) -
                                       std::min<std::size_t>(distance, 4096));
    };
    for (std::size_t old_index = 0; old_index <= old_markers.size(); ++old_index) {
      for (std::size_t new_index = 0; new_index <= new_markers.size(); ++new_index) {
        if (old_index == 0 && new_index == 0) continue;
        const std::size_t cell = old_index * columns + new_index;
        std::int64_t best = -1;
        if (old_index > 0) {
          best = scores[(old_index - 1) * columns + new_index];
          actions[cell] = 1;
        }
        if (new_index > 0 && scores[cell - 1] > best) {
          best = scores[cell - 1];
          actions[cell] = 2;
        }
        if (old_index > 0 && new_index > 0) {
          const auto matched = scores[(old_index - 1) * columns + new_index - 1] +
                               context_score(*old_markers[old_index - 1], new_markers[new_index - 1]);
          if (matched >= best) {
            best = matched;
            actions[cell] = 3;
          }
        }
        scores[cell] = best;
      }
    }
    std::size_t old_index = old_markers.size();
    std::size_t new_index = new_markers.size();
    while (old_index > 0 || new_index > 0) {
      const auto action = actions[old_index * columns + new_index];
      if (action == 3) {
        mapped[new_index - 1] = old_markers[old_index - 1];
        --old_index;
        --new_index;
      } else if (action == 1) {
        --old_index;
      } else {
        --new_index;
      }
    }
  } else {
    for (std::size_t index = 0; index < std::min(old_markers.size(), new_markers.size()); ++index)
      mapped[index] = old_markers[index];
  }

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

}  // namespace

std::size_t EditorSnapshot::SourceToView(std::size_t position) const noexcept {
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
  position = std::min(position, source_size);
  std::ptrdiff_t delta{};
  for (const auto& discontinuity : native_discontinuities) {
    if (position < discontinuity.source_begin) break;
    if (position < discontinuity.source_end) return discontinuity.native_begin;
    const auto source_width = discontinuity.source_end - discontinuity.source_begin;
    const auto native_width = discontinuity.native_end - discontinuity.native_begin;
    delta += static_cast<std::ptrdiff_t>(native_width) -
             static_cast<std::ptrdiff_t>(source_width);
  }
  const auto mapped = static_cast<std::ptrdiff_t>(position) + delta;
  return mapped <= 0 ? 0 : static_cast<std::size_t>(mapped);
}

std::size_t EditorSnapshot::NativeToSource(std::size_t position) const noexcept {
  std::ptrdiff_t delta{};
  for (const auto& discontinuity : native_discontinuities) {
    if (position < discontinuity.native_begin) {
      const auto mapped = static_cast<std::ptrdiff_t>(position) - delta;
      return mapped <= 0 ? 0 : std::min(source_size, static_cast<std::size_t>(mapped));
    }
    if (position < discontinuity.native_end) return discontinuity.source_begin;
    const auto source_width = discontinuity.source_end - discontinuity.source_begin;
    const auto native_width = discontinuity.native_end - discontinuity.native_begin;
    delta += static_cast<std::ptrdiff_t>(source_width) -
             static_cast<std::ptrdiff_t>(native_width);
  }
  const auto mapped = static_cast<std::ptrdiff_t>(position) + delta;
  return mapped <= 0 ? 0 : std::min(source_size, static_cast<std::size_t>(mapped));
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
  std::vector<NativeDiscontinuity> result;
  result.reserve(collapsed.size() + 8);
  std::size_t native_position{};
  std::size_t collapsed_index{};
  for (std::size_t source_position{}; source_position < source.size();) {
    if (collapsed_index < collapsed.size() &&
        source_position == collapsed[collapsed_index].source_begin) {
      const auto& range = collapsed[collapsed_index++];
      result.push_back({range.source_begin, range.source_end, native_position,
                        native_position + 1});
      ++native_position;
      source_position = range.source_end;
      continue;
    }
    if (source[source_position] == L'\r' && source_position + 1 < source.size() &&
        source[source_position + 1] == L'\n') {
      result.push_back({source_position, source_position + 2, native_position,
                        native_position + 1});
      ++native_position;
      source_position += 2;
      continue;
    }
    // A lone LF is one source UTF-16 unit and one native CR unit. It changes
    // the character value but not the coordinate, so retaining a record here
    // only bloats large-file maps without adding an offset discontinuity.
    if (source[source_position] == L'\n') {
      ++native_position;
      ++source_position;
      continue;
    }
    ++native_position;
    ++source_position;
  }
  return result;
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
  auto result = BuildMarkdownEditorSnapshot(source);
  result.native_coordinates = true;
  result.view = BuildNativeView(source, result.collapsed);
  result.inserted_crs.clear();
  for (auto& range : result.collapsed)
    range.view = result.SourceToNative(range.source_begin);
  return result;
}

EditorSnapshot BuildNativeTextEditorSnapshot(std::wstring_view source) {
  auto result = BuildEditorSnapshot(source);
  result.native_coordinates = true;
  result.view = BuildNativeView(source, result.collapsed);
  result.inserted_crs.clear();
  return result;
}

SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view) {
  std::wstring canonical_native;
  if (before.native_coordinates) {
    canonical_native = CanonicalizeNativeText(new_view);
    new_view = canonical_native;
  }
  SourceTransaction result;
  std::size_t prefix = 0;
  while (prefix < before.view.size() && prefix < new_view.size() &&
         before.view[prefix] == new_view[prefix] && before.view[prefix] != 0xFFFC) ++prefix;
  if (prefix == before.view.size() && prefix == new_view.size()) {
    result.source = source;
    return result;
  }
  std::size_t old_suffix = before.view.size();
  std::size_t new_suffix = new_view.size();
  while (old_suffix > prefix && new_suffix > prefix &&
         before.view[old_suffix - 1] == new_view[new_suffix - 1] &&
         before.view[old_suffix - 1] != 0xFFFC) {
    --old_suffix;
    --new_suffix;
  }
  result.begin = before.ViewToSource(prefix);
  result.old_end = before.ViewToSource(old_suffix);
  const bool crlf = PreferCrLf(source, result.begin);
  const std::wstring replacement = RestoreCollapsedRanges(
      before, source, prefix, old_suffix, new_view.substr(prefix, new_suffix - prefix), crlf);
  result.source = source;
  result.source.replace(result.begin, result.old_end - result.begin, replacement);
  result.new_end = result.begin + replacement.size();
  result.changed = true;
  return result;
}

}  // namespace mdlite
