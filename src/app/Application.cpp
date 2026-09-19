#include "app/Application.h"

#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "calendar/JapaneseHolidays.h"
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

#include <algorithm>
#include <array>
#include <cwctype>
#include <map>

namespace mdlite {
namespace {

constexpr wchar_t kWindowClass[] = L"MDLite.MainWindow";
constexpr UINT_PTR kAutosaveTimer = 1;
constexpr UINT kAutosaveDelayMs = 750;
constexpr UINT kTimerPollMs = 250;
constexpr UINT kRecoveryDelayMs = 5000;
constexpr int kTreeWidth = 250;
constexpr int kOutlineWidth = 230;
constexpr int kTabHeight = 30;
constexpr int kFindHeight = 66;

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
  kFileOpen,
  kFileQuickOpen,
  kFileClose,
  kFileSave,
  kFileSaveAs,
  kFileDaily,
  kFileMeeting,
  kFileMemo,
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
  kWorkspaceTrust,
  kWorkspaceUntrust,
  kGitStatus,
  kImageUpload,
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

}  // namespace

Application::Application(HINSTANCE instance) : instance_(instance) {}

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

  window_ = CreateWindowExW(0, kWindowClass, L"MDLite", WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                            CW_USEDEFAULT, CW_USEDEFAULT, 1280, 800, nullptr, nullptr, instance_, this);
  if (window_ == nullptr) return false;
  ShowWindow(window_, show_command);
  UpdateWindow(window_);
  return true;
}

int Application::Run() {
  const ACCEL accelerators[] = {
      {FVIRTKEY | FCONTROL, 'O', kFileOpen},
      {FVIRTKEY | FCONTROL, 'S', kFileSave},
      {FVIRTKEY | FCONTROL, 'F', kEditFind},
      {FVIRTKEY | FCONTROL, 'P', kFileQuickOpen},
      {FVIRTKEY | FCONTROL, 'W', kFileClose},
      {FVIRTKEY, VK_F3, kEditFindNext},
  };
  HACCEL table = CreateAcceleratorTableW(const_cast<ACCEL*>(accelerators),
                                         static_cast<int>(std::size(accelerators)));
  MSG message{};
  while (GetMessageW(&message, nullptr, 0, 0) > 0) {
    if (!TranslateAcceleratorW(window_, table, &message)) {
      TranslateMessage(&message);
      DispatchMessageW(&message);
    }
  }
  DestroyAcceleratorTable(table);
  return static_cast<int>(message.wParam);
}

