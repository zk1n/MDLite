#pragma once

#include "core/Document.h"
#include "editor/EditorAdapter.h"
#include "git/Conflict.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "search/Search.h"
#include "settings/Settings.h"
#include "table/Table.h"
#include "workspace/Workspace.h"

#include <windows.h>
#include <commctrl.h>

#include <cstdint>
#include <filesystem>
#include <limits>
#include <map>
#include <memory>
#include <string>
#include <thread>
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
    struct SourceEdit {
      std::size_t begin{};
      std::wstring before;
      std::wstring after;
    };

    struct AnimatedImageState {
      unsigned frame{};
      ULONGLONG due{};
    };

    // This is deliberately a cheap identity check.  Decoding every image on
    // each text/presentation update made ordinary typing proportional to the
    // total image count.  A changed size or write time invalidates the cached
    // render and causes the normal safety/decode path to run again.
    struct ImageFileIdentity {
      bool available{};
      std::uint64_t size{};
      std::uint64_t write_time{};

      friend bool operator==(const ImageFileIdentity&, const ImageFileIdentity&) = default;
    };

    struct RenderedImage {
      std::size_t source_begin{};
      std::size_t source_end{};
      std::wstring markup;
      std::wstring target_text;
      std::wstring alternate_text;
      unsigned width_dip{};
      std::filesystem::path target;
      ImageFileIdentity file_identity;
      unsigned raster_width{};
      unsigned raster_height{};
      unsigned frame_count{};
      bool animated{};
      bool inserted{};
    };

    Document document;
    HWND editor{};
    MarkdownParseResult parse;
    EditorSnapshot editor_snapshot;
    std::uint64_t derived_image_revision{std::numeric_limits<std::uint64_t>::max()};
    int active_line{-1};
    HWND compact_window{};
    std::shared_ptr<WorkspaceStore> workspace_store;
    ULONGLONG autosave_due{};
    ULONGLONG recovery_due{};
    ULONGLONG sync_due{};
    ULONGLONG presentation_due{};
    std::map<std::size_t, AnimatedImageState> animated_image_frames;
    ULONGLONG animation_due{};
    std::wstring rendered_image_source;
    std::vector<RenderedImage> rendered_images;
    ULONGLONG image_asset_check_due{};
    bool ime_composing{};
    std::vector<SourceEdit> source_undo;
    std::vector<SourceEdit> source_redo;
  };

  enum class SaveAllResult { AllSaved, RecoveryOnly, Discarded, Cancelled };

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
  void NewUntitledDocument();
  void QuickOpen();
  void OpenWorkspace(const std::filesystem::path& path);
  void PopulateWorkspaceTree();
  void AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth);
  void OpenDocument(const std::filesystem::path& path);
  void OpenDocumentView(Document document, std::wstring tab_name);
  void OpenRecoverySnapshot(const std::filesystem::path& path);
  void ActivateDocument(std::size_t index);
  bool SaveDocument(DocumentView& view, bool interactive);
  bool SaveDocumentAs(DocumentView& view);
  void ReloadDocumentFromDisk();
  void CompareDocumentWithDisk();
  SaveAllResult SaveAllForExit(bool interactive);
  bool SaveAllRequired(bool interactive);
  DocumentView* FindDocumentView(HWND editor);
  void SelectDocumentForEditor(HWND editor);
  void ApplySourceTextWithUndo(DocumentView& view, std::wstring text, bool record_history = true);
  bool ApplySourceHistory(DocumentView& view, bool redo);
  void OnEditorChanged(HWND editor);
  void SyncDocumentFromEditor(DocumentView& view);
  void ApplyMarkdownPresentation(DocumentView& view, bool force);
  void DrawTableGrid(const DocumentView& view);
  void RefreshDerivedImages(DocumentView& view);
  void AdvanceAnimatedImages(DocumentView& view, ULONGLONG now);
  static bool ReadImageFileIdentity(const std::filesystem::path& path,
                                    DocumentView::ImageFileIdentity& identity);
  void RebuildOutline(const DocumentView& view);
  std::wstring EditorText(HWND editor, const EditorSnapshot& snapshot) const;
  void UpdateStatus();
  void ShowFindBar();
  SearchQuery SearchQueryFromFindBar() const;
  void FindNext(bool restart_from_beginning = false);
  void ReplaceCurrentDocument(bool all);
  void SearchWorkspaceFromFindBar();
  void ScheduleWorkspaceSearch();
  void ApplyWorkspaceSearchBatch(void* payload);
  void CompleteWorkspaceSearch(void* payload);
  void OpenWorkspaceSearchResult(std::size_t index);
  void ReplaceWorkspaceFromFindBar();
  bool CloseDocument(std::size_t index);
  void CreateProfile(BuiltInProfile profile);
  void CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date);
  void CreateProfileById(std::wstring id, const SYSTEMTIME* requested_date);
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
  void ImportHolidayData();
  void StartHolidayUpdate(bool manual);
  void LoadHolidayCache();
  void CompleteHolidayUpdate(void* payload);
  void ScheduleHolidayUpdate();
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
  HWND find_include_glob_{};
  HWND find_exclude_glob_{};
  HWND replace_one_{};
  HWND replace_document_{};
  HWND find_results_{};
  HWND calendar_{};
  HWND calendar_tooltip_{};
  std::filesystem::path workspace_;
  std::vector<std::filesystem::path> copied_files_;
  std::vector<std::unique_ptr<std::filesystem::path>> tree_paths_;
  std::vector<std::unique_ptr<DocumentView>> documents_;
  std::vector<std::filesystem::path> recent_documents_;
  std::vector<SearchMatch> workspace_search_results_;
  std::size_t workspace_search_issue_count_{};
  std::jthread workspace_search_worker_;
  std::jthread holiday_update_worker_;
  std::uint64_t workspace_search_generation_{};
  std::uint64_t holiday_update_generation_{};
  ULONGLONG workspace_search_due_{};
  bool workspace_search_started_{};
  bool holiday_cache_loaded_{};
  bool holiday_update_running_{};
  std::size_t active_document_{static_cast<std::size_t>(-1)};
  bool suppress_editor_change_{};
  bool external_operation_active_{};
  bool outline_dragging_{};
  bool workspace_dragging_{};
  std::size_t outline_drag_source_{};
  std::vector<std::filesystem::path> workspace_drag_sources_;
  std::shared_ptr<WorkspaceStore> workspace_store_;
  HANDLE workspace_mutex_{};
  HACCEL accelerator_table_{};
  HFONT editor_font_{};
  HBRUSH background_brush_{};
  COLORREF theme_background_{RGB(255, 255, 255)};
  COLORREF theme_foreground_{RGB(24, 24, 24)};
  EffectiveSettings settings_;
  std::wstring calendar_tooltip_text_;
  std::wstring holiday_update_status_;
};

}  // namespace mdlite
