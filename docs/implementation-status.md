# MDLite 表示・UI再構築 実装・受入状況

更新日: 2026-09-28
対象Task: `20260925-mdlite-approved-ui-live-editor-rebuild` Revision 2

この表は実装、自動試験、実GUI経路試験、Human確認、外部準備待ちを分離する。
`実装済み`や`自動PASS`はHumanの操作感・見た目の受入を意味しない。性能値と
R-01〜R-08の根拠は [acceptance-hardening-report.md](acceptance-hardening-report.md) に記録する。今回の座標・UI設計は
[ui-design.md](ui-design.md) と [adr-20260921-native-coordinate-ui.md](adr-20260921-native-coordinate-ui.md) に記録する。

## 今回の変更と境界

- Jev Initial Decision Gate は `sol_xhigh` を選択した。固定 specialist の判定は、既存の座標混同を解消してから UI/table を再構築するよう指示した。実行 provenance が要求 target と不一致だったため dispatch は fail-closed とし、model/effort を推測しない。
- source/export/native の座標を分離し、CRLF/LF と Markdown image object は compact discontinuity で写像する。`EditorText` はraw native textを取得後、CRLF/lone LFをnative CRへcanonicalizeしてから差分・object復元へ渡す。同幅のlone LFはdiscontinuityへ記録せず、巨大文書で不要なmapを増やさない。native readbackの長さ／コピー件数／OLE取得または位置不一致は推測補正せずfail-closedでsyncを再試行する。
- アプリ側の画像drop／clipboard貼り付け／resizeもRichEditへ直接replaceせず、source selectionを記録して既存 `ApplySourceTextWithUndo` transactionへ統合する。画像referenceはHTML/Markdown混在でもsource順に整列し、matched multiline inline-code range内の画像構文をliteralに保つ。code spanのblock境界はMDLiteの現在のparser subsetに限定し、完全なCommonMark適合は主張しない。HTML tagの属性名と引用範囲をtokenizeし、resizeは元tagのwidthだけを更新する。
- presentation reset は underline/effects/paragraph spacing/border を明示的に解除し、表 grid は固定22px矩形でなく実測した行高・共有境界で描画する。mouse hit-test と source caret mapping も同じ共有geometryを使い、クリック時はnative caretへ変換する。
- UI layout は DIP scaling、`WM_DPICHANGED`、PerMonitorV2 manifest、editor font 再生成を実装した。Workspace／Outline paneの明示折畳み、狭幅時の自動hide、Find/Replaceのresponsive配置を追加し、`responsive-layout-native-final9.json` と `responsive-find-native-final9.json` でclient boundsを実測した。実機のDPI・IME・通常GUI画面はまだPASSにしない。
- 設定と作成profileは、共通のnative formで項目を一括編集し、適用／取消を選べるようにした。scope reset、継承値、keybinding／色の複数行、profile入力項目は既存のSettings/Profile validatorとatomic saveへ渡し、連続Prompt/MessageBoxを通常編集経路から除去した。Quick Open／command paletteも共通の検索可能なnative picker（filter Edit＋候補ListBox＋選択／取消）へ統一した。最新 `gui-acceptance-settings-final21.json` で実modal formの設定Apply/Cancelとprofile Apply/Cancelを確認し、設定Cancel・profile Cancel・profile Applyのsilent確認拒否でfixture SHA-256が不変であること、OS/RichEdit/DPI/window/client/font設定/source/settings hashを同じnative起動から記録した。workspace/outline tree、outline pane折畳み／復元、calendar表示／非表示、Quick Open／command paletteの候補filter・選択も同じfixtureで確認した。
- 祝日は同梱データと検証付きローカルCSV取込みを維持する。ネットワーク更新service、scheduled/manual HTTP経路、WinHTTP link、`holiday_auto_update` 設定surfaceは削除済み。Debug/Release CTestは現行parser/import codeを通過。過去のHTTP更新policy・mock HTTP試験は履歴であり、現行機能の受入根拠ではない。

## 現在の構成

- C++20、Win32、RichEdit、Common Controls v6、CMake／Ninja／MSVC。WebView、JavaScript／Node.js、汎用pluginを製品へ含めない。
- Markdown sourceを正本とし、書式・画像・表は派生表示とする。source変更はDocumentのtransaction/Undo履歴を経由し、RichEdit native Undoは表示更新後に破棄する。
- 通常ファイルを正本にし、共有設定は`.mdlite`、再生成可能データは`.cache`、復旧・session・置換journalは`.state`へ分離する。
- TrustはWorkspace内へ書かず、ユーザーlocal storeにcanonical pathとvolume/file identityを保存する。

## A-01〜A-34（実装一覧。Rev2の現行受入は下記crosswalkを参照）

