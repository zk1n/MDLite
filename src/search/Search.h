#pragma once

#include <filesystem>
#include <functional>
#include <map>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct SearchQuery {
  std::wstring text;
  bool match_case{};
  bool regular_expression{};
  bool whole_word{};
  bool include_hidden{};
  bool respect_gitignore{true};
  std::vector<std::wstring> include_globs;
  std::vector<std::wstring> exclude_globs;
};

struct SearchMatch {
  std::filesystem::path path;
  std::size_t begin{};
  std::size_t end{};
  std::size_t line{};
  std::size_t column{};
  std::wstring preview;
};

struct SearchIssue {
  std::filesystem::path path;
  std::wstring message;
};

bool SearchDocumentText(const std::filesystem::path& path, std::wstring_view text,
                        const SearchQuery& query, std::vector<SearchMatch>& matches,
                        std::wstring& error);

bool SearchWorkspace(const std::filesystem::path& root, const SearchQuery& query,
                     const std::map<std::filesystem::path, std::wstring>& unsaved,
                     std::vector<SearchMatch>& matches, std::wstring& error,
                     const std::function<bool()>& cancelled = {},
                     const std::function<void(std::vector<SearchMatch>)>& progress = {},
                     // With an issue sink, unreadable/unprocessed entries are reported and the
                     // remaining workspace is scanned. Without one, the first such issue is fatal.
                     std::vector<SearchIssue>* issues = nullptr,
                     const std::function<void(std::vector<SearchIssue>)>& issue_progress = {});

}  // namespace mdlite
