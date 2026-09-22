#include "app/Application.h"

#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "calendar/JapaneseHolidays.h"
#include "calendar/CalendarDayIndex.h"
#include "git/Conflict.h"
#include "search/Search.h"
#include "git/GitPanel.h"
#include "search/Replace.h"
#include "process/ProcessRunner.h"
#include "workspace/Trust.h"

#include <commctrl.h>
#include <dwmapi.h>
#include <commdlg.h>
#include <richedit.h>
#include <richole.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <shlwapi.h>
#include <tom.h>
#include <wincodec.h>

#include <algorithm>
#include <atomic>
#include <ctime>
#include <array>
#include <chrono>
#include <cwctype>
#include <fstream>
#include <functional>
#include <map>
#include <thread>

namespace mdlite {
namespace {

constexpr wchar_t kWindowClass[] = L"MDLite.MainWindow";
constexpr wchar_t kCompactWindowClass[] = L"MDLite.CompactWindow";
constexpr UINT_PTR kAutosaveTimer = 1;
constexpr UINT kTimerPollMs = 50;
constexpr ULONGLONG kEditorSyncDelayMs = 500;
constexpr ULONGLONG kPresentationDelayMs = 250;
constexpr UINT kRecoveryDelayMs = 5000;
constexpr int kTreeWidth = 250;
constexpr int kOutlineWidth = 230;
constexpr int kTabHeight = 30;
constexpr int kFindHeight = 96;
constexpr int kFindResultsHeight = 170;
constexpr int kMinimumEditorWidth = 360;
constexpr int kMinimumPaneWidth = 156;
constexpr int kFindCompactWidth = 640;
constexpr int kProcessDoneButton = 4400;
constexpr UINT kWorkspaceSearchBatchMessage = WM_APP + 41;
constexpr UINT kWorkspaceSearchCompleteMessage = WM_APP + 42;
constexpr UINT kTestThemeChangeMessage = WM_APP + 43;
constexpr UINT kHolidayUpdateMessage = WM_APP + 44;
constexpr wchar_t kHolidayCacheName[] = L"japanese-holidays.csv";
constexpr wchar_t kHolidayStateName[] = L"japanese-holidays.toml";
constexpr wchar_t kHolidayProvider[] = L"内閣府 国民の祝日・休日CSV";
constexpr ULONG_PTR kOwnerDrawSeparator = 1;

int ScaleDip(HWND window, int value) {
  const UINT dpi = window == nullptr ? 96U : GetDpiForWindow(window);
  return MulDiv(value, dpi == 0 ? 96 : static_cast<int>(dpi), 96);
}

bool TestAutomationSilent() {
  wchar_t enabled[2]{};
  return GetEnvironmentVariableW(L"MDLITE_TEST_SILENT", enabled, 2) == 1 && enabled[0] == L'1';
}

int TestAwareMessageBoxW(HWND owner, LPCWSTR text, LPCWSTR caption, UINT type) {
  if (!TestAutomationSilent()) return ::MessageBoxW(owner, text, caption, type);

  // Acceptance automation must never surface a modal dialog or play the
  // Windows message-box sound.  Expected affirmative choices are handled at
  // their call sites; unexpected confirmations fail closed here.
  switch (type & MB_TYPEMASK) {
    case MB_YESNO:
      return IDNO;
    case MB_YESNOCANCEL:
    case MB_OKCANCEL:
    case MB_RETRYCANCEL:
    case MB_CANCELTRYCONTINUE:
      return IDCANCEL;
    case MB_ABORTRETRYIGNORE:
      return IDABORT;
    default:
      return IDOK;
  }
}

#define MessageBoxW TestAwareMessageBoxW

struct WorkspaceSearchBatchMessage {
  std::uint64_t generation{};
  std::vector<SearchMatch> matches;
  std::vector<SearchIssue> issues;
};

struct WorkspaceSearchCompleteMessage {
  std::uint64_t generation{};
  bool completed{};
  std::wstring error;
};

struct HolidayUpdateMessage {
  std::uint64_t generation{};
  std::filesystem::path cache_path;
  std::filesystem::path state_path;
  JapaneseHolidayOnlineResult result;
};

struct HolidayUpdateState {
  std::int64_t last_attempt_unix{};
  std::int64_t last_successful_check_unix{};
  std::size_t records{};
  int first_year{};
  int last_year{};
  std::wstring etag;
  std::wstring last_modified;
  std::wstring error;
};

std::wstring TrimHoliday(std::wstring value) {
  while (!value.empty() && iswspace(value.front())) value.erase(value.begin());
  while (!value.empty() && iswspace(value.back())) value.pop_back();
  return value;
}

std::optional<std::wstring> UnquoteHoliday(std::wstring value) {
  value = TrimHoliday(std::move(value));
  if (value.size() < 2 || value.front() != L'"' || value.back() != L'"') return std::nullopt;
  value = value.substr(1, value.size() - 2);
  std::wstring result;
  result.reserve(value.size());
  bool escaped{};
  for (wchar_t ch : value) {
    if (escaped) {
      if (ch == L'n') result.push_back(L'\n');
      else result.push_back(ch);
      escaped = false;
    } else if (ch == L'\\') escaped = true;
    else result.push_back(ch);
  }
  return escaped ? std::nullopt : std::optional<std::wstring>(std::move(result));
}

std::wstring QuoteHoliday(std::wstring_view value) {
  std::wstring result = L"\"";
  for (wchar_t ch : value) {
    if (ch == L'\\' || ch == L'"') result.push_back(L'\\');
    if (ch == L'\n') { result += L"\\n"; continue; }
    result.push_back(ch);
  }
  result.push_back(L'"');
  return result;
}

bool DecodeUtf8Holiday(const std::string& bytes, std::wstring& value) {
  if (bytes.empty()) { value.clear(); return true; }
  const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                       static_cast<int>(bytes.size()), nullptr, 0);
  if (size <= 0) return false;
  value.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                             static_cast<int>(bytes.size()), value.data(), size) == size;
}

bool EncodeUtf8Holiday(std::wstring_view text, std::string& bytes) {
  if (text.empty()) { bytes.clear(); return true; }
  const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                       static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
  if (size <= 0) return false;
  bytes.resize(static_cast<std::size_t>(size));
  return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                             static_cast<int>(text.size()), bytes.data(), size, nullptr, nullptr) == size;
}

std::int64_t HolidayNowUnix() {
  return std::chrono::duration_cast<std::chrono::seconds>(
             std::chrono::system_clock::now().time_since_epoch()).count();
}

bool ReadHolidayState(const std::filesystem::path& path, HolidayUpdateState& state) {
  state = {};
  std::ifstream input(path, std::ios::binary);
  if (!input) return true;
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring text;
  if (!DecodeUtf8Holiday(bytes, text)) return false;
  std::wistringstream lines(text);
  std::wstring line;
  while (std::getline(lines, line)) {
    const auto equals = line.find(L'=');
    if (equals == std::wstring::npos) continue;
    const auto key = TrimHoliday(line.substr(0, equals));
    const auto raw = TrimHoliday(line.substr(equals + 1));
    try {
      if (key == L"last_attempt_unix") state.last_attempt_unix = std::stoll(raw);
      else if (key == L"last_successful_check_unix") state.last_successful_check_unix = std::stoll(raw);
      else if (key == L"records") state.records = static_cast<std::size_t>(std::stoull(raw));
      else if (key == L"first_year") state.first_year = std::stoi(raw);
      else if (key == L"last_year") state.last_year = std::stoi(raw);
      else if (key == L"etag") { if (const auto value = UnquoteHoliday(raw)) state.etag = *value; }
      else if (key == L"last_modified") { if (const auto value = UnquoteHoliday(raw)) state.last_modified = *value; }
      else if (key == L"error") { if (const auto value = UnquoteHoliday(raw)) state.error = *value; }
    } catch (...) { return false; }
  }
  return true;
}

bool WriteHolidayState(const std::filesystem::path& path, const HolidayUpdateState& state,
                       std::wstring& error) {
  const std::wstring text = L"schema_version = 1\n"
      L"provider = " + QuoteHoliday(kHolidayProvider) + L"\n"
      L"last_attempt_unix = " + std::to_wstring(state.last_attempt_unix) + L"\n"
      L"last_successful_check_unix = " + std::to_wstring(state.last_successful_check_unix) + L"\n"
      L"records = " + std::to_wstring(state.records) + L"\n"
      L"first_year = " + std::to_wstring(state.first_year) + L"\n"
      L"last_year = " + std::to_wstring(state.last_year) + L"\n"
      L"etag = " + QuoteHoliday(state.etag) + L"\n"
      L"last_modified = " + QuoteHoliday(state.last_modified) + L"\n"
      L"error = " + QuoteHoliday(state.error) + L"\n";
  std::string bytes;
  if (!EncodeUtf8Holiday(text, bytes)) { error = L"祝日更新状態をUTF-8へ変換できません。"; return false; }
  std::error_code filesystem_error;
  std::filesystem::create_directories(path.parent_path(), filesystem_error);
  if (filesystem_error) { error = L"祝日更新状態フォルダーを作成できません。"; return false; }
  auto temporary = path; temporary += L".new";
  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) { error = L"祝日更新状態を保存できません。"; return false; }
  DWORD written{};
  const bool ok = WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    DeleteFileW(temporary.c_str()); error = L"祝日更新状態を安全に保存できません。"; return false;
  }
  return true;
}

bool WriteHolidayCache(const std::filesystem::path& path, std::wstring_view csv, std::wstring& error) {
  std::string bytes;
  if (!EncodeUtf8Holiday(csv, bytes)) { error = L"祝日CSVをUTF-8へ変換できません。"; return false; }
  std::error_code filesystem_error;
  std::filesystem::create_directories(path.parent_path(), filesystem_error);
  if (filesystem_error) { error = L"祝日cacheフォルダーを作成できません。"; return false; }
  auto temporary = path; temporary += L".new";
  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) { error = L"祝日CSV cacheを保存できません。"; return false; }
  DWORD written{};
  const bool ok = WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    DeleteFileW(temporary.c_str()); error = L"祝日CSV cacheを安全に保存できません。"; return false;
  }
  return true;
}

struct ProcessDialogContext {
  HANDLE cancellation{};
  std::atomic<HWND> dialog{};
  std::atomic<bool> finished{};
  bool marquee{};
};

HRESULT CALLBACK ProcessDialogCallback(HWND dialog, UINT notification, WPARAM wparam,
                                       LPARAM, LONG_PTR data) {
  auto& context = *reinterpret_cast<ProcessDialogContext*>(data);
  if (notification == TDN_CREATED) {
    context.dialog.store(dialog);
    if (context.marquee) {
      SendMessageW(dialog, TDM_SET_MARQUEE_PROGRESS_BAR, TRUE, 0);
      SendMessageW(dialog, TDM_SET_PROGRESS_BAR_MARQUEE, TRUE, 30);
    }
    SendMessageW(dialog, TDM_ENABLE_BUTTON, kProcessDoneButton, context.finished.load());
    if (context.finished.load()) PostMessageW(dialog, TDM_CLICK_BUTTON, kProcessDoneButton, 0);
  } else if (notification == TDN_BUTTON_CLICKED) {
    if (static_cast<int>(wparam) == IDCANCEL && !context.finished.load()) {
      SetEvent(context.cancellation);
      SendMessageW(dialog, TDM_SET_ELEMENT_TEXT, TDE_CONTENT,
                   reinterpret_cast<LPARAM>(L"処理を中止しています…"));
      SendMessageW(dialog, TDM_ENABLE_BUTTON, IDCANCEL, FALSE);
      return S_FALSE;
    }
    if (static_cast<int>(wparam) == kProcessDoneButton && !context.finished.load()) return S_FALSE;
  } else if (notification == TDN_DESTROYED) {
    context.dialog.store(nullptr);
  }
  return S_OK;
}

bool RunProcessWithCancel(HWND owner, const std::filesystem::path& executable,
                          const std::vector<std::wstring>& arguments,
                          const std::filesystem::path& working_directory,
                          std::wstring_view title, ProcessResult& result, std::wstring& error) {
  HANDLE cancellation = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!cancellation) { error = L"キャンセルeventを作成できません。"; return false; }
  ProcessDialogContext context{cancellation};
  bool started{};
  std::thread worker([&] {
    started = RunProcess(executable, arguments, working_directory, 64 * 1024, 120000,
                         result, error, cancellation);
    context.finished.store(true);
    if (const HWND dialog = context.dialog.load()) {
      PostMessageW(dialog, TDM_ENABLE_BUTTON, kProcessDoneButton, TRUE);
      PostMessageW(dialog, TDM_CLICK_BUTTON, kProcessDoneButton, 0);
    }
  });
  const TASKDIALOG_BUTTON buttons[]{{kProcessDoneButton, L"完了"}};
  TASKDIALOGCONFIG config{sizeof(config)};
  config.hwndParent = owner;
  config.dwFlags = TDF_POSITION_RELATIVE_TO_WINDOW | TDF_ALLOW_DIALOG_CANCELLATION;
  config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
  config.pszWindowTitle = title.data();
  config.pszMainInstruction = L"外部処理を実行しています";
  config.pszContent = L"MDLite は応答を保ったまま待機しています。必要ならキャンセルできます。";
  config.cButtons = static_cast<UINT>(std::size(buttons));
  config.pButtons = buttons;
  config.pfCallback = ProcessDialogCallback;
  config.lpCallbackData = reinterpret_cast<LONG_PTR>(&context);
  int button{};
  const HRESULT dialog_result = TaskDialogIndirect(&config, &button, nullptr, nullptr);
  if (!context.finished.load()) SetEvent(cancellation);
  worker.join();
  CloseHandle(cancellation);
  if (FAILED(dialog_result)) {
    error = L"進捗dialogを表示できません。";
    return false;
  }
  return started;
}

bool RunCancellableTask(HWND owner, std::wstring_view title, std::wstring_view instruction,
                        const std::function<bool(HANDLE)>& task, std::wstring& error) {
  HANDLE cancellation = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!cancellation) { error = L"キャンセルeventを作成できません。"; return false; }
  ProcessDialogContext context{cancellation};
  context.marquee = true;
  bool completed{};
  std::thread worker([&] {
    completed = task(cancellation);
    context.finished.store(true);
    if (const HWND dialog = context.dialog.load()) {
      PostMessageW(dialog, TDM_ENABLE_BUTTON, kProcessDoneButton, TRUE);
      PostMessageW(dialog, TDM_CLICK_BUTTON, kProcessDoneButton, 0);
    }
  });
  const TASKDIALOG_BUTTON buttons[]{{kProcessDoneButton, L"完了"}};
  TASKDIALOGCONFIG config{sizeof(config)};
  config.hwndParent = owner;
  config.dwFlags = TDF_POSITION_RELATIVE_TO_WINDOW | TDF_ALLOW_DIALOG_CANCELLATION |
                   TDF_SHOW_MARQUEE_PROGRESS_BAR;
  config.dwCommonButtons = TDCBF_CANCEL_BUTTON;
  config.pszWindowTitle = title.data();
  config.pszMainInstruction = instruction.data();
  config.pszContent = L"MDLite は応答を保ったまま処理します。必要ならキャンセルできます。";
  config.cButtons = static_cast<UINT>(std::size(buttons));
  config.pButtons = buttons;
  config.pfCallback = ProcessDialogCallback;
  config.lpCallbackData = reinterpret_cast<LONG_PTR>(&context);
  int button{};
  const HRESULT dialog_result = TaskDialogIndirect(&config, &button, nullptr, nullptr);
  if (!context.finished.load()) SetEvent(cancellation);
  worker.join();
  CloseHandle(cancellation);
  if (FAILED(dialog_result)) { error = L"進捗dialogを表示できません。"; return false; }
  return completed;
}

enum ControlId : int {
  kWorkspaceTree = 100,
  kTabs,
  kEditor,
  kOutline,
  kFindBar,
  kFindEdit,
  kFindNext,
  kFindReplaceEdit,
  kFindWorkspace,
  kReplaceWorkspace,
  kFindCase,
  kFindRegex,
  kFindWord,
  kFindIncludeGlob,
  kFindExcludeGlob,
  kReplaceOne,
  kReplaceDocument,
  kFindResults,
  kFileOpenWorkspace = 1000,
  kFileNew,
  kFileOpen,
  kFileQuickOpen,
  kFileClose,
  kFileSave,
  kFileSaveAs,
  kFileReload,
  kFileCompare,
  kFileDaily,
  kFileMeeting,
  kFileMemo,
  kFileProfile,
  kWorkspaceNewFile,
  kWorkspaceNewFolder,
  kWorkspaceCopy,
  kWorkspacePaste,
  kWorkspaceRenameMove,
  kWorkspaceDelete,
  kWorkspaceCopyPath,
  kWorkspaceShowExplorer,
  kFileExit,
  kEditFind,
  kEditFindNext,
  kEditFindWorkspace,
  kEditReplaceWorkspace,
  kViewCalendar,
  kViewSettings,
  kViewSettingsFiles,
  kViewProfiles,
  kViewCompact,
  kViewCommandPalette,
  kWorkspaceTrust,
  kWorkspaceUntrust,
  kGitStatus,
  kGitDiff,
  kGitStageAll,
  kGitUnstageAll,
  kGitCommit,
  kGitBranchCreate,
  kGitBranchSwitch,
  kGitMerge,
  kGitMergeAbort,
  kGitFetch,
  kGitPull,
  kGitPush,
  kGitConflicts,
  kGitConflictPrevious,
  kGitConflictNext,
  kGitConflictCurrent,
  kGitConflictIncoming,
  kGitConflictBoth,
  kGitConflictResolved,
  kImageUpload,
  kImageWidth320,
  kImageWidth480,
  kImageWidth640,
  kTableRowBefore,
  kTableRowAfter,
  kTableRowDelete,
  kTableColumnBefore,
  kTableColumnAfter,
  kTableColumnDelete,
  // Keep newly added calendar commands after the long-standing command IDs;
  // performance/GUI harnesses send the numeric IDs directly.
  kCalendarImportHolidays,
  kCalendarUpdateHolidays,
  // Keep pane toggles after the long-standing command IDs; automation sends
  // existing numeric IDs directly.
  kViewWorkspacePane,
  kViewOutlinePane,
  kViewResetPanels,
  kViewMoveFocusedLeftTop,
  kViewMoveFocusedLeftBottom,
  kViewMoveFocusedRightTop,
  kViewMoveFocusedRightBottom,
  kPanelHeaderExplorer,
  kPanelHeaderCalendar,
  kPanelHeaderOutline,
  kPanelHeaderGit,
  kViewMoveExplorerLeftTop,
  kViewMoveExplorerLeftBottom,
  kViewMoveExplorerRightTop,
  kViewMoveExplorerRightBottom,
  kViewMoveCalendarLeftTop,
  kViewMoveCalendarLeftBottom,
  kViewMoveCalendarRightTop,
  kViewMoveCalendarRightBottom,
  kViewMoveOutlineLeftTop,
  kViewMoveOutlineLeftBottom,
  kViewMoveOutlineRightTop,
  kViewMoveOutlineRightBottom,
  kViewMoveGitLeftTop,
  kViewMoveGitLeftBottom,
  kViewMoveGitRightTop,
  kViewMoveGitRightBottom,
  kViewResizeFocusedNarrow,
  kViewResizeFocusedWide,
  kCalendarOpenSelected,
};

int PanelIndex(PanelId id) {
  return static_cast<int>(id);
}

const wchar_t* PanelName(PanelId id) {
  switch (id) {
    case PanelId::Explorer: return L"Explorer";
    case PanelId::Calendar: return L"Calendar";
    case PanelId::Outline: return L"Outline";
    case PanelId::Git: return L"Git";
  }
  return L"Panel";
}

const wchar_t* PanelSlotName(PanelSlot slot) {
  switch (slot) {
    case PanelSlot::LeftTop: return L"左上";
    case PanelSlot::LeftBottom: return L"左下";
    case PanelSlot::RightTop: return L"右上";
    case PanelSlot::RightBottom: return L"右下";
  }
  return L"?";
}
bool IsTextFile(const std::filesystem::path& path) {
  std::wstring extension = path.extension().wstring();
  std::transform(extension.begin(), extension.end(), extension.begin(), towlower);
  static constexpr std::array extensions{L".md", L".markdown", L".txt", L".log", L".json",
                                          L".toml", L".yaml", L".yml"};
  return std::ranges::find(extensions, extension) != extensions.end();
}

bool IsMarkdownFile(const std::filesystem::path& path) {
  std::wstring extension = path.extension().wstring();
  std::transform(extension.begin(), extension.end(), extension.begin(), towlower);
  return extension == L".md" || extension == L".markdown";
}

bool IsPathWithin(const std::filesystem::path& root, const std::filesystem::path& candidate) {
  const auto normalized_root = std::filesystem::absolute(root).lexically_normal();
  const auto normalized_candidate = std::filesystem::absolute(candidate).lexically_normal();
  auto root_part = normalized_root.begin();
  auto candidate_part = normalized_candidate.begin();
  for (; root_part != normalized_root.end(); ++root_part, ++candidate_part) {
    if (candidate_part == normalized_candidate.end() ||
        _wcsicmp(root_part->c_str(), candidate_part->c_str()) != 0) return false;
  }
  return true;
}

bool LaunchMDLite(const std::filesystem::path& target, std::wstring& error) {
  std::wstring executable(32768, L'\0');
  const DWORD length = GetModuleFileNameW(nullptr, executable.data(), static_cast<DWORD>(executable.size()));
  if (length == 0 || length >= executable.size()) {
    error = L"MDLiteの実行pathを取得できません。";
    return false;
  }
  executable.resize(length);
  std::wstring command = L"\"" + executable + L"\" \"" + target.wstring() + L"\"";
  STARTUPINFOW startup{sizeof(startup)};
  PROCESS_INFORMATION process{};
  if (!CreateProcessW(executable.c_str(), command.data(), nullptr, nullptr, FALSE, 0,
                      nullptr, nullptr, &startup, &process)) {
    error = L"別のMDLiteウィンドウを起動できません。Windows error " +
            std::to_wstring(GetLastError());
    return false;
  }
  CloseHandle(process.hThread);
  CloseHandle(process.hProcess);
  return true;
}

EditorSnapshot SnapshotFor(const Document& document) {
  return IsMarkdownFile(document.path()) ? BuildMarkdownEditorSnapshot(document.text())
                                         : BuildEditorSnapshot(document.text());
}

EditorSnapshot NativeSnapshotFor(const Document& document) {
  return IsMarkdownFile(document.path()) ? BuildNativeEditorSnapshot(document.text())
                                         : BuildNativeTextEditorSnapshot(document.text());
}

EditorSnapshot NativeSnapshotFor(const std::filesystem::path& path, std::wstring_view text) {
  return IsMarkdownFile(path) ? BuildNativeEditorSnapshot(text)
                              : BuildNativeTextEditorSnapshot(text);
}

std::uint64_t FileTimeValue(const FILETIME& value) {
  return (static_cast<std::uint64_t>(value.dwHighDateTime) << 32U) | value.dwLowDateTime;
}

std::wstring WorkspaceMutexName(const std::filesystem::path& path) {
  std::uint64_t hash = 14695981039346656037ULL;
  const auto normalized = std::filesystem::absolute(path).lexically_normal().wstring();
  for (const wchar_t character : normalized) {
    hash ^= static_cast<std::uint16_t>(towlower(character));
    hash *= 1099511628211ULL;
  }
  wchar_t name[64]{};
  swprintf_s(name, L"Local\\MDLite.Workspace.%016llx", static_cast<unsigned long long>(hash));
  return name;
}

class PresentationUndoGuard {
 public:
  explicit PresentationUndoGuard(HWND editor) : editor_(editor) {}
  ~PresentationUndoGuard() {
    // Source transactions are the only user-visible Undo history.  RichEdit can
    // record presentation-only formatting and embedded-object changes, so clear
    // those native records while EN_CHANGE suppression is still active.  Avoid
    // TOM's tomSuspend/tomResume here: repeated theme and image refresh cycles
    // can re-enter RichEdit's internal Undo manager and corrupt its state.
    if (IsWindow(editor_)) SendMessageW(editor_, EM_EMPTYUNDOBUFFER, 0, 0);
  }
  PresentationUndoGuard(const PresentationUndoGuard&) = delete;
  PresentationUndoGuard& operator=(const PresentationUndoGuard&) = delete;
  explicit operator bool() const noexcept { return IsWindow(editor_); }

 private:
  HWND editor_{};
};

class ScopedEditorChangeSuppression {
 public:
  explicit ScopedEditorChangeSuppression(bool& state) : state_(state), previous_(state) {
    state_ = true;
  }
  ~ScopedEditorChangeSuppression() { state_ = previous_; }
  ScopedEditorChangeSuppression(const ScopedEditorChangeSuppression&) = delete;
  ScopedEditorChangeSuppression& operator=(const ScopedEditorChangeSuppression&) = delete;

 private:
  bool& state_;
  bool previous_{};
};

struct PromptContext {
  std::wstring label;
  std::wstring value;
  int height{155};
  HWND edit{};
  bool accepted{};
  bool completed{};
};

