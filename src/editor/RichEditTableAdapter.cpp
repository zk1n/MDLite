#include "editor/RichEditTableAdapter.h"
#include "markdown/Markdown.h"

#include <richedit.h>
#include <richole.h>
#include <tom.h>

#include <algorithm>
#include <array>
#include <climits>
#include <cstring>
#include <limits>
#include <string>

namespace mdlite {
namespace {

constexpr std::size_t kTextVerificationChunkChars = 4096;

bool ReadRichEditTextLength(HWND editor, std::size_t& length) {
  length = 0;
  if (!editor || !IsWindow(editor)) return false;

  // Omitting GTL_USECRLF keeps RichEdit paragraph separators as single CRs,
  // matching CanonicalizeNativeText's flat-view convention.
  GETTEXTLENGTHEX request{GTL_NUMCHARS | GTL_PRECISE, 1200};
  const LRESULT raw_length = SendMessageW(
      editor, EM_GETTEXTLENGTHEX, reinterpret_cast<WPARAM>(&request), 0);
  if (raw_length < 0 || !IsWindow(editor)) return false;
  const auto converted = static_cast<unsigned long long>(raw_length);
  if (converted > static_cast<unsigned long long>(LONG_MAX)) return false;
  length = static_cast<std::size_t>(converted);
  return true;
}

bool CompareRichEditTextRange(HWND editor, std::size_t begin, std::size_t end,
                              std::wstring_view expected_flat_text) {
  if (begin > end || end - begin != expected_flat_text.size() ||
      end > static_cast<std::size_t>(LONG_MAX)) return false;

  std::array<wchar_t, kTextVerificationChunkChars + 1> buffer{};
  std::size_t offset = begin;
  std::size_t expected_offset{};
  while (offset < end) {
    if (!IsWindow(editor)) return false;
    const std::size_t count = std::min(kTextVerificationChunkChars, end - offset);
    TEXTRANGEW request{};
    request.chrg.cpMin = static_cast<LONG>(offset);
    request.chrg.cpMax = static_cast<LONG>(offset + count);
    request.lpstrText = buffer.data();
    const LRESULT copied = SendMessageW(
        editor, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&request));
    if (copied < 0 || static_cast<std::size_t>(copied) != count || !IsWindow(editor))
      return false;
    if (!std::equal(buffer.begin(), buffer.begin() + count,
                    expected_flat_text.begin() + expected_offset)) return false;
    offset += count;
    expected_offset += count;
  }
  return IsWindow(editor);
}

bool CompareRichEditFlatProjection(HWND editor, const EditorSnapshot& expected) {
  if (expected.view.size() > static_cast<std::size_t>(LONG_MAX)) return false;
  std::size_t previous_marker{};
  bool have_previous_marker{};
  for (const auto& collapsed : expected.collapsed) {
    if (collapsed.view >= expected.view.size() ||
        expected.view[collapsed.view] != static_cast<wchar_t>(0xfffc) ||
        (have_previous_marker && collapsed.view <= previous_marker)) return false;
    previous_marker = collapsed.view;
    have_previous_marker = true;
  }

  std::array<wchar_t, kTextVerificationChunkChars + 1> buffer{};
  std::size_t offset{};
  std::size_t marker_index{};
  while (offset < expected.view.size()) {
    if (!IsWindow(editor)) return false;
    const std::size_t count = std::min(kTextVerificationChunkChars,
                                       expected.view.size() - offset);
    TEXTRANGEW request{};
    request.chrg.cpMin = static_cast<LONG>(offset);
    request.chrg.cpMax = static_cast<LONG>(offset + count);
    request.lpstrText = buffer.data();
    const LRESULT copied = SendMessageW(
        editor, EM_GETTEXTRANGE, 0, reinterpret_cast<LPARAM>(&request));
    if (copied < 0 || static_cast<std::size_t>(copied) != count || !IsWindow(editor))
      return false;

    for (std::size_t local{}; local < count; ++local) {
      const std::size_t position = offset + local;
      const bool collapsed_slot = marker_index < expected.collapsed.size() &&
          expected.collapsed[marker_index].view == position;
      if (collapsed_slot) {
        if (buffer[local] != static_cast<wchar_t>(0xfffc) && buffer[local] != L' ')
          return false;
        ++marker_index;
      } else if (buffer[local] != expected.view[position]) {
        return false;
      }
    }
    offset += count;
  }
  return marker_index == expected.collapsed.size() && IsWindow(editor);
}

struct RtfInput {
  const std::string* data{};
  std::size_t offset{};
};

DWORD CALLBACK StreamRtf(DWORD_PTR cookie, LPBYTE buffer, LONG requested, LONG* copied) {
  auto& input = *reinterpret_cast<RtfInput*>(cookie);
  const auto remaining = input.data->size() - input.offset;
  const auto count = std::min<std::size_t>(remaining,
      static_cast<std::size_t>(std::max<LONG>(requested, 0)));
  if (count != 0) std::memcpy(buffer, input.data->data() + input.offset, count);
  input.offset += count;
  *copied = static_cast<LONG>(count);
  return 0;
}

std::string EscapeRtf(std::wstring_view value) {
  std::string result;
  result.reserve(value.size());
  for (const wchar_t character : value) {
    if (character == L'\\' || character == L'{' || character == L'}') {
      result.push_back('\\');
      result.push_back(static_cast<char>(character));
    } else if (character >= 0x20 && character <= 0x7e) {
      result.push_back(static_cast<char>(character));
    } else {
      result += "\\u" + std::to_string(static_cast<short>(character)) + "?";
    }
  }
  return result;
}

void AppendRtfColor(std::string& target, COLORREF color) {
  target += "\\red" + std::to_string(GetRValue(color));
  target += "\\green" + std::to_string(GetGValue(color));
  target += "\\blue" + std::to_string(GetBValue(color)) + ";";
}

const char* AlignmentTag(TableAlignment alignment) {
  switch (alignment) {
    case TableAlignment::Center: return "\\qc";
    case TableAlignment::Right: return "\\qr";
    case TableAlignment::Unspecified:
    case TableAlignment::Left: return "\\ql";
  }
  return "\\ql";
}

bool BuildRtfTable(HWND editor, const EditorSnapshot& snapshot, std::size_t table_index,
                   const RichEditTableStyle& style, std::string& rtf) {
  if (table_index >= snapshot.tables.size()) return false;
  const auto& table = snapshot.tables[table_index];
  if (table.visual_rows.empty()) return false;
  std::size_t column_count{};
  for (const auto& row : table.visual_rows)
    column_count = std::max(column_count, row.cells.size());
  if (column_count == 0 || column_count > MAX_TAB_STOPS + 16) return false;

  auto widths = MeasureRichEditTableColumns(editor, snapshot, table_index, style);
  if (widths.size() != column_count) return false;
  LONG total_width{};
  for (const auto width : widths) total_width += width;
  if (style.layout_footprint && total_width > style.available_width_twips) {
    for (auto& width : widths) width = std::max<LONG>(1, MulDiv(width, style.available_width_twips, total_width));
  }
  std::vector<LONG> cell_edges(column_count);
  LONG cumulative{};
  for (std::size_t column{}; column < column_count; ++column) {
    const LONG increment = widths[column];
    if (increment <= 0 || cumulative > LONG_MAX - increment) return false;
    cumulative += increment;
    cell_edges[column] = cumulative;
  }

  rtf = "{\\rtf1\\ansi\\ansicpg1252\\deff0\\viewkind4\\uc1";
  rtf += "{\\fonttbl{\\f0\\fnil " + EscapeRtf(style.font_face) + ";}}{\\colortbl;";
  AppendRtfColor(rtf, style.border_color);
  AppendRtfColor(rtf, style.header_background);
  AppendRtfColor(rtf, style.body_background);
  rtf += "}";
  for (std::size_t row_index{}; row_index < table.visual_rows.size(); ++row_index) {
    const auto& row = table.visual_rows[row_index];
    if (row.cells.size() > column_count) return false;
    rtf += "\\trowd\\trgaph" + std::to_string(std::max<LONG>(0, style.cell_margin_twips));
    rtf += "\\trleft0";
    LONG height = style.font_size_half_points * 18;
    const auto measured_heights = style.row_heights_twips.find(table.source_begin);
    if (measured_heights != style.row_heights_twips.end() && row_index < measured_heights->second.size())
      height = measured_heights->second[row_index];
    rtf += "\\trrh" + std::to_string(style.layout_footprint ? -height : height);
    for (std::size_t column{}; column < column_count; ++column) {
      rtf += "\\clvertalt\\clbrdrl\\brdrs\\brdrw6\\brdrcf1"
             "\\clbrdrr\\brdrs\\brdrw6\\brdrcf1"
             "\\clbrdrt\\brdrs\\brdrw6\\brdrcf1"
             "\\clbrdrb\\brdrs\\brdrw6\\brdrcf1";
      rtf += row_index == 0 ? "\\clcbpat2" : "\\clcbpat3";
      rtf += "\\clpadl" + std::to_string(style.cell_margin_twips) + "\\clpadfl3";
      rtf += "\\clpadr" + std::to_string(style.cell_margin_twips) + "\\clpadfr3";
      rtf += "\\clpadt" + std::to_string(style.row_padding_twips) + "\\clpadft3";
      rtf += "\\clpadb" + std::to_string(style.row_padding_twips) + "\\clpadfb3";
      rtf += "\\cellx" + std::to_string(cell_edges[column]);
    }
    for (std::size_t column{}; column < column_count; ++column) {
      rtf += "\\pard\\intbl\\f0\\fs" +
             std::to_string(std::max<LONG>(2, style.font_size_half_points));
      rtf += AlignmentTag(column < table.alignments.size()
                              ? table.alignments[column] : TableAlignment::Left);
      rtf += "\\sl" + std::to_string(style.font_size_half_points * 16) + "\\slmult0";
      rtf.push_back(' ');
      if (style.layout_footprint) rtf += "\\v ";
      const auto& cell = row.cells[column];
      if (cell.view_begin > cell.view_end || cell.view_end > snapshot.view.size()) return false;
      if (!cell.virtual_cell) {
        rtf += EscapeRtf(std::wstring_view(snapshot.view).substr(
            cell.view_begin, cell.view_end - cell.view_begin));
      }
      rtf += "\\cell";
    }
    rtf += "\\row ";
  }
  rtf.push_back('}');
  return true;
}

}  // namespace

