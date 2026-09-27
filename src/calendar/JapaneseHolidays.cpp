#include "calendar/JapaneseHolidays.h"

#include <windows.h>

#include <algorithm>
#include <array>
#include <fstream>
#include <mutex>
#include <set>
#include <sstream>
#include <tuple>
#include <vector>

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

struct ImportedHoliday {
  int year;
  int month;
  int day;
  std::wstring name;
};

std::mutex imported_mutex;
std::vector<ImportedHoliday> imported_holidays;

std::wstring Trim(std::wstring value) {
  while (!value.empty() && iswspace(value.front())) value.erase(value.begin());
  while (!value.empty() && iswspace(value.back())) value.pop_back();
  return value;
}

bool DecodeCsv(const std::string& bytes, std::wstring& value) {
  std::string_view payload(bytes);
  UINT code_page = CP_UTF8;
  if (payload.size() >= 3 && static_cast<unsigned char>(payload[0]) == 0xEF &&
      static_cast<unsigned char>(payload[1]) == 0xBB &&
      static_cast<unsigned char>(payload[2]) == 0xBF) payload.remove_prefix(3);
  int size = MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, payload.data(),
                                static_cast<int>(payload.size()), nullptr, 0);
  if (size <= 0) {
    code_page = 932;
    size = MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, payload.data(),
                               static_cast<int>(payload.size()), nullptr, 0);
  }
  if (size <= 0) return false;
  value.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(code_page, MB_ERR_INVALID_CHARS, payload.data(),
                             static_cast<int>(payload.size()), value.data(), size) == size;
}

bool ParseDate(std::wstring_view value, int& year, int& month, int& day) {
  std::array<int, 3> parts{};
  std::size_t begin{};
  for (std::size_t index{}; index <= value.size(); ++index) {
    if (index != value.size() && value[index] != L'/' && value[index] != L'-') continue;
    if (index == begin || index - begin > 4) return false;
    try {
      std::size_t consumed{};
      parts[std::count(value.begin(), value.begin() + static_cast<std::ptrdiff_t>(begin), L'/') +
            std::count(value.begin(), value.begin() + static_cast<std::ptrdiff_t>(begin), L'-')] =
          std::stoi(std::wstring(value.substr(begin, index - begin)), &consumed);
      if (consumed != index - begin) return false;
    } catch (...) { return false; }
    begin = index + 1;
  }
  const auto separators = std::count(value.begin(), value.end(), L'/') +
                          std::count(value.begin(), value.end(), L'-');
  if (separators != 2) return false;
  year = parts[0]; month = parts[1]; day = parts[2];
  if (year < 1900 || year > 2200 || month < 1 || month > 12 || day < 1 || day > 31) return false;
  const int days_in_month = (month == 2 ? ((year % 4 == 0 && year % 100 != 0) || year % 400 == 0 ? 29 : 28)
                             : (month == 4 || month == 6 || month == 9 || month == 11 ? 30 : 31));
  return day <= days_in_month;
}

bool SplitCsvLine(std::wstring_view line, std::vector<std::wstring>& fields) {
  fields.clear();
  std::wstring field;
  bool quoted{};
  for (std::size_t index{}; index < line.size(); ++index) {
    const wchar_t ch = line[index];
    if (ch == L'"') {
      if (quoted && index + 1 < line.size() && line[index + 1] == L'"') {
        field.push_back(L'"'); ++index;
      } else quoted = !quoted;
    } else if (ch == L',' && !quoted) {
      fields.push_back(Trim(std::move(field))); field.clear();
    } else field.push_back(ch);
  }
  if (quoted) return false;
  fields.push_back(Trim(std::move(field)));
  return true;
}

