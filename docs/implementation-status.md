# MDLite 初版 実装・受入状況

更新日: 2026-09-19

この文書はTask Inbox同梱仕様v0.3のA-01〜A-34と、現在の実装・fresh verificationを対応付ける。
「実装済み」は自動的にHuman UI受入済みという意味ではない。自動試験、実行スモーク、
Human操作、未検証を区別する。

## 現在の構成

- C++20、Win32、RichEdit 4.1、Common Controls、CMake／Ninja／MSVC。
- WebView、JavaScript／Node.js実行環境、汎用プラグインは製品へ含めない。
- 文書は通常ファイルが正本。共有設定は`.mdlite`、再生成可能データは`.cache`、
  復旧・セッション・置換状態は`.state`へ分離する。
- 現在のRichEdit案は見出し混在高さとIME合成中の再装飾停止を実装したが、
  Markdownソースを壊さないインライン画像表示を満たせないためG1は未合格。

## 対応表

| ID | 現在の実装 | fresh verification | 判定 |
|---|---|---|---|
| A-01 | 単一選択の作成、コピー、移動／名前変更、ごみ箱削除。上書き拒否。 | コンパイル済み。GUI操作未実施。複数選択未実装。 | 部分実装 |
| A-02 | 表示書式はDocumentへ書かず、TOMで表示書式中のUndo記録を停止。 | Core本文バイト試験PASS。GUI Dirty/Undo未実施。 | 部分検証 |
| A-03 | IME合成中の再装飾・自動保存・表セル移動を停止。 | Microsoft IME／ATOK Human Gate未実施。 | 未検証 |
| A-04 | 見出しと本文の文字高を分離。表はソース操作。 | Debug起動PASS。画像インライン表示未実装。 | 未達 |
| A-05 | Front Matter、ATX見出し、fence、strong、strike、codeの範囲認識。 | parser unit PASS。完全なCommonMark/GFM未実装。 | 部分実装 |
| A-06 | アウトライン、文書検索、Workspace検索から移動。 | Core検索／見出し試験PASS。リンク移動未実装。 | 部分実装 |
| A-07 | 正確な直接走査のみ。 | UTF-8／CP932の直接走査PASS。索引未実装。 | 未達 |
| A-08 | 未保存バッファ、CP932、短語、記号、ECMAScript regex検索。 | Core検索試験PASS。置換未実装。 | 部分実装 |
| A-09 | 一括置換用`.state/replace`領域のみ作成。 | journal／中止／再開未実装。 | 未達 |
| A-10 | 日付展開、Dairy open-existing、Meeting／Memo連番、cursor除去。 | Core profile試験PASS。 | 自動PASS |
| A-11 | 750ms自動保存、Ctrl+S、改訂番号でDirty管理。 | Core save試験PASS。保存中の同時編集負荷試験未実施。 | 部分検証 |
| A-12 | 指紋による外部変更拒否、同一フォルダーtemp、flush、ReplaceFileW。 | 外部変更unit PASS。ディスク満杯／権限障害未実施。 | 部分検証 |
| A-13 | 未編集Saveは書込みなし。 | exact bytes unit PASS。 | 自動PASS |
| A-14 | CP932 best-fit禁止、変換不能文字を拒否。 | emoji拒否unit PASS。 | 自動PASS |
| A-15 | `.state/recovery`へUTF-8原文を原子的保存し、起動時に検出。 | Core recovery試験PASS。実クラッシュ復元未実施。 | 部分検証 |
| A-16 | 終了時session書込み。 | 復元、コンパクトウィンドウ、単一Workspace排他は未実装。 | 未達 |
| A-17 | なし。 | Git補助未実装。 | 未達 |
| A-18 | 通常操作に外部コマンド／通信なし。 | 静的構成のみ。TrustとGit経路未実装。 | 部分実装 |
| A-19 | 宣言設定ファイルの初期化。 | 設定GUI、テーマ、キー割当て編集未実装。 | 未達 |
| A-20 | 共有設定と`.state`を分離。 | Trust永続化／移行試験未実装。 | 未達 |
| A-21 | アプリ本体にネットワークAPIなし。祝日は同梱。 | Debug起動スモークで通信計測は未実施。 | 部分検証 |
| A-22 | 計測入口あり。 | P0自動スモーク: Debug Private 2,314,240 bytes／Working Set 15,237,120 bytes、Release Private 2,211,840 bytes／Working Set 14,508,032 bytes。2,000文書・約21.1MiBのP1限定スモークは105msで応答、Private 2,654,208 bytes／Working Set 15,503,360 bytes。操作シナリオを含むP1とP2〜P5は未測定。 | 未完了 |
| A-23 | 文書サイズによる機能無効化なし。 | 20MiB文書のRelease限定スモークは5秒時点で応答あり、Private 63,967,232 bytes／Working Set 52,875,264 bytes。読み込み完了時刻、100MiB、反復試験は未測定。 | 部分検証 |
| A-24 | `.cache`と`.state`を別ディレクトリ・別ignore規則に分離。 | Core初期化試験PASS。清掃障害試験未実施。 | 部分検証 |
| A-25 | `vswhere`によるMSVC/CMake/Ninja検出、VS Code task/launch。 | Debug build/test PASS。F5 Human操作未実施。 | 部分検証 |
| A-26 | 公開投影なし。build、`.codex`、秘密拡張子をignore。 | 秘密／個人パスscan未実施。 | 部分検証 |
| A-27 | Tab／Shift+Tab、末尾行追加、行列追加削除のsource transaction。 | Table unit PASS。視覚セル／矢印境界／GUI Undo未達。 | 部分実装 |
| A-28 | 子見出しを含むsection move、子孫drop拒否、単一RichEdit Undo操作。 | section unit PASS。実ドラッグ／Undo Human Gate未実施。 | 部分検証 |
| A-29 | 既定パス、原子的採番、cursor位置。メニュー作成。 | Profile unit PASS。GUI設定編集未実装。 | 部分実装 |
| A-30 | 日曜始まりMonth Calendar、作成済み日／祝日bold、日付作成。 | 公式CSV 2026-02-02取得の2025〜2027をunit確認。GUI tooltip未実装。 | 部分実装 |
| A-31 | 5形式のD&D、assetsコピー、衝突回避、相対リンク、SVG危険要素無効化、width markup。 | Asset unit PASS。インライン表示、貼付け、GUIリサイズ未実装。 | 未達 |
| A-32 | ローカルassetは失敗時リンク未挿入。 | external-command adapter未実装。 | 未達 |
| A-33 | 初回ローカル試験では署名保留。 | 公開packageを生成していない。 | Release時保留 |
| A-34 | feature branch上のローカル実装。 | 両remote push/read-back未実施。 | 未完了 |

## 現在の自動試験

`mdlite_core_tests`は文字コード、改行、外部変更、Markdown範囲、Workspace状態、復旧、
プロファイル、検索、表、アウトライン移動、祝日、画像asset安全性を一つの一時Workspaceで検査する。
GUIを呼ばないため、クリック、D&D、Undo/Redo、Microsoft IME、ATOK、DPI、アニメーション画像は
合格へ昇格させない。

## 次の技術ゲート

G1を合格させるには、Markdown正本を保持しながら、表セルと画像を本文内でネイティブ表示し、
スクロール、hit test、選択、IME、Undoへ統合できる編集表示層が必要である。
`EM_INSERTIMAGE`はRichEdit本文へオブジェクト文字を挿入するため、そのまま採用しない。
