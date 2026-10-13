# Rust試験版の受入記録

この記録は、機械検証、GUIで確認できた範囲、未完了の受入項目を分けて示す。
最後にbuild/test/fmt/Clippyを完了したproduct checkpointのsource aggregateは
`6a408c1b3a5156c45d4e160d20514f73680a0abaa19230472b2c7726af8e3847`。
Debug EXE SHA-256は `f82c2fd9bdfbb98d6497ff69e8ec72ef78b0f34a9ba03ad402c7c2dc481f74c3`、
Release EXEは `16ed729a46e374746c28252b78f29eabd32de12639e8f073b222be58c0db0600`。
両構成で33 library + 33 native = 66 tests、build、fmt、ClippyがPASS。source aggregateはbuild scriptと同じ入力集合から再計算し、両EXEは実ファイルから再hashして受領書と一致した。

現在のproduct GUI試行はこのsource aggregate／Release EXEに紐づく。R1のfocused画面確認、R5/R6の一部操作、R3の合成keydown反復、別試行の実MS IMEコア証明、R9の測定を確認したが、それぞれ未実施・BLOCKEDの項目が残る。各runのFAILを保持し、全体PASSとは扱わない。F5はapp起動前に停止したため、product GUI proofではない。前回のIME候補取消失敗runも保持しており、そのrunでは候補popupを確認できなかった。Microsoft IMEの段階確認では一度目のSpaceが変換、二度目がcandidate UI要求となるため、当該失敗だけでnative candidate cancellationの不具合を証明したとは扱わない。

native focus-loss P2とGUI helperのstage-aware cancellation／R9 geometry gateは、それぞれのscoped reviewでPASSした。最終の独立current-source correctness reviewも割り当てられたscopeでPASSし、新たなcode blockerはなく、過去のP2 findingsは解消済み。このreview結果は全体受入を意味しない。別の現source R7 toolbar supplementではCancel／No経路がPASSしたが、先行Ctrl+W runのBLOCKED reportは保持する。F5は未完了、R9 Working Set目標はFAILである。Git配送とHANDOFFの実施状況はこの文書で先取りせず、repository-local HANDOFFの検証済み記録を参照する。
現在のfocused試験ではclipboardを未操作またはSkipClipboardとした。過去事故でoriginal formatsは失われ回復不能のため、追加clipboard操作は禁止する。
Rustの正式採用とHuman受入は別の判断であり、機械検証や部分的GUI PASSだけからは決定しない。

