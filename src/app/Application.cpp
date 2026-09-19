#include "app/Application.h"

#include "assets/Assets.h"
#include "calendar/JapaneseHolidays.h"
#include "search/Search.h"

#include <commctrl.h>
#include <commdlg.h>
#include <richedit.h>
#include <richole.h>
#include <shellapi.h>
#include <shlobj.h>
#include <shobjidl.h>
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
constexpr int kTreeWidth = 250;
constexpr int kOutlineWidth = 230;
constexpr int kTabHeight = 30;
constexpr int kFindHeight = 34;

enum ControlId : int {
  kWorkspaceTree = 100,
  kTabs,
  kEditor,
  kOutline,
  kFindBar,
  kFindEdit,
  kFindNext,
  kFileOpenWorkspace = 1000,
  kFileOpen,
  kFileSave,
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
  kViewCalendar,
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
    SetTimer(app->window_, kAutosaveTimer, kAutosaveDelayMs, nullptr);
  }
  if (message == WM_KEYDOWN && wparam == VK_TAB && !app->ime_composing_ &&
      app->active_document_ < app->documents_.size() &&
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
        KillTimer(window_, kAutosaveTimer);
        if (!ime_composing_ && active_document_ < documents_.size()) {
          SaveRecovery(*documents_[active_document_]);
          SaveDocument(*documents_[active_document_], false);
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
        ReleaseCapture();
        outline_dragging_ = false;
        const HTREEITEM target = TreeView_GetDropHilight(outline_);
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
    case WM_COMMAND: {
      const int command = LOWORD(wparam);
      if (command == kFileOpenWorkspace) OpenWorkspaceDialog();
      else if (command == kFileOpen) OpenFileDialog();
      else if (command == kFileSave && active_document_ < documents_.size())
        SaveDocument(*documents_[active_document_], true);
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
      else if (command == kFileExit) SendMessageW(window_, WM_CLOSE, 0, 0);
      else if (command == kEditFind) ShowFindBar();
      else if (command == kEditFindNext || command == kFindNext) FindNext();
      else if (command == kEditFindWorkspace) SearchWorkspaceFromFindBar();
      else if (command == kViewCalendar) {
        ShowWindow(calendar_, IsWindowVisible(calendar_) ? SW_HIDE : SW_SHOW);
        LayoutControls();
      }
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
          const auto position = static_cast<LONG>(item.lParam);
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
  AppendMenuW(file, MF_STRING, kFileSave, L"保存\tCtrl+S");
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
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(workspace), L"Workspace");
  HMENU edit = CreatePopupMenu();
  AppendMenuW(edit, MF_STRING, kEditFind, L"検索…\tCtrl+F");
  AppendMenuW(edit, MF_STRING, kEditFindNext, L"次を検索\tF3");
  AppendMenuW(edit, MF_STRING, kEditFindWorkspace, L"Workspaceを検索");
  AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(edit), L"編集");
  HMENU view = CreatePopupMenu();
  AppendMenuW(view, MF_STRING, kViewCalendar, L"カレンダー");
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
                                 TVS_LINESATROOT | TVS_SHOWSELALWAYS | TVS_DISABLEDRAGDROP,
                             0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kOutline), instance_, nullptr);
  status_ = CreateWindowExW(0, STATUSCLASSNAMEW, nullptr, WS_CHILD | WS_VISIBLE,
                            0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  find_bar_ = CreateWindowExW(WS_EX_CLIENTEDGE, L"STATIC", nullptr, WS_CHILD,
                              0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kFindBar), instance_, nullptr);
  find_edit_ = CreateWindowExW(0, L"EDIT", nullptr, WS_CHILD | WS_VISIBLE | WS_BORDER | ES_AUTOHSCROLL,
                               8, 5, 260, 24, find_bar_, reinterpret_cast<HMENU>(kFindEdit), instance_, nullptr);
  find_next_ = CreateWindowExW(0, L"BUTTON", L"次を検索", WS_CHILD | WS_VISIBLE | BS_PUSHBUTTON,
                               276, 4, 90, 25, find_bar_, reinterpret_cast<HMENU>(kFindNext), instance_, nullptr);
  calendar_ = CreateWindowExW(WS_EX_CLIENTEDGE, MONTHCAL_CLASSW, nullptr,
                              WS_CHILD | MCS_DAYSTATE | MCS_WEEKNUMBERS,
                              0, 0, 0, 0, window_, nullptr, instance_, nullptr);
  SendMessageW(calendar_, MCM_SETFIRSTDAYOFWEEK, 0, 6);
  HFONT font = static_cast<HFONT>(GetStockObject(DEFAULT_GUI_FONT));
  for (HWND control : {workspace_tree_, tabs_, outline_, status_, find_edit_, find_next_})
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

