#include "editor/EditorAdapter.h"
#include "editor/RichEditTableAdapter.h"

#include <windows.h>
#include <richedit.h>
#include <richole.h>
#include <tom.h>

#include <algorithm>
#include <iostream>
#include <string>

namespace {

int failures{};

void Check(bool condition, const char* message) {
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

void CheckNativeCollapsedRangesMatchMarkdownReference(std::wstring_view source,
                                                       const char* message) {
  const auto reference = mdlite::BuildMarkdownEditorSnapshot(source);
  const auto native = mdlite::BuildNativeEditorSnapshot(source);
  bool matches = reference.collapsed.size() == native.collapsed.size();
  for (std::size_t index{}; matches && index < reference.collapsed.size(); ++index) {
    matches = reference.collapsed[index].source_begin == native.collapsed[index].source_begin &&
              reference.collapsed[index].source_end == native.collapsed[index].source_end;
  }
  Check(matches, message);
}

void TestNativeCollapsedImageRanges() {
  std::wstring large_plain;
  large_plain.reserve(160'000);
  for (int index{}; index < 4096; ++index)
    large_plain += L"plain text **bold** [link](note.md)\r\n";
  CheckNativeCollapsedRangesMatchMarkdownReference(
      large_plain, "large plain-text snapshot keeps the reference image ranges");

  const std::wstring unicode_and_images =
      L"# 日本語 😀\r\n"
      L"本文 **強調** ![画像](画像/猫.png)\n"
      L"![remote](https://example.test/cat.png)\r\n"
      L"![first](one.png)  ![second](two.png)\r\n"
      L"![separate line](three.png)";
  CheckNativeCollapsedRangesMatchMarkdownReference(
      unicode_and_images, "Unicode, mixed line endings, and adjacent images match the reference");

  const std::wstring tables_and_images =
      L"before ![before](before.png)\r\n"
      L"| 名前 | 説明 |\r\n"
      L"| --- | --- |\r\n"
      L"| 猫 ![inside](images/cat.png) | 😀 |\r\n"
      L"| ragged |\r\n"
      L"after ![after](after.png)";
  CheckNativeCollapsedRangesMatchMarkdownReference(
      tables_and_images, "images before, inside, and after a ragged table match the reference");

  const std::wstring markdown_tokens =
      L"---\r\nimage: ![front matter](metadata.png)\r\n---\r\n"
      L"```md\r\n![fenced](fenced.png)\r\n```\r\n"
      L"`![inline code](inline.png)` ![visible](visible.png)";
  CheckNativeCollapsedRangesMatchMarkdownReference(
      markdown_tokens, "front matter, fences, and inline Markdown tokens match the reference");
}

bool PositionOf(HWND editor, LONG character, POINT& point) {
  return SendMessageW(editor, EM_POSFROMCHAR,
                      reinterpret_cast<WPARAM>(&point), character) != -1;
}

bool TomSelectionRange(HWND editor, LONG& start, LONG& end) {
  IRichEditOle* rich_edit{};
  if (SendMessageW(editor, EM_GETOLEINTERFACE, 0,
                   reinterpret_cast<LPARAM>(&rich_edit)) == 0 || !rich_edit) return false;
  ITextDocument* document{};
  ITextSelection* selection{};
  const bool obtained =
      SUCCEEDED(rich_edit->QueryInterface(__uuidof(ITextDocument),
                                          reinterpret_cast<void**>(&document))) && document &&
      SUCCEEDED(document->GetSelection(&selection)) && selection;
  const bool read = obtained && SUCCEEDED(selection->GetStart(&start)) &&
                    SUCCEEDED(selection->GetEnd(&end));
  if (selection) selection->Release();
  if (document) document->Release();
  rich_edit->Release();
  return read;
}

void TestNativeWrappingAndMapping() {
  HMODULE rich_edit_module = LoadLibraryW(L"Msftedit.dll");
  Check(rich_edit_module != nullptr, "Msftedit.dll loads for native table rendering");
  if (!rich_edit_module) return;

  const HWND host = CreateWindowExW(0, L"STATIC", L"RichEdit table verification",
                                    WS_POPUP | WS_VISIBLE,
                                    40, 40, 380, 500, nullptr, nullptr,
                                    GetModuleHandleW(nullptr), nullptr);
  const HWND editor = CreateWindowExW(0, MSFTEDIT_CLASS, L"",
                                      WS_CHILD | WS_VISIBLE | ES_MULTILINE | WS_VSCROLL,
                                      0, 0, 360, 480, host, nullptr,
                                      GetModuleHandleW(nullptr), nullptr);
  Check(host && editor, "RichEdit control is created under a native window");
  if (!host || !editor) {
    if (host) DestroyWindow(host);
    FreeLibrary(rich_edit_module);
    return;
  }

  const std::wstring flat_projection = L"第一段落\rsecond paragraph 😀\r終わり";
  SetWindowTextW(editor, flat_projection.c_str());
  Check(mdlite::RichEditTextLengthEquals(editor, flat_projection.size()),
        "native RichEdit length matches CR-separated Unicode flat text");
  Check(!mdlite::RichEditTextLengthEquals(editor, flat_projection.size() + 1),
        "native RichEdit length rejects an expected-length mismatch");
  Check(mdlite::RichEditTextEquals(editor, flat_projection),
        "chunked native-text verification matches CR paragraphs and Unicode");
  Check(mdlite::RichEditTextRangeEquals(editor, 1, 4, L"一段落"),
        "native-text range verification matches the requested changed range");
  Check(!mdlite::RichEditTextRangeEquals(editor, 1, 4, L"違う"),
        "native-text range verification rejects changed content");
  Check(!mdlite::RichEditTextEquals(editor, L"短い"),
        "full native-text verification rejects a total-length mismatch");

  const auto collapsed_image_snapshot = mdlite::BuildNativeEditorSnapshot(
      L"before ![asset](image.png) after");
  Check(collapsed_image_snapshot.collapsed.size() == 1,
        "flat projection fixture contains one collapsed-image marker");
  SetWindowTextW(editor, collapsed_image_snapshot.view.c_str());
  std::wstring observed_collapsed_projection;
  const bool observed_collapsed_text =
      mdlite::ReadRichEditNativeText(editor, observed_collapsed_projection);
  const auto marker_position = collapsed_image_snapshot.collapsed.empty()
      ? std::wstring::npos : collapsed_image_snapshot.collapsed.front().view;
  Check(observed_collapsed_text && marker_position < observed_collapsed_projection.size() &&
            (observed_collapsed_projection[marker_position] == static_cast<wchar_t>(0xfffc) ||
             observed_collapsed_projection[marker_position] == L' '),
        "RichEdit reads the collapsed-image slot as U+FFFC or one space");
  Check(mdlite::RichEditFlatProjectionEquals(editor, collapsed_image_snapshot),
        "flat projection verifier accepts RichEdit's observed collapsed-image representation");

  auto space_collapsed_projection = collapsed_image_snapshot.view;
  if (marker_position < space_collapsed_projection.size())
    space_collapsed_projection[marker_position] = L' ';
  SetWindowTextW(editor, space_collapsed_projection.c_str());
  Check(mdlite::RichEditFlatProjectionEquals(editor, collapsed_image_snapshot),
        "flat projection verifier accepts a space at the collapsed-image slot");
  auto non_marker_mismatch = collapsed_image_snapshot;
  non_marker_mismatch.view[0] = L'B';
  Check(!mdlite::RichEditFlatProjectionEquals(editor, non_marker_mismatch),
        "flat projection verifier rejects mismatches outside collapsed-image slots");

  std::wstring chunked_projection;
  chunked_projection.reserve(9000);
  for (int index{}; index < 9000; ++index)
    chunked_projection.push_back(index == 4095 ? L'猫' : L'x');
  SetWindowTextW(editor, chunked_projection.c_str());
  Check(mdlite::RichEditTextEquals(editor, chunked_projection),
        "native-text verification matches text spanning fixed-size chunk boundaries");
  chunked_projection[4096] = L'猫';
  Check(!mdlite::RichEditTextEquals(editor, chunked_projection),
        "native-text verification detects a mismatch after a chunk boundary");

  std::wstring long_cell;
  for (int index{}; index < 48; ++index) long_cell += L"日本語";
  long_cell += L"😀";
  const std::wstring source =
      L"before\r\n"
      L"| 名前 | 状態 |\r\n"
      L"| :--- | ---: |\r\n"
      L"| " + long_cell + L" | right |\r\n"
      L"|  | below |\r\n"
      L"middle\r\n"
      L"| X | Y |\r\n"
      L"| --- | --- |\r\n"
      L"| x | y |\r\n"
      L"| z | w |\r\n"
      L"after";
  auto snapshot = mdlite::BuildNativeEditorSnapshot(source);
  Check(snapshot.tables.size() == 2, "snapshot contains both source tables");
  SetWindowTextW(editor, snapshot.view.c_str());
  UpdateWindow(editor);

  mdlite::RichEditTableStyle style;
  style.available_width_twips = 3600;
  style.minimum_cell_width_twips = 1200;
  style.cell_margin_twips = 80;
  style.font_size_half_points = 20;
  style.border_color = RGB(70, 82, 98);
  style.header_background = RGB(31, 40, 52);
  style.body_background = RGB(24, 31, 40);
  Check(mdlite::RebuildRichEditTables(editor, snapshot, style),
        "production table projection streams native RichEdit cell rows");

  std::wstring raw;
  Check(mdlite::ReadRichEditNativeText(editor, raw),
        "native table text and structural markers read back completely");
  const auto starts = std::count(raw.begin(), raw.end(), static_cast<wchar_t>(0xfff9));
  const auto ends = std::count(raw.begin(), raw.end(), static_cast<wchar_t>(0xfffb));
  const auto cell_marks = std::count(raw.begin(), raw.end(), static_cast<wchar_t>(0x0007));
  Check(starts == 6 && ends == 6 && cell_marks == 12,
        "raw readback exposes all native row delimiters and cell marks");
  Check(raw.find(L"before") != std::wstring::npos && raw.find(L"middle") != std::wstring::npos &&
            raw.find(L"after") != std::wstring::npos,
        "native table stream preserves text before, between, and after tables");

  std::wstring flattened;
  Check(mdlite::FlattenRichEditTableText(snapshot, raw, flattened),
        "native row and cell marks flatten through the source mapping");
  Check(flattened == snapshot.view,
        "unmodified native table stream round-trips to the exact projected editor text");

  const auto raw_cell = raw.find(long_cell);
  const auto raw_right = raw.find(L"right");
  const auto raw_next_row = raw.find(L"below");
  const auto source_cell = source.find(long_cell);
  if (raw_cell != std::wstring::npos && raw_right != std::wstring::npos &&
      raw_next_row != std::wstring::npos && source_cell != std::wstring::npos) {
    POINT first_start{};
    POINT wrapped_mid{};
    POINT right_start{};
    POINT second_row_start{};
    Check(PositionOf(editor, static_cast<LONG>(raw_cell), first_start),
          "raw cell offsets match RichEdit native positions");
    Check(PositionOf(editor, static_cast<LONG>(raw_cell + 30), wrapped_mid),
          "wrapped cell midpoint has a native screen position");
    Check(PositionOf(editor, static_cast<LONG>(raw_right), right_start),
          "adjacent cell has a native screen position");
    Check(PositionOf(editor, static_cast<LONG>(raw_next_row), second_row_start),
          "next row has a native screen position");
    Check(wrapped_mid.y > first_start.y && wrapped_mid.x < right_start.x &&
              second_row_start.y > first_start.y,
          "long Unicode text wraps within its cell and grows the row height");
    POINT hit = wrapped_mid;
    const auto native_hit = static_cast<std::size_t>(std::max<LRESULT>(
        0, SendMessageW(editor, EM_CHARFROMPOS, 0, reinterpret_cast<LPARAM>(&hit))));
    const auto source_hit = snapshot.NativeToSource(native_hit);
    Check(source_hit >= source_cell && source_hit <= source_cell + long_cell.size(),
          "native hit testing maps wrapped text back into the same source cell");
    Check(snapshot.SourceToNative(source_cell) == raw_cell,
          "source-to-native mapping includes RichEdit row and cell delimiters");
  } else {
    Check(false, "native readback contains the expected source cell text");
  }

  const std::wstring ragged_source =
      L"# Table UI probe\n\n"
      L"| Head A | Head B | Head C |\n"
      L"| :--- | ---: | --- |\n"
      L"| one | two |\n";
  auto ragged_snapshot = mdlite::BuildNativeEditorSnapshot(ragged_source);
  SetWindowTextW(editor, ragged_snapshot.view.c_str());
  Check(mdlite::RebuildRichEditTables(editor, ragged_snapshot, style),
        "ragged native table builds for endpoint mapping verification");
  for (const auto& table : ragged_snapshot.tables) {
    for (const auto& cell : table.cells) {
      for (std::size_t source_position = cell.source_begin;
           source_position <= cell.source_end; ++source_position) {
        const auto native_position = ragged_snapshot.SourceToNative(source_position);
        const auto restored_source = ragged_snapshot.NativeToSource(native_position);
        if (restored_source != source_position) {
          std::cerr << "mapping mismatch source=" << source_position
                    << " native=" << native_position
                    << " restored=" << restored_source << '\n';
        }
        Check(restored_source == source_position,
              "ragged table cell endpoints round trip through native coordinates");
      }
    }
  }
  for (std::size_t source_position = ragged_source.rfind(L"two");
       source_position <= ragged_source.size(); ++source_position) {
    const auto native_position = ragged_snapshot.SourceToNative(source_position);
    const auto restored_source = ragged_snapshot.NativeToSource(native_position);
    if (restored_source != source_position) {
      std::cerr << "tail mismatch source=" << source_position
                << " native=" << native_position
                << " restored=" << restored_source << '\n';
    }
    Check(restored_source == source_position,
          "ragged table trailing-pipe and paragraph offsets round trip");
    const LONG requested_native = static_cast<LONG>(native_position);
    CHARRANGE requested{requested_native, requested_native};
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&requested));
    CHARRANGE actual{};
    SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&actual));
    const auto editor_restored_source = ragged_snapshot.NativeToSource(
        static_cast<std::size_t>(std::max<LONG>(0, actual.cpMin)));
    if (editor_restored_source != source_position) {
      std::cerr << "RichEdit selection mismatch source=" << source_position
                << " requested_native=" << requested_native
                << " actual_native=" << actual.cpMin
                << " restored=" << editor_restored_source << '\n';
    }
    Check(editor_restored_source == source_position,
          "RichEdit preserves source positions across ragged table endpoints");
    LONG tom_start{};
    LONG tom_end{};
    const bool tom_read = TomSelectionRange(editor, tom_start, tom_end);
    const auto tom_source = tom_read ? ragged_snapshot.NativeToSource(
        static_cast<std::size_t>(std::max<LONG>(0, tom_start))) : 0;
    if (!tom_read || tom_source != source_position) {
      std::cerr << "TOM selection mismatch source=" << source_position
                << " em=" << actual.cpMin << " tom=" << tom_start
                << " restored=" << tom_source << '\n';
    }
    Check(tom_read && tom_source == source_position,
          "TOM and RichEdit coordinates preserve ragged table source selections");
  }
  auto projection_snapshot = mdlite::BuildNativeEditorSnapshot(ragged_source);
  const std::size_t virtual_anchor = ragged_source.rfind(L'|');
  const auto flat_anchor = projection_snapshot.SourceToNative(virtual_anchor);
  CHARRANGE flat_selection{static_cast<LONG>(flat_anchor), static_cast<LONG>(flat_anchor)};
  SetWindowTextW(editor, projection_snapshot.view.c_str());
  SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&flat_selection));
  LONG flat_tom_start{};
  LONG flat_tom_end{};
  const bool flat_tom_read = TomSelectionRange(editor, flat_tom_start, flat_tom_end);
  const auto flat_restored_anchor = flat_tom_read ? projection_snapshot.NativeToSource(
      static_cast<std::size_t>(std::max<LONG>(0, flat_tom_start))) : 0;
  if (!flat_tom_read || flat_restored_anchor != virtual_anchor) {
    std::cerr << "flat selection mismatch source=" << virtual_anchor
              << " flat=" << flat_anchor << " tom=" << flat_tom_start
              << " restored=" << flat_restored_anchor << '\n';
  }
  Check(flat_tom_read && flat_restored_anchor == virtual_anchor,
        "TOM selection in flattened table projection preserves source offset");
  Check(mdlite::RebuildRichEditTables(editor, projection_snapshot, style),
        "table topology rebuilds from the flattened source view");
  const auto native_anchor = projection_snapshot.SourceToNative(virtual_anchor);
  CHARRANGE native_selection{static_cast<LONG>(native_anchor), static_cast<LONG>(native_anchor)};
  SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&native_selection));
  LONG tom_start{};
  LONG tom_end{};
  const bool tom_read = TomSelectionRange(editor, tom_start, tom_end);
  const auto restored_anchor = tom_read ? projection_snapshot.NativeToSource(
      static_cast<std::size_t>(std::max<LONG>(0, tom_start))) : 0;
  if (!tom_read || restored_anchor != virtual_anchor) {
    std::cerr << "projection restore mismatch source=" << virtual_anchor
              << " flat=" << flat_anchor << " native=" << native_anchor
              << " tom=" << tom_start << " restored=" << restored_anchor << '\n';
  }
  Check(tom_read && restored_anchor == virtual_anchor,
        "source caret survives table projection rebuild through TOM selection");

  const auto& undo_table = projection_snapshot.tables.front();
  const auto& undo_row = undo_table.visual_rows.back();
  const auto virtual_cell = std::ranges::find_if(undo_row.cells, [](const auto& cell) {
    return cell.virtual_cell;
  });
  Check(virtual_cell != undo_row.cells.end(),
        "ragged final row exposes its virtual cell after native table rebuild");
  if (virtual_cell != undo_row.cells.end()) {
    const std::size_t virtual_column =
        static_cast<std::size_t>(virtual_cell - undo_row.cells.begin());
    const std::size_t before_edit = virtual_cell->source_begin;
    const auto source_native = projection_snapshot.SourceToNative(before_edit);
    Check(source_native == virtual_cell->native_begin,
          "source caret restoration targets the virtual cell coordinate");

    CHARRANGE clicked_virtual_cell{static_cast<LONG>(virtual_cell->native_begin),
                                   static_cast<LONG>(virtual_cell->native_begin)};
    SendMessageW(editor, EM_EXSETSEL, 0,
                 reinterpret_cast<LPARAM>(&clicked_virtual_cell));
    LONG virtual_start{};
    LONG virtual_end{};
    const bool virtual_read = TomSelectionRange(editor, virtual_start, virtual_end);
    const auto virtual_source = virtual_read ? projection_snapshot.NativeToSource(
        static_cast<std::size_t>(std::max<LONG>(0, virtual_start))) : 0;
    Check(virtual_read && virtual_source == before_edit,
          "TOM caret in the blank native cell maps to its source insertion point");

    const auto inserted = mdlite::InsertTextIntoMissingTableCell(
        ragged_source, undo_row.source_begin, virtual_column, L"x");
    Check(inserted.changed,
          "typing into a virtual cell creates a source-backed third cell");
    if (inserted.changed) {
      auto edited_snapshot = mdlite::BuildNativeEditorSnapshot(inserted.text);
      SetWindowTextW(editor, edited_snapshot.view.c_str());
      const bool edited_built = mdlite::RebuildRichEditTables(editor, edited_snapshot, style);
      Check(edited_built, "edited table topology builds before caret restoration");
      if (edited_built) {
        const auto edited_native = mdlite::SourceSelectionToNative(
            edited_snapshot, {inserted.selection, inserted.selection});
        CHARRANGE edited_selection{static_cast<LONG>(edited_native.anchor),
                                   static_cast<LONG>(edited_native.active)};
        SendMessageW(editor, EM_EXSETSEL, 0,
                     reinterpret_cast<LPARAM>(&edited_selection));
        LONG edited_start{};
        LONG edited_end{};
        const bool edited_read = TomSelectionRange(editor, edited_start, edited_end);
        const auto edited_source = edited_read ? edited_snapshot.NativeToSource(
            static_cast<std::size_t>(std::max<LONG>(0, edited_start))) : 0;
        Check(edited_read && edited_source == inserted.selection,
              "typed-cell caret round trips after the table gains a source cell");
      }

      auto restored_snapshot = mdlite::BuildNativeEditorSnapshot(ragged_source);
      SetWindowTextW(editor, restored_snapshot.view.c_str());
      const bool undo_built = mdlite::RebuildRichEditTables(editor, restored_snapshot, style);
      Check(undo_built, "ragged table topology rebuilds for Undo selection restoration");
      if (undo_built) {
        const auto restored_native = mdlite::SourceSelectionToNative(
            restored_snapshot, {before_edit, before_edit});
        CHARRANGE restored_selection{static_cast<LONG>(restored_native.anchor),
                                     static_cast<LONG>(restored_native.active)};
        SendMessageW(editor, EM_EXSETSEL, 0,
                     reinterpret_cast<LPARAM>(&restored_selection));
        LONG undo_start{};
        LONG undo_end{};
        const bool undo_read = TomSelectionRange(editor, undo_start, undo_end);
        const auto undo_source = undo_read ? restored_snapshot.NativeToSource(
            static_cast<std::size_t>(std::max<LONG>(0, undo_start))) : 0;
        const auto& restored_virtual =
            restored_snapshot.tables.front().visual_rows.back().cells[virtual_column];
        Check(undo_read && undo_source == before_edit,
              "Undo restores the exact pre-edit source caret after a table shape change");
        Check(undo_read && undo_start >= static_cast<LONG>(restored_virtual.native_begin) &&
                  undo_start <= static_cast<LONG>(restored_virtual.native_end),
              "Undo restores the caret inside the same native virtual cell");
      }
    }
  }

  DestroyWindow(host);
  FreeLibrary(rich_edit_module);
}

}  // namespace

int main() {
  TestNativeCollapsedImageRanges();
  TestNativeWrappingAndMapping();
  if (failures != 0) return 1;
  std::cout << "NativeRichEditTableTests PASS\n";
  return 0;
}
