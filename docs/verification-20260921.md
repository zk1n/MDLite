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
| Debug core checks | PASS | `All 301 MDLite core checks passed.` |
| Release build/test | PASS | `tools/Invoke-Build.ps1 -Preset release -Test`、CTest 1/1 |
| Release quick performance | PASS | `build/verification/performance-release-holiday-policy-quick-final2.json`。P0 18.1MB WS、P1 six-document 29.5MB WS、P4 43.3MB WS、P5 20/20 failures 0、compact 3/3、theme 20/20、入力p95最大3.356ms。 |
| Fresh bounded quick performance | PASS | `build/verification/performance-release-settings-quick-final12.json`。P1 six-document 29,409,280 bytes、P4 42,938,368 bytes、compact 3/3、P5 100/100 failures 0、入力p95最大7.045ms。現行Release exe SHA-256は `132bc8628e50cc7cf2efb5be7bbc11cc3557de1bf7d9b83b476ee47977ce849c`。 |
| Release full performance | PASS / 制約記録 | `build/verification/performance-release-holiday-policy-full-final2.json`。P1 six-document 47,894,528 bytes、P4 42,967,040 bytes、P3 20/100MiB 247.2/1,242.2MB。P2 cancellation completed、P5 100/100 failures 0、compact 3/3、theme 100/100、入力p95最大6.4ms。今回のP1 sixは50MB目標内、100MiBは機能維持による高使用量 |
| native座標・CRLF/LF・画像・表 transaction | PASS / 境界記録 | CoreTestsのnative snapshot/transaction回帰。表の描画・mouse hit-test・caret mappingは同じ可視行横断の共有列境界を使用。`build/verification/table-hit-test-native-responsive-final9.json` で隔離fixtureの4セルへ実native clickを送り、全4件が同一セル近傍のnative caretへ収束。`build/verification/table-edit-native-final10.json` で行列操作、セル編集Undo/Redo、Tab、末尾行追加、矢印を実native経路で確認（10/11）。Shift+Tabはこの実行枠でSendInputのShift modifierをRichEditへ観測できずBLOCKED、CoreTestsの逆移動ロジックはPASS |
| responsive native layout | PASS (evidence only) | `build/verification/responsive-layout-native-final9.json` と `responsive-find-native-final9.json`。1266/800/520/420幅でeditor正幅・client内bounds、Workspace/Outlineの折畳み復元、Find/Replaceの狭幅再配置をnative child rectで確認。Human DPI/IME/主観受入とは分離 |
| 設定・profile native form | PASS (automated native UI path) | `build/verification/gui-acceptance-settings-final12.json`。実native modal `MDLite.NativeFormWindow` を設定Applyとprofile Cancel経路で確認。連続Promptを通常編集経路から除去し、保存は既存のSettings/Profile validatorへ通す |
| native screenshot | PASS (evidence only) | `build/verification/native-screenshot-responsive-final9.json` と `native-screenshot-responsive-final9.png`。隔離fixtureのRelease `MDLite.exe`（SHA-256 `495EBE4CEFFFE1C3E94F83065288D5D99CE291F088A7FBA62E70DCFD9FEBC7C9`）を1280x800でPrintWindow取得。Computer UseはMDLite window列挙後にWindowsロック画面で停止したため、操作・主観受入とは分離 |
| native screenshot before/after | EVIDENCE | 修正前 `native-screenshot-final.json` / `.png`（Release hash `833C975470D9AA5A0B0439E6CAADA830E8883FCB9A9BC92379964197C45F583F`）と修正後 `native-screenshot-responsive-final9.json` / `.png`（Release hash `495EBE4CEFFFE1C3E94F83065288D5D99CE291F088A7FBA62E70DCFD9FEBC7C9`）を同じ隔離fixture・1280x800で取得。before/after画像は実native描画の比較証拠であり、Human視認性PASSではない |
| 休日CSV import・重複拒否・既定OFF | PASS | CoreTests、local parser、fake clock 28日判定、mock HTTP 200/304/404/500/timeout/HTML/大幅減少 |
| 無音回帰 | PASS（既存測定） | 100/100、失敗0。今回の表示変更で再現性を確認 |
| GUI acceptance script | PASS | `build/verification/gui-acceptance-settings-final12.json`、pass=true。Release exe hash `132bc8628e50cc7cf2efb5be7bbc11cc3557de1bf7d9b83b476ee47977ce849c`。compact edit/Undo/Redo、Find/Replace、設定native form、profile native form、画像object隣接編集、recovery、TCP観測を実native UI経路で確認 |
| native screenshot / CUA操作 | PARTIAL / BLOCKED | PrintWindowによる現行Releaseの実native画面は取得済み（`native-screenshot-responsive-final9.json`）。Computer UseではMDLiteの実window列挙まではできたが、画面取得がWindowsロック画面となり前面化に失敗したため、クリック・ドラッグ・Humanの視認性は未確認 |
| IME/ATOK、DPI複数monitor、table操作感、画像視認性 | NOT RUN | Human gate。自動click-routing evidenceをHuman PASSへ昇格しない |
| HTTP月次休日自動更新 | IMPLEMENTED / mock PASS / real HTTP BLOCKED | commonで明示許可した場合だけ、固定の内閣府HTTPS URLを非同期確認。WinHTTPのTLS、timeout、2MiB上限、redirect拒否、304、件数大幅減少拒否、原子的cache置換を実装。opt-in実HTTPを実行したが、試験環境のWinHTTPが `12185 (ERROR_WINHTTP_CLIENT_CERT_NO_PRIVATE_KEY)` でTLS応答を受信できず、status=0となった。資格情報・TLS設定は変更していない |

