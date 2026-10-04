#include "calendar/CalendarView.h"

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <string>
#include <utility>

#include <windowsx.h>

namespace mdlite {
namespace {

constexpr wchar_t kCalendarViewClassName[] = L"MDLite.CalendarView";

bool SameMonth(CalendarDate left, CalendarDate right) noexcept {
  return left.year == right.year && left.month == right.month;
}

int DaysInMonth(int year, int month) noexcept {
  const std::chrono::year_month_day_last last{
      std::chrono::year{year}, std::chrono::month_day_last{
                                    std::chrono::month{static_cast<unsigned>(month)}}};
  return static_cast<int>(static_cast<unsigned>(last.day()));
}

CalendarDate LocalToday() noexcept {
  SYSTEMTIME local{};
  GetLocalTime(&local);
  return {static_cast<int>(local.wYear), static_cast<int>(local.wMonth),
          static_cast<int>(local.wDay)};
}

bool SyntheticMouseTestSuppressesLeaveTracking() noexcept {
  static const bool enabled_by_test = [] {
    wchar_t enabled[2]{};
    return GetEnvironmentVariableW(L"MDLITE_TEST_SYNTHETIC_MOUSE_NO_LEAVE",
                                   enabled, 2) == 1 &&
           enabled[0] == L'1';
  }();
  return enabled_by_test;
}

struct CalendarViewWindowState {
  explicit CalendarViewWindowState(CalendarDate initial_today) noexcept
      : interaction(initial_today) {}