| ID | 対象 | 実装 | 現在の検証 |
|---|---|---|---|
| R0 | main基点、C++保全、環境再利用 | 実験branch、独立C++ checkout | 開始時のmain／develop ref、差分、既存Rust probeとmise環境を確認済み。終了時ref、remote readback、配送は未完了 |
| R1 | 日本語、見出し、表、画像、Front MatterのLive編集 | native view、projection | 最新sourceのRelease focused runで日本語、表、画像sourceとnative editorを確認。R1 checkはPASS。これは全編集要求の完了を意味しない |
| R2 | 原文／表示分離、Unicode、改行、encoding、外部変更 | document、coordinates、storage | 最新sourceのDebug／Release各66 tests、fmt、Clippy PASS。native focus-loss P2のscoped recheck PASS。独立correctness reviewは割り当てscopeでPASS、新しいcode blockerなし。全体GUI受入は別 |
| R3 | repeat中のcaret、残像、selection、hit test | DirectWrite layout、Win32 caret | 最新Releaseで4枚の実desktop frameを取得し、240ms間隔のOS合成SendInput KEYDOWN後に3箇所以上のcaret／selection位置を確認。R3 PASS、source/history不変。物理typematicは未確認 |
| R4 | 英字、実IME、Undo/Redo、OS clipboard | transaction、IMM adapter | 別supplementで実MS IMEを選択したOS合成入力 `にほんご` → `日本語`、Enter commit 1049 bytes（base 1040 bytes）、1 transactionを確認。Undo/Redo/Undoはclean baseへ一致。候補popup associationはBLOCKED。過去の二度Escape timeout reportは保持し、settled preeditからの追加Escape 1回によるsource/history-pure cleanupだけを別途確認。これは通常の三段階取消契約を証明しない。physical IME／reconversionは未確認。clipboardは未操作 |
| R5 | 表操作、境界、矩形TSV、表内scroll | table、native view | table、行／列挿入、alignment、header-only boundary、表内scrollはPASS。矩形copy／TSV paste／overflow rejectはBLOCKED、矩形cutはNOT_RUN |
| R6 | 5形式画像、挿入、resize、安全なSVG | assets、native view | 最新sourceでPNG/JPEG/GIF/WebP/SVGとcorrupt PNG、GIF/WebP animation、前後の通常入力、resize、file import/failureを確認。clipboard pasteはBLOCKED |
| R7 | 切替、閉じる、通知の寿命 | native owner、文書ID／revision | 先行Ctrl+W runはowned confirmation dialogが出ず、2 tabsとdirty test documentが残りBLOCKEDのまま記録。別の前段toolbar visibility-wait BLOCKED reportも保持し、製品不具合の証明とは扱わない。別の現source toolbar supplementは、Cancelでdirtyなfake `x`／document 2を保持し、Noでその文書だけを破棄してclean base document 1を保持した後、normal closeまでPASS。先行reportは変更していない。switch／notification lifetime一式は未受入 |
| R8 | Debug/Release、test/fmt/clippy、F5 | Cargo、VS Code、build script | 最新sourceでDebug／Release各66 tests、fmt、Clippy、build PASS。最終F5試行はUI状態が不十分なためconfiguration確認／prelaunch／F5送信前に停止しNOT_RUN。過去のF5 PASSは別source／EXE。物理F5とHuman受入も未確認 |
| R9 | 同条件C++比較、資源、部品選定 | 設計ADR、測定対象 | 最新Releaseで1／6文書を測定し、geometry／DPI／foreground比較はPASS、50,000,000-byte Working Set目標はFAIL。値と条件は下記。renderer-only Software／DEFAULT比較のprivate-memory差は設計記録に残すが、focused appのFAILを置き換えない |
| R10 | 実験branch同期、HANDOFF | 既存Git／HANDOFF導線 | experiment refの配送、実OID、remote readback、HANDOFF生成／upload結果はこの表では先取りしない。実際の結果はrepository-local HANDOFFの検証済み記録を参照する |

### R7 toolbar supplement

現sourceの別試行では、Cancelを押したowned `#32770` dialog後も、dirtyなfake `x` のdocument 2、2 tabs、revision 1、undo 1／redo 0、selection 1が保持された。次のtoolbar closeで表示したowned dialogのNoを選ぶとtest document 2だけが破棄され、original 1040-byte clean document 1はrevision 0、undo 0、redo 0のまま保持された。owned appはnormal closeした。このtoolbar経路はOS合成操作でPASSだが、先行Ctrl+W runのBLOCKED結果はそのまま残す。物理操作、keyboard、IME、clipboard操作やHuman acceptanceは主張しない。

### R9 focused Release測定

RustとC++は同じ1／6文書fixture pathを使用。両者のnormal visible client areaは1084×761、96 DPI、exact foreground HWND／PID gateを満たした。window screen positionは異なる。C++の6 tab countは11回の一時的なWM_NULL応答失敗後に確認できた。RustはATOK、C++ IME observerはUNKNOWN。両アプリはnormal closeし、source／EXEは測定前後で安定。各CPU値は5秒のone-core idle sampleであり、一般的な性能保証ではない。

| 実行版 | 文書数 | Working Set bytes | Private bytes | Peak Working Set bytes | idle CPU %（one core、5秒） |
|---|---:|---:|---:|---:|---:|
| Rust | 1 | 58,150,912 | 28,811,264 | 58,404,864 | 1.561 |
| Rust | 6 | 57,442,304 | 27,807,744 | 57,520,128 | 1.249 |
| C++ | 1 | 54,636,544 | 16,367,616 | 71,036,928 | 63.116 |
| C++ | 6 | 57,618,432 | 20,566,016 | 79,196,160 | 70.582 |

