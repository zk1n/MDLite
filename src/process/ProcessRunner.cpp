#include "process/ProcessRunner.h"

#include <windows.h>

#include <algorithm>
#include <thread>

namespace mdlite {
namespace {

std::wstring Quote(std::wstring_view value) {
  if (value.find_first_of(L" \t\"") == std::wstring_view::npos) return std::wstring(value);
  std::wstring quoted = L"\"";
  std::size_t slashes{};
  for (wchar_t character : value) {
    if (character == L'\\') { ++slashes; continue; }
    if (character == L'\"') quoted.append(slashes * 2 + 1, L'\\');
    else quoted.append(slashes, L'\\');
    slashes = 0;
    quoted.push_back(character);
  }
  quoted.append(slashes * 2, L'\\');
  quoted.push_back(L'\"');
  return quoted;
}

std::wstring Decode(const std::string& bytes) {
  if (bytes.empty()) return {};
  int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                 static_cast<int>(bytes.size()), nullptr, 0);
  UINT code_page = CP_UTF8;
  DWORD flags = MB_ERR_INVALID_CHARS;
  if (size <= 0) {
    code_page = GetOEMCP();
    flags = 0;
    size = MultiByteToWideChar(code_page, flags, bytes.data(), static_cast<int>(bytes.size()), nullptr, 0);
  }
  if (size <= 0) return L"(process output decode failed)";
  std::wstring value(static_cast<std::size_t>(size), L'\0');
  MultiByteToWideChar(code_page, flags, bytes.data(), static_cast<int>(bytes.size()), value.data(), size);
  return value;
}

}  // namespace

bool RunProcess(const std::filesystem::path& executable, const std::vector<std::wstring>& arguments,
                const std::filesystem::path& working_directory, std::size_t output_limit,
                unsigned long timeout_ms, ProcessResult& result, std::wstring& error,
                void* cancellation_event) {
  result = {};
  SECURITY_ATTRIBUTES security{sizeof(security), nullptr, TRUE};
  HANDLE read_pipe{}, write_pipe{};
  if (!CreatePipe(&read_pipe, &write_pipe, &security, 0) ||
      !SetHandleInformation(read_pipe, HANDLE_FLAG_INHERIT, 0)) {
    error = L"process出力pipeを作成できません。";
    if (read_pipe) CloseHandle(read_pipe);
    if (write_pipe) CloseHandle(write_pipe);
    return false;
  }
  std::wstring command = Quote(executable.wstring());
  for (const auto& argument : arguments) command += L" " + Quote(argument);
  STARTUPINFOW startup{sizeof(startup)};
  startup.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
  startup.wShowWindow = SW_HIDE;
  startup.hStdOutput = write_pipe;
  startup.hStdError = write_pipe;
  startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
  PROCESS_INFORMATION process{};
  const std::wstring directory = working_directory.wstring();
  const BOOL created = CreateProcessW(executable.c_str(), command.data(), nullptr, nullptr, TRUE,
                                      CREATE_NO_WINDOW, nullptr,
                                      directory.empty() ? nullptr : directory.c_str(), &startup, &process);
  CloseHandle(write_pipe);
  if (!created) {
    CloseHandle(read_pipe);
    error = L"外部processを開始できません: " + executable.wstring();
    return false;
  }
  HANDLE job = CreateJobObjectW(nullptr, nullptr);
  if (job) {
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION limits{};
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    SetInformationJobObject(job, JobObjectExtendedLimitInformation, &limits, sizeof(limits));
    AssignProcessToJobObject(job, process.hProcess);
  }
  std::string bytes;
  std::thread reader([&] {
    char buffer[4096];
    DWORD read{};
    while (ReadFile(read_pipe, buffer, sizeof(buffer), &read, nullptr) && read != 0) {
      const std::size_t keep = std::min<std::size_t>(read, output_limit > bytes.size() ? output_limit - bytes.size() : 0);
      bytes.append(buffer, keep);
      if (keep < read) result.truncated = true;
    }
  });
  const ULONGLONG deadline = timeout_ms == INFINITE ? 0 : GetTickCount64() + timeout_ms;
  for (;;) {
    HANDLE waits[2] = {process.hProcess, static_cast<HANDLE>(cancellation_event)};
    const DWORD count = cancellation_event ? 2 : 1;
    const DWORD slice = timeout_ms == INFINITE ? 100 :
        static_cast<DWORD>(std::min<ULONGLONG>(100, deadline > GetTickCount64() ? deadline - GetTickCount64() : 0));
    const DWORD wait = WaitForMultipleObjects(count, waits, FALSE, slice);
    if (wait == WAIT_OBJECT_0) break;
    if (cancellation_event && wait == WAIT_OBJECT_0 + 1) { result.cancelled = true; break; }
    if (timeout_ms != INFINITE && GetTickCount64() >= deadline) { result.timed_out = true; break; }
  }
  if (result.cancelled || result.timed_out) {
    if (job) TerminateJobObject(job, ERROR_CANCELLED);
    else TerminateProcess(process.hProcess, ERROR_CANCELLED);
  }
  WaitForSingleObject(process.hProcess, 5000);
  GetExitCodeProcess(process.hProcess, &result.exit_code);
  CloseHandle(process.hThread);
  CloseHandle(process.hProcess);
  if (job) CloseHandle(job);
  reader.join();
  CloseHandle(read_pipe);
  result.output = Decode(bytes);
  return true;
}

}  // namespace mdlite
