#include "editor/EditorAdapter.h"
#include "markdown/Markdown.h"

#include <algorithm>

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
    if (value[index] == L'\r' && index + 1 < value.size() && value[index + 1] == L'\n') {
      if (crlf) result.push_back(L'\r');
      result.push_back(L'\n');
      ++index;
    } else {
      result.push_back(value[index]);
    }
  }
  return result;
}

}  // namespace

std::size_t EditorSnapshot::SourceToView(std::size_t position) const noexcept {
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
    if (image_index < images.size() && index == images[image_index].begin &&
        images[image_index].target.find(L"://") != std::wstring::npos) ++image_index;
    if (image_index < images.size() && index == images[image_index].begin) {
      const auto& image = images[image_index++];
      const std::size_t view_position = result.view.size();
      result.collapsed.push_back({image.begin, image.end, view_position});
      result.view.push_back(0xFFFC);
      index = image.end;
      continue;
    }
    if (source[index] == L'\n' && (index == 0 || source[index - 1] != L'\r')) {
      result.inserted_crs.push_back(static_cast<std::uint32_t>(index));
      result.view.push_back(L'\r');
    }
    result.view.push_back(source[index]);
    ++index;
  }
  return result;
}

SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view) {
  SourceTransaction result;
  std::size_t prefix = 0;
  while (prefix < before.view.size() && prefix < new_view.size() &&
         before.view[prefix] == new_view[prefix]) ++prefix;
  if (prefix == before.view.size() && prefix == new_view.size()) {
    result.source = source;
    return result;
  }
  std::size_t old_suffix = before.view.size();
  std::size_t new_suffix = new_view.size();
  while (old_suffix > prefix && new_suffix > prefix &&
         before.view[old_suffix - 1] == new_view[new_suffix - 1]) {
    --old_suffix;
    --new_suffix;
  }
  result.begin = before.ViewToSource(prefix);
  result.old_end = before.ViewToSource(old_suffix);
  const bool crlf = PreferCrLf(source, result.begin);
  const std::wstring replacement = NormalizeReplacement(new_view.substr(prefix, new_suffix - prefix), crlf);
  result.source = source;
  result.source.replace(result.begin, result.old_end - result.begin, replacement);
  result.new_end = result.begin + replacement.size();
  result.changed = true;
  return result;
}

}  // namespace mdlite
