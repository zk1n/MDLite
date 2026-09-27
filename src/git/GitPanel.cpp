#include "git/GitPanel.h"

#include <algorithm>
#include <cwctype>
#include <utility>

namespace mdlite {
namespace {

constexpr std::size_t kProcessOutputLimit = 256 * 1024;
constexpr unsigned long kProcessTimeoutMs = 30000;

std::vector<std::wstring> SplitNul(std::wstring_view output) {
  std::vector<std::wstring> records;
  std::size_t begin{};
  for (;;) {
    const auto end = output.find(L'\0', begin);
    if (end == std::wstring_view::npos) {
      if (begin < output.size()) records.emplace_back(output.substr(begin));
      break;
    }
    records.emplace_back(output.substr(begin, end - begin));
    begin = end + 1;
  }
  return records;
}

std::wstring Trim(std::wstring_view value) {
  std::size_t begin{};
  while (begin < value.size() && std::iswspace(value[begin])) ++begin;
  std::size_t end = value.size();
  while (end > begin && std::iswspace(value[end - 1])) --end;
  return std::wstring(value.substr(begin, end - begin));
}

bool IsPathSafe(std::wstring_view value) {
  if (value.empty() || value == L"." || value == L"..") return false;
  if (value.find_first_of(L"*?[") != std::wstring_view::npos) return false;
  if (value.front() == L'/' || value.front() == L'\\') return false;
  if (value.size() >= 2 && std::iswalpha(value[0]) && value[1] == L':') return false;
  for (std::size_t begin{}; begin < value.size();) {
    const auto end = value.find_first_of(L"/\\", begin);
    const auto component = value.substr(begin, end == std::wstring_view::npos ? value.size() - begin
                                                                           : end - begin);
    if (component == L"..") return false;
    if (end == std::wstring_view::npos) break;
    begin = end + 1;
    if (begin == value.size()) return false;
  }
  return true;
}

std::wstring GitPathArgument(const std::filesystem::path& path) {
  std::wstring value = path.generic_wstring();
  while (value.starts_with(L"./")) value.erase(0, 2);
  return value;
}

bool IsNoRepositoryMessage(std::wstring_view output) {
  std::wstring lower(output);
  std::transform(lower.begin(), lower.end(), lower.begin(),
                 [](wchar_t character) { return std::towlower(character); });
  return lower.find(L"not a git repository") != std::wstring::npos ||
         lower.find(L"not a git repo") != std::wstring::npos;
}

void SetFailure(GitPanelStatus& status, GitPanelState state, std::wstring message) {
  status.state = state;
  status.error = std::move(message);
}

bool ParseBranchHeader(std::wstring_view record, GitPanelStatus& status) {
  if (!record.starts_with(L"## ")) return false;
  std::wstring header = Trim(record.substr(3));
  if (header == L"HEAD (no branch)") {
    status.detached_head = true;
    status.branch = L"(detached HEAD)";
    return true;
  }
  constexpr std::wstring_view kNoCommits = L"No commits yet on ";
  constexpr std::wstring_view kInitialCommit = L"Initial commit on ";
  if (header.starts_with(kNoCommits)) {
    header.erase(0, kNoCommits.size());
  } else if (header.starts_with(kInitialCommit)) {
    header.erase(0, kInitialCommit.size());
  } else {
    const auto tracking = header.find(L"...");
    if (tracking != std::wstring::npos) header.erase(tracking);
  }
  const auto separator = header.find(L" [");
  if (separator != std::wstring::npos) header.erase(separator);
  status.branch = Trim(header);
  return true;
}

bool IsConflict(wchar_t index_status, wchar_t worktree_status) {
  return index_status == L'U' || worktree_status == L'U' ||
         (index_status == L'A' && worktree_status == L'A') ||
         (index_status == L'D' && worktree_status == L'D');
}

bool RunGit(const std::filesystem::path& executable, const std::filesystem::path& workspace,
            const std::vector<std::wstring>& arguments, void* cancellation_event,
            ProcessResult& result, std::wstring& error) {
  return RunProcess(executable, arguments, workspace, kProcessOutputLimit,
                    kProcessTimeoutMs, result, error, cancellation_event);
}

}  // namespace

bool ParseGitStatusPorcelain(std::wstring_view output, GitPanelStatus& status,
                             std::wstring& error) {
  status = {};
  status.state = GitPanelState::Ready;
  error.clear();
  const auto records = SplitNul(output);
  bool saw_branch{};
  for (std::size_t index{}; index < records.size(); ++index) {
    const std::wstring& record = records[index];
    if (record.empty()) continue;
    if (record.starts_with(L"## ")) {
      saw_branch = ParseBranchHeader(record, status) || saw_branch;
      continue;
    }
    if (record.size() < 4 || record[2] != L' ') {
      error = L"Git statusのporcelainレコードを解釈できません。";
      return false;
    }
    GitFileStatus file;
    file.index_status = record[0];
    file.worktree_status = record[1];
    file.path = record.substr(3);
    file.staged = file.index_status != L' ' && file.index_status != L'?';
    file.unstaged = file.worktree_status != L' ' && file.worktree_status != L'?';
    file.untracked = file.index_status == L'?' && file.worktree_status == L'?';
    file.conflicted = IsConflict(file.index_status, file.worktree_status);
    if ((file.index_status == L'R' || file.index_status == L'C') && index + 1 < records.size()) {
      file.original_path = records[++index];
    }
    status.has_staged_changes = status.has_staged_changes || file.staged;
    status.has_unstaged_changes = status.has_unstaged_changes || file.unstaged;
    status.has_untracked_files = status.has_untracked_files || file.untracked;
    status.files.push_back(std::move(file));
  }
  if (!saw_branch) {
    error = L"Git statusにbranchヘッダーがありません。";
    return false;
  }
  return true;
}

bool IsGitResultCurrent(
    std::uint64_t result_generation,
    const std::filesystem::path& result_workspace,
    std::uint64_t current_generation,
    const std::filesystem::path& active_workspace,
    const std::filesystem::path& requested_workspace) {
  return result_generation == current_generation &&
         result_workspace == active_workspace &&
         result_workspace == requested_workspace;
}

std::optional<GitCommand> BuildGitCommand(const GitActionRequest& request,
                                           std::wstring& error) {
  error.clear();
  if (request.paths.empty()) {
    error = L"Git操作には対象ファイルを明示してください。";
    return std::nullopt;
  }
  GitCommand command;
  switch (request.operation) {
    case GitOperation::Stage:
      command.arguments = {L"--literal-pathspecs", L"add", L"--"};
      command.description = L"Git stage";
      break;
    case GitOperation::Unstage:
      command.arguments = {L"--literal-pathspecs", L"restore", L"--staged", L"--"};
      command.description = L"Git unstage";
      break;
  }
  for (const auto& path : request.paths) {
    const std::wstring value = GitPathArgument(path);
    if (!IsPathSafe(value)) {
      error = L"Git操作の対象pathはrepository相対の明示pathにしてください。";
      return std::nullopt;
    }
    command.arguments.push_back(value);
  }
  return command;
}

GitPanelModel::GitPanelModel(std::filesystem::path git_executable,
                             std::filesystem::path workspace)
    : git_executable_(std::move(git_executable)), workspace_(std::move(workspace)) {}

GitPanelStatus GitPanelModel::Refresh(void* cancellation_event) const {
  GitPanelStatus status;
  if (git_executable_.empty()) {
    status.state = GitPanelState::NoGit;
    status.error = L"git.exeが見つかりません。";
    return status;
  }

  ProcessResult root_result;
  std::wstring error;
  if (!RunGit(git_executable_, workspace_,
              {L"-C", workspace_.wstring(), L"rev-parse", L"--show-toplevel"},
              cancellation_event, root_result, error)) {
    SetFailure(status, GitPanelState::Error, error);
    return status;
  }
  if (root_result.exit_code != 0) {
    SetFailure(status, IsNoRepositoryMessage(root_result.output) ? GitPanelState::NoRepository
                                                                   : GitPanelState::Error,
               root_result.output.empty() ? L"Git repositoryを確認できません。" : root_result.output);
    return status;
  }
  const auto repository_root = std::filesystem::path(Trim(root_result.output));

  ProcessResult status_result;
  if (!RunGit(git_executable_, workspace_,
              {L"-C", workspace_.wstring(), L"status", L"--porcelain=v1", L"--branch", L"-z",
               L"--untracked-files=all"},
              cancellation_event, status_result, error)) {
    SetFailure(status, GitPanelState::Error, error);
    return status;
  }
  if (status_result.exit_code != 0) {
    SetFailure(status, IsNoRepositoryMessage(status_result.output) ? GitPanelState::NoRepository
                                                                    : GitPanelState::Error,
               status_result.output.empty() ? L"Git statusを取得できません。" : status_result.output);
    return status;
  }
  if (status_result.truncated || status_result.output.empty() ||
      status_result.output.back() != L'\0') {
    SetFailure(status, GitPanelState::Error,
               L"Git statusの出力が上限を超えたか不完全なため、一覧を更新できません。範囲を絞って再試行してください。");
    return status;
  }
  if (!ParseGitStatusPorcelain(status_result.output, status, error)) {
    SetFailure(status, GitPanelState::Error, error);
    return status;
  }
  status.repository_root = repository_root;

  ProcessResult remote_result;
  if (!RunGit(git_executable_, workspace_, {L"-C", workspace_.wstring(), L"remote"},
              cancellation_event, remote_result, error)) {
    SetFailure(status, GitPanelState::Error, error);
    return status;
  }
  if (remote_result.exit_code != 0) {
    SetFailure(status, GitPanelState::Error,
               remote_result.output.empty() ? L"Git remoteを確認できません。" : remote_result.output);
    return status;
  }
  status.has_remote = !Trim(remote_result.output).empty();
  status.state = status.has_remote ? GitPanelState::Ready : GitPanelState::NoRemote;
  return status;
}

GitOperationResult GitPanelModel::Execute(const GitActionRequest& request,
                                          void* cancellation_event) const {
  GitOperationResult result;
  const auto command = BuildGitCommand(request, result.error);
  if (!command) {
    result.state = GitPanelState::Error;
    return result;
  }
  if (git_executable_.empty()) {
    result.state = GitPanelState::NoGit;
    result.error = L"git.exeが見つかりません。";
    return result;
  }
  ProcessResult root_result;
  std::wstring process_error;
  if (!RunGit(git_executable_, workspace_,
              {L"-C", workspace_.wstring(), L"rev-parse", L"--show-toplevel"},
              cancellation_event, root_result, process_error)) {
    result.state = GitPanelState::Error;
    result.error = std::move(process_error);
    return result;
  }
  if (root_result.exit_code != 0) {
    result.state = IsNoRepositoryMessage(root_result.output) ? GitPanelState::NoRepository
                                                             : GitPanelState::Error;
    result.error = root_result.output.empty() ? L"Git repositoryを確認できません。"
                                              : std::move(root_result.output);
    return result;
  }
  const auto repository_root = std::filesystem::path(Trim(root_result.output));
  result.state = GitPanelState::OperationInProgress;
  std::vector<std::wstring> operation_arguments = command->arguments;
  if (request.operation == GitOperation::Unstage) {
    ProcessResult head;
    const bool checked_head = RunGit(
        git_executable_, repository_root,
        {L"-C", repository_root.wstring(), L"rev-parse", L"--verify", L"HEAD"},
        cancellation_event, head, process_error);
    if (!checked_head) {
      result.state = GitPanelState::Error;
      result.error = std::move(process_error);
      return result;
    }
    if (head.exit_code != 0) {
      if (head.output.find(L"Needed a single revision") == std::wstring::npos) {
        result.state = GitPanelState::Error;
        result.error = head.output.empty() ? L"Git HEADを確認できません。" : head.output;
        return result;
      }
      operation_arguments = {L"--literal-pathspecs", L"rm", L"--cached", L"--quiet", L"--"};
      for (const auto& path : request.paths) operation_arguments.push_back(GitPathArgument(path));
    }
  }
  std::vector<std::wstring> arguments{L"-C", repository_root.wstring()};
  arguments.insert(arguments.end(), operation_arguments.begin(), operation_arguments.end());
  const bool started = RunGit(git_executable_, repository_root, arguments, cancellation_event,
                              result.process, process_error);
  result.state = started && result.process.exit_code == 0 ? GitPanelState::Ready
                                                           : GitPanelState::Error;
  if (!started) result.error = process_error;
  else if (!result.process.output.empty()) result.error = result.process.output;
  else if (result.state == GitPanelState::Error) result.error = L"Git操作に失敗しました。";
  return result;
}

GitFileDiffResult GitPanelModel::DiffFile(const std::filesystem::path& path,
                                          void* cancellation_event) const {
  GitFileDiffResult result;
  const std::wstring value = GitPathArgument(path);
  if (!IsPathSafe(value)) {
    result.error = L"Git diffの対象pathはrepository相対の明示pathにしてください。";
    return result;
  }
  if (git_executable_.empty()) {
    result.error = L"git.exeが見つかりません。";
    return result;
  }

  GitPanelStatus status = Refresh(cancellation_event);
  if (status.state != GitPanelState::Ready && status.state != GitPanelState::NoRemote) {
    result.error = status.error.empty() ? L"Git repositoryを確認できません。" : status.error;
    return result;
  }
  const auto selected = std::find_if(status.files.begin(), status.files.end(),
                                     [&](const GitFileStatus& file) {
                                       return GitPathArgument(file.path) == value;
                                     });
  if (selected == status.files.end()) {
    result.error = L"選択したpathにGit変更がありません。";
    return result;
  }

  const auto run_diff = [&](std::vector<std::wstring> arguments,
                            bool accept_no_index_difference) {
    arguments.insert(arguments.begin(), L"--literal-pathspecs");
    arguments.insert(arguments.begin() + 2, L"--no-ext-diff");
    arguments.insert(arguments.begin() + 3, L"--no-textconv");
    ProcessResult process;
    std::wstring process_error;
    if (!RunGit(git_executable_, status.repository_root, arguments,
                cancellation_event, process, process_error)) {
      result.error = std::move(process_error);
      return false;
    }
    if (process.exit_code != 0 && !(accept_no_index_difference && process.exit_code == 1 &&
                                   !process.output.empty())) {
      result.error = process.output.empty() ? L"Git diffを取得できません。" : process.output;
      return false;
    }
    result.diff += process.output;
    result.truncated = result.truncated || process.truncated;
    return true;
  };

  ProcessResult head;
  std::wstring process_error;
  if (!RunGit(git_executable_, status.repository_root,
              {L"rev-parse", L"--verify", L"HEAD"}, cancellation_event,
              head, process_error)) {
    result.error = std::move(process_error);
    return result;
  }
  const bool has_head = head.exit_code == 0;
  if (selected->untracked) {
    if (!run_diff({L"diff", L"--no-index", L"--", L"/dev/null", value}, true)) return result;
  } else if (has_head) {
    if (!run_diff({L"diff", L"HEAD", L"--", value}, false)) return result;
  } else {
    if (selected->staged &&
        !run_diff({L"diff", L"--cached", L"--", value}, false)) return result;
    if (selected->unstaged &&
        !run_diff({L"diff", L"--", value}, false)) return result;
  }
  result.succeeded = true;
  return result;
}

GitPanelStatus GitPanelModel::OperationInProgress(const GitPanelStatus& current) {
  GitPanelStatus status = current;
  status.state = GitPanelState::OperationInProgress;
  status.error.clear();
  return status;
}

}  // namespace mdlite
