#pragma once

#include <cstddef>
#include <optional>
#include <filesystem>
#include <stop_token>
#include <string>
#include <string_view>

namespace mdlite {

std::optional<std::wstring_view> JapaneseHolidayName(int year, int month, int day);
bool JapaneseHolidayYearSupported(int year);
int JapaneseHolidayFirstYear() noexcept;
int JapaneseHolidayLastYear() noexcept;
constexpr std::wstring_view JapaneseHolidayDataVersion() noexcept {
  return L"内閣府 国民の祝日・休日CSV 2026-02-02取得";
}

struct JapaneseHolidayImportInfo {
  std::size_t records{};
  int first_year{};
  int last_year{};
};

struct JapaneseHolidayOnlineResult {
  unsigned status{};
  bool not_modified{};
  std::wstring csv;
  std::wstring etag;
  std::wstring last_modified;
  std::wstring error;
};

bool ValidateJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                                std::wstring& error);
bool ImportJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                              std::wstring& error);
bool ImportJapaneseHolidayCsvFile(const std::filesystem::path& path,
                                  JapaneseHolidayImportInfo& info, std::wstring& error);
void ClearImportedJapaneseHolidays() noexcept;

bool FetchJapaneseHolidayCsv(std::stop_token stop, std::wstring_view etag,
                             std::wstring_view last_modified,
                             JapaneseHolidayOnlineResult& result);

}  // namespace mdlite
