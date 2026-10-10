#pragma once

#include "search/Search.h"

#include <filesystem>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace mdlite {

struct ReplaceMatchSelection {
  std::size_t begin{};
  std::size_t end{};
  std::uint64_t snapshot_hash{};
};

struct ReplaceFilePlan {
  std::filesystem::path path;
  std::wstring before;
  std::wstring after;
  std::size_t replacement_count{};
  std::optional<ReplaceMatchSelection> selected_match;
};

struct ReplacePlan {
  SearchQuery query;
  std::wstring replacement;
  std::vector<ReplaceFilePlan> files;
  std::vector<SearchIssue> search_issues;
};

struct ReplaceApplyResult {
  std::size_t applied_files{};
  std::size_t applied_replacements{};
  std::vector<std::filesystem::path> conflicts;
  std::filesystem::path journal;
};

struct ReplaceRollbackFilePreview {
  std::filesystem::path path;
  std::wstring expected_current;
  std::wstring restore_text;
};

struct ReplaceRollbackPreview {
  std::filesystem::path owning_workspace;
  std::filesystem::path journal;
  std::vector<ReplaceRollbackFilePreview> files;
};

bool ReplaceDocumentText(std::wstring_view source, const SearchQuery& query,
                         std::wstring_view replacement, std::wstring& output,
                         std::size_t& count, std::wstring& error);

bool ReplaceDocumentMatch(std::wstring_view source, const SearchQuery& query,
                          std::wstring_view replacement, std::size_t start,
                          std::wstring& output, std::size_t& replaced_begin,
                          std::size_t& replaced_end, bool& replaced, std::wstring& error);

// Replaces the exact source span selected from search results, preserving regex captures.
bool ReplaceDocumentMatch(std::wstring_view source, const SearchQuery& query,
                          std::wstring_view replacement, std::size_t selected_begin,
                          std::size_t selected_source_end, std::wstring& output,
                          std::size_t& replaced_begin, std::size_t& replaced_end,
                          bool& replaced, std::wstring& error);

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

// Read-only preview of the files that are still in the journal's after-snapshot.
bool PreviewWorkspaceReplaceRollback(const std::filesystem::path& current_workspace,
                                    const std::filesystem::path& journal,
                                    ReplaceRollbackPreview& preview,
                                    std::wstring& error);

}  // namespace mdlite
