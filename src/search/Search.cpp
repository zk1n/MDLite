#include "search/Search.h"

#include "core/Document.h"

#include <algorithm>
#include <cwctype>
#include <map>
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

bool IsExcluded(const std::filesystem::path& relative, const SearchQuery& query) {
  for (const auto& part : relative) {
    const auto name = part.wstring();
    if (name == L".git" || name == L".mdlite" || name == L"build" || name == L"out")
      return true;
    if (name == L".cache" || name == L".state") return true;
    if (!query.include_hidden && name.size() > 1 && name.front() == L'.') return true;
  }
  return false;
}

bool IsPathSeparator(wchar_t character) { return character == L'/' || character == L'\\'; }

bool GlobMatch(std::wstring_view value, std::wstring_view pattern) {
  // Workspace globs use the familiar path rules: '*' and '?' stay inside one
  // path component, while '**' may cross separators. The memo table also
  // bounds pathological patterns instead of relying on exponential backtracking.
  const std::size_t columns = pattern.size() + 1;
  std::vector<signed char> memo((value.size() + 1) * columns, -1);
  const auto match = [&](const auto& self, std::size_t value_index,
                         std::size_t pattern_index) -> bool {
    auto& cached = memo[value_index * columns + pattern_index];
    if (cached >= 0) return cached != 0;
    bool result{};
    if (pattern_index == pattern.size()) {
      result = value_index == value.size();
    } else if (pattern[pattern_index] == L'*') {
      const bool recursive = pattern_index + 1 < pattern.size() &&
                             pattern[pattern_index + 1] == L'*';
      if (recursive) {
        std::size_t next = pattern_index + 2;
        while (next < pattern.size() && pattern[next] == L'*') ++next;
        result = self(self, value_index, next);
        if (!result && next < pattern.size() && IsPathSeparator(pattern[next]))
          result = self(self, value_index, next + 1);
        if (!result && value_index < value.size())
          result = self(self, value_index + 1, pattern_index);
      } else {
        result = self(self, value_index, pattern_index + 1);
        if (!result && value_index < value.size() && !IsPathSeparator(value[value_index]))
          result = self(self, value_index + 1, pattern_index);
      }
    } else if (value_index < value.size() && pattern[pattern_index] == L'?' &&
               !IsPathSeparator(value[value_index])) {
      result = self(self, value_index + 1, pattern_index + 1);
    } else if (value_index < value.size() &&
               IsPathSeparator(pattern[pattern_index]) && IsPathSeparator(value[value_index])) {
      result = self(self, value_index + 1, pattern_index + 1);
    } else if (value_index < value.size() &&
               towlower(pattern[pattern_index]) == towlower(value[value_index])) {
      result = self(self, value_index + 1, pattern_index + 1);
    }
    cached = result ? 1 : 0;
    return result;
  };
  return match(match, 0, 0);
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

bool IsWithin(const std::filesystem::path& relative) {
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
  if (!IsWithin(scoped) || scoped.empty()) return false;
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
                                         std::wstring_view text) {
  std::vector<IgnoreRule> rules;
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
      const bool exists = std::filesystem::exists(ignore_path, exists_error);
      if (exists_error) {
        cached->second.readable = false;
        report_issue({ignore_path, L".gitignore の状態を確認できないため、この配下は未処理です。"});
      } else if (exists) {
        Document ignore_document;
        std::wstring load_error;
        if (!ignore_document.Load(ignore_path, load_error)) {
          cached->second.readable = false;
          report_issue({ignore_path, L".gitignore を読み込めないため、この配下は未処理です: " +
                                         load_error});
        } else {
          const auto base = std::filesystem::relative(current, root).lexically_normal();
          cached->second.rules = ParseIgnoreRules(
              base == L"." ? std::filesystem::path{} : base, ignore_document.text());
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

bool SearchDocumentText(const std::filesystem::path& path, std::wstring_view text,
                        const SearchQuery& query, std::vector<SearchMatch>& matches,
                        std::wstring& error) {
  matches.clear();
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  return SearchText(path, text, query, matches, error);
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
  if (query.text.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  const bool can_report_issues = issues != nullptr || static_cast<bool>(issue_progress);
  const auto report_issue = [&](const SearchIssue& issue) {
    if (!can_report_issues) {
      error = issue.message;
      return false;
    }
    if (issues) issues->push_back(issue);
    if (issue_progress) issue_progress({issue});
    return true;
  };
  const auto absolute_root = std::filesystem::absolute(root).lexically_normal();
  IgnoreCache ignore_cache;
  std::vector<std::filesystem::path> pending_directories{absolute_root};
  while (!pending_directories.empty()) {
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
    for (; iterator != end;) {
    if (cancelled && cancelled()) {
      error = L"検索を中止しました。";
      return false;
    }
    const auto current_path = iterator->path();
    const auto relative = current_path.lexically_relative(absolute_root);
    std::error_code type_error;
    const auto status = iterator->symlink_status(type_error);
    const bool is_directory = !std::filesystem::is_symlink(status) &&
                              std::filesystem::is_directory(status);
    if (type_error) {
      if (!report_issue({current_path, L"ファイル種別を確認できないため未処理です。"})) return false;
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    if (IsExcluded(relative, query)) {
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    const auto ignore_decision = query.respect_gitignore
        ? EvaluateGitIgnore(absolute_root, relative, is_directory, ignore_cache, report_issue)
        : IgnoreDecision::Process;
    if (ignore_decision == IgnoreDecision::Unprocessed) {
      if (!can_report_issues) return false;
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    if (is_directory) {
      // Git does not inspect nested ignore files below an excluded directory;
      // a parent must first re-include that directory.  Avoid enumerating an
      // ignored subtree (and avoid surfacing permission failures from content
      // that is intentionally outside the search set).
      if (ignore_decision == IgnoreDecision::Process) pending_directories.push_back(current_path);
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    if (ignore_decision == IgnoreDecision::Ignore) {
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    const bool is_regular = std::filesystem::is_regular_file(status);
    if (!is_regular || !IsSearchable(current_path) || !MatchesGlobs(relative, query)) {
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    const std::size_t match_begin = matches.size();
    const auto absolute = std::filesystem::absolute(current_path).lexically_normal();
    const auto unsaved_match = unsaved.find(absolute);
    if (unsaved_match != unsaved.end()) {
      if (!SearchText(absolute, unsaved_match->second, query, matches, error)) return false;
      if (progress && matches.size() != match_begin)
        progress(std::vector<SearchMatch>(matches.begin() + static_cast<std::ptrdiff_t>(match_begin),
                                          matches.end()));
      iterator.increment(filesystem_error);
      if (filesystem_error) {
        if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
          return false;
        break;
      }
      continue;
    }
    Document document;
    std::wstring load_error;
    if (!document.Load(absolute, load_error)) {
      if (!report_issue({absolute, L"読み込めないため未処理です: " + load_error})) return false;
      iterator.increment(filesystem_error);
      continue;
    }
    if (!SearchText(absolute, document.text(), query, matches, error)) return false;
    if (progress && matches.size() != match_begin)
      progress(std::vector<SearchMatch>(matches.begin() + static_cast<std::ptrdiff_t>(match_begin),
                                        matches.end()));
    iterator.increment(filesystem_error);
    if (filesystem_error) {
      if (!report_issue({directory, L"ディレクトリの列挙を継続できず、残りは未処理です。"}))
        return false;
      break;
    }
    }
  }
  return true;
}

}  // namespace mdlite
