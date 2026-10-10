#include "search/Search.h"

#define PCRE2_CODE_UNIT_WIDTH 16
#include <pcre2.h>

#include <windows.h>

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <cwctype>
#include <iterator>
#include <limits>
#include <map>
#include <set>
#include <span>
#include <system_error>

namespace mdlite {
namespace {

bool EqualsIgnoreCase(std::wstring_view left, std::wstring_view right) {
  return left.size() == right.size() &&
         std::equal(left.begin(), left.end(), right.begin(), [](wchar_t a, wchar_t b) {
           return towlower(a) == towlower(b);
         });
}

bool IsExcluded(const std::filesystem::path& relative, const SearchQuery& query) {
  for (const auto& part : relative) {
    const auto name = part.wstring();
    if (EqualsIgnoreCase(name, L".git") || EqualsIgnoreCase(name, L".mdlite") ||
        EqualsIgnoreCase(name, L"build") || EqualsIgnoreCase(name, L"out"))
      return true;
    if (EqualsIgnoreCase(name, L".cache") || EqualsIgnoreCase(name, L".state")) return true;
    if (!query.include_hidden && name.size() > 1 && name.front() == L'.') return true;
  }
  return false;
}

std::filesystem::path AbsoluteNormalized(const std::filesystem::path& path) {
  return std::filesystem::absolute(path).lexically_normal();
}

std::wstring PathKey(const std::filesystem::path& path) {
  std::wstring key = AbsoluteNormalized(path).generic_wstring();
  std::ranges::transform(key, key.begin(), towlower);
  return key;
}

bool IsWithin(const std::filesystem::path& root, const std::filesystem::path& candidate) {
  const auto normalized_root = AbsoluteNormalized(root);
  const auto normalized_candidate = AbsoluteNormalized(candidate);
  auto root_part = normalized_root.begin();
  auto candidate_part = normalized_candidate.begin();
  for (; root_part != normalized_root.end(); ++root_part, ++candidate_part) {
    if (candidate_part == normalized_candidate.end() ||
        !EqualsIgnoreCase(root_part->wstring(), candidate_part->wstring()))
      return false;
  }
  return true;
}

bool IsContainedRelative(const std::filesystem::path& relative);

bool IsSafeWorkspacePath(const std::filesystem::path& root,
                         const std::filesystem::path& candidate, bool allow_missing,
                         bool require_regular_file) {
  if (!candidate.is_absolute() || !IsWithin(root, candidate)) return false;
  const auto normalized_root = AbsoluteNormalized(root);
  const auto normalized_candidate = AbsoluteNormalized(candidate);
  const auto relative = normalized_candidate.lexically_relative(normalized_root);
  if (relative.empty() || !IsContainedRelative(relative)) return false;

  std::filesystem::path current = normalized_root;
  auto component = relative.begin();
  for (; component != relative.end(); ++component) {
    current /= *component;
    std::error_code status_error;
    const auto status = std::filesystem::symlink_status(current, status_error);
    const bool last = std::next(component) == relative.end();
    if (status_error == std::errc::no_such_file_or_directory ||
        status.type() == std::filesystem::file_type::not_found) {
      return allow_missing;
    }
    if (status_error || std::filesystem::is_symlink(status)) return false;
    const DWORD attributes = GetFileAttributesW(current.c_str());
    if (attributes == INVALID_FILE_ATTRIBUTES ||
        (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
      return false;
    if (!last && !std::filesystem::is_directory(status)) return false;
    if (last && require_regular_file && !std::filesystem::is_regular_file(status)) return false;
  }
  return true;
}

bool HasNativeHiddenAttributeInPath(const std::filesystem::path& root,
                                    const std::filesystem::path& candidate) {
  const auto absolute_root = AbsoluteNormalized(root);
  const auto absolute_candidate = AbsoluteNormalized(candidate);
  if (!IsWithin(absolute_root, absolute_candidate)) return false;
  const auto relative = absolute_candidate.lexically_relative(absolute_root);
  if (relative.empty() || !IsContainedRelative(relative)) return false;
  std::filesystem::path current = absolute_root;
  for (const auto& component : relative) {
    current /= component;
    const DWORD attributes = GetFileAttributesW(current.c_str());
    if (attributes != INVALID_FILE_ATTRIBUTES &&
        (attributes & FILE_ATTRIBUTE_HIDDEN) != 0)
      return true;
  }
  return false;
}

enum class TextReadResult { Success, Cancelled, Excluded, Issue };

TextReadResult ReadTextSnapshot(const std::filesystem::path& path, std::size_t max_bytes,
                                std::wstring& text, std::wstring& error,
                                const std::function<bool()>& cancelled = {}) {
  struct Handle {
    HANDLE value{INVALID_HANDLE_VALUE};
    ~Handle() { if (value != INVALID_HANDLE_VALUE) CloseHandle(value); }
  } file;
  file.value = CreateFileW(path.c_str(), GENERIC_READ,
                           FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_SEQUENTIAL_SCAN |
                                              FILE_FLAG_OPEN_REPARSE_POINT,
                           nullptr);
  if (file.value == INVALID_HANDLE_VALUE) {
    error = L"ファイルを開けないため未処理です。";
    return TextReadResult::Issue;
  }

  BY_HANDLE_FILE_INFORMATION before{};
  LARGE_INTEGER file_size{};
  if (!GetFileInformationByHandle(file.value, &before) || !GetFileSizeEx(file.value, &file_size) ||
      file_size.QuadPart < 0) {
    error = L"ファイルの状態を確認できないため未処理です。";
    return TextReadResult::Issue;
  }
  if ((before.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
    error = L"シンボリックリンク／reparse pointは検索対象外です。";
    return TextReadResult::Excluded;
  }
  if (static_cast<unsigned long long>(file_size.QuadPart) > max_bytes) {
    error = L"サイズ上限（" + std::to_wstring(max_bytes / (1024U * 1024U)) +
            L" MiB）を超えるため未処理です。";
    return TextReadResult::Issue;
  }
  std::vector<std::byte> bytes(static_cast<std::size_t>(file_size.QuadPart));
  std::size_t offset{};
  while (offset < bytes.size()) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return TextReadResult::Cancelled;
    }
    const auto chunk = static_cast<DWORD>(
        std::min<std::size_t>(bytes.size() - offset, 64U * 1024U));
    DWORD read{};
    if (!ReadFile(file.value, bytes.data() + offset, chunk, &read, nullptr) || read == 0) {
      error = L"ファイルを最後まで読み込めないため未処理です。";
      return TextReadResult::Issue;
    }
    offset += read;
  }

  BY_HANDLE_FILE_INFORMATION after{};
  LARGE_INTEGER size_after{};
  if (!GetFileInformationByHandle(file.value, &after) || !GetFileSizeEx(file.value, &size_after) ||
      size_after.QuadPart != file_size.QuadPart ||
      before.ftLastWriteTime.dwHighDateTime != after.ftLastWriteTime.dwHighDateTime ||
      before.ftLastWriteTime.dwLowDateTime != after.ftLastWriteTime.dwLowDateTime ||
      before.nFileIndexHigh != after.nFileIndexHigh || before.nFileIndexLow != after.nFileIndexLow ||
      before.dwVolumeSerialNumber != after.dwVolumeSerialNumber) {
    error = L"検索中にファイルが変更されたため未処理です。";
    return TextReadResult::Issue;
  }
  if (std::ranges::any_of(bytes, [](std::byte byte) { return byte == std::byte{}; })) {
    error = L"バイナリまたはNUL文字を含むファイルは検索対象外です。";
    return TextReadResult::Excluded;
  }

  std::span<const std::byte> payload(bytes);
  const bool has_utf8_bom = bytes.size() >= 3 && bytes[0] == std::byte{0xEF} &&
                            bytes[1] == std::byte{0xBB} && bytes[2] == std::byte{0xBF};
  if (has_utf8_bom) payload = payload.subspan(3);
  if (payload.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
    error = L"ファイルがデコード上限を超えるため未処理です。";
    return TextReadResult::Issue;
  }
  const auto decode = [&](UINT code_page, DWORD flags, std::wstring& output) {
    if (payload.empty()) { output.clear(); return true; }
    const auto* source = reinterpret_cast<const char*>(payload.data());
    const int size = static_cast<int>(payload.size());
    const int required = MultiByteToWideChar(code_page, flags, source, size, nullptr, 0);
    if (required <= 0) return false;
    output.resize(static_cast<std::size_t>(required));
    return MultiByteToWideChar(code_page, flags, source, size, output.data(), required) == required;
  };
  if (decode(CP_UTF8, MB_ERR_INVALID_CHARS, text)) return TextReadResult::Success;
  if (!has_utf8_bom && decode(932U, MB_ERR_INVALID_CHARS, text)) return TextReadResult::Success;
  text.clear();
  error = has_utf8_bom
              ? L"UTF-8 BOMの内容が正しいUTF-8ではないため未処理です。"
              : L"UTF-8またはCP932として安全に読み込めないため未処理です。";
  return TextReadResult::Issue;
}

bool IsPathSeparator(wchar_t character) { return character == L'/' || character == L'\\'; }

bool GlobMatch(std::wstring_view value, std::wstring_view pattern) {
  enum class Kind { Literal, Any, Star, RecursiveStar };
  struct Token { Kind kind; std::size_t length; wchar_t value; };
  const auto token_at = [&](std::size_t index) {
    if (pattern[index] != L'*') return Token{pattern[index] == L'?' ? Kind::Any : Kind::Literal,
                                             1, pattern[index]};
    std::size_t end = index + 1;
    if (end < pattern.size() && pattern[end] == L'*') {
      while (end < pattern.size() && pattern[end] == L'*') ++end;
      if (end < pattern.size() && IsPathSeparator(pattern[end])) ++end;
      return Token{Kind::RecursiveStar, end - index, L'\0'};
    }
    return Token{Kind::Star, 1, L'\0'};
  };
  const auto close_zero_width_stars = [&](std::vector<unsigned char>& states) {
    for (std::size_t index = 0; index < pattern.size();) {
      const Token token = token_at(index);
      if (states[index] && (token.kind == Kind::Star || token.kind == Kind::RecursiveStar))
        states[index + token.length] = 1;
      index += token.length;
    }
  };

  std::vector<unsigned char> current(pattern.size() + 1, 0);
  std::vector<unsigned char> next(pattern.size() + 1, 0);
  current[0] = 1;
  close_zero_width_stars(current);
  for (const wchar_t character : value) {
    std::ranges::fill(next, static_cast<unsigned char>(0));
    for (std::size_t index = 0; index < pattern.size();) {
      const Token token = token_at(index);
      if (current[index]) {
        if (token.kind == Kind::Star && !IsPathSeparator(character)) {
          next[index] = 1;
        } else if (token.kind == Kind::RecursiveStar) {
          next[index] = 1;
        } else if (token.kind == Kind::Any && !IsPathSeparator(character)) {
          next[index + token.length] = 1;
        } else if (token.kind == Kind::Literal &&
                   ((IsPathSeparator(token.value) && IsPathSeparator(character)) ||
                    towlower(token.value) == towlower(character))) {
          next[index + token.length] = 1;
        }
      }
      index += token.length;
    }
    close_zero_width_stars(next);
    current.swap(next);
  }
  return current[pattern.size()] != 0;
}

struct IgnoreRule {
  std::filesystem::path base;
  std::wstring pattern;
  bool negated{};
  bool directory_only{};
  bool anchored{};
  bool contains_separator{};
};

struct IgnoreCacheEntry {
  std::vector<IgnoreRule> rules;
  bool readable{true};
};

using IgnoreCache = std::map<std::filesystem::path, IgnoreCacheEntry>;

enum class IgnoreDecision { Process, Ignore, Unprocessed };

bool IsContainedRelative(const std::filesystem::path& relative) {
  if (relative.empty()) return true;
  const auto first = *relative.begin();
  return first != L"..";
}

std::vector<std::wstring> PathComponents(std::wstring_view value) {
  std::vector<std::wstring> components;
  std::size_t begin{};
  while (begin < value.size()) {
    const auto separator = value.find(L'/', begin);
    const auto end = separator == std::wstring_view::npos ? value.size() : separator;
    if (end != begin) components.emplace_back(value.substr(begin, end - begin));
    if (separator == std::wstring_view::npos) break;
    begin = separator + 1;
  }
  return components;
}

bool IgnoreRuleMatches(const IgnoreRule& rule, const std::filesystem::path& root_relative,
                       bool is_directory) {
  const auto scoped = root_relative.lexically_relative(rule.base);
  if (!IsContainedRelative(scoped) || scoped.empty()) return false;
  const auto value = scoped.generic_wstring();
  const auto components = PathComponents(value);
  if (components.empty()) return false;

  // A rule matching a directory also applies to all of its descendants.  We
  // therefore check each directory prefix as well as the complete candidate.
  std::vector<std::wstring> candidates;
  std::wstring prefix;
  for (std::size_t index = 0; index < components.size(); ++index) {
    if (!prefix.empty()) prefix += L'/';
    prefix += components[index];
    const bool component_is_directory = index + 1 < components.size() || is_directory;
    if (!rule.directory_only || component_is_directory) candidates.push_back(prefix);
  }

  if (rule.contains_separator) {
    return std::ranges::any_of(candidates, [&](const auto& candidate) {
      return GlobMatch(candidate, rule.pattern);
    });
  }
  if (rule.anchored) {
    return !candidates.empty() && GlobMatch(components.front(), rule.pattern);
  }
  const std::size_t component_limit = rule.directory_only && !is_directory
                                          ? components.size() - 1
                                          : components.size();
  for (std::size_t index = 0; index < component_limit; ++index) {
    if (GlobMatch(components[index], rule.pattern)) return true;
  }
  return false;
}

std::vector<IgnoreRule> ParseIgnoreRules(const std::filesystem::path& base,
                                         std::wstring_view text, bool& too_complex) {
  std::vector<IgnoreRule> rules;
  too_complex = false;
  std::size_t begin{};
  while (begin <= text.size()) {
    const auto newline = text.find(L'\n', begin);
    const auto end = newline == std::wstring_view::npos ? text.size() : newline;
    std::wstring line(text.substr(begin, end - begin));
    if (!line.empty() && line.back() == L'\r') line.pop_back();
    while (!line.empty() && iswspace(line.back()) &&
           (line.size() < 2 || line[line.size() - 2] != L'\\'))
      line.pop_back();
    if (!line.empty() && line.front() != L'#') {
      bool escaped_initial = line.size() > 1 && line.front() == L'\\' &&
                             (line[1] == L'#' || line[1] == L'!');
      if (escaped_initial) line.erase(line.begin());
      bool negated = !escaped_initial && !line.empty() && line.front() == L'!';
      if (negated) line.erase(line.begin());
      const bool directory_only = !line.empty() && line.back() == L'/';
      if (directory_only) line.pop_back();
      const bool anchored = !line.empty() && line.front() == L'/';
      if (anchored) line.erase(line.begin());
      for (std::size_t index = 0; index + 1 < line.size();) {
        if (line[index] == L'\\' &&
            (line[index + 1] == L' ' || line[index + 1] == L'#' || line[index + 1] == L'!'))
          line.erase(index, 1);
        else
          ++index;
      }
      if (!line.empty()) {
        if (line.size() > 256 || rules.size() >= 1024) {
          too_complex = true;
          return {};
        }
        rules.push_back({base, line, negated, directory_only, anchored,
                         line.find(L'/') != std::wstring::npos});
      }
    }
    if (newline == std::wstring_view::npos) break;
    begin = newline + 1;
  }
  return rules;
}

IgnoreDecision EvaluateGitIgnore(const std::filesystem::path& root,
                                 const std::filesystem::path& relative, bool is_directory,
                                 IgnoreCache& cache,
                                 std::size_t& ignore_bytes_read,
                                 std::size_t& ignore_rules_read,
                                 const std::function<bool(const SearchIssue&)>& report_issue) {
  std::vector<const IgnoreRule*> applicable_rules;
  std::filesystem::path directory = root;
  std::vector<std::filesystem::path> directories{root};
  auto parent = relative.parent_path();
  for (const auto& component : parent) {
    directory /= component;
    directories.push_back(directory);
  }
  for (const auto& current : directories) {
    auto [cached, inserted] = cache.try_emplace(current);
    if (inserted) {
      std::error_code exists_error;
      const auto ignore_path = current / L".gitignore";
      const auto ignore_status = std::filesystem::symlink_status(ignore_path, exists_error);
      const bool absent = exists_error == std::errc::no_such_file_or_directory ||
                          (!exists_error && ignore_status.type() == std::filesystem::file_type::not_found);
      if (absent) {
        cached->second.readable = true;
      } else if (exists_error) {
        cached->second.readable = false;
        report_issue({ignore_path, L".gitignore の状態を確認できないため、この配下は未処理です。"});
      } else if (std::filesystem::is_symlink(ignore_status)) {
        cached->second.readable = false;
        report_issue({ignore_path, L"シンボリックリンクの.gitignoreは適用できないため、この配下は未処理です。"});
      } else if (!std::filesystem::is_regular_file(ignore_status)) {
        cached->second.readable = false;
        report_issue({ignore_path, L".gitignoreが通常ファイルでないため、この配下は未処理です。"});
      } else {
        std::error_code size_error;
        const auto size = std::filesystem::file_size(ignore_path, size_error);
        if (size_error || size > kSearchMaxIgnoreFileBytes ||
            size > kSearchMaxIgnoreBytes - ignore_bytes_read) {
          cached->second.readable = false;
          report_issue({ignore_path, L".gitignoreの読み込み上限に達したため、この配下は未処理です。"});
        } else {
          ignore_bytes_read += static_cast<std::size_t>(size);
          std::wstring load_error;
          std::wstring ignore_text;
          if (ReadTextSnapshot(ignore_path, kSearchMaxIgnoreFileBytes, ignore_text, load_error) !=
              TextReadResult::Success) {
            cached->second.readable = false;
            report_issue({ignore_path, L".gitignore を読み込めないため、この配下は未処理です: " +
                                           load_error});
          } else {
            const auto base = current.lexically_relative(root).lexically_normal();
            bool too_complex{};
            auto rules = ParseIgnoreRules(base == L"." ? std::filesystem::path{} : base,
                                          ignore_text, too_complex);
            if (too_complex || rules.size() > kSearchMaxIgnoreRules - ignore_rules_read) {
              cached->second.readable = false;
              report_issue({ignore_path, L".gitignoreのルール数またはパターン長の上限を超えたため、この配下は未処理です。"});
            } else {
              ignore_rules_read += rules.size();
              cached->second.rules = std::move(rules);
            }
          }
        }
      }
    }
    if (!cached->second.readable) return IgnoreDecision::Unprocessed;
    for (const auto& rule : cached->second.rules) applicable_rules.push_back(&rule);
  }
  const auto evaluate = [&](const std::filesystem::path& candidate, bool candidate_is_directory) {
    bool ignored{};
    for (const auto* rule : applicable_rules) {
      if (IgnoreRuleMatches(*rule, candidate, candidate_is_directory)) ignored = !rule->negated;
    }
    return ignored;
  };
  // Git cannot re-include a file while one of its parent directories remains
  // excluded.  Checking every directory prefix preserves that rule while the
  // enumerator itself keeps traversing, so a later `!parent/` can still reopen it.
  std::filesystem::path prefix;
  std::size_t index{};
  const auto component_count = static_cast<std::size_t>(std::distance(relative.begin(), relative.end()));
  for (const auto& component : relative) {
    prefix /= component;
    ++index;
    if ((index < component_count || is_directory) && evaluate(prefix, true))
      return IgnoreDecision::Ignore;
  }
  return evaluate(relative, is_directory) ? IgnoreDecision::Ignore : IgnoreDecision::Process;
}

bool MatchesGlobs(const std::filesystem::path& relative, const SearchQuery& query) {
  const auto value = relative.generic_wstring();
  const auto has_globs = [](const auto& globs) {
    return std::ranges::any_of(globs, [](const auto& glob) { return !glob.empty(); });
  };
  if (has_globs(query.include_globs) &&
      std::ranges::none_of(query.include_globs, [&](const auto& glob) {
        return !glob.empty() && GlobMatch(value, glob);
      }))
    return false;
  return std::ranges::none_of(query.exclude_globs,
                              [&](const auto& glob) { return !glob.empty() && GlobMatch(value, glob); });
}

bool ValidateGlobs(const SearchQuery& query, std::wstring& error) {
  const std::size_t count = query.include_globs.size() + query.exclude_globs.size();
  if (count > 32) {
    error = L"include／exclude globは合計32個までです。";
    return false;
  }
  std::size_t total_length{};
  for (const auto* globs : {&query.include_globs, &query.exclude_globs}) {
    for (const auto& glob : *globs) {
      if (glob.size() > 256) {
        error = L"globは1個あたり256文字までです。";
        return false;
      }
      total_length += glob.size();
    }
  }
  if (total_length > 4096) {
    error = L"glob条件全体が長すぎます。";
    return false;
  }
  return true;
}

std::wstring Fold(std::wstring_view value) {
  std::wstring folded(value);
  std::ranges::transform(folded, folded.begin(), towlower);
  return folded;
}

struct SearchLocation {
  std::size_t cursor{};
  std::size_t line{};
  std::size_t line_start{};
  std::size_t line_end{};
};

bool AddMatch(const std::filesystem::path& path, std::wstring_view text, SearchLocation& location,
              std::size_t begin,
              std::size_t end, std::uint64_t snapshot_hash,
              std::vector<SearchMatch>& matches, std::size_t& result_payload_bytes,
              bool* payload_limited) {
  while (location.cursor < begin) {
    if (text[location.cursor] == L'\n') {
      ++location.line;
      location.line_start = location.cursor + 1;
      location.line_end = text.find(L'\n', location.line_start);
      if (location.line_end == std::wstring_view::npos) location.line_end = text.size();
    }
    ++location.cursor;
  }
  std::size_t visible_line_end = location.line_end;
  if (visible_line_end > location.line_start && text[visible_line_end - 1] == L'\r')
    --visible_line_end;
  const std::size_t line_length = visible_line_end - location.line_start;
  const std::size_t match_line_begin =
      std::clamp(begin, location.line_start, visible_line_end) - location.line_start;
  const std::size_t match_line_end =
      std::clamp(std::max(end, begin), location.line_start, visible_line_end) - location.line_start;
  std::size_t source_begin{};
  std::size_t source_end = line_length;
  if (line_length > kSearchMaxPreviewCodeUnits) {
    constexpr std::size_t source_window = kSearchMaxPreviewCodeUnits - 2;
    const std::size_t matched_length = match_line_end - match_line_begin;
    const std::size_t context_length = matched_length < source_window
                                           ? source_window - matched_length
                                           : 0;
    const std::size_t left_context = context_length / 2;
    source_begin = match_line_begin > left_context ? match_line_begin - left_context : 0;
    source_begin = std::min(source_begin, line_length - source_window);
    source_end = source_begin + source_window;
    const auto is_high_surrogate = [](wchar_t unit) {
      const auto value = static_cast<std::uint16_t>(unit);
      return value >= 0xD800U && value <= 0xDBFFU;
    };
    const auto is_low_surrogate = [](wchar_t unit) {
      const auto value = static_cast<std::uint16_t>(unit);
      return value >= 0xDC00U && value <= 0xDFFFU;
    };
    if (source_begin > 0 && source_begin < line_length &&
        is_high_surrogate(text[location.line_start + source_begin - 1]) &&
        is_low_surrogate(text[location.line_start + source_begin]))
      ++source_begin;
    if (source_end > source_begin && source_end < line_length &&
        is_high_surrogate(text[location.line_start + source_end - 1]) &&
        is_low_surrogate(text[location.line_start + source_end]))
      --source_end;
  }
  const bool left_ellipsis = source_begin != 0;
  const bool right_ellipsis = source_end != line_length;
  const std::size_t preview_length = source_end - source_begin +
      static_cast<std::size_t>(left_ellipsis) + static_cast<std::size_t>(right_ellipsis);
  std::size_t remaining_payload = result_payload_bytes <= kSearchMaxResultPayloadBytes
      ? kSearchMaxResultPayloadBytes - result_payload_bytes
      : 0;
  if (sizeof(SearchMatch) > remaining_payload) {
    if (payload_limited) *payload_limited = true;
    return false;
  }
  remaining_payload -= sizeof(SearchMatch);
  const auto fits_characters = [&remaining_payload](std::size_t count) {
    if (count > remaining_payload / sizeof(wchar_t)) return false;
    remaining_payload -= count * sizeof(wchar_t);
    return true;
  };
  if (!fits_characters(path.native().size()) || !fits_characters(preview_length)) {
    if (payload_limited) *payload_limited = true;
    return false;
  }

  std::wstring preview;
  preview.reserve(preview_length);
  if (left_ellipsis) preview.push_back(L'…');
  preview.append(text.substr(location.line_start + source_begin, source_end - source_begin));
  if (right_ellipsis) preview.push_back(L'…');
  const std::size_t preview_prefix = static_cast<std::size_t>(left_ellipsis);
  const std::size_t preview_begin = preview_prefix +
      std::clamp(match_line_begin, source_begin, source_end) - source_begin;
  const std::size_t preview_end = preview_prefix +
      std::clamp(match_line_end, source_begin, source_end) - source_begin;
  matches.push_back({path, begin, end, location.line + 1, match_line_begin + 1,
                     std::move(preview), preview_begin, preview_end, snapshot_hash});
  result_payload_bytes = kSearchMaxResultPayloadBytes - remaining_payload;
  return true;
}

struct CompiledRegex {
  pcre2_code* code{};
  ~CompiledRegex() { if (code) pcre2_code_free(code); }
  CompiledRegex() = default;
  CompiledRegex(const CompiledRegex&) = delete;
  CompiledRegex& operator=(const CompiledRegex&) = delete;
};

bool CompileRegex(const SearchQuery& query, CompiledRegex& compiled, std::wstring& error) {
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (query.text.size() > kSearchRegexMaxPatternLength) {
    error = L"正規表現が長すぎます（上限 4096 文字）。";
    return false;
  }
  std::uint32_t options = PCRE2_UTF | PCRE2_UCP | PCRE2_ALT_BSUX |
                          PCRE2_MATCH_UNSET_BACKREF | PCRE2_NEVER_BACKSLASH_C |
                          PCRE2_AUTO_CALLOUT | PCRE2_NO_START_OPTIMIZE;
  if (!query.match_case) options |= PCRE2_CASELESS;
  int error_code{};
  PCRE2_SIZE error_offset{};
  compiled.code = pcre2_compile(reinterpret_cast<PCRE2_SPTR>(query.text.data()),
                                static_cast<PCRE2_SIZE>(query.text.size()), options,
                                &error_code, &error_offset, nullptr);
  if (!compiled.code) {
    error = L"正規表現が正しくありません（位置 " +
            std::to_wstring(static_cast<std::size_t>(error_offset) + 1) + L"）。";
    return false;
  }
  return true;
}

struct UnicodeWordPattern {
  pcre2_code* code{};

  UnicodeWordPattern() {
    static constexpr wchar_t pattern[] = L"\\A\\w\\z";
    int error_code{};
    PCRE2_SIZE error_offset{};
    code = pcre2_compile(reinterpret_cast<PCRE2_SPTR>(pattern), PCRE2_ZERO_TERMINATED,
                         PCRE2_UTF | PCRE2_UCP, &error_code, &error_offset, nullptr);
  }

  ~UnicodeWordPattern() { if (code) pcre2_code_free(code); }
  UnicodeWordPattern(const UnicodeWordPattern&) = delete;
  UnicodeWordPattern& operator=(const UnicodeWordPattern&) = delete;
};

const pcre2_code* UnicodeWordCode() {
  static const UnicodeWordPattern pattern;
  return pattern.code;
}

struct WordMatchData {
  pcre2_match_data* data{pcre2_match_data_create(1, nullptr)};
  ~WordMatchData() { if (data) pcre2_match_data_free(data); }
};

bool IsWordCodePoint(std::wstring_view text, std::size_t begin, std::size_t length) {
  if (length == 1) {
    const auto code_unit = static_cast<std::uint16_t>(text[begin]);
    if (code_unit < 0x80U)
      return (code_unit >= L'a' && code_unit <= L'z') ||
             (code_unit >= L'A' && code_unit <= L'Z') ||
             (code_unit >= L'0' && code_unit <= L'9') || code_unit == L'_';
    if (code_unit >= 0xD800U && code_unit <= 0xDFFFU) return false;
  } else if (length != 2 ||
             static_cast<std::uint16_t>(text[begin]) < 0xD800U ||
             static_cast<std::uint16_t>(text[begin]) > 0xDBFFU ||
             static_cast<std::uint16_t>(text[begin + 1]) < 0xDC00U ||
             static_cast<std::uint16_t>(text[begin + 1]) > 0xDFFFU) {
    return false;
  }

  const pcre2_code* code = UnicodeWordCode();
  thread_local WordMatchData match_data;
  // On classifier setup failure, conservatively treat non-ASCII as word text.
  if (!code || !match_data.data) return true;
  const int result = pcre2_match(code,
                                 reinterpret_cast<PCRE2_SPTR>(text.data() + begin),
                                 static_cast<PCRE2_SIZE>(length), 0, 0,
                                 match_data.data, nullptr);
  return result >= 0;
}

bool IsWholeWord(std::wstring_view text, std::size_t begin, std::size_t end) {
  bool has_word_character_before{};
  if (begin != 0) {
    std::size_t previous = begin - 1;
    const auto previous_unit = static_cast<std::uint16_t>(text[previous]);
    if (previous_unit >= 0xDC00U && previous_unit <= 0xDFFFU && previous != 0) {
      const auto high = static_cast<std::uint16_t>(text[previous - 1]);
      if (high >= 0xD800U && high <= 0xDBFFU) --previous;
    }
    has_word_character_before = IsWordCodePoint(text, previous, begin - previous);
  }

  bool has_word_character_after{};
  if (end < text.size()) {
    std::size_t length = 1;
    const auto high = static_cast<std::uint16_t>(text[end]);
    if (high >= 0xD800U && high <= 0xDBFFU && end + 1 < text.size()) {
      const auto low = static_cast<std::uint16_t>(text[end + 1]);
      if (low >= 0xDC00U && low <= 0xDFFFU) length = 2;
    }
    has_word_character_after = IsWordCodePoint(text, end, length);
  }
  return !has_word_character_before && !has_word_character_after;
}

struct RegexCalloutState {
  const std::function<bool()>* cancelled{};
  std::chrono::steady_clock::time_point deadline{};
  bool was_cancelled{};
  bool timed_out{};
};

int RegexCallout(pcre2_callout_block*, void* raw_state) {
  auto& state = *static_cast<RegexCalloutState*>(raw_state);
  if (state.cancelled && *state.cancelled && (*state.cancelled)()) {
    state.was_cancelled = true;
    return PCRE2_ERROR_CALLOUT;
  }
  if (std::chrono::steady_clock::now() >= state.deadline) {
    state.timed_out = true;
    return PCRE2_ERROR_CALLOUT;
  }
  return 0;
}

std::size_t AdvanceCodePoint(std::wstring_view text, std::size_t offset) {
  if (offset >= text.size()) return text.size();
  const auto current = static_cast<std::uint16_t>(text[offset]);
  if (current >= 0xD800U && current <= 0xDBFFU && offset + 1 < text.size()) {
    const auto next = static_cast<std::uint16_t>(text[offset + 1]);
    if (next >= 0xDC00U && next <= 0xDFFFU) return offset + 2;
  }
  return offset + 1;
}

bool CollectRegexMatches(std::wstring_view text, const SearchQuery& query,
                         const CompiledRegex& expression,
                         std::vector<SearchRegexMatch>& matches, std::wstring& error,
                         const std::function<bool()>& cancelled, std::size_t max_matches,
                         bool* truncated, std::size_t minimum_begin, bool stop_after_first,
                         bool include_captures) {
  matches.clear();
  if (truncated) *truncated = false;
  pcre2_match_data* raw_data = pcre2_match_data_create_from_pattern(expression.code, nullptr);
  if (!raw_data) {
    error = L"正規表現検索用メモリを確保できません。";
    return false;
  }
  struct MatchDataGuard {
    pcre2_match_data* value;
    ~MatchDataGuard() { pcre2_match_data_free(value); }
  } data_guard{raw_data};

  pcre2_match_context* raw_context = pcre2_match_context_create(nullptr);
  if (!raw_context) {
    error = L"正規表現検索用メモリを確保できません。";
    return false;
  }
  struct MatchContextGuard {
    pcre2_match_context* value;
    ~MatchContextGuard() { pcre2_match_context_free(value); }
  } context_guard{raw_context};
  pcre2_set_match_limit(raw_context, 1'000'000U);
  pcre2_set_depth_limit(raw_context, 512U);
  pcre2_set_heap_limit(raw_context, 8U * 1024U);
  RegexCalloutState callout_state{&cancelled,
      std::chrono::steady_clock::now() +
          std::chrono::milliseconds(kSearchRegexDeadlineMilliseconds)};
  pcre2_set_callout(raw_context, &RegexCallout, &callout_state);

  const auto* subject = reinterpret_cast<PCRE2_SPTR>(text.data());
  const PCRE2_SIZE length = static_cast<PCRE2_SIZE>(text.size());
  std::size_t offset = std::min(minimum_begin, text.size());
  std::size_t total_capture_count{};
  bool retry_nonempty{};
  for (;;) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return false;
    }
    const std::uint32_t options = retry_nonempty
        ? static_cast<std::uint32_t>(PCRE2_NOTEMPTY_ATSTART | PCRE2_ANCHORED)
        : 0U;
    const int count = pcre2_match(expression.code, subject, length,
                                  static_cast<PCRE2_SIZE>(offset), options, raw_data,
                                  raw_context);
    if (count == PCRE2_ERROR_NOMATCH) {
      if (retry_nonempty) {
        if (offset >= text.size()) break;
        offset = AdvanceCodePoint(text, offset);
        retry_nonempty = false;
        continue;
      }
      break;
    }
    if (count < 0) {
      if (callout_state.was_cancelled || (cancelled && cancelled())) {
        error = L"検索を中止しました。";
      } else if (callout_state.timed_out || count == PCRE2_ERROR_MATCHLIMIT ||
                 count == PCRE2_ERROR_DEPTHLIMIT || count == PCRE2_ERROR_HEAPLIMIT) {
        error = L"正規表現の実行上限に達したため、この対象は未処理です。";
      } else if (count == PCRE2_ERROR_CALLOUT) {
        error = L"正規表現が中断されたため、この対象は未処理です。";
      } else {
        error = L"正規表現検索に失敗しました（" + std::to_wstring(count) + L"）。";
      }
      return false;
    }

    PCRE2_SIZE* offsets = pcre2_get_ovector_pointer(raw_data);
    const std::size_t begin = static_cast<std::size_t>(offsets[0]);
    const std::size_t end = static_cast<std::size_t>(offsets[1]);
    const bool accepted = !query.whole_word || IsWholeWord(text, begin, end);
    if (accepted) {
      if (matches.size() == max_matches) {
        if (truncated) *truncated = true;
        break;
      }
      SearchRegexMatch match{begin, end, {}};
      if (include_captures) {
        const auto capture_count = pcre2_get_ovector_count(raw_data);
        if (capture_count > 100'000U - total_capture_count) {
          error = L"正規表現のcapture結果が上限（100,000個）を超えます。";
          return false;
        }
        total_capture_count += capture_count;
        match.captures.reserve(static_cast<std::size_t>(capture_count));
        for (std::uint32_t index = 0; index < capture_count; ++index) {
          const PCRE2_SIZE capture_begin = offsets[index * 2];
          const PCRE2_SIZE capture_end = offsets[index * 2 + 1];
          if (capture_begin == PCRE2_UNSET || capture_end == PCRE2_UNSET) {
            match.captures.push_back({0, 0, false});
          } else {
            match.captures.push_back({static_cast<std::size_t>(capture_begin),
                                      static_cast<std::size_t>(capture_end), true});
          }
        }
      }
      matches.push_back(std::move(match));
      if (stop_after_first) break;
    }

    if (end == begin) {
      offset = begin;
      retry_nonempty = true;
    } else {
      offset = end;
      retry_nonempty = false;
    }
  }
  return true;
}

bool SearchText(const std::filesystem::path& path, std::wstring_view text,
                const SearchQuery& query, const CompiledRegex* expression,
                std::size_t max_matches, const std::function<bool()>& cancelled,
                std::vector<SearchMatch>& matches, std::wstring& error, bool* truncated,
                std::size_t& result_payload_bytes, bool* payload_limited) {
  if (truncated) *truncated = false;
  if (payload_limited) *payload_limited = false;
  const std::uint64_t snapshot_hash = SearchSnapshotHash(text);
  SearchLocation location{0, 0, 0, text.find(L'\n')};
  if (location.line_end == std::wstring_view::npos) location.line_end = text.size();
  if (query.regular_expression) {
    if (!expression || !expression->code) {
      error = L"正規表現を準備できません。";
      return false;
    }
    std::vector<SearchRegexMatch> regex_matches;
    if (!CollectRegexMatches(text, query, *expression, regex_matches, error, cancelled,
                             max_matches, truncated, 0, false, false))
      return false;
    for (const auto& match : regex_matches) {
      if (!AddMatch(path, text, location, match.begin, match.end, snapshot_hash, matches,
                    result_payload_bytes, payload_limited)) {
        if (truncated) *truncated = true;
        break;
      }
    }
    return true;
  }
  const std::wstring haystack = query.match_case ? std::wstring(text) : Fold(text);
  const std::wstring needle = query.match_case ? query.text : Fold(query.text);
  const std::size_t initial_count = matches.size();
  for (std::size_t position = 0; (position = haystack.find(needle, position)) != std::wstring::npos;) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return false;
    }
    const std::size_t end = position + needle.size();
    if (!query.whole_word || IsWholeWord(text, position, end)) {
      if (matches.size() - initial_count == max_matches) {
        if (truncated) *truncated = true;
        break;
      }
      if (!AddMatch(path, text, location, position, end, snapshot_hash, matches,
                    result_payload_bytes, payload_limited)) {
        if (truncated) *truncated = true;
        break;
      }
    }
    position += std::max<std::size_t>(1, needle.size());
  }
  return true;
}

}  // namespace