std::vector<LONG> MeasureRichEditTableColumns(HWND editor, const EditorSnapshot& snapshot,
                                             std::size_t table_index,
                                             const RichEditTableStyle& style) {
  if (table_index >= snapshot.tables.size()) return {};
  const auto& table = snapshot.tables[table_index];
  std::size_t columns{};
  for (const auto& row : table.visual_rows) columns = std::max(columns, row.cells.size());
  std::vector<LONG> widths(columns, std::max<LONG>(1, style.minimum_cell_width_twips));
  HDC dc = GetDC(editor);
  if (!dc) return {};
  const int dpi = std::max(1, GetDeviceCaps(dc, LOGPIXELSX));
  std::array<HFONT, 8> fonts{};
  for (std::size_t i{}; i < fonts.size(); ++i) {
    fonts[i] = CreateFontW(-MulDiv(style.font_size_half_points, dpi, 144), 0, 0, 0,
        (i & 1) ? FW_BOLD : FW_NORMAL, (i & 2) != 0, FALSE, FALSE, DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, DEFAULT_PITCH,
        (i & 4) ? L"Cascadia Mono" : style.font_face.c_str());
  }
  const auto original_font = GetCurrentObject(dc, OBJ_FONT);
  for (const auto& row : table.visual_rows) {
    for (std::size_t column{}; column < row.cells.size(); ++column) {
      const auto& cell = row.cells[column];
      if (cell.virtual_cell || cell.view_end > snapshot.view.size()) continue;
      const auto text = std::wstring_view(snapshot.view).substr(cell.view_begin,
                                                               cell.view_end - cell.view_begin);
      const auto inline_parse = ParseMarkdown(text);
      LONG measured{};
      for (std::size_t begin{}; begin < text.size();) {
        unsigned font_style{};
        bool hidden{};
        std::size_t end = text.size();
        for (const auto& span : inline_parse.spans) {
          if (span.begin > begin) end = std::min(end, span.begin);
          if (begin >= span.begin && begin < span.end) {
            end = std::min(end, span.end);
            hidden |= span.kind == SpanKind::EmphasisMarker;
            if (span.kind == SpanKind::Strong) font_style |= 1;
            if (span.kind == SpanKind::Emphasis) font_style |= 2;
            if (span.kind == SpanKind::Code) font_style |= 4;
          }
        }
        end = std::max(begin + 1, end);
        if (!hidden) {
          if (fonts[font_style]) SelectObject(dc, fonts[font_style]);
          SIZE extent{};
          if (GetTextExtentPoint32W(dc, text.data() + begin, static_cast<int>(end - begin), &extent))
            measured += extent.cx;
        }
        begin = end;
      }
      const LONG text_twips = MulDiv(measured, 1440, dpi);
      // Reference textWidth + 16 CSS px; px is a DPI-independent logical unit.
      const LONG candidate = std::clamp(text_twips + 240, style.minimum_cell_width_twips,
          std::max(style.minimum_cell_width_twips, style.maximum_cell_width_twips));
      widths[column] = std::max(widths[column], candidate);
    }
  }
  SelectObject(dc, original_font);
  for (const auto font : fonts) if (font) DeleteObject(font);
  ReleaseDC(editor, dc);
  return widths;
}

