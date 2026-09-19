#include "editor/EditorAdapter.h"

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
  return source_to_view.empty() ? 0 : source_to_view[std::min(position, source_to_view.size() - 1)];
}

std::size_t EditorSnapshot::ViewToSource(std::size_t position) const noexcept {
  return view_to_source.empty() ? 0 : view_to_source[std::min(position, view_to_source.size() - 1)];
}

EditorSnapshot BuildEditorSnapshot(std::wstring source) {
  EditorSnapshot result;
  result.source = std::move(source);
  result.source_to_view.resize(result.source.size() + 1);
  result.view_to_source.reserve(result.source.size() + 1);
  for (std::size_t index = 0; index < result.source.size(); ++index) {
    result.source_to_view[index] = result.view.size();
    if (result.source[index] == L'\n' && (index == 0 || result.source[index - 1] != L'\r')) {
      result.view.push_back(L'\r');
      result.view_to_source.push_back(index);
    }
    result.view.push_back(result.source[index]);
    result.view_to_source.push_back(index);
  }
  result.source_to_view.back() = result.view.size();
  result.view_to_source.push_back(result.source.size());
  return result;
}

SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view new_view) {
  SourceTransaction result;
  std::size_t prefix = 0;
  while (prefix < before.view.size() && prefix < new_view.size() &&
         before.view[prefix] == new_view[prefix]) ++prefix;
  if (prefix == before.view.size() && prefix == new_view.size()) {
    result.source = before.source;
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
  const bool crlf = PreferCrLf(before.source, result.begin);
  const std::wstring replacement = NormalizeReplacement(new_view.substr(prefix, new_suffix - prefix), crlf);
  result.source = before.source;
  result.source.replace(result.begin, result.old_end - result.begin, replacement);
  result.new_end = result.begin + replacement.size();
  result.changed = true;
  return result;
}

}  // namespace mdlite
