# 製品要件とUI方針

Rust開発への移行で製品要件を削除しない。ここに記載する操作・安全条件を維持する。
現在のsource接続とslice証拠は[実装・検証表](ui-feature-parity.md)、対象buildのvisible/Human受入は
[受入記録](rust-acceptance.md)を参照する。Workspaceのidentity/lease/別process起動契約は
[Workspace操作](workspace-operations.md)に記録する。

## Workspaceと画面

Activity Barと既存のアイコン体系を維持する。中央は文書tabと一つの
native Live Markdown編集面。ExplorerとSearchは独立した論理viewとし、
現在は同じsidebar領域を排他的に使う。Calendar、Outline、Gitを含む既存配置は
移行まで維持する。補助操作で中央編集面を奪わない。
必要機能への到達性を保ち、低頻度操作はメニュー等へまとめて内容領域を優先する。
native shellのcommand routeは実装・検証表に記録する。

paneは折り畳み・復元できる。DIPで初期寸法を定義し、client幅から
editorの最小幅を先に確保する。狭幅でも負の幅や画面外の操作領域を作らない。
font、背景、入力、選択、境界、muted、accentの色を各面に共通適用する。
light/dark/system、DPI、high contrastを含む受入は実施範囲を明記する。

scrollは通常6 DIP程度、hover/dragで8〜10 DIP程度の灰色thumbを基準にする。
ヒット領域と標準wheel/keyboard操作は維持し、表の横scrollは表内に限定する。
native実装を使い、WebViewやJavaScript実行基盤を追加しない。

### パネル操作の入口

各パネルのheaderメニューとコマンドパレットから、既存registryの操作へ到達できる。
非表示・折畳み・slot移動は既存配置の操作として残す。

| view | 優先する常設内容 | 低頻度操作の入口 |
|---|---|---|
| Explorer | 一覧とファイル／フォルダー作成。NoWorkspaceではWorkspaceを開く入口 | headerメニュー／パレットに改名、削除、path等 |
| Search | scope、query、前後移動、結果。置換と対象詳細を展開 | headerメニュー／パレットにWorkspace置換、journal、復旧、中止 |
| Calendar | 月移動、今日、日付・内容 | 既存操作popupとheaderメニュー／パレット |
| Outline | 見出し内容 | headerメニュー／パレットに見出し移動等 |
| Git | 状態と一覧、既存の安全ガード | headerメニュー／パレットにも既存Git操作 |

操作の移設は実行条件を変更しない。無効な操作の理由は共通registryから表示し、
削除・置換・Git等の確認とbusyガードを維持する。

### 後続のパネル配置案（未実装）

Activity Barで選ぶviewをprimary sidebarに一つ表示し、必要なviewだけ左右へ
pinする方式を推奨する。同じviewを複製せず、unpin時は通常のsidebar選択へ戻す。
片側へ複数pinする場合はtabまたはstackでまとめ、狭幅では本文を優先するdrawerへ
退避する。復帰時はquery、展開、選択などのview状態を保持する。

後続Taskの設計出発点として、次の契約を提案する。現在の実装・設定形式ではない。

| 操作・状態 | 提案する契約 |
|---|---|
| pin | 選んだview一つを指定した左右の補助領域へ移す。同じviewが既にpin済みならその位置を選択し、複製しない。 |
| unpin | viewをprimary sidebarの選択先へ戻して表示する。query・展開・選択は同じviewの状態を引き継ぐ。 |
| 複数pin | 各側で順序付きtab／stackとして保持し、一つを選択する。別側への移動は同じviewの移動として扱う。 |
| 幅不足と復帰 | editorの最小幅を優先して補助viewをdrawerへ退避する。幅が戻れば元の側・順序・選択を復元し、viewを作り直さない。 |
| focus | 操作前にview内で有効だったfocusを、移動後も可視・有効なら維持する。隠す場合は表示先のviewへ移し、妥当な先がなければeditorへ戻す。 |
| 永続化 | viewの配置先・順序・選択・幅をWorkspace設定へ保存する。本文や検索語を配置設定へ複製しない。 |
| 旧四枠からの移行 | 旧slotを論理viewの配置へ一度だけ対応付け、重複viewを作らない。可視性・幅を可能な範囲で保持し、変換不能な設定は元データを保全して既定へ戻せるようにする。 |
| reset | primaryをExplorer、補助pinを空に戻す提案とする。editorへfocusを戻し、本文・選択・履歴・未保存状態を変更しない。 |

今回の検索・コマンド表示復旧では既存の永続schemaを使い、この配置案の
schema変更、migration、drag dockingは後続Taskとする。実装済み・受入済みの記録ではない。

## Editorと表

Markdown原文は文書の唯一の編集正本。描画、選択、リンク、検索、Outline、
表、画像は同じ原文位置へ対応する。UTF-8、UTF-16、grapheme、DIP、physical pixel、
文書/viewport/screen座標を区別し、古いgeometryを新しい文書へ適用しない。
表示だけの変更で原文、encoding、改行、dirty、履歴を書き換えない。

