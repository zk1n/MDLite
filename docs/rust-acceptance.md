# Rust Editor受入記録

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