  CalendarViewTheme theme;
  HFONT font{};
  CalendarViewInteraction interaction;
  bool tracking_mouse{};
  bool suppress_mouse_leave_tracking_for_test{};
  int hovered_navigation{};
  bool hovered_today_action{};
};

CalendarViewWindowState* State(HWND view) noexcept {
  return reinterpret_cast<CalendarViewWindowState*>(
      GetWindowLongPtrW(view, GWLP_USERDATA));
}

bool RectsIntersect(const RECT& left, const RECT& right) noexcept {
  RECT overlap{};
  return IntersectRect(&overlap, &left, &right) != FALSE;
}

RECT ClientRect(HWND view) noexcept {
  RECT result{};
  GetClientRect(view, &result);
  return result;
}

void InvalidateRectWithoutErase(HWND view, const RECT& rect) noexcept {
  if (rect.right > rect.left && rect.bottom > rect.top) {
    InvalidateRect(view, &rect, FALSE);
  }
}

void InvalidateVisibleDate(HWND view, CalendarDate date) noexcept {
  const auto month = CalendarView_GetDisplayedMonth(view);
  if (!month) return;
  const auto dates = GetCalendarViewMonthDates(*month);
  const auto found = std::ranges::find(dates, std::optional<CalendarDate>{date});
  if (found == dates.end()) return;
  const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
  const auto index = static_cast<std::size_t>(found - dates.begin());
  InvalidateRectWithoutErase(view, geometry.date_cells[index]);
}

void InvalidateMonthBody(HWND view) noexcept {
  const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
  InvalidateRectWithoutErase(view, geometry.header);
  InvalidateRectWithoutErase(view, geometry.weekdays);
  InvalidateRectWithoutErase(view, geometry.grid);
}

void FillColor(HDC dc, const RECT& rect, COLORREF color) noexcept {
  if (rect.right <= rect.left || rect.bottom <= rect.top) return;
  SetDCBrushColor(dc, color);
  FillRect(dc, &rect, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
}

void FrameColor(HDC dc, RECT rect, COLORREF color, int inset) noexcept {
  InflateRect(&rect, -inset, -inset);
  if (rect.right <= rect.left || rect.bottom <= rect.top) return;
  SetDCBrushColor(dc, color);
  FrameRect(dc, &rect, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
}

void DrawCenteredText(HDC dc, const RECT& rect, const std::wstring& text,
                      COLORREF color) noexcept {
  if (rect.right <= rect.left || rect.bottom <= rect.top) return;
  SetTextColor(dc, color);
  SetBkMode(dc, TRANSPARENT);
  RECT text_rect = rect;
  DrawTextW(dc, text.c_str(), static_cast<int>(text.size()), &text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
}

void NotifyActivated(HWND view, const CalendarViewInteractionResult& result) noexcept {
  if (!result.activated) return;
  CalendarViewDateActivatedNotification notification{};
  notification.header.hwndFrom = view;
  notification.header.idFrom = static_cast<UINT_PTR>(GetDlgCtrlID(view));
  notification.header.code = CVN_DATE_ACTIVATED;
  notification.date = result.date;
  notification.source = result.source;
  SendMessageW(GetParent(view), WM_NOTIFY, notification.header.idFrom,
               reinterpret_cast<LPARAM>(&notification));
}

void NotifyHovered(HWND view, std::optional<CalendarDate> date) noexcept {
  CalendarViewDateHoveredNotification notification{};
  notification.header.hwndFrom = view;
  notification.header.idFrom = static_cast<UINT_PTR>(GetDlgCtrlID(view));
  notification.header.code = CVN_DATE_HOVERED;
  notification.has_date = date.has_value();
  if (date) notification.date = *date;
  SendMessageW(GetParent(view), WM_NOTIFY, notification.header.idFrom,
               reinterpret_cast<LPARAM>(&notification));
}

void NotifyMonthChanged(HWND view, CalendarDate month) noexcept {
  CalendarViewMonthChangedNotification notification{};
  notification.header.hwndFrom = view;
  notification.header.idFrom = static_cast<UINT_PTR>(GetDlgCtrlID(view));
  notification.header.code = CVN_MONTH_CHANGED;
  notification.displayed_month = {month.year, month.month, 1};
  SendMessageW(GetParent(view), WM_NOTIFY, notification.header.idFrom,
               reinterpret_cast<LPARAM>(&notification));
}

void NotifyDateSelected(HWND view, CalendarDate date) noexcept {
  CalendarViewDateSelectedNotification notification{};
  notification.header.hwndFrom = view;
  notification.header.idFrom = static_cast<UINT_PTR>(GetDlgCtrlID(view));
  notification.header.code = CVN_DATE_SELECTED;
  notification.date = date;
  SendMessageW(GetParent(view), WM_NOTIFY, notification.header.idFrom,
               reinterpret_cast<LPARAM>(&notification));
}

void DrawMarker(HDC dc, const RECT& cell, COLORREF color, int position, int count, UINT dpi) noexcept {
  const int kMarkerSize = std::max(1, MulDiv(3, static_cast<int>(dpi), 96));
  const int kMarkerGap = std::max(1, MulDiv(2, static_cast<int>(dpi), 96));
  const int total_width = count * kMarkerSize + std::max(0, count - 1) * kMarkerGap;
  const int left = (cell.left + cell.right - total_width) / 2 +
                   position * (kMarkerSize + kMarkerGap);
  const int bottom = cell.bottom - 2;
  FillColor(dc, {left, bottom - kMarkerSize, left + kMarkerSize, bottom}, color);
}

void DrawView(HWND view, CalendarViewWindowState& state, const PAINTSTRUCT& paint) {
  HDC dc = paint.hdc;
  const UINT dpi = GetDpiForWindow(view);
  const auto dip = [dpi](int value) { return MulDiv(value, static_cast<int>(dpi), 96); };
  const int saved = SaveDC(dc);
  SelectObject(dc, state.font ? state.font : GetStockObject(DEFAULT_GUI_FONT));
  const auto client = ClientRect(view);
  const auto geometry = CalculateCalendarViewGeometry(client, GetDpiForWindow(view));
  const auto month = state.interaction.displayed_month();
  const auto dates = GetCalendarViewMonthDates(month);

  FillColor(dc, paint.rcPaint, state.theme.background);

  if (RectsIntersect(geometry.header, paint.rcPaint)) {
    const auto title = std::to_wstring(month.year) + L"年 " +
                       std::to_wstring(month.month) + L"月";
    RECT title_rect = geometry.header;
    title_rect.left = geometry.previous_month_button.right;
    title_rect.right = geometry.next_month_button.left;
    DrawCenteredText(dc, title_rect, title, state.theme.heading_text);
    if (state.hovered_navigation < 0) {
      FillColor(dc, geometry.previous_month_button,
                state.theme.navigation_hover_background);
    }
    if (state.hovered_navigation > 0) {
      FillColor(dc, geometry.next_month_button,
                state.theme.navigation_hover_background);
    }
    DrawCenteredText(dc, geometry.previous_month_button, L"‹",
                     state.theme.heading_text);
    DrawCenteredText(dc, geometry.next_month_button, L"›",
                     state.theme.heading_text);
  }

  static constexpr wchar_t kWeekdayNames[] = L"日月火水木金土";
  if (RectsIntersect(geometry.weekdays, paint.rcPaint)) {
    for (std::size_t column = 0; column < 7; ++column) {
      const int width = geometry.weekdays.right - geometry.weekdays.left;
      RECT weekday_cell{
          geometry.weekdays.left + static_cast<int>((static_cast<std::int64_t>(width) * column) / 7),
          geometry.weekdays.top,
          geometry.weekdays.left + static_cast<int>((static_cast<std::int64_t>(width) * (column + 1)) / 7),
          geometry.weekdays.bottom};
      if (RectsIntersect(weekday_cell, paint.rcPaint)) {
        const std::wstring name(1, kWeekdayNames[column]);
        DrawCenteredText(dc, weekday_cell, name, state.theme.adjacent_month_text);
      }
    }
  }

  const auto selected = state.interaction.selected_date();
  const auto hovered = state.interaction.hovered_date();
  const auto focused = state.interaction.focused_date();
  const bool has_focus = GetFocus() == view;
  for (std::size_t index = 0; index < dates.size(); ++index) {
    if (!dates[index] || !RectsIntersect(geometry.date_cells[index], paint.rcPaint)) continue;
    const auto date = *dates[index];
    const bool is_selected = selected && *selected == date;
    const bool is_hovered = hovered && *hovered == date;
    const bool is_today = state.interaction.today() == date;
    const bool is_focused = has_focus && focused && *focused == date;
    const auto marker_flags = state.interaction.MarkersForDate(date);
    const bool is_holiday = HasCalendarViewMarker(
        marker_flags, CalendarViewMarkerFlags::Holiday);
    const bool is_daily = HasCalendarViewMarker(marker_flags, CalendarViewMarkerFlags::Daily);
    const bool is_sunday = HasCalendarViewMarker(marker_flags, CalendarViewMarkerFlags::Sunday);
    const bool is_saturday = HasCalendarViewMarker(marker_flags, CalendarViewMarkerFlags::Saturday);

    if (is_selected) {
      FillColor(dc, geometry.date_cells[index], state.theme.selected_background);
    } else if (is_hovered) {
      FillColor(dc, geometry.date_cells[index], state.theme.hover_background);
    }

    const bool in_month = SameMonth(month, date);
    const COLORREF day_color =
        is_selected ? state.theme.selected_text
                    : (!in_month ? state.theme.adjacent_month_text
                                 : (is_holiday ? state.theme.holiday_text
                                               : (is_sunday ? state.theme.sunday_text
                                                            : (is_saturday ? state.theme.saturday_text
                                                                           : state.theme.day_text))));
    DrawCenteredText(dc, geometry.date_cells[index], std::to_wstring(date.day), day_color);
    const int marker_count = static_cast<int>(is_holiday) + static_cast<int>(is_daily);
    if (is_holiday) DrawMarker(dc, geometry.date_cells[index], state.theme.holiday_marker,
                               0, marker_count, dpi);
    if (is_daily) DrawMarker(dc, geometry.date_cells[index], state.theme.daily_marker,
                             is_holiday ? 1 : 0, marker_count, dpi);
    if (is_hovered && is_selected) {
      FrameColor(dc, geometry.date_cells[index], state.theme.hover_background, 4);
    }
    if (is_today) FrameColor(dc, geometry.date_cells[index], state.theme.today_outline, 2);
    if (is_focused) FrameColor(dc, geometry.date_cells[index], state.theme.focus_outline, 6);
  }

  if (RectsIntersect(geometry.today_action, paint.rcPaint)) {
    if (state.hovered_today_action) {
      RECT hover = geometry.today_action;
      InflateRect(&hover, -4, -2);
      FillColor(dc, hover, state.theme.hover_background);
    }
    const int icon_left = geometry.today_action.left + dip(12);
    const int icon_top = geometry.today_action.top +
                         ((geometry.today_action.bottom - geometry.today_action.top) - dip(12)) / 2;
    const RECT icon{icon_left, icon_top, icon_left + dip(12), icon_top + dip(12)};
    FrameColor(dc, icon, state.theme.today_outline, 0);
    FillColor(dc, {icon_left + dip(2), icon_top + dip(4), icon_left + dip(10), icon_top + dip(5)},
              state.theme.today_outline);
    const CalendarDate today = state.interaction.today();
    wchar_t label[48]{};
    swprintf_s(label, L"今日: %04d/%02d/%02d", today.year, today.month, today.day);
    SetTextColor(dc, state.theme.heading_text);
    SetBkMode(dc, TRANSPARENT);
    RECT text_rect{icon_left + dip(22), geometry.today_action.top,
                   geometry.today_action.right - dip(8), geometry.today_action.bottom};
    DrawTextW(dc, label, -1, &text_rect,
              DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  }
  if (saved) RestoreDC(dc, saved);
}

void SetHoveredDate(HWND view, CalendarViewWindowState& state,
                    std::optional<CalendarDate> date) noexcept {
  const auto old = state.interaction.hovered_date();
  const auto change = state.interaction.SetHover(date);
  if (!change.changed) return;
  if (old) InvalidateVisibleDate(view, *old);
  if (change.date) InvalidateVisibleDate(view, *change.date);
  NotifyHovered(view, change.date);
}

void SetHoveredNavigation(HWND view, CalendarViewWindowState& state, int navigation) noexcept {
  navigation = std::clamp(navigation, -1, 1);
  if (state.hovered_navigation == navigation) return;
  const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
  if (state.hovered_navigation < 0) {
    InvalidateRectWithoutErase(view, geometry.previous_month_button);
  } else if (state.hovered_navigation > 0) {
    InvalidateRectWithoutErase(view, geometry.next_month_button);
  }
  state.hovered_navigation = navigation;
  if (navigation < 0) InvalidateRectWithoutErase(view, geometry.previous_month_button);
  if (navigation > 0) InvalidateRectWithoutErase(view, geometry.next_month_button);
}

void MoveDisplayedMonth(HWND view, CalendarViewWindowState& state, int months) noexcept {
  const auto change = state.interaction.NavigateMonth(months);
  if (!change.changed) return;
  InvalidateMonthBody(view);
  NotifyMonthChanged(view, change.displayed_month);
}

void SelectToday(HWND view, CalendarViewWindowState& state) noexcept {
  const CalendarDate today = state.interaction.today();
  const auto old_month = state.interaction.displayed_month();
  const auto old_selection = state.interaction.selected_date();
  const auto old_focus = state.interaction.focused_date();
  if (!state.interaction.SetSelection(today)) return;
  if (!SameMonth(old_month, state.interaction.displayed_month())) {
    InvalidateMonthBody(view);
    NotifyMonthChanged(view, state.interaction.displayed_month());
  } else {
    if (old_selection && *old_selection != today) InvalidateVisibleDate(view, *old_selection);
    if (old_focus && *old_focus != today && old_focus != old_selection)
      InvalidateVisibleDate(view, *old_focus);
    InvalidateVisibleDate(view, today);
  }
  if (old_selection != std::optional<CalendarDate>{today}) NotifyDateSelected(view, today);
  InvalidateRectWithoutErase(view, CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view)).today_action);
}

LRESULT CALLBACK CalendarViewWindowProc(HWND view, UINT message, WPARAM wparam,
                                         LPARAM lparam) {
  auto* state = State(view);
  switch (message) {
    case WM_NCCREATE: {
      const auto* create = reinterpret_cast<const CREATESTRUCTW*>(lparam);
      auto* created_state = static_cast<CalendarViewWindowState*>(create->lpCreateParams);
      if (!created_state) return FALSE;
      created_state->suppress_mouse_leave_tracking_for_test =
          SyntheticMouseTestSuppressesLeaveTracking();
      SetWindowLongPtrW(view, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(created_state));
      return TRUE;
    }
    case WM_ERASEBKGND:
      return TRUE;
    case WM_PAINT: {
      PAINTSTRUCT paint{};
      BeginPaint(view, &paint);
      if (state) DrawView(view, *state, paint);
      EndPaint(view, &paint);
      return 0;
    }
    case WM_PRINTCLIENT: {
      if (!state || wparam == 0) return 0;
      PAINTSTRUCT paint{};
      paint.hdc = reinterpret_cast<HDC>(wparam);
      if (GetClipBox(paint.hdc, &paint.rcPaint) == ERROR ||
          paint.rcPaint.right <= paint.rcPaint.left ||
          paint.rcPaint.bottom <= paint.rcPaint.top) {
        GetClientRect(view, &paint.rcPaint);
      }
      const int saved_state = SaveDC(paint.hdc);
      DrawView(view, *state, paint);
      if (saved_state != 0) RestoreDC(paint.hdc, saved_state);
      return 0;
    }
    case WM_SIZE:
      InvalidateRect(view, nullptr, FALSE);
      return 0;
    case WM_MOUSEMOVE: {
      if (!state) break;
      if (!state->tracking_mouse && !state->suppress_mouse_leave_tracking_for_test) {
        TRACKMOUSEEVENT tracking{sizeof(TRACKMOUSEEVENT), TME_LEAVE, view, 0};
        state->tracking_mouse = TrackMouseEvent(&tracking) != FALSE;
      }
      const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
      const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
      const bool hovering_today = PtInRect(&geometry.today_action, point) != FALSE;
      if (state->hovered_today_action != hovering_today) {
        state->hovered_today_action = hovering_today;
        InvalidateRectWithoutErase(view, geometry.today_action);
      }
      int navigation = 0;
      if (PtInRect(&geometry.previous_month_button, point)) navigation = -1;
      if (PtInRect(&geometry.next_month_button, point)) navigation = 1;
      SetHoveredNavigation(view, *state, navigation);
      std::optional<CalendarDate> hovered;
      if (const auto index = CalendarViewHitTestDate(geometry, point)) {
        hovered = GetCalendarViewMonthDates(state->interaction.displayed_month())[*index];
      }
      SetHoveredDate(view, *state, hovered);
      return 0;
    }
    case WM_MOUSELEAVE:
      if (state) {
        state->tracking_mouse = false;
        if (state->hovered_today_action) {
          state->hovered_today_action = false;
          InvalidateRectWithoutErase(view,
                                    CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view)).today_action);
        }
        SetHoveredDate(view, *state, std::nullopt);
        SetHoveredNavigation(view, *state, 0);
      }
      return 0;
    case WM_LBUTTONDOWN:
      SetFocus(view);
      SetCapture(view);
      return 0;
    case WM_LBUTTONUP: {
      if (!state) break;
      if (GetCapture() == view) ReleaseCapture();
      const POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
      const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
      if (PtInRect(&geometry.today_action, point)) {
        SelectToday(view, *state);
        return 0;
      }
      if (PtInRect(&geometry.previous_month_button, point)) {
        MoveDisplayedMonth(view, *state, -1);
        return 0;
      }
      if (PtInRect(&geometry.next_month_button, point)) {
        MoveDisplayedMonth(view, *state, 1);
        return 0;
      }
      if (const auto index = CalendarViewHitTestDate(geometry, point)) {
        const auto date = GetCalendarViewMonthDates(
            state->interaction.displayed_month())[*index];
        if (date) {
          const bool month_changes = !SameMonth(state->interaction.displayed_month(), *date);
          const auto old_selection = state->interaction.selected_date();
          const auto old_focus = state->interaction.focused_date();
          const auto result = state->interaction.ClickDate(*date);
          if (month_changes) InvalidateMonthBody(view);
          else {
            if (old_selection && *old_selection != result.date)
              InvalidateVisibleDate(view, *old_selection);
            if (old_focus && *old_focus != result.date && old_focus != old_selection)
              InvalidateVisibleDate(view, *old_focus);
            InvalidateVisibleDate(view, result.date);
          }
          if (month_changes) {
            NotifyMonthChanged(view, state->interaction.displayed_month());
          }
          NotifyActivated(view, result);
        }
      }
      return 0;
    }
    case WM_SETFONT:
      if (state) {
        state->font = reinterpret_cast<HFONT>(wparam);
        if (lparam) InvalidateRect(view, nullptr, FALSE);
      }
      return 0;
    case WM_GETFONT:
      return state ? reinterpret_cast<LRESULT>(state->font) : 0;
    case WM_SETFOCUS:
      if (state) {
        const auto date = state->interaction.focused_date();
        if (date) InvalidateVisibleDate(view, *date);
      }
      return 0;
    case WM_KILLFOCUS:
      if (state) {
        const auto date = state->interaction.focused_date();
        if (date) InvalidateVisibleDate(view, *date);
      }
      return 0;
    case WM_GETDLGCODE:
      return DLGC_WANTARROWS | DLGC_WANTCHARS;
    case WM_KEYDOWN: {
      if (!state) break;
      if (wparam == VK_RETURN) {
        const auto old_selection = state->interaction.selected_date();
        const auto result = state->interaction.ActivateFocusedDate();
        if (old_selection && *old_selection != result.date) {
          InvalidateVisibleDate(view, *old_selection);
        }
        InvalidateVisibleDate(view, result.date);
        NotifyActivated(view, result);
        return 0;
      }
      if (wparam == VK_PRIOR || wparam == VK_NEXT) {
        MoveDisplayedMonth(view, *state, wparam == VK_PRIOR ? -1 : 1);
        return 0;
      }
      int days = 0;
      if (wparam == VK_LEFT) days = -1;
      if (wparam == VK_RIGHT) days = 1;
      if (wparam == VK_UP) days = -7;
      if (wparam == VK_DOWN) days = 7;
      if (days != 0) {
        const auto old_focus = state->interaction.focused_date();
        const auto old_month = state->interaction.displayed_month();
        if (state->interaction.MoveFocusDays(days)) {
          if (!SameMonth(old_month, state->interaction.displayed_month())) {
            const auto new_month = state->interaction.displayed_month();
            InvalidateMonthBody(view);
            NotifyMonthChanged(view, new_month);
          } else {
            if (old_focus) InvalidateVisibleDate(view, *old_focus);
            if (const auto new_focus = state->interaction.focused_date()) {
              InvalidateVisibleDate(view, *new_focus);
            }
          }
        }
        return 0;
      }
      break;
    }
    case WM_NCDESTROY:
      SetWindowLongPtrW(view, GWLP_USERDATA, 0);
      delete state;
      break;
  }
  return DefWindowProcW(view, message, wparam, lparam);
}

}  // namespace

CalendarViewGeometry CalculateCalendarViewGeometry(RECT client, UINT dpi) noexcept {
  const auto dip = [dpi](int value) { return MulDiv(value, static_cast<int>(dpi ? dpi : 96), 96); };
  CalendarViewGeometry geometry{};
  const int width = std::max(0, static_cast<int>(client.right - client.left));
  const int height = std::max(0, static_cast<int>(client.bottom - client.top));
  const int header_height = std::min(height, std::max(dip(24), std::min(dip(36), height / 5)));
  const int header_bottom = client.top + header_height;
  const int weekday_height = std::min(height - header_height,
                                      std::max(dip(20), std::min(dip(24), (height - header_height) / 8)));
  const int grid_top = header_bottom + weekday_height;
  const int footer_height = std::min(std::max(0, height - grid_top),
                                    std::max(dip(24), std::min(dip(32), height / 9)));
  const int grid_bottom = std::max(grid_top, static_cast<int>(client.bottom) - footer_height);
  const int nav_width = std::min(dip(44), width / 5);
  geometry.header = {client.left, client.top, client.right, header_bottom};
  geometry.previous_month_button = {client.left, client.top, client.left + nav_width,
                                    header_bottom};
  geometry.next_month_button = {client.right - nav_width, client.top, client.right,
                                header_bottom};
  geometry.weekdays = {client.left, header_bottom, client.right, grid_top};
  geometry.grid = {client.left, grid_top, client.right, grid_bottom};
  geometry.today_action = {client.left, grid_bottom, client.right, client.bottom};
  const int grid_width =
      std::max(0, static_cast<int>(geometry.grid.right - geometry.grid.left));
  const int grid_height =
      std::max(0, static_cast<int>(geometry.grid.bottom - geometry.grid.top));
  for (std::size_t row = 0; row < 6; ++row) {
    const int top = geometry.grid.top + static_cast<int>(
        (static_cast<std::int64_t>(grid_height) * row) / 6);
    const int bottom = geometry.grid.top + static_cast<int>(
        (static_cast<std::int64_t>(grid_height) * (row + 1)) / 6);
    for (std::size_t column = 0; column < 7; ++column) {
      const int left = geometry.grid.left + static_cast<int>(
          (static_cast<std::int64_t>(grid_width) * column) / 7);
      const int right = geometry.grid.left + static_cast<int>(
          (static_cast<std::int64_t>(grid_width) * (column + 1)) / 7);
      geometry.date_cells[row * 7 + column] = {left, top, right, bottom};
    }
  }
  return geometry;
}

CalendarViewMonthDates GetCalendarViewMonthDates(CalendarDate displayed_month) noexcept {
  CalendarViewMonthDates dates{};
  if (!IsValidCalendarDate(displayed_month)) return dates;
  const std::chrono::year_month_day first_of_month{
      std::chrono::year{displayed_month.year},
      std::chrono::month{static_cast<unsigned>(displayed_month.month)}, std::chrono::day{1}};
  const auto first_day = std::chrono::sys_days{first_of_month};
  const auto sunday_offset = static_cast<int>(std::chrono::weekday{first_day}.c_encoding());
  const auto grid_start = first_day - std::chrono::days{sunday_offset};
  for (std::size_t index = 0; index < dates.size(); ++index) {
    const std::chrono::year_month_day cell_date{
        grid_start + std::chrono::days{static_cast<int>(index)}};
    if (!cell_date.ok()) continue;
    const CalendarDate date{static_cast<int>(cell_date.year()),
                            static_cast<int>(static_cast<unsigned>(cell_date.month())),
                            static_cast<int>(static_cast<unsigned>(cell_date.day()))};
    if (IsValidCalendarDate(date)) dates[index] = date;
  }
  return dates;
}

std::optional<std::size_t> CalendarViewHitTestDate(
    const CalendarViewGeometry& geometry, POINT point) noexcept {
  const int width = geometry.grid.right - geometry.grid.left;
  const int height = geometry.grid.bottom - geometry.grid.top;
  if (width <= 0 || height <= 0 || point.x < geometry.grid.left ||
      point.x >= geometry.grid.right || point.y < geometry.grid.top ||
      point.y >= geometry.grid.bottom) {
    return std::nullopt;
  }
  const auto column = static_cast<std::size_t>(
      (static_cast<std::int64_t>(point.x - geometry.grid.left) * 7) / width);
  const auto row = static_cast<std::size_t>(
      (static_cast<std::int64_t>(point.y - geometry.grid.top) * 6) / height);
  if (column >= 7 || row >= 6) return std::nullopt;
  return row * 7 + column;
}

CalendarViewInteraction::CalendarViewInteraction(CalendarDate today) noexcept
    : displayed_month_(IsValidCalendarDate(today) ? CalendarDate{today.year, today.month, 1}
                                                  : CalendarDate{2000, 1, 1}),
      today_(IsValidCalendarDate(today) ? today : CalendarDate{2000, 1, 1}),
      focused_date_(IsValidCalendarDate(today) ? today : CalendarDate{2000, 1, 1}) {}

void CalendarViewInteraction::SetToday(CalendarDate date) noexcept {
  if (!IsValidCalendarDate(date)) return;
  today_ = date;
}

CalendarViewMonthChange CalendarViewInteraction::SetDisplayedMonth(CalendarDate date) noexcept {
  if (!IsValidCalendarDate(date)) return {false, displayed_month_};
  const bool changed = !SameMonth(displayed_month_, date);
  displayed_month_ = {date.year, date.month, 1};
  const CalendarDate focus = focused_date_.value_or(selected_date_.value_or(today_));
  focused_date_ = CalendarDate{date.year, date.month,
                               std::min(focus.day, DaysInMonth(date.year, date.month))};
  return {changed, displayed_month_};
}

bool CalendarViewInteraction::SetSelection(std::optional<CalendarDate> date) noexcept {
  if (date && !IsValidCalendarDate(*date)) return false;
  selected_date_ = date;
  if (date) {
    focused_date_ = date;
    SetDisplayedMonth(*date);
  }
  return true;
}

CalendarViewHoverChange CalendarViewInteraction::SetHover(
    std::optional<CalendarDate> date) noexcept {
  if (date && !IsValidCalendarDate(*date)) date.reset();
  if (hovered_date_ == date) return {false, date};
  hovered_date_ = date;
  return {true, date};
}

bool CalendarViewInteraction::SetMarkers(
    std::span<const CalendarViewDateMarker> markers) noexcept {
  try {
    std::vector<CalendarViewDateMarker> updated;
    for (const auto& marker : markers) {
      if (!IsValidCalendarDate(marker.date)) continue;
      const auto flags = static_cast<CalendarViewMarkerFlags>(
          static_cast<std::uint8_t>(marker.flags) & 0x0f);
      if (flags == CalendarViewMarkerFlags::None) continue;
      const auto found = std::ranges::find_if(updated, [&](const auto& value) {
        return value.date == marker.date;
      });
      if (found == updated.end()) {
        updated.push_back({marker.date, flags});
      } else {
        found->flags = found->flags | flags;
      }
    }
    markers_.swap(updated);
    return true;
  } catch (...) {
    return false;
  }
}

CalendarViewMarkerFlags CalendarViewInteraction::MarkersForDate(
    CalendarDate date) const noexcept {
  if (!IsValidCalendarDate(date)) return CalendarViewMarkerFlags::None;
  CalendarViewMarkerFlags flags = CalendarViewMarkerFlags::None;
  const auto found = std::ranges::find_if(markers_, [&](const auto& marker) {
    return marker.date == date;
  });
  if (found != markers_.end()) flags = found->flags;
  const std::chrono::year_month_day value{
      std::chrono::year{date.year}, std::chrono::month{static_cast<unsigned>(date.month)},
      std::chrono::day{static_cast<unsigned>(date.day)}};
  switch (std::chrono::weekday{std::chrono::sys_days{value}}.c_encoding()) {
    case 0: flags = flags | CalendarViewMarkerFlags::Sunday; break;
    case 6: flags = flags | CalendarViewMarkerFlags::Saturday; break;
  }
  return flags;
}

bool CalendarViewInteraction::MoveFocusDays(int days) noexcept {
  if (days < -3660000 || days > 3660000) return false;
  const CalendarDate start = focused_date_.value_or(selected_date_.value_or(today_));
  const std::chrono::year_month_day start_date{
      std::chrono::year{start.year},
      std::chrono::month{static_cast<unsigned>(start.month)},
      std::chrono::day{static_cast<unsigned>(start.day)}};
  const std::chrono::year_month_day moved{
      std::chrono::sys_days{start_date} + std::chrono::days{days}};
  if (!moved.ok()) return false;
  const CalendarDate result{static_cast<int>(moved.year()),
                            static_cast<int>(static_cast<unsigned>(moved.month())),
                            static_cast<int>(static_cast<unsigned>(moved.day()))};
  if (!IsValidCalendarDate(result)) return false;
  focused_date_ = result;
  SetDisplayedMonth(result);
  return true;
}

CalendarViewMonthChange CalendarViewInteraction::NavigateMonth(int months) noexcept {
  if (months < -120000 || months > 120000) return {false, displayed_month_};
  const CalendarDate start = focused_date_.value_or(selected_date_.value_or(today_));
  const std::int64_t absolute_month =
      static_cast<std::int64_t>(displayed_month_.year) * 12 +
      static_cast<std::int64_t>(displayed_month_.month - 1) + months;
  const int target_year = static_cast<int>(absolute_month / 12);
  const int target_month = static_cast<int>(absolute_month % 12) + 1;
  if (target_year < 1601 || target_year > 9999) return {false, displayed_month_};
  const int target_day = std::min(start.day, DaysInMonth(target_year, target_month));
  const CalendarDate target{target_year, target_month, target_day};
  if (!IsValidCalendarDate(target)) return {false, displayed_month_};
  const bool changed = !SameMonth(displayed_month_, target);
  displayed_month_ = {target.year, target.month, 1};
  focused_date_ = target;
  return {changed, displayed_month_};
}

CalendarViewInteractionResult CalendarViewInteraction::ClickDate(CalendarDate date) noexcept {
  if (!IsValidCalendarDate(date)) return {};
  const bool changed = selected_date_ != date;
  selected_date_ = date;
  focused_date_ = date;
  SetDisplayedMonth(date);
  return {changed, true, date, CalendarViewActivationSource::Mouse};
}

CalendarViewInteractionResult CalendarViewInteraction::ActivateFocusedDate() noexcept {
  const CalendarDate date = focused_date_.value_or(selected_date_.value_or(today_));
  if (!IsValidCalendarDate(date)) return {};
  const bool changed = selected_date_ != date;
  selected_date_ = date;
  focused_date_ = date;
  return {changed, true, date, CalendarViewActivationSource::Keyboard};
}

bool CalendarView_RegisterClass(HINSTANCE instance) noexcept {
  if (!instance) instance = GetModuleHandleW(nullptr);
  WNDCLASSEXW window_class{sizeof(WNDCLASSEXW)};
  window_class.style = 0;
  window_class.lpfnWndProc = CalendarViewWindowProc;
  window_class.hInstance = instance;
  window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  window_class.lpszClassName = kCalendarViewClassName;
  if (RegisterClassExW(&window_class) != 0) return true;
  return GetLastError() == ERROR_CLASS_ALREADY_EXISTS;
}

HWND CalendarView_Create(HWND parent, int control_id, const RECT& bounds,
                          DWORD style) noexcept {
  if (!parent || !CalendarView_RegisterClass(GetModuleHandleW(nullptr))) return nullptr;
  auto* state = new (std::nothrow) CalendarViewWindowState(LocalToday());
  if (!state) return nullptr;
  HWND view = CreateWindowExW(0, kCalendarViewClassName, L"", style, bounds.left,
                              bounds.top, bounds.right - bounds.left,
                              bounds.bottom - bounds.top, parent,
                              reinterpret_cast<HMENU>(static_cast<INT_PTR>(control_id)),
                              GetModuleHandleW(nullptr), state);
  if (!view) delete state;
  return view;
}

void CalendarView_SetTheme(HWND view, const CalendarViewTheme& theme) noexcept {
  if (auto* state = State(view)) {
    state->theme = theme;
    InvalidateRect(view, nullptr, FALSE);
  }
}

bool CalendarView_SetToday(HWND view, CalendarDate today) noexcept {
  if (!IsValidCalendarDate(today)) return false;
  if (auto* state = State(view)) {
    const auto old_today = state->interaction.today();
    state->interaction.SetToday(today);
    InvalidateVisibleDate(view, old_today);
    InvalidateVisibleDate(view, today);
    InvalidateRectWithoutErase(view,
                               CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view)).today_action);
    return true;
  }
  return false;
}

