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

bool SearchWorkspace(const std::filesystem::path& root, const SearchQuery& query,
                     const std::map<std::filesystem::path, std::wstring>& unsaved,
                     std::vector<SearchMatch>& matches, std::wstring& error,
                     const std::function<bool()>& cancelled = {});

}  // namespace mdlite