bool IsSearchWholeWordMatch(std::wstring_view text, std::size_t begin,
                            std::size_t end) noexcept {
  return begin <= end && end <= text.size() && IsWholeWord(text, begin, end);
}

std::uint64_t SearchSnapshotHash(std::wstring_view text) noexcept {
  std::uint64_t hash = 14695981039346656037ULL;
  for (const wchar_t value : text) {
    hash ^= static_cast<std::uint16_t>(value);
    hash *= 1099511628211ULL;
  }
  return hash;
}

bool ValidateSearchQuery(const SearchQuery& query, std::wstring& error) {
  error.clear();
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (!query.regular_expression) return true;
  CompiledRegex compiled;
  return CompileRegex(query, compiled, error);
}

bool SearchRegexMatches(std::wstring_view text, const SearchQuery& query,
                        std::vector<SearchRegexMatch>& matches, std::wstring& error,
                        const std::function<bool()>& cancelled, std::size_t max_matches,
                        bool* truncated, std::size_t minimum_begin,
                        bool stop_after_first) {
  matches.clear();
  error.clear();
  if (truncated) *truncated = false;
  if (!query.regular_expression) {
    error = L"正規表現検索には正規表現オプションが必要です。";
    return false;
  }
  if (text.size() > kSearchMaxFileBytes / sizeof(wchar_t)) {
    error = L"文書サイズ上限（8 MiB）を超えるため検索できません。";
    return false;
  }
  CompiledRegex expression;
  if (!CompileRegex(query, expression, error)) return false;
  return CollectRegexMatches(text, query, expression, matches, error, cancelled,
                             max_matches, truncated, minimum_begin, stop_after_first, true);
}

