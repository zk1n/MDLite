#pragma once

#include "editor/EditorAdapter.h"

#include <windows.h>

#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct RichEditTableStyle {
  LONG available_width_twips{7200};
  LONG minimum_cell_width_twips{1200};
  LONG cell_margin_twips{80};
  LONG font_size_half_points{20};
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
bool RefreshRichEditTableCoordinates(EditorSnapshot& snapshot,
                                    std::wstring_view native_text);
bool FlattenRichEditTableText(const EditorSnapshot& snapshot,
                              std::wstring_view native_text,
                              std::wstring& flattened);

}  // namespace mdlite
