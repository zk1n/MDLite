#include "search/Replace.h"

#include "core/Document.h"

#include <windows.h>

#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cstddef>
#include <cstdint>
#include <cstring>
#include <fstream>
#include <iterator>
#include <limits>
#include <set>
#include <sstream>
#include <system_error>

namespace mdlite {
namespace {

std::wstring Fold(std::wstring_view value) {
  std::wstring folded(value);
  std::ranges::transform(folded, folded.begin(), towlower);
  return folded;
}

std::filesystem::path AbsoluteNormalized(const std::filesystem::path& path) {
  return std::filesystem::absolute(path).lexically_normal();
}

bool EqualsIgnoreCase(std::wstring_view left, std::wstring_view right) {
  return left.size() == right.size() &&
         std::equal(left.begin(), left.end(), right.begin(), [](wchar_t a, wchar_t b) {
           return towlower(a) == towlower(b);
         });
}

bool IsWithin(const std::filesystem::path& root, const std::filesystem::path& path) {
  const auto normalized_root = AbsoluteNormalized(root);
  const auto normalized_path = AbsoluteNormalized(path);
  auto root_part = normalized_root.begin();
  auto path_part = normalized_path.begin();
  for (; root_part != normalized_root.end(); ++root_part, ++path_part) {
    if (path_part == normalized_path.end() ||
        !EqualsIgnoreCase(root_part->wstring(), path_part->wstring()))
      return false;
  }
  return true;
}

std::wstring PathKey(const std::filesystem::path& path) {
  auto key = AbsoluteNormalized(path).generic_wstring();
  std::ranges::transform(key, key.begin(), towlower);
  return key;
}

bool IsProtectedTarget(const std::filesystem::path& relative) {
  for (const auto& part : relative) {
    const auto name = part.wstring();
    if (EqualsIgnoreCase(name, L".git") || EqualsIgnoreCase(name, L".mdlite") ||
        EqualsIgnoreCase(name, L".cache") || EqualsIgnoreCase(name, L".state") ||
        EqualsIgnoreCase(name, L"build") || EqualsIgnoreCase(name, L"out"))
      return true;
  }
  return false;
}

bool IsSafeWorkspaceTarget(const std::filesystem::path& root,
                           const std::filesystem::path& candidate, bool allow_missing) {
  if (!candidate.is_absolute() || !IsWithin(root, candidate)) return false;
  const auto normalized_root = AbsoluteNormalized(root);
  const auto normalized_candidate = AbsoluteNormalized(candidate);
  const auto relative = normalized_candidate.lexically_relative(normalized_root);
  if (relative.empty() || relative.is_absolute() || IsProtectedTarget(relative))
    return false;

  std::filesystem::path current = normalized_root;
  auto component = relative.begin();
  for (; component != relative.end(); ++component) {
    current /= *component;
    const bool last = std::next(component) == relative.end();
    const DWORD attributes = GetFileAttributesW(current.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES) {
      const DWORD failure = GetLastError();
      if (last && allow_missing && (failure == ERROR_FILE_NOT_FOUND || failure == ERROR_PATH_NOT_FOUND))
        return true;
      return false;
    }
    if ((attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) return false;
    const bool directory = (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
    if ((!last && !directory) || (last && directory)) return false;
  }
  return true;
}

bool EnsureJournalDirectory(const std::filesystem::path& workspace,
                            const std::filesystem::path& directory, std::wstring& error) {
  const auto root = AbsoluteNormalized(workspace);
  const auto target = AbsoluteNormalized(directory);
  if (!IsWithin(root, target)) {
    error = L"置換journal領域がWorkspace外です。";
    return false;
  }
  auto current = root;
  const auto relative = target.lexically_relative(root);
  for (const auto& component : relative) {
    current /= component;
    DWORD attributes = GetFileAttributesW(current.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES) {
      const DWORD failure = GetLastError();
      if (failure != ERROR_FILE_NOT_FOUND && failure != ERROR_PATH_NOT_FOUND) {
        error = L"置換journal領域の状態を確認できません。";
        return false;
      }
      if (!CreateDirectoryW(current.c_str(), nullptr) && GetLastError() != ERROR_ALREADY_EXISTS) {
        error = L"置換journal領域を作成できません。";
        return false;
      }
      attributes = GetFileAttributesW(current.c_str());
    }
    if (attributes == INVALID_FILE_ATTRIBUTES || (attributes & FILE_ATTRIBUTE_DIRECTORY) == 0 ||
        (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
      error = L"置換journal領域に非ディレクトリまたはreparse pointがあります。";
      return false;
    }
  }
  return true;
}

bool AppendBounded(std::wstring& output, std::wstring_view value, std::wstring& error) {
  constexpr std::size_t max_characters = kSearchMaxFileBytes / sizeof(wchar_t);
  if (value.size() > max_characters - std::min(output.size(), max_characters)) {
    error = L"置換後の文書が上限（8 MiB相当のUTF-16本文）を超えます。";
    return false;
  }
  output.append(value);
  return true;
}

bool FormatRegexReplacement(std::wstring_view source, const SearchRegexMatch& match,
                           std::wstring_view replacement, std::wstring& output,
                           std::wstring& error) {
  output.clear();
  for (std::size_t index = 0; index < replacement.size(); ++index) {
    if (replacement[index] != L'$' || index + 1 >= replacement.size()) {
      if (!AppendBounded(output, replacement.substr(index, 1), error)) return false;
      continue;
    }
    const wchar_t token = replacement[index + 1];
    if (token == L'$') {
      if (!AppendBounded(output, L"$", error)) return false;
      ++index;
    } else if (token == L'&' || token == L'0') {
      if (!AppendBounded(output, source.substr(match.begin, match.end - match.begin), error))
        return false;
      ++index;
    } else if (token == L'`') {
      if (!AppendBounded(output, source.substr(0, match.begin), error)) return false;
      ++index;
    } else if (token == L'\'') {
      if (!AppendBounded(output, source.substr(match.end), error)) return false;
      ++index;
    } else if (token >= L'1' && token <= L'9') {
      const std::size_t capture = static_cast<std::size_t>(token - L'0');
      if (capture < match.captures.size() && match.captures[capture].matched) {
        const auto& span = match.captures[capture];
        if (!AppendBounded(output, source.substr(span.begin, span.end - span.begin), error))
          return false;
      }
      ++index;
    } else {
      if (!AppendBounded(output, L"$", error)) return false;
    }
  }
  return true;
}

bool ReplaceText(std::wstring_view source, const SearchQuery& query, std::wstring_view replacement,
                 std::wstring& output, std::size_t& count, std::wstring& error,
                 const std::function<bool()>& cancelled = {}) {
  count = 0;
  output.clear();
  error.clear();
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (source.size() > kSearchMaxFileBytes / sizeof(wchar_t)) {
    error = L"文書サイズ上限（8 MiB）を超えるため置換できません。";
    return false;
  }
  if (query.regular_expression) {
    std::vector<SearchRegexMatch> matches;
    bool truncated{};
    if (!SearchRegexMatches(source, query, matches, error, cancelled, kSearchMaxMatches,
                            &truncated))
      return false;
    if (truncated) {
      error = L"置換件数が上限（20,000件）を超えます。";
      return false;
    }
    std::size_t cursor{};
    for (const auto& match : matches) {
      std::wstring formatted;
      if (!FormatRegexReplacement(source, match, replacement, formatted, error) ||
          !AppendBounded(output, source.substr(cursor, match.begin - cursor), error) ||
          !AppendBounded(output, formatted, error))
        return false;
      cursor = match.end;
      ++count;
    }
    return AppendBounded(output, source.substr(cursor), error);
  }
  const std::wstring haystack = query.match_case ? std::wstring(source) : Fold(source);
  const std::wstring needle = query.match_case ? query.text : Fold(query.text);
  std::size_t scan{};
  std::size_t emitted{};
  while (scan < source.size()) {
    if (cancelled && cancelled()) {
      error = L"置換previewを中止しました。";
      return false;
    }
    const std::size_t found = haystack.find(needle, scan);
    if (found == std::wstring::npos) break;
    const std::size_t finish = found + needle.size();
    if (query.whole_word && !IsSearchWholeWordMatch(source, found, finish)) {
      scan = found + 1;
      continue;
    }
    if (count == kSearchMaxMatches) {
      error = L"置換件数が上限（20,000件）を超えます。";
      return false;
    }
    if (!AppendBounded(output, source.substr(emitted, found - emitted), error) ||
        !AppendBounded(output, replacement, error)) return false;
    scan = finish;
    emitted = finish;
    ++count;
  }
  if (count == 0) return AppendBounded(output, source, error);
  return AppendBounded(output, source.substr(emitted), error);
}

bool ReplaceDocumentMatchImpl(std::wstring_view source, const SearchQuery& query,
                              std::wstring_view replacement, std::size_t start,
                              std::optional<std::size_t> selected_source_end,
                              std::wstring& output, std::size_t& replaced_begin,
                              std::size_t& replaced_end, bool& replaced, std::wstring& error) {
  output.clear();
  error.clear();
  replaced = false;
  replaced_begin = replaced_end = std::min(start, source.size());
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (source.size() > kSearchMaxFileBytes / sizeof(wchar_t)) {
    error = L"文書サイズ上限（8 MiB）を超えるため置換できません。";
    return false;
  }
  if (selected_source_end &&
      (start > source.size() || *selected_source_end < start || *selected_source_end > source.size())) {
    error = L"選択した一致が元の検索結果と一致しません。";
    return false;
  }
  start = std::min(start, source.size());
  const auto write_replacement = [&](std::size_t begin, std::size_t end,
                                     std::wstring_view value) {
    if (!AppendBounded(output, source.substr(0, begin), error) ||
        !AppendBounded(output, value, error) ||
        !AppendBounded(output, source.substr(end), error))
      return false;
    replaced_begin = begin;
    replaced_end = begin + value.size();
    replaced = true;
    return true;
  };
  const auto no_exact_match = [&] {
    error = L"選択した一致が元の検索結果と一致しません。";
    return false;
  };

  if (query.regular_expression) {
    std::vector<SearchRegexMatch> matches;
    bool truncated{};
    const std::size_t match_limit = selected_source_end ? 2 : 1;
    if (!SearchRegexMatches(source, query, matches, error, {}, match_limit, &truncated,
                            start, !selected_source_end))
      return false;
    if (selected_source_end) {
      const auto match = std::ranges::find_if(matches, [&](const SearchRegexMatch& candidate) {
        return candidate.begin == start && candidate.end == *selected_source_end;
      });
      if (match == matches.end()) return no_exact_match();
      std::wstring formatted;
      if (!FormatRegexReplacement(source, *match, replacement, formatted, error)) return false;
      return write_replacement(match->begin, match->end, formatted);
    }
    if (matches.empty()) return AppendBounded(output, source, error);
    std::wstring formatted;
    if (!FormatRegexReplacement(source, matches.front(), replacement, formatted, error)) return false;
    return write_replacement(matches.front().begin, matches.front().end, formatted);
  }

  const std::wstring haystack = query.match_case ? std::wstring(source) : Fold(source);
  const std::wstring needle = query.match_case ? query.text : Fold(query.text);
  if (selected_source_end) {
    const std::size_t found = haystack.find(needle, start);
    if (found != start || found == std::wstring::npos || found + needle.size() != *selected_source_end ||
        (query.whole_word && !IsSearchWholeWordMatch(source, found, *selected_source_end)))
      return no_exact_match();
    return write_replacement(found, *selected_source_end, replacement);
  }
  for (std::size_t found = haystack.find(needle, start); found != std::wstring::npos;
       found = haystack.find(needle, found + 1)) {
    const std::size_t finish = found + needle.size();
    if (query.whole_word && !IsSearchWholeWordMatch(source, found, finish)) continue;
    return write_replacement(found, finish, replacement);
  }
  return AppendBounded(output, source, error);
}

bool DecodeUtf8(const std::string& bytes, std::wstring& text) {
  if (bytes.empty()) { text.clear(); return true; }
  const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                       static_cast<int>(bytes.size()), nullptr, 0);
  if (size <= 0) return false;
  text.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                             static_cast<int>(bytes.size()), text.data(), size) == size;
}

bool Unhex(std::wstring_view value, std::wstring& output) {
  if (value.size() % 2 != 0) return false;
  auto digit = [](wchar_t c) -> int {
    if (c >= L'0' && c <= L'9') return c - L'0';
    if (c >= L'A' && c <= L'F') return c - L'A' + 10;
    return -1;
  };
  std::string bytes(value.size() / 2, '\0');
  for (std::size_t i = 0; i < bytes.size(); ++i) {
    const int high = digit(value[i * 2]);
    const int low = digit(value[i * 2 + 1]);
    if (high < 0 || low < 0) return false;
    bytes[i] = static_cast<char>((high << 4) | low);
  }
  return DecodeUtf8(bytes, output);
}

bool WriteAll(HANDLE file, const void* data, std::size_t size) {
  const auto* bytes = static_cast<const std::byte*>(data);
  while (size > 0) {
    const DWORD chunk = static_cast<DWORD>(std::min<std::size_t>(size, 1U << 20U));
    DWORD written{};
    if (!WriteFile(file, bytes, chunk, &written, nullptr) || written == 0) return false;
    bytes += written;
    size -= written;
  }
  return true;
}

bool WriteJournal(const std::filesystem::path& directory, const ReplacePlan& plan,
                  std::filesystem::path& path, std::wstring& error) {
  static_assert(sizeof(wchar_t) == 2);
  static std::atomic<std::uint64_t> sequence{};
  const auto stamp = static_cast<std::uint64_t>(
      std::chrono::steady_clock::now().time_since_epoch().count());
  HANDLE file = INVALID_HANDLE_VALUE;
  for (std::size_t attempt = 0; attempt < 10; ++attempt) {
    const auto name = L"replace-" + std::to_wstring(stamp) + L"-" +
                      std::to_wstring(GetCurrentProcessId()) + L"-" +
                      std::to_wstring(sequence.fetch_add(1)) + L".journal";
    path = directory / name;
    file = CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                       FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file != INVALID_HANDLE_VALUE) break;
    if (GetLastError() != ERROR_FILE_EXISTS && GetLastError() != ERROR_ALREADY_EXISTS) break;
  }
  if (file == INVALID_HANDLE_VALUE) {
    error = L"置換journalを一意な名前で作成できません。";
    return false;
  }

  struct HandleGuard {
    HANDLE value;
    ~HandleGuard() { if (value != INVALID_HANDLE_VALUE) CloseHandle(value); }
  } guard{file};
  constexpr char magic[] = "MDLITE_REPLACE_JOURNAL_V2\r\n";
  const std::uint64_t count = static_cast<std::uint64_t>(plan.files.size());
  bool ok = WriteAll(file, magic, sizeof(magic) - 1) && WriteAll(file, &count, sizeof(count));
  for (const auto& item : plan.files) {
    const std::wstring full_path = AbsoluteNormalized(item.path).wstring();
    const std::uint64_t path_length = static_cast<std::uint64_t>(full_path.size());
    const std::uint64_t before_length = static_cast<std::uint64_t>(item.before.size());
    const std::uint64_t after_length = static_cast<std::uint64_t>(item.after.size());
    ok = ok && WriteAll(file, &path_length, sizeof(path_length)) &&
         WriteAll(file, &before_length, sizeof(before_length)) &&
         WriteAll(file, &after_length, sizeof(after_length)) &&
         WriteAll(file, full_path.data(), full_path.size() * sizeof(wchar_t)) &&
         WriteAll(file, item.before.data(), item.before.size() * sizeof(wchar_t)) &&
         WriteAll(file, item.after.data(), item.after.size() * sizeof(wchar_t));
    if (!ok) break;
  }
  if (!ok || !FlushFileBuffers(file)) {
    error = L"置換journalを書き込めません。変更はまだ適用していません。";
    guard.value = INVALID_HANDLE_VALUE;
    CloseHandle(file);
    DeleteFileW(path.c_str());
    return false;
  }
  return true;
}

struct JournalEntry {
  std::filesystem::path path;
  std::wstring before;
  std::wstring after;
};

bool WorkspaceForJournal(const std::filesystem::path& journal,
                         std::filesystem::path& workspace) {
  const auto normalized = AbsoluteNormalized(journal);
  const auto replace_directory = normalized.parent_path();
  const auto state_directory = replace_directory.parent_path();
  const auto metadata_directory = state_directory.parent_path();
  if (!EqualsIgnoreCase(replace_directory.filename().wstring(), L"replace") ||
      !EqualsIgnoreCase(state_directory.filename().wstring(), L".state") ||
      !EqualsIgnoreCase(metadata_directory.filename().wstring(), L".mdlite") ||
      !EqualsIgnoreCase(normalized.extension().wstring(), L".journal"))
    return false;
  workspace = metadata_directory.parent_path();
  return !workspace.empty();
}

bool IsSafeJournalFile(const std::filesystem::path& workspace,
                       const std::filesystem::path& journal, bool allow_missing) {
  const auto normalized = AbsoluteNormalized(journal);
  if (!IsWithin(workspace, normalized)) return false;
  const auto relative = normalized.lexically_relative(AbsoluteNormalized(workspace));
  std::vector<std::wstring> components;
  for (const auto& component : relative) components.push_back(component.wstring());
  if (components.size() != 4 || !EqualsIgnoreCase(components[0], L".mdlite") ||
      !EqualsIgnoreCase(components[1], L".state") ||
      !EqualsIgnoreCase(components[2], L"replace") ||
      !EqualsIgnoreCase(std::filesystem::path(components[3]).extension().wstring(), L".journal"))
    return false;
  std::filesystem::path current = AbsoluteNormalized(workspace);
  for (std::size_t index = 0; index < components.size(); ++index) {
    current /= components[index];
    const DWORD attributes = GetFileAttributesW(current.c_str());
    const bool last = index + 1 == components.size();
    if (attributes == INVALID_FILE_ATTRIBUTES) {
      const DWORD failure = GetLastError();
      if (last && allow_missing && (failure == ERROR_FILE_NOT_FOUND || failure == ERROR_PATH_NOT_FOUND))
        return true;
      return false;
    }
    if ((attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) return false;
    const bool directory = (attributes & FILE_ATTRIBUTE_DIRECTORY) != 0;
    if ((!last && !directory) || (last && directory)) return false;
  }
  return true;
}

bool ReadJournal(const std::filesystem::path& journal, const std::filesystem::path& workspace,
                 std::vector<JournalEntry>& entries, std::wstring& error) {
  entries.clear();
  if (!IsSafeJournalFile(workspace, journal, false)) {
    error = L"置換journalがWorkspaceの安全な状態領域にありません。";
    return false;
  }
  std::error_code size_error;
  const auto file_size = std::filesystem::file_size(journal, size_error);
  if (size_error || file_size > 128ULL * 1024ULL * 1024ULL) {
    error = L"置換journalのサイズが不正です。";
    return false;
  }

  std::ifstream input(journal, std::ios::binary);
  if (!input) {
    error = L"置換journalを開けません。";
    return false;
  }
  constexpr char v2_magic[] = "MDLITE_REPLACE_JOURNAL_V2\r\n";
  std::array<char, sizeof(v2_magic) - 1> magic{};
  input.read(magic.data(), static_cast<std::streamsize>(magic.size()));
  const bool is_v2 = input.gcount() == static_cast<std::streamsize>(magic.size()) &&
                     std::equal(magic.begin(), magic.end(), v2_magic);
  input.clear();
  input.seekg(0);

  if (is_v2) {
    if (file_size > kSearchMaxReplacePlanBytes + kSearchMaxMatches * 24ULL + 1024ULL) {
      error = L"置換journalのサイズが保存上限を超えています。";
      return false;
    }
    static_assert(sizeof(wchar_t) == 2);
    input.seekg(static_cast<std::streamoff>(sizeof(v2_magic) - 1));
    std::uint64_t count{};
    if (!input.read(reinterpret_cast<char*>(&count), sizeof(count)) || count > kSearchMaxMatches) {
      error = L"置換journal形式が壊れています。";
      return false;
    }
    std::uint64_t data_size{};
    entries.reserve(static_cast<std::size_t>(count));
    for (std::uint64_t index = 0; index < count; ++index) {
      std::uint64_t path_length{}, before_length{}, after_length{};
      if (!input.read(reinterpret_cast<char*>(&path_length), sizeof(path_length)) ||
          !input.read(reinterpret_cast<char*>(&before_length), sizeof(before_length)) ||
          !input.read(reinterpret_cast<char*>(&after_length), sizeof(after_length)) ||
          path_length == 0 || path_length > 32767 ||
          before_length > kSearchMaxReplacePlanBytes / sizeof(wchar_t) ||
          after_length > kSearchMaxReplacePlanBytes / sizeof(wchar_t)) {
        error = L"置換journal形式が壊れています。";
        return false;
      }
      const std::uint64_t item_size = (path_length + before_length + after_length) * sizeof(wchar_t);
      if (item_size > kSearchMaxReplacePlanBytes - data_size) {
        error = L"置換journalが保存上限を超えています。";
        return false;
      }
      data_size += item_size;
      JournalEntry entry;
      std::wstring path_text(static_cast<std::size_t>(path_length), L'\0');
      entry.before.resize(static_cast<std::size_t>(before_length));
      entry.after.resize(static_cast<std::size_t>(after_length));
      if (!input.read(reinterpret_cast<char*>(path_text.data()),
                      static_cast<std::streamsize>(path_text.size() * sizeof(wchar_t))) ||
          !input.read(reinterpret_cast<char*>(entry.before.data()),
                      static_cast<std::streamsize>(entry.before.size() * sizeof(wchar_t))) ||
          !input.read(reinterpret_cast<char*>(entry.after.data()),
                      static_cast<std::streamsize>(entry.after.size() * sizeof(wchar_t)))) {
        error = L"置換journalの本文が途中で切れています。";
        return false;
      }
      if (path_text.find(L'\0') != std::wstring::npos) {
        error = L"置換journalに不正なpath文字があります。";
        return false;
      }
      entry.path = std::filesystem::path(path_text);
      entries.push_back(std::move(entry));
    }
    if (input.peek() != std::char_traits<char>::eof()) {
      error = L"置換journalに余分なデータがあります。";
      return false;
    }
  } else {
    if (file_size > 128ULL * 1024ULL * 1024ULL) {
      error = L"旧形式の置換journalが読み込み上限を超えています。";
      return false;
    }
    const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
    std::wstring text;
    if (!DecodeUtf8(bytes, text)) {
      error = L"置換journalのUTF-8が壊れています。";
      return false;
    }
    std::wistringstream lines(text);
    std::wstring line;
    if (!std::getline(lines, line) || line != L"MDLITE_REPLACE_JOURNAL_V1") {
      error = L"置換journal形式が一致しません。";
      return false;
    }
    std::uint64_t data_size{};
    while (std::getline(lines, line)) {
      const auto first = line.find(L'\t');
      const auto second = first == std::wstring::npos ? first : line.find(L'\t', first + 1);
      if (first == std::wstring::npos || second == std::wstring::npos) {
        error = L"旧形式の置換journalに不正な行があります。";
        return false;
      }
      JournalEntry entry;
      std::wstring path_text;
      if (!Unhex(line.substr(0, first), path_text) ||
          !Unhex(line.substr(first + 1, second - first - 1), entry.before) ||
          !Unhex(line.substr(second + 1), entry.after)) {
        error = L"旧形式の置換journalのhexデータが壊れています。";
        return false;
      }
      if (path_text.empty() || entry.before.size() > kSearchMaxReplacePlanBytes / sizeof(wchar_t) ||
          entry.after.size() > kSearchMaxReplacePlanBytes / sizeof(wchar_t)) {
        error = L"旧形式の置換journalがサイズ上限を超えています。";
        return false;
      }
      entry.path = std::filesystem::path(path_text);
      data_size += static_cast<std::uint64_t>(entry.path.native().size() +
                                               entry.before.size() + entry.after.size()) * sizeof(wchar_t);
      if (data_size > kSearchMaxReplacePlanBytes || entries.size() >= kSearchMaxMatches) {
        error = L"旧形式の置換journalが保存上限を超えています。";
        return false;
      }
      entries.push_back(std::move(entry));
    }
  }

  std::set<std::wstring> seen_paths;
  for (const auto& entry : entries) {
    if (!entry.path.is_absolute() || !IsSafeWorkspaceTarget(workspace, entry.path, true)) {
      error = L"置換journalにWorkspace外または保護対象のpathがあります。";
      return false;
    }
    auto key = AbsoluteNormalized(entry.path).generic_wstring();
    std::ranges::transform(key, key.begin(), towlower);
    if (!seen_paths.insert(std::move(key)).second) {
      error = L"置換journalに重複したpathがあります。";
      return false;
    }
  }
  return true;
}

}  // namespace

bool ReplaceDocumentText(std::wstring_view source, const SearchQuery& query,
                         std::wstring_view replacement, std::wstring& output,
                         std::size_t& count, std::wstring& error) {
  return ReplaceText(source, query, replacement, output, count, error);
}

bool ReplaceDocumentMatch(std::wstring_view source, const SearchQuery& query,
                          std::wstring_view replacement, std::size_t start,
                          std::wstring& output, std::size_t& replaced_begin,
                          std::size_t& replaced_end, bool& replaced, std::wstring& error) {
  return ReplaceDocumentMatchImpl(source, query, replacement, start, std::nullopt, output,
                                  replaced_begin, replaced_end, replaced, error);
}

bool ReplaceDocumentMatch(std::wstring_view source, const SearchQuery& query,
                          std::wstring_view replacement, std::size_t selected_begin,
                          std::size_t selected_source_end, std::wstring& output,
                          std::size_t& replaced_begin, std::size_t& replaced_end,
                          bool& replaced, std::wstring& error) {
  return ReplaceDocumentMatchImpl(source, query, replacement, selected_begin,
                                  selected_source_end, output, replaced_begin, replaced_end,
                                  replaced, error);
}

bool PreviewWorkspaceReplace(const std::filesystem::path& root, const SearchQuery& query,
                             std::wstring_view replacement,
                             const std::map<std::filesystem::path, std::wstring>& unsaved,
                             ReplacePlan& plan, std::wstring& error,
                             const std::function<bool()>& cancelled) {
  plan = {query, std::wstring(replacement), {}, {}};
  error.clear();
  if (root.empty()) {
    error = L"Workspaceが選択されていません。";
    return false;
  }
  std::vector<SearchMatch> matches;
  if (!SearchWorkspace(root, query, unsaved, matches, error, cancelled, {},
                       &plan.search_issues, {}))
    return false;
  const auto blocking_issue = std::ranges::find_if(plan.search_issues, [](const SearchIssue& issue) {
    return issue.kind != SearchIssueKind::Excluded;
  });
  if (blocking_issue != plan.search_issues.end()) {
    error = L"Workspace全体の置換previewを作れません。一部の対象を検索できません: " +
            blocking_issue->message;
    return false;
  }
  struct ExpectedFile { std::size_t count{}; std::uint64_t snapshot_hash{}; };
  std::map<std::filesystem::path, ExpectedFile> expected_files;
  for (const auto& match : matches) {
    auto [entry, inserted] = expected_files.try_emplace(
        AbsoluteNormalized(match.path), ExpectedFile{0, match.snapshot_hash});
    if (!inserted && entry->second.snapshot_hash != match.snapshot_hash) {
      error = L"検索結果に同じファイルの異なる本文versionが混在しています。";
      return false;
    }
    ++entry->second.count;
  }

  std::map<std::wstring, const std::wstring*> unsaved_by_path;
  for (const auto& [path, text] : unsaved)
    unsaved_by_path.insert_or_assign(PathKey(path), &text);
  ReplacePlan prepared{query, std::wstring(replacement), {}};
  std::uint64_t total_plan_bytes{};
  for (const auto& [path, expected] : expected_files) {
    if (cancelled && cancelled()) {
      error = L"置換previewを中止しました。";
      return false;
    }
    std::wstring before;
    const auto unsaved_it = unsaved_by_path.find(PathKey(path));
    if (unsaved_it != unsaved_by_path.end()) {
      before = *unsaved_it->second;
    } else {
      if (!IsSafeWorkspaceTarget(root, path, false)) {
        error = L"置換対象がWorkspace外、保護領域内、またはreparse pointです。";
        return false;
      }
      std::error_code size_error;
      const auto file_size = std::filesystem::file_size(path, size_error);
      if (size_error || file_size > kSearchMaxFileBytes) {
        error = L"置換preview中に対象ファイルのサイズを安全に確認できません。";
        return false;
      }
      Document document;
      if (!document.Load(path, error)) return false;
      before = document.text();
    }
    if (SearchSnapshotHash(before) != expected.snapshot_hash) {
      error = L"検索後に本文が変わったため、置換previewを破棄しました。";
      return false;
    }
    std::wstring after;
    std::size_t actual_count{};
    if (!ReplaceText(before, query, replacement, after, actual_count, error, cancelled)) return false;
    if (actual_count != expected.count) {
      error = L"置換preview中に検索結果が変化しました。";
      return false;
    }
    const auto file_bytes = static_cast<std::uint64_t>(path.native().size() + before.size() + after.size()) *
                            sizeof(wchar_t);
    if (file_bytes > kSearchMaxReplacePlanBytes - total_plan_bytes) {
      error = L"Workspace置換previewの総量が上限（64 MiB）を超えます。";
      return false;
    }
    total_plan_bytes += file_bytes;
    prepared.files.push_back({path, std::move(before), std::move(after), actual_count});
  }
  prepared.search_issues = std::move(plan.search_issues);
  plan = std::move(prepared);
  return true;
}

bool ApplyWorkspaceReplace(const std::filesystem::path& workspace, const ReplacePlan& plan,
                           ReplaceApplyResult& result, std::wstring& error,
                           const std::function<bool()>& cancelled) {
  result = {};
  error.clear();
  if (workspace.empty()) {
    error = L"Workspaceが選択されていません。";
    return false;
  }
  if (plan.files.empty()) return true;
  if (plan.files.size() > kSearchMaxMatches) {
    error = L"置換previewのファイル数が上限を超えています。";
    return false;
  }
  const auto absolute_workspace = AbsoluteNormalized(workspace);
  const auto journal_directory = absolute_workspace / L".mdlite/.state/replace";
  std::uint64_t total_plan_bytes{};
  std::size_t total_replacements{};
  std::set<std::wstring> targets;
  for (const auto& item : plan.files) {
    const auto path = AbsoluteNormalized(item.path);
    if (!IsSafeWorkspaceTarget(absolute_workspace, path, false)) {
      error = L"置換対象がWorkspace外、保護領域内、またはreparse pointです。";
      return false;
    }
    if (item.before.size() > kSearchMaxFileBytes / sizeof(wchar_t) ||
        item.after.size() > kSearchMaxFileBytes / sizeof(wchar_t) ||
        item.replacement_count == 0 || item.replacement_count > kSearchMaxMatches ||
        total_replacements > kSearchMaxMatches - item.replacement_count) {
      error = L"置換previewの内容または件数が上限を超えています。";
      return false;
    }
    const auto item_bytes = static_cast<std::uint64_t>(path.native().size() +
        item.before.size() + item.after.size()) * sizeof(wchar_t);
    if (item_bytes > kSearchMaxReplacePlanBytes - total_plan_bytes) {
      error = L"置換previewの総量が上限（64 MiB）を超えています。";
      return false;
    }
    std::wstring verified_after;
    std::size_t verified_count{};
    std::wstring validation_error;
    if (item.selected_match) {
      const auto& selected = *item.selected_match;
      if (plan.files.size() != 1 || item.replacement_count != 1 ||
          SearchSnapshotHash(item.before) != selected.snapshot_hash) {
        error = L"選択置換previewが元の検索結果と一致しません。";
        return false;
      }
      std::vector<SearchMatch> source_matches;
      if (!SearchDocumentText(path, item.before, plan.query, source_matches,
                              validation_error)) {
        error = validation_error.empty() ? L"選択置換の検索結果を検証できません。"
                                         : validation_error;
        return false;
      }
      const auto selected_match = std::ranges::find_if(source_matches, [&](const SearchMatch& match) {
        return match.begin == selected.begin && match.end == selected.end &&
               match.snapshot_hash == selected.snapshot_hash;
      });
      if (selected_match == source_matches.end()) {
        error = L"選択した一致が元の検索結果と一致しません。";
        return false;
      }
      std::size_t replaced_begin{}, replaced_end{};
      bool replaced{};
      if (!ReplaceDocumentMatch(item.before, plan.query, plan.replacement,
                                selected.begin, selected.end, verified_after, replaced_begin,
                                replaced_end, replaced, validation_error) ||
          !replaced || replaced_begin != selected.begin) {
        error = validation_error.empty() ? L"選択した一致を置換できません。"
                                         : validation_error;
        return false;
      }
      verified_count = 1;
    } else if (!ReplaceText(item.before, plan.query, plan.replacement, verified_after,
                            verified_count, validation_error)) {
      error = validation_error;
      return false;
    }
    if (verified_after != item.after || verified_count != item.replacement_count) {
      error = validation_error.empty()
                  ? L"置換previewの内容と検索条件が一致しません。"
                  : validation_error;
      return false;
    }
    total_plan_bytes += item_bytes;
    total_replacements += item.replacement_count;
    if (!targets.insert(PathKey(path)).second) {
      error = L"置換previewに重複するファイルがあります。";
      return false;
    }
  }
  if (!EnsureJournalDirectory(absolute_workspace, journal_directory, error) ||
      !WriteJournal(journal_directory, plan, result.journal, error))
    return false;

  for (std::size_t index = 0; index < plan.files.size(); ++index) {
    if (cancelled && cancelled()) {
      error = L"置換適用を中止しました。journalから適用済みファイルを条件付きで戻せます。";
      return false;
    }
    const auto& item = plan.files[index];
    if (!IsSafeWorkspaceTarget(absolute_workspace, item.path, false)) {
      result.conflicts.push_back(item.path);
      continue;
    }
    std::error_code size_error;
    const auto file_size = std::filesystem::file_size(item.path, size_error);
    if (size_error || file_size > 16ULL * 1024ULL * 1024ULL) {
      result.conflicts.push_back(item.path);
      continue;
    }
    Document document;
    std::wstring item_error;
    if (!document.Load(item.path, item_error) || document.text() != item.before) {
      result.conflicts.push_back(item.path);
      continue;
    }
    document.MarkEdited(item.after);
    if (!document.Save(item_error)) {
      result.conflicts.push_back(item.path);
      continue;
    }
    ++result.applied_files;
    result.applied_replacements += item.replacement_count;
  }
  return result.conflicts.empty();
}

bool RollbackWorkspaceReplace(const std::filesystem::path& journal,
                              ReplaceApplyResult& result, std::wstring& error) {
  result = {};
  error.clear();
  std::filesystem::path workspace;
  if (!WorkspaceForJournal(journal, workspace)) {
    error = L"置換journalのWorkspaceを特定できません。";
    return false;
  }
  std::vector<JournalEntry> entries;
  if (!ReadJournal(journal, workspace, entries, error)) return false;
  result.journal = AbsoluteNormalized(journal);
  for (const auto& entry : entries) {
    const auto& path = entry.path;
    if (!IsSafeWorkspaceTarget(workspace, path, true)) {
      result.conflicts.push_back(path);
      continue;
    }
    std::error_code size_error;
    const auto file_size = std::filesystem::file_size(path, size_error);
    if (size_error || file_size > 16ULL * 1024ULL * 1024ULL) {
      result.conflicts.push_back(path);
      continue;
    }
    Document document;
    std::wstring item_error;
    if (!document.Load(path, item_error)) {
      result.conflicts.push_back(path);
      continue;
    }
    if (document.text() == entry.before) continue;
    if (document.text() != entry.after) {
      result.conflicts.push_back(path);
      continue;
    }
    document.MarkEdited(entry.before);
    if (!document.Save(item_error)) { result.conflicts.push_back(path); continue; }
    ++result.applied_files;
  }
  return result.conflicts.empty();
}

bool PreviewWorkspaceReplaceRollback(const std::filesystem::path& current_workspace,
                                    const std::filesystem::path& journal,
                                    ReplaceRollbackPreview& preview,
                                    std::wstring& error) {
  preview = {};
  error.clear();
  if (current_workspace.empty()) {
    error = L"Workspaceが選択されていません。";
    return false;
  }

  std::filesystem::path owning_workspace;
  if (!WorkspaceForJournal(journal, owning_workspace)) {
    error = L"置換journalのWorkspaceを特定できません。";
    return false;
  }
  owning_workspace = AbsoluteNormalized(owning_workspace);
  if (PathKey(current_workspace) != PathKey(owning_workspace)) {
    error = L"置換journalが現在のWorkspaceに属していません。";
    return false;
  }

  std::vector<JournalEntry> entries;
  if (!ReadJournal(journal, owning_workspace, entries, error)) return false;

  ReplaceRollbackPreview prepared{owning_workspace, AbsoluteNormalized(journal), {}};
  for (auto& entry : entries) {
    const auto path = AbsoluteNormalized(entry.path);
    if (!IsSafeWorkspaceTarget(owning_workspace, path, true)) {
      error = L"置換journalの対象がWorkspace外またはreparse pointです。";
      return false;
    }
    std::error_code size_error;
    const auto file_size = std::filesystem::file_size(path, size_error);
    if (size_error || file_size > 16ULL * 1024ULL * 1024ULL) {
      error = L"置換journalの対象fileを安全に確認できません。";
      return false;
    }
    Document document;
    std::wstring item_error;
    if (!document.Load(path, item_error)) {
      error = L"置換journalの対象fileを読み込めません。";
      return false;
    }
    if (document.text() == entry.before) continue;
    if (document.text() != entry.after) {
      error = L"置換後の本文が変化しているためjournal previewを作れません。";
      return false;
    }
    prepared.files.push_back(
        {path, std::move(entry.after), std::move(entry.before)});
  }

  preview = std::move(prepared);
  return true;
}

}  // namespace mdlite