void Application::OpenInitialPath(const std::filesystem::path& path) {
  std::error_code error;
  if (std::filesystem::is_directory(path, error)) OpenWorkspace(path);
  else if (std::filesystem::is_regular_file(path, error)) {
    OpenWorkspace(path.parent_path());
    OpenDocument(path);
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

LRESULT CALLBACK Application::EditorSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                               UINT_PTR, DWORD_PTR reference) {
  auto* app = reinterpret_cast<Application*>(reference);
  if (message == WM_IME_STARTCOMPOSITION) app->ime_composing_ = true;
  if (message == WM_IME_ENDCOMPOSITION) {
    app->ime_composing_ = false;
    app->OnEditorChanged(window);
  }
  if (message == WM_KEYDOWN && wparam == VK_TAB && !app->ime_composing_ &&
      app->active_document_ < app->documents_.size() &&
      IsMarkdownFile(app->documents_[app->active_document_]->document.path()) &&
      app->documents_[app->active_document_]->editor == window) {
    CHARRANGE selection{};
    SendMessageW(window, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
    const auto edit = MoveToAdjacentTableCell(app->EditorText(window),
                                              static_cast<std::size_t>(selection.cpMin),
                                              (GetKeyState(VK_SHIFT) & 0x8000) != 0);
    if (edit.changed) {
      app->suppress_editor_change_ = true;
      SendMessageW(window, EM_SETSEL, 0, -1);
      SendMessageW(window, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(edit.text.c_str()));
      app->suppress_editor_change_ = false;
      app->OnEditorChanged(window);
    }
    if (edit.selection != static_cast<std::size_t>(selection.cpMin) || edit.changed) {
      SendMessageW(window, EM_SETSEL, edit.selection, edit.selection);
      return 0;
    }
  }
  if (message == WM_NCDESTROY) RemoveWindowSubclass(window, EditorSubclass, 1);
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
        const ULONGLONG now = GetTickCount64();
        bool pending = false;
        for (auto& view : documents_) {
          if (!view->document.dirty()) continue;
          pending = true;
          const bool composing_this_view = ime_composing_ && active_document_ < documents_.size() &&
                                           documents_[active_document_].get() == view.get();
          if (composing_this_view) continue;
          if (view->recovery_due != 0 && now >= view->recovery_due) {
            SaveRecovery(*view);
            view->recovery_due = now + kRecoveryDelayMs;
          }
          if (view->autosave_due != 0 && now >= view->autosave_due) {
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
      return DefWindowProcW(window_, message, wparam, lparam);
    case WM_CAPTURECHANGED:
      if (outline_dragging_) {
        outline_dragging_ = false;
        TreeView_SelectDropTarget(outline_, nullptr);
      }
      return 0;
    case WM_COMMAND: {
      const int command = LOWORD(wparam);
      if (command == kFileOpenWorkspace) OpenWorkspaceDialog();
      else if (command == kFileOpen) OpenFileDialog();
      else if (command == kFileQuickOpen) QuickOpen();
      else if (command == kFileClose && active_document_ < documents_.size()) CloseDocument(active_document_);
      else if (command == kFileSave && active_document_ < documents_.size())
        SaveDocument(*documents_[active_document_], true);
      else if (command == kFileSaveAs && active_document_ < documents_.size())
        SaveDocumentAs(*documents_[active_document_]);
      else if (command == kFileDaily) CreateProfile(BuiltInProfile::Daily);
      else if (command == kFileMeeting) CreateProfile(BuiltInProfile::Meeting);
      else if (command == kFileMemo) CreateProfile(BuiltInProfile::Memo);
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
      else if (command == kImageUpload) UploadImageAtCaret();
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
      } else if (header->hwndFrom == workspace_tree_ && header->code == NM_DBLCLK) {
        const auto path = SelectedTreePath();
        if (!path.empty() && std::filesystem::is_regular_file(path)) OpenDocument(path);
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
      }
      return 0;
    }
    case WM_CLOSE:
      if (!SaveAll(true)) return 0;
      SaveSession();
      DestroyWindow(window_);
      return 0;
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
  AppendMenuW(file, MF_STRING, kFileOpen, L"ファイルを開く…\tCtrl+O");
  AppendMenuW(file, MF_STRING, kFileQuickOpen, L"Quick Open…\tCtrl+P");
  AppendMenuW(file, MF_STRING, kFileClose, L"タブを閉じる\tCtrl+W");
  AppendMenuW(file, MF_STRING, kFileSave, L"保存\tCtrl+S");
  AppendMenuW(file, MF_STRING, kFileSaveAs, L"別名で保存…");
  AppendMenuW(file, MF_SEPARATOR, 0, nullptr);
  AppendMenuW(file, MF_STRING, kFileDaily, L"今日のDairyを開く");
  AppendMenuW(file, MF_STRING, kFileMeeting, L"Meetingノートを作成");
  AppendMenuW(file, MF_STRING, kFileMemo, L"Memoを作成");
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
  AppendMenuW(image, MF_STRING, kImageUpload, L"カーソル位置の画像をstorageへupload…");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(image), L"画像");
  HMENU git = CreatePopupMenu();
  AppendMenuW(git, MF_STRING, kGitStatus, L"Status / Branches…");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(git), L"Git");
  SetMenu(window_, menu);
}

void Application::CreateControls() {
  workspace_tree_ = CreateWindowExW(WS_EX_CLIENTEDGE, WC_TREEVIEWW, nullptr,
                                    WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_HASLINES |
                                        TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                                    0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kWorkspaceTree), instance_, nullptr);
  tabs_ = CreateWindowExW(0, WC_TABCONTROLW, nullptr, WS_CHILD | WS_VISIBLE | TCS_TABS,
                          0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kTabs), instance_, nullptr);
  outline_ = CreateWindowExW(WS_EX_CLIENTEDGE, WC_TREEVIEWW, nullptr,
                             WS_CHILD | WS_VISIBLE | TVS_HASBUTTONS | TVS_HASLINES |
                                 TVS_LINESATROOT | TVS_SHOWSELALWAYS,
                             0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kOutline), instance_, nullptr);
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
  for (auto& view : documents_)
    MoveWindow(view->editor, center_left, editor_top, center_width,
               std::max(0, content_height - editor_top), TRUE);
  if (calendar_) {
    RECT required{};
    MonthCal_GetMinReqRect(calendar_, &required);
    const int width = required.right - required.left;
    const int height = required.bottom - required.top;
    MoveWindow(calendar_, std::max(center_left, static_cast<int>(client.right) - kOutlineWidth - width - 8),
               editor_top + 8, width, height, TRUE);
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
  if (GetOpenFileNameW(&dialog)) {
    if (workspace_.empty()) OpenWorkspace(std::filesystem::path(path).parent_path());
    OpenDocument(path);
  }
}

