# Rust Editor受入記録

## 入力・描画回復の現状（2026-10-15）

以下はV5候補に対する記録。現在は借用中のWM_PAINT消失を修正しており、V5のbuild/testはそのsourceに限る履歴となる。新しい候補でGUI受入と最終検証が終わるまでは、Editor復旧完了とは扱わない。
下段の従来の受入結果は当時のsource/buildに限る履歴であり、今回の最終候補へ引き継がない。

| 範囲 | 現在の証拠 | 状態・境界 |
|---|---|---|
| Debug/Release候補 | V5 builder source SHA256 `79b43f41cf0263a874003ed557b5704fae5f416475d237021ebe2a0d51d98ddf`。Debug EXE SHA256 `d52c724bd923ab41559c5254fa4e9b3a1d678d672f0819e5bfd10570c61da3be`、Release EXE SHA256 `46b33c093af25f6bb2bf49bedd8f79500f056622c2a62b1ae90fec6f61a1d52c` | V5 build PASS。source commitは未確定。V3/V4のbuild結果は履歴であり、今回の最終候補には使わない。 |
| 限定回帰 | 合成IME終了通知後の最初の通常入力、測定したfallback layout高、遅延child redraw要求の保持・消費、通常close gate、owner-drawの空/日本語caption経路 | 対応する限定native回帰はPASS。可視GUIのchild描画や一連の実入力を証明しない。Dairyの表示名・directory semanticsは既存契約として維持。元のcommand label assertionを対象とするtestはPASS。 |
| 最終機械検証 | V5 Debug: core 139 + native 154 = 293 tests PASS。Release: core 139 + native 154 = 293 tests PASS。全targetのDebug/Release strict ClippyとfmtもPASS。 | 最終V5機械検証PASS。 |
| 可視画面 | V5の空文書/no-workspace windowでsettled screenを観測。native child labelsとNoWorkspace textは見える一方、parent D2D領域は白く、合成文書sourceはempty・revision 0のまま。 | 描画原因を調査中。画面観測であり、Editor動作や全体GUI受入のPASSではない。現時点でproduction source変更なし。 |
| 連続GUI/実IME | V5 runはforegroundとfresh diagnosticsを取得し、case入力前のinitial screen framesも観測。その後、harnessのevidence assemblyがcase入力前にE01 HARNESS_FAILUREとなり、operationsは0。失敗runのharness SHA256 `3e94064310bbaf9ad6dbab9531f505af52bd9d2f40411a00db960b7b60fc5a33`。修正後harness SHA256 `ca8b478bbccdb3d51a9205052713648888374ab2105e20b20209307461e16698`はGUI未実行。 | GUI/IMEの動作assertionは実行されず、連続受入はNOT_RUN。initial/settled screen framesは診断用であり、全体受入の証拠にしない。 |
| 最終review/F5 | 最終候補への独立correctness review再確認と通常VS Code経由のDebug/F5 | NOT_RUN。以前のsource reviewとroute preflightはこの結果に代えない。 |
| 完了・配送 | Human/physical acceptance、Git統合、HANDOFF | NOT_RUN。 |

2026-10-14のUI・周辺機能再実装は[実装・検証表](ui-feature-parity.md)へ記録する。
以下の既存Editor結果は、その対象source/EXEに限る履歴であり、新UIのGUI合格へ流用しない。
追加分の最終sourceはDebug/Releaseともcore 139 + native 146 = 285 tests、fmt、Clippyと
独立correctness reviewがPASS。対象source/EXE/harness hashは上記対応表に記載する。
最終Releaseは所有window/実行体を確認したがforeground gateで入力前に停止し、GUI操作はBLOCKED。
以前のDebug smokeはsource変更によりidentityが失効しており、今回の合格には流用しない。
PrintWindowはDirect2D内容を取得できないため、承認UIとの比較証拠には使わない。

Rust開発の主線はdevelop。旧実装の正確なdevelop tipは
`cpp-archive/pre-rust-20261013`とGit履歴に保持する。
開発主線の選択は、全製品・全入力機器の受入合格を意味しない。

## Editorの確認対象

入力→原文→selection→layout→次の入力を一つの契約として確認する。
本文はcrop Rope一つ。表示更新は本文、encoding、dirty、履歴を変更しない。

