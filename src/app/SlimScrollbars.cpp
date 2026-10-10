#include "app/SlimScrollbars.h"

#include <commctrl.h>
#include <oleacc.h>
#include <algorithm>
#include <new>

namespace mdlite {
namespace {
constexpr UINT_PTR kSubclass = 0x4d445342;
struct State {
  COLORREF background{};
  COLORREF thumb{};
  bool high_contrast{};
  bool hover{};
  bool pressed{};
  bool painting{};
};

void Paint(HWND window, State& state, HDC output = nullptr) {
  if (state.high_contrast || state.painting || !IsWindowVisible(window)) return;
  state.painting = true;
  HDC dc = output ? output : GetWindowDC(window);
  if (dc) {
    RECT frame{};
    GetWindowRect(window, &frame);
    const int saved = SaveDC(dc);
    SelectObject(dc, GetStockObject(DC_BRUSH));
    SelectObject(dc, GetStockObject(NULL_PEN));
    const int dpi = static_cast<int>(GetDpiForWindow(window));
    const int width = std::max(1, MulDiv(state.hover || state.pressed ? 10 : 6,
                                       dpi ? dpi : 96, 96));
    RECT bars[2]{};
    bool visible[2]{};
    for (int axis = 0; axis < 2; ++axis) {
      SCROLLBARINFO bar{sizeof(bar)};
      if (!GetScrollBarInfo(window, axis == 0 ? OBJID_VSCROLL : OBJID_HSCROLL, &bar) ||
          (bar.rgstate[0] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_OFFSCREEN))) continue;
      RECT track = bar.rcScrollBar;
      OffsetRect(&track, -frame.left, -frame.top);
      if (track.right <= track.left || track.bottom <= track.top) continue;
      bars[axis] = track;
      visible[axis] = true;
      SetDCBrushColor(dc, state.background);
      FillRect(dc, &track, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
      if (bar.xyThumbBottom <= bar.xyThumbTop ||
          (bar.rgstate[3] & (STATE_SYSTEM_INVISIBLE | STATE_SYSTEM_UNAVAILABLE))) continue;
      RECT thumb = track;
      if (axis == 0) {
        thumb.top += bar.xyThumbTop;
        thumb.bottom = track.top + bar.xyThumbBottom;
        const int center = (track.left + track.right) / 2;
        thumb.left = center - width / 2;
        thumb.right = thumb.left + width;
      } else {
        thumb.left += bar.xyThumbTop;
        thumb.right = track.left + bar.xyThumbBottom;
        const int center = (track.top + track.bottom) / 2;
        thumb.top = center - width / 2;
        thumb.bottom = thumb.top + width;
      }
      const COLORREF color = state.pressed ? RGB(160, 166, 174) : state.hover
          ? RGB(136, 143, 153) : state.thumb;
      SetDCBrushColor(dc, color);
      // GDI excludes the right/bottom boundary even with NULL_PEN.
      RoundRect(dc, thumb.left, thumb.top, thumb.right + 1, thumb.bottom + 1, width, width);
    }
    if (visible[0] && visible[1]) {
      RECT corner{bars[0].left, bars[1].top, bars[0].right, bars[1].bottom};
      SetDCBrushColor(dc, state.background);
      FillRect(dc, &corner, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
    }
    if (saved) RestoreDC(dc, saved);
    if (!output) ReleaseDC(window, dc);
  }
  state.painting = false;
}

LRESULT CALLBACK ScrollSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                               UINT_PTR, DWORD_PTR reference) {
  auto* state = reinterpret_cast<State*>(reference);
  if (message == WM_NCDESTROY) {
    RemoveWindowSubclass(window, ScrollSubclass, kSubclass);
    const LRESULT result = DefSubclassProc(window, message, wparam, lparam);
    delete state;
    return result;
  }
  if (message == WM_NCMOUSEMOVE) {
    state->hover = wparam == HTVSCROLL || wparam == HTHSCROLL;
    TRACKMOUSEEVENT track{sizeof(track), TME_LEAVE | TME_NONCLIENT, window, 0};
    TrackMouseEvent(&track);
  } else if (message == WM_NCMOUSELEAVE) {
    state->hover = false;
  } else if (message == WM_NCLBUTTONDOWN &&
             (wparam == HTVSCROLL || wparam == HTHSCROLL) && !state->high_contrast) {
    state->pressed = true;
    Paint(window, *state);
    const LRESULT native_result = DefSubclassProc(window, message, wparam, lparam);
    DWORD_PTR remaining{};
    if (GetWindowSubclass(window, ScrollSubclass, kSubclass, &remaining) &&
        remaining == reference) {
      state->pressed = false;
      Paint(window, *state);
    }
    return native_result;
  }
  const LRESULT result = DefSubclassProc(window, message, wparam, lparam);
  if (message == WM_PRINT && (lparam & PRF_NONCLIENT)) {
    Paint(window, *state, reinterpret_cast<HDC>(wparam));
    return result;
  }
  switch (message) {
    case WM_NCPAINT: case WM_PAINT: case WM_SIZE: case WM_STYLECHANGED:
    case WM_VSCROLL: case WM_HSCROLL: case WM_MOUSEWHEEL: case WM_MOUSEHWHEEL:
    case WM_NCMOUSEMOVE: case WM_NCMOUSELEAVE: case WM_THEMECHANGED:
      Paint(window, *state);
      break;
    default: break;
  }
  return result;
}
}

void ThemeSlimScrollbars(HWND window, COLORREF background, COLORREF thumb,
                         bool high_contrast) {
  if (!window) return;
  DWORD_PTR reference{};
  State* state{};
  if (GetWindowSubclass(window, ScrollSubclass, kSubclass, &reference)) {
    state = reinterpret_cast<State*>(reference);
  } else {
    state = new (std::nothrow) State;
    if (!state) return;
    if (!SetWindowSubclass(window, ScrollSubclass, kSubclass,
                           reinterpret_cast<DWORD_PTR>(state))) { delete state; return; }
  }
  state->background = background;
  state->thumb = thumb;
  state->high_contrast = high_contrast;
  RedrawWindow(window, nullptr, nullptr, RDW_FRAME | RDW_INVALIDATE);
}
}
