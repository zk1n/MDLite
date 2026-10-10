#pragma once

#include "editor/EditorAdapter.h"

#include <windows.h>

#include <string>
#include <string_view>
#include <vector>
#include <map>

namespace mdlite {

// RichEdit owns the single native caret. Presentation must remove its XOR
// pixels before suppressing redraw or moving its selection for formatting.
class RichEditCaretPaintGuard {
 public:
  explicit RichEditCaretPaintGuard(HWND editor) : editor_(editor),
      hidden_(GetFocus() == editor && HideCaret(editor) != FALSE) {}
  ~RichEditCaretPaintGuard() { if (hidden_ && IsWindow(editor_)) ShowCaret(editor_); }
  RichEditCaretPaintGuard(const RichEditCaretPaintGuard&) = delete;
  RichEditCaretPaintGuard& operator=(const RichEditCaretPaintGuard&) = delete;
 private:
  HWND editor_{};
  bool hidden_{};
};

// Apply cell typography without moving RichEdit's live selection/caret.
bool FormatRichEditTableCells(HWND editor, const EditorSnapshot& snapshot,
                             COLORREF header_background, COLORREF body_background,
                             bool hide_content = false);

struct RichEditTableStyle {
  LONG available_width_twips{7200};
  LONG minimum_cell_width_twips{1200};
  LONG cell_margin_twips{80};
  LONG font_size_half_points{20};
  std::wstring font_face{L"Segoe UI"};
  LONG maximum_cell_width_twips{6400};
  LONG row_padding_twips{45};
  bool layout_footprint{};
  std::map<std::size_t, std::vector<LONG>> row_heights_twips;
  COLORREF border_color{RGB(70, 82, 98)};
  COLORREF header_background{RGB(31, 40, 52)};
  COLORREF body_background{RGB(24, 31, 40)};
};

bool ReadRichEditNativeText(HWND editor, std::wstring& text);
bool RichEditTextLengthEquals(HWND editor, std::size_t expected_length);
// Verifies a flat projection while allowing RichEdit's collapsed-image slot
// to read back as either U+FFFC or a single space.
bool RichEditFlatProjectionEquals(HWND editor, const EditorSnapshot& expected);
// Compares canonical flat editor text (CR paragraph separators) without
// allocating a document-sized buffer.
bool RichEditTextEquals(HWND editor, std::wstring_view expected_flat_text);
// Compares a native [begin, end) range with canonical flat text.
bool RichEditTextRangeEquals(HWND editor, std::size_t begin, std::size_t end,
                             std::wstring_view expected_flat_text);
bool RebuildRichEditTables(HWND editor, EditorSnapshot& snapshot,
                           const RichEditTableStyle& style);
std::vector<LONG> MeasureRichEditTableColumns(HWND editor, const EditorSnapshot& snapshot,
                                             std::size_t table_index,
                                             const RichEditTableStyle& style);
bool RefreshRichEditTableCoordinates(EditorSnapshot& snapshot,
                                    std::wstring_view native_text);
bool FlattenRichEditTableText(const EditorSnapshot& snapshot,
                              std::wstring_view native_text,
                              std::wstring& flattened);

}  // namespace mdlite