LRESULT CALLBACK PromptWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  auto* context = reinterpret_cast<PromptContext*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    context = static_cast<PromptContext*>(create->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(context));
  }
  if (!context) return DefWindowProcW(window, message, wparam, lparam);
  if (message == WM_CREATE) {
    CreateWindowExW(0, L"STATIC", context->label.c_str(), WS_CHILD | WS_VISIBLE,
                    12, 12, 436, context->height - 119, window, nullptr, nullptr, nullptr);
    context->edit = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", context->value.c_str(),
                                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
                                    12, context->height - 103, 436, 25, window, reinterpret_cast<HMENU>(100), nullptr, nullptr);
    CreateWindowExW(0, L"BUTTON", L"OK", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                    280, context->height - 65, 80, 27, window, reinterpret_cast<HMENU>(IDOK), nullptr, nullptr);
    CreateWindowExW(0, L"BUTTON", L"キャンセル", WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    368, context->height - 65, 80, 27, window, reinterpret_cast<HMENU>(IDCANCEL), nullptr, nullptr);
    SetFocus(context->edit);
    SendMessageW(context->edit, EM_SETSEL, 0, -1);
    return 0;
  }
  if (message == WM_COMMAND && (LOWORD(wparam) == IDOK || LOWORD(wparam) == IDCANCEL)) {
    if (LOWORD(wparam) == IDOK) {
      const int length = GetWindowTextLengthW(context->edit);
      context->value.resize(static_cast<std::size_t>(length) + 1);
      GetWindowTextW(context->edit, context->value.data(), length + 1);
      context->value.resize(static_cast<std::size_t>(length));
      context->accepted = true;
    }
    context->completed = true;
    DestroyWindow(window);
    return 0;
  }
  if (message == WM_CLOSE) {
    context->completed = true;
    DestroyWindow(window);
    return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

bool PromptText(HWND owner, HINSTANCE instance, std::wstring_view title, std::wstring_view label,
                std::wstring& value) {
  constexpr wchar_t prompt_class[] = L"MDLite.PromptWindow";
  WNDCLASSEXW existing{sizeof(existing)};
  if (!GetClassInfoExW(instance, prompt_class, &existing)) {
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.lpfnWndProc = PromptWindowProc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    window_class.lpszClassName = prompt_class;
    if (!RegisterClassExW(&window_class)) return false;
  }
  const auto lines = static_cast<int>(std::count(label.begin(), label.end(), L'\n')) + 1;
  const int height = std::clamp(130 + lines * 18, 155, 520);
  PromptContext context{std::wstring(label), value, height};
  RECT owner_rect{};
  GetWindowRect(owner, &owner_rect);
  const int x = owner_rect.left + ((owner_rect.right - owner_rect.left) - 480) / 2;
  const int y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;
  HWND dialog = CreateWindowExW(WS_EX_DLGMODALFRAME, prompt_class, std::wstring(title).c_str(),
                                 WS_CAPTION | WS_SYSMENU | WS_POPUP | WS_VISIBLE,
                                 x, y, 480, height, owner, nullptr, instance, &context);
  if (!dialog) return false;
  EnableWindow(owner, FALSE);
  MSG message{};
  int message_result = 1;
  while (!context.completed && (message_result = GetMessageW(&message, nullptr, 0, 0)) > 0) {
    if (!IsDialogMessageW(dialog, &message)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
  }
  if (message_result == 0) PostQuitMessage(static_cast<int>(message.wParam));
  EnableWindow(owner, TRUE);
  SetForegroundWindow(owner);
  if (context.accepted) value = std::move(context.value);
  return context.accepted;
}

std::wstring ControlText(HWND control);

// Quick Open and the command palette use the same native picker surface.  A
// filter edit and a real list box keep the candidate set visible while the
// user narrows it, instead of hiding a second prompt behind the first one.
struct NativePickerItem {
  std::wstring label;
};

struct NativePickerContext {
  std::vector<NativePickerItem>* items{};
  std::wstring label;
  HWND filter{};
  HWND list{};
  int width{720};
  int height{520};
  std::size_t selected{std::numeric_limits<std::size_t>::max()};
  bool accepted{};
  bool completed{};
};

constexpr int kNativePickerFilterId = 100;
constexpr int kNativePickerListId = 101;

std::wstring Lowercase(std::wstring value) {
  std::ranges::transform(value, value.begin(), towlower);
  return value;
}

void RefreshNativePickerList(NativePickerContext& context) {
  if (!context.filter || !context.list || !context.items) return;
  const auto query = Lowercase(ControlText(context.filter));
  SendMessageW(context.list, LB_RESETCONTENT, 0, 0);
  for (std::size_t index = 0; index < context.items->size(); ++index) {
    const auto& item = (*context.items)[index];
    if (!query.empty() && Lowercase(item.label).find(query) == std::wstring::npos) continue;
    const auto row = static_cast<int>(SendMessageW(
        context.list, LB_ADDSTRING, 0, reinterpret_cast<LPARAM>(item.label.c_str())));
    if (row == LB_ERR || row == LB_ERRSPACE) continue;
    SendMessageW(context.list, LB_SETITEMDATA, static_cast<WPARAM>(row),
                 static_cast<LPARAM>(index));
  }
  if (SendMessageW(context.list, LB_GETCOUNT, 0, 0) > 0)
    SendMessageW(context.list, LB_SETCURSEL, 0, 0);
}

bool AcceptNativePickerSelection(NativePickerContext& context) {
  if (!context.list || !context.items) return false;
  const auto row = SendMessageW(context.list, LB_GETCURSEL, 0, 0);
  if (row == LB_ERR) return false;
  const auto item = SendMessageW(context.list, LB_GETITEMDATA, static_cast<WPARAM>(row), 0);
  if (item == LB_ERR || static_cast<std::size_t>(item) >= context.items->size()) return false;
  context.selected = static_cast<std::size_t>(item);
  context.accepted = true;
  context.completed = true;
  return true;
}

LRESULT CALLBACK NativePickerWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  auto* context = reinterpret_cast<NativePickerContext*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    context = static_cast<NativePickerContext*>(create->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(context));
  }
  if (!context || !context->items) return DefWindowProcW(window, message, wparam, lparam);
  if (message == WM_CREATE) {
    const int margin = ScaleDip(window, 16);
    const int label_height = ScaleDip(window, 38);
    const int control_height = ScaleDip(window, 28);
    const int content_width = context->width - margin * 2;
    const HFONT font = reinterpret_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
    HWND label = CreateWindowExW(0, L"STATIC", context->label.c_str(), WS_CHILD | WS_VISIBLE,
                                 margin, margin, content_width, label_height, window, nullptr,
                                 nullptr, nullptr);
    if (label) SendMessageW(label, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    context->filter = CreateWindowExW(
        WS_EX_CLIENTEDGE, L"EDIT", nullptr, WS_CHILD | WS_VISIBLE | WS_TABSTOP | ES_AUTOHSCROLL,
        margin, margin + label_height, content_width, control_height, window,
        reinterpret_cast<HMENU>(static_cast<INT_PTR>(kNativePickerFilterId)), nullptr, nullptr);
    if (context->filter) SendMessageW(context->filter, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    context->list = CreateWindowExW(
        WS_EX_CLIENTEDGE, L"LISTBOX", nullptr,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | LBS_NOTIFY | LBS_NOINTEGRALHEIGHT | WS_VSCROLL,
        margin, margin + label_height + control_height + ScaleDip(window, 10), content_width,
        context->height - margin * 2 - label_height - control_height - ScaleDip(window, 58), window,
        reinterpret_cast<HMENU>(static_cast<INT_PTR>(kNativePickerListId)), nullptr, nullptr);
    if (context->list) SendMessageW(context->list, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    const int button_y = context->height - margin - ScaleDip(window, 30);
    HWND apply = CreateWindowExW(0, L"BUTTON", L"開く", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                                 context->width - margin - ScaleDip(window, 190), button_y,
                                 ScaleDip(window, 84), ScaleDip(window, 30), window,
                                 reinterpret_cast<HMENU>(IDOK), nullptr, nullptr);
    HWND cancel = CreateWindowExW(0, L"BUTTON", L"取消", WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                  context->width - margin - ScaleDip(window, 94), button_y,
                                  ScaleDip(window, 84), ScaleDip(window, 30), window,
                                  reinterpret_cast<HMENU>(IDCANCEL), nullptr, nullptr);
    if (apply) SendMessageW(apply, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    if (cancel) SendMessageW(cancel, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    RefreshNativePickerList(*context);
    if (context->filter) SetFocus(context->filter);
    return 0;
  }
  if (message == WM_COMMAND) {
    const int id = LOWORD(wparam);
    const int code = HIWORD(wparam);
    if (id == kNativePickerFilterId && code == EN_CHANGE) {
      RefreshNativePickerList(*context);
      return 0;
    }
    if (id == kNativePickerListId && code == LBN_DBLCLK && AcceptNativePickerSelection(*context)) {
      DestroyWindow(window);
      return 0;
    }
    if (id == IDOK) {
      if (!AcceptNativePickerSelection(*context)) return 0;
      DestroyWindow(window);
      return 0;
    }
    if (id == IDCANCEL) {
      context->completed = true;
      DestroyWindow(window);
      return 0;
    }
  }
  if (message == WM_CLOSE) {
    context->completed = true;
    DestroyWindow(window);
    return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

bool RunNativePicker(HWND owner, HINSTANCE instance, std::wstring_view title,
                     std::wstring_view label, std::vector<NativePickerItem>& items,
                     std::size_t& selected) {
  if (items.empty()) return false;
  constexpr wchar_t picker_class[] = L"MDLite.NativePickerWindow";
  WNDCLASSEXW existing{sizeof(existing)};
  if (!GetClassInfoExW(instance, picker_class, &existing)) {
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.lpfnWndProc = NativePickerWindowProc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    window_class.lpszClassName = picker_class;
    if (!RegisterClassExW(&window_class)) return false;
  }
  const int width = ScaleDip(owner, 720);
  const int height = ScaleDip(owner, 520);
  NativePickerContext context;
  context.items = &items;
  context.label = std::wstring(label);
  context.width = width;
  context.height = height;
  RECT owner_rect{};
  GetWindowRect(owner, &owner_rect);
  const int x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
  const int y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;
  HWND dialog = CreateWindowExW(WS_EX_DLGMODALFRAME, picker_class, std::wstring(title).c_str(),
                                WS_CAPTION | WS_SYSMENU | WS_POPUP | WS_VISIBLE,
                                x, y, width, height, owner, nullptr, instance, &context);
  if (!dialog) return false;
  EnableWindow(owner, FALSE);
  MSG message{};
  while (!context.completed) {
    const BOOL result = GetMessageW(&message, nullptr, 0, 0);
    if (result <= 0) break;
    if (!IsDialogMessageW(dialog, &message)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
  }
  EnableWindow(owner, TRUE);
  SetForegroundWindow(owner);
  if (!context.accepted) return false;
  selected = context.selected;
  return selected < items.size();
}

// Settings and profile editing use one native form instead of a chain of
// prompt/message-box interactions.  The form deliberately stays small and
// data-oriented: each field owns its control and the caller performs the
// domain validation only after Apply.  This keeps Cancel side-effect free and
// lets the same keyboard/focus behavior serve both settings and profiles.
enum class NativeFormFieldKind { Text, Combo, Multiline };

struct NativeFormField {
  std::wstring label;
  std::wstring value;
  NativeFormFieldKind kind{NativeFormFieldKind::Text};
  std::vector<std::wstring> options;
  HWND control{};
};

struct NativeFormContext {
  std::vector<NativeFormField>* fields{};
  int width{680};
  int height{220};
  bool accepted{};
  bool completed{};
};

LRESULT CALLBACK NativeFormWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  auto* context = reinterpret_cast<NativeFormContext*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    context = static_cast<NativeFormContext*>(create->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(context));
  }
  if (!context || !context->fields) return DefWindowProcW(window, message, wparam, lparam);
  if (message == WM_CREATE) {
    const int margin = ScaleDip(window, 16);
    const int label_height = ScaleDip(window, 19);
    const int control_height = ScaleDip(window, 27);
    const int field_width = context->width - margin * 2;
    int y = margin;
    const HFONT font = reinterpret_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
    for (std::size_t index = 0; index < context->fields->size(); ++index) {
      auto& field = (*context->fields)[index];
      const int label_id = 1000 + static_cast<int>(index) * 2;
      const int control_id = label_id + 1;
      const int field_height = field.kind == NativeFormFieldKind::Multiline ? ScaleDip(window, 78) : control_height;
      const int label_y = y;
      HWND label = CreateWindowExW(0, L"STATIC", field.label.c_str(), WS_CHILD | WS_VISIBLE,
                                   margin, label_y, field_width, label_height, window,
                                   reinterpret_cast<HMENU>(static_cast<INT_PTR>(label_id)), nullptr, nullptr);
      if (label) SendMessageW(label, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
      DWORD style = WS_CHILD | WS_VISIBLE | WS_TABSTOP;
      if (field.kind == NativeFormFieldKind::Multiline)
        style |= ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | WS_VSCROLL;
      else if (field.kind == NativeFormFieldKind::Combo)
        style |= CBS_DROPDOWNLIST | WS_VSCROLL;
      if (field.kind == NativeFormFieldKind::Combo) {
        field.control = CreateWindowExW(WS_EX_CLIENTEDGE, WC_COMBOBOXW, nullptr, style,
                                        margin, label_y + label_height, field_width, ScaleDip(window, 170),
                                        window, reinterpret_cast<HMENU>(static_cast<INT_PTR>(control_id)), nullptr, nullptr);
        if (field.control) {
          for (const auto& option : field.options)
            SendMessageW(field.control, CB_ADDSTRING, 0, reinterpret_cast<LPARAM>(option.c_str()));
          const LRESULT found = SendMessageW(field.control, CB_FINDSTRINGEXACT, static_cast<WPARAM>(-1),
                                              reinterpret_cast<LPARAM>(field.value.c_str()));
          SendMessageW(field.control, CB_SETCURSEL, found >= 0 ? static_cast<WPARAM>(found) : 0, 0);
        }
      } else {
        field.control = CreateWindowExW(WS_EX_CLIENTEDGE, L"EDIT", field.value.c_str(),
                                        style | ES_AUTOHSCROLL, margin, label_y + label_height,
                                        field_width, field_height, window,
                                        reinterpret_cast<HMENU>(static_cast<INT_PTR>(control_id)), nullptr, nullptr);
      }
      if (field.control) SendMessageW(field.control, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
      y += label_height + field_height + ScaleDip(window, 11);
    }
    const int button_y = context->height - margin - ScaleDip(window, 30);
    HWND apply = CreateWindowExW(0, L"BUTTON", L"適用", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON,
                                 context->width - margin - ScaleDip(window, 190), button_y,
                                 ScaleDip(window, 84), ScaleDip(window, 30), window,
                                 reinterpret_cast<HMENU>(IDOK), nullptr, nullptr);
    HWND cancel = CreateWindowExW(0, L"BUTTON", L"取消", WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                                  context->width - margin - ScaleDip(window, 94), button_y,
                                  ScaleDip(window, 84), ScaleDip(window, 30), window,
                                  reinterpret_cast<HMENU>(IDCANCEL), nullptr, nullptr);
    if (apply) SendMessageW(apply, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    if (cancel) SendMessageW(cancel, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
    for (auto& field : *context->fields)
      if (field.control) {
        SetFocus(field.control);
        if (field.kind == NativeFormFieldKind::Text || field.kind == NativeFormFieldKind::Multiline)
          SendMessageW(field.control, EM_SETSEL, 0, -1);
        break;
      }
    return 0;
  }
  if (message == WM_COMMAND && (LOWORD(wparam) == IDOK || LOWORD(wparam) == IDCANCEL)) {
    if (LOWORD(wparam) == IDOK) {
      for (auto& field : *context->fields) {
        if (!field.control) continue;
        field.value = ControlText(field.control);
      }
      context->accepted = true;
    }
    context->completed = true;
    DestroyWindow(window);
    return 0;
  }
  if (message == WM_CLOSE) {
    context->completed = true;
    DestroyWindow(window);
    return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

bool RunNativeForm(HWND owner, HINSTANCE instance, std::wstring_view title,
                   std::vector<NativeFormField>& fields) {
  constexpr wchar_t form_class[] = L"MDLite.NativeFormWindow";
  WNDCLASSEXW existing{sizeof(existing)};
  if (!GetClassInfoExW(instance, form_class, &existing)) {
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.lpfnWndProc = NativeFormWindowProc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
    window_class.lpszClassName = form_class;
    if (!RegisterClassExW(&window_class)) return false;
  }
  const int width = ScaleDip(owner, 680);
  int height = ScaleDip(owner, 132);
  for (const auto& field : fields)
    height += ScaleDip(owner, field.kind == NativeFormFieldKind::Multiline ? 108 : 57);
  height = std::clamp(height, ScaleDip(owner, 220), ScaleDip(owner, 760));
  NativeFormContext context{&fields, width, height};
  RECT owner_rect{};
  GetWindowRect(owner, &owner_rect);
  const int x = owner_rect.left + ((owner_rect.right - owner_rect.left) - width) / 2;
  const int y = owner_rect.top + ((owner_rect.bottom - owner_rect.top) - height) / 2;
  HWND dialog = CreateWindowExW(WS_EX_DLGMODALFRAME, form_class, std::wstring(title).c_str(),
                                WS_CAPTION | WS_SYSMENU | WS_POPUP | WS_VISIBLE,
                                x, y, width, height, owner, nullptr, instance, &context);
  if (!dialog) return false;
  EnableWindow(owner, FALSE);
  MSG message{};
  while (!context.completed) {
    const BOOL result = GetMessageW(&message, nullptr, 0, 0);
    if (result <= 0) break;
    if (!IsDialogMessageW(dialog, &message)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
  }
  EnableWindow(owner, TRUE);
  SetForegroundWindow(owner);
  return context.accepted;
}

std::optional<ACCEL> ParseAccelerator(std::wstring value, WORD command) {
  value.erase(std::remove_if(value.begin(), value.end(), iswspace), value.end());
  std::ranges::transform(value, value.begin(), towupper);
  if (value.empty() || value == L"NONE") return std::nullopt;
  ACCEL accelerator{FVIRTKEY, 0, command};
  std::size_t begin{};
  std::wstring key;
  while (begin <= value.size()) {
    const auto end = value.find(L'+', begin);
    const auto token = value.substr(begin, end == std::wstring::npos ? value.size() - begin : end - begin);
    if (token == L"CTRL") accelerator.fVirt |= FCONTROL;
    else if (token == L"SHIFT") accelerator.fVirt |= FSHIFT;
    else if (token == L"ALT") accelerator.fVirt |= FALT;
    else key = token;
    if (end == std::wstring::npos) break;
    begin = end + 1;
  }
  if (key.size() == 1 && ((key[0] >= L'A' && key[0] <= L'Z') || (key[0] >= L'0' && key[0] <= L'9')))
    accelerator.key = static_cast<WORD>(key[0]);
  else if (key.size() >= 2 && key.front() == L'F') {
    try {
      const int number = std::stoi(key.substr(1));
      if (number < 1 || number > 24) return std::nullopt;
      accelerator.key = static_cast<WORD>(VK_F1 + number - 1);
    } catch (const std::exception&) { return std::nullopt; }
  } else return std::nullopt;
  return accelerator;
}

bool SystemUsesDarkTheme() {
  DWORD light{};
  DWORD size = sizeof(light);
  const auto status = RegGetValueW(HKEY_CURRENT_USER,
      L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize",
      L"AppsUseLightTheme", RRF_RT_REG_DWORD, nullptr, &light, &size);
  return status == ERROR_SUCCESS && light == 0;
}

COLORREF ThemeColor(const EffectiveSettings& settings, std::wstring_view name, COLORREF fallback) {
  if (settings.theme != ThemeMode::Custom) return fallback;
  const auto found = settings.colors.find(std::wstring(name));
  if (found == settings.colors.end() || found->second.size() != 7 || found->second.front() != L'#') return fallback;
  wchar_t* end{};
  const unsigned long rgb = wcstoul(found->second.c_str() + 1, &end, 16);
  if (!end || *end != L'\0' || rgb > 0xFFFFFFUL) return fallback;
  return RGB((rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF);
}

std::wstring ControlText(HWND control) {
  const int length = GetWindowTextLengthW(control);
  if (length <= 0) return {};
  std::wstring value(static_cast<std::size_t>(length) + 1, L'\0');
  GetWindowTextW(control, value.data(), length + 1);
  value.resize(static_cast<std::size_t>(length));
  return value;
}

std::vector<std::wstring> SplitGlobList(std::wstring_view value) {
  std::vector<std::wstring> globs;
  std::size_t begin{};
  while (begin <= value.size()) {
    const auto end = value.find_first_of(L";\r\n", begin);
    auto token = value.substr(begin, end == std::wstring_view::npos ? value.size() - begin
                                                                   : end - begin);
    while (!token.empty() && iswspace(token.front())) token.remove_prefix(1);
    while (!token.empty() && iswspace(token.back())) token.remove_suffix(1);
    if (!token.empty()) {
      std::wstring glob(token);
      std::ranges::replace(glob, L'\\', L'/');
      if (std::ranges::find(globs, glob) == globs.end()) globs.push_back(std::move(glob));
    }
    if (end == std::wstring_view::npos) break;
    begin = end + 1;
  }
  return globs;
}

std::filesystem::path ResolveGitExecutable() {
  wchar_t environment[32768]{};
  const DWORD length = GetEnvironmentVariableW(L"PATH", environment, static_cast<DWORD>(std::size(environment)));
  if (length == 0 || length >= std::size(environment)) return {};
  wchar_t git_path[32768]{};
  if (SearchPathW(environment, L"git.exe", nullptr, static_cast<DWORD>(std::size(git_path)),
                  git_path, nullptr) == 0) return {};
  return git_path;
}

std::wstring MarkdownAnchor(std::wstring_view text) {
  std::wstring anchor;
  bool hyphen{};
  for (wchar_t character : text) {
    if (iswalnum(character) || character >= 0x80) {
      if (hyphen && !anchor.empty()) anchor.push_back(L'-');
      anchor.push_back(static_cast<wchar_t>(towlower(character)));
      hyphen = false;
    } else if (iswspace(character) || character == L'-') {
      hyphen = true;
    }
  }
  return anchor;
}

std::wstring UrlDecode(std::wstring value) {
  DWORD length = static_cast<DWORD>(value.size() + 1);
  std::vector<wchar_t> decoded(length);
  if (SUCCEEDED(UrlUnescapeW(value.data(), decoded.data(), &length, URL_UNESCAPE_AS_UTF8)))
    return std::wstring(decoded.data(), length);
  return value;
}

std::optional<std::filesystem::path> PickFolder(HWND owner, std::wstring_view title,
                                                const std::filesystem::path& initial) {
  IFileDialog* dialog{};
  if (FAILED(CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER,
                              IID_PPV_ARGS(&dialog)))) return std::nullopt;
  DWORD options{};
  dialog->GetOptions(&options);
  dialog->SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST);
  dialog->SetTitle(title.data());
  IShellItem* initial_item{};
  if (SUCCEEDED(SHCreateItemFromParsingName(initial.c_str(), nullptr, IID_PPV_ARGS(&initial_item)))) {
    dialog->SetFolder(initial_item);
    initial_item->Release();
  }
  std::optional<std::filesystem::path> result;
  if (SUCCEEDED(dialog->Show(owner))) {
    IShellItem* item{};
    if (SUCCEEDED(dialog->GetResult(&item))) {
      PWSTR path{};
      if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &path))) {
        result = std::filesystem::path(path);
        CoTaskMemFree(path);
      }
      item->Release();
    }
  }
  dialog->Release();
  return result;
}

void PlaceOnVisibleMonitor(HWND window, int x, int y, int width, int height) {
  width = std::max(width, 320);
  height = std::max(height, 240);
  RECT requested{x, y, x + width, y + height};
  MONITORINFO monitor{sizeof(monitor)};
  GetMonitorInfoW(MonitorFromRect(&requested, MONITOR_DEFAULTTONEAREST), &monitor);
  const int work_width = static_cast<int>(monitor.rcWork.right - monitor.rcWork.left);
  const int work_height = static_cast<int>(monitor.rcWork.bottom - monitor.rcWork.top);
  width = std::min(width, work_width);
  height = std::min(height, work_height);
  x = std::clamp(x, static_cast<int>(monitor.rcWork.left), static_cast<int>(monitor.rcWork.right) - width);
  y = std::clamp(y, static_cast<int>(monitor.rcWork.top), static_cast<int>(monitor.rcWork.bottom) - height);
  SetWindowPos(window, nullptr, x, y, width, height, SWP_NOZORDER | SWP_NOACTIVATE);
}

}  // namespace

Application::Application(HINSTANCE instance) : instance_(instance) {}

Application::~Application() {
  if (menu_ && IsMenu(menu_)) DestroyMenu(menu_);
  menu_ = nullptr;
  if (workspace_search_worker_.joinable()) {
    workspace_search_worker_.request_stop();
    workspace_search_worker_.join();
  }
  if (holiday_update_worker_.joinable()) {
    holiday_update_worker_.request_stop();
    holiday_update_worker_.join();
  }
  if (accelerator_table_) DestroyAcceleratorTable(accelerator_table_);
  if (editor_font_) DeleteObject(editor_font_);
  if (background_brush_) DeleteObject(background_brush_);
  if (surface_brush_) DeleteObject(surface_brush_);
  if (input_brush_) DeleteObject(input_brush_);
  if (editor_brush_) DeleteObject(editor_brush_);
}

bool Application::Initialize(int show_command) {
  INITCOMMONCONTROLSEX controls{sizeof(controls), ICC_WIN95_CLASSES | ICC_TAB_CLASSES | ICC_BAR_CLASSES};
  InitCommonControlsEx(&controls);
  if (LoadLibraryW(L"Msftedit.dll") == nullptr) {
    MessageBoxW(nullptr, L"Windows RichEditを読み込めません。", L"MDLite", MB_ICONERROR);
    return false;
  }

  WNDCLASSEXW window_class{sizeof(window_class)};
  window_class.style = CS_HREDRAW | CS_VREDRAW;
  window_class.lpfnWndProc = WindowProc;
  window_class.hInstance = instance_;
  window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
  window_class.hIcon = LoadIconW(nullptr, IDI_APPLICATION);
  window_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
  window_class.lpszClassName = kWindowClass;
  if (RegisterClassExW(&window_class) == 0) return false;

  WNDCLASSEXW compact_class{sizeof(compact_class)};
  compact_class.style = CS_HREDRAW | CS_VREDRAW;
  compact_class.lpfnWndProc = CompactWindowProc;
  compact_class.hInstance = instance_;
  compact_class.hCursor = LoadCursorW(nullptr, IDC_IBEAM);
  compact_class.hIcon = LoadIconW(nullptr, IDI_APPLICATION);
  compact_class.hbrBackground = reinterpret_cast<HBRUSH>(COLOR_WINDOW + 1);
  compact_class.lpszClassName = kCompactWindowClass;
  if (RegisterClassExW(&compact_class) == 0) return false;

  window_ = CreateWindowExW(0, kWindowClass, L"MDLite", WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                            CW_USEDEFAULT, CW_USEDEFAULT, 1280, 800, nullptr, nullptr, instance_, this);
  if (window_ == nullptr) return false;
  LoadAndApplySettings();
  ShowWindow(window_, show_command);
  UpdateWindow(window_);
  return true;
}

int Application::Run() {
  MSG message{};
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    if (!accelerator_table_ || !TranslateAcceleratorW(window_, accelerator_table_, &message)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
  }
  return static_cast<int>(message.wParam);
}

void Application::OpenInitialPath(const std::filesystem::path& path) {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error) return;
  if (std::filesystem::is_directory(absolute, error)) {
    if (!workspace_.empty() && workspace_ != absolute) {
      std::wstring launch_error;
      if (!LaunchMDLite(absolute, launch_error)) MessageBoxW(window_, launch_error.c_str(), L"Workspaceを開けません", MB_ICONERROR);
      return;
    }
    OpenWorkspace(absolute);
  }
  else if (std::filesystem::is_regular_file(absolute, error)) {
    if (!workspace_.empty() && !IsPathWithin(workspace_, absolute)) {
      std::wstring launch_error;
      if (!LaunchMDLite(absolute, launch_error)) MessageBoxW(window_, launch_error.c_str(), L"ファイルを開けません", MB_ICONERROR);
      return;
    }
    if (workspace_.empty()) OpenWorkspace(absolute.parent_path());
    OpenDocument(absolute);
  }
}

LRESULT CALLBACK Application::WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  Application* app = reinterpret_cast<Application*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    app = static_cast<Application*>(create->lpCreateParams);
    app->window_ = window;
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(app));
  }
  return app != nullptr ? app->HandleMessage(message, wparam, lparam)
                        : DefWindowProcW(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::CompactWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  Application* app = reinterpret_cast<Application*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    app = static_cast<Application*>(create->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(app));
  }
  if (app == nullptr) return DefWindowProcW(window, message, wparam, lparam);
  auto view = std::ranges::find_if(app->documents_, [window](const auto& candidate) {
    return candidate->compact_window == window;
  });
  if (message == WM_SIZE && view != app->documents_.end()) {
    RECT client{};
    GetClientRect(window, &client);
    MoveWindow((*view)->editor, 0, 0, client.right, client.bottom, TRUE);
    return 0;
  }
  if (message == WM_SETFOCUS && view != app->documents_.end()) {
    app->SelectDocumentForEditor((*view)->editor);
    SetFocus((*view)->editor);
    return 0;
  }
  if (message == WM_COMMAND && view != app->documents_.end() &&
      reinterpret_cast<HWND>(lparam) == (*view)->editor && HIWORD(wparam) == EN_CHANGE) {
    app->OnEditorChanged((*view)->editor);
    return 0;
  }
  if (message == WM_NOTIFY && view != app->documents_.end()) {
    const auto* header = reinterpret_cast<const NMHDR*>(lparam);
    if (header && header->hwndFrom == (*view)->editor && header->code == EN_SELCHANGE &&
        !app->suppress_editor_change_ && !(*view)->native_edit_in_flight &&
        !(*view)->ime_composing && (*view)->sync_due == 0) {
      app->ApplyMarkdownPresentation(*(*view), false);
      return 0;
    }
    if (header && header->hwndFrom == (*view)->editor && header->code == EN_LINK) {
      const auto* link = reinterpret_cast<const ENLINK*>(lparam);
      const auto source_position = (*view)->editor_snapshot.NativeToSource(link->chrg.cpMin);
      if (link->msg == WM_LBUTTONUP) app->OpenLinkAtSourcePosition(*(*view), source_position, true);
      else if (link->msg == WM_MOUSEMOVE)
        app->OpenLinkAtSourcePosition(*(*view), source_position, false);
      return 0;
    }
  }
  if (message == WM_CLOSE && view != app->documents_.end()) {
    SetParent((*view)->editor, app->window_);
    (*view)->compact_window = nullptr;
    DestroyWindow(window);
    app->LayoutControls();
    if (app->active_document_ < app->documents_.size()) app->ActivateDocument(app->active_document_);
    return 0;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::EditorSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                               UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  auto* view = app->FindDocumentView(window);
  const bool source_navigation = message == WM_KILLFOCUS || message == WM_LBUTTONDOWN ||
      (message == WM_KEYDOWN &&
       (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN ||
        wparam == VK_HOME || wparam == VK_END || wparam == VK_PRIOR || wparam == VK_NEXT));
  if (view && !view->ime_composing && source_navigation &&
      IsMarkdownFile(view->document.path())) {
    // A caret/focus boundary terminates the pending native burst before the
    // next command can create an unrelated source-history entry.
    app->SyncDocumentFromEditor(*view);
  }
  if (message == WM_SETFOCUS) app->SelectDocumentForEditor(window);
  if (message == WM_PASTE && app->PasteClipboardImage()) return 0;
  if (message == WM_IME_STARTCOMPOSITION && view) view->ime_composing = true;
  const bool native_mutation = view != nullptr &&
      (message == WM_CHAR || message == WM_CUT || message == WM_CLEAR ||
       message == WM_PASTE || message == WM_IME_COMPOSITION ||
       message == WM_IME_ENDCOMPOSITION ||
       (message == WM_KEYDOWN && (wparam == VK_BACK || wparam == VK_DELETE)));
  if (native_mutation) view->native_edit_in_flight = true;
  const bool control = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
  const bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
  if (view && !view->ime_composing &&
      (message == WM_UNDO || message == EM_REDO ||
       (message == WM_KEYDOWN && control && (wparam == L'Z' || wparam == L'Y')))) {
    const bool redo = message == EM_REDO || wparam == L'Y' || (wparam == L'Z' && shift);
    app->ApplySourceHistory(*view, redo);
    return TRUE;
  }
  if (message == WM_KEYDOWN && wparam == VK_TAB && view && !view->ime_composing &&
      IsMarkdownFile(view->document.path())) {
    app->SyncDocumentFromEditor(*view);
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    const auto source_caret = view->editor_snapshot.NativeToSource(selection.cpMin);
    const auto edit = MoveToAdjacentTableCell(view->document.text(), source_caret,
                                              (GetKeyState(VK_SHIFT) & 0x8000) != 0);
    if (edit.changed) {
      app->ApplySourceTextWithUndo(*view, edit.text);
    }
    if (edit.selection != source_caret || edit.changed) {
      const auto view_caret = view->editor_snapshot.SourceToNative(edit.selection);
      SendMessageW(window, EM_SETSEL, view_caret, view_caret);
      return 0;
    }
  }
  if (message == WM_KEYDOWN && view && !view->ime_composing &&
      (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN) &&
      (GetKeyState(VK_SHIFT) & 0x8000) == 0 && (GetKeyState(VK_CONTROL) & 0x8000) == 0 &&
      (GetKeyState(VK_MENU) & 0x8000) == 0 && IsMarkdownFile(view->document.path())) {
    app->SyncDocumentFromEditor(*view);
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    if (selection.cpMin == selection.cpMax) {
      const auto direction = wparam == VK_LEFT ? TableCaretDirection::Left :
          wparam == VK_RIGHT ? TableCaretDirection::Right :
          wparam == VK_UP ? TableCaretDirection::Up : TableCaretDirection::Down;
      const auto destination = MoveTableCaretAtBoundary(
          view->document.text(), view->editor_snapshot.NativeToSource(selection.cpMin), direction);
      if (destination) {
        const auto view_caret = view->editor_snapshot.SourceToNative(*destination);
        SendMessageW(window, EM_SETSEL, view_caret, view_caret);
        return 0;
      }
      const LONG native_length = GetWindowTextLengthW(window);
      const LONG line = static_cast<LONG>(SendMessageW(
          window, EM_EXLINEFROMCHAR, 0, static_cast<LPARAM>(selection.cpMin)));
      const LONG line_count = static_cast<LONG>(SendMessageW(window, EM_GETLINECOUNT, 0, 0));
      const bool at_document_boundary =
          (direction == TableCaretDirection::Left && selection.cpMin == 0) ||
          (direction == TableCaretDirection::Right && selection.cpMax >= native_length) ||
          (direction == TableCaretDirection::Up && line <= 0) ||
          (direction == TableCaretDirection::Down && line_count > 0 && line >= line_count - 1);
      if (at_document_boundary) return 0;
    }
  }
  if (message == WM_LBUTTONDOWN && view && !view->ime_composing &&
      view->sync_due == 0 && IsMarkdownFile(view->document.path())) {
    const POINT point{static_cast<short>(LOWORD(lparam)), static_cast<short>(HIWORD(lparam))};
    if (const auto source = app->HitTestTableCell(*view, point)) {
      const auto native = view->editor_snapshot.SourceToNative(*source);
      app->SelectDocumentForEditor(window);
      SendMessageW(window, EM_SETSEL, static_cast<WPARAM>(native), static_cast<LPARAM>(native));
      SetFocus(window);
      return 0;
    }
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, EditorSubclass, 1);
  RECT update_rect{};
  const bool needs_table_paint = message == WM_PAINT && view &&
                                 GetUpdateRect(window, &update_rect, FALSE) != FALSE;
  const LRESULT result = DefSubclassProc(window, message, wparam, lparam);
  if (view && view->native_edit_in_flight) {
    const bool ime_result = message == WM_IME_COMPOSITION &&
                            (lparam & GCS_RESULTSTR) != 0;
    const bool ordinary_edit = !view->ime_composing &&
        (message == WM_CHAR || message == WM_CUT || message == WM_CLEAR ||
         message == WM_PASTE ||
         (message == WM_KEYDOWN && (wparam == VK_BACK || wparam == VK_DELETE)));
    if (message == WM_IME_ENDCOMPOSITION) view->ime_composing = false;
    if (ime_result || message == WM_IME_ENDCOMPOSITION || ordinary_edit)
      app->CommitPendingNativeEdit(*view);
    view->native_edit_in_flight = false;
  }
  if (needs_table_paint && view && !view->painting_table_grid) {
    HRGN update_region = CreateRectRgnIndirect(&update_rect);
    HDC paint_dc = update_region
        ? GetDCEx(window, update_region, DCX_INTERSECTRGN | DCX_CACHE | DCX_CLIPSIBLINGS)
        : nullptr;
    if (paint_dc) {
      view->painting_table_grid = true;
      app->DrawTableGrid(*view, paint_dc, update_rect);
      view->painting_table_grid = false;
      ReleaseDC(window, paint_dc);
    }
    if (!paint_dc && update_region) DeleteObject(update_region);
  }
  return result;
}

LRESULT CALLBACK Application::CalendarSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                                UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (message == WM_MOUSEMOVE) {
    POINT point{static_cast<short>(LOWORD(lparam)), static_cast<short>(HIWORD(lparam))};
    app->UpdateCalendarTooltip(point);
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, CalendarSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::TreeDragSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                                UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (message == WM_KEYDOWN && wparam == VK_ESCAPE &&
      (app->outline_dragging_ || app->workspace_dragging_)) {
    app->outline_dragging_ = false;
    app->workspace_dragging_ = false;
    app->workspace_drag_sources_.clear();
    TreeView_SelectDropTarget(app->outline_, nullptr);
    TreeView_SelectDropTarget(app->workspace_tree_, nullptr);
    if (GetCapture()) ReleaseCapture();
    return 0;
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, TreeDragSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT Application::HandleMessage(UINT message, WPARAM wparam, LPARAM lparam) {
  switch (message) {
    case WM_CREATE:
      CreateMenuBar();
      CreateControls();
      DragAcceptFiles(window_, TRUE);
      return 0;
    case WM_SIZE:
      LayoutControls();
      return 0;
    case WM_DPICHANGED: {
      const auto* suggested = reinterpret_cast<const RECT*>(lparam);
      if (suggested) {
        SetWindowPos(window_, nullptr, suggested->left, suggested->top,
                     suggested->right - suggested->left, suggested->bottom - suggested->top,
                     SWP_NOZORDER | SWP_NOACTIVATE);
      }
      ApplySettings();
      LayoutControls();
      return 0;
    }
    case WM_TIMER:
      if (wparam == kAutosaveTimer) {
        if (external_operation_active_) return 0;
        const ULONGLONG now = GetTickCount64();
        bool pending = false;
        bool fast_poll = false;
        ULONGLONG next_asset_check = std::numeric_limits<ULONGLONG>::max();
        if (workspace_search_due_ != 0) {
          pending = true;
          fast_poll = true;
          if (now >= workspace_search_due_) {
            workspace_search_due_ = 0;
            SearchWorkspaceFromFindBar();
          }
        }
        for (auto& view : documents_) {
          if (view->sync_due != 0) {
            pending = true;
            fast_poll = true;
            if (!view->ime_composing && now >= view->sync_due) {
              SyncDocumentFromEditor(*view);
              if (IsMarkdownFile(view->document.path()))
                SchedulePresentation(*view);
            }
          }
          if (view->presentation_due != 0) {
            pending = true;
            fast_poll = true;
            if (!view->ime_composing && now >= view->presentation_due) {
              view->presentation_due = 0;
              if (IsMarkdownFile(view->document.path())) {
                ApplyMarkdownPresentation(*view, true);
                if (active_document_ < documents_.size() &&
                    documents_[active_document_].get() == view.get()) RebuildOutline(*view);
              }
            }
          }
          if (!view->rendered_images.empty()) {
            pending = true;
            if (view->image_asset_check_due == 0 || now >= view->image_asset_check_due)
              RefreshDerivedImages(*view);
            if (view->image_asset_check_due != 0)
              next_asset_check = std::min(next_asset_check, view->image_asset_check_due);
          }
          if (!view->animated_image_frames.empty()) {
            pending = true;
            fast_poll = true;
            AdvanceAnimatedImages(*view, now);
          }
          if (!view->document.dirty()) continue;
          pending = true;
          fast_poll = true;
          if (view->ime_composing) continue;
          if (save_dialog_active_) continue;
          if (view->recovery_due != 0 && now >= view->recovery_due) {
            SaveRecovery(*view);
            view->recovery_due = now + kRecoveryDelayMs;
          }
          if (settings_.auto_save && view->autosave_due != 0 && now >= view->autosave_due) {
            const auto save_result = SaveDocument(*view, SaveIntent::BackgroundAutosave);
            if (save_result == SaveResult::Saved || save_result == SaveResult::NoChange) {
              view->autosave_due = 0;
              view->recovery_due = 0;
            } else if (save_result == SaveResult::RecoverySaved) {
              view->autosave_due = now + settings_.auto_save_delay_ms;
              view->recovery_due = now + kRecoveryDelayMs;
            } else {
              view->autosave_due = now + kRecoveryDelayMs;
            }
          }
        }
        if (!pending) {
          KillTimer(window_, kAutosaveTimer);
        } else if (!fast_poll && next_asset_check != std::numeric_limits<ULONGLONG>::max()) {
          const ULONGLONG wait = next_asset_check > now ? next_asset_check - now : 50;
          SetTimer(window_, kAutosaveTimer, static_cast<UINT>(std::clamp<ULONGLONG>(wait, 50, 1000)),
                   nullptr);
        }
      }
      return 0;
    case WM_DROPFILES: {
      HDROP drop = reinterpret_cast<HDROP>(wparam);
      const UINT count = DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
      for (UINT index = 0; index < count; ++index) {
        const UINT length = DragQueryFileW(drop, index, nullptr, 0);
        std::wstring path(static_cast<std::size_t>(length) + 1, L'\0');
        DragQueryFileW(drop, index, path.data(), length + 1);
        path.resize(length);
        if (IsSupportedImage(path) && !workspace_.empty() && active_document_ < documents_.size()) {
          if (documents_[active_document_]->ime_composing ||
              !IsMarkdownFile(documents_[active_document_]->document.path())) continue;
          AssetImportResult imported{};
          std::wstring error;
          const auto& document_path = documents_[active_document_]->document.path();
          if (!ImportImageAsset(path, workspace_, document_path, imported, error)) {
            MessageBoxW(window_, error.c_str(), L"画像の挿入", MB_ICONWARNING);
            continue;
          }
          const std::wstring markup = ImageMarkdown(std::filesystem::path(path).stem().wstring(),
                                                     imported.relative_reference);
          SendMessageW(documents_[active_document_]->editor, EM_REPLACESEL, TRUE,
                       reinterpret_cast<LPARAM>(markup.c_str()));
          if (!imported.safe_to_render)
            MessageBoxW(window_, imported.safety_message.c_str(), L"SVG安全性", MB_ICONWARNING);
          PopulateWorkspaceTree();
        } else {
          OpenInitialPath(path);
        }
      }
      DragFinish(drop);
      return 0;
    }
    case WM_MOUSEMOVE:
      if (outline_dragging_) {
        POINT point{};
        GetCursorPos(&point);
        ScreenToClient(outline_, &point);
        TVHITTESTINFO hit{};
        hit.pt = point;
        const HTREEITEM item = TreeView_HitTest(outline_, &hit);
        if (item) TreeView_SelectDropTarget(outline_, item);
        return 0;
      }
      if (workspace_dragging_) {
        POINT point{};
        GetCursorPos(&point);
        ScreenToClient(workspace_tree_, &point);
        TVHITTESTINFO hit{};
        hit.pt = point;
        const HTREEITEM item = TreeView_HitTest(workspace_tree_, &hit);
        if (item) TreeView_SelectDropTarget(workspace_tree_, item);
        return 0;
      }
      return DefWindowProcW(window_, message, wparam, lparam);
    case WM_LBUTTONUP:
      if (outline_dragging_) {
        const HTREEITEM target = TreeView_GetDropHilight(outline_);
        outline_dragging_ = false;
        ReleaseCapture();
        TreeView_SelectDropTarget(outline_, nullptr);
        if (target) {
          TVITEMW item{};
          item.mask = TVIF_PARAM;
          item.hItem = target;
          if (TreeView_GetItem(outline_, &item))
            MoveOutlineSection(outline_drag_source_, static_cast<std::size_t>(item.lParam));
        }
        return 0;
      }
      if (workspace_dragging_) {
        const HTREEITEM target = TreeView_GetDropHilight(workspace_tree_);
        workspace_dragging_ = false;
        ReleaseCapture();
        TreeView_SelectDropTarget(workspace_tree_, nullptr);
        if (target) {
          TVITEMW item{};
          item.mask = TVIF_PARAM;
          item.hItem = target;
          if (TreeView_GetItem(workspace_tree_, &item) && item.lParam != 0) {
            auto directory = *reinterpret_cast<const std::filesystem::path*>(item.lParam);
            if (!std::filesystem::is_directory(directory)) directory = directory.parent_path();
            MoveWorkspaceSelectionTo(directory);
          }
        }
        workspace_drag_sources_.clear();
        return 0;
      }
      return DefWindowProcW(window_, message, wparam, lparam);
    case WM_CAPTURECHANGED:
      if (outline_dragging_ || workspace_dragging_) {
        outline_dragging_ = false;
        workspace_dragging_ = false;
        workspace_drag_sources_.clear();
        TreeView_SelectDropTarget(outline_, nullptr);
        TreeView_SelectDropTarget(workspace_tree_, nullptr);
      }
      return 0;
    case kWorkspaceSearchBatchMessage:
      ApplyWorkspaceSearchBatch(reinterpret_cast<void*>(lparam));
      return 0;
    case kWorkspaceSearchCompleteMessage:
      CompleteWorkspaceSearch(reinterpret_cast<void*>(lparam));
      return 0;
    case kHolidayUpdateMessage:
      CompleteHolidayUpdate(reinterpret_cast<void*>(lparam));
      return 0;
    case kTestThemeChangeMessage: {
      if (!TestAutomationSilent() || !workspace_store_ || (wparam != 1 && wparam != 2)) return FALSE;
      const auto path = workspace_store_->metadata_root() / L"settings.toml";
      SettingsLayer layer;
      std::wstring error;
      if (!LoadSettingsLayer(path, layer, error)) return FALSE;
      layer.theme = wparam == 1 ? ThemeMode::Light : ThemeMode::Dark;
      if (!SaveSettingsLayer(path, layer, error)) return FALSE;
      LoadAndApplySettings();
      return TRUE;
    }
    case WM_COMMAND: {
      const int command = LOWORD(wparam);
      if (command == kPanelHeaderExplorer || command == kPanelHeaderCalendar ||
          command == kPanelHeaderOutline || command == kPanelHeaderGit) {
        focused_panel_ = command == kPanelHeaderExplorer ? PanelId::Explorer :
            command == kPanelHeaderCalendar ? PanelId::Calendar :
            command == kPanelHeaderOutline ? PanelId::Outline : PanelId::Git;
        SetFocus(panel_headers_[PanelIndex(focused_panel_)]);
        UpdatePanelHeaders();
      }
      else if (command == kViewMoveFocusedLeftTop) MoveFocusedPanelToSlot(PanelSlot::LeftTop);
      else if (command == kViewMoveFocusedLeftBottom) MoveFocusedPanelToSlot(PanelSlot::LeftBottom);
      else if (command == kViewResizeFocusedNarrow) ResizeFocusedPanel(-24.0);
      else if (command == kViewResizeFocusedWide) ResizeFocusedPanel(24.0);
      else if (command == kViewMoveFocusedRightTop) MoveFocusedPanelToSlot(PanelSlot::RightTop);
      else if (command == kViewMoveFocusedRightBottom) MoveFocusedPanelToSlot(PanelSlot::RightBottom);
      else if (command >= kViewMoveExplorerLeftTop && command <= kViewMoveExplorerRightBottom)
        MovePanelToSlot(PanelId::Explorer, static_cast<PanelSlot>(command - kViewMoveExplorerLeftTop));
      else if (command >= kViewMoveCalendarLeftTop && command <= kViewMoveCalendarRightBottom)
        MovePanelToSlot(PanelId::Calendar, static_cast<PanelSlot>(command - kViewMoveCalendarLeftTop));
      else if (command >= kViewMoveOutlineLeftTop && command <= kViewMoveOutlineRightBottom)
        MovePanelToSlot(PanelId::Outline, static_cast<PanelSlot>(command - kViewMoveOutlineLeftTop));
      else if (command >= kViewMoveGitLeftTop && command <= kViewMoveGitRightBottom)
        MovePanelToSlot(PanelId::Git, static_cast<PanelSlot>(command - kViewMoveGitLeftTop));
      if (command == kFileOpenWorkspace) OpenWorkspaceDialog();
      else if (command == kFileNew) NewUntitledDocument();
      else if (command == kFileOpen) OpenFileDialog();
      else if (command == kFileQuickOpen) QuickOpen();
      else if (command == kFileClose && active_document_ < documents_.size()) CloseDocument(active_document_);
      else if (command == kFileSave && active_document_ < documents_.size())
        SaveDocument(*documents_[active_document_], SaveIntent::UserRequested);
      else if (command == kFileSaveAs && active_document_ < documents_.size())
        SaveDocumentAs(*documents_[active_document_]);
      else if (command == kFileReload) ReloadDocumentFromDisk();
      else if (command == kFileCompare) CompareDocumentWithDisk();
      else if (command == kFileDaily) CreateProfile(BuiltInProfile::Daily);
      else if (command == kFileMeeting) CreateProfile(BuiltInProfile::Meeting);
      else if (command == kFileMemo) CreateProfile(BuiltInProfile::Memo);
      else if (command == kFileProfile) CreateProfileById(L"", nullptr);
      else if (command == kWorkspaceNewFile) CreateEmptyFile();
      else if (command == kWorkspaceNewFolder) CreateFolder();
      else if (command == kWorkspaceCopy) CopySelectedFile();
      else if (command == kWorkspacePaste) PasteCopiedFile();
      else if (command == kWorkspaceRenameMove) RenameOrMoveSelected();
      else if (command == kWorkspaceDelete) DeleteSelected();
      else if (command == kWorkspaceCopyPath) CopySelectedPathToClipboard();
      else if (command == kWorkspaceShowExplorer) ShowSelectedInExplorer();
      else if (command == kWorkspaceTrust) SetWorkspaceTrust(true);
      else if (command == kWorkspaceUntrust) SetWorkspaceTrust(false);
      else if (command == kGitStatus) RunGitStatus();
      else if (command >= kGitDiff && command <= kGitPush) RunGitAction(command);
      else if (command == kGitConflicts) ShowGitConflicts();
      else if (command == kGitConflictPrevious) NavigateGitConflict(true);
      else if (command == kGitConflictNext) NavigateGitConflict(false);
      else if (command == kGitConflictCurrent) ResolveGitConflict(ConflictChoice::Current);
      else if (command == kGitConflictIncoming) ResolveGitConflict(ConflictChoice::Incoming);
      else if (command == kGitConflictBoth) ResolveGitConflict(ConflictChoice::Both);
      else if (command == kGitConflictResolved) MarkGitConflictResolved();
      else if (command == kImageUpload) UploadImageAtCaret();
      else if (command == kImageWidth320) ResizeImageAtCaret(320);
      else if (command == kImageWidth480) ResizeImageAtCaret(480);
      else if (command == kImageWidth640) ResizeImageAtCaret(640);
      else if (command == kFileExit) SendMessageW(window_, WM_CLOSE, 0, 0);
      else if (command == kEditFind) ShowFindBar();
      else if (command == kEditFindNext || command == kFindNext) FindNext();
      else if (command == kReplaceOne) ReplaceCurrentDocument(false);
      else if (command == kReplaceDocument) ReplaceCurrentDocument(true);
      else if (command == kEditFindWorkspace) SearchWorkspaceFromFindBar();
      else if (command == kEditReplaceWorkspace || command == kReplaceWorkspace)
        ReplaceWorkspaceFromFindBar();
      else if (command == kFindWorkspace) SearchWorkspaceFromFindBar();
      else if (command == kViewCalendar) {
        if (auto* panel = panel_layout_.Find(PanelId::Calendar))
          panel->hidden = !panel->hidden;
        ShowWindow(calendar_, panel_layout_.Find(PanelId::Calendar)->hidden ? SW_HIDE : SW_SHOW);
        SavePanelLayout();
        LayoutControls();
      }
      else if (command == kCalendarOpenSelected) OpenSelectedCalendarDate();
      else if (command == kCalendarImportHolidays) ImportHolidayData();
      else if (command == kCalendarUpdateHolidays) StartHolidayUpdate(true);
      else if (command == kViewSettings) OpenWorkspaceSettings();
      else if (command == kViewSettingsFiles) OpenWorkspaceSettingsFiles();
      else if (command == kViewProfiles) ManageProfiles();
      else if (command == kViewCompact) ToggleCompactWindow();
      else if (command == kViewWorkspacePane) {
        workspace_pane_collapsed_ = !workspace_pane_collapsed_;
        if (auto* panel = panel_layout_.Find(PanelId::Explorer))
          panel->collapsed = workspace_pane_collapsed_;
        SavePanelLayout();
        LayoutControls();
      }
      else if (command == kViewOutlinePane) {
        outline_pane_collapsed_ = !outline_pane_collapsed_;
        if (auto* panel = panel_layout_.Find(PanelId::Outline))
          panel->collapsed = outline_pane_collapsed_;
        SavePanelLayout();
        LayoutControls();
      }
      else if (command == kViewResetPanels) {
        panel_layout_.Reset();
        workspace_pane_collapsed_ = false;
        outline_pane_collapsed_ = false;
        SavePanelLayout();
        LayoutControls();
      }
      else if (command == kViewCommandPalette) ShowCommandPalette();
      else if (command == kTableRowBefore) ApplyTableAction(TableAction::InsertRowBefore);
      else if (command == kTableRowAfter) ApplyTableAction(TableAction::InsertRowAfter);
      else if (command == kTableRowDelete) ApplyTableAction(TableAction::DeleteRow);
      else if (command == kTableColumnBefore) ApplyTableAction(TableAction::InsertColumnBefore);
      else if (command == kTableColumnAfter) ApplyTableAction(TableAction::InsertColumnAfter);
      else if (command == kTableColumnDelete) ApplyTableAction(TableAction::DeleteColumn);
      else if (workspace_search_started_ &&
               (((command == kFindEdit || command == kFindIncludeGlob ||
                  command == kFindExcludeGlob) && HIWORD(wparam) == EN_CHANGE) ||
                ((command == kFindCase || command == kFindRegex || command == kFindWord) &&
                 HIWORD(wparam) == BN_CLICKED)))
        ScheduleWorkspaceSearch();
      else if (HIWORD(wparam) == EN_CHANGE) OnEditorChanged(reinterpret_cast<HWND>(lparam));
      return 0;
    }
    case WM_MEASUREITEM: {
      auto* measure = reinterpret_cast<MEASUREITEMSTRUCT*>(lparam);
      if (!measure || measure->CtlType != ODT_MENU) return FALSE;
      MeasureMenuItem(*measure);
      return TRUE;
    }
    case WM_DRAWITEM: {
      const auto* draw = reinterpret_cast<const DRAWITEMSTRUCT*>(lparam);
      if (!draw) return FALSE;
      if (draw->CtlType == ODT_MENU) {
        DrawMenuItem(*draw);
        return TRUE;
      }
      if (draw->hwndItem == status_) {
        DrawStatusItem(*draw);
        return TRUE;
      }
      return FALSE;
    }
    case WM_NOTIFY: {
      const auto* header = reinterpret_cast<NMHDR*>(lparam);
      if (header && header->code == NM_CUSTOMDRAW &&
          (header->hwndFrom == workspace_tree_ || header->hwndFrom == outline_ ||
           header->hwndFrom == tabs_ || header->hwndFrom == find_results_ ||
           header->hwndFrom == status_)) {
        auto* draw = reinterpret_cast<NMCUSTOMDRAW*>(lparam);
        if (draw->dwDrawStage == CDDS_PREPAINT) return CDRF_NOTIFYITEMDRAW;
        if (draw->dwDrawStage == CDDS_ITEMPREPAINT) {
          const bool selected = (draw->uItemState & CDIS_SELECTED) != 0;
          SetTextColor(draw->hdc, selected ? RGB(255, 255, 255) : theme_foreground_);
          SetBkColor(draw->hdc, selected ? theme_accent_ : theme_surface_);
          return CDRF_DODEFAULT;
        }
      }
      if (header->hwndFrom == tabs_ && header->code == TCN_SELCHANGE) {
        const int index = TabCtrl_GetCurSel(tabs_);
        if (index >= 0) ActivateDocument(static_cast<std::size_t>(index));
      } else if (header->hwndFrom == workspace_tree_ && header->code == TVN_ITEMEXPANDINGW) {
        const auto* expansion = reinterpret_cast<NMTREEVIEWW*>(lparam);
        if ((expansion->action & TVE_EXPAND) != 0 && expansion->itemNew.lParam != 0) {
          const HTREEITEM first = TreeView_GetChild(workspace_tree_, expansion->itemNew.hItem);
          if (first) {
            TVITEMW child{};
            child.mask = TVIF_PARAM;
            child.hItem = first;
            if (TreeView_GetItem(workspace_tree_, &child) && child.lParam == 0) {
              TreeView_DeleteItem(workspace_tree_, first);
              AddTreeDirectory(expansion->itemNew.hItem,
                  *reinterpret_cast<const std::filesystem::path*>(expansion->itemNew.lParam), 0);
            }
          }
        }
      } else if (header->hwndFrom == workspace_tree_ && header->code == NM_DBLCLK) {
        const auto path = SelectedTreePath();
        if (!path.empty() && std::filesystem::is_regular_file(path)) OpenDocument(path);
      } else if (header->hwndFrom == workspace_tree_ && header->code == TVN_BEGINDRAGW) {
        const auto* drag = reinterpret_cast<NMTREEVIEWW*>(lparam);
        workspace_drag_sources_ = SelectedTreePaths();
        if (drag->itemNew.lParam != 0) {
          const auto dragged = *reinterpret_cast<const std::filesystem::path*>(drag->itemNew.lParam);
          if (!std::ranges::any_of(workspace_drag_sources_, [&](const auto& path) { return path == dragged; }))
            workspace_drag_sources_ = {dragged};
        }
        std::erase_if(workspace_drag_sources_, [&](const auto& path) {
          return _wcsicmp(path.c_str(), workspace_.c_str()) == 0;
        });
        if (!workspace_drag_sources_.empty()) {
          workspace_dragging_ = true;
          SetCapture(window_);
        }
      } else if (header->hwndFrom == outline_ && header->code == NM_DBLCLK &&
                 active_document_ < documents_.size()) {
        TVITEMW item{};
        item.mask = TVIF_PARAM;
        item.hItem = TreeView_GetSelection(outline_);
        if (item.hItem && TreeView_GetItem(outline_, &item)) {
          const auto position = static_cast<LONG>(documents_[active_document_]->editor_snapshot.SourceToNative(
              static_cast<std::size_t>(item.lParam)));
          SendMessageW(documents_[active_document_]->editor, EM_SETSEL, position, position);
          SetFocus(documents_[active_document_]->editor);
        }
      } else if (header->hwndFrom == outline_ && header->code == TVN_BEGINDRAGW) {
        const auto* drag = reinterpret_cast<NMTREEVIEWW*>(lparam);
        outline_drag_source_ = static_cast<std::size_t>(drag->itemNew.lParam);
        outline_dragging_ = true;
        SetCapture(window_);
      } else if (header->hwndFrom == calendar_ && header->code == MCN_GETDAYSTATE) {
        auto* state = reinterpret_cast<NMDAYSTATE*>(lparam);
        SYSTEMTIME month = state->stStart;
        for (int index = 0; index < state->cDayState; ++index) {
          MONTHDAYSTATE days{};
          for (int day = 1; day <= 31; ++day) {
            if (JapaneseHolidayName(month.wYear, month.wMonth, day)) days |= 1U << (day - 1);
            if (!workspace_.empty()) {
              wchar_t relative[128]{};
              swprintf_s(relative, L"Dairy/%04u/%04u%02u/%04u%02u%02d.md",
                         month.wYear, month.wYear, month.wMonth,
                         month.wYear, month.wMonth, day);
              if (std::filesystem::exists(workspace_ / relative)) days |= 1U << (day - 1);
            }
          }
          state->prgDayState[index] = days;
          if (++month.wMonth > 12) {
            month.wMonth = 1;
            ++month.wYear;
          }
        }
      } else if (header->hwndFrom == calendar_ && header->code == NM_DBLCLK) {
        OpenSelectedCalendarDate();
      } else if (header->hwndFrom == calendar_ && header->code == MCN_SELECT) {
        const auto* selection = reinterpret_cast<NMSELCHANGE*>(lparam);
        UpdateCalendarDetails(selection->stSelStart);
      } else if (header->hwndFrom == find_results_ &&
                 (header->code == NM_DBLCLK || header->code == LVN_ITEMACTIVATE)) {
        const int index = ListView_GetNextItem(find_results_, -1, LVNI_SELECTED);
        if (index >= 0) OpenWorkspaceSearchResult(static_cast<std::size_t>(index));
      } else if (!suppress_editor_change_ && active_document_ < documents_.size() &&
                 header->hwndFrom == documents_[active_document_]->editor &&
                 header->code == EN_SELCHANGE &&
                 !documents_[active_document_]->native_edit_in_flight &&
                 !documents_[active_document_]->ime_composing &&
                 documents_[active_document_]->sync_due == 0) {
        ApplyMarkdownPresentation(*documents_[active_document_], false);
      } else if (active_document_ < documents_.size() &&
                 header->hwndFrom == documents_[active_document_]->editor &&
                 header->code == EN_LINK) {
        const auto* link = reinterpret_cast<const ENLINK*>(lparam);
        auto& view = *documents_[active_document_];
        const auto source_position = view.editor_snapshot.NativeToSource(link->chrg.cpMin);
        if (link->msg == WM_LBUTTONUP) OpenLinkAtSourcePosition(view, source_position, true);
        else if (link->msg == WM_MOUSEMOVE) OpenLinkAtSourcePosition(view, source_position, false);
      }
      return 0;
    }
    case WM_CLOSE: {
      if (SaveAllForExit(true) == SaveAllResult::Cancelled) return 0;
      if (workspace_search_worker_.joinable()) {
        workspace_search_worker_.request_stop();
        workspace_search_worker_.join();
      }
      if (holiday_update_worker_.joinable()) {
        holiday_update_worker_.request_stop();
        holiday_update_worker_.join();
      }
      ++workspace_search_generation_;
      ++holiday_update_generation_;
      MSG pending_search{};
      while (PeekMessageW(&pending_search, window_, kWorkspaceSearchBatchMessage,
                          kWorkspaceSearchCompleteMessage, PM_REMOVE)) {
        if (pending_search.message == kWorkspaceSearchBatchMessage)
          delete reinterpret_cast<WorkspaceSearchBatchMessage*>(pending_search.lParam);
        else if (pending_search.message == kWorkspaceSearchCompleteMessage)
          delete reinterpret_cast<WorkspaceSearchCompleteMessage*>(pending_search.lParam);
      }
      MSG pending_holiday{};
      while (PeekMessageW(&pending_holiday, window_, kHolidayUpdateMessage,
                          kHolidayUpdateMessage, PM_REMOVE))
        delete reinterpret_cast<HolidayUpdateMessage*>(pending_holiday.lParam);
      SaveSession();
      for (auto& view : documents_) {
        if (!view->compact_window) continue;
        SetParent(view->editor, window_);
        DestroyWindow(view->compact_window);
        view->compact_window = nullptr;
      }
      DestroyWindow(window_);
      return 0;
    }
    case WM_SETTINGCHANGE:
      if (settings_.theme == ThemeMode::System) ApplySettings();
      return 0;
    case WM_ERASEBKGND: {
      RECT area{};
      GetClientRect(window_, &area);
      FillRect(reinterpret_cast<HDC>(wparam), &area,
               background_brush_ ? background_brush_ : GetSysColorBrush(COLOR_WINDOW));
      return 1;
    }
    case WM_CTLCOLORBTN:
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX: {
      auto dc = reinterpret_cast<HDC>(wparam);
      const HWND control = reinterpret_cast<HWND>(lparam);
      const bool editor = std::ranges::any_of(documents_, [control](const auto& candidate) {
        return candidate->editor == control;
      });
      const bool input = control == find_edit_ || control == replace_edit_ ||
          control == find_include_glob_ || control == find_exclude_glob_;
      const COLORREF background = editor ? theme_editor_ : input ? theme_input_ : theme_surface_;
      HBRUSH brush = editor ? editor_brush_ : input ? input_brush_ : surface_brush_;
      SetTextColor(dc, theme_foreground_);
      SetBkColor(dc, background);
      SetBkMode(dc, OPAQUE);
      return reinterpret_cast<LRESULT>(brush ? brush : GetSysColorBrush(COLOR_WINDOW));
    }
    case WM_DESTROY:
      if (menu_ && IsMenu(menu_)) {
        SetMenu(window_, nullptr);
        DestroyMenu(menu_);
        menu_ = nullptr;
      }
      if (workspace_mutex_) {
        CloseHandle(workspace_mutex_);
        workspace_mutex_ = nullptr;
      }
      PostQuitMessage(0);
      return 0;
    default:
      return DefWindowProcW(window_, message, wparam, lparam);
  }
}

void Application::CreateMenuBar() {
  HMENU menu = CreateMenu();
  HMENU file = CreatePopupMenu();
  AppendMenuW(file, MF_STRING, kFileOpenWorkspace, L"Workspaceを開く…");
  AppendMenuW(file, MF_STRING, kFileNew, L"新規無題文書\tCtrl+N");
  AppendMenuW(file, MF_STRING, kFileOpen, L"ファイルを開く…\tCtrl+O");
  AppendMenuW(file, MF_STRING, kFileQuickOpen, L"Quick Open…\tCtrl+P");
  AppendMenuW(file, MF_STRING, kFileClose, L"タブを閉じる\tCtrl+W");
  AppendMenuW(file, MF_STRING, kFileSave, L"保存\tCtrl+S");
  AppendMenuW(file, MF_STRING, kFileSaveAs, L"別名で保存…");
  AppendMenuW(file, MF_STRING, kFileReload, L"ディスクから再読込み…");
  AppendMenuW(file, MF_STRING, kFileCompare, L"ディスク上の内容と比較…");
  AppendMenuW(file, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(file, MF_STRING, kFileDaily, L"今日のDairyを開く");
  AppendMenuW(file, MF_STRING, kFileMeeting, L"Meetingノートを作成");
  AppendMenuW(file, MF_STRING, kFileMemo, L"Memoを作成");
  AppendMenuW(file, MF_STRING, kFileProfile, L"プロファイルから作成…");
  AppendMenuW(file, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(file, MF_STRING, kFileExit, L"終了");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(file), L"ファイル");
  HMENU workspace = CreatePopupMenu();
  AppendMenuW(workspace, MF_STRING, kWorkspaceNewFile, L"新しいファイル…");
  AppendMenuW(workspace, MF_STRING, kWorkspaceNewFolder, L"新しいフォルダー");
  AppendMenuW(workspace, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(workspace, MF_STRING, kWorkspaceCopy, L"コピー");
  AppendMenuW(workspace, MF_STRING, kWorkspacePaste, L"貼り付け");
  AppendMenuW(workspace, MF_STRING, kWorkspaceRenameMove, L"名前変更・移動…");
  AppendMenuW(workspace, MF_STRING, kWorkspaceDelete, L"ごみ箱へ移動…");
  AppendMenuW(workspace, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(workspace, MF_STRING, kWorkspaceCopyPath, L"パスをコピー");
  AppendMenuW(workspace, MF_STRING, kWorkspaceShowExplorer, L"Explorerで表示");
  AppendMenuW(workspace, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(workspace, MF_STRING, kWorkspaceTrust, L"このWorkspaceを信頼する…");
  AppendMenuW(workspace, MF_STRING, kWorkspaceUntrust, L"信頼を解除");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(workspace), L"Workspace");
  HMENU edit = CreatePopupMenu();
  AppendMenuW(edit, MF_STRING, kEditFind, L"検索…\tCtrl+F");
  AppendMenuW(edit, MF_STRING, kEditFindNext, L"次を検索\tF3");
  AppendMenuW(edit, MF_STRING, kEditFindWorkspace, L"Workspaceを検索");
  AppendMenuW(edit, MF_STRING, kEditReplaceWorkspace, L"Workspaceを置換…");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(edit), L"編集");
  HMENU view = CreatePopupMenu();
  AppendMenuW(view, MF_STRING, kCalendarOpenSelected, L"選択日のDailyを開く");
  AppendMenuW(view, MF_STRING, kViewCalendar, L"カレンダー");
  AppendMenuW(view, MF_STRING, kCalendarImportHolidays, L"祝日CSVをローカル取込み…");
  AppendMenuW(view, MF_STRING, kCalendarUpdateHolidays, L"祝日を内閣府から今すぐ確認");
  AppendMenuW(view, MF_STRING, kViewSettings, L"Workspace設定を開く");
  AppendMenuW(view, MF_STRING, kViewSettingsFiles, L"設定ファイルを詳細編集");
  AppendMenuW(view, MF_STRING, kViewProfiles, L"作成プロファイルを管理…");
  AppendMenuW(view, MF_STRING, kViewCompact, L"現在の文書をコンパクト表示");
  AppendMenuW(view, MF_STRING, kViewWorkspacePane, L"Workspace paneを折り畳む／表示");
  AppendMenuW(view, MF_STRING, kViewOutlinePane, L"Outline paneを折り畳む／表示");
  AppendMenuW(view, MF_STRING, kViewResetPanels, L"パネル配置を初期化");
  HMENU panel_layout = CreatePopupMenu();
  AppendMenuW(panel_layout, MF_STRING, kViewResizeFocusedNarrow, L"フォーカス中を狭くする");
  AppendMenuW(panel_layout, MF_STRING, kViewResizeFocusedWide, L"フォーカス中を広くする");
  AppendMenuW(panel_layout, MF_STRING, kViewMoveFocusedLeftTop, L"フォーカス中を左上へ\tCtrl+Alt+1");
  AppendMenuW(panel_layout, MF_STRING, kViewMoveFocusedLeftBottom, L"フォーカス中を左下へ\tCtrl+Alt+2");
  AppendMenuW(panel_layout, MF_STRING, kViewMoveFocusedRightTop, L"フォーカス中を右上へ\tCtrl+Alt+3");
  AppendMenuW(panel_layout, MF_STRING, kViewMoveFocusedRightBottom, L"フォーカス中を右下へ\tCtrl+Alt+4");
  auto add_panel_layout_menu = [&](const wchar_t* label, int first_command) {
    HMENU destinations = CreatePopupMenu();
    AppendMenuW(destinations, MF_STRING, first_command + 0, L"左上");
    AppendMenuW(destinations, MF_STRING, first_command + 1, L"左下");
    AppendMenuW(destinations, MF_STRING, first_command + 2, L"右上");
    AppendMenuW(destinations, MF_STRING, first_command + 3, L"右下");
    AppendMenuW(panel_layout, MF_POPUP, reinterpret_cast<UINT_PTR>(destinations), label);
  };
  add_panel_layout_menu(L"Explorerを移動", kViewMoveExplorerLeftTop);
  add_panel_layout_menu(L"Calendarを移動", kViewMoveCalendarLeftTop);
  add_panel_layout_menu(L"Outlineを移動", kViewMoveOutlineLeftTop);
  add_panel_layout_menu(L"Gitを移動", kViewMoveGitLeftTop);
  AppendMenuW(view, MF_POPUP, reinterpret_cast<UINT_PTR>(panel_layout), L"パネル配置");
  AppendMenuW(view, MF_STRING, kViewCommandPalette, L"コマンドパレット…\tCtrl+Shift+P");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(view), L"表示");
  HMENU table = CreatePopupMenu();
  AppendMenuW(table, MF_STRING, kTableRowBefore, L"上に行を追加");
  AppendMenuW(table, MF_STRING, kTableRowAfter, L"下に行を追加");
  AppendMenuW(table, MF_STRING, kTableRowDelete, L"行を削除");
  AppendMenuW(table, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(table, MF_STRING, kTableColumnBefore, L"左に列を追加");
  AppendMenuW(table, MF_STRING, kTableColumnAfter, L"右に列を追加");
  AppendMenuW(table, MF_STRING, kTableColumnDelete, L"列を削除");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(table), L"表");
  HMENU image = CreatePopupMenu();
  AppendMenuW(image, MF_STRING, kImageWidth320, L"選択画像の表示幅を320 DIPにする");
  AppendMenuW(image, MF_STRING, kImageWidth480, L"選択画像の表示幅を480 DIPにする");
  AppendMenuW(image, MF_STRING, kImageWidth640, L"選択画像の表示幅を640 DIPにする");
  AppendMenuW(image, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(image, MF_STRING, kImageUpload, L"カーソル位置の画像をstorageへupload…");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(image), L"画像");
  HMENU git = CreatePopupMenu();
  AppendMenuW(git, MF_STRING, kGitStatus, L"Status / Branches…");
  AppendMenuW(git, MF_STRING, kGitDiff, L"差分を表示…");
  AppendMenuW(git, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(git, MF_STRING, kGitStageAll, L"すべての変更をステージ…");
  AppendMenuW(git, MF_STRING, kGitUnstageAll, L"すべてのステージを解除…");
  AppendMenuW(git, MF_STRING, kGitCommit, L"コミット…");
  AppendMenuW(git, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(git, MF_STRING, kGitBranchCreate, L"ブランチを作成して切替…");
  AppendMenuW(git, MF_STRING, kGitBranchSwitch, L"ブランチを切替…");
  AppendMenuW(git, MF_STRING, kGitMerge, L"ブランチをマージ…");
  AppendMenuW(git, MF_STRING, kGitMergeAbort, L"マージを中止…");
  AppendMenuW(git, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(git, MF_STRING, kGitConflicts, L"未解決競合の一覧・ファイルを開く…");
  AppendMenuW(git, MF_STRING, kGitConflictPrevious, L"前の競合箇所へ");
  AppendMenuW(git, MF_STRING, kGitConflictNext, L"次の競合箇所へ");
  AppendMenuW(git, MF_STRING, kGitConflictCurrent, L"競合箇所で現在側を採用");
  AppendMenuW(git, MF_STRING, kGitConflictIncoming, L"競合箇所で相手側を採用");
  AppendMenuW(git, MF_STRING, kGitConflictBoth, L"競合箇所で両方を採用");
  AppendMenuW(git, MF_STRING, kGitConflictResolved, L"現在のファイルを解決済みにする…");
  AppendMenuW(git, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(git, MF_STRING, kGitFetch, L"Fetch…");
  AppendMenuW(git, MF_STRING, kGitPull, L"Pull (fast-forward only)…");
  AppendMenuW(git, MF_STRING, kGitPush, L"Push…");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(git), L"Git");
  menu_ = menu;
  menu_labels_.clear();
  menu_labels_.reserve(128);
  std::function<void(HMENU)> prepare_menu = [&](HMENU current) {
    if (!current) return;
    const int count = GetMenuItemCount(current);
    for (int index = 0; index < count; ++index) {
      MENUITEMINFOW item{sizeof(item)};
      item.fMask = MIIM_FTYPE | MIIM_SUBMENU;
      if (!GetMenuItemInfoW(current, static_cast<UINT>(index), TRUE, &item)) continue;
      if (item.hSubMenu) prepare_menu(item.hSubMenu);
      const UINT length = GetMenuStringW(current, static_cast<UINT>(index), nullptr, 0,
                                         MF_BYPOSITION);
      std::wstring label(length, L'\0');
      if (length != 0)
        GetMenuStringW(current, static_cast<UINT>(index), label.data(), length + 1, MF_BYPOSITION);
      MENUITEMINFOW owner{sizeof(owner)};
      owner.fMask = MIIM_FTYPE | MIIM_DATA;
      owner.fType = item.fType | MFT_OWNERDRAW;
      if (item.fType & MFT_SEPARATOR) {
        owner.dwItemData = kOwnerDrawSeparator;
      } else {
        auto stable_label = std::make_unique<std::wstring>(std::move(label));
        owner.dwItemData = reinterpret_cast<ULONG_PTR>(stable_label.get());
        menu_labels_.push_back(std::move(stable_label));
      }
      SetMenuItemInfoW(current, static_cast<UINT>(index), TRUE, &owner);
    }
  };
  prepare_menu(menu_);
  SetMenu(window_, menu_);
}

void Application::CreateControls() {
  workspace_tree_ = CreateWindowExW(WS_EX_CLIENTEDGE, WC_TREEVIEWW, nullptr,
                                    WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_HASLINES |
                                        TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                                    0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kWorkspaceTree), instance_, nullptr);
  TreeView_SetExtendedStyle(workspace_tree_, TVS_EX_MULTISELECT, TVS_EX_MULTISELECT);
  SetWindowSubclass(workspace_tree_, TreeDragSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  tabs_ = CreateWindowExW(0, WC_TABCONTROLW, nullptr, WS_CHILD | WS_VISIBLE | TCS_TABS,
                          0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kTabs), instance_, nullptr);
  outline_ = CreateWindowExW(WS_EX_CLIENTEDGE, WC_TREEVIEWW, nullptr,
                             WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_HASLINES |
                                 TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                                 0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kOutline), instance_, nullptr);
  SetWindowSubclass(outline_, TreeDragSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  status_ = CreateWindowExW(0, STATUSCLASSNAMEW, nullptr,
                             WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP,
                             0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  SetStatusText(L"");
  find_bar_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC", nullptr, WS_CHILD,
                              0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindBar), instance_, nullptr);
  find_edit_ = CreateWindowExW(0, L"EDIT", nullptr, WS_CHILD | WS_BORDER | ES_AUTOHSCROLL,
                               0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindEdit), instance_, nullptr);
  find_next_ = CreateWindowExW(0, L"BUTTON", L"次を検索", WS_CHILD | BS_PUSHBUTTON,
                               0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindNext), instance_, nullptr);
  replace_edit_ = CreateWindowExW(0, L"EDIT", nullptr, WS_CHILD | WS_BORDER | ES_AUTOHSCROLL,
                                  0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindReplaceEdit), instance_, nullptr);
  find_workspace_ = CreateWindowExW(0, L"BUTTON", L"全体検索", WS_CHILD | BS_PUSHBUTTON,
                                    0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindWorkspace), instance_, nullptr);
  replace_workspace_ = CreateWindowExW(0, L"BUTTON", L"Preview→置換", WS_CHILD | BS_PUSHBUTTON,
                                       0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kReplaceWorkspace), instance_, nullptr);
  find_case_ = CreateWindowExW(0, L"BUTTON", L"大小文字", WS_CHILD | BS_AUTOCHECKBOX,
                               0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindCase), instance_, nullptr);
  find_regex_ = CreateWindowExW(0, L"BUTTON", L"Regex", WS_CHILD | BS_AUTOCHECKBOX,
                                0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindRegex), instance_, nullptr);
  find_word_ = CreateWindowExW(0, L"BUTTON", L"単語", WS_CHILD | BS_AUTOCHECKBOX,
                               0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindWord), instance_, nullptr);
  find_include_glob_ = CreateWindowExW(0, L"EDIT", L"**/*.md;**/*.markdown",
      WS_CHILD | WS_BORDER | ES_AUTOHSCROLL, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kFindIncludeGlob), instance_, nullptr);
  SendMessageW(find_include_glob_, EM_SETCUEBANNER, TRUE,
               reinterpret_cast<LPARAM>(L"対象glob（;区切り）"));
  find_exclude_glob_ = CreateWindowExW(0, L"EDIT", nullptr,
      WS_CHILD | WS_BORDER | ES_AUTOHSCROLL, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kFindExcludeGlob), instance_, nullptr);
  SendMessageW(find_exclude_glob_, EM_SETCUEBANNER, TRUE,
               reinterpret_cast<LPARAM>(L"除外glob（;区切り）"));
  replace_one_ = CreateWindowExW(0, L"BUTTON", L"1件置換", WS_CHILD | BS_PUSHBUTTON,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kReplaceOne), instance_, nullptr);
  replace_document_ = CreateWindowExW(0, L"BUTTON", L"文書置換", WS_CHILD | BS_PUSHBUTTON,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kReplaceDocument), instance_, nullptr);
  find_results_ = CreateWindowExW(WS_EX_CLIENTEDGE, WC_LISTVIEWW, nullptr,
      WS_CHILD | LVS_REPORT | LVS_SINGLESEL | LVS_SHOWSELALWAYS,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindResults), instance_, nullptr);
  ListView_SetExtendedListViewStyle(find_results_, LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER);
  LVCOLUMNW result_column{LVCF_TEXT | LVCF_WIDTH};
  result_column.pszText = const_cast<wchar_t*>(L"ファイル");
  result_column.cx = 230;
  ListView_InsertColumn(find_results_, 0, &result_column);
  result_column.pszText = const_cast<wchar_t*>(L"行");
  result_column.cx = 56;
  ListView_InsertColumn(find_results_, 1, &result_column);
  result_column.pszText = const_cast<wchar_t*>(L"内容");
  result_column.cx = 560;
  ListView_InsertColumn(find_results_, 2, &result_column);
  calendar_ = CreateWindowExW(WS_EX_CLIENTEDGE, MONTHCAL_CLASSW, nullptr,
                              WS_CHILD | MCS_DAYSTATE | MCS_WEEKNUMBERS,
                              0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  git_panel_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC",
                              L"Git\r\n状態を更新するにはGit: Statusを実行してください。",
                              WS_CHILD | SS_LEFT | SS_NOPREFIX,
                              0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  calendar_details_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC",
                                      L"選択日: （カレンダーから選択）",
                                      WS_CHILD | SS_LEFT | SS_NOPREFIX,
                                      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Explorer)] = CreateWindowExW(
      0, L"BUTTON", L"Explorer", WS_CHILD | BS_PUSHBUTTON | BS_FLAT | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderExplorer), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Calendar)] = CreateWindowExW(
      0, L"BUTTON", L"Calendar", WS_CHILD | BS_PUSHBUTTON | BS_FLAT | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderCalendar), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Outline)] = CreateWindowExW(
      0, L"BUTTON", L"Outline", WS_CHILD | BS_PUSHBUTTON | BS_FLAT | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderOutline), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Git)] = CreateWindowExW(
      0, L"BUTTON", L"Git", WS_CHILD | BS_PUSHBUTTON | BS_FLAT | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderGit), instance_, nullptr);
  SendMessageW(calendar_, MCM_SETFIRSTDAYOFWEEK, 0, 6);
  SetWindowSubclass(calendar_, CalendarSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  calendar_tooltip_ = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
      WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT, CW_USEDEFAULT,
      window_, nullptr, instance_, nullptr);
  if (calendar_tooltip_) {
    TTTOOLINFOW tool{sizeof(tool)};
    tool.uFlags = TTF_SUBCLASS;
    tool.hwnd = calendar_;
    tool.uId = 1;
    GetClientRect(calendar_, &tool.rect);
    tool.lpszText = const_cast<wchar_t*>(L"日付");
    SendMessageW(calendar_tooltip_, TTM_ADDTOOLW, 0, reinterpret_cast<LPARAM>(&tool));
    SendMessageW(calendar_tooltip_, TTM_SETMAXTIPWIDTH, 0, 420);
  }
  HFONT font = static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_, replace_edit_,
                       find_workspace_, replace_workspace_, find_case_, find_regex_, find_word_,
                       find_include_glob_, find_exclude_glob_, replace_one_, replace_document_,
                       find_results_, git_panel_, calendar_details_, panel_headers_[0], panel_headers_[1],
                        panel_headers_[2], panel_headers_[3]})
    SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
}

