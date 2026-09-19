#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct CollapsedRange {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t view{};
};

struct EditorSnapshot {
  std::wstring view;
  std::size_t source_size{};
  std::vector<std::uint32_t> inserted_crs;
  std::vector<CollapsedRange> collapsed;

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

EditorSnapshot BuildEditorSnapshot(std::wstring_view source);
EditorSnapshot BuildMarkdownEditorSnapshot(std::wstring_view source);
SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view);

}  // namespace mdlite
