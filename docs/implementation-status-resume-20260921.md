# MDLite 表示・UI再構築 継続ステータス

Task `20260921-mdlite-visual-rebuild-resume` Revision 1の現HEAD checkpoint。

## 完了したmachine-actionable lane

- 現行ReleaseからDebug/Release build + CTest + core 301 checksをfresh実行済み。
- GUI acceptanceをfresh実行し、compact編集、検索/置換、workspace/outline/calendar、Quick Open/command palette、settings/profile form、画像object隣接編集、recovery、TCP観測が`pass=true`。
- Release quick performanceをfresh実行し、通常1〜6文書の作業量とP5 100/100を確認。P4画像/compactも完了。
- Native table probeをfresh実行し、行列操作・セル編集Undo/Redo・Tab・末尾行追加・矢印を確認。Shift+Tabは`SendInput 0/4`・Win32 error 5で製品分岐前にBLOCKED。Coreの逆移動ロジックはPASS。
- 現HEADとRelease exe hashをmetadataへ記録したPrintWindow screenshotを取得。これはnative描画evidenceでありHuman視認性の判定ではない。
- real holiday HTTP opt-inを現Releaseで2回実行し、302 checks PASS。mock/local policyも従来どおりPASS。
- no-remote隔離Git、`git diff --check`、tracked secret/private absolute path scanをPASS。

## Human-only gate

- Microsoft IME / ATOKの合成・候補確定・Undo操作感。
- 100/125/150/200% DPI、複数monitor移動、表cell hit-test/操作感。
- PNG/JPEG/GIF/WebP/SVGの表示、animation、resize、表と画像の主観視認性。
- outline/calendar click-drag、Compact/calendar tooltip、設定/profile連続操作感。

上記は自動probeのPASSへ昇格しない。Computer Useの`orca` CLIがこの環境では利用できず、RootはHuman gate checklistとして切り出す。

## 既知の外部境界

旧環境のWinHTTP 12185ログは履歴として保持する。今後同じTLS/client-certificate環境境界が再現した場合はBLOCKEDと記録し、資格情報追加やOS security変更で通さない。