| ID | 実装・接続 | 今回の証拠 | 判定 |
|---|---|---|---|
| A-01 | native treeの作成・移動・rename・ごみ箱削除、同名／自己配下拒否 | Core回帰とbuild。複数選択D&DのHuman操作は未実施 | 実装済み・Human未確認 |
| A-02 | 表示更新はsourceを書き換えず、アプリ管理source履歴だけをUndo/Redoとして公開。表示専用native Undoは通知抑止中に破棄。同一行で識別不能な複数画像はraw Markdownへfail-closedし、完全にcollapseしたsource rangeだけnative描画。アプリ側の画像drop／clipboard paste／resizeもsource transactionを使う | Core byte/Dirty・複数画像identity・mixed Markdown/HTML順序・双方向nested syntax ownership・block-aware multiline inline-code exclusion・隣接caret境界・numeric entity/HTML width-only/asset ownership回帰、Debug/Release GUI `gui-acceptance-*-rev2-multiline-code-spans-final-20260927.json` のneighbor missing-image slot・adjacent source・HTML attrs保持・caret・Undo/Redo | 自動PASS（drop/clipboard入力とvirtual-cell image paste Undoの専用GUI fixture、Human IME/視認性は未確認） |
| A-03 | IME状態を文書単位で保持し、合成中のsync／保存／表操作を停止 | compactを含むnotification経路を回帰。Microsoft IME／ATOK実変換は未実施 | 実装済み・Human gate |
| A-04 | 見出し・本文・native image object・表presentationを同一RichEditへ統合 | parser/source/native座標回帰。実画像と表の自然な見え方はHuman未確認 | 実装済み・Human gate |
| A-05 | Front Matter、CommonMark/GFM対象構文をsource範囲付き解析 | Core parser回帰 | 自動PASS（公式全corpusは対象外） |
| A-06 | outline、link、現在文書／Workspace検索結果からsource位置へ移動 | GUI acceptanceで検索結果選択経路、最新 `gui-acceptance-settings-final21.json` のworkspace/outline treeとpane折畳み／復元、Core link/outline回帰 | 自動PASS・link/outline clickとdragはHuman未確認 |
| A-07 | canonicalな直接走査。初版は候補indexを採用しない。include/exclude globと階層`.gitignore`を尊重 | UTF-8/CP932、regex、word、`*`/`?`/`**`、否定、directory、anchored、200-file回帰 | 設計差異を記録・自動PASS |
| A-08 | 未保存buffer、CP932、日本語、記号、大小、word、複数行、ECMAScript regex/capture置換 | Core検索／置換、GUI case検索 | 自動PASS |
| A-09 | preview、適用直前改訂照合、部分競合、取消、disk journal／rollback。open bufferはUndo可能 | apply/cancel/rollback回帰 | 自動PASS・長時間cancelのHuman未確認 |
| A-10 | 日付・profile・template・任意入力・path検証を共通処理で展開 | Core profile回帰 | 自動PASS |
| A-11 | 750ms autosave、Ctrl+S、保存世代を分離。保存後もdisk contentを再照合 | 保存中編集・置換前後の障害注入回帰 | 自動PASS |
| A-12 | 外部変更拒否、同一folder temp、flush、再fingerprint、write非共有guard、replace、保存後再照合 | before/guarded/after replace障害注入。rename/deleteを含む完全CASは主張しない | 自動PASS・保証範囲記録 |
| A-13 | 未編集Saveは書込みなし | exact-byte回帰 | 自動PASS |
| A-14 | CP932 best-fitを許さず変換不能文字を拒否 | emoji拒否回帰 | 自動PASS |
| A-15 | `.state/recovery`へ原文をatomic保存し、復元／破棄／保持 | Release GUIで5秒timer後に実process kill、disk不変、再起動promptから未保存本文復元 | 自動PASS |
| A-16 | session v2、tab／選択／scroll／window位置、compact view移送、Workspace mutex | session回帰。新設の停止editorにおけるtable projection／wrapped-line scroll anchorのsession restart復元は未確認。複数monitorもHuman未確認 | 実装済み・scroll/session acceptance部分未確認 |
| A-17 | Trust後のGit status/diff/stage/commit/branch/merge/fetch/ff-only pull/push、diff3競合補助 | Process／cancel／conflict parser回帰。`git-local-first-final11.json` でremoteなし隔離repoのlocal commitとindex-only境界を再実行。実repo GUI操作は未実施 | 自動PASS・Human未確認 |
| A-18 | 未信頼Workspaceで外部processを拒否。引数配列、Job、timeout、log上限 | Trust identity copy/self-declare、process/cancel回帰 | 自動PASS |
| A-19 | common／Workspace設定、由来、theme、font、keybinding、palette、未知field保持。設定はnative formで一括編集・適用・取消し、scope resetも同じUIから行う | 設定階層・atomic save・競合回帰、最新 `gui-acceptance-settings-final21.json` の実native form Apply/Cancel、Cancel時fixture不変、runtime設定/font hash | 自動PASS・HumanのIME/DPI/視認性は未確認 |
| A-20 | 設定はGit移行可能、Trustはuser-local identity storeで移行しない | copyされた設定／自己申告token拒否回帰 | 自動PASS |
| A-21 | 通常起動は無通信、休日は同梱。休日CSVはローカル取込みのみ。自動／明示HTTP更新経路と設定surfaceを削除 | 現行CoreTestsはparser/importと旧`holiday_auto_update` keyの破棄を確認。WinHTTP/API/UI routeを削除し、Debug/Release CTestは7/7通過。過去のmock/HTTP policy結果はRev1履歴 | parser/import・route除去の自動証拠PASS。ローカル取込みのHuman操作感は未確認 |
| A-22 | ReleaseのP0〜P5測定scriptとJSONを作成・実行 | 現行 `../build/verification/rev2-performance-release-full-ps51-compatible-final-20260927.json` はP0-P4 9 scenario failures 0、P5 100/100・failure 0。fixture 12,068 files / 356,531,968 bytes、P1 one/six peak WS 44,007,424 / 42,356,736 bytes、P2は両方10,000 results。`rev2-performance-release-full-after-tab-barrier.json` は履歴証拠として保持 | **current final automated full measurement PASS**。search settledは750ms count-stability heuristic |
| A-23 | 大容量でも機能を無効化せず、入力時全文parse/decodeを遅延し、不変画像object/cacheを再利用 | 現行 final JSONでP3 20MiB/100MiB両方のinput/save/search routeを完了。P3 20MiB peak WS 269,221,888 bytes、100MiB peak WS 1,186,410,496 bytes / private 1,252,904,960 bytes。必要機能は有効のまま | **current final P3 20/100MiB automated PASS**。100MiB memory高負荷を開示 |
| A-24 | `.cache`と`.state`を分離 | WorkspaceStore回帰 | 自動PASS |
| A-25 | vswhere、CMake/Ninja/MSVC検出、VS Code task/launch、静的runtime、v6 manifest、DIP/responsive layout | Debug/Release build/CTestは後述のRev2ログ。F5 demo prepは成功し、既存README hashと`.mdlite` file count/byte totalは不変。VS Code F5 debuggerの実操作は未実施 | build/CTest・prep PASS。F5 interaction NOT_RUN |
| A-26 | 公開投影なし。build、`.codex`、秘密拡張子をignore | tracked fileの秘密形式／個人絶対path scan | 自動PASS |
| A-27 | 同一RichEdit上のcell grid、表source transaction、Tab/Shift+Tab、矢印境界、行列操作、Undo接続 | grid cell解析、EOF/CRLF/escaped pipe/code pipe、可視行横断で共有する実測column boundary、描画・mouse hit-test・caret mappingの共通geometry、`table-hit-test-native-responsive-final9.json` 4/4、最新 `table-edit-native-final13.json` の実native行列操作・セル編集Undo/Redo・Tab/末尾行追加・矢印 10/11。Shift+Tabはこの実行枠でSendInput 0/4、Win32 error 5 (Access Denied)となりBLOCKED（CoreTestsの逆移動ロジックはPASS） | 自動証拠あり・Shift+Tab実native入力と実DPI/hit-testの操作感は未確認 |
| A-28 | 子見出しを含むsection move、子孫drop拒否、単一Undo | section回帰。実dragはHuman未確認 | 実装済み・Human gate |
| A-29 | Dairy/Meeting/Memo既定、採番、任意field、profile GUI、cursor。profileのadd/edit/duplicate/delete/templateと入力項目は一画面formで編集・適用・取消し | profile回帰、最新 `gui-acceptance-settings-final21.json` の実native profile form Apply/Cancel（CancelとApply後のsilent確認NOで保存せず、fixture不変） | 自動PASS・Humanの連続操作感は未確認 |
| A-30 | 日曜始まり、同梱日本休日、検証付きローカルCSV追加、既存Daily、tooltip。ネットワーク更新機能は含めない | Rev2 Debug/Release CTest 7/7、`../build/verification/rev2-calendar-daily-*.json` と `../build/verification/rev2-calendar-hover-*.json` はsynthetic native-message route。物理mouseとDPIごとの見え方は未確認 | parser/import自動試験PASS、native synthetic evidenceあり、Human gate |
| A-31 | PNG/JPEG/GIF/WebP/SVG経路、clipboard、D&D、比率固定resize、SVG XML安全判定、画像ごとのanimation期限 | 5形式のnative RichEdit挿入、GIF partial frame/offset/transparency/disposal 2/3、SVG同一bytes検査、Coreのquoted HTML attrs/numeric entity/重複属性/nested syntax ownership/width-only resize/asset ownership回帰、Debug/Release `gui-acceptance-*-rev2-image-syntax-ownership-final-20260927.json` の隣接missing-image/HTML attrs/source/caret/Undo/Redo保持、full P4 | parser/resize自動PASS。drop/clipboard入力、virtual-cell image paste Undo fixtureとHumanの見え方は未確認 |
| A-32 | 明示Trust、external command adapter、取消、失敗時local保持、hash/revision | mock CLI回帰。本番資格情報は使用していない | 自動PASS（mock） |
| A-33 | 公開package生成をrelease手順まで拒否 | 今回は署名・package・Releaseを実施しない | Release時保留 |
| A-34 | feature checkpointをForgejo/GitHubへ非forceで同一OID配送 | table shared-geometry/hit-testとnative caret mapping、responsive layout（`41303da2cc398f2f94efdfac595ac5f5ad5271ae`）を含むfeature checkpointを両remoteへ非force配送し、対象branch OID一致を`ls-remote`で照合。Human gate未実施のためdevelop統合は保留 | feature配送PASS・develop統合保留 |

