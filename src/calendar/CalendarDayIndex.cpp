#include "calendar/CalendarDayIndex.h"

#include "profiles/Profiles.h"

#include <windows.h>

#include <algorithm>
#include <array>
#include <cstdint>
#include <ctime>
#include <cwctype>
#include <fstream>
#include <ranges>
#include <system_error>

namespace mdlite {
namespace {

std::wstring Lower(std::wstring value) {
  std::ranges::transform(value, value.begin(), towlower);
  return value;
}

std::wstring NormalizeExtension(std::wstring value) {
  value = Lower(std::move(value));
  if (!value.empty() && value.front() != L'.') value.insert(value.begin(), L'.');
  return value;
}

bool IsExcludedDirectory(std::wstring_view name) {
  static constexpr std::array<std::wstring_view, 10> kExcluded{
      L".git", L".mdlite", L"build", L"out", L"binary", L"recovery", L"cache",
      L".cache", L"object", L"objects"};
  const auto lowered = Lower(std::wstring(name));
  return std::ranges::any_of(kExcluded, [&](const auto item) { return lowered == item; });
}

CalendarFileType FileTypeForExtension(std::wstring_view extension) {
  if (extension == L".md" || extension == L".markdown") return CalendarFileType::Markdown;
  if (extension == L".json" || extension == L".toml" || extension == L".yaml" ||
      extension == L".yml" || extension == L".ini" || extension == L".cfg" ||
      extension == L".conf") {
    return CalendarFileType::Configuration;
  }
  return CalendarFileType::Text;
}

bool LooksBinary(const std::filesystem::path& path, bool& binary) {
  binary = false;
  std::ifstream input(path, std::ios::binary);
  if (!input) return false;
  std::array<char, 8192> bytes{};
  input.read(bytes.data(), static_cast<std::streamsize>(bytes.size()));
  const auto count = input.gcount();
  binary = std::ranges::any_of(bytes.begin(), bytes.begin() + count,
                               [](char value) { return value == '\0'; });
  return input.eof() || input.good();
}

std::optional<std::chrono::system_clock::time_point> FileTimeToSystemClock(
    const FILETIME& value) {
  ULARGE_INTEGER raw{};
  raw.LowPart = value.dwLowDateTime;
  raw.HighPart = value.dwHighDateTime;
  constexpr std::uint64_t kUnixEpochInFileTime = 116444736000000000ULL;
  if (raw.QuadPart == 0 || raw.QuadPart < kUnixEpochInFileTime) return std::nullopt;
  using FileTimeDuration = std::chrono::duration<std::int64_t, std::ratio<1, 10000000>>;
  const auto ticks = static_cast<std::int64_t>(raw.QuadPart - kUnixEpochInFileTime);
  return std::chrono::system_clock::time_point(
      std::chrono::duration_cast<std::chrono::system_clock::duration>(FileTimeDuration(ticks)));
}

std::optional<CalendarDate> LocalCalendarDate(
    const std::chrono::system_clock::time_point& value) {
  const std::time_t seconds = std::chrono::system_clock::to_time_t(value);
  tm local{};
  if (localtime_s(&local, &seconds) != 0) return std::nullopt;
  return CalendarDate{local.tm_year + 1900, local.tm_mon + 1, local.tm_mday};
}
std::optional<std::chrono::system_clock::time_point> ReadCreationTime(
    const std::filesystem::path& path) {
  HANDLE file = CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                            nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return std::nullopt;
  FILETIME creation{}, access{}, write{};
  const bool ok = GetFileTime(file, &creation, &access, &write) != FALSE;
  CloseHandle(file);
  return ok ? FileTimeToSystemClock(creation) : std::nullopt;
}

std::wstring FileEnumerationError(const std::filesystem::path& workspace,
                                  const std::error_code& error) {
  return L"Workspaceのカレンダー一覧を読み取れません: " + workspace.wstring() +
         L" (error " + std::to_wstring(error.value()) + L")";
}

std::vector<std::wstring> NormalizedExtensions(const CalendarIndexOptions& options) {
  auto values = options.allowed_extensions.empty() ? DefaultCalendarFileExtensions()
                                                    : options.allowed_extensions;
  for (auto& value : values) value = NormalizeExtension(std::move(value));
  std::ranges::sort(values);
  values.erase(std::ranges::unique(values).begin(), values.end());
  return values;
}

bool IsAllowedExtension(std::wstring_view extension,
                        const std::vector<std::wstring>& allowed) {
  return std::ranges::binary_search(allowed, extension);
}

bool IsValidWorkspaceRoot(const std::filesystem::path& path, std::wstring& error) {
  if (path.empty()) {
    error = L"Workspaceのpathが空です。";
    return false;
  }
  std::error_code filesystem_error;
  if (!std::filesystem::is_directory(path, filesystem_error) || filesystem_error) {
    error = L"Workspaceフォルダーを読み取れません: " + path.wstring();
    return false;
  }
  return true;
}

bool ToSystemTime(CalendarDate date, SYSTEMTIME& result, std::wstring& error) {
  if (!IsValidCalendarDate(date)) {
    error = L"カレンダーの日付が不正です。";
    return false;
  }
  result = {};
  result.wYear = static_cast<WORD>(date.year);
  result.wMonth = static_cast<WORD>(date.month);
  result.wDay = static_cast<WORD>(date.day);
  return true;
}

}  // namespace

bool IsValidCalendarDate(CalendarDate date) noexcept {
  if (date.year < 1601 || date.year > 9999 || date.month < 1 || date.month > 12 || date.day < 1)
    return false;
  const bool leap = date.year % 4 == 0 && (date.year % 100 != 0 || date.year % 400 == 0);
  const int days = date.month == 2 ? (leap ? 29 : 28)
                                  : ((date.month == 4 || date.month == 6 || date.month == 9 ||
                                      date.month == 11)
                                         ? 30
                                         : 31);
  return date.day <= days;
}

std::vector<std::wstring> DefaultCalendarFileExtensions() {
  return {L".md", L".markdown", L".txt", L".log", L".json", L".toml",
          L".yaml", L".yml", L".ini", L".cfg", L".conf", L".csv"};
}

std::wstring_view CalendarFileTypeName(CalendarFileType type) noexcept {
  switch (type) {
    case CalendarFileType::Markdown: return L"Markdown";
    case CalendarFileType::Text: return L"Text";
    case CalendarFileType::Configuration: return L"Configuration";
  }
  return L"Text";
}

CalendarDayFileIndex MakeCalendarDayFileIndexReading() noexcept {
  CalendarDayFileIndex result;
  result.state = CalendarIndexState::Reading;
  return result;
}

CalendarDayFileIndex BuildCalendarDayFileIndex(const std::filesystem::path& workspace,
                                               const CalendarIndexOptions& options) {
  CalendarDayFileIndex result = MakeCalendarDayFileIndexReading();
  std::wstring root_error;
  if (!IsValidWorkspaceRoot(workspace, root_error)) {
    result.state = CalendarIndexState::Error;
    result.error = std::move(root_error);
    return result;
  }

  const auto root = std::filesystem::absolute(workspace).lexically_normal();
  const auto allowed = NormalizedExtensions(options);
  std::error_code iterator_error;
  std::filesystem::recursive_directory_iterator iterator(
      root, std::filesystem::directory_options::skip_permission_denied, iterator_error);
  const std::filesystem::recursive_directory_iterator end;
  if (iterator_error) {
    result.state = CalendarIndexState::Error;
    result.error = FileEnumerationError(root, iterator_error);
    return result;
  }
  for (; iterator != end; iterator.increment(iterator_error)) {
    if (iterator_error) {
      result.error = FileEnumerationError(root, iterator_error);
      break;
    }
    const auto entry = *iterator;
    const auto filename = entry.path().filename().wstring();
    std::error_code entry_error;
    if (entry.is_directory(entry_error)) {
      if (IsExcludedDirectory(filename)) iterator.disable_recursion_pending();
      if (entry_error) result.error = FileEnumerationError(entry.path(), entry_error);
      continue;
    }
    if (entry_error) {
      result.error = FileEnumerationError(entry.path(), entry_error);
      continue;
    }
    if (entry.is_symlink(entry_error) || entry_error) {
      if (entry_error) result.error = FileEnumerationError(entry.path(), entry_error);
      continue;
    }
    if (!entry.is_regular_file(entry_error)) {
      if (entry_error) result.error = FileEnumerationError(entry.path(), entry_error);
      continue;
    }
    if (entry_error) {
      result.error = FileEnumerationError(entry.path(), entry_error);
      continue;
    }

    const auto extension = NormalizeExtension(entry.path().extension().wstring());
    if (!IsAllowedExtension(extension, allowed)) continue;
    bool binary{};
    if (!LooksBinary(entry.path(), binary)) {
      result.error = FileEnumerationError(entry.path(),
                                          std::make_error_code(std::errc::permission_denied));
      continue;
    }
    if (binary) continue;

    CalendarFileDetails details;
    details.relative_path = entry.path().lexically_relative(root);
    details.name = entry.path().filename().wstring();
    details.extension = extension;
    details.type = FileTypeForExtension(extension);
    details.creation_time_utc = ReadCreationTime(entry.path());
    if (details.creation_time_utc) {
      details.creation_time_provenance = CalendarCreationTimeProvenance::FileSystem;
      details.creation_date_local = LocalCalendarDate(*details.creation_time_utc);
    }
    result.files.push_back(std::move(details));
  }

  std::ranges::sort(result.files, [](const auto& left, const auto& right) {
    return _wcsicmp(left.relative_path.c_str(), right.relative_path.c_str()) < 0;
  });
  if (!result.error.empty()) {
    result.state = CalendarIndexState::Error;
  } else if (result.files.empty()) {
    result.state = CalendarIndexState::Zero;
  } else {
    result.state = CalendarIndexState::Ready;
  }
  return result;
}

CalendarDayFileIndex FilterCalendarDayFileIndexForDate(
    const CalendarDayFileIndex& source, CalendarDate date) {
  CalendarDayFileIndex result;
  if (source.state == CalendarIndexState::Error) {
    result.state = CalendarIndexState::Error;
    result.error = source.error;
    return result;
  }
  if (source.state == CalendarIndexState::Reading) {
    result.state = CalendarIndexState::Reading;
    return result;
  }
  if (source.state == CalendarIndexState::Zero) {
    result.state = CalendarIndexState::Zero;
    return result;
  }
  if (!IsValidCalendarDate(date)) {
    result.state = CalendarIndexState::Error;
    result.error = L"カレンダーの日付が不正です。";
    return result;
  }
  for (const auto& file : source.files) {
    if (file.creation_time_provenance != CalendarCreationTimeProvenance::FileSystem ||
        !file.creation_time_utc) {
      continue;
    }
    const auto created_date = file.creation_date_local
                                  ? file.creation_date_local
                                  : LocalCalendarDate(*file.creation_time_utc);
    if (created_date && *created_date == date) result.files.push_back(file);
  }
  result.state = result.files.empty() ? CalendarIndexState::Zero : CalendarIndexState::Ready;
  return result;
}
std::optional<std::filesystem::path> ResolveCalendarDailyPath(
    const std::filesystem::path& workspace, CalendarDate date,
    const ProfileDefinition& daily_profile, std::wstring& error) {
  SYSTEMTIME system_date{};
  if (!ToSystemTime(date, system_date, error)) return std::nullopt;
  return PreviewProfilePath(workspace, daily_profile, system_date, error);
}

CalendarDayDetails BuildCalendarDayDetails(const std::filesystem::path& workspace,
                                           CalendarDate date,
                                           const ProfileDefinition& daily_profile,
                                           const CalendarIndexOptions& options) {
  CalendarDayDetails result;
  result.date = date;
  result.workspace_index = BuildCalendarDayFileIndex(workspace, options);
  result.index = FilterCalendarDayFileIndexForDate(result.workspace_index, date);
  result.configured_daily_path = ResolveCalendarDailyPath(
      workspace, date, daily_profile, result.daily_path_error);
  return result;
}

}  // namespace mdlite
