#include "app/Application.h"

#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "calendar/JapaneseHolidays.h"
#include "git/Conflict.h"
#include "search/Search.h"
#include "search/Replace.h"
#include "process/ProcessRunner.h"
#include "workspace/Trust.h"

#include <commctrl.h>
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
#include <array>
#include <cwctype>
#include <map>
#include <thread>

namespace mdlite {
namespace {

constexpr wchar_t kWindowClass[] = L"MDLite.MainWindow";
constexpr wchar_t kCompactWindowClass[] = L"MDLite.CompactWindow";
constexpr UINT_PTR kAutosaveTimer = 1;
constexpr UINT kTimerPollMs = 250;
constexpr UINT kRecoveryDelayMs = 5000;
constexpr int kTreeWidth = 250;
constexpr int kOutlineWidth = 230;
constexpr int kTabHeight = 30;
constexpr int kFindHeight = 66;
constexpr int kProcessDoneButton = 4400;

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

bool RunSearchWithCancel(HWND owner, const std::filesystem::path& root, const SearchQuery& query,
                         const std::map<std::filesystem::path, std::wstring>& unsaved,
                         std::vector<SearchMatch>& matches, std::wstring& error) {
  HANDLE cancellation = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  if (!cancellation) { error = L"検索のキャンセルeventを作成できません。"; return false; }
  ProcessDialogContext context{cancellation};
  context.marquee = true;
  bool completed{};
  std::thread worker([&] {
    completed = SearchWorkspace(root, query, unsaved, matches, error, [&] {
      return WaitForSingleObject(cancellation, 0) == WAIT_OBJECT_0;
    });
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
  config.pszWindowTitle = L"Workspace検索";
  config.pszMainInstruction = L"Workspaceを検索しています";
  config.pszContent = L"直接走査の完了を待っています。必要ならキャンセルできます。";
  config.cButtons = static_cast<UINT>(std::size(buttons));
  config.pButtons = buttons;
  config.pfCallback = ProcessDialogCallback;
  config.lpCallbackData = reinterpret_cast<LONG_PTR>(&context);
  int button{};
  const HRESULT dialog_result = TaskDialogIndirect(&config, &button, nullptr, nullptr);
  if (!context.finished.load()) SetEvent(cancellation);
  worker.join();
  CloseHandle(cancellation);
  if (FAILED(dialog_result)) { error = L"検索進捗dialogを表示できません。"; return false; }
  return completed;
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
};

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
  explicit PresentationUndoGuard(HWND editor) {
    IRichEditOle* rich_edit = nullptr;
    if (SendMessageW(editor, EM_GETOLEINTERFACE, 0, reinterpret_cast<LPARAM>(&rich_edit)) != 0 &&
        rich_edit != nullptr) {
      rich_edit->QueryInterface(IID_PPV_ARGS(&document_));
      rich_edit->Release();
    }
    if (document_) document_->Undo(tomSuspend, nullptr);
  }
  ~PresentationUndoGuard() {
    if (document_) {
      document_->Undo(tomResume, nullptr);
      document_->Release();
    }
  }
  PresentationUndoGuard(const PresentationUndoGuard&) = delete;
  PresentationUndoGuard& operator=(const PresentationUndoGuard&) = delete;

 private:
  ITextDocument* document_{};
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
  if (accelerator_table_) DestroyAcceleratorTable(accelerator_table_);
  if (editor_font_) DeleteObject(editor_font_);
  if (background_brush_) DeleteObject(background_brush_);
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
    SetFocus((*view)->editor);
    return 0;
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
  if (message == WM_PASTE && app->PasteClipboardImage()) return 0;
  if (message == WM_IME_STARTCOMPOSITION) app->ime_composing_ = true;
  if (message == WM_IME_ENDCOMPOSITION) {
    app->ime_composing_ = false;
    app->OnEditorChanged(window);
  }
  if (message == WM_KEYDOWN && wparam == VK_TAB && !app->ime_composing_ &&
      app->active_document_ < app->documents_.size() &&
      IsMarkdownFile(app->documents_[app->active_document_]->document.path()) &&
      app->documents_[app->active_document_]->editor == window) {
    auto& view = *app->documents_[app->active_document_];
    app->SyncDocumentFromEditor(view);
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    const auto source_caret = view.editor_snapshot.ViewToSource(selection.cpMin);
    const auto edit = MoveToAdjacentTableCell(view.document.text(), source_caret,
                                              (GetKeyState(VK_SHIFT) & 0x8000) != 0);
    if (edit.changed) {
      const auto target = BuildMarkdownEditorSnapshot(edit.text);
      app->suppress_editor_change_ = true;
      SendMessageW(window, EM_SETSEL, 0, -1);
      SendMessageW(window, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(target.view.c_str()));
      app->suppress_editor_change_ = false;
      app->OnEditorChanged(window);
    }
    if (edit.selection != source_caret || edit.changed) {
      const auto view_caret = view.editor_snapshot.SourceToView(edit.selection);
      SendMessageW(window, EM_SETSEL, view_caret, view_caret);
      return 0;
    }
  }
  if (message == WM_KEYDOWN && !app->ime_composing_ &&
      (wparam == VK_LEFT || wparam == VK_RIGHT || wparam == VK_UP || wparam == VK_DOWN) &&
      (GetKeyState(VK_SHIFT) & 0x8000) == 0 && (GetKeyState(VK_CONTROL) & 0x8000) == 0 &&
      (GetKeyState(VK_MENU) & 0x8000) == 0 && app->active_document_ < app->documents_.size() &&
      IsMarkdownFile(app->documents_[app->active_document_]->document.path()) &&
      app->documents_[app->active_document_]->editor == window) {
    auto& view = *app->documents_[app->active_document_];
    app->SyncDocumentFromEditor(view);
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    if (selection.cpMin == selection.cpMax) {
      const auto direction = wparam == VK_LEFT ? TableCaretDirection::Left :
          wparam == VK_RIGHT ? TableCaretDirection::Right :
          wparam == VK_UP ? TableCaretDirection::Up : TableCaretDirection::Down;
      const auto destination = MoveTableCaretAtBoundary(
          view.document.text(), view.editor_snapshot.ViewToSource(selection.cpMin), direction);
      if (destination) {
        const auto view_caret = view.editor_snapshot.SourceToView(*destination);
        SendMessageW(window, EM_SETSEL, view_caret, view_caret);
        return 0;
      }
    }
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, EditorSubclass, 1);
  return DefSubclassProc(window, message, wparam, lparam);
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
    case WM_TIMER:
      if (wparam == kAutosaveTimer) {
        if (external_operation_active_) return 0;
        const ULONGLONG now = GetTickCount64();
        bool pending = false;
        for (auto& view : documents_) {
          if (!view->animated_image_frames.empty()) {
            pending = true;
            AdvanceAnimatedImages(*view, now);
          }
          if (!view->document.dirty()) continue;
          pending = true;
          const bool composing_this_view = ime_composing_ && active_document_ < documents_.size() &&
                                           documents_[active_document_].get() == view.get();
          if (composing_this_view) continue;
          if (view->recovery_due != 0 && now >= view->recovery_due) {
            SaveRecovery(*view);
            view->recovery_due = now + kRecoveryDelayMs;
          }
          if (settings_.auto_save && view->autosave_due != 0 && now >= view->autosave_due) {
            if (SaveDocument(*view, false)) {
              view->autosave_due = 0;
              view->recovery_due = 0;
            } else {
              view->autosave_due = now + kRecoveryDelayMs;
            }
          }
        }
        if (!pending) KillTimer(window_, kAutosaveTimer);
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
          if (ime_composing_ || !IsMarkdownFile(documents_[active_document_]->document.path())) continue;
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
    case WM_COMMAND: {
      const int command = LOWORD(wparam);
      if (command == kFileOpenWorkspace) OpenWorkspaceDialog();
      else if (command == kFileNew) NewUntitledDocument();
      else if (command == kFileOpen) OpenFileDialog();
      else if (command == kFileQuickOpen) QuickOpen();
      else if (command == kFileClose && active_document_ < documents_.size()) CloseDocument(active_document_);
      else if (command == kFileSave && active_document_ < documents_.size())
        SaveDocument(*documents_[active_document_], true);
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
      else if (command == kEditFindWorkspace) SearchWorkspaceFromFindBar();
      else if (command == kEditReplaceWorkspace || command == kReplaceWorkspace)
        ReplaceWorkspaceFromFindBar();
      else if (command == kFindWorkspace) SearchWorkspaceFromFindBar();
      else if (command == kViewCalendar) {
        ShowWindow(calendar_, IsWindowVisible(calendar_) ? SW_HIDE : SW_SHOW);
        LayoutControls();
      }
      else if (command == kViewSettings) OpenWorkspaceSettings();
      else if (command == kViewSettingsFiles) OpenWorkspaceSettingsFiles();
      else if (command == kViewProfiles) ManageProfiles();
      else if (command == kViewCompact) ToggleCompactWindow();
      else if (command == kViewCommandPalette) ShowCommandPalette();
      else if (command == kTableRowBefore) ApplyTableAction(TableAction::InsertRowBefore);
      else if (command == kTableRowAfter) ApplyTableAction(TableAction::InsertRowAfter);
      else if (command == kTableRowDelete) ApplyTableAction(TableAction::DeleteRow);
      else if (command == kTableColumnBefore) ApplyTableAction(TableAction::InsertColumnBefore);
      else if (command == kTableColumnAfter) ApplyTableAction(TableAction::InsertColumnAfter);
      else if (command == kTableColumnDelete) ApplyTableAction(TableAction::DeleteColumn);
      else if (HIWORD(wparam) == EN_CHANGE) OnEditorChanged(reinterpret_cast<HWND>(lparam));
      return 0;
    }
    case WM_NOTIFY: {
      const auto* header = reinterpret_cast<NMHDR*>(lparam);
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
          const auto position = static_cast<LONG>(documents_[active_document_]->editor_snapshot.SourceToView(
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
      } else if (header->hwndFrom == calendar_ && header->code == MCN_SELECT) {
        const auto* selection = reinterpret_cast<NMSELCHANGE*>(lparam);
        CreateProfileForDate(BuiltInProfile::Daily, selection->stSelStart);
        ShowWindow(calendar_, SW_HIDE);
      } else if (active_document_ < documents_.size() &&
                 header->hwndFrom == documents_[active_document_]->editor &&
                 header->code == EN_SELCHANGE) {
        ApplyMarkdownPresentation(*documents_[active_document_], false);
      } else if (active_document_ < documents_.size() &&
                 header->hwndFrom == documents_[active_document_]->editor &&
                 header->code == EN_LINK) {
        const auto* link = reinterpret_cast<const ENLINK*>(lparam);
        auto& view = *documents_[active_document_];
        const auto source_position = view.editor_snapshot.ViewToSource(link->chrg.cpMin);
        if (link->msg == WM_LBUTTONUP) OpenLinkAtSourcePosition(view, source_position, true);
        else if (link->msg == WM_MOUSEMOVE) OpenLinkAtSourcePosition(view, source_position, false);
      }
      return 0;
    }
    case WM_CLOSE:
      if (!SaveAll(true)) return 0;
      SaveSession();
      for (auto& view : documents_) {
        if (!view->compact_window) continue;
        SetParent(view->editor, window_);
        DestroyWindow(view->compact_window);
        view->compact_window = nullptr;
      }
      DestroyWindow(window_);
      return 0;
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
    case WM_CTLCOLOREDIT:
    case WM_CTLCOLORSTATIC:
    case WM_CTLCOLORLISTBOX: {
      auto dc = reinterpret_cast<HDC>(wparam);
      SetTextColor(dc, theme_foreground_);
      SetBkColor(dc, theme_background_);
      return reinterpret_cast<LRESULT>(background_brush_ ? background_brush_ : GetSysColorBrush(COLOR_WINDOW));
    }
    case WM_DESTROY:
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
  AppendMenuW(view, MF_STRING, kViewCalendar, L"カレンダー");
  AppendMenuW(view, MF_STRING, kViewSettings, L"Workspace設定を開く");
  AppendMenuW(view, MF_STRING, kViewSettingsFiles, L"設定ファイルを詳細編集");
  AppendMenuW(view, MF_STRING, kViewProfiles, L"作成プロファイルを管理…");
  AppendMenuW(view, MF_STRING, kViewCompact, L"現在の文書をコンパクト表示");
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
  SetMenu(window_, menu);
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
  status_ = CreateWindowExW(0, STATUSCLASSNAMEW, nullptr, WS_CHILD | WS_VISIBLE,
                            0, 0, 0, 0, window_, nullptr, instance_, nullptr);
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
  calendar_ = CreateWindowExW(WS_EX_CLIENTEDGE, MONTHCAL_CLASSW, nullptr,
                              WS_CHILD | MCS_DAYSTATE | MCS_WEEKNUMBERS,
                              0, 0, 0, 0, window_, nullptr, instance_, nullptr);
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
                       find_workspace_, replace_workspace_, find_case_, find_regex_, find_word_})
    SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(font), TRUE);
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
  const int center_left = kTreeWidth;
  const int center_width = std::max(0L, client.right - kTreeWidth - kOutlineWidth);
  MoveWindow(workspace_tree_, 0, 0, kTreeWidth, content_height, TRUE);
  MoveWindow(outline_, client.right - kOutlineWidth, 0, kOutlineWidth, content_height, TRUE);
  MoveWindow(tabs_, center_left, 0, center_width, kTabHeight, TRUE);
  MoveWindow(find_bar_, center_left, kTabHeight, center_width, find_visible ? kFindHeight : 0, TRUE);
  const int options_width = 210;
  const int button_width = 100;
  const int input_width = std::max(80, center_width - options_width - button_width - 24);
  MoveWindow(find_edit_, center_left + 8, kTabHeight + 4, input_width, 24, TRUE);
  MoveWindow(find_next_, center_left + 12 + input_width, kTabHeight + 3, button_width, 25, TRUE);
  MoveWindow(find_case_, center_left + 116 + input_width, kTabHeight + 5, 72, 22, TRUE);
  MoveWindow(find_regex_, center_left + 188 + input_width, kTabHeight + 5, 62, 22, TRUE);
  MoveWindow(find_word_, center_left + 250 + input_width, kTabHeight + 5, 58, 22, TRUE);
  MoveWindow(replace_edit_, center_left + 8, kTabHeight + 36, input_width, 24, TRUE);
  MoveWindow(find_workspace_, center_left + 12 + input_width, kTabHeight + 35, button_width, 25, TRUE);
  MoveWindow(replace_workspace_, center_left + 116 + input_width, kTabHeight + 35, 118, 25, TRUE);
  const int editor_top = kTabHeight + (find_visible ? kFindHeight : 0);
  for (auto& view : documents_) {
    if (GetParent(view->editor) == window_)
      MoveWindow(view->editor, center_left, editor_top, center_width,
                 std::max(0, content_height - editor_top), TRUE);
  }
  if (calendar_) {
    RECT required{};
    MonthCal_GetMinReqRect(calendar_, &required);
    const int width = required.right - required.left;
    const int height = required.bottom - required.top;
    MoveWindow(calendar_, std::max(center_left, static_cast<int>(client.right) - kOutlineWidth - width - 8),
               editor_top + 8, width, height, TRUE);
    if (calendar_tooltip_) {
      TTTOOLINFOW tool{sizeof(tool)};
      tool.hwnd = calendar_;
      tool.uId = 1;
      GetClientRect(calendar_, &tool.rect);
      SendMessageW(calendar_tooltip_, TTM_NEWTOOLRECTW, 0, reinterpret_cast<LPARAM>(&tool));
    }
    if (IsWindowVisible(calendar_)) SetWindowPos(calendar_, HWND_TOP, 0, 0, 0, 0,
                                                 SWP_NOMOVE | SWP_NOSIZE);
  }
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
  const auto initial = FindQuickOpenCandidates(workspace_, L"", recent_documents_, 12);
  std::wstring list;
  for (const auto& path : initial) {
    std::error_code error;
    const auto relative = std::filesystem::relative(path, workspace_, error);
    if (!error) list += L"  " + relative.generic_wstring() + L"\n";
  }
  std::wstring query;
  if (!PromptText(window_, instance_, L"Quick Open",
                  L"ファイル名またはWorkspace相対pathで絞り込みます。\n"
                  L"空欄なら最近使用した文書を先頭に表示します。\n\n" + list,
                  query)) return;
  auto matches = FindQuickOpenCandidates(workspace_, query, recent_documents_, 20);
  if (matches.empty()) {
    MessageBoxW(window_, L"一致する文書がありません。", L"Quick Open", MB_ICONINFORMATION);
    return;
  }
  if (matches.size() == 1) {
    OpenDocument(matches.front());
    return;
  }
  std::wstring choices;
  for (const auto& path : matches) {
    std::error_code error;
    const auto relative = std::filesystem::relative(path, workspace_, error);
    if (!error) choices += relative.generic_wstring() + L"\n";
  }
  std::error_code relative_error;
  std::wstring selected = std::filesystem::relative(matches.front(), workspace_, relative_error).generic_wstring();
  if (!PromptText(window_, instance_, L"Quick Open — 候補",
                  L"開く相対pathを指定してください。\n\n" + choices, selected)) return;
  const auto match = std::ranges::find_if(matches, [&](const auto& path) {
    std::error_code error;
    const auto relative = std::filesystem::relative(path, workspace_, error).generic_wstring();
    return !error && _wcsicmp(relative.c_str(), selected.c_str()) == 0;
  });
  if (match == matches.end()) {
    MessageBoxW(window_, L"候補一覧にある相対pathを指定してください。", L"Quick Open", MB_ICONWARNING);
    return;
  }
  OpenDocument(*match);
}

void Application::OpenWorkspace(const std::filesystem::path& path) {
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
        const int recover = MessageBoxW(window_,
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
              static_cast<LONG>((*found)->editor_snapshot.SourceToView(item.selection_begin)),
              static_cast<LONG>((*found)->editor_snapshot.SourceToView(item.selection_end))};
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
  SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, RGB(255, 255, 255));
  SetWindowSubclass(view->editor, EditorSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  suppress_editor_change_ = true;
  SetWindowTextW(view->editor, view->editor_snapshot.view.c_str());
  suppress_editor_change_ = false;

  TCITEMW tab{};
  tab.mask = TCIF_TEXT;
  tab.pszText = tab_name.data();
  TabCtrl_InsertItem(tabs_, static_cast<int>(documents_.size()), &tab);
  documents_.push_back(std::move(view));
  ApplySettings();
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
    suppress_editor_change_ = true;
    SetWindowTextW(documents_[index]->editor, documents_[index]->editor_snapshot.view.c_str());
    suppress_editor_change_ = false;
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
  ApplyMarkdownPresentation(*documents_[index], true);
  RebuildOutline(*documents_[index]);
  LayoutControls();
  SetFocus(documents_[index]->editor);
  UpdateStatus();
}

bool Application::CloseDocument(std::size_t index) {
  if (index >= documents_.size() || ime_composing_) return false;
  auto& view = *documents_[index];
  SyncDocumentFromEditor(view);
  if (view.document.dirty()) {
    const int answer = MessageBoxW(window_,
        (L"変更を保存してタブを閉じますか？\n" + view.document.path().wstring()).c_str(),
        L"タブを閉じる", MB_ICONQUESTION | MB_YESNOCANCEL | MB_DEFBUTTON1);
    if (answer == IDCANCEL) return false;
    if (answer == IDYES && !SaveDocument(view, true)) return false;
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

bool Application::SaveDocument(DocumentView& view, bool interactive) {
  if (ime_composing_) return false;
  SyncDocumentFromEditor(view);
  if (!view.document.dirty()) return true;
  if (view.document.untitled()) return SaveDocumentAs(view);
  std::wstring error;
  if (!view.document.Save(error)) {
    const bool recovered = SaveRecovery(view, false);
    UpdateStatus();
    if (interactive) {
      if (recovered) error += L"\n最新の編集内容はWorkspaceの復旧領域へ保存しました。";
      else error += L"\n復旧領域への保存にも失敗しました。別名保存するか編集を続けてください。";
      MessageBoxW(window_, error.c_str(), L"保存できません", MB_ICONWARNING);
    }
    return false;
  }
  if (view.workspace_store) {
    std::wstring recovery_error;
    view.workspace_store->RemoveRecovery(view.document.path(), recovery_error);
  }
  UpdateStatus();
  return true;
}

bool Application::SaveDocumentAs(DocumentView& view) {
  if (ime_composing_) return false;
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
  if (!GetSaveFileNameW(&dialog)) return false;
  const auto old_path = view.document.path();
  std::wstring error;
  if (!view.document.SaveAs(path, error)) {
    MessageBoxW(window_, error.c_str(), L"別名保存できません", MB_ICONWARNING);
    return false;
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
  return true;
}

void Application::ReloadDocumentFromDisk() {
  if (ime_composing_ || active_document_ >= documents_.size()) return;
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
  view.derived_image_revision = std::numeric_limits<std::uint64_t>::max();
  suppress_editor_change_ = true;
  SetWindowTextW(view.editor, view.editor_snapshot.view.c_str());
  suppress_editor_change_ = false;
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

bool Application::SaveAll(bool interactive) {
  bool result = true;
  bool all_recovered = true;
  for (auto& view : documents_) {
    if (!SaveDocument(*view, interactive)) {
      result = false;
      all_recovered = SaveRecovery(*view, false) && all_recovered;
    }
  }
  if (!result && interactive) {
    if (all_recovered) {
      return MessageBoxW(window_,
                         L"保存できない文書があります。最新内容は復旧領域に保存済みです。\n"
                         L"復旧可能な状態で終了しますか？",
                         L"MDLite", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) == IDYES;
    }
    const int choice = MessageBoxW(
        window_, L"文書保存と復旧保存の両方に失敗しました。\n"
                 L"[はい] 別名保存  [いいえ] 編集内容を明示破棄して終了  [キャンセル] 編集へ戻る",
        L"本文保護", MB_ICONERROR | MB_YESNOCANCEL | MB_DEFBUTTON3);
    if (choice == IDCANCEL) return false;
    if (choice == IDNO) return true;
    for (auto& view : documents_) {
      if (view->document.dirty() && !SaveDocumentAs(*view)) return false;
    }
    return true;
  }
  return result;
}

void Application::OnEditorChanged(HWND editor) {
  if (suppress_editor_change_ || ime_composing_) return;
  for (auto& view : documents_) {
    if (view->editor != editor) continue;
    SyncDocumentFromEditor(*view);
    if (IsMarkdownFile(view->document.path())) {
      view->parse = ParseMarkdown(view->document.text());
      ApplyMarkdownPresentation(*view, true);
    } else {
      view->parse = {};
    }
    if (active_document_ < documents_.size() && documents_[active_document_].get() == view.get())
      RebuildOutline(*view);
    const ULONGLONG now = GetTickCount64();
    view->autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
    if (view->recovery_due == 0) view->recovery_due = now + kRecoveryDelayMs;
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
    UpdateStatus();
    break;
  }
}

void Application::SyncDocumentFromEditor(DocumentView& view) {
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_begin = view.editor_snapshot.ViewToSource(selection.cpMin);
  const std::size_t source_end = view.editor_snapshot.ViewToSource(selection.cpMax);
  const std::wstring current_view = EditorText(view.editor);
  const auto transaction = ApplyEditorText(view.editor_snapshot, view.document.text(), current_view);
  if (!transaction.changed) return;
  view.document.MarkEdited(transaction.source);
  view.editor_snapshot = SnapshotFor(view.document);
  if (current_view != view.editor_snapshot.view) {
    suppress_editor_change_ = true;
    SetWindowTextW(view.editor, view.editor_snapshot.view.c_str());
    const CHARRANGE restored{static_cast<LONG>(view.editor_snapshot.SourceToView(source_begin)),
                             static_cast<LONG>(view.editor_snapshot.SourceToView(source_end))};
    SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&restored));
    suppress_editor_change_ = false;
    RefreshDerivedImages(view);
  }
}

void Application::RefreshDerivedImages(DocumentView& view) {
  if (!IsMarkdownFile(view.document.path())) return;
  if (view.derived_image_revision == view.document.revision()) return;
  view.derived_image_revision = view.document.revision();
  view.animated_image_frames.clear();
  view.animation_due = 0;
  const auto parsed = ParseMarkdown(view.document.text());
  if (parsed.images.empty()) return;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  PresentationUndoGuard undo_guard(view.editor);
  suppress_editor_change_ = true;
  unsigned next_delay = std::numeric_limits<unsigned>::max();
  for (auto iterator = parsed.images.rbegin(); iterator != parsed.images.rend(); ++iterator) {
    const auto& image = *iterator;
    std::filesystem::path target(image.target);
    if (target.is_absolute() || image.target.find(L"://") != std::wstring::npos) continue;
    target = (view.document.path().parent_path() / target).lexically_normal();
    std::error_code filesystem_error;
    if (!std::filesystem::is_regular_file(target, filesystem_error)) continue;
    bool safe{};
    std::wstring safety_message;
    std::wstring safety_error;
    if (!InspectImageSafety(target, safe, safety_message, safety_error) || !safe) continue;
    IStream* stream = nullptr;
    RasterImageInfo image_info;
    std::wstring image_error;
    const bool has_image_info = ReadRasterImageInfo(target, image_info, image_error);
    unsigned frame_delay = 100;
    if (has_image_info && image_info.animated) {
      if (!CreateRasterFramePngStream(target, 0, stream, frame_delay, image_error)) continue;
      view.animated_image_frames[image.begin] = 0;
      next_delay = std::min(next_delay, frame_delay);
    } else if (FAILED(SHCreateStreamOnFileEx(target.c_str(), STGM_READ | STGM_SHARE_DENY_WRITE,
                                             FILE_ATTRIBUTE_NORMAL, FALSE, nullptr, &stream))) {
      continue;
    }
    const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToView(image.begin));
    SendMessageW(view.editor, EM_SETSEL, position, position + 1);
    SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L""));
    const unsigned width = image.width_dip == 0 ? 320U : image.width_dip;
    unsigned height = width * 9U / 16U;
    if (has_image_info) {
      const auto scaled = static_cast<unsigned long long>(width) * image_info.height / image_info.width;
      height = static_cast<unsigned>(std::clamp<unsigned long long>(scaled, 16, 8192));
    }
    RICHEDIT_IMAGE_PARAMETERS parameters{};
    parameters.xWidth = static_cast<LONG>(width * 2540U / 96U);
    parameters.yHeight = static_cast<LONG>(height * 2540U / 96U);
    parameters.Ascent = parameters.yHeight;
    parameters.Type = TA_BASELINE;
    parameters.pwszAlternateText = image.alternate_text.c_str();
    parameters.pIStream = stream;
    const auto inserted = static_cast<HRESULT>(
        SendMessageW(view.editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters)));
    if (FAILED(inserted)) {
      SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
    }
    stream->Release();
  }
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  suppress_editor_change_ = false;
  if (!view.animated_image_frames.empty()) {
    view.animation_due = GetTickCount64() +
        (next_delay == std::numeric_limits<unsigned>::max() ? 100 : next_delay);
    SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  }
}

