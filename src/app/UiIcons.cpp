#include "app/UiIcons.h"

#include <algorithm>
#include <array>
#include <cmath>
#include <cstddef>
#include <limits>

namespace mdlite {
namespace {

constexpr int kCoordinateExtent = 2400;
constexpr int kStrokeWidth = 150;
constexpr std::size_t kPathCapacity = 80;

struct Path {
  std::array<POINT, kPathCapacity> points{};
  std::size_t count{};

  void Add(long x, long y) noexcept {
    if (count < points.size()) points[count++] = POINT{x, y};
  }

  void CubicTo(long control1_x, long control1_y, long control2_x,
               long control2_y, long end_x, long end_y) noexcept {
    if (count == 0) return;
    constexpr int kSteps = 16;
    const double start_x = points[count - 1].x;
    const double start_y = points[count - 1].y;
    for (int step = 1; step <= kSteps; ++step) {
      const double t = static_cast<double>(step) / kSteps;
      const double inverse = 1.0 - t;
      const double x = inverse * inverse * inverse * start_x +
                       3.0 * inverse * inverse * t * control1_x +
                       3.0 * inverse * t * t * control2_x +
                       t * t * t * end_x;
      const double y = inverse * inverse * inverse * start_y +
                       3.0 * inverse * inverse * t * control1_y +
                       3.0 * inverse * t * t * control2_y +
                       t * t * t * end_y;
      Add(static_cast<long>(std::lround(x)), static_cast<long>(std::lround(y)));
    }
  }

};

bool IsValidSelection(HGDIOBJ object) noexcept {
  return object != nullptr && object != HGDI_ERROR;
}

void StrokePath(HDC dc, const Path& path) noexcept {
  if (path.count > 1) {
    Polyline(dc, path.points.data(), static_cast<int>(path.count));
  }
}

void DrawCircle(HDC dc, long center_x, long center_y, long radius) noexcept {
  Ellipse(dc, center_x - radius, center_y - radius,
          center_x + radius, center_y + radius);
}

void DrawFilledCircle(HDC dc, COLORREF color, long center_x, long center_y,
                      long radius) noexcept {
  HGDIOBJ old_pen = SelectObject(dc, GetStockObject(NULL_PEN));
  if (!IsValidSelection(old_pen)) return;
  HGDIOBJ old_brush = SelectObject(dc, GetStockObject(DC_BRUSH));
  if (!IsValidSelection(old_brush)) {
    SelectObject(dc, old_pen);
    return;
  }

  const COLORREF old_color = SetDCBrushColor(dc, color);
  if (old_color != CLR_INVALID) {
    DrawCircle(dc, center_x, center_y, radius);
    SetDCBrushColor(dc, old_color);
  }
  SelectObject(dc, old_brush);
  SelectObject(dc, old_pen);
}

bool SetIconCoordinates(HDC dc, const RECT& bounds, int side) noexcept {
  const long long width = static_cast<long long>(bounds.right) - bounds.left;
  const long long height = static_cast<long long>(bounds.bottom) - bounds.top;
  const int origin_x = static_cast<int>(static_cast<long long>(bounds.left) + (width - side) / 2);
  const int origin_y = static_cast<int>(static_cast<long long>(bounds.top) + (height - side) / 2);
  return SetGraphicsMode(dc, GM_COMPATIBLE) != 0 &&
         SetMapMode(dc, MM_ANISOTROPIC) != 0 &&
         SetWindowOrgEx(dc, 0, 0, nullptr) != FALSE &&
         SetWindowExtEx(dc, kCoordinateExtent, kCoordinateExtent, nullptr) != FALSE &&
         SetViewportExtEx(dc, side, side, nullptr) != FALSE &&
         SetViewportOrgEx(dc, origin_x, origin_y, nullptr) != FALSE;
}

void DrawUiIconGeometry(HDC dc, UiIcon icon, COLORREF color) noexcept {
  switch (icon) {
    case UiIcon::Explorer: {
      Path folder;
      folder.Add(300, 750);
      folder.Add(1000, 750);
      folder.Add(1200, 550);
      folder.Add(2100, 550);
      folder.Add(2100, 1950);
      folder.Add(300, 1950);
      folder.Add(300, 750);
      StrokePath(dc, folder);
      Path divider;
      divider.Add(300, 1050);
      divider.Add(2100, 1050);
      StrokePath(dc, divider);
      break;
    }
    case UiIcon::Search: {
      DrawCircle(dc, 1050, 1050, 650);
      Path handle;
      handle.Add(1550, 1550);
      handle.Add(2050, 2050);
      StrokePath(dc, handle);
      break;
    }
    case UiIcon::GitBranch: {
      DrawCircle(dc, 600, 500, 220);
      DrawCircle(dc, 600, 1900, 220);
      DrawCircle(dc, 1800, 600, 220);

      Path trunk;
      trunk.Add(600, 720);
      trunk.Add(600, 1680);
      StrokePath(dc, trunk);

      Path branch;
      branch.Add(1800, 820);
      branch.Add(1800, 1050);
      branch.CubicTo(1800, 1326, 1576, 1550, 1300, 1550);
      branch.Add(1100, 1550);
      branch.CubicTo(886, 1532, 685, 1653, 600, 1850);
      StrokePath(dc, branch);
      break;
    }
    case UiIcon::Calendar: {
      RoundRect(dc, 300, 500, 2100, 2100, 300, 300);
      Path calendar;
      calendar.Add(700, 300);
      calendar.Add(700, 700);
      StrokePath(dc, calendar);
      calendar = {};
      calendar.Add(1700, 300);
      calendar.Add(1700, 700);
      StrokePath(dc, calendar);
      calendar = {};
      calendar.Add(300, 1000);
      calendar.Add(2100, 1000);
      StrokePath(dc, calendar);
      constexpr std::array<std::array<POINT, 2>, 4> entries{{
          {{{700, 1400}, {900, 1400}}},
          {{{1200, 1400}, {1400, 1400}}},
          {{{700, 1750}, {900, 1750}}},
          {{{1200, 1750}, {1400, 1750}}},
      }};
      for (const auto& entry : entries) {
        Path mark;
        mark.Add(entry[0].x, entry[0].y);
        mark.Add(entry[1].x, entry[1].y);
        StrokePath(dc, mark);
      }
      DrawFilledCircle(dc, color, 1700, 1400, 75);
      break;
    }
    case UiIcon::Settings: {
      constexpr std::array<POINT, 25> gear{{
          {950, 300}, {890, 530}, {710, 630}, {480, 570}, {240, 980},
          {410, 1150}, {410, 1350}, {240, 1520}, {480, 1930}, {710, 1870},
          {890, 1970}, {950, 2200}, {1450, 2200}, {1510, 1970}, {1690, 1870},
          {1920, 1930}, {2160, 1520}, {1990, 1350}, {1990, 1150}, {2160, 980},
          {1920, 570}, {1690, 630}, {1510, 530}, {1450, 300}, {950, 300},
      }};
      Polyline(dc, gear.data(), static_cast<int>(gear.size()));
      DrawCircle(dc, 1200, 1250, 320);
      break;
    }
    case UiIcon::Add: {
      Path plus;
      plus.Add(1200, 400);
      plus.Add(1200, 2000);
      StrokePath(dc, plus);
      plus = {};
      plus.Add(400, 1200);
      plus.Add(2000, 1200);
      StrokePath(dc, plus);
      break;
    }
    case UiIcon::Close: {
      Path cross;
      cross.Add(600, 600);
      cross.Add(1800, 1800);
      StrokePath(dc, cross);
      cross = {};
      cross.Add(1800, 600);
      cross.Add(600, 1800);
      StrokePath(dc, cross);
      break;
    }
    case UiIcon::ChevronLeft: {
      Path chevron;
      chevron.Add(1500, 600);
      chevron.Add(900, 1200);
      chevron.Add(1500, 1800);
      StrokePath(dc, chevron);
      break;
    }
    case UiIcon::ChevronRight: {
      Path chevron;
      chevron.Add(900, 600);
      chevron.Add(1500, 1200);
      chevron.Add(900, 1800);
      StrokePath(dc, chevron);
      break;
    }
    case UiIcon::ChevronDown: {
      Path chevron;
      chevron.Add(600, 900);
      chevron.Add(1200, 1500);
      chevron.Add(1800, 900);
      StrokePath(dc, chevron);
      break;
    }
    case UiIcon::More:
      DrawFilledCircle(dc, color, 500, 1200, 155);
      DrawFilledCircle(dc, color, 1200, 1200, 155);
      DrawFilledCircle(dc, color, 1900, 1200, 155);
      break;
    case UiIcon::File: {
      Path file;
      file.Add(500, 300);
      file.Add(1400, 300);
      file.Add(1900, 800);
      file.Add(1900, 2100);
      file.Add(500, 2100);
      file.Add(500, 300);
      StrokePath(dc, file);
      Path fold;
      fold.Add(1400, 300);
      fold.Add(1400, 800);
      fold.Add(1900, 800);
      StrokePath(dc, fold);
      break;
    }
    case UiIcon::Folder: {
      Path folder;
      folder.Add(300, 500);
      folder.Add(1000, 500);
      folder.Add(1200, 800);
      folder.Add(2100, 800);
      folder.Add(2100, 2000);
      folder.Add(300, 2000);
      folder.Add(300, 500);
      StrokePath(dc, folder);
      break;
    }
    case UiIcon::Check: {
      Path check;
      check.Add(400, 1200);
      check.Add(900, 1700);
      check.Add(2000, 600);
      StrokePath(dc, check);
      break;
    }
    case UiIcon::Shield: {
      Path shield;
      shield.Add(1200, 300);
      shield.Add(2000, 600);
      shield.Add(2000, 1200);
      shield.CubicTo(2000, 1700, 1200, 2100, 1200, 2100);
      shield.CubicTo(1200, 2100, 400, 1700, 400, 1200);
      shield.Add(400, 600);
      shield.Add(1200, 300);
      StrokePath(dc, shield);
      Path mark;
      mark.Add(1200, 800);
      mark.Add(1200, 1300);
      StrokePath(dc, mark);
      DrawFilledCircle(dc, color, 1200, 1600, 75);
      break;
    }
    case UiIcon::Refresh: {
      Path refresh;
      refresh.Add(2000, 1000);
      refresh.CubicTo(1884, 646, 1541, 419, 1170, 452);
      refresh.CubicTo(800, 484, 500, 767, 448, 1135);
      refresh.CubicTo(395, 1504, 603, 1859, 950, 1994);
      refresh.CubicTo(1296, 2129, 1690, 2007, 1900, 1700);
      StrokePath(dc, refresh);
      Path arrow;
      arrow.Add(2000, 400);
      arrow.Add(2000, 1000);
      arrow.Add(1400, 1000);
      StrokePath(dc, arrow);
      break;
    }
    case UiIcon::Outline: {
      constexpr std::array<std::array<POINT, 2>, 6> lines{{
          {{{900, 500}, {2100, 500}}},
          {{{900, 1200}, {2100, 1200}}},
          {{{900, 1900}, {2100, 1900}}},
          {{{300, 500}, {400, 500}}},
          {{{300, 1200}, {400, 1200}}},
          {{{300, 1900}, {400, 1900}}},
      }};
      for (const auto& line : lines) {
        Path stroke;
        stroke.Add(line[0].x, line[0].y);
        stroke.Add(line[1].x, line[1].y);
        StrokePath(dc, stroke);
      }
      break;
    }
    case UiIcon::Minimize: {
      Path line;
      line.Add(500, 1500);
      line.Add(1900, 1500);
      StrokePath(dc, line);
      break;
    }
    case UiIcon::Maximize:
      Rectangle(dc, 500, 500, 1900, 1900);
      break;
    case UiIcon::Restore: {
      Path rear_window;
      rear_window.Add(800, 500);
      rear_window.Add(800, 300);
      rear_window.Add(2100, 300);
      rear_window.Add(2100, 1600);
      rear_window.Add(1900, 1600);
      StrokePath(dc, rear_window);
      Path front_window;
      front_window.Add(300, 800);
      front_window.Add(1600, 800);
      front_window.Add(1600, 2100);
      front_window.Add(300, 2100);
      front_window.Add(300, 800);
      StrokePath(dc, front_window);
      break;
    }
  }
}

}  // namespace

void DrawUiIcon(HDC dc, RECT bounds, UiIcon icon, COLORREF color) noexcept {
  const long long width = static_cast<long long>(bounds.right) - bounds.left;
  const long long height = static_cast<long long>(bounds.bottom) - bounds.top;
  if (dc == nullptr || width <= 0 || height <= 0) return;
  const long long side_value = std::min(width, height);
  if (side_value > std::numeric_limits<int>::max()) return;

  const int saved_dc = SaveDC(dc);
  if (saved_dc == 0) return;

  const int side = static_cast<int>(side_value);
  if (!SetIconCoordinates(dc, bounds, side)) {
    RestoreDC(dc, saved_dc);
    return;
  }

  const LOGBRUSH brush{BS_SOLID, color, 0};
  HPEN pen = ExtCreatePen(PS_GEOMETRIC | PS_SOLID | PS_ENDCAP_ROUND | PS_JOIN_ROUND,
                          kStrokeWidth, &brush, 0, nullptr);
  if (pen == nullptr) {
    RestoreDC(dc, saved_dc);
    return;
  }

  HGDIOBJ old_pen = SelectObject(dc, pen);
  if (!IsValidSelection(old_pen)) {
    RestoreDC(dc, saved_dc);
    DeleteObject(pen);
    return;
  }
  HGDIOBJ old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
  if (!IsValidSelection(old_brush)) {
    RestoreDC(dc, saved_dc);
    DeleteObject(pen);
    return;
  }
  SetBkMode(dc, TRANSPARENT);

  DrawUiIconGeometry(dc, icon, color);

  RestoreDC(dc, saved_dc);
  DeleteObject(pen);
}

}  // namespace mdlite
