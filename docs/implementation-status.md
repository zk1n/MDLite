# MDLite 初版 実装・受入状況

更新日: 2026-09-20

この文書はTask Inbox同梱仕様v0.3のA-01〜A-34と、現在の実装・fresh verificationを対応付ける。
「実装済み」は自動的にHuman UI受入済みという意味ではない。自動試験、実行スモーク、
Human操作、未検証を区別する。

## 現在の構成

- C++20、Win32、RichEdit 4.1、Common Controls v6 manifest、CMake／Ninja／MSVC。MSVC runtimeは
  クリーンなWindows 11で追加インストールを要求しない静的リンク。
- WebView、JavaScript／Node.js実行環境、汎用プラグインは製品へ含めない。
- 文書は通常ファイルが正本。共有設定は`.mdlite`、再生成可能データは`.cache`、
  復旧・セッション・置換状態は`.state`へ分離する。
- source-authoritativeな`EditorAdapter`がsource範囲とRichEdit UTF-16位置を対応付ける。
  画像はsourceのMarkdown範囲を派生ビューのU+FFFCへ写像し、RichEdit native image objectとして
  挿入する。表は同じ編集面へ等幅・背景・罫線presentationを適用し、セル操作はsource transactionへ戻す。
  自動testはsource保全・画像削除範囲・表範囲を確認済み。Computer Useがnative app inventoryを
  返さなかったため、クリック／スクロール／DPI／GIF animationを含むG1実操作は未検証。

## 対応表