caretや選択に関係する構文記号をLiveで直接編集できる。IMEの未確定本文は
永続本文へ入れず、確定・取消・移動・選択・Undo/Redoが継続して動くことを要求する。
表はセル本文と構造記号を区別し、セル本文の意味のあるパイプは保持する。
Tab/Shift+Tab、Enter/Shift+Enter、セル端の矢印、行列操作を同じ文書と履歴へ適用する。
内容とfontから列幅・行高を計測し、短い表を画面幅へ無理に拡張しない。
hover/focusの操作表示が出ても本文の位置を動かさない。
現Editorの具体的なキー規則は[操作仕様](editor-operations.md)に固定する。

## 検索・置換とNavigation

Ctrl+Fは現在文書、Ctrl+Shift+Fと検索アイコンはWorkspace検索を左paneに開く。
scope、検索/置換入力、option、対象詳細、結果数・状態、ファイル別の結果を明示する。
include/excludeが空の場合に既定値を実入力として挿入しない。
dirty bufferを優先し、結果は原文revisionと検索条件に結び付ける。
未入力、検索中、該当なし、構文エラー、取消、上限、失敗を混同しない。
現在の一致は通常一致と区別し、非表示構文の一致も正しい原文位置へ移動する。
IME変換中のEscと表操作中のEscを検索の終了で横取りしない。

ExplorerとSearchの切替では、非表示側の一覧・toolbar・formを入力対象から外す。
隠れる部品にfocusを残さず、query、置換語、option、Explorerの展開・選択を保持する。
表示を続ける部品から検索へ移る際は、途中でeditorへfocusを移さない。
非表示にする部品のfocus移動は、その部品を実際に隠す経路で扱う。
置換のbusyや取消、検索結果の世代判定は表示状態によって迂回しない。
検索はquery・scope・前後移動を優先し、置換と対象詳細は折り畳める構成とする。
折り畳んだ設定が有効なら要約を表示する。低頻度操作の移設先を用意してから
常設ボタンを減らし、機能・有効条件・無効理由への到達性を維持する。

Workspace置換はプレビューと本文の再照合後に適用する。binaryやreparse pointは
対象から除外し、走査/正規表現/結果/previewの上限を扱う。
閉じた文書の置換はWorkspace内の復旧記録へ保存し、後続変更と衝突した場合は戻さない。
Outlineの階層移動は子見出しを含む原文範囲と一つのUndo単位に対応する。
code/Front Matter内の見出し風文字列はOutlineへ含めない。
Quick Openと検索結果の移動に別Editorへの切替を要求しない。

## コマンドパレットの表示と終了

queryと候補を一つのsurfaceとしてclient内の他panelより前に表示し、resizeとDPI変更後も
client内へ収める。候補名、shortcut、無効理由を読み分けられる余白と行高を確保する。
OS全体の常時最前面化や毎frameのz-order修復は行わない。

Escはパレット内のfocus位置によらず閉じる。ただし実IME変換中はIME取消を優先し、
composition終了後のEscで閉じる。外側の最初のクリックはdismissだけに消費し、
背後の操作を同時実行しない。有効なコマンドは一度だけ実行して閉じ、行先が指定する
focusを優先する。無効なコマンドは理由を表示し、実行せずパレットを維持する。

cancel時は開く前の部品が同じ所有範囲で可視・有効な場合だけfocusを戻し、
そうでなければ現在のeditor等へ戻す。本文選択の復元は同じ文書・revisionに限る。
終了後に残像や透明な入力領域を残さず、次の文字・矢印・Enterを一度だけ受け付ける。
表示切替だけで本文、dirty、revision、Undo/Redoを変更しない。

これらは2026年10月17日の復旧Taskの操作契約であり、実画面での合否は
[受入記録](rust-acceptance.md)と対象buildの証拠で別に示す。

## 保存・復旧・追加機能の要求

UTF-8/BOM、CP932、LF/CRLFと混在改行、外部変更、保存失敗を保護する。
autosaveとWorkspace復旧のsource経路は実装済みで、visible/再起動受入は別に確認する。復旧本文は
`.mdlite/.state`、破棄可能cacheは`.mdlite/.cache`に置き、AppDataへ本文を保存しない。

Template、Dairy/Memo/MeetingとCalendar、Git UI、Workspace trust、設定GUI、
変更可能keybinding、compact window、診断と手動更新のnative routeを接続する。実装接続だけで
GUI受入を確定せず、slice証拠と未確認条件を[実装・検証表](ui-feature-parity.md)に分けて記録する。
祝日は同梱データと明示したlocal取込みを基準にし、通常起動や月移動で通信しない。
壊れた取込みは既知データを維持する。Git/storage外部実行はTrustと明示確認を通し、
任意process launch UIを追加しない。
公開配布の署名・checksum・timestampとRelease受入は別の作業とする。
