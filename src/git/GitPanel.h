#pragma once

#include "process/ProcessRunner.h"

#include <cstdint>
#include <filesystem>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

enum class GitPanelState {
  NoGit,
  NoRepository,
  Ready,
  NoRemote,
  OperationInProgress,
  Error,
};

enum class GitOperation {
  Stage,
  Unstage,
};

struct GitFileStatus {
  std::filesystem::path path;
  std::filesystem::path original_path;
  wchar_t index_status{' '};
  wchar_t worktree_status{' '};
  bool staged{};
  bool unstaged{};
  bool untracked{};
  bool conflicted{};
};

struct GitPanelStatus {
  GitPanelState state{GitPanelState::NoGit};
  std::filesystem::path repository_root;
  std::wstring branch;
  bool detached_head{};
  bool has_remote{};
  bool has_staged_changes{};
  bool has_unstaged_changes{};
  bool has_untracked_files{};
  std::vector<GitFileStatus> files;
  std::wstring error;
};

// Parse `git status --porcelain=v1 --branch -z` without changing repository
// state. Keeping this independent from ProcessRunner makes captured status
// responses easy to render and test.
bool ParseGitStatusPorcelain(std::wstring_view output, GitPanelStatus& status,
                             std::wstring& error);

// A completed asynchronous Git result is current only when its generation and
// both workspace identities still match the active request.
bool IsGitResultCurrent(
    std::uint64_t result_generation,
    const std::filesystem::path& result_workspace,
    std::uint64_t current_generation,
    const std::filesystem::path& active_workspace,
    const std::filesystem::path& requested_workspace);

struct GitActionRequest {
  GitOperation operation{GitOperation::Stage};
  std::vector<std::filesystem::path> paths;
};

struct GitCommand {
  std::vector<std::wstring> arguments;
  std::wstring description;
};

// Builds an explicit Stage or Unstage command for repository-relative paths.
// This function never emits `--all`, `.`, or an empty pathspec.
std::optional<GitCommand> BuildGitCommand(const GitActionRequest& request,
                                           std::wstring& error);

struct GitOperationResult {
  GitPanelState state{GitPanelState::Error};
  ProcessResult process;
  std::wstring error;
};

struct GitFileDiffResult {
  bool succeeded{};
  std::wstring diff;
  std::wstring error;
  bool truncated{};
};

class GitPanelModel final {
 public:
  GitPanelModel(std::filesystem::path git_executable,
                std::filesystem::path workspace);

  // Read-only refresh: no document save and no repository mutation.
  GitPanelStatus Refresh(void* cancellation_event = nullptr) const;

  // Executes only one validated explicit action. It never saves documents or
  // stages, commits, or pushes implicitly.
  GitOperationResult Execute(const GitActionRequest& request,
                             void* cancellation_event = nullptr) const;

  // Read-only diff for one explicit repository-relative file. Untracked files
  // use `diff --no-index /dev/null`; unborn repositories return staged and
  // worktree diffs in separate sections.
  GitFileDiffResult DiffFile(const std::filesystem::path& path,
                             void* cancellation_event = nullptr) const;

  static GitPanelStatus OperationInProgress(const GitPanelStatus& current);

 private:
  std::filesystem::path git_executable_;
  std::filesystem::path workspace_;
};

}  // namespace mdlite
