# Rust UI・周辺機能の実装記録

今回の基準は Rust `develop` の `671e25a637d07828c02048145e8e84f1a30625ca`。
旧C++は `cpp-archive/pre-rust-20261013` を参照し、現在のRope編集基盤へ機能を接続する。
Markdown本文、encoding、改行、向き付き選択、IMEの実通知、Undo/Redo、安全保存を保護する。

## 現在の確認範囲

実装・最終機械検証・独立correctness reviewは完了。GUI/physical/Human受入は未確認であり、製品全体の合格を示さない。

統合時の追加是正：パネルにfocusがある場合だけNavigationキーを処理し、Git/Searchのbody clickを
別パネルが消費しない。Outlineのdrag/dropはrevisionに束縛した子節移動へ接続する。
文書検索は同じqueryでもactive文書ID/revisionの変化で再予約する。
Save Asは相対画像のlayout/cacheを破棄し、新しい文書directoryから解決し直す。
非Markdown文書はliteral表示を保持し、Markdown構造操作を無効にする。

独立correctness reviewの初回は変更要求。GitとWorkspace置換/復元の排他、dirty sessionの
source identity、横scroll復元、固定shortcut予約、実効F3、色設定とstatus、未保存文書の
外部リンクを修正。同じ独立reviewerの安定後再確認で指摘なし。初回レビューだけを合格として扱わない。

過去に記録したlibrary 134件PASS、native全体試験109/110からのpalette期待値修正、
無効選択を確認する局所試験PASSはslice履歴であり、現在の最終Debug/Release結果ではない。
所有Debug EXEの起動は過去に確認したが、Windows foreground確認で入力前に停止し、その後のsource変更で
その実行identityは無効になった。PrintWindowはDirect2D内容を取得できず、承認UIとの画面比較やGUI合格の
証拠には使わない。現在のGUI/physical/Human操作はNOT_RUN。

Ponytail自己確認：操作契約に必要なWin32/process/format境界を残し、汎用pluginや別routerを追加しない。
Feature同期の全本文再parse・未使用Outline deep clone、statusのCR行cache複製、
画像/表menu有効判定時の不要な編集文字列生成、harnessの未使用helperを除去した。
これは実装者の複雑性確認であり、別Agentのcorrectness reviewとは別に扱う。

Feature ownerはRootへ返却済み。Rootからの現在のsource-slice結果はExplorer list 4件、Explorer tree 4件
（reparse privilege fixtureはSKIPとして明示）、icon分類1件、owner-draw dispatch fixture、Calendar open前
recheck、Outline親子section移動/Escape、shared button enablementがPASS。これはW03/N03/C05の該当source-slice
証拠。最終full suiteと実行体identityは下記に別記し、GUI/Human受入とは分ける。

W03/N03/C05とowner-draw dispatchは独立再確認で是正を受け入れられた。
source `57e2307814b72075be244c923486578ed5cf068b11bfd3715bec4aec28b547f2` はDebug/Releaseとも
core 139 + native 144、fmt、Clippy PASS。ただし旧menuの最終照合で「今日のDairy」と
「選択日のDaily」の時刻・入口の区別が不足していることが分かり、Jev Findingは `fix_current`。
このsourceの結果は修正前の履歴として保持し、時計routing修正後の証拠には流用しない。

| 項目 | 実装・局所確認 | native操作・統合 |
|---|---|---|
| 基準Editor | 変更前Debug 131 tests PASS | 前回のGUI結果を今回の結果へ流用しない |
| native command registry | `native/shell.rs::commands()` がFeature 4000–4011/4020–4028/4040–4042/4080–4088/4090、Search 5000–5010、Git 6000–6025、Editor 20–39、Diagnostics 167を列挙。`App::shell_command()` から各module APIへdispatch | 現行sourceで接続を確認。今回buildのvisible操作は未実施 |
| 同一文書・Save As・多tab（U04/D01/R01–R03） | canonical/物理identityによる既存文書選択、別open文書へのSave As拒否、6件制限撤廃・overflow。focused 4 tests PASS | 今回buildのvisible操作は未実施 |
| 4パネル配置（U02/D07） | schema 1読取り、slot交換、collapsed/hidden、中央幅優先のgeometry。2 tests PASS | `shell_geometry` とpanel commandへ接続済み。今回buildのvisible操作は未実施 |
| Explorer・Outline・リンク（W03–W07/N01–N04） | workspace tree/create/move、Quick Open、Outline移動、anchor/scheme検証のmodule API | Feature 4000–4011/4040–4042とLink 166をnative shellへ接続済み。今回buildのvisible操作は未実施 |
| Calendar・profile/template（C01–C06） | calendar/profile coreとworkspace create/validation API | Feature 4020–4028/4080–4088/4090をnative shellへ接続済み。今回buildのvisible操作は未実施 |
| Search/replace（F01–F08） | search snapshot/revision、PCRE2/capture replacement、workspace scanner/journal coreのslice試験 | Search 5000–5010をpane controls・shared command routeへ接続済み。今回buildのvisible操作は未実施 |
| Git/Trust/storage（G01–G06/I04） | git/trust/process/storage adapterの局所・合成repo試験 | Git 6000–6025をGit paneへ接続済み。remote/storageとvisible操作は未実施 |
| Table/image actions（I01–I03） | table/image source editと既存layoutの局所確認 | Editor 20–34/38–39をtoolbar/context menu/shared command routeへ接続済み。今回buildのvisible操作は未実施 |
| Diagnostics/privacy（A01） | privacy canaryとcurrent-process ProcessStatusのfocused 2 tests PASS | Diagnostics 167と手動Refresh windowをshared command routeへ接続済み。今回buildのvisible操作は未実施 |
| session・保存予約（D02/D03/D06） | schema 4のsource/recovery束縛、旧1/2 UTF-16と旧3 UTF-8読取り、dirty/無題の一致本文へview復元、横scrollのsource anchor。core 4/native persistence 10 tests PASS | timer・再起動復元へ接続済み。今回buildのvisible操作は未実施 |
| recovery（D03/D04） | Workspace内snapshot、旧header/body、unknown schema原本保持、変更後候補の破棄拒否、明示Noの再生成防止。core/native局所PASS | 候補選択・定期保存へ接続済み。今回buildのvisible操作は未実施 |