void Application::ApplyChromeTheme() {
  if (!window_) return;

  // Windows 11 owns the non-client frame; these documented DWM attributes keep
  // the title bar and border on the same token palette as the client surfaces.
  const BOOL use_dark_mode = dark_theme_ ? TRUE : FALSE;
  constexpr DWORD kDwmUseImmersiveDarkMode = 20;
  constexpr DWORD kDwmBorderColor = 34;
  constexpr DWORD kDwmCaptionColor = 35;
  constexpr DWORD kDwmTextColor = 36;
  DwmSetWindowAttribute(window_, kDwmUseImmersiveDarkMode, &use_dark_mode,
                        sizeof(use_dark_mode));
  DwmSetWindowAttribute(window_, kDwmBorderColor, &theme_border_, sizeof(theme_border_));
  DwmSetWindowAttribute(window_, kDwmCaptionColor, &theme_surface_, sizeof(theme_surface_));
  DwmSetWindowAttribute(window_, kDwmTextColor, &theme_foreground_, sizeof(theme_foreground_));

  if (menu_) {
    MENUINFO menu_info{sizeof(menu_info)};
    menu_info.fMask = MIM_BACKGROUND | MIM_APPLYTOSUBMENUS;
    menu_info.hbrBack = surface_brush_;
    SetMenuInfo(menu_, &menu_info);
    DrawMenuBar(window_);
  }
  if (status_) InvalidateRect(status_, nullptr, TRUE);
  InvalidateRect(window_, nullptr, TRUE);
}

void Application::SetStatusText(std::wstring_view text) {
  status_text_.assign(text);
  if (!status_) return;
  // Owner-draw keeps the native status layout and accessibility identity while
  // making its background/text/border use the same tokens as the other panes.
  SendMessageW(status_, SB_SETTEXTW, SBT_OWNERDRAW,
               reinterpret_cast<LPARAM>(status_text_.c_str()));
  InvalidateRect(status_, nullptr, TRUE);
}

void Application::MeasureMenuItem(MEASUREITEMSTRUCT& measure) const {
  if (measure.itemData == kOwnerDrawSeparator) {
    measure.itemWidth = static_cast<UINT>(ScaleDip(window_, 16));
    measure.itemHeight = static_cast<UINT>(std::max(1, ScaleDip(window_, 8)));
    return;
  }
  const auto* label = reinterpret_cast<const std::wstring*>(measure.itemData);
  if (!label) {
    measure.itemWidth = static_cast<UINT>(ScaleDip(window_, 32));
    measure.itemHeight = static_cast<UINT>(ScaleDip(window_, 28));
    return;
  }
  HDC dc = GetDC(window_);
  if (!dc) {
    measure.itemWidth = static_cast<UINT>(ScaleDip(window_, 96));
    measure.itemHeight = static_cast<UINT>(ScaleDip(window_, 28));
    return;
  }
  HFONT font = editor_font_ ? editor_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  HGDIOBJ previous = SelectObject(dc, font);
  TEXTMETRICW metrics{};
  GetTextMetricsW(dc, &metrics);
  const auto tab = label->find(L'\t');
  const std::wstring_view full_text(*label);
  const std::wstring_view main_text = full_text.substr(0, tab);
  const std::wstring_view accelerator = tab == std::wstring::npos
      ? std::wstring_view{} : std::wstring_view(*label).substr(tab + 1);
  SIZE main_size{};
  SIZE accelerator_size{};
  GetTextExtentPoint32W(dc, main_text.data(), static_cast<int>(main_text.size()), &main_size);
  if (!accelerator.empty())
    GetTextExtentPoint32W(dc, accelerator.data(), static_cast<int>(accelerator.size()),
                          &accelerator_size);
  SelectObject(dc, previous);
  ReleaseDC(window_, dc);
  const int padding = ScaleDip(window_, 12);
  const int gap = accelerator.empty() ? 0 : ScaleDip(window_, 28);
  measure.itemWidth = static_cast<UINT>(std::max(ScaleDip(window_, 72),
      static_cast<int>(main_size.cx + accelerator_size.cx) + padding * 2 + gap));
  measure.itemHeight = static_cast<UINT>(std::max(ScaleDip(window_, 28),
      static_cast<int>(metrics.tmHeight) + ScaleDip(window_, 8)));
}

