#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct EditorSnapshot {
  std::wstring source;
  std::wstring view;
  std::vector<std::size_t> source_to_view;
  std::vector<std::size_t> view_to_source;

  [[nodiscard]] std::size_t SourceToView(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t ViewToSource(std::size_t position) const noexcept;
};

struct SourceTransaction {
  std::wstring source;
  std::size_t begin{};
  std::size_t old_end{};
  std::size_t new_end{};
  bool changed{};
};

EditorSnapshot BuildEditorSnapshot(std::wstring source);
SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view new_view);

}  // namespace mdlite