bool FormatRichEditTableCells(HWND editor, const EditorSnapshot& snapshot,
                             COLORREF header_background, COLORREF body_background,
                             bool hide_content) {
  if (snapshot.tables.empty()) return true;
  IRichEditOle* rich_edit{};
  if (!SendMessageW(editor, EM_GETOLEINTERFACE, 0,
                    reinterpret_cast<LPARAM>(&rich_edit)) || !rich_edit) return false;
  ITextDocument* document{};
  const HRESULT queried = rich_edit->QueryInterface(__uuidof(ITextDocument),
                                                    reinterpret_cast<void**>(&document));
  rich_edit->Release();
  if (FAILED(queried) || !document) return false;
  bool formatted = true;
  for (const auto& table : snapshot.tables) {
    for (std::size_t row_index{}; formatted && row_index < table.visual_rows.size(); ++row_index) {
      for (const auto& cell : table.visual_rows[row_index].cells) {
        if (cell.virtual_cell || cell.native_begin >= cell.native_end) continue;
        if (cell.native_end > static_cast<std::size_t>(LONG_MAX)) { formatted = false; break; }
        ITextRange* range{};
        ITextFont* font{};
        formatted = SUCCEEDED(document->Range(static_cast<LONG>(cell.native_begin),
                                              static_cast<LONG>(cell.native_end), &range)) && range &&
                    SUCCEEDED(range->GetFont(&font)) && font &&
                    (!hide_content || SUCCEEDED(font->SetHidden(tomTrue))) &&
                    SUCCEEDED(font->SetBackColor(static_cast<LONG>(
                        row_index == 0 ? header_background : body_background)));
        if (font) font->Release();
        if (range) range->Release();
        if (!formatted) break;
      }
    }
    if (!formatted) break;
  }
  document->Release();
  return formatted;
}

