# Acceptance hardening report

Task: `20260920-mdlite-acceptance-hardening` Revision 1
Date: 2026-09-20

## R-01〜R-08

| 指摘 | 変更 | 回帰証拠 | 残余境界 |
|---|---|---|---|
| R-01 SaveAll | 終了時の復旧／破棄判断と、Git前の「全保存成功」を別APIにした。Workspace置換はopen bufferを保存強制せず、Undo可能なbuffer transactionとして扱う | Core置換回帰、GUI Save/Undo/Redo | 終了時の明示破棄は利用者選択として維持 |
| R-02 Compact/IME | compact側でEN_CHANGE/EN_SELCHANGE/EN_LINKを処理し、focus対象をactiveにする。IME状態をDocumentView単位化 | Debug/Release GUI経路、build/test | MS IME/ATOKの変換感触はHuman gate |
| R-03 source/display/Undo | source差分transactionとアプリ管理のUndo/Redo historyへ統合し、表示更新で生じるRichEdit native Undoは通知抑止中に破棄する。複数画像は通常は隣接textで同一性を対応付け、同一行で画像間が空白のみの曖昧な組はMarkdown原文表示へfail-closedし、snapshot上で完全に折り畳まれたsource rangeだけをnative描画する。アプリ所有の画像drop／clipboard貼り付け／resizeも既存source transaction経由にし、HTML sourceはattributeをtokenizeしてwidthだけを書き換える | Core source/history・mixed image order・adjacent boundary・numeric entity・duplicate/malformed tag・HTML alt overlap、Debug/Release GUIのDirty・Save・Undo/Redo・画像object前後編集・隣接画像raw保存、resize時のconfirmed collapsed slot/source・exact HTML attrs・caret・Undo/Redo回帰 | Microsoft IME／ATOKの実変換単位、drop／clipboard入力とvirtual-cell image-paste Undoの専用GUI fixtureは未確認 |
| R-04 safe save | 通常preflight後も再fingerprintし、対象handleを外部write非共有で保持して置換する。置換後にsize/hashを再照合し、一致しないdiskをclean baselineへ昇格しない | before/guarded/after replace、競合writer、edit-revert Dirty回帰 | rename/deleteを許す共有条件のためpath差替えを含む完全CASは主張しない。検出できた競合と保存後改変は拒否し、編集本文をDirtyで保持 |
| R-05 search/replace | 現在文書／Workspaceをsource検索へ統一。逐次結果・個別未処理表示、世代cancel、include/exclude glob、階層`.gitignore`、未保存buffer preview、open buffer Undo、closed file journalを接続。ignore済みsubtreeは列挙しない | 日本語／CP932／複数行／capture／invalid regex、274-check suite内のglob/gitignore/issue、GUI case・2-result・pending buffer | `.gitignore`の文字class、global excludes、Git設定由来case sensitivityは初版subset外。対応する `*`/`?`/`**`、否定、directory、anchoredは回帰済み |
| R-06 table/images | 同一RichEdit上へcell単位gridを描画し、escaped pipe/inline codeを区切りから除外、EOF Shift+TabとCRLFを修正。libwebp 1.6.0を固定hashで静的リンクし、SVGはDirect2DでPNG化、animated WebPとGIF partial frame/offset/transparency/disposal 2/3を合成 | 5形式すべての実 `EM_INSERTIMAGE` 成功、grid/table境界、GIF disposal、GIF/WebP異周期timing、Release P4 | grid/hit-testと5形式の主観的な見え方はHuman確認対象 |
| R-07 SVG/Trust/no-network | SVGをXMLLiteで解析しDTD、processing instruction、event、active/external resource、CSS外部URLを拒否しinternal gradientを許可。同じ読込bytesを検査後にrasterizeする。Trustをuser-local path+file identity storeへ移した | whitespace href/event/CSS/DTD/xml-stylesheet/internal fragment、unsafe raster入口拒否、copy/self-declare Trust、Release GUIのTCP endpoint 0 | OS全体のpacket captureは未実施。明示外部URL/CLIは別境界 |
| R-08 hot path | 入力直後のsource syncとpresentationを遅延し、source range/markup/target/file identity cacheで不変画像object・animationを再利用。asset更新だけを1秒周期で再decodeし、画像・theme書式中の再入を遮断 | 下記Release測定、P4画像負荷、無音P5 100反復 | 100MiBは機能を保つがメモリ負荷が大きい |

