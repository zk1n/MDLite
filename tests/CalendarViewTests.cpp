#include "calendar/CalendarView.h"

#include <algorithm>
#include <array>
#include <iostream>

#include <windows.h>

namespace {

int failures = 0;
int checks = 0;

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

using mdlite::CalendarDate;

void TestMonthGrid() {
  const auto september = mdlite::GetCalendarViewMonthDates({2024, 9, 1});
  Check(september.size() == 42, "month grid always contains six rows of seven dates");
  Check(september[0] == CalendarDate{2024, 9, 1},
        "Sunday-first month starts in the first date cell");
  Check(september[6] == CalendarDate{2024, 9, 7},
        "Saturday occupies the seventh column");

  const auto february = mdlite::GetCalendarViewMonthDates({2021, 2, 15});
  Check(february[0] == CalendarDate{2021, 1, 31},
        "Monday-start month shows the previous Sunday in column one");
  Check(february[1] == CalendarDate{2021, 2, 1},
        "Monday-start month begins in the second column");
  Check(february[28] == CalendarDate{2021, 2, 28},
        "month dates retain row-major Sunday-to-Saturday ordering");

  const auto leap_day = mdlite::GetCalendarViewMonthDates({2024, 2, 1});
  Check(std::ranges::find(leap_day, std::optional<CalendarDate>{{2024, 2, 29}}) !=
            leap_day.end(),
        "leap day is included in the displayed month");
  Check(mdlite::GetCalendarViewMonthDates({2026, 2, 30})[0] == std::nullopt,
        "invalid month produces an empty grid");
}

void TestGeometryAndHitTesting() {
  const RECT client{0, 0, 707, 360};
  const auto geometry = mdlite::CalculateCalendarViewGeometry(client);
  Check(geometry.date_cells.size() == 42,
        "calendar hit area has six date rows and no week-number gutter");
  Check(geometry.date_cells[0].left == 0 && geometry.date_cells[0].right == 101,
        "first column begins at the client edge and uses one seventh of the grid");
  Check(geometry.date_cells[6].right == 707,
        "seventh column ends at the client edge");
  Check(geometry.date_cells[0].right == geometry.date_cells[1].left,
        "seven date columns are contiguous with no gutter");
  Check(geometry.previous_month_button.bottom == geometry.header.bottom &&
            geometry.next_month_button.right == client.right,
        "month navigation controls are contained in the header");
  Check(geometry.today_action.top == geometry.grid.bottom &&
            geometry.today_action.bottom == client.bottom,
        "today action occupies a footer below the date grid");

  const RECT first_cell = geometry.date_cells[0];
  const POINT center{(first_cell.left + first_cell.right) / 2,
                     (first_cell.top + first_cell.bottom) / 2};
  Check(mdlite::CalendarViewHitTestDate(geometry, center) == 0,
        "date hit testing maps the first cell to index zero");
  const POINT final_cell{geometry.date_cells[41].right - 1,
                         geometry.date_cells[41].bottom - 1};
  Check(mdlite::CalendarViewHitTestDate(geometry, final_cell) == 41,
        "date hit testing includes the bottom-right date cell");
  Check(!mdlite::CalendarViewHitTestDate(geometry, {client.right, client.bottom}),
        "footer action is outside the date hit-test grid");
}

void TestNavigationAndActivation() {
  mdlite::CalendarViewInteraction interaction({2024, 1, 31});
  Check(interaction.displayed_month() == CalendarDate{2024, 1, 1},
        "interaction state normalizes the displayed month to its first day");
  const auto next_month = interaction.NavigateMonth(1);
  Check(next_month.changed, "next-month navigation reports a month-change notification");
  Check(next_month.displayed_month == CalendarDate{2024, 2, 1} &&
            interaction.displayed_month() == next_month.displayed_month &&
            interaction.focused_date() == CalendarDate{2024, 2, 29},
        "month-change payload carries the normalized month and clamped focus date");
  Check(!interaction.selected_date(),
        "month navigation never changes selection or activates a date");

  Check(interaction.MoveFocusDays(-7), "keyboard week navigation moves focus by seven days");
  Check(interaction.focused_date() == CalendarDate{2024, 2, 22} &&
            interaction.displayed_month() == CalendarDate{2024, 2, 1},
        "keyboard navigation updates focus while preserving the displayed month");
  Check(!interaction.selected_date(),
        "keyboard arrows move focus without selecting or activating a date");

  interaction.SetHover(CalendarDate{2024, 2, 14});
  Check(interaction.hovered_date() == CalendarDate{2024, 2, 14} &&
            !interaction.selected_date(),
        "hover updates only hover state and cannot activate or select a date");

  const auto clicked = interaction.ClickDate({2024, 2, 14});
  Check(clicked.activated && clicked.selection_changed &&
            clicked.source == mdlite::CalendarViewActivationSource::Mouse &&
            clicked.date == CalendarDate{2024, 2, 14},
        "single click returns an explicit mouse activation for the selected date");
  const auto clicked_again = interaction.ClickDate({2024, 2, 14});
  Check(clicked_again.activated && !clicked_again.selection_changed,
        "clicking an already selected date still activates exactly that date");

  Check(interaction.MoveFocusDays(1), "keyboard navigation can move focus to the next date");
  const auto entered = interaction.ActivateFocusedDate();
  Check(entered.activated && entered.source == mdlite::CalendarViewActivationSource::Keyboard &&
            entered.date == CalendarDate{2024, 2, 15},
        "Enter activation identifies the focused date and keyboard source");

  const auto focus_before_today_change = interaction.focused_date();
  interaction.SetToday({2024, 2, 16});
  Check(interaction.focused_date() == focus_before_today_change &&
            interaction.selected_date() == CalendarDate{2024, 2, 15},
        "changing the today marker preserves keyboard focus and selection");

  const auto programmatic_month = interaction.SetDisplayedMonth({2024, 3, 31});
  Check(programmatic_month.changed &&
            programmatic_month.displayed_month == CalendarDate{2024, 3, 1},
        "programmatic month changes return the normalized month notification payload");
  Check(interaction.displayed_month() == CalendarDate{2024, 3, 1} &&
            interaction.focused_date() == CalendarDate{2024, 3, 15} &&
            interaction.selected_date() == CalendarDate{2024, 2, 15},
        "programmatic month changes normalize the month and move focus without activation");

  const auto previous_selection = interaction.selected_date();
  interaction.SetHover(std::nullopt);
  Check(interaction.selected_date() == previous_selection,
        "clearing hover does not change the selected date");
}

void TestHoverNotificationModel() {
  mdlite::CalendarViewInteraction interaction({2024, 2, 14});
  const auto entered = interaction.SetHover(CalendarDate{2024, 2, 15});
  Check(entered.changed && entered.date == CalendarDate{2024, 2, 15},
        "hover-date change carries the hovered date for notification");
  Check(!interaction.selected_date(), "hover notification does not select a date");

  const auto unchanged = interaction.SetHover(CalendarDate{2024, 2, 15});
  Check(!unchanged.changed && unchanged.date == CalendarDate{2024, 2, 15},
        "repeated hover over one date does not produce another change notification");

  const auto cleared = interaction.SetHover(std::nullopt);
  Check(cleared.changed && !cleared.date.has_value() && !interaction.hovered_date(),
        "clearing hover reports an explicit no-date notification state");
  Check(!interaction.selected_date(), "clearing hover still does not select a date");
}

void TestDateMarkers() {
  using Marker = mdlite::CalendarViewMarkerFlags;
  const std::array<mdlite::CalendarViewDateMarker, 3> markers{{
      {{2024, 2, 14}, Marker::Holiday},
      {{2024, 2, 14}, Marker::Daily},
      {{2024, 3, 2}, Marker::Daily},
  }};
  mdlite::CalendarViewInteraction interaction({2024, 2, 14});
  Check(interaction.SetMarkers(markers), "valid date markers are stored");
  const auto holiday_daily = interaction.MarkersForDate({2024, 2, 14});
  Check(mdlite::HasCalendarViewMarker(holiday_daily, Marker::Holiday) &&
            mdlite::HasCalendarViewMarker(holiday_daily, Marker::Daily),
        "duplicate date markers merge holiday and daily flags");
  Check(mdlite::HasCalendarViewMarker(interaction.MarkersForDate({2024, 2, 18}),
                                      Marker::Sunday),
        "Sunday marker is derived from the date");
  Check(mdlite::HasCalendarViewMarker(interaction.MarkersForDate({2024, 2, 17}),
                                      Marker::Saturday),
        "Saturday marker is derived from the date");

  interaction.SetDisplayedMonth({2024, 3, 1});
  const auto march_daily = interaction.MarkersForDate({2024, 3, 2});
  Check(mdlite::HasCalendarViewMarker(march_daily, Marker::Daily) &&
            mdlite::HasCalendarViewMarker(march_daily, Marker::Saturday),
        "daily and weekend marker state survives month navigation");
  Check(mdlite::HasCalendarViewMarker(interaction.MarkersForDate({2024, 2, 14}),
                                      Marker::Holiday),
        "stored holiday marker remains available after changing months");
}

void TestDateBoundaries() {
  mdlite::CalendarViewInteraction interaction({1601, 1, 1});
  Check(!interaction.MoveFocusDays(-1), "keyboard navigation stops before the supported date range");
  Check(!interaction.NavigateMonth(-1).changed,
        "month navigation stops before the supported date range");
  Check(!interaction.SetSelection(CalendarDate{2024, 2, 30}),
        "invalid date selection is rejected");
  Check(interaction.selected_date() == std::nullopt,
        "rejected selection leaves selection unchanged");
}

void TestNativeControlHoverPaint() {
  constexpr int width = 308;
  constexpr int height = 247;
  const HINSTANCE instance = GetModuleHandleW(nullptr);
  if (!mdlite::CalendarView_RegisterClass(instance)) {
    Check(false, "native calendar control class registers for paint verification");
    return;
  }
  HWND parent = CreateWindowExW(0, L"STATIC", L"", WS_POPUP,
                                0, 0, width, height, nullptr, nullptr,
                                instance, nullptr);
  if (!parent) {
    Check(false, "native calendar paint fixture parent is created");
    return;
  }
  const RECT bounds{0, 0, width, height};
  HWND calendar = mdlite::CalendarView_Create(parent, 9001, bounds, WS_CHILD);
  if (!calendar) {
    Check(false, "native calendar paint fixture control is created");
    DestroyWindow(parent);
    return;
  }

  mdlite::CalendarViewTheme theme;
  theme.background = RGB(42, 46, 54);
  theme.hover_background = RGB(50, 55, 64);
  theme.selected_background = RGB(49, 97, 153);
  mdlite::CalendarView_SetTheme(calendar, theme);
  mdlite::CalendarView_SetDisplayedMonth(calendar, {2026, 9, 1});
  mdlite::CalendarView_SetToday(calendar, {2026, 9, 25});
  mdlite::CalendarView_SetSelection(calendar, CalendarDate{2026, 9, 25});
  RECT client{};
  GetClientRect(calendar, &client);
  const auto geometry = mdlite::CalculateCalendarViewGeometry(client);

  HDC reference = GetDC(calendar);
  HDC buffer = reference ? CreateCompatibleDC(reference) : nullptr;
  HBITMAP bitmap = reference ? CreateCompatibleBitmap(reference, width, height) : nullptr;
  HGDIOBJ previous = buffer && bitmap ? SelectObject(buffer, bitmap) : nullptr;
  if (!reference || !buffer || !bitmap || !previous || previous == HGDI_ERROR) {
    Check(false, "offscreen bitmap is available for native calendar paint");
    if (previous && previous != HGDI_ERROR) SelectObject(buffer, previous);
    if (bitmap) DeleteObject(bitmap);
    if (buffer) DeleteDC(buffer);
    if (reference) ReleaseDC(calendar, reference);
    DestroyWindow(parent);
    return;
  }

  const auto render = [&] {
    SendMessageW(calendar, WM_PRINTCLIENT, reinterpret_cast<WPARAM>(buffer), 0);
  };
  const auto sample_cell = [&](std::size_t index) {
    const RECT cell = geometry.date_cells[index];
    return GetPixel(buffer, cell.left + 3, cell.top + 3);
  };
  render();
  const COLORREF baseline_first = sample_cell(0);
  const COLORREF baseline_second = sample_cell(1);
  Check(baseline_first == theme.background && baseline_second == theme.background,
        "non-hovered date cells paint the configured calendar surface");

  const auto first = geometry.date_cells[0];
  const POINT first_point{(first.left + first.right) / 2,
                          (first.top + first.bottom) / 2};
  SendMessageW(calendar, WM_MOUSEMOVE, 0,
               MAKELPARAM(first_point.x, first_point.y));
  render();
  Check(sample_cell(0) == theme.hover_background &&
            sample_cell(1) == theme.background,
        "hover paints only the date cell under the native mouse message");
  const auto selected_before_hover = mdlite::CalendarView_GetSelection(calendar);
  Check(selected_before_hover == CalendarDate{2026, 9, 25},
        "hover paint preserves the independently selected date");

  const auto second = geometry.date_cells[1];
  const POINT second_point{(second.left + second.right) / 2,
                           (second.top + second.bottom) / 2};
  SendMessageW(calendar, WM_MOUSEMOVE, 0,
               MAKELPARAM(second_point.x, second_point.y));
  render();
  Check(sample_cell(0) == theme.background &&
            sample_cell(1) == theme.hover_background,
        "moving hover invalidates the old cell and paints the new cell locally");
  SendMessageW(calendar, WM_MOUSELEAVE, 0, 0);
  render();
  Check(sample_cell(0) == theme.background &&
            sample_cell(1) == theme.background,
        "leaving the calendar restores both cells without changing selection");
  Check(mdlite::CalendarView_GetSelection(calendar) == selected_before_hover,
        "native hover and leave do not activate or change the selected date");

  SelectObject(buffer, previous);
  DeleteObject(bitmap);
  DeleteDC(buffer);
  ReleaseDC(calendar, reference);
  DestroyWindow(parent);
}

}  // namespace

int wmain() {
  TestMonthGrid();
  TestGeometryAndHitTesting();
  TestNavigationAndActivation();
  TestHoverNotificationModel();
  TestDateMarkers();
  TestDateBoundaries();
  TestNativeControlHoverPaint();
  if (failures == 0) {
    std::cout << "All " << checks << " calendar view checks passed.\n";
  }
  return failures == 0 ? 0 : 1;
}
