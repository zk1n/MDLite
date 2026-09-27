#include "git/GitPanel.h"
#include "process/ProcessRunner.h"

#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <utility>
#include <vector>

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

namespace {

int failures = 0;
int checks = 0;

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

void AppendRecord(std::wstring& output, std::wstring_view record) {
  output.append(record);
  output.push_back(L'\0');
}

void TestPorcelainStatus() {
  std::wstring output;
  AppendRecord(output, L"## feature/demo...origin/feature/demo [ahead 1]");
  AppendRecord(output, L" M notes/edited.md");
  AppendRecord(output, L"M  notes/staged.md");
  AppendRecord(output, L"?? notes/new.md");
  AppendRecord(output, L"R  notes/renamed.md");
  AppendRecord(output, L"notes/old.md");
  mdlite::GitPanelStatus status;
  std::wstring error;
  Check(mdlite::ParseGitStatusPorcelain(output, status, error), "porcelain status parses");
  Check(error.empty(), "successful parse has no error");
  Check(status.branch == L"feature/demo", "tracking suffix is removed from branch");
  Check(status.files.size() == 4, "all porcelain entries are exposed");
  Check(status.has_staged_changes, "staged bucket is detected");
  Check(status.has_unstaged_changes, "unstaged bucket is detected");
  Check(status.has_untracked_files, "untracked bucket is detected");
  Check(status.files[0].unstaged && !status.files[0].staged, "worktree-only entry is classified");
  Check(status.files[1].staged && !status.files[1].unstaged, "index-only entry is classified");
  Check(status.files[2].untracked, "untracked entry is classified");
  Check(status.files[3].staged && status.files[3].original_path == L"notes/old.md",
        "rename carries its original path");
}

void TestDetachedAndConflictStatus() {
  std::wstring detached;
  AppendRecord(detached, L"## HEAD (no branch)");
  AppendRecord(detached, L"UU merge.md");
  mdlite::GitPanelStatus status;
  std::wstring error;
  Check(mdlite::ParseGitStatusPorcelain(detached, status, error), "detached status parses");
  Check(status.detached_head, "detached head is explicit");
  Check(status.branch == L"(detached HEAD)", "detached branch label is stable");
  Check(status.files.size() == 1 && status.files[0].conflicted, "conflict is explicit");
  std::wstring malformed;
  AppendRecord(malformed, L" M malformed-without-space");
  Check(!mdlite::ParseGitStatusPorcelain(malformed, status, error),
        "missing branch header is rejected");
}

void TestExplicitCommands() {
  std::wstring error;
  auto stage = mdlite::BuildGitCommand(
      {mdlite::GitOperation::Stage, {L"notes/edited.md", L"folder/other.md"}}, error);
  Check(stage.has_value(), "stage command is built");
  Check(stage && stage->arguments == std::vector<std::wstring>{L"--literal-pathspecs", L"add", L"--",
                                                               L"notes/edited.md", L"folder/other.md"},
        "stage command has literal explicit paths");
  auto unstage = mdlite::BuildGitCommand(
      {mdlite::GitOperation::Unstage, {L"notes/staged.md"}}, error);
  Check(unstage && unstage->arguments == std::vector<std::wstring>{L"--literal-pathspecs", L"restore",
                                                                   L"--staged", L"--", L"notes/staged.md"},
        "unstage command has literal explicit paths");
  auto pathspec_magic = mdlite::BuildGitCommand(
      {mdlite::GitOperation::Stage, {L":(top)selected.md"}}, error);
  Check(pathspec_magic && pathspec_magic->arguments.back() == L":(top)selected.md",
        "pathspec-looking filename is passed as one literal path");
  auto empty = mdlite::BuildGitCommand({mdlite::GitOperation::Stage, {}}, error);
  Check(!empty && !error.empty(), "empty scope is rejected");
  auto wildcard = mdlite::BuildGitCommand(
      {mdlite::GitOperation::Stage, {L"*.md"}}, error);
  Check(!wildcard && !error.empty(), "wildcard scope is rejected");
}

void TestOperationState() {
  mdlite::GitPanelStatus current;
  current.state = mdlite::GitPanelState::NoRemote;
  current.branch = L"main";
  current.has_unstaged_changes = true;
  const auto busy = mdlite::GitPanelModel::OperationInProgress(current);
  Check(busy.state == mdlite::GitPanelState::OperationInProgress, "operation state is explicit");
  Check(busy.branch == L"main" && busy.has_unstaged_changes, "operation state preserves panel data");
}

void TestGitStatusResultFreshness() {
  const std::filesystem::path workspace_a = L"fixture-a";
  const std::filesystem::path workspace_b = L"fixture-b";
  Check(mdlite::IsGitResultCurrent(4, workspace_a, 4, workspace_a, workspace_a),
        "current generation and workspace result is accepted");
  Check(!mdlite::IsGitResultCurrent(3, workspace_a, 4, workspace_a, workspace_a),
        "stale generation result is rejected");
  Check(!mdlite::IsGitResultCurrent(4, workspace_a, 4, workspace_b, workspace_b),
        "result for a previous workspace is rejected after a workspace change");
  Check(!mdlite::IsGitResultCurrent(4, workspace_a, 4, workspace_a, workspace_b),
        "result is rejected when the pending request belongs to another workspace");
}

std::filesystem::path FindGit() {
  wchar_t* raw_path{};
  std::size_t path_size{};
  if (_wdupenv_s(&raw_path, &path_size, L"PATH") != 0 || !raw_path) return {};
  const std::wstring paths(raw_path);
  std::free(raw_path);
  std::size_t begin{};
  while (begin <= paths.size()) {
    const auto end = paths.find(L';', begin);
    auto directory = std::filesystem::path(paths.substr(begin, end == std::wstring::npos
                                                                   ? paths.size() - begin
                                                                   : end - begin));
    const auto git = directory / L"git.exe";
    if (std::filesystem::is_regular_file(git)) return git;
    if (end == std::wstring::npos) break;
    begin = end + 1;
  }
  return {};
}

bool RunGit(const std::filesystem::path& git, const std::filesystem::path& root,
            std::vector<std::wstring> args, std::wstring* output = nullptr) {
  args.insert(args.begin(), {L"-C", root.wstring()});
  mdlite::ProcessResult result;
  std::wstring error;
  const bool succeeded = mdlite::RunProcess(git, args, root, 64 * 1024, 30000, result, error) &&
                         result.exit_code == 0;
  if (succeeded && output) *output = std::move(result.output);
  return succeeded;
}

bool Contains(std::wstring_view text, std::wstring_view value) {
  return text.find(value) != std::wstring_view::npos;
}

void TestFileDiffsInTemporaryRepositories() {
  const auto git = FindGit();
  Check(!git.empty(), "git executable is available for isolated diff tests");
  if (git.empty()) return;
  std::error_code ec;
  const auto temp = std::filesystem::temp_directory_path();
  const auto owner_token = std::to_string(GetCurrentProcessId()) + "-" +
                           std::to_string(GetTickCount64());
  std::filesystem::path root;
  bool created{};
  for (unsigned int attempt = 0; attempt < 1000 && !created; ++attempt) {
    root = temp / (L"mdlite-git-diff-tests-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
                   std::to_wstring(GetTickCount64()) + L"-" + std::to_wstring(attempt));
    created = std::filesystem::create_directory(root, ec);
    if (ec) {
      Check(false, "unique temporary test directory is created without deleting an existing path");
      return;
    }
  }
  Check(created, "unique temporary test directory is created without deleting an existing path");
  if (!created) return;
  const auto ownership_marker = root / L".mdlite-test-owner";
  {
    std::ofstream marker(ownership_marker, std::ios::out | std::ios::trunc);
    marker << owner_token;
    if (!marker) {
      Check(false, "temporary directory ownership marker is written");
      return;
    }
  }
  const auto no_git = mdlite::GitPanelModel({}, root).Refresh();
  Check(no_git.state == mdlite::GitPanelState::NoGit && no_git.files.empty(),
        "missing Git executable is reported without attempting a process");
  const auto no_repository = root / L"no-repository";
  std::filesystem::create_directories(no_repository, ec);
  const auto no_repository_status = mdlite::GitPanelModel(git, no_repository).Refresh();
  Check(no_repository_status.state == mdlite::GitPanelState::NoRepository &&
            no_repository_status.files.empty(),
        "ordinary directory is reported as no repository without publishing stale files");
  const auto no_repository_operation = mdlite::GitPanelModel(git, no_repository).Execute(
      {mdlite::GitOperation::Stage, {L"new.md"}});
  Check(no_repository_operation.state == mdlite::GitPanelState::NoRepository &&
            !no_repository_operation.error.empty(),
        "stage in an ordinary directory reports NoRepository without success");

  const auto untracked = root / L"untracked";
  std::filesystem::create_directories(untracked, ec);
  Check(RunGit(git, untracked, {L"init", L"-b", L"main"}), "temporary untracked repository initializes");
  {
    std::ofstream file(untracked / L"new.md");
    file << "untracked body\n";
  }
  mdlite::GitPanelModel untracked_model(git, untracked);
  const auto new_file = untracked_model.DiffFile(L"new.md");
  Check(new_file.succeeded, "untracked file diff accepts no-index exit 1");
  Check(Contains(new_file.diff, L"untracked body") && Contains(new_file.diff, L"/dev/null"),
        "untracked diff is source-backed against /dev/null");

  const auto tracked = root / L"tracked";
  std::filesystem::create_directories(tracked, ec);
  Check(RunGit(git, tracked, {L"init", L"-b", L"main"}), "temporary tracked repository initializes");
  {
    std::ofstream file(tracked / L"tracked.md");
    file << "committed body\n";
  }
  Check(RunGit(git, tracked, {L"add", L"--", L"tracked.md"}), "tracked test file stages");
  Check(RunGit(git, tracked, {L"-c", L"user.name=MDLite Test", L"-c", L"user.email=test@example.invalid",
                              L"commit", L"-m", L"initial"}),
        "tracked test file commits in temporary repository");
  {
    std::ofstream file(tracked / L"tracked.md", std::ios::trunc);
    file << "changed body\n";
  }
  mdlite::GitPanelModel tracked_model(git, tracked);
  const auto tracked_diff = tracked_model.DiffFile(L"tracked.md");
  Check(tracked_diff.succeeded && Contains(tracked_diff.diff, L"committed body") &&
            Contains(tracked_diff.diff, L"changed body"),
        "tracked file diff compares source against HEAD");

  const auto unborn = root / L"unborn";
  std::filesystem::create_directories(unborn, ec);
  Check(RunGit(git, unborn, {L"init", L"-b", L"main"}), "temporary unborn repository initializes");
  {
    std::ofstream file(unborn / L"staged.md");
    file << "staged version\n";
  }
  Check(RunGit(git, unborn, {L"add", L"--", L"staged.md"}), "unborn test file stages without a commit");
  {
    std::ofstream file(unborn / L"staged.md", std::ios::trunc);
    file << "worktree version\n";
  }
  mdlite::GitPanelModel unborn_model(git, unborn);
  const auto staged_and_worktree = unborn_model.DiffFile(L"staged.md");
  Check(staged_and_worktree.succeeded, "unborn repository file diff succeeds");
  Check(Contains(staged_and_worktree.diff, L"staged version") &&
            Contains(staged_and_worktree.diff, L"worktree version"),
        "unborn diff includes separate staged and worktree changes");
  const auto unsafe = unborn_model.DiffFile(L"../outside.md");
  Check(!unsafe.succeeded && !unsafe.error.empty(), "diff rejects paths escaping the repository");

  const auto nested_repository = root / L"nested-repository";
  const auto nested_workspace = nested_repository / L"workspace";
  std::filesystem::create_directories(nested_workspace / L"workspace", ec);
  Check(RunGit(git, nested_repository, {L"init", L"-b", L"main"}),
        "temporary nested-workspace repository initializes");
  {
    std::ofstream selected_file(nested_workspace / L"selected.md");
    selected_file << "repository-root selected body\n";
    std::ofstream decoy_file(nested_workspace / L"workspace" / L"selected.md");
    decoy_file << "nested decoy body\n";
  }
  mdlite::GitPanelModel nested_model(git, nested_workspace);
  auto nested_status = nested_model.Refresh();
  Check(nested_status.repository_root == nested_repository,
        "nested workspace refresh identifies repository root");
  bool selected_untracked{};
  bool decoy_untracked{};
  for (const auto& file : nested_status.files) {
    selected_untracked = selected_untracked ||
                         (file.path == L"workspace/selected.md" && file.untracked);
    decoy_untracked = decoy_untracked ||
                      (file.path == L"workspace/workspace/selected.md" && file.untracked);
  }
  Check(selected_untracked && decoy_untracked,
        "nested refresh lists both untracked files individually");
  bool selected_staged{};
  decoy_untracked = false;
  auto nested_operation = nested_model.Execute(
      {mdlite::GitOperation::Stage, {L"workspace/selected.md"}});
  Check(nested_operation.state == mdlite::GitPanelState::Ready,
        "nested workspace stages repository-root-relative selected path");
  nested_status = nested_model.Refresh();
  selected_staged = false;
  decoy_untracked = false;
  for (const auto& file : nested_status.files) {
    selected_staged = selected_staged ||
                      (file.path == L"workspace/selected.md" && file.staged);
    decoy_untracked = decoy_untracked ||
                      (file.path == L"workspace/workspace/selected.md" && file.untracked);
  }
  Check(selected_staged && decoy_untracked,
        "stage selects repository-root path and leaves colliding decoy untracked");
  nested_operation = nested_model.Execute(
      {mdlite::GitOperation::Unstage, {L"workspace/selected.md"}});
  Check(nested_operation.state == mdlite::GitPanelState::Ready,
        "nested workspace unstages repository-root-relative selected path");
  nested_status = nested_model.Refresh();
  selected_untracked = false;
  decoy_untracked = false;
  for (const auto& file : nested_status.files) {
    selected_untracked = selected_untracked ||
                         (file.path == L"workspace/selected.md" && file.untracked);
    decoy_untracked = decoy_untracked ||
                      (file.path == L"workspace/workspace/selected.md" && file.untracked);
  }
  Check(!nested_status.has_staged_changes && selected_untracked && decoy_untracked,
        "unstage affects selected repository-root path and preserves decoy as untracked");

  const auto large_status_workspace = root / L"large-status";
  std::filesystem::create_directories(large_status_workspace, ec);
  Check(RunGit(git, large_status_workspace, {L"init", L"-b", L"main"}),
        "temporary large-status repository initializes");
  bool large_status_files_created = true;
  const std::wstring long_name_suffix(140, L'x');
  for (unsigned int index{}; index < 1800; ++index) {
    const auto path = large_status_workspace /
        (L"untracked_" + std::to_wstring(index) + L"_" + long_name_suffix + L".md");
    std::ofstream file(path);
    if (!file) {
      large_status_files_created = false;
      break;
    }
  }
  Check(large_status_files_created,
        "temporary large-status fixture creates enough untracked paths to exceed the output cap");
  mdlite::GitPanelModel large_status_model(git, large_status_workspace);
  const auto truncated_status = large_status_model.Refresh();
  Check(truncated_status.state == mdlite::GitPanelState::Error &&
            truncated_status.files.empty() && !truncated_status.error.empty(),
        "truncated Git status fails closed instead of publishing an incomplete file list");

  const auto offline = root / L"offline";
  std::filesystem::create_directories(offline, ec);
  Check(RunGit(git, offline, {L"init", L"-b", L"main"}), "offline test repository initializes without a remote");
  Check(RunGit(git, offline, {L"config", L"user.name", L"MDLite Test"}), "offline test author name is configured locally");
  Check(RunGit(git, offline, {L"config", L"user.email", L"test@example.invalid"}), "offline test author email is configured locally");
  {
    std::ofstream selected_file(offline / L"selected.md");
    selected_file << "selected body\n";
    std::ofstream other_file(offline / L"other.md");
    other_file << "other body\n";
  }
  mdlite::GitPanelModel offline_model(git, offline);
  auto offline_status = offline_model.Refresh();
  Check(offline_status.state == mdlite::GitPanelState::NoRemote,
        "local status remains available without a remote");
  auto operation = offline_model.Execute({mdlite::GitOperation::Stage, {L"selected.md"}});
  Check(operation.state == mdlite::GitPanelState::Ready, "explicit stage works offline");
  offline_status = offline_model.Refresh();
  Check(offline_status.has_staged_changes && offline_status.has_untracked_files,
        "status separates selected staged and other untracked files");
  operation = offline_model.Execute({mdlite::GitOperation::Unstage, {L"selected.md"}});
  Check(operation.state == mdlite::GitPanelState::Ready, "explicit unstage works offline");
  offline_status = offline_model.Refresh();
  Check(!offline_status.has_staged_changes && offline_status.has_untracked_files,
        "unstage leaves unrelated untracked files untouched");
  operation = offline_model.Execute({mdlite::GitOperation::Stage, {L"selected.md"}});
  Check(operation.state == mdlite::GitPanelState::Ready, "selected file can be staged again offline");
  {
    std::ofstream selected_file(offline / L"selected.md", std::ios::trunc);
    selected_file << "worktree edit B\n";
  }
  offline_status = offline_model.Refresh();
  bool selected_staged_mixed{};
  bool selected_unstaged_mixed{};
  bool other_untracked_mixed{};
  for (const auto& file : offline_status.files) {
    selected_staged_mixed = selected_staged_mixed ||
                            (file.path == L"selected.md" && file.staged);
    selected_unstaged_mixed = selected_unstaged_mixed ||
                              (file.path == L"selected.md" && file.unstaged);
    other_untracked_mixed = other_untracked_mixed ||
                            (file.path == L"other.md" && file.untracked);
  }
  Check(offline_status.state == mdlite::GitPanelState::NoRemote &&
            offline_status.has_staged_changes && offline_status.has_unstaged_changes &&
            offline_status.has_untracked_files && selected_staged_mixed &&
            selected_unstaged_mixed && other_untracked_mixed,
        "offline mixed status keeps staged, unstaged, and unrelated untracked changes distinct");
  Check(RunGit(git, offline, {L"commit", L"-m", L"commit staged index"}),
        "pathless Git commit records the staged index");
  std::wstring committed_content;
  Check(RunGit(git, offline, {L"show", L"HEAD:selected.md"}, &committed_content),
        "committed selected file can be read from HEAD");
  Check(Contains(committed_content, L"selected body") &&
            !Contains(committed_content, L"worktree edit B"),
        "pathless commit records staged A instead of later worktree edit B");
  offline_status = offline_model.Refresh();
  bool selected_unstaged{};
  bool other_untracked{};
  for (const auto& file : offline_status.files) {
    selected_unstaged = selected_unstaged ||
                        (file.path == L"selected.md" && file.unstaged);
    other_untracked = other_untracked ||
                      (file.path == L"other.md" && file.untracked);
  }
  Check(!offline_status.has_staged_changes && selected_unstaged && other_untracked,
        "pathless commit leaves edit B unstaged and unrelated file untracked");
  std::string recorded_owner;
  {
    std::ifstream marker(ownership_marker);
    marker >> recorded_owner;
  }
  const bool owns_root = std::filesystem::equivalent(root.parent_path(), temp, ec) &&
                         std::filesystem::is_directory(root) && recorded_owner == owner_token;
  Check(owns_root, "temporary test directory ownership is verified before cleanup");
  if (owns_root) std::filesystem::remove_all(root, ec);
  Check(!owns_root || !ec, "owned temporary test directory is removed");
}

}  // namespace

int main() {
  TestPorcelainStatus();
  TestDetachedAndConflictStatus();
  TestExplicitCommands();
  TestOperationState();
  TestGitStatusResultFreshness();
  TestFileDiffsInTemporaryRepositories();
  std::cout << "Git panel checks: " << checks << ", failures: " << failures << '\n';
  return failures == 0 ? 0 : 1;
}
