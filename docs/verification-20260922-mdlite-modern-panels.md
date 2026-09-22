# MDLite 編集基盤・パネル再是正 検証記録

Task Inbox: `20260922-mdlite-editor-reliability-modern-panels`（取得済み immutable snapshot）。
この文書は実装・自動検証・Human Gateを分離して記録する。添付された Visual C++ ダイアログ画像は指示書ではなく、`mdlite_panel_layout_tests.exe` の abort 症状を示す証拠として扱った。

## 根因と修正

- Windows のパネルテストが `panel-layout.toml` を `ifstream` で開いたまま `std::filesystem::remove` していた。Windows の共有モードで削除が失敗し、未捕捉例外が `abort()` へ到達していた。
- テストで入力streamを明示的にcloseしてから削除するように修正した。Workspace状態のatomic writeも `FlushFileBuffers` と `CloseHandle` の後に `MoveFileEx` する。
- 無題文書のbackground autosaveを通常Save Asへ落とさず、復旧保存と明示保存を分離した。保存ダイアログ再入を抑止し、native edit commit後にpresentationを再スケジュールする。
- 表はGFM delimiter、escaped/code-span pipe、ragged/invalid/fenced、LF/CRLFを共通モデルで解析し、行列/cell/Tab/矢印操作とsource transactionを保持する。A5では通常文字の背景をRichEdit editor surface tokenへ統一した。

## 変更範囲

- `src/app/Application.*`: 保存・入力履歴・presentation scheduler、panel header、slot移動、折り畳み、非表示、リセット、幅/高さ変更メニューとCtrl+Alt+1..4。
- `src/app/PanelLayout.*`, `src/workspace/Workspace.*`: 4-slot layoutの検証、swap、寸法clamp、workspace-local `.mdlite/.state/panel-layout.toml` のatomic persistence。
- `src/table/*`: strict GFM modelと共通visual row/cell range。
- `src/calendar/CalendarDayIndex.*`: 許可されたtext/config拡張子の一覧、除外ディレクトリ、NUL binary除外、filesystem creation-time provenance、selected-date filter、Daily profile path。
- `src/git/GitPanel.*`: repository/branch/detached、staged/unstaged/untracked/conflict、NoGit/NoRepository/Ready/NoRemote/OperationInProgress/Error、明示path-scoped stage/unstage/commit。
- `tools/*Acceptance.ps1`: GUI evidence boundary、SendInput配送未証明のBLOCKED分類、calendar toggle trace。

## Fresh verification

| Gate | 結果 | 証拠 |
|---|---|---|
| Debug build + CTest | PASS、5/5 | `debug-build-resize.log` |
| Release build + CTest | PASS、5/5 | `release-build-resize.log` |
| Debug GUI native harness | PASS | `gui-acceptance-final2.json`、E01/E02/E03/E06/E12/E13の自動観測 |
| Release GUI native harness | PASS | `gui-acceptance-final-release.json` |
| Debug table native harness | table操作/paint PASS、配送依存はBLOCKED | `native-table-final2.json` |
| Release table native harness | table操作/paint PASS、配送依存はBLOCKED | `native-table-final-release.json` |
| abort regression | PASS、20/20 | `panel-test-1..20.log` |
| whitespace | PASS（既存CRLF警告のみ） | `git diff --check` |

`BLOCKED` は製品FAILではない。Shift+Tab、Unicode連続入力、Ctrl+Z、arrow repeatは、AttachThreadInput後もSendInputがRichEditへ配送されたことを証明できず、`BLOCKED_INPUT_DELIVERY_UNPROVEN` とした。IME/ATOK、物理長押し、DPI/visualはこの自動証拠に含めない。

## R-01〜R-20 対応境界

| 要求 | 現時点 |
|---|---|
| R-01 Workspace/file操作 | 実装済み・既存GUI回帰PASS |
| R-02 Live editor | 実装済み・source/Dirty/Undo回帰PASS |
| R-03 Markdown/GFM | 実装済み・表モデル追加、Human visualは未完 |
| R-04 navigation | 実装済み・tab/Quick Open/outline回帰PASS |
| R-05 search/replace | 実装済み・GUI workspace search回帰PASS |
| R-06 profile/template/calendar | 部分実装・calendar index/details model追加、UI一覧/日付activationは要統合 |
| R-07 save | A1是正済み・外部変更/権限/実IMEはHuman/追加回帰待ち |
| R-08 encoding/EOL | 既存実装を保持、CP932変換不能のHuman境界は未完 |
| R-09 recovery/session | GUI recovery/session回帰PASS |
| R-10 Git | modelと明示scope API追加、Application panel action UIは要統合 |
| R-11 settings | 既存native form回帰PASS |
| R-12 theme/appearance | A5 token整合とpanel surface、DPI/visual Human Gateは未完 |
| R-13 keybindings/command palette | 既存palette回帰PASS、全keybinding編集は要確認 |
| R-14 window/compact | GUI compact/recovery回帰PASS |
| R-15 safety/communication | Trust境界とoffline-firstを保持、remote操作は明示のみ |
| R-16 maintenance/diagnostics | 既存導線を保持、診断測定は未完 |
| R-17 performance | GDI/User反復は測定、50MB/CPUの条件付き実測は未完 |
| R-18 large files | 未完、無制限増加の追加測定が必要 |
| R-19 F5/development | build wrapperとDebug/Release検証PASS |
| R-20 public operation | local Git policyを保持、remote OID/HANDOFFは最終配送Gate |

## E-01〜E-23 境界

GUI harnessでE01/E02/E03/E06/E07(限定)/E08/E12/E13の自動観測を収集した。table harnessでE04/E05/E06/E07/E08のOS-native部分を収集した。E09 IME、E10 physical long-press、E11/E23 DPI・visual、E14 panel drag/splitter、E16/E17/E18 calendar Human activation、E19/E20 Git async/offline UX、E21 external-change race、E22 stale build parityの一部は未実施またはUNKNOWNである。

特に、panel header/menu/keyboard移動と寸法変更は実装したが、native画像による全スロット移動・DPI・狭幅・drag/splitterのHuman Gateは未完である。Taskの全R-01〜R-20/A群を完成扱いにはしない。