## Historical Release性能測定 (prior artifact; not Revision 2 evidence)

測定JSON: `build/verification/performance-release-full-silent-v3.json`
実行環境: Windows NT build 26200、AMD64 Family 26 Model 68、16 logical processors（processから可視）、RAM 66,094,223,360 bytes、PowerShell 7.6.6。CIMが拒否されたためOS/CPU/RAMは.NET／registry fallbackで採取した。
fixture: 12,068 files、356,536,896 bytes。exe SHA-256: `b03027d19e8dad1af68075fe350377225174f2407b2098dd9e0f974ed00a65d7`。

| Profile | 結果 |
|---|---|
| P0 | first-run approximation ready 38.114ms、warm 24.459ms、peak WS 17,825,792 bytes |
| P1 1文書 | load 1,739.091ms、input p95 0.578ms、search first 81.652ms / settled 3,377.996ms、peak WS 36,904,960 bytes |
| P1 6文書 | load 2,079.683ms、input p95 0.701ms、peak WS 44,343,296 bytes（50,000,000-byte目標内） |
| P2 | 10,000 files/200MiB。1文書 load 20,778.434ms、search first 33.032ms / settled 13,735.162ms、GUI cancel成功。6文書 load 20,625.654ms、settled 13,050.818ms |
| P3 20MiB | load 1,706.827ms、input p95 1.237ms、peak WS 232,783,872 bytes |
| P3 100MiB | load 11,514.485ms、input p95 0.371ms（max 3.425ms）、peak WS 1,135,865,856 bytes |
| P4 | 3文書・7 assets・3 compact、load 572.814ms、input p95 1.805ms、settled 774.174ms / CPU 31ms、peak WS 41,828,352 bytes |
| P5 | 100/100 iterations、失敗0。handleは全回238、GDI 64〜69。Private Bytesは初回3,219,456、最終3,641,344、最大3,715,072 bytes。各回input p95の最大3.848ms |

測定上の限定: `search_settled_ms` は750ms件数不変の外部観測で、private completion messageを直接読んだ値ではない。P4の見た目・animation品質は自動判定していない。P5は毎回fresh processである。`MDLITE_TEST_SILENT=1` の子processではテーマ変更の試験専用window messageを受理し、復旧はテスト専用経路で自動承認し、その他のMessageBoxも表示・通知音とも抑止する。通常起動時の復旧確認・設定保存通知は変更していない。

## Revision 2 Release性能測定 (historical/superseded run, 2026-09-27)

測定JSON: `build/verification/rev2-performance-release-full-final-20260927.json`

この節の数値は履歴証拠として保持する。下記のWindows PowerShell 5.1互換のcurrent final artifactが、現行Rev2 crosswalkの数値上の正本である。

結果: `measurement_status=PASS`、P0–P4の9シナリオ失敗0、P5 100/100・失敗0。exe SHA-256: `a53797ad1ebe1a165c549ceca98858cb1f05980845e858e457bdb3368176a85b`。Debug/Release CTestはいずれも7/7。実行環境はWindows build 26200、AMD64 Family 26 Model 68、16 logical processors、RAM 66,094,223,360 bytes、PowerShell 7.6.6。.NET/registry fallbackから取得した。

