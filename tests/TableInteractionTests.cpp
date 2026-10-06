#include "table/Table.h"

#include <iostream>
#include <string>
#include <utility>

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
  Check(mdlite::IsGfmTableDelimiter(L"| - | :- | -: | :-: |"),
        "one hyphen with optional edge colons is valid in each alignment form");
  Check(!mdlite::IsGfmTableDelimiter(L"| : | --- |"),
        "delimiter cells require at least one hyphen");
  Check(!mdlite::IsGfmTableDelimiter(L"| :-x: | --- |"),
        "delimiter cells reject interior non-hyphen content");
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

void TestSharedGfmTableParser() {
  const std::wstring source =
      L"before\r\n"
      L"| H1 | H2 | H3 |\r\n"
      L"| :--- | :---: | ---: |\r\n"
      L"| a\\|b | `c|d` | x |\r\n"
      L"| short | row |\r\n"
      L"\r\n"
      L"| next | table |\r\n"
      L"| --- | --- |\r\n"
      L"end";
  const auto first = mdlite::ParseGfmTableAt(source, source.find(L"H1"));
  Check(first && first->rows.size() == 4 && first->alignments.size() == 3,
        "shared GFM parser returns header, delimiter, ragged body, and alignment data");
  if (first) {
    Check(first->begin == source.find(L"| H1") &&
              first->end == source.find(L"\r\n\r\n"),
          "shared GFM parser returns exact CRLF source block boundaries");
    Check(first->alignments[0] == mdlite::TableAlignment::Left &&
              first->alignments[1] == mdlite::TableAlignment::Center &&
              first->alignments[2] == mdlite::TableAlignment::Right,
          "alignment markers map to the corresponding table columns");
    const auto& escaped_cell = first->rows[2].cells[0];
    const auto& code_cell = first->rows[2].cells[1];
    Check(source.substr(escaped_cell.begin, escaped_cell.end - escaped_cell.begin) == L" a\\|b " &&
              source.substr(code_cell.begin, code_cell.end - code_cell.begin) == L" `c|d` ",
          "shared parser exposes raw cell ranges without splitting escaped or code-span pipes");
    Check(first->rows[3].cells.size() == 2 && first->rows[3].end == source.find(L"\r\n\r\n"),
          "ragged body rows retain their own exact source ranges");
  }

  const auto second = mdlite::ParseGfmTableAt(source, source.find(L"next"));
  Check(second && second->begin == source.find(L"| next") && second->rows.size() == 2,
        "multiple tables are independently addressable by source position");

  const std::wstring single_hyphen =
      L"| left | center | right |\n"
      L"| :- | :-: | -: |\n"
      L"| a | b |";
  const auto aligned = mdlite::ParseGfmTableAt(single_hyphen, 0);
  Check(aligned && aligned->alignments.size() == 3 &&
            aligned->alignments[0] == mdlite::TableAlignment::Left &&
            aligned->alignments[1] == mdlite::TableAlignment::Center &&
            aligned->alignments[2] == mdlite::TableAlignment::Right,
        "one-hyphen delimiter cells produce left, center, and right alignment");
  Check(aligned && aligned->rows.size() == 3 && aligned->rows[2].cells.size() == 2 &&
            single_hyphen.substr(aligned->rows[2].begin,
                                 aligned->rows[2].end - aligned->rows[2].begin) == L"| a | b |",
        "one-hyphen parsing preserves the raw short body row without padding it");

  const std::wstring mismatched = L"h | v | x\n--- | ---\na | b | c";
  Check(!mdlite::ParseGfmTableAt(mismatched, 0),
        "header and delimiter column mismatch is rejected by shared recognition");
  const std::wstring no_hyphen_delimiter = L"h | v\n: | ---\na | b";
  Check(!mdlite::ParseGfmTableAt(no_hyphen_delimiter, 0),
        "a delimiter cell with no hyphen is rejected by shared recognition");
  const std::wstring fenced = L"~~~\nh | v\n--- | ---\na | b\n~~~";
  Check(!mdlite::ParseGfmTableAt(fenced, fenced.find(L"h | v")),
        "shared recognition excludes table-like text inside a fenced code block");

  for (const auto& [source, row] : {
           std::pair{std::wstring(L"    a | b\n|---|---|\n"), std::wstring_view(L"a | b")},
           std::pair{std::wstring(L"\ta | b\n|---|---|\n"), std::wstring_view(L"a | b")}}) {
    const auto position = source.find(row);
    Check(!mdlite::ParseGfmTableAt(source, position),
          "shared recognition excludes indented code rows followed by a table delimiter");
    Check(mdlite::ParseTableVisualRows(source, 0, source.size()).empty(),
          "indented code with a table-like delimiter has no visual table rows");
    const auto edit = mdlite::InsertTableColumn(source, position, true);
    Check(!edit.changed && edit.text == source,
          "structural table edits leave indented code source unchanged");
    Check(mdlite::ParseGfmTableAtHeader(source, 0).has_value(),
          "header-anchored parsing retains its caller-supplied candidate contract");
  }
}