bool ReadRichEditNativeText(HWND editor, std::wstring& text) {
  text.clear();
  GETTEXTLENGTHEX length_request{GTL_NUMCHARS | GTL_PRECISE, 1200};
  const LRESULT raw_length = SendMessageW(editor, EM_GETTEXTLENGTHEX,
                                          reinterpret_cast<WPARAM>(&length_request), 0);
  if (raw_length < 0) return false;
  const auto length = static_cast<std::size_t>(raw_length);
  text.assign(length + 1, L'\0');
  GETTEXTEX text_request{static_cast<DWORD>(text.size() * sizeof(wchar_t)),
                         GT_RAWTEXT, 1200, nullptr, nullptr};
  const LRESULT raw_copied = SendMessageW(editor, EM_GETTEXTEX,
                                          reinterpret_cast<WPARAM>(&text_request),
                                          reinterpret_cast<LPARAM>(text.data()));
  if (raw_copied < 0 || static_cast<std::size_t>(raw_copied) != length) {
    text.clear();
    return false;
  }
  text.resize(length);
  text = CanonicalizeNativeText(text);
  return true;
}

bool RichEditTextLengthEquals(HWND editor, std::size_t expected_length) {
  std::size_t length{};
  return ReadRichEditTextLength(editor, length) && length == expected_length;
}

