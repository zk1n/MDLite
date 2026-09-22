#pragma once

#include <chrono>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct ProfileDefinition;

struct CalendarDate {
  int year{};
  int month{};
  int day{};
  friend constexpr bool operator==(const CalendarDate&, const CalendarDate&) = default;
};

bool IsValidCalendarDate(CalendarDate date) noexcept;

enum class CalendarIndexState {
  Zero,
  Empty = Zero,
  Reading,
  Ready,
  Error,
};

enum class CalendarFileType {
  Markdown,
  Text,
  Configuration,
};

enum class CalendarCreationTimeProvenance {
  Unknown,
  FileSystem,
};

struct CalendarFileDetails {
  std::filesystem::path relative_path;
  std::wstring name;
  std::wstring extension;
  CalendarFileType type{CalendarFileType::Text};
  std::optional<std::chrono::system_clock::time_point> creation_time_utc;
  // Local calendar date derived from the same filesystem creation timestamp; absent means UNKNOWN.
  std::optional<CalendarDate> creation_date_local;
  CalendarCreationTimeProvenance creation_time_provenance{
      CalendarCreationTimeProvenance::Unknown};
};

struct CalendarIndexOptions {
  std::vector<std::wstring> allowed_extensions;
};

std::vector<std::wstring> DefaultCalendarFileExtensions();
std::wstring_view CalendarFileTypeName(CalendarFileType type) noexcept;

struct CalendarDayFileIndex {
  CalendarIndexState state{CalendarIndexState::Zero};
  std::vector<CalendarFileDetails> files;
  std::wstring error;
};

CalendarDayFileIndex MakeCalendarDayFileIndexReading() noexcept;
CalendarDayFileIndex BuildCalendarDayFileIndex(
    const std::filesystem::path& workspace,
    const CalendarIndexOptions& options = {});
CalendarDayFileIndex FilterCalendarDayFileIndexForDate(
    const CalendarDayFileIndex& source, CalendarDate date);

struct CalendarDayDetails {
  CalendarDate date;
  // index contains only files created on date. UNKNOWN creation provenance is
  // intentionally excluded; workspace_index retains the complete scan.
  CalendarDayFileIndex index;
  CalendarDayFileIndex workspace_index;
  std::optional<std::filesystem::path> configured_daily_path;
  std::wstring daily_path_error;
};

std::optional<std::filesystem::path> ResolveCalendarDailyPath(
    const std::filesystem::path& workspace, CalendarDate date,
    const ProfileDefinition& daily_profile, std::wstring& error);

CalendarDayDetails BuildCalendarDayDetails(
    const std::filesystem::path& workspace, CalendarDate date,
    const ProfileDefinition& daily_profile,
    const CalendarIndexOptions& options = {});

}  // namespace mdlite
