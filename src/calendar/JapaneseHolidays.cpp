#include "calendar/JapaneseHolidays.h"

#include <array>

namespace mdlite {
namespace {

struct Holiday {
  int year;
  int month;
  int day;
  std::wstring_view name;
};

constexpr std::array kHolidays{
    Holiday{2025, 1, 1, L"元日"}, Holiday{2025, 1, 13, L"成人の日"},
    Holiday{2025, 2, 11, L"建国記念の日"}, Holiday{2025, 2, 23, L"天皇誕生日"},
    Holiday{2025, 2, 24, L"休日"}, Holiday{2025, 3, 20, L"春分の日"},
    Holiday{2025, 4, 29, L"昭和の日"}, Holiday{2025, 5, 3, L"憲法記念日"},
    Holiday{2025, 5, 4, L"みどりの日"}, Holiday{2025, 5, 5, L"こどもの日"},
    Holiday{2025, 5, 6, L"休日"}, Holiday{2025, 7, 21, L"海の日"},
    Holiday{2025, 8, 11, L"山の日"}, Holiday{2025, 9, 15, L"敬老の日"},
    Holiday{2025, 9, 23, L"秋分の日"}, Holiday{2025, 10, 13, L"スポーツの日"},
    Holiday{2025, 11, 3, L"文化の日"}, Holiday{2025, 11, 23, L"勤労感謝の日"},
    Holiday{2025, 11, 24, L"休日"},
    Holiday{2026, 1, 1, L"元日"}, Holiday{2026, 1, 12, L"成人の日"},
    Holiday{2026, 2, 11, L"建国記念の日"}, Holiday{2026, 2, 23, L"天皇誕生日"},
    Holiday{2026, 3, 20, L"春分の日"}, Holiday{2026, 4, 29, L"昭和の日"},
    Holiday{2026, 5, 3, L"憲法記念日"}, Holiday{2026, 5, 4, L"みどりの日"},
    Holiday{2026, 5, 5, L"こどもの日"}, Holiday{2026, 5, 6, L"休日"},
    Holiday{2026, 7, 20, L"海の日"}, Holiday{2026, 8, 11, L"山の日"},
    Holiday{2026, 9, 21, L"敬老の日"}, Holiday{2026, 9, 22, L"休日"},
    Holiday{2026, 9, 23, L"秋分の日"}, Holiday{2026, 10, 12, L"スポーツの日"},
    Holiday{2026, 11, 3, L"文化の日"}, Holiday{2026, 11, 23, L"勤労感謝の日"},
    Holiday{2027, 1, 1, L"元日"}, Holiday{2027, 1, 11, L"成人の日"},
    Holiday{2027, 2, 11, L"建国記念の日"}, Holiday{2027, 2, 23, L"天皇誕生日"},
    Holiday{2027, 3, 21, L"春分の日"}, Holiday{2027, 3, 22, L"休日"},
    Holiday{2027, 4, 29, L"昭和の日"}, Holiday{2027, 5, 3, L"憲法記念日"},
    Holiday{2027, 5, 4, L"みどりの日"}, Holiday{2027, 5, 5, L"こどもの日"},
    Holiday{2027, 7, 19, L"海の日"}, Holiday{2027, 8, 11, L"山の日"},
    Holiday{2027, 9, 20, L"敬老の日"}, Holiday{2027, 9, 23, L"秋分の日"},
    Holiday{2027, 10, 11, L"スポーツの日"}, Holiday{2027, 11, 3, L"文化の日"},
    Holiday{2027, 11, 23, L"勤労感謝の日"},
};

}  // namespace

std::optional<std::wstring_view> JapaneseHolidayName(int year, int month, int day) {
  for (const auto& holiday : kHolidays) {
    if (holiday.year == year && holiday.month == month && holiday.day == day) return holiday.name;
  }
  return std::nullopt;
}

bool JapaneseHolidayYearSupported(int year) {
  return year >= JapaneseHolidayFirstYear() && year <= JapaneseHolidayLastYear();
}

}  // namespace mdlite