bool CalendarView_SetDisplayedMonth(HWND view, CalendarDate month) noexcept {
  if (!IsValidCalendarDate(month)) return false;
  if (auto* state = State(view)) {
    const auto change = state->interaction.SetDisplayedMonth(month);
    if (change.changed) {
      InvalidateMonthBody(view);
      NotifyMonthChanged(view, change.displayed_month);
    }
    return true;
  }
  return false;
}

bool CalendarView_SetSelection(HWND view,
                               std::optional<CalendarDate> selection) noexcept {
  if (selection && !IsValidCalendarDate(*selection)) return false;
  if (auto* state = State(view)) {
    const auto old_selection = state->interaction.selected_date();
    const auto old_focus = state->interaction.focused_date();
    const auto old_month = state->interaction.displayed_month();
    if (!state->interaction.SetSelection(selection)) return false;
    if (!SameMonth(old_month, state->interaction.displayed_month())) {
      InvalidateMonthBody(view);
      NotifyMonthChanged(view, state->interaction.displayed_month());
    } else {
      if (old_selection && old_selection != selection)
        InvalidateVisibleDate(view, *old_selection);
      if (selection && old_focus && *old_focus != *selection && old_focus != old_selection)
        InvalidateVisibleDate(view, *old_focus);
      if (selection && (old_selection != selection || old_focus != selection))
        InvalidateVisibleDate(view, *selection);
    }
    return true;
  }
  return false;
}

