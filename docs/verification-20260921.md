# 2026-09-21 検証記録

対象Taskは `20260920-mdlite-visual-rebuild-localfirst` Revision 1。Task Inboxの
`TASK.md` と `REVIEW.md` を正本とし、実装・自動検証・Human/GUI受入を分離する。

## Initial Decision Gate

- Root profile: `luna-max-gate`。Decision PacketはTask、Current State、レビュー指摘、
  制約、分解案、未知点を含む。
- Jev choice: `sol_xhigh`、confidence `0.81`。固定targetは
  `sol_expert_xhigh / gpt-5.6-sol / xhigh`。
- specialist結論: 高レベルの優先順位は妥当だが、UI/table再構築の前にsource/export/native
  座標境界、改行・画像object、RichEdit raw text、巨大文書の写像量を是正すること。
- 実行provenanceは要求targetと一致せず、dispatchはfail-closedで記録した。
  model/effortを推測していない。詳細は [ADR](adr-20260921-native-coordinate-ui.md) を参照。

## 自動検証

| 項目 | 結果 | 証拠 |
|---|---|---|
| Debug build (`MDLite`, `mdlite_core_tests`) | PASS | MSVC/CMake build完了 |
| Debug core checks | PASS | `All 297 MDLite core checks passed.` |
| Release build/test | PASS | `tools/Invoke-Build.ps1 -Preset release -Test`、CTest 1/1 |
| Release quick performance | PASS | `build/verification/performance-release-holiday-policy-quick-final.json`。P0 18.1MB WS、P1 six-document 29.5MB WS、P4 42.3MB WS、P5 20/20 failures 0、compact 3/3、theme 20/20、入力p95最大3.081ms。 |
| Release full performance | PASS / 制約記録 | `build/verification/performance-release-holiday-policy-full-final.json`。P1 six-document 50,372,608 bytes、P4 41,848,832 bytes、P3 20/100MiB 246.8/1,244.1MB。P2 cancellation completed、P5 100/100 failures 0、compact 3/3、theme 100/100、入力p95最大6.57ms。50MB目標はP1 sixで約0.37MB超過、100MiBは機能維持による高使用量 |
| native座標・CRLF/LF・画像・表 transaction | PASS | CoreTestsのnative snapshot/transaction回帰 |
| 休日CSV import・重複拒否・既定OFF | PASS | CoreTests、local parser、fake clock 28日判定、mock HTTP 200/304/404/500/timeout/HTML/大幅減少 |
| 無音回帰 | PASS（既存測定） | 100/100、失敗0。今回の表示変更で再現性を確認 |
| GUI acceptance script | PASS | `build/verification/gui-acceptance-holiday-policy-final.json`、pass=true。Release exe hash `82eb71783a07796d2c7788bab9ab135fbde86e1e018bfc6c6faa86eef743a15d`。compact edit/Undo/Redo、Find/Replace、画像object隣接編集、recovery、TCP観測を実native UI経路で確認 |
| native screenshot / CUA操作 | NOT RUN | CUAにはnative appが列挙されず、スクリーンショット・Humanの視認性は未確認 |
| IME/ATOK、DPI複数monitor、table hit-test、画像視認性 | NOT RUN | Human gate。自動PASSへ昇格しない |
| HTTP月次休日自動更新 | IMPLEMENTED / mock PASS / real HTTP BLOCKED | commonで明示許可した場合だけ、固定の内閣府HTTPS URLを非同期確認。WinHTTPのTLS、timeout、2MiB上限、redirect拒否、304、件数大幅減少拒否、原子的cache置換を実装。opt-in実HTTPを実行したが、試験環境のWinHTTPが `12185 (ERROR_WINHTTP_CLIENT_CERT_NO_PRIVATE_KEY)` でTLS応答を受信できず、status=0となった。資格情報・TLS設定は変更していない |

## QA disposition

| ID | 自動/API証拠 | Human/未実施境界 |
|---|---|---|
| QA-01 | GUI acceptanceのsource保存・recovery、Debug/Release build | 実画面screenshot・主観的な文字位置は未実施 |
| QA-02 | native snapshotのCRLF/LF、画像object境界、GUI selection/保存 | IME・結合文字の実入力は未実施 |
| QA-03 | native transaction、GUI Save/Undo/Redo、presentation reset | 構文切替の全視認確認は未実施 |
| QA-04 | table parser/grid geometryとGUI編集経路 | 実DPIのcell hit-test/操作感は未実施 |
| QA-05 | 5形式画像、隣接raw、full P4 compact 3/3 | 画像の主観的表示・animation視認性は未実施 |
| QA-06 | PerMonitorV2 manifest、DIP layout、theme 100/100 | 100/125/150/200%・複数monitorは未実施 |
| QA-07 | GUI acceptanceの検索/置換/compact/recovery、Core calendar/settings | 全設定・calendar連続promptは未実施 |
| QA-08 | local Git staged-only proof、GUI TCP観測0、feature ref push/read-back | 実remote認証は今回のfeature配送で確認済み。develop統合はHuman gate後 |
| QA-09 | 同梱/local CSV、重複/HTML/encoding拒否、OFF既定、fake clock、mock HTTP 200/304/404/500/timeout/HTML/大幅減少、cache保全ポリシー | 実HTTPは `MDLITE_TEST_REAL_HOLIDAY_HTTP=1` で実行したが、WinHTTP 12185 によりBLOCKED。offline transport/timeoutの実WinHTTP matrixは未実施 |
| QA-10 | 297 checks、Release CTest、silent P5 100/100 | ATOK・Human通知/視認性は未実施 |

## Git検証境界

変更は `fix/visual-rebuild-localfirst` 上で行い、`develop`/`main`/release tagは変更しない。
コミットはindexのみを対象にし、作業ツリー全体をpathspecで巻き込まない。隔離repoで
`c.txt`をstageした後に未stage変更を加え、`git commit -m ...` のHEADが staged-version、
作業ツリーがunstaged-version、未追跡 `b.txt` がHEAD外であることを確認した。実装checkpoint
実装commit `ff42e30349138a4ce370bb8b9d791cab13cd01d3` と検証記録commit
`92759b5d8955f0851cdc234696765ef1f886cd60` を origin (Forgejo) と github (GitHub) の
`fix/visual-rebuild-localfirst` へ非force通常pushし、最終OID `92759b5d8955f0851cdc234696765ef1f886cd60` の
`ls-remote` read-back が同一であることを確認した。
であることを確認した。Human gate未実施のためdevelop統合は保留している。

## 残りの受入

GUI/Human gate（IME、実DPI、画像・表の操作感、通常画面 screenshot）と、公式サイトへの
実HTTP正常系確認は、試験環境のTLS client-certificate不足（WinHTTP 12185）により
未達である。これらは自動試験・過去のGUI JSON結果から推測しない。
