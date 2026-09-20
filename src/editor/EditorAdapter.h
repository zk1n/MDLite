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

// A compact mapping for source ranges which occupy a different number of
// RichEdit character positions.  Typical entries are paragraph breaks
// (CRLF/LF -> one native CR) and derived image objects (Markdown -> U+FFFC).
// Keeping only discontinuities avoids a per-UTF-16-unit map for large notes.
struct NativeDiscontinuity {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t native_begin{};
  std::size_t native_end{};
};

struct EditorSnapshot {
  std::wstring view;
  std::size_t source_size{};
  std::vector<std::uint32_t> inserted_crs;
  std::vector<CollapsedRange> collapsed;
  std::vector<NativeDiscontinuity> native_discontinuities;
  bool native_coordinates{};

  [[nodiscard]] std::size_t SourceToView(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t ViewToSource(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t SourceToNative(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t NativeToSource(std::size_t position) const noexcept;
  [[nodiscard]] bool HasCollapsedSourceRange(std::size_t begin,
                                             std::size_t end) const noexcept;
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
EditorSnapshot BuildNativeEditorSnapshot(std::wstring_view source);
EditorSnapshot BuildNativeTextEditorSnapshot(std::wstring_view source);
SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view);

}  // namespace mdlite