void Application::QuickOpen() {
  wchar_t path[32768]{};
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrTitle = L"Quick Open — Workspace内の文書を選択";
  dialog.lpstrFilter = L"Markdown・テキスト\0*.md;*.markdown;*.txt;*.log;*.json;*.toml;*.yaml;*.yml\0すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  const std::wstring initial = workspace_.wstring();
  dialog.lpstrInitialDir = initial.empty() ? nullptr : initial.c_str();
  dialog.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
  if (GetOpenFileNameW(&dialog)) OpenDocument(path);
}

void Application::OpenWorkspace(const std::filesystem::path& path) {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error || !std::filesystem::is_directory(absolute)) {
    MessageBoxW(window_, L"Workspaceフォルダーを開けません。", L"MDLite", MB_ICONERROR);
    return;
  }
  if (workspace_ == absolute) return;
  HANDLE new_mutex = CreateMutexW(nullptr, FALSE, WorkspaceMutexName(absolute).c_str());
  if (new_mutex == nullptr || GetLastError() == ERROR_ALREADY_EXISTS) {
    if (new_mutex) CloseHandle(new_mutex);
    MessageBoxW(window_, L"このWorkspaceは別のMDLiteウィンドウで既に開かれています。",
                L"Workspace重複起動", MB_ICONWARNING);
    return;
  }
  if (!workspace_.empty() && !CloseDocumentsForWorkspaceSwitch()) {
    CloseHandle(new_mutex);
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
      if (!recoveries.empty() && MessageBoxW(window_,
            L"前回の復旧スナップショットがあります。内容を別タブで開きますか？",
            L"MDLite 復旧", MB_ICONWARNING | MB_YESNO) == IDYES) {
        for (const auto& recovery : recoveries) OpenDocument(recovery);
      }
      std::vector<std::filesystem::path> session_documents;
      if (!workspace_store_->ReadSession(session_documents, initialize_error)) {
        MessageBoxW(window_, initialize_error.c_str(), L"セッション復元", MB_ICONWARNING);
      } else {
        for (const auto& document : session_documents) OpenDocument(document);
      }
    }
  }
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
    if (entry.is_directory()) AddTreeDirectory(item, entry.path(), depth + 1);
  }
}

void Application::OpenDocument(const std::filesystem::path& path) {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error || !IsTextFile(absolute)) return;
  for (std::size_t i = 0; i < documents_.size(); ++i) {
    if (documents_[i]->document.path() == absolute) {
      ActivateDocument(i);
      return;
    }
  }
  auto view = std::make_unique<DocumentView>();
  view->workspace_store = workspace_store_;
  std::wstring load_error;
  if (!view->document.Load(absolute, load_error)) {
    MessageBoxW(window_, load_error.c_str(), L"ファイルを開けません", MB_ICONERROR);
    return;
  }
  view->editor_snapshot = SnapshotFor(view->document);
  view->editor = CreateWindowExW(WS_EX_CLIENTEDGE, MSFTEDIT_CLASS, nullptr,
                                  WS_CHILD | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE |
                                      ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL |
                                      ES_WANTRETURN,
                                  0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kEditor), instance_, nullptr);
  SendMessageW(view->editor, EM_SETEVENTMASK, 0, ENM_CHANGE | ENM_SELCHANGE | ENM_UPDATE | ENM_SCROLL);
  SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, RGB(255, 255, 255));
  SetWindowSubclass(view->editor, EditorSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  suppress_editor_change_ = true;
  SetWindowTextW(view->editor, view->editor_snapshot.view.c_str());
  suppress_editor_change_ = false;

  TCITEMW tab{};
  tab.mask = TCIF_TEXT;
  std::wstring name = absolute.filename().wstring();
  tab.pszText = name.data();
  TabCtrl_InsertItem(tabs_, static_cast<int>(documents_.size()), &tab);
  documents_.push_back(std::move(view));
  ActivateDocument(documents_.size() - 1);
}