void Application::DrawMenuItem(const DRAWITEMSTRUCT& draw) const {
  if (!draw.hDC) return;
  RECT item = draw.rcItem;
  const bool separator = draw.itemData == kOwnerDrawSeparator;
  const bool selected = (draw.itemState & ODS_SELECTED) != 0;
  const bool disabled = (draw.itemState & ODS_DISABLED) != 0;
  const COLORREF background = selected ? theme_accent_ : theme_surface_;
  const COLORREF foreground = disabled ? theme_muted_ :
      (selected ? RGB(255, 255, 255) : theme_foreground_);
  HBRUSH background_brush = CreateSolidBrush(background);
  if (background_brush) {
    FillRect(draw.hDC, &item, background_brush);
    DeleteObject(background_brush);
  }
  if (separator) {
    const int y = item.top + (item.bottom - item.top) / 2;
    HPEN pen = CreatePen(PS_SOLID, 1, theme_border_);
    if (pen) {
      HGDIOBJ previous = SelectObject(draw.hDC, pen);
      MoveToEx(draw.hDC, item.left + ScaleDip(window_, 8), y, nullptr);
      LineTo(draw.hDC, item.right - ScaleDip(window_, 8), y);
      SelectObject(draw.hDC, previous);
      DeleteObject(pen);
    }
    return;
  }
  const auto* label = reinterpret_cast<const std::wstring*>(draw.itemData);
  if (!label) return;
  HFONT font = editor_font_ ? editor_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  HGDIOBJ previous = SelectObject(draw.hDC, font);
  SetBkMode(draw.hDC, TRANSPARENT);
  SetTextColor(draw.hDC, foreground);
  const int padding = ScaleDip(window_, 12);
  RECT text_rect = item;
  text_rect.left += padding;
  text_rect.right -= padding;
  const auto tab = label->find(L'\t');
  const std::wstring_view full_text(*label);
  const std::wstring_view main_text = full_text.substr(0, tab);
  const std::wstring_view accelerator = tab == std::wstring::npos
      ? std::wstring_view{} : std::wstring_view(*label).substr(tab + 1);
  DrawTextW(draw.hDC, main_text.data(), static_cast<int>(main_text.size()), &text_rect,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
  if (!accelerator.empty()) {
    RECT accelerator_rect = text_rect;
    accelerator_rect.left = accelerator_rect.right - ScaleDip(window_, 150);
    DrawTextW(draw.hDC, accelerator.data(), static_cast<int>(accelerator.size()), &accelerator_rect,
              DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_RIGHT);
  }
  if ((draw.itemState & ODS_FOCUS) != 0 && (draw.itemState & ODS_NOFOCUSRECT) == 0) {
    HPEN pen = CreatePen(PS_DOT, 1, theme_accent_);
    HBRUSH brush = static_cast<HBRUSH>(GetStockObject(HOLLOW_BRUSH));
    if (pen && brush) {
      HGDIOBJ old_pen = SelectObject(draw.hDC, pen);
      HGDIOBJ old_brush = SelectObject(draw.hDC, brush);
      Rectangle(draw.hDC, item.left + 1, item.top + 1, item.right - 1, item.bottom - 1);
      SelectObject(draw.hDC, old_brush);
      SelectObject(draw.hDC, old_pen);
    }
    if (pen) DeleteObject(pen);
  }
  SelectObject(draw.hDC, previous);
}

void Application::DrawStatusItem(const DRAWITEMSTRUCT& draw) const {
  if (!draw.hDC) return;
  RECT item = draw.rcItem;
  HBRUSH brush = CreateSolidBrush(theme_surface_);
  if (brush) {
    FillRect(draw.hDC, &item, brush);
    DeleteObject(brush);
  }
  HPEN pen = CreatePen(PS_SOLID, 1, theme_border_);
  if (pen) {
    HGDIOBJ previous = SelectObject(draw.hDC, pen);
    MoveToEx(draw.hDC, item.left, item.top, nullptr);
    LineTo(draw.hDC, item.right, item.top);
    SelectObject(draw.hDC, previous);
    DeleteObject(pen);
  }
  HFONT font = editor_font_ ? editor_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  HGDIOBJ previous = SelectObject(draw.hDC, font);
  SetBkMode(draw.hDC, TRANSPARENT);
  SetTextColor(draw.hDC, theme_foreground_);
  item.left += ScaleDip(window_, 10);
  item.right -= ScaleDip(window_, 10);
  DrawTextW(draw.hDC, status_text_.c_str(), static_cast<int>(status_text_.size()), &item,
            DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS);
  SelectObject(draw.hDC, previous);
}
void Application::LoadPanelLayout() {
  panel_layout_ = PanelLayout::Default();
  if (workspace_store_) {
    std::wstring error;
    if (!workspace_store_->ReadPanelLayout(panel_layout_, error)) {
      panel_layout_.Reset();
      SetStatusText(L"パネル配置を読み込めないため既定配置を使用します。" );
    }
  }
  if (const auto* explorer = panel_layout_.Find(PanelId::Explorer))
    workspace_pane_collapsed_ = explorer->collapsed;
  if (const auto* outline = panel_layout_.Find(PanelId::Outline))
    outline_pane_collapsed_ = outline->collapsed;
  LayoutControls();
}

void Application::SavePanelLayout() {
  if (!workspace_store_) return;
  std::wstring error;
  if (!workspace_store_->WritePanelLayout(panel_layout_, error))
    SetStatusText(L"パネル配置を保存できません。" );
}
void Application::MovePanelToSlot(PanelId id, PanelSlot slot) {
  std::wstring error;
  if (!panel_layout_.Move(id, slot, error)) {
    SetStatusText(error.empty() ? L"パネル配置を変更できません。" : error);
    return;
  }
  focused_panel_ = id;
  SavePanelLayout();
  LayoutControls();
  SetStatusText(std::wstring(PanelName(id)) + L"を" + PanelSlotName(slot) + L"へ移動しました。");
}

void Application::MoveFocusedPanelToSlot(PanelSlot slot) {
  MovePanelToSlot(focused_panel_, slot);
}

void Application::UpdatePanelHeaders() {
  for (const auto id : {PanelId::Explorer, PanelId::Calendar, PanelId::Outline, PanelId::Git}) {
    const auto* panel = panel_layout_.Find(id);
    const HWND header = panel_headers_[PanelIndex(id)];
    if (!panel || !header) continue;
    std::wstring text = PanelName(id);
    text += L"  [";
    text += PanelSlotName(panel->slot);
    text += L"]";
    if (focused_panel_ == id) text += L"  •";
    SetWindowTextW(header, text.c_str());
    if (editor_font_) SendMessageW(header, WM_SETFONT, reinterpret_cast<WPARAM>(editor_font_), TRUE);
  }
}
void Application::LayoutControls() {
  RECT client{};
  GetClientRect(window_, &client);
  SendMessageW(status_, WM_SIZE, 0, 0);
  RECT status_rect{};
  GetWindowRect(status_, &status_rect);
  const int status_height = status_rect.bottom - status_rect.top;
  const int content_height = std::max(0L, client.bottom - status_height);
  const bool find_visible = IsWindowVisible(find_bar_) != FALSE;
  const bool results_visible = IsWindowVisible(find_results_) != FALSE;
  const auto state_for_slot = [&](PanelSlot slot) -> const PanelState* {
    for (const auto& panel : panel_layout_.panels())
      if (panel.slot == slot) return &panel;
    return nullptr;
  };
  const auto is_visible = [&](const PanelState* panel) {
    if (!panel || panel->hidden || panel->collapsed) return false;
    if (panel->id == PanelId::Explorer && workspace_pane_collapsed_) return false;
    if (panel->id == PanelId::Outline && outline_pane_collapsed_) return false;
    return true;
  };
  const auto control_for = [&](PanelId id) {
    switch (id) {
      case PanelId::Explorer: return workspace_tree_;
      case PanelId::Calendar: return calendar_;
      case PanelId::Outline: return outline_;
      case PanelId::Git: return git_panel_;
    }
    return static_cast<HWND>(nullptr);
  };
  const auto header_for = [&](PanelId id) {
    return panel_headers_[PanelIndex(id)];
  };
  const auto left_top = state_for_slot(PanelSlot::LeftTop);
  const auto left_bottom = state_for_slot(PanelSlot::LeftBottom);
  const auto right_top = state_for_slot(PanelSlot::RightTop);
  const auto right_bottom = state_for_slot(PanelSlot::RightBottom);
  const bool show_left_top = is_visible(left_top);
  const bool show_left_bottom = is_visible(left_bottom);
  const bool show_right_top = is_visible(right_top);
  const bool show_right_bottom = is_visible(right_bottom);
  const auto preferred_width = [&](const PanelState* top, const PanelState* bottom, int fallback) {
    if (is_visible(top)) return ScaleDip(window_, static_cast<int>(top->width));
    if (is_visible(bottom)) return ScaleDip(window_, static_cast<int>(bottom->width));
    return ScaleDip(window_, fallback);
  };
  const int desired_tree_width = preferred_width(left_top, left_bottom, kTreeWidth);
  const int desired_outline_width = preferred_width(right_top, right_bottom, kOutlineWidth);
  const int minimum_editor_width = ScaleDip(window_, kMinimumEditorWidth);
  const int minimum_pane_width = ScaleDip(window_, kMinimumPaneWidth);
  const int pane_budget = std::max(0L, client.right - minimum_editor_width);
  int tree_width = 0;
  int outline_width = 0;
  const bool want_tree = show_left_top || show_left_bottom;
  const bool want_outline = show_right_top || show_right_bottom;
  if (want_tree && want_outline && pane_budget >= minimum_pane_width * 2) {
    const int desired_total = desired_tree_width + desired_outline_width;
    if (pane_budget >= desired_total) {
      tree_width = desired_tree_width;
      outline_width = desired_outline_width;
    } else {
      tree_width = std::max(minimum_pane_width,
                            MulDiv(pane_budget, desired_tree_width, std::max(1, desired_total)));
      outline_width = pane_budget - tree_width;
      if (outline_width < minimum_pane_width) {
        outline_width = minimum_pane_width;
        tree_width = pane_budget - outline_width;
      }
    }
  } else if (want_tree && pane_budget >= minimum_pane_width) {
    tree_width = std::min(desired_tree_width, pane_budget);
  } else if (want_outline && pane_budget >= minimum_pane_width) {
    outline_width = std::min(desired_outline_width, pane_budget);
  }
  if (tree_width == 0 && outline_width == 0 && pane_budget >= minimum_pane_width) {
    if (want_tree) tree_width = std::min(desired_tree_width, pane_budget);
    else if (want_outline) outline_width = std::min(desired_outline_width, pane_budget);
  }
  for (const auto& panel : panel_layout_.panels()) {
    ShowWindow(control_for(panel.id), SW_HIDE);
    ShowWindow(header_for(panel.id), SW_HIDE);
  }
  ShowWindow(calendar_details_, SW_HIDE);
  const int header_height = ScaleDip(window_, 26);
  const auto side_height = [&](const PanelState* top, const PanelState* bottom, bool top_visible,
                               bool bottom_visible) {
    if (!top_visible) return 0;
    if (!bottom_visible) return content_height;
    if (content_height <= header_height * 2 + 2) return content_height / 2;
    const int top_weight = std::max(1, static_cast<int>(top->height));
    const int bottom_weight = std::max(1, static_cast<int>(bottom->height));
    return std::clamp(MulDiv(content_height, top_weight, top_weight + bottom_weight),
                      header_height + 1, content_height - header_height - 1);
  };
  const int left_top_height = side_height(left_top, left_bottom, show_left_top, show_left_bottom);
  const int right_top_height = side_height(right_top, right_bottom, show_right_top, show_right_bottom);
  const auto place_panel = [&](const PanelState* panel, int x, int y, int width, int height) {
    if (!panel || !is_visible(panel) || width <= 0 || height <= 0) return;
    const HWND header = header_for(panel->id);
    const HWND control = control_for(panel->id);
    MoveWindow(header, x, y, width, std::min(header_height, height), TRUE);
    ShowWindow(header, SW_SHOW);
    const int content_top = y + std::min(header_height, height);
    const int panel_content_height = std::max(0, height - std::min(header_height, height));
    MoveWindow(control, x, content_top, width, panel_content_height, TRUE);
    ShowWindow(control, panel_content_height > 0 ? SW_SHOW : SW_HIDE);
    if (panel->id == PanelId::Calendar) {
      const int calendar_height = std::max(0, panel_content_height * 2 / 3);
      MoveWindow(calendar_, x, content_top, width, calendar_height, TRUE);
      MoveWindow(calendar_details_, x, content_top + calendar_height, width,
                 std::max(0, panel_content_height - calendar_height), TRUE);
      ShowWindow(calendar_, calendar_height > 0 ? SW_SHOW : SW_HIDE);
      ShowWindow(calendar_details_, calendar_height > 0 ? SW_SHOW : SW_HIDE);
    }
  };
  place_panel(left_top, 0, 0, tree_width, left_top_height);
  place_panel(left_bottom, 0, left_top_height, tree_width,
              std::max(0, content_height - left_top_height));
  place_panel(right_top, client.right - outline_width, 0, outline_width, right_top_height);
  place_panel(right_bottom, client.right - outline_width, right_top_height, outline_width,
              std::max(0, content_height - right_top_height));
  const int tab_height = ScaleDip(window_, kTabHeight);
  const int center_left = tree_width;
  const int center_width = std::max(0L, client.right - tree_width - outline_width);
  const int compact_find_width = ScaleDip(window_, kFindCompactWidth);
  const bool compact_find = center_width < compact_find_width;
  const int find_height = ScaleDip(window_, find_visible ? (compact_find ? 64 : kFindHeight) : 0);
  const int results_height = ScaleDip(window_, kFindResultsHeight);
  const int padding = ScaleDip(window_, 8);
  const int gap = ScaleDip(window_, 4);
  const int input_height = ScaleDip(window_, 24);
  const int button_height = ScaleDip(window_, 25);
  const int minimum_input_width = ScaleDip(window_, 80);
  const int base_button_width = ScaleDip(window_, 100);
  const bool show_advanced_find = find_visible && !compact_find;
  const bool show_workspace_actions = show_advanced_find && center_width >= ScaleDip(window_, 720);
  const bool show_globs = show_advanced_find && center_width >= ScaleDip(window_, 600);
  auto place = [](HWND control, int x, int y, int width, int height, bool visible) {
    if (!control) return;
    MoveWindow(control, x, y, std::max(0, width), std::max(0, height), TRUE);
    ShowWindow(control, visible && width > 0 && height > 0 ? SW_SHOW : SW_HIDE);
  };
  const int option_cluster = show_advanced_find ? ScaleDip(window_, 72 + 62 + 58 + 8) : 0;
  const int first_row_available = std::max(0, center_width - padding * 2);
  const int button_width = std::min(base_button_width,
                                    std::max(0, first_row_available - minimum_input_width -
                                                   option_cluster - gap * 2));
  const int input_width = std::max(0, first_row_available - button_width - option_cluster - gap * 2);
  MoveWindow(tabs_, center_left, 0, center_width, tab_height, TRUE);
  MoveWindow(find_bar_, center_left, tab_height, center_width, find_visible ? find_height : 0, TRUE);
  place(find_edit_, center_left + padding, tab_height + ScaleDip(window_, 4), input_width,
        input_height, find_visible);
  const int find_next_x = center_left + padding + input_width + gap;
  place(find_next_, find_next_x, tab_height + ScaleDip(window_, 3), button_width,
        button_height, find_visible);
  int option_x = find_next_x + button_width + gap;
  place(find_case_, option_x, tab_height + ScaleDip(window_, 5), ScaleDip(window_, 72),
        ScaleDip(window_, 22), show_advanced_find);
  option_x += ScaleDip(window_, 72) + gap;
  place(find_regex_, option_x, tab_height + ScaleDip(window_, 5), ScaleDip(window_, 62),
        ScaleDip(window_, 22), show_advanced_find);
  option_x += ScaleDip(window_, 62) + gap;
  place(find_word_, option_x, tab_height + ScaleDip(window_, 5), ScaleDip(window_, 58),
        ScaleDip(window_, 22), show_advanced_find);
  const int replacement_cluster = show_workspace_actions ?
      ScaleDip(window_, 82 + 92 + 118 + 12) : ScaleDip(window_, 82);
  const int replacement_width = std::max(0, first_row_available - replacement_cluster - gap);
  place(replace_edit_, center_left + padding, tab_height + ScaleDip(window_, 36), replacement_width,
        input_height, find_visible);
  int replace_x = center_left + padding + replacement_width + gap;
  place(replace_one_, replace_x, tab_height + ScaleDip(window_, 35), ScaleDip(window_, 82),
        button_height, find_visible);
  replace_x += ScaleDip(window_, 82) + gap;
  place(replace_document_, replace_x, tab_height + ScaleDip(window_, 35), ScaleDip(window_, 92),
        button_height, show_workspace_actions);
  replace_x += ScaleDip(window_, 92) + gap;
  place(replace_workspace_, replace_x, tab_height + ScaleDip(window_, 35), ScaleDip(window_, 118),
        button_height, show_workspace_actions);
  const int glob_button_width = show_workspace_actions ? base_button_width : 0;
  const int glob_width = show_globs ? std::max(0, (center_width - padding * 2 - glob_button_width - gap * 2) / 2) : 0;
  place(find_include_glob_, center_left + padding, tab_height + ScaleDip(window_, 66), glob_width,
        input_height, show_globs);
  place(find_exclude_glob_, center_left + padding + glob_width + gap,
        tab_height + ScaleDip(window_, 66), glob_width, input_height, show_globs);
  place(find_workspace_, center_left + center_width - padding - glob_button_width,
        tab_height + ScaleDip(window_, 65), glob_button_width, button_height,
        show_workspace_actions);
  const int results_top = tab_height + (find_visible ? find_height : 0);
  MoveWindow(find_results_, center_left, results_top, center_width,
             results_visible ? results_height : 0, TRUE);
  const int editor_top = results_top + (results_visible ? results_height : 0);
  for (auto& view : documents_) {
    if (GetParent(view->editor) == window_)
      MoveWindow(view->editor, center_left, editor_top, center_width,
                 std::max(0, content_height - editor_top), TRUE);
  }
  if (calendar_ && calendar_tooltip_) {
    TTTOOLINFOW tool{sizeof(tool)};
    tool.hwnd = calendar_;
    tool.uId = 1;
    GetClientRect(calendar_, &tool.rect);
    SendMessageW(calendar_tooltip_, TTM_NEWTOOLRECTW, 0, reinterpret_cast<LPARAM>(&tool));
  }
  UpdatePanelHeaders();
}
void Application::OpenWorkspaceDialog() {
  IFileDialog* dialog = nullptr;
  if (FAILED(CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER,
                              IID_PPV_ARGS(&dialog)))) return;
  DWORD options{};
  dialog->GetOptions(&options);
  dialog->SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM);
  if (SUCCEEDED(dialog->Show(window_))) {
    IShellItem* item = nullptr;
    if (SUCCEEDED(dialog->GetResult(&item))) {
      PWSTR path = nullptr;
      if (SUCCEEDED(item->GetDisplayName(SIGDN_FILESYSPATH, &path))) {
        OpenWorkspace(path);
        CoTaskMemFree(path);
      }
      item->Release();
    }
  }
  dialog->Release();
}

void Application::OpenFileDialog() {
  wchar_t path[32768]{};
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrFilter = L"Markdown・テキスト\0*.md;*.markdown;*.txt;*.log;*.json;*.toml;*.yaml;*.yml\0すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
  if (GetOpenFileNameW(&dialog)) OpenInitialPath(path);
}

void Application::NewUntitledDocument() {
  if (workspace_.empty()) {
    auto recovery_workspace = settings_.default_memo_workspace;
    std::error_code path_error;
    if (recovery_workspace.empty() || !std::filesystem::is_directory(recovery_workspace, path_error)) {
      MessageBoxW(window_,
                  L"無題文書の復旧先として使う既定のメモ用Workspaceを一度選択します。\n"
                  L"通常の本文ファイルは保存操作まで作成しません。",
                  L"新規無題文書", MB_ICONINFORMATION);
      const auto selected = PickFolder(window_, L"既定のメモ用Workspace", {});
      if (!selected) return;
      recovery_workspace = std::filesystem::weakly_canonical(*selected, path_error);
      if (path_error || !std::filesystem::is_directory(recovery_workspace)) return;
      SettingsLayer common;
      std::wstring settings_error;
      const auto common_path = CommonSettingsPath();
      if (!LoadSettingsLayer(common_path, common, settings_error)) {
        MessageBoxW(window_, settings_error.c_str(), L"既定のメモ用Workspace", MB_ICONERROR);
        return;
      }
      common.default_memo_workspace = recovery_workspace;
      if (!SaveSettingsLayer(common_path, common, settings_error)) {
        MessageBoxW(window_, settings_error.c_str(), L"既定のメモ用Workspace", MB_ICONERROR);
        return;
      }
      LoadAndApplySettings();
    }
    OpenWorkspace(recovery_workspace);
    if (workspace_.empty() || !workspace_store_) return;
  }
  Document document;
  const auto identity = workspace_store_->state_root() / L"untitled" /
      (std::to_wstring(GetCurrentProcessId()) + L"-" + std::to_wstring(GetTickCount64()) + L".md");
  document.CreateUntitled(identity);
  OpenDocumentView(std::move(document), L"無題");
  if (active_document_ < documents_.size()) {
    auto& view = *documents_[active_document_];
    view.recovery_due = GetTickCount64() + kRecoveryDelayMs;
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  }
}

void Application::QuickOpen() {
  if (workspace_.empty()) return;
  // Keep a bounded, ranked candidate set in memory.  Filtering happens in the
  // picker itself so typing never opens a second prompt or loses candidates
  // that were outside the initial recent-file slice.
  const auto initial = FindQuickOpenCandidates(workspace_, L"", recent_documents_, 200);
  std::vector<std::filesystem::path> candidates;
  std::vector<NativePickerItem> items;
  for (const auto& path : initial) {
    std::error_code error;
    const auto relative = std::filesystem::relative(path, workspace_, error);
    if (error) continue;
    candidates.push_back(path);
    items.push_back(NativePickerItem{relative.generic_wstring()});
  }
  if (items.empty()) return;
  std::size_t selected{};
  if (!RunNativePicker(window_, instance_, L"Quick Open",
                       L"ファイル名またはWorkspace相対pathで絞り込み、開く文書を選択してください。",
                       items, selected)) return;
  if (selected < candidates.size()) OpenDocument(candidates[selected]);
}

void Application::OpenWorkspace(const std::filesystem::path& path) {
  if (workspace_search_worker_.joinable()) workspace_search_worker_.request_stop();
  ++workspace_search_generation_;
  workspace_search_due_ = 0;
  workspace_search_started_ = false;
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error || !std::filesystem::is_directory(absolute)) {
    MessageBoxW(window_, L"Workspaceフォルダーを開けません。", L"MDLite", MB_ICONERROR);
    return;
  }
  if (workspace_ == absolute) return;
  if (!workspace_.empty()) {
    std::wstring launch_error;
    if (!LaunchMDLite(absolute, launch_error))
      MessageBoxW(window_, launch_error.c_str(), L"Workspaceを開けません", MB_ICONERROR);
    return;
  }
  HANDLE new_mutex = CreateMutexW(nullptr, FALSE, WorkspaceMutexName(absolute).c_str());
  if (new_mutex == nullptr || GetLastError() == ERROR_ALREADY_EXISTS) {
    if (new_mutex) CloseHandle(new_mutex);
    MessageBoxW(window_, L"このWorkspaceは別のMDLiteウィンドウで既に開かれています。",
                L"Workspace重複起動", MB_ICONWARNING);
    return;
  }
  if (workspace_mutex_) CloseHandle(workspace_mutex_);
  workspace_mutex_ = new_mutex;
  workspace_ = absolute;
  workspace_store_ = std::make_shared<WorkspaceStore>(workspace_);
  if (!std::filesystem::exists(workspace_ / L".mdlite")) {
    if (MessageBoxW(window_, L"このWorkspace用の設定・復旧領域 .mdlite を作成しますか？\n"
                             L"文書本文や復旧データをAppDataへ保存することはありません。",
                    L"MDLite Workspace", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON1) != IDYES) {
      workspace_store_.reset();
    }
  }
  if (workspace_store_) {
    std::wstring initialize_error;
    if (!workspace_store_->Initialize(initialize_error)) {
      MessageBoxW(window_, initialize_error.c_str(), L"Workspace設定", MB_ICONWARNING);
      workspace_store_.reset();
    } else {
      const auto recoveries = workspace_store_->RecoveryFiles();
      if (!recoveries.empty()) {
        const int recover = TestAutomationSilent() ? IDYES : MessageBoxW(window_,
            L"前回の復旧スナップショットがあります。\n"
            L"元文書への未保存編集として開きますか？",
            L"MDLite 復旧", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON1);
        if (recover == IDYES) {
          for (const auto& recovery : recoveries) OpenRecoverySnapshot(recovery);
        } else if (MessageBoxW(window_,
                   L"復旧スナップショットを明示的に破棄しますか？\n"
                   L"「いいえ」なら次回起動のために保持します。",
                   L"MDLite 復旧", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) == IDYES) {
          for (const auto& recovery : recoveries) {
            std::wstring discard_error;
            if (!workspace_store_->DiscardRecoverySnapshot(recovery, discard_error))
              MessageBoxW(window_, discard_error.c_str(), L"復旧スナップショット", MB_ICONWARNING);
          }
        }
      }
      SessionState session;
      if (!workspace_store_->ReadSessionState(session, initialize_error)) {
        MessageBoxW(window_, initialize_error.c_str(), L"セッション復元", MB_ICONWARNING);
      } else {
        recent_documents_ = session.recent_documents;
        PlaceOnVisibleMonitor(window_, session.main_x, session.main_y,
                              session.main_width, session.main_height);
        for (const auto& item : session.documents) OpenDocument(item.path);
        for (const auto& item : session.documents) {
          const auto found = std::ranges::find_if(documents_, [&](const auto& view) {
            return view->document.path() == item.path;
          });
          if (found == documents_.end()) continue;
          const auto index = static_cast<std::size_t>(std::distance(documents_.begin(), found));
          ActivateDocument(index);
          CHARRANGE selection{
              static_cast<LONG>((*found)->editor_snapshot.SourceToNative(item.selection_begin)),
              static_cast<LONG>((*found)->editor_snapshot.SourceToNative(item.selection_end))};
          SendMessageW((*found)->editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
          SendMessageW((*found)->editor, EM_LINESCROLL, 0, item.first_visible_line);
          if (item.compact) {
            ToggleCompactWindow();
            if ((*found)->compact_window)
              PlaceOnVisibleMonitor((*found)->compact_window, item.x, item.y, item.width, item.height);
          }
        }
        if (!documents_.empty()) ActivateDocument(std::min(session.active_index, documents_.size() - 1));
      }
    }
  }
  LoadAndApplySettings();
  LoadPanelLayout();
  SetWindowTextW(window_, (L"MDLite — " + workspace_.filename().wstring()).c_str());
  PopulateWorkspaceTree();
  UpdateStatus();
}

void Application::PopulateWorkspaceTree() {
  TreeView_DeleteAllItems(workspace_tree_);
  tree_paths_.clear();
  AddTreeDirectory(TVI_ROOT, workspace_, 0);
}

void Application::AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth) {
  if (depth > 32) return;
  std::vector<std::filesystem::directory_entry> entries;
  std::error_code error;
  for (std::filesystem::directory_iterator iterator(directory, std::filesystem::directory_options::skip_permission_denied, error), end;
       iterator != end && !error; iterator.increment(error)) entries.push_back(*iterator);
  std::ranges::sort(entries, [](const auto& a, const auto& b) {
    if (a.is_directory() != b.is_directory()) return a.is_directory() > b.is_directory();
    return a.path().filename().wstring() < b.path().filename().wstring();
  });
  for (const auto& entry : entries) {
    const auto name = entry.path().filename().wstring();
    if (name == L".git" || name == L"build" || name == L"out" ||
        (name == L".cache" && entry.path().parent_path().filename() == L".mdlite") ||
        (name == L".state" && entry.path().parent_path().filename() == L".mdlite")) continue;
    if (!entry.is_directory() && !IsTextFile(entry.path())) continue;
    tree_paths_.push_back(std::make_unique<std::filesystem::path>(entry.path()));
    TVINSERTSTRUCTW insert{};
    insert.hParent = parent;
    insert.hInsertAfter = TVI_SORT;
    insert.item.mask = TVIF_TEXT | TVIF_PARAM;
    insert.item.pszText = const_cast<wchar_t*>(name.c_str());
    insert.item.lParam = reinterpret_cast<LPARAM>(tree_paths_.back().get());
    HTREEITEM item = TreeView_InsertItem(workspace_tree_, &insert);
    if (entry.is_directory()) {
      TVINSERTSTRUCTW placeholder{};
      placeholder.hParent = item;
      placeholder.hInsertAfter = TVI_LAST;
      placeholder.item.mask = TVIF_TEXT | TVIF_PARAM;
      placeholder.item.pszText = const_cast<wchar_t*>(L"");
      placeholder.item.lParam = 0;
      TreeView_InsertItem(workspace_tree_, &placeholder);
    }
  }
}

void Application::OpenDocument(const std::filesystem::path& path) {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error || !IsTextFile(absolute)) return;
  if (!workspace_.empty() && !IsPathWithin(workspace_, absolute)) {
    std::wstring launch_error;
    if (!LaunchMDLite(absolute, launch_error))
      MessageBoxW(window_, launch_error.c_str(), L"ファイルを開けません", MB_ICONERROR);
    return;
  }
  recent_documents_.erase(std::remove_if(recent_documents_.begin(), recent_documents_.end(),
      [&](const auto& recent) { return _wcsicmp(recent.c_str(), absolute.c_str()) == 0; }),
      recent_documents_.end());
  recent_documents_.insert(recent_documents_.begin(), absolute);
  if (recent_documents_.size() > 20) recent_documents_.resize(20);
  for (std::size_t i = 0; i < documents_.size(); ++i) {
    if (documents_[i]->document.path() == absolute) {
      ActivateDocument(i);
      return;
    }
  }
  std::wstring load_error;
  Document document;
  if (!document.Load(absolute, load_error)) {
    MessageBoxW(window_, load_error.c_str(), L"ファイルを開けません", MB_ICONERROR);
    return;
  }
  OpenDocumentView(std::move(document), absolute.filename().wstring());
}

void Application::OpenDocumentView(Document document, std::wstring tab_name) {
  auto view = std::make_unique<DocumentView>();
  view->workspace_store = workspace_store_;
  view->document = std::move(document);
  view->editor_snapshot = SnapshotFor(view->document);
  view->editor = CreateWindowExW(WS_EX_CLIENTEDGE, MSFTEDIT_CLASS, nullptr,
                                  WS_CHILD | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE |
                                      ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL |
                                      ES_WANTRETURN,
                                  0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kEditor), instance_, nullptr);
  SendMessageW(view->editor, EM_SETEVENTMASK, 0,
               ENM_CHANGE | ENM_SELCHANGE | ENM_UPDATE | ENM_SCROLL | ENM_LINK);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0,
               ThemeColor(settings_, L"editor_background", dark ? RGB(28, 31, 36) : RGB(252, 253, 255)));
  SetWindowSubclass(view->editor, EditorSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    const auto native_snapshot = NativeSnapshotFor(view->document);
    SetWindowTextW(view->editor, native_snapshot.view.c_str());
  }

  TCITEMW tab{};
  tab.mask = TCIF_TEXT;
  tab.pszText = tab_name.data();
  TabCtrl_InsertItem(tabs_, static_cast<int>(documents_.size()), &tab);
  documents_.push_back(std::move(view));
  ActivateDocument(documents_.size() - 1);
}

void Application::OpenRecoverySnapshot(const std::filesystem::path& path) {
  if (!workspace_store_) return;
  RecoverySnapshot snapshot;
  std::wstring error;
  if (!workspace_store_->ReadRecoverySnapshot(path, snapshot, error)) {
    MessageBoxW(window_, (error + L"\nスナップショットは破棄していません。").c_str(),
                L"復旧できません", MB_ICONWARNING);
    return;
  }
  for (std::size_t index = 0; index < documents_.size(); ++index) {
    if (documents_[index]->document.path() != snapshot.source_path) continue;
    if (documents_[index]->document.dirty()) {
      MessageBoxW(window_, L"同じ元文書の未保存タブがすでに開いているため、復旧内容を重ねていません。",
                  L"復旧の競合", MB_ICONWARNING);
      return;
    }
    documents_[index]->document.MarkEdited(std::move(snapshot.text));
    documents_[index]->editor_snapshot = SnapshotFor(documents_[index]->document);
    documents_[index]->rendered_images.clear();
    documents_[index]->rendered_image_source.clear();
    documents_[index]->animated_image_frames.clear();
    documents_[index]->animation_due = 0;
    documents_[index]->image_asset_check_due = 0;
    documents_[index]->derived_image_revision = std::numeric_limits<std::uint64_t>::max();
    {
      ScopedEditorChangeSuppression suppression(suppress_editor_change_);
      const auto native_snapshot = NativeSnapshotFor(documents_[index]->document);
      SetWindowTextW(documents_[index]->editor, native_snapshot.view.c_str());
    }
    ActivateDocument(index);
    return;
  }
  Document document;
  if (std::filesystem::is_regular_file(snapshot.source_path)) {
    if (!document.Load(snapshot.source_path, error)) {
      MessageBoxW(window_, error.c_str(), L"復旧元を開けません", MB_ICONWARNING);
      return;
    }
  } else {
    document.CreateUntitled(snapshot.source_path);
  }
  document.MarkEdited(std::move(snapshot.text));
  OpenDocumentView(std::move(document), snapshot.source_path.filename().wstring() + L" (復旧)");
}

void Application::ActivateDocument(std::size_t index) {
  if (index >= documents_.size()) return;
  for (std::size_t i = 0; i < documents_.size(); ++i) {
    if (documents_[i]->compact_window == nullptr)
      ShowWindow(documents_[i]->editor, i == index ? SW_SHOW : SW_HIDE);
  }
  active_document_ = index;
  TabCtrl_SetCurSel(tabs_, static_cast<int>(index));
  ApplyMarkdownPresentation(*documents_[index], false);
  RebuildOutline(*documents_[index]);
  LayoutControls();
  SetFocus(documents_[index]->editor);
  UpdateStatus();
}

Application::DocumentView* Application::FindDocumentView(HWND editor) {
  const auto found = std::ranges::find_if(documents_, [editor](const auto& view) {
    return view->editor == editor;
  });
  return found == documents_.end() ? nullptr : found->get();
}

void Application::SelectDocumentForEditor(HWND editor) {
  for (std::size_t index = 0; index < documents_.size(); ++index) {
    if (documents_[index]->editor != editor || active_document_ == index) continue;
    active_document_ = index;
    TabCtrl_SetCurSel(tabs_, static_cast<int>(index));
    RebuildOutline(*documents_[index]);
    UpdateStatus();
    return;
  }
}

bool Application::CloseDocument(std::size_t index) {
  if (index >= documents_.size() || documents_[index]->ime_composing) return false;
  auto& view = *documents_[index];
  SyncDocumentFromEditor(view);
  if (view.document.dirty()) {
    const int answer = MessageBoxW(window_,
        (L"変更を保存してタブを閉じますか？\n" + view.document.path().wstring()).c_str(),
        L"タブを閉じる", MB_ICONQUESTION | MB_YESNOCANCEL | MB_DEFBUTTON1);
    if (answer == IDCANCEL) return false;
    if (answer == IDYES) {
      const auto save_result = SaveDocument(view, SaveIntent::UserRequested);
      if (save_result != SaveResult::Saved && save_result != SaveResult::NoChange) return false;
    }
    if (answer == IDNO && view.workspace_store) {
      std::wstring ignored;
      view.workspace_store->RemoveRecovery(view.document.path(), ignored);
    }
  }
  if (view.compact_window) {
    SetParent(view.editor, window_);
    DestroyWindow(view.compact_window);
    view.compact_window = nullptr;
  }
  DestroyWindow(view.editor);
  TabCtrl_DeleteItem(tabs_, static_cast<int>(index));
  documents_.erase(documents_.begin() + static_cast<std::ptrdiff_t>(index));
  if (documents_.empty()) {
    active_document_ = static_cast<std::size_t>(-1);
    TreeView_DeleteAllItems(outline_);
    UpdateStatus();
  } else {
    ActivateDocument(std::min(index, documents_.size() - 1));
  }
  SaveSession();
  return true;
}

Application::SaveResult Application::SaveDocument(DocumentView& view, SaveIntent intent) {
  if (view.ime_composing) return SaveResult::Failed;
  SyncDocumentFromEditor(view);
  if (!view.document.dirty()) return SaveResult::NoChange;
  if (view.document.untitled()) {
    if (intent == SaveIntent::BackgroundAutosave) {
      const bool recovered = SaveRecovery(view, false);
      UpdateStatus();
      return recovered ? SaveResult::RecoverySaved : SaveResult::Failed;
    }
    return SaveDocumentAs(view);
  }
  std::wstring error;
  if (!view.document.Save(error)) {
    const bool recovered = SaveRecovery(view, false);
    UpdateStatus();
    if (intent == SaveIntent::UserRequested) {
      if (recovered) error += L"\n最新の編集内容はWorkspaceの復旧領域へ保存しました。";
      else error += L"\n復旧領域への保存にも失敗しました。別名保存するか編集を続けてください。";
      MessageBoxW(window_, error.c_str(), L"保存できません", MB_ICONWARNING);
    }
    return recovered ? SaveResult::RecoverySaved : SaveResult::Failed;
  }
  if (view.workspace_store) {
    std::wstring recovery_error;
    view.workspace_store->RemoveRecovery(view.document.path(), recovery_error);
  }
  UpdateStatus();
  return SaveResult::Saved;
}

Application::SaveResult Application::SaveDocumentAs(DocumentView& view) {
  if (view.ime_composing || save_dialog_active_) return SaveResult::Cancelled;
  SyncDocumentFromEditor(view);
  wchar_t path[32768]{};
  std::wstring suggested = view.document.untitled()
      ? L"untitled.md"
      : view.document.path().stem().wstring() + L"-recovered" + view.document.path().extension().wstring();
  wcsncpy_s(path, suggested.c_str(), _TRUNCATE);
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrFilter = L"Markdown・テキスト\0*.md;*.markdown;*.txt;*.log;*.json;*.toml;*.yaml;*.yml\0すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  const std::wstring initial_directory = view.document.untitled() && !workspace_.empty()
      ? workspace_.wstring() : view.document.path().parent_path().wstring();
  dialog.lpstrInitialDir = initial_directory.c_str();
  dialog.Flags = OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
  save_dialog_active_ = true;
  const BOOL selected = GetSaveFileNameW(&dialog);
  save_dialog_active_ = false;
  if (!selected) return SaveResult::Cancelled;
  const auto old_path = view.document.path();
  std::wstring error;
  if (!view.document.SaveAs(path, error)) {
    MessageBoxW(window_, error.c_str(), L"別名保存できません", MB_ICONWARNING);
    return SaveResult::Failed;
  }
  if (view.workspace_store) {
    std::wstring ignored;
    view.workspace_store->RemoveRecovery(old_path, ignored);
  }
  for (std::size_t i = 0; i < documents_.size(); ++i) {
    if (documents_[i].get() != &view) continue;
    TCITEMW tab{TCIF_TEXT};
    std::wstring name = view.document.path().filename().wstring();
    tab.pszText = name.data();
    TabCtrl_SetItem(tabs_, static_cast<int>(i), &tab);
    break;
  }
  view.autosave_due = 0;
  view.recovery_due = 0;
  UpdateStatus();
  return SaveResult::Saved;
}