void Application::AdvanceAnimatedImages(DocumentView& view, ULONGLONG now) {
  if (view.animation_due == 0 || now < view.animation_due || !IsWindowVisible(view.editor)) return;
  const auto parsed = ParseMarkdown(view.document.text());
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  RECT client{};
  GetClientRect(view.editor, &client);
  unsigned next_delay = std::numeric_limits<unsigned>::max();
  bool kept_animation{};
  bool changed{};
  PresentationUndoGuard undo_guard(view.editor);
  suppress_editor_change_ = true;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  for (const auto& image : parsed.images) {
    auto frame = view.animated_image_frames.find(image.begin);
    if (frame == view.animated_image_frames.end()) continue;
    std::filesystem::path target(image.target);
    if (target.is_absolute() || image.target.find(L"://") != std::wstring::npos) continue;
    target = (view.document.path().parent_path() / target).lexically_normal();
    RasterImageInfo image_info;
    std::wstring image_error;
    if (!ReadRasterImageInfo(target, image_info, image_error) || image_info.frame_count < 2) continue;
    kept_animation = true;
    const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToView(image.begin));
    POINT point{};
    SendMessageW(view.editor, EM_POSFROMCHAR, reinterpret_cast<WPARAM>(&point), position);
    if (point.y < client.top || point.y >= client.bottom ||
        point.x < client.left || point.x >= client.right) continue;
    const unsigned next_frame = (frame->second + 1) % image_info.frame_count;
    IStream* stream{};
    unsigned frame_delay{};
    if (!CreateRasterFramePngStream(target, next_frame, stream, frame_delay, image_error)) continue;
    const unsigned width = image.width_dip == 0 ? 320U : image.width_dip;
    const auto scaled = static_cast<unsigned long long>(width) * image_info.height / image_info.width;
    const unsigned height = static_cast<unsigned>(std::clamp<unsigned long long>(scaled, 16, 8192));
    SendMessageW(view.editor, EM_SETSEL, position, position + 1);
    SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L""));
    RICHEDIT_IMAGE_PARAMETERS parameters{};
    parameters.xWidth = static_cast<LONG>(width * 2540U / 96U);
    parameters.yHeight = static_cast<LONG>(height * 2540U / 96U);
    parameters.Ascent = parameters.yHeight;
    parameters.Type = TA_BASELINE;
    parameters.pwszAlternateText = image.alternate_text.c_str();
    parameters.pIStream = stream;
    const auto inserted = static_cast<HRESULT>(
        SendMessageW(view.editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters)));
    if (FAILED(inserted))
      SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L"\uFFFC"));
    else
      frame->second = next_frame;
    stream->Release();
    next_delay = std::min(next_delay, frame_delay);
    changed = true;
  }
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  if (changed) InvalidateRect(view.editor, nullptr, FALSE);
  suppress_editor_change_ = false;
  if (!kept_animation) {
    view.animated_image_frames.clear();
    view.animation_due = 0;
  } else {
    view.animation_due = now +
        (next_delay == std::numeric_limits<unsigned>::max() ? kTimerPollMs : next_delay);
  }
}

