#pragma once

#include "core/Document.h"
#include "editor/EditorAdapter.h"
#include "git/Conflict.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "settings/Settings.h"
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
  ~Application();
  bool Initialize(int show_command);
  int Run();
  void OpenInitialPath(const std::filesystem::path& path);

 private:
  struct DocumentView {
    Document document;
    HWND editor{};
    MarkdownParseResult parse;
    EditorSnapshot editor_snapshot;
    int active_line{-1};
    HWND compact_window{};
    std::shared_ptr<WorkspaceStore> workspace_store;
    ULONGLONG autosave_due{};
    ULONGLONG recovery_due{};
  };

  enum class TableAction { InsertRowBefore, InsertRowAfter, DeleteRow, InsertColumnBefore,
                           InsertColumnAfter, DeleteColumn };

  static LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
  static LRESULT CALLBACK CompactWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
  static LRESULT CALLBACK EditorSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                          UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK CalendarSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                            UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK TreeDragSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                            UINT_PTR subclass_id, DWORD_PTR reference);
  LRESULT HandleMessage(UINT message, WPARAM wparam, LPARAM lparam);

  void CreateControls();
  void LayoutControls();
  void CreateMenuBar();
  void OpenWorkspaceDialog();
  void OpenFileDialog();
  void QuickOpen();
  void OpenWorkspace(const std::filesystem::path& path);
  void PopulateWorkspaceTree();
  void AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth);
  void OpenDocument(const std::filesystem::path& path);
  void ActivateDocument(std::size_t index);
  bool SaveDocument(DocumentView& view, bool interactive);
  bool SaveDocumentAs(DocumentView& view);
  bool SaveAll(bool interactive);
  bool CloseDocumentsForWorkspaceSwitch();
  void OnEditorChanged(HWND editor);
  void SyncDocumentFromEditor(DocumentView& view);
  void ApplyMarkdownPresentation(DocumentView& view, bool force);
  void RefreshDerivedImages(DocumentView& view);
  void RebuildOutline(const DocumentView& view);
  std::wstring EditorText(HWND editor) const;
  void UpdateStatus();
  void ShowFindBar();
  void FindNext(bool restart_from_beginning = false);
  void SearchWorkspaceFromFindBar();
  void ReplaceWorkspaceFromFindBar();
  bool CloseDocument(std::size_t index);
  void CreateProfile(BuiltInProfile profile);
  void CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date);
  void ApplyTableAction(TableAction action);
  void MoveOutlineSection(std::size_t source_begin, std::size_t target_begin);
  bool SaveRecovery(DocumentView& view, bool interactive = false);
  void SaveSession();
  void CreateEmptyFile();
  void CreateFolder();
  void CopySelectedFile();
  void PasteCopiedFile();
  void RenameOrMoveSelected();
  void MoveWorkspaceSelectionTo(const std::filesystem::path& directory);
  void DeleteSelected();
  void CopySelectedPathToClipboard();
  void ShowSelectedInExplorer();
  void SetWorkspaceTrust(bool trusted);
  void RunGitStatus();
  void RunGitAction(int command);
  bool QueryGitConflicts(std::vector<std::filesystem::path>& files, std::wstring& error);
  void ShowGitConflicts();
  void NavigateGitConflict(bool previous);
  void ResolveGitConflict(ConflictChoice choice);
  void MarkGitConflictResolved();
  void SetExternalOperationActive(bool active);
  void OpenWorkspaceSettings();
  void OpenWorkspaceSettingsFiles();
  void ManageProfiles();
  void ShowCommandPalette();
  void LoadAndApplySettings();
  void ApplySettings();
  void RebuildAccelerators();
  void ToggleCompactWindow();
  void UploadImageAtCaret();
  bool PasteClipboardImage();
  void ResizeImageAtCaret(unsigned width_dip);
  void OpenLinkAtSourcePosition(DocumentView& view, std::size_t source_position, bool activate);
  void UpdateCalendarTooltip(POINT point);
  bool IsDocumentOpen(const std::filesystem::path& path) const;
  std::filesystem::path SelectedTreePath() const;
  std::vector<std::filesystem::path> SelectedTreePaths() const;

  HINSTANCE instance_{};
  HWND window_{};
  HWND workspace_tree_{};
  HWND tabs_{};
  HWND outline_{};
  HWND status_{};
  HWND find_bar_{};
  HWND find_edit_{};
  HWND find_next_{};
  HWND replace_edit_{};
  HWND find_workspace_{};
  HWND replace_workspace_{};
  HWND find_case_{};
  HWND find_regex_{};
  HWND find_word_{};
  HWND calendar_{};
  HWND calendar_tooltip_{};
  std::filesystem::path workspace_;
  std::vector<std::filesystem::path> copied_files_;
  std::vector<std::unique_ptr<std::filesystem::path>> tree_paths_;
  std::vector<std::unique_ptr<DocumentView>> documents_;
  std::size_t active_document_{static_cast<std::size_t>(-1)};
  bool suppress_editor_change_{};
  bool external_operation_active_{};
  bool ime_composing_{};
  bool outline_dragging_{};
  bool workspace_dragging_{};
  std::size_t outline_drag_source_{};
  std::vector<std::filesystem::path> workspace_drag_sources_;
  std::shared_ptr<WorkspaceStore> workspace_store_;
  HANDLE workspace_mutex_{};
  HACCEL accelerator_table_{};
  HFONT editor_font_{};
  EffectiveSettings settings_;
  std::wstring calendar_tooltip_text_;
};

}  // namespace mdlite