void Application::ActivateDocument(std::size_t index) {
  if (index >= documents_.size()) return;
  for (std::size_t i = 0; i < documents_.size(); ++i) ShowWindow(documents_[i]->editor, i == index ? SW_SHOW : SW_HIDE);
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
  std::wstring suggested = view.document.path().stem().wstring() + L"-recovered" +
                           view.document.path().extension().wstring();
  wcsncpy_s(path, suggested.c_str(), _TRUNCATE);
  OPENFILENAMEW dialog{sizeof(dialog)};
  dialog.hwndOwner = window_;
  dialog.lpstrFilter = L"Markdown・テキスト\0*.md;*.markdown;*.txt;*.log;*.json;*.toml;*.yaml;*.yml\0すべて\0*.*\0";
  dialog.lpstrFile = path;
  dialog.nMaxFile = static_cast<DWORD>(std::size(path));
  const std::wstring initial_directory = view.document.path().parent_path().wstring();
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

bool Application::CloseDocumentsForWorkspaceSwitch() {
  if (!SaveAll(true)) return false;
  for (auto& view : documents_) DestroyWindow(view->editor);
  documents_.clear();
  TabCtrl_DeleteAllItems(tabs_);
  TreeView_DeleteAllItems(outline_);
  active_document_ = static_cast<std::size_t>(-1);
  return true;
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
    view->autosave_due = now + kAutosaveDelayMs;
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
  const auto parsed = ParseMarkdown(view.document.text());
  if (parsed.images.empty()) return;
  IRichEditOle* rich_edit = nullptr;
  if (!SendMessageW(view.editor, EM_GETOLEINTERFACE, 0, reinterpret_cast<LPARAM>(&rich_edit)) || !rich_edit) return;
  const LONG existing = rich_edit->GetObjectCount();
  rich_edit->Release();
  if (existing == static_cast<LONG>(parsed.images.size())) return;
  PresentationUndoGuard undo_guard(view.editor);
  suppress_editor_change_ = true;
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
    if (FAILED(SHCreateStreamOnFileEx(target.c_str(), STGM_READ | STGM_SHARE_DENY_WRITE,
                                      FILE_ATTRIBUTE_NORMAL, FALSE, nullptr, &stream))) continue;
    const LONG position = static_cast<LONG>(view.editor_snapshot.SourceToView(image.begin));
    SendMessageW(view.editor, EM_SETSEL, position, position + 1);
    SendMessageW(view.editor, EM_REPLACESEL, FALSE, reinterpret_cast<LPARAM>(L""));
    const unsigned width = image.width_dip == 0 ? 320U : image.width_dip;
    RICHEDIT_IMAGE_PARAMETERS parameters{};
    parameters.xWidth = static_cast<LONG>(width * 2540U / 96U);
    parameters.yHeight = static_cast<LONG>(width * 9U / 16U * 2540U / 96U);
    parameters.Ascent = parameters.yHeight;
    parameters.Type = TA_BASELINE;
    parameters.pwszAlternateText = image.alternate_text.c_str();
    parameters.pIStream = stream;
    SendMessageW(view.editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters));
    stream->Release();
  }
  suppress_editor_change_ = false;
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
  CHARFORMAT2W normal{sizeof(normal)};
  normal.dwMask = CFM_FACE | CFM_SIZE | CFM_COLOR | CFM_BOLD | CFM_ITALIC | CFM_STRIKEOUT |
                  CFM_HIDDEN | CFM_BACKCOLOR;
  normal.dwEffects = CFE_AUTOCOLOR | CFE_AUTOBACKCOLOR;
  normal.yHeight = 220;
  wcscpy_s(normal.szFaceName, L"Segoe UI");
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
    format.crTextColor = RGB(38, 90, 140);
    if (span.kind == SpanKind::Heading) {
      format.dwMask |= CFM_SIZE | CFM_BOLD;
      format.dwEffects |= CFE_BOLD;
      format.yHeight = std::max(260, 440 - span.level * 30);
    } else if (span.kind == SpanKind::Strong) {
      format.dwMask |= CFM_BOLD;
      format.dwEffects |= CFE_BOLD;
    } else if (span.kind == SpanKind::Strike) {
      format.dwMask |= CFM_STRIKEOUT;
      format.dwEffects |= CFE_STRIKEOUT;
    } else if (span.kind == SpanKind::Code || span.kind == SpanKind::CodeFence) {
      format.dwMask |= CFM_FACE | CFM_BACKCOLOR;
      wcscpy_s(format.szFaceName, L"Cascadia Mono");
      format.crBackColor = RGB(242, 242, 242);
    } else if (span.kind == SpanKind::HeadingMarker || span.kind == SpanKind::EmphasisMarker) {
      const bool intersects_active = static_cast<LONG>(view_end) >= active_start &&
                                     static_cast<LONG>(view_begin) <= active_end;
      if (!intersects_active) {
        format.dwMask |= CFM_HIDDEN;
        format.dwEffects |= CFE_HIDDEN;
      } else {
        format.crTextColor = RGB(128, 128, 128);
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
    table_format.crBackColor = RGB(244, 247, 250);
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
  if (!SearchWorkspace(workspace_, query, unsaved, matches, error)) {
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
  if (!PreviewWorkspaceReplace(workspace_, query, replacement, {}, plan, error)) {
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
  const bool complete = ApplyWorkspaceReplace(workspace_, plan, result, error);
  for (auto& view : documents_) {
    if (std::ranges::any_of(plan.files, [&](const auto& item) { return item.path == view->document.path(); })) {
      Document reloaded;
      std::wstring load_error;
      if (reloaded.Load(view->document.path(), load_error)) {
        view->document = std::move(reloaded);
        view->editor_snapshot = SnapshotFor(view->document);
        suppress_editor_change_ = true;
        SetWindowTextW(view->editor, view->editor_snapshot.view.c_str());
        suppress_editor_change_ = false;
        ApplyMarkdownPresentation(*view, true);
      }
    }
  }
  PopulateWorkspaceTree();
  const std::wstring message = std::to_wstring(result.applied_files) + L"ファイル、" +
      std::to_wstring(result.applied_replacements) + L"箇所を置換しました。\njournal: " +
      result.journal.wstring() + (complete ? L"" : L"\n競合したファイルは変更していません。");
  MessageBoxW(window_, message.c_str(), L"Workspace置換", complete ? MB_ICONINFORMATION : MB_ICONWARNING);
}

void Application::CreateProfile(BuiltInProfile profile) {
  SYSTEMTIME date{};
  GetLocalTime(&date);
  CreateProfileForDate(profile, date);
}

void Application::CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date) {
  if (workspace_.empty() || !workspace_store_) {
    MessageBoxW(window_, L"先にWorkspaceを開き、.mdliteの作成を許可してください。",
                L"ノート作成", MB_ICONINFORMATION);
    return;
  }
  NoteCreationResult result{};
  std::wstring error;
  if (!CreateProfileNote(workspace_, profile, date, result, error)) {
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
  HWND editor = documents_[active_document_]->editor;
  CHARRANGE selection{};
  SendMessageW(editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const std::wstring source = EditorText(editor);
  TableEditResult edit;
  switch (action) {
    case TableAction::InsertRowBefore: edit = InsertTableRow(source, selection.cpMin, false); break;
    case TableAction::InsertRowAfter: edit = InsertTableRow(source, selection.cpMin, true); break;
    case TableAction::DeleteRow: edit = DeleteTableRow(source, selection.cpMin); break;
    case TableAction::InsertColumnBefore: edit = InsertTableColumn(source, selection.cpMin, false); break;
    case TableAction::InsertColumnAfter: edit = InsertTableColumn(source, selection.cpMin, true); break;
    case TableAction::DeleteColumn: edit = DeleteTableColumn(source, selection.cpMin); break;
  }
  if (!edit.changed) {
    MessageBoxW(window_, L"カーソルをMarkdown表のセル内へ移動してください。",
                L"表の編集", MB_ICONINFORMATION);
    return;
  }
  suppress_editor_change_ = true;
  SendMessageW(editor, EM_SETSEL, 0, -1);
  SendMessageW(editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(edit.text.c_str()));
  suppress_editor_change_ = false;
  SendMessageW(editor, EM_SETSEL, edit.selection, edit.selection);
  OnEditorChanged(editor);
}

void Application::MoveOutlineSection(std::size_t source_begin, std::size_t target_begin) {
  if (active_document_ >= documents_.size() || ime_composing_ || source_begin == target_begin ||
      !IsMarkdownFile(documents_[active_document_]->document.path())) return;
  auto& view = *documents_[active_document_];
  const std::wstring source = EditorText(view.editor);
  const auto edit = MoveHeadingSection(source, source_begin, target_begin);
  if (!edit.changed) return;

  suppress_editor_change_ = true;
  SendMessageW(view.editor, EM_SETSEL, 0, -1);
  SendMessageW(view.editor, EM_REPLACESEL, TRUE, reinterpret_cast<LPARAM>(edit.text.c_str()));
  suppress_editor_change_ = false;
  SendMessageW(view.editor, EM_SETSEL, edit.selection, edit.selection);
  OnEditorChanged(view.editor);
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
  std::vector<std::filesystem::path> paths;
  for (const auto& view : documents_) paths.push_back(view->document.path());
  std::wstring error;
  if (!workspace_store_->WriteSession(paths, error)) {
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
  const auto path = SelectedTreePath();
  if (path.empty() || !std::filesystem::is_regular_file(path)) {
    MessageBoxW(window_, L"コピーするファイルを選択してください。", L"コピー", MB_ICONINFORMATION);
    return;
  }
  copied_file_ = path;
}

void Application::PasteCopiedFile() {
  if (copied_file_.empty() || !std::filesystem::is_regular_file(copied_file_)) return;
  auto directory = SelectedTreePath();
  if (directory.empty()) directory = workspace_;
  else if (!std::filesystem::is_directory(directory)) directory = directory.parent_path();
  const auto destination = directory / copied_file_.filename();
  if (!CopyFileW(copied_file_.c_str(), destination.c_str(), TRUE)) {
    MessageBoxW(window_, L"同名ファイルがあるため貼り付けませんでした。既存内容は変更していません。",
                L"貼り付け", MB_ICONWARNING);
    return;
  }
  PopulateWorkspaceTree();
}

void Application::RenameOrMoveSelected() {
  const auto source = SelectedTreePath();
  if (source.empty() || !std::filesystem::is_regular_file(source)) {
    MessageBoxW(window_, L"名前変更または移動するファイルを選択してください。",
                L"名前変更・移動", MB_ICONINFORMATION);
    return;
  }
  if (IsDocumentOpen(source)) {
    MessageBoxW(window_, L"開いている文書は保存してタブを閉じてから移動してください。",
                L"本文保護", MB_ICONWARNING);
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

void Application::DeleteSelected() {
  const auto path = SelectedTreePath();
  if (path.empty()) return;
  const auto is_open_or_parent = [&](const auto& view) {
    if (view->document.path() == path) return true;
    if (!std::filesystem::is_directory(path)) return false;
    std::error_code relative_error;
    const auto relative = std::filesystem::relative(view->document.path(), path, relative_error);
    return !relative_error && !relative.empty() && !relative.native().starts_with(L"..");
  };
  if (std::ranges::any_of(documents_, is_open_or_parent)) {
    MessageBoxW(window_, L"開いている文書は保存してタブを閉じてから削除してください。",
                L"本文保護", MB_ICONWARNING);
    return;
  }
  std::wstring from = path.wstring();
  from.push_back(L'\0');
  SHFILEOPSTRUCTW operation{};
  operation.hwnd = window_;
  operation.wFunc = FO_DELETE;
  operation.pFrom = from.c_str();
  operation.fFlags = FOF_ALLOWUNDO | FOF_WANTNUKEWARNING;
  if (SHFileOperationW(&operation) == 0 && !operation.fAnyOperationsAborted) PopulateWorkspaceTree();
}

void Application::CopySelectedPathToClipboard() {
  const auto path = SelectedTreePath();
  if (path.empty() || !OpenClipboard(window_)) return;
  EmptyClipboard();
  const std::wstring value = path.wstring();
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
  wchar_t git_path[32768]{};
  if (SearchPathW(nullptr, L"git.exe", nullptr, static_cast<DWORD>(std::size(git_path)), git_path, nullptr) == 0) {
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

void Application::OpenWorkspaceSettings() {
  if (!workspace_store_) return;
  const auto root = workspace_store_->metadata_root();
  for (const auto& relative : {L"workspace.toml", L"profiles.toml", L"keybindings.toml", L"commands.toml"}) {
    const auto path = root / relative;
    if (std::filesystem::is_regular_file(path)) OpenDocument(path);
  }
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
  view.autosave_due = now + kAutosaveDelayMs;
  view.recovery_due = now + kRecoveryDelayMs;
  SetTimer(window_, kAutosaveTimer, kTimerPollMs, nullptr);
  UpdateStatus();
}

std::filesystem::path Application::SelectedTreePath() const {
  TVITEMW item{};
  item.mask = TVIF_PARAM;
  item.hItem = TreeView_GetSelection(workspace_tree_);
  if (item.hItem == nullptr || !TreeView_GetItem(workspace_tree_, &item) || item.lParam == 0) return {};
  return *reinterpret_cast<const std::filesystem::path*>(item.lParam);
}

}  // namespace mdlite