## Human専用の残り

- Microsoft IME／ATOKの実変換、候補確定、削除、選択、Undoの操作感。
- 100/125/150% DPI、複数monitor、outline drag、table cell grid/hit-test、画像resizeの視認性。
- PNG/JPEG/GIF/WebP/SVGの実表示とanimated GIF/WebPの主観的な見え方。WebP decoderはlibwebp 1.6.0を静的リンクし、Windows側の任意codecには依存しない。
- Compact window配置、calendar tooltip、設定／profile formの連続操作感、実repositoryのGit credential/hook操作。修正前後と現行Releaseの実native PrintWindow画像（`native-screenshot-current-20260921.png`）は保存済みだが、Computer UseはMDLite window列挙後にWindowsロック画面で停止したため、実クリック・ドラッグ・視認性は未実施。

これらはAI側の実装不足を隠すための待機ではなく、実機入力方式・主観評価・外部環境を必要とする最小Human gateである。

## Closeout: Task `20260927-mdlite-finish-nonhuman-closure` (2026-09-28)

Authority: immutable Task Inbox Revision 1 snapshot, read once at task start. The work closes automatable portions of the Rev2 implementation; it does not change or reread the snapshot. `PASS` applies only to the named automated/build route. `PARTIAL`, `BLOCKED`, `NOT_RUN`, `UNKNOWN`, and Human/external gates remain open. No overall Task acceptance PASS is asserted.