void Application::OpenWorkspace(const std::filesystem::path& path) {
  std::error_code error;
  const auto absolute = std::filesystem::weakly_canonical(path, error);
  if (error || !std::filesystem::is_directory(absolute)) {
    MessageBoxW(window_, L"Workspaceフォルダーを開けません。", L"MDLite", MB_ICONERROR);
    return;
  }
  workspace_ = absolute;
  workspace_store_ = std::make_unique<WorkspaceStore>(workspace_);
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
  std::wstring load_error;
  if (!view->document.Load(absolute, load_error)) {
    MessageBoxW(window_, load_error.c_str(), L"ファイルを開けません", MB_ICONERROR);
    return;
  }
  view->editor = CreateWindowExW(WS_EX_CLIENTEDGE, MSFTEDIT_CLASS, nullptr,
                                  WS_CHILD | WS_VSCROLL | WS_HSCROLL | ES_MULTILINE |
                                      ES_AUTOVSCROLL | ES_AUTOHSCROLL | ES_NOHIDESEL |
                                      ES_WANTRETURN,
                                  0, 0, 0, 0, window_, reinterpret_cast<HMENU>(kEditor), instance_, nullptr);
  SendMessageW(view->editor, EM_SETEVENTMASK, 0, ENM_CHANGE | ENM_SELCHANGE | ENM_UPDATE | ENM_SCROLL);
  SendMessageW(view->editor, EM_SETBKGNDCOLOR, 0, RGB(255, 255, 255));
  SetWindowSubclass(view->editor, EditorSubclass, 1, reinterpret_cast<DWORD_PTR>(this));
  suppress_editor_change_ = true;
  SetWindowTextW(view->editor, view->document.text().c_str());
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

bool Application::SaveDocument(DocumentView& view, bool interactive) {
  if (ime_composing_) return false;
  view.document.MarkEditedFromEditor(EditorText(view.editor));
  if (!view.document.dirty()) return true;
  std::wstring error;
  if (!view.document.Save(error)) {
    UpdateStatus();
    if (interactive) MessageBoxW(window_, error.c_str(), L"保存できません", MB_ICONWARNING);
    return false;
  }
  if (workspace_store_) {
    std::wstring recovery_error;
    workspace_store_->RemoveRecovery(view.document.path(), recovery_error);
  }
  UpdateStatus();
  return true;
}

bool Application::SaveAll(bool interactive) {
  bool result = true;
  for (auto& view : documents_) {
    if (!SaveDocument(*view, interactive)) result = false;
  }
  if (!result && interactive) {
    return MessageBoxW(window_, L"保存できない文書があります。編集内容を保持したまま終了しますか？",
                       L"MDLite", MB_ICONWARNING | MB_YESNO | MB_DEFBUTTON2) == IDYES;
  }
  return result;
}

void Application::OnEditorChanged(HWND editor) {
  if (suppress_editor_change_ || ime_composing_) return;
  for (auto& view : documents_) {
    if (view->editor != editor) continue;
    view->document.MarkEditedFromEditor(EditorText(editor));
    view->parse = ParseMarkdown(view->document.text());
    ApplyMarkdownPresentation(*view, true);
    if (active_document_ < documents_.size() && documents_[active_document_].get() == view.get()) RebuildOutline(*view);
    SetTimer(window_, kAutosaveTimer, kAutosaveDelayMs, nullptr);
    UpdateStatus();
    break;
  }
}

void Application::ApplyMarkdownPresentation(DocumentView& view, bool force) {
  if (ime_composing_) return;
  CHARRANGE selection{};
  SendMessageW(view.editor, EM_EXGETSEL, 0, reinterpret_cast<LPARAM>(&selection));
  const int active_line = static_cast<int>(SendMessageW(view.editor, EM_LINEFROMCHAR, selection.cpMin, 0));
  if (!force && view.active_line == active_line) return;
  view.active_line = active_line;
  view.parse = ParseMarkdown(EditorText(view.editor));

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
    if (span.begin >= static_cast<std::size_t>(length)) continue;
    SendMessageW(view.editor, EM_SETSEL, static_cast<WPARAM>(span.begin), static_cast<LPARAM>(span.end));
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
      const bool intersects_active = static_cast<LONG>(span.end) >= active_start &&
                                     static_cast<LONG>(span.begin) <= active_end;
      if (!intersects_active) {
        format.dwMask |= CFM_HIDDEN;
        format.dwEffects |= CFE_HIDDEN;
      } else {
        format.crTextColor = RGB(128, 128, 128);
      }
    }
    SendMessageW(view.editor, EM_SETCHARFORMAT, SCF_SELECTION, reinterpret_cast<LPARAM>(&format));
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
  if (!SearchWorkspace(workspace_, {query_text, false, false}, unsaved, matches, error)) {
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
    const LONG line_start = static_cast<LONG>(SendMessageW(
        editor, EM_LINEINDEX, static_cast<WPARAM>(matches.front().line - 1), 0));
    const LONG begin = line_start + static_cast<LONG>(matches.front().column - 1);
    const LONG end = begin + static_cast<LONG>(matches.front().end - matches.front().begin);
    SendMessageW(editor, EM_SETSEL, begin, end);
    SendMessageW(editor, EM_SCROLLCARET, 0, 0);
  }
  const std::wstring message = std::to_wstring(matches.size()) +
                               L"件見つかりました。最初の一致を開きました。";
  MessageBoxW(window_, message.c_str(), L"Workspace検索", MB_ICONINFORMATION);
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
  if (active_document_ >= documents_.size()) return;
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
  if (active_document_ >= documents_.size() || source_begin == target_begin) return;
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

void Application::SaveRecovery(DocumentView& view) {
  if (!workspace_store_ || !view.document.dirty()) return;
  std::wstring error;
  if (!workspace_store_->WriteRecovery(view.document.path(), view.document.text(), error)) {
    SendMessageW(status_, SB_SETTEXTW, 0, reinterpret_cast<LPARAM>(error.c_str()));
  }
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
  if (IsDocumentOpen(path)) {
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

std::filesystem::path Application::SelectedTreePath() const {
  TVITEMW item{};
  item.mask = TVIF_PARAM;
  item.hItem = TreeView_GetSelection(workspace_tree_);
  if (item.hItem == nullptr || !TreeView_GetItem(workspace_tree_, &item) || item.lParam == 0) return {};
  return *reinterpret_cast<const std::filesystem::path*>(item.lParam);
}

}  // namespace mdlite
