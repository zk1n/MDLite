#pragma once

#include <filesystem>
#include <functional>
#include <map>
#include <cstdint>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

// Search is intentionally bounded so a workspace with large or generated files
// cannot consume unbounded memory or leave stale UI results behind.
inline constexpr std::uint64_t kSearchMaxFileBytes = 8ULL * 1024ULL * 1024ULL;
inline constexpr std::uint64_t kSearchMaxWorkspaceBytes = 512ULL * 1024ULL * 1024ULL;
inline constexpr std::size_t kSearchMaxIgnoreFileBytes = 64ULL * 1024ULL;
inline constexpr std::size_t kSearchMaxIgnoreBytes = 4ULL * 1024ULL * 1024ULL;
inline constexpr std::size_t kSearchMaxIgnoreRules = 10'000;
inline constexpr std::size_t kSearchMaxEntries = 200'000;
inline constexpr std::size_t kSearchMaxMatches = 20'000;
inline constexpr std::size_t kSearchMaxPreviewCodeUnits = 256;
inline constexpr std::size_t kSearchMaxResultPayloadBytes = 8ULL * 1024ULL * 1024ULL;
inline constexpr std::size_t kSearchMaxReplacePlanBytes = 64ULL * 1024ULL * 1024ULL;
inline constexpr std::size_t kSearchRegexMaxPatternLength = 4'096;
inline constexpr std::uint64_t kSearchRegexDeadlineMilliseconds = 200;

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
  std::size_t preview_match_begin{};
  std::size_t preview_match_end{};
  std::uint64_t snapshot_hash{};
};

enum class SearchIssueKind { Error, Excluded };

struct SearchIssue {
  std::filesystem::path path;
  std::wstring message;
  SearchIssueKind kind{SearchIssueKind::Error};
};

struct SearchRegexCapture {
  std::size_t begin{};
  std::size_t end{};
  bool matched{};
};

struct SearchRegexMatch {
  std::size_t begin{};
  std::size_t end{};
  std::vector<SearchRegexCapture> captures;
};

bool SearchDocumentText(const std::filesystem::path& path, std::wstring_view text,
                        const SearchQuery& query, std::vector<SearchMatch>& matches,
                        std::wstring& error,
                        const std::function<bool()>& cancelled = {},
                        bool* truncated = nullptr);

// Stable, non-cryptographic content identity used to match search results to
// the exact text snapshot shown in a replacement preview.
std::uint64_t SearchSnapshotHash(std::wstring_view text) noexcept;

// Applies the same Unicode whole-word boundary rule used by document search.
bool IsSearchWholeWordMatch(std::wstring_view text, std::size_t begin,
                            std::size_t end) noexcept;

// Validates empty/overly complex patterns before a query touches document data.
bool ValidateSearchQuery(const SearchQuery& query, std::wstring& error);

bool SearchRegexMatches(std::wstring_view text, const SearchQuery& query,
                        std::vector<SearchRegexMatch>& matches, std::wstring& error,
                        const std::function<bool()>& cancelled = {},
                        std::size_t max_matches = kSearchMaxMatches,
                        bool* truncated = nullptr,
                        std::size_t minimum_begin = 0,
                        bool stop_after_first = false);

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
