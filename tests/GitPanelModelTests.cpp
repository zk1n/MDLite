#include "git/GitPanel.h"

#include <iostream>
#include <string>
#include <vector>

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
      {mdlite::GitOperation::Stage, {L"notes/edited.md", L"folder/other.md"}, {}}, error);
  Check(stage.has_value(), "stage command is built");
  Check(stage && stage->arguments == std::vector<std::wstring>{L"add", L"--", L"notes/edited.md", L"folder/other.md"},
        "stage command has explicit paths");
  auto unstage = mdlite::BuildGitCommand({mdlite::GitOperation::Unstage, {L"notes/staged.md"}, {}}, error);
  Check(unstage && unstage->arguments == std::vector<std::wstring>{L"restore", L"--staged", L"--", L"notes/staged.md"},
        "unstage command has explicit paths");
  auto commit = mdlite::BuildGitCommand(
      {mdlite::GitOperation::Commit, {L"notes/staged.md"}, L"save selected"}, error);
  Check(commit && commit->arguments == std::vector<std::wstring>{L"commit", L"-m", L"save selected", L"--", L"notes/staged.md"},
        "commit command has explicit paths");
  auto empty = mdlite::BuildGitCommand({mdlite::GitOperation::Stage, {}, {}}, error);
  Check(!empty && !error.empty(), "empty scope is rejected");
  auto wildcard = mdlite::BuildGitCommand({mdlite::GitOperation::Stage, {L"*.md"}, {}}, error);
  Check(!wildcard && !error.empty(), "wildcard scope is rejected");
  auto blank_message = mdlite::BuildGitCommand({mdlite::GitOperation::Commit, {L"a.md"}, L"  \t"}, error);
  Check(!blank_message && !error.empty(), "blank commit message is rejected");
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

}  // namespace

int main() {
  TestPorcelainStatus();
  TestDetachedAndConflictStatus();
  TestExplicitCommands();
  TestOperationState();
  std::cout << "Git panel checks: " << checks << ", failures: " << failures << '\n';
  return failures == 0 ? 0 : 1;
}
