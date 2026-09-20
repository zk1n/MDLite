# Acceptance hardening report

Task: `20260920-mdlite-acceptance-hardening` Revision 1
Date: 2026-09-20

## R-01〜R-08

| 指摘 | 変更 | 回帰証拠 | 残余境界 |
|---|---|---|---|
| R-01 SaveAll | 終了時の復旧／破棄判断と、Git前の「全保存成功」を別APIにした。Workspace置換はopen bufferを保存強制せず、Undo可能なbuffer transactionとして扱う | Core置換回帰、GUI Save/Undo/Redo | 終了時の明示破棄は利用者選択として維持 |
| R-02 Compact/IME | compact側でEN_CHANGE/EN_SELCHANGE/EN_LINKを処理し、focus対象をactiveにする。IME状態をDocumentView単位化 | Debug/Release GUI経路、build/test | MS IME/ATOKの変換感触はHuman gate |
| R-03 source/display/Undo | source差分transactionとアプリ管理のUndo/Redo historyへ統合し、表示更新で生じるRichEdit native Undoは通知抑止中に破棄する。複数画像は通常は隣接textで同一性を対応付け、同一行で画像間が空白のみの曖昧な組はMarkdown原文表示へfail-closedし、snapshot上で完全に折り畳まれたsource rangeだけをnative描画する | 274 Core checks、Debug/Release GUIのDirty・Save・Undo/Redo・画像object前後編集・隣接画像raw保存 | Microsoft IME／ATOKの実変換単位はHuman gate |
| R-04 safe save | 通常preflight後も再fingerprintし、対象handleを外部write非共有で保持して置換する。置換後にsize/hashを再照合し、一致しないdiskをclean baselineへ昇格しない | before/guarded/after replace、競合writer、edit-revert Dirty回帰 | rename/deleteを許す共有条件のためpath差替えを含む完全CASは主張しない。検出できた競合と保存後改変は拒否し、編集本文をDirtyで保持 |
| R-05 search/replace | 現在文書／Workspaceをsource検索へ統一。逐次結果・個別未処理表示、世代cancel、include/exclude glob、階層`.gitignore`、未保存buffer preview、open buffer Undo、closed file journalを接続。ignore済みsubtreeは列挙しない | 日本語／CP932／複数行／capture／invalid regex、274-check suite内のglob/gitignore/issue、GUI case・2-result・pending buffer | `.gitignore`の文字class、global excludes、Git設定由来case sensitivityは初版subset外。対応する `*`/`?`/`**`、否定、directory、anchoredは回帰済み |
| R-06 table/images | 同一RichEdit上へcell単位gridを描画し、escaped pipe/inline codeを区切りから除外、EOF Shift+TabとCRLFを修正。libwebp 1.6.0を固定hashで静的リンクし、SVGはDirect2DでPNG化、animated WebPとGIF partial frame/offset/transparency/disposal 2/3を合成 | 5形式すべての実 `EM_INSERTIMAGE` 成功、grid/table境界、GIF disposal、GIF/WebP異周期timing、Release P4 | grid/hit-testと5形式の主観的な見え方はHuman確認対象 |
| R-07 SVG/Trust/no-network | SVGをXMLLiteで解析しDTD、processing instruction、event、active/external resource、CSS外部URLを拒否しinternal gradientを許可。同じ読込bytesを検査後にrasterizeする。Trustをuser-local path+file identity storeへ移した | whitespace href/event/CSS/DTD/xml-stylesheet/internal fragment、unsafe raster入口拒否、copy/self-declare Trust、Release GUIのTCP endpoint 0 | OS全体のpacket captureは未実施。明示外部URL/CLIは別境界 |
| R-08 hot path | 入力直後のsource syncとpresentationを遅延し、source range/markup/target/file identity cacheで不変画像object・animationを再利用。asset更新だけを1秒周期で再decodeし、画像・theme書式中の再入を遮断 | 下記Release測定、P4画像負荷、無音P5 100反復 | 100MiBは機能を保つがメモリ負荷が大きい |

## Release性能測定

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

## 検証成果物

- `build/verification/gui-acceptance-debug-final-silent-v4.json`
- `build/verification/gui-acceptance-release-final-silent-v4.json`
- `build/verification/performance-release-full-silent-v3.json`

`build/verification`は再生成可能なlocal evidenceでありGit追跡しない。最終commit、両remote OID、receipt、実行物hashはHANDOFF manifestを正本とする。