void Application::ReloadDocumentFromDisk() {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  auto& view = *documents_[active_document_];
  if (view.document.untitled()) {
    MessageBoxW(window_, L"無題文書には再読込みするディスク版がありません。", L"再読込み", MB_ICONINFORMATION);
    return;
  }
  SyncDocumentFromEditor(view);
  std::wstring prompt = L"ディスク上の内容を再読込みしますか？";
  if (view.document.dirty()) {
    prompt += L"\n\n未保存の編集内容は復旧領域へ保全してから、表示をディスク版へ置き換えます。";
  }
  if (MessageBoxW(window_, prompt.c_str(), L"再読込み", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  if (view.document.dirty() && !SaveRecovery(view, true)) return;
  Document replacement;
  std::wstring error;
  if (!replacement.Load(view.document.path(), error)) {
    MessageBoxW(window_, error.c_str(), L"再読込みできません", MB_ICONWARNING);
    return;
  }
  view.document = std::move(replacement);
  view.editor_snapshot = SnapshotFor(view.document);
  view.rendered_images.clear();
  view.rendered_image_source.clear();
  view.animated_image_frames.clear();
  view.animation_due = 0;
  view.image_asset_check_due = 0;
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    const auto native_snapshot = NativeSnapshotFor(view.document);
    SetWindowTextW(view.editor, native_snapshot.view.c_str());
  }
  ApplyMarkdownPresentation(view, true);
  RebuildOutline(view);
  UpdateStatus();
}

void Application::CompareDocumentWithDisk() {
  if (active_document_ >= documents_.size()) return;
  auto& view = *documents_[active_document_];
  if (view.document.untitled()) {
    MessageBoxW(window_, L"無題文書には比較するディスク版がありません。", L"外部変更の比較", MB_ICONINFORMATION);
    return;
  }
  SyncDocumentFromEditor(view);
  Document disk;
  std::wstring error;
  if (!disk.Load(view.document.path(), error)) {
    MessageBoxW(window_, error.c_str(), L"比較できません", MB_ICONWARNING);
    return;
  }
  const auto& editor = view.document.text();
  const auto& stored = disk.text();
  if (editor == stored) {
    MessageBoxW(window_, L"編集内容とディスク上の内容は一致しています。", L"外部変更の比較", MB_ICONINFORMATION);
    return;
  }
  std::size_t first{};
  while (first < editor.size() && first < stored.size() && editor[first] == stored[first]) ++first;
  const auto line_of = [](std::wstring_view text, std::size_t position) {
    return 1U + static_cast<unsigned>(std::count(text.begin(), text.begin() + std::min(position, text.size()), L'\n'));
  };
  const auto excerpt = [first](std::wstring_view text) {
    const auto begin = first > 80 ? first - 80 : 0;
    std::wstring value(text.substr(begin, std::min<std::size_t>(240, text.size() - begin)));
    std::ranges::replace(value, L'\r', L' ');
    return value;
  };
  const std::wstring message = L"最初の相違位置（UTF-16）: " + std::to_wstring(first) +
      L"\n編集側 line " + std::to_wstring(line_of(editor, first)) + L" / ディスク側 line " +
      std::to_wstring(line_of(stored, first)) + L"\n\n編集側:\n" + excerpt(editor) +
      L"\n\nディスク側:\n" + excerpt(stored) +
      L"\n\n編集内容を残す場合は別名保存、ディスク版を採用する場合は再読込みを使用してください。";
  MessageBoxW(window_, message.c_str(), L"外部変更の比較", MB_ICONINFORMATION);
}

bool Application::SaveAllRequired(bool interactive) {
  bool all_saved = true;
  for (auto& view : documents_) {
    const auto save_result = SaveDocument(*view,
        interactive ? SaveIntent::UserRequested : SaveIntent::BackgroundAutosave);
    if (save_result != SaveResult::Saved && save_result != SaveResult::NoChange) all_saved = false;
  }
  return all_saved;
}

Application::SaveAllResult Application::SaveAllForExit(bool interactive) {
  bool result = true;
  bool all_recovered = true;
  for (auto& view : documents_) {
    const auto save_result = SaveDocument(*view,
        interactive ? SaveIntent::UserRequested : SaveIntent::BackgroundAutosave);
    if (save_result != SaveResult::Saved && save_result != SaveResult::NoChange) {
      result = false;
      all_recovered = (save_result == SaveResult::RecoverySaved ||
                       SaveRecovery(*view, false)) && all_recovered;
    }
  }
  if (!result && interactive) {
    if (all_recovered) {
      return MessageBoxW(window_,
                         L"保存できない文書があります。最新内容は復旧領域に保存済みです。\n"
                         L"復旧可能な状態で終了しますか？",
                         L"MDLite", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) == IDYES
                 ? SaveAllResult::RecoveryOnly
                 : SaveAllResult::Cancelled;
    }
    const int choice = MessageBoxW(
        window_, L"文書保存と復旧保存の両方に失敗しました。\n"
                 L"[はい] 別名保存  [いいえ] 編集内容を明示破棄して終了  [キャンセル] 編集へ戻る",
        L"本文保護", MB_ICONERROR | MB_YESNOCANCEL | MB_DEFBUTTON3);
    if (choice == IDCANCEL) return SaveAllResult::Cancelled;
    if (choice == IDNO) return SaveAllResult::Discarded;
    for (auto& view : documents_) {
      if (view->document.dirty() && SaveDocumentAs(*view) != SaveResult::Saved)
        return SaveAllResult::Cancelled;
    }
    return SaveAllResult::AllSaved;
  }
  return result ? SaveAllResult::AllSaved : SaveAllResult::Cancelled;
}

void Application::OnEditorChanged(HWND editor) {
  if (suppress_editor_change_) return;
  for (auto& view : documents_) {
    if (view->editor != editor) continue;
    const ULONGLONG now = GetTickCount64();
    view->native_edit_pending = true;
    InvalidateTableGrid(*view);
    if (view->ime_composing) {
      UpdateStatus();
      return;
    }
    // EN_CHANGE records a pending native edit. Normal WM_CHAR/command dispatch
    // commits it immediately; the timer remains a safe fallback for messages
    // whose mutation boundary is not classified.
    view->sync_due = now + kEditorSyncDelayMs;
    view->presentation_due = 0;
    view->autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
    if (view->recovery_due == 0) view->recovery_due = now + kRecoveryDelayMs;
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
    UpdateStatus();
    break;
  }
}

void Application::CommitPendingNativeEdit(DocumentView& view) {
  if (!view.native_edit_pending && view.sync_due == 0) return;
  if (view.sync_due == 0) {
    const ULONGLONG now = GetTickCount64();
    view.sync_due = now;
    view.autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
    if (view.recovery_due == 0) view.recovery_due = now + kRecoveryDelayMs;
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  }
  SyncDocumentFromEditor(view);
  SchedulePresentation(view);
}

void Application::SchedulePresentation(DocumentView& view) {
  if (view.ime_composing || !IsMarkdownFile(view.document.path())) return;
  view.presentation_due = GetTickCount64() + kPresentationDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
}

void Application::SyncDocumentFromEditor(DocumentView& view) {
  // EN_CHANGE is the authority for pending native-editor input. Presentation
  // updates are performed with notifications suppressed and must never be read
  // back as Markdown source (RichEdit may expose an image object as a space).
  if (view.sync_due == 0 && !view.native_edit_pending) return;
  view.sync_due = 0;
  const auto native_snapshot = NativeSnapshotFor(view.document);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_begin = native_snapshot.NativeToSource(selection.cpMin);
  const std::size_t source_end = native_snapshot.NativeToSource(selection.cpMax);
  std::wstring current_view;
  if (!EditorText(view.editor, native_snapshot, current_view)) {
    // Never infer a source edit from a truncated/misaligned native readback.
    // Keep the pending input alive for a later timer pass without showing a
    // dialog or turning a presentation-only failure into data loss.
    view.native_edit_pending = true;
    view.sync_due = GetTickCount64() + kEditorSyncDelayMs;
    return;
  }
  const auto transaction = ApplyEditorText(native_snapshot, view.document.text(), current_view);
  if (!transaction.changed) {
    view.native_edit_pending = false;
    return;
  }
  InvalidateTableGrid(view);
  view.source_undo.push_back({transaction.begin,
      view.document.text().substr(transaction.begin, transaction.old_end - transaction.begin),
      transaction.source.substr(transaction.begin, transaction.new_end - transaction.begin)});
  if (view.source_undo.size() > 100) view.source_undo.erase(view.source_undo.begin());
  view.source_redo.clear();
  view.document.MarkEdited(transaction.source);
  view.editor_snapshot = SnapshotFor(view.document);
  view.native_edit_pending = false;
  const auto target_native = NativeSnapshotFor(view.document);
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    PresentationUndoGuard undo_guard(view.editor);
    if (!undo_guard) return;
    if (current_view != target_native.view) {
      std::size_t view_prefix{};
      while (view_prefix < current_view.size() &&
             view_prefix < target_native.view.size() &&
             current_view[view_prefix] == target_native.view[view_prefix]) {
        ++view_prefix;
      }
      std::size_t current_suffix = current_view.size();
      std::size_t target_suffix = target_native.view.size();
      while (current_suffix > view_prefix && target_suffix > view_prefix &&
             current_view[current_suffix - 1] == target_native.view[target_suffix - 1]) {
        --current_suffix;
        --target_suffix;
      }
      const std::wstring replacement = target_native.view.substr(
          view_prefix, target_suffix - view_prefix);
      SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
      SendMessageW(view.editor, EM_SETSEL, view_prefix, current_suffix);
      SendMessageW(view.editor, EM_REPLACESEL, FALSE,
                   reinterpret_cast<LPARAM>(replacement.c_str()));
      const CHARRANGE restored{static_cast<LONG>(target_native.SourceToNative(source_begin)),
                               static_cast<LONG>(target_native.SourceToNative(source_end))};
      SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&restored));
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, FALSE);
    }
    // Source history is authoritative.  The native control's typing undo can
    // no longer reach that pre-transaction state, so discard it explicitly.
    SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
  }
  // The timer schedules presentation after the source transaction settles.
  // Parsing or decoding here would move the deferred work back onto commands
  // that only need the Markdown source (Save, Find, table navigation).
}

void Application::ApplySourceTextWithUndo(DocumentView& view, std::wstring text, bool record_history) {
  if (text == view.document.text()) return;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_begin = view.editor_snapshot.NativeToSource(selection.cpMin);
  const std::size_t source_end = view.editor_snapshot.NativeToSource(selection.cpMax);
  const auto target_native = NativeSnapshotFor(view.document.path(), text);
  InvalidateTableGrid(view);
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    PresentationUndoGuard undo_guard(view.editor);
    if (!undo_guard) return;
    if (record_history) {
      const auto& before = view.document.text();
      std::size_t prefix{};
      while (prefix < before.size() && prefix < text.size() && before[prefix] == text[prefix]) ++prefix;
      std::size_t before_suffix = before.size();
      std::size_t after_suffix = text.size();
      while (before_suffix > prefix && after_suffix > prefix &&
             before[before_suffix - 1] == text[after_suffix - 1]) {
        --before_suffix;
        --after_suffix;
      }
      view.source_undo.push_back({prefix, before.substr(prefix, before_suffix - prefix),
                                 text.substr(prefix, after_suffix - prefix)});
      if (view.source_undo.size() > 100) view.source_undo.erase(view.source_undo.begin());
      view.source_redo.clear();
    }
    view.document.MarkEdited(std::move(text));
    SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
    SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
    SendMessageW(view.editor, EM_SETSEL, 0, -1);
    SendMessageW(view.editor, EM_REPLACESEL, FALSE,
                 reinterpret_cast<LPARAM>(target_native.view.c_str()));
    SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
    view.editor_snapshot = SnapshotFor(view.document);
    const CHARRANGE restored{
        static_cast<LONG>(target_native.SourceToNative(std::min(source_begin, view.document.text().size()))),
        static_cast<LONG>(target_native.SourceToNative(std::min(source_end, view.document.text().size())))};
    SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&restored));
    SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(view.editor, nullptr, TRUE);
    SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
  }
  view.rendered_images.clear();
  view.rendered_image_source.clear();
  view.animated_image_frames.clear();
  view.animation_due = 0;
  view.image_asset_check_due = 0;
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  view.parse = IsMarkdownFile(view.document.path()) ? ParseMarkdown(view.document.text())
                                                    : MarkdownParseResult{};
  if (IsMarkdownFile(view.document.path())) ApplyMarkdownPresentation(view, true);
  if (active_document_ < documents_.size() && documents_[active_document_].get() == &view)
    RebuildOutline(view);
  const ULONGLONG now = GetTickCount64();
  view.autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
  view.recovery_due = now + kRecoveryDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  UpdateStatus();
}

bool Application::ApplySourceHistory(DocumentView& view, bool redo) {
  if (view.sync_due != 0 || view.native_edit_pending) SyncDocumentFromEditor(view);
  auto& source = redo ? view.source_redo : view.source_undo;
  auto& destination = redo ? view.source_undo : view.source_redo;
  if (source.empty()) return false;
  const auto edit = std::move(source.back());
  source.pop_back();
  const auto& expected = redo ? edit.before : edit.after;
  const auto& replacement = redo ? edit.after : edit.before;
  std::wstring text = view.document.text();
  if (edit.begin > text.size() || expected.size() > text.size() - edit.begin ||
      text.compare(edit.begin, expected.size(), expected) != 0) {
    source.push_back(edit);
    return false;
  }
  text.replace(edit.begin, expected.size(), replacement);
  destination.push_back(edit);
  ApplySourceTextWithUndo(view, std::move(text), false);
  return true;
}

bool Application::ReadImageFileIdentity(const std::filesystem::path& path,
                                        DocumentView::ImageFileIdentity& identity) {
  identity = {};
  WIN32_FILE_ATTRIBUTE_DATA attributes{};
  if (!GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &attributes) ||
      (attributes.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) != 0) {
    return false;
  }
  identity.available = true;
  identity.size = (static_cast<std::uint64_t>(attributes.nFileSizeHigh) << 32U) |
                  attributes.nFileSizeLow;
  identity.write_time = FileTimeValue(attributes.ftLastWriteTime);
  return true;
}

void Application::RefreshDerivedImages(DocumentView& view) {
  if (!IsMarkdownFile(view.document.path())) return;
  const ULONGLONG now = GetTickCount64();
  const bool source_unchanged = view.rendered_image_source == view.document.text();
  const bool check_assets = view.image_asset_check_due == 0 || now >= view.image_asset_check_due;
  if (view.derived_image_revision == view.document.revision() && source_unchanged && !check_assets) return;
  const auto& parsed = view.parse;
  std::vector<const ImageReference*> renderable_images;
  renderable_images.reserve(parsed.images.size());
  for (const auto& image : parsed.images) {
    // Only replace markup that the editor snapshot intentionally collapsed to
    // one U+FFFC marker. Ambiguous same-line image groups remain literal
    // Markdown; replacing their leading character would corrupt the source
    // reconstructed from the RichEdit surface.
    if (view.editor_snapshot.HasCollapsedSourceRange(image.begin, image.end)) {
      renderable_images.push_back(&image);
    }
  }
  if (renderable_images.empty()) {
    view.animated_image_frames.clear();
    view.animation_due = 0;
    view.rendered_images.clear();
    view.rendered_image_source = view.document.text();
    view.image_asset_check_due = 0;
    view.derived_image_revision = view.document.revision();
    return;
  }

  // The source-to-view edit path preserves untouched embedded objects.  Match
  // cached images only when their complete markup, resolved target and cheap
  // on-disk identity are unchanged.  The common prefix/suffix mapping keeps
  // an object reusable when ordinary typing shifts its source range.
  const std::wstring& previous_source = view.rendered_image_source;
  std::size_t prefix{};
  while (prefix < previous_source.size() && prefix < view.document.text().size() &&
         previous_source[prefix] == view.document.text()[prefix]) {
    ++prefix;
  }
  std::size_t previous_suffix = previous_source.size();
  std::size_t current_suffix = view.document.text().size();
  while (previous_suffix > prefix && current_suffix > prefix &&
         previous_source[previous_suffix - 1] == view.document.text()[current_suffix - 1]) {
    --previous_suffix;
    --current_suffix;
  }
  const auto previous_images = std::move(view.rendered_images);
  const auto previous_frames = std::move(view.animated_image_frames);
  std::vector<bool> previous_used(previous_images.size());
  view.rendered_images.clear();
  view.rendered_images.reserve(renderable_images.size());
  view.animated_image_frames.clear();
  std::vector<bool> render_needed;
  render_needed.reserve(renderable_images.size());
  std::vector<std::size_t> matched_previous;
  matched_previous.reserve(renderable_images.size());

  const auto maps_unchanged_range = [&](const DocumentView::RenderedImage& previous,
                                        const ImageReference& current) {
    if (previous_source == view.document.text()) {
      return previous.source_begin == current.begin && previous.source_end == current.end;
    }
    if (previous.source_end <= prefix) {
      return previous.source_begin == current.begin && previous.source_end == current.end;
    }
    if (previous.source_begin >= previous_suffix) {
      const auto offset = static_cast<std::ptrdiff_t>(current_suffix) -
                          static_cast<std::ptrdiff_t>(previous_suffix);
      return static_cast<std::ptrdiff_t>(previous.source_begin) + offset ==
                 static_cast<std::ptrdiff_t>(current.begin) &&
             static_cast<std::ptrdiff_t>(previous.source_end) + offset ==
                 static_cast<std::ptrdiff_t>(current.end);
    }
    return false;
  };

  for (const auto* image_pointer : renderable_images) {
    const auto& image = *image_pointer;
    DocumentView::RenderedImage current{};
    current.source_begin = image.begin;
    current.source_end = image.end;
    current.markup = view.document.text().substr(image.begin, image.end - image.begin);
    current.target_text = image.target;
    current.alternate_text = image.alternate_text;
    current.width_dip = image.width_dip;
    std::filesystem::path target(image.target);
    if (!target.is_absolute() && image.target.find(L"://") == std::wstring::npos) {
      current.target = (view.document.path().parent_path() / target).lexically_normal();
      ReadImageFileIdentity(current.target, current.file_identity);
    }

    std::size_t matched = previous_images.size();
    for (std::size_t index = 0; index < previous_images.size(); ++index) {
      const auto& previous = previous_images[index];
      if (previous_used[index] || !maps_unchanged_range(previous, image) ||
          previous.markup != current.markup || previous.target_text != current.target_text ||
          previous.alternate_text != current.alternate_text || previous.width_dip != current.width_dip ||
          previous.target != current.target) {
        continue;
      }
      matched = index;
      previous_used[index] = true;
      break;
    }

    const bool can_reuse = matched != previous_images.size() &&
                           previous_images[matched].file_identity == current.file_identity;
    if (can_reuse) {
      const auto& previous = previous_images[matched];
      current.raster_width = previous.raster_width;
      current.raster_height = previous.raster_height;
      current.frame_count = previous.frame_count;
      current.animated = previous.animated;
      current.inserted = previous.inserted;
      if (const auto frame = previous_frames.find(previous.source_begin);
          frame != previous_frames.end()) {
        view.animated_image_frames.emplace(current.source_begin, frame->second);
      }
    }
    view.rendered_images.push_back(std::move(current));
    render_needed.push_back(!can_reuse);
    matched_previous.push_back(matched);
  }

  bool needs_editor_update = std::ranges::any_of(render_needed, [](bool needed) { return needed; });
  if (needs_editor_update) {
    CHARRANGE selection{};
    SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    PresentationUndoGuard undo_guard(view.editor);
    if (!undo_guard) {
      view.rendered_images = previous_images;
      view.animated_image_frames = previous_frames;
      return;
    }
    for (std::size_t index = view.rendered_images.size(); index-- > 0;) {
      if (!render_needed[index]) continue;
      auto& current = view.rendered_images[index];
      const auto& image = *renderable_images[index];
  const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToNative(image.begin));
      const std::size_t previous_index = matched_previous[index];
      const bool replaces_existing = previous_index < previous_images.size();
      if (!current.file_identity.available) {
        if (replaces_existing && previous_images[previous_index].inserted) {
          SendMessageW(view.editor, EM_SETSEL, position, position + 1);
          SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
        }
        continue;
      }
      bool safe{};
      std::wstring safety_message;
      std::wstring safety_error;
      if (!InspectImageSafety(current.target, safe, safety_message, safety_error) || !safe) {
        if (replaces_existing && previous_images[previous_index].inserted) {
          SendMessageW(view.editor, EM_SETSEL, position, position + 1);
          SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
        }
        continue;
      }
      RasterImageInfo image_info;
      std::wstring image_error;
      const bool has_image_info = ReadRasterImageInfo(current.target, image_info, image_error);
      IStream* stream = nullptr;
      unsigned frame_delay = 100;
      std::wstring extension = current.target.extension().wstring();
      std::ranges::transform(extension, extension.begin(), towlower);
      const bool requires_bundled_decode = extension == L".webp" || extension == L".svg";
      if (requires_bundled_decode || (has_image_info && image_info.animated)) {
        if (!CreateRasterFramePngStream(current.target, 0, stream, frame_delay, image_error)) continue;
      } else if (FAILED(SHCreateStreamOnFileEx(current.target.c_str(), STGM_READ | STGM_SHARE_DENY_WRITE,
                                                FILE_ATTRIBUTE_NORMAL, FALSE, nullptr, &stream))) {
        continue;
      }
      DocumentView::ImageFileIdentity opened_identity;
      if (!ReadImageFileIdentity(current.target, opened_identity) ||
          opened_identity != current.file_identity) {
        stream->Release();
        view.image_asset_check_due = 0;
        continue;
      }
      SendMessageW(view.editor, EM_SETSEL, position, position + 1);
      SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L""));
      const unsigned width = std::clamp(image.width_dip == 0 ? 320U : image.width_dip, 16U, 8192U);
      unsigned height = width * 9U / 16U;
      if (has_image_info && image_info.width != 0) {
        const auto scaled = static_cast<unsigned long long>(width) * image_info.height / image_info.width;
        height = static_cast<unsigned>(std::clamp<unsigned long long>(scaled, 16, 8192));
      }
      RICHEDIT_IMAGE_PARAMETERS parameters{};
      parameters.xWidth = static_cast<LONG>(static_cast<std::uint64_t>(width) * 2540U / 96U);
      parameters.yHeight = static_cast<LONG>(static_cast<std::uint64_t>(height) * 2540U / 96U);
      parameters.Ascent = parameters.yHeight;
      parameters.Type = TA_BASELINE;
      parameters.pwszAlternateText = image.alternate_text.c_str();
      parameters.pIStream = stream;
      const auto inserted = static_cast<HRESULT>(
          SendMessageW(view.editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters)));
      current.inserted = SUCCEEDED(inserted);
      current.raster_width = has_image_info ? image_info.width : 0;
      current.raster_height = has_image_info ? image_info.height : 0;
      current.frame_count = has_image_info ? image_info.frame_count : 0;
      current.animated = has_image_info && image_info.animated && SUCCEEDED(inserted);
      if (FAILED(inserted)) {
        SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
      } else if (current.animated) {
        view.animated_image_frames[current.source_begin] = {0, now + frame_delay};
      }
      stream->Release();
    }
    SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  }

  ULONGLONG next_due = std::numeric_limits<ULONGLONG>::max();
  for (const auto& [unused, frame] : view.animated_image_frames) {
    next_due = std::min(next_due, frame.due);
  }
  view.derived_image_revision = view.document.revision();
  view.rendered_image_source = view.document.text();
  view.image_asset_check_due = now + 1000;
  if (!view.animated_image_frames.empty()) {
    view.animation_due = next_due;
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  } else {
    view.animation_due = 0;
  }
}

void Application::AdvanceAnimatedImages(DocumentView& view, ULONGLONG now) {
  if (view.animation_due == 0 || now < view.animation_due || !IsWindowVisible(view.editor)) return;
  const auto& parsed = view.parse;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  RECT client{};
  GetClientRect(view.editor, &client);
  ULONGLONG next_due = std::numeric_limits<ULONGLONG>::max();
  bool kept_animation{};
  bool changed{};
  ScopedEditorChangeSuppression suppression(suppress_editor_change_);
  PresentationUndoGuard undo_guard(view.editor);
  if (!undo_guard) return;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  for (const auto& image : parsed.images) {
    auto frame = view.animated_image_frames.find(image.begin);
    if (frame == view.animated_image_frames.end()) continue;
    const auto cached = std::ranges::find_if(view.rendered_images, [&](const auto& item) {
      return item.source_begin == image.begin;
    });
    if (cached == view.rendered_images.end() || !cached->inserted || !cached->animated ||
        cached->frame_count < 2 || cached->raster_width == 0) {
      continue;
    }
    kept_animation = true;
    if (now < frame->second.due) {
      next_due = std::min(next_due, frame->second.due);
      continue;
    }
  const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToNative(image.begin));
    POINT point{};
    SendMessageW(view.editor, EM_POSFROMCHAR, reinterpret_cast<WPARAM>(&point), position);
    if (point.y < client.top || point.y >= client.bottom ||
        point.x < client.left || point.x >= client.right) continue;
    const unsigned next_frame = (frame->second.frame + 1) % cached->frame_count;
    IStream* stream{};
    unsigned frame_delay{};
    std::wstring image_error;
    if (!CreateRasterFramePngStream(cached->target, next_frame, stream, frame_delay, image_error)) {
      // The file may have been replaced between the periodic metadata check
      // and this frame tick.  Revalidate it before trying another frame.
      view.image_asset_check_due = 0;
      continue;
    }
    const unsigned width = std::clamp(image.width_dip == 0 ? 320U : image.width_dip, 16U, 8192U);
    const auto scaled = static_cast<unsigned long long>(width) * cached->raster_height /
                        cached->raster_width;
    const unsigned height = static_cast<unsigned>(std::clamp<unsigned long long>(scaled, 16, 8192));
    SendMessageW(view.editor, EM_SETSEL, position, position + 1);
    SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L""));
    RICHEDIT_IMAGE_PARAMETERS parameters{};
    parameters.xWidth = static_cast<LONG>(static_cast<std::uint64_t>(width) * 2540U / 96U);
    parameters.yHeight = static_cast<LONG>(static_cast<std::uint64_t>(height) * 2540U / 96U);
    parameters.Ascent = parameters.yHeight;
    parameters.Type = TA_BASELINE;
    parameters.pwszAlternateText = image.alternate_text.c_str();
    parameters.pIStream = stream;
    const auto inserted = static_cast<HRESULT>(
        SendMessageW(view.editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters)));
    if (FAILED(inserted))
      SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
    else
      frame->second.frame = next_frame;
    stream->Release();
    frame->second.due = now + (FAILED(inserted) ? kTimerPollMs : frame_delay);
    next_due = std::min(next_due, frame->second.due);
    changed = true;
  }
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  if (changed) InvalidateRect(view.editor, nullptr, FALSE);
  if (!kept_animation) {
    view.animated_image_frames.clear();
    view.animation_due = 0;
  } else {
    view.animation_due = next_due == std::numeric_limits<ULONGLONG>::max()
        ? now + kTimerPollMs : next_due;
  }
}

void Application::ApplyMarkdownPresentation(DocumentView& view, bool force) {
  if (!IsMarkdownFile(view.document.path())) return;
  if (view.ime_composing) return;
  if (view.sync_due != 0) return;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const int active_line = static_cast<int>(SendMessageW(view.editor, EM_LINEFROMCHAR, selection.cpMin, 0));
  if (!force && view.active_line == active_line) return;
  view.active_line = active_line;
  InvalidateTableGrid(view);
  view.parse = ParseMarkdown(view.document.text());
  RefreshDerivedImages(view);

  ScopedEditorChangeSuppression suppression(suppress_editor_change_);
  PresentationUndoGuard undo_guard(view.editor);
  if (!undo_guard) return;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  const LONG length = GetWindowTextLengthW(view.editor);
  SendMessageW(view.editor, EM_SETSEL, 0, length);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  const COLORREF foreground = ThemeColor(settings_, L"foreground", dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
  const COLORREF background = theme_editor_;
  CHARFORMAT2W normal{sizeof(normal)};
  normal.dwMask = CFM_FACE | CFM_SIZE | CFM_COLOR | CFM_BOLD | CFM_ITALIC | CFM_STRIKEOUT |
                  CFM_UNDERLINE | CFM_EFFECTS | CFM_HIDDEN | CFM_BACKCOLOR | CFM_LINK;
  normal.dwEffects = 0;
  normal.crTextColor = foreground;
  normal.crBackColor = background;
  normal.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
  wcsncpy_s(normal.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
  SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&normal));
  PARAFORMAT2 base_paragraph{sizeof(base_paragraph)};
  base_paragraph.dwMask = PFM_SPACEBEFORE | PFM_SPACEAFTER | PFM_LINESPACING | PFM_BORDER;
  base_paragraph.dySpaceBefore = 0;
  base_paragraph.dySpaceAfter = 0;
  base_paragraph.bLineSpacingRule = 0;
  base_paragraph.wBorders = 0;
  base_paragraph.wBorderWidth = 0;
  base_paragraph.wBorderSpace = 0;
  SendMessageW(view.editor, EM_SETPARAFORMAT, 0,
               reinterpret_cast<LPARAM>(&base_paragraph));

  const LONG active_start = static_cast<LONG>(SendMessageW(view.editor, EM_LINEINDEX, active_line, 0));
  const LONG active_length = static_cast<LONG>(SendMessageW(view.editor, EM_LINELENGTH, active_start, 0));
  const LONG active_end = active_start + active_length;
  for (const auto& span : view.parse.spans) {
  const auto view_begin = view.editor_snapshot.SourceToNative(span.begin);
  const auto view_end = view.editor_snapshot.SourceToNative(span.end);
    if (view_begin >= static_cast<std::size_t>(length) || view_end <= view_begin) continue;
    SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(view_begin), static_cast<LPARAM>(view_end));
    CHARFORMAT2W format{sizeof(format)};
    format.dwMask = CFM_COLOR;
    format.crTextColor = ThemeColor(settings_, L"heading", dark ? RGB(128, 190, 238) : RGB(38, 90, 140));
    if (span.kind == SpanKind::Heading) {
      format.dwMask |= CFM_SIZE | CFM_BOLD;
      format.dwEffects |= CFE_BOLD;
      const LONG base = static_cast<LONG>(settings_.font_size_pt * 20);
      format.yHeight = std::max(base + 40, base * (190 - std::min(span.level, 6) * 10) / 100);
    } else if (span.kind == SpanKind::Strong) {
      format.dwMask |= CFM_BOLD;
      format.dwEffects |= CFE_BOLD;
    } else if (span.kind == SpanKind::Emphasis) {
      format.dwMask |= CFM_ITALIC;
      format.dwEffects |= CFE_ITALIC;
    } else if (span.kind == SpanKind::Strike) {
      format.dwMask |= CFM_STRIKEOUT;
      format.dwEffects |= CFE_STRIKEOUT;
    } else if (span.kind == SpanKind::Code || span.kind == SpanKind::CodeFence) {
      format.dwMask |= CFM_FACE | CFM_BACKCOLOR;
      wcscpy_s(format.szFaceName, L"Cascadia Mono");
      format.crBackColor = ThemeColor(settings_, L"code_background", dark ? RGB(45, 45, 45) : RGB(242, 242, 242));
    } else if (span.kind == SpanKind::Link) {
      format.dwMask |= CFM_LINK | CFM_UNDERLINE;
      format.dwEffects |= CFE_LINK | CFE_UNDERLINE;
      format.crTextColor = ThemeColor(settings_, L"link", dark ? RGB(78, 160, 255) : RGB(0, 102, 204));
    } else if (span.kind == SpanKind::HeadingMarker || span.kind == SpanKind::EmphasisMarker) {
      const bool intersects_active = static_cast<LONG>(view_end) >= active_start &&
                                     static_cast<LONG>(view_begin) <= active_end;
      if (!intersects_active) {
        format.dwMask |= CFM_HIDDEN;
        format.dwEffects |= CFE_HIDDEN;
      } else {
        format.crTextColor = ThemeColor(settings_, L"marker", dark ? RGB(150, 150, 150) : RGB(128, 128, 128));
      }
    }
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
  }
  for (const auto& table : view.parse.tables) {
  const LONG begin = static_cast<LONG>(view.editor_snapshot.SourceToNative(table.begin));
  const LONG end = static_cast<LONG>(view.editor_snapshot.SourceToNative(table.end));
    SendMessageW(view.editor, EM_SETSEL, begin, end);
    CHARFORMAT2W table_format{sizeof(table_format)};
    table_format.dwMask = CFM_FACE | CFM_BACKCOLOR;
    wcscpy_s(table_format.szFaceName, L"Cascadia Mono");
    table_format.crBackColor = ThemeColor(settings_, L"table_background", dark ? RGB(38, 42, 48) : RGB(244, 247, 250));
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&table_format));
    PARAFORMAT2 paragraph{sizeof(paragraph)};
    paragraph.dwMask = PFM_SPACEBEFORE | PFM_SPACEAFTER | PFM_LINESPACING;
    paragraph.dySpaceBefore = 40;
    paragraph.dySpaceAfter = 40;
    paragraph.bLineSpacingRule = 0;
    SendMessageW(view.editor, EM_SETPARAFORMAT, 0, reinterpret_cast<LPARAM>(&paragraph));
  }
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  InvalidateRect(view.editor, nullptr, TRUE);
}
void Application::InvalidateTableGrid(DocumentView& view) {
  if (!view.editor) return;
  RECT client{};
  if (!GetClientRect(view.editor, &client) || client.right <= client.left ||
      client.bottom <= client.top) return;
  const auto geometry = BuildTableGridGeometry(view, client);
  if (geometry.empty()) {
    InvalidateRect(view.editor, nullptr, TRUE);
    return;
  }
  for (const auto& table : geometry) {
    if (table.rows.empty()) continue;
    RECT dirty{table.left - 2, table.rows.front().top - 2,
               table.right + 2, table.rows.front().bottom + 2};
    for (const auto& row : table.rows) {
      dirty.top = std::min<LONG>(dirty.top, static_cast<LONG>(row.top - 2));
      dirty.bottom = std::max<LONG>(dirty.bottom, static_cast<LONG>(row.bottom + 2));
    }
    RECT clipped{};
    if (IntersectRect(&clipped, &dirty, &client)) InvalidateRect(view.editor, &clipped, TRUE);
  }
}

std::vector<Application::TableGridGeometry> Application::BuildTableGridGeometry(
    const DocumentView& view, const RECT& client, HDC metrics_dc) const {
  if (!IsMarkdownFile(view.document.path()) || view.parse.tables.empty() ||
      client.right <= client.left || client.bottom <= client.top) return {};
  HDC dc = metrics_dc;
  const bool release_dc = dc == nullptr;
  if (release_dc) dc = GetDC(view.editor);
  if (!dc) return {};
  POINTL first_point{client.left, client.top};
  POINTL last_point{std::max(client.left, client.right - 1),
                    std::max(client.top, client.bottom - 1)};
  const auto first_view = static_cast<std::size_t>(std::max<LRESULT>(
      0, SendMessageW(view.editor, EM_CHARFROMPOS, 0, reinterpret_cast<LPARAM>(&first_point))));
  const auto last_view = static_cast<std::size_t>(std::max<LRESULT>(
      0, SendMessageW(view.editor, EM_CHARFROMPOS, 0, reinterpret_cast<LPARAM>(&last_point))));
  std::size_t visible_begin = view.editor_snapshot.NativeToSource(first_view);
  std::size_t visible_end = view.editor_snapshot.NativeToSource(last_view);
  visible_begin = visible_begin == 0 ? 0 : view.document.text().rfind(L'\n', visible_begin - 1) + 1;
  const auto next_line = view.document.text().find(L'\n', visible_end);
  visible_end = next_line == std::wstring::npos ? view.document.text().size() : next_line + 1;
  const HFONT font = reinterpret_cast<HFONT>(SendMessageW(view.editor, WM_GETFONT, 0, 0));
  const HGDIOBJ previous_font = font ? SelectObject(dc, font) : nullptr;
  TEXTMETRICW metrics{};
  GetTextMetricsW(dc, &metrics);
  const int line_height = std::max(1, static_cast<int>(metrics.tmHeight));
  if (previous_font) SelectObject(dc, previous_font);
  if (release_dc) ReleaseDC(view.editor, dc);

  std::vector<TableGridGeometry> result;
  for (const auto& table : view.parse.tables) {
    const std::size_t draw_begin = std::max(table.begin, visible_begin);
    const std::size_t draw_end = std::min(table.end, visible_end);
    if (draw_begin >= draw_end) continue;
    const auto rows = ParseTableVisualRows(view.document.text(), draw_begin, draw_end);
    std::vector<TableGridRow> row_geometry;
    row_geometry.reserve(rows.size());
    std::size_t column_count{};
    int shared_left = client.right;
    int shared_right = client.left;
    for (std::size_t row_index = 0; row_index < rows.size(); ++row_index) {
      const auto& row = rows[row_index];
      if (row.cells.empty()) continue;
      std::vector<POINT> starts(row.cells.size());
      std::vector<POINT> ends(row.cells.size());
      for (std::size_t cell_index = 0; cell_index < row.cells.size(); ++cell_index) {
        const auto& cell = row.cells[cell_index];
        const LONG native_begin = static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.begin));
        const LONG native_end = static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.end));
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&starts[cell_index]), native_begin);
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&ends[cell_index]), native_end);
      }
      const int top = static_cast<int>(starts.front().y) - 3;
      int bottom = top + line_height + 6;
      if (row_index + 1 < rows.size()) {
        POINT next_start{};
        const auto next_source = rows[row_index + 1].begin;
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&next_start),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(next_source)));
        bottom = static_cast<int>(next_start.y) - 3;
      }
      if (bottom <= top) bottom = top + line_height + 1;
      row_geometry.push_back({row.begin, row.end, row.cells, top, bottom});
      shared_left = std::min(shared_left, static_cast<int>(starts.front().x) - 5);
      shared_right = std::max(shared_right, static_cast<int>(ends.back().x) + 5);
      column_count = std::max(column_count, row.cells.size());
    }
    if (row_geometry.empty()) continue;
    const int left = std::clamp(shared_left, static_cast<int>(client.left),
                                static_cast<int>(client.right));
    if (left >= client.right) continue;
    const int right = std::clamp(std::max(shared_right, left + 8),
                                 left + 1, static_cast<int>(client.right));
    std::vector<int> shared_boundaries(column_count > 0 ? column_count - 1 : 0, left + 1);
    for (const auto& row_geometry_entry : row_geometry) {
      const auto& cells = row_geometry_entry.cells;
      if (cells.empty()) continue;
      std::vector<POINT> ends(cells.size());
      for (std::size_t cell_index = 0; cell_index < cells.size(); ++cell_index) {
        const auto& cell = cells[cell_index];
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&ends[cell_index]),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.end)));
      }
      for (std::size_t cell_index = 0;
           cell_index + 1 < ends.size() && cell_index < shared_boundaries.size();
           ++cell_index) {
        shared_boundaries[cell_index] = std::max(shared_boundaries[cell_index],
                                                 static_cast<int>(ends[cell_index].x) + 4);
      }
    }
    int previous_boundary = left;
    for (std::size_t cell_index = 0; cell_index < shared_boundaries.size(); ++cell_index) {
      const int remaining = static_cast<int>(shared_boundaries.size() - cell_index - 1);
      const int maximum = std::max(previous_boundary + 1, right - 1 - remaining);
      shared_boundaries[cell_index] = std::clamp(shared_boundaries[cell_index],
                                                previous_boundary + 1, maximum);
      previous_boundary = shared_boundaries[cell_index];
    }
    result.push_back({left, right, std::move(shared_boundaries), std::move(row_geometry)});
  }
  return result;
}