Fresh shared gates: Debug build/CTest passed 7/7 in 6.84s and Release build/CTest passed 7/7 in 5.52s. `../build/verification/gui-acceptance-debug-rev2-final-20260928.json` and `../build/verification/gui-acceptance-release-rev2-final-20260928.json` both pass (Debug SHA-256 `03806808316bc5696b85fa9ecb631a969e167220d6437ea00e319d064cecadca`; Release SHA-256 `54dc744e3bd7e9e99f14762449531f475f4c11b8b54d46a9388594abaf14433d`). They include E01/E02 untitled recovery and Save As cancel/resume/retry with snapshot payload identity checks; E18 details-row open; E20 delayed Stage during compact editing/recovery plus concurrent Commit rejection; palette Escape selection/focus/buffer/disk preservation; editor suspension, session anchors, table rollback/retry, and history checks. `../build/verification/rev2-performance-release-full-20260928.json` passes P0–P5 with 0 scenario failures and P5 100/100. Dedicated NativeTable, PanelLayout, Calendar Daily/Hover and performance artifacts remain synthetic/native evidence, separate from physical input and Human acceptance.

### E01–E25

| ID | Current evidence | Revision 2 status |
|---|---|---|
| E01 | The current Debug/Release GUI artifacts verify two independent untitled buffers, both recovery snapshots, tab restoration and continued edits. | **PASS synthetic for covered multi-untitled recovery route** |
| E02 | The current Debug/Release GUI artifacts verify Save-to-Save-As, Cancel preservation, resumed editing, retry exact write and per-tab recovery cleanup. | **PASS synthetic for covered Save As cancel/resume/retry route** |
| E03 | Current GUI artifacts record recovery snapshot creation, simulated process crash, and recovery restore. | **PARTIAL**; remaining exit/prompt branches not fully established |
| E04 | `../build/verification/calendar-daily-acceptance-rev2-final-20260928.json` verifies configured Daily path/date creation, repeat activation, sentinel preservation and a single file. Meeting/Memo input prompts and their paths remain untested. | **PARTIAL; Daily route PASS** |
| E05 | Final Debug/Release GUI artifacts record synthetic WM_CHAR/Undo/Redo save round-trip, including an existing source edit across partial table-presentation failure and retry; the artifacts explicitly mark physical-key evidence UNKNOWN. | **PARTIAL**; physical keyboard behavior UNKNOWN |
| E06 | No physical Microsoft IME/ATOK composition, commit, cancel, or reconversion evidence. | **BLOCKED / Human evidence NOT_RUN** |
| E07 | `../build/verification/table-edit-native-debug-rev2-final-20260928.json`: automated product checks pass with no failed cases; `all_cases_pass=false` because Shift+Tab, continuous Unicode input, Ctrl+Z transaction, and arrow-repeat checks are BLOCKED. Paste/cut, general deletion, selection-direction and full history matrix remain uncovered. | **PARTIAL synthetic**; four input cases BLOCKED |
| E08 | Final Debug/Release GUI artifacts cover selected history, preserve application-managed Undo/Redo after a partial table-presentation failure, verify adjacent-image resize source/caret restoration across Undo/Redo with a confirmed collapsed missing-image slot, and preserve exact HTML image attributes through repeated resize. Virtual-cell image-paste Undo and the full cross-tab/current-task history matrix are not established. | **PARTIAL** |
| E09 | Rev2 table probes cover selected Tab/Shift+Tab and arrow cases synthetically. OS input delivery remains BLOCKED; caret/hit-test and full cell matrix are not proven. | **PARTIAL native / BLOCKED OS input** |
| E10 | No physical 3–5 second arrow-hold recording or timed caret/selection/scroll frame sequence. | **BLOCKED / NOT_PROVEN** |
| E11 | No current regression artifact establishes the specified physical boundary-arrow behavior. | **UNKNOWN / NOT_RUN** |
| E12 | Native table paint/resource probes exist; combined IME, resize, scroll, DPI, history, and continuous visual matrix is absent. OS input evidence remains BLOCKED. | **PARTIAL native / BLOCKED OS input** |
| E13 | Current GUI harness checks selected editor/theme-related routes; approved black-background visual regression comparison is not established. | **PARTIAL**; Human visual gate open |
| E14 | `../build/verification/panel-layout-acceptance-rev2-final-20260928.json` passes 10 synthetic native-message/HWND-geometry checks. Physical drag/input and DPI/Human review are UNKNOWN. | **PARTIAL synthetic** |
| E15 | GUI artifacts observe Explorer/Outline trees, expand/select and pane collapse/restore. Required physical hierarchy/section drag and Undo sequence is not established. | **PARTIAL synthetic** |
| E16 | Debug/Release CTest pass; final Calendar Daily and Hover artifacts cover synthetic native-message routes. Full calendar state/holiday visual acceptance is not established. | **PARTIAL** |
| E17 | Debug `../build/verification/calendar-daily-acceptance-rev2-final-20260928.json` and Release `../build/verification/calendar-daily-acceptance-release-rev2-final-20260928.json`: Daily count stays 0 on hover/month/focus movement, becomes 1 on activation and remains 1 on repeat. Physical mouse/keyboard delivery remains UNKNOWN. | **PARTIAL synthetic** |
| E18 | The same Debug/Release Calendar Daily artifacts verify filesystem creation-time provenance and Enter from a selected creation-date row opens that exact document. They also confirm path/timestamp refresh after rename and move without reselecting the date, using filesystem mutations followed by the test-only production tree-refresh route; internal rename/move UI delivery is not established. | **PARTIAL native; list-open PASS, internal rename/move route NOT_RUN** |
| E19 | `../build/verification/git-local-followup-20260927.json` and fresh Git model tests cover local index boundaries, NoGit/NoRepository/offline mixed state and selected-file stage/unstage; native Git panel rendering is not established. | **PARTIAL model/local probe** |
| E20 | Debug/Release GUI artifacts run Stage with a 7-second worker delay, keep the Compact editor enabled, deliver editor input in 20.4/20.0ms, and capture a recovery snapshot while Stage is active. A concurrent Commit command is rejected with HEAD unchanged; the requested file enters the index after Stage completes. Git model tests cover stale generation/workspace results and NoRepository operation status. | **PARTIAL; worker responsiveness/serialization/recovery PASS, same-instance workspace switch NOT_RUN** |
| E21 | GUI artifacts cover multiple injected save/replace/resize/upload failures, buffer preservation, retries, and exit/recovery boundaries. These are automated injected paths, not the complete external permission/disk-failure matrix. | **PARTIAL automated** |
| E22 | Fresh Debug/Release build and CTest pass 7/7, and current Debug/Release GUI harnesses start and close the app. F5 demo preparation passes. Actual IDE F5 interaction is **BLOCKED** because Orca is unavailable (`orca` command not found). | **Build/CTest PASS; IDE F5 NOT_VERIFIED** |
| E23 | GUI/runtime and PrintWindow captures exist at the recorded 96-DPI environment. Approved UI image comparison, physical DPI and Human visual acceptance remain open. | **PARTIAL native capture / Human gate open** |
| E24 | `../build/verification/calendar-hover-acceptance-rev2-final-20260928.json` passes a 30.06-second synthetic route: 435/435 messages delivered and hover-state checks matched, six PrintWindow frames, zero Daily files. Physical mouse UNKNOWN. | **PARTIAL synthetic / Human input UNKNOWN** |
| E25 | Current GUI artifacts cover picker filtering, Settings form selection, posted Escape cancellation, directional selection and editor focus restoration, and unchanged buffer/disk SHA-256. Panel-header/focus entry, physical keyboard, and custom binding/hint consistency remain unproven. | **PARTIAL synthetic** |

