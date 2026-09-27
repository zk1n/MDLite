#pragma once

#include "table/Table.h"

#include <cstddef>
#include <cstdint>
#include <limits>
#include <optional>
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

struct EditorTableCellMapping {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t view_begin{};
  std::size_t view_end{};
  std::size_t native_begin{};
  std::size_t native_end{};
  std::vector<CollapsedRange> collapsed;
};

struct EditorTableGapMapping {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t view_begin{};
  std::size_t view_end{};
  std::size_t left_cell{};
  std::size_t right_cell{};
};

struct EditorTableVisualCellMapping {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t view_begin{};
  std::size_t view_end{};
  std::size_t native_begin{};
  std::size_t native_end{};
  bool virtual_cell{};
};

struct EditorTableVisualRowMapping {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t native_begin{};
  std::size_t native_end{};
  std::vector<EditorTableVisualCellMapping> cells;
};

struct EditorTableMapping {
  std::size_t source_begin{};
  std::size_t source_end{};
  std::size_t view_begin{};
  std::size_t view_end{};
  std::size_t native_begin{};
  std::size_t native_end{};
  bool native_coordinates_set{};
  std::vector<TableAlignment> alignments;
  std::vector<EditorTableCellMapping> cells;
  std::vector<EditorTableGapMapping> gaps;
  std::vector<EditorTableVisualRowMapping> visual_rows;
};

struct NativeCellRange {
  std::size_t begin{};
  std::size_t end{};
};

struct NativeTableCoordinates {
  std::size_t begin{};
  std::size_t end{};
  std::vector<NativeCellRange> cells;
  std::vector<NativeCellRange> rows;
  std::vector<std::vector<NativeCellRange>> visual_rows;
};

struct EditorSnapshot {
  std::wstring view;
  std::size_t source_size{};
  std::vector<std::uint32_t> inserted_crs;
  std::vector<CollapsedRange> collapsed;
  std::vector<EditorTableMapping> tables;
  std::vector<NativeDiscontinuity> view_discontinuities;
  std::vector<NativeDiscontinuity> native_discontinuities;
  bool native_coordinates{};

  [[nodiscard]] std::size_t SourceToView(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t ViewToSource(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t SourceToNative(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t NativeToSource(std::size_t position) const noexcept;
  [[nodiscard]] std::size_t NativeToView(std::size_t position) const noexcept;
  [[nodiscard]] bool HasCollapsedSourceRange(std::size_t begin,
                                             std::size_t end) const noexcept;
};

struct SourceTransaction {
  std::wstring source;
  std::size_t begin{};
  std::size_t old_end{};
  std::size_t new_end{};
  bool changed{};
  bool identity_ambiguous{};
};

struct SourceSelection {
  std::size_t anchor{};
  std::size_t active{};

  friend bool operator==(const SourceSelection&, const SourceSelection&) = default;
};

enum class EditorEditKind : std::uint8_t {
  Unknown,
  Insert,
  Backspace,
  Delete,
  Replace,
};

struct EditorEditHint {
  std::optional<SourceSelection> selection_before;
  EditorEditKind kind{EditorEditKind::Unknown};
};

EditorSnapshot BuildEditorSnapshot(std::wstring_view source);
EditorSnapshot BuildMarkdownEditorSnapshot(std::wstring_view source);
EditorSnapshot BuildNativeEditorSnapshot(std::wstring_view source);
EditorSnapshot BuildNativeTextEditorSnapshot(std::wstring_view source);
// RichEdit's native coordinate space uses one CR per paragraph. Some control
// builds/export flags can still return CRLF (or a lone LF), so callers reading
// the control normalize through this boundary before diffing or mapping.
std::wstring CanonicalizeNativeText(std::wstring_view native_text);
SourceTransaction ApplyEditorText(const EditorSnapshot& before, std::wstring_view source,
                                  std::wstring_view new_view,
                                  EditorEditHint edit_hint = {});
SourceSelection NativeSelectionToSource(const EditorSnapshot& snapshot,
                                        std::size_t native_start,
                                        std::size_t native_end,
                                        bool start_active) noexcept;
SourceSelection NativeSelectionToView(const EditorSnapshot& snapshot,
                                      std::size_t native_start,
                                      std::size_t native_end,
                                      bool start_active) noexcept;
SourceSelection SourceSelectionToNative(const EditorSnapshot& snapshot,
                                        SourceSelection selection) noexcept;
// Replace one projected table's TOM character coordinates after native table
// insertion. Cell ranges are in the same flattened visible-row order as table.cells.
// On success, native discontinuities are rebuilt for all tables.
bool ReplaceNativeTableCoordinates(EditorSnapshot& snapshot, std::size_t table_index,
                                   const NativeTableCoordinates& coordinates);

}  // namespace mdlite