## 最終機械検証（時計routing修正後）

source SHA256: `0242a4eca5d28f3b17abf6a9bd4a3f2de3c9962fe34e65a15f7f780a9923e9ec`。
既存の登録build scriptを使い、両構成ともcore 139 + native 146 = **285 tests PASS**、fmt、Clippy `-D warnings` PASS。
Debugは古い検証windowを保全するため専用targetへbuildした。Releaseは通常の `target/release`。

| 構成 | EXE | SHA256 |
|---|---|---|
| Debug | `target/ui-feature-parity-final/debug/mdlite.exe` | `5c7607126b23d4267c7961718b4fb601750aece23109e3141ad2936c25deceab` |
| Release | `target/release/mdlite.exe` | `b80c9639cbc646195157c61e92440d7ccaa6f506d43fd215727017bcaf6a9c26` |

harness SHA256: `ace0a3dd8d4d9928d6003de32514b7f46e13ea02789fc472e1affe38711ea509`。
最終GUI実行では共通設定/Trust用LOCALAPPDATAと共通profile用APPDATAを所有fixtureへ隔離する。
以前のDebug smokeはLOCALAPPDATAだけを隔離しており、Roaming側の非参照を証明しない。
そのsource/harness実行は失効済みの履歴として扱う。現在のGUI結果は別欄で記録する。

独立reviewはRoot/実装ownerと別のSol Highのread-only依頼。実効role自体は書込み可能であり、
read-only sandboxと偽らない。Sourceと回帰fixtureを確認し、最終指摘なし。reviewerは試験/GUIを実行していない。
Ponytailは実装者の別確認。Reparse fixtureのsymlink作成は権限error 1314でSKIPし、実再parse挙動の試験合格には含めない。

## 最終Releaseの起動・GUI制約

最終Release EXEの所有PID/HWNDとhash一致、visible/respondingを確認した。
Windows foreground確認で停止し、harness結果の原記録は `FAIL / RUNTIMEEXCEPTION`、
操作上の分類は **BLOCKED（foreground gate）**。製品操作のassertion失敗ではない。
UiParity 15 caseは全てNOT_RUN。keyboard/mouse入力、screen capture、実IME操作は一切実行していない。
identityのbefore/afterは不変で `final_proof_valid=true` だが、これはGUI合格を示さない。

登録buildのsource digestは前記 `0242a4...`。harnessは別manifest形式
（`relative path` + space + file SHA、旧Windows separator処理）のbefore/after照合で
`b45c8bd8987db9012ece7c6651c9c590cdd59c3914369f1d71deb176997041b3` を記録した。
現在の同じfile bytesから両algorithmを計算して照合し、異なるhashを同じ形式とは扱わない。

所有processの1回だけの読取りではWorking Set 45,527,040 B、Private 26,595,328 B、
Peak Working Set 45,568,000 B。文書数はgate前のため未観測で、50MB目標や製品性能の合格には使わない。
実windowのUI比較・全新入口の操作・連続Editor/IME/保存/closeは未確認。
禁止中clipboard、実storage、physical/MS IME/ATOK/device/Human条件は未実施のまま保持する。

## 完了時に必要な証拠

要件ごとに実装箇所、実入口、保存先、取消・失敗、機械試験、対象buildのnative操作、
Human/機器/権限待ちを分ける。helper試験だけでUI接続を合格にしない。
最終Debug/Release、fmt、Clippy、独立correctness reviewとPonytail確認、通常develop統合、
両remoteの固定OID読戻し、local/Drive HANDOFFが必要。

OS clipboardの読取り・形式列挙・書込み・消去、Ctrl+C/X/V、画像pasteの試験は禁止を維持する。
payload/mockの検証とOS統合受入は別に扱う。実Git UI試験は架空repo/local bare remoteに限定する。
実ノート、実index、実storage uploadを試験に使用しない。

MS IME全matrix、物理長押し/F5、別monitor/DPI、Narrator/touchpad、実storageサービス、
本人の操作感は、実行できた範囲と未実行条件を対象buildに結び付けて記録する。
実装不足をHuman待ちに分類しない。

## 全機能の接続状況

作業途中の棚卸し。coreのPASSは製品の実入口・visible合格を意味しない。

接続状況：`native.rs::App::command()` はshell commandを先に解決し、`native/shell.rs::commands()` が
module command tableをpaletteへ追加する。`WM_COMMAND`・toolbar/menu・登録済みkeybinding・paletteから
`App::shell_command()` を通じてFeature/Search/Git/Editor/Diagnosticsへdispatchする。
検索paneは5000–5010、Git paneは6000–6025、table/image actionsは20–39、診断窓は167。
これはsource接続の証拠であり、visible/GUI合格ではない。

設計判断：Compactは同じApp/Document/historyを単一の編集ownerで別windowへ表示する。
復旧ファイルは寿命identityとcreate-newで旧候補を保護し、原disk fingerprintを照合できない場合は
encoding hint付き無題forkとして保持する。本文はAppData/sessionへ入れない。
文書SVGは今回の契約に合わせて取込み・解決を拒否し、共有UI icon用の安全なdecoderとは分ける。

