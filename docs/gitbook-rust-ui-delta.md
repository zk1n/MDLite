# GitBook更新用・Rust UI差分

旧C++の説明と履歴は残し、以下をRust節として反映するための日本語差分。
このファイル自体はGitBookへの書込み・公開の証拠ではない。
操作契約は[Workspace操作](workspace-operations.md)、実施結果は
[実装・検証表](ui-feature-parity.md)と[受入記録](rust-acceptance.md)を正本とする。

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
