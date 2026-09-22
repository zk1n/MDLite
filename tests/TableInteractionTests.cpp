#include "table/Table.h"

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

void TestDelimiterValidity() {
  Check(mdlite::IsGfmTableDelimiter(L"| :--- | ---: | :---: |"),
        "alignment colons are valid delimiter cells");
  Check(mdlite::IsGfmTableDelimiter(L"--- | ---"),
        "outer pipes are optional for delimiter rows");
  Check(mdlite::IsGfmTableDelimiter(L"| --- | --- |  \r"),
        "whitespace after a trailing outer pipe is ignored");
  Check(!mdlite::IsGfmTableDelimiter(L"| -- | --- |"),
        "delimiter cells require at least three dashes");
  Check(!mdlite::IsGfmTableDelimiter(L"| -:- | --- |"),
        "alignment colons cannot occur inside delimiter dashes");
  Check(!mdlite::IsGfmTableDelimiter(L"| ---x | --- |"),
        "delimiter rows reject non-dash cell content");
}

void TestRangesAndVisualRows() {
  const std::wstring table =
      L"A | B\r\n"
      L":--- | ---:\r\n"
      L"東京 | `C|D`\r\n"
      L"last | B\\| raw";
  const auto rows = mdlite::ParseTableVisualRows(table, 0, table.size());
  Check(rows.size() == 4 && rows[0].cells.size() == 2 && rows[2].cells.size() == 2,
        "visual rows expose all valid GFM rows and columns");
  Check(rows[0].begin == 0 && rows[0].end == 5 &&
            table.substr(rows[0].cells[0].begin,
                         rows[0].cells[0].end - rows[0].cells[0].begin) == L"A ",
        "visual ranges retain raw cell padding and source offsets");
  Check(table.substr(rows[2].cells[1].begin,
                     rows[2].cells[1].end - rows[2].cells[1].begin) == L" `C|D`",
        "code-span pipes remain inside one visual cell");
  const auto clipped = mdlite::ParseTableVisualRows(table, table.find(L"東京"), table.size());
  Check(clipped.size() == 2 && clipped.front().begin == table.find(L"東京"),
        "viewport-clipped parsing keeps source row ranges");
  const auto escaped = mdlite::ParseTableVisualRows(
      L"| h | v |\n| --- | --- |\n| a\\|b | c |", 0, 35);
  Check(escaped.size() == 3 && escaped.back().cells.size() == 2,
        "escaped pipes do not create phantom visual columns");
}

void TestMalformedFallback() {
  const std::wstring malformed = L"plain | prose\nnot a delimiter | text\nvalue | another";
  Check(mdlite::ParseTableVisualRows(malformed, 0, malformed.size()).empty(),
        "non-GFM pipe text has no visual table rows");
  const auto edit = mdlite::InsertTableColumn(malformed, malformed.find(L"prose"), true);
  Check(!edit.changed && edit.text == malformed,
        "non-GFM pipe text is immutable under table actions");
  const std::wstring fenced =
      L"```\n| h | v |\n| --- | --- |\n| a | b |\n```";
  Check(mdlite::ParseTableVisualRows(fenced, 0, fenced.size()).empty(),
        "pipe text inside fenced code is not a visual table");
  const std::wstring bad_delimiter = L"| h | v |\n| -- | --- |\n| a | b |";
  Check(mdlite::ParseTableVisualRows(bad_delimiter, 0, bad_delimiter.size()).empty(),
        "a malformed delimiter falls back to ordinary Markdown");
}

void TestNavigationAndEditing() {
  const std::wstring table =
      L"| A | B |\n"
      L"| --- | --- |\n"
      L"| 1 | 2 |\n"
      L"| 3 |";
  const auto right = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"1") + 1, mdlite::TableCaretDirection::Right);
  Check(right && *right == table.find(L"2"),
        "right navigation moves only at a cell boundary");
  const auto down = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"A"), mdlite::TableCaretDirection::Down);
  Check(down && *down == table.find(L"1"),
        "vertical navigation skips the GFM delimiter row");
  const auto ragged_down = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"B"), mdlite::TableCaretDirection::Down);
  Check(ragged_down && *ragged_down == table.find(L"2"),
        "vertical navigation keeps the selected column before ragged rows");
  const std::wstring tab_table = L"| A | B |\n| --- | --- |\n| 1 | 2 |";
  const auto tab = mdlite::MoveToAdjacentTableCell(tab_table, tab_table.find(L"2"), false);
  Check(tab.changed && tab.text.ends_with(L"|  |  |"),
        "Tab in the final data cell appends a same-width source row");
  const auto shift_tab = mdlite::MoveToAdjacentTableCell(table, table.find(L"1"), true);
  Check(!shift_tab.changed && shift_tab.selection == table.rfind(L"---"),
        "Shift+Tab from a first data cell reaches the preceding row");

  const std::wstring literals =
      L"|  A  | B\\| raw | `C|D` |\r\n"
      L"| :--- | ---: | :---: |\r\n"
      L"| left  |  middle  | right |";
  const auto inserted = mdlite::InsertTableColumn(literals, literals.find(L"middle"), true);
  Check(inserted.changed && inserted.text.find(L"B\\| raw") != std::wstring::npos &&
            inserted.text.find(L"`C|D`") != std::wstring::npos,
        "column insertion preserves escaped/code-span source literals");
  const auto deleted = mdlite::DeleteTableColumn(literals, literals.find(L"middle"));
  Check(deleted.changed && deleted.text.find(L"middle") == std::wstring::npos &&
            deleted.text.find(L"`C|D`") != std::wstring::npos,
        "column deletion removes only the selected source column");
  const auto row = mdlite::InsertTableRow(literals, literals.find(L"middle"), false);
  Check(row.changed && row.text.find(L"|  |  |\r\n| left") != std::wstring::npos,
        "row insertion preserves the surrounding CRLF convention");

  const std::wstring ragged =
      L"| A | B |\n| --- | --- |\n| 1 |\n| 2 | 3 | 4 |";
  const auto ragged_insert = mdlite::InsertTableColumn(ragged, ragged.find(L"1"), true);
  Check(ragged_insert.changed && ragged_insert.text.find(L"| 1 |  |") != std::wstring::npos &&
            ragged_insert.text.find(L"| 2 |  | 3 | 4 |") != std::wstring::npos,
        "column insertion extends unequal rows without rewriting their literals");
  const auto ragged_delete = mdlite::DeleteTableColumn(ragged, ragged.find(L"3"));
  Check(ragged_delete.changed && ragged_delete.text.find(L"| 1 |\n") != std::wstring::npos &&
            ragged_delete.text.find(L"| 2 | 4 |") != std::wstring::npos,
        "column deletion leaves rows missing the selected column untouched");
}

}  // namespace

int main() {
  TestDelimiterValidity();
  TestRangesAndVisualRows();
  TestMalformedFallback();
  TestNavigationAndEditing();
  if (failures != 0) return 1;
  std::cout << "TableInteractionTests PASS\n";
  return 0;
}
