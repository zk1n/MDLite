#include "app/Application.h"

#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "app/DiagnosticsView.h"
#include "app/UiIcons.h"
#include <windowsx.h>
#include "editor/RichEditTableAdapter.h"
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
#include <imm.h>
#include <psapi.h>
#include <richedit.h>
#include <richole.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>
#include <shlwapi.h>
#include <tom.h>
#include <uxtheme.h>
#include <wincodec.h>
#include <webp/decode.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstring>
#include <ctime>
#include <array>
#include <chrono>
#include <climits>
#include <cwctype>
#include <fstream>
#include <functional>
#include <map>
#include <new>
#include <thread>

namespace mdlite {
namespace {

constexpr wchar_t kWindowClass[] = L"MDLite.MainWindow";
constexpr wchar_t kCompactWindowClass[] = L"MDLite.CompactWindow";
bool test_trust_ui_enabled{};
constexpr UINT_PTR kAutosaveTimer = 1;
constexpr UINT kTimerPollMs = 50;
constexpr ULONGLONG kEditorSyncDelayMs = 500;
constexpr ULONGLONG kPresentationDelayMs = 250;
constexpr UINT kRecoveryDelayMs = 5000;
constexpr int kTreeWidth = 250;
constexpr int kOutlineWidth = 230;
constexpr int kTabHeight = 34;
constexpr int kTopbarHeight = 40;
constexpr int kCommandSearchHeight = 30;
constexpr int kEditorHorizontalInset = 24;
constexpr int kEditorVerticalInset = 16;
constexpr int kTableCellPadding = 12;
constexpr int kTableMinimumCellEmCount = 14;
constexpr int kFindHeight = 96;
constexpr int kFindResultsHeight = 170;
constexpr int kMinimumEditorWidth = 360;
constexpr int kMinimumPaneWidth = 156;
constexpr int kFindCompactWidth = 640;
constexpr int kProcessDoneButton = 4400;
constexpr UINT kWorkspaceSearchBatchMessage = WM_APP + 41;
constexpr UINT kWorkspaceSearchCompleteMessage = WM_APP + 42;
constexpr UINT kTestThemeChangeMessage = WM_APP + 43;
constexpr UINT kGitActionCompleteMessage = WM_APP + 44;
constexpr UINT kGitStatusCompleteMessage = WM_APP + 45;
constexpr UINT kTestFailNextEditorReadbackMessage = WM_APP + 46;
constexpr UINT kTestGetCharFormatAtSourceRangeMessage = WM_APP + 47;
constexpr UINT kTestGetCalendarHoverCellMessage = WM_APP + 48;
constexpr UINT kActiveLinePresentationMessage = WM_APP + 49;
constexpr UINT kTestSetSelectionBySourceMessage = WM_APP + 50;
constexpr UINT kTestGetSourceAnchorMessage = WM_APP + 51;
constexpr UINT kTestGetSourceActiveMessage = WM_APP + 52;
constexpr UINT kTestGetSourcePositionMessage = WM_APP + 53;
constexpr UINT kTestGetEditorReadinessMessage = WM_APP + 54;
constexpr UINT kTestGetSourceAtPointMessage = WM_APP + 55;
constexpr UINT kTestGetNativeAtPointMessage = WM_APP + 56;
constexpr UINT kTestGetLastHistoryBeforeMessage = WM_APP + 57;
constexpr UINT kTestGetTabItemCenterMessage = WM_APP + 58;
constexpr UINT kTestGetSuspendedViewAnchorLineMessage = WM_APP + 59;
constexpr UINT kTestFailNextMarkdownPresentationMessage = WM_APP + 60;
constexpr UINT kTestGetVisibleSourceOffsetMessage = WM_APP + 61;
constexpr UINT kTestGetVerticalSourceOffsetMessage = WM_APP + 62;
constexpr UINT kTestFailMarkdownPresentationAfterFirstTableMessage = WM_APP + 63;
constexpr UINT kTestSetNativeProjectionFailureStagesMessage = WM_APP + 64;
constexpr UINT kRepairInvalidEditorProjectionMessage = WM_APP + 65;
constexpr UINT kTestRefreshWorkspaceTreeMessage = WM_APP + 66;
constexpr UINT kTestTrustWorkspaceForGitMessage = WM_APP + 67;
constexpr UINT kTestGetGitActionActiveMessage = WM_APP + 68;
constexpr UINT kTestGetDocumentStateMessage = WM_APP + 69;
constexpr std::uint32_t kNativeProjectionFaultProgrammaticWrite = 1U << 0U;
constexpr std::uint32_t kNativeProjectionFaultProgrammaticRestore = 1U << 1U;
constexpr std::uint32_t kNativeProjectionFaultIncrementalWrite = 1U << 2U;
constexpr std::uint32_t kNativeProjectionFaultRebuildWrite = 1U << 3U;
constexpr std::uint32_t kNativeProjectionFaultFlatFallback = 1U << 4U;
constexpr wchar_t kHolidayCacheName[] = L"japanese-holidays.csv";
constexpr ULONG_PTR kOwnerDrawSeparator = 1;

int ScaleDip(HWND window, int value) {
  const UINT dpi = window == nullptr ? 96U : GetDpiForWindow(window);
  return MulDiv(value, dpi == 0 ? 96 : static_cast<int>(dpi), 96);
}

std::pair<std::size_t, std::size_t> SourceLineRange(std::wstring_view source,
                                                   std::size_t position) {
  position = std::min(position, source.size());
  const std::size_t previous_newline = position == 0
      ? std::wstring_view::npos : source.rfind(L'\n', position - 1);
  const std::size_t begin = previous_newline == std::wstring_view::npos
      ? 0 : previous_newline + 1;
  std::size_t end = source.find(L'\n', position);
  if (end == std::wstring_view::npos) end = source.size();
  if (end > begin && source[end - 1] == L'\r') --end;
  return {begin, end};
}

bool SameNativeTableTopology(const EditorSnapshot& left,
                             const EditorSnapshot& right) {
  if (left.tables.size() != right.tables.size()) return false;
  for (std::size_t table_index{}; table_index < left.tables.size(); ++table_index) {
    const auto& a = left.tables[table_index];
    const auto& b = right.tables[table_index];
    if (a.alignments != b.alignments || a.visual_rows.size() != b.visual_rows.size()) return false;
    for (std::size_t row_index{}; row_index < a.visual_rows.size(); ++row_index) {
      if (a.visual_rows[row_index].cells.size() != b.visual_rows[row_index].cells.size()) return false;
      for (std::size_t column{}; column < a.visual_rows[row_index].cells.size(); ++column) {
        if (a.visual_rows[row_index].cells[column].virtual_cell !=
            b.visual_rows[row_index].cells[column].virtual_cell) return false;
      }
    }
  }
  return true;
}

std::size_t MapPositionBetweenViews(std::wstring_view from, std::wstring_view to,
                                    std::size_t position) {
  position = std::min(position, from.size());
  std::size_t prefix{};
  while (prefix < from.size() && prefix < to.size() && from[prefix] == to[prefix]) ++prefix;
  std::size_t old_suffix = from.size();
  std::size_t new_suffix = to.size();
  while (old_suffix > prefix && new_suffix > prefix &&
         from[old_suffix - 1] == to[new_suffix - 1]) {
    --old_suffix;
    --new_suffix;
  }
  if (position <= prefix) return position;
  if (position >= old_suffix) {
    const auto mapped = static_cast<std::ptrdiff_t>(position) +
        static_cast<std::ptrdiff_t>(new_suffix) - static_cast<std::ptrdiff_t>(old_suffix);
    return static_cast<std::size_t>(std::clamp<std::ptrdiff_t>(
        mapped, 0, static_cast<std::ptrdiff_t>(to.size())));
  }
  return prefix + std::min(position - prefix, new_suffix - prefix);
}

std::optional<std::wstring> ReadClipboardUnicodeText(HWND owner) {
  if (!OpenClipboard(owner)) return std::nullopt;
  std::optional<std::wstring> text;
  if (HANDLE data = GetClipboardData(CF_UNICODETEXT)) {
    const SIZE_T bytes = GlobalSize(data);
    if (bytes >= sizeof(wchar_t) && bytes % sizeof(wchar_t) == 0 &&
        bytes / sizeof(wchar_t) <= static_cast<SIZE_T>(LONG_MAX)) {
      if (const auto* value = static_cast<const wchar_t*>(GlobalLock(data))) {
        std::size_t length{};
        const std::size_t capacity = bytes / sizeof(wchar_t);
        while (length < capacity && value[length] != L'\0') ++length;
        if (length < capacity) {
          try { text.emplace(value, length); }
          catch (const std::bad_alloc&) { /* Keep the selected source intact. */ }
        }
        GlobalUnlock(data);
      }
    }
  }
  CloseClipboard();
  return text;
}

bool PublishClipboardUnicodeText(HWND owner, std::wstring_view text) {
  HGLOBAL data = GlobalAlloc(GMEM_MOVEABLE, (text.size() + 1) * sizeof(wchar_t));
  if (!data) return false;
  auto* value = static_cast<wchar_t*>(GlobalLock(data));
  if (!value) { GlobalFree(data); return false; }
  std::memcpy(value, text.data(), text.size() * sizeof(wchar_t));
  value[text.size()] = L'\0';
  GlobalUnlock(data);
  if (!OpenClipboard(owner)) { GlobalFree(data); return false; }
  const bool published = EmptyClipboard() && SetClipboardData(CF_UNICODETEXT, data);
  CloseClipboard();
  if (!published) GlobalFree(data);
  return published;
}

bool TestAutomationSilent() {
  wchar_t enabled[2]{};
  return GetEnvironmentVariableW(L"MDLITE_TEST_SILENT", enabled, 2) == 1 && enabled[0] == L'1';
}

DWORD TestGitActionDelay() {
  if (!TestAutomationSilent()) return 0;
  wchar_t value[16]{};
  const DWORD length = GetEnvironmentVariableW(
      L"MDLITE_TEST_GIT_ACTION_DELAY_MS", value, static_cast<DWORD>(std::size(value)));
  if (length == 0 || length >= std::size(value)) return 0;
  DWORD milliseconds{};
  for (DWORD index{}; index < length; ++index) {
    if (value[index] < L'0' || value[index] > L'9') return 0;
    milliseconds = std::min<DWORD>(10000, milliseconds * 10 + static_cast<DWORD>(value[index] - L'0'));
  }
  return milliseconds;
}

std::optional<std::wstring> ReadImeResultString(HWND window) {
  HIMC context = ImmGetContext(window);
  if (!context) return std::nullopt;
  const LONG byte_count = ImmGetCompositionStringW(context, GCS_RESULTSTR, nullptr, 0);
  if (byte_count <= 0 || byte_count % static_cast<LONG>(sizeof(wchar_t)) != 0) {
    ImmReleaseContext(window, context);
    return std::nullopt;
  }
  std::wstring result(static_cast<std::size_t>(byte_count) / sizeof(wchar_t), L'\0');
  const LONG copied = ImmGetCompositionStringW(
      context, GCS_RESULTSTR, result.data(), static_cast<DWORD>(byte_count));
  ImmReleaseContext(window, context);
  if (copied != byte_count) return std::nullopt;
  return result;
}

int TestAwareMessageBoxW(HWND owner, LPCWSTR text, LPCWSTR caption, UINT type) {
  if (!TestAutomationSilent()) return ::MessageBoxW(owner, text, caption, type);
  if (test_trust_ui_enabled && caption && wcscmp(caption, L"Workspace Trust") == 0)
    return ::MessageBoxW(owner, text, caption, type);

  // Other silent acceptance routes never surface a modal dialog or play the
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

struct GitStatusCompleteMessage {
  std::uint64_t generation{};
  std::filesystem::path workspace;
  GitPanelStatus status;
};

struct GitActionCompleteMessage {
  std::uint64_t generation{};
  std::filesystem::path workspace;
  int command{};
  GitOperationResult result;
};

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
  kCalendarView = 118,
  kBrand = 119,
  kChromeBar,
  kActivityRail,
  kCommandSearch,
  kActivityExplorer,
  kActivitySearch,
  kActivityGit,
  kActivityCalendar,
  kActivitySettings,
  kTabNew,
  kTabClose,
  kGitCommitEdit,
  kCalendarDetailsToggle = 134,
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
  kReservedHolidayOnlineUpdate,
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
  kCalendarGoToToday,
  kViewGitPane,
  kViewResizeFocusedShorter,
  kViewResizeFocusedTaller,
  // Keep diagnostics after existing IDs; acceptance harnesses send the other
  // command IDs directly.
  kViewDiagnostics,
};

int PanelIndex(PanelId id) {
  return static_cast<int>(id);
}

const wchar_t* PanelName(PanelId id) {
  switch (id) {
    case PanelId::Explorer: return L"エクスプローラー";
    case PanelId::Calendar: return L"カレンダー";
    case PanelId::Outline: return L"アウトライン";
    case PanelId::Git: return L"Git";
  }
  return L"パネル";
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
  return IsMarkdownFile(document.path()) ? BuildNativeEditorSnapshot(document.text())
                                         : BuildNativeTextEditorSnapshot(document.text());
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

struct NativeDialogTheme {
  COLORREF background{};
  COLORREF surface{};
  COLORREF input{};
  COLORREF foreground{};
  COLORREF muted{};
  COLORREF accent{};
  COLORREF border{};
};

struct NativePickerContext {
  std::vector<NativePickerItem>* items{};
  std::wstring label;
  NativeDialogTheme theme;
  HWND filter{};
  HWND list{};
  int width{720};
  int height{520};
  std::size_t selected{std::numeric_limits<std::size_t>::max()};
  bool accepted{};
  bool completed{};
};

LRESULT NativeDialogControlColor(UINT message, WPARAM wparam, LPARAM,
                                 const NativeDialogTheme& theme);
void DrawNativeDialogButton(const DRAWITEMSTRUCT& draw, const NativeDialogTheme& theme);

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
  if (message == WM_ERASEBKGND) {
    RECT client{};
    GetClientRect(window, &client);
    SetDCBrushColor(reinterpret_cast<HDC>(wparam), context->theme.background);
    FillRect(reinterpret_cast<HDC>(wparam), &client,
             static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
    return 1;
  }
  if (message == WM_CTLCOLORSTATIC || message == WM_CTLCOLOREDIT ||
      message == WM_CTLCOLORLISTBOX || message == WM_CTLCOLORBTN)
    return NativeDialogControlColor(message, wparam, lparam, context->theme);
  if (message == WM_DRAWITEM) {
    const auto* draw = reinterpret_cast<const DRAWITEMSTRUCT*>(lparam);
    if (draw && draw->CtlType == ODT_BUTTON) {
      DrawNativeDialogButton(*draw, context->theme);
      return TRUE;
    }
  }
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
    HWND apply = CreateWindowExW(0, L"BUTTON", L"開く", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW,
                                 context->width - margin - ScaleDip(window, 190), button_y,
                                 ScaleDip(window, 84), ScaleDip(window, 30), window,
                                 reinterpret_cast<HMENU>(IDOK), nullptr, nullptr);
    HWND cancel = CreateWindowExW(0, L"BUTTON", L"取消", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW,
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
                     NativeDialogTheme theme,
                     std::size_t& selected) {
  if (items.empty()) return false;
  constexpr wchar_t picker_class[] = L"MDLite.NativePickerWindow";
  WNDCLASSEXW existing{sizeof(existing)};
  if (!GetClassInfoExW(instance, picker_class, &existing)) {
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.lpfnWndProc = NativePickerWindowProc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = nullptr;
    window_class.lpszClassName = picker_class;
    if (!RegisterClassExW(&window_class)) return false;
  }
  const int width = ScaleDip(owner, 720);
  const int height = ScaleDip(owner, 520);
  NativePickerContext context;
  context.items = &items;
  context.label = std::wstring(label);
  context.theme = theme;
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
  std::function<bool()> apply;
  NativeDialogTheme theme;
  int width{680};
  int height{220};
  bool accepted{};
  bool completed{};
};

LRESULT NativeDialogControlColor(UINT message, WPARAM wparam, LPARAM,
                                 const NativeDialogTheme& theme) {
  const bool input = message == WM_CTLCOLOREDIT || message == WM_CTLCOLORLISTBOX;
  const COLORREF background = input ? theme.input : theme.surface;
  HDC dc = reinterpret_cast<HDC>(wparam);
  SetTextColor(dc, theme.foreground);
  SetBkColor(dc, background);
  SetBkMode(dc, OPAQUE);
  SetDCBrushColor(dc, background);
  return reinterpret_cast<LRESULT>(GetStockObject(DC_BRUSH));
}

void DrawNativeDialogButton(const DRAWITEMSTRUCT& draw, const NativeDialogTheme& theme) {
  if (!draw.hDC) return;
  const bool selected = (draw.itemState & ODS_SELECTED) != 0;
  const bool disabled = (draw.itemState & ODS_DISABLED) != 0;
  const COLORREF background = selected ? theme.accent : theme.surface;
  const COLORREF foreground = disabled ? theme.muted :
      (selected ? RGB(255, 255, 255) : theme.foreground);
  HBRUSH brush = CreateSolidBrush(background);
  if (brush) {
    FillRect(draw.hDC, &draw.rcItem, brush);
    DeleteObject(brush);
  }
  HPEN pen = CreatePen(PS_SOLID, 1, selected ? theme.accent : theme.border);
  if (pen) {
    const HGDIOBJ old_pen = SelectObject(draw.hDC, pen);
    const HGDIOBJ old_brush = SelectObject(draw.hDC, GetStockObject(HOLLOW_BRUSH));
    Rectangle(draw.hDC, draw.rcItem.left, draw.rcItem.top,
              draw.rcItem.right, draw.rcItem.bottom);
    SelectObject(draw.hDC, old_brush);
    SelectObject(draw.hDC, old_pen);
    DeleteObject(pen);
  }
  wchar_t label[128]{};
  GetWindowTextW(draw.hwndItem, label, static_cast<int>(std::size(label)));
  RECT text = draw.rcItem;
  SetBkMode(draw.hDC, TRANSPARENT);
  SetTextColor(draw.hDC, foreground);
  DrawTextW(draw.hDC, label, -1, &text,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
  if ((draw.itemState & ODS_FOCUS) != 0) {
    RECT focus = draw.rcItem;
    InflateRect(&focus, -3, -3);
    DrawFocusRect(draw.hDC, &focus);
  }
}

LRESULT CALLBACK NativeFormWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
  auto* context = reinterpret_cast<NativeFormContext*>(GetWindowLongPtrW(window, GWLP_USERDATA));
  if (message == WM_NCCREATE) {
    const auto* create = reinterpret_cast<CREATESTRUCTW*>(lparam);
    context = static_cast<NativeFormContext*>(create->lpCreateParams);
    SetWindowLongPtrW(window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(context));
  }
  if (!context || !context->fields) return DefWindowProcW(window, message, wparam, lparam);
  if (message == WM_ERASEBKGND) {
    RECT client{};
    GetClientRect(window, &client);
    SetDCBrushColor(reinterpret_cast<HDC>(wparam), context->theme.background);
    FillRect(reinterpret_cast<HDC>(wparam), &client,
             static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
    return 1;
  }
  if (message == WM_CTLCOLORSTATIC || message == WM_CTLCOLOREDIT ||
      message == WM_CTLCOLORLISTBOX || message == WM_CTLCOLORBTN)
    return NativeDialogControlColor(message, wparam, lparam, context->theme);
  if (message == WM_DRAWITEM) {
    const auto* draw = reinterpret_cast<const DRAWITEMSTRUCT*>(lparam);
    if (draw && draw->CtlType == ODT_BUTTON) {
      DrawNativeDialogButton(*draw, context->theme);
      return TRUE;
    }
  }
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
    HWND apply = CreateWindowExW(0, L"BUTTON", L"適用", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW,
                                 context->width - margin - ScaleDip(window, 190), button_y,
                                 ScaleDip(window, 84), ScaleDip(window, 30), window,
                                 reinterpret_cast<HMENU>(IDOK), nullptr, nullptr);
    HWND cancel = CreateWindowExW(0, L"BUTTON", L"取消", WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW,
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
      if (context->apply && !context->apply()) return 0;
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
                   std::vector<NativeFormField>& fields, NativeDialogTheme theme,
                   std::function<bool()> apply = {}) {
  constexpr wchar_t form_class[] = L"MDLite.NativeFormWindow";
  WNDCLASSEXW existing{sizeof(existing)};
  if (!GetClassInfoExW(instance, form_class, &existing)) {
    WNDCLASSEXW window_class{sizeof(window_class)};
    window_class.lpfnWndProc = NativeFormWindowProc;
    window_class.hInstance = instance;
    window_class.hCursor = LoadCursorW(nullptr, IDC_ARROW);
    window_class.hbrBackground = nullptr;
    window_class.lpszClassName = form_class;
    if (!RegisterClassExW(&window_class)) return false;
  }
  const int width = ScaleDip(owner, 680);
  int height = ScaleDip(owner, 132);
  for (const auto& field : fields)
    height += ScaleDip(owner, field.kind == NativeFormFieldKind::Multiline ? 108 : 57);
  height = std::clamp(height, ScaleDip(owner, 220), ScaleDip(owner, 760));
  NativeFormContext context;
  context.fields = &fields;
  context.apply = std::move(apply);
  context.theme = theme;
  context.width = width;
  context.height = height;
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
  StopGitActionWorker();
  StopGitStatusWorker();
  if (workspace_search_worker_.joinable()) {
    workspace_search_worker_.request_stop();
    workspace_search_worker_.join();
  }
  if (accelerator_table_) DestroyAcceleratorTable(accelerator_table_);
  if (editor_font_) DeleteObject(editor_font_);
  if (ui_font_) DeleteObject(ui_font_);
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
  // Keep the OS caption and its buttons: Windows owns Snap, drag restoration,
  // the system menu, accessible caption names and the outer resize frame.
  LoadAndApplySettings();
  ShowWindow(window_, show_command);
  UpdateWindow(window_);
  RecordDiagnosticSummary(L"アプリケーションを起動しました");
  return true;
}

int Application::Run() {
  MSG message{};
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    if (!accelerator_table_ || !TranslateAcceleratorW(window_, accelerator_table_, &message)) {
      if (message.message == WM_KEYDOWN && message.wParam == VK_TAB &&
          FindDocumentView(message.hwnd)) {
        // TranslateMessage would queue a second native table navigation as
        // WM_CHAR(Tab). Let the existing custom keydown route decide first.
        table_tab_keydown_handled_ = false;
        DispatchMessageW(&message);
        const bool handled = table_tab_keydown_handled_;
        table_tab_keydown_handled_ = false;
        if (!handled) TranslateMessage(&message);
        continue;
      }
      const bool editor_focused = std::ranges::any_of(documents_, [](const auto& view) {
        return view->editor == GetFocus();
      });
      if (message.message == WM_KEYDOWN && message.wParam == VK_TAB && !editor_focused &&
          IsDialogMessageW(window_, &message)) continue;
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
    if ((*view)->editor) MoveWindow((*view)->editor, 0, 0, client.right, client.bottom, TRUE);
    return 0;
  }
  if (message == WM_SETFOCUS && view != app->documents_.end()) {
    if ((*view)->editor) {
      app->SelectDocumentForEditor((*view)->editor);
      SetFocus((*view)->editor);
    }
    return 0;
  }
  if (message == WM_COMMAND && view != app->documents_.end() &&
      (*view)->editor && reinterpret_cast<HWND>(lparam) == (*view)->editor &&
      HIWORD(wparam) == EN_CHANGE) {
    app->OnEditorChanged((*view)->editor);
    return 0;
  }
  if (message == WM_NOTIFY && view != app->documents_.end()) {
    const auto* header = reinterpret_cast<const NMHDR*>(lparam);
    if (header && (*view)->editor && header->hwndFrom == (*view)->editor && header->code == EN_SELCHANGE &&
        !app->suppress_editor_change_ && !(*view)->native_edit_in_flight &&
        !(*view)->ime_composing && (*view)->sync_due == 0) {
      app->QueueActiveLinePresentation(*(*view));
      return 0;
    }
    if (header && (*view)->editor && header->hwndFrom == (*view)->editor && header->code == EN_LINK) {
      const auto* link = reinterpret_cast<const ENLINK*>(lparam);
      const auto source_position = (*view)->editor_snapshot.NativeToSource(link->chrg.cpMin);
      if (link->msg == WM_LBUTTONUP) app->OpenLinkAtSourcePosition(*(*view), source_position, true);
      else if (link->msg == WM_MOUSEMOVE)
        app->OpenLinkAtSourcePosition(*(*view), source_position, false);
      return 0;
    }
  }
  if (message == WM_CLOSE && view != app->documents_.end()) {
    if ((*view)->editor) SetParent((*view)->editor, app->window_);
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
  std::optional<std::wstring> body_scalar;
  if (view && (view->editor_projection_invalid || view->native_readback_failed ||
      message == WM_KILLFOCUS || (message == WM_KEYDOWN && wparam != VK_PACKET) || message == WM_SYSKEYDOWN ||
      message == WM_LBUTTONDOWN || message == WM_IME_STARTCOMPOSITION ||
      message == WM_IME_COMPOSITION || message == WM_IME_ENDCOMPOSITION ||
      message == WM_CUT || message == WM_CLEAR || message == WM_PASTE ||
      message == WM_UNDO || message == EM_REDO || message == EM_REPLACESEL ||
      message == WM_NCDESTROY ||
      (!app->suppress_editor_change_ && (message == EM_SETSEL || message == EM_EXSETSEL))))
    view->pending_body_high_surrogate = 0;
  if (view && view->editor_projection_invalid) {
    if (message == WM_NCDESTROY) RemoveWindowSubclass(window, EditorSubclass, 1);
    if (message == WM_DESTROY || message == WM_NCDESTROY)
      return DefSubclassProc(window, message, wparam, lparam);
    return 0;
  }
  if (view && view->native_readback_failed) {
    const bool control = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
    const bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
    const bool alt_x_shortcut =
        ((message == WM_SYSKEYDOWN || message == WM_SYSCHAR) &&
         (wparam == L'X' || wparam == L'x') &&
         (GetKeyState(VK_MENU) & 0x8000) != 0);
    const bool edit_message = message == WM_CHAR || message == WM_CUT || message == WM_CLEAR ||
        message == WM_PASTE || message == WM_IME_STARTCOMPOSITION ||
        message == WM_IME_COMPOSITION || message == WM_IME_ENDCOMPOSITION ||
        alt_x_shortcut ||
        (message == WM_KEYDOWN &&
         (wparam == VK_BACK || wparam == VK_DELETE || wparam == VK_F16 ||
          (control && (wparam == L'V' || wparam == L'X')) ||
          (shift && wparam == VK_INSERT)));
    const bool continuing_ime = view->ime_composing &&
        (message == WM_IME_COMPOSITION || message == WM_IME_ENDCOMPOSITION);
    if (edit_message && !continuing_ime) {
      app->SetStatusText(L"入力内容の読戻しを再試行中です。確認が終わるまで追加編集を保留します。");
      return 0;
    }
  }
  const LRESULT first_visible_line_on_focus_loss =
      message == WM_KILLFOCUS && view
          ? SendMessageW(window, EM_GETFIRSTVISIBLELINE, 0, 0)
          : -1;
  const bool source_navigation = message == WM_KILLFOCUS || message == WM_LBUTTONDOWN ||
      (message == WM_KEYDOWN &&
       (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN ||
        wparam == VK_HOME || wparam == VK_END || wparam == VK_PRIOR || wparam == VK_NEXT));
  const bool table_arrow = message == WM_KEYDOWN &&
      (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN);
  const auto flush_pending_virtual_cell_click = [&]() {
    if (!view || !view->pending_virtual_table_cell ||
        (view->sync_due == 0 && !view->native_edit_pending)) return true;
    const auto revision = view->document.revision();
    if (!app->SyncDocumentFromEditor(*view)) return false;
    if (view->document.revision() != revision) {
      view->pending_virtual_table_cell.reset();
      app->UpdatePendingVirtualTableCellFromCaret(*view);
    }
    return view->pending_virtual_table_cell.has_value();
  };
  if (view && source_navigation && !table_arrow) {
    view->pending_virtual_table_cell.reset();
    view->pending_table_high_surrogate = 0;
  }
  if (view && !view->ime_composing && !view->native_readback_failed && source_navigation &&
      IsMarkdownFile(view->document.path())) {
    // A caret/focus boundary terminates the pending native burst before the
    // next command can create an unrelated source-history entry.
    if (!app->SyncDocumentFromEditor(*view)) {
      app->SetStatusText(L"入力内容を読み取れなかったため移動を中止しました。再試行してください。");
      if (message != WM_KILLFOCUS) return 0;
    }
  }
  if (message == WM_SETFOCUS) app->SelectDocumentForEditor(window);
  if (view && view->pending_virtual_table_cell && message == WM_KEYDOWN &&
      (wparam == VK_BACK || wparam == VK_DELETE)) {
    view->pending_virtual_table_cell.reset();
    view->pending_table_high_surrogate = 0;
    return 0;
  }
  if (view && !view->pending_virtual_table_cell && !view->ime_composing && message == WM_CHAR) {
    const wchar_t character = static_cast<wchar_t>(wparam & 0xffffU);
    const bool high = character >= 0xd800 && character <= 0xdbff;
    const bool low = character >= 0xdc00 && character <= 0xdfff;
    if (high || low) {
      if (!app->SyncDocumentFromEditor(*view)) {
        view->pending_body_high_surrogate = 0;
        return 0;
      }
      const auto selection = app->CaptureSourceSelection(window, view->editor_snapshot);
      if (high) {
        view->pending_body_high_surrogate = character;
        view->pending_body_surrogate_selection = selection;
        view->pending_body_surrogate_revision = view->document.revision();
        return 0;
      }
      const wchar_t pending = view->pending_body_high_surrogate;
      view->pending_body_high_surrogate = 0;
      if (pending == 0 || view->pending_body_surrogate_revision != view->document.revision() ||
          selection.anchor != view->pending_body_surrogate_selection.anchor ||
          selection.active != view->pending_body_surrogate_selection.active) return 0;
      const wchar_t pair[]{pending, character};
      body_scalar.emplace(pair, 2);
    } else {
      view->pending_body_high_surrogate = 0;
    }
  }
  if (view && view->pending_virtual_table_cell && !view->ime_composing &&
      message == WM_CHAR) {
    if (!flush_pending_virtual_cell_click()) {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
      app->SetStatusText(L"空セルの位置を確認できなかったため入力を中止しました。表は変更していません。再試行してください。");
      return 0;
    }
    const wchar_t character = static_cast<wchar_t>(wparam & 0xffffU);
    std::wstring inserted;
    if (character >= 0xd800 && character <= 0xdbff) {
      view->pending_table_high_surrogate = character;
      return 0;
    }
    if (character >= 0xdc00 && character <= 0xdfff) {
      if (view->pending_table_high_surrogate != 0) {
        inserted.push_back(view->pending_table_high_surrogate);
        inserted.push_back(character);
      }
    } else if (view->pending_table_high_surrogate == 0 &&
               character >= L' ' && character != 0x7f) {
      inserted.push_back(character);
    }
    view->pending_table_high_surrogate = 0;
    if (inserted.empty()) {
      view->pending_virtual_table_cell.reset();
      app->SetStatusText(L"選択した空セルへ入力できませんでした。表の内容は変更していません。");
      return 0;
    }
    if (!app->InsertTextIntoPendingVirtualTableCell(*view, inserted))
      app->SetStatusText(L"選択した空セルへ入力できませんでした。表の内容は変更していません。");
    return 0;
  }
  if (view && view->pending_virtual_table_cell && message == WM_PASTE &&
      !flush_pending_virtual_cell_click()) {
    view->pending_virtual_table_cell.reset();
    view->pending_table_high_surrogate = 0;
    app->SetStatusText(L"空セルの位置を確認できなかったため貼り付けを中止しました。表は変更していません。再試行してください。");
    return 0;
  }
  if (view && message == WM_PASTE &&
      !app->PrepareTableProjectionForNativeMutation(*view, message, wparam))
    return 0;
  if (view && view->pending_virtual_table_cell && message == WM_PASTE &&
      app->PasteClipboardImage()) return 0;
  if (view && view->pending_virtual_table_cell && message == WM_PASTE) {
    std::wstring pasted;
    if (OpenClipboard(window)) {
      if (HANDLE data = GetClipboardData(CF_UNICODETEXT)) {
        const SIZE_T size = GlobalSize(data) / sizeof(wchar_t);
        if (const wchar_t* value = static_cast<const wchar_t*>(GlobalLock(data))) {
          std::size_t length{};
          while (length < size && value[length] != L'\0') ++length;
          pasted.assign(value, length);
          GlobalUnlock(data);
        }
      }
      CloseClipboard();
    }
    if (pasted.empty()) {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
      app->SetStatusText(L"クリップボードにテキストがないため、空セルへの貼り付けを中断しました。");
      return 0;
    }
    if (!app->InsertTextIntoPendingVirtualTableCell(*view, pasted))
      app->SetStatusText(L"クリップボードの内容を選択した空セルへ貼り付けられませんでした。");
    return 0;
  }
  if (message == WM_PASTE && app->PasteClipboardImage()) return 0;
  if (message == WM_IME_STARTCOMPOSITION && view) {
    if (!flush_pending_virtual_cell_click()) {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
      app->SetStatusText(L"空セルの位置を確認できなかったためIME入力を開始できません。表は変更していません。再試行してください。");
      return 0;
    }
    if (!app->PrepareTableProjectionForNativeMutation(*view, message, wparam))
      return 0;
    view->ime_selection_before = app->CaptureSourceSelection(window, view->editor_snapshot);
    view->ime_composing = true;
  }
  const bool native_mutation = view != nullptr &&
      (message == WM_CHAR || message == WM_CUT || message == WM_CLEAR ||
       message == WM_PASTE || message == WM_IME_COMPOSITION ||
       message == WM_IME_ENDCOMPOSITION ||
       (message == WM_KEYDOWN && (wparam == VK_BACK || wparam == VK_DELETE || wparam == VK_RETURN ||
          (((GetKeyState(VK_CONTROL) & 0x8000) != 0) &&
           (wparam == L'V' || wparam == L'X')) ||
          (((GetKeyState(VK_SHIFT) & 0x8000) != 0) &&
           (wparam == VK_INSERT || wparam == VK_DELETE)))));
  const bool capture_edit_selection = native_mutation ||
      (view != nullptr && message == EM_REPLACESEL);
  const bool had_pending_edit_selection = view && view->pending_selection_before.has_value();
  if (capture_edit_selection && view && message != WM_PASTE &&
      !app->PrepareTableProjectionForNativeMutation(*view, message, wparam))
    return 0;
  if (capture_edit_selection && view && !view->pending_selection_before) {
    view->pending_selection_before = view->ime_selection_before
        ? *view->ime_selection_before
        : app->CaptureSourceSelection(window, view->editor_snapshot);
    if ((message == WM_KEYDOWN && wparam == VK_BACK) ||
        (message == WM_CHAR && wparam == 0x08)) {
      view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Backspace;
    } else if ((message == WM_KEYDOWN && wparam == VK_DELETE) ||
               (message == WM_CHAR && wparam == 0x7f)) {
      view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Delete;
    } else if (message == WM_CHAR || message == WM_IME_COMPOSITION) {
      view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Insert;
    } else {
      view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Replace;
    }
  }
  if (native_mutation) view->native_edit_in_flight = true;
  const bool control = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
  const bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
  if (view && table_arrow && (control || shift || (GetKeyState(VK_MENU) & 0x8000) != 0)) {
    view->pending_virtual_table_cell.reset();
    view->pending_table_high_surrogate = 0;
  }
  if (view && !view->ime_composing &&
      (message == WM_UNDO || message == EM_REDO ||
       (message == WM_KEYDOWN && control && (wparam == L'Z' || wparam == L'Y')))) {
    const bool redo = message == EM_REDO || wparam == L'Y' || (wparam == L'Z' && shift);
    view->pending_virtual_table_cell.reset();
    view->pending_table_high_surrogate = 0;
    app->ApplySourceHistory(*view, redo);
    return TRUE;
  }
  if (message == WM_KEYDOWN && wparam == VK_TAB && view && !view->ime_composing &&
      IsMarkdownFile(view->document.path())) {
    view->pending_table_high_surrogate = 0;
    if (!app->SyncDocumentFromEditor(*view)) {
      app->table_tab_keydown_handled_ = true;
      app->SetStatusText(L"入力内容を読み取れなかったため表の移動を中止しました。再試行してください。");
      return 0;
    }
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    const auto source_caret = view->editor_snapshot.NativeToSource(selection.cpMin);
    const auto current_cell = view->pending_virtual_table_cell
        ? view->pending_virtual_table_cell
        : ResolveTableCellIntent(view->document.text(), source_caret);
    const auto edit = current_cell
        ? MoveToAdjacentTableCell(view->document.text(), *current_cell,
                                  (GetKeyState(VK_SHIFT) & 0x8000) != 0)
        : TableEditResult{view->document.text(), source_caret, false};
    if (edit.changed) {
      if (!app->ApplySourceTextWithUndo(*view, edit.text, true,
                                        SourceSelection{edit.selection, edit.selection})) {
        app->table_tab_keydown_handled_ = true;
        view->pending_virtual_table_cell.reset();
        app->SetStatusText(L"本文が同期中のため表の移動を中止しました。再試行してください。");
        return 0;
      }
    }
    if (edit.target_cell) {
      if (edit.target_cell->virtual_cell) view->pending_virtual_table_cell = edit.target_cell;
      else view->pending_virtual_table_cell.reset();
      const auto view_caret = view->editor_snapshot.SourceToNative(
          edit.target_cell->source_position);
      SendMessageW(window, EM_SETSEL, view_caret, view_caret);
      app->table_tab_keydown_handled_ = true;
      return 0;
    }
  }
  if (message == WM_KEYDOWN && view && !view->ime_composing &&
      (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN) &&
      (GetKeyState(VK_SHIFT) & 0x8000) == 0 && (GetKeyState(VK_CONTROL) & 0x8000) == 0 &&
      (GetKeyState(VK_MENU) & 0x8000) == 0 && IsMarkdownFile(view->document.path())) {
    if (!app->SyncDocumentFromEditor(*view)) {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
      return DefSubclassProc(window, message, wparam, lparam);
    }
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    if (selection.cpMin == selection.cpMax) {
      const auto direction = wparam == VK_LEFT ? TableCaretDirection::Left :
          wparam == VK_RIGHT ? TableCaretDirection::Right :
          wparam == VK_UP ? TableCaretDirection::Up : TableCaretDirection::Down;
      const auto source_caret = view->editor_snapshot.NativeToSource(selection.cpMin);
      const GfmTable* current_table{};
      if (view->presentation_revision == view->document.revision() &&
          view->presentation_due == 0) {
        const std::size_t table_position = view->pending_virtual_table_cell
            ? view->pending_virtual_table_cell->row_begin : source_caret;
        for (const auto& table : view->parse.tables) {
          if (table_position < table.begin || table_position > table.end) continue;
          current_table = &table;
          break;
        }
      }
      if (current_table) {
        const auto current_table_cell = view->pending_virtual_table_cell
            ? view->pending_virtual_table_cell
            : ResolveTableCellIntent(*current_table, view->document.text(), source_caret);
        bool at_cell_boundary = current_table_cell && current_table_cell->virtual_cell;
        if (current_table_cell) {
          const auto row = std::lower_bound(current_table->rows.begin(), current_table->rows.end(),
              current_table_cell->row_begin, [](const TableVisualRow& candidate, std::size_t begin) {
                return candidate.begin < begin;
              });
          if (row != current_table->rows.end() && row->begin == current_table_cell->row_begin &&
              current_table_cell->column < row->cells.size()) {
            const auto& cell = row->cells[current_table_cell->column];
            std::size_t content_begin = cell.begin;
            std::size_t content_end = cell.end;
            const auto& source = view->document.text();
            while (content_begin < content_end && iswspace(source[content_begin])) ++content_begin;
            while (content_end > content_begin && iswspace(source[content_end - 1])) --content_end;
            switch (direction) {
              case TableCaretDirection::Left:
              case TableCaretDirection::Up: at_cell_boundary |= source_caret == content_begin; break;
              case TableCaretDirection::Right:
              case TableCaretDirection::Down: at_cell_boundary |= source_caret == content_end; break;
            }
          }
        }
        if (current_table_cell && at_cell_boundary) {
          const auto destination = MoveTableCaretTargetAtBoundary(
              *current_table, view->document.text(), *current_table_cell, direction);
          if (destination) {
            if (destination->virtual_cell) view->pending_virtual_table_cell = destination;
            else view->pending_virtual_table_cell.reset();
            const auto view_caret = view->editor_snapshot.SourceToNative(
                destination->source_position);
            SendMessageW(window, EM_SETSEL, view_caret, view_caret);
            return 0;
          }
          view->pending_virtual_table_cell.reset();
        }
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
    } else {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
    }
  }
  if (message == WM_LBUTTONDOWN && view && !view->native_tables_ready && !view->ime_composing &&
      view->sync_due == 0 && IsMarkdownFile(view->document.path())) {
    const POINT point{static_cast<short>(LOWORD(lparam)), static_cast<short>(HIWORD(lparam))};
    if (const auto hit = app->HitTestTableCell(*view, point)) {
      if (hit->virtual_cell) {
        view->pending_virtual_table_cell = *hit;
      }
      const auto native = view->editor_snapshot.SourceToNative(hit->source_position);
      app->SelectDocumentForEditor(window);
      SendMessageW(window, EM_SETSEL, static_cast<WPARAM>(native), static_cast<LPARAM>(native));
      SetFocus(window);
      return 0;
    }
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, EditorSubclass, 1);
  RECT update_rect{};
  const bool needs_table_paint = message == WM_PAINT && view && !view->native_tables_ready;
  const bool route_ime_result_to_virtual_cell = view && view->pending_virtual_table_cell &&
      message == WM_IME_COMPOSITION && (lparam & GCS_RESULTSTR) != 0;
  const auto ime_result_text = route_ime_result_to_virtual_cell
      ? ReadImeResultString(window) : std::optional<std::wstring>{};
  if (needs_table_paint && !GetUpdateRect(window, &update_rect, FALSE))
    GetClientRect(window, &update_rect);
  UINT native_message = body_scalar ? EM_REPLACESEL : message;
  WPARAM native_wparam = body_scalar ? TRUE : wparam;
  LPARAM native_lparam = body_scalar ? reinterpret_cast<LPARAM>(body_scalar->c_str()) : lparam;
  std::optional<std::wstring> literal_paste;
  if (view && native_mutation && !view->ime_composing &&
      !view->native_edit_pending && view->sync_due == 0 &&
      view->pending_selection_before && !view->pending_virtual_table_cell) {
    const auto selection = *view->pending_selection_before;
    const std::size_t begin = std::min(selection.anchor, selection.active);
    const std::size_t end = std::max(selection.anchor, selection.active);
    const auto& source = view->document.text();
    const bool paragraph_end_selected = begin < end && end <= source.size() &&
        (source[end - 1] == L'\r' || source[end - 1] == L'\n');
    const bool cut = message == WM_CUT ||
        (message == WM_KEYDOWN && ((control && (wparam == L'X' || wparam == L'x')) ||
                                  (shift && wparam == VK_DELETE)));
    const bool paste = (message == WM_PASTE ||
        (message == WM_KEYDOWN && ((control && (wparam == L'V' || wparam == L'v')) ||
                                  (shift && wparam == VK_INSERT)))) &&
        IsClipboardFormatAvailable(CF_UNICODETEXT) &&
        !IsClipboardFormatAvailable(CF_BITMAP) && !IsClipboardFormatAvailable(CF_DIB) &&
        !IsClipboardFormatAvailable(CF_DIBV5);
    const bool deletion = message == WM_KEYDOWN &&
        (wparam == VK_BACK || wparam == VK_DELETE);
    const bool character = body_scalar || (message == WM_CHAR && !control &&
        wparam >= L' ' && wparam != 0x7f && !(wparam >= 0xd800 && wparam <= 0xdfff));
    // RichEdit's ordinary typing/cut/delete paths retain the last selected
    // paragraph mark. Markdown range edits must replace the literal range.
    // Bypass only that native adjustment, inside the existing outer mutation,
    // so EN_CHANGE/readback still produces one source history transaction.
    if (paragraph_end_selected && (cut || deletion || character || paste)) {
      bool clipboard_ready = true;
      if (cut) clipboard_ready = PublishClipboardUnicodeText(
          window, std::wstring_view(source).substr(begin, end - begin));
      if (paste) {
        literal_paste = ReadClipboardUnicodeText(window);
        clipboard_ready = literal_paste && !literal_paste->empty();
      }
      if (!clipboard_ready) {
        view->native_edit_in_flight = false;
        if (!had_pending_edit_selection) {
          view->pending_selection_before.reset();
          view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
        }
        app->SetStatusText(L"クリップボードを確認できなかったため編集を中止しました。本文と選択範囲は保持されています。");
        return 0;
      }
      if (cut || deletion) {
        native_message = WM_CLEAR;
        native_wparam = 0;
        native_lparam = 0;
      } else {
        // Direct native dispatch avoids a nested subclass/history commit.
        DefSubclassProc(window, WM_CLEAR, 0, 0);
        if (paste) {
          native_message = EM_REPLACESEL;
          native_wparam = TRUE;
          native_lparam = reinterpret_cast<LPARAM>(literal_paste->c_str());
        }
      }
    }
  }
  const LRESULT result = DefSubclassProc(window, native_message, native_wparam, native_lparam);
  if (message == WM_KILLFOCUS && view && IsWindow(window) &&
      first_visible_line_on_focus_loss >= 0) {
    const LRESULT current_first_visible_line =
        SendMessageW(window, EM_GETFIRSTVISIBLELINE, 0, 0);
    if (current_first_visible_line >= 0 &&
        current_first_visible_line != first_visible_line_on_focus_loss) {
      // RichEdit may scroll the caret into view while focus leaves the editor.
      // Preserve the user's viewport; tab reactivation restores this source anchor.
      SendMessageW(window, EM_LINESCROLL, 0,
                   first_visible_line_on_focus_loss - current_first_visible_line);
    }
  }
  if (message == EM_REPLACESEL && view && !had_pending_edit_selection &&
      !view->native_edit_pending && view->sync_due == 0) {
    // EM_REPLACESEL relies on EN_CHANGE/debounce. Drop a fresh selection
    // capture when RichEdit reported no mutation.
    view->pending_selection_before.reset();
    view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
  }
  if (message == WM_LBUTTONDOWN && view && view->native_tables_ready) {
    const POINT click_point{static_cast<short>(LOWORD(lparam)),
                            static_cast<short>(HIWORD(lparam))};
    app->UpdatePendingVirtualTableCellFromCaret(*view, &click_point);
  }
  if (route_ime_result_to_virtual_cell && view) {
    const auto selection_before = view->ime_selection_before;
    view->native_edit_in_flight = false;
    view->native_edit_pending = false;
    view->sync_due = 0;
    view->ime_composing = false;
    view->ime_selection_before.reset();
    view->pending_selection_before.reset();
    view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
    if (!ime_result_text || ime_result_text->empty() ||
        !app->InsertTextIntoPendingVirtualTableCell(*view, *ime_result_text, selection_before)) {
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
      if (view->sync_due == 0 && !view->native_edit_pending) {
        view->native_edit_pending = true;
        view->sync_due = GetTickCount64();
      }
      app->CommitPendingNativeEdit(*view);
      app->SetStatusText(L"IME入力を空セルへ配置できなかったため、入力内容を現在のセルに保持しました。");
    }
    return result;
  }
  if (view && native_mutation && view->native_edit_in_flight) {
    const bool ime_result = message == WM_IME_COMPOSITION &&
                            (lparam & GCS_RESULTSTR) != 0;
    const bool ordinary_edit = !view->ime_composing &&
        (message == WM_CHAR || message == WM_CUT || message == WM_CLEAR ||
         message == WM_PASTE ||
         (message == WM_KEYDOWN && (wparam == VK_BACK || wparam == VK_DELETE || wparam == VK_RETURN ||
          (((GetKeyState(VK_CONTROL) & 0x8000) != 0) &&
           (wparam == L'V' || wparam == L'X')) ||
          (((GetKeyState(VK_SHIFT) & 0x8000) != 0) &&
           (wparam == VK_INSERT || wparam == VK_DELETE)))));
    // IME result strings can be revised before composition ends. Keep one
    // source selection/history boundary for the full composition transaction.
    const bool defer_ime_result = ime_result && view->ime_composing;
    if (message == WM_IME_ENDCOMPOSITION) view->ime_composing = false;
    if (!defer_ime_result &&
        (ime_result || message == WM_IME_ENDCOMPOSITION || ordinary_edit)) {
      // Some programmatic WM_CHAR and clipboard paths deliver their RichEdit
      // change notification after the subclass returns. Commit the resulting
      // text here as well so adjacent characters remain separate source edits.
      const bool ime_completion = message == WM_IME_ENDCOMPOSITION;
      if ((ordinary_edit || ime_result || ime_completion) &&
          !view->native_edit_pending && view->sync_due == 0) {
        view->native_edit_pending = true;
        view->sync_due = GetTickCount64();
      }
      app->CommitPendingNativeEdit(*view);
    if (message == WM_IME_ENDCOMPOSITION) {
      view->ime_selection_before.reset();
      view->pending_virtual_table_cell.reset();
      view->pending_table_high_surrogate = 0;
    }
      if (!view->native_edit_pending) {
        view->pending_selection_before.reset();
        view->pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
      }
    }
    view->native_edit_in_flight = false;
  }
  if (needs_table_paint && view && !view->painting_table_grid) {
    // Keep update-rectangle clipping in DrawTableGrid's RichEdit client space;
    // passing a derived HRGN to GetDCEx clipped child controls at an offset edge.
    HDC paint_dc = GetDCEx(window, nullptr, DCX_CACHE | DCX_CLIPSIBLINGS);
    if (paint_dc) {
      view->painting_table_grid = true;
      app->DrawTableGrid(*view, paint_dc, update_rect);
      view->painting_table_grid = false;
      ReleaseDC(window, paint_dc);
    }
  }
  return result;
}

bool Application::PrepareTableProjectionForNativeMutation(DocumentView& view,
                                                           UINT message, WPARAM wparam) {
  if (!IsMarkdownFile(view.document.path()) || view.editor_projection_invalid ||
      view.ime_composing || view.editor_snapshot.tables.empty()) return true;

  const bool control = (GetKeyState(VK_CONTROL) & 0x8000) != 0;
  const bool shift = (GetKeyState(VK_SHIFT) & 0x8000) != 0;
  const bool alt = (GetKeyState(VK_MENU) & 0x8000) != 0;
  const bool keyboard_paste = message == WM_KEYDOWN &&
      ((control && (wparam == L'V' || wparam == L'v')) ||
       (shift && wparam == VK_INSERT));
  const bool keyboard_cut = message == WM_KEYDOWN &&
      ((control && (wparam == L'X' || wparam == L'x')) ||
       (shift && wparam == VK_DELETE));
  const bool alt_cut = message == WM_SYSKEYDOWN && alt &&
      (wparam == L'X' || wparam == L'x');
  const bool paste = message == WM_PASTE || keyboard_paste;
  const bool backspace = (message == WM_KEYDOWN && wparam == VK_BACK) ||
                         (message == WM_CHAR && wparam == 0x08);
  const bool forward_delete = (message == WM_KEYDOWN && wparam == VK_DELETE) ||
                              (message == WM_CHAR && wparam == 0x7f);
  const bool row_break = (message == WM_KEYDOWN && wparam == VK_RETURN) ||
                         (message == WM_CHAR && wparam == L'\r');
  const bool replace = message == EM_REPLACESEL;
  const bool native_edit = message == WM_CHAR || message == WM_CUT ||
      message == WM_CLEAR || paste || backspace || forward_delete || row_break ||
      keyboard_cut || alt_cut || replace || message == WM_IME_STARTCOMPOSITION ||
      (message == WM_IME_COMPOSITION && !view.ime_composing);
  if (!native_edit) return true;
  if (view.pending_virtual_table_cell &&
      (message == WM_IME_STARTCOMPOSITION ||
       (message == WM_IME_COMPOSITION && !view.ime_composing))) return true;

  // RichEdit coordinates and table cells may have changed after the prior
  // native message. Commit that burst first so the projection and selection
  // are mapped against the current Markdown source.
  if ((view.sync_due != 0 || view.native_edit_pending) &&
      !SyncDocumentFromEditor(view)) {
    SetStatusText(L"表の構造を確認できないため、入力を中止しました。本文は保持されています。再試行してください。");
    return false;
  }
  if (view.editor_snapshot.tables.empty()) return true;

  const auto payload_has_table_separators = [](std::wstring_view value) {
    return value.find_first_of(L"\r\n\t") != std::wstring_view::npos;
  };
  bool paste_may_add_rows_or_cells{};
  if (paste) {
    if (!OpenClipboard(view.editor)) {
      paste_may_add_rows_or_cells = true;
    } else {
      bool found_plain_text{};
      bool malformed_plain_text{};
      bool has_structural_text{};
      if (IsClipboardFormatAvailable(CF_UNICODETEXT)) {
        HANDLE data = GetClipboardData(CF_UNICODETEXT);
        if (data) {
          const SIZE_T size = GlobalSize(data) / sizeof(wchar_t);
          const wchar_t* value = static_cast<const wchar_t*>(GlobalLock(data));
          if (value) {
            std::size_t length{};
            while (length < size && value[length] != L'\0') ++length;
            found_plain_text = true;
            has_structural_text = payload_has_table_separators(
                std::wstring_view(value, length));
            GlobalUnlock(data);
          } else {
            malformed_plain_text = true;
          }
        } else {
          malformed_plain_text = true;
        }
      } else if (IsClipboardFormatAvailable(CF_TEXT)) {
        HANDLE data = GetClipboardData(CF_TEXT);
        if (data) {
          const SIZE_T size = GlobalSize(data);
          const char* value = static_cast<const char*>(GlobalLock(data));
          if (value) {
            std::size_t length{};
            while (length < size && value[length] != '\0') ++length;
            found_plain_text = true;
            has_structural_text = std::string_view(value, length).find_first_of("\r\n\t") !=
                                  std::string_view::npos;
            GlobalUnlock(data);
          } else {
            malformed_plain_text = true;
          }
        } else {
          malformed_plain_text = true;
        }
      }
      const UINT rtf_format = RegisterClipboardFormatW(L"Rich Text Format");
      const UINT html_format = RegisterClipboardFormatW(L"HTML Format");
      const bool has_rich_text =
          (rtf_format != 0 && IsClipboardFormatAvailable(rtf_format)) ||
          (html_format != 0 && IsClipboardFormatAvailable(html_format));
      const bool has_image = IsClipboardFormatAvailable(CF_BITMAP) ||
          IsClipboardFormatAvailable(CF_DIB) || IsClipboardFormatAvailable(CF_DIBV5);
      paste_may_add_rows_or_cells = has_structural_text || malformed_plain_text || has_rich_text ||
          (!found_plain_text && !has_image);
      CloseClipboard();
    }
    if (view.pending_virtual_table_cell && !paste_may_add_rows_or_cells) return true;
  } else if (message == WM_CHAR && wparam == L'\t') {
    paste_may_add_rows_or_cells = true;
  }

  CHARRANGE native_range{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&native_range));
  const std::size_t native_begin = static_cast<std::size_t>(std::max<LONG>(native_range.cpMin, 0));
  const std::size_t native_end = static_cast<std::size_t>(std::max<LONG>(native_range.cpMax, 0));
  const bool collapsed_selection = native_begin == native_end;
  SourceSelection selection = view.pending_virtual_table_cell
      ? SourceSelection{view.pending_virtual_table_cell->source_position,
                        view.pending_virtual_table_cell->source_position}
      : CaptureSourceSelection(view.editor, view.editor_snapshot);
  selection.anchor = std::min(selection.anchor, view.document.text().size());
  selection.active = std::min(selection.active, view.document.text().size());
  const std::size_t source_begin = std::min(selection.anchor, selection.active);
  const std::size_t source_end = std::max(selection.anchor, selection.active);

  bool touches_table{};
  bool deletes_cell_boundary{};
  bool selection_crosses_native_structure{};
  bool needs_flat_source{};
  for (const auto& table : view.editor_snapshot.tables) {
    const bool deletes_table_edge = collapsed_selection &&
        (backspace || forward_delete) &&
        (native_begin == table.native_begin || native_begin == table.native_end);
    const bool source_overlap = collapsed_selection
        ? source_begin >= table.source_begin && source_begin < table.source_end
        : source_begin < table.source_end && source_end > table.source_begin;
    const bool native_overlap = table.native_coordinates_set &&
        (collapsed_selection
            ? native_begin >= table.native_begin && native_begin < table.native_end
            : native_begin < table.native_end && native_end > table.native_begin);
    if (!source_overlap && !native_overlap && !deletes_table_edge) continue;
    touches_table = true;
    if (deletes_table_edge) needs_flat_source = true;
    if (!table.native_coordinates_set) {
      needs_flat_source = true;
      selection_crosses_native_structure |= !collapsed_selection;
      continue;
    }

    const EditorTableCellMapping* selected_cell{};
    for (const auto& cell : table.cells) {
      const bool native_contained = native_begin >= cell.native_begin &&
                                    native_end <= cell.native_end;
      if (native_contained) {
        selected_cell = &cell;
        break;
      }
    }
    if (!selected_cell) {
      needs_flat_source = true;
      selection_crosses_native_structure |= !collapsed_selection;
      continue;
    }
    if (collapsed_selection &&
        ((backspace && native_begin == selected_cell->native_begin) ||
         (forward_delete && native_begin == selected_cell->native_end))) {
      deletes_cell_boundary = true;
    }
  }

  // A table rebuild may be pending after a source-side action. If the old
  // mapping cannot prove that the native range stays within one cell, use the
  // current source as the edit surface and let normal sync rebuild topology.
  if (touches_table && (deletes_cell_boundary || row_break || replace ||
                        (paste && paste_may_add_rows_or_cells))) {
    needs_flat_source = true;
  }
  if (!needs_flat_source) return true;

  if (selection_crosses_native_structure) {
    std::size_t expanded_begin = source_begin;
    std::size_t expanded_end = source_end;
    for (const auto& table : view.editor_snapshot.tables) {
      if (!table.native_coordinates_set || table.visual_rows.empty()) continue;
      const auto& first_row = table.visual_rows.front();
      const auto& last_row = table.visual_rows.back();
      if (first_row.cells.empty() || last_row.cells.empty()) continue;
      if (native_begin <= first_row.cells.front().native_begin &&
          native_end >= last_row.cells.back().native_end) {
        expanded_begin = std::min(expanded_begin, table.source_begin);
        expanded_end = std::max(expanded_end, table.source_end);
        continue;
      }
      for (const auto& row : table.visual_rows) {
        if (row.cells.empty() || native_begin >= row.native_end || native_end <= row.native_begin)
          continue;
        if (native_begin <= row.cells.front().native_begin)
          expanded_begin = std::min(expanded_begin, row.source_begin);
        if (native_end >= row.cells.back().native_end)
          expanded_end = std::max(expanded_end, row.source_end);
      }
    }
    if (selection.anchor <= selection.active) {
      selection = {expanded_begin, expanded_end};
    } else {
      selection = {expanded_end, expanded_begin};
    }
  }

  const auto outcome = RebuildEditorProjection(
      view, BuildNativeTextEditorSnapshot(view.document.text()), selection);
  if (outcome == EditorProjectionRebuildResult::Failed ||
      !view.editor_snapshot.tables.empty()) {
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    SetStatusText(L"表を原文表示へ切り替えられないため、入力を中止しました。本文は保持されています。");
    return false;
  }
  auto restored = CaptureSourceSelection(view.editor, view.editor_snapshot);
  if (restored.anchor != selection.anchor || restored.active != selection.active) {
    RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
    restored = CaptureSourceSelection(view.editor, view.editor_snapshot);
  }
  if (restored.anchor != selection.anchor || restored.active != selection.active) {
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    SetStatusText(L"表を原文表示へ切り替えましたが、選択位置を復元できないため入力を中止しました。本文は保持されています。");
    return false;
  }
  view.pending_virtual_table_cell.reset();
  view.pending_table_high_surrogate = 0;
  SchedulePresentation(view);
  return true;
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

LRESULT CALLBACK Application::PanelHeaderSubclass(HWND window, UINT message, WPARAM wparam,
                                                   LPARAM lparam, UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (message == WM_LBUTTONUP && app) {
    RECT client{};
    GetClientRect(window, &client);
    const int x = static_cast<short>(LOWORD(lparam));
    const int button_edge = client.right - ScaleDip(window, 28);
    const int menu_edge = client.right - ScaleDip(window, 60);
    int command{};
    if (x >= button_edge) {
      switch (GetDlgCtrlID(window)) {
        case kPanelHeaderExplorer: command = kViewWorkspacePane; break;
        case kPanelHeaderCalendar: command = kViewCalendar; break;
        case kPanelHeaderOutline: command = kViewOutlinePane; break;
        case kPanelHeaderGit: command = kViewGitPane; break;
      }
    } else if (x >= menu_edge) {
      command = kViewCommandPalette;
    }
    if (command != 0) {
      const LRESULT result = DefSubclassProc(window, message, wparam, lparam);
      SendMessageW(app->window_, WM_COMMAND, MAKEWPARAM(command, 0), 0);
      return result;
    }
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, PanelHeaderSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::CalendarDetailsSubclass(HWND window, UINT message, WPARAM wparam,
                                                       LPARAM lparam, UINT_PTR,
                                                       DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, CalendarDetailsSubclass, 1);
  const bool activate = message == WM_KEYDOWN && wparam == VK_RETURN;
  const bool double_click = message == WM_LBUTTONDBLCLK;
  LRESULT result = 0;
  if (double_click) result = DefSubclassProc(window, message, wparam, lparam);
  if (app && (activate || double_click)) {
    DWORD begin{};
    DWORD end{};
    SendMessageW(window, EM_GETSEL, reinterpret_cast<WPARAM>(&begin),
                 reinterpret_cast<LPARAM>(&end));
    if (app->OpenCalendarDetailAtOffset(std::min(begin, end))) return 0;
  }
  return double_click ? result : DefSubclassProc(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::ChromeBarSubclass(HWND window, UINT message, WPARAM wparam,
                                                 LPARAM lparam, UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (app && message == WM_KEYDOWN && wparam == VK_RETURN && IsWindowEnabled(window)) {
    SendMessageW(app->window_, WM_COMMAND,
                 MAKEWPARAM(GetDlgCtrlID(window), BN_CLICKED), reinterpret_cast<LPARAM>(window));
    return 0;
  }
  const bool activity = app && std::ranges::find(app->activity_buttons_.begin(),
      app->activity_buttons_.begin() + 4, window) != app->activity_buttons_.begin() + 4;
  if (activity && (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wparam);
    DRAWITEMSTRUCT draw{};
    draw.CtlType = ODT_BUTTON;
    draw.CtlID = GetDlgCtrlID(window);
    draw.hwndItem = window;
    draw.hDC = dc;
    GetClientRect(window, &draw.rcItem);
    if (GetFocus() == window) draw.itemState |= ODS_FOCUS;
    if (!IsWindowEnabled(window)) draw.itemState |= ODS_DISABLED;
    if (SendMessageW(window, BM_GETSTATE, 0, 0) & BST_PUSHED) draw.itemState |= ODS_SELECTED;
    app->DrawChromeButton(draw);
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  if (activity && message == WM_ERASEBKGND) return 1;
  if (message == WM_MOUSEMOVE && IsWindowEnabled(window)) {
    if (!GetPropW(window, L"MDLite.Hover")) {
      SetPropW(window, L"MDLite.Hover", reinterpret_cast<HANDLE>(1));
      TRACKMOUSEEVENT tracking{sizeof(tracking), TME_LEAVE, window, 0};
      TrackMouseEvent(&tracking);
      InvalidateRect(window, nullptr, FALSE);
    }
  } else if (message == WM_MOUSELEAVE || message == WM_CAPTURECHANGED ||
             message == WM_ENABLE) {
    RemovePropW(window, L"MDLite.Hover");
    InvalidateRect(window, nullptr, FALSE);
  } else if (message == WM_NCDESTROY) {
    RemovePropW(window, L"MDLite.Hover");
    RemoveWindowSubclass(window, ChromeBarSubclass, 1);
  }
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::TabStripSubclass(HWND window, UINT message, WPARAM wparam,
                                                LPARAM lparam, UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (app && (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wparam);
    if (dc) {
      const int saved = SaveDC(dc);
      RECT client{};
      GetClientRect(window, &client);
      SetDCBrushColor(dc, app->theme_surface_);
      FillRect(dc, &client, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
      for (int index = 0; index < TabCtrl_GetItemCount(window); ++index) {
        DRAWITEMSTRUCT draw{};
        draw.CtlType = ODT_TAB; draw.CtlID = GetDlgCtrlID(window);
        draw.itemID = index; draw.hwndItem = window; draw.hDC = dc;
        if (!TabCtrl_GetItemRect(window, index, &draw.rcItem)) continue;
        if (index == TabCtrl_GetCurSel(window)) draw.itemState |= ODS_SELECTED;
        if (GetFocus() == window && index == TabCtrl_GetCurFocus(window)) draw.itemState |= ODS_FOCUS;
        if (!IsWindowEnabled(window)) draw.itemState |= ODS_DISABLED;
        app->DrawTabItem(draw);
      }
      if (saved) RestoreDC(dc, saved);
    }
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  if (message == WM_ERASEBKGND && app) {
    RECT client{};
    GetClientRect(window, &client);
    HBRUSH background = CreateSolidBrush(app->theme_surface_);
    if (background) {
      FillRect(reinterpret_cast<HDC>(wparam), &client, background);
      DeleteObject(background);
    }
    return 1;
  }
  if (message == WM_THEMECHANGED) InvalidateRect(window, nullptr, TRUE);
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, TabStripSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT CALLBACK Application::StatusBarSubclass(HWND window, UINT message, WPARAM wparam,
                                                 LPARAM lparam, UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (app && (message == WM_PAINT || message == WM_PRINTCLIENT)) {
    PAINTSTRUCT paint{};
    HDC dc = message == WM_PAINT ? BeginPaint(window, &paint) : reinterpret_cast<HDC>(wparam);
    if (dc) {
      const int saved = SaveDC(dc);
      RECT client{};
      GetClientRect(window, &client);
      SetDCBrushColor(dc, app->theme_surface_);
      FillRect(dc, &client, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
      for (std::size_t index = 0; index < app->status_segments_.size(); ++index) {
        DRAWITEMSTRUCT draw{};
        draw.hwndItem = window; draw.hDC = dc; draw.itemID = static_cast<UINT>(index);
        draw.itemData = reinterpret_cast<ULONG_PTR>(app->status_segments_[index].c_str());
        if (SendMessageW(window, SB_GETRECT, index, reinterpret_cast<LPARAM>(&draw.rcItem)))
          app->DrawStatusItem(draw);
      }
      if (saved) RestoreDC(dc, saved);
    }
    if (message == WM_PAINT) EndPaint(window, &paint);
    return 0;
  }
  if (app && message == WM_ERASEBKGND) return 1;
  if (message == WM_THEMECHANGED) InvalidateRect(window, nullptr, FALSE);
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, StatusBarSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
}

LRESULT Application::HandleMessage(UINT message, WPARAM wparam, LPARAM lparam) {
  switch (message) {
    case WM_NCHITTEST: {
      const LRESULT native = DefWindowProcW(window_, message, wparam, lparam);
      if (native != HTCLIENT) return native;
      POINT point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
      ScreenToClient(window_, &point);
      RECT client{};
      GetClientRect(window_, &client);
      if (point.y >= 0 && point.y < current_topbar_height_ &&
          point.x >= 0 && point.x < client.right) {
        RECT search{};
        GetWindowRect(command_search_, &search);
        POINT screen_point{GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
        if (!IsWindowVisible(command_search_) || !PtInRect(&search, screen_point))
          return HTCAPTION;
      }
      return native;
    }
    case kActiveLinePresentationMessage:
      ApplyPendingActiveLinePresentations();
      return 0;
    case WM_CREATE:
      CreateMenuBar();
      CreateControls();
      DragAcceptFiles(window_, TRUE);
      return 0;
    case WM_SIZE:
      LayoutControls();
      InvalidateRect(window_, nullptr, FALSE);
      return 0;
    case WM_GETMINMAXINFO: {
      DefWindowProcW(window_, message, wparam, lparam);
      const auto* calendar = panel_layout_.Find(PanelId::Calendar);
      if (!calendar || calendar->hidden || calendar->collapsed || !lparam) return 0;
      const bool left = calendar->slot == PanelSlot::LeftTop || calendar->slot == PanelSlot::LeftBottom;
      const bool neighbor_visible = std::ranges::any_of(panel_layout_.panels(), [&](const auto& panel) {
        if (panel.id == PanelId::Calendar || panel.hidden || panel.collapsed ||
            (panel.id == PanelId::Explorer && workspace_pane_collapsed_) ||
            (panel.id == PanelId::Outline && outline_pane_collapsed_)) return false;
        return left == (panel.slot == PanelSlot::LeftTop || panel.slot == PanelSlot::LeftBottom);
      });
      const int minimum_side = MinimumCalendarPanelHeight(ScaleDip(window_, kMinimumPaneWidth)) +
          (neighbor_visible ? ScaleDip(window_, kTabHeight) + 1 + ScaleDip(window_, 4) : 0);
      RECT minimum_client{0, 0, 0, minimum_side + ScaleDip(window_, kTopbarHeight) +
          std::max(ScaleDip(window_, 26), current_status_height_)};
      if (AdjustWindowRectExForDpi(&minimum_client,
              static_cast<DWORD>(GetWindowLongPtrW(window_, GWL_STYLE)), GetMenu(window_) != nullptr,
              static_cast<DWORD>(GetWindowLongPtrW(window_, GWL_EXSTYLE)), GetDpiForWindow(window_))) {
        auto* information = reinterpret_cast<MINMAXINFO*>(lparam);
        information->ptMinTrackSize.y = std::max(information->ptMinTrackSize.y,
                                                minimum_client.bottom - minimum_client.top);
      }
      return 0;
    }
    case WM_PAINT: {
      PAINTSTRUCT paint{};
      HDC dc = BeginPaint(window_, &paint);
      HGDIOBJ previous_pen = SelectObject(dc, GetStockObject(DC_PEN));
      const COLORREF splitter_color = splitter_drag_ == SplitterDrag::None
          ? theme_border_ : theme_accent_;
      SetDCPenColor(dc, splitter_color);
      RECT client{};
      GetClientRect(window_, &client);
      const int bottom = static_cast<int>(client.bottom) - current_status_height_;
      if (left_width_splitter_x_ >= 0) {
        MoveToEx(dc, left_width_splitter_x_, current_topbar_height_, nullptr);
        LineTo(dc, left_width_splitter_x_, bottom);
      }
      if (right_width_splitter_x_ >= 0) {
        MoveToEx(dc, right_width_splitter_x_, current_topbar_height_, nullptr);
        LineTo(dc, right_width_splitter_x_, bottom);
      }
      if (left_height_splitter_visible_) {
        MoveToEx(dc, current_rail_width_, left_height_splitter_y_, nullptr);
        LineTo(dc, current_rail_width_ + current_tree_width_, left_height_splitter_y_);
      }
      if (right_height_splitter_visible_) {
        MoveToEx(dc, static_cast<int>(client.right) - current_outline_width_,
                 right_height_splitter_y_, nullptr);
        LineTo(dc, static_cast<int>(client.right), right_height_splitter_y_);
      }
      SelectObject(dc, previous_pen);
      EndPaint(window_, &paint);
      return 0;
    }
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
        CompleteGitActionDeliveryFailure();
        CompleteGitStatusDeliveryFailure();
        if (external_operation_active_) return 0;
        const ULONGLONG now = GetTickCount64();
        bool pending = git_status_worker_.joinable();
        bool fast_poll = git_status_worker_.joinable();
        ULONGLONG next_asset_check = std::numeric_limits<ULONGLONG>::max();
        if (editor_projection_repair_retry_due_ != 0) {
          pending = true;
          fast_poll = true;
          if (now >= editor_projection_repair_retry_due_) {
            editor_projection_repair_retry_due_ = 0;
            if (!editor_projection_repair_message_posted_) {
              editor_projection_repair_message_posted_ = PostMessageW(
                  window_, kRepairInvalidEditorProjectionMessage, 0, 0) != FALSE;
              if (!editor_projection_repair_message_posted_)
                editor_projection_repair_retry_due_ = now + 1000;
            }
          }
        }
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
              if (!SyncDocumentFromEditor(*view)) continue;
              if (IsMarkdownFile(view->document.path()))
                SchedulePresentation(*view);
            }
          }
          if (view->presentation_due != 0) {
            pending = true;
            fast_poll = true;
            if (!view->ime_composing && !view->native_edit_in_flight &&
                !view->native_edit_pending && view->sync_due == 0 &&
                now >= view->presentation_due) {
              view->presentation_due = 0;
              if (!view->editor) {
                view->presentation_revision = std::numeric_limits<std::uint64_t>::max();
              } else if (IsMarkdownFile(view->document.path())) {
                ApplyMarkdownPresentation(*view, true);
                if (active_document_ < documents_.size() &&
                    documents_[active_document_].get() == view.get()) RebuildOutline(*view);
              }
            }
          }
          if (view->editor && !view->rendered_images.empty()) {
            pending = true;
            if (view->image_asset_check_due == 0 || now >= view->image_asset_check_due)
              RefreshDerivedImages(*view);
            if (view->image_asset_check_due != 0)
              next_asset_check = std::min(next_asset_check, view->image_asset_check_due);
          }
          if (view->editor && !view->animated_image_frames.empty()) {
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
          auto& view = *documents_[active_document_];
          if (view.ime_composing || view.native_readback_failed ||
              view.editor_projection_invalid || !IsMarkdownFile(view.document.path())) continue;
          if (!SyncDocumentFromEditor(view)) {
            SetStatusText(L"入力内容を読み取れなかったため画像の挿入を中止しました。再試行してください。");
            continue;
          }
          const SourceSelection selection_before = view.editor && IsWindow(view.editor)
              ? CaptureSourceSelection(view.editor, view.editor_snapshot)
              : view.suspended_selection;
          AssetImportResult imported{};
          std::wstring error;
          const auto& document_path = view.document.path();
          if (!ImportImageAsset(path, workspace_, document_path, imported, error)) {
            MessageBoxW(window_, error.c_str(), L"画像の挿入", MB_ICONWARNING);
            continue;
          }
          const std::wstring markup = ImageMarkdown(std::filesystem::path(path).stem().wstring(),
                                                     imported.relative_reference);
          std::wstring source = view.document.text();
          const std::size_t begin = std::min({selection_before.anchor, selection_before.active,
                                              source.size()});
          const std::size_t end = std::min(std::max(selection_before.anchor, selection_before.active),
                                            source.size());
          source.replace(begin, end - begin, markup);
          const std::size_t caret_after = begin + markup.size();
          if (!ApplySourceTextWithUndo(view, std::move(source), true,
                                       SourceSelection{caret_after, caret_after}, selection_before)) {
            if (imported.created_new_asset) {
              std::error_code remove_error;
              std::filesystem::remove(imported.stored_path, remove_error);
            }
            SetStatusText(L"本文が同期中のため画像の挿入を中止しました。再試行してください。");
            continue;
          }
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
      if (splitter_drag_ != SplitterDrag::None) {
        RECT client{};
        GetClientRect(window_, &client);
        const int x = static_cast<short>(LOWORD(lparam));
        const int y = static_cast<short>(HIWORD(lparam));
        const int dpi = std::max(1, static_cast<int>(GetDpiForWindow(window_)));
        const int minimum_panel = ScaleDip(window_, kMinimumPaneWidth);
        const int minimum_editor = ScaleDip(window_, kMinimumEditorWidth);
        std::wstring error;
        PanelLayout resized = panel_layout_;
        if (splitter_drag_ == SplitterDrag::LeftWidth ||
            splitter_drag_ == SplitterDrag::RightWidth) {
          const bool left = splitter_drag_ == SplitterDrag::LeftWidth;
          const bool other_side_visible = left ? right_width_splitter_x_ >= 0
                                                : left_width_splitter_x_ >= 0;
          const int other_width = left ? current_outline_width_ : current_tree_width_;
          const int available = static_cast<int>(client.right) - current_rail_width_ - minimum_editor -
              other_width - (other_side_visible ? current_splitter_width_ * 2 : current_splitter_width_);
          const int maximum_panel = std::max(minimum_panel, available);
          const int target_pixels = left
              ? x - current_rail_width_ - current_splitter_width_ / 2
              : static_cast<int>(client.right) - x - current_splitter_width_ / 2;
          const int clamped_pixels = std::clamp(target_pixels, minimum_panel, maximum_panel);
          const double target_width = static_cast<double>(MulDiv(clamped_pixels, 96, dpi));
          const PanelId first = left ? PanelId::Explorer : PanelId::Outline;
          const PanelId second = left ? PanelId::Calendar : PanelId::Git;
          const auto* first_panel = resized.Find(first);
          const auto* second_panel = resized.Find(second);
          if (first_panel && second_panel &&
              resized.Resize(first, target_width, first_panel->height, error) &&
              resized.Resize(second, target_width, second_panel->height, error)) {
            panel_layout_ = std::move(resized);
            LayoutControls();
            InvalidateRect(window_, nullptr, FALSE);
          }
        } else {
          const bool left = splitter_drag_ == SplitterDrag::LeftHeight;
          const int side_y = y - current_topbar_height_ - current_splitter_width_ / 2;
          const int available = std::max(0, static_cast<int>(client.bottom) - current_status_height_ -
                                           current_topbar_height_ - current_splitter_width_);
          const int top_pixels = std::clamp(side_y, minimum_panel,
                                            std::max(minimum_panel, available - minimum_panel));
          const int bottom_pixels = std::max(minimum_panel, available - top_pixels);
          const double top_height = static_cast<double>(MulDiv(top_pixels, 96, dpi));
          const double bottom_height = static_cast<double>(MulDiv(bottom_pixels, 96, dpi));
          const PanelSlot top_slot = left ? PanelSlot::LeftTop : PanelSlot::RightTop;
          const PanelSlot bottom_slot = left ? PanelSlot::LeftBottom : PanelSlot::RightBottom;
          PanelId top_id{};
          PanelId bottom_id{};
          bool has_top{};
          bool has_bottom{};
          for (const auto& panel : resized.panels()) {
            if (panel.slot == top_slot) { top_id = panel.id; has_top = true; }
            if (panel.slot == bottom_slot) { bottom_id = panel.id; has_bottom = true; }
          }
          PanelState* top_panel = has_top ? resized.Find(top_id) : nullptr;
          PanelState* bottom_panel = has_bottom ? resized.Find(bottom_id) : nullptr;
          if (top_panel && bottom_panel &&
              resized.Resize(top_panel->id, top_panel->width, top_height, error) &&
              resized.Resize(bottom_panel->id, bottom_panel->width, bottom_height, error)) {
            panel_layout_ = std::move(resized);
            LayoutControls();
            InvalidateRect(window_, nullptr, FALSE);
          }
        }
        SetCursor(LoadCursorW(nullptr, splitter_drag_ == SplitterDrag::LeftWidth ||
                              splitter_drag_ == SplitterDrag::RightWidth ? IDC_SIZEWE : IDC_SIZENS));
        return 0;
      }
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
    case WM_SETCURSOR: {
      if (splitter_drag_ != SplitterDrag::None) {
        SetCursor(LoadCursorW(nullptr,
            splitter_drag_ == SplitterDrag::LeftWidth || splitter_drag_ == SplitterDrag::RightWidth
                ? IDC_SIZEWE : IDC_SIZENS));
        return TRUE;
      }
      if (LOWORD(lparam) == HTCLIENT) {
        POINT point{};
        GetCursorPos(&point);
        ScreenToClient(window_, &point);
        RECT client{};
        GetClientRect(window_, &client);
        const int tolerance = std::max(2, current_splitter_width_);
        const bool inside_content = point.y >= current_topbar_height_ &&
                                    point.y < client.bottom - current_status_height_;
        const bool width_splitter =
            inside_content &&
            ((left_width_splitter_x_ >= 0 && std::abs(point.x - left_width_splitter_x_) <= tolerance) ||
             (right_width_splitter_x_ >= 0 && std::abs(point.x - right_width_splitter_x_) <= tolerance));
        const bool height_splitter =
            (left_height_splitter_visible_ && point.x >= current_rail_width_ &&
             point.x <= current_rail_width_ + current_tree_width_ &&
             std::abs(point.y - left_height_splitter_y_) <= tolerance) ||
            (right_height_splitter_visible_ &&
             point.x >= client.right - current_outline_width_ &&
             std::abs(point.y - right_height_splitter_y_) <= tolerance);
        if (width_splitter || height_splitter) {
          SetCursor(LoadCursorW(nullptr, width_splitter ? IDC_SIZEWE : IDC_SIZENS));
          return TRUE;
        }
      }
      return DefWindowProcW(window_, message, wparam, lparam);
    }
    case WM_LBUTTONDOWN: {
      const int x = static_cast<short>(LOWORD(lparam));
      const int y = static_cast<short>(HIWORD(lparam));
      const int tolerance = std::max(2, current_splitter_width_);
      RECT client{};
      GetClientRect(window_, &client);
      const bool inside_content = y >= current_topbar_height_ &&
                                  y < client.bottom - current_status_height_;
      if (inside_content && left_width_splitter_x_ >= 0 &&
          std::abs(x - left_width_splitter_x_) <= tolerance) {
        splitter_drag_ = SplitterDrag::LeftWidth;
      } else if (inside_content && right_width_splitter_x_ >= 0 &&
                 std::abs(x - right_width_splitter_x_) <= tolerance) {
        splitter_drag_ = SplitterDrag::RightWidth;
      } else if (left_height_splitter_visible_ &&
                 x >= current_rail_width_ && x <= current_rail_width_ + current_tree_width_ &&
                 std::abs(y - left_height_splitter_y_) <= tolerance) {
        splitter_drag_ = SplitterDrag::LeftHeight;
      } else if (right_height_splitter_visible_ &&
                 x >= client.right - current_outline_width_ &&
                 std::abs(y - right_height_splitter_y_) <= tolerance) {
        splitter_drag_ = SplitterDrag::RightHeight;
      }
      if (splitter_drag_ != SplitterDrag::None) {
        SetCapture(window_);
        SetCursor(LoadCursorW(nullptr, splitter_drag_ == SplitterDrag::LeftWidth ||
                              splitter_drag_ == SplitterDrag::RightWidth ? IDC_SIZEWE : IDC_SIZENS));
        return 0;
      }
      return DefWindowProcW(window_, message, wparam, lparam);
    }
    case WM_LBUTTONUP:
      if (splitter_drag_ != SplitterDrag::None) {
        splitter_drag_ = SplitterDrag::None;
        if (GetCapture() == window_) ReleaseCapture();
        SavePanelLayout();
        InvalidateRect(window_, nullptr, FALSE);
        SetStatusText(L"パネル配置を保存しました。");
        return 0;
      }
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
      if (splitter_drag_ != SplitterDrag::None) {
        splitter_drag_ = SplitterDrag::None;
        SavePanelLayout();
        InvalidateRect(window_, nullptr, FALSE);
      }
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
    case kGitActionCompleteMessage:
      CompleteGitAction(reinterpret_cast<void*>(lparam));
      return 0;
    case kGitStatusCompleteMessage:
      CompleteGitStatus(reinterpret_cast<void*>(lparam));
      return 0;
    case kTestFailNextEditorReadbackMessage:
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return FALSE;
      {
        const auto requested = static_cast<std::size_t>(wparam);
        documents_[active_document_]->force_editor_readback_failures_for_test =
            static_cast<unsigned>(std::min<std::size_t>(
                requested == 0 ? 1 : requested, 16));
      }
      return TRUE;
    case kTestFailNextMarkdownPresentationMessage:
      if (!TestAutomationSilent() || wparam >= documents_.size() || lparam < 0 || lparam > 2) return FALSE;
      if (lparam == 2) {
        auto& view = *documents_[static_cast<std::size_t>(wparam)];
        if (!view.editor || !IsWindow(view.editor) || view.editor_projection_invalid ||
            view.native_readback_failed || view.sync_due != 0 || view.native_edit_in_flight ||
            view.native_edit_pending || view.ime_composing || view.editor_snapshot.tables.empty()) return FALSE;
        view.force_table_cell_formatting_failure_for_test = true;
        ApplyMarkdownPresentation(view, true);
      } else if (lparam == 1)
        documents_[static_cast<std::size_t>(wparam)]->force_table_cell_formatting_failure_for_test = true;
      else
        documents_[static_cast<std::size_t>(wparam)]->force_markdown_presentation_failure_for_test = true;
      return TRUE;
    case kTestFailMarkdownPresentationAfterFirstTableMessage:
      if (!TestAutomationSilent() || wparam >= documents_.size()) return FALSE;
      if (documents_[static_cast<std::size_t>(wparam)]->parse.tables.size() < 2) return FALSE;
      documents_[static_cast<std::size_t>(wparam)]->force_partial_markdown_presentation_failure_for_test = true;
      return TRUE;
    case kTestSetNativeProjectionFailureStagesMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return FALSE;
      const auto mask = static_cast<std::uint32_t>(wparam);
      constexpr std::uint32_t supported = kNativeProjectionFaultProgrammaticWrite |
          kNativeProjectionFaultProgrammaticRestore | kNativeProjectionFaultIncrementalWrite |
          kNativeProjectionFaultRebuildWrite | kNativeProjectionFaultFlatFallback;
      if (static_cast<WPARAM>(mask) != wparam || (mask & ~supported) != 0) return FALSE;
      documents_[active_document_]->native_projection_failures_for_test = mask;
      return TRUE;
    }
    case kTestRefreshWorkspaceTreeMessage:
      if (!TestAutomationSilent() || !workspace_tree_) return FALSE;
      PopulateWorkspaceTree();
      return TRUE;
    case kTestTrustWorkspaceForGitMessage: {
      if (!TestAutomationSilent() || workspace_.empty() || !workspace_store_ || git_action_active_)
        return FALSE;
      if (wparam != 0 && wparam != 1) return FALSE;
      const auto test_trust_root = workspace_store_->metadata_root() / L".state" / L"test-trust";
      SetTrustStoreRootForTesting(test_trust_root);
      test_trust_ui_enabled = wparam == 1;
      if (test_trust_ui_enabled) return TRUE;
      std::wstring error;
      if (!SetWorkspaceTrusted(workspace_, true, error)) {
        SetStatusText(error);
        return FALSE;
      }
      LayoutControls();
      RunGitStatus();
      return TRUE;
    }
    case kTestGetGitActionActiveMessage:
      if (!TestAutomationSilent()) return -1;
      return git_action_active_ ? TRUE : FALSE;
    case kRepairInvalidEditorProjectionMessage:
      editor_projection_repair_message_posted_ = false;
      ProcessDeferredEditorRepairs();
      return 0;
    case kTestGetCalendarHoverCellMessage: {
      if (!TestAutomationSilent() || !calendar_) return -1;
      const auto hovered = CalendarView_GetHoveredDate(calendar_);
      const auto month = CalendarView_GetDisplayedMonth(calendar_);
      if (!hovered || !month) return 0;
      const auto dates = GetCalendarViewMonthDates(*month);
      const auto found = std::ranges::find(dates, hovered);
      if (found == dates.end()) return 0;
      return static_cast<LRESULT>(std::distance(dates.begin(), found) + 1);
    }
    case kTestGetCharFormatAtSourceRangeMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return 0;
      auto& view = *documents_[active_document_];
      const auto source_begin = static_cast<std::size_t>(wparam);
      const auto source_end = static_cast<std::size_t>(lparam);
      if (source_begin >= source_end || source_end > view.document.text().size()) return 0;
      const auto native_begin = view.editor_snapshot.SourceToNative(source_begin);
      const auto native_end = view.editor_snapshot.SourceToNative(source_end);
      IRichEditOle* rich_edit{};
      ITextDocument* document{};
      ITextRange* range{};
      ITextFont* font{};
      LONG hidden = tomUndefined;
      const bool inspected = SendMessageW(view.editor, EM_GETOLEINTERFACE, 0,
          reinterpret_cast<LPARAM>(&rich_edit)) && rich_edit &&
          SUCCEEDED(rich_edit->QueryInterface(__uuidof(ITextDocument),
              reinterpret_cast<void**>(&document))) && document &&
          SUCCEEDED(document->Range(static_cast<LONG>(native_begin),
              static_cast<LONG>(native_end), &range)) && range &&
          SUCCEEDED(range->GetFont(&font)) && font && SUCCEEDED(font->GetHidden(&hidden));
      if (font) font->Release();
      if (range) range->Release();
      if (document) document->Release();
      if (rich_edit) rich_edit->Release();
      if (!inspected || hidden == tomUndefined) return 0;
      const auto packed = (static_cast<std::uint64_t>(CFM_HIDDEN) << 32) |
                          (hidden == tomTrue ? CFE_HIDDEN : 0);
      return static_cast<LRESULT>(packed);
    }
    case kTestSetSelectionBySourceMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return FALSE;
      auto& view = *documents_[active_document_];
      const auto source_size = view.document.text().size();
      const SourceSelection selection{
          std::min<std::size_t>(static_cast<std::size_t>(wparam), source_size),
          std::min<std::size_t>(static_cast<std::size_t>(lparam), source_size)};
      RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
      SetFocus(view.editor);
      return TRUE;
    }
    case kTestGetSourceAnchorMessage:
    case kTestGetSourceActiveMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto selection = CaptureSourceSelection(
          documents_[active_document_]->editor,
          documents_[active_document_]->editor_snapshot);
      return static_cast<LRESULT>(message == kTestGetSourceAnchorMessage
                                      ? selection.anchor : selection.active);
    }
    case kTestGetSourcePositionMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      auto& view = *documents_[active_document_];
      const auto source_position = std::min<std::size_t>(
          static_cast<std::size_t>(wparam), view.document.text().size());
      const LONG native = static_cast<LONG>(view.editor_snapshot.SourceToNative(source_position));
      POINT point{};
      if (SendMessageW(view.editor, EM_POSFROMCHAR,
                       reinterpret_cast<WPARAM>(&point), native) == -1) return -1;
      const auto packed = static_cast<std::uint32_t>(static_cast<std::uint16_t>(point.x)) |
          (static_cast<std::uint32_t>(static_cast<std::uint16_t>(point.y)) << 16U);
      return static_cast<LRESULT>(packed);
    }
    case kTestGetEditorReadinessMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return 0;
      const auto& view = *documents_[active_document_];
      const bool presentation_ready = view.presentation_revision == view.document.revision() &&
                                      view.presentation_due == 0;
      const bool source_sync_ready = view.sync_due == 0 && !view.native_edit_pending &&
                                     !view.native_edit_in_flight;
      const bool gfm_table_projection_ready = !view.parse.tables.empty() &&
          !view.editor_snapshot.tables.empty() && view.native_tables_ready;
      return (presentation_ready ? 1 : 0) | (view.native_tables_ready ? 2 : 0) |
             (source_sync_ready ? 4 : 0) |
             (view.pending_virtual_table_cell ? 8 : 0) | (view.sync_due != 0 ? 16 : 0) |
             (view.native_edit_pending ? 32 : 0) | (view.native_edit_in_flight ? 64 : 0) |
             (view.presentation_due != 0 ? 128 : 0) |
             (gfm_table_projection_ready ? 256 : 0);
    }
    case kTestGetSourceAtPointMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      auto& view = *documents_[active_document_];
      const POINT point{static_cast<short>(LOWORD(lparam)),
                        static_cast<short>(HIWORD(lparam))};
      const LRESULT native = SendMessageW(view.editor, EM_CHARFROMPOS, 0,
                                          reinterpret_cast<LPARAM>(&point));
      if (native < 0) return -1;
      return static_cast<LRESULT>(view.editor_snapshot.NativeToSource(
          static_cast<std::size_t>(native)));
    }
    case kTestGetNativeAtPointMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      auto& view = *documents_[active_document_];
      const POINT point{static_cast<short>(LOWORD(lparam)),
                        static_cast<short>(HIWORD(lparam))};
      return SendMessageW(view.editor, EM_CHARFROMPOS, 0,
                          reinterpret_cast<LPARAM>(&point));
    }
    case kTestGetDocumentStateMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto& view = *documents_[active_document_];
      // Scalar-only inspection avoids cross-process pointers and does not save,
      // synchronize, or otherwise change the state being verified.
      switch (wparam) {
        case 0: return view.document.dirty();
        case 1: return static_cast<LRESULT>(view.source_undo.size());
        case 2: return static_cast<LRESULT>(view.source_redo.size());
        case 3: return static_cast<LRESULT>(view.document.revision());
        case 4: return static_cast<LRESULT>(view.document.text().size());
        case 6: return static_cast<LRESULT>(view.document.saved_revision());
        case 5:
          return static_cast<std::size_t>(lparam) < view.document.text().size()
              ? static_cast<LRESULT>(view.document.text()[static_cast<std::size_t>(lparam)]) : -1;
        default: return -1;
      }
    }
    case kTestGetLastHistoryBeforeMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto& history = documents_[active_document_]->source_undo;
      if (history.empty()) return -1;
      return static_cast<LRESULT>(wparam == 0 ? history.back().selection_before.anchor
                                               : history.back().selection_before.active);
    }
    case kTestGetTabItemCenterMessage: {
      if (!TestAutomationSilent() || !tabs_) return -1;
      const int index = static_cast<int>(wparam);
      if (index < 0 || index >= TabCtrl_GetItemCount(tabs_)) return -1;
      RECT rectangle{};
      if (!TabCtrl_GetItemRect(tabs_, index, &rectangle)) return -1;
      const LONG x = (rectangle.left + rectangle.right) / 2;
      const LONG y = (rectangle.top + rectangle.bottom) / 2;
      if (x < SHRT_MIN || x > SHRT_MAX || y < SHRT_MIN || y > SHRT_MAX) return -1;
      const auto packed = static_cast<std::uint32_t>(static_cast<std::uint16_t>(x)) |
          (static_cast<std::uint32_t>(static_cast<std::uint16_t>(y)) << 16U);
      return static_cast<LRESULT>(packed);
    }
    case kTestGetSuspendedViewAnchorLineMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto& view = *documents_[active_document_];
      if (!view.suspended_view_state_valid || !view.editor || !IsWindow(view.editor)) return -1;
      const auto native_anchor = static_cast<LONG>(
          view.editor_snapshot.SourceToNative(view.suspended_first_visible_source));
      return SendMessageW(view.editor, EM_LINEFROMCHAR, native_anchor, 0);
    }
    case kTestGetVisibleSourceOffsetMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto& view = *documents_[active_document_];
      if (!view.editor || !IsWindow(view.editor)) return -1;
      const auto source_offset = CaptureVisibleLeftEdgeSourceOffset(
          view.editor, view.editor_snapshot);
      return source_offset ? static_cast<LRESULT>(*source_offset) : -1;
    }
    case kTestGetVerticalSourceOffsetMessage: {
      if (!TestAutomationSilent() || active_document_ >= documents_.size()) return -1;
      const auto& view = *documents_[active_document_];
      if (!view.editor || !IsWindow(view.editor)) return -1;
      const LRESULT first_visible_line =
          SendMessageW(view.editor, EM_GETFIRSTVISIBLELINE, 0, 0);
      const LRESULT native_position = SendMessageW(
          view.editor, EM_LINEINDEX,
          static_cast<WPARAM>(std::max<LRESULT>(0, first_visible_line)), 0);
      if (native_position < 0) return -1;
      return static_cast<LRESULT>(view.editor_snapshot.NativeToSource(
          static_cast<std::size_t>(native_position)));
    }
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
      if (HIWORD(wparam) == 0)
        for (auto& view : documents_) view->pending_body_high_surrogate = 0;
      const int command = LOWORD(wparam);
      if (command == kCommandSearch) ShowCommandPalette();
      else if (command == kTabNew) NewUntitledDocument();
      else if (command == kTabClose && active_document_ < documents_.size())
        CloseDocument(active_document_);
      else if (command == kActivityExplorer) ActivatePanel(PanelId::Explorer);
      else if (command == kActivitySearch) ShowFindBar();
      else if (command == kActivityGit) ActivatePanel(PanelId::Git);
      else if (command == kActivityCalendar) ActivatePanel(PanelId::Calendar);
      else if (command == kActivitySettings) OpenWorkspaceSettings();
      if (command == kPanelHeaderExplorer || command == kPanelHeaderCalendar ||
          command == kPanelHeaderOutline || command == kPanelHeaderGit) {
        focused_panel_ = command == kPanelHeaderExplorer ? PanelId::Explorer :
            command == kPanelHeaderCalendar ? PanelId::Calendar :
            command == kPanelHeaderOutline ? PanelId::Outline : PanelId::Git;
        active_activity_ = focused_panel_ == PanelId::Explorer ? 0 :
            focused_panel_ == PanelId::Git ? 2 : focused_panel_ == PanelId::Calendar ? 3 : -1;
        SetFocus(panel_headers_[PanelIndex(focused_panel_)]);
        UpdatePanelHeaders();
      }
      else if (command == kViewMoveFocusedLeftTop) MoveFocusedPanelToSlot(PanelSlot::LeftTop);
      else if (command == kViewMoveFocusedLeftBottom) MoveFocusedPanelToSlot(PanelSlot::LeftBottom);
      else if (command == kViewResizeFocusedNarrow) ResizeFocusedPanel(-24.0);
      else if (command == kViewResizeFocusedWide) ResizeFocusedPanel(24.0);
      else if (command == kViewResizeFocusedShorter) ResizeFocusedPanelHeight(-24.0);
      else if (command == kViewResizeFocusedTaller) ResizeFocusedPanelHeight(24.0);
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
      else if (command == kCalendarGoToToday) GoToCalendarToday();
      else if (command == kCalendarDetailsToggle) {
        calendar_details_expanded_ = !calendar_details_expanded_;
        SetWindowTextW(calendar_details_toggle_, calendar_details_expanded_ ? L"詳細を閉じる" : L"詳細を表示");
        LayoutControls();
        SetFocus(calendar_details_expanded_ ? calendar_details_ : calendar_details_toggle_);
      }
      else if (command == kCalendarImportHolidays) ImportHolidayData();
      else if (command == kViewSettings) OpenWorkspaceSettings();
      else if (command == kViewSettingsFiles) OpenWorkspaceSettingsFiles();
      else if (command == kViewProfiles) ManageProfiles();
      else if (command == kViewCompact) ToggleCompactWindow();
      else if (command == kViewDiagnostics) ShowDiagnostics();
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
      else if (command == kViewGitPane) {
        if (auto* panel = panel_layout_.Find(PanelId::Git)) panel->hidden = !panel->hidden;
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
      if (draw->hwndItem == tabs_ && draw->CtlType == ODT_TAB) {
        DrawTabItem(*draw);
        return TRUE;
      }
      if (draw->CtlType == ODT_MENU) {
        DrawMenuItem(*draw);
        return TRUE;
      }
      if (draw->hwndItem == status_) {
        DrawStatusItem(*draw);
        return TRUE;
      }
      if (draw->hwndItem == chrome_bar_ || draw->hwndItem == activity_rail_ ||
          draw->hwndItem == brand_ || draw->hwndItem == command_search_ ||
          draw->hwndItem == tab_new_ || draw->hwndItem == tab_close_ ||
          draw->hwndItem == git_refresh_ || draw->hwndItem == git_stage_ ||
          draw->hwndItem == git_unstage_ || draw->hwndItem == git_diff_ ||
          draw->hwndItem == git_commit_ || draw->hwndItem == git_trust_ ||
          draw->hwndItem == calendar_daily_ || draw->hwndItem == calendar_details_toggle_ ||
          std::ranges::find(activity_buttons_, draw->hwndItem) != activity_buttons_.end() ||
          std::ranges::find(panel_headers_, draw->hwndItem) != panel_headers_.end()) {
        DrawChromeButton(*draw);
        return TRUE;
      }
      return FALSE;
    }
    case WM_NOTIFY: {
      const auto* header = reinterpret_cast<NMHDR*>(lparam);
      if (header && header->hwndFrom == chrome_tooltip_ && header->code == TTN_GETDISPINFOW) {
        auto* info = reinterpret_cast<NMTTDISPINFOW*>(lparam);
        GetWindowTextW(reinterpret_cast<HWND>(header->idFrom), info->szText,
                       static_cast<int>(std::size(info->szText)));
        return 0;
      }
      if (header && header->hwndFrom == git_files_ && header->code == NM_CUSTOMDRAW) {
        auto* draw = reinterpret_cast<NMLVCUSTOMDRAW*>(lparam);
        if (draw->nmcd.dwDrawStage == CDDS_PREPAINT) return CDRF_NOTIFYITEMDRAW;
        if (draw->nmcd.dwDrawStage == CDDS_ITEMPREPAINT) return CDRF_NOTIFYSUBITEMDRAW;
        if (draw->nmcd.dwDrawStage == (CDDS_ITEMPREPAINT | CDDS_SUBITEM)) {
          const bool selected = (draw->nmcd.uItemState & CDIS_SELECTED) != 0;
          draw->clrText = selected ? theme_selected_text_ : theme_foreground_;
          draw->clrTextBk = selected ? theme_accent_ : theme_surface_;
          return CDRF_NEWFONT;
        }
      }
      if (header && header->code == NM_CUSTOMDRAW &&
          (header->hwndFrom == workspace_tree_ || header->hwndFrom == outline_ ||
           header->hwndFrom == find_results_ ||
           header->hwndFrom == status_)) {
        auto* draw = reinterpret_cast<NMCUSTOMDRAW*>(lparam);
        if (draw->dwDrawStage == CDDS_PREPAINT) return CDRF_NOTIFYITEMDRAW;
        if (draw->dwDrawStage == CDDS_ITEMPREPAINT) {
          const bool selected = (draw->uItemState & CDIS_SELECTED) != 0;
          SetTextColor(draw->hdc, selected ? theme_selected_text_ : theme_foreground_);
          SetBkColor(draw->hdc, selected ? theme_accent_ : theme_surface_);
          return CDRF_DODEFAULT;
        }
      }
      if (header->hwndFrom == tabs_ && header->code == TCN_SELCHANGE) {
        const int index = TabCtrl_GetCurSel(tabs_);
        if (index >= 0) ActivateDocument(static_cast<std::size_t>(index));
      } else if (header->hwndFrom == git_files_ && header->code == LVN_ITEMCHANGED) {
        UpdateGitPanelActions();
      } else if (header->hwndFrom == git_files_ &&
                 (header->code == NM_DBLCLK || header->code == LVN_ITEMACTIVATE)) {
        ShowSelectedGitDiff();
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
        if (!documents_[active_document_]->editor) ActivateDocument(active_document_);
        if (!documents_[active_document_]->editor) return 0;
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
      } else if (header->hwndFrom == calendar_ && header->code == CVN_DATE_ACTIVATED) {
        const auto* activation = reinterpret_cast<const CalendarViewDateActivatedNotification*>(lparam);
        OpenCalendarDate(activation->date);
      } else if (header->hwndFrom == calendar_ && header->code == CVN_DATE_HOVERED) {
        const auto* hover = reinterpret_cast<const CalendarViewDateHoveredNotification*>(lparam);
        UpdateCalendarTooltip(hover->has_date ? std::optional<CalendarDate>{hover->date}
                                              : std::nullopt);
      } else if (header->hwndFrom == calendar_ && header->code == CVN_DATE_SELECTED) {
        const auto* selection = reinterpret_cast<const CalendarViewDateSelectedNotification*>(lparam);
        UpdateCalendarDetails(selection->date);
      } else if (header->hwndFrom == calendar_ && header->code == CVN_MONTH_CHANGED) {
        UpdateCalendarViewMarkers();
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
         QueueActiveLinePresentation(*documents_[active_document_]);
        UpdateStatus();
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
      StopGitActionWorker();
      StopGitStatusWorker();
      ++workspace_search_generation_;
      MSG pending_search{};
      while (PeekMessageW(&pending_search, window_, kWorkspaceSearchBatchMessage,
                          kWorkspaceSearchCompleteMessage, PM_REMOVE)) {
        if (pending_search.message == kWorkspaceSearchBatchMessage)
          delete reinterpret_cast<WorkspaceSearchBatchMessage*>(pending_search.lParam);
        else if (pending_search.message == kWorkspaceSearchCompleteMessage)
          delete reinterpret_cast<WorkspaceSearchCompleteMessage*>(pending_search.lParam);
      }
      MSG pending_git_status{};
      while (PeekMessageW(&pending_git_status, window_, kGitStatusCompleteMessage,
                          kGitStatusCompleteMessage, PM_REMOVE))
        delete reinterpret_cast<GitStatusCompleteMessage*>(pending_git_status.lParam);
      MSG pending_git_action{};
      while (PeekMessageW(&pending_git_action, window_, kGitActionCompleteMessage,
                          kGitActionCompleteMessage, PM_REMOVE))
        delete reinterpret_cast<GitActionCompleteMessage*>(pending_git_action.lParam);
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
      if (settings_.theme == ThemeMode::System || wparam == SPI_SETHIGHCONTRAST) ApplySettings();
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
          control == find_include_glob_ || control == find_exclude_glob_ ||
          control == git_commit_edit_;
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
  AppendMenuW(view, MF_STRING, kViewCommandPalette, L"コマンドパレット…\tCtrl+K");
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
  AppendMenuW(git, MF_STRING, kGitStageAll, L"選択ファイルをステージ");
  AppendMenuW(git, MF_STRING, kGitUnstageAll, L"選択ファイルのステージを解除");
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
}

void Application::CreateControls() {
  chrome_bar_ = CreateWindowExW(0, L"STATIC", nullptr, WS_CHILD | WS_VISIBLE | SS_OWNERDRAW,
                                0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kChromeBar),
                                instance_, nullptr);
  activity_rail_ = CreateWindowExW(0, L"STATIC", nullptr, WS_CHILD | WS_VISIBLE | SS_OWNERDRAW,
                                   0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kActivityRail),
                                   instance_, nullptr);
  workspace_tree_ = CreateWindowExW(0, WC_TREEVIEWW, nullptr,
                                    WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_SHOWSELALWAYS,
                                    0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kWorkspaceTree), instance_, nullptr);
  if (workspace_tree_) {
    SetWindowTheme(workspace_tree_, L"Explorer", nullptr);
    TreeView_SetExtendedStyle(workspace_tree_,
        TVS_EX_MULTISELECT | TVS_EX_FADEINOUTEXPANDOS,
        TVS_EX_MULTISELECT | TVS_EX_FADEINOUTEXPANDOS);
    SHFILEINFOW shell_image{};
    const HIMAGELIST shell_images = reinterpret_cast<HIMAGELIST>(SHGetFileInfoW(
        L".", FILE_ATTRIBUTE_DIRECTORY, &shell_image, sizeof(shell_image),
        SHGFI_SYSICONINDEX | SHGFI_SMALLICON));
    if (shell_images) TreeView_SetImageList(workspace_tree_, shell_images, TVSIL_NORMAL);
  }
  SetWindowSubclass(workspace_tree_, TreeDragSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  tabs_ = CreateWindowExW(0, WC_TABCONTROLW, nullptr,
                          WS_CHILD | WS_VISIBLE | WS_TABSTOP | TCS_BUTTONS | TCS_FLATBUTTONS | TCS_FIXEDWIDTH |
                              TCS_OWNERDRAWFIXED,
                          0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kTabs), instance_, nullptr);
  if (tabs_) {
    SetWindowSubclass(tabs_, TabStripSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
    SendMessageW(tabs_, WM_SETFONT, reinterpret_cast<WPARAM>(editor_font_), TRUE);
    SendMessageW(tabs_, TCM_SETITEMSIZE, 0,
                 MAKELPARAM(ScaleDip(window_, 172), ScaleDip(window_, kTabHeight)));
  }
  brand_ = CreateWindowExW(0, L"STATIC", nullptr, WS_CHILD | SS_OWNERDRAW,
                           0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kBrand), instance_, nullptr);
  command_search_ = CreateWindowExW(0, L"BUTTON", L"コマンドを検索...",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kCommandSearch), instance_, nullptr);
  tab_new_ = CreateWindowExW(0, L"BUTTON", L"新しい文書",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kTabNew), instance_, nullptr);
  tab_close_ = CreateWindowExW(0, L"BUTTON", L"現在の文書を閉じる",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kTabClose), instance_, nullptr);
  activity_buttons_[0] = CreateWindowExW(0, L"BUTTON", L"エクスプローラー",
      WS_CHILD | WS_TABSTOP | WS_GROUP | BS_RADIOBUTTON | BS_PUSHLIKE, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kActivityExplorer), instance_, nullptr);
  activity_buttons_[1] = CreateWindowExW(0, L"BUTTON", L"検索・置換（文書 / Workspace）",
      WS_CHILD | WS_TABSTOP | BS_RADIOBUTTON | BS_PUSHLIKE, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kActivitySearch), instance_, nullptr);
  activity_buttons_[2] = CreateWindowExW(0, L"BUTTON", L"Git（ソース管理）",
      WS_CHILD | WS_TABSTOP | BS_RADIOBUTTON | BS_PUSHLIKE, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kActivityGit), instance_, nullptr);
  activity_buttons_[3] = CreateWindowExW(0, L"BUTTON", L"カレンダー",
      WS_CHILD | WS_TABSTOP | BS_RADIOBUTTON | BS_PUSHLIKE, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kActivityCalendar), instance_, nullptr);
  activity_buttons_[4] = CreateWindowExW(0, L"BUTTON", L"Workspace設定",
      WS_CHILD | WS_GROUP | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kActivitySettings), instance_, nullptr);
  outline_ = CreateWindowExW(0, WC_TREEVIEWW, nullptr,
                             WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_HASLINES |
                                 TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                                 0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kOutline), instance_, nullptr);
  SetWindowSubclass(outline_, TreeDragSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  status_ = CreateWindowExW(0, STATUSCLASSNAMEW, nullptr,
                             WS_CHILD | WS_VISIBLE | SBARS_SIZEGRIP,
                             0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  if (status_) SetWindowSubclass(status_, StatusBarSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  SendMessageW(status_, SB_SETMINHEIGHT, ScaleDip(window_, 26), 0);
  SetStatusText(L"");
  find_bar_ = CreateWindowExW(0, L"STATIC", nullptr, WS_CHILD,
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
  find_results_ = CreateWindowExW(0, WC_LISTVIEWW, nullptr,
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
  if (CalendarView_RegisterClass(instance_)) {
    calendar_ = CalendarView_Create(window_, kCalendarView, RECT{0, 0, 0, 0});
  } else {
    SetStatusText(L"カレンダーcontrol classを登録できません。");
  }
  git_panel_ = CreateWindowExW(0, L"EDIT", L"Gitの状態を更新してください。",
                              WS_CHILD | WS_TABSTOP | ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | WS_VSCROLL,
                              0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  git_refresh_ = CreateWindowExW(0, L"BUTTON", L"更新",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitStatus), instance_, nullptr);
  git_files_ = CreateWindowExW(0, WC_LISTVIEWW, nullptr,
      WS_CHILD | LVS_REPORT | LVS_SHOWSELALWAYS | LVS_NOSORTHEADER,
      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  ListView_SetExtendedListViewStyle(git_files_, LVS_EX_FULLROWSELECT | LVS_EX_DOUBLEBUFFER);
  LVCOLUMNW git_column{LVCF_TEXT | LVCF_WIDTH};
  git_column.pszText = const_cast<wchar_t*>(L"状態");
  git_column.cx = 42;
  ListView_InsertColumn(git_files_, 0, &git_column);
  git_column.pszText = const_cast<wchar_t*>(L"ファイル");
  git_column.cx = 260;
  ListView_InsertColumn(git_files_, 1, &git_column);
  git_diff_view_ = CreateWindowExW(0, L"EDIT", L"ファイルを選択して差分を表示してください。",
      WS_CHILD | ES_MULTILINE | ES_READONLY | ES_NOHIDESEL | WS_VSCROLL | ES_AUTOVSCROLL,
      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  git_stage_ = CreateWindowExW(0, L"BUTTON", L"Stage",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitStageAll), instance_, nullptr);
  git_unstage_ = CreateWindowExW(0, L"BUTTON", L"Unstage",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitUnstageAll), instance_, nullptr);
  git_diff_ = CreateWindowExW(0, L"BUTTON", L"差分",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitDiff), instance_, nullptr);
  git_commit_edit_ = CreateWindowExW(0, L"EDIT", nullptr,
      WS_CHILD | WS_BORDER | ES_AUTOHSCROLL, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitCommitEdit), instance_, nullptr);
  SendMessageW(git_commit_edit_, EM_SETCUEBANNER, TRUE,
               reinterpret_cast<LPARAM>(L"Commit message"));
  git_commit_ = CreateWindowExW(0, L"BUTTON", L"Commit",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kGitCommit), instance_, nullptr);
  git_trust_ = CreateWindowExW(0, L"BUTTON", L"信頼を確認...",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kWorkspaceTrust), instance_, nullptr);
  calendar_summary_ = CreateWindowExW(0, L"STATIC", L"選択日: —\n文書数: 未取得",
      WS_CHILD | SS_LEFT | SS_NOPREFIX, 0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  calendar_daily_ = CreateWindowExW(0, L"BUTTON", L"Dailyを開く",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kCalendarOpenSelected), instance_, nullptr);
  calendar_details_toggle_ = CreateWindowExW(0, L"BUTTON", L"詳細を表示",
      WS_CHILD | WS_TABSTOP | BS_OWNERDRAW, 0, 0, 0, 0, window_,
      reinterpret_cast<HMENU>(kCalendarDetailsToggle), instance_, nullptr);
  calendar_details_ = CreateWindowExW(0, L"EDIT",
                                      L"選択日: （カレンダーから選択）",
                                      WS_CHILD | WS_TABSTOP | ES_MULTILINE | ES_READONLY |
                                          ES_NOHIDESEL | WS_VSCROLL | ES_AUTOVSCROLL,
                                      0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  if (calendar_details_)
    SetWindowSubclass(calendar_details_, CalendarDetailsSubclass, 1,
                      reinterpret_cast<DWORD_PTR>(this));
  panel_headers_[PanelIndex(PanelId::Explorer)] = CreateWindowExW(
      0, L"BUTTON", L"Explorer", WS_CHILD | BS_OWNERDRAW | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderExplorer), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Calendar)] = CreateWindowExW(
      0, L"BUTTON", L"Calendar", WS_CHILD | BS_OWNERDRAW | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderCalendar), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Outline)] = CreateWindowExW(
      0, L"BUTTON", L"Outline", WS_CHILD | BS_OWNERDRAW | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderOutline), instance_, nullptr);
  panel_headers_[PanelIndex(PanelId::Git)] = CreateWindowExW(
      0, L"BUTTON", L"Git", WS_CHILD | BS_OWNERDRAW | WS_TABSTOP,
      0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kPanelHeaderGit), instance_, nullptr);
  for (const HWND header : panel_headers_)
    SetWindowSubclass(header, PanelHeaderSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
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
  chrome_tooltip_ = CreateWindowExW(WS_EX_TOPMOST, TOOLTIPS_CLASSW, nullptr,
      WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX, 0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  for (HWND control : {command_search_, tab_new_, tab_close_, activity_buttons_[0],
                       activity_buttons_[1], activity_buttons_[2], activity_buttons_[3],
                       activity_buttons_[4], git_refresh_, git_stage_, git_unstage_, git_diff_,
                       git_commit_, git_trust_, calendar_daily_, calendar_details_toggle_}) {
    SetWindowSubclass(control, ChromeBarSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
    if (chrome_tooltip_) {
      TTTOOLINFOW tool{sizeof(tool)};
      tool.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
      tool.hwnd = window_;
      tool.uId = reinterpret_cast<UINT_PTR>(control);
      tool.lpszText = LPSTR_TEXTCALLBACKW;
      SendMessageW(chrome_tooltip_, TTM_ADDTOOLW, 0, reinterpret_cast<LPARAM>(&tool));
    }
  }
  HFONT font = static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_, replace_edit_,
                       find_workspace_, replace_workspace_, find_case_, find_regex_, find_word_,
                       find_include_glob_, find_exclude_glob_, replace_one_, replace_document_,
                       find_results_, git_panel_, calendar_details_, brand_, command_search_, tab_new_,
                       tab_close_, activity_buttons_[0], activity_buttons_[1], activity_buttons_[2],
                       activity_buttons_[3], activity_buttons_[4], git_files_, git_diff_view_,
                       git_commit_edit_, git_refresh_, git_stage_, git_unstage_, git_diff_, git_commit_,
                       git_trust_, panel_headers_[0], panel_headers_[1],
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
  }
  if (status_) InvalidateRect(status_, nullptr, TRUE);
  InvalidateRect(window_, nullptr, TRUE);
}

void Application::SetStatusText(std::wstring_view text) {
  status_text_.assign(text);
  if (!status_) return;
  UpdateStatus();
  if (!status_text_.empty()) status_segments_[1] += L"   " + status_text_;
  SetStatusSegments(status_segments_);
}

void Application::SetStatusSegments(std::array<std::wstring, 4> segments) {
  status_segments_ = std::move(segments);
  if (!status_) return;
  RECT client{};
  GetClientRect(status_, &client);
  const int width = std::max(0, static_cast<int>(client.right));
  const int first = std::min(ScaleDip(window_, 300), width * 30 / 100);
  const int second = first + std::min(ScaleDip(window_, 225), width * 25 / 100);
  const int third = second + std::min(ScaleDip(window_, 175), width * 20 / 100);
  const int parts[] = {first, second, third, -1};
  SendMessageW(status_, SB_SETPARTS, 4, reinterpret_cast<LPARAM>(parts));
  for (int index = 0; index < static_cast<int>(status_segments_.size()); ++index) {
    SendMessageW(status_, SB_SETTEXTW, static_cast<WPARAM>(index) | SBT_OWNERDRAW | SBT_NOBORDERS,
                 reinterpret_cast<LPARAM>(status_segments_[index].c_str()));
  }
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
  HFONT font = ui_font_ ? ui_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
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
      (selected ? theme_selected_text_ : theme_foreground_);
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
  HFONT font = ui_font_ ? ui_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
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
  HFONT font = ui_font_ ? ui_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  HGDIOBJ previous = SelectObject(draw.hDC, font);
  SetBkMode(draw.hDC, TRANSPARENT);
  const auto* segment = reinterpret_cast<const wchar_t*>(draw.itemData);
  const std::wstring_view text = segment ? std::wstring_view(segment) : std::wstring_view{};
  const bool clean = draw.itemID == 1 && text.find(L"同期") != std::wstring_view::npos;
  SetTextColor(draw.hDC, clean ? (dark_theme_ ? RGB(104, 202, 153) : RGB(39, 132, 88))
                              : theme_foreground_);
  item.left += ScaleDip(window_, 10);
  item.right -= ScaleDip(window_, 10);
  DrawTextW(draw.hDC, text.data(), static_cast<int>(text.size()), &item,
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
  const std::array selected{active_activity_ == 0, active_activity_ == 1,
                            active_activity_ == 2, active_activity_ == 3};
  for (std::size_t index = 0; index < selected.size(); ++index) {
    const HWND button = activity_buttons_[index];
    const LRESULT checked = selected[index] ? BST_CHECKED : BST_UNCHECKED;
    if (SendMessageW(button, BM_GETCHECK, 0, 0) != checked) {
      SendMessageW(button, BM_SETCHECK, checked, 0);
      NotifyWinEvent(EVENT_OBJECT_STATECHANGE, button, OBJID_CLIENT, CHILDID_SELF);
    }
    InvalidateRect(button, nullptr, FALSE);
  }
  for (const auto id : {PanelId::Explorer, PanelId::Calendar, PanelId::Outline, PanelId::Git}) {
    const auto* panel = panel_layout_.Find(id);
    const HWND header = panel_headers_[PanelIndex(id)];
    if (!panel || !header) continue;
    const std::wstring text = PanelName(id);
    SetWindowTextW(header, text.c_str());
    if (ui_font_) SendMessageW(header, WM_SETFONT, reinterpret_cast<WPARAM>(ui_font_), TRUE);
    InvalidateRect(header, nullptr, FALSE);
  }
}

void Application::DrawChromeButton(const DRAWITEMSTRUCT& draw) const {
  if (!draw.hDC) return;
  HDC dc = draw.hDC;
  struct FontSelection {
    HDC dc;
    HGDIOBJ previous;
    ~FontSelection() { if (previous) SelectObject(dc, previous); }
  } font_selection{dc, SelectObject(dc, ui_font_ ? ui_font_ : GetStockObject(DEFAULT_GUI_FONT))};
  RECT rect = draw.rcItem;
  const int id = static_cast<int>(draw.CtlID);
  const bool focused = (draw.itemState & ODS_FOCUS) != 0;
  const bool hot = (draw.itemState & (ODS_HOTLIGHT | ODS_SELECTED)) != 0 ||
      GetPropW(draw.hwndItem, L"MDLite.Hover") != nullptr;
  const bool disabled = (draw.itemState & ODS_DISABLED) != 0;
  const COLORREF foreground = disabled
      ? (high_contrast_ ? GetSysColor(COLOR_GRAYTEXT) : theme_muted_) : theme_foreground_;
  const auto fill = [&](RECT bounds, COLORREF color) {
    if (bounds.right <= bounds.left || bounds.bottom <= bounds.top) return;
    SetDCBrushColor(dc, color);
    FillRect(dc, &bounds, static_cast<HBRUSH>(GetStockObject(DC_BRUSH)));
  };
  const auto stroke = [&](RECT bounds, COLORREF color, int width = 1) {
    HPEN pen = CreatePen(PS_SOLID, width, color);
    if (!pen) return;
    const HGDIOBJ old_pen = SelectObject(dc, pen);
    const HGDIOBJ old_brush = SelectObject(dc, GetStockObject(HOLLOW_BRUSH));
    Rectangle(dc, bounds.left, bounds.top, bounds.right, bounds.bottom);
    SelectObject(dc, old_brush);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
  };
  const auto line = [&](int x1, int y1, int x2, int y2, COLORREF color, int width = 1) {
    HPEN pen = CreatePen(PS_SOLID, width, color);
    if (!pen) return;
    const HGDIOBJ old_pen = SelectObject(dc, pen);
    MoveToEx(dc, x1, y1, nullptr);
    LineTo(dc, x2, y2);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
  };
  const auto centered = [&](std::wstring_view text, RECT bounds, COLORREF color) {
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, color);
    DrawTextW(dc, text.data(), static_cast<int>(text.size()), &bounds,
              DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
  };

  if (draw.hwndItem == chrome_bar_) {
    fill(rect, theme_surface_);
    line(rect.left, rect.bottom - 1, rect.right, rect.bottom - 1, theme_border_);
    return;
  }
  if (draw.hwndItem == activity_rail_) {
    fill(rect, theme_surface_);
    line(rect.right - 1, rect.top, rect.right - 1, rect.bottom, theme_border_);
    return;
  }
  if (draw.hwndItem == brand_) {
    fill(rect, theme_surface_);
    const int side = std::min(static_cast<int>(rect.bottom - rect.top) - ScaleDip(window_, 4),
                              ScaleDip(window_, 24));
    RECT logo{rect.left + ScaleDip(window_, 10), rect.top + (rect.bottom - rect.top - side) / 2,
              rect.left + ScaleDip(window_, 10) + side,
              rect.top + (rect.bottom - rect.top - side) / 2 + side};
    fill(logo, theme_accent_);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, RGB(255, 255, 255));
    centered(L"M", logo, RGB(255, 255, 255));
    RECT name{logo.right + ScaleDip(window_, 8), rect.top,
              rect.right, rect.bottom};
    HFONT font = ui_font_ ? ui_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
    const HGDIOBJ old_font = SelectObject(dc, font);
    SetTextColor(dc, foreground);
    DrawTextW(dc, L"MDLite", 6, &name, DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    SelectObject(dc, old_font);
    return;
  }
  if (draw.hwndItem == command_search_) {
    RECT field = rect;
    InflateRect(&field, -1, -1);
    fill(field, hot ? theme_surface_alt_ : theme_input_);
    stroke(field, focused ? theme_accent_ : theme_border_, focused ? 2 : 1);
    const int center_y = rect.top + (rect.bottom - rect.top) / 2;
    const int side = ScaleDip(window_, 16);
    DrawUiIcon(dc, {rect.left + ScaleDip(window_, 14), center_y - side / 2,
                   rect.left + ScaleDip(window_, 14) + side, center_y + side / 2},
               UiIcon::Search, theme_muted_);
    RECT label{rect.left + ScaleDip(window_, 42), rect.top,
               rect.right - ScaleDip(window_, 94), rect.bottom};
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, theme_muted_);
    DrawTextW(dc, L"コマンドを検索...", -1, &label,
              DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    const auto binding = settings_.keybindings.find(L"view.commandPalette");
    const std::wstring hint_text = binding == settings_.keybindings.end() || binding->second.empty()
        ? L"Ctrl+K" : binding->second;
    RECT hint{rect.right - ScaleDip(window_, 112), rect.top,
              rect.right - ScaleDip(window_, 10), rect.bottom};
    DrawTextW(dc, hint_text.c_str(), -1, &hint,
              DT_RIGHT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    return;
  }
  if (draw.hwndItem == tab_new_ || draw.hwndItem == tab_close_) {
    fill(rect, hot ? theme_surface_alt_ : theme_surface_);
    RECT icon = rect;
    const int side = ScaleDip(window_, 16);
    icon.left += (rect.right - rect.left - side) / 2;
    icon.top += (rect.bottom - rect.top - side) / 2;
    icon.right = icon.left + side; icon.bottom = icon.top + side;
    DrawUiIcon(dc, icon, draw.hwndItem == tab_new_ ? UiIcon::Add : UiIcon::Close, foreground);
    if (focused) stroke(rect, theme_accent_);
    return;
  }
  if (draw.hwndItem == git_refresh_ || draw.hwndItem == git_stage_ ||
      draw.hwndItem == git_unstage_ || draw.hwndItem == git_diff_ ||
      draw.hwndItem == git_commit_ || draw.hwndItem == git_trust_ ||
      draw.hwndItem == calendar_daily_ || draw.hwndItem == calendar_details_toggle_) {
    fill(rect, hot ? theme_surface_alt_ : theme_surface_);
    stroke(rect, focused ? theme_accent_ : theme_border_, focused ? 2 : 1);
    wchar_t label[64]{};
    GetWindowTextW(draw.hwndItem, label, static_cast<int>(std::size(label)));
    RECT text_rect{rect.left + ScaleDip(window_, 4), rect.top,
                   rect.right - ScaleDip(window_, 4), rect.bottom};
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, disabled ? theme_muted_ : foreground);
    DrawTextW(dc, label, -1, &text_rect,
              DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
    return;
  }
  if (std::ranges::find(activity_buttons_, draw.hwndItem) != activity_buttons_.end()) {
    fill(rect, hot ? theme_surface_alt_ : theme_surface_);
    const bool selected =
        (id == kActivityExplorer && active_activity_ == 0) ||
        (id == kActivitySearch && active_activity_ == 1) ||
        (id == kActivityGit && active_activity_ == 2) ||
        (id == kActivityCalendar && active_activity_ == 3);
    if (selected) fill(rect, theme_surface_alt_);
    if (selected) fill({rect.left, rect.top + ScaleDip(window_, 8),
                        rect.left + ScaleDip(window_, 3), rect.bottom - ScaleDip(window_, 8)},
                       theme_accent_);
    if ((draw.itemState & ODS_SELECTED) != 0) {
      RECT pressed = rect;
      InflateRect(&pressed, -ScaleDip(window_, 2), -ScaleDip(window_, 2));
      stroke(pressed, foreground, ScaleDip(window_, 2));
    }
    const int cx = rect.left + (rect.right - rect.left) / 2;
    const int cy = rect.top + (rect.bottom - rect.top) / 2;
    const int size = ScaleDip(window_, 20);
    const UiIcon icon = id == kActivityExplorer ? UiIcon::Explorer :
        id == kActivitySearch ? UiIcon::Search : id == kActivityGit ? UiIcon::GitBranch :
        id == kActivityCalendar ? UiIcon::Calendar : UiIcon::Settings;
    DrawUiIcon(dc, {cx - size / 2, cy - size / 2, cx + size / 2, cy + size / 2},
               icon, selected ? theme_accent_ : foreground);
    if (focused) stroke({rect.left + 3, rect.top + 3, rect.right - 3, rect.bottom - 3}, theme_accent_);
    return;
  }

  const auto panel = std::ranges::find_if(panel_headers_, [&](HWND header) {
    return header == draw.hwndItem;
  });
  if (panel != panel_headers_.end()) {
    const auto panel_index = static_cast<std::size_t>(panel - panel_headers_.begin());
    const auto id_panel = static_cast<PanelId>(panel_index);
    fill(rect, hot ? theme_surface_alt_ : theme_surface_);
    line(rect.left, rect.bottom - 1, rect.right, rect.bottom - 1, theme_border_);
    if (focused_panel_ == id_panel)
      fill({rect.left, rect.bottom - ScaleDip(window_, 2), rect.right, rect.bottom}, theme_accent_);
    wchar_t name[64]{};
    GetWindowTextW(draw.hwndItem, name, static_cast<int>(std::size(name)));
    RECT text_rect{rect.left + ScaleDip(window_, 14), rect.top,
                   rect.right - ScaleDip(window_, 66), rect.bottom};
    SetTextColor(dc, foreground);
    SetBkMode(dc, TRANSPARENT);
    DrawTextW(dc, name, -1, &text_rect,
              DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS);
    const int center_y = rect.top + (rect.bottom - rect.top) / 2;
    const int side = ScaleDip(window_, 16);
    DrawUiIcon(dc, {rect.right - ScaleDip(window_, 56), center_y - side / 2,
                   rect.right - ScaleDip(window_, 56) + side, center_y + side / 2},
               UiIcon::More, theme_muted_);
    DrawUiIcon(dc, {rect.right - ScaleDip(window_, 27), center_y - side / 2,
                   rect.right - ScaleDip(window_, 27) + side, center_y + side / 2},
               UiIcon::ChevronDown, theme_muted_);
    if (focused) stroke(rect, theme_accent_);
  }
}

void Application::DrawTabItem(const DRAWITEMSTRUCT& draw) const {
  if (!draw.hDC) return;
  const bool selected = (draw.itemState & ODS_SELECTED) != 0;
  const bool focused = (draw.itemState & ODS_FOCUS) != 0;
  const bool hot = (draw.itemState & (ODS_HOTLIGHT | ODS_SELECTED)) != 0;
  const RECT item = draw.rcItem;
  HBRUSH background = CreateSolidBrush(hot ? theme_surface_alt_ : theme_surface_);
  if (background) {
    FillRect(draw.hDC, &item, background);
    DeleteObject(background);
  }
  HPEN border = CreatePen(PS_SOLID, 1, theme_border_);
  if (border) {
    const HGDIOBJ old_pen = SelectObject(draw.hDC, border);
    MoveToEx(draw.hDC, item.left, item.bottom - 1, nullptr);
    LineTo(draw.hDC, item.right, item.bottom - 1);
    if (selected) {
      HPEN accent = CreatePen(PS_SOLID, ScaleDip(window_, 2), theme_accent_);
      if (accent) {
        SelectObject(draw.hDC, accent);
        MoveToEx(draw.hDC, item.left, item.top + 1, nullptr);
        LineTo(draw.hDC, item.right, item.top + 1);
        SelectObject(draw.hDC, border);
        DeleteObject(accent);
      }
    }
    SelectObject(draw.hDC, old_pen);
    DeleteObject(border);
  }

  wchar_t text[260]{};
  TCITEMW tab{TCIF_TEXT};
  tab.pszText = text;
  tab.cchTextMax = static_cast<int>(std::size(text));
  if (!TabCtrl_GetItem(tabs_, static_cast<int>(draw.itemID), &tab)) return;
  const COLORREF foreground = (draw.itemState & ODS_DISABLED) != 0
      ? theme_muted_ : theme_foreground_;
  RECT icon{item.left + ScaleDip(window_, 12),
            item.top + (item.bottom - item.top - ScaleDip(window_, 18)) / 2,
            item.left + ScaleDip(window_, 30),
            item.top + (item.bottom - item.top + ScaleDip(window_, 18)) / 2};
  DrawUiIcon(draw.hDC, icon, UiIcon::File, theme_accent_);
  SetBkMode(draw.hDC, TRANSPARENT);

  RECT text_rect{item.left + ScaleDip(window_, 40), item.top,
                 item.right - ScaleDip(window_, 12), item.bottom};
  SetTextColor(draw.hDC, foreground);
  HFONT font = ui_font_ ? ui_font_ : static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  const HGDIOBJ old_font = SelectObject(draw.hDC, font);
  DrawTextW(draw.hDC, text, -1, &text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX);
  if (old_font) SelectObject(draw.hDC, old_font);
  if (focused) {
    RECT focus{item.left + ScaleDip(window_, 2), item.top + ScaleDip(window_, 2),
               item.right - ScaleDip(window_, 2), item.bottom - ScaleDip(window_, 2)};
    DrawFocusRect(draw.hDC, &focus);
  }
}

void Application::ActivatePanel(PanelId id) {
  auto* panel = panel_layout_.Find(id);
  if (!panel) return;
  bool changed = panel->hidden || panel->collapsed;
  panel->hidden = false;
  panel->collapsed = false;
  if (id == PanelId::Explorer && workspace_pane_collapsed_) {
    workspace_pane_collapsed_ = false;
    changed = true;
  }
  if (id == PanelId::Outline && outline_pane_collapsed_) {
    outline_pane_collapsed_ = false;
    changed = true;
  }
  focused_panel_ = id;
  active_activity_ = id == PanelId::Explorer ? 0 : id == PanelId::Git ? 2 :
      id == PanelId::Calendar ? 3 : -1;
  if (changed) SavePanelLayout();
  LayoutControls();
  UpdatePanelHeaders();
  SetFocus(panel_headers_[PanelIndex(id)]);
}

void Application::GoToCalendarToday() {
  if (!calendar_) return;
  SYSTEMTIME local{};
  GetLocalTime(&local);
  const CalendarDate today{static_cast<int>(local.wYear), static_cast<int>(local.wMonth),
                           static_cast<int>(local.wDay)};
  CalendarView_SetToday(calendar_, today);
  CalendarView_SetDisplayedMonth(calendar_, today);
  CalendarView_SetSelection(calendar_, today);
  UpdateCalendarDetails(today);
}
int Application::MeasurePanelTextHeight(HWND item, int width) const {
  if (!item) return ScaleDip(window_, 40);
  const int length = GetWindowTextLengthW(item);
  std::wstring text(static_cast<std::size_t>(std::max(0, length)) + 1, L'\0');
  GetWindowTextW(item, text.data(), length + 1);
  HDC dc = GetDC(window_);
  if (!dc) return ScaleDip(window_, 60);
  HGDIOBJ previous = SelectObject(dc, ui_font_ ? ui_font_ : GetStockObject(DEFAULT_GUI_FONT));
  RECT measured{0, 0, std::max(1, width), 0};
  DrawTextW(dc, text.c_str(), -1, &measured, DT_CALCRECT | DT_WORDBREAK | DT_NOPREFIX);
  SelectObject(dc, previous);
  ReleaseDC(window_, dc);
  return std::max(ScaleDip(window_, 20), static_cast<int>(measured.bottom));
}

int Application::MinimumCalendarPanelHeight(int width) const {
  const int padding = std::min(ScaleDip(window_, 8), width / 4);
  const int inner_width = std::max(0, width - padding * 2);
  const int button_height = ScaleDip(window_, 28);
  const int actions_height = inner_width >= ScaleDip(window_, 240) ? button_height : button_height * 2;
  return ScaleDip(window_, kTabHeight) + ScaleDip(window_, 228) + ScaleDip(window_, 8) * 3 +
      (calendar_details_expanded_ ? ScaleDip(window_, 44) : 0) +
      MeasurePanelTextHeight(calendar_summary_, inner_width) + actions_height;
}

void Application::LayoutControls() {
  RECT client{};
  GetClientRect(window_, &client);
  SendMessageW(status_, WM_SIZE, 0, 0);
  RECT status_rect{};
  GetWindowRect(status_, &status_rect);
  const int status_height = status_rect.bottom - status_rect.top;
  SetStatusSegments(status_segments_);
  const int topbar_height = ScaleDip(window_, kTopbarHeight);
  const int content_height = std::max(0L, client.bottom - status_height - topbar_height);
  const int rail_width = ScaleDip(window_, 48);
  MoveWindow(chrome_bar_, 0, 0, client.right, topbar_height, TRUE);
  MoveWindow(activity_rail_, 0, topbar_height, rail_width, content_height, TRUE);
  MoveWindow(brand_, ScaleDip(window_, 10), 0, ScaleDip(window_, 130), topbar_height, TRUE);
  const int search_right = std::max(0, static_cast<int>(client.right) - ScaleDip(window_, 24));
  const int search_left = std::min(ScaleDip(window_, 160), search_right);
  const int search_available = std::max(0, search_right - search_left);
  const int search_width = std::min(ScaleDip(window_, 584), search_available);
  const int search_x = search_left + (search_available - search_width) / 2;
  const int search_height = std::min(topbar_height, ScaleDip(window_, kCommandSearchHeight));
  const int search_y = (topbar_height - search_height) / 2;
  MoveWindow(command_search_, search_x, search_y, search_width, search_height, TRUE);
  ShowWindow(brand_, client.right >= ScaleDip(window_, 300) ? SW_SHOW : SW_HIDE);
  ShowWindow(command_search_, search_width >= ScaleDip(window_, 140) ? SW_SHOW : SW_HIDE);
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
  const bool want_tree = show_left_top || show_left_bottom;
  const bool want_outline = show_right_top || show_right_bottom;
  const int splitter_width = ScaleDip(window_, 4);
  const int pane_budget = std::max(0L, client.right - rail_width - minimum_editor_width -
      (want_tree ? splitter_width : 0) - (want_outline ? splitter_width : 0));
  int tree_width = 0;
  int outline_width = 0;
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
  for (HWND control : {calendar_details_, calendar_summary_, calendar_daily_, calendar_details_toggle_,
                       git_files_, git_diff_view_, git_commit_edit_, git_refresh_, git_stage_,
                       git_unstage_, git_diff_, git_commit_, git_trust_})
    ShowWindow(control, SW_HIDE);
  const int header_height = ScaleDip(window_, kTabHeight);
  const auto side_height = [&](const PanelState* top, const PanelState* bottom, bool top_visible,
                               bool bottom_visible, int width) {
    if (!top_visible) return 0;
    const int separator = bottom_visible ? splitter_width : 0;
    const int available_height = std::max(0, content_height - separator);
    if (!bottom_visible) return available_height;
    const int minimum_top = top->id == PanelId::Calendar
        ? MinimumCalendarPanelHeight(width) : header_height + 1;
    const int minimum_bottom = bottom->id == PanelId::Calendar
        ? MinimumCalendarPanelHeight(width) : header_height + 1;
    if (available_height < minimum_top + minimum_bottom) {
      // A restored/programmatically sized window can precede native minimum-size enforcement.
      // Keep the calendar's budget whenever the side can fit it, without changing panel state.
      if (top->id == PanelId::Calendar) return std::min(available_height, minimum_top);
      if (bottom->id == PanelId::Calendar) return std::max(0, available_height - minimum_bottom);
      return available_height / 2;
    }
    const int top_weight = std::max(1, static_cast<int>(top->height));
    const int bottom_weight = std::max(1, static_cast<int>(bottom->height));
    return std::clamp(MulDiv(available_height, top_weight, top_weight + bottom_weight),
                      minimum_top, available_height - minimum_bottom);
  };
  const int left_top_height = side_height(left_top, left_bottom, show_left_top, show_left_bottom, tree_width);
  const int right_top_height = side_height(right_top, right_bottom, show_right_top, show_right_bottom, outline_width);
  current_rail_width_ = rail_width;
  current_tree_width_ = tree_width;
  current_outline_width_ = outline_width;
  current_splitter_width_ = splitter_width;
  current_topbar_height_ = topbar_height;
  current_status_height_ = status_height;
  left_width_splitter_x_ = want_tree ? rail_width + tree_width + splitter_width / 2 : -1;
  right_width_splitter_x_ = want_outline
      ? static_cast<int>(client.right) - outline_width - splitter_width / 2 : -1;
  left_height_splitter_visible_ = show_left_top && show_left_bottom && tree_width > 0;
  right_height_splitter_visible_ = show_right_top && show_right_bottom && outline_width > 0;
  left_height_splitter_y_ = topbar_height + left_top_height + splitter_width / 2;
  right_height_splitter_y_ = topbar_height + right_top_height + splitter_width / 2;
  const auto place_panel = [&](const PanelState* panel, int x, int y, int width, int height) {
    if (!panel || !is_visible(panel) || width <= 0 || height <= 0) return;
    const HWND header = header_for(panel->id);
    const HWND control = control_for(panel->id);
    const int panel_y = topbar_height + y;
    MoveWindow(header, x, panel_y, width, std::min(header_height, height), TRUE);
    ShowWindow(header, SW_SHOW);
    const int content_top = panel_y + std::min(header_height, height);
    const int panel_content_height = std::max(0, height - std::min(header_height, height));
    MoveWindow(control, x, content_top, width, panel_content_height, TRUE);
    ShowWindow(control, panel_content_height > 0 ? SW_SHOW : SW_HIDE);
    const int padding = std::min(ScaleDip(window_, 8), width / 4);
    const int inner_x = x + padding;
    const int inner_width = std::max(0, width - padding * 2);
    const int gap = ScaleDip(window_, 8);
    const int button_height = ScaleDip(window_, 28);
    const int bottom = content_top + panel_content_height;
    const auto place = [&](HWND item, int top, int item_height, bool visible = true) {
      const int clipped = std::min(std::max(0, item_height), std::max(0, bottom - top));
      MoveWindow(item, inner_x, top, inner_width, clipped, TRUE);
      ShowWindow(item, visible && clipped > 0 ? SW_SHOW : SW_HIDE);
    };
    const auto text_height = [&](HWND item) {
      return MeasurePanelTextHeight(item, inner_width -
          (item == git_panel_ ? GetSystemMetricsForDpi(SM_CXVSCROLL, GetDpiForWindow(window_)) + gap : 0));
    };
    if (panel->id == PanelId::Calendar) {
      const int summary_height = text_height(calendar_summary_);
      const bool wide = inner_width >= ScaleDip(window_, 240);
      const int actions_height = wide ? button_height : button_height * 2;
      const int footer_height = summary_height + actions_height + gap * 3;
      const int detail_budget = std::max(0, panel_content_height - footer_height - ScaleDip(window_, 228));
      const int detail_height = calendar_details_expanded_
          ? std::min(ScaleDip(window_, 120), detail_budget) : 0;
      const int calendar_height = std::min(ScaleDip(window_, 228),
          std::max(0, panel_content_height - footer_height - detail_height));
      MoveWindow(calendar_, x, content_top, width, calendar_height, TRUE);
      ShowWindow(calendar_, calendar_height > 0 ? SW_SHOW : SW_HIDE);
      int top = content_top + calendar_height + gap;
      place(calendar_summary_, top, summary_height);
      top += summary_height + gap;
      place(calendar_daily_, top, button_height);
      EnableWindow(calendar_daily_, !workspace_.empty());
      if (wide) {
        const int half = (inner_width - gap) / 2;
        const int row_height = std::min(button_height, std::max(0, bottom - top));
        MoveWindow(calendar_daily_, inner_x, top, half, row_height, TRUE);
        MoveWindow(calendar_details_toggle_, inner_x + half + gap, top,
                   inner_width - half - gap, row_height, TRUE);
        ShowWindow(calendar_details_toggle_, row_height > 0 ? SW_SHOW : SW_HIDE);
      } else {
        top += button_height;
        place(calendar_details_toggle_, top, button_height);
      }
      top += button_height + gap;
      place(calendar_details_, top, detail_height, calendar_details_expanded_);
    } else if (panel->id == PanelId::Git) {
      const bool has_workspace = !workspace_.empty();
      const bool trusted = has_workspace && IsWorkspaceTrusted(workspace_);
      const bool repository_ready = trusted &&
          (git_panel_status_.state == GitPanelState::Ready ||
           git_panel_status_.state == GitPanelState::NoRemote);
      int top = content_top + padding;
      const int summary_height = text_height(git_panel_);
      place(git_panel_, top, summary_height);
      top += summary_height + gap;
      if (!trusted) {
        place(git_trust_, top, button_height, has_workspace);
        return;
      }
      place(git_refresh_, top, button_height);
      EnableWindow(git_refresh_, !git_action_active_ && !git_status_worker_.joinable());
      top += button_height + gap;
      if (!repository_ready) return;
      const int actions_height = button_height * 2 + gap;
      const int remaining = std::max(0, bottom - top);
      // Preserve useful content when resized: omit the lower action rows until
      // a complete row fits rather than drawing them over the list or summary.
      const bool show_actions = remaining >= actions_height + ScaleDip(window_, 44);
      const int content_available = std::max(0, remaining - (show_actions ? actions_height + gap : 0));
      const int diff_height = content_available >= ScaleDip(window_, 100)
          ? std::min(ScaleDip(window_, 96), content_available / 3) : 0;
      const int list_height = std::max(0, content_available - diff_height - (diff_height ? gap : 0));
      place(git_files_, top, list_height);
      ListView_SetColumnWidth(git_files_, 0, ScaleDip(window_, 42));
      ListView_SetColumnWidth(git_files_, 1, std::max(0, inner_width - ScaleDip(window_, 46)));
      top += list_height + (diff_height ? gap : 0);
      place(git_diff_view_, top, diff_height, diff_height > 0);
      top += diff_height + gap;
      if (show_actions) {
        const int third = std::max(0, (inner_width - gap * 2) / 3);
        int button_x = inner_x;
        for (HWND button : {git_stage_, git_unstage_, git_diff_}) {
          MoveWindow(button, button_x, top, third, button_height, TRUE);
          ShowWindow(button, SW_SHOW);
          button_x += third + gap;
        }
        top += button_height + gap;
        const int commit_width = std::min(ScaleDip(window_, 76), inner_width / 2);
        MoveWindow(git_commit_edit_, inner_x, top, std::max(0, inner_width - commit_width - gap), button_height, TRUE);
        MoveWindow(git_commit_, inner_x + inner_width - commit_width, top, commit_width, button_height, TRUE);
        ShowWindow(git_commit_edit_, SW_SHOW);
        ShowWindow(git_commit_, SW_SHOW);
      }
    }

  };
  place_panel(left_top, rail_width, 0, tree_width, left_top_height);
  const int left_splitter_offset = show_left_top && show_left_bottom ? splitter_width : 0;
  const int right_splitter_offset = show_right_top && show_right_bottom ? splitter_width : 0;
  place_panel(left_bottom, rail_width, left_top_height + left_splitter_offset, tree_width,
              std::max(0, content_height - left_top_height - left_splitter_offset));
  place_panel(right_top, client.right - outline_width, 0, outline_width, right_top_height);
  place_panel(right_bottom, client.right - outline_width,
              right_top_height + right_splitter_offset, outline_width,
              std::max(0, content_height - right_top_height - right_splitter_offset));
  const int tab_height = ScaleDip(window_, kTabHeight);
  const int center_left = rail_width + tree_width + (want_tree ? splitter_width : 0);
  const int center_width = std::max(0L, client.right - center_left - outline_width -
      (want_outline ? splitter_width : 0));
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
  const int center_top = topbar_height;
  const int tab_action_width = ScaleDip(window_, 68);
  const int tab_strip_width = std::max(0, center_width - tab_action_width);
  MoveWindow(tabs_, center_left, center_top, tab_strip_width, tab_height, TRUE);
  place(tab_close_, center_left + tab_strip_width, center_top,
        ScaleDip(window_, 34), tab_height, true);
  place(tab_new_, center_left + tab_strip_width + ScaleDip(window_, 34), center_top,
        ScaleDip(window_, 34), tab_height, true);
  MoveWindow(find_bar_, center_left, center_top + tab_height, center_width,
             find_visible ? find_height : 0, TRUE);
  place(find_edit_, center_left + padding, center_top + tab_height + ScaleDip(window_, 4), input_width,
        input_height, find_visible);
  const int find_next_x = center_left + padding + input_width + gap;
  place(find_next_, find_next_x, center_top + tab_height + ScaleDip(window_, 3), button_width,
        button_height, find_visible);
  int option_x = find_next_x + button_width + gap;
  place(find_case_, option_x, center_top + tab_height + ScaleDip(window_, 5), ScaleDip(window_, 72),
        ScaleDip(window_, 22), show_advanced_find);
  option_x += ScaleDip(window_, 72) + gap;
  place(find_regex_, option_x, center_top + tab_height + ScaleDip(window_, 5), ScaleDip(window_, 62),
        ScaleDip(window_, 22), show_advanced_find);
  option_x += ScaleDip(window_, 62) + gap;
  place(find_word_, option_x, center_top + tab_height + ScaleDip(window_, 5), ScaleDip(window_, 58),
        ScaleDip(window_, 22), show_advanced_find);
  const int replacement_cluster = show_workspace_actions ?
      ScaleDip(window_, 82 + 92 + 118 + 12) : ScaleDip(window_, 82);
  const int replacement_width = std::max(0, first_row_available - replacement_cluster - gap);
  place(replace_edit_, center_left + padding, center_top + tab_height + ScaleDip(window_, 36), replacement_width,
        input_height, find_visible);
  int replace_x = center_left + padding + replacement_width + gap;
  place(replace_one_, replace_x, center_top + tab_height + ScaleDip(window_, 35), ScaleDip(window_, 82),
        button_height, find_visible);
  replace_x += ScaleDip(window_, 82) + gap;
  place(replace_document_, replace_x, center_top + tab_height + ScaleDip(window_, 35), ScaleDip(window_, 92),
        button_height, show_workspace_actions);
  replace_x += ScaleDip(window_, 92) + gap;
  place(replace_workspace_, replace_x, center_top + tab_height + ScaleDip(window_, 35), ScaleDip(window_, 118),
        button_height, show_workspace_actions);
  const int glob_button_width = show_workspace_actions ? base_button_width : 0;
  const int glob_width = show_globs ? std::max(0, (center_width - padding * 2 - glob_button_width - gap * 2) / 2) : 0;
  place(find_include_glob_, center_left + padding, center_top + tab_height + ScaleDip(window_, 66), glob_width,
        input_height, show_globs);
  place(find_exclude_glob_, center_left + padding + glob_width + gap,
        center_top + tab_height + ScaleDip(window_, 66), glob_width, input_height, show_globs);
  place(find_workspace_, center_left + center_width - padding - glob_button_width,
        center_top + tab_height + ScaleDip(window_, 65), glob_button_width, button_height,
        show_workspace_actions);
  const int results_top = tab_height + (find_visible ? find_height : 0);
  MoveWindow(find_results_, center_left, center_top + results_top, center_width,
             results_visible ? results_height : 0, TRUE);
  const int editor_top = center_top + results_top + (results_visible ? results_height : 0);
  for (auto& view : documents_) {
    if (view->editor && GetParent(view->editor) == window_) {
      MoveWindow(view->editor, center_left, editor_top, center_width,
                 std::max(0, static_cast<int>(client.bottom) - status_height - editor_top), TRUE);
      RECT editor_client{};
      if (GetClientRect(view->editor, &editor_client) &&
          editor_client.right > editor_client.left &&
          editor_client.bottom > editor_client.top) {
        const int horizontal_inset = std::min(
            ScaleDip(window_, kEditorHorizontalInset),
            std::max(0, static_cast<int>(editor_client.right - editor_client.left - 1) / 2));
        const int vertical_inset = std::min(
            ScaleDip(window_, kEditorVerticalInset),
            std::max(0, static_cast<int>(editor_client.bottom - editor_client.top - 1) / 2));
        RECT formatting_rect{
            editor_client.left + horizontal_inset,
            editor_client.top + vertical_inset,
            editor_client.right - horizontal_inset,
            editor_client.bottom - vertical_inset};
        if (formatting_rect.right <= formatting_rect.left)
          formatting_rect.right = formatting_rect.left + 1;
        if (formatting_rect.bottom <= formatting_rect.top)
          formatting_rect.bottom = formatting_rect.top + 1;
        // EM_GETRECT may round the request by a few pixels. Compare requests
        // for this live HWND so unchanged layout does not schedule full styling.
        if (!view->editor_formatting_rect_request ||
            !EqualRect(&*view->editor_formatting_rect_request, &formatting_rect)) {
          SendMessageW(view->editor, EM_SETRECTNP, 0,
                       reinterpret_cast<LPARAM>(&formatting_rect));
          view->editor_formatting_rect_request = formatting_rect;
          InvalidateRect(view->editor, nullptr, TRUE);
          if (!view->parse.tables.empty()) SchedulePresentation(*view);
        }
      }
    }
  }
  const int rail_button_size = ScaleDip(window_, 44);
  const int rail_button_x = (rail_width - rail_button_size) / 2;
  const int rail_first_y = topbar_height + ScaleDip(window_, 14);
  const int rail_step = rail_button_size + ScaleDip(window_, 8);
  for (std::size_t index = 0; index < 4; ++index) {
    MoveWindow(activity_buttons_[index], rail_button_x,
               rail_first_y + static_cast<int>(index) * rail_step,
               rail_button_size, rail_button_size, TRUE);
    ShowWindow(activity_buttons_[index], SW_SHOW);
  }
  MoveWindow(activity_buttons_[4], rail_button_x,
             std::max(topbar_height, static_cast<int>(client.bottom) - status_height -
                                      rail_button_size - ScaleDip(window_, 8)),
             rail_button_size, rail_button_size, TRUE);
  ShowWindow(activity_buttons_[4], SW_SHOW);
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
                       items, NativeDialogTheme{theme_background_, theme_surface_, theme_input_,
                                                theme_foreground_, theme_muted_, theme_accent_,
                                                theme_border_}, selected)) return;
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
  git_panel_status_ = {};
  git_status_workspace_ = workspace_;
  std::optional<SessionDocument> active_session_view_to_restore;
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
          std::optional<POINT> source_anchor_scroll;
          if (item.first_visible_source_offset) {
            const auto viewport_source_anchor = item.horizontal_left_edge_source_offset
                .value_or(*item.first_visible_source_offset);
            source_anchor_scroll = SourceAnchoredScrollPosition(
                (*found)->editor, (*found)->editor_snapshot, viewport_source_anchor);
          }
          RestoreSourceSelection((*found)->editor, (*found)->editor_snapshot,
                                 SourceSelection{item.selection_begin, item.selection_end});
          if (item.first_visible_source_offset) {
            const auto source_anchor = std::min(*item.first_visible_source_offset,
                                                (*found)->document.text().size());
            const auto native_anchor = static_cast<LONG>(
                (*found)->editor_snapshot.SourceToNative(source_anchor));
            const LRESULT target_line =
                SendMessageW((*found)->editor, EM_LINEFROMCHAR, native_anchor, 0);
            const LRESULT current_first_line =
                SendMessageW((*found)->editor, EM_GETFIRSTVISIBLELINE, 0, 0);
            if (target_line >= 0 && current_first_line >= 0)
              SendMessageW((*found)->editor, EM_LINESCROLL, 0,
                           target_line - current_first_line);
          } else {
            // Legacy session files store a line count rather than a source anchor.
            const LRESULT current_first_line =
                SendMessageW((*found)->editor, EM_GETFIRSTVISIBLELINE, 0, 0);
            if (current_first_line >= 0)
              SendMessageW((*found)->editor, EM_LINESCROLL, 0,
                           item.first_visible_line - current_first_line);
          }
          if (source_anchor_scroll)
            SendMessageW((*found)->editor, EM_SETSCROLLPOS, 0,
                         reinterpret_cast<LPARAM>(&*source_anchor_scroll));
          CaptureEditorViewState(**found);
          if (item.compact) {
            ToggleCompactWindow();
            if ((*found)->compact_window)
              PlaceOnVisibleMonitor((*found)->compact_window, item.x, item.y, item.width, item.height);
          }
        }
        if (!documents_.empty()) {
          ActivateDocument(std::min(session.active_index, documents_.size() - 1));
          if (active_document_ < documents_.size()) {
            const auto active_path = documents_[active_document_]->document.path();
            const auto saved_active_view = std::ranges::find_if(
                session.documents, [&](const SessionDocument& item) {
                  return item.path == active_path;
                });
            if (saved_active_view != session.documents.end())
              active_session_view_to_restore = *saved_active_view;
          }
        }
      }
    }
  }
  LoadAndApplySettings();
  LoadPanelLayout();
  SetWindowTextW(window_, (L"MDLite — " + workspace_.filename().wstring()).c_str());
  PopulateWorkspaceTree();
  UpdateStatus();
  LayoutControls();
  // Settings and panel restoration above can resize RichEdit after the session
  // was initially rehydrated. Reapply the active document's source viewport
  // once against the final formatting rectangle and table projection.
  if (active_session_view_to_restore && active_document_ < documents_.size()) {
    auto& view = *documents_[active_document_];
    if (view.document.path() == active_session_view_to_restore->path &&
        view.editor && IsWindow(view.editor)) {
      view.suspended_selection = {active_session_view_to_restore->selection_begin,
                                  active_session_view_to_restore->selection_end};
      if (active_session_view_to_restore->first_visible_source_offset)
        view.suspended_first_visible_source = std::min(
            *active_session_view_to_restore->first_visible_source_offset,
            view.document.text().size());
      view.suspended_horizontal_left_edge_source =
          active_session_view_to_restore->horizontal_left_edge_source_offset;
      view.suspended_view_state_valid = true;
      RestoreEditorViewState(view);
      CaptureEditorViewState(view);
      view.suspended_view_state_valid = false;
    }
  }
  RunGitStatus();
}

void Application::PopulateWorkspaceTree() {
  TreeView_DeleteAllItems(workspace_tree_);
  tree_paths_.clear();
  AddTreeDirectory(TVI_ROOT, workspace_, 0);
  RefreshCalendarAfterWorkspaceMutation();
}

void Application::RefreshCalendarAfterWorkspaceMutation() {
  UpdateCalendarViewMarkers();
  if (const auto selected = CalendarView_GetSelection(calendar_))
    UpdateCalendarDetails(*selected);
}

void Application::AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth) {
  if (depth > 32) return;
  std::vector<std::filesystem::directory_entry> entries;
  std::error_code error;
  for (std::filesystem::directory_iterator iterator(directory, std::filesystem::directory_options::skip_permission_denied, error), end;
       iterator != end && !error; iterator.increment(error)) entries.push_back(*iterator);
  std::ranges::sort(entries, [](const auto& a, const auto& b) {
    const bool a_directory = a.is_directory();
    const bool b_directory = b.is_directory();
    if (a_directory != b_directory) return a_directory;
    const auto a_name = a.path().filename().wstring();
    const auto b_name = b.path().filename().wstring();
    const int insensitive = CompareStringOrdinal(a_name.c_str(), -1, b_name.c_str(), -1, TRUE);
    if (insensitive != CSTR_EQUAL && insensitive != 0) return insensitive == CSTR_LESS_THAN;
    return a_name < b_name;
  });
  for (const auto& entry : entries) {
    const auto name = entry.path().filename().wstring();
    if (name == L".git" || name == L"build" || name == L"out" ||
        (name == L".cache" && entry.path().parent_path().filename() == L".mdlite") ||
        (name == L".state" && entry.path().parent_path().filename() == L".mdlite")) continue;
    if (!entry.is_directory() && !IsTextFile(entry.path())) continue;
    tree_paths_.push_back(std::make_unique<std::filesystem::path>(entry.path()));
    const DWORD attributes = entry.is_directory() ? FILE_ATTRIBUTE_DIRECTORY : FILE_ATTRIBUTE_NORMAL;
    SHFILEINFOW image_info{};
    const bool has_image = SHGetFileInfoW(entry.path().c_str(), attributes, &image_info,
        sizeof(image_info), SHGFI_SYSICONINDEX | SHGFI_SMALLICON) != 0;
    int selected_image = has_image ? image_info.iIcon : 0;
    if (has_image && entry.is_directory()) {
      SHFILEINFOW open_image{};
      if (SHGetFileInfoW(entry.path().c_str(), attributes, &open_image, sizeof(open_image),
                         SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_OPENICON) != 0)
        selected_image = open_image.iIcon;
    }
    TVINSERTSTRUCTW insert{};
    insert.hParent = parent;
    // The directory entries above are already ordered. TVI_SORT re-sorts the
    // growing sibling list for every insertion, which stalls large workspaces.
    insert.hInsertAfter = TVI_LAST;
    insert.item.mask = TVIF_TEXT | TVIF_PARAM;
    if (has_image) insert.item.mask |= TVIF_IMAGE | TVIF_SELECTEDIMAGE;
    insert.item.pszText = const_cast<wchar_t*>(name.c_str());
    insert.item.lParam = reinterpret_cast<LPARAM>(tree_paths_.back().get());
    if (has_image) {
      insert.item.iImage = image_info.iIcon;
      insert.item.iSelectedImage = selected_image;
    }
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
  if (!EnsureEditor(*view)) {
    SetStatusText(L"文書エディターを作成できなかったため、文書を開けませんでした。");
    return;
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
    auto& view = *documents_[index];
    if (view.document.dirty()) {
      MessageBoxW(window_, L"同じ元文書の未保存タブがすでに開いているため、復旧内容を重ねていません。",
                  L"復旧の競合", MB_ICONWARNING);
      return;
    }
    const auto selection = view.editor && IsWindow(view.editor)
        ? CaptureSourceSelection(view.editor, view.editor_snapshot)
        : view.suspended_selection;
    view.document.MarkEdited(std::move(snapshot.text));
    view.flat_source_fallback_retry_used = false;
    auto recovered_projection = NativeSnapshotFor(view.document);
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    view.rendered_images.clear();
    view.rendered_image_source.clear();
    view.animated_image_frames.clear();
    view.animation_due = 0;
    view.image_asset_check_due = 0;
    view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
    if (view.editor && IsWindow(view.editor))
      (void)RebuildEditorProjection(view, std::move(recovered_projection), selection);
    else {
      view.editor_snapshot = std::move(recovered_projection);
      view.flat_source_fallback_pending = false;
      view.flat_source_fallback_retry_used = false;
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
  auto& target = *documents_[index];
  if (active_document_ < documents_.size() && active_document_ != index &&
      documents_[active_document_]->ime_composing) {
    TabCtrl_SetCurSel(tabs_, static_cast<int>(active_document_));
    SetStatusText(L"IME変換中のため、文書切替を保留しました。");
    return;
  }
  const std::size_t previous_active_document = active_document_;
  const bool restore_view_state = target.suspended_view_state_valid;
  if (!EnsureEditor(target)) {
    SetStatusText(L"文書エディターを再作成できなかったため、タブを切り替えませんでした。");
    if (active_document_ < documents_.size())
      TabCtrl_SetCurSel(tabs_, static_cast<int>(active_document_));
    return;
  }
  // Prepare the target in the background before publishing the tab switch or
  // releasing the current view. A table/image presentation failure must leave
  // the old active document and editor available for retry.
  LayoutControls();
  if (IsMarkdownFile(target.document.path())) {
    const bool reuse_current_presentation =
        restore_view_state && previous_active_document == index &&
        target.presentation_revision == target.document.revision() &&
        target.presentation_due == 0 && target.sync_due == 0 &&
        !target.ime_composing && !target.native_edit_in_flight &&
        !target.native_edit_pending && !target.native_readback_failed &&
        !target.flat_source_fallback_pending &&
        !target.force_markdown_presentation_failure_for_test &&
        !target.force_table_cell_formatting_failure_for_test &&
        !target.force_partial_markdown_presentation_failure_for_test;
    const bool force_presentation = restore_view_state && !reuse_current_presentation;
    if (reuse_current_presentation)
      RestoreSourceSelection(target.editor, target.editor_snapshot, target.suspended_selection);
    if (force_presentation)
      target.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    ApplyMarkdownPresentation(target, force_presentation,
                              restore_view_state
                                  ? std::optional<SourceSelection>(target.suspended_selection)
                                  : std::nullopt);
    if (target.presentation_revision != target.document.revision()) {
      SetStatusText(L"文書表示を再構築できなかったため、タブを切り替えませんでした。");
      if (previous_active_document < documents_.size())
        TabCtrl_SetCurSel(tabs_, static_cast<int>(previous_active_document));
      else
        TabCtrl_SetCurSel(tabs_, -1);
      if (previous_active_document < documents_.size() &&
          documents_[previous_active_document]->editor &&
          IsWindow(documents_[previous_active_document]->editor))
        SetFocus(documents_[previous_active_document]->editor);
      return;
    }
    target.presentation_due = 0;
  }
  active_document_ = index;
  TabCtrl_SetCurSel(tabs_, static_cast<int>(index));
  for (std::size_t i = 0; i < documents_.size(); ++i) {
    auto& view = *documents_[i];
    if (view.compact_window != nullptr) continue;
    if (i == index) {
      if (view.editor) ShowWindow(view.editor, SW_SHOW);
    } else if (view.editor && !SuspendEditor(view)) {
      ShowWindow(view.editor, SW_HIDE);
    }
  }
  LayoutControls();
  RebuildOutline(target);
  SetFocus(target.editor);
  if (restore_view_state) {
    RestoreEditorViewState(target);
    target.suspended_view_state_valid = false;
  }
  UpdateStatus();
}

bool Application::EnsureEditor(DocumentView& view) {
  if (view.editor_projection_invalid) return false;
  if (view.editor && IsWindow(view.editor)) return true;
  if (view.compact_window) return false;
  view.editor_formatting_rect_request.reset();
  view.editor = CreateWindowExW(0, MSFTEDIT_CLASS, nullptr,
                                WS_CHILD | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE |
                                    ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL |
                                    ES_WANTRETURN,
                                0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kEditor),
                                instance_, nullptr);
  if (!view.editor) return false;
  // RichEdit's default user-entry limit is 32,767 characters even when a
  // larger plain-text document can be loaded. Keep the full supported source
  // editable; Document loading and editor coordinates are already bounded by
  // LONG-sized RichEdit positions.
  SendMessageW(view.editor, EM_EXLIMITTEXT, 0,
               static_cast<LPARAM>(std::numeric_limits<LONG>::max()));
  SendMessageW(view.editor, EM_SETEVENTMASK, 0,
               ENM_CHANGE | ENM_SELCHANGE | ENM_UPDATE | ENM_SCROLL | ENM_LINK);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  SendMessageW(view.editor, EM_SETBKGNDCOLOR, 0,
               ThemeColor(settings_, L"editor_background", dark ? RGB(28, 31, 36)
                                                                 : RGB(252, 253, 255)));
  if (editor_font_)
    SendMessageW(view.editor, WM_SETFONT, reinterpret_cast<WPARAM>(editor_font_), FALSE);
  if (!SetWindowSubclass(view.editor, EditorSubclass, 1, reinterpret_cast<DWORD_PTR>(this))) {
    DestroyWindow(view.editor);
    view.editor = nullptr;
    return false;
  }
  if (!view.suspended_view_state_valid) {
    view.editor_snapshot = NativeSnapshotFor(view.document);
  } else {
    for (auto& table : view.editor_snapshot.tables) table.native_coordinates_set = false;
  }
  view.native_tables_ready = false;
  view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
  view.active_line = -1;
  view.active_source_line_begin = std::numeric_limits<std::size_t>::max();
  for (auto& image : view.rendered_images) image.inserted = false;
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    if (!SetEditorFlatTextVerified(view, view.editor_snapshot)) {
      DestroyWindow(view.editor);
      view.editor = nullptr;
      return false;
    }
    CHARFORMAT2W format{sizeof(format)};
    format.dwMask = CFM_COLOR | CFM_FACE | CFM_SIZE;
    format.crTextColor = ThemeColor(settings_, L"foreground",
                                    dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
    format.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
    wcsncpy_s(format.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
    SendMessageW(view.editor, EM_SETSEL, 0, -1);
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION,
                 reinterpret_cast<LPARAM>(&format));
    SendMessageW(view.editor, EM_SETSEL, 0, 0);
  }
  const ULONGLONG now = GetTickCount64();
  if (!view.rendered_images.empty()) view.image_asset_check_due = now;
  if (!view.animated_image_frames.empty()) view.animation_due = now;
  return true;
}

void Application::CaptureEditorViewState(DocumentView& view) {
  if (view.editor_projection_invalid || !view.editor || !IsWindow(view.editor)) return;
  view.suspended_selection = CaptureSourceSelection(view.editor, view.editor_snapshot);
  const LRESULT first_visible_line = SendMessageW(view.editor, EM_GETFIRSTVISIBLELINE, 0, 0);
  const LRESULT first_visible_native = SendMessageW(
      view.editor, EM_LINEINDEX, static_cast<WPARAM>(std::max<LRESULT>(0, first_visible_line)), 0);
  if (first_visible_native >= 0) {
    view.suspended_first_visible_source = view.editor_snapshot.NativeToSource(
        static_cast<std::size_t>(first_visible_native));
    view.suspended_first_visible_source = std::min(
        view.suspended_first_visible_source, view.document.text().size());
  } else {
    view.suspended_first_visible_source = 0;
  }
  view.suspended_horizontal_left_edge_source =
      CaptureVisibleLeftEdgeSourceOffset(view.editor, view.editor_snapshot);
  view.suspended_view_state_valid = true;
}

void Application::RestoreEditorViewState(DocumentView& view) {
  if (!view.editor || !IsWindow(view.editor) || !view.suspended_view_state_valid) return;
  const auto horizontal_source_anchor = view.suspended_horizontal_left_edge_source
      .value_or(view.suspended_first_visible_source);
  const auto source_anchor_scroll = SourceAnchoredScrollPosition(
      view.editor, view.editor_snapshot, horizontal_source_anchor);
  RestoreSourceSelection(view.editor, view.editor_snapshot, view.suspended_selection);
  const auto native_anchor = static_cast<LONG>(
      view.editor_snapshot.SourceToNative(view.suspended_first_visible_source));
  const LRESULT target_line = SendMessageW(view.editor, EM_LINEFROMCHAR, native_anchor, 0);
  const LRESULT current_first_line = SendMessageW(view.editor, EM_GETFIRSTVISIBLELINE, 0, 0);
  if (target_line >= 0 && current_first_line >= 0)
    SendMessageW(view.editor, EM_LINESCROLL, 0, target_line - current_first_line);
  if (source_anchor_scroll)
    SendMessageW(view.editor, EM_SETSCROLLPOS, 0,
                 reinterpret_cast<LPARAM>(&*source_anchor_scroll));
}

std::optional<std::size_t> Application::CaptureVisibleLeftEdgeSourceOffset(
    HWND editor, const EditorSnapshot& snapshot) const {
  RECT formatting{};
  SendMessageW(editor, EM_GETRECT, 0, reinterpret_cast<LPARAM>(&formatting));
  if (formatting.right <= formatting.left || formatting.bottom <= formatting.top)
    return std::nullopt;

  const LRESULT first_visible_line = SendMessageW(editor, EM_GETFIRSTVISIBLELINE, 0, 0);
  if (first_visible_line < 0) return std::nullopt;
  const LONG last_probe_y = std::min<LONG>(formatting.bottom, formatting.top + 256);
  LONG best_native = -1;
  LONG best_distance = LONG_MAX;
  for (LONG y = formatting.top; y < last_probe_y; ++y) {
    POINT point{formatting.left, y};
    const LRESULT native = SendMessageW(editor, EM_CHARFROMPOS, 0,
                                        reinterpret_cast<LPARAM>(&point));
    if (native < 0 || SendMessageW(editor, EM_LINEFROMCHAR, native, 0) != first_visible_line)
      continue;
    POINT position{};
    SendMessageW(editor, EM_POSFROMCHAR, reinterpret_cast<WPARAM>(&position), native);
    const LONG distance = static_cast<LONG>(std::min<std::int64_t>(
        std::llabs(static_cast<long long>(position.x) - formatting.left), LONG_MAX));
    if (distance < best_distance) {
      best_native = static_cast<LONG>(native);
      best_distance = distance;
      if (distance == 0) break;
    }
  }
  if (best_native < 0) return std::nullopt;
  return std::min(snapshot.NativeToSource(static_cast<std::size_t>(best_native)),
                  snapshot.source_size);
}

std::optional<POINT> Application::SourceAnchoredScrollPosition(
    HWND editor, const EditorSnapshot& snapshot, std::size_t viewport_source_offset) const {
  RECT formatting{};
  SendMessageW(editor, EM_GETRECT, 0, reinterpret_cast<LPARAM>(&formatting));
  if (formatting.right <= formatting.left || formatting.bottom <= formatting.top)
    return std::nullopt;

  // Resolve virtual document coordinates from a stable origin, rather than
  // reconstructing them from the RichEdit scrollbar thumb (which can be scaled
  // or narrower than the document's character range).
  const POINT origin{};
  SendMessageW(editor, EM_SETSCROLLPOS, 0, reinterpret_cast<LPARAM>(&origin));
  const LONG native = static_cast<LONG>(snapshot.SourceToNative(
      std::min(viewport_source_offset, snapshot.source_size)));
  POINT position{};
  SendMessageW(editor, EM_POSFROMCHAR, reinterpret_cast<WPARAM>(&position), native);
  if (position.x < formatting.left || position.y < formatting.top)
    return std::nullopt;

  return POINT{position.x - formatting.left, position.y - formatting.top};
}

bool Application::SuspendEditor(DocumentView& view) {
  if (view.editor_projection_invalid) {
    if (view.editor && IsWindow(view.editor)) ShowWindow(view.editor, SW_HIDE);
    return true;
  }
  if (!view.editor || !IsWindow(view.editor)) return true;
  if (view.compact_window || GetParent(view.editor) != window_)
    return false;
  if (view.ime_composing || view.native_edit_in_flight || view.pending_table_high_surrogate != 0 ||
      view.pending_virtual_table_cell || view.ime_selection_before)
    return false;
  if (!SyncDocumentFromEditor(view)) return false;
  if (!view.native_edit_in_flight && !view.native_edit_pending && view.sync_due == 0) {
    view.pending_selection_before.reset();
    view.pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
  }
  if (view.ime_composing || view.native_edit_in_flight || view.native_edit_pending ||
      view.sync_due != 0 || view.pending_table_high_surrogate != 0 ||
      view.pending_virtual_table_cell || view.pending_selection_before || view.ime_selection_before)
    return false;
  CaptureEditorViewState(view);
  if (IsMarkdownFile(view.document.path())) {
    // Only source synchronization and source anchors are needed before this
    // native view is destroyed. Reformatting the outgoing table re-enters
    // RichEdit layout/scrollbar work during activation; the recreated target
    // receives its required presentation in ActivateDocument instead.
    view.presentation_due = 0;
  }
  if (GetFocus() == view.editor) SetFocus(window_);
  ShowWindow(view.editor, SW_HIDE);
  DestroyWindow(view.editor);
  view.editor = nullptr;
  view.editor_formatting_rect_request.reset();
  view.native_tables_ready = false;
  view.active_line = -1;
  view.active_source_line_begin = std::numeric_limits<std::size_t>::max();
  view.active_line_update_pending = false;
  view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
  return true;
}

Application::DocumentView* Application::FindDocumentView(HWND editor) {
  if (!editor) return nullptr;
  const auto found = std::ranges::find_if(documents_, [editor](const auto& view) {
    return view->editor == editor;
  });
  return found == documents_.end() ? nullptr : found->get();
}

void Application::SelectDocumentForEditor(HWND editor) {
  if (!editor) return;
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため、タブを閉じませんでした。再試行してください。");
    return false;
  }
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
    if (view.editor) SetParent(view.editor, window_);
    DestroyWindow(view.compact_window);
    view.compact_window = nullptr;
  }
  if (view.editor) DestroyWindow(view.editor);
  view.editor = nullptr;
  view.editor_formatting_rect_request.reset();
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため、保存を中止しました。再試行してください。");
    return SaveResult::ReadbackFailed;
  }
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため、別名保存を中止しました。再試行してください。");
    return SaveResult::ReadbackFailed;
  }
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため、再読込みを中止しました。再試行してください。");
    return;
  }
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
  const auto selection = view.editor && IsWindow(view.editor)
      ? CaptureSourceSelection(view.editor, view.editor_snapshot)
      : view.suspended_selection;
  view.document = std::move(replacement);
  view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
  view.rendered_images.clear();
  view.rendered_image_source.clear();
  view.animated_image_frames.clear();
  view.animation_due = 0;
  view.image_asset_check_due = 0;
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  view.flat_source_fallback_retry_used = false;
  auto reloaded_projection = NativeSnapshotFor(view.document);
  if (view.editor && IsWindow(view.editor))
    (void)RebuildEditorProjection(view, std::move(reloaded_projection), selection);
  else {
    view.editor_snapshot = std::move(reloaded_projection);
    view.flat_source_fallback_pending = false;
    view.flat_source_fallback_retry_used = false;
  }
  if (!view.editor_projection_invalid) ApplyMarkdownPresentation(view, true, selection);
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったためディスク比較を中止しました。再試行してください。");
    return;
  }
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
    if (save_result == SaveResult::ReadbackFailed) return SaveAllResult::Cancelled;
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
    if (view->editor_projection_invalid) return;
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため処理を保留しました。再試行してください。");
    return;
  }
  SchedulePresentation(view);
}

void Application::SchedulePresentation(DocumentView& view) {
  if (view.ime_composing || !IsMarkdownFile(view.document.path())) return;
  view.presentation_due = GetTickCount64() + kPresentationDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
}

void Application::ScheduleFlatSourceFallbackRetry(DocumentView& view) {
  if (!view.flat_source_fallback_pending || view.flat_source_fallback_retry_used) return;
  view.flat_source_fallback_retry_used = true;
  SchedulePresentation(view);
}

void Application::QueueActiveLinePresentation(DocumentView& view) {
  if (!window_ || !view.editor || suppress_editor_change_ || view.ime_composing ||
      view.native_edit_in_flight || view.native_edit_pending || view.sync_due != 0 ||
      view.presentation_due != 0 || !IsMarkdownFile(view.document.path()) ||
      view.presentation_revision != view.document.revision()) return;
  view.active_line_update_revision = view.document.revision();
  if (view.active_line_update_pending) return;
  view.active_line_update_pending = true;
  if (!PostMessageW(window_, kActiveLinePresentationMessage, 0, 0))
    view.active_line_update_pending = false;
}

void Application::ApplyPendingActiveLinePresentations() {
  for (auto& view : documents_) {
    if (!view->active_line_update_pending) continue;
    const auto queued_revision = view->active_line_update_revision;
    view->active_line_update_pending = false;
    if (queued_revision != view->document.revision() || GetFocus() != view->editor) continue;
    ApplyActiveLinePresentation(*view);
  }
}

void Application::ApplyActiveLinePresentation(DocumentView& view) {
  const std::uint64_t revision = view.document.revision();
  if (!IsWindow(view.editor) || view.ime_composing || view.native_edit_in_flight ||
      view.native_edit_pending || view.sync_due != 0 || view.presentation_due != 0 ||
      suppress_editor_change_ || !IsMarkdownFile(view.document.path()) ||
      view.presentation_revision != revision) return;

  const SourceSelection selection = CaptureSourceSelection(view.editor, view.editor_snapshot);
  const auto [active_source_begin, active_source_end] =
      SourceLineRange(view.document.text(), selection.active);
  const LONG active_native = static_cast<LONG>(
      view.editor_snapshot.SourceToNative(selection.active));
  const int active_line = static_cast<int>(
      SendMessageW(view.editor, EM_LINEFROMCHAR, active_native, 0));
  const std::size_t previous_source_begin = view.active_source_line_begin;
  if (previous_source_begin == active_source_begin) {
    view.active_line = active_line;
    return;
  }
  if (previous_source_begin == std::numeric_limits<std::size_t>::max()) {
    view.active_source_line_begin = active_source_begin;
    view.active_line = active_line;
    return;
  }
  const auto [previous_source_begin_checked, previous_source_end] =
      SourceLineRange(view.document.text(), previous_source_begin);
  (void)previous_source_begin_checked;

  ScopedEditorChangeSuppression suppression(suppress_editor_change_);
  PresentationUndoGuard undo_guard(view.editor);
  if (!undo_guard) return;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  const LONG length = GetWindowTextLengthW(view.editor);
  bool changed{};
  int invalidate_top = std::numeric_limits<int>::max();
  RECT client{};
  GetClientRect(view.editor, &client);
  const auto include_line_top = [&](std::size_t source_position) {
    const LONG native = static_cast<LONG>(view.editor_snapshot.SourceToNative(source_position));
    POINT point{};
    if (SendMessageW(view.editor, EM_POSFROMCHAR, reinterpret_cast<WPARAM>(&point), native) != -1)
      invalidate_top = std::min(invalidate_top, static_cast<int>(point.y));
  };
  include_line_top(previous_source_begin);
  include_line_top(active_source_begin);

  const auto is_marker = [](SpanKind kind) {
    return kind == SpanKind::HeadingMarker || kind == SpanKind::EmphasisMarker ||
           kind == SpanKind::ListMarker;
  };
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  const auto apply_marker_spans = [&](std::size_t line_begin, std::size_t line_end,
                                      bool active) {
    auto span = std::lower_bound(view.parse.spans.begin(), view.parse.spans.end(), line_begin,
        [](const StyleSpan& candidate, std::size_t begin) { return candidate.begin < begin; });
    for (; span != view.parse.spans.end() && span->begin < line_end; ++span) {
      if (!is_marker(span->kind) && span->kind != SpanKind::CodeFence) continue;
      const auto native_begin = view.editor_snapshot.SourceToNative(span->begin);
      const auto native_end = view.editor_snapshot.SourceToNative(span->end);
      if (native_begin >= native_end || native_begin >= static_cast<std::size_t>(length)) continue;
      SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(native_begin),
                   static_cast<LPARAM>(std::min<std::size_t>(native_end, length)));
      CHARFORMAT2W format{sizeof(format)};
      format.dwMask = CFM_HIDDEN;
      format.dwEffects = active ? 0 : CFE_HIDDEN;
      if (is_marker(span->kind)) {
        format.dwMask |= CFM_COLOR;
        format.crTextColor = ThemeColor(settings_, L"marker",
                                        dark ? RGB(150, 150, 150) : RGB(128, 128, 128));
      }
      SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION,
                   reinterpret_cast<LPARAM>(&format));
      changed = true;
    }
  };
  const auto apply_list_blocks = [&](std::size_t line_begin, std::size_t line_end,
                                     bool active) {
    auto block = std::lower_bound(view.parse.blocks.begin(), view.parse.blocks.end(), line_begin,
        [](const MarkdownBlock& candidate, std::size_t begin) { return candidate.begin < begin; });
    for (; block != view.parse.blocks.end() && block->begin < line_end; ++block) {
      if (block->kind != BlockKind::BulletListItem && block->kind != BlockKind::TaskListItem) continue;
      const LONG native_begin = static_cast<LONG>(view.editor_snapshot.SourceToNative(block->begin));
      const LONG native_end = static_cast<LONG>(view.editor_snapshot.SourceToNative(block->end));
      if (native_begin >= length || native_end <= native_begin) continue;
      std::size_t indent{};
      while (block->begin + indent < block->end) {
        const wchar_t character = view.document.text()[block->begin + indent];
        if (character == L' ') ++indent;
        else if (character == L'\t') indent += 4;
        else break;
      }
      SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(native_begin),
                   static_cast<LPARAM>(std::min<LONG>(native_end, length)));
      PARAFORMAT2 paragraph{sizeof(paragraph)};
      paragraph.dwMask = PFM_STARTINDENT | PFM_OFFSET | PFM_NUMBERING | PFM_NUMBERINGTAB;
      const LONG nesting_dip = static_cast<LONG>((indent / 2) * 16);
      paragraph.dxStartIndent = static_cast<LONG>((28 + nesting_dip) * 15);
      paragraph.dxOffset = -16 * 15;
      if (!active) {
        paragraph.wNumbering = PFN_BULLET;
        paragraph.wNumberingTab = 16 * 15;
      }
      SendMessageW(view.editor, EM_SETPARAFORMAT, 0,
                   reinterpret_cast<LPARAM>(&paragraph));
      changed = true;
    }
  };
  const auto apply_ragged_row = [&](std::size_t line_begin, bool active) {
    auto table = std::upper_bound(view.parse.tables.begin(), view.parse.tables.end(), line_begin,
        [](std::size_t position, const GfmTable& candidate) {
          return position < candidate.begin;
        });
    if (table == view.parse.tables.begin()) return;
    --table;
    if (line_begin < table->begin || line_begin > table->end) return;
    auto row = std::lower_bound(table->rows.begin(), table->rows.end(), line_begin,
        [](const TableVisualRow& candidate, std::size_t begin) { return candidate.begin < begin; });
    if (row == table->rows.end() || row->begin != line_begin) return;
    const auto row_index = static_cast<std::size_t>(row - table->rows.begin());
    const std::size_t visible_columns = table->alignments.size();
    if (row_index < 2 || row->cells.size() <= visible_columns) return;
    for (std::size_t column = visible_columns; column < row->cells.size(); ++column) {
      const auto& cell = row->cells[column];
      const LONG cell_begin = static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.begin));
      const LONG cell_end = static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.end));
      if (cell_begin >= cell_end || cell_begin >= length) continue;
      SendMessageW(view.editor, EM_SETSEL, cell_begin, std::min<LONG>(cell_end, length));
      CHARFORMAT2W hidden{sizeof(hidden)};
      hidden.dwMask = CFM_HIDDEN;
      hidden.dwEffects = active ? 0 : CFE_HIDDEN;
      SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION,
                   reinterpret_cast<LPARAM>(&hidden));
      changed = true;
    }
  };
  apply_marker_spans(previous_source_begin, previous_source_end, false);
  apply_list_blocks(previous_source_begin, previous_source_end, false);
  apply_ragged_row(previous_source_begin, false);
  apply_marker_spans(active_source_begin, active_source_end, true);
  apply_list_blocks(active_source_begin, active_source_end, true);
  apply_ragged_row(active_source_begin, true);

  RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  view.active_source_line_begin = active_source_begin;
  view.active_line = active_line;
  if (changed) {
    RECT invalid = client;
    invalid.top = std::clamp(invalidate_top == std::numeric_limits<int>::max()
                                 ? static_cast<int>(client.top)
                                 : invalidate_top - ScaleDip(window_, 2),
                             static_cast<int>(client.top), static_cast<int>(client.bottom));
    InvalidateRect(view.editor, &invalid, FALSE);
  }
}

SourceSelection Application::CaptureSourceSelection(HWND editor,
                                                     const EditorSnapshot& snapshot) const {
  CHARRANGE range{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&range));
  LONG flags{};
  IRichEditOle* rich_edit_ole{};
  if (SendMessageW(editor, EM_GETOLEINTERFACE, 0,
                   reinterpret_cast<LPARAM>(&rich_edit_ole)) != 0 && rich_edit_ole) {
    ITextDocument* document{};
    if (SUCCEEDED(rich_edit_ole->QueryInterface(__uuidof(ITextDocument),
                                                reinterpret_cast<void**>(&document))) && document) {
      ITextSelection* selection{};
      if (SUCCEEDED(document->GetSelection(&selection)) && selection) {
        LONG start{};
        LONG end{};
        if (SUCCEEDED(selection->GetStart(&start)) && SUCCEEDED(selection->GetEnd(&end)) &&
            SUCCEEDED(selection->GetFlags(&flags))) {
          range.cpMin = std::min(start, end);
          range.cpMax = std::max(start, end);
        }
        selection->Release();
      }
      document->Release();
    }
    rich_edit_ole->Release();
  }
  return NativeSelectionToSource(
      snapshot, static_cast<std::size_t>(std::max<LONG>(range.cpMin, 0)),
      static_cast<std::size_t>(std::max<LONG>(range.cpMax, 0)),
      (flags & tomSelStartActive) != 0);
}

SourceSelection Application::CaptureViewSelection(HWND editor,
                                                   const EditorSnapshot& snapshot) const {
  CHARRANGE range{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&range));
  LONG flags{};
  IRichEditOle* rich_edit_ole{};
  if (SendMessageW(editor, EM_GETOLEINTERFACE, 0,
                   reinterpret_cast<LPARAM>(&rich_edit_ole)) != 0 && rich_edit_ole) {
    ITextDocument* document{};
    if (SUCCEEDED(rich_edit_ole->QueryInterface(__uuidof(ITextDocument),
                                                reinterpret_cast<void**>(&document))) && document) {
      ITextSelection* selection{};
      if (SUCCEEDED(document->GetSelection(&selection)) && selection) {
        LONG start{};
        LONG end{};
        if (SUCCEEDED(selection->GetStart(&start)) && SUCCEEDED(selection->GetEnd(&end)) &&
            SUCCEEDED(selection->GetFlags(&flags))) {
          range.cpMin = std::min(start, end);
          range.cpMax = std::max(start, end);
        }
        selection->Release();
      }
      document->Release();
    }
    rich_edit_ole->Release();
  }
  return NativeSelectionToView(
      snapshot, static_cast<std::size_t>(std::max<LONG>(range.cpMin, 0)),
      static_cast<std::size_t>(std::max<LONG>(range.cpMax, 0)),
      (flags & tomSelStartActive) != 0);
}

void Application::RestoreSourceSelection(HWND editor, const EditorSnapshot& snapshot,
                                          SourceSelection selection) const {
  const auto native_selection = SourceSelectionToNative(snapshot, selection);
  const auto anchor = static_cast<LONG>(native_selection.anchor);
  const auto active = static_cast<LONG>(native_selection.active);
  const CHARRANGE range{std::min(anchor, active), std::max(anchor, active)};
  SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&range));
  if (anchor == active) return;

  IRichEditOle* rich_edit_ole{};
  if (SendMessageW(editor, EM_GETOLEINTERFACE, 0,
                   reinterpret_cast<LPARAM>(&rich_edit_ole)) == 0 || !rich_edit_ole) return;
  ITextDocument* document{};
  if (SUCCEEDED(rich_edit_ole->QueryInterface(__uuidof(ITextDocument),
                                              reinterpret_cast<void**>(&document))) && document) {
    ITextSelection* text_selection{};
    if (SUCCEEDED(document->GetSelection(&text_selection)) && text_selection) {
      LONG flags{};
      if (SUCCEEDED(text_selection->GetFlags(&flags))) {
        flags = active < anchor ? flags | tomSelStartActive : flags & ~tomSelStartActive;
        text_selection->SetFlags(flags);
      }
      text_selection->Release();
    }
    document->Release();
  }
  rich_edit_ole->Release();
}

bool Application::RestoreVirtualTableCellCaret(DocumentView& view,
                                                 const TableCellIntent& intent) {
  // Source punctuation around a virtual cell collapses in the plain-text view;
  // use the rebuilt native cell boundary to restore its editing intent.
  if (!view.native_tables_ready || !IsWindow(view.editor) || !intent.virtual_cell) return false;
  for (const auto& table : view.editor_snapshot.tables) {
    if (intent.row_begin < table.source_begin || intent.row_begin >= table.source_end ||
        !table.native_coordinates_set) continue;
    const auto row = std::ranges::find_if(table.visual_rows, [&](const auto& candidate) {
      return candidate.source_begin == intent.row_begin;
    });
    if (row == table.visual_rows.end() || intent.column >= row->cells.size()) return false;
    const auto& cell = row->cells[intent.column];
    if (!cell.virtual_cell || cell.native_begin > cell.native_end ||
        cell.native_begin > static_cast<std::size_t>(LONG_MAX)) return false;
    const LONG position = static_cast<LONG>(cell.native_begin);
    CHARRANGE selection{position, position};
    SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    view.pending_virtual_table_cell = TableCellIntent{
        intent.row_begin, intent.column, cell.source_begin, true};
    view.pending_table_high_surrogate = 0;
    SetFocus(view.editor);
    return true;
  }
  return false;
}

RichEditTableStyle Application::TableStyleForEditor(HWND editor) const {
  RECT formatting_rect{};
  SendMessageW(editor, EM_GETRECT, 0, reinterpret_cast<LPARAM>(&formatting_rect));
  const LONG dpi = static_cast<LONG>(std::max<UINT>(1, GetDpiForWindow(window_)));
  const LONG available_width = std::max<LONG>(1, formatting_rect.right - formatting_rect.left);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  RichEditTableStyle style;
  style.available_width_twips = MulDiv(available_width, 1440, dpi);
  style.minimum_cell_width_twips =
      std::max<LONG>(800, static_cast<LONG>(settings_.font_size_pt) * 20 * 6);
  style.cell_margin_twips = ScaleDip(window_, kTableCellPadding / 2) * 15;
  style.font_size_half_points = static_cast<LONG>(settings_.font_size_pt) * 2;
  style.border_color = theme_border_;
  style.header_background = ThemeColor(settings_, L"table_header_background",
                                        dark ? RGB(31, 40, 52) : RGB(231, 236, 242));
  style.body_background = ThemeColor(settings_, L"table_background",
                                     dark ? RGB(38, 42, 48) : RGB(244, 247, 250));
  return style;
}

Application::EditorProjectionRebuildResult Application::RebuildEditorProjection(
    DocumentView& view, EditorSnapshot target, SourceSelection selection) {
  if (view.editor_projection_invalid || !view.editor || !IsWindow(view.editor))
    return EditorProjectionRebuildResult::Failed;
  ScopedEditorChangeSuppression suppression(suppress_editor_change_);
  PresentationUndoGuard undo_guard(view.editor);
  if (!undo_guard) {
    QuarantineEditorProjection(
        view, selection,
        L"本文は保持されていますが、ネイティブエディターの更新を開始できません。再構築します。");
    return EditorProjectionRebuildResult::Failed;
  }
  POINT scroll{};
  SendMessageW(view.editor, EM_GETSCROLLPOS, 0, reinterpret_cast<LPARAM>(&scroll));
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  bool flat_fallback{};
  if (!SetEditorFlatTextVerified(view, target, kNativeProjectionFaultRebuildWrite)) {
    target = BuildNativeTextEditorSnapshot(view.document.text());
    flat_fallback = true;
    if (!SetEditorFlatTextVerified(view, target, kNativeProjectionFaultFlatFallback)) {
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, TRUE);
      SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
      QuarantineEditorProjection(
          view, selection,
          L"本文は保持されていますが、表示の再構築に失敗しました。編集領域を再作成します。");
      return EditorProjectionRebuildResult::Failed;
    }
  }
  bool tables_rebuilt = true;
  if (!flat_fallback && IsMarkdownFile(view.document.path()) && !target.tables.empty()) {
    if (TestAutomationSilent() && view.force_partial_markdown_presentation_failure_for_test &&
        target.tables.size() >= 2) {
      view.force_partial_markdown_presentation_failure_for_test = false;
      EditorSnapshot first_table_target = target;
      first_table_target.tables.resize(1);
      (void)RebuildRichEditTables(view.editor, first_table_target, TableStyleForEditor(view.editor));
      tables_rebuilt = false;
    } else {
      tables_rebuilt = RebuildRichEditTables(view.editor, target, TableStyleForEditor(view.editor));
    }
  }
  if (!tables_rebuilt) {
    target = BuildNativeTextEditorSnapshot(view.document.text());
    flat_fallback = true;
    if (!SetEditorFlatTextVerified(view, target, kNativeProjectionFaultFlatFallback)) {
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, TRUE);
      SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
      QuarantineEditorProjection(
          view, selection,
          L"表表示の失敗後に本文表示を復元できませんでした。本文を保持して編集領域を再作成します。");
      return EditorProjectionRebuildResult::Failed;
    }
  }
  view.editor_snapshot = std::move(target);
  view.flat_source_fallback_pending = flat_fallback && IsMarkdownFile(view.document.path());
  if (flat_fallback) {
    view.editor_projection_repair_attempts = 0;
    ScheduleFlatSourceFallbackRetry(view);
  } else {
    view.flat_source_fallback_retry_used = false;
    view.editor_projection_repair_attempts = 0;
  }
  view.native_tables_ready = !flat_fallback &&
      std::ranges::all_of(view.editor_snapshot.tables,
          [](const EditorTableMapping& table) { return table.native_coordinates_set; });
  view.parse = IsMarkdownFile(view.document.path()) ? ParseMarkdown(view.document.text())
                                                    : MarkdownParseResult{};
  for (auto& image : view.rendered_images) image.inserted = false;
  view.animated_image_frames.clear();
  view.animation_due = 0;
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
  SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
  SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  SendMessageW(view.editor, EM_SETSCROLLPOS, 0, reinterpret_cast<LPARAM>(&scroll));
  InvalidateRect(view.editor, nullptr, TRUE);
  view.active_line = -1;
  view.active_source_line_begin = std::numeric_limits<std::size_t>::max();
  view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
  if (flat_fallback)
    SetStatusText(L"本文は同期済みです。表または画像の表示を再構築しています。");
  return flat_fallback ? EditorProjectionRebuildResult::FlatSourceFallback
                       : EditorProjectionRebuildResult::Native;
}

void Application::UpdatePendingVirtualTableCellFromCaret(DocumentView& view,
                                                         const POINT* click_point) {
  if (view.editor_projection_invalid || !view.native_tables_ready ||
      view.sync_due != 0 || view.presentation_due != 0 ||
      view.ime_composing) {
    view.pending_virtual_table_cell.reset();
    return;
  }
  if (click_point) {
    const LRESULT hit = SendMessageW(view.editor, EM_CHARFROMPOS, 0,
                                     reinterpret_cast<LPARAM>(click_point));
    if (hit >= 0) {
      for (const auto& table : view.editor_snapshot.tables) {
        for (const auto& row : table.visual_rows) {
          for (std::size_t column{}; column < row.cells.size(); ++column) {
            const auto& cell = row.cells[column];
            if (!cell.virtual_cell || static_cast<std::size_t>(hit) < cell.native_begin ||
                static_cast<std::size_t>(hit) > cell.native_end) continue;
            CHARRANGE virtual_caret{static_cast<LONG>(hit), static_cast<LONG>(hit)};
            SendMessageW(view.editor, EM_EXSETSEL, 0,
                         reinterpret_cast<LPARAM>(&virtual_caret));
            view.pending_virtual_table_cell = TableCellIntent{
                row.source_begin, column, cell.source_begin, true};
            view.pending_table_high_surrogate = 0;
            return;
          }
        }
      }
    }
  }
  CHARRANGE selection_range{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection_range));
  if (selection_range.cpMin != selection_range.cpMax) {
    view.pending_virtual_table_cell.reset();
    return;
  }
  IRichEditOle* rich_edit{};
  if (SendMessageW(view.editor, EM_GETOLEINTERFACE, 0,
                   reinterpret_cast<LPARAM>(&rich_edit)) == 0 || !rich_edit) {
    view.pending_virtual_table_cell.reset();
    return;
  }
  ITextDocument* document{};
  ITextSelection* selection{};
  ITextSelection2* selection2{};
  ITextRow* row{};
  LONG column{-1};
  if (SUCCEEDED(rich_edit->QueryInterface(__uuidof(ITextDocument),
                                         reinterpret_cast<void**>(&document))) && document &&
      SUCCEEDED(document->GetSelection(&selection)) && selection &&
      SUCCEEDED(selection->QueryInterface(__uuidof(ITextSelection2),
                                          reinterpret_cast<void**>(&selection2))) && selection2 &&
      SUCCEEDED(selection2->GetRow(&row)) && row) {
    row->GetCellIndex(&column);
  }
  if (row) row->Release();
  if (selection2) selection2->Release();
  if (selection) selection->Release();
  if (document) document->Release();
  rich_edit->Release();
  if (column < 0) {
    view.pending_virtual_table_cell.reset();
    return;
  }

  const auto native_position = static_cast<std::size_t>(selection_range.cpMin);
  for (const auto& table : view.editor_snapshot.tables) {
    if (native_position < table.native_begin || native_position > table.native_end) continue;
    for (const auto& visual_row : table.visual_rows) {
      if (native_position < visual_row.native_begin || native_position > visual_row.native_end ||
          static_cast<std::size_t>(column) >= visual_row.cells.size()) continue;
      const auto& cell = visual_row.cells[static_cast<std::size_t>(column)];
      if (cell.virtual_cell) {
        view.pending_virtual_table_cell = TableCellIntent{
            visual_row.source_begin, static_cast<std::size_t>(column), cell.source_begin, true};
      } else {
        view.pending_virtual_table_cell.reset();
      }
      return;
    }
  }
  view.pending_virtual_table_cell.reset();
}

bool Application::SyncDocumentFromEditor(DocumentView& view) {
  // EN_CHANGE is the authority for pending native-editor input. Presentation
  // updates are performed with notifications suppressed and must never be read
  // back as Markdown source (RichEdit may expose an image object as a space).
  if (view.editor_projection_invalid) {
    if (view.sync_due != 0 || view.native_edit_pending || view.native_edit_in_flight) {
      SetStatusText(L"エディター表示を修復中のため、入力を読み戻せません。本文は保持されています。");
      return false;
    }
    return true;
  }
  if (!view.editor || !IsWindow(view.editor))
    return view.sync_due == 0 && !view.native_edit_pending && !view.native_edit_in_flight;
  if (view.sync_due == 0 && !view.native_edit_pending) return true;
  view.sync_due = 0;
  auto before_native_snapshot = view.editor_snapshot;
  std::wstring current_view;
  std::wstring current_native_text;
  const bool force_readback_failure = TestAutomationSilent() &&
      view.force_editor_readback_failures_for_test > 0;
  if (force_readback_failure) --view.force_editor_readback_failures_for_test;
  if (force_readback_failure ||
      !EditorText(view.editor, before_native_snapshot, current_view, &current_native_text)) {
    RecordDiagnosticSummary(L"エディター内容の読戻しを再試行します");
    // Never infer a source edit from a truncated/misaligned native readback.
    // Keep the pending input alive for a later timer pass without showing a
    // dialog or turning a presentation-only failure into data loss.
    view.native_readback_failed = true;
    // Lock the control while its text is untrusted. This also prevents RichEdit's
    // OLE drag/drop path from editing behind the failed-readback guard.
    if (!view.ime_composing && view.editor && IsWindow(view.editor) &&
        !view.editor_locked_for_readback) {
      view.editor_enabled_before_readback_lock = IsWindowEnabled(view.editor) != FALSE;
      view.editor_locked_for_readback = true;
      const LRESULT read_only_result = SendMessageW(view.editor, EM_SETREADONLY, TRUE, 0);
      const auto style = static_cast<DWORD_PTR>(GetWindowLongPtrW(view.editor, GWL_STYLE));
      view.editor_readonly_lock_applied = read_only_result != 0 ||
          (style & ES_READONLY) != 0;
      if (!view.editor_readonly_lock_applied) {
        RecordDiagnosticSummary(L"読戻し再試行中のRichEdit読み取り専用化に失敗しました");
        SetStatusText(L"読戻しを再試行中です。編集領域を無効化して本文を保護しています。");
      }
      if (view.editor_enabled_before_readback_lock) EnableWindow(view.editor, FALSE);
    }
    view.native_edit_pending = true;
    view.sync_due = GetTickCount64() + kEditorSyncDelayMs;
    return false;
  }
  const auto release_readback_lock = [&]() {
    if (!view.editor_locked_for_readback) return true;
    if (!view.editor || !IsWindow(view.editor)) return false;
    if (!view.editor_readonly_lock_applied) {
      const LRESULT read_only_result = SendMessageW(view.editor, EM_SETREADONLY, TRUE, 0);
      const auto style = static_cast<DWORD_PTR>(GetWindowLongPtrW(view.editor, GWL_STYLE));
      view.editor_readonly_lock_applied = read_only_result != 0 ||
          (style & ES_READONLY) != 0;
      if (!view.editor_readonly_lock_applied) return false;
    }
    SendMessageW(view.editor, EM_SETREADONLY, FALSE, 0);
    const auto style = static_cast<DWORD_PTR>(GetWindowLongPtrW(view.editor, GWL_STYLE));
    if ((style & ES_READONLY) != 0) return false;
    if (view.editor_enabled_before_readback_lock) EnableWindow(view.editor, TRUE);
    view.editor_locked_for_readback = false;
    view.editor_readonly_lock_applied = false;
    view.editor_enabled_before_readback_lock = false;
    return true;
  };
  const bool editor_unlocked = release_readback_lock();
  view.native_readback_failed = !editor_unlocked;
  const EditorEditHint edit_hint{view.pending_selection_before,
                                 view.pending_native_edit_kind};
  const auto transaction = ApplyEditorText(
      before_native_snapshot, view.document.text(), current_view, edit_hint);
  if (transaction.identity_ambiguous) {
    view.pending_selection_before.reset();
    const SourceSelection selection = CaptureSourceSelection(view.editor, before_native_snapshot);
    QuarantineEditorProjection(
        view, selection,
        L"画像オブジェクトの元本文を特定できないため同期を保留しました。本文は保持されています。");
    return false;
  }
  if (!transaction.changed) {
    view.native_edit_pending = false;
    view.pending_selection_before.reset();
    view.pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
    if (!editor_unlocked) {
      const SourceSelection selection = CaptureSourceSelection(view.editor, before_native_snapshot);
      QuarantineEditorProjection(
          view, selection,
          L"入力内容は確認しましたが、エディターのロックを解除できないため再作成します。本文は保持されています。");
      return true;
    }
    view.native_readback_failed = false;
    UpdateStatus();
    return true;
  }
  auto target_native = NativeSnapshotFor(view.document.path(), transaction.source);
  const bool table_topology_same = SameNativeTableTopology(before_native_snapshot, target_native);
  const bool target_native_mapped = table_topology_same &&
      RefreshRichEditTableCoordinates(target_native, current_native_text);
  const SourceSelection selection_after = target_native_mapped
      ? CaptureSourceSelection(view.editor, target_native)
      : [&] {
          const SourceSelection current_view_selection =
              CaptureViewSelection(view.editor, before_native_snapshot);
          return SourceSelection{
              target_native.ViewToSource(MapPositionBetweenViews(
                  current_view, target_native.view, current_view_selection.anchor)),
              target_native.ViewToSource(MapPositionBetweenViews(
                  current_view, target_native.view, current_view_selection.active))};
        }();
  const bool projection_changed = current_view != target_native.view;
  const bool rebuild_projection = !table_topology_same || !target_native_mapped ||
      (projection_changed && !target_native.tables.empty()) ||
      (projection_changed && (!before_native_snapshot.collapsed.empty() ||
                              !target_native.collapsed.empty()));
  InvalidateTableGrid(view);
  const std::size_t old_length = transaction.old_end - transaction.begin;
  const std::size_t new_length = transaction.new_end - transaction.begin;
  const auto map_after_to_before = [&](std::size_t position) {
    if (position <= transaction.begin) return position;
    if (position >= transaction.new_end) return position - new_length + old_length;
    return transaction.begin + std::min(position - transaction.begin, old_length);
  };
  const SourceSelection selection_before = view.pending_selection_before.value_or(
      SourceSelection{map_after_to_before(selection_after.anchor),
                      map_after_to_before(selection_after.active)});
  view.source_undo.push_back({transaction.begin,
      view.document.text().substr(transaction.begin, old_length),
      transaction.source.substr(transaction.begin, new_length),
      selection_before, selection_after});
  if (view.source_undo.size() > 100) view.source_undo.erase(view.source_undo.begin());
  view.source_redo.clear();
  view.document.MarkEdited(transaction.source);
  view.flat_source_fallback_retry_used = false;
  view.native_edit_pending = false;
  view.pending_selection_before.reset();
  view.pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
  if (!editor_unlocked) {
    view.parse = IsMarkdownFile(view.document.path()) ? ParseMarkdown(view.document.text())
                                                      : MarkdownParseResult{};
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
    if (active_document_ < documents_.size() && documents_[active_document_].get() == &view)
      RebuildOutline(view);
    UpdateStatus();
    QuarantineEditorProjection(
        view, selection_after,
        L"入力本文は同期済みですが、エディターのロックを解除できないため再作成します。本文は保持されています。");
    return true;
  }
  view.native_readback_failed = false;
  if (rebuild_projection) {
    const auto outcome = RebuildEditorProjection(view, std::move(target_native), selection_after);
    if (outcome == EditorProjectionRebuildResult::Native)
      SchedulePresentation(view);
    return true;
  }
  bool projection_verified = true;
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    PresentationUndoGuard undo_guard(view.editor);
    if (!undo_guard) {
      QuarantineEditorProjection(
          view, selection_after,
          L"本文は同期しましたが、ネイティブ表示を更新できません。編集領域を再構築します。");
      return true;
    }
    if (projection_changed) {
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
      if (ConsumeNativeProjectionFailureForTest(
              view, kNativeProjectionFaultIncrementalWrite)) {
        std::wstring partial = target_native.view;
        if (partial.empty()) partial.push_back(L'x');
        else partial.resize(partial.size() - 1);
        SetWindowTextW(view.editor, partial.c_str());
      }
      projection_verified = RichEditTextEquals(view.editor, target_native.view);
      if (projection_verified)
        RestoreSourceSelection(view.editor, target_native, selection_after);
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, FALSE);
    }
    // Source history is authoritative.  The native control's typing undo can
    // no longer reach that pre-transaction state, so discard it explicitly.
    SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
  }
  if (!projection_verified) {
    const auto outcome = RebuildEditorProjection(view, std::move(target_native), selection_after);
    if (outcome == EditorProjectionRebuildResult::Native)
      SchedulePresentation(view);
    return true;
  }
  view.editor_snapshot = std::move(target_native);
  view.flat_source_fallback_pending = false;
  view.flat_source_fallback_retry_used = false;
  view.native_tables_ready = std::ranges::all_of(view.editor_snapshot.tables,
      [](const EditorTableMapping& table) { return table.native_coordinates_set; });
  if (projection_changed) {
    view.parse = ParseMarkdown(view.document.text());
    RefreshDerivedImages(view);
    SchedulePresentation(view);
  }
  // The timer schedules presentation after the source transaction settles.
  // Parsing or decoding here would move the deferred work back onto commands
  // that only need the Markdown source (Save, Find, table navigation).
  return true;
}

bool Application::ConsumeNativeProjectionFailureForTest(DocumentView& view,
                                                        std::uint32_t stage) {
  if (!TestAutomationSilent() || stage == 0 ||
      (view.native_projection_failures_for_test & stage) == 0) return false;
  view.native_projection_failures_for_test &= ~stage;
  return true;
}

bool Application::SetEditorFlatTextVerified(DocumentView& view,
                                            const EditorSnapshot& expected,
                                            std::uint32_t test_failure_stage) {
  if (!view.editor || !IsWindow(view.editor)) return false;
  if (ConsumeNativeProjectionFailureForTest(view, test_failure_stage)) {
    std::wstring partial = expected.view;
    if (partial.empty()) partial.push_back(L'x');
    else partial.resize(partial.size() - 1);
    if (!SetWindowTextW(view.editor, partial.c_str())) return false;
    return RichEditFlatProjectionEquals(view.editor, expected);
  }
  return SetWindowTextW(view.editor, expected.view.c_str()) &&
         RichEditFlatProjectionEquals(view.editor, expected);
}

void Application::QuarantineEditorProjection(DocumentView& view, SourceSelection selection,
                                             std::wstring_view status_text) {
  const auto source_size = view.document.text().size();
  view.editor_projection_invalid = true;
  view.editor_formatting_rect_request.reset();
  view.native_readback_failed = false;
  view.editor_locked_for_readback = false;
  view.editor_readonly_lock_applied = false;
  view.editor_enabled_before_readback_lock = false;
  view.native_tables_ready = false;
  view.pending_virtual_table_cell.reset();
  view.pending_native_edit_kind = DocumentView::PendingNativeEditKind::Unknown;
  view.pending_table_high_surrogate = 0;
  view.suspended_selection.anchor = std::min(selection.anchor, source_size);
  view.suspended_selection.active = std::min(selection.active, source_size);
  view.suspended_first_visible_source = view.suspended_selection.active;
  view.suspended_horizontal_left_edge_source = view.suspended_selection.active;
  view.suspended_view_state_valid = true;
  view.presentation_due = 0;
  if (view.editor && IsWindow(view.editor)) {
    EnableWindow(view.editor, FALSE);
    ShowWindow(view.editor, SW_HIDE);
  }
  SetStatusText(std::wstring(status_text));
  if (!editor_projection_repair_message_posted_) {
    if (view.editor_projection_repair_attempts == 0) {
      editor_projection_repair_message_posted_ = PostMessageW(
          window_, kRepairInvalidEditorProjectionMessage, 0, 0) != FALSE;
      editor_projection_repair_retry_due_ = editor_projection_repair_message_posted_
          ? 0 : GetTickCount64() + 1000;
    } else {
      editor_projection_repair_retry_due_ = GetTickCount64() + 1000;
    }
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  }
}

void Application::ProcessDeferredEditorRepairs() {
  editor_projection_repair_retry_due_ = 0;
  for (std::size_t index{}; index < documents_.size(); ++index) {
    auto& view = *documents_[index];
    if (!view.editor_projection_invalid) continue;
    ++view.editor_projection_repair_attempts;

    if (view.compact_window && IsWindow(view.compact_window))
      SendMessageW(view.compact_window, WM_CLOSE, 0, 0);

    const HWND unusable_editor = view.editor;
    view.editor = nullptr;
    view.editor_formatting_rect_request.reset();
    if (unusable_editor && IsWindow(unusable_editor)) {
      RemoveWindowSubclass(unusable_editor, EditorSubclass, 1);
      DestroyWindow(unusable_editor);
    }
    view.editor_projection_invalid = false;
    view.native_readback_failed = false;
    view.editor_locked_for_readback = false;
    view.editor_readonly_lock_applied = false;
    view.editor_enabled_before_readback_lock = false;
    view.flat_source_fallback_pending = false;
    view.flat_source_fallback_retry_used = false;
    view.editor_snapshot = NativeSnapshotFor(view.document);
    view.native_tables_ready = false;
    view.active_line = -1;
    view.active_source_line_begin = std::numeric_limits<std::size_t>::max();
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();

    if (index != active_document_ || view.compact_window) {
      view.editor_projection_repair_attempts = 0;
      continue;
    }
    if (!EnsureEditor(view)) {
      view.editor_projection_invalid = true;
      editor_projection_repair_retry_due_ = GetTickCount64() + 1000;
      SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
      SetStatusText(L"本文は保持されていますが、エディターを再構築できませんでした。保存後に再試行してください。");
      continue;
    }
    if (!IsMarkdownFile(view.document.path())) view.editor_projection_repair_attempts = 0;
    LayoutControls();
    ShowWindow(view.editor, SW_SHOW);
    RestoreSourceSelection(view.editor, view.editor_snapshot, view.suspended_selection);
    view.suspended_view_state_valid = false;
    SetFocus(view.editor);
    ApplyMarkdownPresentation(view, true, view.suspended_selection);
  }
}

bool Application::ApplySourceTextWithUndo(DocumentView& view, std::wstring text, bool record_history,
                                          std::optional<SourceSelection> selection_after,
                                          std::optional<SourceSelection> selection_before_override,
                                          std::optional<TableCellIntent> selection_before_virtual_cell) {
  if (view.editor_projection_invalid || view.native_readback_failed ||
      view.sync_due != 0 || view.native_edit_pending) return false;
  if (text == view.document.text()) return true;
  const bool was_suspended = !view.editor;
  const SourceSelection stored_selection = view.suspended_selection;
  const bool stored_view_state_valid = view.suspended_view_state_valid;
  if (!EnsureEditor(view)) return false;
  if (was_suspended) {
    LayoutControls();
    RestoreEditorViewState(view);
  }
  const bool scroll_after = selection_after.has_value() || !record_history;
  const SourceSelection selection_before = selection_before_override
      ? *selection_before_override
      : was_suspended && stored_view_state_valid
          ? stored_selection
          : CaptureSourceSelection(view.editor, view.editor_snapshot);
  auto target_native = NativeSnapshotFor(view.document.path(), text);
  std::optional<DocumentView::SourceEdit> history_entry;
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
    const auto map_position = [&](std::size_t position) {
      if (position <= prefix) return position;
      if (position >= before_suffix) {
        if (after_suffix >= before_suffix) return position + (after_suffix - before_suffix);
        return position - (before_suffix - after_suffix);
      }
      return prefix + std::min(position - prefix, after_suffix - prefix);
    };
    SourceSelection restored = selection_after.value_or(SourceSelection{
        map_position(selection_before.anchor), map_position(selection_before.active)});
    restored.anchor = std::min(restored.anchor, text.size());
    restored.active = std::min(restored.active, text.size());
    history_entry.emplace(DocumentView::SourceEdit{
        prefix, before.substr(prefix, before_suffix - prefix),
        text.substr(prefix, after_suffix - prefix), selection_before, restored,
        selection_before_virtual_cell});
    selection_after = restored;
  }
  InvalidateTableGrid(view);
  {
    ScopedEditorChangeSuppression suppression(suppress_editor_change_);
    PresentationUndoGuard undo_guard(view.editor);
    if (!undo_guard) return false;
    SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
    const bool target_installed = SetEditorFlatTextVerified(
        view, target_native, kNativeProjectionFaultProgrammaticWrite);
    if (!target_installed) {
      auto original_flat_snapshot = BuildNativeTextEditorSnapshot(view.document.text());
      const bool original_restored = SetEditorFlatTextVerified(
          view, original_flat_snapshot, kNativeProjectionFaultProgrammaticRestore);
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, TRUE);
      SendMessageW(view.editor, EM_EMPTYUNDOBUFFER, 0, 0);
      if (original_restored) {
        view.editor_snapshot = std::move(original_flat_snapshot);
        view.flat_source_fallback_pending = IsMarkdownFile(view.document.path());
        view.editor_projection_repair_attempts = 0;
        view.native_tables_ready = false;
        for (auto& image : view.rendered_images) image.inserted = false;
        view.animated_image_frames.clear();
        view.animation_due = 0;
        view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
        view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
        RestoreSourceSelection(view.editor, view.editor_snapshot, selection_before);
        ScheduleFlatSourceFallbackRetry(view);
        SetStatusText(L"本文のネイティブ表示を更新できませんでした。本文とUndo履歴は変更せず、元の編集表示へ戻しました。");
        return false;
      }
      QuarantineEditorProjection(
          view, selection_before,
          L"本文のネイティブ表示を復元できませんでした。本文とUndo履歴は保持してエディターを再構築します。");
      return false;
    }
    SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
    SourceSelection restored = selection_after.value_or(selection_before);
    RestoreSourceSelection(view.editor, target_native, restored);
    if (history_entry) {
      if (view.source_undo.size() >= 100) view.source_undo.erase(view.source_undo.begin());
      view.source_undo.push_back(std::move(*history_entry));
      view.source_redo.clear();
    }
    view.document.MarkEdited(std::move(text));
    view.editor_snapshot = std::move(target_native);
    view.flat_source_fallback_pending = false;
    view.flat_source_fallback_retry_used = false;
    view.native_tables_ready = false;
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
  if (IsMarkdownFile(view.document.path()))
    // Do not recapture from the flattened RichEdit projection here: table-tail
    // source positions can share one native offset until the RTF table is built.
    ApplyMarkdownPresentation(view, true, selection_after.value_or(selection_before));
  if (scroll_after) SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  if (active_document_ < documents_.size() && documents_[active_document_].get() == &view)
    RebuildOutline(view);
  const ULONGLONG now = GetTickCount64();
  view.autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
  view.recovery_due = now + kRecoveryDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  UpdateStatus();
  if (was_suspended) {
    ShowWindow(view.editor, SW_HIDE);
    (void)SuspendEditor(view);
  }
  return true;
}

bool Application::ApplySourceHistory(DocumentView& view, bool redo) {
  if ((view.sync_due != 0 || view.native_edit_pending) && !SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力を読み取れなかったためUndo/Redoを中断しました。もう一度実行してください。");
    return false;
  }
  if (view.sync_due != 0 || view.native_edit_pending) {
    SetStatusText(L"入力を読み取れなかったためUndo/Redoを中断しました。もう一度実行してください。");
    return false;
  }
  auto& source = redo ? view.source_redo : view.source_undo;
  auto& destination = redo ? view.source_undo : view.source_redo;
  if (source.empty()) return false;
  auto edit = std::move(source.back());
  source.pop_back();
  const auto& expected = redo ? edit.before : edit.after;
  const auto& replacement = redo ? edit.after : edit.before;
  const SourceSelection selection_after = redo ? edit.selection_after : edit.selection_before;
  std::wstring text = view.document.text();
  if (edit.begin > text.size() || expected.size() > text.size() - edit.begin ||
      text.compare(edit.begin, expected.size(), expected) != 0) {
    source.push_back(edit);
    return false;
  }
  text.replace(edit.begin, expected.size(), replacement);
  destination.push_back(edit);
  if (!ApplySourceTextWithUndo(view, std::move(text), false, selection_after)) {
    destination.pop_back();
    source.push_back(std::move(edit));
    SetStatusText(L"本文が同期中のためUndo/Redoを中断しました。もう一度実行してください。");
    return false;
  }
  if (!redo && edit.selection_before_virtual_cell &&
      !RestoreVirtualTableCellCaret(view, *edit.selection_before_virtual_cell)) {
    SetStatusText(L"Undo後の空セル位置を復元できませんでした。本文は復元済みです。");
  }
  return true;
}

bool Application::InsertTextIntoPendingVirtualTableCell(
    DocumentView& view, std::wstring_view text,
    std::optional<SourceSelection> selection_before) {
  if (!view.pending_virtual_table_cell || text.empty()) return false;
  const auto target = *view.pending_virtual_table_cell;
  view.pending_virtual_table_cell.reset();
  view.pending_table_high_surrogate = 0;
  const auto edit = InsertTextIntoMissingTableCell(view.document.text(), target.row_begin,
                                                    target.column, text);
  if (!edit.changed) return false;
  const bool applied = ApplySourceTextWithUndo(
      view, edit.text, true, SourceSelection{edit.selection, edit.selection}, selection_before,
      target);
  if (!applied) return false;
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
  if (view.editor_projection_invalid || view.native_readback_failed ||
      view.sync_due != 0 || view.native_edit_pending || view.ime_composing ||
      !IsMarkdownFile(view.document.path())) return;
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
    render_needed.push_back(!can_reuse || !view.rendered_images.back().inserted);
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
  if (view.editor_projection_invalid || view.native_readback_failed || view.sync_due != 0 ||
      view.native_edit_pending || view.ime_composing || view.animation_due == 0 ||
      now < view.animation_due || !IsWindowVisible(view.editor)) return;
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

void Application::ApplyMarkdownPresentation(
    DocumentView& view, bool force,
    std::optional<SourceSelection> source_selection) {
  if (view.editor_projection_invalid || view.native_readback_failed) return;
  if (!IsMarkdownFile(view.document.path())) return;
  if (view.ime_composing) return;
  if (view.sync_due != 0) return;
  if (!view.editor || !IsWindow(view.editor)) {
    view.parse = ParseMarkdown(view.document.text());
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    view.active_line = -1;
    view.active_source_line_begin = std::numeric_limits<std::size_t>::max();
    return;
  }
  if (TestAutomationSilent() && view.force_markdown_presentation_failure_for_test) {
    view.force_markdown_presentation_failure_for_test = false;
    return;
  }
  const std::uint64_t revision = view.document.revision();
  const SourceSelection selection = source_selection.value_or(
      CaptureSourceSelection(view.editor, view.editor_snapshot));
  const std::wstring& source = view.document.text();
  if (view.flat_source_fallback_pending) {
    const auto outcome = RebuildEditorProjection(
        view, NativeSnapshotFor(view.document), selection);
    if (outcome != EditorProjectionRebuildResult::Native) return;
  }
  const auto [active_source_line_begin, active_source_line_end] =
      SourceLineRange(source, selection.active);
  if (!force && view.presentation_revision == revision) {
    if (view.active_source_line_begin != active_source_line_begin)
      ApplyActiveLinePresentation(view);
    return;
  }
  InvalidateTableGrid(view);
  view.parse = ParseMarkdown(source);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());

  ScopedEditorChangeSuppression suppression(suppress_editor_change_);
  PresentationUndoGuard undo_guard(view.editor);
  if (!undo_guard) return;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  const bool tables_mapped = std::ranges::all_of(view.editor_snapshot.tables,
      [](const EditorTableMapping& table) { return table.native_coordinates_set; });
  if (!tables_mapped) {
    RECT formatting_rect{};
    SendMessageW(view.editor, EM_GETRECT, 0, reinterpret_cast<LPARAM>(&formatting_rect));
    const LONG available_width = std::max<LONG>(1, formatting_rect.right - formatting_rect.left);
    const LONG dpi = static_cast<LONG>(std::max<UINT>(1, GetDpiForWindow(window_)));
    RichEditTableStyle table_style;
    table_style.available_width_twips = MulDiv(available_width, 1440, dpi);
    table_style.minimum_cell_width_twips =
        std::max<LONG>(800, static_cast<LONG>(settings_.font_size_pt) * 20 * 6);
    table_style.cell_margin_twips = ScaleDip(window_, 6) * 15;
    table_style.font_size_half_points = static_cast<LONG>(settings_.font_size_pt) * 2;
    table_style.border_color = theme_border_;
    table_style.header_background = ThemeColor(settings_, L"table_header_background",
                                                dark ? RGB(31, 40, 52) : RGB(231, 236, 242));
    table_style.body_background = ThemeColor(settings_, L"table_background",
                                              dark ? RGB(38, 42, 48) : RGB(244, 247, 250));
    bool tables_rebuilt{};
    if (TestAutomationSilent() &&
        view.force_partial_markdown_presentation_failure_for_test &&
        view.parse.tables.size() >= 2) {
      view.force_partial_markdown_presentation_failure_for_test = false;
      EditorSnapshot first_table_snapshot = view.editor_snapshot;
      first_table_snapshot.tables.resize(1);
      tables_rebuilt = RebuildRichEditTables(view.editor, first_table_snapshot, table_style);
      tables_rebuilt = false;
    } else {
      tables_rebuilt = RebuildRichEditTables(view.editor, view.editor_snapshot, table_style);
    }
    if (!tables_rebuilt) {
      view.pending_virtual_table_cell.reset();
      view.pending_table_high_surrogate = 0;
      auto flat_snapshot = BuildNativeTextEditorSnapshot(source);
      if (SetEditorFlatTextVerified(view, flat_snapshot,
                                    kNativeProjectionFaultFlatFallback)) {
        view.editor_snapshot = std::move(flat_snapshot);
        view.flat_source_fallback_pending = true;
        view.editor_projection_repair_attempts = 0;
        ScheduleFlatSourceFallbackRetry(view);
        view.native_tables_ready = false;
        for (auto& image : view.rendered_images) image.inserted = false;
        view.animated_image_frames.clear();
        view.animation_due = 0;
        view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
        RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
      } else {
        SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
        InvalidateRect(view.editor, nullptr, TRUE);
        QuarantineEditorProjection(
            view, selection,
            L"表のネイティブ表示を復元できません。本文を保持してエディター領域を再構築します。");
        return;
      }
      SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
      InvalidateRect(view.editor, nullptr, TRUE);
      SetStatusText(L"表のセル表示を構築できませんでした。原文は保持されています。");
      return;
    }
    RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
  }
  view.native_tables_ready = std::ranges::all_of(view.editor_snapshot.tables,
      [](const EditorTableMapping& table) { return table.native_coordinates_set; });
  const auto native_active = static_cast<LONG>(
      view.editor_snapshot.SourceToNative(selection.active));
  const int active_line = static_cast<int>(
      SendMessageW(view.editor, EM_LINEFROMCHAR, native_active, 0));
  RefreshDerivedImages(view);
  const LONG length = GetWindowTextLengthW(view.editor);
  const COLORREF foreground = ThemeColor(settings_, L"foreground", dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
  const COLORREF background = theme_editor_;
  SendMessageW(view.editor, EM_SETSEL, 0, length);
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
  base_paragraph.dwMask |= PFM_TABSTOPS | PFM_NUMBERING | PFM_STARTINDENT |
                           PFM_OFFSET | PFM_NUMBERINGTAB;
  base_paragraph.cTabCount = 0;
  base_paragraph.dySpaceBefore = 0;
  base_paragraph.dySpaceAfter = 0;
  base_paragraph.bLineSpacingRule = 0;
  base_paragraph.wBorders = 0;
  base_paragraph.wBorderWidth = 0;
  base_paragraph.wBorderSpace = 0;
  base_paragraph.wNumbering = 0;
  base_paragraph.dxStartIndent = 0;
  base_paragraph.dxOffset = 0;
  base_paragraph.wNumberingTab = 0;
  const auto reset_paragraph_format = [&](LONG begin, LONG end) {
    begin = std::clamp<LONG>(begin, 0, length);
    end = std::clamp<LONG>(end, begin, length);
    if (begin >= end) return;
    SendMessageW(view.editor, EM_SETSEL, begin, end);
    SendMessageW(view.editor, EM_SETPARAFORMAT, 0,
                 reinterpret_cast<LPARAM>(&base_paragraph));
  };
  if (view.native_tables_ready) {
    LONG plain_begin{};
    for (const auto& table : view.editor_snapshot.tables) {
      const LONG table_begin = static_cast<LONG>(table.native_begin);
      const LONG table_end = static_cast<LONG>(table.native_end);
      reset_paragraph_format(plain_begin, table_begin);
      plain_begin = std::max(plain_begin, table_end);
    }
    reset_paragraph_format(plain_begin, length);
  } else {
    reset_paragraph_format(0, length);
  }

  for (const auto& block : view.parse.blocks) {
    if (block.kind != BlockKind::BulletListItem &&
        block.kind != BlockKind::TaskListItem &&
        block.kind != BlockKind::OrderedListItem) continue;
    if (block.begin >= source.size() || block.end <= block.begin) continue;
    std::size_t indent{};
    while (block.begin + indent < block.end) {
      const wchar_t character = source[block.begin + indent];
      if (character == L' ') {
        ++indent;
      } else if (character == L'\t') {
        indent += 4;
      } else {
        break;
      }
    }
    const LONG native_begin = static_cast<LONG>(
        view.editor_snapshot.SourceToNative(block.begin));
    const LONG native_end = static_cast<LONG>(
        view.editor_snapshot.SourceToNative(block.end));
    if (native_begin >= length || native_end <= native_begin) continue;
    SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(native_begin),
                 static_cast<LPARAM>(std::min<LONG>(native_end, length)));
    PARAFORMAT2 list_paragraph{sizeof(list_paragraph)};
    list_paragraph.dwMask = PFM_STARTINDENT | PFM_OFFSET;
    const LONG nesting_dip = static_cast<LONG>((indent / 2) * 16);
    list_paragraph.dxStartIndent = static_cast<LONG>((28 + nesting_dip) * 15);
    list_paragraph.dxOffset = -16 * 15;
    if (block.kind == BlockKind::BulletListItem ||
        block.kind == BlockKind::TaskListItem) {
      const auto block_line_begin = SourceLineRange(source, block.begin).first;
      if (block_line_begin != active_source_line_begin) {
        list_paragraph.dwMask |= PFM_NUMBERING | PFM_NUMBERINGTAB;
        list_paragraph.wNumbering = PFN_BULLET;
        list_paragraph.wNumberingTab = 16 * 15;
      }
    }
    SendMessageW(view.editor, EM_SETPARAFORMAT, 0,
                 reinterpret_cast<LPARAM>(&list_paragraph));
  }

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
      if (span.kind == SpanKind::CodeFence) {
        format.dwMask |= CFM_HIDDEN;
        const bool intersects_active = span.begin < active_source_line_end &&
                                       span.end > active_source_line_begin;
        if (!intersects_active) format.dwEffects |= CFE_HIDDEN;
      }
    } else if (span.kind == SpanKind::Link) {
      format.dwMask |= CFM_LINK | CFM_UNDERLINE;
      format.dwEffects |= CFE_LINK | CFE_UNDERLINE;
      format.crTextColor = ThemeColor(settings_, L"link", dark ? RGB(78, 160, 255) : RGB(0, 102, 204));
    } else if (span.kind == SpanKind::OrderedListMarker) {
      format.crTextColor = foreground;
    } else if (span.kind == SpanKind::HeadingMarker || span.kind == SpanKind::EmphasisMarker ||
               span.kind == SpanKind::ListMarker) {
      const bool intersects_active = span.begin < active_source_line_end &&
                                     span.end > active_source_line_begin;
      if (!intersects_active) {
        format.dwMask |= CFM_HIDDEN;
        format.dwEffects |= CFE_HIDDEN;
      } else {
        format.crTextColor = ThemeColor(settings_, L"marker", dark ? RGB(150, 150, 150) : RGB(128, 128, 128));
      }
    }
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
  }
  const COLORREF table_body = ThemeColor(settings_, L"table_background",
                                          dark ? RGB(38, 42, 48) : RGB(244, 247, 250));
  const COLORREF table_header = ThemeColor(settings_, L"table_header_background",
                                            dark ? RGB(31, 40, 52) : RGB(231, 236, 242));
  const bool force_cell_failure = TestAutomationSilent() && view.force_table_cell_formatting_failure_for_test;
  view.force_table_cell_formatting_failure_for_test = false;
  if (force_cell_failure || !FormatRichEditTableCells(view.editor, view.editor_snapshot, table_header, table_body)) {
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    SchedulePresentation(view);
    RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
    SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
    InvalidateRect(view.editor, nullptr, FALSE);
    SetStatusText(L"表のセル装飾を更新できませんでした。本文と編集履歴は保持されています。");
    return;
  }
  for (const auto& table : view.parse.tables) {
    const std::size_t visible_columns = table.alignments.size();
    for (std::size_t row_index = 2; row_index < table.rows.size(); ++row_index) {
      const auto& row = table.rows[row_index];
      if (row.cells.size() <= visible_columns) continue;
      if (SourceLineRange(source, row.begin).first == active_source_line_begin) continue;
      for (std::size_t column = visible_columns; column < row.cells.size(); ++column) {
        const auto& cell = row.cells[column];
        const LONG cell_begin = static_cast<LONG>(
            view.editor_snapshot.SourceToNative(cell.begin));
        const LONG cell_end = static_cast<LONG>(
            view.editor_snapshot.SourceToNative(cell.end));
        if (cell_begin >= cell_end || cell_begin >= length) continue;
        SendMessageW(view.editor, EM_SETSEL, cell_begin, std::min(cell_end, length));
        CHARFORMAT2W hidden{sizeof(hidden)};
        hidden.dwMask = CFM_HIDDEN;
        hidden.dwEffects = CFE_HIDDEN;
        SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION,
                     reinterpret_cast<LPARAM>(&hidden));
      }
    }
  }
  RestoreSourceSelection(view.editor, view.editor_snapshot, selection);
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  InvalidateRect(view.editor, nullptr, TRUE);
  view.active_source_line_begin = active_source_line_begin;
  view.active_line = active_line;
  if (view.document.revision() == revision) {
    view.presentation_revision = revision;
    view.flat_source_fallback_pending = false;
    view.flat_source_fallback_retry_used = false;
    view.editor_projection_repair_attempts = 0;
  } else {
    view.presentation_revision = std::numeric_limits<std::uint64_t>::max();
    SchedulePresentation(view);
  }
}
void Application::InvalidateTableGrid(DocumentView& view) {
  if (!view.editor) return;
  if (view.native_tables_ready) {
    InvalidateRect(view.editor, nullptr, FALSE);
    return;
  }
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
  if (view.native_tables_ready || !IsMarkdownFile(view.document.path()) || view.parse.tables.empty() ||
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
    const int cell_padding = ScaleDip(window_, kTableCellPadding);
    std::vector<TableVisualRow> rows;
    for (std::size_t row_index{}; row_index < table.rows.size(); ++row_index) {
      if (row_index == 1) continue;
      const auto& row = table.rows[row_index];
      if (row.end < draw_begin || row.begin > draw_end) continue;
      rows.push_back(row);
    }
    std::vector<TableGridRow> row_geometry;
    row_geometry.reserve(rows.size());
    const std::size_t column_count = table.alignments.size();
    if (column_count == 0) continue;
    RECT formatting_rect{};
    SendMessageW(view.editor, EM_GETRECT, 0,
                 reinterpret_cast<LPARAM>(&formatting_rect));
    const int available_table_width = std::max(
        1, static_cast<int>(formatting_rect.right - formatting_rect.left));
    const int track_width_cap = std::max(
        1, available_table_width / static_cast<int>(column_count));
    const int minimum_cell_width = std::min(track_width_cap, ScaleDip(window_, MulDiv(
        static_cast<int>(settings_.font_size_pt) * kTableMinimumCellEmCount, 96, 72)));
    int shared_left = client.right;
    int shared_right = client.left;
    for (std::size_t row_index = 0; row_index < rows.size(); ++row_index) {
      const auto& row = rows[row_index];
      if (row.cells.empty()) continue;
      const std::size_t visible_cell_count = std::min(row.cells.size(), column_count);
      std::vector<POINT> starts(visible_cell_count);
      std::vector<POINT> ends(visible_cell_count);
      for (std::size_t cell_index = 0; cell_index < visible_cell_count; ++cell_index) {
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
        const auto& next_row = rows[row_index + 1];
        const auto next_source = next_row.cells.empty()
            ? next_row.begin : next_row.cells.front().begin;
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&next_start),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(next_source)));
        bottom = static_cast<int>(next_start.y) - 3;
      }
      if (bottom <= top) bottom = top + line_height + 1;
      std::vector<TableVisualCell> displayed_cells(
          row.cells.begin(), row.cells.begin() + static_cast<std::ptrdiff_t>(visible_cell_count));
      const auto virtual_anchor = displayed_cells.empty() ? row.begin : displayed_cells.back().end;
      displayed_cells.resize(column_count, TableVisualCell{virtual_anchor, virtual_anchor});
      row_geometry.push_back({row.begin, row.end, visible_cell_count,
                              std::move(displayed_cells), top, bottom});
      shared_left = std::min(shared_left, static_cast<int>(starts.front().x) - cell_padding);
      shared_right = std::max(shared_right, static_cast<int>(ends.back().x) + cell_padding);
    }
    if (row_geometry.empty()) continue;
    if (!table.rows.empty() && table.rows.front().cells.size() >= column_count) {
      const auto& header = table.rows.front();
      POINT header_start{};
      SendMessageW(view.editor, EM_POSFROMCHAR,
                   reinterpret_cast<WPARAM>(&header_start),
                   static_cast<LONG>(view.editor_snapshot.SourceToNative(
                       header.cells.front().begin)));
      shared_left = std::min(shared_left, static_cast<int>(header_start.x) - cell_padding);
      for (std::size_t column = 0; column < column_count; ++column) {
        POINT header_cell_start{};
        POINT header_end{};
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&header_cell_start),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(
                         header.cells[column].begin)));
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&header_end),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(
                         header.cells[column].end)));
        if (column + 1 == column_count) {
          shared_right = std::max(shared_right, static_cast<int>(header_end.x) + cell_padding);
          shared_right = std::max(
              shared_right, static_cast<int>(header_cell_start.x) +
                                minimum_cell_width - cell_padding);
        }
      }
    }
    const int left = std::clamp(shared_left, static_cast<int>(client.left),
                                static_cast<int>(client.right));
    if (left >= client.right) continue;
    const int right = std::clamp(std::max(shared_right, left + 8),
                                 left + 1, static_cast<int>(client.right));
    std::vector<int> shared_boundaries(column_count > 0 ? column_count - 1 : 0, left + 1);
    for (const auto& row_geometry_entry : row_geometry) {
      const auto& cells = row_geometry_entry.cells;
      if (cells.empty()) continue;
      std::vector<POINT> starts(cells.size());
      std::vector<POINT> ends(cells.size());
      for (std::size_t cell_index = 0; cell_index < cells.size(); ++cell_index) {
        const auto& cell = cells[cell_index];
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&starts[cell_index]),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.begin)));
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&ends[cell_index]),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(cell.end)));
      }
      for (std::size_t cell_index = 0;
           cell_index + 1 < ends.size() && cell_index < shared_boundaries.size();
           ++cell_index) {
        shared_boundaries[cell_index] = std::max(
            shared_boundaries[cell_index],
            std::max(static_cast<int>(ends[cell_index].x) + cell_padding,
                     static_cast<int>(starts[cell_index + 1].x) - cell_padding));
      }
    }
    if (!table.rows.empty() && table.rows.front().cells.size() >= column_count) {
      const auto& header = table.rows.front();
      for (std::size_t cell_index = 0;
           cell_index + 1 < column_count && cell_index < shared_boundaries.size();
           ++cell_index) {
        POINT next_header_start{};
        POINT header_end{};
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&next_header_start),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(
                         header.cells[cell_index + 1].begin)));
        SendMessageW(view.editor, EM_POSFROMCHAR,
                     reinterpret_cast<WPARAM>(&header_end),
                     static_cast<LONG>(view.editor_snapshot.SourceToNative(
                         header.cells[cell_index].end)));
        shared_boundaries[cell_index] = std::max(
            shared_boundaries[cell_index],
            std::max(static_cast<int>(header_end.x) + cell_padding,
                     static_cast<int>(next_header_start.x) - cell_padding));
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

std::optional<TableCellIntent> Application::HitTestTableCell(
    const DocumentView& view, POINT point) const {
  if (view.native_tables_ready || view.sync_due != 0 || view.presentation_due != 0 ||
      !IsMarkdownFile(view.document.path())) return std::nullopt;
  RECT client{};
  GetClientRect(view.editor, &client);
  const auto geometry = BuildTableGridGeometry(view, client);
  for (const auto& table : geometry) {
    if (point.x < table.left || point.x > table.right) continue;
    for (const auto& row : table.rows) {
      if (point.y < row.top || point.y > row.bottom || row.cells.empty()) continue;
      std::size_t cell_index{};
      while (cell_index < table.boundaries.size() && point.x >= table.boundaries[cell_index]) ++cell_index;
      if (cell_index >= row.cells.size()) continue;
      const bool virtual_cell = cell_index >= row.source_cell_count;
      if (virtual_cell) {
        return TableCellIntent{row.begin, cell_index, row.cells[cell_index].begin, true};
      }
      POINT hit = point;
      const auto native = static_cast<std::size_t>(std::max<LRESULT>(
          0, SendMessageW(view.editor, EM_CHARFROMPOS, 0, reinterpret_cast<LPARAM>(&hit))));
      const auto source = view.editor_snapshot.NativeToSource(native);
      return TableCellIntent{row.begin, cell_index,
          std::clamp(source, row.cells[cell_index].begin, row.cells[cell_index].end), false};
    }
  }
  return std::nullopt;
}

void Application::DrawTableGrid(DocumentView& view, HDC paint_dc, const RECT& clip) {
  if (view.native_tables_ready || !paint_dc || view.sync_due != 0 || view.presentation_due != 0 ||
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

bool Application::EditorText(HWND editor, EditorSnapshot& snapshot,
                             std::wstring& text, std::wstring* raw_native_text) const {
  std::wstring raw;
  if (!ReadRichEditNativeText(editor, raw)) return false;
  if (raw_native_text) *raw_native_text = raw;
  if (!snapshot.tables.empty()) {
    if (!RefreshRichEditTableCoordinates(snapshot, raw) ||
        !FlattenRichEditTableText(snapshot, raw, text)) return false;
  } else {
    text = raw;
  }

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
  std::vector<std::size_t> object_positions;
  if (rich_edit) {
    const LONG count = rich_edit->GetObjectCount();
    object_positions.reserve(static_cast<std::size_t>(std::max<LONG>(count, 0)));
    for (LONG index = 0; index < count; ++index) {
      REOBJECT object{sizeof(object)};
      if (FAILED(rich_edit->GetObject(index, &object, REO_GETOBJ_NO_INTERFACES)) ||
          object.cp < 0 || static_cast<std::size_t>(object.cp) >= text.size()) {
        rich_edit->Release();
        text.clear();
        return false;
      }
      text[static_cast<std::size_t>(object.cp)] = L'\uFFFC';
      object_positions.push_back(static_cast<std::size_t>(object.cp));
    }
    rich_edit->Release();
  }

  // Before the image renderer inserts an OLE object, RichEdit can expose its
  // projected U+FFFC placeholder as a plain space. Preserve the placeholder
  // only when the recorded native edit is outside the image's source range.
  const DocumentView* owner_view{};
  for (const auto& candidate : documents_) {
    if (candidate && candidate->editor == editor) {
      owner_view = candidate.get();
      break;
    }
  }
  if (owner_view && owner_view->pending_selection_before) {
    const auto selection = *owner_view->pending_selection_before;
    const std::size_t selection_begin = std::min(selection.anchor, selection.active);
    const std::size_t selection_end = std::max(selection.anchor, selection.active);
    for (const auto& collapsed : snapshot.collapsed) {
      int edit_side{};  // -1: before the image, +1: after it, 0: overlaps/ambiguous.
      if (selection_begin != selection_end) {
        if (selection_end <= collapsed.source_begin) edit_side = -1;
        else if (selection_begin >= collapsed.source_end) edit_side = 1;
      } else {
        const auto caret = selection_begin;
        switch (owner_view->pending_native_edit_kind) {
          case DocumentView::PendingNativeEditKind::Backspace:
            if (caret <= collapsed.source_begin) edit_side = -1;
            else if (caret > collapsed.source_end) edit_side = 1;
            break;
          case DocumentView::PendingNativeEditKind::Delete:
            if (caret < collapsed.source_begin) edit_side = -1;
            else if (caret >= collapsed.source_end) edit_side = 1;
            break;
          case DocumentView::PendingNativeEditKind::Insert:
            if (caret <= collapsed.source_begin) edit_side = -1;
            else if (caret >= collapsed.source_end) edit_side = 1;
            break;
          case DocumentView::PendingNativeEditKind::Replace:
          case DocumentView::PendingNativeEditKind::Unknown:
            if (size_delta >= 0) {
              if (caret <= collapsed.source_begin) edit_side = -1;
              else if (caret >= collapsed.source_end) edit_side = 1;
            } else {
              if (caret < collapsed.source_begin) edit_side = -1;
              else if (caret > collapsed.source_end) edit_side = 1;
            }
            break;
        }
      }
      if (edit_side == 0) continue;
      const auto rendered = std::ranges::find_if(
          owner_view->rendered_images, [&](const auto& image) {
            return image.source_begin == collapsed.source_begin &&
                   image.source_end == collapsed.source_end;
          });
      if (rendered != owner_view->rendered_images.end() && rendered->inserted) continue;
      const auto candidate = static_cast<std::ptrdiff_t>(collapsed.view) +
          (edit_side < 0 ? size_delta : 0);
      if (candidate < 0 || static_cast<std::size_t>(candidate) >= text.size()) continue;
      const auto position = static_cast<std::size_t>(candidate);
      if (text[position] != L' ' ||
          std::ranges::find(object_positions, position) != object_positions.end()) continue;
      text[position] = L'\uFFFC';
    }
  }
  return true;
}

void Application::UpdateStatus() {
  std::array<std::wstring, 4> segments{};
  segments[0] = workspace_.empty() ? L"MDLite" : workspace_.filename().wstring();
  if (git_status_workspace_ == workspace_ &&
      (git_panel_status_.state == GitPanelState::Ready ||
       git_panel_status_.state == GitPanelState::NoRemote) &&
      !git_panel_status_.branch.empty()) {
    segments[0] += L"    " + git_panel_status_.branch;
  }
  if (active_document_ < documents_.size()) {
    const auto& view = *documents_[active_document_];
    const auto& document = view.document;
    segments[1] = (document.dirty() || view.sync_due != 0) ? L"● 未保存" : L"✓ 保存済み";
    if (document.HasExternalChange()) segments[1] += L"  外部変更あり";
    std::size_t source_caret = CaptureSourceSelection(view.editor, view.editor_snapshot).active;
    source_caret = std::min(source_caret, document.text().size());
    std::size_t line = 1;
    std::size_t line_begin{};
    for (std::size_t index{}; index < source_caret; ++index) {
      if (document.text()[index] == L'\n') {
        ++line;
        line_begin = index + 1;
      }
    }
    segments[2] = L"行 " + std::to_wstring(line) + L"  列 " +
                  std::to_wstring(source_caret - line_begin + 1);
    segments[3] = EncodingLabel(document.encoding()) + L"    " +
                  LineEndingLabel(document.line_ending()) + L"    " +
                  (IsMarkdownFile(document.path()) ? L"Markdown" : L"Text");
  } else {
    segments[1] = workspace_.empty() ? L"Workspaceを開いてください" : L"準備完了";
    segments[3] = L"UTF-8    LF";
  }
  std::wstring title = L"MDLite";
  if (active_document_ < documents_.size()) {
    const auto& document = documents_[active_document_]->document;
    title += L" — " + (document.path().empty() ? std::wstring(L"無題") : document.path().filename().wstring());
    if (document.dirty()) title += L" *";
  }
  if (!workspace_.empty()) title += L" — " + workspace_.filename().wstring();
  if (ControlText(window_) != title) SetWindowTextW(window_, title.c_str());
  SetStatusSegments(std::move(segments));
}

void Application::ShowFindBar() {
  active_activity_ = 1;
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
  if (!documents_[active_document_]->editor) ActivateDocument(active_document_);
  if (active_document_ >= documents_.size() || !documents_[active_document_]->editor) return;
  const auto search = SearchQueryFromFindBar();
  if (search.text.empty()) {
    ShowFindBar();
    return;
  }
  auto& view = *documents_[active_document_];
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため検索を中止しました。再試行してください。");
    return;
  }
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため置換を中止しました。再試行してください。");
    return;
  }
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
    if (!ApplySourceTextWithUndo(view, std::move(output))) {
      SetStatusText(L"本文が同期中のため置換できません。再試行してください。");
      return;
    }
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
  if (!ApplySourceTextWithUndo(view, std::move(output), true,
                               SourceSelection{replaced_begin, replaced_end})) {
    SetStatusText(L"本文が同期中のため置換できません。再試行してください。");
    return;
  }
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
    if (view->ime_composing || !SyncDocumentFromEditor(*view)) {
      SetStatusText(L"開いている文書の入力内容を読み取れなかったためWorkspace検索を中止しました。再試行してください。");
      return;
    }
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
    if (payload->error.find(L"中止") == std::wstring::npos && !payload->error.empty()) {
      RecordDiagnosticSummary(L"Workspace検索でエラーを検出しました");
      MessageBoxW(window_, payload->error.c_str(), L"Workspace検索", MB_ICONWARNING);
    }
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
  if (!documents_[active_document_]->editor) ActivateDocument(active_document_);
  if (active_document_ >= documents_.size() || !documents_[active_document_]->editor) return;
  auto& view = *documents_[active_document_];
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため検索結果への移動を中止しました。再試行してください。");
    return;
  }
  const CHARRANGE selection{
      static_cast<LONG>(view.editor_snapshot.SourceToNative(match.begin)),
      static_cast<LONG>(view.editor_snapshot.SourceToNative(match.end))};
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(view.editor);
}

void Application::ReplaceWorkspaceFromFindBar() {
  if (workspace_.empty()) return;
  // The preview and closed-file transaction must be based on every open
  // editor's latest text.  Fail before either operation if any editor cannot
  // be read back.
  for (auto& view : documents_) {
    if (view->ime_composing || !SyncDocumentFromEditor(*view)) {
      SetStatusText(L"開いている文書の入力内容を読み取れなかったためWorkspace置換を中止しました。再試行してください。");
      return;
    }
  }
  ShowFindBar();
  const auto query = SearchQueryFromFindBar();
  const std::wstring replacement = ControlText(replace_edit_);
  if (query.text.empty()) { SetFocus(find_edit_); return; }
  std::map<std::filesystem::path, std::wstring> buffers;
  for (auto& view : documents_) {
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
  // The preview can take time.  Recheck open editors after confirmation and
  // before any closed-file write; if a buffer changed, the preview is stale.
  for (auto& view : documents_) {
    if (view->ime_composing || !SyncDocumentFromEditor(*view)) {
      SetStatusText(L"開いている文書の入力内容を読み取れなかったためWorkspace置換を中止しました。再試行してください。");
      return;
    }
  }
  for (const auto* item : buffer_plans) {
    const auto open = std::ranges::find_if(documents_, [&](const auto& view) {
      return _wcsicmp(view->document.path().c_str(), item->path.c_str()) == 0;
    });
    if (open == documents_.end() || (*open)->document.text() != item->before ||
        (*open)->ime_composing) {
      SetStatusText(L"置換preview後に開いている文書が変更されたためWorkspace置換を中止しました。もう一度previewしてください。");
      return;
    }
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
      if (!ApplySourceTextWithUndo(*(*open), item->after)) {
        result.conflicts.push_back(item->path);
        complete = false;
        continue;
      }
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

void Application::UpdateCalendarTooltip(std::optional<CalendarDate> date_value) {
  if (!calendar_tooltip_) return;
  if (!date_value || !IsValidCalendarDate(*date_value)) {
    SendMessageW(calendar_tooltip_, TTM_POP, 0, 0);
    return;
  }
  wchar_t text_date[32]{};
  swprintf_s(text_date, L"%04d-%02d-%02d", date_value->year, date_value->month, date_value->day);
  calendar_tooltip_text_ = text_date;
  if (const auto holiday = JapaneseHolidayName(date_value->year, date_value->month, date_value->day))
    calendar_tooltip_text_ += L"  " + std::wstring(*holiday);
  else if (!JapaneseHolidayYearSupported(date_value->year))
    calendar_tooltip_text_ += L"  祝日データ収録範囲外（不明）";
  calendar_tooltip_text_ += L"\n" + std::wstring(JapaneseHolidayDataVersion()) + L" / " +
                            std::to_wstring(JapaneseHolidayFirstYear()) + L"–" +
                            std::to_wstring(JapaneseHolidayLastYear());
  if (!holiday_data_status_.empty()) calendar_tooltip_text_ += L"\n" + holiday_data_status_;
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
                 L"祝日データはローカルファイルから取り込み、ここでは通信しません。", path) || path.empty()) return;
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
  holiday_data_status_ = cache_saved
      ? L"ローカルCSVを取込み済み（外部通信なし）"
      : L"ローカルCSVはメモリへ取込みましたがcache保存に失敗しました。";
  UpdateCalendarViewMarkers();
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
    holiday_data_status_ = L"cacheの祝日データを使用中（" + std::to_wstring(info.first_year) + L"–" +
                             std::to_wstring(info.last_year) + L"）";
  } else {
    holiday_data_status_ = L"祝日cacheを検証できません。内蔵データを使用中。";
  }
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
    auto& view = *documents_[active_document_];
    RestoreSourceSelection(view.editor, view.editor_snapshot,
                           SourceSelection{result.cursor, result.cursor});
  }
  PopulateWorkspaceTree();
}

void Application::ApplyTableAction(TableAction action) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  HWND editor = view.editor;
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため表の操作を中止しました。再試行してください。");
    return;
  }
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
  if (!ApplySourceTextWithUndo(view, edit.text, true,
                               SourceSelection{edit.selection, edit.selection})) {
    SetStatusText(L"本文が同期中のため表の操作を中止しました。再試行してください。");
    return;
  }
  const auto view_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(editor, EM_SETSEL, view_caret, view_caret);
}

void Application::MoveOutlineSection(std::size_t source_begin, std::size_t target_begin) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing ||
      source_begin == target_begin ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため見出し移動を中止しました。再試行してください。");
    return;
  }
  const std::wstring source = view.document.text();
  const auto edit = MoveHeadingSection(source, source_begin, target_begin);
  if (!edit.changed) return;

  if (!ApplySourceTextWithUndo(view, edit.text, true,
                               SourceSelection{edit.selection, edit.selection})) {
    SetStatusText(L"本文が同期中のため見出し移動を中止しました。再試行してください。");
    return;
  }
  const auto view_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(view.editor, EM_SETSEL, view_caret, view_caret);
}

bool Application::SaveRecovery(DocumentView& view, bool interactive) {
  if (!view.workspace_store || view.ime_composing) return false;
  if (!SyncDocumentFromEditor(view)) {
    const std::wstring error = L"入力内容を読み取れなかったため、復旧保存を中止しました。再試行してください。";
    SetStatusText(error);
    if (interactive) MessageBoxW(window_, error.c_str(), L"復旧保存できません", MB_ICONERROR);
    return false;
  }
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
  for (auto& view : documents_) {
    if (view->ime_composing || !SyncDocumentFromEditor(*view)) {
      SetStatusText(L"入力内容を読み取れなかったためWorkspace状態の保存を中止しました。再試行してください。");
      return;
    }
  }
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
    SessionDocument item;
    item.path = view->document.path();
    if (view->editor && IsWindow(view->editor)) {
      CaptureEditorViewState(*view);
    }
    item.selection_begin = view->suspended_selection.anchor;
    item.selection_end = view->suspended_selection.active;
    item.first_visible_source_offset = std::min(view->suspended_first_visible_source,
                                                 view->document.text().size());
    if (view->suspended_horizontal_left_edge_source)
      item.horizontal_left_edge_source_offset = std::min(
          *view->suspended_horizontal_left_edge_source, view->document.text().size());
    if (view->editor && IsWindow(view->editor)) {
      item.first_visible_line = static_cast<int>(
          SendMessageW(view->editor, EM_GETFIRSTVISIBLELINE, 0, 0));
    } else {
      const auto anchor = std::min(view->suspended_first_visible_source,
                                   view->document.text().size());
      item.first_visible_line = static_cast<int>(std::count(
          view->document.text().begin(), view->document.text().begin() +
              static_cast<std::ptrdiff_t>(anchor), L'\n'));
    }
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
  if (git_action_active_ || external_operation_active_) {
    SetStatusText(L"Git操作中はWorkspace Trustを変更できません。");
    return;
  }
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
  LayoutControls();
  RunGitStatus();
}

void Application::RunGitStatus() {
  if (git_action_active_) {
    git_status_refresh_pending_ = true;
    return;
  }
  ++git_status_generation_;
  git_status_workspace_ = workspace_;
  git_status_refresh_pending_ = false;
  if (git_status_cancellation_event_) SetEvent(git_status_cancellation_event_);

  git_panel_status_ = {};
  if (workspace_.empty()) {
    git_panel_status_.state = GitPanelState::NoRepository;
    git_panel_status_.error = L"Workspace未選択です。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
    return;
  }
  if (!IsWorkspaceTrusted(workspace_)) {
    git_panel_status_.state = GitPanelState::NoRepository;
    git_panel_status_.error = L"Gitを使うにはWorkspaceを明示的に信頼してください。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
    return;
  }
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) {
    git_panel_status_.state = GitPanelState::NoGit;
    git_panel_status_.error = L"git.exeが見つかりません。PATHまたはGit for Windowsを確認してください。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
    return;
  }

  git_panel_status_.state = GitPanelState::OperationInProgress;
  if (git_status_worker_.joinable()) {
    // Do not join a cancellable Git process on the UI thread. Its completion
    // message will discard the stale generation and start the latest request.
    git_status_refresh_pending_ = true;
  } else {
    if (SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr) == 0) {
      git_panel_status_ = {};
      git_panel_status_.state = GitPanelState::Error;
      git_panel_status_.error = L"Git status workerを監視するtimerを開始できません。";
      RenderGitPanel();
      SetStatusText(git_panel_status_.error);
      return;
    }
    StartGitStatusRefresh();
  }
  RenderGitPanel();
  if (git_panel_status_.state == GitPanelState::OperationInProgress)
    SetStatusText(L"Git statusを更新しています…");
  else if (!git_panel_status_.error.empty())
    SetStatusText(git_panel_status_.error);
}

void Application::StartGitStatusRefresh() {
  if (git_status_worker_.joinable() || workspace_.empty() ||
      git_status_workspace_ != workspace_ || !IsWorkspaceTrusted(workspace_)) return;
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) {
    git_panel_status_ = {};
    git_panel_status_.state = GitPanelState::NoGit;
    git_panel_status_.error = L"git.exeが見つかりません。PATHまたはGit for Windowsを確認してください。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
    return;
  }
  git_status_cancellation_event_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!git_status_cancellation_event_) {
    git_panel_status_ = {};
    git_panel_status_.state = GitPanelState::Error;
    git_panel_status_.error = L"Git status用のキャンセルeventを作成できません。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
    return;
  }

  const auto generation = git_status_generation_;
  const auto workspace = workspace_;
  const auto cancellation = git_status_cancellation_event_;
  const auto target_window = window_;
  git_status_delivery_failed_.store(false, std::memory_order_release);
  try {
    git_status_worker_ = std::jthread(
        [this, generation, workspace, git_path, cancellation, target_window] {
          auto message = std::unique_ptr<GitStatusCompleteMessage>(
              new (std::nothrow) GitStatusCompleteMessage{});
          if (!message) {
            git_status_delivery_failed_.store(true, std::memory_order_release);
            return;
          }
          message->generation = generation;
          message->workspace = workspace;
          try {
            message->status = GitPanelModel(git_path, workspace).Refresh(cancellation);
          } catch (...) {
            message->status.state = GitPanelState::Error;
            message->status.error = L"Git status更新中に予期しないエラーが発生しました。";
          }
          if (!PostMessageW(target_window, kGitStatusCompleteMessage, 0,
                            reinterpret_cast<LPARAM>(message.get()))) {
            git_status_delivery_failed_.store(true, std::memory_order_release);
            return;
          }
          message.release();
        });
  } catch (...) {
    CloseHandle(git_status_cancellation_event_);
    git_status_cancellation_event_ = nullptr;
    git_panel_status_ = {};
    git_panel_status_.state = GitPanelState::Error;
    git_panel_status_.error = L"Git status workerを開始できません。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
  }
}

void Application::CompleteGitStatus(void* payload) {
  std::unique_ptr<GitStatusCompleteMessage> message(
      reinterpret_cast<GitStatusCompleteMessage*>(payload));
  if (!message) return;
  if (git_status_worker_.joinable()) git_status_worker_.join();
  if (git_status_cancellation_event_) {
    CloseHandle(git_status_cancellation_event_);
    git_status_cancellation_event_ = nullptr;
  }

  const bool current = IsGitResultCurrent(
      message->generation, message->workspace, git_status_generation_, workspace_,
      git_status_workspace_);
  if (current) {
    git_panel_status_ = std::move(message->status);
    RenderGitPanel();
    SetStatusText(git_panel_status_.error.empty() ? L"Git statusを更新しました。"
                                                  : git_panel_status_.error);
  }
  if (git_status_refresh_pending_) {
    git_status_refresh_pending_ = false;
    StartGitStatusRefresh();
  }
}

void Application::StopGitStatusWorker() {
  ++git_status_generation_;
  git_status_refresh_pending_ = false;
  if (git_status_cancellation_event_) SetEvent(git_status_cancellation_event_);
  if (git_status_worker_.joinable()) git_status_worker_.join();
  git_status_delivery_failed_.store(false, std::memory_order_release);
  if (git_status_cancellation_event_) {
    CloseHandle(git_status_cancellation_event_);
    git_status_cancellation_event_ = nullptr;
  }
}

void Application::CompleteGitStatusDeliveryFailure() {
  if (!git_status_delivery_failed_.exchange(false, std::memory_order_acq_rel)) return;
  if (git_status_worker_.joinable()) git_status_worker_.join();
  if (git_status_cancellation_event_) {
    CloseHandle(git_status_cancellation_event_);
    git_status_cancellation_event_ = nullptr;
  }
  if (git_status_refresh_pending_) {
    git_status_refresh_pending_ = false;
    StartGitStatusRefresh();
    return;
  }
  if (workspace_.empty() || workspace_ != git_status_workspace_ ||
      git_panel_status_.state != GitPanelState::OperationInProgress) return;
  git_panel_status_ = {};
  git_panel_status_.state = GitPanelState::Error;
  git_panel_status_.error = L"Git status結果を画面へ通知できませんでした。再試行してください。";
  RenderGitPanel();
  SetStatusText(git_panel_status_.error);
}

void Application::StartGitAction(int command, std::filesystem::path git_path,
                                 GitActionRequest request) {
  if (git_action_active_ || git_action_worker_.joinable()) {
    SetStatusText(L"別のGit操作を実行中です。");
    return;
  }
  git_action_cancellation_event_ = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!git_action_cancellation_event_) {
    SetWindowTextW(git_diff_view_, L"Git操作用のキャンセルeventを作成できません。");
    SetStatusText(L"Git操作用のキャンセルeventを作成できません。");
    return;
  }
  if (SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr) == 0) {
    CloseHandle(git_action_cancellation_event_);
    git_action_cancellation_event_ = nullptr;
    SetWindowTextW(git_diff_view_, L"Git操作workerを監視するtimerを開始できません。");
    SetStatusText(L"Git操作workerを監視するtimerを開始できません。");
    return;
  }

  const auto generation = ++git_action_generation_;
  const auto workspace = workspace_;
  const auto cancellation = git_action_cancellation_event_;
  const auto target_window = window_;
  git_action_workspace_ = workspace;
  git_action_active_ = true;
  git_action_delivery_failed_.store(false, std::memory_order_release);
  git_panel_status_ = GitPanelModel::OperationInProgress(git_panel_status_);
  RenderGitPanel();
  SetStatusText(command == kGitStageAll ? L"選択ファイルをstageしています…" :
                                         L"選択ファイルをunstageしています…");
  try {
    git_action_worker_ = std::jthread(
        [this, generation, workspace, git_path = std::move(git_path), request = std::move(request),
         cancellation, target_window, command]() mutable {
          auto message = std::unique_ptr<GitActionCompleteMessage>(
              new (std::nothrow) GitActionCompleteMessage{});
          if (!message) {
            git_action_delivery_failed_.store(true, std::memory_order_release);
            return;
          }
          message->generation = generation;
          message->workspace = workspace;
          message->command = command;
          try {
            const DWORD delay = TestGitActionDelay();
            if (delay != 0 && WaitForSingleObject(cancellation, delay) == WAIT_OBJECT_0) {
              message->result.state = GitPanelState::Error;
              message->result.error = L"Git操作はWorkspace終了時に中止されました。";
            } else {
              message->result = GitPanelModel(git_path, workspace).Execute(request, cancellation);
            }
          } catch (...) {
            message->result.state = GitPanelState::Error;
            message->result.error = L"Git操作中に予期しないエラーが発生しました。";
          }
          if (!PostMessageW(target_window, kGitActionCompleteMessage, 0,
                            reinterpret_cast<LPARAM>(message.get()))) {
            git_action_delivery_failed_.store(true, std::memory_order_release);
            return;
          }
          message.release();
        });
  } catch (...) {
    CloseHandle(git_action_cancellation_event_);
    git_action_cancellation_event_ = nullptr;
    git_action_active_ = false;
    git_action_workspace_.clear();
    git_panel_status_ = {};
    git_panel_status_.state = GitPanelState::Error;
    git_panel_status_.error = L"Git操作workerを開始できません。";
    RenderGitPanel();
    SetStatusText(git_panel_status_.error);
  }
}

void Application::CompleteGitAction(void* payload) {
  std::unique_ptr<GitActionCompleteMessage> message(
      reinterpret_cast<GitActionCompleteMessage*>(payload));
  if (!message) return;
  if (git_action_worker_.joinable()) git_action_worker_.join();
  if (git_action_cancellation_event_) {
    CloseHandle(git_action_cancellation_event_);
    git_action_cancellation_event_ = nullptr;
  }
  const bool current = IsGitResultCurrent(
      message->generation, message->workspace, git_action_generation_, workspace_,
      git_action_workspace_);
  git_action_active_ = false;
  git_action_workspace_.clear();
  if (!current) {
    if (git_status_refresh_pending_) RunGitStatus();
    return;
  }

  const bool succeeded = message->result.state == GitPanelState::Ready;
  const std::wstring error = message->result.error.empty()
      ? L"選択ファイルのGit操作に失敗しました。" : message->result.error;
  if (!succeeded && git_diff_view_) SetWindowTextW(git_diff_view_, error.c_str());
  RunGitStatus();
  SetStatusText(succeeded
      ? (message->command == kGitStageAll ? L"選択ファイルをstageしました。"
                                         : L"選択ファイルをunstageしました。")
      : error);
}

void Application::StopGitActionWorker() {
  ++git_action_generation_;
  if (git_action_cancellation_event_) SetEvent(git_action_cancellation_event_);
  if (git_action_worker_.joinable()) git_action_worker_.join();
  git_action_delivery_failed_.store(false, std::memory_order_release);
  if (git_action_cancellation_event_) {
    CloseHandle(git_action_cancellation_event_);
    git_action_cancellation_event_ = nullptr;
  }
  git_action_active_ = false;
  git_action_workspace_.clear();
}

void Application::CompleteGitActionDeliveryFailure() {
  if (!git_action_delivery_failed_.exchange(false, std::memory_order_acq_rel)) return;
  if (git_action_worker_.joinable()) git_action_worker_.join();
  if (git_action_cancellation_event_) {
    CloseHandle(git_action_cancellation_event_);
    git_action_cancellation_event_ = nullptr;
  }
  const bool current = git_action_active_ && git_action_workspace_ == workspace_ &&
      IsWorkspaceTrusted(workspace_);
  git_action_active_ = false;
  git_action_workspace_.clear();
  if (!current) return;
  git_panel_status_ = {};
  git_panel_status_.state = GitPanelState::Error;
  git_panel_status_.error = L"Git操作結果を画面へ通知できませんでした。statusを再確認してください。";
  const auto error = git_panel_status_.error;
  if (git_diff_view_) SetWindowTextW(git_diff_view_, error.c_str());
  RunGitStatus();
  SetStatusText(error);
}

void Application::RenderGitPanel() {
  if (!git_panel_) return;
  ListView_DeleteAllItems(git_files_);
  const bool has_workspace = !workspace_.empty();
  const bool trusted = has_workspace && IsWorkspaceTrusted(workspace_);
  if (!has_workspace) {
    SetWindowTextW(git_panel_, L"Workspace未選択\r\nWorkspaceを開くとGit状態を表示します。");
    if (git_diff_view_) SetWindowTextW(git_diff_view_, L"Workspaceを開くとGit状態を表示します。");
  } else if (!trusted) {
    SetWindowTextW(git_panel_, L"Workspaceは未信頼です\r\nGitコマンドの実行には、このWorkspaceの信頼が必要です。内容を確認してから有効にできます。");
    if (git_diff_view_) SetWindowTextW(git_diff_view_, L"Gitを使うにはWorkspaceを信頼してください。");
  } else {
    const wchar_t* state = L"不明";
    switch (git_panel_status_.state) {
      case GitPanelState::NoGit: state = L"Git未導入"; break;
      case GitPanelState::NoRepository: state = L"repositoryなし"; break;
      case GitPanelState::Ready: state = L"準備完了"; break;
      case GitPanelState::NoRemote: state = L"remoteなし"; break;
      case GitPanelState::OperationInProgress: state = L"操作中"; break;
      case GitPanelState::Error: state = L"エラー"; break;
    }
    std::wstring summary = std::wstring(state) + L"\r\n";
    summary += git_panel_status_.branch.empty() ? state : git_panel_status_.branch;
    if (git_panel_status_.detached_head) summary += L" (detached)";
    if (git_panel_status_.state == GitPanelState::NoRemote) summary += L"  ·  remoteなし";
    if (git_panel_status_.has_staged_changes || git_panel_status_.has_unstaged_changes ||
        git_panel_status_.has_untracked_files) {
      summary += L"   staged " + std::to_wstring(std::ranges::count_if(
          git_panel_status_.files, [](const auto& file) { return file.staged; }));
      summary += L"   unstaged " + std::to_wstring(std::ranges::count_if(
          git_panel_status_.files, [](const auto& file) { return file.unstaged; }));
      summary += L"   untracked " + std::to_wstring(std::ranges::count_if(
          git_panel_status_.files, [](const auto& file) { return file.untracked; }));
    } else if (git_panel_status_.state == GitPanelState::Ready ||
               git_panel_status_.state == GitPanelState::NoRemote) {
      summary += L"\r\n変更なし（working tree clean）";
    }
    if (!git_panel_status_.error.empty()) {
      summary += L"\r\n";
      for (const wchar_t character : git_panel_status_.error) {
        if (character == L'\n' && summary.back() != L'\r') summary.push_back(L'\r');
        summary.push_back(character);
      }
    }
    SetWindowTextW(git_panel_, summary.c_str());
    for (std::size_t index{}; index < git_panel_status_.files.size(); ++index) {
      const auto& file = git_panel_status_.files[index];
      std::wstring status;
      if (file.conflicted) status = L"!";
      else if (file.untracked) status = L"?";
      else {
        if (file.staged) status.push_back(file.index_status);
        if (file.unstaged) status.push_back(file.worktree_status);
      }
      LVITEMW item{};
      item.mask = LVIF_TEXT | LVIF_PARAM;
      item.iItem = static_cast<int>(index);
      item.pszText = status.data();
      item.lParam = static_cast<LPARAM>(index);
      const int row = ListView_InsertItem(git_files_, &item);
      if (row >= 0) {
        const auto path = file.path.generic_wstring();
        ListView_SetItemText(git_files_, row, 1, const_cast<wchar_t*>(path.c_str()));
      }
    }
    if (!git_panel_status_.error.empty() && git_panel_status_.files.empty() && git_diff_view_)
      SetWindowTextW(git_diff_view_, git_panel_status_.error.c_str());
  }
  if (git_refresh_) EnableWindow(git_refresh_, trusted);
  UpdateGitPanelActions();
  LayoutControls();
}

void Application::UpdateGitPanelActions() {
  if (!git_files_) return;
  const bool trusted = !workspace_.empty() && IsWorkspaceTrusted(workspace_);
  const bool can_use_git = trusted &&
      (git_panel_status_.state == GitPanelState::Ready ||
       git_panel_status_.state == GitPanelState::NoRemote);
  bool stageable{};
  bool unstageable{};
  for (int row = ListView_GetNextItem(git_files_, -1, LVNI_SELECTED); row >= 0;
       row = ListView_GetNextItem(git_files_, row, LVNI_SELECTED)) {
    LVITEMW item{};
    item.mask = LVIF_PARAM;
    item.iItem = row;
    if (!ListView_GetItem(git_files_, &item) || item.lParam < 0) continue;
    const auto index = static_cast<std::size_t>(item.lParam);
    if (index >= git_panel_status_.files.size()) continue;
    const auto& file = git_panel_status_.files[index];
    stageable = stageable || file.unstaged || file.untracked || file.conflicted;
    unstageable = unstageable || file.staged;
  }
  if (git_stage_) EnableWindow(git_stage_, can_use_git && stageable);
  if (git_unstage_) EnableWindow(git_unstage_, can_use_git && unstageable);
  if (git_diff_) EnableWindow(git_diff_, can_use_git && ListView_GetSelectedCount(git_files_) > 0);
  if (git_commit_) EnableWindow(git_commit_, can_use_git && git_panel_status_.has_staged_changes);
}

std::vector<std::filesystem::path> Application::SelectedGitPaths() const {
  std::vector<std::filesystem::path> paths;
  if (!git_files_) return paths;
  for (int row = ListView_GetNextItem(git_files_, -1, LVNI_SELECTED); row >= 0;
       row = ListView_GetNextItem(git_files_, row, LVNI_SELECTED)) {
    LVITEMW item{};
    item.mask = LVIF_PARAM;
    item.iItem = row;
    if (!ListView_GetItem(git_files_, &item) || item.lParam < 0) continue;
    const auto index = static_cast<std::size_t>(item.lParam);
    if (index < git_panel_status_.files.size()) paths.push_back(git_panel_status_.files[index].path);
  }
  return paths;
}

void Application::ShowSelectedGitDiff() {
  if (git_action_active_ || external_operation_active_) {
    SetStatusText(L"別のGit操作を実行中です。");
    return;
  }
  if (workspace_.empty() || !IsWorkspaceTrusted(workspace_)) {
    SetStatusText(L"Git diffには信頼済みWorkspaceが必要です。");
    return;
  }
  const auto paths = SelectedGitPaths();
  if (paths.empty()) {
    SetWindowTextW(git_diff_view_, L"差分を表示するファイルを選択してください。");
    return;
  }
  const auto git_path = ResolveGitExecutable();
  if (git_path.empty()) {
    SetWindowTextW(git_diff_view_, L"git.exeが見つかりません。");
    return;
  }
  const auto workspace = workspace_;
  std::wstring output;
  std::wstring error;
  constexpr std::size_t kGitDiffDisplayLimit = 512 * 1024;
  bool output_truncated{};
  bool diff_succeeded{};
  SetWindowTextW(git_diff_view_, L"選択ファイルの差分を取得しています…");
  SetExternalOperationActive(true);
  const bool completed = RunCancellableTask(
      window_, L"選択ファイルの差分", L"選択したGit差分を読み込んでいます",
      [&](HANDLE cancellation) {
        GitPanelModel model(git_path, workspace);
        for (const auto& path : paths) {
          if (WaitForSingleObject(cancellation, 0) == WAIT_OBJECT_0) {
            error = L"選択ファイルの差分をキャンセルしました。";
            return false;
          }
          const auto diff = model.DiffFile(path, cancellation);
          if (!diff.succeeded) {
            error = diff.error.empty() ? L"選択ファイルの差分を取得できません。" : diff.error;
            return false;
          }
          const std::size_t remaining = kGitDiffDisplayLimit - output.size();
          std::size_t copied = std::min(remaining, diff.diff.size());
          if (copied < diff.diff.size() && copied > 0 &&
              diff.diff[copied - 1] >= 0xd800 && diff.diff[copied - 1] <= 0xdbff)
            --copied;
          output.append(diff.diff, 0, copied);
          output_truncated = output_truncated || diff.truncated || copied < diff.diff.size();
        }
        diff_succeeded = true;
        return true;
      }, error);
  SetExternalOperationActive(false);
  if (!completed || !diff_succeeded) {
    if (error.empty()) error = L"選択ファイルの差分を取得できませんでした。";
    SetWindowTextW(git_diff_view_, error.c_str());
    SetStatusText(error);
    return;
  }
  if (output.empty()) output = L"選択したファイルに差分はありません。";
  if (output_truncated) output += L"\r\n(出力上限で省略しました)";
  SetWindowTextW(git_diff_view_, output.c_str());
  SetStatusText(L"選択ファイルの差分を更新しました。");
}

void Application::RunGitAction(int command) {
  if (git_action_active_ || external_operation_active_) {
    SetStatusText(L"別のGit操作を実行中です。");
    return;
  }
  if (command == kGitDiff) {
    ShowSelectedGitDiff();
    return;
  }
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
  if (command == kGitStageAll || command == kGitUnstageAll) {
    const auto paths = SelectedGitPaths();
    if (paths.empty()) {
      SetStatusText(L"StageまたはUnstageするファイルを選択してください。");
      return;
    }
    GitActionRequest request;
    request.operation = command == kGitStageAll ? GitOperation::Stage : GitOperation::Unstage;
    request.paths = paths;
    StartGitAction(command, git_path, std::move(request));
    return;
  }
  std::vector<std::wstring> arguments{L"-C", workspace_.wstring()};
  std::wstring value;
  bool save_first{};
  std::wstring action;
  switch (command) {
    case kGitCommit:
      if (git_commit_edit_) {
        std::wstring message(static_cast<std::size_t>(std::max(0, GetWindowTextLengthW(git_commit_edit_))) + 1,
                             L'\0');
        const int copied = GetWindowTextW(git_commit_edit_, message.data(), static_cast<int>(message.size()));
        message.resize(static_cast<std::size_t>(std::max(0, copied)));
        value = std::move(message);
      }
      if (value.empty() &&
          !PromptText(window_, instance_, L"Git commit", L"コミットメッセージ", value)) return;
      if (value.empty()) return;
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
  if (command != kGitCommit) {
    const auto question = action + L"を実行しますか？\n\n作業ディレクトリ: " + workspace_.wstring() +
                          L"\nコマンド: " + command_text;
    if (MessageBoxW(window_, question.c_str(), L"Git 明示操作",
                    MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
  }
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
  if (result.exit_code == 0 && command == kGitCommit && git_commit_edit_)
    SetWindowTextW(git_commit_edit_, L"");
  RunGitStatus();
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため競合位置への移動を中止しました。再試行してください。");
    return;
  }
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
  if (git_action_active_ || external_operation_active_) {
    SetStatusText(L"別のGit操作を実行中です。");
    return;
  }
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
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため競合解決を中止しました。再試行してください。");
    return;
  }
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
  if (!ApplySourceTextWithUndo(view, edit.text, true,
                               SourceSelection{edit.selection, edit.selection})) {
    SetStatusText(L"本文が同期中のため競合解決を中止しました。再試行してください。");
    return;
  }
  const auto conflict_caret = view.editor_snapshot.SourceToNative(edit.selection);
  SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(conflict_caret),
               static_cast<LPARAM>(conflict_caret));
  const auto remaining = ParseConflictBlocks(view.document.text()).size();
  MessageBoxW(window_, (L"競合箇所を1つ解決しました。残存marker: " + std::to_wstring(remaining) +
                        L"\n保存後に『解決済みにする』を明示実行してください。").c_str(),
              L"Git競合", MB_ICONINFORMATION);
}

void Application::MarkGitConflictResolved() {
  if (git_action_active_ || external_operation_active_) {
    SetStatusText(L"別のGit操作を実行中です。");
    return;
  }
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  std::vector<std::filesystem::path> files;
  std::wstring error;
  if (!QueryGitConflicts(files, error)) { MessageBoxW(window_, error.c_str(), L"Git競合", MB_ICONERROR); return; }
  auto& view = *documents_[active_document_];
  if (!std::ranges::any_of(files, [&](const auto& file) { return _wcsicmp(file.c_str(), view.document.path().c_str()) == 0; })) {
    MessageBoxW(window_, L"現在の文書はGitの未マージ一覧にありません。", L"Git競合", MB_ICONINFORMATION);
    return;
  }
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため解決済み操作を中止しました。再試行してください。");
    return;
  }
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
  UpdateCalendarViewMarkers();
  if (const auto selected = CalendarView_GetSelection(calendar_))
    UpdateCalendarDetails(*selected);
  const ULONGLONG now = GetTickCount64();
  for (auto& view : documents_) {
    if (view->document.dirty())
      view->autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
  }
}

void Application::UpdateCalendarViewTheme() {
  if (!calendar_) return;
  CalendarViewTheme theme;
  theme.background = theme_surface_;
  theme.heading_text = theme_foreground_;
  theme.day_text = theme_foreground_;
  theme.sunday_text = high_contrast_ ? theme_foreground_ : dark_theme_ ? RGB(218, 139, 145) : RGB(170, 64, 70);
  theme.saturday_text = theme_accent_;
  theme.holiday_text = high_contrast_ ? theme_foreground_ : dark_theme_ ? RGB(218, 139, 145) : RGB(170, 64, 70);
  theme.adjacent_month_text = theme_muted_;
  theme.hover_background = theme_surface_alt_;
  theme.selected_background = theme_accent_;
  theme.selected_text = theme_selected_text_;
  theme.today_outline = theme_accent_;
  theme.focus_outline = theme_foreground_;
  theme.navigation_hover_background = theme_surface_alt_;
  theme.holiday_marker = theme.holiday_text;
  theme.daily_marker = dark_theme_ ? RGB(110, 181, 145) : RGB(43, 127, 86);
  CalendarView_SetTheme(calendar_, theme);

  SYSTEMTIME local{};
  GetLocalTime(&local);
  const CalendarDate today{static_cast<int>(local.wYear), static_cast<int>(local.wMonth),
                           static_cast<int>(local.wDay)};
  CalendarView_SetToday(calendar_, today);
  if (!CalendarView_GetDisplayedMonth(calendar_)) CalendarView_SetDisplayedMonth(calendar_, today);
  if (!CalendarView_GetSelection(calendar_)) CalendarView_SetSelection(calendar_, today);
}

void Application::UpdateCalendarViewMarkers() {
  if (!calendar_) return;
  std::vector<CalendarViewDateMarker> markers;
  const auto month = CalendarView_GetDisplayedMonth(calendar_);
  if (!month) return;
  std::optional<ProfileDefinition> daily_profile;
  if (!workspace_.empty() && workspace_store_) {
    std::vector<ProfileDefinition> profiles;
    std::wstring profile_error;
    if (ResolveProfiles(CommonProfilesPath(), workspace_store_->metadata_root() / L"profiles.toml",
                        profiles, profile_error)) {
      const auto found = std::ranges::find_if(profiles, [](const auto& profile) {
        return profile.id == L"daily";
      });
      if (found != profiles.end()) daily_profile = *found;
    }
  }
  for (const auto& value : GetCalendarViewMonthDates(*month)) {
    if (!value) continue;
    CalendarViewMarkerFlags flags = CalendarViewMarkerFlags::None;
    if (JapaneseHolidayName(value->year, value->month, value->day))
      flags = flags | CalendarViewMarkerFlags::Holiday;
    if (daily_profile) {
      SYSTEMTIME date{};
      date.wYear = static_cast<WORD>(value->year);
      date.wMonth = static_cast<WORD>(value->month);
      date.wDay = static_cast<WORD>(value->day);
      std::wstring path_error;
      const auto path = PreviewProfilePath(workspace_, *daily_profile, date, path_error);
      std::error_code exists_error;
      if (path && std::filesystem::exists(*path, exists_error) && !exists_error)
        flags = flags | CalendarViewMarkerFlags::Daily;
    }
    if (flags != CalendarViewMarkerFlags::None) markers.push_back({*value, flags});
  }
  CalendarView_SetMarkers(calendar_, markers);
}

void Application::ApplySettings() {
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  COLORREF background = ThemeColor(settings_, L"background", dark ? RGB(31, 31, 31) : RGB(255, 255, 255));
  COLORREF foreground = ThemeColor(settings_, L"foreground", dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
  theme_surface_ = ThemeColor(settings_, L"surface", dark ? RGB(42, 46, 54) : RGB(248, 250, 253));
  theme_surface_alt_ = ThemeColor(settings_, L"surface_alt", dark ? RGB(50, 55, 64) : RGB(241, 245, 249));
  theme_editor_ = ThemeColor(settings_, L"editor_background", dark ? RGB(28, 31, 36) : RGB(252, 253, 255));
  theme_input_ = ThemeColor(settings_, L"input_background", dark ? RGB(36, 40, 47) : RGB(255, 255, 255));
  theme_border_ = ThemeColor(settings_, L"border", dark ? RGB(83, 92, 105) : RGB(210, 218, 228));
  theme_muted_ = ThemeColor(settings_, L"muted", dark ? RGB(170, 180, 194) : RGB(92, 104, 120));
  theme_accent_ = ThemeColor(settings_, L"accent", dark ? RGB(108, 170, 255) : RGB(56, 112, 194));
  HIGHCONTRASTW contrast{sizeof(contrast)};
  high_contrast_ = SystemParametersInfoW(SPI_GETHIGHCONTRAST, sizeof(contrast), &contrast, 0) &&
                   (contrast.dwFlags & HCF_HIGHCONTRASTON) != 0;
  theme_selected_text_ = high_contrast_ ? GetSysColor(COLOR_HIGHLIGHTTEXT) : RGB(255, 255, 255);
  if (high_contrast_) {
    background = theme_surface_ = theme_editor_ = theme_input_ = GetSysColor(COLOR_WINDOW);
    foreground = theme_muted_ = GetSysColor(COLOR_WINDOWTEXT);
    theme_border_ = GetSysColor(COLOR_WINDOWTEXT);
    theme_surface_alt_ = GetSysColor(COLOR_BTNFACE);
    theme_accent_ = GetSysColor(COLOR_HIGHLIGHT);
  }
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
  HFONT ui_replacement = CreateFontW(-ScaleDip(window_, 14), 0, 0, 0, FW_NORMAL,
      FALSE, FALSE, FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
      CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE, L"Segoe UI");
  if (ui_replacement) {
    for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_,
                         replace_edit_, find_workspace_, replace_workspace_, find_case_, find_regex_,
                         find_word_, find_include_glob_, find_exclude_glob_, replace_one_,
                         replace_document_, find_results_, calendar_, git_panel_, calendar_details_,
                         calendar_summary_, calendar_daily_, calendar_details_toggle_,
                         git_files_, git_diff_view_, git_commit_edit_, brand_, command_search_,
                         tab_new_, tab_close_, activity_buttons_[0], activity_buttons_[1],
                         activity_buttons_[2], activity_buttons_[3], activity_buttons_[4],
                         git_refresh_, git_stage_, git_unstage_, git_diff_, git_commit_, git_trust_,
                         panel_headers_[0], panel_headers_[1], panel_headers_[2], panel_headers_[3]})
      if (control) SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(ui_replacement), TRUE);
    HFONT previous = ui_font_;
    ui_font_ = ui_replacement;
    if (previous) DeleteObject(previous);
  }
  if (replacement)
    for (const auto& view : documents_)
      if (view->editor) SendMessageW(view->editor, WM_SETFONT,
                                     reinterpret_cast<WPARAM>(replacement), TRUE);
  TreeView_SetBkColor(workspace_tree_, theme_surface_);
  TreeView_SetTextColor(workspace_tree_, foreground);
  TreeView_SetBkColor(outline_, theme_surface_);
  TreeView_SetTextColor(outline_, foreground);
  ListView_SetBkColor(find_results_, theme_surface_);
  ListView_SetTextBkColor(find_results_, theme_surface_);
  ListView_SetTextColor(find_results_, foreground);
  ListView_SetBkColor(git_files_, theme_surface_);
  ListView_SetTextBkColor(git_files_, theme_surface_);
  ListView_SetTextColor(git_files_, foreground);
  UpdateCalendarViewTheme();
  UpdateCalendarViewMarkers();
  RenderGitPanel();
  for (auto& view : documents_) {
    if (!view->editor) {
      view->presentation_revision = std::numeric_limits<std::uint64_t>::max();
      continue;
    }
    {
      ScopedEditorChangeSuppression suppression(suppress_editor_change_);
      PresentationUndoGuard guard(view->editor);
      if (!guard) continue;
      const auto selection = CaptureSourceSelection(view->editor, view->editor_snapshot);
      SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, theme_editor_);
      CHARFORMAT2W format{};
      format.cbSize = sizeof(format);
      format.dwMask = CFM_COLOR | CFM_FACE | CFM_SIZE;
      format.crTextColor = foreground;
      format.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
      wcsncpy_s(format.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
      SendMessageW(view->editor, EM_SETSEL, 0, -1);
      SendMessageW(view->editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
      RestoreSourceSelection(view->editor, view->editor_snapshot, selection);
    }
    ApplyMarkdownPresentation(*view, true);
  }
  if (replacement) {
    HFONT previous = editor_font_;
    editor_font_ = replacement;
    if (previous) DeleteObject(previous);
  }
  ApplyChromeTheme();
  LayoutControls();
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
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>(VK_UP),
                               static_cast<WORD>(kViewResizeFocusedTaller)});
  accelerators.push_back(ACCEL{static_cast<BYTE>(FCONTROL | FALT), static_cast<WORD>(VK_DOWN),
                               static_cast<WORD>(kViewResizeFocusedShorter)});
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
  SettingsFileSnapshot common_snapshot, workspace_snapshot;
  std::wstring error;
  if (!CaptureSettingsFile(common_path, common_snapshot, error) ||
      (!workspace_path.empty() && !CaptureSettingsFile(workspace_path, workspace_snapshot, error)) ||
      !ParseSettingsSnapshot(scope == L"common" ? common_snapshot : workspace_snapshot, layer, error)) {
    MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return;
  }
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
      {L"既定メモWorkspaceの絶対path（inherit可）",
       layer.default_memo_workspace ? layer.default_memo_workspace->wstring() : L"inherit"},
      {L"カスタム色（1行1件: name=#RRGGBB。削除は行を消す）", colors, NativeFormFieldKind::Multiline},
      {L"キー割当て（1行1件: command=shortcut。noneで解除）", bindings, NativeFormFieldKind::Multiline},
  };
  bool settings_saved{};
  const auto apply_settings = [&]() -> bool {
    scope = fields[0].value;
    std::ranges::transform(scope, scope.begin(), towlower);
    if (scope == L"reset-common" || scope == L"reset-workspace") {
      const auto reset_target = scope == L"reset-common" ? common_path : workspace_path;
      if (reset_target.empty()) {
        MessageBoxW(window_, L"Workspaceが開かれていません。", L"設定", MB_ICONWARNING); return false;
      }
      if (MessageBoxW(window_, (reset_target.wstring() + L"\nの上書きを解除して継承へ戻しますか？").c_str(),
                      L"設定を既定へ戻す", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return false;
      const auto& reset_snapshot = scope == L"reset-common" ? common_snapshot : workspace_snapshot;
      if (!CheckSettingsSnapshot(reset_snapshot, error)) {
        MessageBoxW(window_, error.c_str(), L"設定の競合", MB_ICONWARNING); return false;
      }
      std::error_code remove_error;
      std::filesystem::remove(reset_target, remove_error);
      if (remove_error) {
        MessageBoxW(window_, L"設定上書きを削除できません。", L"設定", MB_ICONERROR); return false;
      }
      LoadAndApplySettings();
      return true;
    }
    if ((scope == L"workspace" && workspace_path.empty()) || (scope != L"common" && scope != L"workspace")) {
      MessageBoxW(window_, L"保存先の編集範囲が不正か、Workspaceが開かれていません。", L"設定", MB_ICONWARNING);
      return false;
    }
    // Each attempt starts from its captured scope, including retries after a
    // conflict, so unknown fields cannot leak from a previously selected layer.
    if (!ParseSettingsSnapshot(scope == L"common" ? common_snapshot : workspace_snapshot, layer, error)) {
      MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return false;
    }
    auto lower = [](std::wstring value) { std::ranges::transform(value, value.begin(), towlower); return value; };
    const auto auto_save = lower(fields[1].value);
    if (auto_save == L"inherit") layer.auto_save.reset();
    else if (auto_save == L"on") layer.auto_save = true;
    else if (auto_save == L"off") layer.auto_save = false;
    else { MessageBoxW(window_, L"自動保存の値が不正です。", L"設定", MB_ICONWARNING); return false; }
    const auto delay = lower(fields[2].value);
    if (delay == L"inherit") layer.auto_save_delay_ms.reset();
    else {
      try { std::size_t consumed{}; const auto parsed = std::stoul(delay, &consumed);
        if (consumed != delay.size()) throw std::invalid_argument("trailing characters");
        layer.auto_save_delay_ms = static_cast<unsigned>(parsed);
      } catch (...) { MessageBoxW(window_, L"自動保存待機時間が数値ではありません。", L"設定", MB_ICONWARNING); return false; }
    }
    const auto theme = lower(fields[3].value);
    if (theme == L"inherit") layer.theme.reset();
    else {
      const auto parsed = ParseTheme(theme);
      if (!parsed) { MessageBoxW(window_, L"テーマの値が不正です。", L"設定", MB_ICONWARNING); return false; }
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
      } catch (...) { MessageBoxW(window_, L"フォントサイズが数値ではありません。", L"設定", MB_ICONWARNING); return false; }
    }
    if (scope == L"common") {
      const auto memo = fields[6].value;
      if (lower(memo) == L"inherit" || memo.empty()) layer.default_memo_workspace.reset();
      else layer.default_memo_workspace = std::filesystem::path(memo);
    }
    layer.colors.clear();
    {
      std::wistringstream stream(fields[7].value);
      std::wstring line;
      while (std::getline(stream, line)) {
        if (!line.empty() && line.back() == L'\r') line.pop_back();
        if (line.empty()) continue;
        const auto equals = line.find(L'=');
        if (equals == std::wstring::npos) { MessageBoxW(window_, L"カスタム色はname=#RRGGBB形式で入力してください。", L"設定", MB_ICONWARNING); return false; }
        const auto name = line.substr(0, equals);
        const auto value = line.substr(equals + 1);
        if (value != L"inherit") layer.colors[name] = value;
      }
    }
    layer.keybindings.clear();
    {
      std::wistringstream stream(fields[8].value);
      std::wstring line;
      while (std::getline(stream, line)) {
        if (!line.empty() && line.back() == L'\r') line.pop_back();
        if (line.empty()) continue;
        const auto equals = line.find(L'=');
        if (equals == std::wstring::npos) { MessageBoxW(window_, L"キー割当てはcommand=shortcut形式で入力してください。", L"設定", MB_ICONWARNING); return false; }
        layer.keybindings[line.substr(0, equals)] = line.substr(equals + 1);
      }
    }
    if (!ValidateSettingsLayer(layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONWARNING); return false; }
    SettingsLayer common, workspace_layer;
    if (!LoadSettingsLayer(common_path, common, error) ||
        (!workspace_path.empty() && !LoadSettingsLayer(workspace_path, workspace_layer, error))) {
      MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return false;
    }
    if (scope == L"common") common = layer; else workspace_layer = layer;
    auto effective_bindings = DefaultSettingsLayer().keybindings;
    for (const auto& item : common.keybindings) effective_bindings[item.first] = item.second;
    for (const auto& item : workspace_layer.keybindings) effective_bindings[item.first] = item.second;
    if (!ValidateKeybindingConflicts(effective_bindings, error)) {
      MessageBoxW(window_, error.c_str(), L"キー割当て競合", MB_ICONWARNING); return false;
    }
    auto& selected_snapshot = scope == L"common" ? common_snapshot : workspace_snapshot;
    if (!SaveSettingsSnapshot(selected_snapshot, layer, error)) {
      MessageBoxW(window_, error.c_str(), L"設定の競合・保存エラー", MB_ICONERROR); return false;
    }
    LoadAndApplySettings();
    settings_saved = true;
    return true;
  };
  const bool applied = RunNativeForm(window_, instance_, L"MDLite 設定", fields,
                     NativeDialogTheme{theme_background_, theme_surface_, theme_input_,
                                       theme_foreground_, theme_muted_, theme_accent_,
                                       theme_border_}, apply_settings);
  if (applied && settings_saved && !TestAutomationSilent())
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
  if (!RunNativeForm(window_, instance_, L"作成プロファイル", fields,
                     NativeDialogTheme{theme_background_, theme_surface_, theme_input_,
                                       theme_foreground_, theme_muted_, theme_accent_,
                                       theme_border_})) return;
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
  const bool has_markdown = has_document && IsMarkdownFile(documents_[active_document_]->document.path());
  const std::array entries{
      Entry{L"ファイル: 新規無題文書", kFileNew, true, L""},
      Entry{L"ファイル: 開く", kFileOpen, true, L""},
      Entry{L"ファイル: Quick Open", kFileQuickOpen, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: 保存", kFileSave, has_document, L"文書が開かれていません"},
      Entry{L"編集: 検索", kEditFind, has_document, L"文書が開かれていません"},
      Entry{L"編集: Workspace検索", kEditFindWorkspace, has_workspace, L"Workspaceが未選択です"},
      Entry{L"表示: コンパクト表示", kViewCompact, has_document, L"文書が開かれていません"},
      Entry{L"カレンダー: 祝日CSVをローカル取込み", kCalendarImportHolidays, true, L""},
      Entry{L"表示: Workspace設定", kViewSettings, has_workspace, L"Workspaceが未選択です"},
      Entry{L"表示: 診断情報", kViewDiagnostics, true, L""},
      Entry{L"Git: Status / Branches", kGitStatus, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: ブランチ切替", kGitBranchSwitch, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Merge", kGitMerge, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Pull (fast-forward only)", kGitPull, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"画像: 表示幅480 DIP", kImageWidth480, has_document, L"Markdown文書が必要です"},
      Entry{L"画像: Storageへupload", kImageUpload, trusted && has_document, L"信頼済みWorkspaceと文書が必要です"},
      Entry{L"ファイル: Workspaceを開く", kFileOpenWorkspace, true, L""},
      Entry{L"ファイル: 別名で保存", kFileSaveAs, has_document, L"文書が開かれていません"},
      Entry{L"ファイル: ディスクから再読込み", kFileReload, has_document, L"文書が開かれていません"},
      Entry{L"ファイル: ディスク内容と比較", kFileCompare, has_document, L"文書が開かれていません"},
      Entry{L"ファイル: 今日のDairyを開く", kFileDaily, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: Meetingノートを作成", kFileMeeting, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: Memoを作成", kFileMemo, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: プロファイルから作成", kFileProfile, has_workspace, L"Workspaceが未選択です"},
      Entry{L"ファイル: タブを閉じる", kFileClose, has_document, L"文書が開かれていません"},
      Entry{L"ファイル: 終了", kFileExit, true, L""},
      Entry{L"Workspace: 新しいファイル", kWorkspaceNewFile, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 新しいフォルダー", kWorkspaceNewFolder, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 選択項目をコピー", kWorkspaceCopy, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: コピー項目を貼り付け", kWorkspacePaste, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 名前変更・移動", kWorkspaceRenameMove, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 選択項目を削除", kWorkspaceDelete, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: パスをコピー", kWorkspaceCopyPath, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: Explorerで表示", kWorkspaceShowExplorer, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 信頼する", kWorkspaceTrust, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Workspace: 信頼を解除", kWorkspaceUntrust, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"編集: Workspaceを置換", kEditReplaceWorkspace, has_workspace, L"Workspaceが未選択です"},
      Entry{L"表示: 選択日のDailyを開く", kCalendarOpenSelected, has_workspace, L"Workspaceが未選択です"},
      Entry{L"カレンダー: 今日へ移動", kCalendarGoToToday, true, L""},
      Entry{L"表示: Calendarを表示/非表示", kViewCalendar, true, L""},
      Entry{L"表示: Explorerを折り畳む/表示", kViewWorkspacePane, true, L""},
      Entry{L"表示: Outlineを折り畳む/表示", kViewOutlinePane, true, L""},
      Entry{L"表示: Gitを表示/非表示", kViewGitPane, true, L""},
      Entry{L"表示: パネル配置を初期化", kViewResetPanels, true, L""},
      Entry{L"表示: フォーカス中のパネルを狭くする", kViewResizeFocusedNarrow, true, L""},
      Entry{L"表示: フォーカス中のパネルを広くする", kViewResizeFocusedWide, true, L""},
      Entry{L"表示: フォーカス中のパネルを小さくする", kViewResizeFocusedShorter, true, L""},
      Entry{L"表示: フォーカス中のパネルを縦に広くする", kViewResizeFocusedTaller, true, L""},
      Entry{L"表: 上に行を追加", kTableRowBefore, has_markdown, L"Markdown文書が必要です"},
      Entry{L"表: 下に行を追加", kTableRowAfter, has_markdown, L"Markdown文書が必要です"},
      Entry{L"表: 行を削除", kTableRowDelete, has_markdown, L"Markdown文書が必要です"},
      Entry{L"表: 左に列を追加", kTableColumnBefore, has_markdown, L"Markdown文書が必要です"},
      Entry{L"表: 右に列を追加", kTableColumnAfter, has_markdown, L"Markdown文書が必要です"},
      Entry{L"表: 列を削除", kTableColumnDelete, has_markdown, L"Markdown文書が必要です"},
      Entry{L"画像: 表示幅320 DIP", kImageWidth320, has_markdown, L"Markdown文書が必要です"},
      Entry{L"画像: 表示幅640 DIP", kImageWidth640, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 差分を表示", kGitDiff, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: 選択ファイルをStage", kGitStageAll, trusted, L"信頼済みWorkspaceと選択ファイルが必要です"},
      Entry{L"Git: 選択ファイルをUnstage", kGitUnstageAll, trusted, L"信頼済みWorkspaceと選択ファイルが必要です"},
      Entry{L"Git: コミット", kGitCommit, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: ブランチを作成して切替", kGitBranchCreate, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: マージを中止", kGitMergeAbort, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: 未解決競合を表示", kGitConflicts, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: 前の競合箇所", kGitConflictPrevious, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 次の競合箇所", kGitConflictNext, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 競合で現在側を採用", kGitConflictCurrent, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 競合で相手側を採用", kGitConflictIncoming, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 競合で両方を採用", kGitConflictBoth, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: 現在のファイルを解決済みにする", kGitConflictResolved, has_markdown, L"Markdown文書が必要です"},
      Entry{L"Git: Fetch", kGitFetch, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"Git: Push", kGitPush, trusted, L"信頼済みWorkspaceが必要です"},
      Entry{L"表示: Compact windowを閉じる/開く", kViewCompact, has_document, L"文書が開かれていません"},
      Entry{L"表示: 設定ファイルを編集", kViewSettingsFiles, true, L""},
      Entry{L"表示: 作成プロファイルを管理", kViewProfiles, has_workspace, L"Workspaceが未選択です"},
      Entry{L"表示: フォーカス中のパネルを左上へ移動", kViewMoveFocusedLeftTop, true, L""},
      Entry{L"表示: フォーカス中のパネルを左下へ移動", kViewMoveFocusedLeftBottom, true, L""},
      Entry{L"表示: フォーカス中のパネルを右上へ移動", kViewMoveFocusedRightTop, true, L""},
      Entry{L"表示: フォーカス中のパネルを右下へ移動", kViewMoveFocusedRightBottom, true, L""},
      Entry{L"表示: Explorerを左上へ移動", kViewMoveExplorerLeftTop, true, L""},
      Entry{L"表示: Explorerを左下へ移動", kViewMoveExplorerLeftBottom, true, L""},
      Entry{L"表示: Explorerを右上へ移動", kViewMoveExplorerRightTop, true, L""},
      Entry{L"表示: Explorerを右下へ移動", kViewMoveExplorerRightBottom, true, L""},
      Entry{L"表示: Calendarを左上へ移動", kViewMoveCalendarLeftTop, true, L""},
      Entry{L"表示: Calendarを右上へ移動", kViewMoveCalendarRightTop, true, L""},
      Entry{L"表示: Calendarを右下へ移動", kViewMoveCalendarRightBottom, true, L""},
      Entry{L"表示: Outlineを左上へ移動", kViewMoveOutlineLeftTop, true, L""},
      Entry{L"表示: Outlineを左下へ移動", kViewMoveOutlineLeftBottom, true, L""},
      Entry{L"表示: Outlineを右上へ移動", kViewMoveOutlineRightTop, true, L""},
      Entry{L"表示: Outlineを右下へ移動", kViewMoveOutlineRightBottom, true, L""},
      Entry{L"表示: Gitを左上へ移動", kViewMoveGitLeftTop, true, L""},
      Entry{L"表示: Gitを左下へ移動", kViewMoveGitLeftBottom, true, L""},
      Entry{L"表示: Gitを右上へ移動", kViewMoveGitRightTop, true, L""},
      Entry{L"Git: ブランチを作成して切替", kGitBranchCreate, trusted, L"信頼済みWorkspaceが必要です"},
  };
  std::vector<NativePickerItem> items;
  items.reserve(entries.size());
  const std::array bindings{
      std::pair{static_cast<int>(kFileNew), std::wstring_view(L"file.new")},
      std::pair{static_cast<int>(kFileOpen), std::wstring_view(L"file.open")},
      std::pair{static_cast<int>(kFileSave), std::wstring_view(L"file.save")},
      std::pair{static_cast<int>(kFileQuickOpen), std::wstring_view(L"file.quickOpen")},
      std::pair{static_cast<int>(kFileClose), std::wstring_view(L"file.close")},
      std::pair{static_cast<int>(kEditFind), std::wstring_view(L"edit.find")},
      std::pair{static_cast<int>(kEditFindNext), std::wstring_view(L"edit.findNext")},
      std::pair{static_cast<int>(kViewCommandPalette), std::wstring_view(L"view.commandPalette")},
  };
  for (const auto& entry : entries) {
    std::wstring label = entry.name;
    const auto binding = std::ranges::find_if(bindings, [&](const auto& item) {
      return item.first == entry.command;
    });
    if (binding != bindings.end()) {
      const auto shortcut = settings_.keybindings.find(std::wstring(binding->second));
      if (shortcut != settings_.keybindings.end() && !shortcut->second.empty())
        label += L"  (" + shortcut->second + L")";
    }
    if (!entry.enabled) label += L"（実行不可: " + std::wstring(entry.reason) + L"）";
    items.push_back(NativePickerItem{std::move(label)});
  }
  DocumentView* editing = active_document_ < documents_.size()
      ? documents_[active_document_].get() : nullptr;
  std::optional<SourceSelection> saved_selection;
  if (editing) {
    if (editing->ime_composing || !SyncDocumentFromEditor(*editing)) {
      SetStatusText(L"入力内容を読み取れなかったためコマンドパレットを開けません。再試行してください。");
      return;
    }
    saved_selection = CaptureSourceSelection(editing->editor, editing->editor_snapshot);
  }
  std::size_t selected{};
  const bool accepted = RunNativePicker(window_, instance_, L"MDLite コマンドパレット",
      L"コマンド名の一部を入力し、実行する項目を選択してください。", items,
      NativeDialogTheme{theme_background_, theme_surface_, theme_input_, theme_foreground_,
                        theme_muted_, theme_accent_, theme_border_}, selected) &&
      selected < entries.size();
  if (editing && saved_selection &&
      std::ranges::any_of(documents_, [&](const auto& view) { return view.get() == editing; })) {
    RestoreSourceSelection(editing->editor, editing->editor_snapshot, *saved_selection);
    if (!accepted) SetFocus(editing->editor);
  }
  if (!accepted) return;
  const auto& match = entries[selected];
  if (!match.enabled) {
    MessageBoxW(window_, match.reason, L"このコマンドは実行できません", MB_ICONWARNING);
    return;
  }
  SendMessageW(window_, WM_COMMAND, MAKEWPARAM(match.command, 0), 0);
}

void Application::RecordDiagnosticSummary(std::wstring_view summary) {
  constexpr std::size_t kMaxEntries = 64;
  constexpr std::size_t kMaxEntryLength = 256;
  std::wstring safe_summary;
  safe_summary.reserve(std::min(summary.size(), kMaxEntryLength));
  for (const wchar_t character : summary) {
    if (safe_summary.size() == kMaxEntryLength) break;
    safe_summary.push_back(character == L'\r' || character == L'\n' ||
                                   character < 0x20 || character == 0x7f
                               ? L' ' : character);
  }
  if (safe_summary.empty()) return;
  if (summary.size() > kMaxEntryLength) safe_summary.back() = L'…';
  if (recent_diagnostic_summaries_.size() == kMaxEntries)
    recent_diagnostic_summaries_.erase(recent_diagnostic_summaries_.begin());
  // Callers pass fixed summaries only; never pass document text, credentials,
  // user queries, or workspace paths into this in-memory ring.
  recent_diagnostic_summaries_.push_back(std::move(safe_summary));
}

void Application::ShowDiagnostics() {
  const auto provider = [](void* context, DiagnosticsSnapshot& snapshot) -> bool {
    auto* application = static_cast<Application*>(context);
    if (application == nullptr) return false;
    application->RecordDiagnosticSummary(L"診断スナップショットを取得しました");

    PROCESS_MEMORY_COUNTERS_EX memory{};
    memory.cb = static_cast<DWORD>(sizeof(memory));
    if (GetProcessMemoryInfo(
            GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&memory),
            static_cast<DWORD>(sizeof(memory)))) {
      snapshot.memory_metrics_available = true;
      snapshot.working_set_bytes = static_cast<std::uint64_t>(memory.WorkingSetSize);
      snapshot.private_bytes = static_cast<std::uint64_t>(memory.PrivateUsage);
      snapshot.peak_working_set_bytes =
          static_cast<std::uint64_t>(memory.PeakWorkingSetSize);
    }

    snapshot.open_document_count = application->documents_.size();
    if (application->workspace_search_started_) {
      snapshot.search_state = L"Workspace検索中: " +
          std::to_wstring(application->workspace_search_results_.size()) + L"件";
    } else if (application->find_bar_ && IsWindowVisible(application->find_bar_)) {
      snapshot.search_state = L"検索パネル表示中";
    } else {
      snapshot.search_state = L"待機中";
    }

    if (application->active_document_ < application->documents_.size()) {
      snapshot.active_document_encoding = EncodingLabel(
          application->documents_[application->active_document_]->document.encoding());
    } else {
      snapshot.active_document_encoding = L"なし";
    }

    snapshot.app_version = L"MDLite " MDLITE_VERSION;
    snapshot.application_update_status = L"更新元未設定（確認・通信なし）";
    snapshot.dependency_update_status = L"libwebp固定版。URL・SHA-256を確認して手動更新";
    const auto webp_version = static_cast<unsigned int>(WebPGetDecoderVersion());
    snapshot.dependency_versions.push_back({
        L"libwebp", std::to_wstring((webp_version >> 16) & 0xffU) + L"." +
                        std::to_wstring((webp_version >> 8) & 0xffU) + L"." +
                        std::to_wstring(webp_version & 0xffU)});
    snapshot.recent_summaries = application->recent_diagnostic_summaries_;
    return true;
  };

  if (!ShowDiagnosticsView(window_, provider, this))
    SetStatusText(L"診断画面を開けませんでした。");
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
  if (!SyncDocumentFromEditor(view)) {
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    SetStatusText(L"入力内容を読み取れなかったため画像貼り付けを中止しました。再試行してください。");
    return true;
  }
  if (!OpenClipboard(window_)) {
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    return true;
  }
  HBITMAP bitmap = static_cast<HBITMAP>(GetClipboardData(CF_BITMAP));
  if (bitmap == nullptr) {
    CloseClipboard();
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    MessageBoxW(window_, L"クリップボード画像を読み取れません。本文は変更していません。",
                L"画像の貼り付け", MB_ICONWARNING);
    return true;
  }
  const auto virtual_target = view.pending_virtual_table_cell;
  if (virtual_target) {
    const auto probe = InsertTextIntoMissingTableCell(
        view.document.text(), virtual_target->row_begin, virtual_target->column, L"x");
    view.pending_virtual_table_cell.reset();
    view.pending_table_high_surrogate = 0;
    if (!probe.changed) {
      CloseClipboard();
      SetStatusText(L"クリップボード画像を選択した空セルへ貼り付けられませんでした。");
      return true;
    }
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
  if (virtual_target) {
    const auto edit = InsertTextIntoMissingTableCell(
        view.document.text(), virtual_target->row_begin, virtual_target->column, markup);
    if (!edit.changed) {
      std::filesystem::remove(path, filesystem_error);
      SetStatusText(L"クリップボード画像のMarkdownを空セルへ挿入できませんでした。画像ファイルを削除しました。");
      return true;
    }
    if (!ApplySourceTextWithUndo(view, edit.text, true,
                                 SourceSelection{edit.selection, edit.selection}, std::nullopt,
                                 virtual_target)) {
      std::filesystem::remove(path, filesystem_error);
      SetStatusText(L"本文が同期中のため画像貼り付けを中止しました。作成した画像ファイルを削除しました。再試行してください。");
      return true;
    }
  } else {
    const SourceSelection selection_before = CaptureSourceSelection(view.editor, view.editor_snapshot);
    std::wstring source_text = view.document.text();
    const std::size_t begin = std::min({selection_before.anchor, selection_before.active,
                                        source_text.size()});
    const std::size_t end = std::min(std::max(selection_before.anchor, selection_before.active),
                                      source_text.size());
    source_text.replace(begin, end - begin, markup);
    const std::size_t caret_after = begin + markup.size();
    if (!ApplySourceTextWithUndo(view, std::move(source_text), true,
                                 SourceSelection{caret_after, caret_after}, selection_before)) {
      std::filesystem::remove(path, filesystem_error);
      SetStatusText(L"本文が同期中のため画像貼り付けを中止しました。作成した画像ファイルを削除しました。再試行してください。");
      return true;
    }
  }
  PopulateWorkspaceTree();
  return true;
}

void Application::ResizeImageAtCaret(unsigned width_dip) {
  if (active_document_ >= documents_.size() || documents_[active_document_]->ime_composing) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため画像サイズ変更を中止しました。再試行してください。");
    return;
  }
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const auto source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto parsed = ParseMarkdown(view.document.text());
  const auto* image = FindImageAtSourcePosition(parsed, source_position);
  if (!image) {
    MessageBoxW(window_, L"カーソルをMarkdown画像の上へ移動してください。", L"画像サイズ",
                MB_ICONINFORMATION);
    return;
  }
  std::wstring source = view.document.text();
  const std::wstring_view original_markup =
      std::wstring_view(source).substr(image->begin, image->end - image->begin);
  std::wstring replacement;
  if (original_markup.starts_with(L"<img")) {
    auto resized_html = ResizeHtmlImageWidth(original_markup, width_dip);
    if (!resized_html) {
      SetStatusText(L"画像のHTML属性を安全に読み取れないためサイズ変更を中止しました。本文は変更していません。");
      return;
    }
    replacement = std::move(*resized_html);
  } else {
    replacement = ImageHtml(image->alternate_text, image->target, width_dip);
  }
  const SourceSelection selection_before = CaptureSourceSelection(view.editor, view.editor_snapshot);
  source.replace(image->begin, image->end - image->begin, replacement);
  const std::size_t caret_after = image->begin + replacement.size();
  if (!ApplySourceTextWithUndo(view, std::move(source), true,
                               SourceSelection{caret_after, caret_after}, selection_before)) {
    SetStatusText(L"本文が同期中のため画像サイズ変更を中止しました。再試行してください。");
  }
}

void Application::UploadImageAtCaret() {
  if (active_document_ >= documents_.size() || !workspace_store_ ||
      documents_[active_document_]->ime_composing) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  if (!SyncDocumentFromEditor(view)) {
    SetStatusText(L"入力内容を読み取れなかったため画像uploadを中止しました。再試行してください。");
    return;
  }
  if (!IsWorkspaceTrusted(workspace_)) {
    MessageBoxW(window_, L"未信頼Workspaceではstorage adapterを実行しません。", L"画像upload", MB_ICONWARNING);
    return;
  }
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_position = view.editor_snapshot.NativeToSource(selection.cpMin);
  const auto parsed = ParseMarkdown(view.document.text());
  const auto* image = FindImageAtSourcePosition(parsed, source_position);
  if (!image) {
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
  const auto updated_source = ReplaceImageReferenceTarget(
      view.document.text(), *image, uploaded.reference);
  if (!updated_source) {
    SetStatusText(L"画像はuploadしましたが、元の参照先を特定できないため本文は変更していません。");
    return;
  }
  if (!ApplySourceTextWithUndo(view, std::move(*updated_source)))
    SetStatusText(L"画像はuploadしましたが、本文が同期中のため参照先を更新できませんでした。本文を確認して再試行してください。");
}

void Application::OpenLinkAtSourcePosition(DocumentView& view, std::size_t source_position, bool activate) {
  if (!SyncDocumentFromEditor(view)) {
    if (activate)
      SetStatusText(L"入力内容を読み取れなかったためリンクを開けません。再試行してください。");
    return;
  }
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
  PanelLayout resized = panel_layout_;
  std::wstring error;
  if (!resized.Resize(focused_panel_, width, panel->height, error)) {
    SetStatusText(error.empty() ? L"パネル寸法を変更できません。" : error);
    return;
  }
  const PanelId partner = focused_panel_ == PanelId::Explorer ? PanelId::Calendar :
      focused_panel_ == PanelId::Calendar ? PanelId::Explorer :
      focused_panel_ == PanelId::Outline ? PanelId::Git : PanelId::Outline;
  if (auto* sibling = resized.Find(partner)) {
    if (!resized.Resize(partner, width, sibling->height, error)) {
      SetStatusText(error.empty() ? L"パネル寸法を変更できません。" : error);
      return;
    }
  }
  panel_layout_ = std::move(resized);
  SavePanelLayout();
  LayoutControls();
  SetStatusText(std::wstring(PanelName(focused_panel_)) + L"の寸法を変更しました。");
}

void Application::ResizeFocusedPanelHeight(double delta) {
  auto* panel = panel_layout_.Find(focused_panel_);
  if (!panel || !std::isfinite(delta)) return;
  std::wstring error;
  if (!panel_layout_.Resize(focused_panel_, panel->width, panel->height + delta, error)) {
    SetStatusText(error.empty() ? L"パネル高さを変更できません。" : error);
    return;
  }
  SavePanelLayout();
  LayoutControls();
  SetStatusText(std::wstring(PanelName(focused_panel_)) + L"の高さを変更しました。");
}

bool Application::OpenCalendarDetailAtOffset(std::size_t offset) {
  const auto target = std::ranges::find_if(calendar_detail_targets_, [&](const auto& item) {
    return offset >= item.begin && offset < item.end;
  });
  if (target == calendar_detail_targets_.end()) return false;
  std::error_code error;
  const auto root = std::filesystem::weakly_canonical(workspace_, error);
  if (error) {
    SetStatusText(L"Workspace pathを確認できないため、一覧の文書を開けません。");
    return true;
  }
  const auto path = std::filesystem::weakly_canonical(target->path, error);
  if (error || !IsPathWithin(root, path) ||
      !std::filesystem::is_regular_file(path, error) || error || !IsTextFile(path)) {
    SetStatusText(L"一覧の文書がWorkspace内にありません。一覧を更新してください。");
    return true;
  }
  OpenDocument(path);
  return true;
}

void Application::UpdateCalendarDetails(const SYSTEMTIME& date) {
  if (!calendar_details_) return;
  calendar_detail_targets_.clear();
  const CalendarDate selected{static_cast<int>(date.wYear), static_cast<int>(date.wMonth),
                              static_cast<int>(date.wDay)};
  std::wstring output = L"選択日: " + std::to_wstring(selected.year) + L"-" +
      (selected.month < 10 ? L"0" : L"") + std::to_wstring(selected.month) + L"-" +
      (selected.day < 10 ? L"0" : L"") + std::to_wstring(selected.day);
  std::wstring count = L"文書数: 未取得";
  const auto publish = [&] {
    SetWindowTextW(calendar_details_, output.c_str());
    const std::wstring summary = output.substr(0, output.find(L'\n')) + L"\n" + count;
    SetWindowTextW(calendar_summary_, summary.c_str());
    LayoutControls();
  };
  if (const auto holiday = JapaneseHolidayName(selected.year, selected.month, selected.day))
    output += L"\n祝日: " + std::wstring(*holiday);
  else if (!JapaneseHolidayYearSupported(selected.year))
    output += L"\n祝日: データ収録範囲外（不明）";
  if (workspace_.empty() || !workspace_store_) {
    output += L"\nテキスト一覧: Workspace未選択";
    publish();
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
    publish();
    return;
  }

  const auto details = BuildCalendarDayDetails(workspace_, selected, daily);
  switch (details.index.state) {
    case CalendarIndexState::Zero: count = L"文書数: 0件"; break;
    case CalendarIndexState::Reading: count = L"文書数: 取得中"; break;
    case CalendarIndexState::Error: count = L"文書数: 読み取りエラー"; break;
    case CalendarIndexState::Ready: count = L"文書数: " + std::to_wstring(details.index.files.size()) + L"件"; break;
  }
  switch (details.index.state) {
    case CalendarIndexState::Zero: output += L"\n作成ファイル: 0件"; break;
    case CalendarIndexState::Reading: output += L"\n作成ファイル: 取得中"; break;
    case CalendarIndexState::Error: output += L"\n作成ファイル: 読み取りエラー"; break;
    case CalendarIndexState::Ready:
      output += L"\n作成ファイル: " + std::to_wstring(details.index.files.size()) + L"件";
      break;
  }
  for (const auto& file : details.index.files) {
    const auto begin = output.size() + 1;
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
    calendar_detail_targets_.push_back(
        CalendarDetailTarget{begin, output.size(), workspace_ / file.relative_path});
  }
  if (!calendar_detail_targets_.empty())
    output += L"\n行を選択してEnter、またはダブルクリックで開けます。";
  if (details.configured_daily_path) {
    std::error_code path_error;
    const auto relative = std::filesystem::relative(*details.configured_daily_path,
                                                    workspace_, path_error);
    if (!path_error) output += L"\nDaily path: " + relative.generic_wstring();
  }
  if (!details.workspace_index.error.empty()) output += L"\n" + details.workspace_index.error;
  publish();
}

void Application::UpdateCalendarDetails(CalendarDate date) {
  SYSTEMTIME value{};
  value.wYear = static_cast<WORD>(date.year);
  value.wMonth = static_cast<WORD>(date.month);
  value.wDay = static_cast<WORD>(date.day);
  UpdateCalendarDetails(value);
}

void Application::OpenCalendarDate(CalendarDate date) {
  if (!IsValidCalendarDate(date)) return;
  SYSTEMTIME value{};
  value.wYear = static_cast<WORD>(date.year);
  value.wMonth = static_cast<WORD>(date.month);
  value.wDay = static_cast<WORD>(date.day);
  CreateProfileForDate(BuiltInProfile::Daily, value);
  UpdateCalendarViewMarkers();
  UpdateCalendarDetails(value);
}

void Application::OpenSelectedCalendarDate() {
  if (!calendar_) return;
  const auto selected = CalendarView_GetSelection(calendar_);
  if (selected) OpenCalendarDate(*selected);
}
}  // namespace mdlite