Rustの両Working Set値が固定50,000,000-byte目標を超えるため、R9 budgetはFAIL。目標を上げず、Working Set trimも行わない。

## 採用勧告

既存の機械検証はRust試験を続ける材料になるが、実アプリでの最終受入には至っていない。
最新sourceのbuild/test/fmt/ClippyはPASSであり、割り当てられたscopeの独立current-source correctness reviewもPASSしたが、GUI受入は部分的である。R3と実MS IMEのcore edit transactionは確認した一方、IME candidate associationはBLOCKED、R7のCtrl+W／switch／notification経路とF5は未完了、R9 Working Set budgetはFAIL。
clipboard original formatsは回復不能で、追加操作は禁止。Rustの正式採用とC++統合はHuman判断として保留する。
完成製品の機能群は後続作業とし、対応表から削除しない。

DirectWrite／Direct2D、Markdown parser、image／SVG decoderは既存libraryやOS機能を使う。
source mapping、編集transaction、table editing、文書lifecycleはMDLiteが維持する中核コードとして残る。
詳しくは[Rust編集基盤の試験設計](rust-architecture.md)。

## 製品全体への対応

このTaskで編集基盤を試験し、次の製品要求を削除しない。

| 元仕様 | 今回の責務／後続 |
|---|---|
| R-01 Workspace | 今回は安全なファイル操作の入口。Explorer全操作は後続 |
| R-02 Live Editor | 今回の中核。現sourceの実動・未検証条件を上表に記録 |
| R-03 Markdown | source projection、GFM表、Front Matter。構文完全性は試験範囲を明記 |
| R-04 Navigation | 文書タブとsource mapping。Outline／Quick Open等は後続 |
| R-05 Search | 左パネル検索・dirty buffer優先・任意glob・source一致移動は後続 |
| R-06 Templates/calendar | 後続。Dairy表記、祝日任意更新の既定OFFを保持 |
| R-07 Save | 基本I/Oと外部変更拒否。autosaveは後続 |
| R-08 Encoding | UTF-8/BOM、CP932、混在改行を今回検証 |
| R-09 Recovery | session／復旧は後続。有効なように表示しない |
| R-10 Git | 製品Git UIは後続。Gitなし／pushなし編集を可能にする |
| R-11 Settings | 完全なGUI／Workspace上書きは後続 |
| R-12 Appearance | ダークshell、可変文字、細いグレーscrollの基盤。承認UI全体は後続 |
| R-13 Commands | 必要な編集操作。palette／変更可能keybindingは後続 |
| R-14 Windows | 標準Windows操作。compact／重複Workspace管理は後続 |
| R-15 Safety | source／画像の安全境界。完全なworkspace trustは後続 |
| R-16 Maintenance | 手動更新・診断体系は後続 |
| R-17 Performance | 今回のRelease編集基盤のみ実測。完成製品の保証としない |
| R-18 Large files | 自動機能削除を行わず、今回の測定・試験限界を明記 |
| R-19 Development | Cargo／F5の再現経路と実行確認範囲を今回記録 |
| R-20 Public Git | 実験refだけ二つのremoteへ配送。正式採用・develop統合はHuman判断 |

## C++と正式移行

C++の過去PASSをRustへ転用しない。比較は同じ6文書とassetsを一つのWorkspaceに置き、同じVMとRelease条件で行う。
今回のC++ 1／6文書比較はgeometry、DPI、foreground条件を揃えた試行として成立したが、Rustは固定Working Set目標を超えている。測定は短い限定条件で、C++ IME profileはUNKNOWN、RustはATOK、screen positionも異なるため一般性能や完全同条件の結論へ広げない。将来の正式移行は、developだけに存在する旧C++ファイルを
明示的な通常commitで整理してから検証する必要がある。main起点branchのmergeだけでは旧ファイルは自動削除されない。
今回この移行は実施しない。
