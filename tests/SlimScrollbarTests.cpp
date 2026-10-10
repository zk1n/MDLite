#include "app/SlimScrollbars.h"
#include <commctrl.h>
#include <oleacc.h>
#include <iostream>
#include <thread>
#include <atomic>

namespace {
int failures{};
void Check(bool condition, const char* label) {
  if (!condition) { std::cerr << "FAIL: " << label << '\n'; ++failures; }
}
int ThumbPixels(HWND window, const SCROLLBARINFO& info, COLORREF color) {
  RECT frame{};
  GetWindowRect(window, &frame);
  HDC dc = GetWindowDC(window);
  int count{};
  const int y = info.rcScrollBar.top - frame.top +
                (info.xyThumbTop + info.xyThumbBottom) / 2;
  for (int x = info.rcScrollBar.left - frame.left;
       x < info.rcScrollBar.right - frame.left; ++x)
    if (GetPixel(dc, x, y) == color) ++count;
  ReleaseDC(window, dc);
  return count;
}
}

int main() {
  INITCOMMONCONTROLSEX controls{sizeof(controls), ICC_TREEVIEW_CLASSES};
  InitCommonControlsEx(&controls);
  WNDCLASSW host_class{};
  host_class.lpfnWndProc = DefWindowProcW;
  host_class.hInstance = GetModuleHandleW(nullptr);
  host_class.lpszClassName = L"MDLite.ScrollbarTest";
  RegisterClassW(&host_class);
  HWND host = CreateWindowExW(0, host_class.lpszClassName, L"MDLite scrollbar test", WS_OVERLAPPEDWINDOW,
                              100, 100, 360, 340, nullptr, nullptr, host_class.hInstance, nullptr);
  HWND tree = CreateWindowExW(0, WC_TREEVIEWW, L"", WS_CHILD | WS_VISIBLE | WS_VSCROLL,
                              0, 0, 250, 250, host, nullptr, nullptr, nullptr);
  for (int i = 0; i < 60; ++i) {
    TVINSERTSTRUCTW item{};
    item.hInsertAfter = TVI_LAST;
    item.item.mask = TVIF_TEXT;
    item.item.pszText = const_cast<wchar_t*>(L"Scrollbar geometry fixture");
    TreeView_InsertItem(tree, &item);
  }
  ShowWindow(host, SW_SHOWNOACTIVATE);
  UpdateWindow(host);
  UpdateWindow(tree);
  RECT before{};
  GetClientRect(tree, &before);
  SCROLLINFO range_before{sizeof(range_before), SIF_ALL};
  GetScrollInfo(tree, SB_VERT, &range_before);
  const COLORREF background = RGB(28, 31, 36), thumb = RGB(103, 111, 122);
  mdlite::ThemeSlimScrollbars(tree, background, thumb, false);
  RedrawWindow(tree, nullptr, nullptr, RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW);
  RECT after{};
  GetClientRect(tree, &after);
  Check(EqualRect(&before, &after), "theme keeps viewport unchanged");
  SCROLLINFO range_after{sizeof(range_after), SIF_ALL};
  GetScrollInfo(tree, SB_VERT, &range_after);
  Check(range_before.nMin == range_after.nMin && range_before.nMax == range_after.nMax &&
        range_before.nPage == range_after.nPage, "native range and page preserved");
  SCROLLBARINFO bar{sizeof(bar)};
  Check(GetScrollBarInfo(tree, OBJID_VSCROLL, &bar) &&
        !(bar.rgstate[0] & STATE_SYSTEM_INVISIBLE), "native accessible scrollbar exists");
  const int dpi = static_cast<int>(GetDpiForWindow(tree));
  std::cout << "DPI " << dpi << " thumb " << bar.xyThumbTop << ':' << bar.xyThumbBottom
            << " pixels " << ThumbPixels(tree, bar, thumb) << '\n';
  Check(ThumbPixels(tree, bar, thumb) == MulDiv(6, dpi, 96), "normal thumb is six logical pixels");
  SendMessageW(tree, WM_NCMOUSEMOVE, HTVSCROLL, 0);
  std::cout << "hover pixels " << ThumbPixels(tree, bar, RGB(136,143,153)) << '\n';
  Check(ThumbPixels(tree, bar, RGB(136,143,153)) == MulDiv(10, dpi, 96),
        "hover thumb is ten logical pixels");
  SendMessageW(tree, WM_NCMOUSELEAVE, 0, 0);
  SendMessageW(tree, WM_VSCROLL, SB_PAGEDOWN, 0);
  GetScrollInfo(tree, SB_VERT, &range_after);
  Check(range_after.nPos > range_before.nPos, "native page scroll still works");
  // Explicit product-verification run only: ordinary CTest never moves the
  // user's pointer or injects input. Capture the visible thumb during the
  // native modal drag, which can draw without a WM_PAINT notification.
  wchar_t drag_enabled[2]{};
  if (GetEnvironmentVariableW(L"MDLITE_TEST_SCROLLBAR_OS_DRAG", drag_enabled, 2) &&
      drag_enabled[0] == L'1') {
    SendMessageW(tree, WM_VSCROLL, SB_TOP, 0);
    RedrawWindow(tree, nullptr, nullptr, RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW);
    GetScrollBarInfo(tree, OBJID_VSCROLL, &bar);
    POINT original{};
    GetCursorPos(&original);
    const int x = (bar.rcScrollBar.left + bar.rcScrollBar.right) / 2;
    const int y = bar.rcScrollBar.top + (bar.xyThumbTop + bar.xyThumbBottom) / 2;
    SetWindowPos(host, HWND_TOP, 0, 0, 0, 0,
                 SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    SetCursorPos(x, y);
    // Mouse delivery is guarded by the exact visible hit target, unlike key
    // injection, which requires foreground/focus ownership.
    if (WindowFromPoint(POINT{x, y}) == tree) {
      std::atomic<bool> finished{};
      std::atomic<int> active_pixels{};
      std::thread drag([&]() {
        const DWORD ui_thread = GetWindowThreadProcessId(tree, nullptr);
        const ULONGLONG ready_until = GetTickCount64() + 1000;
        GUITHREADINFO gui{sizeof(gui)};
        do {
          GetGUIThreadInfo(ui_thread, &gui);
          if (gui.hwndCapture == tree) break;
          Sleep(5);
        } while (GetTickCount64() < ready_until);
        SetCursorPos(x, y + 60);
        Sleep(100);
        SCROLLBARINFO current{sizeof(current)};
        GetScrollBarInfo(tree, OBJID_VSCROLL, &current);
        active_pixels = ThumbPixels(tree, current, RGB(160,166,174));
        RECT frame{};
        GetWindowRect(tree, &frame);
        HDC screen = GetWindowDC(tree);
        HDC pixels = CreateCompatibleDC(screen);
        HBITMAP bitmap = CreateCompatibleBitmap(screen, frame.right - frame.left, frame.bottom - frame.top);
        HGDIOBJ previous = SelectObject(pixels, bitmap);
        BitBlt(pixels, 0, 0, frame.right-frame.left, frame.bottom-frame.top, screen, 0, 0, SRCCOPY);
        int active_total{}, normal_total{}, hover_total{}, white_total{};
        for (int py = current.rcScrollBar.top - frame.top;
             py < current.rcScrollBar.bottom - frame.top; ++py)
          for (int px = current.rcScrollBar.left - frame.left;
               px < current.rcScrollBar.right - frame.left; ++px) {
            const COLORREF value = GetPixel(pixels, px, py);
            active_total += value == RGB(160,166,174);
            normal_total += value == thumb;
            hover_total += value == RGB(136,143,153);
            white_total += value == RGB(255,255,255);
          }
        SelectObject(pixels, previous);
        DeleteObject(bitmap);
        DeleteDC(pixels);
        ReleaseDC(tree, screen);
        std::cout << "drag colors active=" << active_total << " normal=" << normal_total
                  << " hover=" << hover_total << " white=" << white_total
                  << " capture=" << (gui.hwndCapture == tree) << '\n';
        INPUT up{};
        up.type = INPUT_MOUSE;
        up.mi.dwFlags = MOUSEEVENTF_LEFTUP;
        SendInput(1, &up, sizeof(up));
        finished = true;
      });
      INPUT down{};
      down.type = INPUT_MOUSE;
      down.mi.dwFlags = MOUSEEVENTF_LEFTDOWN;
      Check(SendInput(1, &down, sizeof(down)) == 1, "OS scrollbar press delivered");
      const ULONGLONG until = GetTickCount64() + 1500;
      while (!finished && GetTickCount64() < until) {
        MSG message{};
        while (PeekMessageW(&message, nullptr, 0, 0, PM_REMOVE)) {
          TranslateMessage(&message);
          DispatchMessageW(&message);
        }
        Sleep(1);
      }
      drag.join();
      std::cout << "OS drag active pixels " << active_pixels << '\n';
      Check(active_pixels == MulDiv(10, dpi, 96), "active drag remains slim gray");
      GetScrollInfo(tree, SB_VERT, &range_after);
      Check(range_after.nPos > 0, "OS thumb drag scrolls the native control");
    } else { Check(false, "visible hit-target guard prevented OS drag"); }
    SetCursorPos(original.x, original.y);
  }
  mdlite::ThemeSlimScrollbars(tree, background, thumb, true);
  RedrawWindow(tree, nullptr, nullptr, RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW);
  GetScrollBarInfo(tree, OBJID_VSCROLL, &bar);
  Check(ThumbPixels(tree, bar, thumb) == 0, "high contrast delegates native rendering");
  DestroyWindow(host);
  if (!failures) std::cout << "Slim scrollbar native geometry, pixels and page scroll PASS\n";
  return failures ? 1 : 0;
}
