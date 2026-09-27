#pragma once

#include <cstddef>
#include <optional>
#include <filesystem>
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

bool ValidateJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                                std::wstring& error);
bool ImportJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                              std::wstring& error);
bool ImportJapaneseHolidayCsvFile(const std::filesystem::path& path,
                                  JapaneseHolidayImportInfo& info, std::wstring& error);
void ClearImportedJapaneseHolidays() noexcept;

}  // namespace mdlite