void Application::ApplyMarkdownPresentation(DocumentView& view, bool force) {
  if (!IsMarkdownFile(view.document.path())) return;
  if (ime_composing_) return;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const int active_line = static_cast<int>(SendMessageW(view.editor, EM_LINEFROMCHAR, selection.cpMin, 0));
  if (!force && view.active_line == active_line) return;
  view.active_line = active_line;
  view.parse = ParseMarkdown(view.document.text());
  RefreshDerivedImages(view);

  PresentationUndoGuard undo_guard(view.editor);
  suppress_editor_change_ = true;
  SendMessageW(view.editor, WM_SETREDRAW, FALSE, 0);
  const LONG length = GetWindowTextLengthW(view.editor);
  SendMessageW(view.editor, EM_SETSEL, 0, length);
  const bool dark = settings_.theme == ThemeMode::Dark ||
                    (settings_.theme == ThemeMode::System && SystemUsesDarkTheme());
  const COLORREF foreground = ThemeColor(settings_, L"foreground", dark ? RGB(230, 230, 230) : RGB(24, 24, 24));
  const COLORREF background = ThemeColor(settings_, L"background", dark ? RGB(31, 31, 31) : RGB(255, 255, 255));
  CHARFORMAT2W normal{sizeof(normal)};
  normal.dwMask = CFM_FACE | CFM_SIZE | CFM_COLOR | CFM_BOLD | CFM_ITALIC | CFM_STRIKEOUT |
                  CFM_HIDDEN | CFM_BACKCOLOR | CFM_LINK;
  normal.dwEffects = 0;
  normal.crTextColor = foreground;
  normal.crBackColor = background;
  normal.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
  wcsncpy_s(normal.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
  SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&normal));

  const LONG active_start = static_cast<LONG>(SendMessageW(view.editor, EM_LINEINDEX, active_line, 0));
  const LONG active_length = static_cast<LONG>(SendMessageW(view.editor, EM_LINELENGTH, active_start, 0));
  const LONG active_end = active_start + active_length;
  for (const auto& span : view.parse.spans) {
    const auto view_begin = view.editor_snapshot.SourceToView(span.begin);
    const auto view_end = view.editor_snapshot.SourceToView(span.end);
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
    const LONG begin = static_cast<LONG>(view.editor_snapshot.SourceToView(table.begin));
    const LONG end = static_cast<LONG>(view.editor_snapshot.SourceToView(table.end));
    SendMessageW(view.editor, EM_SETSEL, begin, end);
    CHARFORMAT2W table_format{sizeof(table_format)};
    table_format.dwMask = CFM_FACE | CFM_BACKCOLOR;
    wcscpy_s(table_format.szFaceName, L"Cascadia Mono");
    table_format.crBackColor = ThemeColor(settings_, L"table_background", dark ? RGB(38, 42, 48) : RGB(244, 247, 250));
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&table_format));
    PARAFORMAT2 paragraph{sizeof(paragraph)};
    paragraph.dwMask = PFM_SPACEBEFORE | PFM_SPACEAFTER | PFM_LINESPACING | PFM_BORDER;
    paragraph.dySpaceBefore = 40;
    paragraph.dySpaceAfter = 40;
    paragraph.bLineSpacingRule = 0;
    paragraph.wBorders = 0x0F;
    paragraph.wBorderWidth = 8;
    paragraph.wBorderSpace = 2;
    SendMessageW(view.editor, EM_SETPARAFORMAT, 0, reinterpret_cast<LPARAM>(&paragraph));
  }
  SendMessageW(view.editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  SendMessageW(view.editor, WM_SETREDRAW, TRUE, 0);
  InvalidateRect(view.editor, nullptr, TRUE);
  suppress_editor_change_ = false;
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

std::wstring Application::EditorText(HWND editor) const {
  const int length = GetWindowTextLengthW(editor);
  std::wstring text(static_cast<std::size_t>(length) + 1, L'\0');
  const int copied = GetWindowTextW(editor, text.data(), length + 1);
  text.resize(static_cast<std::size_t>(std::max(0, copied)));
  return text;
}

void Application::UpdateStatus() {
  std::wstring text;
  if (active_document_ < documents_.size()) {
    const auto& document = documents_[active_document_]->document;
    text = document.dirty() ? L"未保存  |  " : L"保存済み  |  ";
    text += EncodingLabel(document.encoding()) + L"  |  " + LineEndingLabel(document.line_ending());
    if (document.HasExternalChange()) text += L"  |  外部変更あり";
  } else if (!workspace_.empty()) {
    text = workspace_.wstring();
  } else {
    text = L"Workspaceまたはファイルを開いてください。";
  }
  SendMessageW(status_, SB_SETTEXTW, 0, reinterpret_cast<LPARAM>(text.c_str()));
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
  LayoutControls();
  SetFocus(find_edit_);
  SendMessageW(find_edit_, EM_SETSEL, 0, -1);
}

void Application::FindNext(bool restart_from_beginning) {
  if (active_document_ >= documents_.size()) return;
  wchar_t query[1024]{};
  GetWindowTextW(find_edit_, query, static_cast<int>(std::size(query)));
  if (query[0] == L'\0') {
    ShowFindBar();
    return;
  }
  HWND editor = documents_[active_document_]->editor;
  CHARRANGE selection{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  FINDTEXTEXW find{};
  find.chrg.cpMin = restart_from_beginning ? 0 : selection.cpMax;
  find.chrg.cpMax = -1;
  find.lpstrText = query;
  LRESULT found = SendMessageW(editor, EM_FINDTEXTEXW, FR_DOWN, reinterpret_cast<LPARAM>(&find));
  if (found < 0 && !restart_from_beginning) {
    FindNext(true);
    return;
  }
  if (found >= 0) {
    SendMessageW(editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&find.chrgText));
    SendMessageW(editor, EM_SCROLLCARET, 0, 0);
    SetFocus(editor);
  }
}

void Application::SearchWorkspaceFromFindBar() {
  if (workspace_.empty()) return;
  wchar_t query_text[1024]{};
  GetWindowTextW(find_edit_, query_text, static_cast<int>(std::size(query_text)));
  if (query_text[0] == L'\0') {
    ShowFindBar();
    return;
  }
  std::map<std::filesystem::path, std::wstring> unsaved;
  for (const auto& view : documents_) {
    if (view->document.dirty()) unsaved.emplace(view->document.path(), view->document.text());
  }
  std::vector<SearchMatch> matches;
  std::wstring error;
  SearchQuery query{query_text,
                    SendMessageW(find_case_, BM_GETCHECK, 0, 0) == BST_CHECKED,
                    SendMessageW(find_regex_, BM_GETCHECK, 0, 0) == BST_CHECKED};
  query.whole_word = SendMessageW(find_word_, BM_GETCHECK, 0, 0) == BST_CHECKED;
  if (!RunSearchWithCancel(window_, workspace_, query, unsaved, matches, error)) {
    MessageBoxW(window_, error.c_str(), L"Workspace検索", MB_ICONWARNING);
    return;
  }
  if (matches.empty()) {
    MessageBoxW(window_, L"一致する箇所はありません。", L"Workspace検索", MB_ICONINFORMATION);
    return;
  }
  OpenDocument(matches.front().path);
  if (active_document_ < documents_.size()) {
    HWND editor = documents_[active_document_]->editor;
    const auto& snapshot = documents_[active_document_]->editor_snapshot;
    const LONG begin = static_cast<LONG>(snapshot.SourceToView(matches.front().begin));
    const LONG end = static_cast<LONG>(snapshot.SourceToView(matches.front().end));
    SendMessageW(editor, EM_SETSEL, begin, end);
    SendMessageW(editor, EM_SCROLLCARET, 0, 0);
  }
  const std::wstring message = std::to_wstring(matches.size()) +
                               L"件見つかりました。最初の一致を開きました。";
  MessageBoxW(window_, message.c_str(), L"Workspace検索", MB_ICONINFORMATION);
}

void Application::ReplaceWorkspaceFromFindBar() {
  if (workspace_.empty()) return;
  ShowFindBar();
  wchar_t query_text[1024]{};
  wchar_t replacement[1024]{};
  GetWindowTextW(find_edit_, query_text, static_cast<int>(std::size(query_text)));
  GetWindowTextW(replace_edit_, replacement, static_cast<int>(std::size(replacement)));
  if (query_text[0] == L'\0') { SetFocus(find_edit_); return; }
  if (!SaveAll(true)) {
    MessageBoxW(window_, L"置換previewの前に全タブを保存してください。保存できない文書は変更しません。",
                L"Workspace置換", MB_ICONWARNING);
    return;
  }
  SearchQuery query{query_text,
                    SendMessageW(find_case_, BM_GETCHECK, 0, 0) == BST_CHECKED,
                    SendMessageW(find_regex_, BM_GETCHECK, 0, 0) == BST_CHECKED};
  query.whole_word = SendMessageW(find_word_, BM_GETCHECK, 0, 0) == BST_CHECKED;
  ReplacePlan plan;
  std::wstring error;
  if (!RunCancellableTask(window_, L"置換preview", L"置換対象を確認しています",
      [&](HANDLE cancellation) {
        return PreviewWorkspaceReplace(workspace_, query, replacement, {}, plan, error, [&] {
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
  const std::wstring preview = std::to_wstring(plan.files.size()) + L"ファイル、" +
      std::to_wstring(replacements) + L"箇所を置換します。\n"
      L"適用直前に各文書の改訂を再確認し、journalから条件付きで戻せます。続行しますか？";
  if (MessageBoxW(window_, preview.c_str(), L"置換preview", MB_ICONQUESTION | MB_YESNO) != IDYES) return;
  ReplaceApplyResult result;
  const bool complete = RunCancellableTask(window_, L"Workspace置換", L"安全保存とjournalを適用しています",
      [&](HANDLE cancellation) {
        return ApplyWorkspaceReplace(workspace_, plan, result, error, [&] {
          return WaitForSingleObject(cancellation, 0) == WAIT_OBJECT_0;
        });
      }, error);
  for (auto& view : documents_) {
    if (std::ranges::any_of(plan.files, [&](const auto& item) { return item.path == view->document.path(); })) {
      Document reloaded;
      std::wstring load_error;
      if (reloaded.Load(view->document.path(), load_error)) {
        view->document = std::move(reloaded);
        view->editor_snapshot = SnapshotFor(view->document);
        view->derived_image_revision = std::numeric_limits<std::uint64_t>::max();
        suppress_editor_change_ = true;
        SetWindowTextW(view->editor, view->editor_snapshot.view.c_str());
        suppress_editor_change_ = false;
        ApplyMarkdownPresentation(*view, true);
      }
    }
  }
  PopulateWorkspaceTree();
  std::wstring message = std::to_wstring(result.applied_files) + L"ファイル、" +
      std::to_wstring(result.applied_replacements) + L"箇所を置換しました。\njournal: " +
      result.journal.wstring();
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
  TTTOOLINFOW tool{sizeof(tool)};
  tool.hwnd = calendar_;
  tool.uId = 1;
  tool.lpszText = calendar_tooltip_text_.data();
  SendMessageW(calendar_tooltip_, TTM_UPDATETIPTEXTW, 0, reinterpret_cast<LPARAM>(&tool));
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
  if (active_document_ >= documents_.size() || ime_composing_ ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  HWND editor = view.editor;
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::wstring source = view.document.text();
  const auto source_caret = view.editor_snapshot.ViewToSource(selection.cpMin);
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
  const auto target = BuildMarkdownEditorSnapshot(edit.text);
  suppress_editor_change_ = true;
  SendMessageW(editor, EM_SETSEL, 0, -1);
  SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(target.view.c_str()));
  suppress_editor_change_ = false;
  OnEditorChanged(editor);
  const auto view_caret = view.editor_snapshot.SourceToView(edit.selection);
  SendMessageW(editor, EM_SETSEL, view_caret, view_caret);
}

void Application::MoveOutlineSection(std::size_t source_begin, std::size_t target_begin) {
  if (active_document_ >= documents_.size() || ime_composing_ || source_begin == target_begin ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  SyncDocumentFromEditor(view);
  const std::wstring source = view.document.text();
  const auto edit = MoveHeadingSection(source, source_begin, target_begin);
  if (!edit.changed) return;

  const auto target = BuildMarkdownEditorSnapshot(edit.text);
  suppress_editor_change_ = true;
  SendMessageW(view.editor, EM_SETSEL, 0, -1);
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(target.view.c_str()));
  suppress_editor_change_ = false;
  OnEditorChanged(view.editor);
  const auto view_caret = view.editor_snapshot.SourceToView(edit.selection);
  SendMessageW(view.editor, EM_SETSEL, view_caret, view_caret);
}

bool Application::SaveRecovery(DocumentView& view, bool interactive) {
  if (!view.workspace_store || ime_composing_) return false;
  SyncDocumentFromEditor(view);
  if (!view.document.dirty()) return false;
  std::wstring error;
  if (!view.workspace_store->WriteRecovery(view.document.path(), view.document.text(), error)) {
    SendMessageW(status_, SB_SETTEXTW, 0, reinterpret_cast<LPARAM>(error.c_str()));
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
    item.selection_begin = view->editor_snapshot.ViewToSource(selection.cpMin);
    item.selection_end = view->editor_snapshot.ViewToSource(selection.cpMax);
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
  ProcessResult status;
  std::wstring error;
  if (!RunProcess(git_path, {L"-C", workspace_.wstring(), L"status", L"--short", L"--branch"},
                  workspace_, 64 * 1024, 30000, status, error)) {
    MessageBoxW(window_, error.c_str(), L"Git", MB_ICONERROR);
    return;
  }
  ProcessResult branches;
  RunProcess(git_path, {L"-C", workspace_.wstring(), L"branch", L"--format=%(HEAD) %(refname:short)"},
             workspace_, 64 * 1024, 30000, branches, error);
  std::wstring output = L"status\n" + status.output + L"\nbranches\n" + branches.output;
  if (status.truncated || branches.truncated) output += L"\n(出力上限で省略しました)";
  MessageBoxW(window_, output.c_str(), L"Git Status / Branches", status.exit_code == 0 ? MB_ICONINFORMATION : MB_ICONWARNING);
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
      arguments.insert(arguments.end(), {L"commit", L"-m", value, L"--", L"."}); action = L"Workspace内をコミット"; break;
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
  if (save_first && !SaveAll(true)) {
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
  const auto source_position = view.editor_snapshot.ViewToSource(selection.cpMin);
  const auto block_index = FindConflictBlock(view.document.text(), source_position, previous);
  const auto blocks = ParseConflictBlocks(view.document.text());
  if (!block_index || *block_index >= blocks.size()) {
    MessageBoxW(window_, L"Gitは未マージと報告していますが、本文に解決用markerがありません。binary競合等を確認してください。",
                L"Git競合", MB_ICONWARNING);
    return;
  }
  const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToView(blocks[*block_index].begin));
  SendMessageW(view.editor, EM_SETSEL, position, position);
  SendMessageW(view.editor, EM_SCROLLCARET, 0, 0);
  SetFocus(view.editor);
}

void Application::ResolveGitConflict(ConflictChoice choice) {
  if (active_document_ >= documents_.size() || ime_composing_) return;
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
  const auto source_position = view.editor_snapshot.ViewToSource(selection.cpMin);
  const auto block_index = FindConflictBlock(view.document.text(), source_position, false);
  if (!block_index) {
    MessageBoxW(window_, L"解決できるtext競合markerがありません。", L"Git競合", MB_ICONWARNING);
    return;
  }
  const auto edit = ResolveConflictBlock(view.document.text(), *block_index, choice);
  if (!edit.changed) return;
  suppress_editor_change_ = true;
  SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
  SendMessageW(view.editor, EM_SETSEL, 0, -1);
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(edit.text.c_str()));
  SendMessageW(view.editor, EM_STOPGROUPTYPING, 0, 0);
  suppress_editor_change_ = false;
  SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(edit.selection), static_cast<LPARAM>(edit.selection));
  OnEditorChanged(view.editor);
  const auto remaining = ParseConflictBlocks(view.document.text()).size();
  MessageBoxW(window_, (L"競合箇所を1つ解決しました。残存marker: " + std::to_wstring(remaining) +
                        L"\n保存後に『解決済みにする』を明示実行してください。").c_str(),
              L"Git競合", MB_ICONINFORMATION);
}

void Application::MarkGitConflictResolved() {
  if (active_document_ >= documents_.size() || ime_composing_) return;
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
  if (!SaveDocument(view, true)) return;
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
  HBRUSH replacement_brush = CreateSolidBrush(background);
  if (replacement_brush) {
    if (background_brush_) DeleteObject(background_brush_);
    background_brush_ = replacement_brush;
  }
  theme_background_ = background;
  theme_foreground_ = foreground;
  const int height = -MulDiv(static_cast<int>(settings_.font_size_pt),
                             static_cast<int>(GetDpiForWindow(window_)), 72);
  HFONT replacement = CreateFontW(height, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
                                  DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
                                  CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE,
                                  settings_.font_face.c_str());
  if (replacement) {
    for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_,
                         replace_edit_, find_workspace_, replace_workspace_, find_case_, find_regex_,
                         find_word_, calendar_})
      if (control) SendMessageW(control, WM_SETFONT, reinterpret_cast<WPARAM>(replacement), TRUE);
  }
  TreeView_SetBkColor(workspace_tree_, background);
  TreeView_SetTextColor(workspace_tree_, foreground);
  TreeView_SetBkColor(outline_, background);
  TreeView_SetTextColor(outline_, foreground);
  for (auto& view : documents_) {
    PresentationUndoGuard guard(view->editor);
    CHARRANGE selection{};
    SendMessageW(view->editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, background);
    CHARFORMAT2W format{};
    format.cbSize = sizeof(format);
    format.dwMask = CFM_COLOR | CFM_FACE | CFM_SIZE;
    format.crTextColor = foreground;
    format.yHeight = static_cast<LONG>(settings_.font_size_pt * 20);
    wcsncpy_s(format.szFaceName, settings_.font_face.c_str(), _TRUNCATE);
    SendMessageW(view->editor, EM_SETSEL, 0, -1);
    SendMessageW(view->editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
    SendMessageW(view->editor, EM_EXSETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    ApplyMarkdownPresentation(*view, true);
  }
  if (replacement) {
    HFONT previous = editor_font_;
    editor_font_ = replacement;
    if (previous) DeleteObject(previous);
  }
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
  std::wstring summary = L"現在の実効値と継承元\n"
      L"auto save: " + std::wstring(settings_.auto_save ? L"on" : L"off") + L" / " +
      std::to_wstring(settings_.auto_save_delay_ms) + L"ms (" + settings_.origins[L"auto_save"] + L" / " +
      settings_.origins[L"auto_save_delay_ms"] + L")\n" +
      L"theme: " + ThemeName(settings_.theme) + L" (" + settings_.origins[L"theme"] + L")\n" +
      L"font: " + settings_.font_face + L" " + std::to_wstring(settings_.font_size_pt) + L"pt (" +
      settings_.origins[L"font_face"] + L" / " + settings_.origins[L"font_size_pt"] + L")\n" +
      L"default memo Workspace: " +
      (settings_.default_memo_workspace.empty() ? std::wstring(L"未設定") : settings_.default_memo_workspace.wstring()) +
      L" (" + settings_.origins[L"default_memo_workspace"] + L")\n\n" +
      L"編集範囲を common または workspace で指定します。\n"
      L"範囲全体を既定へ戻す場合は reset-common / reset-workspace。";
  std::wstring scope = workspace_store_ ? L"workspace" : L"common";
  if (!PromptText(window_, instance_, L"MDLite 設定", summary, scope)) return;
  std::ranges::transform(scope, scope.begin(), towlower);
  if ((scope == L"workspace" || scope == L"reset-workspace") && workspace_path.empty()) {
    MessageBoxW(window_, L"Workspaceが開かれていません。", L"設定", MB_ICONWARNING);
    return;
  }
  const auto target = (scope == L"common" || scope == L"reset-common") ? common_path : workspace_path;
  if (scope == L"reset-common" || scope == L"reset-workspace") {
    if (MessageBoxW(window_, (target.wstring() + L"\nの上書きを解除して継承へ戻しますか？").c_str(),
                    L"設定を既定へ戻す", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    std::error_code remove_error;
    std::filesystem::remove(target, remove_error);
    if (remove_error) { MessageBoxW(window_, L"設定上書きを削除できません。", L"設定", MB_ICONERROR); return; }
    LoadAndApplySettings();
    return;
  }
  if (scope != L"common" && scope != L"workspace") {
    MessageBoxW(window_, L"common / workspace / reset-common / reset-workspace のいずれかを指定してください。",
                L"設定", MB_ICONWARNING);
    return;
  }
  SettingsLayer layer;
  std::wstring error;
  if (!LoadSettingsLayer(target, layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return; }
  std::wstring auto_save = layer.auto_save ? (*layer.auto_save ? L"on" : L"off") : L"inherit";
  if (!PromptText(window_, instance_, L"編集", L"auto save: on / off / inherit", auto_save)) return;
  std::ranges::transform(auto_save, auto_save.begin(), towlower);
  if (auto_save == L"inherit") layer.auto_save.reset();
  else if (auto_save == L"on") layer.auto_save = true;
  else if (auto_save == L"off") layer.auto_save = false;
  else { MessageBoxW(window_, L"auto save値が不正です。", L"設定", MB_ICONWARNING); return; }
  std::wstring auto_save_delay = layer.auto_save_delay_ms ? std::to_wstring(*layer.auto_save_delay_ms) : L"inherit";
  if (!PromptText(window_, instance_, L"編集", L"auto save delay (100〜60000ms)。継承はinherit", auto_save_delay)) return;
  if (auto_save_delay == L"inherit") layer.auto_save_delay_ms.reset();
  else {
    try {
      std::size_t consumed{};
      const auto parsed = std::stoul(auto_save_delay, &consumed);
      if (consumed != auto_save_delay.size()) throw std::invalid_argument("trailing characters");
      layer.auto_save_delay_ms = static_cast<unsigned>(parsed);
    } catch (const std::exception&) {
      MessageBoxW(window_, L"auto save delayが数値ではありません。", L"設定", MB_ICONWARNING); return;
    }
  }
  std::wstring theme = layer.theme ? ThemeName(*layer.theme) : L"inherit";
  if (!PromptText(window_, instance_, L"外観", L"theme: system / light / dark / custom / inherit", theme)) return;
  if (theme == L"inherit") layer.theme.reset();
  else {
    const auto parsed = ParseTheme(theme);
    if (!parsed) { MessageBoxW(window_, L"theme値が不正です。", L"設定", MB_ICONWARNING); return; }
    layer.theme = *parsed;
  }
  std::wstring color;
  if (!PromptText(window_, instance_, L"カスタムテーマ",
      L"任意: color.name=#RRGGBB を1件指定。nameは background / foreground / link / heading / marker / code_background / table_background。\n"
      L"継承へ戻す場合は color.name=inherit。空欄なら変更しません。", color)) return;
  if (!color.empty()) {
    const auto equals = color.find(L'=');
    if (!color.starts_with(L"color.") || equals == std::wstring::npos || equals <= 6) {
      MessageBoxW(window_, L"color.name=#RRGGBB形式で指定してください。", L"設定", MB_ICONWARNING); return;
    }
    const auto name = color.substr(6, equals - 6);
    const auto value = color.substr(equals + 1);
    if (value == L"inherit") layer.colors.erase(name);
    else layer.colors[name] = value;
  }
  std::wstring font = layer.font_face.value_or(L"inherit");
  if (!PromptText(window_, instance_, L"外観", L"font face。継承する場合は inherit", font)) return;
  if (font == L"inherit") layer.font_face.reset(); else layer.font_face = font;
  std::wstring size = layer.font_size_pt ? std::to_wstring(*layer.font_size_pt) : L"inherit";
  if (!PromptText(window_, instance_, L"外観", L"font size (6〜96)。継承する場合は inherit", size)) return;
  if (size == L"inherit") layer.font_size_pt.reset();
  else {
    try { layer.font_size_pt = static_cast<unsigned>(std::stoul(size)); }
    catch (const std::exception&) { MessageBoxW(window_, L"font sizeが数値ではありません。", L"設定", MB_ICONWARNING); return; }
  }
  if (scope == L"common") {
    std::wstring memo_workspace = layer.default_memo_workspace
        ? layer.default_memo_workspace->wstring() : L"inherit";
    if (!PromptText(window_, instance_, L"ファイル",
                    L"既定のメモ用Workspaceの絶対path。未設定へ戻す場合は inherit",
                    memo_workspace)) return;
    if (memo_workspace == L"inherit") layer.default_memo_workspace.reset();
    else layer.default_memo_workspace = std::filesystem::path(memo_workspace);
  }
  std::wstring binding;
  if (!PromptText(window_, instance_, L"キー割当て",
      L"任意: command=shortcut を1件指定。例 file.save=Ctrl+Shift+S\n"
      L"解除は command=none。空欄なら変更しません。", binding)) return;
  if (!binding.empty()) {
    const auto equals = binding.find(L'=');
    if (equals == std::wstring::npos) { MessageBoxW(window_, L"command=shortcut形式で指定してください。", L"設定", MB_ICONWARNING); return; }
    const auto command = binding.substr(0, equals);
    static constexpr std::array known{L"file.new", L"file.open", L"file.save", L"file.quickOpen", L"file.close",
                                      L"edit.find", L"edit.findNext", L"view.commandPalette"};
    if (std::ranges::find(known, command) == known.end()) {
      MessageBoxW(window_, L"未対応のcommand名です。", L"設定", MB_ICONWARNING); return;
    }
    layer.keybindings[command] = binding.substr(equals + 1);
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
  if (!SaveSettingsLayer(target, layer, error)) { MessageBoxW(window_, error.c_str(), L"設定", MB_ICONERROR); return; }
  LoadAndApplySettings();
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
  std::wstring list;
  for (const auto& profile : effective) list += profile.id + L" — " + profile.name + L"\n";
  std::wstring scope = L"workspace";
  if (!PromptText(window_, instance_, L"作成プロファイル",
                  L"編集範囲: common / workspace\nWorkspace上書きは同じidの共通定義を置き換えます。", scope)) return;
  std::ranges::transform(scope, scope.begin(), towlower);
  if (scope != L"common" && scope != L"workspace") {
    MessageBoxW(window_, L"commonまたはworkspaceを指定してください。", L"作成プロファイル", MB_ICONWARNING);
    return;
  }
  const auto target = scope == L"common" ? common_path : workspace_path;
  std::vector<ProfileDefinition> layer;
  if (!LoadProfileFile(target, layer, error)) {
    MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR);
    return;
  }
  std::wstring action = L"edit";
  if (!PromptText(window_, instance_, L"作成プロファイル",
                  L"操作: add / edit / duplicate / delete / template\n\n現在の実効profile:\n" + list,
                  action)) return;
  std::ranges::transform(action, action.begin(), towlower);
  if (action != L"add" && action != L"edit" && action != L"duplicate" &&
      action != L"delete" && action != L"template") {
    MessageBoxW(window_, L"未対応の操作です。", L"作成プロファイル", MB_ICONWARNING);
    return;
  }
  std::wstring id = action == L"add" ? L"custom" : L"daily";
  if (!PromptText(window_, instance_, L"作成プロファイル", L"profile id", id) || id.empty()) return;
  const auto effective_item = std::ranges::find_if(effective, [&](const auto& item) { return item.id == id; });
  if (action == L"template") {
    if (effective_item == effective.end()) {
      MessageBoxW(window_, L"指定したprofileが見つかりません。", L"作成プロファイル", MB_ICONWARNING);
      return;
    }
    const auto template_path = workspace_store_->metadata_root() / effective_item->template_path;
    if (!std::filesystem::is_regular_file(template_path)) {
      MessageBoxW(window_, (L"template fileがありません:\n" + template_path.wstring()).c_str(),
                  L"作成プロファイル", MB_ICONWARNING);
      return;
    }
    OpenDocument(template_path);
    return;
  }
  if (action == L"delete") {
    const auto item = std::ranges::find_if(layer, [&](const auto& profile) { return profile.id == id; });
    if (item == layer.end()) {
      MessageBoxW(window_, L"この範囲には定義がありません。継承元の範囲を選んでください。",
                  L"作成プロファイル", MB_ICONINFORMATION);
      return;
    }
    if (MessageBoxW(window_, (L"この範囲のprofile定義を削除しますか？\n" + id +
                              L"\n下位または組込み定義があれば再び継承されます。").c_str(),
                    L"作成プロファイル", MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2) != IDYES) return;
    layer.erase(item);
    if (!SaveProfileFile(target, layer, error)) {
      MessageBoxW(window_, error.c_str(), L"作成プロファイル", MB_ICONERROR);
      return;
    }
    MessageBoxW(window_, L"この範囲の定義を削除しました。", L"作成プロファイル", MB_ICONINFORMATION);
    return;
  }

  ProfileDefinition profile;
  if (action == L"add") {
    if (std::ranges::any_of(effective, [&](const auto& item) { return item.id == id; })) {
      MessageBoxW(window_, L"既存idです。上書きする場合はeditを選んでください。",
                  L"作成プロファイル", MB_ICONWARNING);
      return;
    }
    profile = {id, id, L"Notes/{{date:yyyy}}", L"{{date:yyyyMMdd}}.md",
               L"templates/memo.md", ProfileCollision::Sequence};
  } else {
    if (effective_item == effective.end()) {
      MessageBoxW(window_, L"指定したprofileが見つかりません。", L"作成プロファイル", MB_ICONWARNING);
      return;
    }
    profile = *effective_item;
    if (action == L"duplicate") {
      std::wstring duplicate_id = id + L"-copy";
      if (!PromptText(window_, instance_, L"作成プロファイルを複製", L"新しいprofile id", duplicate_id) ||
          duplicate_id.empty()) return;
      if (std::ranges::any_of(effective, [&](const auto& item) { return item.id == duplicate_id; })) {
        MessageBoxW(window_, L"複製先idは既に存在します。", L"作成プロファイル", MB_ICONWARNING);
        return;
      }
      profile.id = duplicate_id;
      profile.name += L" コピー";
    }
  }
  std::wstring name = profile.name;
  std::wstring directory = profile.directory.generic_wstring();
  std::wstring filename = profile.filename.generic_wstring();
  std::wstring template_path = profile.template_path.generic_wstring();
  std::wstring collision = profile.collision == ProfileCollision::Sequence ? L"sequence" : L"open-existing";
  if (!PromptText(window_, instance_, L"作成プロファイル", L"表示名", name) ||
      !PromptText(window_, instance_, L"作成プロファイル", L"Workspace相対directory", directory) ||
      !PromptText(window_, instance_, L"作成プロファイル", L"filename", filename) ||
      !PromptText(window_, instance_, L"作成プロファイル", L".mdlite相対template path", template_path) ||
      !PromptText(window_, instance_, L"作成プロファイル", L"collision: open-existing / sequence", collision)) return;
  std::ranges::transform(collision, collision.begin(), towlower);
  profile.name = name;
  profile.directory = directory;
  profile.filename = filename;
  profile.template_path = template_path;
  if (collision == L"sequence") profile.collision = ProfileCollision::Sequence;
  else if (collision == L"open-existing") profile.collision = ProfileCollision::OpenExisting;
  else {
    MessageBoxW(window_, L"collision値が不正です。", L"作成プロファイル", MB_ICONWARNING);
    return;
  }
  std::wstring input_count = std::to_wstring(profile.inputs.size());
  if (!PromptText(window_, instance_, L"作成プロファイル",
                  L"任意入力項目数（0～8）。titleというidは{{title}}、その他は{{input:id}}でtemplate／pathに使用できます。",
                  input_count)) return;
  if (input_count.empty() || !std::ranges::all_of(input_count, [](wchar_t value) { return iswdigit(value) != 0; }) || input_count.size() > 1 ||
      input_count.front() > L'8') {
    MessageBoxW(window_, L"入力項目数は0～8で指定してください。", L"作成プロファイル", MB_ICONWARNING);
    return;
  }
  const auto requested_inputs = static_cast<std::size_t>(input_count.front() - L'0');
  profile.inputs.resize(requested_inputs);
  for (std::size_t index = 0; index < profile.inputs.size(); ++index) {
    auto& input = profile.inputs[index];
    if (input.id.empty()) input.id = index == 0 ? L"title" : L"field" + std::to_wstring(index + 1);
    if (input.label.empty()) input.label = input.id;
    std::wstring required = input.required ? L"yes" : L"no";
    const auto number = std::to_wstring(index + 1);
    if (!PromptText(window_, instance_, L"作成プロファイル", L"入力" + number + L" id", input.id) ||
        !PromptText(window_, instance_, L"作成プロファイル", L"入力" + number + L" 表示名", input.label) ||
        !PromptText(window_, instance_, L"作成プロファイル", L"入力" + number + L" 既定値（空でも可）", input.default_value) ||
        !PromptText(window_, instance_, L"作成プロファイル", L"入力" + number + L" 必須: yes / no", required)) return;
    std::ranges::transform(required, required.begin(), towlower);
    if (required == L"yes") input.required = true;
    else if (required == L"no") input.required = false;
    else {
      MessageBoxW(window_, L"必須はyesまたはnoで指定してください。", L"作成プロファイル", MB_ICONWARNING);
      return;
    }
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
      Entry{L"表示: Workspace設定", kViewSettings, has_workspace, L"Workspaceが未選択です"},
      Entry{L"Git: Status / Branches", kGitStatus, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: ブランチ切替", kGitBranchSwitch, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Merge", kGitMerge, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"Git: Pull (fast-forward only)", kGitPull, trusted, L"Workspaceの信頼が必要です"},
      Entry{L"画像: 表示幅480 DIP", kImageWidth480, has_document, L"Markdown文書が必要です"},
      Entry{L"画像: Storageへupload", kImageUpload, trusted && has_document, L"信頼済みWorkspaceと文書が必要です"},
  };
  std::wstring label = L"コマンド名の一部を入力してください。\n";
  for (const auto& entry : entries) {
    label += L"・" + std::wstring(entry.name);
    if (!entry.enabled) label += L"（実行不可: " + std::wstring(entry.reason) + L"）";
    label += L"\n";
  }
  std::wstring query;
  if (!PromptText(window_, instance_, L"MDLite コマンドパレット", label, query) || query.empty()) return;
  std::ranges::transform(query, query.begin(), towlower);
  const Entry* match{};
  std::wstring candidates;
  for (const auto& entry : entries) {
    std::wstring name(entry.name);
    std::ranges::transform(name, name.begin(), towlower);
    if (name.find(query) == std::wstring::npos) continue;
    if (!match) match = &entry;
    else candidates += L"\n";
    candidates += entry.name;
  }
  if (!match) {
    MessageBoxW(window_, L"一致するコマンドがありません。", L"コマンドパレット", MB_ICONINFORMATION);
    return;
  }
  if (candidates.find(L'\n') != std::wstring::npos) {
    MessageBoxW(window_, (L"候補を一つに絞ってください。\n\n" + candidates).c_str(),
                L"コマンドパレット", MB_ICONINFORMATION);
    return;
  }
  if (!match->enabled) {
    MessageBoxW(window_, match->reason, L"このコマンドは実行できません", MB_ICONWARNING);
    return;
  }
  SendMessageW(window_, WM_COMMAND, MAKEWPARAM(match->command, 0), 0);
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
  if (ime_composing_ || workspace_.empty() || active_document_ >= documents_.size()) return false;
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
  if (ime_composing_ || active_document_ >= documents_.size()) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const auto source_position = view.editor_snapshot.ViewToSource(selection.cpMin);
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
  const auto begin = static_cast<LONG>(view.editor_snapshot.SourceToView(image->begin));
  const auto end = static_cast<LONG>(view.editor_snapshot.SourceToView(image->end));
  SendMessageW(view.editor, EM_SETSEL, begin, end);
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(replacement.c_str()));
}

void Application::UploadImageAtCaret() {
  if (active_document_ >= documents_.size() || !workspace_store_ || ime_composing_) return;
  auto& view = *documents_[active_document_];
  if (!IsMarkdownFile(view.document.path())) return;
  if (!IsWorkspaceTrusted(workspace_)) {
    MessageBoxW(window_, L"未信頼Workspaceではstorage adapterを実行しません。", L"画像upload", MB_ICONWARNING);
    return;
  }
  SyncDocumentFromEditor(view);
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::size_t source_position = view.editor_snapshot.ViewToSource(selection.cpMin);
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
  view.document.MarkEdited(std::move(source));
  view.editor_snapshot = SnapshotFor(view.document);
  suppress_editor_change_ = true;
  SetWindowTextW(view.editor, view.editor_snapshot.view.c_str());
  suppress_editor_change_ = false;
  ApplyMarkdownPresentation(view, true);
  const ULONGLONG now = GetTickCount64();
  view.autosave_due = settings_.auto_save ? now + settings_.auto_save_delay_ms : 0;
  view.recovery_due = now + kRecoveryDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  UpdateStatus();
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
    SendMessageW(status_, SB_SETTEXTW, 0, reinterpret_cast<LPARAM>(status.c_str()));
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
  const LONG position = static_cast<LONG>(destination_view.editor_snapshot.SourceToView(heading->begin));
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

}  // namespace mdlite
