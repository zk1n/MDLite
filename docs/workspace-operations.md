# Workspace UIと保存形式

Rust版の操作入口を記録する。最終buildで実施した操作と未確認条件は
[実装・検証表](ui-feature-parity.md)と[受入記録](rust-acceptance.md)を参照する。

## 画面とNavigation

中央の文書tabとEditorを、Explorer/Search、Calendar、Outline、Gitの4パネルが囲む。
パネル見出しから折り畳み・非表示・slot移動を選べる。Ctrl+Alt+1〜4で対象パネルを
移し、Ctrl+Alt+上下で高さを調整する。狭幅時は中央の幅を優先しdrawerで開く。
配置resetは表示メニューにある。Compactは同じ文書・選択・履歴の別windowで、
×は主windowへ戻す。主windowの終了は全tabの未保存確認を行う。

statusはWorkspace、明示取得したbranch、保存/外部変更、行列、encoding/EOL、文書種類を示す。
Ln/Colは1起算で、Colは行頭からのUnicode scalar数に1を加える。UTF-8 byte、UTF-16 unit、
graphemeや表示上の横位置とは別。外部変更は検出済みと未確認を区別し、Gitを自動起動しない。

Ctrl+Kとタイトルの入口でコマンド検索を開く。無効な操作も理由付きで表示し、
実効keybindingをヒントに使う。設定は共通/Workspaceの層を選び、空欄は継承する。
Applyは開始時snapshotと照合し、外部変更・不明schema・書込み失敗ではフォームを保持する。
詳細編集では未知の行を保持する。変更可能bindingと固定Editor/表/パネル操作の衝突を拒否する。

Ctrl+Pは相対path/filenameでQuick Openを絞る。表示は200件までで、queryは走査対象全体へ
適用する。候補の続きと走査未完了を表示する。Explorerのcopy/pasteは選択file一覧を
保持するアプリ内操作で、本文clipboardと別。開いたfileやその祖先のmove/delete、
同名上書き、自己配下移動、Workspace外やreparse point経由の書込みを拒否する。
Explorerは展開したfolderだけを読み、Windowsの種別iconを表示する。folderのEnter/左右で
展開・折り畳みを行い、折り畳んだ子の展開状態と選択を保持する。treeは表示20,000行・深さ64を
保護上限とし、打切りや読取り失敗を未完了として示す。常駐indexを作らない。

Outlineは現行parserのATX/Setext見出しを使う。子見出しを含む節の移動は1 Undoで、
stale revision・自己内部・IME中を拒否する。リンクはCtrl+click、右click、コマンドから開く。
外部URLとWorkspace外文書は対象を示した既定No確認を通す。任意schemeは実行しない。

## ノートとCalendar

Calendarは日曜始まり42セル。月移動・今日の選択と、日付のDaily起動を区別する。
日別一覧はWindows作成日時のローカル日付であり、更新日時やfilenameの日付ではない。
祝日は同梱データと明示local CSV取込みを使い、起動や月移動でHTTPを行わない。
高さが小さいpaneでは「操作…」から同じCalendarコマンドを開く。profile管理中は専用一覧・formを
同じpaneに表示する。日別一覧から開く直前に、Workspace・作成日・file種類を再確認する。

| 既定profile | 保存先 | 既存file |
|---|---|---|
| Daily | `Dairy/yyyy/yyyyMM/yyyyMMdd.md` | 開く |
| Meeting | `Meeting/yyyy/yyyyMM/yyyyMMdd.md` | 連番 |
| Memo | `Memo/yyyy/yyyyMM/yyyyMMdd/yyyyMMdd.md` | 連番 |

profile管理から追加・編集・複製・上書き定義の削除・template編集を行う。
同IDは定義全体でoverlayする。組込みprofileの上書きを削除すると下位定義へ戻る。
必須入力の空defaultは定義保存とpreviewでは許し、実作成時に検査する。
unknown token、複数cursor、衝突、危険pathは本文作成前に拒否する。
日付click/選択日のDailyはその日の0時。「今日のDairy」と通常profile作成は現在local日時を使う。
入力formが必要な場合も、作成開始時の日時をpreviewから送信まで保持する。

## 検索・置換

Ctrl+Fは現在文書、Ctrl+Shift+Fと検索iconはWorkspace。case/regex/word/hiddenは
既定OFF、ignoreはON。空include/excludeには実文字を挿入しない。F3/Shift+F3で巡回する。
dirty bufferを優先し、文書ID/revision・Workspace・query generationへ結果を拘束する。
検索表示だけでは本文・Undoを変えず、隠れた表構文の一致は一時的にsourceを表示する。

