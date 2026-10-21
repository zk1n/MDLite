# GitBook更新用・Rust UI差分

旧C++の説明と履歴は残し、以下をRust節として反映するための日本語差分。
このファイル自体はGitBookへの書込み・公開の証拠ではない。
操作契約は[Workspace操作](workspace-operations.md)、実施結果は
[実装・検証表](ui-feature-parity.md)と[受入記録](rust-acceptance.md)を正本とする。

## 2026年10月22日・描画負荷とLive編集の追補差分

Task `20261022-mdlite-rendering-live-editing-repair` Revision 1。
Liveは原文の明示改行をcaret位置によらず別の編集行として保つ。
Front Matterとfenceの区切りはcaretが来た原文行を編集表示し、旧Rev3の全区切り常時表示を置き換える。
区切りの再入力／Undoで再構成した後も終端を再編集できることを確認する。
原文・encoding・LF/CRLF・dirty・履歴を表示のために変更せず、表のcell圧縮は別の規則を保つ。

設定は対象、有効値と適用元、下書きを区別し、優先順位を初期設定→ユーザー共通→Workspaceと説明する。
解除・resetは対象を明示する下書き操作とし、明示的な空・false・0の保存契約を維持する。
statusはWorkspace/Gitと文書情報を項目別に配置し、未確認のGit状態をcleanと表示しない。
静止時の不要なpaint/control同期と設定再配置途中の白化は、対象EXEを固定して原因・修正前後を確認する。
最終196b4b82…のRelease候補でLF/CRLF、Main/Compact caret、dark/light設定遷移と保存、
実local/noRemote Gitのstatus全文popupを確認した。VMの診断off・操作後idle CPUは
1コア換算76.15%→0.78%。全frame/持続GPU/他端末/物理IME/Human受入とは区別する。
詳細な対象・条件・raw commandの失敗分類は[受入記録](rust-acceptance.md)に従う。

開始時にGitBook CR7 `fkQ2xRsXyzfbDP89y6SX` と公開版を照合したところ、CR7は既にmergedで、
編集・設定・構成の3ページ本文は公開版と同一だった。過去の「draftのまま」は当時の記録として保持する。
今回の更新先は後継CR8 `K0pAzsgY5hINhqfbNZMj`（draft）。公開版とmerged CR7を保持する。
この差分ファイル自体を外部書込の証明とせず、実更新/読戻しは最終HANDOFFの記録を参照する。
CR8の製品概要・編集・設定・構成の4ページへ追補し、既存suffix全文保持を読戻した。
新規節は編集ページでexact一致、他3ページはunderscoreのMarkdown escapeだけの正規化と確認した。
4ページの公開版は旧本文と一致し、CR8はdraftのまま、publish操作は行っていない。

## Rev6候補：Git button描画の借用中処理漏れ（2026年10月21日）

Git hostから転送されるowner-draw要求が、App借用中かつenabled状態不変の場合に未処理となる経路を、
既存native fixtureで再現した。Gitの7操作buttonのimmutableな描画状態をnative配置前に公開し、
既存の所有同定・描画器・themeの確定時期・window破棄時の失効を維持して処理する候補へ修正した。
TrustやGit操作の有効条件、文書source／保存／履歴の契約は変更しない。

関連native 9試験、Clippy、format、登録Release buildは成立した。実画面の最終候補確認、
親panel titleの欠落とSettings後の提示遅延、drag／Compact／配置復元の受入は別に判定する。
この候補は未統合で、現行develop `902dfd17754d45d782508c137c63950b2bc495c4` と
GitBookのRev3統合済み節／CR6の扱いを変更しない。最終到達点は受入記録とHANDOFFに従う。

## 2026年10月19日・Rev2候補の下書き

製品source local commit `7847b04688b57a9216a9c402d7ef88fac65f6cd5` の候補契約と検証境界を、
既存draft CR6 `GEWkXQliAGYt3jQyWpVW` の4ページへ追補した。
製品概要、日次／Calendar、編集／Markdown、構成／検証が対象で、revisionは
`lhaBZKkokHRgZnmWH0eS`。左活動bar・領域開閉・上部一体query、1click選択／doubleclick Daily、
Daily一覧filter、Front Matter全行編集を未統合候補と明記した。
4ページの候補節と旧C++本文の完全保持を読戻し、公開main revision
`RMRPLqlyutca5w8bY68z` の不変を確認。merge／公開は行っていない。
実GUIは最終EXEのforeground拒否によりNOT_RUN、150 DPI・物理IME／Humanも未確認。
Task完了や公開developへの採用とは区別する。

## Rev1下書きの履歴（2026年10月19日・未統合）

GitBook draft CR #6 `GEWkXQliAGYt3jQyWpVW` に「構成と検証の境界」と
「検索・置換・Git」の候補節だけを追加した。revision `4OFWYabNwGB6bjwKYQ7w`。
読戻しで候補節と従来本文の完全保持を確認。CRはdraftで、merge／公開は行っていない。
Task12未成立のため現行develop仕様へ採用せず、150 DPI・全GUI未確認を明記した。

## 2026年10月18日・Live移動／UI／Git候補（未公開）

