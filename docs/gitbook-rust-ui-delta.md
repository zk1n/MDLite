# GitBook更新用・Rust UI差分

旧C++の説明と履歴は残し、以下をRust節として反映するための日本語差分。
このファイル自体はGitBookへの書込み・公開の証拠ではない。
操作契約は[Workspace操作](workspace-operations.md)、実施結果は
[実装・検証表](ui-feature-parity.md)と[受入記録](rust-acceptance.md)を正本とする。

## 検索・コマンド表示復旧の反映対象

Task `20261017-mdlite-ui-surface-recovery` Rev1の局所差分。2026年10月17日の
`product-ui`／`architecture-verification`限定更新を基準とし、旧C++節の履歴は維持する。
最終commit・build・実画面の合否は受入記録とHANDOFFに結び付ける。
今回の成果は未完了の作業branch checkpointで、developへは統合しない。
初回queryの一括入力で文字順が入れ替わるC05 FAILと、C01／C03のPARTIALを明記し、
GitBookの公開仕様へ受入済みとして反映しない。

| 対象章 | 反映する契約・実装差分 |
|---|---|
| 製品概要と画面操作 | ExplorerとSearchは独立した論理viewで、既存sidebar領域を排他的に使う。非表示部品へfocusや入力を残さず、query・設定・Explorer状態を保持する。 |
| 製品概要と画面操作 | パレットは一つのclient内hostで表示し、resize後も収める。IME取消を優先するEsc、外側の最初のクリックを消費するdismiss、実行・取消ごとのfocus復帰契約を記す。 |
| 検索・置換とパネル操作 | queryと結果を優先し、置換・対象詳細を展開する。Explorer／Search／Calendar／Outline／Gitの低頻度操作はheaderメニュー・既存popup・パレットへ移し、registryと安全条件を維持する。 |
| 構成と検証の境界 | App借用中の子部品の色問い合わせには描画専用の所有情報を使う。検索入力欄の白fallback回帰と通常起動W01/W03の白画面を別に扱う。単体試験、実画面、実IME、Human受入の境界を明記する。 |

左右pin／unpin、複数pin、狭幅drawer、旧四枠設定の移行とresetは後続設計であり、
今回の永続schemaやdrag dockingの実装ではない。W01/W03とE08の未解決記録も保持する。
タイトル・メニューバー全体の再設計、全IME／DPI／物理入力matrixは今回の合格へ含めない。

| 更新する章 | Rust節の変更内容 |
|---|---|
| 起動・Workspace | 明示初期化、未選択でも新規/開く、物理identityによる重複起動・重複文書検出、別Workspaceの別process起動 |
| 基本画面 | 4パネル、slot移動/折り畳み/非表示/reset、狭幅drawer、多tab、同じ文書と履歴を使うCompact |
| コマンド・設定 | 共通registry、実効bindingヒント、無効理由、共通/Workspace overlay、Apply/Cancel/継承、外部変更の照合 |
| 保存・復旧 | 既存file autosave既定750ms、独立recovery約5秒、`.state`と`.cache`、不明旧形式原本保持、UTF-16旧session読取りとsource/recovery束縛のUTF-8 schema 4 |
| Explorer・Navigation | 内部file一覧copy/paste、安全なmove/delete、query全体走査のQuick Open、現行parserのOutline、子節ごと1 Undo移動、安全なリンク確認 |
| ノート作成 | Dairy表記、Daily/Meeting/Memo、profile定義全体overlay、template token/cursor/必須入力、非上書き |
| Calendar | 日曜42セル、作成日時のローカル日付一覧、明示local祝日CSV、通常操作でHTTPなし |
| 検索・置換 | dirty buffer優先、PCRE2とUTF-8 source位置、bounded走査、不完全表示、preview/直前照合/flush済みjournal、旧V1/V2読取り |
| Git・Trust | Status/Diffも端末local Trust、選択だけStage/Unstage、indexだけCommit、明示remote、ff-only pull、競合block 1 Undo、owned有限process |
| 表・画像・storage | 上/下行と左/右列、source/toolbar入口、320/480/640 DIP、限定legacy img、document SVG拒否、確認付き外部adapterとhash/revision照合 |
| 診断・受入 | 手動local snapshotとprivacy、依存版、実行体identity、機械検証とphysical/IME/Human受入の区別 |

旧RichEditの同期timerや旧C++の表示用text複製は移植しない。Rustはcrop Ropeを唯一の編集正本とし、
表示・選択・検索・Outline・表・画像が同じ原文位置へ対応する。WebView、JavaScript runtime、
汎用plugin framework、常駐telemetry、自動updaterは追加しない。

未実施のphysical長押し/F5、MS IME全matrix、別monitor/DPI、Narrator/touchpad、本人の操作感、
禁止中clipboard、実storageサービスは、機械試験のPASSへ含めない。最終結果は対象buildとともに
受入記録/HANDOFFへ転記する。

## 新規の限定回帰とGitBook対象章

| GitBook対象章 | Rust節へ加える内容 | 記録する限定回帰 |
|---|---|---|
| 編集とMarkdown | 入力通知順序、fallback layout、通常close gate | 合成IME終了通知の次の入力が本文・selection・Undoへ反映されること、測定したfallback高が次段落と重ならないこと、closeが通常終了gateを通ること。native回帰の結果であり、連続GUI/実IME受入ではない。 |
| 日次ノートとカレンダー | 日付セル内のイベント表示 | 合成fixtureでイベントmarkerが所属するcell内に収まること。 |
| 製品概要と画面操作 | native child redraw、Feature/Git/Explorer/settingsのowner-draw | control identityに対応する描画先、空captionの安全なno-op、日本語captionの正確な文字数経路。native GDI回帰であり、最終画面のGUI確認ではない。 |
| 構成と検証の境界 | source／EXE／harnessの対応、借用中の再描画、機械・GUI・IMEの証拠 | 修正前の失敗と限定回帰を保持し、最終buildの実測結果と未確認条件を分ける。 |

章名は2026年10月15日の現行GitBookと照合した。今回の新しい受入結果は、最終buildのGUI・機械検証とreviewを確定してから記入する。根拠source commitは未確定。