std::optional<std::size_t> Application::HitTestTableCell(const DocumentView& view, POINT point) const {
  if (view.sync_due != 0 || view.presentation_due != 0 || !IsMarkdownFile(view.document.path())) return std::nullopt;
  RECT client{};
  GetClientRect(view.editor, &client);
  const auto geometry = BuildTableGridGeometry(view, client);
  for (const auto& table : geometry) {
    if (point.x < table.left || point.x > table.right) continue;
    for (const auto& row : table.rows) {
      if (point.y < row.top || point.y > row.bottom || row.cells.empty()) continue;
      std::size_t cell_index{};
      while (cell_index < table.boundaries.size() && point.x >= table.boundaries[cell_index]) ++cell_index;
      cell_index = std::min(cell_index, row.cells.size() - 1);
      POINT hit = point;
      const auto native = static_cast<std::size_t>(std::max<LRESULT>(
          0, SendMessageW(view.editor, EM_CHARFROMPOS, 0, reinterpret_cast<LPARAM>(&hit))));
      const auto source = view.editor_snapshot.NativeToSource(native);
      return std::clamp(source, row.cells[cell_index].begin, row.cells[cell_index].end);
    }
  }
  return std::nullopt;
}

void Application::DrawTableGrid(const DocumentView& view, HDC paint_dc, const RECT& clip) {
  if (!paint_dc || view.sync_due != 0 || view.presentation_due != 0 ||
      !IsMarkdownFile(view.document.path()) || view.parse.tables.empty()) return;
  RECT client{};
  GetClientRect(view.editor, &client);
  if (clip.right <= clip.left || clip.bottom <= clip.top) return;
  const auto geometry = BuildTableGridGeometry(view, client, paint_dc);
  HPEN pen = CreatePen(PS_SOLID, 1, theme_border_);
  if (!pen) return;
  IntersectClipRect(paint_dc, clip.left, clip.top, clip.right, clip.bottom);
  const HGDIOBJ previous_pen = SelectObject(paint_dc, pen);
  if (previous_pen == nullptr || previous_pen == HGDI_ERROR) {
    DeleteObject(pen);
    return;
  }
  for (const auto& table : geometry) {
    for (const auto& row : table.rows) {
      if (row.bottom < client.top || row.top >= client.bottom) continue;
      MoveToEx(paint_dc, table.left, row.top, nullptr);
      LineTo(paint_dc, table.right, row.top);
      MoveToEx(paint_dc, table.left, row.bottom, nullptr);
      LineTo(paint_dc, table.right, row.bottom);
      MoveToEx(paint_dc, table.left, row.top, nullptr);
      LineTo(paint_dc, table.left, row.bottom);
      for (std::size_t cell_index = 0;
           cell_index + 1 < row.cells.size() && cell_index < table.boundaries.size();
           ++cell_index) {
        MoveToEx(paint_dc, table.boundaries[cell_index], row.top, nullptr);
        LineTo(paint_dc, table.boundaries[cell_index], row.bottom);
      }
      MoveToEx(paint_dc, table.right, row.top, nullptr);
      LineTo(paint_dc, table.right, row.bottom);
    }
  }
  SelectObject(paint_dc, previous_pen);
  DeleteObject(pen);
}

void Application::RebuildOutline(const DocumentView& view) {
  TreeView_DeleteAllItems(outline_);
  std::array<HTREEITEM, 6> parents{};
  for (const auto& heading : view.parse.headings) {
    TVINSERTSTRUCTW insert{};
    insert.hParent = heading.level > 1 && parents[heading.level - 2] != nullptr
                         ? parents[heading.level - 2]
                         : TVI_ROOT;
    insert.hInsertAfter = TVI_LAST;
    insert.item.mask = TVIF_TEXT | TVIF_PARAM;
    insert.item.pszText = const_cast<wchar_t*>(heading.text.c_str());
    insert.item.lParam = static_cast<LPARAM>(heading.begin);
    HTREEITEM item = TreeView_InsertItem(outline_, &insert);
    parents[heading.level - 1] = item;
    for (int level = heading.level; level < 6; ++level) parents[level] = nullptr;
  }
  TreeView_Expand(outline_, TreeView_GetRoot(outline_), TVE_EXPAND);
}

bool Application::EditorText(HWND editor, const EditorSnapshot& snapshot,
                             std::wstring& text) const {
  text.clear();
  GETTEXTLENGTHEX length_request{GTL_NUMCHARS | GTL_PRECISE, 1200};
  const LRESULT raw_length = SendMessageW(editor, EM_GETTEXTLENGTHEX,
                                          reinterpret_cast<WPARAM>(&length_request), 0);
  if (raw_length < 0) return false;
  const auto length = static_cast<std::size_t>(raw_length);
  text.assign(length + 1, L'\0');
  GETTEXTEX text_request{static_cast<DWORD>(text.size() * sizeof(wchar_t)),
                         GT_RAWTEXT, 1200, nullptr, nullptr};
  const LRESULT raw_copied = SendMessageW(editor, EM_GETTEXTEX,
                                          reinterpret_cast<WPARAM>(&text_request),
                                          reinterpret_cast<LPARAM>(text.data()));
  if (raw_copied < 0) {
    text.clear();
    return false;
  }
  const auto copied = static_cast<std::size_t>(raw_copied);
  if (copied != length) {
    text.clear();
    return false;
  }
  text.resize(length);
  // Keep every EM/TOM/OLE position in the same native space as the snapshot.
  // RichEdit normally returns one CR per paragraph with GT_RAWTEXT, but older
  // builds and alternate export paths can expose CRLF or lone LF instead.
  text = CanonicalizeNativeText(text);

  // RichEdit exposes an embedded image as a space through its text APIs on some
  // builds.  TOM2 still exposes the raw character, so restore object markers
  // near each known collapsed range. A genuine user replacement with a plain
  // space has no inline-object character and remains a source edit.
  const auto size_delta = static_cast<std::ptrdiff_t>(text.size()) -
                          static_cast<std::ptrdiff_t>(snapshot.view.size());
  IRichEditOle* rich_edit{};
  ITextDocument* text_document{};
  if (SendMessageW(editor, EM_GETOLEINTERFACE, 0, reinterpret_cast<LPARAM>(&rich_edit)) != 0 &&
      rich_edit != nullptr) {
    rich_edit->QueryInterface(IID_PPV_ARGS(&text_document));
  }
  if (text_document) {
    for (const auto& collapsed : snapshot.collapsed) {
      const std::array<std::ptrdiff_t, 2> candidates{
          static_cast<std::ptrdiff_t>(collapsed.view),
          static_cast<std::ptrdiff_t>(collapsed.view) + size_delta};
      for (const auto candidate : candidates) {
        if (candidate < 0 || static_cast<std::size_t>(candidate) >= text.size() ||
            text[static_cast<std::size_t>(candidate)] != L' ') continue;
        ITextRange* range{};
        ITextRange2* range2{};
        long character{};
        long type{}, align{}, character1{}, character2{}, count{}, tex_style{}, columns{}, level{};
        if (SUCCEEDED(text_document->Range(static_cast<long>(candidate),
                                           static_cast<long>(candidate + 1), &range)) &&
            range != nullptr && SUCCEEDED(range->QueryInterface(IID_PPV_ARGS(&range2))) &&
            range2 != nullptr) {
          const bool object_character = SUCCEEDED(range2->GetChar2(&character, 0)) &&
                                        character == 0xFFFC;
          const bool inline_object = range2->GetInlineObject(
              &type, &align, &character, &character1, &character2, &count,
              &tex_style, &columns, &level) == S_OK;
          if (object_character || inline_object)
            text[static_cast<std::size_t>(candidate)] = L'\uFFFC';
        }
        if (range2) range2->Release();
        if (range) range->Release();
        if (text[static_cast<std::size_t>(candidate)] == L'\uFFFC') break;
      }
    }
    text_document->Release();
  }

  // Classic OLE objects are also covered when the RichEdit build exposes
  // their positions through IRichEditOle.
  if (rich_edit) {
    const LONG count = rich_edit->GetObjectCount();
    for (LONG index = 0; index < count; ++index) {
      REOBJECT object{sizeof(object)};
      if (FAILED(rich_edit->GetObject(index, &object, REO_GETOBJ_NO_INTERFACES)) ||
          object.cp < 0 || static_cast<std::size_t>(object.cp) >= text.size()) {
        rich_edit->Release();
        text.clear();
        return false;
      }
      text[static_cast<std::size_t>(object.cp)] = L'\uFFFC';
    }
    rich_edit->Release();
  }
  return true;
}

void Application::UpdateStatus() {
  std::wstring text;
  if (active_document_ < documents_.size()) {
    const auto& view = *documents_[active_document_];
    const auto& document = view.document;
    text = (document.dirty() || view.sync_due != 0) ? L"未保存  |  " : L"保存済み  |  ";
    text += EncodingLabel(document.encoding()) + L"  |  " + LineEndingLabel(document.line_ending());
    if (document.HasExternalChange()) text += L"  |  外部変更あり";
  } else if (!workspace_.empty()) {
    text = workspace_.wstring();
  } else {
    text = L"Workspaceまたはファイルを開いてください。";
  }
  SetStatusText(text);
}

void Application::ShowFindBar() {
  ShowWindow(find_bar_, SW_SHOW);
  ShowWindow(find_edit_, SW_SHOW);
  ShowWindow(find_next_, SW_SHOW);
  ShowWindow(replace_edit_, SW_SHOW);
  ShowWindow(find_workspace_, SW_SHOW);
  ShowWindow(replace_workspace_, SW_SHOW);
  ShowWindow(find_case_, SW_SHOW);
  ShowWindow(find_regex_, SW_SHOW);
  ShowWindow(find_word_, SW_SHOW);
  ShowWindow(find_include_glob_, SW_SHOW);
  ShowWindow(find_exclude_glob_, SW_SHOW);
  ShowWindow(replace_one_, SW_SHOW);
  ShowWindow(replace_document_, SW_SHOW);
  LayoutControls();
  SetFocus(find_edit_);
  SendMessageW(find_edit_, EM_SETSEL, 0, -1);
}

SearchQuery Application::SearchQueryFromFindBar() const {
  SearchQuery query;
  query.text = ControlText(find_edit_);
  query.match_case = SendMessageW(find_case_, BM_GETCHECK, 0, 0) == BST_CHECKED;
  query.regular_expression = SendMessageW(find_regex_, BM_GETCHECK, 0, 0) == BST_CHECKED;
  query.whole_word = SendMessageW(find_word_, BM_GETCHECK, 0, 0) == BST_CHECKED;
  query.include_globs = SplitGlobList(ControlText(find_include_glob_));
  query.exclude_globs = SplitGlobList(ControlText(find_exclude_glob_));
  return query;
}

void Application::FindNext(bool restart_from_beginning) {
  if (active_document_ >= documents_.size()) return;
  const auto search = SearchQueryFromFindBar();
  if (search.text.empty()) {
    ShowFindBar();
    return;
  }
  auto& view = *documents_[active_document_];
  SyncDocumentFromEditor(view);
  HWND editor = view.editor;
  CHARRANGE selection{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  std::vector<SearchMatch> matches;
  std::wstring error;
  if (!SearchDocumentText(view.document.path(), view.document.text(), search, matches, error)) {
    MessageBoxW(window_, error.c_str(), L"文書検索", MB_ICONWARNING);
    return;
  }
  if (matches.empty()) {
    MessageBoxW(window_, L"一致する箇所はありません。", L"文書検索", MB_ICONINFORMATION);
    return;
  }
  const std::size_t source_start = restart_from_beginning
      ? 0
      : view.editor_snapshot.NativeToSource(static_cast<std::size_t>(selection.cpMax));
  auto match = std::ranges::find_if(matches, [source_start](const auto& item) {
    return item.begin >= source_start;
  });
  if (match == matches.end()) match = matches.begin();
  const CHARRANGE target{
        static_cast<LONG>(view.editor_snapshot.SourceToNative(match->begin)),
        static_cast<LONG>(view.editor_snapshot.SourceToNative(match->end))};
  SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&target));
  SendMessageW(editor, EM_SCROLLCARET, 0, 0);
  SetFocus(editor);
}

void Application::ReplaceCurrentDocument(bool all) {
  if (active_document_ >= documents_.size()) return;
  auto& view = *documents_[active_document_];
  if (view.ime_composing) return;
  SyncDocumentFromEditor(view);
  const auto query = SearchQueryFromFindBar();
  if (query.text.empty()) {
    ShowFindBar();
    return;
  }
  const std::wstring replacement = ControlText(replace_edit_);
  std::wstring output;
  std::wstring error;
  std::size_t count{};
  std::size_t replaced_begin{};
  std::size_t replaced_end{};
  if (all) {
    if (!ReplaceDocumentText(view.document.text(), query, replacement, output, count, error)) {
      MessageBoxW(window_, error.c_str(), L"文書置換", MB_ICONWARNING);
      return;
    }
    if (count == 0) {
      MessageBoxW(window_, L"一致する箇所はありません。", L"文書置換", MB_ICONINFORMATION);
      return;
    }
    ApplySourceTextWithUndo(view, std::move(output));
    const std::wstring message = std::to_wstring(count) + L"箇所を1つのUndo操作として置換しました。";
    SetStatusText(message);
    return;
  }

  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_begin = view.editor_snapshot.NativeToSource(selection.cpMin);
  const std::size_t source_end = view.editor_snapshot.NativeToSource(selection.cpMax);
  const std::size_t start = source_begin == source_end ? source_end : source_begin;
  bool replaced{};
  if (!ReplaceDocumentMatch(view.document.text(), query, replacement, start, output,
                            replaced_begin, replaced_end, replaced, error)) {
    MessageBoxW(window_, error.c_str(), L"1件置換", MB_ICONWARNING);
    return;
  }
  if (!replaced && start != 0 &&
      !ReplaceDocumentMatch(view.document.text(), query, replacement, 0, output,
                            replaced_begin, replaced_end, replaced, error)) {
    MessageBoxW(window_, error.c_str(), L"1件置換", MB_ICONWARNING);
    return;
  }
  if (!replaced) {
    MessageBoxW(window_, L"一致する箇所はありません。", L"1件置換", MB_ICONINFORMATION);
    return;
  }
  ApplySourceTextWithUndo(view, std::move(output));
  const CHARRANGE target{
        static_cast<LONG>(view.editor_snapshot.SourceToNative(replaced_begin)),
        static_cast<LONG>(view.editor_snapshot.SourceToNative(replaced_end))};
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&target));
  SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(view.editor);
}

void Application::SearchWorkspaceFromFindBar() {
  if (workspace_.empty()) return;
  const auto query = SearchQueryFromFindBar();
  if (query.text.empty()) {
    ShowFindBar();
    return;
  }
  std::map<std::filesystem::path, std::wstring> unsaved;
  for (auto& view : documents_) {
    if (!view->ime_composing) SyncDocumentFromEditor(*view);
    if (!view->document.untitled())
      unsaved.insert_or_assign(std::filesystem::absolute(view->document.path()).lexically_normal(),
                               view->document.text());
  }
  if (workspace_search_worker_.joinable()) {
    workspace_search_worker_.request_stop();
    workspace_search_worker_.join();
  }
  workspace_search_started_ = true;
  workspace_search_due_ = 0;
  const std::uint64_t generation = ++workspace_search_generation_;
  workspace_search_results_.clear();
  workspace_search_issue_count_ = 0;
  ListView_DeleteAllItems(find_results_);
  ShowWindow(find_results_, SW_SHOW);
  LayoutControls();
  SetStatusText(L"Workspace検索中… 0件（逐次結果）");
  const auto root = workspace_;
  const HWND owner = window_;
  workspace_search_worker_ = std::jthread(
      [root, query, unsaved = std::move(unsaved), generation, owner](std::stop_token stop) {
        std::vector<SearchMatch> final_matches;
        std::wstring error;
        const bool completed = SearchWorkspace(
            root, query, unsaved, final_matches, error, [&] { return stop.stop_requested(); },
            [generation, owner, &stop](std::vector<SearchMatch> batch) {
              if (stop.stop_requested()) return;
              auto* payload = new WorkspaceSearchBatchMessage{generation, std::move(batch), {}};
              if (!PostMessageW(owner, kWorkspaceSearchBatchMessage, 0,
                                reinterpret_cast<LPARAM>(payload))) delete payload;
            }, nullptr,
            [generation, owner, &stop](std::vector<SearchIssue> issues) {
              if (stop.stop_requested()) return;
              auto* payload = new WorkspaceSearchBatchMessage{generation, {}, std::move(issues)};
              if (!PostMessageW(owner, kWorkspaceSearchBatchMessage, 0,
                                reinterpret_cast<LPARAM>(payload))) delete payload;
            });
        auto* payload = new WorkspaceSearchCompleteMessage{generation, completed, std::move(error)};
        if (!PostMessageW(owner, kWorkspaceSearchCompleteMessage, 0,
                          reinterpret_cast<LPARAM>(payload))) delete payload;
      });
}

void Application::ScheduleWorkspaceSearch() {
  if (!workspace_search_started_) return;
  if (workspace_search_worker_.joinable()) workspace_search_worker_.request_stop();
  ++workspace_search_generation_;
  workspace_search_results_.clear();
  workspace_search_issue_count_ = 0;
  ListView_DeleteAllItems(find_results_);
  workspace_search_due_ = GetTickCount64() + 250;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  SetStatusText(L"検索条件が変わりました。旧結果を破棄して再検索します…");
}

void Application::ApplyWorkspaceSearchBatch(void* raw_payload) {
  std::unique_ptr<WorkspaceSearchBatchMessage> payload(
      static_cast<WorkspaceSearchBatchMessage*>(raw_payload));
  if (!payload || payload->generation != workspace_search_generation_) return;
  for (auto& match : payload->matches) {
    const std::size_t index = workspace_search_results_.size();
    workspace_search_results_.push_back(std::move(match));
    const auto& inserted = workspace_search_results_.back();
    std::error_code relative_error;
    auto relative = std::filesystem::relative(inserted.path, workspace_, relative_error).wstring();
    if (relative_error) relative = inserted.path.wstring();
    LVITEMW item{LVIF_TEXT | LVIF_PARAM};
    item.iItem = ListView_GetItemCount(find_results_);
    item.pszText = relative.data();
    item.lParam = static_cast<LPARAM>(index);
    ListView_InsertItem(find_results_, &item);
    auto line = std::to_wstring(inserted.line);
    ListView_SetItemText(find_results_, item.iItem, 1, line.data());
    auto preview = inserted.preview;
    std::ranges::replace(preview, L'\r', L' ');
    std::ranges::replace(preview, L'\n', L' ');
    ListView_SetItemText(find_results_, item.iItem, 2, preview.data());
  }
  for (auto& issue : payload->issues) {
    ++workspace_search_issue_count_;
    std::error_code relative_error;
    auto relative = std::filesystem::relative(issue.path, workspace_, relative_error).wstring();
    if (relative_error) relative = issue.path.wstring();
    LVITEMW item{LVIF_TEXT | LVIF_PARAM};
    item.iItem = ListView_GetItemCount(find_results_);
    item.pszText = relative.data();
    item.lParam = static_cast<LPARAM>(-1);
    ListView_InsertItem(find_results_, &item);
    auto issue_mark = std::wstring(L"!");
    ListView_SetItemText(find_results_, item.iItem, 1, issue_mark.data());
    ListView_SetItemText(find_results_, item.iItem, 2, issue.message.data());
  }
  const std::wstring status = L"Workspace検索中… " +
      std::to_wstring(workspace_search_results_.size()) + L"件 / 未処理 " +
      std::to_wstring(workspace_search_issue_count_) + L"件（逐次結果）";
  SetStatusText(status);
}

void Application::CompleteWorkspaceSearch(void* raw_payload) {
  std::unique_ptr<WorkspaceSearchCompleteMessage> payload(
      static_cast<WorkspaceSearchCompleteMessage*>(raw_payload));
  if (!payload || payload->generation != workspace_search_generation_) return;
  if (!payload->completed) {
    if (payload->error.find(L"中止") == std::wstring::npos && !payload->error.empty())
      MessageBoxW(window_, payload->error.c_str(), L"Workspace検索", MB_ICONWARNING);
    const std::wstring status = payload->error.empty() ? L"Workspace検索を中止しました。"
                                                       : payload->error;
    SetStatusText(status);
    return;
  }
  const std::wstring status = L"Workspace検索完了: " +
      std::to_wstring(workspace_search_results_.size()) +
      L"件 / 未処理 " + std::to_wstring(workspace_search_issue_count_) +
      L"件。結果一覧で詳細を確認できます。";
  SetStatusText(status);
}

void Application::OpenWorkspaceSearchResult(std::size_t index) {
  LVITEMW item{LVIF_PARAM};
  item.iItem = static_cast<int>(index);
  if (!ListView_GetItem(find_results_, &item) || item.lParam < 0 ||
      static_cast<std::size_t>(item.lParam) >= workspace_search_results_.size()) return;
  const auto match = workspace_search_results_[static_cast<std::size_t>(item.lParam)];
  OpenDocument(match.path);
  if (active_document_ >= documents_.size()) return;
  auto& view = *documents_[active_document_];
  SyncDocumentFromEditor(view);
  const CHARRANGE selection{
      static_cast<LONG>(view.editor_snapshot.SourceToNative(match.begin)),
      static_cast<LONG>(view.editor_snapshot.SourceToNative(match.end))};
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(view.editor);
}

void Application::ReplaceWorkspaceFromFindBar() {
  if (workspace_.empty()) return;
  ShowFindBar();
  const auto query = SearchQueryFromFindBar();
  const std::wstring replacement = ControlText(replace_edit_);
  if (query.text.empty()) { SetFocus(find_edit_); return; }
  std::map<std::filesystem::path, std::wstring> buffers;
  for (auto& view : documents_) {
    SyncDocumentFromEditor(*view);
    if (!view->document.untitled()) buffers.emplace(view->document.path(), view->document.text());
  }
  ReplacePlan plan;
  std::wstring error;
  if (!RunCancellableTask(window_, L"置換preview", L"置換対象を確認しています",
      [&](HANDLE cancellation) {
        return PreviewWorkspaceReplace(workspace_, query, replacement, buffers, plan, error, [&] {
          return WaitForSingleObject(cancellation, 0) == WAIT_OBJECT_0;
        });
      }, error)) {
    MessageBoxW(window_, error.c_str(), L"置換preview", MB_ICONWARNING);
    return;
  }
  std::size_t replacements{};
  for (const auto& file : plan.files) replacements += file.replacement_count;
  if (replacements == 0) {
    MessageBoxW(window_, L"置換対象はありません。", L"置換preview", MB_ICONINFORMATION);
    return;
  }
  std::wstring preview = std::to_wstring(plan.files.size()) + L"ファイル、" +
      std::to_wstring(replacements) + L"箇所を置換します。\n"
      L"開いている文書は未保存bufferへ1つのUndo操作として適用し、閉じた文書は安全保存とjournalを使います。\n";
  const std::size_t preview_limit = std::min<std::size_t>(plan.files.size(), 8);
  for (std::size_t index = 0; index < preview_limit; ++index) {
    std::error_code relative_error;
    const auto relative = std::filesystem::relative(plan.files[index].path, workspace_, relative_error);
    preview += L"\n- " + (relative_error ? plan.files[index].path.wstring() : relative.wstring()) +
               L" (" + std::to_wstring(plan.files[index].replacement_count) + L")";
  }
  if (plan.files.size() > preview_limit)
    preview += L"\n- ほか " + std::to_wstring(plan.files.size() - preview_limit) + L" ファイル";
  preview += L"\n\n続行しますか？";
  if (MessageBoxW(window_, preview.c_str(), L"置換preview", MB_ICONQUESTION | MB_YESNO) != IDYES) return;

  ReplacePlan disk_plan{plan.query, plan.replacement, {}};
  std::vector<const ReplaceFilePlan*> buffer_plans;
  for (const auto& item : plan.files) {
    const auto open = std::ranges::find_if(documents_, [&](const auto& view) {
      return _wcsicmp(view->document.path().c_str(), item.path.c_str()) == 0;
    });
    if (open == documents_.end()) disk_plan.files.push_back(item);
    else buffer_plans.push_back(&item);
  }
  ReplaceApplyResult result;
  bool complete = true;
  if (!disk_plan.files.empty()) {
    complete = RunCancellableTask(window_, L"Workspace置換", L"安全保存とjournalを適用しています",
        [&](HANDLE cancellation) {
          return ApplyWorkspaceReplace(workspace_, disk_plan, result, error, [&] {
            return WaitForSingleObject(cancellation, 0) == WAIT_OBJECT_0;
          });
        }, error);
  }
  const bool cancelled = error.find(L"中止") != std::wstring::npos;
  if (!cancelled) {
    for (const auto* item : buffer_plans) {
      const auto open = std::ranges::find_if(documents_, [&](const auto& view) {
        return _wcsicmp(view->document.path().c_str(), item->path.c_str()) == 0;
      });
      if (open == documents_.end() || (*open)->document.text() != item->before ||
          (*open)->ime_composing) {
        result.conflicts.push_back(item->path);
        complete = false;
        continue;
      }
      ApplySourceTextWithUndo(*(*open), item->after);
      ++result.applied_files;
      result.applied_replacements += item->replacement_count;
    }
  }
  PopulateWorkspaceTree();
  std::wstring message = std::to_wstring(result.applied_files) + L"ファイル、" +
      std::to_wstring(result.applied_replacements) + L"箇所を置換しました。";
  if (!result.journal.empty()) message += L"\nclosed文書 journal: " + result.journal.wstring();
  if (!buffer_plans.empty()) message += L"\nopen文書の置換は各タブでUndoできます。";
  if (!complete && !error.empty()) message += L"\n" + error;
  if (!result.conflicts.empty()) {
    message += L"\n競合した" + std::to_wstring(result.conflicts.size()) +
        L"ファイルは変更していません。";
  }
  MessageBoxW(window_, message.c_str(), L"Workspace置換", complete ? MB_ICONINFORMATION : MB_ICONWARNING);
}

void Application::CreateProfile(BuiltInProfile profile) {
  SYSTEMTIME date{};
  GetLocalTime(&date);
  CreateProfileForDate(profile, date);
}

void Application::UpdateCalendarTooltip(POINT point) {
  if (!calendar_tooltip_) return;
  MCHITTESTINFO hit{sizeof(hit)};
  hit.pt = point;
  MonthCal_HitTest(calendar_, &hit);
  if (hit.uHit != MCHT_CALENDARDATE && hit.uHit != MCHT_CALENDARDATENEXT &&
      hit.uHit != MCHT_CALENDARDATEPREV) {
    SendMessageW(calendar_tooltip_, TTM_POP, 0, 0);
    return;
  }
  wchar_t date[32]{};
  swprintf_s(date, L"%04u-%02u-%02u", hit.st.wYear, hit.st.wMonth, hit.st.wDay);
  calendar_tooltip_text_ = date;
  if (const auto holiday = JapaneseHolidayName(hit.st.wYear, hit.st.wMonth, hit.st.wDay))
    calendar_tooltip_text_ += L"  " + std::wstring(*holiday);
  else if (!JapaneseHolidayYearSupported(hit.st.wYear))
    calendar_tooltip_text_ += L"  祝日データ収録範囲外（不明）";
  if (!workspace_.empty()) {
    wchar_t relative[128]{};
    swprintf_s(relative, L"Dairy/%04u/%04u%02u/%04u%02u%02u.md", hit.st.wYear,
               hit.st.wYear, hit.st.wMonth, hit.st.wYear, hit.st.wMonth, hit.st.wDay);
    if (std::filesystem::exists(workspace_ / relative)) calendar_tooltip_text_ += L"  Dairy作成済み";
  }
  calendar_tooltip_text_ += L"\n" + std::wstring(JapaneseHolidayDataVersion()) + L" / " +
                            std::to_wstring(JapaneseHolidayFirstYear()) + L"–" +
                            std::to_wstring(JapaneseHolidayLastYear());
  if (!holiday_update_status_.empty()) calendar_tooltip_text_ += L"\n" + holiday_update_status_;
  if (calendar_details_) SetWindowTextW(calendar_details_, calendar_tooltip_text_.c_str());
  TTTOOLINFOW tool{sizeof(tool)};
  tool.hwnd = calendar_;
  tool.uId = 1;
  tool.lpszText = calendar_tooltip_text_.data();
  SendMessageW(calendar_tooltip_, TTM_UPDATETIPTEXTW, 0, reinterpret_cast<LPARAM>(&tool));
}

void Application::ImportHolidayData() {
  std::wstring path;
  if (!PromptText(window_, instance_, L"祝日CSVのローカル取込み",
                 L"内閣府の公開CSVをダウンロード済みの場合は、そのローカルpathを指定してください。\n"
                 L"自動更新は既定でOFFで、ここでは通信しません。", path) || path.empty()) return;
  JapaneseHolidayImportInfo info;
  std::wstring error;
  if (!ImportJapaneseHolidayCsvFile(path, info, error)) {
    MessageBoxW(window_, error.c_str(), L"祝日データを取込めません", MB_ICONWARNING);
    return;
  }
  bool cache_saved = true;
  if (workspace_store_) {
    const auto cache = workspace_store_->metadata_root() / L".cache" / L"holidays" / kHolidayCacheName;
    std::error_code copy_error;
    std::filesystem::create_directories(cache.parent_path(), copy_error);
    auto temporary = cache; temporary += L".new";
    if (!copy_error && CopyFileW(path.c_str(), temporary.c_str(), FALSE) &&
        MoveFileExW(temporary.c_str(), cache.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
      holiday_cache_loaded_ = true;
    } else {
      DeleteFileW(temporary.c_str());
      cache_saved = false;
    }
  }
  holiday_update_status_ = cache_saved
      ? L"ローカルCSVを取込み済み（外部通信なし）"
      : L"ローカルCSVはメモリへ取込みましたがcache保存に失敗しました。";
  if (calendar_) InvalidateRect(calendar_, nullptr, TRUE);
  const std::wstring message = L"祝日データを取込みました。\n件数: " +
      std::to_wstring(info.records) + L"\n対象年: " + std::to_wstring(info.first_year) + L"–" +
      std::to_wstring(info.last_year) + L"\n\n通常起動・月移動では外部通信しません。";
  MessageBoxW(window_, message.c_str(), L"祝日データ", MB_ICONINFORMATION);
}

void Application::LoadHolidayCache() {
  if (holiday_cache_loaded_ || !workspace_store_) return;
  holiday_cache_loaded_ = true;
  const auto cache = workspace_store_->metadata_root() / L".cache" / L"holidays" / kHolidayCacheName;
  if (!std::filesystem::exists(cache)) return;
  JapaneseHolidayImportInfo info;
  std::wstring error;
  if (ImportJapaneseHolidayCsvFile(cache, info, error)) {
    holiday_update_status_ = L"cacheの祝日データを使用中（" + std::to_wstring(info.first_year) + L"–" +
                             std::to_wstring(info.last_year) + L"）";
  } else {
    holiday_update_status_ = L"祝日cacheを検証できません。内蔵データを使用中。";
  }
}

void Application::ScheduleHolidayUpdate() {
  if (!workspace_store_ || !settings_.holiday_auto_update || TestAutomationSilent()) {
    if (holiday_update_running_) {
      ++holiday_update_generation_;
      holiday_update_running_ = false;
      if (holiday_update_worker_.joinable()) holiday_update_worker_.request_stop();
    }
    return;
  }
  HolidayUpdateState state;
  const auto state_path = workspace_store_->metadata_root() / L".cache" / L"holidays" / kHolidayStateName;
  if (!ReadHolidayState(state_path, state)) {
    holiday_update_status_ = L"祝日更新状態を読めません。自動確認は保留します。";
    return;
  }
  if (!JapaneseHolidayUpdateDue(state.last_attempt_unix, state.last_successful_check_unix,
                                HolidayNowUnix())) return;
  StartHolidayUpdate(false);
}

void Application::StartHolidayUpdate(bool manual) {
  if (!workspace_store_) {
    holiday_update_status_ = L"Workspaceを開いてから祝日更新を実行してください。";
    return;
  }
  if (holiday_update_running_) {
    holiday_update_status_ = L"祝日更新は既に実行中です。";
    return;
  }
  if (!manual && (!settings_.holiday_auto_update || TestAutomationSilent())) return;
  const auto holiday_root = workspace_store_->metadata_root() / L".cache" / L"holidays";
  const auto cache_path = holiday_root / kHolidayCacheName;
  const auto state_path = holiday_root / kHolidayStateName;
  HolidayUpdateState state;
  if (!ReadHolidayState(state_path, state)) state = {};
  if (!manual && !JapaneseHolidayUpdateDue(state.last_attempt_unix,
                                            state.last_successful_check_unix,
                                            HolidayNowUnix())) return;
  state.last_attempt_unix = HolidayNowUnix();
  state.error.clear();
  std::wstring state_error;
  if (!WriteHolidayState(state_path, state, state_error)) {
    holiday_update_status_ = state_error;
    return;
  }
  if (holiday_update_worker_.joinable()) holiday_update_worker_.join();
  holiday_update_running_ = true;
  const auto generation = ++holiday_update_generation_;
  const auto etag = state.etag;
  const auto last_modified = state.last_modified;
  const HWND owner = window_;
  holiday_update_status_ = manual ? L"内閣府CSVを確認中…" : L"祝日更新を月次確認中…";
  holiday_update_worker_ = std::jthread(
      [owner, generation, cache_path, state_path, etag, last_modified](std::stop_token stop) {
        JapaneseHolidayOnlineResult result;
        FetchJapaneseHolidayCsv(stop, etag, last_modified, result);
        auto* payload = new HolidayUpdateMessage{generation, cache_path, state_path, std::move(result)};
        if (!PostMessageW(owner, kHolidayUpdateMessage, 0, reinterpret_cast<LPARAM>(payload))) delete payload;
      });
}

void Application::CompleteHolidayUpdate(void* raw_payload) {
  std::unique_ptr<HolidayUpdateMessage> payload(static_cast<HolidayUpdateMessage*>(raw_payload));
  if (!payload || payload->generation != holiday_update_generation_) return;
  holiday_update_running_ = false;
  if (holiday_update_worker_.joinable()) holiday_update_worker_.join();
  HolidayUpdateState state;
  if (!ReadHolidayState(payload->state_path, state)) state = {};
  state.last_attempt_unix = std::max(state.last_attempt_unix, HolidayNowUnix());
  JapaneseHolidayImportInfo info;
  std::wstring cache_error;
  const bool verified_cache = payload->result.not_modified &&
      std::filesystem::exists(payload->cache_path) &&
      ImportJapaneseHolidayCsvFile(payload->cache_path, info, cache_error);
  auto assessment = AssessJapaneseHolidayResponse(
      payload->result.status, payload->result.not_modified, verified_cache, state.records,
      payload->result.csv);
  bool accepted = assessment.accepted;
  std::wstring error = assessment.error;
  if (!accepted && !payload->result.error.empty())
    error = payload->result.error;
  if (accepted && assessment.replace_cache) {
    if (!WriteHolidayCache(payload->cache_path, payload->result.csv, error) ||
        !ImportJapaneseHolidayCsv(payload->result.csv, info, error)) {
      accepted = false;
    }
  }
  if (accepted) {
    state.last_successful_check_unix = HolidayNowUnix();
    state.records = info.records;
    state.first_year = info.first_year;
    state.last_year = info.last_year;
    if (!payload->result.etag.empty()) state.etag = payload->result.etag;
    if (!payload->result.last_modified.empty()) state.last_modified = payload->result.last_modified;
    state.error.clear();
    holiday_update_status_ = payload->result.not_modified
        ? L"祝日cacheを再確認しました（変更なし）"
        : L"内閣府の祝日データを更新しました";
    if (calendar_) InvalidateRect(calendar_, nullptr, TRUE);
  } else {
    state.error = error;
    holiday_update_status_ = L"祝日自動更新は失敗しました。既知データを継続利用します。";
  }
  std::wstring state_error;
  if (!WriteHolidayState(payload->state_path, state, state_error) && !state_error.empty())
    holiday_update_status_ += L"（状態保存失敗）";
  if (status_) SetStatusText(holiday_update_status_);
}

void Application::CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date) {
  if (workspace_.empty() || !workspace_store_) {
    MessageBoxW(window_, L"先にWorkspaceを開き、.mdliteの作成を許可してください。",
                L"ノート作成", MB_ICONINFORMATION);
    return;
  }
  const std::wstring id = profile == BuiltInProfile::Daily ? L"daily" :
                          profile == BuiltInProfile::Meeting ? L"meeting" : L"memo";
  CreateProfileById(id, &date);
}

void Application::CreateProfileById(std::wstring id, const SYSTEMTIME* requested_date) {
  if (workspace_.empty() || !workspace_store_) {
    MessageBoxW(window_, L"先にWorkspaceを開き、.mdliteの作成を許可してください。",
                L"ノート作成", MB_ICONINFORMATION);
    return;
  }
  std::wstring error;
  std::vector<ProfileDefinition> profiles;
  if (!ResolveProfiles(CommonProfilesPath(), workspace_store_->metadata_root() / L"profiles.toml",
                       profiles, error)) {
    MessageBoxW(window_, error.c_str(), L"ノート作成", MB_ICONWARNING);
    return;
  }
  if (id.empty()) {
    std::wstring choices;
    for (const auto& profile : profiles) choices += profile.id + L" — " + profile.name + L"\n";
    if (!profiles.empty()) id = profiles.front().id;
    if (!PromptText(window_, instance_, L"プロファイルから作成",
                    L"profile idを指定してください。\n\n" + choices, id) || id.empty()) return;
  }
  const auto profile = std::ranges::find_if(profiles, [&](const auto& item) { return item.id == id; });
  if (profile == profiles.end()) {
    MessageBoxW(window_, L"指定したprofileが見つかりません。", L"ノート作成", MB_ICONWARNING);
    return;
  }
  SYSTEMTIME date{};
  if (requested_date) date = *requested_date;
  else GetLocalTime(&date);
  ProfileValues values;
  for (const auto& input : profile->inputs) {
    std::wstring value = input.default_value;
    std::wstring label = input.label;
    if (input.required) label += L"（必須）";
    if (!PromptText(window_, instance_, profile->name, label, value)) return;
    values[input.id] = std::move(value);
  }
  const auto preview = PreviewProfilePath(workspace_, *profile, date, values, error);
  if (!preview) {
    MessageBoxW(window_, error.c_str(), L"ノート作成", MB_ICONWARNING);
    return;
  }
  NoteCreationResult result{};
  if (!CreateProfileNote(workspace_, *profile, date, values, result, error)) {
    MessageBoxW(window_, error.c_str(), L"ノート作成", MB_ICONWARNING);
    return;
  }
  OpenDocument(result.path);
  if (result.created && active_document_ < documents_.size()) {
    SendMessageW(documents_[active_document_]->editor, EM_SETSEL, result.cursor, result.cursor);
  }
  PopulateWorkspaceTree();
}

