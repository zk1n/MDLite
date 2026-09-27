#pragma once

#include "calendar/CalendarDayIndex.h"

#include <array>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <span>
#include <vector>

#include <windows.h>

namespace mdlite {

inline constexpr std::size_t kCalendarViewCellCount = 42;

struct CalendarViewGeometry {
  RECT header{};
  RECT previous_month_button{};
  RECT next_month_button{};
  RECT weekdays{};
  RECT grid{};
  RECT today_action{};
  std::array<RECT, kCalendarViewCellCount> date_cells{};
};

using CalendarViewMonthDates =
    std::array<std::optional<CalendarDate>, kCalendarViewCellCount>;

enum class CalendarViewMarkerFlags : std::uint8_t {
  None = 0,
  Sunday = 1 << 0,
  Saturday = 1 << 1,
  Holiday = 1 << 2,
  Daily = 1 << 3,
};

constexpr CalendarViewMarkerFlags operator|(CalendarViewMarkerFlags left,
                                             CalendarViewMarkerFlags right) noexcept {
  return static_cast<CalendarViewMarkerFlags>(static_cast<std::uint8_t>(left) |
                                              static_cast<std::uint8_t>(right));
}

constexpr bool HasCalendarViewMarker(CalendarViewMarkerFlags flags,
                                     CalendarViewMarkerFlags marker) noexcept {
  return (static_cast<std::uint8_t>(flags) & static_cast<std::uint8_t>(marker)) != 0;
}

struct CalendarViewDateMarker {
  CalendarDate date{};
  CalendarViewMarkerFlags flags{CalendarViewMarkerFlags::None};
};

CalendarViewGeometry CalculateCalendarViewGeometry(RECT client) noexcept;
CalendarViewMonthDates GetCalendarViewMonthDates(CalendarDate displayed_month) noexcept;
std::optional<std::size_t> CalendarViewHitTestDate(
    const CalendarViewGeometry& geometry, POINT point) noexcept;

enum class CalendarViewActivationSource {
  Mouse,
  Keyboard,
};

struct CalendarViewInteractionResult {
  bool selection_changed{};
  bool activated{};
  CalendarDate date{};
  CalendarViewActivationSource source{CalendarViewActivationSource::Mouse};
};

struct CalendarViewHoverChange {
  bool changed{};
  std::optional<CalendarDate> date;
};

struct CalendarViewMonthChange {
  bool changed{};
  CalendarDate displayed_month{};
};

// Small date interaction state shared by the Win32 control and focused tests.
// Hover and month navigation intentionally return no activation event.
class CalendarViewInteraction {
 public:
  explicit CalendarViewInteraction(CalendarDate today) noexcept;

  CalendarDate displayed_month() const noexcept { return displayed_month_; }
  CalendarDate today() const noexcept { return today_; }
  std::optional<CalendarDate> selected_date() const noexcept { return selected_date_; }
  std::optional<CalendarDate> focused_date() const noexcept { return focused_date_; }
  std::optional<CalendarDate> hovered_date() const noexcept { return hovered_date_; }

  void SetToday(CalendarDate date) noexcept;
  CalendarViewMonthChange SetDisplayedMonth(CalendarDate date) noexcept;
  bool SetSelection(std::optional<CalendarDate> date) noexcept;
  CalendarViewHoverChange SetHover(std::optional<CalendarDate> date) noexcept;
  bool SetMarkers(std::span<const CalendarViewDateMarker> markers) noexcept;
  CalendarViewMarkerFlags MarkersForDate(CalendarDate date) const noexcept;
  bool MoveFocusDays(int days) noexcept;
  CalendarViewMonthChange NavigateMonth(int months) noexcept;
  CalendarViewInteractionResult ClickDate(CalendarDate date) noexcept;
  CalendarViewInteractionResult ActivateFocusedDate() noexcept;

 private:
  CalendarDate displayed_month_{};
  CalendarDate today_{};
  std::optional<CalendarDate> selected_date_;
  std::optional<CalendarDate> focused_date_;
  std::optional<CalendarDate> hovered_date_;
  std::vector<CalendarViewDateMarker> markers_;
};

struct CalendarViewTheme {
  COLORREF background{RGB(28, 30, 34)};
  COLORREF heading_text{RGB(226, 229, 235)};
  COLORREF day_text{RGB(222, 225, 231)};
  COLORREF sunday_text{RGB(231, 137, 145)};
  COLORREF saturday_text{RGB(127, 174, 229)};
  COLORREF holiday_text{RGB(231, 137, 145)};
  COLORREF adjacent_month_text{RGB(122, 128, 138)};
  COLORREF hover_background{RGB(53, 57, 65)};
  COLORREF selected_background{RGB(49, 97, 153)};
  COLORREF selected_text{RGB(255, 255, 255)};
  COLORREF today_outline{RGB(93, 164, 236)};
  COLORREF focus_outline{RGB(222, 222, 222)};
  COLORREF navigation_hover_background{RGB(53, 57, 65)};
  COLORREF holiday_marker{RGB(231, 137, 145)};
  COLORREF daily_marker{RGB(113, 190, 150)};
};

inline constexpr UINT CVN_DATE_ACTIVATED = 1;
inline constexpr UINT CVN_DATE_HOVERED = 2;
inline constexpr UINT CVN_MONTH_CHANGED = 3;
inline constexpr UINT CVN_DATE_SELECTED = 4;

struct CalendarViewDateActivatedNotification {
  NMHDR header{};
  CalendarDate date{};
  CalendarViewActivationSource source{CalendarViewActivationSource::Mouse};
};

struct CalendarViewDateHoveredNotification {
  NMHDR header{};
  bool has_date{};
  CalendarDate date{};
};

struct CalendarViewMonthChangedNotification {
  NMHDR header{};
  CalendarDate displayed_month{};
};

struct CalendarViewDateSelectedNotification {
  NMHDR header{};
  CalendarDate date{};
};

bool CalendarView_RegisterClass(HINSTANCE instance) noexcept;
HWND CalendarView_Create(HWND parent, int control_id, const RECT& bounds,
                          DWORD style = WS_CHILD | WS_VISIBLE | WS_TABSTOP) noexcept;
void CalendarView_SetTheme(HWND view, const CalendarViewTheme& theme) noexcept;
bool CalendarView_SetToday(HWND view, CalendarDate today) noexcept;
bool CalendarView_SetDisplayedMonth(HWND view, CalendarDate month) noexcept;
bool CalendarView_SetSelection(HWND view,
                               std::optional<CalendarDate> selection) noexcept;
bool CalendarView_SetMarkers(HWND view,
                             std::span<const CalendarViewDateMarker> markers) noexcept;
std::optional<CalendarDate> CalendarView_GetSelection(HWND view) noexcept;
std::optional<CalendarDate> CalendarView_GetDisplayedMonth(HWND view) noexcept;
std::optional<CalendarDate> CalendarView_GetHoveredDate(HWND view) noexcept;

}  // namespace mdlite
