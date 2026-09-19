#pragma once

#include "search/Search.h"

#include <filesystem>
#include <map>
#include <string>
#include <vector>

namespace mdlite {

struct ReplaceFilePlan {
  std::filesystem::path path;
  std::wstring before;
  std::wstring after;
  std::size_t replacement_count{};
};

struct ReplacePlan {
  SearchQuery query;
  std::wstring replacement;
  std::vector<ReplaceFilePlan> files;
};

struct ReplaceApplyResult {
  std::size_t applied_files{};
  std::size_t applied_replacements{};
  std::vector<std::filesystem::path> conflicts;
  std::filesystem::path journal;
};

bool PreviewWorkspaceReplace(const std::filesystem::path& root, const SearchQuery& query,
                             std::wstring_view replacement,
                             const std::map<std::filesystem::path, std::wstring>& unsaved,
                             ReplacePlan& plan, std::wstring& error,
                             const std::function<bool()>& cancelled = {});
bool ApplyWorkspaceReplace(const std::filesystem::path& workspace, const ReplacePlan& plan,
                           ReplaceApplyResult& result, std::wstring& error,
                           const std::function<bool()>& cancelled = {});
bool RollbackWorkspaceReplace(const std::filesystem::path& journal,
                              ReplaceApplyResult& result, std::wstring& error);

}  // namespace mdlite