void Application::ApplyTableAction(TableAction action) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  HWND editor = view.editor;
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::wstring source = view.document.text();
  const auto source_caret = view.editor_snapshot.NativeToSource(selection.cpMin);
  TableEditResult edit;
  switch (action) {
    case TableAction::InsertRowBefore: edit = InsertTableRow(source, source_caret, false); break;
    case TableAction::InsertRowAfter: edit = InsertTableRow(source, source_caret, true); break;
    case TableAction::DeleteRow: edit = DeleteTableRow(source, source_caret); break;
    case TableAction::InsertColumnBefore: edit = InsertTableColumn(source, source_caret, false); break;
    case TableAction::InsertColumnAfter: edit = InsertTableColumn(source, source_caret, true); break;
    case TableAction::DeleteColumn: edit = DeleteTableColumn(source, source_caret); break;
  }
  if (!edit.changed) {
    MessageBoxW(window_, L"カーソルをMarkdown表のセル内へ移動してください。",
                L"表の編集", MB_ICONINFORMATION);
    return;
  }
  ApplySourceTextWithUndo(view, edit.text);
  const auto view_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(editor, EM_SETSEL, view_caret, view_caret);
}

void Application::MoveOutlineSection(std::size_t source_begin, std::size_t target_begin) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing ||
      source_begin == target_begin ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  SyncDocumentFromEditor(view);
  const std::wstring source = view.document.text();
  const auto edit = MoveHeadingSection(source, source_begin, target_begin);
  if (!edit.changed) return;

  ApplySourceTextWithUndo(view, edit.text);
  const auto view_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(view.editor, EM_SETSEL, view_caret, view_caret);
}

bool Application::SaveRecovery(DocumentView& view, bool interactive) {
  if (!view.workspace_store || view.ime_composing) return false;
  SyncDocumentFromEditor(view);
  if (!view.document.dirty()) return false;
  std::wstring error;
  if (!view.workspace_store->WriteRecovery(view.document.path(), view.document.text(), error)) {
    SetStatusText(error);
    if (interactive) MessageBoxW(window_, error.c_str(), L"復旧保存できません", MB_ICONERROR);
    return false;
  }
  return true;
}

void Application::SaveSession() {
  if (!workspace_store_) return;
  SessionState session;
  session.active_index = active_document_ < documents_.size() ? active_document_ : 0;
  RECT main_rect{};
  GetWindowRect(window_, &main_rect);
  session.main_x = main_rect.left;
  session.main_y = main_rect.top;
  session.main_width = main_rect.right - main_rect.left;
  session.main_height = main_rect.bottom - main_rect.top;
  session.recent_documents = recent_documents_;
  for (auto& view : documents_) {
    SyncDocumentFromEditor(*view);
    CHARRANGE selection{};
    SendMessageW(view->editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    SessionDocument item;
    item.path = view->document.path();
    item.selection_begin = view->editor_snapshot.NativeToSource(selection.cpMin);
    item.selection_end = view->editor_snapshot.NativeToSource(selection.cpMax);
    item.first_visible_line = static_cast<int>(SendMessageW(view->editor, EM_GETFIRSTVISIBLELINE, 0, 0));
    item.compact = view->compact_window != nullptr;
    if (item.compact) {
      RECT compact{};
      GetWindowRect(view->compact_window, &compact);
      item.x = compact.left;
      item.y = compact.top;
      item.width = compact.right - compact.left;
      item.height = compact.bottom - compact.top;
    }
    session.documents.push_back(std::move(item));
  }
  std::wstring error;
  if (!workspace_store_->WriteSessionState(session, error)) {
    MessageBoxW(window_, error.c_str(), L"セッション保存", MB_ICONWARNING);
  }
}

bool Application::IsDocumentOpen(const std::filesystem::path& path) const {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error) return false;
  return std::ranges::any_of(documents_, [&](const auto& view) {
    return view->document.path() == absolute;
  });
}

void Application::CreateEmptyFile() {
  if (workspace_.empty()) return;
  wchar_t path[32768]{};
  const std::wstring initial = (workspace_ / L"新しいノート.md").wstring();
  wcscpy_s(path, initial.c_str());
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrTitle = L"新しいファイル";
  dialog.lpstrFilter = L"Markdown\0*.md\0テキスト\0*.txt\0すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  dialog.Flags = OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
  if (!GetSaveFileNameW(&dialog)) return;
  const std::filesystem::path destination(path);
  std::error_code error;
  const auto parent = std::filesystem::weakly_canonical(destination.parent_path(), error);
  const auto relative = error ? std::filesystem::path{} : std::filesystem::relative(parent, workspace_, error);
  if (error || relative.empty() || relative.native().starts_with(L"..")) {
    MessageBoxW(window_, L"新しいファイルは現在のWorkspace内に作成してください。",
                L"ファイル作成", MB_ICONWARNING);
    return;
  }
  HANDLE file = CreateFileW(destination.c_str(), GENERIC_WRITE, FILE_SHARE_READ, nullptr,
                            CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    MessageBoxW(window_, L"同名ファイルがあるか、作成権限がありません。既存内容は上書きしていません。",
                L"ファイル作成", MB_ICONWARNING);
    return;
  }
  CloseHandle(file);
  PopulateWorkspaceTree();
  OpenDocument(destination);
}

void Application::CreateFolder() {
  if (workspace_.empty()) return;
  auto directory = SelectedTreePath();
  if (directory.empty()) directory = workspace_;
  else if (!std::filesystem::is_directory(directory)) directory = directory.parent_path();
  for (unsigned suffix = 0; suffix < 1000; ++suffix) {
    const std::wstring name = suffix == 0 ? L"新しいフォルダー"
                                          : L"新しいフォルダー (" + std::to_wstring(suffix + 1) + L")";
    std::error_code error;
    if (std::filesystem::create_directory(directory / name, error)) {
      PopulateWorkspaceTree();
      return;
    }
    if (error && error != std::errc::file_exists) {
      MessageBoxW(window_, L"フォルダーを作成できません。", L"フォルダー作成", MB_ICONWARNING);
      return;
    }
  }
}

void Application::CopySelectedFile() {
  copied_files_ = SelectedTreePaths();
  std::erase_if(copied_files_, [&](const auto& path) { return _wcsicmp(path.c_str(), workspace_.c_str()) == 0; });
  if (copied_files_.empty()) {
    MessageBoxW(window_, L"コピーするファイルまたはフォルダーを選択してください。", L"コピー", MB_ICONINFORMATION);
    return;
  }
}

void Application::PasteCopiedFile() {
  if (copied_files_.empty()) return;
  auto directory = SelectedTreePath();
  if (directory.empty()) directory = workspace_;
  else if (!std::filesystem::is_directory(directory)) directory = directory.parent_path();
  for (const auto& source : copied_files_) {
    if (std::filesystem::is_directory(source)) {
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(directory, source, relative_error);
      if (!relative_error && (relative.empty() || !relative.native().starts_with(L".."))) {
        MessageBoxW(window_, L"フォルダーを自分自身または配下へ貼り付けることはできません。",
                    L"貼り付け", MB_ICONWARNING);
        return;
      }
    }
    if (!std::filesystem::exists(source) || std::filesystem::exists(directory / source.filename())) {
      MessageBoxW(window_, L"コピー元が消失したか同名項目があります。何も貼り付けていません。",
                  L"貼り付け", MB_ICONWARNING);
      return;
    }
  }
  std::vector<std::filesystem::path> completed;
  for (const auto& source : copied_files_) {
    std::error_code error;
    std::filesystem::copy(source, directory / source.filename(), std::filesystem::copy_options::recursive, error);
    if (error) {
      MessageBoxW(window_, (L"貼り付けに失敗しました。完了済み: " + std::to_wstring(completed.size()) +
                            L" / " + std::to_wstring(copied_files_.size()) + L"\nerror code: " +
                            std::to_wstring(error.value())).c_str(),
                  L"貼り付け", MB_ICONWARNING);
      PopulateWorkspaceTree();
      return;
    }
    completed.push_back(source);
  }
  PopulateWorkspaceTree();
}

void Application::RenameOrMoveSelected() {
  auto sources = SelectedTreePaths();
  std::erase_if(sources, [&](const auto& path) { return _wcsicmp(path.c_str(), workspace_.c_str()) == 0; });
  if (sources.empty()) {
    MessageBoxW(window_, L"名前変更または移動するファイル／フォルダーを選択してください。",
                L"名前変更・移動", MB_ICONINFORMATION);
    return;
  }
  const auto contains_open_document = [&](const std::filesystem::path& source) {
    return std::ranges::any_of(documents_, [&](const auto& view) {
      if (view->document.path() == source) return true;
      if (!std::filesystem::is_directory(source)) return false;
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(view->document.path(), source, relative_error);
      return !relative_error && !relative.empty() && !relative.native().starts_with(L"..");
    });
  };
  if (std::ranges::any_of(sources, contains_open_document)) {
    MessageBoxW(window_, L"開いている文書は保存してタブを閉じてから移動してください。",
                L"本文保護", MB_ICONWARNING);
    return;
  }
  if (sources.size() > 1) {
    const auto selected_folder = PickFolder(window_, L"選択した項目の移動先", workspace_);
    if (!selected_folder) return;
    std::error_code error;
    const auto destination_directory = std::filesystem::weakly_canonical(*selected_folder, error);
    const auto relative = error ? std::filesystem::path{} :
        std::filesystem::relative(destination_directory, workspace_, error);
    if (error || (destination_directory != workspace_ &&
                  (relative.empty() || relative.native().starts_with(L"..")))) {
      MessageBoxW(window_, L"移動先は現在のWorkspace内を選択してください。",
                  L"名前変更・移動", MB_ICONWARNING);
      return;
    }
    for (const auto& source : sources) {
      const auto destination = destination_directory / source.filename();
      std::error_code nested_error;
      const auto nested = std::filesystem::relative(destination_directory, source, nested_error);
      if (std::filesystem::exists(destination) ||
          (std::filesystem::is_directory(source) && !nested_error &&
           (nested.empty() || !nested.native().starts_with(L"..")))) {
        MessageBoxW(window_, L"同名項目があるか、フォルダーを自身の配下へ移動しようとしています。何も移動していません。",
                    L"名前変更・移動", MB_ICONWARNING);
        return;
      }
    }
    if (MessageBoxW(window_, (std::to_wstring(sources.size()) + L" 項目を次へ移動しますか？\n" +
                              destination_directory.wstring()).c_str(),
                    L"名前変更・移動", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    std::size_t completed{};
    for (const auto& source : sources) {
      const auto destination = destination_directory / source.filename();
      if (!MoveFileExW(source.c_str(), destination.c_str(), MOVEFILE_WRITE_THROUGH)) {
        MessageBoxW(window_, (L"移動に失敗しました。完了済み: " + std::to_wstring(completed) + L" / " +
                              std::to_wstring(sources.size())).c_str(),
                    L"名前変更・移動", MB_ICONWARNING);
        PopulateWorkspaceTree();
        return;
      }
      ++completed;
    }
    PopulateWorkspaceTree();
    return;
  }
  const auto source = sources.front();
  if (std::filesystem::is_directory(source)) {
    std::wstring destination = source.wstring();
    if (!PromptText(window_, instance_, L"フォルダーの名前変更・移動",
                    L"Workspace内の新しい完全path", destination)) return;
    const std::filesystem::path target(destination);
    std::error_code error;
    const auto parent = std::filesystem::weakly_canonical(target.parent_path(), error);
    const auto relative = error ? std::filesystem::path{} : std::filesystem::relative(parent, workspace_, error);
    if (error || (parent != workspace_ && (relative.empty() || relative.native().starts_with(L".."))) ||
        std::filesystem::exists(target) || !MoveFileExW(source.c_str(), target.c_str(), MOVEFILE_WRITE_THROUGH)) {
      MessageBoxW(window_, L"フォルダーを移動できません。同名項目やWorkspace外pathを確認してください。",
                  L"名前変更・移動", MB_ICONWARNING);
      return;
    }
    PopulateWorkspaceTree();
    return;
  }
  wchar_t path[32768]{};
  wcscpy_s(path, source.wstring().c_str());
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrTitle = L"新しい名前または移動先";
  dialog.lpstrFilter = L"すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  dialog.Flags = OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
  if (!GetSaveFileNameW(&dialog)) return;
  const std::filesystem::path destination(path);
  std::error_code error;
  const auto parent = std::filesystem::weakly_canonical(destination.parent_path(), error);
  const auto relative = error ? std::filesystem::path{} : std::filesystem::relative(parent, workspace_, error);
  if (error || relative.empty() || relative.native().starts_with(L"..")) {
    MessageBoxW(window_, L"移動先は現在のWorkspace内を選択してください。",
                L"名前変更・移動", MB_ICONWARNING);
    return;
  }
  if (!MoveFileExW(source.c_str(), destination.c_str(), MOVEFILE_WRITE_THROUGH)) {
    MessageBoxW(window_, L"移動できません。同名の項目がある場合も上書きしません。",
                L"名前変更・移動", MB_ICONWARNING);
    return;
  }
  PopulateWorkspaceTree();
}

void Application::MoveWorkspaceSelectionTo(const std::filesystem::path& directory) {
  if (workspace_drag_sources_.empty()) return;
  std::error_code error;
  const auto target = std::filesystem::weakly_canonical(directory, error);
  const auto workspace_relative = error ? std::filesystem::path{} : std::filesystem::relative(target, workspace_, error);
  if (error || (target != workspace_ && (workspace_relative.empty() || workspace_relative.native().starts_with(L"..")))) {
    MessageBoxW(window_, L"移動先は現在のWorkspace内を選択してください。", L"ドラッグ移動", MB_ICONWARNING);
    return;
  }
  for (const auto& source : workspace_drag_sources_) {
    if (std::ranges::any_of(documents_, [&](const auto& view) {
      if (view->document.path() == source) return true;
      if (!std::filesystem::is_directory(source)) return false;
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(view->document.path(), source, relative_error);
      return !relative_error && !relative.empty() && !relative.native().starts_with(L"..");
    })) {
      MessageBoxW(window_, L"開いている文書またはその親フォルダーは移動できません。", L"本文保護", MB_ICONWARNING);
      return;
    }
    const auto destination = target / source.filename();
    std::error_code relative_error;
    const auto nested = std::filesystem::relative(target, source, relative_error);
    if (destination == source || std::filesystem::exists(destination) ||
        (std::filesystem::is_directory(source) && !relative_error &&
         (nested.empty() || !nested.native().starts_with(L"..")))) {
      MessageBoxW(window_, L"同名項目があるか、フォルダーを自身の配下へ移動しようとしています。",
                  L"ドラッグ移動", MB_ICONWARNING);
      return;
    }
  }
  if (MessageBoxW(window_, (std::to_wstring(workspace_drag_sources_.size()) +
                            L" 項目を次へ移動しますか？\n" + target.wstring()).c_str(),
                  L"ドラッグ移動", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  std::size_t completed{};
  for (const auto& source : workspace_drag_sources_) {
    if (!MoveFileExW(source.c_str(), (target / source.filename()).c_str(), MOVEFILE_WRITE_THROUGH)) {
      MessageBoxW(window_, (L"移動に失敗しました。完了済み: " + std::to_wstring(completed) + L" / " +
                            std::to_wstring(workspace_drag_sources_.size())).c_str(),
                  L"ドラッグ移動", MB_ICONWARNING);
      PopulateWorkspaceTree();
      return;
    }
    ++completed;
  }
  PopulateWorkspaceTree();
}

void Application::DeleteSelected() {
  auto paths = SelectedTreePaths();
  std::erase_if(paths, [&](const auto& path) { return _wcsicmp(path.c_str(), workspace_.c_str()) == 0; });
  if (paths.empty()) return;
  const auto selected = paths;
  std::erase_if(paths, [&](const auto& candidate) {
    return std::ranges::any_of(selected, [&](const auto& parent) {
      if (parent == candidate || !std::filesystem::is_directory(parent)) return false;
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(candidate, parent, relative_error);
      return !relative_error && !relative.empty() && !relative.native().starts_with(L"..");
    });
  });
  const auto is_open_or_parent = [&](const auto& view) {
    return std::ranges::any_of(paths, [&](const auto& path) {
      if (view->document.path() == path) return true;
      if (!std::filesystem::is_directory(path)) return false;
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(view->document.path(), path, relative_error);
      return !relative_error && !relative.empty() && !relative.native().starts_with(L"..");
    });
  };
  if (std::ranges::any_of(documents_, is_open_or_parent)) {
    MessageBoxW(window_, L"開いている文書は保存してタブを閉じてから削除してください。",
                L"本文保護", MB_ICONWARNING);
    return;
  }
  std::wstring from;
  for (const auto& path : paths) { from += path.wstring(); from.push_back(L'\0'); }
  from.push_back(L'\0');
  SHFILEOPSTRUCTW operation{};
  operation.hwnd = window_;
  operation.wFunc = FO_DELETE;
  operation.pFrom = from.c_str();
  operation.fFlags = FOF_ALLOWUNDO | FOF_WANTNUKEWARNING;
  if (SHFileOperationW(&operation) == 0 && !operation.fAnyOperationsAborted) PopulateWorkspaceTree();
}

void Application::CopySelectedPathToClipboard() {
  const auto paths = SelectedTreePaths();
  if (paths.empty() || !OpenClipboard(window_)) return;
  EmptyClipboard();
  std::wstring value;
  for (const auto& path : paths) {
    if (!value.empty()) value += L"\r\n";
    value += path.wstring();
  }
  HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, (value.size() + 1) * sizeof(wchar_t));
  if (memory) {
    void* target = GlobalLock(memory);
    memcpy(target, value.c_str(), (value.size() + 1) * sizeof(wchar_t));
    GlobalUnlock(memory);
    if (!SetClipboardData(CF_UNICODETEXT, memory)) GlobalFree(memory);
  }
  CloseClipboard();
}

void Application::ShowSelectedInExplorer() {
  const auto path = SelectedTreePath();
  if (path.empty()) return;
  PIDLIST_ABSOLUTE item = nullptr;
  if (SUCCEEDED(SHParseDisplayName(path.c_str(), nullptr, &item, 0, nullptr))) {
    SHOpenFolderAndSelectItems(item, 0, nullptr, 0);
    CoTaskMemFree(item);
  }
}

void Application::SetWorkspaceTrust(bool trusted) {
  if (workspace_.empty() || !workspace_store_) return;
  if (trusted && MessageBoxW(window_,
      L"このWorkspaceを信頼すると、明示したGit操作と外部storage adapterを実行できます。\n"
      L"設定ファイルをGitで移しても、この端末localの信頼状態は引き継がれません。信頼しますか？",
      L"Workspace Trust", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  std::wstring error;
  if (!SetWorkspaceTrusted(workspace_, trusted, error)) {
    MessageBoxW(window_, error.c_str(), L"Workspace Trust", MB_ICONERROR);
    return;
  }
  MessageBoxW(window_, trusted ? L"このWorkspaceを信頼しました。" : L"Workspaceの信頼を解除しました。",
              L"Workspace Trust", MB_ICONINFORMATION);
}

void Application::RunGitStatus() {
  if (workspace_.empty()) return;
  if (!IsWorkspaceTrusted(workspace_)) {
    MessageBoxW(window_, L"未信頼Workspaceでは外部processを実行しません。Workspaceメニューから明示的に信頼してください。",
                L"Git", MB_ICONWARNING);
    return;
  }
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) {
    MessageBoxW(window_, L"git.exeが見つかりません。PATHまたはGit for Windowsを確認してください。",
                L"Git", MB_ICONWARNING);
    return;
  }
  GitPanelModel model(git_path, workspace_);
  const auto snapshot = model.Refresh();
  auto state_name = [](GitPanelState state) {
    switch (state) {
      case GitPanelState::NoGit: return L"Git未導入";
      case GitPanelState::NoRepository: return L"repositoryなし";
      case GitPanelState::Ready: return L"準備完了";
      case GitPanelState::NoRemote: return L"remoteなし";
      case GitPanelState::OperationInProgress: return L"操作中";
      case GitPanelState::Error: return L"エラー";
    }
    return L"不明";
  };
  std::wstring output = L"Git\n状態: " + std::wstring(state_name(snapshot.state));
  if (!snapshot.branch.empty()) output += L"\nbranch: " + snapshot.branch;
  if (!snapshot.repository_root.empty()) output += L"\nrepository: " + snapshot.repository_root.wstring();
  output += L"\n変更: staged=" + std::to_wstring(snapshot.has_staged_changes ? 1 : 0) +
            L" / unstaged=" + std::to_wstring(snapshot.has_unstaged_changes ? 1 : 0) +
            L" / untracked=" + std::to_wstring(snapshot.has_untracked_files ? 1 : 0);
  for (const auto& file : snapshot.files) {
    const wchar_t marker = file.conflicted ? L'!' : file.untracked ? L'?' :
        file.staged && file.unstaged ? L'±' : file.staged ? L'+' : L'~';
    output += L"\n" + std::wstring(1, marker) + L" " + file.path.generic_wstring();
  }
  if (!snapshot.error.empty()) output += L"\n" + snapshot.error;
  if (git_panel_) SetWindowTextW(git_panel_, output.c_str());
  SetStatusText(snapshot.error.empty() ? L"Git statusを更新しました。" : snapshot.error);
}

void Application::RunGitAction(int command) {
  if (workspace_.empty()) return;
  if (!IsWorkspaceTrusted(workspace_)) {
    MessageBoxW(window_, L"未信頼WorkspaceではGit操作を実行しません。", L"Git", MB_ICONWARNING);
    return;
  }
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) {
    MessageBoxW(window_, L"git.exeが見つかりません。", L"Git", MB_ICONWARNING);
    return;
  }
  std::vector<std::wstring> arguments{L"-C", workspace_.wstring()};
  std::wstring value;
  bool save_first{};
  std::wstring action;
  switch (command) {
    case kGitDiff: arguments.insert(arguments.end(), {L"diff", L"--no-ext-diff", L"--no-textconv", L"--", L"."}); action = L"差分表示"; break;
    case kGitStageAll: arguments.insert(arguments.end(), {L"add", L"--all", L"--", L"."}); action = L"Workspace内の全変更をステージ"; break;
    case kGitUnstageAll: arguments.insert(arguments.end(), {L"restore", L"--staged", L"--", L"."}); action = L"Workspace内のステージ解除"; break;
    case kGitCommit:
      if (!PromptText(window_, instance_, L"Git commit", L"コミットメッセージ", value) || value.empty()) return;
      // `git commit` without a pathspec consumes only the current index.  A
      // pathspec such as `-- .` would also commit later unstaged changes and
      // violates MDLite's explicit stage-then-commit boundary.
      arguments.insert(arguments.end(), {L"commit", L"-m", value}); action = L"ステージ済み変更をコミット"; break;
    case kGitBranchCreate:
      if (!PromptText(window_, instance_, L"Git branch", L"作成するブランチ名", value) || value.empty()) return;
      arguments.insert(arguments.end(), {L"switch", L"-c", value}); action = L"ブランチ作成・切替"; save_first = true; break;
    case kGitBranchSwitch:
      if (!PromptText(window_, instance_, L"Git switch", L"切り替える既存ブランチ名", value) || value.empty()) return;
      arguments.insert(arguments.end(), {L"switch", L"--", value}); action = L"ブランチ切替"; save_first = true; break;
    case kGitMerge:
      if (!PromptText(window_, instance_, L"Git merge", L"現在のブランチへマージするbranch/ref", value) || value.empty()) return;
      arguments.insert(arguments.end(), {L"merge", L"--no-edit", L"--", value}); action = L"マージ"; save_first = true; break;
    case kGitMergeAbort: arguments.insert(arguments.end(), {L"merge", L"--abort"}); action = L"マージ中止"; save_first = true; break;
    case kGitFetch: arguments.insert(arguments.end(), {L"fetch", L"--prune"}); action = L"Fetch"; break;
    case kGitPull: arguments.insert(arguments.end(), {L"pull", L"--ff-only"}); action = L"Pull"; save_first = true; break;
    case kGitPush: arguments.push_back(L"push"); action = L"Push"; break;
    default: return;
  }
  if (save_first && !SaveAllRequired(true)) {
    MessageBoxW(window_, L"全文書を保存できなかったためGit操作を開始しません。", L"Git", MB_ICONWARNING);
    return;
  }
  std::wstring command_text = L"git";
  for (std::size_t index = 2; index < arguments.size(); ++index) command_text += L" " + arguments[index];
  const auto question = action + L"を実行しますか？\n\n作業ディレクトリ: " + workspace_.wstring() +
                        L"\nコマンド: " + command_text;
  if (MessageBoxW(window_, question.c_str(), L"Git 明示操作", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES)
    return;
  ProcessResult result;
  std::wstring error;
  SetExternalOperationActive(true);
  const bool started = RunProcessWithCancel(window_, git_path, arguments, workspace_, action, result, error);
  SetExternalOperationActive(false);
  if (!started) {
    MessageBoxW(window_, error.c_str(), L"Git", MB_ICONERROR);
    return;
  }
  std::wstring output = L"作業ディレクトリ: " + workspace_.wstring() + L"\nコマンド: " + command_text +
                        L"\n終了コード: " + std::to_wstring(result.exit_code) + L"\n\n" + result.output;
  if (result.truncated) output += L"\n(出力上限で省略しました)";
  if (result.cancelled) output += L"\n(ユーザーがキャンセルしました)";
  if (result.timed_out) output += L"\n(120秒でタイムアウトしました)";
  MessageBoxW(window_, output.c_str(), action.c_str(), result.exit_code == 0 ? MB_ICONINFORMATION : MB_ICONWARNING);
  if (result.exit_code == 0 && save_first) {
    PopulateWorkspaceTree();
    MessageBoxW(window_,
                L"Git操作後の外部変更を検出できるよう、開いている文書の保存指紋は更新していません。"
                L"変更された文書は閉じて開き直してください。",
                L"Git", MB_ICONINFORMATION);
  }
}

bool Application::QueryGitConflicts(std::vector<std::filesystem::path>& files, std::wstring& error) {
  files.clear();
  if (workspace_.empty() || !IsWorkspaceTrusted(workspace_)) {
    error = L"未信頼WorkspaceではGit競合を照会しません。";
    return false;
  }
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) { error = L"git.exeが共通PATHに見つかりません。"; return false; }
  ProcessResult root_result;
  if (!RunProcess(git_path, {L"-C", workspace_.wstring(), L"rev-parse", L"--show-toplevel"},
                  workspace_, 32768, 30000, root_result, error) || root_result.exit_code != 0) {
    if (error.empty()) error = L"Git repository rootを確認できません。\n" + root_result.output;
    return false;
  }
  while (!root_result.output.empty() &&
         (root_result.output.back() == L'\r' || root_result.output.back() == L'\n'))
    root_result.output.pop_back();
  const std::filesystem::path repository_root(root_result.output);
  ProcessResult conflicts;
  if (!RunProcess(git_path, {L"-C", workspace_.wstring(), L"diff", L"--name-only", L"--diff-filter=U", L"-z", L"--", L"."},
                  workspace_, 256 * 1024, 30000, conflicts, error) || conflicts.exit_code != 0) {
    if (error.empty()) error = L"未解決競合を取得できません。\n" + conflicts.output;
    return false;
  }
  std::size_t begin{};
  while (begin < conflicts.output.size()) {
    const auto end = conflicts.output.find(L'\0', begin);
    const auto length = end == std::wstring::npos ? conflicts.output.size() - begin : end - begin;
    if (length != 0) {
      const auto candidate = (repository_root / conflicts.output.substr(begin, length)).lexically_normal();
      std::error_code relative_error;
      const auto relative = std::filesystem::relative(candidate, workspace_, relative_error);
      if (!relative_error && !relative.empty() && !relative.native().starts_with(L"..")) files.push_back(candidate);
    }
    if (end == std::wstring::npos) break;
    begin = end + 1;
  }
  return true;
}

void Application::ShowGitConflicts() {
  std::vector<std::filesystem::path> files;
  std::wstring error;
  if (!QueryGitConflicts(files, error)) { MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return; }
  if (files.empty()) {
    MessageBoxW(window_, L"現在のWorkspace内に未解決競合はありません。", L"Git競合", MB_ICONINFORMATION);
    return;
  }
  std::wstring label = L"Gitの未マージ一覧です。開くWorkspace相対pathを入力してください。\n\n";
  for (const auto& file : files) label += std::filesystem::relative(file, workspace_).generic_wstring() + L"\n";
  std::wstring selected = std::filesystem::relative(files.front(), workspace_).generic_wstring();
  if (!PromptText(window_, instance_, L"未解決競合", label, selected)) return;
  const auto requested = (workspace_ / selected).lexically_normal();
  const auto match = std::ranges::find_if(files, [&](const auto& file) {
    return _wcsicmp(file.c_str(), requested.c_str()) == 0;
  });
  if (match == files.end()) {
    MessageBoxW(window_, L"一覧にあるWorkspace内のpathを指定してください。", L"Git競合", MB_ICONWARNING);
    return;
  }
  OpenDocument(*match);
}

void Application::NavigateGitConflict(bool previous) {
  if (active_document_ >= documents_.size()) return;
  std::vector<std::filesystem::path> files;
  std::wstring error;
  if (!QueryGitConflicts(files, error)) { MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return; }
  auto& view = *documents_[active_document_];
  const bool unmerged = std::ranges::any_of(files, [&](const auto& file) {
    return _wcsicmp(file.c_str(), view.document.path().c_str()) == 0;
  });
  if (!unmerged) {
    MessageBoxW(window_, L"現在の文書はGitの未マージ一覧にありません。", L"Git競合", MB_ICONINFORMATION);
    return;
  }
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const auto source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto block_index = FindConflictBlock(view.document.text(), source_position, previous);
  const auto blocks = ParseConflictBlocks(view.document.text());
  if (!block_index || *block_index >= blocks.size()) {
    MessageBoxW(window_, L"Gitは未マージと報告していますが、本文に解決用markerがありません。binary競合等を確認してください。",
                L"Git競合", MB_ICONWARNING);
    return;
  }
  const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToNative(blocks[*block_index].begin));
  SendMessageW(view.editor, EM_SETSEL, position, position);
  SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(view.editor);
}

void Application::ResolveGitConflict(ConflictChoice choice) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  std::vector<std::filesystem::path> files;
  std::wstring error;
  if (!QueryGitConflicts(files, error)) { MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return; }
  auto& view = *documents_[active_document_];
  if (!std::ranges::any_of(files, [&](const auto& file) { return _wcsicmp(file.c_str(), view.document.path().c_str()) == 0; })) {
    MessageBoxW(window_, L"現在の文書はGitの未マージ一覧にありません。marker文字列だけでは競合と判定しません。",
                L"Git競合", MB_ICONWARNING);
    return;
  }
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const auto source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto block_index = FindConflictBlock(view.document.text(), source_position, false);
  if (!block_index) {
    MessageBoxW(window_, L"解決できるtext競合markerがありません。", L"Git競合", MB_ICONWARNING);
    return;
  }
  const auto edit = ResolveConflictBlock(view.document.text(), *block_index, choice);
  if (!edit.changed) return;
  ApplySourceTextWithUndo(view, edit.text);
  const auto conflict_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(conflict_caret),
               static_cast<LPARAM>(conflict_caret));
  const auto remaining = ParseConflictBlocks(view.document.text()).size();
  MessageBoxW(window_, (L"競合箇所を1つ解決しました。残存marker: " + std::to_wstring(remaining) +
                        L"\n保存後に『解決済みにする』を明示実行してください。").c_str(),
              L"Git競合", MB_ICONINFORMATION);
}

void Application::MarkGitConflictResolved() {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  std::vector<std::filesystem::path> files;
  std::wstring error;
  if (!QueryGitConflicts(files, error)) { MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return; }
  auto& view = *documents_[active_document_];
  if (!std::ranges::any_of(files, [&](const auto& file) { return _wcsicmp(file.c_str(), view.document.path().c_str()) == 0; })) {
    MessageBoxW(window_, L"現在の文書はGitの未マージ一覧にありません。", L"Git競合", MB_ICONINFORMATION);
    return;
  }
  SyncDocumentFromEditor(view);
  const auto marker_count = ParseConflictBlocks(view.document.text()).size();
  if (marker_count != 0 && MessageBoxW(window_,
      (L"競合markerが " + std::to_wstring(marker_count) + L" 件残っています。それでも解決済みとしてstageしますか？").c_str(),
      L"Git競合", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  const auto save_result = SaveDocument(view, SaveIntent::UserRequested);
  if (save_result != SaveResult::Saved && save_result != SaveResult::NoChange) return;
  if (MessageBoxW(window_, (L"次の文書だけをGit indexへ追加し、解決済みにしますか？\n\n" +
                            std::filesystem::relative(view.document.path(), workspace_).generic_wstring()).c_str(),
                  L"Git競合", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) { MessageBoxW(window_, L"git.exeが共通PATHに見つかりません。", L"Git", MB_ICONERROR); return; }
  const auto relative = std::filesystem::relative(view.document.path(), workspace_).generic_wstring();
  ProcessResult result;
  SetExternalOperationActive(true);
  const bool started = RunProcessWithCancel(window_, git_path,
      {L"-C", workspace_.wstring(), L"add", L"--", relative},
      workspace_, L"競合を解決済みにする", result, error);
  SetExternalOperationActive(false);
  if (!started) {
    MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return;
  }
  std::vector<std::filesystem::path> remaining;
  std::wstring query_error;
  const bool queried = QueryGitConflicts(remaining, query_error);
  std::wstring output = L"終了コード: " + std::to_wstring(result.exit_code) + L"\n" + result.output;
  if (result.cancelled) output += L"\nキャンセルしました。";
  if (result.exit_code == 0 && queried) output += L"\nWorkspace内の未解決ファイル: " + std::to_wstring(remaining.size());
  MessageBoxW(window_, output.c_str(), L"Git競合", result.exit_code == 0 ? MB_ICONINFORMATION : MB_ICONWARNING);
}

void Application::SetExternalOperationActive(bool active) {
  external_operation_active_ = active;
  for (const auto& view : documents_)
    if (view->compact_window) EnableWindow(view->compact_window, !active);
}

void Application::LoadAndApplySettings() {
  const auto workspace_path = workspace_store_ ? workspace_store_->metadata_root() / L"settings.toml"
                                                : std::filesystem::path{};
  std::wstring error;
  if (!ResolveSettings(CommonSettingsPath(), workspace_path, settings_, error)) {
    MessageBoxW(window_, error.c_str(), L"設定を読み込めません", MB_ICONWARNING);
    settings_ = {};
    EffectiveSettings fallback;
    std::wstring ignored;
    ResolveSettings({}, {}, fallback, ignored);
    settings_ = std::move(fallback);
  }
  ApplySettings();
  RebuildAccelerators();
  LoadHolidayCache();
  ScheduleHolidayUpdate();
  const ULONGLONG now = GetTickCount64();
  for (auto& view : documents_) {
    if (view->document.dirty())
      view->autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
  }
}

void Application::ApplySettings() {
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  const COLORREF background = ThemeColor(settings_, L"background", dark ? RGB(31, 31, 31) : RGB(255, 255, 255));
  const COLORREF foreground = ThemeColor(settings_, L"foreground", dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
  theme_surface_ = ThemeColor(settings_, L"surface", dark ? RGB(42, 46, 54) : RGB(248, 250, 253));
  theme_surface_alt_ = ThemeColor(settings_, L"surface_alt", dark ? RGB(50, 55, 64) : RGB(241, 245, 249));
  theme_editor_ = ThemeColor(settings_, L"editor_background", dark ? RGB(28, 31, 36) : RGB(252, 253, 255));
  theme_input_ = ThemeColor(settings_, L"input_background", dark ? RGB(36, 40, 47) : RGB(255, 255, 255));
  theme_border_ = ThemeColor(settings_, L"border", dark ? RGB(83, 92, 105) : RGB(210, 218, 228));
  theme_muted_ = ThemeColor(settings_, L"muted", dark ? RGB(170, 180, 194) : RGB(92, 104, 120));
  theme_accent_ = ThemeColor(settings_, L"accent", dark ? RGB(108, 170, 255) : RGB(56, 112, 194));
  HBRUSH replacement_brush = CreateSolidBrush(background);
  if (replacement_brush) {
    if (background_brush_) DeleteObject(background_brush_);
    background_brush_ = replacement_brush;
  }
  auto replace_brush = [](HBRUSH& target, COLORREF color) {
    HBRUSH replacement = CreateSolidBrush(color);
    if (!replacement) return;
    if (target) DeleteObject(target);
    target = replacement;
  };
  replace_brush(surface_brush_, theme_surface_);
  replace_brush(input_brush_, theme_input_);
  replace_brush(editor_brush_, theme_editor_);
  theme_background_ = background;
  theme_foreground_ = foreground;
  dark_theme_ = dark;
  const int height = -MulDiv(static_cast<int>(settings_.font_size_pt),
                             static_cast<int>(GetDpiForWindow(window_)), 72);
  HFONT replacement = CreateFontW(height, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
                                  DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                  CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE,
                                  settings_.font_face.c_str());
  if (replacement) {
    for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_,
                         replace_edit_, find_workspace_, replace_workspace_, find_case_, find_regex_,
                         find_word_, find_include_glob_, find_exclude_glob_, replace_one_,
                         replace_document_, find_results_, calendar_, git_panel_, calendar_details_, panel_headers_[0], panel_headers_[1],
                         panel_headers_[2], panel_headers_[3]})
      if (control) SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(replacement), TRUE);
    for (const auto& view : documents_)
      if (view->editor) SendMessageW(view->editor, WM_SETFONT,
                                     reinterpret_cast<WPARAM>(replacement), TRUE);
  }
  TreeView_SetBkColor(workspace_tree_, theme_surface_);
  TreeView_SetTextColor(workspace_tree_, foreground);
  TreeView_SetBkColor(outline_, theme_surface_);
  TreeView_SetTextColor(outline_, foreground);
  ListView_SetBkColor(find_results_, theme_surface_);
  ListView_SetTextBkColor(find_results_, theme_surface_);
  ListView_SetTextColor(find_results_, foreground);
  for (auto& view : documents_) {
    {
      ScopedEditorChangeSuppression suppression(suppress_editor_change_);
      PresentationUndoGuard guard(view->editor);
      if (!guard) continue;
      CHARRANGE selection{};
      SendMessageW(view->editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
      SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, theme_editor_);
      CHARFORMAT2W format{};
      format.cbSize = sizeof(format);
      format.dwMask = CFM_COLOR | CFM_FACE | CFM_SIZE;
      format.crTextColor = foreground;
      format.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
      wcsncpy_s(format.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
      SendMessageW(view->editor, EM_SETSEL, 0, -1);
      SendMessageW(view->editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
      SendMessageW(view->editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    }
    ApplyMarkdownPresentation(*view, true);
  }
  if (replacement) {
    HFONT previous = editor_font_;
    editor_font_ = replacement;
    if (previous) DeleteObject(previous);
  }
  ApplyChromeTheme();
  InvalidateRect(window_, nullptr, TRUE);
}

void Application::RebuildAccelerators() {
  const std::array commands{
      std::pair{std::wstring_view(L"file.new"), static_cast<WORD>(kFileNew)},
      std::pair{std::wstring_view(L"file.open"), static_cast<WORD>(kFileOpen)},
      std::pair{std::wstring_view(L"file.save"), static_cast<WORD>(kFileSave)},
      std::pair{std::wstring_view(L"file.quickOpen"), static_cast<WORD>(kFileQuickOpen)},
      std::pair{std::wstring_view(L"file.close"), static_cast<WORD>(kFileClose)},
      std::pair{std::wstring_view(L"edit.find"), static_cast<WORD>(kEditFind)},
      std::pair{std::wstring_view(L"edit.findNext"), static_cast<WORD>(kEditFindNext)},
      std::pair{std::wstring_view(L"view.commandPalette"), static_cast<WORD>(kViewCommandPalette)},
  };
  std::vector<ACCEL> accelerators;
  // Panel placement remains available even when the workspace keybinding file
  // is absent or intentionally minimal. The active header selects the panel.
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>('1'),
                               static_cast<WORD>(kViewMoveFocusedLeftTop)});
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>('2'),
                               static_cast<WORD>(kViewMoveFocusedLeftBottom)});
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>('3'),
                               static_cast<WORD>(kViewMoveFocusedRightTop)});
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>('4'),
                               static_cast<WORD>(kViewMoveFocusedRightBottom)});
  for (const auto& [name, command] : commands) {
    const auto value = settings_.keybindings.find(std::wstring(name));
    if (value == settings_.keybindings.end()) continue;
    const auto parsed = ParseAccelerator(value->second, command);
    if (parsed) accelerators.push_back(*parsed);
  }
  HACCEL replacement = accelerators.empty() ? nullptr :
      CreateAcceleratorTableW(accelerators.data(), static_cast<int>(accelerators.size()));
  if (accelerator_table_) DestroyAcceleratorTable(accelerator_table_);
  accelerator_table_ = replacement;
}