bool RichEditFlatProjectionEquals(HWND editor, const EditorSnapshot& expected) {
  return RichEditTextLengthEquals(editor, expected.view.size()) &&
         CompareRichEditFlatProjection(editor, expected);
}

bool RichEditTextEquals(HWND editor, std::wstring_view expected_flat_text) {
  return RichEditTextLengthEquals(editor, expected_flat_text.size()) &&
         CompareRichEditTextRange(editor, 0, expected_flat_text.size(), expected_flat_text);
}

bool RichEditTextRangeEquals(HWND editor, std::size_t begin, std::size_t end,
                             std::wstring_view expected_flat_text) {
  std::size_t length{};
  return ReadRichEditTextLength(editor, length) && end <= length &&
         CompareRichEditTextRange(editor, begin, end, expected_flat_text);
}

bool RebuildRichEditTables(HWND editor, EditorSnapshot& snapshot,
                           const RichEditTableStyle& style) {
  if (snapshot.tables.empty()) return true;
  for (std::size_t reverse = snapshot.tables.size(); reverse > 0; --reverse) {
    const auto& table = snapshot.tables[reverse - 1];
    if (table.native_coordinates_set || table.view_begin > table.view_end ||
        table.view_end > snapshot.view.size() || table.view_end > LONG_MAX) return false;
    std::string rtf;
    if (!BuildRtfTable(editor, snapshot, reverse - 1, style, rtf)) return false;
    CHARRANGE range{static_cast<LONG>(table.view_begin), static_cast<LONG>(table.view_end)};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&range));
    RtfInput input{&rtf, 0};
    EDITSTREAM stream{};
    stream.dwCookie = reinterpret_cast<DWORD_PTR>(&input);
    stream.pfnCallback = StreamRtf;
    SendMessageW(editor, EM_STREAMIN, SF_RTF | SFF_SELECTION,
                 reinterpret_cast<LPARAM>(&stream));
    if (stream.dwError != 0 || input.offset != rtf.size()) return false;
  }
  std::wstring raw;
  return ReadRichEditNativeText(editor, raw) &&
         RefreshRichEditTableCoordinates(snapshot, raw);
}