局所検証：基準Debug131、identity4、workspace6、settings8、native persistence5、
panel2、navigation4、session3、recovery3、search2、workspace search3、
profile6、calendar7、image reference4がPASS。Git/Trust/processは担当の合成repo試験とClippyがPASS。
診断sliceでは `cargo test native::diagnostics::tests -- --nocapture` が2 PASS
(privacy canary/current-process memory)。この診断テストはshared command route接続前の記録。
これらは各slice時点の結果であり、最終sourceの全suite/visible合格へ読み替えない。
Initial設計判断の指摘（復旧寿命・baseline・anchor重複/二重decode・collapsed寸法）は局所修正済み。
Jev Reviewはsol61_highを選択。同じ独立reviewerが修正後と最後の時計routing/harness隔離を再確認し、指摘なし。ClosureとGit/HANDOFFの結果はlocal packetへ記録する。

W02のsource経路は`WorkspaceIdentity`のphysical identity、`instance::claim`のnamed mutex
lease、`Persistence`のlease保持、current-executable windowだけへのduplicate通知、
別Workspace時の`current_exe`起動まで接続済み。X01はGit/storageの明示操作に使う
owned bounded process workerであり、任意processを起動するUI要求ではない。どちらもvisible操作は未確認。
clipboard OS統合試験は禁止を維持する。最終横断試験、独立review、配送、HANDOFFも未完了で、
いずれもsource接続とは別の完了条件。
入力fetch/旧snapshot保全・基準branch作成・変更前131 testsは完了しており、再開時にやり直さない。

下表のtest IDは現在のsourceにある関数名で、PASS表示はRootが報告した該当sliceだけを指す。
UiParity欄は`Invoke-UiParityCase`に定義されたcase名であり、実行結果やtarget-build合格を意味しない。
現在のvisible input、physical input、MS IME/ATOK/current profile、OS clipboard、storage service、Human受入はNOT_RUN。
PrintWindowではDirect2D内容を確認できない。最終Debug/Release source・EXE SHA-256 identity、full suite、review、
GUI配送欄はRootが最終build後に記入する。