fixtureは12,068 files / 356,537,021 bytes。P1は2,000-file workspace、P2は10,000 files / 200 MiB、P3は20/100 MiB、P4は3 documents / 7 assets / 3 compact windows、P5は100 fresh-process iterationsである。P1/P2の1文書入力・保存往復はUTF-8、6文書はWindows-932/CP932。P3/P4はUTF-8である。

| Scenario | 結果 |
|---|---|
| P0 | first-run approximation 47.373 ms、warm 12.395 ms、peak WS 21,946,368 / 21,938,176 bytes |
| P1 1文書 | load 4,261.979 ms、input p95 20.584 ms、settled input 996.623 ms / CPU 938 ms、save 17.449 ms、peak WS 45,076,480 bytes、1 tab / 1 RichEdit |
| P1 6文書 | load 4,301.242 ms、input p95 7.007 ms、settled input 757.644 ms / CPU 266 ms、save 12.829 ms、peak WS 44,961,792 bytes、6 tabs / 1 RichEdit |
| P2 1文書検索 | 10,000-file workspace、load 20,147.282 ms、first result 30.470 ms、settled heuristic 58,290.075 ms、10,000 results。Progressive cancel completed in 139.670 ms and stale results were cleared. |
| P2 6文書検索 | load 20,122.175 ms、first result 53.157 ms、settled heuristic 44,991.569 ms、10,000 results |
| P3 20 MiB | load 2,035.043 ms、input p95 321.001 ms (max 490.303 ms)、save 155.218 ms、peak WS 274,399,232 bytes |
| P3 100 MiB | load 11,283.734 ms、input p95 1,574.002 ms (max 2,361.451 ms)、save 719.334 ms、peak WS 1,160,257,536 bytes |
| P4 | 3/3 compact windows、input p95 2.312 ms、settled input 771.415 ms / CPU 94 ms、peak WS 54,001,664 bytes |
| P5 | 100/100、失敗0、theme change完了100/100。Private Bytes 5,795,840 → 5,873,664 bytes |

P1-one and P1-six each had exactly one resident RichEdit in all resource samples. The six-document case had six tabs and stayed below the 50,000,000-byte working-set target. P3 100 MiB remains functional with editor, input, save, and search paths enabled; its high memory use is disclosed. P4's zero main-pane RichEdit count reflects all three editors being in compact windows.

測定上の限定: P0 first-run is an approximation; Windows filesystem cache was not flushed. `search_settled_ms` is a 750 ms stable-result-count heuristic, not private completion-message timing. P4 visual/animation quality is not evaluated. P3 resources after process exit cannot be sampled. This harness uses ASCII `WM_CHAR`; physical input, IME/ATOK, DPI/Human visual acceptance and VS Code F5 were not measured. This report does not claim overall Task acceptance PASS.

## Revision 2 Release性能測定 (current final, 2026-09-27)

測定JSON: `build/verification/rev2-performance-release-full-ps51-compatible-final-20260927.json`。これはWindows PowerShell 5.1互換修正後の数値上の正本であり、前節のRev2 JSONは削除せず歴史的/superseded evidenceとして残す。

結果: `measurement_status=PASS`、P0–P4の9 scenario failures 0、P5 100/100・failure 0。Release exe SHA-256: `b54e3f64327d3829282041b5b5457e105763e2d89710b75b807bfd75e178a301`。fresh Debug build/CTestは7/7（8.03s）、Release build/CTestは7/7（6.39s）。current GUI artifactはDebug `gui-acceptance-debug-rev2-final-review-20260927.json` PASS（SHA-256 `0f92d303c91f5f5e9fc251223117c308ad6ef11ec7e8b5414cf64dd5a8e7f642`）、Release `gui-acceptance-release-rev2-final-review-20260927.json` PASS（同Release SHA）である。

fixtureは12,068 files / 356,531,968 bytes。P1は2,000-file workspace、P2は10,000 files / 200 MiB、P3は20/100 MiB、P4は3 documents / 7 assets / 3 compact windows、P5は100 fresh-process iterationsである。

