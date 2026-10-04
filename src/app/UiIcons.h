#pragma once

#include <windows.h>

namespace mdlite {

enum class UiIcon {
  Explorer,
  Search,
  GitBranch,
  Calendar,
  Settings,
  Add,
  Close,
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  More,
  File,
  Folder,
  Check,
  Shield,
  Refresh,
  Outline,
  Minimize,
  Maximize,
  Restore,
};

void DrawUiIcon(HDC dc, RECT bounds, UiIcon icon, COLORREF color) noexcept;

}  // namespace mdlite