void TestCachedTableCaretNavigation() {
  const std::wstring source =
      L"a long body before the table\r\n"
      L"~~~\r\n| not | table |\r\n| --- | --- |\r\n~~~\r\n"
      L"| H1 | H2 | H3 |\r\n"
      L"| :--- | :---: | ---: |\r\n"
      L"| a\\|b | `c|d` | x |\r\n"
      L"| short | row |\r\n\r\n"
      L"| next | table |\r\n"
      L"| --- | --- |\r\n"
      L"| last | row |";
  const auto first = mdlite::ParseGfmTableAt(source, source.find(L"H1"));
  Check(first.has_value(), "cached caret navigation receives the parser's table snapshot");
  if (!first) return;

  for (const auto& row : first->rows) {
    for (std::size_t caret = row.begin; caret <= row.end; ++caret) {
      const auto scanned = mdlite::ResolveTableCellIntent(source, caret);
      const auto cached = mdlite::ResolveTableCellIntent(*first, source, caret);
      Check(scanned.has_value() == cached.has_value(),
            "cached and scanning caret lookup agree on whether the caret is in a table row");
      if (scanned && cached) {
        Check(scanned->row_begin == cached->row_begin &&
                  scanned->column == cached->column &&
                  scanned->source_position == cached->source_position &&
                  scanned->virtual_cell == cached->virtual_cell,
              "cached caret lookup preserves source position and ragged-cell intent");
        for (const auto direction : {mdlite::TableCaretDirection::Left,
                                     mdlite::TableCaretDirection::Right,
                                     mdlite::TableCaretDirection::Up,
                                     mdlite::TableCaretDirection::Down}) {
          const auto expected = mdlite::MoveTableCaretTargetAtBoundary(source, *scanned, direction);
          const auto actual = mdlite::MoveTableCaretTargetAtBoundary(*first, source, *cached,
                                                                     direction);
          Check(expected.has_value() == actual.has_value(),
                "cached arrow lookup preserves table-edge transitions");
          if (expected && actual) {
            Check(expected->row_begin == actual->row_begin &&
                      expected->column == actual->column &&
                      expected->source_position == actual->source_position &&
                      expected->virtual_cell == actual->virtual_cell,
                  "cached arrow lookup targets the same adjacent source cell");
          }
        }
      }
    }
  }
  Check(!mdlite::ResolveTableCellIntent(*first, source, source.find(L"not | table")),
        "a cached table never captures table-like content in an earlier fenced code block");
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
  const std::wstring bad_delimiter = L"| h | v |\n| : | --- |\n| a | b |";
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
  Check(!shift_tab.changed && shift_tab.selection == table.find(L"B") &&
            shift_tab.target_cell && shift_tab.target_cell->row_begin == 0 &&
            shift_tab.target_cell->column == 1,
        "Shift+Tab from a first data cell reaches the preceding visible row and skips the delimiter");

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

void TestColumnInsertionPreservesPadding() {
  const std::wstring table =
      L"before\r\n\r\n| A | B | C |\r\n| :--- | :---: | ---: |\r\n"
      L"| one |  | 日本語 |\r\n| three | four | five |\r\n\r\nafter\r\n";
  const auto middle = mdlite::InsertTableColumn(table, table.find(L"one"), true);
  Check(middle.changed && middle.text ==
      L"before\r\n\r\n| A |  | B | C |\r\n| :--- | --- | :---: | ---: |\r\n"
      L"| one |  |  | 日本語 |\r\n| three |  | four | five |\r\n\r\nafter\r\n",
      "inserting before an empty cell preserves its padding, CRLF and surrounding source");
  const std::wstring last = L"| A | B |\n| --- | --- |\n| one |  |";
  const auto appended = mdlite::InsertTableColumn(last, last.size() - 1, true);
  Check(appended.changed && appended.text.ends_with(L"| one |  |  |"),
      "appending after an empty last cell keeps both empty cells' padding");
  const std::wstring padded = L"| A | B |\n| --- | --- |\n| one |  two  |";
  const auto spaced = mdlite::InsertTableColumn(padded, padded.find(L"one"), true);
  Check(spaced.changed && spaced.text.ends_with(L"| one |  |  two  |"),
      "a new column does not move a pre-existing nonempty cell's whitespace");
  const std::wstring tabs = L"| A | B |\n| --- | --- |\n| one | \t |";
  const auto tabbed = mdlite::InsertTableColumn(tabs, tabs.find(L"one"), true);
  Check(tabbed.changed && tabbed.text.ends_with(L"| one |  | \t |"),
      "a pre-existing empty cell retains its literal tab padding");
  const auto first = mdlite::InsertTableColumn(last, last.find(L"one"), false);
  Check(first.changed && first.text.ends_with(L"|  | one |  |"),
      "inserting before the first column preserves all existing cell ranges");
  const std::wstring unframed = L"A | B\n--- | ---\none | two";
  const auto outerless = mdlite::InsertTableColumn(unframed, unframed.find(L"two"), true);
  const auto parsed = mdlite::ParseGfmTableAt(outerless.text, outerless.selection);
  Check(outerless.changed && parsed && parsed->alignments.size() == 3 &&
      outerless.text.ends_with(L"one | two|  |"),
      "unframed rows remain valid while existing last-cell bytes stay intact");
}

void TestMissingTableCellInsertion() {
  const std::wstring outer_pipe_table =
      L"| A | B | C | D |\r\n"
      L"| --- | --- | --- | --- |\r\n"
      L"| first | second |\r\n"
      L"| excess | one | two | three | overflow |\r\n"
      L"tail";
  const auto short_row_begin = outer_pipe_table.find(L"| first");
  const auto inserted = mdlite::InsertTextIntoMissingTableCell(
      outer_pipe_table, short_row_begin, 3, L"fourth");
  const std::wstring expected_outer_pipe_table =
      L"| A | B | C | D |\r\n"
      L"| --- | --- | --- | --- |\r\n"
      L"| first | second |  | fourth |\r\n"
      L"| excess | one | two | three | overflow |\r\n"
      L"tail";
  const auto excess_row_begin = outer_pipe_table.find(L"| excess");
  const auto excess_row_end = outer_pipe_table.find(L"\r\n", excess_row_begin);
  const auto excess_row = outer_pipe_table.substr(excess_row_begin,
                                                   excess_row_end - excess_row_begin);
  const auto excess_row_after = inserted.text.find(L"| excess");
  Check(inserted.changed && inserted.text == expected_outer_pipe_table,
        "typing into a later missing outer-pipe cell inserts blank predecessors and preserves CRLF");
  Check(inserted.selection == inserted.text.find(L"fourth") + std::wstring_view(L"fourth").size(),
        "missing-cell insertion returns a caret after the supplied text");
  Check(excess_row_after != std::wstring::npos &&
            inserted.text.substr(excess_row_after, excess_row.size()) == excess_row,
        "missing-cell insertion preserves a neighboring excess-cell row byte-for-byte");
  const auto parsed_outer_pipe = mdlite::ParseGfmTableAt(inserted.text, short_row_begin);
  Check(parsed_outer_pipe && parsed_outer_pipe->rows[2].cells.size() == 4,
        "materializing skipped columns produces the requested body-cell index");

  const std::wstring no_outer_pipe_table =
      L"A | B | C\n--- | --- | ---\none | two\ntail";
  const auto no_outer_row_begin = no_outer_pipe_table.find(L"one | two");
  const auto no_outer_inserted = mdlite::InsertTextIntoMissingTableCell(
      no_outer_pipe_table, no_outer_row_begin, 2, L"three");
  Check(no_outer_inserted.changed &&
            no_outer_inserted.text == L"A | B | C\n--- | --- | ---\none | two | three\ntail" &&
            no_outer_inserted.selection == no_outer_inserted.text.find(L"three") + 5,
        "missing-cell insertion supports rows without outer pipes and preserves LF");
  const auto escaped_pipe = mdlite::InsertTextIntoMissingTableCell(
      no_outer_pipe_table, no_outer_row_begin, 2, L"escaped\\|pipe");
  const auto escaped_pipe_table = mdlite::ParseGfmTableAt(escaped_pipe.text, no_outer_row_begin);
  const auto escaped_pipe_cell = escaped_pipe_table
      ? escaped_pipe.text.substr(escaped_pipe_table->rows[2].cells[2].begin,
                                 escaped_pipe_table->rows[2].cells[2].end -
                                     escaped_pipe_table->rows[2].cells[2].begin)
      : std::wstring{};
  Check(escaped_pipe.changed && escaped_pipe_table &&
            escaped_pipe_table->rows[2].cells.size() == 3 &&
            escaped_pipe_cell == L" escaped\\|pipe",
        "already escaped pipes are inserted verbatim without creating an extra cell");

  const auto header = outer_pipe_table.find(L"| A");
  const auto delimiter = outer_pipe_table.find(L"| ---");
  const auto body = outer_pipe_table.find(L"| first");
  const auto unchanged = [&](std::size_t row, std::size_t column, std::wstring_view text) {
    const auto result = mdlite::InsertTextIntoMissingTableCell(
        outer_pipe_table, row, column, text);
    return !result.changed && result.text == outer_pipe_table;
  };
  Check(unchanged(header, 3, L"x") && unchanged(delimiter, 3, L"x"),
        "header and delimiter rows reject virtual-cell insertion");
  Check(unchanged(body, 1, L"x") && unchanged(body, 4, L"x"),
        "existing-cell and out-of-range targets are rejected without source changes");
  Check(unchanged(body, 3, L"x|y") && unchanged(body, 3, L"x\\\\|y") &&
            unchanged(body, 3, L"x\ny"),
        "bare/even-escaped pipes and newline input are rejected as unsafe row structure");
  Check(unchanged(body + 1, 3, L"x"),
        "row positions that are not source line starts are rejected");
}

void TestHeaderAnchoredNavigation() {
  const std::wstring source =
      L"| A | B | C | D |\n"
      L"| --- | --- | --- | --- |\n"
      L"| first | second |\n"
      L"| x | y | z | fourth | overflow |\n"
      L"| last |";
  const auto short_row_begin = source.find(L"| first");
  const auto excess_row_begin = source.find(L"| x");
  const auto last_row_begin = source.find(L"| last");
  const auto from_second = mdlite::ResolveTableCellIntent(source, source.find(L"second"));
  Check(from_second && from_second->row_begin == short_row_begin &&
            from_second->column == 1 && !from_second->virtual_cell,
        "source caret resolves to a header-bounded row and column intent");
  if (!from_second) return;

  const auto virtual_third = mdlite::MoveToAdjacentTableCell(source, *from_second, false);
  const auto virtual_fourth = virtual_third.target_cell
      ? mdlite::MoveToAdjacentTableCell(source, *virtual_third.target_cell, false)
      : mdlite::TableEditResult{};
  Check(!virtual_third.changed && virtual_third.text == source && virtual_third.target_cell &&
            virtual_third.target_cell->row_begin == short_row_begin &&
            virtual_third.target_cell->column == 2 && virtual_third.target_cell->virtual_cell &&
            !virtual_fourth.changed && virtual_fourth.text == source && virtual_fourth.target_cell &&
            virtual_fourth.target_cell->row_begin == short_row_begin &&
            virtual_fourth.target_cell->column == 3 && virtual_fourth.target_cell->virtual_cell,
        "Tab visits each implied trailing body slot without editing Markdown");
  if (!virtual_fourth.target_cell) return;

  const auto next_row = mdlite::MoveToAdjacentTableCell(
      source, *virtual_fourth.target_cell, false);
  Check(!next_row.changed && next_row.text == source && next_row.target_cell &&
            next_row.target_cell->row_begin == excess_row_begin &&
            next_row.target_cell->column == 0 && !next_row.target_cell->virtual_cell,
        "Tab leaves a short row only after its declared columns and skips overflow cells");
  if (next_row.target_cell) {
    const auto backward = mdlite::MoveToAdjacentTableCell(source, *next_row.target_cell, true);
    Check(!backward.changed && backward.target_cell &&
              backward.target_cell->row_begin == short_row_begin &&
              backward.target_cell->column == 3 && backward.target_cell->virtual_cell,
          "Shift+Tab from an excess row returns to the prior row's last declared virtual slot");
  }

  const auto header_last = mdlite::ResolveTableCellIntent(source, source.find(L"D"));
  const auto header_tab = header_last
      ? mdlite::MoveToAdjacentTableCell(source, *header_last, false)
      : mdlite::TableEditResult{};
  const auto header_down = header_last
      ? mdlite::MoveTableCaretTargetAtBoundary(source, *header_last,
                                                mdlite::TableCaretDirection::Down)
      : std::nullopt;
  Check(header_tab.target_cell && header_tab.target_cell->row_begin == short_row_begin &&
            header_tab.target_cell->column == 0 && header_down &&
            header_down->row_begin == short_row_begin && header_down->column == 3 &&
            header_down->virtual_cell,
        "Tab and Down from the header skip the delimiter and retain the target column intent");

  const auto short_virtual = mdlite::MoveTableCaretTargetAtBoundary(
      source, *virtual_fourth.target_cell, mdlite::TableCaretDirection::Down);
  const auto short_virtual_up = short_virtual
      ? mdlite::MoveTableCaretTargetAtBoundary(source, *short_virtual,
                                                mdlite::TableCaretDirection::Up)
      : std::nullopt;
  Check(short_virtual && short_virtual->row_begin == excess_row_begin &&
            short_virtual->column == 3 && !short_virtual->virtual_cell &&
            short_virtual_up && short_virtual_up->row_begin == short_row_begin &&
            short_virtual_up->column == 3 && short_virtual_up->virtual_cell,
        "Up and Down preserve the header column across excess and short rows");

  auto last_cell = mdlite::ResolveTableCellIntent(source, source.find(L"last"));
  std::optional<mdlite::TableCellIntent> last_column;
  if (last_cell) {
    auto step = mdlite::MoveToAdjacentTableCell(source, *last_cell, false);
    if (step.target_cell) step = mdlite::MoveToAdjacentTableCell(source, *step.target_cell, false);
    if (step.target_cell) step = mdlite::MoveToAdjacentTableCell(source, *step.target_cell, false);
    last_column = step.target_cell;
  }
  const auto appended_row = last_column
      ? mdlite::MoveToAdjacentTableCell(source, *last_column, false)
      : mdlite::TableEditResult{};
  const auto appended_table = appended_row.target_cell
      ? mdlite::ParseGfmTableAt(appended_row.text, appended_row.target_cell->row_begin)
      : std::nullopt;
  Check(last_column && last_column->row_begin == last_row_begin && last_column->column == 3 &&
            last_column->virtual_cell && appended_row.changed && appended_row.target_cell &&
            appended_row.target_cell->column == 0 &&
            appended_row.target_cell->row_begin > last_row_begin && appended_table &&
            appended_table->rows.back().cells.size() == 4 &&
            appended_row.text.starts_with(source) &&
            appended_row.text.ends_with(L"\n|  |  |  |  |"),
        "Tab creates a header-width row only after the final row's declared slots and preserves overflow source");
}

}  // namespace

int main() {
  TestDelimiterValidity();
  TestRangesAndVisualRows();
  TestSharedGfmTableParser();
  TestCachedTableCaretNavigation();
  TestMalformedFallback();
  TestNavigationAndEditing();
  TestColumnInsertionPreservesPadding();
  TestMissingTableCellInsertion();
  TestHeaderAnchoredNavigation();
  if (failures != 0) return 1;
  std::cout << "TableInteractionTests PASS\n";
  return 0;
}