| Scenario | 結果 |
|---|---|
| P0 | first-run approximation ready 40.950ms、warm 13.061ms |
| P1 1文書 | input p95 23.234ms、settled input 1,062.556ms、save 18.626ms、peak WS 44,007,424 bytes |
| P1 6文書 | input p95 9.072ms、settled input 763.358ms、save 15.616ms、peak WS 42,356,736 bytes |
| P2 1文書検索 | 10,000 results、first result 40.345ms、settled heuristic 75,158.398ms、progressive cancel 156.950ms、stale results clear |
| P2 6文書検索 | 10,000 results、first result 67.648ms、settled heuristic 66,953.026ms |
| P3 20 MiB | input p95 398.976ms、save 201.347ms、peak WS 269,221,888 bytes |
| P3 100 MiB | input p95 1,899.731ms (max 3,071.335ms)、save 795.462ms、peak WS 1,186,410,496 bytes、private 1,252,904,960 bytes |
| P4 | 3/3 compact windows、input p95 3.377ms、settled input 763.761ms / CPU 125ms、peak WS 51,027,968 bytes |
| P5 | 100/100、failure 0、theme change 100/100、private bytes 5,337,088 → 5,500,928 |

測定上の限定: P0 first-run is an approximation; Windows filesystem cache was not flushed. `search_settled_ms` is a 750ms stable-result-count heuristic, not private completion-message timing. P4 visual/animation quality is not evaluated. The harness uses ASCII `WM_CHAR`; physical input, IME/ATOK, DPI/Human visual acceptance, and VS Code F5 remain unmeasured. This current automated evidence does not claim overall Task acceptance PASS, Git/remote delivery, or external delivery.

## 検証成果物

- `build/verification/gui-acceptance-debug-final-silent-v4.json`
- `build/verification/gui-acceptance-release-final-silent-v4.json`
- `build/verification/gui-acceptance-debug-rev2-transaction-history-final-20260927.json`
- `build/verification/gui-acceptance-release-rev2-transaction-history-final-20260927.json`
- `build/verification/gui-acceptance-debug-rev2-app-edit-transaction-final-20260927.json`
- `build/verification/gui-acceptance-release-rev2-app-edit-transaction-final-20260927.json`
- `build/verification/gui-acceptance-debug-rev2-image-boundaries-final-20260927.json`
- `build/verification/gui-acceptance-release-rev2-image-boundaries-final-20260927.json`
- `build/verification/gui-acceptance-debug-rev2-html-tag-overlap-final-20260927.json`
- `build/verification/gui-acceptance-release-rev2-html-tag-overlap-final-20260927.json`
- `build/verification/gui-acceptance-debug-rev2-image-syntax-ownership-final-20260927.json`
- `build/verification/gui-acceptance-release-rev2-image-syntax-ownership-final-20260927.json`
- `build/verification/table-edit-native-debug-rev2-final-20260927.json`
- `build/verification/panel-layout-acceptance-rev2-final-20260927.json`
- `build/verification/calendar-daily-acceptance-rev2-final-20260927.json`
- `build/verification/calendar-hover-acceptance-rev2-final-20260927.json`
- `build/verification/performance-release-full-silent-v3.json`
- `build/verification/rev2-performance-release-full-after-tab-barrier.json`
- `build/verification/rev2-performance-release-full-final-20260927.json`
- `build/verification/gui-acceptance-debug-rev2-final-review-20260927.json` (current final, PASS)
- `build/verification/gui-acceptance-release-rev2-final-review-20260927.json` (current final, PASS)
- `build/verification/rev2-performance-release-full-ps51-compatible-final-20260927.json` (current final, PASS)

`build/verification`は再生成可能なlocal evidenceでありGit追跡しない。最終commit、両remote OID、receipt、実行物hashはHANDOFF manifestを正本とする。
