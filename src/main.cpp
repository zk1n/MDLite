#include "app/Application.h"

#include <objbase.h>
#include <shellapi.h>
#include <werapi.h>

#include <filesystem>

namespace {

void ConfigureUnattendedCrashHandling() {
  wchar_t enabled[2]{};
  if (GetEnvironmentVariableW(L"MDLITE_TEST_NO_CRASH_UI", enabled, 2) != 1 || enabled[0] != L'1') return;
  SetErrorMode(GetErrorMode() | SEM_NOGPFAULTERRORBOX);
  WerSetFlags(WER_FAULT_REPORTING_NO_UI);
}

}  // namespace

int WINAPI wWinMain(HINSTANCE instance, HINSTANCE, PWSTR, int show_command) {
  ConfigureUnattendedCrashHandling();
  if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE))) return 1;
  int result = 1;
  {
    mdlite::Application application(instance);
    if (application.Initialize(show_command)) {
      int argc = 0;
      PWSTR* argv = CommandLineToArgvW(GetCommandLineW(), &argc);
      if (argv != nullptr) {
        for (int index = 1; index < argc; ++index)
          application.OpenInitialPath(std::filesystem::path(argv[index]));
      }
      if (argv != nullptr) LocalFree(argv);
      result = application.Run();
    }
  }
  CoUninitialize();
  return result;
}
