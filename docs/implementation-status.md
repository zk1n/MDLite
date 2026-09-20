# MDLite 表示・UI再構築 実装・受入状況

更新日: 2026-09-21
対象Task: `20260920-mdlite-visual-rebuild-localfirst` Revision 1

この表は実装、自動試験、実GUI経路試験、Human確認、外部準備待ちを分離する。
`実装済み`や`自動PASS`はHumanの操作感・見た目の受入を意味しない。性能値と
R-01〜R-08の根拠は [acceptance-hardening-report.md](acceptance-hardening-report.md) に記録する。今回の座標・UI設計は
[ui-design.md](ui-design.md) と [adr-20260921-native-coordinate-ui.md](adr-20260921-native-coordinate-ui.md) に記録する。

## 今回の変更と境界

- Jev Initial Decision Gate は `sol_xhigh` を選択した。固定 specialist の判定は、既存の座標混同を解消してから UI/table を再構築するよう指示した。実行 provenance が要求 target と不一致だったため dispatch は fail-closed とし、model/effort を推測しない。
- source/export/native の座標を分離し、CRLF/LF と Markdown image object は compact discontinuity で写像する。`EditorText` は raw native text を読む。
- presentation reset は underline/effects/paragraph spacing/border を明示的に解除し、表 grid は固定22px矩形でなく実測した行高・共有境界で描画する。
- UI layout は DIP scaling、`WM_DPICHANGED`、PerMonitorV2 manifest、editor font 再生成を実装した。実機のDPI・IME・通常GUI画面はまだPASSにしない。
- 祝日は同梱データを維持し、検証付きのローカルCSV取込みと user-wide `holiday_auto_update=false` 設定を追加した。明示許可時だけ既知の内閣府HTTPS CSVを月次確認し、Workspace cacheをlast-known-goodとして更新する。公式サイトへの実HTTP受入は未実施。

## 現在の構成

- C++20、Win32、RichEdit、Common Controls v6、CMake／Ninja／MSVC。WebView、JavaScript／Node.js、汎用pluginを製品へ含めない。
- Markdown sourceを正本とし、書式・画像・表は派生表示とする。source変更はDocumentのtransaction/Undo履歴を経由し、RichEdit native Undoは表示更新後に破棄する。
- 通常ファイルを正本にし、共有設定は`.mdlite`、再生成可能データは`.cache`、復旧・session・置換journalは`.state`へ分離する。
- TrustはWorkspace内へ書かず、ユーザーlocal storeにcanonical pathとvolume/file identityを保存する。

## A-01〜A-34

