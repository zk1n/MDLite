#include "calendar/CalendarDayIndex.h"

#include "profiles/Profiles.h"

#include <windows.h>

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>

namespace {

int failures = 0;
int checks = 0;

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

void WriteText(const std::filesystem::path& path, std::string_view value) {
  std::filesystem::create_directories(path.parent_path());
  std::ofstream output(path, std::ios::binary);
  output.write(value.data(), static_cast<std::streamsize>(value.size()));
}

bool SetCreationDateLocal(const std::filesystem::path& path, mdlite::CalendarDate date) {
  SYSTEMTIME local{};
  local.wYear = static_cast<WORD>(date.year);
  local.wMonth = static_cast<WORD>(date.month);
  local.wDay = static_cast<WORD>(date.day);
  SYSTEMTIME utc{};
  FILETIME creation{};
  if (!TzSpecificLocalTimeToSystemTime(nullptr, &local, &utc) ||
      !SystemTimeToFileTime(&utc, &creation)) {
    return false;
  }
  HANDLE file = CreateFileW(path.c_str(), FILE_WRITE_ATTRIBUTES,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return false;
  const bool ok = SetFileTime(file, &creation, nullptr, nullptr) != FALSE;
  CloseHandle(file);
  return ok;
}
bool HasRelative(const mdlite::CalendarDayFileIndex& index, std::wstring_view path) {
  return std::ranges::any_of(index.files, [&](const auto& item) {
    return item.relative_path.generic_wstring() == path;
  });
}

void TestIndex(const std::filesystem::path& root) {
  WriteText(root / L"notes/alpha.md", "# alpha\n");
  WriteText(root / L"notes/config.JSON", "{}\n");
  WriteText(root / L"notes/plain.txt", "plain\n");
  WriteText(root / L"notes/trace.log", "trace\n");
  WriteText(root / L"notes/options.ini", "enabled=true\n");
  WriteText(root / L"notes/table.csv", "a,b\n");
  WriteText(root / L"notes/unsupported.png", "not an indexed extension\n");
  WriteText(root / L"notes/object.obj", "object\n");
  WriteText(root / L".git/hidden.md", "hidden\n");
  WriteText(root / L".mdlite/hidden.md", "hidden\n");
  WriteText(root / L"build/generated.md", "generated\n");
  WriteText(root / L"out/generated.md", "generated\n");
  WriteText(root / L"binary/blob.md", std::string("a\0b", 3));
  WriteText(root / L"recovery/recovered.md", "recovered\n");
  WriteText(root / L"cache/cached.md", "cached\n");
  WriteText(root / L"object/generated.md", "object\n");

  const auto index = mdlite::BuildCalendarDayFileIndex(root);
  Check(index.state == mdlite::CalendarIndexState::Ready, "eligible files produce ready index");
  Check(index.files.size() == 6, "only allowed text/config files are indexed");
  Check(HasRelative(index, L"notes/alpha.md"), "markdown relative path is reported");
  Check(HasRelative(index, L"notes/config.JSON"), "extension matching is case insensitive");
  Check(!HasRelative(index, L".git/hidden.md"), "git metadata is excluded");
  Check(!HasRelative(index, L"notes/unsupported.png"), "unsupported binary extension is excluded");
  Check(!HasRelative(index, L"binary/blob.md"), "NUL-containing binary content is excluded");

  const auto markdown = std::ranges::find_if(index.files, [](const auto& item) {
    return item.relative_path.generic_wstring() == L"notes/alpha.md";
  });
  Check(markdown != index.files.end() && markdown->name == L"alpha.md",
        "file name is separated from relative path");
  Check(markdown != index.files.end() && markdown->type == mdlite::CalendarFileType::Markdown,
        "markdown type is classified");
  Check(markdown != index.files.end() &&
            markdown->creation_time_provenance == mdlite::CalendarCreationTimeProvenance::FileSystem &&
            markdown->creation_time_utc.has_value(),
        "filesystem creation time is reported with provenance");
}

void TestSelectedDateFilter(const std::filesystem::path& root) {
  const auto workspace = root / L"selected-date-workspace";
  const auto today_file = workspace / L"notes/today.md";
  const auto old_file = workspace / L"notes/old.md";
  WriteText(today_file, "today\n");
  WriteText(old_file, "old\n");
  SYSTEMTIME now{};
  GetLocalTime(&now);
  const mdlite::CalendarDate today{static_cast<int>(now.wYear), static_cast<int>(now.wMonth),
                                   static_cast<int>(now.wDay)};
  Check(SetCreationDateLocal(today_file, today), "test fixture sets selected-day creation time");
  Check(SetCreationDateLocal(old_file, {2000, 1, 1}), "test fixture sets old creation time");

  mdlite::ProfileDefinition daily{
      L"daily", L"Daily", L"journal/{{date:yyyy}}", L"{{date:yyyyMMdd}}.md",
      L"templates/daily.md", mdlite::ProfileCollision::OpenExisting};
  const auto details = mdlite::BuildCalendarDayDetails(workspace, today, daily);
  Check(details.workspace_index.files.size() == 2, "workspace index retains all eligible files");
  Check(details.index.state == mdlite::CalendarIndexState::Ready && details.index.files.size() == 1,
        "day index includes only files created on selected date");
  Check(HasRelative(details.index, L"notes/today.md"), "selected-day file is retained");
  Check(!HasRelative(details.index, L"notes/old.md"), "older file is excluded from selected-day list");

  mdlite::CalendarDayFileIndex unknown_source;
  unknown_source.state = mdlite::CalendarIndexState::Ready;
  mdlite::CalendarFileDetails unknown;
  unknown.relative_path = L"unknown.md";
  unknown.name = L"unknown.md";
  unknown.extension = L".md";
  unknown.type = mdlite::CalendarFileType::Markdown;
  unknown.creation_time_utc = std::chrono::system_clock::now();
  unknown.creation_date_local = today;
  unknown.creation_time_provenance = mdlite::CalendarCreationTimeProvenance::Unknown;
  unknown_source.files.push_back(std::move(unknown));
  const auto filtered_unknown = mdlite::FilterCalendarDayFileIndexForDate(unknown_source, today);
  Check(filtered_unknown.state == mdlite::CalendarIndexState::Zero && filtered_unknown.files.empty(),
        "UNKNOWN creation provenance is excluded from selected-day files");
}
void TestStatesAndConfiguredDailyPath(const std::filesystem::path& root) {
  const auto reading = mdlite::MakeCalendarDayFileIndexReading();
  Check(reading.state == mdlite::CalendarIndexState::Reading && reading.files.empty(),
        "reading state is representable before asynchronous publication");

  const auto empty_root = root / L"empty";
  std::filesystem::create_directories(empty_root);
  const auto empty = mdlite::BuildCalendarDayFileIndex(empty_root);
  Check(empty.state == mdlite::CalendarIndexState::Zero && empty.files.empty(),
        "empty workspace reports zero state");

  const auto missing = mdlite::BuildCalendarDayFileIndex(root / L"missing");
  Check(missing.state == mdlite::CalendarIndexState::Error && !missing.error.empty(),
        "missing workspace reports error state");

  mdlite::ProfileDefinition daily{
      L"daily", L"Configured Daily", L"journal/{{date:yyyy}}/{{date:yyyyMM}}",
      L"{{date:yyyyMMdd}}.md", L"templates/daily.md", mdlite::ProfileCollision::OpenExisting};
  const mdlite::CalendarDate selected{2026, 9, 22};
  std::wstring error;
  const auto path = mdlite::ResolveCalendarDailyPath(root, selected, daily, error);
  Check(path.has_value() && error.empty(), "configured daily profile resolves for selected date");
  Check(path.has_value() && path->lexically_relative(root).generic_wstring() ==
                                  L"journal/2026/202609/20260922.md",
        "daily path uses configured profile tokens and no hardcoded directory");

  const auto details = mdlite::BuildCalendarDayDetails(root, selected, daily);
  Check(details.date == selected && details.configured_daily_path == path,
        "day details retain selected date and resolved daily path");

  const auto invalid = mdlite::ResolveCalendarDailyPath(root, {2026, 2, 30}, daily, error);
  Check(!invalid.has_value() && !error.empty(), "invalid selected date is rejected");
}

}  // namespace

int wmain() {
  const auto root = std::filesystem::temp_directory_path() /
                    (L"mdlite-calendar-index-tests-" + std::to_wstring(GetCurrentProcessId()));
  std::error_code error;
  std::filesystem::remove_all(root, error);
  std::filesystem::create_directories(root);
  TestIndex(root / L"workspace");
  TestSelectedDateFilter(root);
  TestStatesAndConfiguredDailyPath(root);
  std::filesystem::remove_all(root, error);
  if (failures == 0) std::cout << "All " << checks << " MDLite calendar index checks passed.\n";
  return failures == 0 ? 0 : 1;
}