### R-01–R-20

| ID / scope | Current evidence | Revision 2 status |
|---|---|---|
| R-01 Workspace/file operations | GUI artifacts observe Explorer tree items and expand/select; complete create/move/rename/delete physical sequence is not established. | **PARTIAL** |
| R-02 Live editing | GUI synthetic edit/history/readback and table probes exist; Debug/Release GUI verifies adjacent-image resize source preservation, the collapsed missing-image slot, source-exact HTML attribute resize, and Undo/Redo selection restoration. Physical typing, drop/clipboard image input, virtual-cell image-paste Undo, IME, and the full interaction matrix remain open. | **PARTIAL** |
| R-03 CommonMark/GFM/Front Matter | Debug/Release CTest pass 7/7, including current Core, table, and calendar suites; approved full live-render/edit matrix is not established. | **Automated suite PASS; acceptance PARTIAL** |
| R-04 Navigation | GUI artifacts cover selected tabs, Quick Open, Outline, and palette routes; full physical focus/link route remains open. | **PARTIAL** |
| R-05 Search/replace | Current GUI artifacts cover selected search/replace and Undo/Redo cases; complete current-task route is not established. | **PARTIAL** |
| R-06 Templates/profiles/Daily/calendar | Calendar Daily path/date creation, repeat-no-overwrite, creation-list provenance and hover routes pass synthetically; Meeting/Memo prompts and Human visual/mouse matrix remain open. | **PARTIAL** |
| R-07 Autosave/explicit safe save | GUI injected failure/retry and recovery observations exist; full race/prompt acceptance is not established. | **PARTIAL** |
| R-08 UTF-8/BOM/Shift-JIS/EOL | Current CTest passes; this Rev2 evidence set does not establish the complete encoding/EOL acceptance matrix. | **PARTIAL / full case NOT_RUN** |
| R-09 Recovery/session | GUI artifacts record crash recovery restoration; full session/window/monitor matrix is not established. | **PARTIAL** |
| R-10 Git assistance | Model tests cover NoGit/NoRepository including Stage returning NoRepository, offline mixed staged/unstaged/untracked entries, selected stage/unstage and stale-result predicate. Debug/Release E20 native tests verify Stage responsiveness, compact editing/recovery and rejection of a concurrent Commit. Native Git panel rendering and same-instance workspace switching remain open. | **PARTIAL** |
| R-11 Settings/configuration | GUI artifacts check native settings form Apply/Cancel and preservation on cancel. Full persistence/conflict route is not established. | **PARTIAL** |
| R-12 Appearance/theme/font | Runtime capture exists; approved visual and DPI/Human comparison remains open. | **PARTIAL / Human gate** |
| R-13 Palette/keybindings | Palette filter, settings route, Escape cancellation, directional selection restoration, focus, buffer and disk preservation pass synthetically. Custom binding hint and all-focus routes remain unproven. | **PARTIAL** |
| R-14 Window/compact mode | Panel/window synthetic routes and quick performance P4 compact-window route exist; complete physical window matrix is not established. | **PARTIAL** |
| R-15 Trust/communication safety | Current GUI artifact records zero TCP endpoints in its harness run; complete trust/external-action Rev2 route is not established. | **PARTIAL** |
| R-16 Maintenance/updates/diagnostics | `../build/verification/rev2-gui-*-final.json` checks Diagnostics capture/refresh. General app update source remains undefined; diagnostics are manual, dependency path is manual pinned version/URL/SHA-256. | **PARTIAL** |
| R-17 Performance | `../build/verification/rev2-performance-release-full-20260928.json`: P0–P5 PASS, zero scenario failures. P0 ready first/warm 39.932/23.260ms; P1 one/six peak WS 48,320,512/44,556,288 bytes, input p95 22.570/7.606ms, settled 958.836/780.842ms, save 16.040/11.152ms; P2 returned 10,000 results each, first result 33.197/35.660ms and one-document cancellation cleared stale results; P4 opens 3/3 compact windows, input p95 2.070ms; P5 100/100 with 0 failures (private bytes 5,435,392 → 5,353,472). | **Current full automated measurement PASS**; search-settled values remain a 750ms stability heuristic, not private completion-message timing |
| R-18 Large documents | Same artifact: P3 20 MiB input p95 321.492ms, save 188.081ms, peak WS 272,371,712 bytes; P3 100 MiB input p95 1,685.272ms, save 737.479ms, peak WS 1,160,126,464 bytes / private 1,253,777,408 bytes. Both scenarios completed live editor/input/save/search paths. | **Current automated P3 20/100 MiB PASS**; 100 MiB memory use remains high and is reported |
| R-19 VS Code/F5 development | Debug/Release builds and CTest pass, and F5 demo preparation passed; actual IDE F5 input could not be exercised because Orca is unavailable. | **Build/CTest PASS; IDE F5 NOT_VERIFIED** |
| R-20 Local Git/public operation | Feature commit `75482ee4ce7b29582cdc3c752fb9150ca76a9820` is merged to `develop` by no-ff merge `0f4b54092ab39554ace4eff827845b312da67817`. Fresh `ls-remote` checks and the normal `Push-Remotes.ps1` push failed for both `origin` and `github` with `SEC_E_NO_CREDENTIALS`; no remote OIDs are confirmed and no rollback was attempted. | **Local integration PASS / dual-remote push-readback BLOCKED by credentials** |