| 項目 | 今回の契約 | 確認状態 |
|---|---|---|
| 入力・IME | 実preedit範囲、本文とのbaseline、通知順序、終了後の通常入力 | 実装・関連回帰・独立review PASS |
| 移動・選択 | Unicode、Ctrl/Shift、preferred x、空行・EOF・block境界 | 実装・関連回帰・独立review PASS |
| Live執筆 | 未完成Markdownから成立・修正、code内literal、表保護 | 実装・関連回帰・独立review PASS |
| Undo/Redo | 原文と向き付きselection、保存点、typing/IME編集単位 | 実装・関連回帰・独立review PASS |
| 回帰 | BOM/CP932/CRLF、保存失敗、画像・表・未保存文書 | 実装・関連回帰・独立review PASS |
| 終了 | composition終了、保存/破棄/Cancel、失敗時本文保持 | 実装・関連回帰・独立review PASS |
| 通常開発 | 最終develop Debug/Release、tests、fmt、Clippy、通常起動 | 実装・関連回帰・独立review PASS |
| review | 実装者のPonytailと別Agentのcorrectness review | 実装・関連回帰・独立review PASS |

以前の試験結果と失敗記録はGit履歴に保持する。過去のPASSは今回の
連続操作・IME・GUIの証明へ転用しない。各経路の最終状態を別々に記録する。

## 最終確認

統合後のdevelopから、既存mise Rust1.99/MSVCでDebug/Release各131 tests
（library58、native73）、fmt、Clippyを確認した。source aggregateは
`09e23ff5fa27225c000f183b2c5fb0a5f459d905565015807d87143795a050d9`。
Debug EXE SHA256は`a96d72f35570569c155f8bc2603d0e3abe076c35ecfbd1c53f57bad46a030297`、
Releaseは`b96a80d31afab6e0ad48c4a66a78df36ed13b275f03596a025a5a35f2305eba0`。

Root/実装者と別のAstra XHighによるread-only correctness reviewを実施。
起動順序、表前の位置、隠れたfenceの移動、toolbar終了、harnessのABI/期待値を
修正し、同reviewerの局所再確認がPASS。実装者のPonytail確認は別に実施した。
原文・保存・encoding・画像・表の既存回帰を維持し、未移植機能を合格へ付け替えない。

所有するvisible Release、96DPIの合成fixtureで通常のwindow message経路を確認。
通常ASCII仮想キーで表編集、Enter/EOF、code内literalとcode外の見出し、
反復矢印、Shift/Ctrl選択、逆向きselectionのUndo/Redo、Source/Live切替をPASS。
現在のATOK profileのまま、romaji、Space変換、Esc取消、Enter確定と1単位Undo/Redoを
確認し、app-local open状態を元のclosedへ戻した。profileは選択・変更していない。
Unicode packet入力はこのATOK環境でIME lifecycleを通る別経路だったため、
通常キーのtyping-group試験と区別した。per-message全通知列を採取したとは主張しない。

未保存closeのCancelは本文・selection・dirtyを保持し、その後の保存・通常終了をPASS。
別の所有する4tab試験では保存Yesと明示Noも確認し、合成本文の証拠を保持して通常終了した。
Save失敗・外部変更拒否はstorage/native関連回帰で確認し、全dialog分岐の物理受入とは区別する。

通常VS CodeのStart Debugging controlからprelaunch/CodeLLDB/native Debug起動、IDE Stop、
所有IDE windowのcloseをPASS。起動fixtureのbytesと最終Debug source/EXE対応を確認。
物理F5入力を送った証拠ではない。Releaseはdebuggerなしで確認した。

C++保管branchは`cpp-archive/pre-rust-20261013`、保管元developの正確なtip
`e852ee768a9fe3708e5e7eb7c33ab5987fb71734`を両remoteで保持する。
通常mergeは修正済みRustと旧develop双方を祖先として持つ。現行tracked treeから
旧専用98pathsを整理し、共有UI icons、editor settings、licenseと製品要件を維持した。
main/tag/公開Releaseは移行対象外。最終配送OIDとreadbackはlocal配送記録へ固定する。

## 検証の境界

OS clipboardの読み取り・列挙・書き込み・backup/restore/clearは行わない。
Ctrl+C/X/Vと画像貼付けのOS統合は検証対象外。純粋payload試験はOS統合の証拠ではない。
入力profileやdefault IME、global設定を変更せず、所有fixture/processだけを使う。
合成window通知、OS合成キー、実IME、物理typematic、Human受入を区別する。
50MBは望ましい目安であり、機能削除やworking-set trimを行う合否gateではない。
実MS IME、再変換、候補popup位置・文節操作の全組合せ、物理typematic/F5、別monitor/DPI、
Narrator、主観的UIと全製品Human受入はNOT_RUNまたはHuman/機器確認待ち。
今回のATOK基本変換とnative geometry回帰を、これらの受入へ拡張しない。

## 製品全体への対応

今回のEditor範囲外の要求を削除しない。承認UIと元仕様の要求を維持する。

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
| R-20 Public Git | Rust専用developを両remoteへ配送。main、tag、Releaseは別指示 |