| ID | 実装・接続 | 今回の証拠 | 判定 |
|---|---|---|---|
| A-01 | native treeの作成・移動・rename・ごみ箱削除、同名／自己配下拒否 | Core回帰とbuild。複数選択D&DのHuman操作は未実施 | 実装済み・Human未確認 |
| A-02 | 表示更新はsourceを書き換えず、アプリ管理source履歴だけをUndo/Redoとして公開。表示専用native Undoは通知抑止中に破棄。同一行で識別不能な複数画像はraw Markdownへfail-closedし、完全にcollapseしたsource rangeだけnative描画。source/export/nativeの座標境界と画像objectのdiscontinuityを明示 | Core byte/Dirty・複数画像identity・隣接画像・native transaction回帰、Release GUI acceptanceのSave/Undo/Redo/隣接raw保存 | 自動PASS（Human IME/視認性は未確認） |
| A-03 | IME状態を文書単位で保持し、合成中のsync／保存／表操作を停止 | compactを含むnotification経路を回帰。Microsoft IME／ATOK実変換は未実施 | 実装済み・Human gate |
| A-04 | 見出し・本文・native image object・表presentationを同一RichEditへ統合 | parser/source/native座標回帰。実画像と表の自然な見え方はHuman未確認 | 実装済み・Human gate |
| A-05 | Front Matter、CommonMark/GFM対象構文をsource範囲付き解析 | Core parser回帰 | 自動PASS（公式全corpusは対象外） |
| A-06 | outline、link、現在文書／Workspace検索結果からsource位置へ移動 | GUI acceptanceで検索結果選択経路、Core link/outline回帰 | 自動PASS・link clickはHuman未確認 |
| A-07 | canonicalな直接走査。初版は候補indexを採用しない。include/exclude globと階層`.gitignore`を尊重 | UTF-8/CP932、regex、word、`*`/`?`/`**`、否定、directory、anchored、200-file回帰 | 設計差異を記録・自動PASS |
| A-08 | 未保存buffer、CP932、日本語、記号、大小、word、複数行、ECMAScript regex/capture置換 | Core検索／置換、GUI case検索 | 自動PASS |
| A-09 | preview、適用直前改訂照合、部分競合、取消、disk journal／rollback。open bufferはUndo可能 | apply/cancel/rollback回帰 | 自動PASS・長時間cancelのHuman未確認 |
| A-10 | 日付・profile・template・任意入力・path検証を共通処理で展開 | Core profile回帰 | 自動PASS |
| A-11 | 750ms autosave、Ctrl+S、保存世代を分離。保存後もdisk contentを再照合 | 保存中編集・置換前後の障害注入回帰 | 自動PASS |
| A-12 | 外部変更拒否、同一folder temp、flush、再fingerprint、write非共有guard、replace、保存後再照合 | before/guarded/after replace障害注入。rename/deleteを含む完全CASは主張しない | 自動PASS・保証範囲記録 |
| A-13 | 未編集Saveは書込みなし | exact-byte回帰 | 自動PASS |
| A-14 | CP932 best-fitを許さず変換不能文字を拒否 | emoji拒否回帰 | 自動PASS |
| A-15 | `.state/recovery`へ原文をatomic保存し、復元／破棄／保持 | Release GUIで5秒timer後に実process kill、disk不変、再起動promptから未保存本文復元 | 自動PASS |
| A-16 | session v2、tab／選択／scroll／window位置、compact view移送、Workspace mutex | session回帰。複数monitor復元はHuman未確認 | 実装済み・Human gate |
| A-17 | Trust後のGit status/diff/stage/commit/branch/merge/fetch/ff-only pull/push、diff3競合補助 | Process／cancel／conflict parser回帰。実repo GUI操作は未実施 | 実装済み・Human未確認 |
| A-18 | 未信頼Workspaceで外部processを拒否。引数配列、Job、timeout、log上限 | Trust identity copy/self-declare、process/cancel回帰 | 自動PASS |
| A-19 | common／Workspace設定、由来、theme、font、keybinding、palette、未知field保持 | 設定階層・atomic save・競合回帰 | 自動PASS・連続promptはHuman未確認 |
| A-20 | 設定はGit移行可能、Trustはuser-local identity storeで移行しない | copyされた設定／自己申告token拒否回帰 | 自動PASS |
| A-21 | 通常起動は無通信、休日は同梱。更新は明示したローカルCSV取込みまたは、commonで明示許可した既知の内閣府HTTPS CSV月次確認のみ。Workspace設定は通信許可を引き上げない | 追跡依存／文字列scan、休日CSV parser／重複拒否／設定既定値回帰。HTTPはWinHTTPの固定host・TLS・timeout・サイズ・redirect拒否経路。OS shell/外部CLIは明示操作のみ | 自動・local経路自動PASS（実HTTP/Human受入は未実施） |
| A-22 | ReleaseのP0〜P5測定scriptとJSONを作成し実行 | full rebuild: P1 six WS 50,257,920 bytes、P4 WS 43,376,640 bytes、入力p95最大6.801ms、compact 3/3、theme 100/100、P5 100/100失敗0。P3 20/100MiBは246.6/1,234.5MBで機能を無効化しない制約を記録 | 通常1〜6文書の50,000,000-byte目標は約0.26MB超過（未達を隠さない） |
| A-23 | 大容量でも機能を無効化せず、入力時全文parse/decodeを遅延し、不変画像object/cacheを再利用 | 20MiB/100MiBと100回反復を実測。100MiB peak WS 1,135,865,856 bytesは高水準 | 測定済み・制約記録 |
| A-24 | `.cache`と`.state`を分離 | WorkspaceStore回帰 | 自動PASS |
| A-25 | vswhere、CMake/Ninja/MSVC検出、VS Code task/launch、静的runtime、v6 manifest | Debug/Release build/testとexe起動。F5キー操作自体はHuman未実施 | 自動PASS・Human未確認 |
| A-26 | 公開投影なし。build、`.codex`、秘密拡張子をignore | tracked fileの秘密形式／個人絶対path scan | 自動PASS |
| A-27 | 同一RichEdit上のcell grid、表source transaction、Tab/Shift+Tab、矢印境界、行列操作、Undo接続 | grid cell解析、EOF/CRLF/escaped pipe/code pipe、Release GUI acceptanceの編集/Undo/Redo | 自動PASS・hit-testの操作感はHuman評価待ち |
| A-28 | 子見出しを含むsection move、子孫drop拒否、単一Undo | section回帰。実dragはHuman未確認 | 実装済み・Human gate |
| A-29 | Dairy/Meeting/Memo既定、採番、任意field、profile GUI、cursor | profile回帰 | 自動PASS・GUI promptはHuman未確認 |
| A-30 | 日曜始まり、同梱日本休日、検証付きローカルCSV追加、common許可時の月次cache、既存Dairy、tooltip | 同梱データとCSV importのCore回帰、cache last-known-good。DPIごとの見え方はHuman未確認 | 自動PASS・Human gate（実HTTPは未実施） |
| A-31 | PNG/JPEG/GIF/WebP/SVG経路、clipboard、D&D、比率固定resize、SVG XML安全判定、画像ごとのanimation期限 | 5形式のnative RichEdit挿入、GIF partial frame/offset/transparency/disposal 2/3、GIF/WebP異周期timing、SVG同一bytes検査、Release GUI acceptanceの複数object前後source保存、隣接raw保存、full P4 | 自動PASS・見え方はHuman gate |
| A-32 | 明示Trust、external command adapter、取消、失敗時local保持、hash/revision | mock CLI回帰。本番資格情報は使用していない | 自動PASS（mock） |
| A-33 | 公開package生成をrelease手順まで拒否 | 今回は署名・package・Releaseを実施しない | Release時保留 |
| A-34 | featureとdevelopをForgejo/GitHubへ非forceで同一OID配送 | 最終OIDはHANDOFFのGit欄と両remote read-backへ記録 | 配送時に確定 |

## Human専用の残り

- Microsoft IME／ATOKの実変換、候補確定、削除、選択、Undoの操作感。
- 100/125/150% DPI、複数monitor、outline drag、table cell grid/hit-test、画像resizeの視認性。
- PNG/JPEG/GIF/WebP/SVGの実表示とanimated GIF/WebPの主観的な見え方。WebP decoderはlibwebp 1.6.0を静的リンクし、Windows側の任意codecには依存しない。
- Compact window配置、calendar tooltip、設定／profileの連続prompt、実repositoryのGit credential/hook操作。

これらはAI側の実装不足を隠すための待機ではなく、実機入力方式・主観評価・外部環境を必要とする最小Human gateである。
