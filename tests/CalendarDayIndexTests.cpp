#include "calendar/CalendarDayIndex.h"

#include "profiles/Profiles.h"

#include <windows.h>

#include <algorithm>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <thread>
#include <future>

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

void TestCancellation(const std::filesystem::path& root) {
  const auto workspace = root / L"cancel-workspace";
  for (int i = 0; i < 8; ++i)
    WriteText(workspace / (L"note-" + std::to_wstring(i) + L".md"), "synthetic note\n");
  WriteText(workspace / L"nul.md", std::string("a\0b", 3));
  const auto before = mdlite::BuildCalendarDayFileIndex(workspace);
  const auto early = mdlite::BuildCalendarDayFileIndex(root / L"missing", {}, [] { return true; });
  Check(early.state == mdlite::CalendarIndexState::Reading && early.files.empty() && early.error.empty(),
        "cancelled request does not validate or publish a missing root");
  int checkpoints{};
  const auto partial = mdlite::BuildCalendarDayFileIndex(workspace, {}, [&] {
    return ++checkpoints >= 6;
  });
  Check(checkpoints == 6 && partial.state == mdlite::CalendarIndexState::Reading &&
            partial.files.empty() && partial.error.empty(),
        "mid-scan cancellation discards every partial file and error");
  const auto selected = mdlite::FilterCalendarDayFileIndexForDate(partial, {2026, 10, 6});
  Check(selected.state == mdlite::CalendarIndexState::Reading && selected.files.empty(),
        "cancelled partial scan cannot become a zero or ready selected-date result");
  auto failed_partial = before;
  failed_partial.state = mdlite::CalendarIndexState::Error;
  failed_partial.error = L"synthetic enumeration failure";
  const auto rejected = mdlite::FilterCalendarDayFileIndexForDate(failed_partial, {2026, 10, 6});
  Check(rejected.state == mdlite::CalendarIndexState::Error && rejected.files.empty() &&
            rejected.error == failed_partial.error,
        "enumeration error suppresses every partial file in selected-date publication");
  const auto after = mdlite::BuildCalendarDayFileIndex(workspace);
  Check(after.files.size() == 8 && !HasRelative(after, L"nul.md"),
        "fresh scan after cancellation preserves all eligible files and binary filtering");
  Check(before.files.size() == after.files.size() &&
            std::ranges::equal(before.files, after.files, [](const auto& a, const auto& b) {
              return a.relative_path == b.relative_path && a.creation_time_utc == b.creation_time_utc &&
                     a.creation_date_local == b.creation_date_local &&
                     a.creation_time_provenance == b.creation_time_provenance;
            }), "cancellation does not write or change creation provenance");
  std::promise<void> reached;
  auto ready = reached.get_future();
  mdlite::CalendarDayFileIndex stopped;
  std::jthread worker([&](std::stop_token stop) {
    bool announced{};
    stopped = mdlite::BuildCalendarDayFileIndex(workspace, {}, [&] {
      if (!announced) {
        announced = true;
        reached.set_value();
        while (!stop.stop_requested()) std::this_thread::yield();
      }
      return stop.stop_requested();
    });
  });
  ready.wait();
  worker.request_stop();
  worker.join();
  Check(stopped.state == mdlite::CalendarIndexState::Reading && stopped.files.empty(),
        "owned jthread stop/join returns without publishing a cancelled result");
  mdlite::ProfileDefinition daily{L"daily", L"Daily", L"journal", L"{{date:yyyyMMdd}}.md",
      L"templates/daily.md", mdlite::ProfileCollision::OpenExisting};
  const auto details = mdlite::BuildCalendarDayDetails(workspace, {2026, 10, 6}, daily, {},
                                                       [] { return true; });
  Check(details.index.state == mdlite::CalendarIndexState::Reading &&
            details.workspace_index.state == mdlite::CalendarIndexState::Reading &&
            details.index.files.empty() && details.workspace_index.files.empty() &&
            !details.configured_daily_path,
        "cancelled details expose neither a partial list nor a resolved ready Daily result");
}

void TestExternalFreshness(const std::filesystem::path& root) {
  const auto workspace = root / L"external-workspace";
  WriteText(workspace / L"first.md", "first synthetic note\n");
  const auto initial = mdlite::BuildCalendarDayFileIndex(workspace);
  WriteText(workspace / L"created.md", "external synthetic note\n");
  Check(initial.files.size() == 1 && !HasRelative(initial, L"created.md"),
        "completed snapshot does not silently acquire external files");
  const auto created = mdlite::BuildCalendarDayFileIndex(workspace);
  Check(created.files.size() == 2 && HasRelative(created, L"created.md"),
        "fresh request observes external creation");
  std::filesystem::remove(workspace / L"first.md");
  std::filesystem::rename(workspace / L"created.md", workspace / L"renamed.md");
  const auto renamed = mdlite::BuildCalendarDayFileIndex(workspace);
  Check(renamed.files.size() == 1 && HasRelative(renamed, L"renamed.md") &&
            !HasRelative(renamed, L"created.md") && !HasRelative(renamed, L"first.md"),
        "fresh request observes external deletion and rename");
  const auto original = std::ranges::find_if(created.files, [](const auto& file) {
    return file.relative_path == L"created.md";
  });
  Check(original != created.files.end() &&
            original->creation_time_utc == renamed.files.front().creation_time_utc,
        "external rename retains filesystem creation provenance");
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
  TestCancellation(root);
  TestExternalFreshness(root);
  std::filesystem::remove_all(root, error);
  if (failures == 0) std::cout << "All " << checks << " MDLite calendar index checks passed.\n";
  return failures == 0 ? 0 : 1;
}