bool CalendarView_SetMarkers(HWND view,
                             std::span<const CalendarViewDateMarker> markers) noexcept {
  if (auto* state = State(view)) {
    const auto month_dates = GetCalendarViewMonthDates(state->interaction.displayed_month());
    const auto geometry = CalculateCalendarViewGeometry(ClientRect(view), GetDpiForWindow(view));
    std::array<CalendarViewMarkerFlags, kCalendarViewCellCount> old_flags{};
    for (std::size_t index = 0; index < month_dates.size(); ++index) {
      if (month_dates[index]) old_flags[index] = state->interaction.MarkersForDate(*month_dates[index]);
    }
    if (!state->interaction.SetMarkers(markers)) return false;
    for (std::size_t index = 0; index < month_dates.size(); ++index) {
      if (month_dates[index] &&
          old_flags[index] != state->interaction.MarkersForDate(*month_dates[index])) {
        InvalidateRectWithoutErase(view, geometry.date_cells[index]);
      }
    }
    return true;
  }
  return false;
}

std::optional<CalendarDate> CalendarView_GetSelection(HWND view) noexcept {
  if (const auto* state = State(view)) return state->interaction.selected_date();
  return std::nullopt;
}

std::optional<CalendarDate> CalendarView_GetDisplayedMonth(HWND view) noexcept {
  if (const auto* state = State(view)) return state->interaction.displayed_month();
  return std::nullopt;
}

std::optional<CalendarDate> CalendarView_GetHoveredDate(HWND view) noexcept {
  if (const auto* state = State(view)) return state->interaction.hovered_date();
  return std::nullopt;
}

}  // namespace mdlite
