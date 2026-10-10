#pragma once

#include <windows.h>

namespace mdlite {
// Retain native scrolling, full-width hit testing and accessibility objects.
// Theme the non-client scrollbar pixels without changing the client viewport.
void ThemeSlimScrollbars(HWND window, COLORREF background, COLORREF thumb,
                         bool high_contrast);
}
