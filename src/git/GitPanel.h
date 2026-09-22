#pragma once

#include "process/ProcessRunner.h"

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
  Commit,
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

struct GitActionRequest {
  GitOperation operation{GitOperation::Stage};
  std::vector<std::filesystem::path> paths;
  std::wstring commit_message;
};

struct GitCommand {
  std::vector<std::wstring> arguments;
  std::wstring description;
};

// Every mutating action requires an explicit, repository-relative path list.
// This function never emits `--all`, `.`, or an empty pathspec.
std::optional<GitCommand> BuildGitCommand(const GitActionRequest& request,
                                           std::wstring& error);

struct GitOperationResult {
  GitPanelState state{GitPanelState::Error};
  ProcessResult process;
  std::wstring error;
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

  static GitPanelStatus OperationInProgress(const GitPanelStatus& current);

 private:
  std::filesystem::path git_executable_;
  std::filesystem::path workspace_;
};

}  // namespace mdlite
