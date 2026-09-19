#pragma once

#include "core/Document.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "table/Table.h"
#include "workspace/Workspace.h"

#include <windows.h>
#include <commctrl.h>

#include <filesystem>
#include <memory>
#include <string>
#include <vector>

namespace mdlite {

class Application {
 public:
  explicit Application(HINSTANCE instance);
  bool Initialize(int show_command);
  int Run();
  void OpenInitialPath(const std::filesystem::path& path);

 private:
  struct DocumentView {
    Document document;
    HWND editor{};
    MarkdownParseResult parse;
    int active_line{-1};
  };

  enum class TableAction { InsertRowBefore, InsertRowAfter, DeleteRow, InsertColumnBefore,
                           InsertColumnAfter, DeleteColumn };

  static LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
  static LRESULT CALLBACK EditorSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                          UINT_PTR subclass_id, DWORD_PTR reference);
  LRESULT HandleMessage(UINT message, WPARAM wparam, LPARAM lparam);

  void CreateControls();
  void LayoutControls();
  void CreateMenuBar();
  void OpenWorkspaceDialog();
  void OpenFileDialog();
  void OpenWorkspace(const std::filesystem::path& path);
  void PopulateWorkspaceTree();
  void AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth);
  void OpenDocument(const std::filesystem::path& path);
  void ActivateDocument(std::size_t index);
  bool SaveDocument(DocumentView& view, bool interactive);
  bool SaveAll(bool interactive);
  void OnEditorChanged(HWND editor);
  void ApplyMarkdownPresentation(DocumentView& view, bool force);
  void RebuildOutline(const DocumentView& view);
  std::wstring EditorText(HWND editor) const;
  void UpdateStatus();
  void ShowFindBar();
  void FindNext(bool restart_from_beginning = false);
  void SearchWorkspaceFromFindBar();
  void CreateProfile(BuiltInProfile profile);
  void CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date);
  void ApplyTableAction(TableAction action);
  void MoveOutlineSection(std::size_t source_begin, std::size_t target_begin);
  void SaveRecovery(DocumentView& view);
  void SaveSession();
  void CreateEmptyFile();
  void CreateFolder();
  void CopySelectedFile();
  void PasteCopiedFile();
  void RenameOrMoveSelected();
  void DeleteSelected();
  void CopySelectedPathToClipboard();
  void ShowSelectedInExplorer();
  bool IsDocumentOpen(const std::filesystem::path& path) const;
  std::filesystem::path SelectedTreePath() const;

  HINSTANCE instance_{};
  HWND window_{};
  HWND workspace_tree_{};
  HWND tabs_{};
  HWND outline_{};
  HWND status_{};
  HWND find_bar_{};
  HWND find_edit_{};
  HWND find_next_{};
  HWND calendar_{};
  std::filesystem::path workspace_;
  std::filesystem::path copied_file_;
  std::vector<std::unique_ptr<std::filesystem::path>> tree_paths_;
  std::vector<std::unique_ptr<DocumentView>> documents_;
  std::size_t active_document_{static_cast<std::size_t>(-1)};
  bool suppress_editor_change_{};
  bool ime_composing_{};
  bool outline_dragging_{};
  std::size_t outline_drag_source_{};
  std::unique_ptr<WorkspaceStore> workspace_store_;
};

}  // namespace mdlite