## QA disposition

| ID | 自動/API証拠 | Human/未実施境界 |
|---|---|---|
| QA-01 | GUI acceptanceのsource保存・recovery、Debug/Release build、修正前後のnative PrintWindow screenshot | クリック操作・主観的な文字位置は未実施 |
| QA-02 | native snapshotのCRLF/LF、画像object境界、GUI selection/保存 | IME・結合文字の実入力は未実施 |
| QA-03 | native transaction、GUI Save/Undo/Redo、presentation reset | 構文切替の全視認確認は未実施 |
| QA-04 | table parser、可視行横断の共有column geometry、描画とmouse/caret hit-testの共通経路、`table-hit-test-native-responsive-final9.json` 4/4、`table-edit-native-final10.json` の行列操作・Undo/Redo・Tab・矢印 10/11 | Shift+Tab実native入力（SendInput modifier未観測）と実DPIのcell hit-test/操作感は未実施 |
| QA-05 | 5形式画像、隣接raw、full P4 compact 3/3 | 画像の主観的表示・animation視認性は未実施 |
| QA-06 | PerMonitorV2 manifest、DIP layout、responsive child bounds at 1266/800/520/420, theme 100/100 | 100/125/150/200%・複数monitorは未実施 |
| QA-07 | GUI acceptanceの検索/置換/compact/recovery、Core calendar/settings、設定Applyとprofile Cancelのnative form | calendarのHuman操作とIME/視認性は未実施 |
| QA-08 | `git-local-first-final11.json`（`Invoke-GitLocalAcceptance.ps1`）でremoteなし隔離repoのlocal commit、staged/unstaged/untracked分離、Application.cppのindex-only commit形を確認。GUI TCP観測0、feature ref push/read-back | 実remote認証は今回のfeature配送で確認済み。実repoのcredential/hook操作とdevelop統合はHuman gate後 |
| QA-09 | 同梱/local CSV、重複/HTML/encoding拒否、OFF既定、fake clock、mock HTTP 200/304/404/500/timeout/HTML/大幅減少、cache保全ポリシー | 実HTTPは `MDLITE_TEST_REAL_HOLIDAY_HTTP=1` で実行したが、WinHTTP 12185 によりBLOCKED。offline transport/timeoutの実WinHTTP matrixは未実施 |
| QA-10 | 301 checks、Release CTest、silent P5 100/100、設定/profile native form、現行Release native screenshot evidence | ATOK・Human通知/視認性は未実施 |

## Git検証境界

変更は `fix/visual-rebuild-localfirst` 上で行い、`develop`/`main`/release tagは変更しない。
コミットはindexのみを対象にし、作業ツリー全体をpathspecで巻き込まない。`tools/Invoke-GitLocalAcceptance.ps1`
をfresh実行し、`build/verification/git-local-first-final11.json` でremoteなし隔離repoの
`git commit -m ...` がstaged-versionだけをHEADへ取り込み、作業ツリーのunstaged-versionと
未追跡 `untracked.md` / `other.md` を保持すること、およびApplication.cppのcommit形がpathspecなしであることを確認した。
実装checkpoint
実装commit `ff42e30349138a4ce370bb8b9d791cab13cd01d3`、検証記録commit
`92759b5d8955f0851cdc234696765ef1f886cd60`、休日policy/診断commit
`de172b72f6c16f3f21da85340c953a56a3612fc5`、空本文診断修正commit
`2f7401e224718752e98bd30eb72a0dd2ed2d8ece`、native canonicalization checkpoint
`837f24c6de9d1dcd6fe48ebaa09ec4d31bc57df8`、`856a419e2890053a91c673e732e9113a4e39f103`、
`40d88b73d556f994c9705f415685c0e0470b8477`、`da86373fbd780252345ee8583cf64bd4c8742e57`、`7abd422641888e402463ee01581a1d875674c3c6`、`8ee82cc16b3a0fcf7b40b97effac939fe032d76a`、`41303da2cc398f2f94efdfac595ac5f5ad5271ae` を origin (Forgejo) と github (GitHub) の
`fix/visual-rebuild-localfirst` へ非force通常pushし、各push時の対象OIDが両remoteで同一であることを
`ls-remote` read-backで確認した。responsive実装commit `41303da2cc398f2f94efdfac595ac5f5ad5271ae` の配送を両remoteで確認し、この参照更新を含むdocs commitもfeature branchへ配送する。main/tag/developは変更していない。
であることを確認した。Human gate未実施のためdevelop統合は保留している。
今回の設定/profile native form実装とGUI証拠更新は checkpoint `7d9f5e874e7096f41e50c9618faa23e1c20c6d47` に記録した。

## 残りの受入

GUI/Human gate（IME、実DPI、画像・表の操作感、クリック/ドラッグ）は残り、native screenshot
のevidenceのみ取得済みである。公式サイトへの
実HTTP正常系確認は、試験環境のTLS client-certificate不足（WinHTTP 12185）により
未達である。これらは自動試験・過去のGUI JSON結果から推測しない。