The current full P0–P5 run uses GUID-owned fixtures, alternates UTF-8 and Windows-932 files, clears only the fixture `.mdlite/.state/session.toml` before paired scenarios, and verifies input/save round-trips. The 50,000,000-byte P1 target is met for one and six documents. P2 settled values use a 750ms count-stability heuristic; P4 image/animation quality is not measured. Performance input uses ASCII `WM_CHAR`, so physical input and IME are not measured. E06/E07/E09/E10/E12 physical input, Human IME/ATOK and approved visual/DPI review, IDE F5 interaction, remote Git push, and Drive delivery remain separate open gates. General app-update source is undefined; diagnostics and dependency updates remain manual. No overall Task acceptance PASS is claimed.

### Historical Task snapshot Revision 1 crosswalk (2026-09-26)

Authority at the time: a separate immutable Task snapshot from 2026-09-26. This historical table is distinct from the 2026-09-27 closeout Task Revision 1 above. Its ID descriptions and artifact dates are not evidence of the later closeout. No tests or UI runs from that historical snapshot are implied by the table. Cited GUI/model evidence was prior evidence; `NOT_RUN` meant the full stated case lacked current evidence at that time. Product routes refer to implementation paths, not proof that routes were exercised.

### E01–E25

| ID | Implementation and product route | Existing evidence and Revision 1 status |
|---|---|---|
| E01 | `src/app/Application.cpp`, `src/core/Document.cpp`; new untitled document → editor tabs → autosave/recovery. | Prior GUI harness recorded E01 observation (`verification-20260922-mdlite-modern-panels.md`); multi-untitled/1-minute exact scenario not established. **NOT_RUN (full case)**. |
| E02 | `src/app/Application.cpp`; explicit Save/Save As dialog and autosave timer. | Prior GUI harness E02 observation only; dialog re-entry/cancel-resume sequence not evidenced for Revision 1. **NOT_RUN (full case)**. |
| E03 | `src/app/Application.cpp`, `src/core/Document.cpp`; Save/close/recovery prompt and restart. | Prior GUI harness E03 observation; all specified failure/exit/restart branches not evidenced. **NOT_RUN (full case)**. |
| E04 | `src/profiles/Profiles.cpp`, `src/app/Application.cpp`; new-document/profile action → Daily/Memo/Meeting editor. | Profile form Apply/Cancel is recorded under prior A-29, but specified path/date/re-run/no-overwrite sequence is not. **NOT_RUN**. |
| E05 | `src/core/Document.cpp`, `src/editor/EditorAdapter.cpp`; editor typing, caret, Undo/Redo, save. | Prior core/history checks exist, but no current evidence for speed parity, mid-buffer caret restoration, and save-between sequence. **NOT_RUN**. |
| E06 | `src/editor/EditorAdapter.cpp`, `src/core/Document.cpp`; native editor IME composition and history. | Synthetic/core IME-path checks are not physical IME evidence. Microsoft IME/ATOK composition, partial commit, cancel, reconversion remain **NOT_RUN (Human)**. |
| E07 | `src/core/Document.cpp`, `src/editor/EditorAdapter.cpp`, `src/table/Table.cpp`; editor/table edit commands and Undo/Redo. | Earlier core and limited native table observations exist; complete clipboard, directional selection, deletion, template, section move and selection-direction matrix is **NOT_RUN**. |
| E08 | `src/core/Document.cpp`, `src/editor/EditorAdapter.cpp`, `src/app/Application.cpp`; per-document history across tabs and presentation updates. | Prior regression coverage is described in the 2026-09-22 verification record, but Revision 1 full matrix has not been run. **NOT_RUN**. |
| E09 | `src/table/Table.cpp`, `src/editor/EditorAdapter.cpp`; click/caret/table-cell navigation, Tab/Shift+Tab. | Prior native table record reports 10/11 delivered operations; Shift+Tab SendInput was denied (Win32 error 5), so that route is **BLOCKED**. Core reverse-navigation logic is synthetic evidence only; full native acceptance is **NOT_RUN**. |
| E10 | `src/editor/EditorAdapter.cpp`, `src/app/Application.cpp`; physical arrow/selection/scroll in editor and table. | No timed native video/frame series establishing caret continuity is recorded. Synthetic key-repeat cannot satisfy this case. **NOT_RUN**. |
| E11 | `src/editor/EditorAdapter.cpp`; boundary arrow behavior in editor/table. | Prior user observation that boundary warning sounds improved is noted in Task input; no Revision 1 regression evidence. **NOT_RUN (regression)**. |
| E12 | `src/table/Table.cpp`, `src/editor/EditorAdapter.cpp`; live table layout, paint, edit, resize/scroll. | Earlier table parser/paint/native checks are recorded, but not this Revision 1 repeated-input/IME/Undo/DPI matrix. **NOT_RUN**. |
| E13 | `src/editor/EditorAdapter.cpp`, `src/settings/Settings.cpp`; editor formatting/theme routes. | Prior A-05/theme-surface evidence exists; no current route-by-route black-background regression evidence. **NOT_RUN**. |
| E14 | `src/app/PanelLayout.cpp`, `src/app/Application.cpp`; panel headers/keyboard actions, splitter and restore/reset. | Prior panel model/build checks exist; drag, splitter, restart and narrow-width full matrix not evidenced. **NOT_RUN**. |
| E15 | `src/workspace/Workspace.cpp`, `src/app/Application.cpp`, `src/editor/EditorAdapter.cpp`; Explorer/Outline navigation and section move. | Prior tree/outline display observation exists; required mouse/keyboard hierarchy, chevron and section-drag/Undo sequence is **NOT_RUN**. |
| E16 | `src/calendar/CalendarView.cpp`, `src/calendar/CalendarDayIndex.cpp`, `src/calendar/JapaneseHolidays.cpp`; Calendar panel/month navigation. | Calendar view is present in the current source tree; no Revision 1 UI evidence for the required seven-column/state/holiday cases. **NOT_RUN**. |
| E17 | `src/calendar/CalendarView.cpp`, `src/calendar/CalendarDayIndex.cpp`, `src/profiles/Profiles.cpp`; activate selected date to open/create Daily. | No current activation, hover/focus no-create, or duplicate-creation evidence recorded. **NOT_RUN**. |
| E18 | `src/calendar/CalendarDayIndex.cpp`, `src/calendar/CalendarView.cpp`, `src/app/Application.cpp`; creation-date list → open document. | Prior index/model checks do not establish native list route, provenance display, or rename/move behavior. **NOT_RUN**. |
| E19 | `src/git/GitPanel.cpp`, `src/app/Application.cpp`; Git panel status/diff and explicit path-scoped stage/commit actions. | Prior isolated model commit/index-boundary evidence exists; no Revision 1 no-Git/no-remote/offline mixed-state native UX evidence. **NOT_RUN**. |
| E20 | `src/git/GitPanel.cpp`, `src/app/Application.cpp`, `src/workspace/Workspace.cpp`; editor input during Git/index work and workspace switch. | No current responsiveness, stale-result rejection, or status-semantics evidence recorded. **NOT_RUN**. |
| E21 | `src/core/Document.cpp`, `src/app/Application.cpp`; save conflict/error/recovery path. | Prior safe-save fault-injection evidence is documented under A-11/A-12; complete external-change, permission, conversion, disk-failure plus concurrent-edit scenario is not evidenced for Revision 1. **NOT_RUN (full case)**. |
| E22 | `CMakePresets.json`, `tools/`, `.vscode/`; F5 and Debug/Release build/run. | Prior Debug/Release build records exist; no Revision 1 fresh-exe provenance/header-change/F5/samples-protection result is recorded. **NOT_RUN**. |
| E23 | `src/app/Application.cpp`, `src/app/PanelLayout.cpp`, `src/settings/Settings.cpp`, `src/editor/EditorAdapter.cpp`; normal/compact layout, theme and approved UI. | Prior screenshot is dated 2026-09-21 and not a Revision 1 approved-image comparison. DPI/layout/theme visual review is **NOT_RUN (Human)**. |
| E24 | `src/calendar/CalendarView.cpp`, `src/app/Application.cpp`; calendar hover/tooltip/month/resize. | Prior calendar toggle trace is not continuous hover/flicker evidence. No required frame/paint observation recorded. **NOT_RUN**. |
| E25 | `src/app/Application.cpp`, `src/settings/Settings.cpp`; command palette from editor/panels, command dispatch, Esc and Settings rail action. | Prior palette filter/selection observation exists; all-focus dispatch, binding/hint match and caret restoration are not evidenced. **NOT_RUN (full case)**. |

