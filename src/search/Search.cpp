#include "search/Search.h"

#include "core/Document.h"

#include <algorithm>
#include <cwctype>
#include <regex>

namespace mdlite {
namespace {

bool IsSearchable(const std::filesystem::path& path) {
  std::wstring extension = path.extension().wstring();
  std::ranges::transform(extension, extension.begin(), towlower);
  static constexpr std::wstring_view extensions[] = {
      L".md", L".markdown", L".txt", L".log", L".json", L".toml", L".yaml", L".yml"};
  return std::ranges::find(extensions, extension) != std::end(extensions);
}

bool IsExcluded(const std::filesystem::path& relative) {
  for (const auto& part : relative) {
    if (part == L".git" || part == L"build" || part == L"out") return true;
    if (part == L".cache" || part == L".state") return true;
  }
  return false;
}

bool GlobMatch(std::wstring_view value, std::wstring_view pattern) {
  std::size_t value_index{};
  std::size_t pattern_index{};
  std::size_t star = std::wstring_view::npos;
  std::size_t retry{};
  while (value_index < value.size()) {
    if (pattern_index < pattern.size() &&
        (pattern[pattern_index] == L'?' || towlower(pattern[pattern_index]) == towlower(value[value_index]))) {
      ++value_index;
      ++pattern_index;
    } else if (pattern_index < pattern.size() && pattern[pattern_index] == L'*') {
      star = pattern_index++;
      retry = value_index;
    } else if (star != std::wstring_view::npos) {
      pattern_index = star + 1;
      value_index = ++retry;
    } else {
      return false;
    }
  }
  while (pattern_index < pattern.size() && pattern[pattern_index] == L'*') ++pattern_index;
  return pattern_index == pattern.size();
}

bool MatchesGlobs(const std::filesystem::path& relative, const SearchQuery& query) {
  const auto value = relative.generic_wstring();
  if (!query.include_globs.empty() &&
      std::ranges::none_of(query.include_globs, [&](const auto& glob) { return GlobMatch(value, glob); }))
    return false;
  return std::ranges::none_of(query.exclude_globs,
                              [&](const auto& glob) { return GlobMatch(value, glob); });
}

bool IsWordCharacter(wchar_t character) {
  return iswalnum(character) != 0 || character == L'_';
}

bool IsWholeWord(std::wstring_view text, std::size_t begin, std::size_t end) {
  return (begin == 0 || !IsWordCharacter(text[begin - 1])) &&
         (end == text.size() || !IsWordCharacter(text[end]));
}

std::wstring Fold(std::wstring_view value) {
  std::wstring folded(value);
  std::ranges::transform(folded, folded.begin(), towlower);
  return folded;
}

void AddMatch(const std::filesystem::path& path, std::wstring_view text, std::size_t begin,
              std::size_t end, std::vector<SearchMatch>& matches) {
  const std::size_t line_start = begin == 0 ? 0 : text.rfind(L'\n', begin - 1) + 1;
  std::size_t line_end = text.find(L'\n', end);
  if (line_end == std::wstring_view::npos) line_end = text.size();
  const std::size_t line = static_cast<std::size_t>(
      std::count(text.begin(), text.begin() + static_cast<std::ptrdiff_t>(begin), L'\n')) + 1;
  matches.push_back({path, begin, end, line, begin - line_start + 1,
                     std::wstring(text.substr(line_start, line_end - line_start))});
}

bool SearchText(const std::filesystem::path& path, std::wstring_view text, const SearchQuery& query,
                std::vector<SearchMatch>& matches, std::wstring& error) {
  if (query.regular_expression) {
    try {
      const auto flags = std::regex_constants::ECMAScript |
                         (query.match_case ? std::regex_constants::syntax_option_type{}
                                           : std::regex_constants::icase);
      const std::wregex expression(query.text, flags);
      const std::wstring owned(text);
      for (std::wsregex_iterator iterator(owned.begin(), owned.end(), expression), end;
           iterator != end; ++iterator) {
        const std::size_t begin = static_cast<std::size_t>(iterator->position());
        const std::size_t end_position = begin + static_cast<std::size_t>(iterator->length());
        if (!query.whole_word || IsWholeWord(text, begin, end_position))
          AddMatch(path, text, begin, end_position, matches);
      }
      return true;
    } catch (const std::regex_error&) {
      error = L"正規表現が正しくありません。";
      return false;
    }
  }
  const std::wstring haystack = query.match_case ? std::wstring(text) : Fold(text);
  const std::wstring needle = query.match_case ? query.text : Fold(query.text);
  for (std::size_t position = 0; (position = haystack.find(needle, position)) != std::wstring::npos;) {
    const std::size_t end = position + needle.size();
    if (!query.whole_word || IsWholeWord(text, position, end)) AddMatch(path, text, position, end, matches);
    position += std::max<std::size_t>(1, needle.size());
  }
  return true;
}

}  // namespace

bool SearchWorkspace(const std::filesystem::path& root, const SearchQuery& query,
                     const std::map<std::filesystem::path, std::wstring>& unsaved,
                     std::vector<SearchMatch>& matches, std::wstring& error,
                     const std::function<bool()>& cancelled) {
  matches.clear();
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  std::error_code filesystem_error;
  for (std::filesystem::recursive_directory_iterator iterator(
           root, std::filesystem::directory_options::skip_permission_denied, filesystem_error), end;
       iterator != end && !filesystem_error; iterator.increment(filesystem_error)) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return false;
    }
    const auto relative = std::filesystem::relative(iterator->path(), root, filesystem_error);
    if (filesystem_error) break;
    if (IsExcluded(relative)) {
      if (iterator->is_directory()) iterator.disable_recursion_pending();
      continue;
    }
    if (!iterator->is_regular_file() || !IsSearchable(iterator->path()) ||
        !MatchesGlobs(relative, query)) continue;
    const auto absolute = std::filesystem::absolute(iterator->path()).lexically_normal();
    const auto unsaved_match = unsaved.find(absolute);
    if (unsaved_match != unsaved.end()) {
      if (!SearchText(absolute, unsaved_match->second, query, matches, error)) return false;
      continue;
    }
    Document document;
    if (!document.Load(absolute, error)) return false;
    if (!SearchText(absolute, document.text(), query, matches, error)) return false;
  }
  if (filesystem_error) {
    error = L"Workspace検索中にファイルを列挙できませんでした。";
    return false;
  }
  return true;
}

}  // namespace mdlite