void Application::OpenWorkspaceSettings() {
  const auto common_path = CommonSettingsPath();
  const auto workspace_path = workspace_store_ ? workspace_store_->metadata_root() / L"settings.toml"
                                                : std::filesystem::path{};
  std::wstring scope = workspace_store_ ? L"workspace" : L"common";
  const auto target = scope == L"common" ? common_path : workspace_path;
  if (target.empty()) return;
  SettingsLayer layer;
  std::wstring error;
  if (!LoadSettingsLayer(target, layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return; }
  std::wstring colors;
  for (const auto& [name, value] : layer.colors) colors += name + L"=" + value + L"\r\n";
  std::wstring bindings;
  for (const auto& [command, shortcut] : layer.keybindings) bindings += command + L"=" + shortcut + L"\r\n";
  std::vector<NativeFormField> fields{
      {L"編集範囲（Applyで一括保存）", scope, NativeFormFieldKind::Combo,
       {L"common", L"workspace", L"reset-common", L"reset-workspace"}},
      {L"自動保存", layer.auto_save ? (*layer.auto_save ? L"on" : L"off") : L"inherit",
       NativeFormFieldKind::Combo, {L"inherit", L"on", L"off"}},
      {L"自動保存待機時間（100〜60000ms、inherit可）",
       layer.auto_save_delay_ms ? std::to_wstring(*layer.auto_save_delay_ms) : L"inherit"},
      {L"テーマ", layer.theme ? ThemeName(*layer.theme) : L"inherit", NativeFormFieldKind::Combo,
       {L"inherit", L"system", L"light", L"dark", L"custom"}},
      {L"フォント名（inherit可）", layer.font_face.value_or(L"inherit")},
      {L"フォントサイズ（6〜96pt、inherit可）",
       layer.font_size_pt ? std::to_wstring(*layer.font_size_pt) : L"inherit"},
      {L"祝日更新許可（commonのみ。inherit可）",
       layer.holiday_auto_update ? (*layer.holiday_auto_update ? L"on" : L"off") : L"inherit",
       NativeFormFieldKind::Combo, {L"inherit", L"on", L"off"}},
      {L"既定メモWorkspaceの絶対path（inherit可）",
       layer.default_memo_workspace ? layer.default_memo_workspace->wstring() : L"inherit"},
      {L"カスタム色（1行1件: name=#RRGGBB。削除は行を消す）", colors, NativeFormFieldKind::Multiline},
      {L"キー割当て（1行1件: command=shortcut。noneで解除）", bindings, NativeFormFieldKind::Multiline},
  };
  if (!RunNativeForm(window_, instance_, L"MDLite 設定", fields)) return;
  scope = fields[0].value;
  std::ranges::transform(scope, scope.begin(), towlower);
  if (scope == L"reset-common" || scope == L"reset-workspace") {
    const auto reset_target = scope == L"reset-common" ? common_path : workspace_path;
    if (reset_target.empty()) {
      MessageBoxW(window_, L"Workspaceが開かれていません。", L"設定", MB_ICONWARNING); return;
    }
    if (MessageBoxW(window_, (reset_target.wstring() + L"\nの上書きを解除して継承へ戻しますか？").c_str(),
                    L"設定を既定へ戻す", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    std::error_code remove_error;
    std::filesystem::remove(reset_target, remove_error);
    if (remove_error) {
      MessageBoxW(window_, L"設定上書きを削除できません。", L"設定", MB_ICONERROR); return;
    }
    LoadAndApplySettings();
    return;
  }
  if ((scope == L"workspace" && workspace_path.empty()) || (scope != L"common" && scope != L"workspace")) {
    MessageBoxW(window_, L"保存先の編集範囲が不正か、Workspaceが開かれていません。", L"設定", MB_ICONWARNING);
    return;
  }
  const auto save_target = scope == L"common" ? common_path : workspace_path;
  // The form starts from the selected layer.  If the user changes scope, load
  // that layer now and apply the entered values as an explicit override.
  if (save_target != target && !LoadSettingsLayer(save_target, layer, error)) {
    MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return;
  }
  auto lower = [](std::wstring value) { std::ranges::transform(value, value.begin(), towlower); return value; };
  const auto auto_save = lower(fields[1].value);
  if (auto_save == L"inherit") layer.auto_save.reset();
  else if (auto_save == L"on") layer.auto_save = true;
  else if (auto_save == L"off") layer.auto_save = false;
  else { MessageBoxW(window_, L"自動保存の値が不正です。", L"設定", MB_ICONWARNING); return; }
  const auto delay = lower(fields[2].value);
  if (delay == L"inherit") layer.auto_save_delay_ms.reset();
  else {
    try { std::size_t consumed{}; const auto parsed = std::stoul(delay, &consumed);
      if (consumed != delay.size()) throw std::invalid_argument("trailing characters");
      layer.auto_save_delay_ms = static_cast<unsigned>(parsed);
    } catch (...) { MessageBoxW(window_, L"自動保存待機時間が数値ではありません。", L"設定", MB_ICONWARNING); return; }
  }
  const auto theme = lower(fields[3].value);
  if (theme == L"inherit") layer.theme.reset();
  else {
    const auto parsed = ParseTheme(theme);
    if (!parsed) { MessageBoxW(window_, L"テーマの値が不正です。", L"設定", MB_ICONWARNING); return; }
    layer.theme = *parsed;
  }
  const auto font = fields[4].value;
  if (lower(font) == L"inherit") layer.font_face.reset(); else layer.font_face = font;
  const auto size = lower(fields[5].value);
  if (size == L"inherit") layer.font_size_pt.reset();
  else {
    try { std::size_t consumed{}; const auto parsed = std::stoul(size, &consumed);
      if (consumed != size.size()) throw std::invalid_argument("trailing characters");
      layer.font_size_pt = static_cast<unsigned>(parsed);
    } catch (...) { MessageBoxW(window_, L"フォントサイズが数値ではありません。", L"設定", MB_ICONWARNING); return; }
  }
  if (scope == L"common") {
    const auto holiday = lower(fields[6].value);
    if (holiday == L"inherit") layer.holiday_auto_update.reset();
    else if (holiday == L"on") layer.holiday_auto_update = true;
    else if (holiday == L"off") layer.holiday_auto_update = false;
    else { MessageBoxW(window_, L"祝日更新許可の値が不正です。", L"設定", MB_ICONWARNING); return; }
    const auto memo = fields[7].value;
    if (lower(memo) == L"inherit" || memo.empty()) layer.default_memo_workspace.reset();
    else layer.default_memo_workspace = std::filesystem::path(memo);
  }
  layer.colors.clear();
  {
    std::wistringstream stream(fields[8].value);
    std::wstring line;
    while (std::getline(stream, line)) {
      if (!line.empty() && line.back() == L'\r') line.pop_back();
      if (line.empty()) continue;
      const auto equals = line.find(L'=');
      if (equals == std::wstring::npos) { MessageBoxW(window_, L"カスタム色はname=#RRGGBB形式で入力してください。", L"設定", MB_ICONWARNING); return; }
      const auto name = line.substr(0, equals);
      const auto value = line.substr(equals + 1);
      if (value != L"inherit") layer.colors[name] = value;
    }
  }
  layer.keybindings.clear();
  {
    std::wistringstream stream(fields[9].value);
    std::wstring line;
    while (std::getline(stream, line)) {
      if (!line.empty() && line.back() == L'\r') line.pop_back();
      if (line.empty()) continue;
      const auto equals = line.find(L'=');
      if (equals == std::wstring::npos) { MessageBoxW(window_, L"キー割当てはcommand=shortcut形式で入力してください。", L"設定", MB_ICONWARNING); return; }
      layer.keybindings[line.substr(0, equals)] = line.substr(equals + 1);
    }
  }
  if (!ValidateSettingsLayer(layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONWARNING); return; }
  SettingsLayer common, workspace_layer;
  if (!LoadSettingsLayer(common_path, common, error) ||
      (!workspace_path.empty() && !LoadSettingsLayer(workspace_path, workspace_layer, error))) {
    MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return;
  }
  if (scope == L"common") common = layer; else workspace_layer = layer;
  auto effective_bindings = DefaultSettingsLayer().keybindings;
  for (const auto& item : common.keybindings) effective_bindings[item.first] = item.second;
  for (const auto& item : workspace_layer.keybindings) effective_bindings[item.first] = item.second;
  if (!ValidateKeybindingConflicts(effective_bindings, error)) {
    MessageBoxW(window_, error.c_str(), L"キー割当て競合", MB_ICONWARNING); return;
  }
  if (!SaveSettingsLayer(save_target, layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return; }
  LoadAndApplySettings();
  if (!TestAutomationSilent())
    MessageBoxW(window_, L"設定を保存して適用しました。", L"設定", MB_ICONINFORMATION);
}

void Application::OpenWorkspaceSettingsFiles() {
  if (!workspace_store_) return;
  const auto root = workspace_store_->metadata_root();
  for (const auto& relative : {L"settings.toml", L"workspace.toml", L"profiles.toml", L"keybindings.toml", L"commands.toml"}) {
    const auto path = root / relative;
    if (std::filesystem::is_regular_file(path)) OpenDocument(path);
  }
}

void Application::ManageProfiles() {
  if (!workspace_store_) {
    MessageBoxW(window_, L"保存先previewとtemplate編集のため、先にWorkspaceを開いてください。",
                L"作成プロファイル", MB_ICONINFORMATION);
    return;
  }
  const auto common_path = CommonProfilesPath();
  const auto workspace_path = workspace_store_->metadata_root() / L"profiles.toml";
  std::vector<ProfileDefinition> effective;
  std::wstring error;
  if (!ResolveProfiles(common_path, workspace_path, effective, error)) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR);
    return;
  }
  std::wstring scope = L"workspace";
  ProfileDefinition seed;
  if (const auto daily = std::ranges::find_if(effective, [](const auto& item) { return item.id == L"daily"; });
      daily != effective.end()) seed = *daily;
  else if (!effective.empty()) seed = effective.front();
  else seed = {L"custom", L"Custom", L"Notes/{{date:yyyy}}", L"{{date:yyyyMMdd}}.md",
               L"templates/memo.md", ProfileCollision::Sequence};
  std::wstring inputs;
  for (const auto& input : seed.inputs)
    inputs += input.id + L"|" + input.label + L"|" + (input.required ? L"yes" : L"no") + L"|" + input.default_value + L"\r\n";
  std::vector<NativeFormField> fields{
      {L"編集範囲（Applyで保存）", scope, NativeFormFieldKind::Combo, {L"common", L"workspace"}},
      {L"操作", L"edit", NativeFormFieldKind::Combo, {L"add", L"edit", L"duplicate", L"delete", L"template"}},
      {L"元のprofile id（edit／template／delete／duplicate）", seed.id},
      {L"保存するprofile id（add／edit／duplicate）", seed.id},
      {L"表示名", seed.name},
      {L"Workspace相対directory", seed.directory.generic_wstring()},
      {L"filename", seed.filename.generic_wstring()},
      {L".mdlite相対template path", seed.template_path.generic_wstring()},
      {L"collision", seed.collision == ProfileCollision::Sequence ? L"sequence" : L"open-existing",
       NativeFormFieldKind::Combo, {L"open-existing", L"sequence"}},
      {L"入力項目（1行: id|表示名|required(yes/no)|既定値、最大8行）", inputs, NativeFormFieldKind::Multiline},
  };
  if (!RunNativeForm(window_, instance_, L"作成プロファイル", fields)) return;
  scope = fields[0].value;
  std::ranges::transform(scope, scope.begin(), towlower);
  const auto action = [&] { auto value = fields[1].value; std::ranges::transform(value, value.begin(), towlower); return value; }();
  const auto source_id = fields[2].value;
  const auto id = fields[3].value;
  if ((scope != L"common" && scope != L"workspace") || source_id.empty() || id.empty()) {
    MessageBoxW(window_, L"編集範囲またはprofile idが不正です。", L"作成プロファイル", MB_ICONWARNING); return;
  }
  const auto target = scope == L"common" ? common_path : workspace_path;
  std::vector<ProfileDefinition> layer;
  if (!LoadProfileFile(target, layer, error)) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR); return;
  }
  const auto effective_item = std::ranges::find_if(effective, [&](const auto& item) { return item.id == source_id; });
  if (action == L"template") {
    if (effective_item == effective.end()) {
      MessageBoxW(window_, L"指定したprofileが見つかりません。", L"作成プロファイル", MB_ICONWARNING); return;
    }
    const auto template_path = workspace_store_->metadata_root() / effective_item->template_path;
    if (!std::filesystem::is_regular_file(template_path)) {
      MessageBoxW(window_, (L"template fileがありません:\n" + template_path.wstring()).c_str(),
                  L"作成プロファイル", MB_ICONWARNING); return;
    }
    OpenDocument(template_path); return;
  }
  if (action == L"delete") {
    const auto item = std::ranges::find_if(layer, [&](const auto& profile) { return profile.id == source_id; });
    if (item == layer.end()) {
      MessageBoxW(window_, L"この範囲には定義がありません。", L"作成プロファイル", MB_ICONINFORMATION); return;
    }
    if (MessageBoxW(window_, (L"この範囲のprofile定義を削除しますか？\n" + source_id).c_str(),
                    L"作成プロファイル", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    layer.erase(item);
    if (!SaveProfileFile(target, layer, error)) {
      MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR); return;
    }
    return;
  }
  if (action != L"add" && action != L"edit" && action != L"duplicate") {
    MessageBoxW(window_, L"未対応の操作です。", L"作成プロファイル", MB_ICONWARNING); return;
  }
  if (action == L"edit" && source_id != id) {
    MessageBoxW(window_, L"editでは元のprofile idと保存するidを一致させてください。",
                L"作成プロファイル", MB_ICONWARNING); return;
  }
  ProfileDefinition profile;
  if (action == L"add") {
    if (std::ranges::any_of(effective, [&](const auto& item) { return item.id == id; })) {
      MessageBoxW(window_, L"既存idです。上書きする場合はeditを選んでください。", L"作成プロファイル", MB_ICONWARNING); return;
    }
    profile = {id, id, L"Notes/{{date:yyyy}}", L"{{date:yyyyMMdd}}.md", L"templates/memo.md", ProfileCollision::Sequence};
  } else {
    if (effective_item == effective.end()) {
      MessageBoxW(window_, L"指定したprofileが見つかりません。", L"作成プロファイル", MB_ICONWARNING); return;
    }
    profile = *effective_item;
    if (action == L"duplicate") {
      if (std::ranges::any_of(effective, [&](const auto& item) { return item.id == id; })) {
        MessageBoxW(window_, L"複製先idは既に存在します。", L"作成プロファイル", MB_ICONWARNING); return;
      }
      profile.id = id; profile.name += L" コピー";
    }
  }
  profile.id = id;
  profile.name = fields[4].value;
  profile.directory = fields[5].value;
  profile.filename = fields[6].value;
  profile.template_path = fields[7].value;
  auto collision = fields[8].value;
  std::ranges::transform(collision, collision.begin(), towlower);
  if (collision == L"sequence") profile.collision = ProfileCollision::Sequence;
  else if (collision == L"open-existing") profile.collision = ProfileCollision::OpenExisting;
  else { MessageBoxW(window_, L"collision値が不正です。", L"作成プロファイル", MB_ICONWARNING); return; }
  profile.inputs.clear();
  std::wistringstream input_stream(fields[9].value);
  std::wstring line;
  while (std::getline(input_stream, line)) {
    if (!line.empty() && line.back() == L'\r') line.pop_back();
    if (line.empty()) continue;
    std::array<std::wstring, 4> parts;
    std::size_t begin{};
    for (std::size_t index = 0; index < parts.size(); ++index) {
      const auto end = line.find(L'|', begin);
      parts[index] = line.substr(begin, end == std::wstring::npos ? line.size() - begin : end - begin);
      if (end == std::wstring::npos) { begin = line.size(); break; }
      begin = end + 1;
    }
    if (parts[0].empty() || parts[1].empty() || (parts[2] != L"yes" && parts[2] != L"no") || profile.inputs.size() >= 8) {
      MessageBoxW(window_, L"入力項目は id|表示名|required(yes/no)|既定値 の形式で最大8行です。", L"作成プロファイル", MB_ICONWARNING); return;
    }
    profile.inputs.push_back({parts[0], parts[1], parts[3], parts[2] == L"yes"});
  }
  if (!ValidateProfile(profile, error)) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONWARNING); return;
  }
  SYSTEMTIME now{};
  GetLocalTime(&now);
  const auto preview = PreviewProfilePath(workspace_, profile, now, error);
  if (!preview) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONWARNING);
    return;
  }
  if (MessageBoxW(window_, (L"次の定義を保存しますか？\n\n範囲: " + scope + L"\nid: " + profile.id +
                            L"\n今日の保存先preview:\n" + preview->wstring()).c_str(),
                  L"作成プロファイル", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON1) != IDYES) return;
  const auto existing = std::ranges::find_if(layer, [&](const auto& item) { return item.id == profile.id; });
  if (existing == layer.end()) layer.push_back(profile);
  else *existing = profile;
  if (!SaveProfileFile(target, layer, error)) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR);
    return;
  }
  MessageBoxW(window_, L"profileを保存しました。template操作で本文と{{cursor}}を編集できます。",
              L"作成プロファイル", MB_ICONINFORMATION);
}

void Application::ShowCommandPalette() {
  struct Entry { const wchar_t* name; int command; bool enabled; const wchar_t* reason; };
  const bool has_document = active_document_ < documents_.size();
  const bool has_workspace = !workspace_.empty();
  const bool trusted = has_workspace && IsWorkspaceTrusted(workspace_);
  const std::array entries{
      Entry{L"ファイル: 新規無題文書", kFileNew, true, L""},
      Entry{L"ファイル: 開く", kFileOpen, true, L""},
      Entry{L"ファイル: Quick Open", kFileQuickOpen, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: 保存", kFileSave, has_document, L"文書が開かれていません"},
      Entry{L"編集: 検索", kEditFind, has_document, L"文書が開かれていません"},
      Entry{L"編集: Workspace検索", kEditFindWorkspace, has_workspace, L"Workspaceが未選択です"},
      Entry{L"表示: コンパクト表示", kViewCompact, has_document, L"文書が開かれていません"},
      Entry{L"カレンダー: 祝日CSVをローカル取込み", kCalendarImportHolidays, true, L""},
      Entry{L"カレンダー: 祝日を内閣府から今すぐ確認", kCalendarUpdateHolidays, true, L""},
      Entry{L"表示: Workspace設定", kViewSettings, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Git: Status / Branches", kGitStatus, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: ブランチ切替", kGitBranchSwitch, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Merge", kGitMerge, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Pull (fast-forward only)", kGitPull, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"画像: 表示幅480 DIP", kImageWidth480, has_document, L"Markdown文書が必要です"},
      Entry{L"画像: Storageへupload", kImageUpload, trusted && has_document, L"信頼済みWorkspaceと文書が必要です"},
  };
  std::vector<NativePickerItem> items;
  items.reserve(entries.size());
  for (const auto& entry : entries) {
    std::wstring label = entry.name;
    if (!entry.enabled) label += L"（実行不可: " + std::wstring(entry.reason) + L"）";
    items.push_back(NativePickerItem{std::move(label)});
  }
  std::size_t selected{};
  if (!RunNativePicker(window_, instance_, L"MDLite コマンドパレット",
                       L"コマンド名の一部を入力し、実行する項目を選択してください。",
                       items, selected) || selected >= entries.size()) return;
  const auto& match = entries[selected];
  if (!match.enabled) {
    MessageBoxW(window_, match.reason, L"このコマンドは実行できません", MB_ICONWARNING);
    return;
  }
  SendMessageW(window_, WM_COMMAND, MAKEWPARAM(match.command, 0), 0);
}

void Application::ToggleCompactWindow() {
  if (active_document_ >= documents_.size()) return;
  auto& view = *documents_[active_document_];
  if (view.compact_window) {
    SendMessageW(view.compact_window, WM_CLOSE, 0, 0);
    return;
  }
  const auto title = view.document.path().filename().wstring() + L" — MDLite コンパクト";
  view.compact_window = CreateWindowExW(WS_EX_TOOLWINDOW, kCompactWindowClass, title.c_str(),
                                         WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_CLIPCHILDREN,
                                         CW_USEDEFAULT, CW_USEDEFAULT, 720, 520, window_, nullptr,
                                         instance_, this);
  if (!view.compact_window) {
    MessageBoxW(window_, L"コンパクトウィンドウを作成できません。", L"コンパクト表示",
                MB_ICONERROR);
    return;
  }
  SetParent(view.editor, view.compact_window);
  ShowWindow(view.editor, SW_SHOW);
  RECT client{};
  GetClientRect(view.compact_window, &client);
  MoveWindow(view.editor, 0, 0, client.right, client.bottom, TRUE);
  SetFocus(view.editor);
}

bool Application::PasteClipboardImage() {
  if (workspace_.empty() || active_document_ >= documents_.size() ||
      documents_[active_document_]->ime_composing) return false;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path()) || !IsClipboardFormatAvailable(CF_BITMAP)) return false;
  if (!OpenClipboard(window_)) return true;
  HBITMAP bitmap = static_cast<HBITMAP>(GetClipboardData(CF_BITMAP));
  if (bitmap == nullptr) {
    CloseClipboard();
    MessageBoxW(window_, L"クリップボード画像を読み取れません。本文は変更していません。",
                L"画像の貼り付け", MB_ICONWARNING);
    return true;
  }

  std::error_code filesystem_error;
  const auto directory = workspace_ / L"assets";
  std::filesystem::create_directories(directory, filesystem_error);
  std::filesystem::path path = directory / L"clipboard.png";
  for (unsigned suffix = 1; std::filesystem::exists(path); ++suffix)
    path = directory / (L"clipboard_" + std::to_wstring(suffix) + L".png");

  IWICImagingFactory* factory{};
  IWICBitmap* source{};
  IWICStream* stream{};
  IWICBitmapEncoder* encoder{};
  IWICBitmapFrameEncode* frame{};
  IPropertyBag2* properties{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) result = factory->CreateBitmapFromHBITMAP(bitmap, nullptr,
                                                                   WICBitmapUseAlpha, &source);
  if (SUCCEEDED(result)) result = factory->CreateStream(&stream);
  if (SUCCEEDED(result)) result = stream->InitializeFromFilename(path.c_str(), GENERIC_WRITE);
  if (SUCCEEDED(result)) result = factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder);
  if (SUCCEEDED(result)) result = encoder->Initialize(stream, WICBitmapEncoderNoCache);
  if (SUCCEEDED(result)) result = encoder->CreateNewFrame(&frame, &properties);
  if (SUCCEEDED(result)) result = frame->Initialize(properties);
  UINT width{}, height{};
  if (SUCCEEDED(result)) result = source->GetSize(&width, &height);
  if (SUCCEEDED(result)) result = frame->SetSize(width, height);
  WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
  if (SUCCEEDED(result)) result = frame->SetPixelFormat(&format);
  if (SUCCEEDED(result)) result = frame->WriteSource(source, nullptr);
  if (SUCCEEDED(result)) result = frame->Commit();
  if (SUCCEEDED(result)) result = encoder->Commit();
  if (properties) properties->Release();
  if (frame) frame->Release();
  if (encoder) encoder->Release();
  if (stream) stream->Release();
  if (source) source->Release();
  if (factory) factory->Release();
  CloseClipboard();

  if (FAILED(result)) {
    std::filesystem::remove(path, filesystem_error);
    MessageBoxW(window_, L"クリップボード画像をPNGとして保存できません。本文は変更していません。",
                L"画像の貼り付け", MB_ICONWARNING);
    return true;
  }
  const auto relative = std::filesystem::relative(path, view.document.path().parent_path(), filesystem_error);
  if (filesystem_error) {
    std::filesystem::remove(path, filesystem_error);
    MessageBoxW(window_, L"画像への相対パスを作成できません。本文は変更していません。",
                L"画像の貼り付け", MB_ICONWARNING);
    return true;
  }
  const auto markup = ImageMarkdown(L"clipboard", relative.generic_wstring());
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(markup.c_str()));
  PopulateWorkspaceTree();
  return true;
}

void Application::ResizeImageAtCaret(unsigned width_dip) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const auto source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto parsed = ParseMarkdown(view.document.text());
  const auto image = std::ranges::find_if(parsed.images, [&](const auto& item) {
    return source_position >= item.begin && source_position <= item.end;
  });
  if (image == parsed.images.end()) {
    MessageBoxW(window_, L"カーソルをMarkdown画像の上へ移動してください。", L"画像サイズ",
                MB_ICONINFORMATION);
    return;
  }
  const auto replacement = ImageHtml(image->alternate_text, image->target, width_dip);
  const auto begin = static_cast<LONG>(view.editor_snapshot.SourceToNative(image->begin));
  const auto end = static_cast<LONG>(view.editor_snapshot.SourceToNative(image->end));
  SendMessageW(view.editor, EM_SETSEL, begin, end);
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(replacement.c_str()));
}

void Application::UploadImageAtCaret() {
  if (active_document_ >= documents_.size() || !workspace_store_ ||
      documents_[active_document_]->ime_composing) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  if (!IsWorkspaceTrusted(workspace_)) {
    MessageBoxW(window_, L"未信頼Workspaceではstorage adapterを実行しません。", L"画像upload", MB_ICONWARNING);
    return;
  }
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto parsed = ParseMarkdown(view.document.text());
  auto image = std::ranges::find_if(parsed.images, [&](const auto& item) {
    return source_position >= item.begin && source_position <= item.end;
  });
  if (image == parsed.images.end()) {
    MessageBoxW(window_, L"カーソルをlocal画像の上へ移動してください。", L"画像upload", MB_ICONINFORMATION);
    return;
  }
  const auto asset = (view.document.path().parent_path() / image->target).lexically_normal();
  StorageAdapter adapter;
  std::wstring error;
  if (!LoadStorageAdapter(workspace_store_->metadata_root() / L"storage.toml", adapter, error)) {
    MessageBoxW(window_, error.c_str(), L"画像upload", MB_ICONWARNING);
    return;
  }
  if (MessageBoxW(window_, (L"次のlocal画像を設定済みadapterへ渡します。\n" + asset.wstring() +
      L"\n資格情報はMDLiteへ保存しません。続行しますか？").c_str(), L"画像upload",
      MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  StorageUploadResult uploaded;
  if (!UploadWithStorageAdapter(adapter, asset, std::to_wstring(view.document.revision()), nullptr,
                                uploaded, error)) {
    MessageBoxW(window_, error.c_str(), L"画像upload", MB_ICONWARNING);
    return;
  }
  std::wstring source = view.document.text();
  const auto target_begin = source.find(image->target, image->begin);
  if (target_begin == std::wstring::npos || target_begin >= image->end) return;
  source.replace(target_begin, image->target.size(), uploaded.reference);
  ApplySourceTextWithUndo(view, std::move(source));
}

void Application::OpenLinkAtSourcePosition(DocumentView& view, std::size_t source_position, bool activate) {
  SyncDocumentFromEditor(view);
  const auto parsed = ParseMarkdown(view.document.text());
  const auto link = std::ranges::find_if(parsed.links, [&](const auto& item) {
    return source_position >= item.begin && source_position < item.end;
  });
  if (link == parsed.links.end()) return;
  if (!activate) {
    const std::wstring status = L"リンク: " + link->target;
    SetStatusText(status);
    return;
  }
  std::wstring target = link->target;
  if (target.starts_with(L"http://") || target.starts_with(L"https://") || target.starts_with(L"mailto:")) {
    if (MessageBoxW(window_, (L"外部リンクを既定のアプリで開きますか？\n\n" + target).c_str(),
                    L"外部リンク", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    if (reinterpret_cast<INT_PTR>(ShellExecuteW(window_, L"open", target.c_str(), nullptr, nullptr, SW_SHOWNORMAL)) <= 32)
      MessageBoxW(window_, L"外部リンクを開けませんでした。", L"リンク", MB_ICONWARNING);
    return;
  }
  const auto hash = target.find(L'#');
  const std::wstring fragment = hash == std::wstring::npos ? std::wstring{} : UrlDecode(target.substr(hash + 1));
  std::wstring path_text = UrlDecode(hash == std::wstring::npos ? target : target.substr(0, hash));
  std::filesystem::path destination = path_text.empty() ? view.document.path()
      : (view.document.path().parent_path() / std::filesystem::path(path_text)).lexically_normal();
  std::error_code canonical_error;
  const auto canonical = std::filesystem::weakly_canonical(destination, canonical_error);
  if (canonical_error || !std::filesystem::is_regular_file(canonical)) {
    MessageBoxW(window_, (L"リンク先ファイルが見つかりません。\n" + destination.wstring()).c_str(),
                L"リンク", MB_ICONWARNING);
    return;
  }
  if (!workspace_.empty()) {
    std::error_code relative_error;
    const auto relative = std::filesystem::relative(canonical, workspace_, relative_error);
    if ((relative_error || relative.empty() || relative.native().starts_with(L"..")) &&
        MessageBoxW(window_, (L"Workspace外のファイルを開きますか？\n\n" + canonical.wstring()).c_str(),
                    L"リンク", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  }
  OpenDocument(canonical);
  if (fragment.empty() || active_document_ >= documents_.size()) return;
  auto& destination_view = *documents_[active_document_];
  const auto destination_parse = ParseMarkdown(destination_view.document.text());
  const auto heading = std::ranges::find_if(destination_parse.headings, [&](const auto& item) {
    return MarkdownAnchor(item.text) == MarkdownAnchor(fragment);
  });
  if (heading == destination_parse.headings.end()) {
    MessageBoxW(window_, (L"見出しanchorが見つかりません: #" + fragment).c_str(), L"リンク", MB_ICONINFORMATION);
    return;
  }
  const LONG position = static_cast<LONG>(destination_view.editor_snapshot.SourceToNative(heading->begin));
  SendMessageW(destination_view.editor, EM_SETSEL, position, position);
  SendMessageW(destination_view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(destination_view.editor);
}

std::filesystem::path Application::SelectedTreePath() const {
  TVITEMW item{};
  item.mask = TVIF_PARAM;
  item.hItem = TreeView_GetSelection(workspace_tree_);
  if (item.hItem == nullptr || !TreeView_GetItem(workspace_tree_, &item) || item.lParam == 0) return {};
  return *reinterpret_cast<const std::filesystem::path*>(item.lParam);
}

std::vector<std::filesystem::path> Application::SelectedTreePaths() const {
  std::vector<std::filesystem::path> paths;
  HTREEITEM item = TreeView_GetRoot(workspace_tree_);
  while (item) {
    TVITEMW tree_item{};
    tree_item.mask = TVIF_PARAM | TVIF_STATE;
    tree_item.stateMask = TVIS_SELECTED;
    tree_item.hItem = item;
    if (TreeView_GetItem(workspace_tree_, &tree_item) && (tree_item.state & TVIS_SELECTED) != 0 && tree_item.lParam != 0)
      paths.push_back(*reinterpret_cast<const std::filesystem::path*>(tree_item.lParam));
    if (const HTREEITEM child = TreeView_GetChild(workspace_tree_, item)) {
      item = child;
      continue;
    }
    while (item && TreeView_GetNextSibling(workspace_tree_, item) == nullptr)
      item = TreeView_GetParent(workspace_tree_, item);
    if (item) item = TreeView_GetNextSibling(workspace_tree_, item);
  }
  if (paths.empty()) {
    const auto primary = SelectedTreePath();
    if (!primary.empty()) paths.push_back(primary);
  }
  return paths;
}

void Application::ResizeFocusedPanel(double delta) {
  auto* panel = panel_layout_.Find(focused_panel_);
  if (!panel || !std::isfinite(delta)) return;
  const double width = panel->width + delta;
  const double height = panel->height + delta;
  std::wstring error;
  if (!panel_layout_.Resize(focused_panel_, width, height, error)) {
    SetStatusText(error.empty() ? L"パネル寸法を変更できません。" : error);
    return;
  }
  SavePanelLayout();
  LayoutControls();
  SetStatusText(std::wstring(PanelName(focused_panel_)) + L"の寸法を変更しました。");
}
void Application::UpdateCalendarDetails(const SYSTEMTIME& date) {
  if (!calendar_details_) return;
  const CalendarDate selected{static_cast<int>(date.wYear), static_cast<int>(date.wMonth),
                              static_cast<int>(date.wDay)};
  std::wstring output = L"選択日: " + std::to_wstring(selected.year) + L"-" +
      (selected.month < 10 ? L"0" : L"") + std::to_wstring(selected.month) + L"-" +
      (selected.day < 10 ? L"0" : L"") + std::to_wstring(selected.day);
  if (const auto holiday = JapaneseHolidayName(selected.year, selected.month, selected.day))
    output += L"\n祝日: " + std::wstring(*holiday);
  else if (!JapaneseHolidayYearSupported(selected.year))
    output += L"\n祝日: データ収録範囲外（不明）";
  if (workspace_.empty() || !workspace_store_) {
    output += L"\nテキスト一覧: Workspace未選択";
    SetWindowTextW(calendar_details_, output.c_str());
    return;
  }

  std::vector<ProfileDefinition> profiles;
  std::wstring profile_error;
  ProfileDefinition daily;
  const bool profiles_ok = ResolveProfiles(
      CommonProfilesPath(), workspace_store_->metadata_root() / L"profiles.toml",
      profiles, profile_error);
  if (profiles_ok) {
    const auto found = std::ranges::find_if(profiles, [](const auto& item) {
      return item.id == L"daily";
    });
    if (found != profiles.end()) daily = *found;
  }
  if (!profiles_ok || daily.id.empty()) {
    output += L"\nDaily profile: 不明";
    output += L"\nテキスト一覧: 取得できません";
    SetWindowTextW(calendar_details_, output.c_str());
    return;
  }

  const auto details = BuildCalendarDayDetails(workspace_, selected, daily);
  switch (details.index.state) {
    case CalendarIndexState::Zero: output += L"\n作成ファイル: 0件"; break;
    case CalendarIndexState::Reading: output += L"\n作成ファイル: 取得中"; break;
    case CalendarIndexState::Error: output += L"\n作成ファイル: 読み取りエラー"; break;
    case CalendarIndexState::Ready:
      output += L"\n作成ファイル: " + std::to_wstring(details.index.files.size()) + L"件";
      break;
  }
  for (const auto& file : details.index.files) {
    output += L"\n・" + file.name + L" [" + std::wstring(CalendarFileTypeName(file.type)) + L"] " +
              file.relative_path.generic_wstring();
    if (file.creation_time_utc) {
      const auto seconds = std::chrono::system_clock::to_time_t(*file.creation_time_utc);
      tm local{};
      if (localtime_s(&local, &seconds) == 0) {
        wchar_t stamp[32]{};
        wcsftime(stamp, std::size(stamp), L"%Y-%m-%d %H:%M", &local);
        output += L" (作成 " + std::wstring(stamp) + L")";
      }
    } else {
      output += L" (作成日時不明)";
    }
  }
  if (details.configured_daily_path) {
    std::error_code path_error;
    const auto relative = std::filesystem::relative(*details.configured_daily_path,
                                                    workspace_, path_error);
    if (!path_error) output += L"\nDaily path: " + relative.generic_wstring();
  }
  if (!details.workspace_index.error.empty()) output += L"\n" + details.workspace_index.error;
  SetWindowTextW(calendar_details_, output.c_str());
}

void Application::OpenSelectedCalendarDate() {
  if (!calendar_) return;
  SYSTEMTIME selected{};
  if (!SendMessageW(calendar_, MCM_GETCURSEL, 0, reinterpret_cast<LPARAM>(&selected))) return;
  CreateProfileForDate(BuiltInProfile::Daily, selected);
  UpdateCalendarDetails(selected);
}
}  // namespace mdlite