### Original R-01–R-20

| ID / scope | Implementation path and product route | Existing evidence and Revision 1 status |
|---|---|---|
| R-01 Workspace/file operations | `src/workspace/Workspace.cpp`, `src/app/Application.cpp`; Explorer open/create/move/rename/delete. | Prior A-01/Core and GUI regression is recorded; Revision 1 route not rerun. **Implemented; current acceptance NOT_RUN**. |
| R-02 Live editing | `src/editor/EditorAdapter.cpp`, `src/core/Document.cpp`, `src/table/Table.cpp`; central live editor and table cells. | Prior source/Dirty/history/table evidence exists; Revision 1 key/IME/visual matrix incomplete. **Partial evidence; NOT_RUN**. |
| R-03 CommonMark/GFM/Front Matter | `src/markdown/Markdown.cpp`, `src/table/Table.cpp`; open/edit Markdown in Live editor. | Prior parser and table-model checks are recorded; approved live rendering/editing remains unproven. **Partial evidence; NOT_RUN**. |
| R-04 Navigation | `src/app/Application.cpp`, `src/workspace/Workspace.cpp`; tabs, Quick Open, Outline, links. | Prior tab/Quick Open/Outline regression is recorded; full current UI route not rerun. **Implemented; current acceptance NOT_RUN**. |
| R-05 Search/replace | `src/search/Search.cpp`, `src/search/Replace.cpp`, `src/app/Application.cpp`; Find/Replace and Workspace search. | Prior core and GUI search evidence is recorded; Revision 1 current UI route not rerun. **Implemented; current acceptance NOT_RUN**. |
| R-06 Templates/profiles/Daily/calendar | `src/profiles/Profiles.cpp`, `src/calendar/*`, `src/app/Application.cpp`; profile action and Calendar panel. | Profile/index/model code and prior profile form evidence exist; date activation/list/hover UI matrix remains **NOT_RUN**. |
| R-07 Autosave/explicit safe save | `src/core/Document.cpp`, `src/app/Application.cpp`; autosave, Save, Save As, recovery prompt. | Prior save/fault-injection evidence is recorded; specified Revision 1 races and prompts not rerun. **Partial evidence; NOT_RUN**. |
| R-08 UTF-8/BOM/Shift-JIS/EOL | `src/core/Document.cpp`, `src/editor/EditorAdapter.cpp`; open/save existing documents. | Prior byte/EOL and CP932 conversion checks are recorded; complete Revision 1 encoding matrix not run. **Partial evidence; NOT_RUN**. |
| R-09 Recovery/session | `src/core/Document.cpp`, `src/app/Application.cpp`; recovery prompt and session reopen. | Prior GUI crash-recovery/session evidence is recorded; not rerun in Revision 1. **Implemented; current acceptance NOT_RUN**. |
| R-10 Git assistance | `src/git/GitPanel.cpp`, `src/git/Conflict.cpp`, `src/process/ProcessRunner.cpp`, `src/app/Application.cpp`; explicit Git panel commands. | Prior isolated model/process tests exist; complete Revision 1 native/offline/async workflow not run. **Partial evidence; NOT_RUN**. |
| R-11 Settings/configuration | `src/settings/Settings.cpp`, `src/app/Application.cpp`; Settings form and apply/cancel. | Prior settings Apply/Cancel and persistence evidence is recorded; current route not rerun. **Implemented; current acceptance NOT_RUN**. |
| R-12 Appearance/theme/font | `src/settings/Settings.cpp`, `src/editor/EditorAdapter.cpp`, `src/app/Application.cpp`; Settings and editor/panel surfaces. | Prior token/panel evidence exists; approved UI and DPI visual comparison remains **NOT_RUN (Human)**. |
| R-13 Palette/keybindings | `src/app/Application.cpp`, `src/settings/Settings.cpp`; palette/search/dispatch and keybinding Settings. | Prior palette interaction exists; editable bindings and full focus/return path not proven. **Partial evidence; NOT_RUN**. |
| R-14 Window/compact mode | `src/app/Application.cpp`, `src/app/PanelLayout.cpp`; compact toggle and workspace-instance startup. | Prior compact/session evidence exists; Revision 1 complete window matrix not run. **Partial evidence; NOT_RUN**. |
| R-15 Trust/communication safety | `src/workspace/Trust.cpp`, `src/process/ProcessRunner.cpp`, `src/assets/StorageAdapter.cpp`; trust gate and explicit external actions. | Prior trust-boundary and offline-policy checks are recorded; Revision 1 route not rerun. **Implemented evidence exists; current acceptance NOT_RUN**. |
| R-16 Maintenance/updates/diagnostics | `src/app/DiagnosticsView.cpp`, `src/app/Application.cpp`; diagnostics panel/action. | Diagnostics view is present; no current diagnostic UI evidence. The general/manual updater source is **undefined** in the available implementation/snapshot evidence; dependency-update route also lacks evidence. **Partial / NOT_RUN**. |
| R-17 Performance | `src/app/`, `src/editor/`; normal workspace/editor runtime. | Prior A-22 measurements include one-to-six-document memory and input metrics; not measured under Revision 1 build/conditions. **Historical evidence only; NOT_RUN**. |
| R-18 Large documents | `src/core/Document.cpp`, `src/markdown/Markdown.cpp`, `src/editor/EditorAdapter.cpp`; open/edit large document. | Prior A-23 20/100 MiB measurements are recorded, including high 100 MiB memory; Revision 1 conditions not measured. **Historical evidence only; NOT_RUN**. |
| R-19 VS Code/F5 development | `CMakePresets.json`, `tools/`, `.vscode/`; F5 build/run/debug. | Prior build wrapper and Debug/Release records exist; current header-change/F5 parity not proven. **Historical evidence only; NOT_RUN**. |
| R-20 Local Git/public operation | repository Git configuration and tracked source; local Git → configured remotes. | Current task requires local integration before any delivery; no Revision 1 remote OID or distribution evidence is recorded here. **NOT_RUN**. |

Evidence references above point to the prior records in this file, [verification-20260922-mdlite-modern-panels.md](verification-20260922-mdlite-modern-panels.md), and [acceptance-hardening-report.md](acceptance-hardening-report.md). Keep those historical results separate from Revision 2 fresh acceptance; promote a row only when its complete stated path and required evidence are available.
