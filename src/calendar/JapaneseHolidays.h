#pragma once

#include <optional>
#include <string_view>

namespace mdlite {

std::optional<std::wstring_view> JapaneseHolidayName(int year, int month, int day);
bool JapaneseHolidayYearSupported(int year);
constexpr int JapaneseHolidayFirstYear() noexcept { return 2025; }
constexpr int JapaneseHolidayLastYear() noexcept { return 2027; }
constexpr std::wstring_view JapaneseHolidayDataVersion() noexcept {
  return L"内閣府 国民の祝日・休日CSV 2026-02-02取得";
}

}  // namespace mdlite