| ID | 実装入口・適用条件 | 保存先・失敗/取消 | 現在のsource test ID（名前のみ） | UiParity case / GUI | Human |
|---|---|---|---|---|---|
| E01 | `document::Document` と `coordinates`; 全編集・選択はnative editorから`crop::Rope`へ | 編集中はmemoryのみ。範囲/座標エラーはatomic reject、Undo/Redoで選択も復元 | `document::tests::{randomized_edits_and_history_match_independent_string_states,invalid_ranges_and_exhausted_revisions_are_atomic,edit_undo_redo_restores_source_selection_and_saved_state}`; `coordinates::tests::utf8_byte_and_utf16_offsets_round_trip_unicode` | 専用caseなし; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| E02 | `native::window_proc`のIME通知、`persistence`; Save 3 / Close 5。composition中は実通知を優先 | sourceは指定file、recoveryはWorkspaceの`.mdlite/.state`; IME cancel失敗やclose cancelは未保存sourceを保持 | `native::tests::{ime_failed_cancel_and_composing_escape_enter_preserve_table_and_history,editor_close_waits_for_result_and_real_end_without_discarding_unsaved_source}`; `native::persistence::tests::persistence_autosave_defers_ime_and_preserves_peer_changes` | `DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE`, `TABS_DUPLICATE_OPEN_SAVEAS_COLLISION`; 定義済み、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| E03 | `native/editing` と `native::window_proc`; editor focus時の通常key/Markdown入力、IME通知を先に処理 | document内の`crop::Rope`; invalid editは変更なし、Undoで戻せる | `native::tests::{editor_live_authoring_heading_delimiters_enter_undo_and_followup,editor_enter_continuation_exit_code_indent_and_shift_tab,editor_home_end_page_control_lines_unicode_selection_and_idle_group}` | `COMPACT_EDIT_UNDO_CLOSE_RETURN`は限定編集のみで全key matrixではない; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| E04 | `table::TableEdit`, `native/editor_actions` 20–39, `assets`; 有効なtable cellまたはlocal imageが対象 | Markdown sourceはdocument内。importはbounded local read、拒否時はdocument/fileを保持。menu cancelは変更なし | `table::tests::structural_edits_are_one_atomic_bounded_replacement_and_protect_minimum_shape`; `native::editor_actions::tests::table_entry_edits_insert_above_and_left_in_one_undo`; `assets::tests::rejected_import_keeps_the_document_and_source_file_unchanged` | 専用caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U01 | `native::window_proc`, `NativeShell`; startupとmain-window caption/button input | shell stateはprocess memory; source/historyはAppが所有。window creation errorはOS return経由 | `native::tests::startup_initializer_creates_default_document_before_window_state`; title-specific testなし | `STARTUP_VISIBLE_LAYOUT`はStartupSmoke layoutのみ; title操作caseなし、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U02 | `panel::PanelLayout`, `native/shell.rs::App::{load_panel_layout,save_panel_layout}`; 4 slot/collapse/resize command 160–165 | `.mdlite/.state/panel-layout.toml`; invalid schema/geometryはdefault/原本保持、save errorはstatus表示 | `panel::tests::{legacy_roundtrip_rejects_duplicates_unknown_schema_and_nan,narrow_geometry_reserves_editor_and_keeps_rects_in_window}`; `native::shell::tests::shell_geometry_narrow_drawer_and_slot_swap_remain_reachable` | `SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL`; 定義済み、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U03 | `native/shell.rs::App::shell_window_message`, activity control IDs 110–114; button clickでExplorer/Search/Git/Calendar/Settingsを選ぶ | active panelはmemory only; failed panel layoutはU02の原本保持 | `native::shell::tests::shell_geometry_narrow_drawer_and_slot_swap_remain_reachable`; activity click専用testなし | `SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U04 | `App::open_path`, document identity and tab switch; 同一physical fileは既存tabを選択、open文書へのSave As collisionは拒否 | file sourceとsessionのrecent path; 9件固定制限なし。collisionでtargetを上書きしない | `native::tests::{document_identity_duplicate_open_activates_existing_for_path_aliases,document_identity_save_as_refuses_clean_or_dirty_open_target,document_identity_tab_overflow_remains_reachable_by_ctrl_tab}` | `TABS_DUPLICATE_OPEN_SAVEAS_COLLISION`; 定義済み、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U05 | Compact 102/Return 103 → `native/shell.rs::App::{open_compact,return_from_compact}`; same App/Document owner in separate window | session window placementは`.mdlite/.state/session.toml`; compact open/close failureはsource/historyを維持 | `native::shell::tests::shell_focus_guard_and_compact_surface_share_canonical_history`; `native::tests::document_identity_tab_overflow_remains_reachable_by_ctrl_tab` | `COMPACT_EDIT_UNDO_CLOSE_RETURN`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| U06 | `native/presentation::editor_status_text`, shell footer paint; current tab/revision status | status/EOL cacheはmemory only、document bodyを複製保存しない。cacheはrevisionで更新 | `native::presentation::tests::{editor_status_uses_recovery_format_and_known_external_state,status_clipping_is_bounded_and_removes_line_breaks}`; `native::shell::tests::custom_heading_and_code_background_colors_reach_shell_tokens` | 専用caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U07 | `native.rs::App::command` → `native/shell.rs::App::shell_command` → `shell::commands`; Feature/Search/Git/Editor/Diagnostics registry | 各command固有の保存先。`App::command_enabled`はIME/background mutation等をgateし、dispatch errorは各pane statusへ | `native::shell::tests::shell_binding_dispatch_respects_remapping_and_reserved_commands`; `native::features::tests::feature_buttons_use_shared_command_enablement_gates` | shell caseと各feature caseに分散; 定義済みcaseのGUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U08 | palette command IDs 100/105–106、`native/shell.rs::App::command_enabled`; settings command 101; metadata登録commandのみkeybinding対象 | `%LOCALAPPDATA%/MDLite/settings.toml`とWorkspace `.mdlite/settings.toml`; reserved/conflicting bindingはapply拒否、cancelはwriteなし | `settings::tests::binding_validation_covers_editor_and_panel_reservations`; `native::shell::tests::{shell_palette_query_cancel_and_native_ime_keep_document_selection,shell_binding_dispatch_respects_remapping_and_reserved_commands}` | `SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL`, `SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| U09 | `native/shell.rs::App::apply_shell_settings`, `GetDpiForWindow`, FeaturePanes geometry; theme/DPI対応pane | common/workspace settings TOML; invalid per-key valueはfallback、apply conflict/cancelでは外部変更を上書きしない | `settings::tests::{layers_are_per_key_and_defaults_match_the_product_contract,malformed_values_fall_back_per_key_and_schema_or_duplicates_block_apply}`; `native::shell::tests::shell_final_rect_font_and_panel_changes_preserve_source_and_hit_coordinates`; `native::features::tests::calendar_toolbar_bounds_do_not_overlap_month_or_weekday_headers` | `SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP`; DPI/accessibilityは未実施、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| W01 | `workspace::Workspace::{open,initialize,list_directory}`, `native/persistence.rs::App::{choose_workspace,open_workspace}`; Workspace command 7/Feature 4011でlocal directoryが選択された時 | Workspaceに`.mdlite/.state`/`.cache`;初期化は同意後。invalid root/reparse escapeで拒否、prompt Noなら編集継続 | `workspace::tests::{open_does_not_initialize_and_identity_uses_canonical_root,initialize_preserves_existing_files_and_is_idempotent,explorer_is_folder_first_stable_and_hides_protected_paths,symlink_escape_is_rejected_and_listed_as_incomplete}` | `EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL`; workspace root/open全体は未証明、GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| W02 | `instance::claim`, `native/persistence.rs::App::open_workspace_new_process`, recent/session hydrate; canonical Workspace identityでduplicate判定 | `.mdlite/.state/session.toml`; lock取得/読み込み失敗はstatus、別Workspaceのみ`current_exe`起動 | `instance::tests::duplicate_lease_releases_without_writing_workspace`; `session::tests::current_session_roundtrip_contains_no_body`; `native::tests::{startup_initializer_creates_default_document_before_window_state,startup_initializer_opens_multiple_synthetic_files_before_window_state}` | `PARENT_ROOT_METADATA`; `STARTUP_VISIBLE_LAYOUT`はStartupSmokeのみ。GUI NOT_RUN（foreground未確認） | NOT_RUN |
| W03 | `explorer_tree` projection、`FeaturePanes::sync_explorer_control`、`App::features_command` 4009、owner-draw 4690/itemData depth+folder+expanded | tree expansionはmemory view state; Workspace files以外に本文を保存しない。read issue/row boundはincomplete状態、取消はworker cancel | `explorer_tree::tests::{expanded_rows_are_lazy_stable_and_leave_selection_to_the_caller,protected_paths_invalid_roots_and_row_bounds_are_reported,reparse_directories_are_not_followed,depth_bound_returns_an_incomplete_projection}`; `native::features::tests::{explorer_list_sync_preserves_hidden_selection_and_scroll_position,explorer_list_enter_and_arrows_expand_folders_from_native_messages,explorer_selection_notifications_keep_collapsed_rows_and_set_owner_draw_data}`; `native::explorer_icons::tests::explorer_tree_rows_select_directory_and_extension_classes`; `native::shell::tests::shell_owner_draw_does_not_consume_explorer_rows` — Root報告source slice PASS、reparse privilege fixtureは明示SKIP。final full suite pending | tree/icon専用UiParity caseなし。`EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL`はtree expansion/iconを証明しない; GUI NOT_RUN | NOT_RUN |
| W04 | `native/features.rs::App::features_command` 4001–4002 → `Workspace` create; valid parent/name and write-enabled Workspace | new file/folder is Workspace local path; existing target/name syntax errors refuse replace; 4086 cancels prompt | `workspace::tests::create_uses_validated_names_and_never_replaces`; `native::features::tests::explorer_name_validation_rejects_windows_path_syntax` | `EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| W05 | Feature 4003–4004 `copy_selected_paths/paste_files`; selection must be copyable and destination inside Workspace | internal copy buffer is process memory; files are copied within Workspace. preflight prevents partial overwrite; OS clipboard is not used/tested | `workspace::tests::copy_preflights_all_targets_and_copies_folder_trees` | `EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL` covers internal copy only; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| W06 | Feature 4004–4005 move/rename and Explorer drop; selected item and destination folder required | Workspace file paths; open-file/ancestor/overwrite targets rejected; form cancel is 4086, drag Escape cancels | `workspace::tests::move_refuses_open_files_and_ancestors_then_moves_without_overwrite`; `native::features::tests::pane_keys_do_not_consume_editor_navigation_and_drag_cancel_is_unchanged` | `EXPLORER_NEW_RENAME_INTERNAL_COPY_PASTE_OPEN_REFUSAL` covers rename, not drag/drop; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| W07 | Feature 4006–4008 `recycle_selected/copy_path_payload/reveal_selected`; selected local item | Recycle bin/Explorer are OS actions; path copy may touch OS clipboard and is intentionally not tested. shell failure surfaces in status/dialog | dedicated testなし; OS clipboard behavior excluded | Explorer case does not cover trash/path clipboard/reveal; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| N01 | Feature 4000 → `native/features.rs::App::open_quick_open` / `FeaturePanes::start_quick_open` → `workspace_search::quick_open`; Workspace required, query/recent paths constrain results | Results/cancellation are memory worker state; no persistent index. invalid ignore/cancel is incomplete/error, not empty success | `workspace_search::tests::{ignore_globs_dirty_sources_binary_and_late_quick_open_target,invalid_ignore_and_cancel_are_incomplete_not_empty_success}` | `QUICK_OPEN_BLANK_LIMIT_AND_MORE`, `QUICK_OPEN_LATE_FILE_QUERY_201_PLUS`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| N02 | Feature 4040 → `native/features.rs::App::open_outline_selection` / `navigation::Outline`; Markdown outline row must resolve to current revision | selection/scroll only, source unchanged; stale selection refuses jump | `navigation::tests::parser_outline_setext_frontmatter_code_duplicate_and_untitled_anchor`; `native::features::tests::feature_snapshot_reuses_current_outline_for_same_revision` | `OUTLINE_JUMP_MOVE_SECTION_UNDO`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| N03 | Feature 4041–4042 → `App::move_outline_selection` → `navigation::move_section`; heading selection must be current | document source gets one undoable move; stale revision/self/invalid target returns error without edit; drag Escape cancels capture | `navigation::tests::move_section_with_children_is_one_undo_and_rejects_stale_ime_and_self`; `native::features::tests::{outline_toolbar_moves_whole_parent_sections_in_one_undo,outline_drag_escape_from_header_cancels_capture_without_edit}` — Root報告parent subtree up/downとheader Escape slice PASS、final full suite pending | `OUTLINE_JUMP_MOVE_SECTION_UNDO` is defined but GUI NOT_RUN | NOT_RUN |
| N04 | Link 166 → `native/shell.rs::App::shell_command` and `App::link_destination_at`; Markdown link/anchor under caret | local link opens only resolved file; invalid escapes/arbitrary schemes rejected; no source rewrite | `navigation::tests::{anchor_collision_and_single_fragment_decode_are_unambiguous,links_reject_arbitrary_schemes_and_broken_escapes}` | dedicated link activation caseなし; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| S01 | `settings::resolve_settings`; combine machine common and Workspace layer per key | `%LOCALAPPDATA%/MDLite/settings.toml`, `.mdlite/settings.toml`; malformed values fallback per key; unsupported schema/conflicts block apply | `settings::tests::{layers_are_per_key_and_defaults_match_the_product_contract,malformed_values_fall_back_per_key_and_schema_or_duplicates_block_apply,added_root_override_is_inserted_before_first_section_and_inherit_removes_it}` | `SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP`; resolver itself not GUI evidence, GUI NOT_RUN | NOT_RUN |
| S02 | `native/settings_dialog.rs::App::show_settings` → captured-layer patch; only edited fields apply | common/workspace settings TOML; cancel creates no absent file, peer edits/deletion refuse apply | `native::settings_dialog::tests::settings_form_patch_only_changes_edited_layer_fields`; `settings::tests::{absent_target_is_not_created_on_cancel_and_create_new_detects_peer_creation,captured_existing_file_refuses_external_replacement_or_deletion}` | `SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| S03 | settings command 101 → `App::show_settings` and registered detail controls; only supported registered fields are editable | same settings files; shortcut conflicts/schema errors block write, Cancel leaves file untouched | `settings::tests::{binding_validation_covers_editor_and_panel_reservations,patch_rejects_shortcut_conflicts_without_writing}`; `native::settings_dialog::tests::settings_form_patch_only_changes_edited_layer_fields` | `SETTINGS_APPLY_CANCEL_KEYBINDING_REMAP`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D01 | `App::save` and `FileSnapshot`; Save 3 / Save As 4; current document and chosen target must be valid | source saved to requested document path with selected encoding/EOL; stale bytes/open-target collision preserve existing file and dirty state; dialog Esc cancels | `storage::tests::{save_as_refuses_an_existing_path_and_writes_a_new_utf8_file,stale_file_bytes_are_refused_without_overwriting_external_content,cp932_round_trips_supported_text_and_rejects_unmappable_edits}`; `native::tests::document_identity_save_as_refuses_clean_or_dirty_open_target` | `TABS_DUPLICATE_OPEN_SAVEAS_COLLISION`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D02 | `native/persistence.rs::App::persistence_tick` autosave schedule; only enabled, dirty, eligible document outside IME | recovery record under `.mdlite/.state`; write failure retains old record/status, IME defers write | `native::persistence::tests::persistence_autosave_defers_ime_and_preserves_peer_changes` | `DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D03 | Recovery 8/autosave schedule → `RecoveryStore`; snapshots are created for eligible dirty source | Workspace `.mdlite/.state`; record failure retains previous candidate; no document body in AppData | `recovery::tests::{recovery_roundtrip_untitled_never_marks_saved_and_requires_explicit_unchanged_discard,legacy_body_and_unknown_schema_are_preserved}`; `native::persistence::tests::persistence_recovery_keeps_one_owned_record_and_preserves_workspace` | `DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| D04 | Recovery 8/Discard recovery 11 → `native/persistence.rs::App::{restore_recovery,discard_tab_recovery}`; candidate must match identity/baseline and user action | `.mdlite/.state`; unknown schema/original candidate preserved; changed source cannot be discarded as unchanged; dialog No/cancel retains session | `recovery::tests::recovery_roundtrip_untitled_never_marks_saved_and_requires_explicit_unchanged_discard`; `native::persistence::tests::{persistence_restore_uses_verified_baseline_and_preserves_fork_format,persistence_untitled_view_binds_recovery_and_close_no_does_not_recreate}` | `DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| D05 | shell Compare 9/Reload 10 → `native/persistence.rs::App::{compare_disk,reload_disk}`; a saved document snapshot is required | Reads source file only; reload replaces only after confirmation/current identity check; stale disk or decode error preserves document | `storage::tests::{stale_file_bytes_are_refused_without_overwriting_external_content,save_as_existing_retains_source_format_and_rejects_loss}`; `native::persistence::tests::persistence_reload_failures_preserve_source_history_and_baseline` | dedicated compare/reload caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D06 | `session::{save,hydrate}` plus `App::persistence_tick` view capture; view applies only to matching source identity/hash | `.mdlite/.state/session.toml` stores view/selection/placement/recent paths, not body; stale view is deferred without clamping | `session::tests::{reversed_legacy_utf16_and_view_offsets_convert_without_clamping,current_session_roundtrip_contains_no_body,source_bound_views_reject_different_text_even_with_valid_offsets}`; `native::persistence::tests::persistence_horizontal_table_view_captures_latest_scroll_and_restores_range_key` | `DIRTY_CLOSE_CANCEL_RESTART_RECOVERY_NORMAL_CLOSE`, `COMPACT_EDIT_UNDO_CLOSE_RETURN`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D07 | `native/shell.rs::App::{load_panel_layout,save_panel_layout}`; layout commands 160–165, only valid panel geometry | `.mdlite/.state/panel-layout.toml`; invalid/changed peer snapshot refuses overwrite; Reset and Escape have separate effects | `panel::tests::legacy_roundtrip_rejects_duplicates_unknown_schema_and_nan`; `native::shell::tests::shell_geometry_narrow_drawer_and_slot_swap_remain_reachable` | `SHELL_ACTIVITY_PANEL_SLOT_COLLAPSE_RESET_RESIZE_PALETTE_CANCEL`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| D08 | `native/persistence.rs::App::{open_workspace,restore_recovery}`; records remain bound to their owning Workspace | Each root's `.mdlite/.state`; mismatch defers record rather than cross-writing; read/write failure preserves source and existing record | `recovery::tests::lifetimes_keep_retained_records_and_preserve_opened_disk_baseline`; `native::persistence::tests::{persistence_recovery_keeps_one_owned_record_and_preserves_workspace,persistence_dirty_restart_defers_view_until_exact_recovery_source}` | dedicated Workspace replacement/recovery caseなし; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| C01 | Feature 4023/4028/4024/4025 → `prepare_profile_at` / `profiles::create_profile_note`; 4023は選択日0時、4028と通常profile作成は現在local日時。Workspace/profileが必要 | Note is create-new under Workspace calendar path; template validation/collision errors create no partial note; form cancel does not save | `native::features::tests::{calendar_daily_click_uses_midnight_while_today_and_generic_routes_capture_local_now,today_daily_command_uses_workspace_profile_and_ime_gates}`; `profiles::tests::{create_validates_template_before_directories_and_sequences_collisions,broken_template_does_not_block_opening_an_existing_daily}`; `calendar::tests::daily_marker_requires_a_regular_file` | `CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| C02 | Feature 4027/4080–4088 → profile manager and template forms; valid profile/template selected | Workspace `.mdlite/profiles.toml`; common `%APPDATA%/MDLite/profiles.toml`; invalid/default required input blocks create, Cancel leaves layer unchanged | `profiles::tests::{required_empty_default_is_valid_for_save_and_preview_but_blocks_creation,default_templates_are_created_only_when_missing,profile_layers_replace_whole_definitions_and_retain_peer_edits}` | `CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE`; profile form GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| C03 | Feature 4088 profile create → `create_profile_note`; validates path and template before create-new | Workspace note path; collision/template/symlink failures create no partial directories or overwrite existing notes | `profiles::tests::{create_validates_template_before_directories_and_sequences_collisions,template_symlink_escape_is_rejected_before_note_creation}` | `CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| C04 | Feature 4020–4022 Calendar month/date navigation; calendar pane is visible and date in supported range | view state only; no file write until create command. Invalid/out-of-range date does not select/create | `calendar::tests::{month_grid_has_sunday_first_42_valid_cells_and_handles_range_edges,navigation_focus_selection_and_activation_are_separate}` | `CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE` selects a date but is not a full month-navigation matrix; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| C05 | Calendar list request/action and Feature 4023–4025; selected Workspace/day and rechecked regular text file required | Directory listing is read-only; missing/binary/stale request is refused, worker cancellation is reported. No list cache/body persistence | `native::features::tests::calendar_file_action_rechecks_request_missing_and_binary_entries_before_open` — Root報告recheck slice PASS; `calendar::tests::{daily_marker_requires_a_regular_file,supported_text_extensions_align_with_calendar_open_and_binary_check}` are source IDs; final full suite pending | `CALENDAR_SELECTED_DAILY_AND_PROFILE_CREATE` does not establish list-row behavior; GUI NOT_RUN | NOT_RUN |
| C06 | Feature 4026 → `native/features.rs::App::import_holidays` → `HolidayCalendar::{import_csv_file,import_csv_bytes}`; user-selected CSV and supported encoding required | `.mdlite/.cache/holidays.csv`; decode/parse/cache failure keeps current calendar state; file dialog cancel makes no import | `calendar::tests::{holiday_import_decodes_utf8_bom_and_keeps_memory_on_cache_or_csv_failure,holiday_import_accepts_cp932_and_validates_real_dates_and_names}` | `CALENDAR_INVALID_HOLIDAY_CSV`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| F01 | Search 5000–5010 → `native/search_ui.rs::App::search_command` and pane controls; search pane and query scope selected | query/result data memory only; no files written by search; invalid query shows error and retains doc | `native::search_ui::tests::{empty_glob_lines_remain_empty_and_nonempty_patterns_are_preserved,line_column_counts_unicode_scalars_and_crlf}` | `SEARCH_DIRTY_DOCUMENT_AND_WORKSPACE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| F02 | `SearchPane` generation plus `search::SearchSnapshot`; query reruns on active document ID/revision change | result snapshots memory only; stale generation/cancel discards old results | `native::search_ui::tests::document_query_refreshes_on_edit_and_tab_switch_without_query_change`; `workspace_search::tests::physical_alias_uses_one_dirty_source_and_one_replacement_target` | `SEARCH_DIRTY_DOCUMENT_AND_WORKSPACE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| F03 | Search 5000–5006 → `App::search_command`, `search::{Query,search_snapshot,Results::replace}` PCRE2 and capture formatter; regex mode selected | no persistent storage until replace; compile/limit errors do not edit source; cancel stops worker | `search::tests::{lookaround_backreference_unicode_captures_and_literal_replacement,boundaries_zero_length_cancel_deadline_and_pathological_regex_are_bounded}` | `REPLACE_PREVIEW_AND_JOURNAL_RESTORE`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| F04 | Workspace Search scanner in `workspace_search::quick_open`/SearchPane; Workspace root, ignore rules and dirty docs apply | scan results memory only; invalid ignore, I/O error or cancel is incomplete/error, not empty success | `workspace_search::tests::{ignore_globs_dirty_sources_binary_and_late_quick_open_target,physical_alias_uses_one_dirty_source_and_one_replacement_target,invalid_ignore_and_cancel_are_incomplete_not_empty_success}` | `SEARCH_DIRTY_DOCUMENT_AND_WORKSPACE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| F05 | Search/scanner and SearchPane count/status; configured limits apply before excessive work | no persistent storage; deadline/cancel/limit marks results incomplete and reports status | `search::tests::boundaries_zero_length_cancel_deadline_and_pathological_regex_are_bounded`; `workspace_search::tests::invalid_ignore_and_cancel_are_incomplete_not_empty_success` | Search UI case covers ordinary results only, not all limit boundaries; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| F06 | Search 5004–5005 → `search::Results::replace` for open documents; document/revision plan must still match | open document changed in memory as one Undo; stale plan rejects without edit; user can Undo | `replace::tests::open_document_plan_is_revision_and_source_guarded`; `document::tests::each_range_replacement_is_one_transaction_and_branches_drop_redo` | `REPLACE_PREVIEW_AND_JOURNAL_RESTORE`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| F07 | Search 5006 preview/apply → `replace::{preview_workspace_replace,apply_closed_replacements}`; selected targets must be closed and snapshot-stable | Workspace targets plus `.mdlite/.state/replace/*.journal`; stale preflight/journal failure changes no files; cancel stops remaining writes | `replace::tests::{stale_preflight_and_journal_directory_failure_leave_closed_files_unchanged,cancellation_stops_after_one_closed_file_and_reports_remaining_counts}` | `REPLACE_PREVIEW_AND_JOURNAL_RESTORE`; GUI NOT_RUN（foreground未確認） | NOT_RUN |
| F08 | Search 5007–5008 → `enumerate_journals/preview_recovery/restore_journal`; valid selected journal and exact after-snapshot required | `.mdlite/.state/replace`; broken/unsupported journal is preserved, open/changed targets refuse restore, preview cancel writes nothing | `replace::tests::{recovery_refuses_open_tabs_and_any_changed_after_snapshot,legacy_v1_and_v2_journals_are_read_and_restored_without_rewriting_them,broken_legacy_journal_is_listed_with_message_and_preserved}` | `REPLACE_PREVIEW_AND_JOURNAL_RESTORE`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G01 | Git 6001–6002 → `TrustStore`; trust is explicit for current canonical Workspace identity | `%LOCALAPPDATA%/MDLite/trust/v2`; revoke removes token; stale/foreign identity denies Git and external execution | `trust::tests::physical_identity_revocation_and_busy`; `native::git_ui::tests::native_git_disabled_reasons_match_workspace_trust_selection_remote_and_ime` | `GIT_UNTRUSTED_GATE_AND_LOCAL_STAGE_UNSTAGE_COMMIT`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G02 | Git 6000/6004 → `GitPane` status/diff worker; trusted Workspace with repository required | read-only `.git`/worktree snapshot; untrusted/no-repository error retains current UI state | `git::tests::{complete_porcelain_rename_conflict_and_malformed,no_repository_no_git_and_untrusted_read_states}`; `native::git_ui::tests::native_git_explicit_worker_fixture_selected_literal_diff_and_untrusted` | `GIT_UNTRUSTED_GATE_AND_LOCAL_STAGE_UNSTAGE_COMMIT`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G03 | Git 6005/6006/6008 stage/unstage/commit; trusted repository and explicit selection required | local `.git/index` and commit history; command failure leaves status visible, no automatic commit/push | `git::tests::synthetic_repo_literal_selected_unborn_index_commit_and_local_remote`; `native::git_ui::tests::native_git_explicit_worker_fixture_selected_literal_diff_and_untrusted` | `GIT_UNTRUSTED_GATE_AND_LOCAL_STAGE_UNSTAGE_COMMIT`; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G04 | Git 6009–6012 branch/switch/merge/abort; trusted repo, valid branch state and explicit action | local `.git`/worktree; conflict is surfaced, Abort returns to prior branch state; merge error does not auto-resolve | `git::tests::synthetic_conflict_merge_abort_and_same_literal_stage_contract`; `native::git_ui::tests::native_git_conflict_one_undo_and_stale_selection_guard` | dedicated branch/merge UI caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G05 | Git 6013–6015 fetch/pull/push; trusted configured remote and explicit operation required | remote refs/worktree; tests use local bare remote only. remote/auth/cancel errors are surfaced; no real remote executed | `git::tests::synthetic_repo_literal_selected_unborn_index_commit_and_local_remote`; `native::git_ui::tests::native_git_storage_worker_snapshot_pipeline_mock_only` | Git UI case covers local stage/commit, not remote; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| G06 | Git 6016–6022 conflict selection/resolve; current conflict path and literal diff selection required | local worktree/index; stale selection/conflict state refuses write, Undo only for supported source edits | `git::tests::synthetic_conflict_merge_abort_and_same_literal_stage_contract`; `native::git_ui::tests::{native_git_conflict_one_undo_and_stale_selection_guard,native_git_source_edit_preserves_reverse_selection_outside_destination}` | dedicated conflict UI caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| X01 | `ProcessRequest` → `process::run`; only Git/storage explicitly request bounded commands, Workspace trust required | process output bounded in memory, Job Object owns child; timeout/cancel/overflow become typed outcomes; no arbitrary-process UI | `process::tests::{cancelled_request_does_not_spawn,owned_process_outcomes_and_simultaneous_bounded_output}`; `storage_adapter::tests::mock_executable_gates_failures_cancel_hash_and_stale_remote_success` | dedicated external-process UI caseなし; GUI/service操作未実施 | NOT_RUN |
| I01 | Editor 20–26/32–34/38–39 → `native/editor_actions.rs::App::editor_action` table command; valid current table/caret boundary required | edits only document source in one transaction; malformed/minimum-shape table refuses; Undo restores | `table::tests::{structural_edits_are_one_atomic_bounded_replacement_and_protect_minimum_shape,header_only_tables_include_delimiter_and_insert_after_it_with_table_eol}`; `native::editor_actions::tests::{table_entry_edits_insert_above_and_left_in_one_undo,table_entry_edits_keep_header_and_minimum_structure_protected}` | dedicated table menu/toolbar caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| I02 | Editor 27–31 → image reference/width commands; selected supported local image token required | only Markdown width digits/source spelling change; image file remains untouched; invalid ratio/geometry refuses | `image_reference::tests::{legacy_width_keeps_alt_title_quotes_and_other_attributes_literal,markdown_width_changes_only_digits_and_path_roundtrips}`; `native::editor_actions::tests::{fixed_image_widths_preserve_markdown_and_legacy_spelling,ratio_widths_are_bounded_and_reject_invalid_geometry}` | dedicated image-width caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| I03 | `assets` decode/projection and image layout; supported local raster/animation only, document SVG is refused | reads local image only within decoder limits; reject preserves document/file, no network fetch | `assets::tests::{normal_fixture_formats_decode_with_the_checked_dimensions,oversized_sparse_inputs_are_rejected_before_reading_bodies,rejected_import_keeps_the_document_and_source_file_unchanged,svg_is_local_and_passive}`; `image_reference::tests::encoded_local_paths_resolve_once_and_document_svg_is_refused` | dedicated image draw/import caseなし; GUI NOT_RUN（foreground gate未確認） | NOT_RUN |
| I04 | Git 6023–6024 → `GitPane` storage adapter controls and `storage_adapter::UploadPlan`; trusted Workspace and confirmed target required | `.mdlite/storage.toml`; mock executable only; hash/stale target/cancel/failure prevents marking success; live service not exercised | `storage_adapter::tests::{configuration_snapshot_validation_and_literal_expansion,uri_validation_rejects_unsafe_and_accepts_encoded_https,mock_executable_gates_failures_cancel_hash_and_stale_remote_success}`; `native::git_ui::tests::native_git_storage_worker_snapshot_pipeline_mock_only` | no live storage UI/service case; GUI/service未実施 | NOT_RUN |
| A01 | Diagnostics 167 → modeless manual-refresh window; reads current process only | snapshot memory only; sensitive note/query/path canaries omitted; no document-body storage | `native::diagnostics::tests::{snapshot_omits_synthetic_note_query_and_absolute_path_canaries,process_status_reports_nonzero_finite_values_for_this_process}` | Diagnostics caseなし; command route/current visible operation未確認 | NOT_RUN |
| A02 | Release/acceptance gates, not a runtime command: final source/build identity, suite, Clippy, independent correctness and Ponytail review, refs/HANDOFF | 最終source/EXE identityとDebug/Release各285 tests・fmt・Clippy・独立reviewは上記。Git/HANDOFF結果はlocal packetへ記録; no runtime storage | 品質専用runtime test IDなし。全suite core139/native146の最終PASSは上記global evidence。個別rowは対応する境界を示す | UiParityは品質/配送の代替でない; 最終Release foreground gateで入力前停止、GUI操作BLOCKED | NOT_RUN |