| ID | 現在の実装 | fresh verification | 判定 |
|---|---|---|---|
| A-01 | native tree複数選択、ファイル／フォルダー作成、複数コピー・移動、ごみ箱削除、名前変更、tree内D&D、上書き・自己配下移動拒否。開いている文書またはその親フォルダーの移動／削除を抑止し、Escでdragを取消。 | コンパイル済み。複数選択とD&DのGUI操作は未実施。 | 実装済み・GUI未検証 |
| A-02 | 表示書式はDocumentへ書かず、TOMで表示書式中のUndo記録を停止。 | Core本文バイト試験PASS。GUI Dirty/Undo未実施。 | 部分検証 |
| A-03 | IME合成中の再装飾・自動保存・表セル移動を停止。 | Microsoft IME／ATOK Human Gate未実施。 | 未検証 |
| A-04 | 見出し混在高さ、derived native image object、罫線付き表presentationを同一RichEditへ統合。 | source/view mapping unit PASS。実画面の一体操作はComputer Use bind不能で未検証。 | 実装済み・実操作未検証 |
| A-05 | Front Matter、ATX見出し、fence、strong、strike、codeの範囲認識。 | parser unit PASS。完全なCommonMark/GFM未実装。 | 部分実装 |
| A-06 | アウトライン、文書検索、Workspace検索から移動。Markdown link／autolinkをnative hover表示し、同一／相対文書と見出しanchorへ移動、外部URLは確認後に既定アプリで開く。 | Core検索／見出し／link parser試験PASS。link実クリックは未検証。 | 実装済み・GUI未検証 |
| A-07 | 正確な直接走査を採用。include/exclude glob・取消callbackを実装。初版では候補indexを採用せず直接走査をcanonical結果とする。 | UTF-8／CP932、regex、単語、glob unit PASS。index比較は非採用判断のため該当なし。 | 自動PASS・設計差異記録 |
| A-08 | 未保存buffer、CP932、短語、記号、literal/ECMAScript regex、大小文字、単語、複数行を検索。GUI条件checkboxと置換欄を接続。 | Core検索・置換PASS。GUIクリック未検証。 | 実装済み・GUI未検証 |
| A-09 | preview、適用直前の全文改訂照合、部分競合、`.state/replace` journal、再起動後の条件付きrollbackを実装。 | apply/rollback unit PASS。取消は検索callbackとprocess handle、置換適用中UI取消は未実装。 | 部分検証 |
| A-10 | 日付展開、Dairy open-existing、Meeting／Memo連番、cursor除去。組込み→共通→Workspaceのprofile定義を作成経路とpreviewで共用。未知token、禁止文字、path脱出を拒否。 | Core profile作成、TOML round-trip、階層上書き、preview、拒否試験PASS。 | 自動PASS（定義済み変数） |
| A-11 | 750ms自動保存、Ctrl+S、改訂番号でDirty管理。 | Core save試験PASS。保存中の同時編集負荷試験未実施。 | 部分検証 |
| A-12 | 指紋による外部変更拒否、同一フォルダーtemp、flush、ReplaceFileW。 | 外部変更unit PASS。ディスク満杯／権限障害未実施。 | 部分検証 |
| A-13 | 未編集Saveは書込みなし。 | exact bytes unit PASS。 | 自動PASS |
| A-14 | CP932 best-fit禁止、変換不能文字を拒否。 | emoji拒否unit PASS。 | 自動PASS |
| A-15 | `.state/recovery`へUTF-8原文を原子的保存し、起動時に検出。 | Core recovery試験PASS。実クラッシュ復元未実施。 | 部分検証 |
| A-16 | session schema v2でtab順、active tab、source選択、scroll、main／各compact windowの配置を保存・画面内へclampして復元。Ctrl+W、Workspace正規path別mutex、文書ビューを移送する複数compact windowを実装。 | 詳細session round-trip unit PASS。compact配置の実GUI復元は未検証。 | 実装済み・部分検証 |
| A-17 | 信頼済みWorkspaceだけでGit CLIを引数配列実行。status/diff、Workspace範囲stage/unstage、commit、branch作成/切替、merge/abort、fetch、ff-only pull、pushを確認付きGUIへ接続。未マージ一覧を根拠にdiff3を含む競合へ移動し、現在側／相手側／両方を一Undo操作で採用、残存marker警告後の明示stageを実装。長時間操作は応答する進捗dialogとJob取消へ接続。 | Process cancel、通常／diff3競合parser unit PASS。実repository GUI操作とUndoは未検証。 | 実装済み・部分検証 |
| A-18 | 未信頼Workspaceでは外部processを拒否。GitはWorkspace内の同名exeを探索せず共通PATHから解決。ProcessRunnerは引数配列、Job、timeout/cancel、64KiBログ上限。外部処理中はautosaveを調停する。 | Trust／process／cancel unit PASS。実Git hook／credential経路は未検証。 | 自動PASS・実環境未検証 |
| A-19 | standard→共通 `%LOCALAPPDATA%`→Workspaceの設定階層と由来表示、system/light/dark、font、keybinding、上書き解除をGUIからatomic保存・即時適用。競合／不正値を拒否し、名称検索command palette（Ctrl+Shift+P）も実装。 | 階層・由来・atomic save・競合・不正TOML unit PASS。専用フォームではなく連続prompt UI、custom theme色は未実装。 | 部分実装 |
| A-20 | 共有設定と`.state`を分離し、Trustは`.state/trust.local`のpath固有tokenでGit移行されない。 | trust設定・解除unit PASS。 | 自動PASS |
| A-21 | アプリ本体にネットワークAPIなし。祝日は同梱。 | Debug起動スモークで通信計測は未実施。 | 部分検証 |
| A-22 | `tools/Measure-Performance.ps1`が隔離Workspaceを生成しReleaseを測定。 | 2026-09-20 fresh 5秒sample: P0 WS 14,794,752/private 2,293,760 bytes、2,000 files WS 15,974,400/private 3,457,024 bytes。通常目標50MB内。UI操作p95は未測定。 | 部分PASS |
| A-23 | 文書サイズによる機能無効化なし。EditorSnapshotを改行ごとの複数16-byte pairから32-bit改行索引1本へ圧縮し、画像だけのsnapshot作成は全Markdown解析から分離。P3 fixtureは巨大1行＋plain Markdownとし、全行headingという別負荷を除外。 | 2026-09-20 fresh: 20MiB WS 111,689,728/private 144,719,872、100MiB WS 415,903,744/private 670,212,096、いずれも5秒時点Responding。100MiBはなお高水準、P5反復増加は未測定。 | 未達（改善・実測済み） |
| A-24 | `.cache`と`.state`を別ディレクトリ・別ignore規則に分離。 | Core初期化試験PASS。清掃障害試験未実施。 | 部分検証 |
| A-25 | `vswhere`によるMSVC/CMake/Ninja検出、VS Code task/launch、静的MSVC runtime、Common Controls v6 manifest。 | Debug/Release build/test PASS。通常環境起動時の「序数345」loader errorを再現し、v6 manifest欠落を修正。埋込みmanifest抽出と6秒起動生存・応答を確認。F5 Human操作は未実施。 | 部分検証 |
| A-26 | 公開投影なし。build、`.codex`、秘密拡張子をignore。 | 秘密／個人パスscan未実施。 | 部分検証 |
| A-27 | Tab／Shift+Tab、末尾行追加、行列追加削除のsource transactionと同一面の罫線presentation。 | Table unit PASS。矢印境界／GUI Undoは未検証。 | 実装済み・実操作未検証 |
| A-28 | 子見出しを含むsection move、子孫drop拒否、単一RichEdit Undo操作。 | section unit PASS。実ドラッグ／Undo Human Gate未実施。 | 部分検証 |
| A-29 | 既定パス、原子的採番、cursor位置。共通／Workspace範囲のadd・edit・duplicate・delete、保存先preview、template本文を開くGUIを実装し、作成時に同じprofile定義を再読込。 | Profile unit PASS。連続prompt GUIの実操作と任意入力field定義は未実装／未検証。 | 部分実装 |
| A-30 | 日曜始まりMonth Calendar、作成済み日／祝日bold、日付作成。hover tooltipに祝日名、Dairy有無、データ版・収録年、範囲外は不明と表示。 | 公式CSV 2026-02-02取得の2025〜2027をunit確認。tooltip実操作未検証。 | 実装済み・GUI未検証 |
| A-31 | 5形式のD&D、assetsコピー、衝突回避、相対リンク、危険SVG拒否、native inline object、source mapping、clipboard bitmapの一意PNG保存、Markdownから限定HTML imgへのUndo可能な幅変更を実装。 | Asset／mapping／img width parser unit PASS。実decode 5形式、clipboard GUI、比率表示、GIF/animated WebP再生は未検証／未実装。 | 部分実装 |
| A-32 | 明示GUI操作・Trust・設定可能なexternal command adapter、取消handle、失敗時local保持、実行前後hash、revision引数を実装。 | mock CLI成功とlocal不変 unit PASS。本番資格・通信は未使用。 | 自動PASS（mock） |
| A-33 | 初回ローカル試験では署名保留。 | 公開packageを生成していない。 | Release時保留 |
| A-34 | feature checkpointを両remote同名refへ通常push。 | `4702cf76c0abbb0af5ecf79188e9d26704dafd21`がForgejo/GitHubで一致。最終develop統合は未実施。 | checkpoint PASS |

## 現在の自動試験

`mdlite_core_tests`は文字コード、改行、外部変更、Markdown範囲、Workspace状態、復旧、
プロファイル、検索、表、アウトライン移動、祝日、画像asset安全性を一つの一時Workspaceで検査する。
GUIを呼ばないため、クリック、D&D、Undo/Redo、Microsoft IME、ATOK、DPI、アニメーション画像は
合格へ昇格させない。

## 未完了のHuman／実装ゲート

derived viewに対する実クリック、スクロール、hit test、画像resize／animation、Microsoft IME、ATOK、DPI、
table矢印・Undoは未検証。Computer Use helperがnative app inventoryを返さなかったことは環境上の証拠であり、
Human受入PASSを意味しない。A-16 compact配置復元、A-17競合補助／cancel、A-19設定prompt、A-29 profile管理prompt、A-30 tooltipは
実GUI未検証。A-19 custom theme／専用フォーム、A-23大容量最適化、A-29任意入力field、A-31 animationは実装未完として残す。