bool RefreshRichEditTableCoordinates(EditorSnapshot& snapshot,
                                    std::wstring_view native_text) {
  const std::wstring raw = CanonicalizeNativeText(native_text);
  std::size_t expected_rows{};
  std::size_t expected_cells{};
  for (const auto& table : snapshot.tables) {
    if (table.visual_rows.size() > std::numeric_limits<std::size_t>::max() - expected_rows)
      return false;
    expected_rows += table.visual_rows.size();
    for (const auto& row : table.visual_rows) {
      if (row.cells.size() > std::numeric_limits<std::size_t>::max() - expected_cells)
        return false;
      expected_cells += row.cells.size();
    }
  }
  std::size_t row_start_markers{};
  std::size_t row_end_markers{};
  std::size_t cell_separators{};
  for (const wchar_t character : raw) {
    if (character == static_cast<wchar_t>(0xfff9)) ++row_start_markers;
    else if (character == static_cast<wchar_t>(0xfffb)) ++row_end_markers;
    else if (character == static_cast<wchar_t>(0x0007)) ++cell_separators;
  }
  if (row_start_markers != expected_rows || row_end_markers != expected_rows ||
      cell_separators != expected_cells) return false;

  std::size_t search_position{};
  std::vector<NativeTableCoordinates> all_coordinates;
  all_coordinates.reserve(snapshot.tables.size());
  for (std::size_t table_index{}; table_index < snapshot.tables.size(); ++table_index) {
    const auto& table = snapshot.tables[table_index];
    if (table.visual_rows.empty()) return false;
    const auto marker = raw.find(static_cast<wchar_t>(0xfff9), search_position);
    if (marker == std::wstring::npos) return false;
    NativeTableCoordinates coordinates;
    coordinates.begin = marker;
    coordinates.visual_rows.reserve(table.visual_rows.size());
    std::size_t cursor = marker;
    for (const auto& row : table.visual_rows) {
      const auto row_marker = raw.find(static_cast<wchar_t>(0xfff9), cursor);
      if (row_marker == std::wstring::npos) return false;
      const auto intervening_cell_mark = raw.find(static_cast<wchar_t>(0x0007), cursor);
      const auto intervening_row_end = raw.find(static_cast<wchar_t>(0xfffb), cursor);
      if ((intervening_cell_mark != std::wstring::npos && intervening_cell_mark < row_marker) ||
          (intervening_row_end != std::wstring::npos && intervening_row_end < row_marker))
        return false;
      cursor = row_marker + 1;
      if (cursor < raw.size() && raw[cursor] == L'\r') ++cursor;
      const auto row_end = raw.find(static_cast<wchar_t>(0xfffb), cursor);
      if (row_end == std::wstring::npos) return false;
      const auto nested_row_marker = raw.find(static_cast<wchar_t>(0xfff9), cursor);
      if (nested_row_marker != std::wstring::npos && nested_row_marker < row_end) return false;
      std::vector<NativeCellRange> visual_cells;
      visual_cells.reserve(row.cells.size());
      for (const auto& cell : row.cells) {
        const auto cell_mark = raw.find(static_cast<wchar_t>(0x0007), cursor);
        if (cell_mark == std::wstring::npos || cell_mark > row_end)
          return false;
        const NativeCellRange range{cursor, cell_mark};
        visual_cells.push_back(range);
        if (!cell.virtual_cell) coordinates.cells.push_back(range);
        cursor = cell_mark + 1;
      }
      const auto extra_cell_mark = raw.find(static_cast<wchar_t>(0x0007), cursor);
      if (extra_cell_mark != std::wstring::npos && extra_cell_mark < row_end) return false;
      coordinates.visual_rows.push_back(std::move(visual_cells));
      if (cursor > row_end) return false;
      cursor = row_end + 1;
      if (cursor < raw.size() && raw[cursor] == L'\r') ++cursor;
      coordinates.rows.push_back({row_marker, cursor});
    }
    coordinates.end = cursor;
    if (coordinates.cells.size() != table.cells.size() ||
        coordinates.visual_rows.size() != table.visual_rows.size() ||
        coordinates.rows.size() != table.visual_rows.size()) return false;
    for (std::size_t row_index{}; row_index < coordinates.visual_rows.size(); ++row_index) {
      if (coordinates.visual_rows[row_index].size() != table.visual_rows[row_index].cells.size())
        return false;
    }
    all_coordinates.push_back(std::move(coordinates));
    search_position = cursor;
  }
  for (std::size_t table_index{}; table_index < all_coordinates.size(); ++table_index)
    if (!ReplaceNativeTableCoordinates(snapshot, table_index, all_coordinates[table_index])) return false;
  return true;
}

bool FlattenRichEditTableText(const EditorSnapshot& snapshot,
                              std::wstring_view native_text,
                              std::wstring& flattened) {
  const std::wstring raw = CanonicalizeNativeText(native_text);
  flattened.clear();
  std::size_t native_position{};
  for (const auto& table : snapshot.tables) {
    if (!table.native_coordinates_set || table.native_begin < native_position ||
        table.native_end < table.native_begin || table.native_end > raw.size()) return false;
    flattened.append(raw.substr(native_position, table.native_begin - native_position));
    std::size_t source_cell_index{};
    std::size_t view_position = table.view_begin;
    for (const auto& row : table.visual_rows) {
      for (const auto& cell : row.cells) {
        if (cell.view_begin < view_position || cell.view_begin > cell.view_end ||
            cell.view_end > table.view_end) return false;
        flattened.append(snapshot.view.substr(view_position, cell.view_begin - view_position));
        if (cell.virtual_cell) {
          flattened.append(snapshot.view.substr(cell.view_begin, cell.view_end - cell.view_begin));
        } else {
          if (source_cell_index >= table.cells.size()) return false;
          const auto& source_cell = table.cells[source_cell_index++];
          if (source_cell.native_begin > source_cell.native_end ||
              source_cell.native_end > raw.size()) return false;
          flattened.append(raw.substr(source_cell.native_begin,
                                      source_cell.native_end - source_cell.native_begin));
        }
        view_position = cell.view_end;
      }
    }
    if (source_cell_index != table.cells.size()) return false;
    if (view_position > table.view_end) return false;
    flattened.append(snapshot.view.substr(view_position, table.view_end - view_position));
    native_position = table.native_end;
  }
  flattened.append(raw.substr(native_position));
  return true;
}

}  // namespace mdlite
