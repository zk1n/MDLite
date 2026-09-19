#include "app/Application.h"

#include <objbase.h>
#include <shellapi.h>

#include <filesystem>

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int show_command) {
  if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE))) return 1;
  int result = 1;
  {
    mdlite::Application application(instance);
    if (application.Initialize(show_command)) {
      int argc = 0;
      PWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
      if (argv != nullptr && argc > 1) application.OpenInitialPath(std::filesystem::path(argv[1]));
      if (argv != nullptr) LocalFree(argv);
      result = application.Run();
    }
  }
  CoUninitialize();
  return result;
}