bool ParseCsvRecords(std::wstring_view csv, std::vector<ImportedHoliday>& parsed,
                     JapaneseHolidayImportInfo& info, std::wstring& error) {
  info = {};
  parsed.clear();
  std::set<std::tuple<int, int, int>> seen;
  std::wistringstream lines{std::wstring(csv)};
  std::wstring line;
  std::size_t line_number{};
  std::vector<std::wstring> fields;
  while (std::getline(lines, line)) {
    ++line_number;
    if (line_number > 10000) { error = L"祝日CSVの行数が上限を超えています。"; return false; }
    if (!line.empty() && line.back() == L'\r') line.pop_back();
    if (Trim(line).empty()) continue;
    if (!SplitCsvLine(line, fields) || fields.size() < 2) {
      error = L"祝日CSVの形式が不正です（行 " + std::to_wstring(line_number) + L"）。"; return false;
    }
    if (line_number == 1 && (fields[0].find(L"日") != std::wstring::npos ||
                             fields[0].find(L"date") != std::wstring::npos)) continue;
    int year{}, month{}, day{};
    if (!ParseDate(fields[0], year, month, day) || fields[1].empty() || fields[1].size() > 128) {
      error = L"祝日CSVの日付または名称が不正です（行 " + std::to_wstring(line_number) + L"）。"; return false;
    }
    if (!seen.emplace(year, month, day).second) {
      error = L"祝日CSVに重複日付があります（行 " + std::to_wstring(line_number) + L"）。"; return false;
    }
    parsed.push_back({year, month, day, fields[1]});
  }
  if (parsed.empty()) { error = L"祝日CSVに有効なデータがありません。"; return false; }
  int first = parsed.front().year, last = parsed.front().year;
  for (const auto& holiday : parsed) {
    first = std::min(first, holiday.year);
    last = std::max(last, holiday.year);
  }
  info.records = parsed.size();
  info.first_year = first;
  info.last_year = last;
  return true;
}

}  // namespace

std::optional<std::wstring_view> JapaneseHolidayName(int year, int month, int day) {
  std::scoped_lock lock(imported_mutex);
  for (const auto& holiday : imported_holidays) {
    if (holiday.year == year && holiday.month == month && holiday.day == day) {
      thread_local std::wstring imported_name;
      imported_name = holiday.name;
      return imported_name;
    }
  }
  for (const auto& holiday : kHolidays) {
    if (holiday.year == year && holiday.month == month && holiday.day == day) return holiday.name;
  }
  return std::nullopt;
}

bool JapaneseHolidayYearSupported(int year) {
  return year >= JapaneseHolidayFirstYear() && year <= JapaneseHolidayLastYear();
}

int JapaneseHolidayFirstYear() noexcept {
  std::scoped_lock lock(imported_mutex);
  int result = 2025;
  for (const auto& holiday : imported_holidays) result = std::min(result, holiday.year);
  return result;
}

int JapaneseHolidayLastYear() noexcept {
  std::scoped_lock lock(imported_mutex);
  int result = 2027;
  for (const auto& holiday : imported_holidays) result = std::max(result, holiday.year);
  return result;
}

bool ValidateJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                                std::wstring& error) {
  std::vector<ImportedHoliday> parsed;
  return ParseCsvRecords(csv, parsed, info, error);
}

bool ImportJapaneseHolidayCsv(std::wstring_view csv, JapaneseHolidayImportInfo& info,
                              std::wstring& error) {
  std::vector<ImportedHoliday> parsed;
  if (!ParseCsvRecords(csv, parsed, info, error)) return false;
  {
    std::scoped_lock lock(imported_mutex);
    imported_holidays = std::move(parsed);
  }
  return true;
}

bool ImportJapaneseHolidayCsvFile(const std::filesystem::path& path,
                                  JapaneseHolidayImportInfo& info, std::wstring& error) {
  std::ifstream input(path, std::ios::binary);
  if (!input) { error = L"祝日CSVを開けません: " + path.wstring(); return false; }
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring csv;
  if (!DecodeCsv(bytes, csv)) { error = L"祝日CSVの文字コードを判別できません。"; return false; }
  return ImportJapaneseHolidayCsv(csv, info, error);
}

void ClearImportedJapaneseHolidays() noexcept {
  std::scoped_lock lock(imported_mutex);
  imported_holidays.clear();
}

}  // namespace mdlite
