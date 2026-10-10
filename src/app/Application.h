#pragma once
#include "app/PanelLayout.h"

#include "calendar/CalendarView.h"
#include "calendar/CalendarDayIndex.h"
#include "core/Document.h"
#include "editor/EditorAdapter.h"
#include "editor/RichEditTableAdapter.h"
#include "git/Conflict.h"
#include "git/GitPanel.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "search/Search.h"
#include "settings/Settings.h"
#include "table/Table.h"
#include "workspace/Workspace.h"

#include <windows.h>
#include <commctrl.h>

#include <array>
#include <atomic>
#include <cstdint>
#include <filesystem>
#include <limits>
#include <map>
#include <memory>
#include <optional>
#include <string>
#include <string_view>
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
  enum class SearchScope : std::uint8_t { CurrentFile, Workspace };

  enum class EditorProjectionRebuildResult {
    Failed,
    Native,
    FlatSourceFallback,
  };

  struct DocumentView {
    using PendingNativeEditKind = EditorEditKind;

    struct SourceEdit {
      std::size_t begin{};
      std::wstring before;
      std::wstring after;
      SourceSelection selection_before;
      SourceSelection selection_after;
      // A virtual Markdown cell has no unique plain-text projection offset.
      std::optional<TableCellIntent> selection_before_virtual_cell;
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

    struct TableViewport {
      HWND editor{};
      std::size_t source_begin{};
      std::size_t source_end{};
      EditorSnapshot snapshot;
      std::uint64_t revision{std::numeric_limits<std::uint64_t>::max()};
      bool native_tables_ready{};
      std::uint64_t presentation_revision{std::numeric_limits<std::uint64_t>::max()};
      int active_line{-1};
      std::size_t active_source_line_begin{std::numeric_limits<std::size_t>::max()};
      std::uint64_t derived_image_revision{std::numeric_limits<std::uint64_t>::max()};
      std::vector<RenderedImage> rendered_images;
      std::map<std::size_t, AnimatedImageState> animated_image_frames;
      std::wstring rendered_image_source;
      std::vector<LONG> row_heights_twips;
      UINT dpi{};
      unsigned font_size_pt{};
      std::wstring font_face;
      LONG available_width{};
      LONG natural_width_twips{};
      std::array<HWND, 10> operation_buttons{};
      std::optional<TableCellIntent> hovered_cell;
      bool hover_visible{};
      bool button_press_armed{};
      bool pointer_down{};
      bool pointer_dragged{};
      POINT pointer_origin{};
    };

    Document document;
    HWND editor{};
    std::optional<RECT> editor_formatting_rect_request;
    SourceSelection suspended_selection{};
    std::size_t suspended_first_visible_source{};
    std::optional<std::size_t> suspended_horizontal_left_edge_source;
    std::optional<int> suspended_legacy_first_visible_line;
    bool suspended_view_state_valid{};
    MarkdownParseResult parse;
    EditorSnapshot editor_snapshot;
    std::uint64_t derived_image_revision{std::numeric_limits<std::uint64_t>::max()};
    std::uint64_t presentation_revision{std::numeric_limits<std::uint64_t>::max()};
    int active_line{-1};
    std::size_t active_source_line_begin{std::numeric_limits<std::size_t>::max()};
    bool active_line_update_pending{};
    std::uint64_t active_line_update_revision{std::numeric_limits<std::uint64_t>::max()};
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
    bool native_edit_in_flight{};
    bool native_edit_pending{};
    bool native_readback_failed{};
    bool editor_locked_for_readback{};
    bool editor_readonly_lock_applied{};
    bool editor_enabled_before_readback_lock{};
    bool editor_projection_invalid{};
    bool flat_source_fallback_pending{};
    bool flat_source_fallback_retry_used{};
    unsigned editor_projection_repair_attempts{};
    unsigned force_editor_readback_failures_for_test{};
    bool force_markdown_presentation_failure_for_test{};
    bool force_table_cell_formatting_failure_for_test{};
    bool force_partial_markdown_presentation_failure_for_test{};
    std::uint32_t native_projection_failures_for_test{};
    std::optional<SourceSelection> pending_selection_before;
    PendingNativeEditKind pending_native_edit_kind{PendingNativeEditKind::Unknown};
    std::optional<SourceSelection> ime_selection_before;
    std::optional<TableCellIntent> pending_virtual_table_cell;
    wchar_t pending_table_high_surrogate{};
    wchar_t pending_body_high_surrogate{};
    SourceSelection pending_body_surrogate_selection{};
    std::uint64_t pending_body_surrogate_revision{};
    bool native_tables_ready{};
    std::vector<SourceEdit> source_undo;
    std::vector<SourceEdit> source_redo;
    bool painting_table_grid{};
    bool table_source_mode{};
    bool table_source_revealed_for_search{};
    bool table_return_key_handled{};
    std::vector<std::unique_ptr<TableViewport>> table_viewports;
    TableViewport* active_table_viewport{};
    HWND table_parent_editor{};
    bool refreshing_table_viewports{};
    std::optional<SourceSelection> table_exit_selection;
  };

  class TableViewportContext {
   public:
    TableViewportContext(Application& app, DocumentView& view, DocumentView::TableViewport& viewport);
    ~TableViewportContext();
   private:
    void SwapProjection();
    Application& app_;
    DocumentView& view_;
    DocumentView::TableViewport& viewport_;
    HWND parent_editor_{};
    std::uint64_t revision_{};
    bool source_mode_{};
  };

  struct RetiredSearchWorker {
    std::jthread worker;
    std::shared_ptr<std::atomic_bool> finished;
  };

  struct SearchFormatSpan {
    LONG begin{};
    LONG end{};
    COLORREF background{};
    bool automatic_background{};
  };

  struct SearchHighlightRange {
    LONG begin{};
    LONG end{};
    bool current{};
    friend bool operator==(const SearchHighlightRange&, const SearchHighlightRange&) = default;
  };

  struct SearchHighlightState {
    std::filesystem::path path;
    std::uint64_t revision{};
    std::vector<SearchFormatSpan> original_backgrounds;
    std::vector<SearchHighlightRange> ranges;
  };

  struct TableGridRow {
    std::size_t begin{};
    std::size_t end{};
    std::size_t source_cell_count{};
    std::vector<TableVisualCell> cells;
    int top{};
    int bottom{};
  };

  struct TableGridGeometry {
    int left{};
    int right{};
    std::vector<int> boundaries;
    std::vector<TableGridRow> rows;
  };

  struct CalendarDetailTarget {
    std::size_t begin{};
    std::size_t end{};
    std::filesystem::path path;
  };

  enum class SaveAllResult { AllSaved, RecoveryOnly, Discarded, Cancelled };
  enum class SaveIntent { UserRequested, BackgroundAutosave };
  enum class SaveResult { Saved, RecoverySaved, NoChange, Cancelled, ReadbackFailed, Failed };

  enum class TableAction { InsertRowBefore, InsertRowAfter, DeleteRow, InsertColumnBefore,
                           InsertColumnAfter, DeleteColumn };
  enum class SplitterDrag { None, LeftWidth, RightWidth, LeftHeight, RightHeight };

  static LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
  static LRESULT CALLBACK CompactWindowProc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
  static LRESULT CALLBACK EditorSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                          UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK TableViewportSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                                UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK TableOperationButtonSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                                       UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK TreeDragSubclass(HWND window, UINT message, WPARAM wparam, LPARAM lparam,
                                            UINT_PTR subclass_id, DWORD_PTR reference);
  static LRESULT CALLBACK PanelHeaderSubclass(HWND window, UINT message, WPARAM wparam,
                                               LPARAM lparam, UINT_PTR subclass_id,
                                               DWORD_PTR reference);
  static LRESULT CALLBACK SearchControlSubclass(HWND window, UINT message, WPARAM wparam,
                                                LPARAM lparam, UINT_PTR subclass_id,
                                                DWORD_PTR reference);
  static LRESULT CALLBACK ChromeBarSubclass(HWND window, UINT message, WPARAM wparam,
                                             LPARAM lparam, UINT_PTR subclass_id,
                                             DWORD_PTR reference);
  static LRESULT CALLBACK TabStripSubclass(HWND window, UINT message, WPARAM wparam,
                                            LPARAM lparam, UINT_PTR subclass_id,
                                            DWORD_PTR reference);
  static LRESULT CALLBACK StatusBarSubclass(HWND window, UINT message, WPARAM wparam,
                                             LPARAM lparam, UINT_PTR subclass_id,
                                             DWORD_PTR reference);
  static LRESULT CALLBACK CalendarDetailsSubclass(HWND window, UINT message, WPARAM wparam,
                                                   LPARAM lparam, UINT_PTR subclass_id,
                                                   DWORD_PTR reference);
  LRESULT HandleMessage(UINT message, WPARAM wparam, LPARAM lparam);

  void CreateControls();
  void LayoutControls();
  int MeasurePanelTextHeight(HWND item, int width) const;
  int MinimumCalendarPanelHeight(int width) const;
  void LoadPanelLayout();
  void SavePanelLayout();
  void MovePanelToSlot(PanelId id, PanelSlot slot);
  void MoveFocusedPanelToSlot(PanelSlot slot);
  void ResizeFocusedPanel(double delta);
  void ResizeFocusedPanelHeight(double delta);
  void UpdatePanelHeaders();
  void DrawChromeButton(const DRAWITEMSTRUCT& draw) const;
  void DrawTabItem(const DRAWITEMSTRUCT& draw) const;
  void DrawSearchScopeItem(const DRAWITEMSTRUCT& draw) const;
  void DrawSearchScopeArrow(HWND combo, HDC dc) const;
  void DrawSearchScopeFrame(HWND combo, HDC dc) const;
  void DrawSearchCheckbox(HWND checkbox) const;
  void ActivatePanel(PanelId id);
  void GoToCalendarToday();
  void SetStatusSegments(std::array<std::wstring, 4> segments);
  std::vector<std::filesystem::path> SelectedGitPaths() const;
  void RenderGitPanel();
  void UpdateGitPanelActions();
  void ShowSelectedGitDiff();
  void CreateMenuBar();
  void ApplyChromeTheme();
  void SetStatusText(std::wstring_view text);
  void MeasureMenuItem(MEASUREITEMSTRUCT& measure) const;
  void DrawMenuItem(const DRAWITEMSTRUCT& draw) const;
  void DrawStatusItem(const DRAWITEMSTRUCT& draw) const;
  void OpenWorkspaceDialog();
  void OpenFileDialog();
  void NewUntitledDocument();
  void QuickOpen();
  void OpenWorkspace(const std::filesystem::path& path);
  void PopulateWorkspaceTree(bool refresh_calendar_index = true);
  void RefreshCalendarAfterWorkspaceMutation();
  void AddTreeDirectory(HTREEITEM parent, const std::filesystem::path& directory, int depth);
  void OpenDocument(const std::filesystem::path& path, bool activate = true);
  void OpenDocumentView(Document document, std::wstring tab_name, bool activate = true);
  void SeedSessionViewState(DocumentView& view, const SessionDocument& item);
  void OpenRecoverySnapshot(const std::filesystem::path& path);
  void ActivateDocument(std::size_t index);
  bool EnsureEditor(DocumentView& view);
  bool SuspendEditor(DocumentView& view);
  void CaptureEditorViewState(DocumentView& view);
  void RestoreEditorViewState(DocumentView& view);
  SaveResult SaveDocument(DocumentView& view, SaveIntent intent);
  SaveResult SaveDocumentAs(DocumentView& view);
  void ReloadDocumentFromDisk();
  void CompareDocumentWithDisk();
  SaveAllResult SaveAllForExit(bool interactive);
  bool SaveAllRequired(bool interactive);
  DocumentView* FindDocumentView(HWND editor);
  void SelectDocumentForEditor(HWND editor);
  bool ApplySourceTextWithUndo(DocumentView& view, std::wstring text, bool record_history = true,
                               std::optional<SourceSelection> selection_after = std::nullopt,
                               std::optional<SourceSelection> selection_before = std::nullopt,
                               std::optional<TableCellIntent> selection_before_virtual_cell =
                                   std::nullopt);
  bool ConsumeNativeProjectionFailureForTest(DocumentView& view, std::uint32_t stage);
  bool SetEditorFlatTextVerified(DocumentView& view, const EditorSnapshot& expected,
                                 std::uint32_t test_failure_stage = 0);
  void QuarantineEditorProjection(DocumentView& view, SourceSelection selection,
                                  std::wstring_view status_text);
  void ProcessDeferredEditorRepairs();
  bool ApplySourceHistory(DocumentView& view, bool redo);
  bool PrepareTableProjectionForNativeMutation(DocumentView& view, UINT message,
                                               WPARAM wparam);
  bool HandleTableCommandInput(DocumentView& view, UINT message, WPARAM wparam, LPARAM lparam);
  void ToggleTableSource(DocumentView& view, bool search_reveal = false);
  void ReleaseSearchTableSource(DocumentView& view);
  EditorSnapshot NativeSnapshotForView(DocumentView& view, std::wstring_view text) const;
  void RefreshTableViewports(DocumentView& view);
  void LayoutTableViewports(DocumentView& view);
  bool SelectTableSourceRange(DocumentView& view, SourceSelection selection, bool focus = true);
  void ApplyTableSearchHighlights(DocumentView& view);
  void ShowTableHoverControls(DocumentView& view, DocumentView::TableViewport& viewport, bool show);
  void UpdateTableHoverTarget(DocumentView& view, DocumentView::TableViewport& viewport, POINT point);
  void ExecuteTableHoverCommand(DocumentView& view, DocumentView::TableViewport& viewport, unsigned command);
  void DrawTableOperationButton(const DRAWITEMSTRUCT& draw) const;
  void UpdatePendingVirtualTableCellFromCaret(DocumentView& view,
                                              const POINT* click_point = nullptr);
  bool InsertTextIntoPendingVirtualTableCell(DocumentView& view,
                                              std::wstring_view text,
                                              std::optional<SourceSelection> selection_before = std::nullopt);
  SourceSelection CaptureSourceSelection(HWND editor, const EditorSnapshot& snapshot) const;
  std::optional<std::size_t> CaptureVisibleLeftEdgeSourceOffset(
      HWND editor, const EditorSnapshot& snapshot) const;
  std::optional<POINT> SourceAnchoredScrollPosition(
      HWND editor, const EditorSnapshot& snapshot, std::size_t viewport_source_offset) const;
  SourceSelection CaptureViewSelection(HWND editor, const EditorSnapshot& snapshot) const;
  void RestoreSourceSelection(HWND editor, const EditorSnapshot& snapshot,
                              SourceSelection selection) const;
  bool RestoreVirtualTableCellCaret(DocumentView& view,
                                    const TableCellIntent& intent);
  RichEditTableStyle TableStyleForEditor(HWND editor) const;
  EditorProjectionRebuildResult RebuildEditorProjection(
      DocumentView& view, EditorSnapshot target, SourceSelection selection);
  void OnEditorChanged(HWND editor);
  void CommitPendingNativeEdit(DocumentView& view);
  void SchedulePresentation(DocumentView& view);
  void ScheduleFlatSourceFallbackRetry(DocumentView& view);
  void QueueActiveLinePresentation(DocumentView& view);
  void ApplyPendingActiveLinePresentations();
  void ApplyActiveLinePresentation(DocumentView& view);
  bool SyncDocumentFromEditor(DocumentView& view);
  void ApplyMarkdownPresentation(
      DocumentView& view, bool force,
      std::optional<SourceSelection> source_selection = std::nullopt);
  void InvalidateTableGrid(DocumentView& view);
  std::vector<TableGridGeometry> BuildTableGridGeometry(const DocumentView& view,
                                                        const RECT& client,
                                                        HDC metrics_dc = nullptr) const;
  std::optional<TableCellIntent> HitTestTableCell(const DocumentView& view, POINT point) const;
  void DrawTableGrid(DocumentView& view, HDC paint_dc, const RECT& clip);
  void RefreshDerivedImages(DocumentView& view);
  void AdvanceAnimatedImages(DocumentView& view, ULONGLONG now);
  static bool ReadImageFileIdentity(const std::filesystem::path& path,
                                    DocumentView::ImageFileIdentity& identity);
  void RebuildOutline(const DocumentView& view);
  bool EditorText(HWND editor, EditorSnapshot& snapshot, std::wstring& text,
                  std::wstring* raw_native_text = nullptr) const;
  void UpdateStatus();
  void ShowFindBar(SearchScope scope = SearchScope::Workspace);
  void CloseSearchPanel();
  void SetSearchScope(SearchScope scope);
  void ScheduleSearch();
  void SearchNow();
  void PruneSearchWorkers();
  void NavigateSearchMatch(int direction);
  void UpdateCurrentSearchMatchForCaret(DocumentView& view);
  std::uint64_t SearchContentHash(const DocumentView& view);
  void ClearSearchResults();
  void UpdateSearchPanelLabels();
  void ApplyDocumentSearchHighlights(DocumentView& view, bool formatting_reset = false);
  void OpenSearchResult(std::size_t index, bool focus_editor = false);
  void ReplaceSelectedWorkspace(bool all_matches);
  void ReplaceWorkspaceFile(const SearchMatch& match, bool all_matches);
  SearchQuery SearchQueryFromFindBar() const;
  void FindNext(bool restart_from_beginning = false);
  void ReplaceCurrentDocument(bool all);
  void SearchWorkspaceFromFindBar();
  void ScheduleWorkspaceSearch();
  void ApplyWorkspaceSearchBatch(void* payload);
  void CompleteWorkspaceSearch(void* payload);
  void OpenWorkspaceSearchResult(std::size_t index);
  void ReplaceWorkspaceFromFindBar();
  void RollbackClosedWorkspaceReplace();
  bool CloseDocument(std::size_t index);
  void CreateProfile(BuiltInProfile profile);
  void CreateProfileForDate(BuiltInProfile profile, const SYSTEMTIME& date);
  void OpenSelectedCalendarDate();
  void OpenCalendarDate(CalendarDate date);
  bool OpenCalendarDetailAtOffset(std::size_t offset);
  void UpdateCalendarDetails(const SYSTEMTIME& date);
  void UpdateCalendarDetails(CalendarDate date);
  void StartCalendarIndexWorker();
  void StopCalendarIndexWorker();
  void CompleteCalendarIndex(void* raw_payload);
  void CompleteCalendarIndexDeliveryFailure();
  void RefreshCalendarForSelection(CalendarDate date);
  void UpdateCalendarViewTheme();
  void UpdateCalendarViewMarkers();
  void CreateProfileById(std::wstring id, const SYSTEMTIME* requested_date);
  void ApplyTableAction(TableAction action, std::optional<std::size_t> target_source = std::nullopt);
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
  void StartGitStatusRefresh();
  void CompleteGitStatus(void* payload);
  void CompleteGitStatusDeliveryFailure();
  void StopGitStatusWorker();
  void StartGitAction(int command, std::filesystem::path git_path,
                      GitActionRequest request);
  void CompleteGitAction(void* payload);
  void CompleteGitActionDeliveryFailure();
  void StopGitActionWorker();
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
  void ShowDiagnostics();
  void RecordDiagnosticSummary(std::wstring_view summary);
  void LoadAndApplySettings();
  void ApplySettings();
  void RebuildAccelerators();
  void ToggleCompactWindow();
  void UploadImageAtCaret();
  bool PasteClipboardImage();
  void ResizeImageAtCaret(unsigned width_dip);
  void OpenLinkAtSourcePosition(DocumentView& view, std::size_t source_position, bool activate);
  void ImportHolidayData();
  void LoadHolidayCache();
  void UpdateCalendarTooltip(std::optional<CalendarDate> date);
  bool IsDocumentOpen(const std::filesystem::path& path) const;
  std::filesystem::path SelectedTreePath() const;
  std::vector<std::filesystem::path> SelectedTreePaths() const;

  HINSTANCE instance_{};
  HWND window_{};
  HMENU menu_{};
  HWND chrome_bar_{};
  HWND activity_rail_{};
  HWND brand_{};
  HWND command_search_{};
  HWND tab_new_{};
  HWND tab_close_{};
  HWND chrome_tooltip_{};
  std::array<HWND, 5> activity_buttons_{};
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
  HWND rollback_workspace_replace_{};
  HWND find_case_{};
  HWND find_regex_{};
  HWND find_word_{};
  HWND find_include_glob_{};
  HWND find_exclude_glob_{};
  HWND replace_one_{};
  HWND replace_document_{};
  HWND find_results_{};
  HWND search_scope_{};
  HWND search_target_{};
  HWND search_previous_{};
  HWND search_replace_toggle_{};
  HWND search_details_toggle_{};
  HWND search_count_{};
  HWND search_status_{};
  HWND search_ignore_{};
  HWND search_hidden_{};
  HWND calendar_{};
  HWND calendar_tooltip_{};
  HWND git_panel_{};
  HWND git_files_{};
  HWND git_diff_view_{};
  HWND git_commit_edit_{};
  HWND git_refresh_{};
  HWND git_stage_{};
  HWND git_unstage_{};
  HWND git_diff_{};
  HWND git_commit_{};
  HWND git_trust_{};
  HWND calendar_details_{};
  HWND calendar_summary_{};
  HWND calendar_daily_{};
  HWND calendar_details_toggle_{};
  bool calendar_details_expanded_{};
  std::vector<CalendarDetailTarget> calendar_detail_targets_;
  std::jthread calendar_index_worker_;
  std::uint64_t calendar_index_generation_{};
  std::filesystem::path calendar_index_workspace_;
  std::optional<CalendarDayFileIndex> calendar_workspace_index_;
  bool calendar_index_active_{};
  std::atomic_bool calendar_index_delivery_failed_{};
  std::array<HWND, 4> panel_headers_{};
  std::filesystem::path workspace_;
  std::filesystem::path latest_closed_replace_journal_;
  std::vector<std::filesystem::path> copied_files_;
  std::vector<std::unique_ptr<std::filesystem::path>> tree_paths_;
  std::vector<std::unique_ptr<DocumentView>> documents_;
  std::vector<std::filesystem::path> recent_documents_;
  std::vector<std::wstring> recent_diagnostic_summaries_;
  std::vector<SearchMatch> workspace_search_results_;
  std::vector<SearchMatch> document_search_results_;
  std::vector<HTREEITEM> search_result_items_;
  std::map<std::filesystem::path, HTREEITEM> search_file_nodes_;
  std::map<std::filesystem::path, std::size_t> search_file_first_result_;
  std::map<std::filesystem::path, std::size_t> search_file_match_counts_;
  std::map<HWND, SearchHighlightState> search_highlight_states_;
  std::map<std::filesystem::path, std::pair<std::uint64_t, std::uint64_t>> search_content_hash_cache_;
  std::size_t workspace_search_error_count_{};
  std::size_t workspace_search_excluded_count_{};
  bool workspace_search_failed_{};
  std::jthread workspace_search_worker_;
  std::shared_ptr<std::atomic_bool> workspace_search_worker_finished_;
  std::vector<RetiredSearchWorker> retired_search_workers_;
  std::jthread git_status_worker_;
  std::jthread git_action_worker_;
  std::uint64_t workspace_search_generation_{};
  std::uint64_t git_status_generation_{};
  std::uint64_t git_action_generation_{};
  std::atomic_bool git_status_delivery_failed_{};
  std::atomic_bool git_action_delivery_failed_{};
  HANDLE git_status_cancellation_event_{};
  HANDLE git_action_cancellation_event_{};
  std::filesystem::path git_status_workspace_;
  std::filesystem::path git_action_workspace_;
  bool git_status_refresh_pending_{};
  bool git_action_active_{};
  ULONGLONG workspace_search_due_{};
  bool workspace_search_started_{};
  SearchScope search_scope_state_{SearchScope::Workspace};
  bool search_replace_expanded_{};
  bool search_details_expanded_{};
  bool search_ime_composing_{};
  bool search_tree_selection_update_{};
  int pending_search_navigation_{};
  std::filesystem::path document_search_path_;
  std::uint64_t document_search_revision_{};
  std::size_t current_search_match_index_{static_cast<std::size_t>(-1)};
  std::size_t current_workspace_match_index_{static_cast<std::size_t>(-1)};
  bool holiday_cache_loaded_{};
  bool save_dialog_active_{};
  std::size_t active_document_{static_cast<std::size_t>(-1)};
  bool suppress_editor_change_{};
  bool table_tab_keydown_handled_{};
  bool editor_projection_repair_message_posted_{};
  ULONGLONG editor_projection_repair_retry_due_{};
  bool external_operation_active_{};
  bool outline_dragging_{};
  bool workspace_dragging_{};
  bool workspace_pane_collapsed_{};
  bool outline_pane_collapsed_{};
  SplitterDrag splitter_drag_{SplitterDrag::None};
  int left_width_splitter_x_{};
  int right_width_splitter_x_{};
  int left_height_splitter_y_{};
  int right_height_splitter_y_{};
  int current_rail_width_{};
  int current_tree_width_{};
  int current_outline_width_{};
  int current_splitter_width_{};
  int current_topbar_height_{};
  int current_status_height_{};
  bool left_height_splitter_visible_{};
  bool right_height_splitter_visible_{};
  std::size_t outline_drag_source_{};
  std::vector<std::filesystem::path> workspace_drag_sources_;
  std::shared_ptr<WorkspaceStore> workspace_store_;
  HANDLE workspace_mutex_{};
  HACCEL accelerator_table_{};
  HFONT editor_font_{};
  HFONT ui_font_{};
  HBRUSH background_brush_{};
  HBRUSH surface_brush_{};
  HBRUSH input_brush_{};
  HBRUSH editor_brush_{};
  COLORREF theme_background_{RGB(255, 255, 255)};
  COLORREF theme_foreground_{RGB(24, 24, 24)};
  COLORREF theme_surface_{RGB(255, 255, 255)};
  COLORREF theme_surface_alt_{RGB(243, 246, 250)};
  COLORREF theme_editor_{RGB(252, 253, 255)};
  COLORREF theme_input_{RGB(255, 255, 255)};
  COLORREF theme_border_{RGB(210, 218, 228)};
  COLORREF theme_muted_{RGB(92, 104, 120)};
  COLORREF theme_accent_{RGB(56, 112, 194)};
  bool dark_theme_{};
  bool high_contrast_{};
  COLORREF theme_selected_text_{RGB(255, 255, 255)};
  std::vector<std::unique_ptr<std::wstring>> menu_labels_;
  std::wstring status_text_;
  std::array<std::wstring, 4> status_segments_{};
  PanelLayout panel_layout_ = PanelLayout::Default();
  PanelId focused_panel_{PanelId::Explorer};
  int active_activity_{};
  EffectiveSettings settings_;
  GitPanelStatus git_panel_status_;
  std::wstring calendar_tooltip_text_;
  std::wstring holiday_data_status_;
};

}  // namespace mdlite