bool SearchDocumentText(const std::filesystem::path& path, std::wstring_view text,
                        const SearchQuery& query, std::vector<SearchMatch>& matches,
                        std::wstring& error, const std::function<bool()>& cancelled,
                        bool* truncated) {
  matches.clear();
  error.clear();
  if (truncated) *truncated = false;
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (text.size() > kSearchMaxFileBytes / sizeof(wchar_t)) {
    error = L"文書サイズ上限（8 MiB）を超えるため検索できません。";
    return false;
  }
  CompiledRegex expression;
  if (query.regular_expression && !CompileRegex(query, expression, error)) return false;
  bool was_truncated{};
  bool payload_limited{};
  std::size_t result_payload_bytes{};
  if (!SearchText(path, text, query, query.regular_expression ? &expression : nullptr,
                  kSearchMaxMatches, cancelled, matches, error, &was_truncated,
                  result_payload_bytes, &payload_limited)) {
    matches.clear();
    return false;
  }
  if (truncated) *truncated = was_truncated;
  if (was_truncated) {
    error = payload_limited
        ? L"検索結果の総量上限（8 MiB）に達したため、一部のみ表示しています。"
        : L"検索結果が上限（20,000件）に達したため、一部のみ表示しています。";
    return false;
  }
  return true;
}