今回の候補は全native scrollbar共通化と必須GUI条件が未達であり、developへ統合しない。
そのためGitBookの現行develop仕様を候補の実装済み仕様で置き換えない。
後続の限定反映先は「製品概要と画面操作」（上部bar、font、pointer）、
「編集とMarkdown」（物理行番号、実空行、上下移動）、
「検索・置換・Git」（Trust明示操作後だけlocal init、親repo／worktree、remoteなし）、
「設定と保存形式」（modal ownershipと縦scroll）、
「構成と検証の境界」（最終buildと未確認境界）。旧C++節とstable linkは維持する。
公開時には候補・統合・実GUI・Human受入の区別と、その時点のHANDOFF根拠を明記する。

## Rev3の文字重なり修正と確認範囲

主画面のパレット／Git host自身のcaption描画を切り分け、名前と入力・通知を保った
native container描画に変更した。Searchの明示queryラベルは維持した。
直接関係するCompactの描画配送・clip・パレット子部品の色通知も確認したが、
当初のCompact採取では文字欠落を観測した。後続の通常操作・受動撮影では初回／再表示とも
見出しと複数行を読めることを確認した。旧採取のFAILと再表示の撮影制限は受入記録に保持する。
10月18日の同一EXEでの再試行では文書／Workspaceの実結果移動と主画面のパレット・window操作を
追加確認した。後続runでは各切替先を実画面で確認する10往復・20切替のnav送信区間432.990秒を記録した。
準備時のquery再入力を含む観測窓561.880秒とは分けている。
以前の即時frameの旧表示／Git白化を保持し、確認済み範囲は受入記録に限定する。
今回必須の代表条件の確認と、全IME／DPI matrix・物理入力・全製品Human受入を区別する。
開発線の通常統合とGitBook反映は最終HANDOFFの実結果で確認する。候補ごとの機械試験・実画面・残件は
[受入記録](rust-acceptance.md)と最終HANDOFFを参照する。

## Rev2の限定差分と未完了境界

検索viewを開く際の不要なeditor経由のfocus移動を削除し、非表示にする部品の既存focus保護を維持した。
診断なし最終候補で初回Unicode入力は5 fresh process連続PASS。query選択準備の順序は維持し、
選択messageの遮断や入力を遅らせる処置は追加していない。
一方、C01のWorkspace結果選択混在、C03の操作を覆う連続観測、最終候補の一部代表確認は未完了。
作業branchのcheckpointに留め、develop統合や公開仕様の受入済み更新は行わない。
buildと実画面の境界は[受入記録](rust-acceptance.md)と最終HANDOFFで示す。

## Rev1の検索・コマンド表示復旧の反映対象（履歴）

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

章名は2026年10月18日の現行GitBookでも照合した。今回の新しい受入結果は、最終buildのGUI・機械検証とreviewを確定してから反映する。Rev3製品source commitは`6d84d8c9762fa8857e141deb82ff38d8f9b27f78`。統合・公開の実結果は最終HANDOFFに記録する。

Rev6最終候補はsource47 efed05695df9375e843006fdfbea111784ac772a74178408fd15576f7b87a909、EXE15c3e041ff0102e38c51158e06c2507ee128a7291fbb220bdd975d3acfef0e94、harness05957f223d35f4d19f6de300dfd4c12a39326cbe8f8fd11c1d70893e24edceae。Git借用中描画の限定修正とnative9件、親damage保持1件、lint/buildを確認した。Git通常／狭幅の暗背景glyph、TitlebarClose後の文字・Undo機能は実確認したが、約678–699msのΩ画面反映遅れ、親見出し欠落とpanel／Compact／restart／Calendarの残件を保持する。先行「both-hidden本文空白」「Escape残像継続」の誤注記はSol直接表示auditで撤回した。根拠付き未完了checkpointとしdevelopへ未統合。新CR7 `fkQ2xRsXyzfbDP89y6SX` はdraftの候補節だけ、現行Rev3／merged CR6は変更しない。

## Rev8の統合と最終検証境界（2026年10月21日 JST）

Rev8は製品source commit `5e58a8e2df6853e5eb7e586caa7de203a7c1b2e4` をdevelop `c3779b48dceceb6416e19ac470bc0f81c804dfc5` へ通常統合し、origin/githubでOIDを読戻した。source47 fed94d3c…、登録Release EXE9a89bc48…、最終harness4b057495…が対象。Calendar固定行のsingle-line表示と同値splitter更新抑制を局所修正し、必要な4状態/次入力/held幅・高さ/Compact/非default通常9文書restart/Calendar・Git操作と限定Ω比較を完了した。旧原FAIL・訂正・無関係な未確認は保持する。

GitBook CR7は製品概要・Calendar・構成と検証境界の3ページだけへ追補し、exact markdownと既存本文保持を読戻した。CR7はdraftのまま、CR6 merged/公開現行は変更しない。HANDOFFは入力Rev8をarchiveしてprivate Driveへ初回保存/readback済み。最終文書版の配送結果と証拠はHANDOFFのreceiptを参照する。物理IME/Human/全製品受入、即時表示や全frame性能、公開binary/正式Releaseは別の境界である。