PCRE2を使いlookaround/backreferenceとcapture置換を扱う。regexは4096 UTF-16単位、
literalは8 MiB UTF-8まで。照合deadline 200msとengine資源上限を設ける。
走査は8 MiB/file、512 MiB/走査、200,000 entry、20,000一致、8 MiB結果を上限とし、
取消・読取り失敗・不正ignore・上限を全件完了と表示しない。
Workspace置換はpreview・確認・直前照合後に行い、closed fileは変更前にjournalをflushする。
open文書は文書ごと1 Undo。journal復元はopen tabと後続編集を拒否し、部分成功を明示する。

## Trust・Git・storage

Status/Diffも端末local Trustが必要。未TrustのWorkspaceではGitを起動しない。
Trust済みWorkspaceを開いた際は、既存repositoryを読取りで検出しStatusを更新する。
Trustはcanonical pathと物理directory identityへ結び付け、Workspace内の設定で自己信頼しない。
明示的なTrust付与・再試行では、親repositoryとworktreeを含むGitの検出結果を優先する。
repositoryがないことを直前に再確認できた場合だけWorkspace rootでlocal initする。
既定branch設定を尊重し、remote、user設定、initial commitを作らない。init失敗でも
Trustは維持し、原因と再試行を表示する。通常のrefreshだけではinitしない。
remote未設定でもStatus、Diff、Stage、Unstage、Commit、branch等のlocal操作を利用できる。
Fetch/Pull/Pushは開始前に無効にし、短い理由を表示する。
Gitは選択だけのStage/Unstage、indexだけのCommitを行う。通常操作で暗黙の保存・stage・
commit・pushを連鎖しない。branch/worktree変更の事前保存とGit確認は別段階である。
fetch/ff-only pull/pushは既存remoteと明示確認を使う。forceや自動rebaseを追加しない。
競合はGitの未マージ認定とmarker parserの両方を確認し、1 block採用は1 Undo。

`.mdlite/storage.toml`の外部adapterは実行file、反復argv、`{file}`/`{revision}`と
有限timeoutを設定する。Trustと既定No確認後だけ起動する。local asset前後hash、
文書ID/revision/元destination、exit code、上限内先頭行HTTPSを確認した場合だけ
destinationを1 Undoで変更する。remote成功後の本文反映失敗や取消は別の結果として表示する。
取消やtimeoutは外部副作用のrollbackを保証しない。

## 状態と互換性

| 対象 | 場所と扱い |
|---|---|
| 設定 | 標準→ユーザー共通→Workspaceのper-key overlay。未知行を保持 |
| session | `.mdlite/.state/session.toml`。schema 4はUTF-8 byte・source SHA256・復旧recordへの関連付け。旧1/2は本文に対してUTF-16変換、旧3も読取り |
| recovery | `.mdlite/.state/recovery`。約5秒の独立予約。寿命identityとdisk baselineを保持 |
| 置換journal | `.mdlite/.state/replace`。V3を書き、旧V1/V2を検査付きで読む |
| cache | `.mdlite/.cache`。破棄可能。文書本文をAppDataへ保存しない |

Workspaceの初期化は明示確認する。拒否・書込み不能でも通常編集を続け、使えない復旧を通知する。
既存fileのautosaveは既定ON、750ms（100〜60000ms）。IME/modal/mutation後に再予約する。
無題を自動Save Asしない。読み取れない旧形式・不明schema・不正座標は原本を保持する。
baseline不一致の復旧は既存dirtyへ上書きせず、未保存forkとして回収する。
dirty/無題のviewは一致するsource hashと復旧recordを持つ本文へだけ適用し、disk本文へ誤適用しない。
古い文書座標の不一致だけでsession全体を読取り専用にしない。明示破棄したrevisionを再生成しない。
Workspace外Save Asでは本文保存と復旧の関連付けを区別し、相対参照を勝手に再配線しない。

表・画像メニューと右clickから上/下行、左/右列、削除、alignment、source表示を操作する。
Ctrl+Shift+Tは表source、Ctrl+Alt+Tは操作帯focus。見出し・最後の本文行・最後の列を保護する。
画像は320/480/640 DIPと比率resize。限定legacy `img`互換は
[Markdown方言](markdown-dialect.md)に従い、document SVGは拒否する。

診断は表示/Refresh時だけlocal snapshotを採る。Working Set/Private Bytes/Peak、文書数、
encoding、検索状態、依存版を示し、本文・検索語・資格情報・Workspace絶対pathを含めない。
常駐telemetry、更新feed、installerは追加しない。