bool SearchWorkspace(const std::filesystem::path& root, const SearchQuery& query,
                     const std::map<std::filesystem::path, std::wstring>& unsaved,
                     std::vector<SearchMatch>& matches, std::wstring& error,
                     const std::function<bool()>& cancelled,
                     const std::function<void(std::vector<SearchMatch>)>& progress,
                     std::vector<SearchIssue>* issues,
                     const std::function<void(std::vector<SearchIssue>)>& issue_progress) {
  matches.clear();
  if (issues) issues->clear();
  error.clear();
  if (root.empty()) {
    error = L"Workspaceが選択されていません。";
    return false;
  }
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  if (!ValidateGlobs(query, error)) return false;
  const bool can_report_issues = issues != nullptr || static_cast<bool>(issue_progress);
  std::size_t emitted_issue_count{};
  bool issue_limit_reported{};
  const auto report_issue = [&](const SearchIssue& issue) {
    if (!can_report_issues) {
      error = issue.message;
      return false;
    }
    if (emitted_issue_count < 999) {
      if (issues) issues->push_back(issue);
      if (issue_progress) issue_progress({issue});
      ++emitted_issue_count;
    } else if (!issue_limit_reported) {
      issue_limit_reported = true;
      const SearchIssue summary{issue.path, L"未処理項目が多いため、以降の詳細報告を省略しました。"};
      if (issues) issues->push_back(summary);
      if (issue_progress) issue_progress({summary});
    }
    return true;
  };

  CompiledRegex expression;
  if (query.regular_expression && !CompileRegex(query, expression, error)) return false;

  const auto absolute_root = AbsoluteNormalized(root);
  struct UnsavedEntry {
    std::filesystem::path path;
    const std::wstring* text{};
  };
  std::map<std::wstring, UnsavedEntry> unsaved_by_path;
  for (const auto& [path, text] : unsaved) {
    const auto absolute = AbsoluteNormalized(path);
    if (!IsWithin(absolute_root, absolute) || !IsSafeWorkspacePath(absolute_root, absolute, true, false))
      continue;
    const auto relative = absolute.lexically_relative(absolute_root);
    if (relative.empty() || IsExcluded(relative, query) || !MatchesGlobs(relative, query)) continue;
    if (!query.include_hidden && HasNativeHiddenAttributeInPath(absolute_root, absolute)) continue;
    unsaved_by_path.insert_or_assign(PathKey(absolute), UnsavedEntry{absolute, &text});
  }

  IgnoreCache ignore_cache;
  std::size_t ignore_bytes_read{};
  std::size_t ignore_rules_read{};
  std::set<std::wstring> consumed_unsaved;
  std::vector<std::filesystem::path> pending_directories{absolute_root};
  std::uint64_t scanned_bytes{};
  std::size_t scanned_entries{};
  std::size_t result_payload_bytes{};
  bool stop_scan{};

  const auto search_text = [&](const std::filesystem::path& path, std::wstring_view text,
                               std::uint64_t byte_cost) -> bool {
    if (text.size() > kSearchMaxFileBytes / sizeof(wchar_t)) {
      return report_issue({path, L"文書サイズ上限（8 MiB）を超えるため未処理です。"});
    }
    if (byte_cost > kSearchMaxWorkspaceBytes - scanned_bytes) {
      if (!report_issue({path, L"Workspace検索の読み込み上限（512 MiB）に達したため、一部のみ検索しました。"}))
        return false;
      stop_scan = true;
      return true;
    }
    scanned_bytes += byte_cost;
    const std::size_t match_begin = matches.size();
    const std::size_t remaining = matches.size() < kSearchMaxMatches
                                      ? kSearchMaxMatches - matches.size()
                                      : 0;
    bool truncated{};
    bool payload_limited{};
    std::wstring search_error;
    if (!SearchText(path, text, query, query.regular_expression ? &expression : nullptr,
                    remaining, cancelled, matches, search_error, &truncated,
                    result_payload_bytes, &payload_limited)) {
      if (cancelled && cancelled()) {
        error = L"検索を中止しました。";
        return false;
      }
      if (!report_issue({path, search_error.empty() ? L"検索に失敗したため未処理です。" : search_error}))
        return false;
      error.clear();
      return true;
    }
    if (progress && matches.size() != match_begin)
      progress(std::vector<SearchMatch>(matches.begin() + static_cast<std::ptrdiff_t>(match_begin),
                                        matches.end()));
    if (truncated) {
      const auto message = payload_limited
          ? L"検索結果の総量上限（8 MiB）に達したため、一部のみ表示しています。"
          : L"検索結果が上限（20,000件）に達したため、一部のみ表示しています。";
      if (!report_issue({path, message}))
        return false;
      stop_scan = true;
    }
    return true;
  };

  while (!pending_directories.empty()) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return false;
    }
    const auto directory = std::move(pending_directories.back());
    pending_directories.pop_back();
    std::error_code filesystem_error;
    std::filesystem::directory_iterator iterator(
        directory, std::filesystem::directory_options::none, filesystem_error);
    const std::filesystem::directory_iterator end;
    if (filesystem_error) {
      if (!report_issue({directory, L"ディレクトリを列挙できないため、この配下は未処理です。"}))
        return false;
      continue;
    }
    for (; iterator != end && !stop_scan; iterator.increment(filesystem_error)) {
      if (cancelled && cancelled()) {
        error = L"検索を中止しました。";
        return false;
      }
      if (++scanned_entries > kSearchMaxEntries) {
        if (!report_issue({directory, L"Workspace検索の対象上限（200,000項目）に達したため、一部のみ検索しました。"}))
          return false;
        stop_scan = true;
        break;
      }

      const auto current_path = iterator->path();
      const auto relative = current_path.lexically_relative(absolute_root);
      std::error_code type_error;
      const auto status = iterator->symlink_status(type_error);
      if (type_error) {
        if (!report_issue({current_path, L"ファイル種別を確認できないため未処理です。"})) return false;
        continue;
      }
      const DWORD attributes = GetFileAttributesW(current_path.c_str());
      if (attributes == INVALID_FILE_ATTRIBUTES) {
        if (!report_issue({current_path, L"ファイルpathの状態を確認できないため未処理です。"}))
          return false;
        continue;
      }
      if (std::filesystem::is_symlink(status) || (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
        if (!report_issue({current_path, L"シンボリックリンク／reparse pointは検索対象外です。",
                           SearchIssueKind::Excluded}))
          return false;
        continue;
      }
      if (IsExcluded(relative, query)) continue;
      if (!query.include_hidden && (attributes & FILE_ATTRIBUTE_HIDDEN) != 0) continue;
      const bool is_directory = std::filesystem::is_directory(status);
      const auto ignore_decision = query.respect_gitignore
          ? EvaluateGitIgnore(absolute_root, relative, is_directory, ignore_cache,
                              ignore_bytes_read, ignore_rules_read, report_issue)
          : IgnoreDecision::Process;
      if (ignore_decision == IgnoreDecision::Unprocessed) {
        if (!can_report_issues) return false;
        continue;
      }
      if (is_directory) {
        if (ignore_decision == IgnoreDecision::Process) pending_directories.push_back(current_path);
        continue;
      }
      if (ignore_decision == IgnoreDecision::Ignore || !std::filesystem::is_regular_file(status) ||
          !MatchesGlobs(relative, query))
        continue;

      const auto absolute = AbsoluteNormalized(current_path);
      const auto key = PathKey(absolute);
      const auto unsaved_match = unsaved_by_path.find(key);
      if (unsaved_match != unsaved_by_path.end()) {
        consumed_unsaved.insert(key);
        const auto byte_cost = static_cast<std::uint64_t>(unsaved_match->second.text->size()) * sizeof(wchar_t);
        if (!search_text(absolute, *unsaved_match->second.text, byte_cost)) return false;
        continue;
      }

      std::error_code size_error;
      const auto file_size = std::filesystem::file_size(current_path, size_error);
      if (size_error) {
        if (!report_issue({absolute, L"ファイルサイズを確認できないため未処理です。"})) return false;
        continue;
      }
      if (file_size > kSearchMaxFileBytes) {
        if (!report_issue({absolute, L"サイズ上限（8 MiB）を超えるため未処理です。"})) return false;
        continue;
      }
      if (file_size > kSearchMaxWorkspaceBytes - scanned_bytes) {
        if (!report_issue({absolute, L"Workspace検索の読み込み上限（512 MiB）に達したため、一部のみ検索しました。"}))
          return false;
        stop_scan = true;
        break;
      }
      scanned_bytes += file_size;
      std::wstring text;
      std::wstring load_error;
      const auto read_result = ReadTextSnapshot(current_path,
                                                static_cast<std::size_t>(kSearchMaxFileBytes),
                                                text, load_error, cancelled);
      if (read_result == TextReadResult::Cancelled) {
        error = L"検索を中止しました。";
        return false;
      }
      if (read_result != TextReadResult::Success) {
        const SearchIssueKind kind = read_result == TextReadResult::Excluded
                                         ? SearchIssueKind::Excluded
                                         : SearchIssueKind::Error;
        if (!report_issue({absolute, std::move(load_error), kind})) return false;
        continue;
      }
      if (!search_text(absolute, text, 0)) return false;
    }
    if (filesystem_error && !stop_scan) {
      if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
        return false;
    }
  }

  if (!stop_scan) {
    for (const auto& [key, entry] : unsaved_by_path) {
      if (consumed_unsaved.contains(key)) continue;
      if (cancelled && cancelled()) {
        error = L"検索を中止しました。";
        return false;
      }
      const auto& path = entry.path;
      const auto relative = path.lexically_relative(absolute_root);
      if (relative.empty() || IsExcluded(relative, query) || !MatchesGlobs(relative, query)) continue;
      if (!query.include_hidden && HasNativeHiddenAttributeInPath(absolute_root, path)) continue;
      if (!IsSafeWorkspacePath(absolute_root, path, true, false)) continue;
      std::error_code status_error;
      const auto status = std::filesystem::symlink_status(path, status_error);
      if (!status_error && status.type() != std::filesystem::file_type::not_found &&
          !std::filesystem::is_regular_file(status))
        continue;
      if (status_error && status_error != std::errc::no_such_file_or_directory) {
        if (!report_issue({path, L"未保存文書のpathを確認できないため未処理です。"})) return false;
        continue;
      }
      const auto ignore_decision = query.respect_gitignore
          ? EvaluateGitIgnore(absolute_root, relative, false, ignore_cache,
                              ignore_bytes_read, ignore_rules_read, report_issue)
          : IgnoreDecision::Process;
      if (ignore_decision == IgnoreDecision::Unprocessed) {
        if (!can_report_issues) return false;
        continue;
      }
      if (ignore_decision == IgnoreDecision::Ignore) continue;
      const auto byte_cost = static_cast<std::uint64_t>(entry.text->size()) * sizeof(wchar_t);
      if (!search_text(path, *entry.text, byte_cost)) return false;
      if (stop_scan) break;
    }
  }
  return true;
}

}  // namespace mdlite
