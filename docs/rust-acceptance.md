# Rust Editor受入記録

## 2026年10月24日・最終候補の対象確認（全体受入 pending）

Task `20261023-mdlite-ui-properties-scroll-polish` Rev1の最終候補pin:

| 要素 | SHA-256 |
|---|---|
| Release EXE (`target/ui-properties-a-fix/target/release/mdlite.exe`) | `494cd6c9ca7857661630cba576aed7c903353c74def6d9da38684a5af16b0280` |
| Builder aggregate | `a560b8b1d0b80b7837ff2326bcdb29d3c77c1960e4952e4ea87b02bb3303306e` |
| GUI source aggregate | `dc887ba29f2ad548c7f56f21def89109793411de202366e1ab94533c88c30ad0` |
| Acceptance harness | `4b11b56b6dcc9d5cab55cdc189b09a079ce3011225c41a7e6fe4f593ae5cd2e5` |
| Final A private runner | `ae263f44325ea48e58f7cd1015626ecde3f8466aed2728b517754454685cf10d` |

Rust fmt、global check、strict Clippy、registered Release buildはこの候補でPASSした。
これらは全UI suiteやHuman受入のPASSを意味しない。
独立correctness reviewは最終候補をCLEARとし、product must-fixなしと判断した。
Final Aのprivate runner hashとtracked acceptance harness hashは別identityである。
過去のreceiptへcurrent harness hashを遡及して割り当てない。

- Aの最終native fixtureではkeyboard selectionとpointer hoverの両方からmeaningful PanelMove 162 helpを確認した。`TTN_SHOW`のcustom-position結果は1で、tooltipはmenu stackと重ならずwork area内に収まった。source/layout/history/focusと保護disk fixtureは不変で、processはexit 0で終了した。これはこの合成fixtureのkeyboard/hover経路だけを確認する。
- 別のread-only既知synthetic fixtureでnative Ctrl+V pasteとUndo/Redoのsource、control、disk不変条件を確認した。元のOS clipboard内容や復元状態は主張しない。

### A–G/R adoption matrix

これは要求ごとの採用範囲と未確認境界を示す。B–Gの以前のreceiptは最終diffで影響を受けない経路に限って引き継ぎ、全体suite PASSへ拡張しない。

| ID | 最終候補で採用する実装・確認 | 未確認・残余境界 |
|---|---|---|
| A | 重複説明を省き、警告・無効理由・meaningful helpを保持。最終keyboard/hover画像と通常終了で`TTN_SHOW`の実寸配置を確認。 | 選択したPanelMove 162経路。全tooltip組合せは未確認。 |
| B | 実geometry/drawerに基づくicon状態、変更childだけのinvalidate。4状態、empty right、保存意図/所属、左右移動、狭幅drawerを確認。 | 他端末の主観的表示差は未確認。 |
| C1 | code/YAMLの物理終端行をblock内に所有。LF/CRLF、区切り解除/再入力/Undo、pointer/keyboard、短いCRLF終端を確認。 | zero-extent Ctrl+End viewと実終端行を区別する。全YAML行列ではない。 |
| C2 | 専用header/native menu、既知/未知/未指定、取消/同値/Source往復/履歴、狭幅を確認。 | 追加info付き言語削除は理由付き無効。全言語組合せは未確認。 |
| D1 | native EDITのkey/value/list、追加削除・型変更、title/source/URL/date/author/tags/description、Tab/Esc/複数行、代表ATOK、read-only既知fixtureのCtrl+Vを確認。 | 物理手入力、preedit/candidate pixels、別IMEは未確認。clipboard復旧とは別。 |
| D2 | 範囲限定transaction、comment/unknown/order/EOL/body保全、UndoRedo/Source/save-reopen、stale/invalid/complex fallbackを確認。 | YAML全仕様GUIではない。未対応構造は明示Source入口へ案内。 |
| E | bounded worker/cacheと未保存snapshot。local note/fragment/image、missing/outside/URL no-fetch、Esc後のstationary抑止とmotion再hoverを確認。 | 未試験preview内容・外部取得は未実施。 |
| F | quote 3.5DIP、measured gutter gap、3桁番号/表/画像位置、calendar日付/曜日/祝なし/Daily印を対象画像・操作で確認。 | 全Calendar経路と他DPIは未確認。 |
| G | common geometry/grab offset、track/clamp/capture終了/native range。Editor press/drag/track/end/loss/resizeとQuickOpen ListBox scrollを確認。 | Outline/Calendarに表示thumbなし。全native-control scrollと体感performanceは未確認。 |
| R | 影響範囲のregression/targeted tests、fmt/check/strict Clippy/registered Release buildを確認。 | Monolithic full-suite PASS、physical input、Human受入、Git配送とは別。 |

Human受入、physical keyboard/typematic、別IME、別DPI/monitor、全UI suiteは未確認。OS clipboard original content/restorationも未確認のまま別扱いとする。これはTask全体の完了ではなく、Git統合・配送は別の配送記録で管理する。過去の失敗候補は下の歴史記録として保持し、現在候補の一括PASSへ読み替えない。

## Historical checkpoint: UI・プロパティ編集改善（2026年10月23日 JST、最終候補前）

Task `20261023-mdlite-ui-properties-scroll-polish` Rev1。開始developは
`0c1d0c2bf45cb4ad30292f1c456562daead436f1`。過去の節は当時の履歴として保持する。
以下はde083候補時点の記録であり、現在のRelease pinとA結果は上の最終候補節を参照する。
当時もtask branch上の候補で、develop統合・Git配送・Human受入は完了していなかった。

### 当時の候補と機械確認

- Rust 1.99.1、Release。source aggregate `001bbb7e3cce46d0926dafc145d79868f7545a3623cb84c2736da45c31bc5197`。
- EXE `D:/codex_work/MDLite/target/ui-properties-reviewed/target/release/mdlite.exe`、SHA-256 `de083444469e87e22f42f2f8c636536756934089fc15d2d281e442e7dc969688`。
- 通常の`target/release/mdlite.exe`は過去taskの起動中processが使用しているため保持した。
  通常F5のDebug/Release設定はそれぞれ従来のtargetへ向き、この候補へ自動では切り替わらない。
- Release native試験は345成功、0失敗、11明示GUI専用ignored。Release all-target strict Clippyとfmt確認成功。
- Release lib生結果は181成功・1失敗。未変更のprocess fixtureでready event後のstdout marker確認が失敗し、
  同じRelease・同じoracleのexact隔離試験は成功した。元commandを全面PASSへ書き換えない。
- properties core15、local preview core13、code language core3の対象確認を採用する。
  最後のD入力継続修正はnative property15と関連12件（計27件）、strict Clippy、整形確認が成功した。
  続くRelease起動で発見した空tab時のアクセスを修正し、起動3件・property16件が成功した。
  失敗候補`b7b44a24…`と起動panicの原記録は別に保持する。
  frameや見た目・実IMEの確認とは別である。build receiptの`test/fmt/clippy=NOT_RUN`はそのbuild呼出し内の有無で、別logの結果を表さない。

### 当時の契約と受入範囲

| ID | 当時の実装・機械確認 | 当時の候補での実操作・実画面 |
|---|---|---|
| A | UI見直し一覧、不要な常設説明整理、必要な状態と理由保持、tooltip登録ABI修正 | 受入中 |
| B | 表示領域geometryとdrawerに基づくicon状態。保存意図・所属・幅とは分離 | 4状態の領域・保存配置と狭幅を観測。ボタンの古いON線が残る必須修正を実画像で発見し、修正中 |
| C1 | 認識済み区切りの物理行所有と背景。LF/CRLF・入力/Undo/再入力・原文位置の対象試験 | コード終端のSource/Live caret・pointerと背景を確認。再入力の履歴、YAML等は受入中 |
| C2 | 別header行の言語menu、原文のtoken限定transaction、既知/未知/未指定と履歴 | `982cb322…`の対象実操作・header/menu画像を確認。最後のD変更とは影響範囲を分けて採用 |
| D | source範囲を持つtyped投影、native EDIT一時入力、IME/保存/切替guard、Document履歴、fallback | `801058ce…`の起動・key変更/Tab・title確定・次文字のnative欄内保持・Escを確認。残項目は受入中 |
| E | bounded worker/cache、local文書/fragment/未保存snapshot/画像、Unicodeとroot/ancestor junctionの拒否試験 | 受入中 |
| F | 引用線3.5 DIP、本文font計測の追加gutter間隔、拡大日付と太字曜日、祝日詳細、別位置のDaily印 | 引用・表・画像・3桁gutterとCalendarの実画像、単/二重clickを確認。Dailyからの復帰は受入中 |
| G | 共通geometry、grab offset、track/capture/cancel、native range、横EDITのpixel/column変換 | Editorの押下・drag・track・端・capture loss・resize・短文を実操作で確認。native controlは受入中 |
| R | 現在sourceに対する上記Release regressionと対象試験。source/dirty/保存/Undoの保持 | 受入中 |

追加infoのあるcodeから言語だけを消すと、残りの先頭tokenが言語へ変わる。
この組合せの「未指定」は理由付きで無効にし、原文編集へ案内する。追加infoを削除したり空labelだけを保持したりしない。
追加infoのない通常の言語削除は対応する。

当時の独立Sol High reviewerはread-onlyでtransaction、range、Trust、capture/単位、assertionを確認した。
indentless YAML listの所有、Unicode path、root/ancestor reparse、横EDIT単位を修正し、対象確認を行った。
実画面で見つかった通常property focus時のraw YAML重複は、form所有範囲をcaret fallbackから除いて修正した。
修正前失敗・修正後成功の同一対象試験とLF/CRLF/Source確認を保持する。Ponytail自己点検とは別のreviewである。
native EDITで表現できないdecoded control文字は、完全な対象範囲を原文fallbackへ戻す。
Enter/Esc/保存後の親windowへの入力落下は、文書に結び付けた欄の所有とpristine再開で修正した。
当時の独立reviewは、その候補の修正とclipboard helper範囲を再確認し、別のPonytail complexity reviewも指摘なしだった。
この候補限定評価は現在のOS clipboard original-content recovery/restoreの安全性証明ではない。

### 当時の実画面採取状態

旧候補`33fbe39e…`ではkey変更・Tab・保持値の正常Enter確定と保存を限定確認したが、
raw重複修正後の本候補のGUI合格へ読み替えない。失敗したharnessの原記録は保持する。
旧候補のcaseはpaint admissionで停止した。原記録は保持し、A〜G/Rの完了へ読み替えない。
診断messageがMainを更新することと、ATOKの所有浮動paletteがYAML入口へ重なることをread-onlyで確認した。
paletteの下のpixelsを製品描画の合格/不具合と断定しない。task専用のpassive採取はsource・新しい完成frame・identityを保持し、
既知の所有ATOK paletteとの交差を明示する。その他の重なりは拒否する。
`982cb322…`のC2は実際の言語変更・取消・通常の指定削除・Source同期・UndoRedo・keyboard・狭幅を確認した。
Rootは専用headerとmenuの実画像を確認した。このFront Matterを持たないC2 fixtureは最後のD変更の影響範囲外として採用する。
decoded-control fallbackの実表示も同候補で確認した。D入力継続修正後の本候補と、未実行の独立sectionは受入中である。
物理手入力、別IME/別DPI、本人の主観的受入は別途pendingとする。

### Historical: de083候補の追加確認（2026年10月24日 JST）

`de083444…`はB/Aの描画・必要なhelp保持/配置と、EのEsc抑止を統合したRelease候補。
fmt、all-target check、warnings禁止のall-target Clippy、登録Release buildが成功した。
Bのpublisher/実toggle/idle native検査とA/Shell/Menuの8対象filter、preview core13とApp3が成功した。
新しい広いsuiteは繰り返していない。過去のFAILとその後の原因・対象確認は別の記録として保持する。

- B：この候補の4表示状態・狭幅の左右assigned drawerを実操作/実画像で確認。両方非表示の古いON線は消え、表示とstripeが一致した。配置・幅・所属と保護fixture bytesを保持し、元配置へ戻して通常終了exit0。
- E：対象の表示完了後に一度Escを送り、診断要求なしの静止観測で1000ms以上非表示が継続。実際のpointer移動で同じlinkへ再hoverでき、離脱で閉じた。原文・選択・scroll・focus・履歴・disk不変。
- C1：短い66byte、7CRLF、末尾改行なしのfixtureで、YAML終端と次の本文、code終端のLeft/Home/End/pointerを確認。Ctrl+Endの既存zero-extent EOF viewは別のaffinity位置として記録し、終端行へのアクセスと混同しない。previewなしのnative欄X→Esc取消も成功。
- D：`801058ce…`のURL/長いdescriptionの専用欄編集、保存、通常File close、再openとSource/Liveでbytes一致。実ATOKの日本語確定・取消・UndoRedoと制御文字fallbackを別scopeで確認。実OS clipboard貼付けは専用caseを準備中で、まだPASSではない。
- A：このcheckpointのNew重複help省略・安全条件や意味のある説明の保持はsourceと対象検査で確認した。PanelMove help実画面はこの候補の操作では未確認で、原因も未確定だった。後の最終候補実画面結果は上段に記録する。

B/E/C1の古いharness失敗は書き換えていない。caption overlapは所有B windowを通常終了して解消した。
Ctrl+Wのdelivery/routeはUNKNOWNで、File menu closeが実際に動いた結果と分けて記録する。
このcheckpoint時点でOS clipboard操作は行っていなかった。A実画面と実貼付けの未確認が残り、Git統合・配送・Task完了はまだ行っていなかった。

## 描画負荷・Live編集修正（2026年10月22日 JST、今回範囲のMachine確認済み）

Task `20261022-mdlite-rendering-live-editing-repair` Revision 1。開始developは
`ff0328b8d42531aaf788f0e25ef3c93259a27c80`。以下の旧Rev節は各固定時点の履歴として保持する。
Git統合・配送のOIDとHANDOFF保存結果は最終HANDOFFを正本とする。正式Release・全製品・Human受入ではない。

### 最終対象と原因修正

登録Release buildのsource aggregateは`c49de271395fc2eaeade31725196d7a4f1446870459a6e8e139345beade82377`、
EXE SHA-256は`196b4b827becc9decc7f8c7ab93139bffb01cea5194dbaf064935441cca73f00`。
対象は`D:/codex_work/MDLite/.codex-runtime/rendering-live-editing-repair/target/release/mdlite.exe`、
Rust1.99.1、VM/QEMU Q35、16logical processors、96 DPI、通常client1084×761。
最終source/EXEは登録build receiptと一致した。通常`target/release/mdlite.exe`の既存PID19464は未変更で、
そのEXE SHA-256は`81b30d4669aecca7166b8b378d71e4e0f788800a6040967ce6c7933054f23be6`。
通常起動先を検証版へ自動置換したという意味ではない。

- SoftBreakを空白にする表示を改め、Liveでも明示改行を保持。HardBreakのCRLF対応も維持する。
- Front Matter/fenceはcaretの原文区切り行だけを編集表示する。parser本文spanで未閉鎖code内の偽区切りを保ち、
  再構成後の境界移動/削除、wrap高さ、Front Matter終端mappingと実編集領域に基づくcaret判定を修正した。
- 同一EXEのstyle比較でMainの`WS_EX_COMPOSITED`をpaint反復の原因に分離し、同じ方式のCompactからも除去した。
  同一Settings HWNDの比較では親eraseが子controlを上書きすることを確認し、`WS_CLIPCHILDREN`で除外した。
- Settingsの借用中もtheme/brush snapshotで色・erase・owner-drawを処理する。対象/有効値/適用元/下書きと解除を明示した。
- statusを項目別に配置し、既存Git cacheの未確認/Trust/branch/変更/remote有無を区別する。
  対象tooltip登録のclassic ABI sizeを修正し、狭幅で省略した情報を含む全文popupを実確認した。
  新しいlibrary、bufferの重ね増し、固定FPS、強制paint、manifest/framework変更は採用していない。

### 今回の受入と有効範囲

| 対象 | 採用した確認 |
|---|---|
| A/B | LF/CRLFの2行とDown先空行、原文/revision/dirty/履歴不変、Source切替/保存bytes、区切り解除→Undo→終端再編集をb644候補で確認。関連mapping code不変の最終196候補でもMain最上段glyph・2行移動・Compact最上段/入力/Undo/Main復帰を確認。 |
| C | 同一EXEのstyle比較で原因分離、最終196の診断on/off・Down/操作後・最小化/復帰・button・Settings/Main非focusをquiet条件で比較。 |
| D | 最終196のdark/light各4burstで開閉途中も文字/controlを保持。false/明示空Apply読戻し、draft Cancel/reset Cancel hash保持、Workspace指定解除Apply、有効値/適用元表示を確認。 |
| E | Workspaceなし/Trust未確認と独立fieldの通常/狭幅表示、TEMP内の実local Git root・acceptance(unborn)・remotes[]を照合。実hoverで所有popup全文を確認し、encoding/EOL/mode/保存/外部変更を含む。 |
| R | 変更範囲のnative/source/selection/IME境界/Undo/保存/caret/scroll/resize/復帰をfocused testsと実画面で確認。物理IME・他DPI/端末は別記未確認。 |

独立Sol High reviewerはP1（偽fence終端）/P2（Front Matter余分な行/終端map）を指摘し、修正後の
assertionとsourceを再確認した。後続のSettings clipping/tooltip ABI・buffer寿命も同reviewerが静的clear。
reviewer自身の試験実行はない。Ponytail complexity self-checkは別に実施した。

### 負荷比較

単位はprocess CPU時間÷経過時間の1コア換算。通常約4秒、小さな同一合成文書、記録撮影/buildと分離。

| 条件 | 修正前 | 最終196 |
|---|---:|---:|
| 診断on：idle | 83.91% | 5.84% |
| 診断on：10 Down / 操作後idle | 81.07% / 80.83% | 8.12% / 2.73% |
| 診断on：最小化 / 復帰後idle | 3.90% / 75.74% | 3.51% / 2.73% |
| Source button / 操作後 | 72.25% / 81.57% | 4.22% / 1.56% |
| Settings作成＋4秒 / 閉じた後 | 55.29% / 75.98% | 16.83% / 6.92% |
| 診断off：idle / 10 Down / 操作後idle | 69.77% / 69.21% / 76.15% | 7.02% / 5.44% / 0.78% |
| 診断off：最小化 / 復帰後idle | 3.89% / 75.71% | 2.73% / 3.12% |

Settings openは作成を含む7347ms→7149msの区間で、純粋な4秒idle値ではない。
診断onのpaint増分は5区間で67/65/74/1/68→2/2/1/1/1。診断読取り自体がrefreshを要求するため、
残る境界frameを自然な連続描画と混同しない。`layout_count`はview数であり、14で安定してもrebuild数ではない。
実traceのlayout.rebuildは起動/Down/復帰時に14件、補助runは起動/button時3件で2.31秒以降25.09秒まで増加なし。
今回のサンプルで不要な自律paint反復を解消した。全frame・長時間・全hardwareの保証ではない。

GPU17 engine別の最終endpoint値は取得可でmax0だったが、継続GPU総量やmain PC改善の証明にはしない。
DWM CPUはnull propertiesによりUNAVAILABLE。旧empty sum由来0は測定0として採用しない。
診断offはdisk不変のみ確認し、memory/historyはNOT_OBSERVED。ホスト側負荷は未知。

### 自動確認の扱いと残る境界

fmt、明示all-target Release check、strict Clippy、登録Release build、diff-checkは成功。
Build receiptのtest/fmt/clippy欄はそのbuildコマンド内の実行有無であり、別logの成功を示す欄ではない。
projection22、native focused20、Settings clip実DC1件とsettings group34件、production tooltip1件を確認した。
新しいvisible clip試験は実GUI leaseで成功後、default並列では実行しないignore属性だけを追加した。
広いRelease commandの生結果はlib154pass、native319pass/6fail/10explicitGUIignored。
同じsource/oracleのexact6をfresh processで実行すると全件成功し、foreground/wheel共有fixture干渉に分類した。
元command全体をPASSと書き換えず、不要な広い再実行もしない。最終の限定followupは影響範囲だけ実行した。

GitUI/menu_styleの未変更tooltip size不整合は今回のstatus/Settings対象外としてdeferした。
物理IME、別DPI/端末、持続GPU総量、本人の最終主観的受入は未確認。
private入力・原証拠・合成fixture・確認用EXEはGitへ追跡せず、Project内HANDOFFへまとめる。

## Rev2方向修正のcheckpoint（2026年10月19日 JST、未完了）

製品source local commit: `7847b04688b57a9216a9c402d7ef88fac65f6cd5`。
source aggregate `c2a286a481d677392fc2ad0d6e0c86a534a5a70683d7a3516b4449f8a0d83eec`、
Release EXE SHA256 `8549142476651093435fa23831520ce480b2db448f1b1de211f7c84bbbd0e14d`。
登録GUI harness SHA256 `1028285e433d70328b9029ecdd47a8dca30462e91260f48375a77a731af8f416`。
source／compile-time資材・fixture・toolsの83ファイルをbyte hash付きsnapshotへ保存した。
Gitの改行正規化後blobとコンパイラ入力bytesを区別する。

| 確認 | 結果・境界 |
|---|---|
| Debug通常 | LIB146、BIN278 PASS。LIBは同一SHA ddd42697...の141成功＋失敗5件の成功。BIN ignored5。 |
| Release通常 | LIB146、BIN278 PASS、BIN ignored5。 |
| 排他visible fixture | 各構成4件PASS。editor reentry、palette focus、EDIT mutation、Settings exposure。exposure各31段階。実製品受入ではない。 |
| fmt／strict Clippy／build | fmt、Debug／Release all-targets -D warnings、登録Release build PASS。build JSONのtest等NOT_RUNはbuild単体の範囲で、試験receiptは別。 |
| 独立correctness | Root／実装者と別Sol High。狭幅drawer復帰のsole P2を修正し、後続の配色所有ガード・機械的修正・harness適合も同reviewerが限定確認。指摘解消済み。 |
| 最終製品GUI | PID23696、実96 DPI。通常前面化1回が拒否されNO INPUT／NO CAPTURE。本文hash／dirty／revision／Undo／Redo／保存bytes不変、通常close exit0。 |
| 必須実GUI／150 DPI／物理IME／Human | UNVERIFIED。準備・fixture PASSを受入へ読み替えない。 |

runnerのGit PATH不足と、repo内TEMPによる親repo検出を分離して修正し、製品のGit期待値を変えていない。
最初のDebug BINは277成功／配色所有1失敗。foreign HWNDへのCompact配色を所有範囲で拒否し、
既存focused回帰と最終全BINを通した。初期失敗と旧sourceの証拠を最終成功に混ぜない。
foreground拒否後は再activation、入力、撮影、利用者への追加前面化／DPI操作要求を行っていない。

状態はBLOCKED／INCOMPLETE_CHECKPOINT。develop統合・push・GitBook merge／公開・Inbox cleanupなし。
W01／W03、E08、高度なpin／unpin全体は既存残件として保持する。
以降は各固定commit当時の履歴で、この最終EXEの証明ではない。

## Rev1 Live移動・UI・Git候補の履歴（2026年10月19日 JST、未完了）

### 最終修正後の結果（未完了checkpoint）

製品sourceのlocal保存commit: `4702012a7daf1bfe08ea7aed8e874d6778031b0f`。developへは未統合。

source aggregate `a204bc7fe5922515967895869c2772fefc03ea42dc4ea592d08e108c7094f5d0`、
最終Release EXE SHA256 `7dfdaccac11c965e86c20dcdb9103b2bd9058001788fe65ce3e7b42a05645267`。
Debug／Release BIN各269件（ignored5件）PASS。LIB各141件は再compile後の同一EXE hashを照合して既存結果を再利用。
排他visibleは各構成4件：editor reentry、palette focus、EDIT mutation、EDIT exposure。Debugの後2件は同一BIN hashの直前結果を再利用。
exposureは各31段階。fmt、Debug／Release strict all-targets Clippy、登録済みRelease build PASS。
同じ独立Sol High reviewerの追加修正限定reviewは指摘なし。Ponytail確認とは別。

この最終EXEの製品GUIは96 DPIで起動したが、1回の通常前面化をWindowsが拒否したため入力・撮影なし。
対象PID24400のsource／履歴／disk不変を読戻し、通常終了exit0を確認した。利用者へ追加操作を依頼していない。
従って修正後の製品Settings railのpixel確認、Trust→init等のGit GUI代表、150 DPI、全必須GUIは未確認。
直前の`3ac8b609...`／source `fbacf547...`での上下13steps、四つの設定閉鎖、外側thumb drag、light Apply、Source／Live画像は履歴に限定し、新EXEの成功証拠へ流用しない。
初期化promptは旧runの所有dialogを通常処理済み。最終runは状態folderを用意して起動し、promptで停止していない。

Task12未達のためdevelop統合・pushは行わず、Inboxと未統合作業branchを保持する。
GitBookは候補明記のdraft CR6のみを保存し、現行公開revisionは変更していない。


### 同日machine確認の継続記録

hidden native確認でLISTBOX V6、Explorer H6、共通settings rail 3件、runtime snapshot 2件がPASS。
publication order修正後のruntime snapshot 2件と、module order修正後のformatterもPASS。
全i32／H範囲、既知suffix pixel >65535、native font units 8/7/13、独立V wheel／key／geometry／HC／lifecycleを確認。
`apply_source_styles`のUTF-16 prefix再走査を既存crop metricに置換し、対象外kindをskipした。
native DWrite Unicode／clipped styleは1件PASS。

診断Debug source aggregate `97de2d6e094dfc47ce63b5c779afacad50a478523d7d3c20f71e81d2f1bef136`、
EXE SHA256 `9d570249ca18ba9f5557b49bbb45bd6ea4b86e1caaa10c0e7a053760597fee16`。
100,000行観測はready 3691ms、gutter 100001、59.4375 DIP、通常close exit 0。
別の自然paint観測は59.9ms、frame finish 5.6ms、paint total 299.8msでpending phaseなし。
任意WaitForPaint observerは失敗し、証拠に採用しない。

EDITの共通factory接続10欄とDiagnostics出力1欄を追加し、統合focused 12件PASS。
native style・入力・selection・Undoを維持し、native nonclient部位への操作委譲と描画領域の除外を行う。
独立reviewで見つかったnative文字変更／selection／scroll後の同期漏れを修正。

Settings実画面で見つかったExplorerテーマ適用時のnative EDIT rail消失を追加修正した。
原因はregion除外中のSetWindowThemeがmessage dispatchの外でもnonclient geometryを更新すること。
既存API呼出し全体で元regionを保持し、完了後にnative geometryを再取得する最小helperへ変更した。
Debug重点6件（hidden4・visible mutation・exposure31段階）PASS、同じ独立reviewerの限定再確認で追加指摘なし。
source aggregate `a204bc7fe5922515967895869c2772fefc03ea42dc4ea592d08e108c7094f5d0`。
最終Release実画面はこの修正後に改めて確認するまで未確認として扱う。

専用NOACTIVATE visible試験で実WM_PAINT／native geometry／本文・selection・Undo同値を確認し、指摘は解消済み。
専用GUI比較ではV69989、移動後取消・capture解除を確認。製品pilot ReleaseのGit DiffではH5174／V71、
文書・履歴保持、通常closeを確認した。LFのDiffは表示コピーだけCRLFへ変換し、元stdout不変の回帰1件PASS。

これらは診断Debug／pilotの限定結果であり、最終Release、50MB、全GUI／Human受入ではない。
正式UiParityはmetadataのみPASS、Save As dialogの制限時間内検出でBLOCKED、後続の全件受入は未成立。
150 DPI、physical HC／IME、UIA visibilityは未証明。最終machine／GUI確認とHANDOFFは進行中。

Task `20261018-mdlite-live-navigation-ui-git-polish` Revision 1の作業branch
`fix/live-navigation-ui-git-polish-20261018`。全native scrollbarのstyle統一と
必須GUI条件が未達のため、developへ統合・pushしない。Task Inboxは保持する。
GUI利用の予約は解除され作業を再開した。以降、利用者への追加手動操作依頼は行わない。

保存済み旧checkpointのReleaseはRust 1.99.1、source集約
`65cd3cc9b935a01e0de500cef88523a8ec87ce781ec0be534fd92271a32aa312`、EXE SHA256
`81b30d4669aecca7166b8b378d71e4e0f788800a6040967ce6c7933054f23be6`。
compile-time入力20件と正式GUI harnessのhashはlocal HANDOFFで対応付ける。

| 検証 | 実結果・境界 |
|---|---|
| Debug通常 | lib141＋bin239 PASS。libは同一test EXE hashの既存141件receiptを再利用。binのexit0・hash・件数は記録、child PIDだけ記録漏れとしてUNKNOWN。 |
| Release通常 | lib141＋bin239 PASS、両test process exit0。 |
| 排他visible | 既存NOACTIVATE 2件をDebug／Release各一processずつ実行、全PASS。SendInputなし。実製品GUIの代替ではない。 |
| fmt／Clippy | fmt、Debug／Releaseのstrict all-targets Clippy PASS。 |
| 独立review | 実装者と別のSol Highがread-onlyで確認。表の列位置とharnessの表示／toggle前提を修正後、追加指摘なし。RootのPonytail確認とは別。 |
| 最終EXE・96 DPI dark | 問題fixtureの通常上下で先頭→末尾→先頭、13 steps・循環なし。Source／Live行番号、上部bar、1候補paletteを実画面で確認。設定のCancel／Esc／Alt+F4／×閉じ後に次の編集操作を確認。 |
| 設定scroll | 実resize、capture確認付きthumb drag、top／middle／bottomを観測。EDITへのnative WM_MOUSEWHEEL注入でもdraft18と選択を維持。物理wheel／touchpadの証拠ではない。draft破棄で設定file不変。 |
| 最終EXE・96 DPI light | 合成LocalAppDataだけへthemeをApplyし、通知を閉じてmainへ復帰。実画面と本文・履歴不変を確認。Calendarの土日・祝日・前後月を代表画面で確認。全custom色matrixではない。 |
| 100,000行runtime | 100,000内容行＋EOF行の起動観測は60秒以内にsemantic layoutへ到達せず。その後process不在、exit code／原因UNKNOWN。fixture file不変、入力・captureなし。helperの6桁／行数試験をruntime PASSへ読み替えない。 |

検証の隔離AppData内で、fixture compilerの`rustc.exe`がmise shimを経由して
ツールcacheを展開した。後続の排他試験は既存compilerの実binをchild PATHの先頭へ固定した。
Cargo／miseの親環境とglobal設定は変更していない。これは試験実行環境の記録であり、製品失敗の断定ではない。

残存標準LISTBOX／EDIT等の全axis style、Calendar次月buttonのicon化、実tooltip／pointer、
Compact、Git GUI matrix、実150 DPI、physical／ATOK／Microsoft IME・再変換とHuman受入は未完了または未確認。
OS menu／dialog fontの全条件も未確認。既知W01／W03、E08、pin／unpinを解消済みにしない。
設定Apply通知の実OK buttonはID2であり、正式harnessの固定ID1前提は後続修正が必要。suite全体は未実行。
観測手順の早期focus guardやAPI識別失敗はlocal記録に保持し、未観測の入力を成功にしない。
GUI停止時のPID22580はユーザーが使用し得るため、その後のclean状態や所有を推定して閉じない。

## 文字重なり修正と実画面受入（Rev3）

Task `20261017-mdlite-ui-surface-recovery` Revision 3は、作業branchの
`f6354c8db7eb362c262c063c1db0d726c38ddc5e`から継続した。
基準Releaseの同一processでパレット／Git hostのcaptionだけを消すと文字片が消え、
戻すと再発した。名前付きSTATIC hostを既存テーマ色のnative owner-drawにし、
見出し・通知・子部品・focus／IME経路とhost名を維持した。
Search入力欄下の「検索」は明示ラベル5620であり、同じ不具合ではなかった。

Compactからのパレットにも描画を配送する局所回帰を追加した。
別Agentのread-only correctness reviewで見つかったCompact配送漏れは修正し、
最終差分の再確認に追加指摘はない。実装者のPonytail確認とは別のreviewである。
Compactの既存不具合も基準EXEで再現した。子領域clipとパレット子部品の色通知を
直接の関連範囲として修正したが、その時の初回／再表示の採取では見出し・非選択行の文字が
欠けた。局所描画試験のPASSを実画面合格にせず、この修正手法の反復を止めた。
後続の追加refreshを送らない通常操作の観測は、下記run07として区別する。

固定Release EXEは`24719c6a1b5dedf793e56a212e607fe6fb2ed53b304a3c40da7d7733a98053c9`、
source集約は`ded5523f1f338c2ef78b671166f07c8fabe17fbc42f72074609092cb2e403459`。
実画面は96 DPI、主画面1086×793／client1070×734、Compact800×700／client784×661。
元添付画像の実行EXEは未確認で、同一EXEだったとは断定しない。
build入力資材・機械試験・操作記録・Git結合情報は最終HANDOFFに記録する。

この固定候補でDebug／Release各通常346件＋排他visible 2件、fmt、
両構成のstrict ClippyはPASS。通常試験はRev2の345件に、名前付きcontainerの
main／Compact描画・色・再入・文書不変の回帰1件を加えた。
mise／Cargoは通常環境、試験子processだけ合成AppDataを使い、全試験processの終了を確認した。
Debug EXEは`6d121c62bc32d10bc9d2b1ad2d72b18aa9a44f6174f6a18aa87f80a8ac3bccdf`。
機械試験だけで実画面を合格にせず、以下の同一EXEでの操作証拠を別に確認した。

| Rev3必須項目／対応 | 最終候補の状態 |
|---|---|
| A・文字重なり | PASS（観測した96 DPIの範囲）。主画面の初回／再表示／切替／resizeと、後続10往復の対象領域を確認。Compactも通常の初回／再表示で見出しと複数行が読めた。旧採取の欠落／白化は保持し、全frame・遷移性能の保証にはしない。 |
| B・C01 結果移動 | PASS。10月18日の検証出力付き合成Workspaceで文書4結果から `[313,319)`、Workspace 2ファイル5結果からsecond.mdの `[8,14)` へ実結果選択で移動。全タブの原文・dirty・revision・履歴は基準どおり。 |
| C・C03 10往復／60秒以上 | PASS（後続run06の観測範囲）。各切替先clientを確認する10往復・20実表示切替のnav送信区間は432.990秒。query再入力の準備操作も含む観測窓は561.880秒で、両scope・詳細／置換開閉と全6タブ照合を記録。旧2往復と855.2秒の内部状態上10往復は別履歴。 |
| D・C02／C04 パレット／window | 主画面96 DPIでExplorer／Search背面、保存実行、無効理由、Esc・外側クリック、開閉両状態のresize・最小化復帰、狭幅drawerを確認。追加のCompact初回／再表示も後続の受動観測で読めた。 |
| E・C05 入力／実IME／編集保存 | 必須代表を確認。Unicode初回入力5新規起動、編集→Undo→Redo→通常保存→再読込、ATOKとMicrosoft IMEの検索／パレット／本文の代表的実入力を確認。全matrix・物理入力・Human受入へ拡張しない。 |

最初の主画面追加採取では、helper import不足を修正後、入力helperが前面化前の
foreground条件で送信前に停止した。この方法の反復を止め、既存Orcaのrestoreと
window操作に切り替えたが、対象のnative foregroundを確認できなかった。
Orcaの操作は未検証のsynthetic結果で、query／control入力は送っていない。
別windowに覆われた画像を製品画面の合格や配送用画像にせず、raw記録はlocalだけに保持する。
所有PIDは通常終了し、fixture／合成領域の復元・未変更を記録した。
これは製品の検索・IME・保存FAILを示す試験ではなく、実行前のGUI採取blockerである。

10月18日、本人から前面化への操作干渉の可能性と再試行の指示があり、同じ最終EXEで再開した。
前面化は成立し、NoWorkspaceの文書検索結果移動と、通常のWorkspace起動から別文書への
実結果移動を確認した。ただし通常起動の子windowは検証用出力を引き継がず、そのwindowの
未保存タブに対し、保全指示に反して「保存しない」が送信された。画面は空表示だったが、
厳密な原文・revision・履歴は未取得なので、このcaseの本文不変・無損失は合格にしない。

既存の`--diagnostic-path`で同じ合成Workspaceを起動した新しいcaseでは、入力前に全3タブを
取得した。editor.mdとsecond.mdはファイル一致・保存済み・revision／履歴0、無題タブは
厳密に空の原文・dirty=true・revision／履歴0だった。この基準に対し文書とWorkspaceの実結果を
UIから選び直し、UTF-8 byte offsetと原文・dirty・revision・Undo／Redoを確認した。
移動によるactive document／選択の意図した変更を、表示操作の不変条件と混同しない。
後者の新caseを、先に閉じたwindowの未取得状態の代わりにはしない。

Cでは、対象詳細がWorkspace scope限定であることと、詳細表示中はquery／置換欄が
隠れる条件を確認して別の有効な操作手順を一度試した。追加区間では置換欄を閉じる
操作後の条件が成立せず、追加完了往復は0だった。置換文字・置換実行は送っていない。
閉じる操作の原因は未確定であり、10往復・20切替・60秒の受入を満たさない。
表示操作後の全タブ原文・dirty・revision・履歴は基準どおりで、本文focusのための
クリックによる選択位置変更は意図した操作として別に記録した。
新caseの厳密に空だった無題タブは新規ファイルへ通常保存し、0 bytes／空hashを照合した。
全タブ保存済みのWorkspace windowを通常終了し、NoWorkspace windowを元の寸法へ復元した。

その後、Eで「操作直後の診断は未変化でも後続で完了した」という新観測を得た。
別担当の限定確認では、既存click helperは入力数確認後100ms待つだけで、置換欄の可視／focusの
完了を待たないことが分かった。旧Cの148ms単発判定とは異なる検証として、表示・focusの
完了条件を待つ一度の開閉を別caseで確認した。各1クリックで開174ms／閉149msに完了し、
原文・選択・dirty・revision・履歴は不変だった。これは旧失敗原因の確定でも10往復の合格でもない。

この新しい完了観測方式で別の合成Workspace caseを進め、nav送信20回と内部状態上の10往復を
855.2秒の区間に記録した。文書／Workspace検索、query再入力、対象詳細／置換の代表開閉と
操作中のframe採取を含む。補助観測を早く終了したround3は、後続の実Explorer表示を確認した
時点で到達を記録した。Taskに5秒性能条件はなく、5秒以内の完了はUNKNOWNのまま保持した。
しかし中盤／最終のSearch frameにはExplorerのような古い領域が残り、最終切替frameには
Git欄の白い矩形があった。後の最終Explorer frameは正常だったが、代表frameの不一致を
打ち消さない。送信数・内部stateの一致を実画面C03の合格へ変換しない。
active文書の原文・選択・dirty・revision・履歴は不変で、入力前は全4タブの基準を取得した。
終端の他タブ再訪はExplorer focusからのCtrl+Tabが進まず一部未確認である。
これらの新しい描画事実は、既存のclip／色修正の反復とは別の原因確認へ渡した。

別Agentの描画経路確認では、診断refresh後の内部状態更新と実際のWM_PAINTは別であり、
短い間隔のread-backは画面完了を保証しなかった。追加の1切替では、クリック後74.536msの
client画像はExplorerのまま、49.342秒後の画像はSearchのquery・結果を表示していた。
間の完了時刻は未観測で、最初の5秒以内の完了や遅延原因は確定しない。
即時画像をSearchとした担当の記述は、Rootと別Agentの画像照合で訂正した。
後半5枚のSearch画像は同一hashだった。この結果だけで持続する描画欠落や修正必要性は
確定せず、先の10往復を実画面合格へ昇格させるものでもない。

その後は各切替先のclient画像を実際に確認してから次のnavを送る方法で、同じ最終EXEの
run06に10往復・20回の実表示切替を記録した。query再入力は準備中のUTC18:59:54頃に一度だけ
実施し、18:59:56.830の画像で「編集」を確認した。19:01:05.598に実Explorer基準画面を確認後、
19:01:23.690〜19:08:36.679のnav送信区間432.990秒で、query保持・両scope・対象詳細／置換欄の
代表開閉を確認した。この区間内に再入力を繰り返したとは扱わない。
再入力前の基準画像18:59:16.532から最後のExplorer画像19:08:38.412までの観測窓は561.880秒。
これは連続した入力の実働時間ではない。
数えた全切替先が読める状態で、本文の置換は実行していない。遷移の性能保証や未撮影の全frameの
保証ではない。終了時は実本文focusから全6タブを再訪し、原文・clean・revision／履歴を照合した。

主画面のパレットでは、Search背面で「保存」を絞り込んで実行し、保存完了と閉鎖を確認した。
Workspace未指定の復旧候補には無効理由が表示された。Esc後は検索欄へfocusが戻り、
次のRight入力で本文は変わらなかった。外側クリックで閉じた場合も本文・選択・履歴に
クリックの通り抜けはなかった。Explorer背面からも候補入力・無効理由・Esc後のnav復帰を確認した。
パレット開閉それぞれでresizeと最小化復帰、770×650の狭幅Search drawerを確認し、
元の1100×800へ戻した。実行は96 DPIに限り、他DPIの合格にはしない。
配送する追加画面はtarget client内のcaptureを使い、他windowが端に写るraw画像はlocalだけに保持する。

Eの初回入力は同じ最終EXEの5新規processで、ExplorerからSearchを開いてquery欄5602へ
Unicodeの「Markdown」を一括入力した。queryとcaret 8/8、原文hash・文書選択・dirty・revision・
Undo／Redoの不変を各回で確認し、通常終了した。旧EXEの5回合格を転記していない。
別の合成本文ではMicrosoft IMEの通常key入力から候補「日本語」を選んで確定した。
Undoで元本文へ、Redoで確定後本文へ戻り、処理完了後の原文・選択・履歴を確認した。
操作直後の診断が未変化でも後続状態が変わったため、短い固定待ちだけで失敗とはしない。
明示したSave前にもdisk一致・cleanへの変化を観測したが、その保存原因は未特定である。
その後Fileの通常Saveを実行し、61 bytesのUTF-8・BOMなし・LF 2／CRLF 0を本文hashと照合した。
通常パレットの「ディスクから再読込み」と所有確認ダイアログの「はい」で再読込し、
保存本文一致・clean・選択0/0・revision／Undo／Redo 0への初期化を確認した。

実IMEはATOKの検索欄で通常keyによる候補表示・取消・「日本語」確定と、パレットで
composition中の最初のEscによるIME取消、次のEscによるパレット閉鎖・検索欄focus復帰・次入力を確認した。
Microsoft IMEは検索欄の候補表示・取消とかな確定、パレットの「日本語」変換確定・取消、
本文の取消・「日本語」変換確定を確認した。検索欄のかな確定を漢字確定へ読み替えない。
追加の代表試験では入力前提未成立の試行があり、既知の切替・入力状態確認後のATOK本文でも
preedit／候補が発生せず、原文は変わらなかった。追加受入0の手段反復を止め、
この時点ではATOKパレット確定・本文代表とMicrosoft IME検索欄の明示漢字確定が未確認だった。
入力方式・受信欄・実確定文字・取消／終了優先順はHANDOFFの今回集約に限定する。
追加の限定コード確認では、ATOK本文試験で記録したfocus ID100は本文でなくcommand buttonだった。
profile有効、IME open、composition中は別の状態であり、mainのIMM状態をnative query欄へ転記しない。
この試行を有効な本文変換試験にしたり、preeditなしを製品FAILとしたりしない。

外部PowerShellのIMM読取は別threadのcontextを取得できない場合があり、利用可能IMEの必須判定に
した条件を訂正した（[IMMのthread制約](https://learn.microsoft.com/en-us/windows/win32/intl/developing-ime-aware-multiple-thread-applications)）。本文は所有UI threadの診断と実候補、native Edit欄は実focus・候補表示・
確定後queryで観測する。UnicodeによるLatin文字入力は実IME試験でなくcontrol入力として保持し、
Undo後の原文・revision・redoを記録した。IMEが閉じた状態の通常仮想キー入力も変換合格にはしない。
所有threadのATOK有効・open=trueと本文HWNDへのfocusを確認後、OS合成の通常仮想キーnihongoから
「にほんご」のpreeditとATOK候補を観測し、取消の本文／履歴不変と「日本語」確定を確認した。
focus click後の実基準はUTF-8位置5で、9 bytesの確定後は14。初期位置51を維持したとはしない。
確定本文61 bytesを通常保存後、新規committed-body.mdへSave Asしてhash
`1cfc8885209ab210e84b83370166d76589916a00f02aa7703ee4fb4a0be04460`を照合した。
パレットEdit150でもATOK実候補から日本語を確定し、閉鎖後の有効focus／次入力で本文不変を確認した。
Microsoft IME検索欄では正しいprofile・focus・日本語modeで通常仮想キーとSpaceを送ったが、
観測時には候補・確定queryが見えず、ATOKへ戻した後のqueryは日本語になっていた。
処理時点と確定元を特定できないため、入力欠落のFAILにもMicrosoft IME漢字確定PASSにもせず未確認とする。

後続run06では、Microsoft IME profile・日本語modeとSearch Edit5602のfocusを入力前に確認し、
通常keyのNIHONGOから「にほんご」と候補1「日本語」を実画面で観測した。候補が既に出ていたため
Spaceは送らず、Tabで選択・Enterで確定し、query「日本語」と候補閉鎖を確認した。
composition終了後の診断で本文hash・clean・revision／Undo／Redoは基準どおりだった。
これは先の出所不明なqueryの読み替えではなく、受信欄と操作を特定した別の代表確認である。

Compactの追加判断では、旧採取は毎回WM_APP+1のrefreshとcontrol読取りをpixel撮影前に送り、
同じ表示を静観した後続frameがなかったことを別Agentが確認した。主画面の即時／後続表示の
違いを新たな根拠として、同じEXEで通常Ctrl+Kの後にrefresh・control polling・入力・強制再描画を
加えない一度の観測を行った。run07の初回は直後にはパレットがなく、約1秒後から55秒後まで
見出しと複数の非選択行が読めた。queryは意図どおり空だった。
一度閉じて再表示した場合も後続client画像は読めた。ただし撮影helperがCtrl+K送信後に失敗し、
再表示の即時／1秒画像と正確なkey送信時刻は未取得である。再試行や描画の強制は行っていない。
閉鎖後の主画面も本文が読め、原文hash・clean・revision／履歴0・ATOK closedを保って通常終了した。
旧採取のFAILを消したり、samplingが原因だったと確定したりはせず、通常操作で読めるという
追加の受入証拠として扱う。製品コードはこの再試行中に変更していない。

今回必須A〜Eは上記の観測範囲で確認した。全IME／DPI matrix・物理手入力・全製品Human受入は
別の未確認である。開発線の通常統合・Git配送と最終結合情報は、最終HANDOFFの実際のOIDを正本とする。
W01／W03、E08、pin／unpin、全DPI／IME matrix・物理入力・Human受入は別の残件として保持する。
以下のRev2以前の証拠は、それぞれのbuildに限定した履歴である。

## 検索初回入力・表示復旧継続のcheckpoint（2026-10-17・Rev2未完了）

Task `20261017-mdlite-ui-surface-recovery` Rev2は、下記Rev1の保存commit
`69beb0998981f1b1914e3257e915357544e6dc1b`から継続した。view切替時の先回りした
editorへのfocus移動を削除し、実際に部品を隠す既存経路のfocus／子孫判定を維持した。
queryの選択準備をfocus前に行う順序は維持し、選択messageの抑止や入力の低速化はしていない。
承認された導入済みRust 1.99.1へProject内のpinと厳密なbuild検査を整合した。

一時診断では、文字順が正しいUnicode入力の途中に、`GetMessage`が戻る前の同期経路から
遅い`EM_SETSEL(0,0)`が届くことを確認した。callerにMSCTF／text-input frameworkが含まれるが、
送信元processやIMEの責任を特定した意味ではない。通常key対照、変更前buildとの対照、
実測した検索入口からeditorへの中間移動だけを省く一因子実験を経て、上記局所修正を行った。
一時診断と入口111専用の実験分岐は製品から除去した。

最終候補のsource集約は`18c0fbc137368721a55aa2b89381809846883b40d657bb29111c12b6be130148`、
診断なしRelease EXEは`daf66dbffa432f805a42dfba6649d062e66992e046ab3d85218a9e9b7b4674bd`。
compile-timeのSVG等は補足manifestで照合し、wrapperの集約単独を全資材の固定とはしない。
focused native 2件とfmtはPASS、別担当のread-only correctness reviewに残指摘なし。
最終機械試験はDebug／Release各通常345件と専用可視2件がPASS、失敗0。
fmtと両profileのstrict ClippyもPASS。mise／Cargoは通常環境で実行し、
Cargoが報告したtest EXEだけを子processの合成AppDataで逐次実行した。
最終Debug EXEは`d67154ac05f07b4f1921cdabb7d9909571a11a5d40d454a05c638d2e0059e354`。
検証後のsource／Release EXE／harness一致とprocess不在を確認した。
Git配送・HANDOFFの実績は最終HANDOFFへ結び付ける。

| 行 | Rev2候補で確認した範囲と残件 |
|---|---|
| C01 | PARTIAL。文書内の実結果選択で原文位置へ移動し、source／revision／dirty不変を確認。Searchのqueryと詳細・置換の表示操作を確認したが、Workspace結果選択を混ぜた10往復は未完了。無効な前後移動を強制実行していない。 |
| C02 | Rev1の限定PASSを保持。今回のSearch Escとeditor復帰はパレットの再確認とは別であり、最終候補でのパレット変更経路の代表確認は未完了。 |
| C03 | PARTIAL。静止画面だけの55.58秒記録は操作中の受入から除外。続く2,700 frames／146.40秒の記録は平均54.24ms・最大83ms間隔で、9件の操作を記録したが必要な10往復は未確認。抽出frameのquery・button・captionは読めるが、全区間の白化解消とは扱わない。OS scrollbarの明るい帯は別に分類した。 |
| C04 | Rev1の96 DPI限定PASSを保持。今回のresize補助APIは利用できず、操作を送っていない。最終候補の代表resize／復帰確認は未完了。 |
| C05 | 初回Unicode入力の必須5 fresh processは5/5 PASS。全回で`Markdown`、caret 8、query focus、本文・revision・dirty・選択不変を確認。Ctrl+F、通常key、途中選択の置換・削除も観測した。最終候補の実IME・Undo／Redo・保存再open等を含むC05全体は未完了。 |

query設定の補助操作ではCtrl+Aの全選択を観測できず、shortcutの扱いは未確認として残す。
Home／Shift+Endによる全選択後の入力は、固定150ms後に途中の一文字だけを観測したが、
追加入力なしの後続読取りでは全文に到達していた。到達時間は測れていない。
固定待ちの途中観測を文字欠落と断定せず、既存の意味的な完了待ちを使う。
これらを初回入力5/5や実IME・Human受入の代用にしない。

C01／C03と必須代表確認が残るため、作業branchのcheckpointとして保存し、developへ統合しない。
W01／W03、E08、pin／unpin、物理入力・全IME／DPI matrix・Human受入は独立した残件を維持する。
下記Rev1のFAIL／PARTIALとbuild境界は履歴であり、Rev2候補へ付け替えない。

## 検索・コマンド表示復旧のcheckpoint（2026-10-17・未完了）

Task `20261017-mdlite-ui-surface-recovery` Rev1、基準develop `48555ea840abd4fcb17ba1601eeb4f67ec9ad209`。
作業branchは `fix/ui-surface-recovery-20261017`。Explorer/Searchの排他表示、
パレットの表示・終了とfocus、子部品の色、局所UI整理と恒久回帰を追加した。
操作契約と後続pin案は[製品要件](product-requirements.md)、GitBook向け差分は
[更新差分](gitbook-rust-ui-delta.md)に記録する。
必要受入が未完了のためdevelopへ完成扱いで統合しない。Git配送・HANDOFFの実績は最終HANDOFFを参照する。

固定source集約SHA-256は `d8c52bf010a543b2968d1dd93eadf80cd075ba51092e898ce730216de056efe5`。
Release EXEは `67258c9a9eab2d606ced32a7c2ebf328d2ce4bb4466128fe3bc7067db9a796b7`、
Debug EXEは `7ffe25e3e1b9cc29dcd73e2554c7fa698e41011121eea8875ecaaefaa6890f28`。
Debug／Release各profileでcore 139＋native 205＝通常344件PASS、失敗0。
通常suiteでignoredのnative 2件も、各profileの排他的な別実行でPASSした。
fmt、Debug／明示Releaseのstrict ClippyもPASS。source／Release EXE／canonical harnessの
前後一致と、cargo／rustc／MDLite processの不在を確認した。
Release buildは同じ固定sourceの登録receiptとEXEを再利用し、Debugは登録手順でbuildした。
build receiptの個別test／fmt／clippy欄は別実行のためNOT_RUNで、全体結果は機械検証summaryを正とする。
source集約は登録wrapperの対象（Cargo関連3ファイルとsrc以下）であり、
src外のcompile-time assetsを含む完全な資材pinではない。今回変更したdocsはsrcからの埋込み対象ではない。

独立Sol High correctness reviewの指摘を修正し、最終sourceの局所再確認で残指摘なし。
別Ponytail reviewで提案された常時非表示HWNDの生成・配置・有効化を削り、registryとメニュー経路を保持した。
reviewは静的読取りでありGUI合格ではない。App借用中の色問い合わせ、owner-draw、
scope／詳細のbounds、可視入力へのfocus、置換欄のEN_CHANGE後の連続入力、
外側scrollbarのdismiss、Tab移動とパレット復帰focusにnative回帰を加えた。

以下は合成Workspace・可視所有windowに対するOS合成入力と実IMEの観測で、物理入力・Human受入とは分ける。
配置・色・IMEなど変更されていない部分は前候補の証拠を継承し、最後のfocus復帰変更は最終Releaseで更新した。
各観測のEXE／PID／HWNDと介入はHANDOFFの証拠へ結び付け、途中候補全体を最終buildの全面合格へ転用しない。

| 行 | 状態・確認できた範囲と残件 |
|---|---|
| C01 | PARTIAL。10回のnative commandによる排他切替、query／設定保持、可視windowでのTab／Shift+Tabと次の入力、Explorer親描画の抑止を確認。混合した結果選択ケースは未実施。初回query入力はC05 FAIL。 |
| C02 | PASS（観測範囲）。10回開閉、query／list移動、Esc、無効理由と維持、有効Searchへのfocus、外側NewFile click／子scrollbarのdismiss消費、部品破棄と文書選択保全を確認。ATOK／MS IMEの補完確認で最初のEscが変換取消を優先し、後続Escで閉じることも確認。 |
| C03 | PARTIAL。検索pushbuttonの白化とExplorerタイトルの混在を解消。補正後の160 client frame・9.72秒では大きな白いSearch／button矩形を観測しなかった（平均60.53ms、最大80.61ms間隔）。captionの描き直し途中やOS scrollbarの白いstripは残り、未観測の瞬間・全状態の見た目まで保証しない。 |
| C04 | PASS（96 DPIの観測範囲）。幅1086／1500／700と復帰のboundsを確認。最終Releaseで最小化・最大化復帰後のquery focus、続くxy、Esc後のeditor復帰、保存fixtureの再openと本文不変を確認。他monitor／DPIはNOT_RUN。 |
| C05 | FAIL。最終shipping Releaseでも、前面・query focusを確認した一括Unicode入力MarkdownがwnMarkdoとなった。安定focus後のabcdは正順。本文・revision・dirty・選択・Undo／Redoは不変。限定ATOK／MS IME取消・editorへの次の入力、ATOK日本語確定とUndo／Redo、673-byte保存／再openは別のPASS。 |

初回入力ではcaret準備をfocus前へ移した局所回帰はPASSしたが、実画面の文字順問題は残った。
二つの一時診断でWM_CHAR到着順は正しく、途中の二回の `EM_SETSEL(0,0)` が挿入位置を先頭へ戻すことを確認した。
印付き診断は既知の同期focus helperからの送信を除外したが、送信元・queued／directの区別は未確定。
IME・accessibility・外部processのいずれかを原因と断定しない。入力直前の追加前面化を省く比較でも再現した。
診断sourceは完全に戻し、shipping四ファイルとRelease EXEのhash一致・process不在を確認した。
未知のselection変更を遮断する製品patchは入れず、Taskの未完了時の保存手順として区切る。

正常保存した合成fixtureは673 bytes、SHA-256 `cd41641aeff281e497da279ed0425a0328aac7612659ef563ef38dc7677b8cd4`。
所有windowで元のATOK profile／IME closedを確認。通常CloseMainWindow完了とPID消滅を確認し、
exit codeを取得しなかった記録はUNKNOWNとする。OS clipboardは未操作。
原点計算が誤っていた最初のframe記録は無効・ローカル限定で、HANDOFFへ含めない。

W01/W03の既存FAIL14とE08のFAILを継承し、今回の合格へ読み替えない。
通常起動の新しい因果仮説がないため旧実験を再開せず、表／parserのE08修正も行っていない。
全IME matrix、物理操作感、他DPI、F5準備系の全面修理、左右pin schema／migration／drag dockingは後続。

## 前Taskの起動と入力復旧の残確認結果（2026-10-17）

以下の「現行」は前Taskの固定source/buildを指す。今回変更後の検証結果ではない。

現行source集約SHA-256は `2fb81b7e33a4746aae931504660fa70299539eef04e5ffabc18ad217dc0748f4`。
Releaseは `8ddf5b99a8f34c0e241cd51da95778393e63f65bce9bd40bf28133fa83480807`、
Debugは `81cffeaf9eaa6e0c1200a201cdf6009ed344bfe8de691f4e6d6bccf6a926a792`。
表間の境界に空の入力markerを作る局所修正とLF/CRLF回帰を追加した。
Debug/Release各139 library + 201 native tests、各1件の専用可視試験、fmt、strict ClippyはPASS。
独立した静的correctness reviewに追加の製品コード所見はなく、別のPonytail reviewにも
複雑性の所見はなかった。以下の実操作上の失敗・未確認とは分けて扱う。

ただし、現行Releaseの無介入起動14回は全て親画面とeditorが白く、W01/W03はFAILである。
同条件の旧Releaseとの空文書対照でも両方が白かった。通常タイトルバーactivation直後と
1.6秒後はrefreshなしで正常表示したが、これはW02の復帰確認でありW01の合格に代用しない。
現行sourceに対応する隔離診断版では、背景状態の描画先から取得した画像が正常である一方、
同時期の実画面は白かった。前面化後の描画先画像も同一だったため、文書の描画内容が
生成されていないという説明では足りない。表示への反映経路の原因は未確定である。
その後の通常起動では、診断版2回と製品版の空文書1回が初回描画前に前面化して正常表示した。
この条件差は背景起動の失敗を取り消さない。診断版のGDI反映比較は背景条件を満たさず、
制御呼び出し・GdiFlushとも未実行で通常終了した。
さらに、製品版を起動情報の `SW_SHOWNOACTIVATE` 指定で1回起動したところ、
別processが前面のままでも正常表示し、文書・EXE不変、通常終了code 0を確認した。
背景状態だけでは失敗条件を説明できない。この診断用起動経路をW01の合格には数えない。
2026-10-17のユーザー指示により、非アクティブ時の白画面は未解決のまま次回レビューへ回す。
この問題の追加調査・介入試験は停止し、W01/W03の失敗記録と診断証拠を保持する。
以後に発生した課題も修正せず残し、最終レビュー・Git配送・HANDOFF更新・アップロードへ
進むようユーザーから明示指示を受けた。現行buildの残確認は以下の結果である。

- 連続EditorRecoveryはE01 PASS。E02のASCII/Markdown、ATOK/MS-IMEのCoreとExtra、
  選択・置換・preferred-x・Undo、折返しdrag、task編集に個別操作の証拠を得た。
  E02〜E07の複合行は、未確認項目を含むためNOT_RUNのままとする。
- E08の修正対象である最初の表末尾、表間、次の表の前の入力とUndoは通過した。
  後続の「2番目の表の後の空行」でxとEnterを入力すると、表示表数が2から1となる
  assertionでFAIL。入力sourceと選択位置のassertionは通過しており、製品原因か
  判定条件の問題かは追加調査していない。失敗sourceはBOM付きUTF-8で正確に保存し通常終了した。
  E09以降と同一sessionのI06には到達していない。
- E14の外部変更時の保存拒否では、draft・選択・履歴の保持、通常保存、次のUndoと
  元sourceの保存、通常終了code 0まで通過した。これはE14全体の合格ではない。
- 状態領域を作らないWorkspaceでは、警告と編集継続、source保存、通常終了を確認した。
  実際には自動保存でcleanとなり、予定したdirty-close Cancelと次の編集・Undoは未確認。
- CoreはACK記録変数の未初期化、W02は予約変数PIDへの代入で停止した。I01はRootの
  prompt操作が期限に間に合わず、試験本体は未実行。いずれも合格や製品不具合とは数えない。
- UiParityはSave As関連のBLOCKEDと後続の待機timeoutで停止し、通常復元まで未到達。
  IME中の終了はCONTROL_UNAVAILABLEでBLOCKED。Rootは残ったdialogを通常操作し、
  確認用sourceを保持して終了した。各失敗記録とcleanup記録は区別して保存している。
- F5は準備処理の空pathで停止し、IDE/debugger起動は未実行。PageUp/Down・折返しaffinity、
  表の横scroll、長いIME候補の全bounds等の補完確認も、準備依存の不足により未実行である。
  private I06 helperのchar配列変換に関するP2所見も、未修正の残課題として引き継ぐ。

製品source・EXE・正規harnessは機械試験時から不変である。以上を完全なGUI・物理入力・
Human受入とはしない。配送OIDとremote readback、残課題の詳細は最終HANDOFFに記録する。

### 以前のbuildで得た証拠（現行buildの合格記録とは区別）

旧時点の製品source集約SHA-256は `342092d27c0208031ce4810952cd77858505c7d9013a7b8c0fb82f8f8597bb20`。
Release EXEは `6f0bf15155b7f5d5dad4c671c64f4c03ef76bbddfd87d275992dd6deb84a6ad6`、
Debug EXEは `1f4e70ae4a2e32216f9c089b453f2063b95b399121027e1b294f1272a75d0e12`。
canonical GUI harnessは `c832a4f7de2da3b1df50b22395860edc0d9aa2f864c44fb3ae251b4f63667acc`。
未コミットの検証対象であり、Git配送・Human受入は未完了である。

隔離診断版では、初回のWM_ACTIVATEとWM_SETFOCUS、および成功したEndDrawが、
所有Mainが実際に最前面になる前に発生していた。通常タイトルバー操作後には正の
WM_NCACTIVATEだけが届き、追加の本文描画がなかった。所有Main/Compactの正のactivationで、
最小化中を除き全体描画を非同期予約するよう修正した。既存のdefault処理とfocus処理は維持する。
WM_ACTIVATEだけを扱った中間候補の白画面は失敗記録として保持する。
CheckWindowStateやEndDrawの成功だけで実画面正常を判定していない。

最終Releaseの無介入起動14回をRootが実画像で確認した。ファイル指定10回
（空4回・固定878-byte fixture6回）と設定済みWorkspace指定4回で、診断ON/OFF各7回、
96 DPIのclient全体が正常だった。撮影前の本文click、resize、diagnostic RPC、入力はない。
全14回とも通常終了code 0、fixture不変。別起動でも、操作前・通常タイトルバー操作直後・
1.6秒後のclient全体が正常だった。最小化・最大化・遮蔽からの復帰の最終確認は継続中である。

最終Debug/Releaseのfull testsは、それぞれ139 library + 200 native testsがPASS。
通常suiteでignoredの可視native試験は専用実行で各1件PASSした。
fmtと両profileのstrict ClippyもPASS。検証前後のsource・EXE・harnessのhashを照合した。
この機械検証のharnessラベルは `d15f8c92…`。以後のharness修正ではRust sourceとEXEは変わっておらず、
新harnessの実操作検証は別の継続記録で扱う。
独立correctness reviewと、別のPonytail complexity reviewを実施した。
Core/F5および連続GUI受入は未完了であり、機械試験から合格を推定しない。

source集約はRust sourceとCargo/toolchainの42ファイルを対象とする。
製品へ組み込むSVG5件とtest専用include fixtureは別に照合した。
source ZIPは42ファイル、icons21件、fixtures9件、登録tools2件の計74件をbyte照合した。
これらの対応付けはcommit・配送・実IMEの証明とは別である。

連続EditorRecoveryでは、E01の作成・保存、E02のASCII/Markdown入力、
E03のATOK通常変換・再変換・長いcompositionの機械検査を実行した。
短いATOK candidateの位置と文節表示を画像で確認した。長いcompositionの画像には
小さいIME panelの重なりがあり、その所有と原因は未測定のため長いcandidate配置の合格とはしない。
harness変更前のMicrosoft IME試験は、最初のSpace後に異なる日本語preeditとなる条件で停止した。Enterは送らず、
composition取消と元のATOK closedへの復元を確認した。同じ読みの候補か未処理かは未確定である。
この失敗を残して追加観測したところ、1回目は同じ読み、2回目で「日本語きょうはいめへんか」へ変化した。
Enterは送らず、4回のEscapeとprofile復元後、本文140 bytes・revision・dirty・履歴・選択が不変で、
元のATOK closedへ戻った。client画像でも本文とcandidate表示を確認した。
観測後の集計はevidenceの戻り値が配列へ混入して失敗したため、試験全体のPASSとはしない。
Microsoft IMEだけで最大3回まで変換を観測するharness修正を、独立reviewとオフライン検証後に反映した。
異なるstable Japanese preeditを確認するまでEnterを送らない条件は維持した。
Microsoft IMEの `b328ab19…` harnessでは、2回ずつのSpace変換、候補巡回、取消、Enter確定と一単位のUndo/RedoがPASSした。
再開用の試験行初期化漏れで停止した後も同じprocessを保持し、確定操作を再生せずExtraだけを継続した。
再変換、全文F6ひらがな、異なる文字への確定置換、80文字の読みと長いcompositionの取消・確定・
Undo/Redoを確認し、160-byte本文、U25/R1、元のATOK closedへ戻った。
折返したpreeditと確定後のbaselineを画像で確認した。文節移動のRight側のtarget highlightと、
親windowの撮影範囲を越える長いprediction popup全体の範囲は未検証として残す。

E05では、見出し前の空行からCtrl+Downが進まない製品不具合の修正を最終EXEで確認した。
Live表示の挿入位置補正を物理行へのCtrl上下移動に適用していたことが原因で、
水平移動とtable処理を維持して修正した。LF/CRLF、Live/raw、Shift選択を含む恒久回帰を追加した。
その後preferred-x試験が、残っているpointer fixtureを含む3タブに対し2タブを要求して停止した。
本文140 bytesと履歴を保持し、E05だけ3タブを期待する修正helperで同じprocessから継続した。
上下移動のpreferred-x、短い行での末尾への制限、逆向き選択と6回のUndoがPASSし、
元の124-byte本文、dirty、U24/R6、EOFのcaretとATOK closedへ戻った。
対応する画像でもcaret位置・選択・復元後の本文を確認した。元の停止記録は保持する。
同じprocessのE06では、Coreを保存してから固定fixtureの日本語ドラッグ選択で停止した。
本文673 bytes、clean、U0/R0は不変だが、期待した61–70 byteの選択が61–61に留まった。
続く一回の観測では、DOWN後のcapture、終点座標付きのabsolute MOVE、UP前の61–70 byteの選択、
UP後の選択維持とcapture／ボタン解放を確認した。本文・履歴・IME状態は不変で、画像でも日本語だけの選択を確認した。
旧一括送信が失敗した具体的なOSイベント順は未確定だが、製品のドラッグ選択は段階を分けた操作で機能した。
同じ検査手順を独立review後にcanonical harnessへ反映し、現在の選択から継続した。
この変更はドラッグとその3つの呼出しに限定し、既存のIME検査条件は変更していない。
新harnessで日本語・結合文字・ZWJ絵文字・全文の選択、置換／削除とUndo、
145-byteの段落挿入・折返しをまたぐドラッグ・置換・2回のUndoがPASSした。
Sourceの13画像とLive表示を確認し、673-byteの本文、clean、履歴を維持した。
LiveのSVGは非対応の案内を表示しており、SVG描画のPASSとはしていない。
再開用の集計変数漏れによる停止は別記録に保持している。

E07では、段落・引用・リスト・コードのTab/Shift+TabとUndo、チェックリストのEnter継続・
空項目での終了とUndoを実行し、17画像で表示を確認した。本文は正確な673 bytesへ戻った。
Ctrl+W後には外部ランチャーのメニューが現れ、タブを閉じられなかった。
MDLiteの通常メニュー「文書を閉じる」はenabledで、その操作では同じprocessのpointerを閉じ、
保存済みCore160 bytes・U25/R1・2タブへ復帰した。外部設定やprocessは変更していない。
Ctrl+Wの実到達と残るEnterの組合せは未検証として保持する。
E08以降・保存復旧・Main/Compactの連続往復は未完了である。

描画失敗時にはclipとEndDrawを対応させ、失敗targetを破棄し、一回だけ復旧を予約する。
Resize失敗を処理し、target生成・再生成・表示先切替後は全体frameを作る。
Git prompt Escape後は編集先surfaceを選んでからfocusを戻す。
Main/Compactのnative message-loop回帰とtoolbar focus時のUndo回帰を追加している。
calendarの状態行は行高17pxと文字サイズ10pxを分け、文字の下端切れを修正した。

再変換取消は文書とsession generationを確認してnative CPS_CANCELを要求し、
成功した世代の終了通知だけを処理する。失敗時は未確定文字を保持する。
ATOK取消後は、本文・revision・dirty・履歴を保った有効位置への選択解除も許容する。
Microsoft IMEや通常選択・確定置換のoracleは維持する。
旧private診断版での取消と直後の同一文字Enter確定は、最終製品の実IME合格へ転用しない。
最終EXEでの通常Enter・Undo/Redoは確認済み。同一文字の連続確定の追加確認は未完了である。

現在の証拠はlocalの `.codex-runtime/input-rendering-recovery/` に保持する。
以前の候補の失敗・成功記録は保持し、最終buildの結果と区別する。
以下は過去の履歴であり、現在の受入状態を示さない。

## 入力・描画回復の現状（2026-10-15）

以下はV5候補に対する記録。現在は借用中のWM_PAINT消失を修正しており、V5のbuild/testはそのsourceに限る履歴となる。新しい候補でGUI受入と最終検証が終わるまでは、Editor復旧完了とは扱わない。
下段の従来の受入結果は当時のsource/buildに限る履歴であり、今回の最終候補へ引き継がない。

| 範囲 | 現在の証拠 | 状態・境界 |
|---|---|---|
| Debug/Release候補 | V5 builder source SHA256 `79b43f41cf0263a874003ed557b5704fae5f416475d237021ebe2a0d51d98ddf`。Debug EXE SHA256 `d52c724bd923ab41559c5254fa4e9b3a1d678d672f0819e5bfd10570c61da3be`、Release EXE SHA256 `46b33c093af25f6bb2bf49bedd8f79500f056622c2a62b1ae90fec6f61a1d52c` | V5 build PASS。source commitは未確定。V3/V4のbuild結果は履歴であり、今回の最終候補には使わない。 |
| 限定回帰 | 合成IME終了通知後の最初の通常入力、測定したfallback layout高、遅延child redraw要求の保持・消費、通常close gate、owner-drawの空/日本語caption経路 | 対応する限定native回帰はPASS。可視GUIのchild描画や一連の実入力を証明しない。Dairyの表示名・directory semanticsは既存契約として維持。元のcommand label assertionを対象とするtestはPASS。 |
| 最終機械検証 | V5 Debug: core 139 + native 154 = 293 tests PASS。Release: core 139 + native 154 = 293 tests PASS。全targetのDebug/Release strict ClippyとfmtもPASS。 | 最終V5機械検証PASS。 |
| 可視画面 | V5の空文書/no-workspace windowでsettled screenを観測。native child labelsとNoWorkspace textは見える一方、parent D2D領域は白く、合成文書sourceはempty・revision 0のまま。 | 描画原因を調査中。画面観測であり、Editor動作や全体GUI受入のPASSではない。現時点でproduction source変更なし。 |
| 連続GUI/実IME | V5 runはforegroundとfresh diagnosticsを取得し、case入力前のinitial screen framesも観測。その後、harnessのevidence assemblyがcase入力前にE01 HARNESS_FAILUREとなり、operationsは0。失敗runのharness SHA256 `3e94064310bbaf9ad6dbab9531f505af52bd9d2f40411a00db960b7b60fc5a33`。修正後harness SHA256 `ca8b478bbccdb3d51a9205052713648888374ab2105e20b20209307461e16698`はGUI未実行。 | GUI/IMEの動作assertionは実行されず、連続受入はNOT_RUN。initial/settled screen framesは診断用であり、全体受入の証拠にしない。 |
| 最終review/F5 | 最終候補への独立correctness review再確認と通常VS Code経由のDebug/F5 | NOT_RUN。以前のsource reviewとroute preflightはこの結果に代えない。 |
| 完了・配送 | Human/physical acceptance、Git統合、HANDOFF | NOT_RUN。 |

2026-10-14のUI・周辺機能再実装は[実装・検証表](ui-feature-parity.md)へ記録する。
以下の既存Editor結果は、その対象source/EXEに限る履歴であり、新UIのGUI合格へ流用しない。
追加分の最終sourceはDebug/Releaseともcore 139 + native 146 = 285 tests、fmt、Clippyと
独立correctness reviewがPASS。対象source/EXE/harness hashは上記対応表に記載する。
最終Releaseは所有window/実行体を確認したがforeground gateで入力前に停止し、GUI操作はBLOCKED。
以前のDebug smokeはsource変更によりidentityが失効しており、今回の合格には流用しない。
PrintWindowはDirect2D内容を取得できないため、承認UIとの比較証拠には使わない。

Rust開発の主線はdevelop。旧実装の正確なdevelop tipは
`cpp-archive/pre-rust-20261013`とGit履歴に保持する。
開発主線の選択は、全製品・全入力機器の受入合格を意味しない。

## Rev4 残件修正の受入記録

Task `20261018-mdlite-live-navigation-ui-git-polish` Revision 4を受領し、Rev3の成果を継続する。
以下は進行中の対応表であり、今回の最終実行体の全項目合格を示さない。
旧節の結果は各実行時点の履歴として保持する。今回の対象表示倍率は100%。
150%等の追加倍率は対象外であり、保留・本人判断待ちにはしない。
旧報告の「本人指定の150 DPI保留」は撤回し、本人指示として引き継がない。
実測の96 DPIという値と、追加倍率の受入範囲は区別する。

| ID | 要求 | 有効な既存証拠と不足 | 次の確認 |
|---|---|---|---|
| G01 | 通常起動で本文・controlが描画され、入力へ進める | 現行製品の白画面を再現。Main描画通知だけでは不十分 | 子windowとMainの描画順を修正し、初期画面と最初の入力を確認 |
| P01 | 新規初期配置と旧配置の移行、幅・順序・折畳み保全 | 旧schema v1の移行・再serialize試験に加え、旧候補のAppロードとschema v2保存で幅210/230・順序・flagsを観測。最終buildの起動fenceは未成立、実再起動は未確認 | Appでの配置ロード・保存読戻し、新規起動と再起動 |
| P02 | 左右独立のgroup切替と代表icon | 独立性の状態試験、限定代表のみ | 左groupの連続切替中に右groupの配置・内容を確認 |
| P03 | 明示的移動・追加・解除、重複なし、再起動・Compact | persistence/layout試験のみでは全GUI経路を満たさない | 操作ごとの表示・保存、再起動・Compact復帰 |
| P04 | 左右4状態で連続resize、本文幅回収、focus/hit test整合 | 旧候補の左幅3地点と保存・原文不変を観測したが途中control空白で描画FAIL。描画borderのDOWN優先処理は修正・直接試験済。最終buildのdrag画像は未確認 | 正しいsplitter経路と新描画候補の中間・復帰frameを確認 |
| E01 | fenceの区切り・言語・未完状態を直接編集 | raw projection試験、新候補の言語指定への直接入力・保存・Undo代表成立。全delimiter条件ではない | 残る区切り・未完状態を既存機械証拠と対応 |
| E02 | Front Matter・水平線・Setext・code内literalの区別 | 分類・source mapping試験あり。操作時表示は未確認 | move/scroll/resize/edit/Undoで分類と原文を確認 |
| E03 | 各見出しの直接入力、端・折返し・scroll後のcaret | 境界試験、新候補の直接click・ASCII/Unicode日本語・caret代表成立 | 残るlevel・端・折返し・scrollを既存機械証拠と対応 |
| E04 | 原文物理行番号、Source/Live切替と先頭末尾移動 | 混在改行・行番号試験あり。全GUI代表は未成立 | 切替・移動と本文・dirty・revision・履歴不変 |
| U01 | query周辺の当たり判定、戻る/進むの文書・位置復元 | hit area/永続query試験と限定GUI代表 | 各境界と文書・位置復元をUndoとは別に確認 |
| U02 | 短いmenu、必要な説明・無効理由、keyboard/theme/focus | menu/style/mnemonic試験と限定操作のみ。新selected driverは初回準備失敗でmenu前に停止 | UI定義照合、残るhover/keyboard・説明表示 |
| U03 | Git toolbar・Calendar余白・共通密度、狭幅の到達性 | geometry試験と限定静止画のみ | 通常幅と狭幅の操作・可視性 |
| S01 | 全設定項目、層・継承・false/0/空、色・key・競合・Reset | 全23行への到達・実効値・入力・継承と代表A–D、Custom/Darkリンクの実色・保存読戻し・復元が成立 | 最終描画変更の影響を確認。全項目の全値組合せをGUI済とはしない |
| S02 | Apply後Main復帰と次入力、遷移中の空白・混在なし | 隔離共通DC候補の46frameで混在なし、各125回のwindow DC最大13.4/6.8/3.8ms、実ATOK成立。製品最終確認は未完 | 診断追加を除いた製品へ反映し、最終buildで必要なclose/inputを確認 |
| R01 | 影響するCalendar/Git/Search/実IME/表/画像/保存と履歴 | Rev3同sourceのBIN各302/LIB各150などを対応範囲で再利用 | 変更した共有経路の回帰。Unicode合成を実IMEに代替しない |

S01の現行定義は23項目（scalar 6、登録色7、登録keybinding 10）。
General/Appearance/Colors/Keysの各生成行はlabel・実効値・入力・継承を接続し、
Pathの明示空欄化とShortcutの割当解除を別に扱う。Commonと利用可能なWorkspaceを対象とする。
全定義のUI接続に欠落は見つからず、registry・false/0/空/解除・patch・下書きResetの
同source機械証拠を再利用する。全23行の到達・表示と代表値操作を確認したが、
全項目の全値・全層の組合せをGUIで網羅したという意味ではない。

再開基準はbranch `fix/live-navigation-ui-git-polish-20261018`、HEAD `59ce952`、
既存製品compiler inputs `6abe0168…`、Release EXE `f9713b5e…`。
起動観測 `20261020T161634Z-3ad006c41ee941f39e17e42d07cc9266` は、
強制paintなしでMainの白画面を確認した。採取は所有ATOK補助windowの重なりで安全停止し、
run全体はFAIL。保存された安全frameだけを採用し、採取失敗を製品合格にしない。
原文・selection・dirty・履歴を変えた操作は送っていない。
独立review、最終GUI、Git統合・配送、HANDOFF保存の完了は今後の実結果で追記する。

modal/theme修正候補の登録Releaseはsource `dcd9e8be…`、EXE `33321808…`。
局所9試験・format・Debug strict Clippyは成立した。実GUI run
`20261020T163224Z-b1880bd0de964f7292fca86d580f150e` では、Apply後のclose状態保持と
Mainへの一度のクリック・Unicode文字入力を確認した。後者はnative focusと26個のtyped stamp、
期待する原文・selection・履歴が対応する。診断ファイルの一時lockで一つのstampが取得不能となり、
collectorはUNBOUNDとしてrun全体をFAILにした。取得不能なframeを合格へ補間しない。
また実画像014～018に白いボタン・空のExplorer・異なるテーマの混在が残るため、
テーマ描画の製品受入もFAILである。最終静止画像の正常表示でこの未達を覆わない。

同製品候補のrun `20261020T164604Z-ceaaede268c14fa3abf94609c4aa6bcf` は
`E01_E04_DIRECT_LIVE` の二つの直接click・入力・保存・Undo代表がPASS。
初期client画像と見出し・fence言語指定の画像を確認し、表示とsource/selectionの対応を採用した。
Front Matterと全fence区切りmatrixはこのrunではNOT_RUN。Unicode合成は実IMEではない。
他caseはNOT_SELECTEDのため全体表示はINCOMPLETEであり、全項目PASSとはしない。
fresh clean gateによる通常終了・actual exit 0も成立した。

第2修正のsource `927c8015…` / Release `38943a26…` のrun
`20261020T172933Z-102a6f12b07c445491f55176c45642f3` は、選択した
S02 Apply/closeと実ATOKのpreedit・Space変換・候補段階・Esc取消・Enter確定・
一単位Undo/RedoがPASS。元の入力profileとapp-local IME open=falseへ復元し、
既知の一回Undoでfixtureはclean、undo=0/redo=1、全11個のdisk hash不変、通常終了exit 0。
候補popupの物理操作・実表示位置を確認したとはしない。
44枚の安全frame（最大COPY開始間隔67.3684ms）ではExplorerは完全な旧表示から新表示へ移ったが、
015–016のtoolbar空白と017–018のLight Main/Dark Explorer混在が残り、S02描画受入はFAIL。
同じ順序調整の反復を止め、独立した方式検討を経て隔離候補を準備中。製品採用は未実施。

同source/EXEのrun `20261020T173606Z-68011a20210341238a9a7dbea3b29895` は
S01_FIELDS_COLORの選択caseがPASS。全23行（6/7/10）へのscroll到達と4枚の所有Settings画像を採取し、代表2枚を目視した。
CustomリンクのmagentaとDarkのblueの本文画像、保存値と実効値の区別、再open読戻し、
元のtheme/link設定への復元を確認。Common・未知key・文書の原文/dirty/履歴を保った。
両runの他caseはNOT_SELECTEDのため全体はINCOMPLETE。これを全要件合格へ読み替えない。
P01のrun `20261020T180644Z-f4a1a9f164ab410fb6e29932989dcb84` は旧配置のApp load/save、
左幅3地点のheld-drag・幅264保存を観測。4枚の画像では途中のtoolbar/Explorer controlに空白が
残り、P04描画はFAIL。右幅の押下点は描画borderとSIZEWEカーソルに一致するが、Outline本文左端でもあるため、
feature操作がsplitter操作より先に始まり期待幅待ちがtimeoutとなった。これはP04の製品側
hit-test不一致であり、試験点を描画線外へずらして受入にはしない。判定順の修正が残る。
Compact・実再起動・残るdragはNOT_RUN。fresh clean gate後の一回WM_CLOSE、exit 0、
全9文書disk不変、元のFAIL記録と停止時diagnostic保全を確認した。

共通DC/合成描画の隔離候補 `cc3bef2a…` / EXE `9d2cc8da…` のrun
`20261020T185738Z-ac17762e464741f4b3ea7bc2f7ddb040` は、元の121行fixtureで
S02 ApplyThenCancel/CloseStateと実ATOKがPASS、通常終了exit 0、proof valid。
全46枚と切替直前/直後を目視し、以前の空白・Main/Explorer混在は観測されなかった。
最大COPY開始間隔78.7975msより短い変化まで観測したとはしない。
Main/Explorer/準備用window DCは各125組完結、最大13.352/6.764/3.804ms、100ms以上0。
Explorerの準備最大179.431msはwindow DC取得前へ移り、全125回buffered、fallback/dropは0。
この方式を製品反映へ進めるが、診断追加を除いた最終buildの受入・独立reviewは残る。

途中のrun `20261020T183527Z-ac3e13f02e994d6ab1d731a8091198b9` はSettings準備待ち7秒超過。
後の観測で描画待ちは解消しており、dialog不在・永久停止とは扱わない。同runの多数の再描画は
短い診断と元の操作列では再現せず、原因未確定の記録として保持する。診断タグで修正したとはしない。
この失敗を後のPASSへ付け替えず、所有するSettings/Mainは通常終了し文書を保全した。

## Editorの確認対象

入力→原文→selection→layout→次の入力を一つの契約として確認する。
本文はcrop Rope一つ。表示更新は本文、encoding、dirty、履歴を変更しない。

| 項目 | 今回の契約 | 確認状態 |
|---|---|---|
| 入力・IME | 実preedit範囲、本文とのbaseline、通知順序、終了後の通常入力 | 実装・関連回帰・独立review PASS |
| 移動・選択 | Unicode、Ctrl/Shift、preferred x、空行・EOF・block境界 | 実装・関連回帰・独立review PASS |
| Live執筆 | 未完成Markdownから成立・修正、code内literal、表保護 | 実装・関連回帰・独立review PASS |
| Undo/Redo | 原文と向き付きselection、保存点、typing/IME編集単位 | 実装・関連回帰・独立review PASS |
| 回帰 | BOM/CP932/CRLF、保存失敗、画像・表・未保存文書 | 実装・関連回帰・独立review PASS |
| 終了 | composition終了、保存/破棄/Cancel、失敗時本文保持 | 実装・関連回帰・独立review PASS |
| 通常開発 | 最終develop Debug/Release、tests、fmt、Clippy、通常起動 | 実装・関連回帰・独立review PASS |
| review | 実装者のPonytailと別Agentのcorrectness review | 実装・関連回帰・独立review PASS |

以前の試験結果と失敗記録はGit履歴に保持する。過去のPASSは今回の
連続操作・IME・GUIの証明へ転用しない。各経路の最終状態を別々に記録する。

## 最終確認

統合後のdevelopから、既存mise Rust1.99/MSVCでDebug/Release各131 tests
（library58、native73）、fmt、Clippyを確認した。source aggregateは
`09e23ff5fa27225c000f183b2c5fb0a5f459d905565015807d87143795a050d9`。
Debug EXE SHA256は`a96d72f35570569c155f8bc2603d0e3abe076c35ecfbd1c53f57bad46a030297`、
Releaseは`b96a80d31afab6e0ad48c4a66a78df36ed13b275f03596a025a5a35f2305eba0`。

Root/実装者と別のAstra XHighによるread-only correctness reviewを実施。
起動順序、表前の位置、隠れたfenceの移動、toolbar終了、harnessのABI/期待値を
修正し、同reviewerの局所再確認がPASS。実装者のPonytail確認は別に実施した。
原文・保存・encoding・画像・表の既存回帰を維持し、未移植機能を合格へ付け替えない。

所有するvisible Release、96DPIの合成fixtureで通常のwindow message経路を確認。
通常ASCII仮想キーで表編集、Enter/EOF、code内literalとcode外の見出し、
反復矢印、Shift/Ctrl選択、逆向きselectionのUndo/Redo、Source/Live切替をPASS。
現在のATOK profileのまま、romaji、Space変換、Esc取消、Enter確定と1単位Undo/Redoを
確認し、app-local open状態を元のclosedへ戻した。profileは選択・変更していない。
Unicode packet入力はこのATOK環境でIME lifecycleを通る別経路だったため、
通常キーのtyping-group試験と区別した。per-message全通知列を採取したとは主張しない。

未保存closeのCancelは本文・selection・dirtyを保持し、その後の保存・通常終了をPASS。
別の所有する4tab試験では保存Yesと明示Noも確認し、合成本文の証拠を保持して通常終了した。
Save失敗・外部変更拒否はstorage/native関連回帰で確認し、全dialog分岐の物理受入とは区別する。

通常VS CodeのStart Debugging controlからprelaunch/CodeLLDB/native Debug起動、IDE Stop、
所有IDE windowのcloseをPASS。起動fixtureのbytesと最終Debug source/EXE対応を確認。
物理F5入力を送った証拠ではない。Releaseはdebuggerなしで確認した。

C++保管branchは`cpp-archive/pre-rust-20261013`、保管元developの正確なtip
`e852ee768a9fe3708e5e7eb7c33ab5987fb71734`を両remoteで保持する。
通常mergeは修正済みRustと旧develop双方を祖先として持つ。現行tracked treeから
旧専用98pathsを整理し、共有UI icons、editor settings、licenseと製品要件を維持した。
main/tag/公開Releaseは移行対象外。最終配送OIDとreadbackはlocal配送記録へ固定する。

## 検証の境界

OS clipboardの読み取り・列挙・書き込み・backup/restore/clearは行わない。
Ctrl+C/X/Vと画像貼付けのOS統合は検証対象外。純粋payload試験はOS統合の証拠ではない。
入力profileやdefault IME、global設定を変更せず、所有fixture/processだけを使う。
合成window通知、OS合成キー、実IME、物理typematic、Human受入を区別する。
50MBは望ましい目安であり、機能削除やworking-set trimを行う合否gateではない。
実MS IME、再変換、候補popup位置・文節操作の全組合せ、物理typematic/F5、別monitor/DPI、
Narrator、主観的UIと全製品Human受入はNOT_RUNまたはHuman/機器確認待ち。
今回のATOK基本変換とnative geometry回帰を、これらの受入へ拡張しない。

## 製品全体への対応

今回のEditor範囲外の要求を削除しない。承認UIと元仕様の要求を維持する。

| 元仕様 | 今回の責務／後続 |
|---|---|
| R-01 Workspace | 今回は安全なファイル操作の入口。Explorer全操作は後続 |
| R-02 Live Editor | 今回の中核。現sourceの実動・未検証条件を上表に記録 |
| R-03 Markdown | source projection、GFM表、Front Matter。構文完全性は試験範囲を明記 |
| R-04 Navigation | 文書タブとsource mapping。Outline／Quick Open等は後続 |
| R-05 Search | 左パネル検索・dirty buffer優先・任意glob・source一致移動は後続 |
| R-06 Templates/calendar | 後続。Dairy表記、祝日任意更新の既定OFFを保持 |
| R-07 Save | 基本I/Oと外部変更拒否。autosaveは後続 |
| R-08 Encoding | UTF-8/BOM、CP932、混在改行を今回検証 |
| R-09 Recovery | session／復旧は後続。有効なように表示しない |
| R-10 Git | 製品Git UIは後続。Gitなし／pushなし編集を可能にする |
| R-11 Settings | 完全なGUI／Workspace上書きは後続 |
| R-12 Appearance | ダークshell、可変文字、細いグレーscrollの基盤。承認UI全体は後続 |
| R-13 Commands | 必要な編集操作。palette／変更可能keybindingは後続 |
| R-14 Windows | 標準Windows操作。compact／重複Workspace管理は後続 |
| R-15 Safety | source／画像の安全境界。完全なworkspace trustは後続 |
| R-16 Maintenance | 手動更新・診断体系は後続 |
| R-17 Performance | 今回のRelease編集基盤のみ実測。完成製品の保証としない |
| R-18 Large files | 自動機能削除を行わず、今回の測定・試験限界を明記 |
| R-19 Development | Cargo／F5の再現経路と実行確認範囲を今回記録 |
| R-20 Public Git | Rust専用developを両remoteへ配送。main、tag、Releaseは別指示 |

Rev4の現在の最終製品候補はsource47 `bd6b9500cae50d087d3407fbe5ecd036c674e9864fbdf139dc867593280515b5`、
登録Release EXE `8f4e76b02ec31fc06c594118fb315b6513d7b767bce2d3373a2423fcee2dc109`。
共通DC描画とsplitter優先処理を統合し、隠れたnative scrollbarの無変更更新での再描画要求も局所修正した。
後者の5関連native試験は成立したが、同じ旧配置でのP01実行
`20261020T194919Z-51e825d781e54c01a660e1bbe0724102`は操作前の初期fenceで再び停止した。
13個の可視Main/button/query surfaceにpending damageがあり、1411msで未収束と分類した。
7秒待機を使い切ったtimeoutではない。診断追加を除いた候補なので、このrunの再描画頻度は測定していない。
単独のidempotence修正を全体解消として採用せず、隔離した既存診断機構で発生箇所を確認する。

U02/U03初回実行 `20261020T195313Z-d4fa8b97244c41468be7653f6c33ec77` は、
準備時のpanel-layout読出しがクリック処理完了を確認しておらず停止した。メニュー・狭幅の合否は未観測。
Explorer選択時の保存は製品sourceで無条件に接続されており、「選択済みだから保存されない」という初期仮説は撤回した。
両fixtureは新鮮な原文・履歴・IME・所有確認後に通常WM_CLOSE各1回でexit0、文書9個と設定・配置・session bytesを保全した。

独立したSol Highの静的正しさreviewは、製品の追加不具合を特定しなかった一方、harnessに2点の必須修正を返した。
held dragの到達済みframeよりさらに新しいframeを要求して正常な静止状態を拒否し得ること、
menuのEscape後focusを開いた後のfocusと比較し、開く前のfocusを判定に用いていないことである。
両点の局所修正・反例確認・安定差分の独立再reviewは完了した。実GUI確認の代わりにはしない。
元のdamage再登録を省いた隔離候補（source47 `5da073fe…`、EXE `80162feb…`）では、
対象分岐の機会5回に対してMain描画は約478秒で6回だった。変更前の約145秒で3490回とは異なり、
持続的な再描画は再現していない。deferred damage・full frame・失敗時の回復は保持し、
実際のborrowed WM_PAINT後に正しいpartial damageを描く試験を含む関連5試験を通した。
これは完全な原因説明や全パネル受入を意味しない。

同候補の `20261020T210013Z-5f2f155cbada43189e381cfda882fe44` は、既存の1回のpaint pass後、
追加のpaint／invalidationなしで13個のpendingが818msでゼロとなり、presentation後の最終確認も849msだった。
元の7000ms期限とsource／frame／COPYの一致を維持し、Rootは保存画像で本文・toolbar・右Outlineを確認した。
自律的な最初のpaintと入力を含むG01全体は別に確認する。run全体は、正常終了送信の1500ms応答待ちが切れ、
その88ms後にprocess exit0となったため `REV3_NORMAL_CLOSE_UNCERTAIN`。配送確認とexit0を同一視しない。
終了後は対象process／windowなし、文書9個・設定・panel seedのhash一致を確認したが、同runの終了前session hashはない。

full P01試行 `20261020T210839Z-26e0c6000fba4566b9b73d368c577c51` では、App移行保存と、
両側表示で左幅を押下保持した3地点の画像を採取した。3画像はsource／履歴と到達geometryの観測に対応し、
Root確認で以前のtoolbar／Explorer headerの空白を認めなかった。各COPY間の画面は未観測である。
その後のsettled確認は162回の受動観測でも可視childのpendingが残り、7006msで期限切れとなった。
右幅・内部height・全4状態・Compact／再起動へは未到達。P01～P04全体をPASSにしない。
失敗後は通常終了し、pre-actor disk_beforeを独立oracleとして文書9個のhash一致を確認した。
部分操作によるschema2／左幅264の配置は復元せず保全した。

U02は、Home後には項目選択が観測されず、同じ所有menuへのDown1回でcommand1のstate128と実画像のhighlightが一致した。
仕様のAlt／矢印／Enter／Escに沿って検証を矢印へ合わせ、Home固有の未確定挙動を製品不具合としない。
説明表示・disabled理由・後続menuと狭幅の全経路は未確認である。menu shadowの採取は、同PID／UI thread、
検証済みpopupとの直接z-order隣接と限定geometryを満たす正確なHWNDだけを対象にし、下位foreign overlap確認は保持した。
ネイティブLISTBOXの測定では、最初の標準STATIC親クラスにCS_PARENTDCがあり、WS_EX_COMPOSITEDとの互換性を欠いていた。
互換性のある専用CS_DBLCLKSクラスで1回測り直すと、非空選択の解除・再設定はLISTBOXと対応する親領域をdamageにした。
空選択の解除、同じtop／行高、変更のないadapter同期ではdamageを認めなかった。
したがって以前の「親へ伝播しない」という推論は採用しない。実際の失敗場面のnative選択は未記録で、原因の確定や製品修正の根拠にはまだ不足する。

U02追加試行 `20261020T214539Z-c753bdee8b1b427096cabf74c6abfe5f` では、Fileの矢印選択、Viewの無効項目hover、Calendar headerの3画像をRootが確認した。
File／Viewの約1秒のhelp観測はいずれもtooltip候補0・textなしで、説明文と無効理由は未確認のままである。
region menuはMainのcrop外へ伸び、既存の厳格な所有・採取条件が停止した。範囲を緩めたPASSにはしていない。
その後の所有確認付きEscapeと通常WM_CLOSEは535msでexit0、所有windowなし、文書9個はpre-actor oracleと一致した。
menu選択自体、説明表示、crop外popupの安全な観測を分けて原因を確認する。

隔離候補の追加測定 `20261020T215547Z-a14fda35583847d09b0375b16a470956` は、
同じ左幅drag後のsettled確認で7018msに失敗した。3地点の画像はRoot確認で一貫して読めるが、P01全体は未達。
終了後traceはzero dropsでMain1581回／250.926秒。両側表示では1541回中1540回、EndPaint後はMainの要求がなく、
次のprepare入口までに再び全面要求が入っていた。子surfaceは各同期境界で既にdirtyのため、同期処理の再無効化を除外しない。
失敗後の所有確認付きnative queryはExplorer選択数0・top0・行高20だった。通常終了exit0、文書9個と保護対象のhashを保全。
この記録は原因の区間を絞るもので、合成描画の因果や出荷修正を確定しない。

U02の `20261020T220127Z-e06df088fd6c4f37bc6d3f7aba58bd51` はCalendar headerのpopup所有確認で停止した。
region本体採取・狭幅確認には未到達。停止後の安定した所有metadataから、元の瞬間の失敗条件は推定しない。
所有確認付きEscapeと通常終了は595ms／exit0、文書・保護hash一致。繰り返しを止め、方法の再評価待ちとする。

MainのみWS_EX_COMPOSITEDを外した隔離比較 `20261020T221920Z-077e8eef02fb4e67bc69232b130e6414` は、
左264・右216の幅変更後に元のsettled fenceを通過した。Mainは229.124秒で90回、両側表示48回すべてEndPaint後clean。
一方、Rootが確認した全12画像では、左幅押下中のExplorer／Calendarボタン行欠落、右幅・内部高さ押下中の古い輪郭やボタンの重複がある。
静止後の画像が正常でもP04表示品質を満たさない。このフラグ除去を出荷修正として採用しない。
比率保存assertionで停止したが、原因はdriverの整数pixel丸めを期待値に反映していなかったことだった。
410.5 DIPの要求は410pxとして届き、実際の374/701に対応したweights256.0913/223.90869が保存されていた。
ISO検証式は要求座標のpixel丸めから独立に期待値を計算するよう修正し、許容差0.1は維持した。元runのFAILは書き換えない。
再起動・Compact・全状態の受入は未達。通常終了exit0、文書9個と保護hashを保全した。
このrunでは一度だけのATOK後再採取branchは使われず、各対象captureはCOPY1回だった。

Mainを私用DCへ描いてから転送する隔離候補（source30058505…／EXEed769393…）は、限定13試験と独立reviewを通したが、
実P01 `20261020T230602Z-186bfe636a024dcd8de9573220e793b3` では最初の左幅settledが7003msに失敗した。
166回の受動観測後も17surfaceがpending。閉鎖traceはMain644回／121.052秒、両側表示604組で描画後clear→次入口fulldirtyだった。
Root確認の初期＋押下中3画像は一貫していたが、実場面の収束は解消していない。この方式を正本へ統合しない。

同方式のMain互換性参照だけを表示装置DCへ変えた比較（source7e63c38a…／EXE0ac54a10…）の実行
`20261020T231721Z-495fab8379d949f1a76540bba15968cd` は、診断要求の延期応答で押下中採取前に停止した。
閉鎖traceにはMain728回／136.149秒、両側表示691組のclear→fulldirtyが残る。操作の受入は未観測とし、Main参照先だけでは収束しないと判断する。
どちらも通常終了exit0、文書・設定・配置・保護hashを保全した。次の比較は保全したdirect-Main基準へ戻し、Explorerの私用bitmap互換性参照だけを分離する。

Rev4は利用者の終了指示でUSER_CUTOFF／INCOMPLETEとして保存する。必須受入は未達で、developへの統合は行わない。
作業branchのみcheckpoint配送する。最後に推奨されたExplorer互換性参照の比較は未実装・未実行のまま停止した。
表示倍率100%が対象で、150%等は対象外。「本人指示の保留」という旧記述は撤回済みである。

## Rev5 重点確認の到達点（2026年10月21日、部分完了／INCOMPLETE）

Task `20261018-mdlite-live-navigation-ui-git-polish` Revision 5。旧Rev4 snapshotは固定Downloaderでhash検証・保全後、CURRENT/MANIFEST/受領ファイルを検証して更新した。旧checkpoint `6f5530fa0223c05565b8a6a116bfc239692b82e9` と隔離候補を保全し、製品ソースは変更していない。

対象source47は `0faaa0c82009a1bab67d5f3ba2f7bcb2ce6b5de2e3a19db69e09d7e7ff77f917`、EXEは `.codex-runtime/live-navigation-ui-git-polish/rev4-native-tests/target/release/mdlite.exe`、SHA256 `de3087d96b1d7cf83d122e0add74e58c94aa2df6932b359675f47770eb027dd0`。登録済みRelease buildを同一targetで成功させ、同一source/EXEを読戻した（`rev5-build/build-release.json`）。Rootは実runtime `gpt-6.1-sol / medium`、VERIFIED_ROOT。倍率は実96 DPIのみ。

| Rev5の要求 | 確認した動作・証拠 | 到達点と残件 |
|---|---|---|
| 通常起動→表示・入力・保存・Undo/Redo | 所有foregroundを既存操作で確立した通常起動の本文/control画像。run `20261021T012413Z-bfd602a07ae64537ad550c19434172bd` で独立期待文字列、保存bytes、Undo/Redo、原bytes復元。保存後50frameは全て正しい追記本文。Undoは最初の1frameだけ旧表示、以降49frameは全て正しい原文 | 観測範囲PASS。初回のEndDraw直後COPYが旧表示だった事実を保持。持続するstaleではなく自然追従を観測した。自律的背景起動・最小化/restore・全条件の即時表示まで主張しない |
| 左右パネルの切替・代表icon・移動/resize・保存再起動 | グループ配置と左切替は既存UI操作で到達。最終run `20261021T013604Z-a58a0b9b39014eca83b300e2afbe6f52` は右regionの最初のpopup所有確認で停止 | 未完了。変更対象drag・押下中品質・Compact・通常再起動保存は未到達。既知の不採用COMPOSITED除去案のボタン欠落/残像を解消済みにしない。正本で同症状を観測した証拠もない |
| Settings Apply→Main入力・テーマ遷移 | run `20261021T011845Z-515e851d6d954163b3236a77147fcb45` 選択S02 PASS。42frame全てを画像で確認。dialog閉鎖の透過遷移が消えた後、Main/ExplorerはDarkからLightへ揃って変化し、採取frameにtoolbar白抜け・混色なし。実ATOKの変換/Space候補移動/Esc/Enter/一単位Undo/Redoと保存、profile/open状態復元 | 今回の代表経路PASS。最大COPY開始間隔76.756ms。Apply前半の画面、別閉鎖経路、物理操作・候補popup・Human全体は別未確認 |
| 共有focus/描画が影響するLive直接編集 | run `20261021T013406Z-f0b8910894bb4665a52bd763c7d01b87` 選択E01 PASS。実caretクリックの原文位置から見出しUnicode、fence言語、開き区切りに直接入力。原文・保存・Undoを独立期待値で確認。52/53/53frameの各PNG同値groupを目視し、言語rustunknownと4backtickに対する未閉鎖code表示を確認 | 今回の代表操作PASS。最初の旧表示frameは保持し、後続の自然追従を確認。Front Matter/全端点/Source-Liveの旧全IDを埋め直した主張ではない。Unicode合成と別runの実ATOKを区別 |

### 検査側の修正と停止判断

登録済みharnessだけを変更した。Rev5 opt-inのMain/Compact実画面採取は、所有・入力focus・遮蔽・固定crop・原文/成功EndDrawの前後対応を維持し、pending完全ゼロを画面採取の前提から外す。EndDrawと実画面提示を同一視せず、既存の受動2秒採取を再利用した。診断LRESULT0はApp借用中の未処理要求であり、queued成功ではない。元の3秒期限内で同じ要求だけを再試行し、各回の所有確認、LRESULT1、新しいtimestamp、既存JSON/selection検証が揃うまで受け入れない。

旧比率FAILは製品不具合ではなく、driverが送る整数pixelの丸めを期待値へ反映しなかった検査誤り。保存済みISOの式を反映し、元のFAILは保持した。新しいRight切替actorの非表示rail113参照は、仕様に沿う既存region109メニュー操作へ訂正した。最初のqueryクリックを使うパレット待ちの停止原因は未確定で、終了前には151が可視だった。最後は登録済みCtrl+K入口へ方法を変更し配置まで到達したが、既存popup厳格所有guardで停止した。これらを製品の入力不能・ボタン欠落の証明にしない。

最終harness SHA256は `29a1b34aa6e08539beca0fc4540a1475202ff907e50dcb6cadaeec1d2e0d9ece`。S02証拠のharnessは `c773591791eadd50696a0dbe444972506328e9ca0452910a098ec21165ab6860`、初回安定表示seriesは `331d9d47e1b8a25d0a2659f3ec4e52fa0e6720f7b7055c51b09b4c5423944796`、Liveは `f533776d565f52c091c8ee60212b96b8637fd237bebc14d587936681dd30db1c`。後続変更はpanel用actor/入口と選択の限定差分で、同一製品source/EXEの成立済み証拠を無効にする変更はない。各runの元receiptとRootのpixel reviewはlocal受入directoryに保持する。

製品変更がないため、同source47を明示したserialized Release BIN314 PASS/10 ignoredと既存Debug314、条件付きLIB150を再利用した。全`-Verify` PASSではなく、既存parallel/LIB fixture失敗は保持する。追加のPowerShell構文・native C#helper compile・guard/期待値・丸め・診断retry反例確認がPASS。実装者のPonytail自己確認と、別Sol Highの静的correctness reviewは別に実施し、checkpoint保存への追加必須修正はない。reviewerは試験を再実行していない。

パネル受入が進まない修正roundを引継ぎ、方法を見直しても最終的に既存popup所有の観測で止まった。guardを緩める代替は採用しない。これ以上の所有観測の調査を今回の小修正として継続せず、部分完了として保存する。単なる時間切れや検査成功による合格ではない。次担当は保持したpopup失敗と同時所有metadataの対応を既存driverで調査し、正確な所有条件を維持した方法を選んで、未到達のpanel/Compact/restartだけを確認する。成立済みの編集/Settings/Liveを担当交代だけで再実行しない。

最終失敗時のmenuはfresh PID/生成時刻/path/hash/HWND/foreground/menu-owner確認後、既存guard付きEscapeを1回だけ送り、fresh clean stateで通常WM_CLOSE。exit0、残processなし。元FAILは不変、cleanupは `rev5-harness-check/panel-keyboard-cleanup.json` に別保存。以前の各runも通常終了済み。

**今回の修正範囲は未完了**。依存するRev4製品変更を安全に分離する大きな切分けは行わず、作業branchのcheckpoint保存・通常配送だけを行う。develop `902dfd17754d45d782508c137c63950b2bc495c4`、main/tag/Release、GitBookの現行Rev3/CR6を保つ。未完了Inboxはcleanupしない。旧U01/U02/U03、W01/W03全背景経路、E08、pin/unpin等は既存の実不具合/未確認区分のまま後続へ残し、元Task全体や全Human受入の合格へ読み替えない。

## Rev6 パネル残確認の再開（2026年10月21日、進行中）

Task Revision 6を固定Downloaderで取得し、Rev5 snapshotはhash検証付きで保全した。開始HEAD `a4943f108bcb4dc9869896bc38d9786d60ec08d2`、develop `902dfd17754d45d782508c137c63950b2bc495c4`、開始時の追跡差分なし。実Rootは `VERIFIED_ROOT / gpt-6.1-sol / medium`、thread `01a15c59-ee98-7cb0-bd81-36aae7a5648e`、turn `01a15c5a-1fe9-7851-91fc-4f1d41fe5e11`。既存Release EXE `de3087d9…` とharness `29a1b34a…` はRev6指定値と一致した。製品sourceの適用条件を照合し、同一なら再buildしない。

再利用する証拠は直前Rev5表の編集・保存・Undo/Redo、Settings ApplyThenCancelとATOK／Dark→Light、代表Live直接編集、および同一製品への既存機械試験の限定範囲。run全体のBLOCKEDや後段NOT_RUNは保持する。今回残る必須操作は右regionメニューの実選択・左右独立性、両側幅と内部高さのdrag画面・文書／履歴保全、Compact実入力／Undo／Main復帰、通常終了と新processでの配置復元。未統合製品差分の共有依存に必要な現在要件も照合する。

局所修正の入口は既存popup predicateの同時点診断と、別に確定しているowner-draw label未取得の誤分類。最初のGUI観測で所有条件と撮影geometryのどちらが失敗するかを区別する。所有・前面・入力直前の同定と撮影境界を維持し、実測前に過去の停止原因を断定しない。GUI操作は単一ownerで直列実行する。Jev Initialは `sol_high_specialist` を選択し、既存harnessの限定修正をSol Highに割り当てた。

独立した読取担当が既存47-path inventoryと現在の各fileからsource47を再計算し、`0faaa0c82009a1bab67d5f3ba2f7bcb2ce6b5de2e3a19db69e09d7e7ff77f917` と一致した。保存inventory内の旧hashそのものを現在の個別file証拠にはせず、今回の再hashを採用する（`.codex-runtime/live-navigation-ui-git-polish/rev6-evidence-map.md`）。製品source/EXEが同じため既存Release buildと限定成功証拠の再利用条件を満たす。共有Settings閉鎖、query/navigation、Git/Calendar密度、Live端点等は未統合差分の直接依存を限定照合中であり、代表証拠だけで全件PASSにしない。

最初の診断run `20261021T025501Z-cf5c1723bdf24c6f99bd6c3048780a44` は右menu前の `PALETTE_QUERY_FOCUS_TIMEOUT`。controls_ready=2943msに対してcontrol取得前からの2500ms期限をfocus確認にも共用し、focus判定が未評価のまま停止した検査側タイマー不整合を特定した。既存guardを保った段階別期限へ局所修正する。これを以前のquery不調すべての原因とはしない。通常WM_CLOSEは605ms＋wait47ms、exit0、process_retained=false。元resultを保持する。

直接影響照合（`rev6-impact-scope.md`）から、panel caseの早期returnで未実行になる左右4表示状態、Settingsの残るCancel/Escape/titlebar close各代表の次入力、実queryクリックとBack/Forward、Git/Calendarの通常幅／狭幅表示と到達性、既存doc-08でSource/Liveと先頭／末尾表示を追加対象とする。既存helper／fixtureを使い、tooltip全巡回や旧全IDには広げない。Front Matterの意味・mapping・入力・端点ロジックに変更がなく既存BIN314の該当試験を再利用するが、完全なFront Matter GUI行列は旧未確認として保持する。これらの追加観測も未実行時点ではNOT_RUNである。

2回目 `20261021T025800Z-80daabf0b74a4ae294bf9f66b5d526c3` は元popup入口まで到達した。同一判定呼出し内でMain/popup生存・PID・class・前面・同thread・menu owner、client/crop一致がPASS。候補1件、Main固定crop `[138,161,1222,922]` に対してpopup矩形 `[1196,179,1328,233]` の `surface_inside_crop` だけがFAILした。所有喪失と撮影geometry混在を実測で区別できたため、操作用所有判定と撮影可否の最小分離へ方法を変更する。最初の2試行は実受入を増やしていないが、同目的の履歴を継承し、次の試行はこの特定条件の除去が実選択まで届くかを判別する。撮影guardの緩和や製品変更は行わない。

失敗後、fresh identity・前面・menu owner確認付きEscape1回と通常WM_CLOSEでexit0、残processなし、元result hash不変（`rev6-harness-check/panel-diagnostic-cleanup.json`）。これは停止時の原観測と別時刻のcleanup証拠として扱う。Jev Findingは `REFINE_REQUIRED`（暫定fix_current）、refinement呼出しは `ERROR / semantic_round_plan_mismatch`。成功choiceには読み替えず記録し、Rootが実測済みの安全な局所修正を既存High担当へ継続指示した。Jev基盤改修へは広げない。

登録wrapperの既存手順を確認し、追加factなしはround2ではなく `SemanticRound 3 / NoMoreFacts` で確定する呼出し規約へ訂正した。同じJudgment ID・同じpacketで `RESOLVED / fix_current`（round3）。先のERRORはそのまま保持し、新しい判断単位へ付替えたりwrapperを改修したりしていない。

局所分離と追加case統合後のharness `b766becf1f0f0868192fc69cab3209051eb0cc161de955e0f8d2f0bda63b5e36` でrun `20261021T030940Z-870c135c572b44789310c2d49193e1e8` を実行。元のpopup入口は通過し、期待command3のread/highlight/Enterと、所有popup項目矩形だけの安全なCOPYまで到達した。後続のLeft groups JSON比較が `PRODUCT_ASSERTION: Right group selection changed Left groups` で停止したため、左右独立性／panel全体PASSにはしない。実製品変化か比較の不整合かは限定調査中。通常終了は643ms＋wait62ms、exit0、process_retained=false。drag／Compact／restartは未到達。安全なlabel PNGは別の視覚担当が実体を確認する。

同runの `rev6-pixel-review.json/md` は実item画像の「アウトライン」を正常と確認した（exact-owned-item capture=true）。これはlabelのみの成立でpanel全体ではない。独立してrun `20261021T031124Z-83f173a6939543329c2ee9a1aa847b9a` のSource/Live／BOF=0・EOF=65byte・原文／履歴／disk不変はharness PASS、通常終了exit0、EXE hash不変だった。一方、同runの視覚担当はSource BOF/EOF画像の同hashと、Liveの選択に対して逆に見えるcaretを報告した。したがって端点の視覚受入は未確認で、初回COPYのstaleと持続的表示不一致を区別する既存受動採取を追加する。元の4画像とresultは変更しない。

before/after記録だけ追加したrun `20261021T031519Z-6b04c9dd43464f7c8cded82faf74ef7c` では、最初のOutline選択のLeft JSONは完全一致（Explorer＋Search）、semantic deltaなし。以前のhelper内assertionは通過し、外側の `Right switching changed Left membership` で停止した。直前Calendar移動後の即時Read-Diagnosticをbaselineにしている完了待ち不足を限定調査する。単にJSON比較を緩めず、要求から定めた移動後membershipを待つ案を確認中。通常終了603ms＋wait49ms、exit0、残processなし。元resultは保持する。

製品 `execute_palette` はpalette閉鎖後にcommandを実行するため、閉鎖だけの待機を移動完了としていた検査不整合を確認した。独立期待値による左右所属・active・両領域表示の完了待ちを追加し、原文／履歴の不変を保つ。先行run `20261021T031845Z-0e590e11cbfb416c8e91ac0a5b6bd9c7` は起動caption再確認未完了で入力0件、編集／capture前に停止。別の既存有限前面化1回で回復し、clean通常終了exit0、元result不変（`panel-activation-cleanup.json`）。guardや製品コードは変更していない。

同じharness `741b44bbf0299cf4123a2131317384081fdf27c0d67ff5928d31244af1b6e265` のrun `20261021T032041Z-a991b3722c6943be998c905cf96b836a` は右のOutline/Calendar/Outline/Calendar/Outlineを実メニューで選択し、5回ともLeft完全不変・semantic deltaなし。左幅dragの3押下中画像とsettled、独立pixel整数oracle／保存配置／文書履歴保全まで成立した。右幅の最初の画像後、次のheld COPYが `OBSERVER_FAILURE: held COPY lacks matching painted source/selection/history/view state` で停止した。右drag全体、内部height、4表示状態、Compact、restartは未達。通常終了714ms＋wait115ms、exit0。採れた各画像は視覚担当が限定確認中、失敗fieldは別担当が限定調査し、独立Settingsケースを直列実行する。

同runの実画像はRootとHigh担当が照合し、右幅画像のLeft Explorer/Calendarと上Right Outlineは存在する一方、Gitの5操作buttonが白い空欄、左幅中間2画像ではpanel title文字が欠けることを確認した。現行sourceはdisabledでも暗背景＋muted icon、可視panelはtitleを要求するため、採取時点の描画欠落であり左dragの視覚PASSは付けない。最初の視覚担当の「左pane／Outline不在」という誤った所見は原reportに残し、`rev6-pixel-review-correction.json/md` で訂正した。持続時間とroot causeは未確定。held waypoint4はreached width224/editor508/min sequence73まで確認したが、拒否stampを保存する前にthrowしたため失敗fieldはUNKNOWN。source／履歴破損の証拠にはしない。

独立Settings run `20261021T032255Z-e613362c7d4c4ec7ae322ce94f069481` はCancel/EscapeのClickChar＋正確な原文／選択／履歴oracleと既存Undo cleanupが成立（実画像review待ち）。TitlebarCloseは `S02_HTCLOSE_CHANGED`、close入力0件でmodalを保持した。別cleanupでowned modalのCancel211、元rowのhash/revision/selection/historyと既知saved+Xの一致、準備Xだけの既存Undo1回を確認し、元disk全hash不変・clean Undo0/Redo2から通常終了exit0（`settings-cleanup-final.json`）。途中のlocal cleanupはVisible metadata誤参照と未設定待機期限で観測を止めたため別receiptを保持し、Undo再送はせず既に送信済みの完了だけ回収した。元result／画像は不変。

既存受動2秒採取をSource/Live各端点へ追加したrun `20261021T033154Z-fe561574f2a54742ac2ddd1a53494119` はharness PASS・通常終了。初回画像を保持し、frameのsource/revision/selection、rawのseries前後を検証した（per-frame rawは非公開）。実画像reviewで自然追従／持続不一致を判別中で、旧runを視覚PASSへ改変しない。

Jev Difficulty `rev6-render-discriminator` は `RESOLVED / sol_medium_specialist`。Mediumの限定相談を採用し、Highが拒否stamp保存、既知2地点だけの同座標・受動2秒後の再COPY、既存release後比較、dragから独立したCompact/restart opt-in、Titlebar同時点診断と単一row filterを局所準備する。元guard／元入力／元FAILは維持し、安全に再採取できなければそのdiscriminatorは止め、追加capture基盤は作らない。並行する読取とGUIは対象を分離し、GUIはRoot単独・harnessは凍結してquery/navigationとdensityを直列確認する。製品修正はまだ行っていない。

### Rev6 後続の限定観測と製品側の再現

- Source/Live `033154…` は201受動frame／4 unique hashの実体を確認し、各seriesの最初からSource/Live・BOF/EOFの正しいcaret／本文を確認した。初回画像3枚の旧端点は保持し、そこからseries開始までの間は未観測。per-frame raw非公開の限界も保持する。
- query/navigation `20261021T033526Z-a3798fc4eec6438cbe9c75dd7c85292e` の選択U01はPASS、他4件NOT_SELECTEDのためsuiteはINCOMPLETE（実失敗なし）。通常終了exit0。実画像はstartupのみでBack/Forward icon／本文を確認、候補表示や次入力の画素証拠までは無い。
- density `20261021T033751Z-b66e7f1bff2743229a6eddc162307219` は100DIP幅へ到達後、Untrusted fixtureのRefreshを有効と要求する不適切なpredicateで停止。製品のTrust条件と独立に、fresh idle UntrustedではRefresh6000は無効・MORE6027は有効が期待値。busy=falseが実diagnosticに存在することとnative summaryのUntrustedを前提に、2項目／1行／境界／overflowを維持する局所修正と11正負例を確認した。新GUI結果はまだ無い。元FAILは保持、通常終了exit0。
- Compact分離 `20261021T034344Z-49d2f65405e949d583fb74425bc8ab76` は最初のpalette controls待ちでBLOCKED、後段は未実行。通常終了exit0。これをCompact不具合または合格とはしない。
- paired drag `20261021T034614Z-3c90776d05f142acb3929d09a77ffc04` は既知2地点の同座標・受動2秒後COPYを取得。LeftではMain sequence61→63の間にtitle／Outline内容が欠け、Rightでは同sequence70のまま旧欠落から回復した。Rootも実体を確認した。Main EndDrawだけを画素提示／child描画freshnessに読み替えない。最終Right waypointは唯一sequence71<要求72で停止し、source／revision／history／selection／全geometry条件はPASS。追加capture基盤を作らず、このdiscriminatorを閉じる。既存settled画像はrelease後にRead-Diagnosticがapp.refreshを起こすため、release-only回復の証拠ではない。
- SettingsのCancel/Escape画像は、入力後の旧画素／残像を確認した。CancelはΩ送信後約648msまで旧画素、最初のΩ COPY開始が約680ms後。Escapeはmodal消失後約739msまで残像、Ω送信後約515msまで旧画素。該当旧画素frameのsource/historyは既に入力後で、CancelはMain painted rev4/seq89、Escapeも最後はrev10/seq112へ進んでいた。source未処理だけの説明では足りず、child／screen／captureのどの段階かはUNKNOWN。sampling bracket外を補間しない。
- 単一Titlebar `20261021T035022Z-595dedafdd434318baf55cad8f6f8f97` はsame-owner・同bounds・同hit HWNDを確認したままWM_NCHITTESTが100ms timeout1460、結果UNKNOWN・close入力0件。別Cancel＋既知準備X Undo1回で原bytes／cleanを確認して通常終了exit0（`titlebar-cleanup.json`）。この1つの読取budgetを500msへ変更し、HTCLOSE20とpost-call生存／PID／thread／class／前面／同bounds／hit HWND再確認を維持した。actorの有限2秒budgetは変更しない。17正負例とstub compile、統合後の実C#全体compileを確認、実GUIは未実行。最新harness `3e4fc5ba809de4f66284fddc41460d28de28472f8d4c2b4476643c8d45e77d5a`。

HighはGit host経由WM_DRAWITEMの具体的な処理漏れを特定した。git_syncはApp借用中にshow/positionを行うが、borrowed fallbackにGit snapshotが無く、既存temporary OwnerButtonPaintはEnableWindow変化時だけで、enabled不変では使えない。`rev6-git-paint/phase-a-receipt.json` の単一serialized Debug native試験は、通常host draw=LRESULT1・暗背景・iconありに対し、借用中=LRESULT0・白背景・icon0・deferred=trueを実再現した（期待FAIL exit101、identity負例／文書保全成立）。これはscreenで同じcallbackが起きた直接traceではないが、独立した製品contract defectである。

Jev Finding `rev6-git-borrowed-paint` はround1 REFINE_REQUIRED。最初のrefinementは旧unknownsを落としたためERRORとして保存、元packetを保持して実再現factだけ追加しround2 `RESOLVED / fix_current`。Highが既存immutable paint cache方式で7 Git buttonをnative配置前にpublishする局所製品修正と関連検証／登録Release buildを担当する。親title／S02提示遅延を同じ原因と断定しない。実装中はHighがnative fixture／build outputsを独占しRootはGUIを停止する。ここから製品sourceが変わるため、旧EXE `de3087d9…` の証拠を新buildの描画修正成立へ流用しない。

### Rev6 Git局所修正後の最終候補（GUI確認中）

製品変更は `src/native.rs`、`src/native/git_ui.rs`、`src/native/shell.rs`。7 buttonのimmutable snapshotをshow/position前に公開し、既存identity／rendererを使用、staged themeとhost/Main破棄時の失効を維持した。元の赤試験はLRESULT0→1、白→暗背景、icon0→103pixel、deferred=true→falseへ変わり、文書は不変。実配置中の再入を含む関連native 9件がserialized PASS、strict Clippy／fmt／diffと登録Release buildがPASS（`rev6-git-paint/phase-b-receipt.json`、`build-release.json`）。RootはこのWorker検証を重複実行していない。

新source47 `085ed763e931c2e9a0f28caa007b97650ace4236501b33b4e10dd5ec0e2704e4`。
同じ登録絶対pathの新Release EXEはSHA256 `ead52724c2be966b965a20abfbf92f6c8c9f17f6bdc69125ac8a1467ae2b810e`。
Rootがこのhashを指定してGUI残確認を再開する。旧証拠の原文／mapping等の未変更部分と、新Git描画の実画面受入を区別する。親titleのD2D経路／S02遅延はなお別の未解消事項である。

Jev Review `rev6-final-review-selection` は `RESOLVED / sol61_high`。実装者・Rootとは別のHigh reviewerへ、実効権限は書込可能だが明示read-onlyで静的reviewを依頼した。reviewerは試験やGUIを実行せず、未確認受入を免除しない。Rootは新EXEのCompact／配置復元を直列実行中。

### Rev6 視覚判定の訂正と最終対象

保存済みPNGのSol直接表示audit（local `rev6-sol-visual-audit.md/json`）により、先行Luna注記を次の範囲で訂正する。4表示状態すべてで短文の本文は見えており、both-hiddenの「本文空白」は誤判定。both-shownの採取画像では親Outline/Git見出し等が見えず、4状態一括の視覚PASSにはしない。Escape frame013–033は、ghostのない033と全21枚同hashであり、「Settings残像が033まで続く／約0.74秒」の主張は撤回する。元注記と原画像は保持し、この訂正を優先する。

Cancel036→037、Escape033→034の本文末尾 `abcdefX`→`abcdefΩX` は直接確認済み。入力終了から旧文字画像COPY末尾まで約648ms／515ms、Ωの見えるCOPY開始まで約680ms／534msで、画面更新の遅れは残る。正確な提示時刻や原因はUNKNOWN。Source/Liveの4代表本文とEOF caretは確認済み、BOF caret glyphはUNKNOWN（Ln1Col1やbyte/focus成功と区別する）。

最終source47は `efed05695df9375e843006fdfbea111784ac772a74178408fd15576f7b87a909`、登録Release EXEは `15c3e041ff0102e38c51158e06c2507ee128a7291fbb220bdd975d3acfef0e94`。追加した親damage試験1件は借用中UpdateWindowから通常paintへのdamage保持を確認しただけで、実画面の見出し欠落原因解消とはしない。関連Git native9件＋親1件、strict Clippy/fmt/Release buildはWorker証拠を採用し重複実行しない。最終EXEの通常Git画像では暗背景とaction glyphをRootが確認したが、drag全場面へ拡張しない。

独立reviewのP2（Left group JSONが同じでもLeft visibility差分を見落とす）はsemantic delta全件拒否へ修正し、同reviewerの限定再確認で解消した。Compactはquery150クリック経由で実window生成まで到達したが、旧Mainの直前frameをCompactのforeign frameとして停止する検査不整合で入力／restart未達。厳密な同一process/thread/ownerとCOPY対象を保った局所待機修正を最終確認する。Densityの空配列がStrictModeでnullへunrollする既知のCount検査不整合も修正対象。過去のBLOCKEDは変更しない。

### Rev6 最終限定実行（harness 05957f22）

最終harnessは `05957f223d35f4d19f6de300dfd4c12a39326cbe8f8fd11c1d70893e24edceae`。Main→Compactの既知の同一owner frameだけをPENDINGとして待機し、COPYはexact Compactと前後identityを要求する。WM_CLOSE成功で対象が消滅した場合のみpost所有確認を終端成功とし、生存対象／他messageのguardを保持する。完了済み4表示frameとCompact前layoutを即時保存する。Density空配列の固定も含め、既存11反例をStrictMode LatestでPASS。Highの限定静的reviewは追加必須指摘なし。fixtureは実画面を代替しない。

`20261021T045505Z-d9a95265df4c46ac934c5c67ec6aeab9` のCompact最終実行は、遷移より前の `PALETTE_CONTROLS_UNAVAILABLE` でBLOCKED。修正した遷移の実受入・Compact文字入力／Undo／通常再起動は未到達。入力なし、正常WM_CLOSE exit0、残processなし。同じ入口の追加修理を拡大せず、成功した042041の部分画像・生成までの証拠と区別して残す。

`20261021T045602Z-584b85ef208641f99e848e226ffb89b3` はTitlebarCloseのClickChar代表が機能PASS。500msのexact HTCLOSE確認後の実クリック、次Ω入力・Undo／原文履歴／disk保全が成立し、通常終了exit0。run全体INCOMPLETEは他4caseのNOT_SELECTEDを含む。画面採取の視覚確認は別に行い、機能PASSで遅延を免除しない。

最終Density `20261021T045819Z-5cd51fa12b334f67bb4b9533503394fd` はCount不整合を通過し、Git通常240DIP／狭幅100DIPの独立密度oracleと採取、overflowの実popup native項目読取りまで成立。Rootが2PNGを直接表示し、本文・Git見出し・通常5glyphと狭幅Refresh＋Moreの暗背景を確認した。通常画像の残り画面は合成短文として正常。overflowを閉じるkeyboard直前に `OWNERSHIP_GATE: native menu keyboard target changed; no refocus/input sent` で停止したため、メニューaction完了やCalendar通常／狭幅／代表操作はNOT_RUN、suite PASSではない。通常終了exit0／残processなし。新しいmenu所有回避や検査基盤修理へ広げず、原結果を保持する。

### Rev6 収束判断

右の実メニュー5切替とLeft独立性、限定dragの原文／履歴／配置不変、Source/Live代表、U01機能、Git実描画とTitlebar次入力には部分成功がある。一方、親見出し／Outline消失、確認済みΩ画面更新遅延、drag全方向／内部高さ、4状態完全表示、Compact入力Undo／通常restart、Calendar密度到達性が残る。親damageの実native試験が正常でも画面欠落は解消しておらず、EndDraw／frame番号で画面を代替できない。既存frame採取と限定source調査で原因を特定できず、次の局所製品修正を裏付ける仮説は不足している。Main私用DCやCOMPOSITED除去の不採用案を再開せず、新capture／renderer基盤の改造も今回へ追加しない。

Taskの局所修理・既存代替経路を試した結果として **GROUNDED_INCOMPLETE** を採用し、実装を作業branchへcheckpoint配送する。develop `902dfd17754d45d782508c137c63950b2bc495c4` へ統合しない。原FAIL／画像と訂正、所有process通常終了、Rev6入力をHANDOFFへ残し、Inboxをcleanupしない。本人操作が必須という根拠はない。次の技術判断は、最終15c3の親表示欠落／Ω表示遅延について実画面提示までの経路を既存実装で再現できるか、同じ原証拠に基づき調査範囲を別途定めること。準備済みCompact遷移修正は実window入力まで未検証のまま引き継ぐ。

Titlebar視覚auditは `rev6-titlebar-visual.md/json`。44PNGを11hash群へ照合し5実画像を直接表示。modal消滅観測675.041msより前の012以降はclean Mainで、採取したpostmodal ghostはない。入力終了824.6467ms、Ωなしの032COPY末尾1502.7164ms（+678.0697）、Ωあり033COPY開始1524.009ms（+699.3623）。最終15c3にも次文字の画面更新遅れが残る。機能PASS、描画遅れ、正確な提示時刻UNKNOWNを分ける。

Closure wrapper初回は `GateKind Closure` の大文字をCLIが拒否しERROR。既存wrapperの小文字 `closure` に訂正した同判断ID `rev6-grounded-checkpoint-closure` は `RESOLVED / git_delivery`。未完了checkpoint配送の判断であり、develop統合／GUI受入の合格ではない。

### Rev8 開始時の証拠採否と担当

Rev8を登録Downloaderで取得・検証した（Task `20261018-mdlite-live-navigation-ui-git-polish`、receipt `720e86b0…`）。旧snapshot `aa494060…` はpreservedへ検証保存。Root session `01a15d45-5c50-7200-a7e6-baf27bd3f99c` / turn `01a15d45-8388-7de1-aa8f-af21050d5cb0` はexact runtimeのVERIFIED_ROOT、gpt-6.1-sol / medium。実効permissionはunrestricted、承認はneverであり、設定変更・権限昇格は行わない。

開始時worktreeはclean、HEADは `a8b70b10f7b7ed0e507eb58ab975004c224fb4e1`。local/origin/githubのtask branchは同OID、developはすべて `902dfd17754d45d782508c137c63950b2bc495c4`。登録Release EXEのraw SHA256 `15c3e041ff0102e38c51158e06c2507ee128a7291fbb220bdd975d3acfef0e94`、tools harnessのraw SHA256 `05957f223d35f4d19f6de300dfd4c12a39326cbe8f8fd11c1d70893e24edceae` は保存値と一致。担当変更のみのrebuild・全suite再実行はしない。

Rev8 reference REVIEW/EVIDENCE_MAP/REFERENCE_INDEXを採用し、Git owner-draw native RED/GREEN・限定glyph、右menu5切替と左右独立、原文/保存/履歴・Source/Live・Settings入力Undoは記録された範囲でREUSED。親title/Outline欠落を機能stateやdamage replayでPASSにしない。Ωの既存値はCOPY区間による観測であり設定後固有の製品不具合とは未確定。旧ghost/both-hidden blankの訂正を維持する。

Jev Initial `rev8-initial` はRESOLVED / sol_medium_specialist。親paint/layout/child/DC clipの限定調査はSol Medium、既存harnessの4状態受動採取・Ω少数対照・Compact/menu停止条件の準備はLuna Max、GUI/process/windowsはRootが直列所有する。最初の判別は同じ4表示操作の前・直後・追加入力なし後続画面を比較し、Read-Diagnosticのrefresh介入と分離する。必要修正後の残対象は左右幅/内部高さdrag、Compact入力Undo/Main復帰、通常restart配置、Calendar密度/代表操作、関連query/端点の不足画素。完了条件はRev8第6章のまま、現時点は未完了・develop未統合。

開始source47も47ファイル・`efed05695df9375e843006fdfbea111784ac772a74178408fd15576f7b87a909` で一致。限定診断buildはsource47 `3eae033170c7c502e7db398db8fa2cf45ef29b7c0cd7d053ceb19ce9af612467`、登録pathのEXE `fff7fcb67cf33c82ddd244e9b96f93f8ceb52f77ccf9d7f17d7c4297c5d1d315`。担当のcargo check/fmt/Release build PASSを採用し重複実行しない（`rev8-parent-paint/instrumentation-receipt.json`）。製品動作修正ではなく既存phases.jsonlへの限定観測追加。実geometry、BeginPaint damage/DC clip、title/Outline領域と重なる可視child、EndPaint後の結果を記録し、Outline/Git同時表示から最大32件・同geometry3件に限る。実画面受入は未実行。

Rev6 density停止の限定読取では、先行記録のmenu owner/captureはMainから0へ変わっていたが、throwした同じ呼出しのactivation記録は無く、原因UNKNOWN。次はguard判定に使った値と期待値をthrow前に保存するだけでguardを維持する。既存densityはGit→Calendarの連結なので、Calendarだけを同じ準備・判定で選択する局所経路が必要。旧runの後段NOT_RUNは保持する。

専用selector `REV8_TITLE_FOUR_STATE`（harness raw `d4ebc91d…`）は構文・既存前面限定の正負helper・selector順序/Compact前return確認がPASS。Rootの実run `20261021T072318Z-cbd73f64fb854880ba3bdc341cf3269f` は最初の配置reset palette入口で `PALETTE_CONTROLS_UNAVAILABLE` / BLOCKED。4状態も新probe対象配置も未到達。EXE/source/harness不変、final_proof_valid=trueはこの入口までの識別であって受入PASSではない。WM_CLOSE607ms＋exit待機61ms、exit0、process_retained=false。元resultを保持する。新selectorがCtrl+K経路を継承していたため、次は既にCompact生成に到達したowned query EDIT150クリックの既存経路を同selectorへ適用し、入口の差を判別する。guard/待機条件は維持し、同目的の試行をリセットしない。

query150既存経路を同selectorへ適用したharness `a88d9440…` は限定route正負チェック/Powershell構文/diff PASS（`rev8-harness-check/palette-entry-route-check.json`）。run `20261021T072556Z-1f0d71ce60a3458398359f2857e9387f` はpaletteより前の初期COPY guardでENV_BLOCKED。Main foreground HWND29757622、PID1972、client `[242,265,1326,1026]` に対し、別Qt HWND590410/PID15888のrect `[1318,111,1913,1142]` が8px重なりUNEXPECTED_OVERLAP。source/frame READYでもCOPY未送出。failure_message/stackとreadiness原記録を保持。WM_CLOSE830ms＋待機91ms、exit0、残processなし。製品FAILへ分類しない。新受入はまだ進んでいないため同位置での反復を止め、既存のowned window移動でclear領域へ置けるかへ方法を見直す。Jev Difficulty `rev8-occlusion-method` はRESOLVED / sol_medium_specialist、同Medium担当が既存移動手段と最小局所修理を限定判断。別window操作・guard緩和・全画面captureは行わない。

同Mediumの判断では既存ClickOwnedCaptionはcaption取得不能時だけ一時移動し最後に元位置へ戻すため、既にforegroundのMainの撮影重複を解消しない。RootはRev8 selectorだけの初回COPY前setupとして、同monitor work-area内へ同サイズの所有Mainを一度移動する局所helper拡張を採用。既存SetWindowPos/所有確認/有限矩形待ちを再利用し、NOACTIVATE/NOZORDER/NOSIZE、同PID/thread/HWND/class/creation/EXE・session/desktop・96DPIと前後矩形を要求する。入力やmodal/menu/captureがある場合・領域不足・移動不成立は停止。移動後も元の撮影重複guardを使い、別windowを動かさない。これは観測前のsetup介入であり、欠落の自然回復や受入の証拠ではない。

配置helperのfull inline C#compile、無効HWNDでmove_attempted=false、領域不足/正の矩形例、PS parse/Rev8限定経路はPASS。checkerのhere-string抽出とliteral照合の2エラーはchecker-onlyとしてreceiptへ保持し、成功済みcompile等を再実行せず不足照合だけ回収した（`rev8-harness-check/rev8-owned-placement-check.json`）。harness `b8f4beda…` の実run `20261021T074404Z-b3b82e72bc8344499cd86d655d0d1516` は所有Main `[104,104,1204,904]`→`[16,16,1116,816]` を546msで確認、同monitor work `[0,0,1920,1032]`・同一性/96DPI不変。初期client COPY `[24,47,1108,808]` が安全に成立した。配置の実動作証拠のみで製品受入へ加算しない。

同runはquery150クリック（owned child6491738、x612/y65、2events）後も最初のreset-command待機でPALETTE_CONTROLS_UNAVAILABLE。終了前のRead-Diagnosticを伴う別snapshotでは同childにfocus/caretがあり候補151はvisible/enabled/class=ListBoxだった。query_control_count=0は診断側のcase-sensitive `EDIT`比較であり、制御lookupのcase-insensitive比較とは違うためquery欠落の根拠にしない。4状態/probe対象には未到達、document/history不変、通常終了731ms＋待機50ms・exit0・残processなし。Jev Finding `rev8-palette-readiness` round1はREFINE_REQUIRED / gather_facts。同じ2500ms待機のどのpredicateが拒否したか、元lookupのexpected/actual/control/timeをrefresh前に保存する局所観測へ限定する。timeout延長・guard緩和・同一条件反復へは進まない。

lookupの元1回の列挙からexpected/actualを例外へ添付し、paletteは最初/最後の失敗だけ保存する変更（harness `73bd278a…`）を実施。PS parseと意味的な正負lookup/count・単一列挙確認はPASS。source文字列に依存したmatcherは検査器の誤りでERROR_NOT_ADOPTEDとし、文字列matcher修理を打切り、Rootが実差分のpredicate/budget/throw不変を確認した。C#再compileや旧試験の反復はしていない。証拠 `rev8-harness-check/palette-readiness-evidence-check.json`。

run `20261021T075225Z-d1962fc4e10046faa6c38454a83a470a` は選択した4状態caseの操作・原文/履歴/配置不変がPASS、通常終了exit0・残processなし・source/EXE/harness不変。最初のresetでresults151は520〜1665msに候補0件、その後1955msで自然にready。他のsetup commandは382〜930msでready、diagnostic refreshやbudget/input変更なし。過去の断続的palette停止原因は未確定のまま。Jev同Finding round2初回はunknownsを落としたpacketをERROR/facts_dropped_during_refinementとして保持し、元packet全項目を維持して追加factのみでround2へ進めた（REFINE_REQUIRED/gather_facts）。round3 NoMoreFactsはRESOLVED/investigate_root_cause。既存codeと今回同時点データから製品欠陥を裏付けられるか、Mediumへread-only限定判断を依頼し、追加反復で失敗を探さない。

4状態の画素は `rev8-parent-paint/four-state-correlation.md/json` を採用。追加入力/refresh前の受動204frameで、各状態は正しい見出し・Outline内容/関連child・本文へ自然追従した。click完了から最後の旧/混在COPY末尾→最初の正しいCOPY開始は、right-only 463.2→480.0ms、both-hidden 245.2→273.8ms、left-only 476.9→501.5ms、both-shown 477.1→496.6ms。処理/提示の厳密時間ではない。state3 frame005〜010は新childと旧親画素が混在し、011〜050は同hash `cbb1049aee2667316c4ffec32ef6dd32b9adeb804d1f3d7893c75131d7056794` の正しい両側表示。Rootの当初のまとめ表示の読違いによる「frame050も欠落」は単独ファイル表示/hashで訂正し、不採用とした。原画像/結果は不変。

23 probeはpaint成功・全面damage/clip bounding box、対象に重なる可視childなし。state3 sequence143〜145の実editor/Outline/Git矩形は正しく、非ゼロtitle/Outline行のRectVisible=true（全域可視の保証ではない）。mixed frame010→correct011は同sequence143で、EndDrawは画素提示を保証しないという旧限界を実証した。今回は2秒内の自然表示を確認したが、旧Rev6単一画像の同原因/持続時間、一般性能/全GUIの合格は断定しない。現時点ではtitleの製品patchを裏付けず、一時probeを除去し元の製品動作の最終buildへ戻して、Ωの少数対照と残drag/Compact/restart/Calendarを続ける。

一時probe151行だけを撤去しsrc/native.rs差分はゼロ、source47は元の `efed05695df9375e843006fdfbea111784ac772a74178408fd15576f7b87a909`。登録Releaseの再buildで実EXEは `2612e93d3ff9e406b5ffedc58007d7c97d9bc95287d4ef757f6b1c50fda3c18a`（旧15c3とは別の実hash）。fmt/check/build/hash読戻しは担当証拠を採用（`rev8-parent-paint/final-uninstrumented/`）、GUIはこの実hashをpinする。Mediumのpalette原因確認では、同期focus→host/list生成の実装と今回の自然readyを確認したが、旧失敗時点の実lookup不足からborrowed WM_COMMAND経路等への因果帰属はできず、追加修理/traceの根拠なし。titleのJev Finding `rev8-title-observation` はround1 REFINE_REQUIRED/gather_facts、最終buildの残確認とΩ対照の事実を得て同IDへ戻す。

最終buildの4状態試行 `20261021T080318Z-1ef1d7a0e8924b129fd936402e6ec614` はstartup captionの再hit-testが完了せずACTIVATION_POINT_CHANGED、caption入力0・capture0・case未到達。guardを維持して停止。PID12936/creation08:03:19.7548995Z/EXE2612をfresh照合した既存有限activation回復はMain foregroundを回復し、clean再確認から通常closeを呼んだ。helperはREV3_NORMAL_CLOSE_UNCERTAIN（send delivery未確認）のため正常close PASSにしない。同helperのこの例外はexit確認/exitcode0の後だけの分岐であり、fresh PID不在も確認、残processなし。send/wait個別値は旧cleanup wrapperがthrow後にexportしなかったためUNKNOWN。元result/cleanupは不変、別 `rev8-harness-check/final-target-activation-collection.json` に終了確認とWM_CLOSE送達不確実を分けて保存した。再close/強制終了はしていない。Ω selectorの独立準備を継続する。

### Rev8 最終buildのΩ少数対照

既存ClickChar actor/2秒samplerを共有し、Normalはmodal作成/終了だけ省略、TitlebarCloseは設定を変更しない終了とした。共通light-theme setupは1回、順序N/C・C/N・N/C、各rowで同source/caret/dirty/history/raw/diskを要求（revision増分は記録）。6証拠directoryは独立し既存pathがあれば停止する。checker側のmock出力収集とbyte期待値の誤りは別記し、orchestrationの成立部分と実source/selection/history oracleの正負PASSだけ採用。最後のdirectory guard後のPS parseはRootが不足範囲として実施。Rootが発見したselector→S01_S02とexpected-caseIDの不整合は1行修正し、旧S02のNOT_SELECTED集計へ戻してfinal parse PASS（harness `0f976040…`）。GUIはこれを固定した。

run `20261021T081709Z-7cdb580cfbe6449fbaf397e15647e8d0` はEXE2612・final_proof_valid=true、選択S01_S02_FORM_CLOSEの全6rowが機能PASS。suite INCOMPLETEは他4件NOT_SELECTEDによる。全rowの正位置Ω入力/原文・選択・disk保全とUndo2回による元source/clean/Undo0/Redo2を確認、通常終了exit0・残processなし。旧redo分岐はtypingで意図的に切られ、元branch完全保存とは主張しない。

`rev8-parent-paint/omega-comparison.md/json` に6条件の原文/時刻/COPY/画像/Undoを対応付けた。Unicode SendInput終了から最終no-Ω COPY末尾→最初Ω COPY開始は、pair1 Normal629.7→645.7ms/Close581.9→610.7ms、pair2 Normal498.2→517.9ms/Close386.7→406.2ms、pair3 Normal637.1→660.9ms/Close375.9→406.1ms。全6の最終画像は同hash `60fd2650aa4648e56e8a598269f5efcc190a3744d622bdd88cfaf86ef71b6aa9`、light Mainの`abcdefΩX`を直接確認（Rootも単独代表画像を確認）。最終no-Ωでも既にtyped source/選択5545/Undo2と同EndDraw番号があり、共通するsource→提示/採取の隔たりが残る。COPY費用約9〜31ms・最大開始gap約53〜68ms・16ms予定の欠落78〜82slot、host VM負荷はUNKNOWN。これらは採取境界であり処理時間ではない。

3組で設定終了後固有の追加停滞、入力消失、追加入力を要する回復、保存/Undo破損は再現せず、今回このための設定終了専用patchを裏付けない。一般入力性能や共通製品経路が無欠陥/原因は環境と確定したとは言わない。新しい閾値/全閉鎖matrix/大規模benchmarkは追加せず、この範囲のΩ確認を採用し残drag/Compact/restart/Calendar/Git対象操作へ進む。

### Rev8 残確認の独立経路とdrag停止

harness `22e0ca2306b392fa7b7fa8c9d1767855d857647ad09ff352ab955431fc1003ec` で3方向drag・Compact/restart・Calendar/Git密度の独立selectorを準備。最終raw copyを同hashで `rev8-harness-check/Invoke-RustGuiAcceptance-22e0…ps1` に保存。PS parse・case/Panel選択・same-call menu-owner拒否時のno-key正負確認を採用。Ω本体4関数はこのbatchで変更していない。実行時0f976のraw全体snapshotは未保存で、既存check領域にも無いため再構成せず、元run/hash・保存済み証拠とfunction範囲の変更履歴で採否を判断する。以後のfrozen rawは保存する。

run `20261021T083456Z-c2a249103ed740f0a896fd7d91843404` は最初のLeft幅dragの2番目waypoint（x304/y81）でOBSERVER_FAILURE、source/EXE/harness不変、通常終了exit0・残processなし。幅256/editor500には到達したがgesture全体PASSではない。拒否条件はsequenceのみ（要求65/実64）、doc/revision/source/selection/dirty/UndoRedo/scroll/geometry/self-frameは一致、COPY未開始。Medium調査でfloorはmax(pre-MOVE64+1,reached frame)、fresh診断fileのwrite08:36:08.2859833/read08:36:08.3120221も新geometry＋64。単純off-by-oneや古いfile読みと断定できず、traceのMain paint完了は約7.6245秒途切れていた。失敗時の実画素が無いので表示停滞か記録との乖離か未確定。元resultは不変。

同じ要求の失敗履歴を通算し、同方法の再実行やfloor削除は行わない。Jev Finding `rev8-held-frame` はround1 REFINE_REQUIRED/gather_facts。次の最小判別はsequenceだけ拒否された時に既存held所有/入力/crop guardを維持して診断専用COPYを保存する案であり、受入対象外・鮮度UNKNOWNと明示して元throw/PASS条件を維持する。実画素の旧/新で次の描画経路/記録経路調査を分けられる場合だけ実施する。Calendar/Git・Compact機能は独立経路で継続し、非default寸法restartはCompact機能の後の実resizeと保存値で確認する。

### Rev8 Calendar/Git・Compactとheld dragの原因絞込み

Calendar固定18DIP行はwrap可能な本文描画を使い、100DIP幅で長いfilenameが次行へ重なった。既存single-line描画へcallerだけ変更（features.rs、font/文字列/操作維持）。fmt/check/strict Clippy/登録Release PASSを担当receiptから採用。source47 `1c958fb1bd5d051f98d825e35cad62c090ffd14b8a542da29c66936c72e63310`、EXE `fea119e8f780417aa1e7e9d6c9dea6939a246e504a4b4c4576b804a99d42a250`。run `091027Z-3c2610c183de466a934c76282ddfb2f2` の240/100DIP実PNGで重なり解消、月往復/原文/履歴を確認した。結果収集のPSCustomObject追加property例外は別修正。既保存のcoherent原/終bytesとclean preclose診断からexact config復元 `c99d76c8…` を回収し `calendar-collection-recovery.json` に記録。原FAILをPASSへ書換えず複合証拠として採用、通常終了exit0。

Git run `091513Z-e953be4f563d4288a9603b1c29df78d7` はEXEfea/harness1a4、選択U02/U03 PASS（他NOT_SELECTED、suite INCOMPLETE）。通常240/狭幅100DIP画素、代表overflow表示/Escape1、config exact復元、原文/履歴を確認。same-call menu expected/actual owner=8785700、終了後owner0・focus12259392、通常close0。旧menu停止の原因確定やguard回避ではない。

Compact run `091824Z-0b4962089b2d4dd5aed38117ed72541a` はEXEfea/harness1a4。4状態受動画素/原文/配置に加え、実Compact X入力→Undo→Main復帰と配置維持が `rev5_panel_proof.compact` に保存された。その後非default配置用の最初のLeft resizeで要求frame183/実182のみ拒否、restart未到達。新しい診断専用COPYは受入不可/鮮度UNKNOWNのまま、他11条件とheld所有/cropを維持して保存した。実親divider287/288/editor開始308は旧幅240、model幅248/editor開始316と異なる。frame182が約7.4秒維持され、trace7.631558秒に252diagnosticと248layout.rebuild、全thread停止ではない。4状態終端の読違いは単独path/hashで訂正し正しい表示を採用、原画像不変。通常終了exit0。

一時probeは既存phase channelにheld moveの変更/不変累計を限定記録。read-only GetUpdateRect(FALSE)を失敗COPY直前/後へ追加し、floor/guard/throwを維持。source47 `f74e78eadeadf6bd2c2615f12a1aa994f4579c8f89e2c8c2c741001ed301e899`、probe EXE `7eb02e41bfbceb83d6b193b2ef86bc1e4b53606b9b3810b448ef86725266c739`。先行run093428はpalette151未生成が2975msまで続きdrag未到達。新Rev5の2独立selectorだけ既存TimeoutMs=7000のcontrols readinessへ揃え、focus/text/入力/判定は維持（最終harness `138e01b0c37ec8aae49f44a3b8661f483e70a1965d52e65713027c342f5c5222`）。

run `093855Z-c13da5385b6344758373163bdf0d9a99` は同probe/harness138。最初の幅248で同frame拒否、phaseにmove128/changed1/unchanged127、同x296/y81を記録。09:40:00.152/00.179のCOPY直前/後ともMain full1084x761のpending update=true。damage消失より、同寸法moveが継続する間の低優先paint dispatch停滞へ絞れた。通常終了exit0、残owned processなし。実behavior修正はまだなし。Jev Difficulty `rev8-held-producer` round2はREFINE_REQUIRED/sol_medium_specialist、Mediumがsame-dimension refreshとnative子配置の関係を限定読取中。暫定probeは最終製品へ残さない。未達は全方向/内部height dragと非default9-doc restartであり、develop未統合。

Jev held-producer round3初回はRefinedPacketとNoMoreFactsを同時指定しsemantic_round_plan_mismatch ERROR。既存round2 packetにNoMoreFactsだけで同ID/round3を正しく再提出し、RESOLVED/sol_medium_specialistを得た。API semantic roundの新設や上限迂回なし。Mediumに実stored値の差分だけでrefreshを予約するwidth/height局所修正・既存native fixture・登録Release buildを一括委譲した。RootはGUIを占有せず、最終EXEで残drag/restartを直列実行する。

Query/candidateの不足画素は091824の `rev8-compact-palette-initial.png` をRootが単独直接表示して補完した。query `Compactへ移す` と候補2行が読め、選択済み先頭行と別候補が区別できる。これは関連palette/Compact場面の表示確認で、Workspace検索全体の再試験ではない。旧033154のBOF Source44/Live53受動frameも限定確認し、保存caret_rect内にblink差分がなく代表画像でもglyphを確信できないためUNKNOWNを維持する。今回のsplitter同値更新gateは編集caret生成/表示・Source/Live切替を変更せず、旧端点の機能/原文/history/diskはその条件でREUSED。BOF glyphをstatus値だけでPASSへ変更せず、旧全IDを今回完了gateへ復活させない。

最終製品修正はshell_mouse_moveで実stored幅／上下2height weightsを比較し、実変更時だけpresentation_changedを呼ぶ。active splitterの同値moveは従来通り消費済みtrue、hover/capture/enddrag/saveを保持。一時計測は撤去しnative.rs差分ゼロ。既存splitter native fixtureに実COM layoutの変更/保持（同座標・100/600幅clamp・heightclamp）とsource/revision/dirty/UndoRedo/selection/capture不変を追加しPASS、変更heightで実layout再構築も要求する。fmt/check/strictClippy/Release PASSとPonytail自己確認を担当receiptから採用し再実行しない。`rev8-held-fix/fix-receipt.json`、source47 `fed94d3c5dddf5b1dd5e370b5d1fb3ea15c13472d86f61667a430ee341b6ee14`、登録最終EXE `9a89bc48b64f43313661dd409c143d6b287b67a087961ee38c8075bbd17bd9bf`。build JSONのfmt/clippy/test NOT_RUNはVerify=falseによるもので、別の直接実行log/receiptを根拠にする。実画面の停滞解消はまだ未確認。

Jev Review `rev8-final-review` はRESOLVED/sol61_high。独立Sol Highへ実装・native assertion・harness判定/安全性の最終静的reviewを依頼した。実効shared roleは書込可能だが依頼をread-onlyに限定し、reviewerはtest/build/GUIを実行しない。Rootだけが最終9a89/ハーネス138でdrag→Compact/restartを直列確認する。

最終drag `095047Z-ece6d64afad34549b7163b54bd4f2eb6` は機能PASS/final_proof_valid/normalclose0だが、実画素全点PASSではない。Mediumの14画像単独監査 `rev8-held-fix/drag-pixel-audit.md/json` で、即時width/height点は前waypoint寸法、Rightには部分child混在を確認。既存2秒held repeatのLeft256/Right232は追加入力/refreshなしで正しい寸法へ追従し、旧7.7秒frame停止は再現しない。高さも押下中に進むが最後のheld端点は未採取、settled264/216/374はrelease後ReadDiagnostic refreshあり。原resultと画像を保持し、機能成功と自然終端表示の不足を分離する。

独立reviewの必須P2は2件。Panel省略時にstring nullが空文字へ束縛されるのにnull判定したdensity filterをIsNullOrWhiteSpaceへ修正し、実束縛/選択3条件PASS。restart高さは保存TOML hashだけでなくruntime panel_rects比較と実画面を要求する。旧BOF glyph UNKNOWNをPASSへ変えない。他に製品差分/安全性findingなし。reviewer自身の実行はゼロ。別に実観測されたrectangle arrayのcomma演算優先順位エラーも各差分を括弧で修正、実RHS4座標条件/PS parse PASS。最終harness `04636c59cac2def056a6f01ab984f0e495a09a4bd52b158005ee7284fdea2590` のraw/receipt保存。

Compact run `095239Z-b70c0d63d03e486a99b23d29f29bd5fc` は実X/Undo/Main、非default3dimension実resize、freshprocess9tabsまで到達したが、上記arrayエラーで残9文書照合/復元画像に未到達。再試行 `095618Z-b8c3329cc32348eb9107abb62db64617` は最初のresizeCOPY中にframe176→177、source/geometry/history/所有不変でも同frame条件により画像破棄・停止した。両方normalcloseexit0/sourceEXE/harness不変、原FAIL保持。同じ要求で2 correction roundを通算し、同じ即時COPYの盲目的再試行を打切り、観測方法を再評価した。Rootは既存2秒held待機をCOPY前へ適用する最小案をMediumへ限定判断依頼し、自然追従/持続停滞を判別できる場合だけ続ける。製品patchやfloor削除・captureguard緩和は追加しない。

Jev held-frame round3はRESOLVED/investigate_root_cause。producer-discriminatorでdamage保持/同値move反復を突き止めたMedium laneを継続し、残る提示/採取の時差を既存観測で分ける。担当変更・helper改名による試行履歴リセットなし。

Mediumは撮影方法の再評価で、新Rev8の2selectorだけgeometry/floor成立後・元copyDeadline内に2秒の受動held待機を置く案を妥当と判断した。毎周の既存held所有/capture/button検査、元deadline超過停止、元Captureのframe/source/history/geometry/crop guardを保持し、入力・refocus・ReadDiagnostic・paint要求・deadline更新・retryを追加しない。2秒経過を合格とはせず、待機後の実PNGが期待寸法かを確認する。旧追加2秒repeatは新2caseだけ抑制し重複4秒待機を避ける。旧rawは保持。既存restart-only入口はなく、失敗fixtureからbaselineを再構築する方が大きいため採用しない。

Lowが同変更を実施し、PS parseと実AST predicate20条件をPASS、raw一致を保存。最終ハーネス `fa0a6fc08cf04ed441b83335ae2290bd7504e1531307c1a6645601005d6af08d`、receipt `rev8-harness-check/passive-held-before-copy-fa0…json`。同じ独立reviewerへこの安定差分だけread-only再確認を依頼し、全reviewや試験を繰り返さない。実最終buildの必要drag/restartはこの観測修理で不足だけを続ける。

同じ独立SolHighの安定harness fa0限定追認は追加blockerなし。priorPanel P2修正、array括弧、元deadlineと全held/COPYguard維持を静的確認。実行なし。restart runtimeheight比較は未解消のままRootが担当する。

最終9a89の4状態は095239の各受動終端が既存091824と同hash（right-only a0489245、both-hidden72a8449b、left-only377a2f98、both-shown ea087ad8）。Lunaのboth-hidden blank判定はRootが exact State1/frame049 を単独original表示し、UI heading A/Bとalpha/beta本文の可視を確認して訂正した。原画像/hash不変、当該判定は不採用。left-onlyにOutline行があるとの記載も状態契約に反するため採用しない。最終Compactのquery/candidate可読とX実glyphを確認、Undo/Main復帰の機能receiptと対応付ける。撮影系の単独表示で訂正した視覚誤判定を製品findingへ昇格しない。

2秒受動hold後のrun `100249Z-63fa27e13a784f06942f34b83a6323cc`（9a89/fa0）は最初のLeft width248でCOPY中frame74→75となり、画像破棄・元throwでFAIL。source/geometry/history/所有は同一、実hold2004ms、normalcloseexit0/final_proof_valid/sourceEXEハーネス不変。これで2秒経過を表示/同frame保証へ読み替えられないことが実証された。同じ待機条件の再実行はしない。Mediumへpaint cadence/observerの有無をraw traceで限定判別する判断を返し、独立restartは既存no-HeldWaypointsの実ドラッグ経路で非default設定準備だけを行えるか分けて検討する。held実画素の不足は残し、restart達成で黙って全dragをPASSにしない。

Mediumの100249 raw分析ではpaint完了は約148〜277ms間隔で続き、該当区間にlayout.rebuildはない。今回COPY14msとbeforestamp読出しの間に74→75が進んだ。観測helperはfile読取/GetUpdateRect(FALSE)/metadata確認で、明示的paint/refresh呼出しなし。周期producerはtraceにmessage種別がなくUNKNOWN。旧7.7秒のlayout連打とは分け、根拠のない製品patchを追加しない。

次は同じ待機条件の反復ではなく、既存heldCOPY内の限定再観測をLunaMaxへ委譲した。新2Rev8selectorだけ、両stampがbaseline/source/doc/revision/selection/history/scroll/reachedgeometry/floor/Mainを個別に満たし、held/crop所有が前後safeで、唯一の差が同Mainのsequence単調増加の場合だけfailedbitmapを破棄し元deadline内で次COPYを観測する。欠落/退行/foreign/source/geometry/history/所有など他差分は元FAILへ直行し、frame不一致の画像は採用しない。採用には従来の同frame条件とRoot実PNG確認が必要。元期限を延ばす・catch-all retry・入力・refocus・ReadDiagnostic・強制paintは禁止。

独立restartのため、Compactの非default準備だけは既存no-HeldWaypointsの実down/endpointmove/upを使い、文書/反対側/最終geometry/保存bytes/capture解放のoracleを維持する案も同batchで実装する。setupをSEMANTIC_PERSISTENCE_ONLYと明示し、専用dragの3waypoint実画面要件は維持。新resume loaderや設定ファイル直書きは使わない。独立restartが成功しても未達held視覚をPASSへ混ぜない。変更後は独立reviewerの限定静的確認を経て不足GUIだけを実行する。

Title Finding `rev8-title-observation` はfinal9a89の自然4状態画素・Compact次入力と6Ω対照を追加してround2 REFINE_REQUIRED/gather_facts、同packet NoMoreFactsのround3はRESOLVED/defer。defer対象は再現/原因根拠のない追加title renderer patchと旧単一snapshotの原因追究であり、必須の現在4状態実画素確認を延期/免除した意味ではない。現在の自然追従は原証拠/最終build画素の有効範囲で採用し、歴史的瞬間の原因・持続時間はUNKNOWNとして保持する。

LunaMaxは限定再観測とCompact setup分離の差分を保存したが、初回checkerを旧Windows PowerShell5で起動して実行policy拒否を受け、迂回せずRootへ返した。Rootが通常taskホストのPowerShell7.6.7/existing RemoteSignedをread-only確認し、設定変更/Bypassなしで同checkerを通常実行した。PS parse PASSだがstrict sequence-advance正例がfalseとなりFAIL（c954173b…、native/GUI/build実行なし）。旧hostの拒否とpure predicate正例失敗を別に記録。Rootは具体的な型/shapeまたは実装原因の判別と狭い修理だけをSolLowへ委譲し、失敗checkerを回し続けたりadmissionを緩めたりしない。製品source/EXEはfed94/9a89で不変。

SolLowは正例FAILを新実装のownership membership判定へ特定した。nativeとmockのGeneric.DictionaryはIDictionaryを実装するが、PowerShellからの `.Contains('safe')` は1引数overloadを持たず、catchがfalseを返していた。self/waypoint/baseline/sequence orderは正例で成立。Dictionary Keysのcase-sensitive membershipへ局所修正し、missing/ownership等の拒否を維持してcheckerを確認する。期待値をfalseへ変えて合わせない。

有限再観測のfinal harnessは `abf29edd7ecb7c63d3e78ea264b452f35ee13445dc9aa489a40b3bfdb056fd70`。新2selectorだけ、frame単調増加のdiscard後1回再観測（元期限内、fresh-before即一致/順序検査）。採用は元の同frame条件のみ、失敗bitmap保存なし。他変化/欠落/所有不一致は即FAIL。Compactの3setup呼出しだけ既存zero-held actorを使用しSEMANTIC_PERSISTENCE_ONLYと記録、専用dragのheld要件不変。

通常PowerShell7のfocused checkerはparse、21 unique admission正負、18 actor、1 selectorをPASS。Dictionary membershipの実装不具合を局所修正した後、mockのif出力が単一boolへunwrapされ `.Count` を失うchecker不具合も発見・明示修正した（HeldWaypoints defaultは実装と同じ@()で正しかった）。元2FAIL・c954/abf29 rawを保持。期待値の緩和やGUI/native/build実行なし。deadline/bitmap保存経路は静的review範囲とし、同独立reviewerへ安定新差分だけ確認依頼した。

新helperの独立静的reviewはdeadlineの必須P2を1件指摘した。COPY開始前だけの検査では、post-observationが元期限を超えて同frame保存を許すため、最終held所有確認の後・保存採用の直前に単一admission UTCを取得し、元Deadline以上ならDEADLINE_EXHAUSTED_AFTER_COPY/元OBSERVER_FAILUREで未保存・非eligibleとして破棄する。valid時はadmission/deadline UTCを記録。予算はnativeCOPY＋post-observation/admissionまでで、確定bitmapのPNG永続化はその観測window外と明示した。元deadline/retry回数/floorに変更なし。

最終harness `95372d0204ae7f55d81ec065e5ee3189b095cc1f706cd04a6034b173a81704b3`、actual AST期限guardのpast/future正負とPSparse PASS、wholecapturebody未実行を明記。別SolHighの同patch限定確認でP2解消・追加blockerなし。全review/test再実行なし。Rootは独立restartを先に、次に専用helddragを同9a89/9537で直列実行し、欠けているruntime高さ/画素だけを確認する。

### Rev8 最終held drag受入

`103214Z-7d462feaed08427e9d718436ebcf380a` は9a89/9537で選択case PASS、final_proof_valid=true、通常closeexit0、source/EXE/harness不変。Rootのadmission回収 `rev8-held-fix/final-held-admission-collection.json` は全9点の採用1件/元deadline内admission/eligible=trueと、破棄3件のpath=null/eligible=falseを確認。Right4/Height7/8はframeだけ増加した最初のbitmapを未保存で破棄し、2回目は元の同frame/source/history/geometry/所有条件を満たした。期限延長や進行frameの採用はない。

Mediumが9held＋3settledの実PNGを個別表示して独立要求geometryと照合しPASS（`final-drag-pixel-audit.md/json`）。実Left248/256/264、Right232/224/216、Explorerheight358/366/374・Calendarheader394/402/410が一致。見出し/Outline A+B/本文/反対側/native buttonsは保全、旧Git partial glyph residueなし。Rootも最終height8の正しい374/header410を単独original画像で直接確認した。全before/after条件・floor・所有・COPY終端/後stampも元期限内。これは2秒受動hold後の疎な押下中表示の確認で、即時表示/全frame/連続drag性能保証とはしない。原095047の一時的な前geometry/混在や旧7.7秒FAILは保持する。

独立Compact/restart `102907Z-9911f0ddb6bd4daaab369582bddea724` はSEMANTIC_PERSISTENCE_ONLYの実3寸法変更・通常close・fresh9tabs/placementに到達後、全UiDocumentSources.Keys（9初期＋205QuickOpen用file）を9tabs内で探す既存loop誤りによりSYNTHETIC_TAB_NOT_FOUND。初期9文書が正本なのでUiInitialDocPathsだけを照合する局所修理を行い、214mapから9unique expected source・MaxTabs9/clean-historyを保つactualAST fixture PASS。restart runtime geometryはclose後のrawdiagでparent_root=null、precloseSnapshotはsource/history subsetだけであり、回収だけではP2を閉じられなかった。新run用に既存membership/config assert後のfresh.parent_rootをruntime_layout_after_restartへ保持する1fieldを追加した。新expected値を製品出力から作っていない。

最終harness `4b057495c8886e5628c5c204df0169489d9f9d857fb44c790b640ea5ca5140dc` の2局所変更を同独立Highが静的確認し追加blockerなし。専用drag経路は9537から不変なので上記proofは有効。Rootは同9a89/4b05でCompact/restart最終確認を実行中、runtime高さ比較・画像はその結果で判断する。

### Rev8 通常9文書restart・範囲完了判断

`103524Z-cada814c999e4476b28e69abab69b156` は9a89/4b05で選択REV8_COMPACT_RESTART PASS、final_proof_valid=true/sourceEXEハーネス不変、通常closeexit0。Compact実X→Undo→Main保全から、実非default3寸法setup、clean確認・通常close、fresh process（10:35:26.2975624Z→10:36:27.1152354Z）で9tabs起動、全9expected source/clean Undo0Redo0、左右membership/config hashの照合を完了。setupの非held経路をheld視覚受入へ代用していない（専用103214で別証明）。

Rootがruntime_layout_after_restartをlast preclose drag.afterとid別に比較し、同client1084×761/96DPI、editor/active_editor矩形、左右表示、group順序・所属・active・width、panel flagsとbody/rectの全x/y/width/heightがexact一致。Left264/Right216、Explorer374/Calendar327・header410の非default高さ復元をsavedTOMLだけでなく実runtimeで確認。最終通常close後も9文書bytesとlayout hash25fb50e8…が前値と一致。`rev8-harness-check/restart-runtime-geometry-collection.json` PASS。Rootが `rev5-restarted-panels.png` を単独original表示し同寸法・見出し/Outline A+B/本文/native controlsの正常表示を確認した。独立reviewのruntime-height evidence P2をこの複合証拠で解消した。

必須残件は現在対象へのPASSまたはTask許可の有効REUSEDがそろった。Calendar固定行label/同値splitter連打を局所修正、最終4状態/次入力/全3方向held画素/Compact/通常9文書非default復元/Calendar・Git密度操作/Ω少数比較を採用する。旧source/保存/history/右menu/Source-Live等は変更影響条件で再利用する。BOF glyph/物理IME/Human/旧起動白画面等の無関係な歴史的不足は残し、今回変更はそれらの生成/描画経路を変えていない。全製品/全DPI/全frame/一般応答時間の合格とはしない。

同値move gateは実変更時の既存presentation_changedを保持し、共有renderer・input/caret・Markdown/source/history処理を変えていないため、旧端点機能/Ω比較/Settings/save/history成功を一括失効させない。最新2harness差分はCompact内部loop/receiptだけで専用helddrag経路不変、9537の最終drag証拠は4b05にも有効。独立静的reviewの必要指摘は全解消、Ponytail自己確認と区別。最終integrity/Git Closure判断後、通常history-preserving統合/配送を行う。

Jev Closure `rev8-complete-closure` はRESOLVED/git_delivery（confidence0.94）。現Taskの必須engineering/受入と独立review完了に基づく通常Git配送判断であり、配送済みの証拠ではない。finaldiffcheck PASS、開始TaskHEADa8b70b1/develop902dfd1と両remoteの対応、delivery worktree cleanをfresh確認。4対象pathだけをcommitし、通常no-ff統合後は同treeを確認してから既存origin/githubへ通常push/OID読戻しする。

### Rev8 配送結果の記録

製品/受入記録commit `5e58a8e2df6853e5eb7e586caa7de203a7c1b2e4` を通常merge `c3779b48dceceb6416e19ac470bc0f81c804dfc5` でdevelopへ統合した。task/develop両tree `cab22e4f97a43c68415214d75454167cc08d6036` 一致により統合で製品/build条件は変わらず、再build/GUI再実行を追加しない。origin/github両方へtaskbranch/developを通常pushし、両remoteで同OIDを読戻した。main/archive/tag/Releaseは対象外で不変。

登録generatorでHANDOFFを生成しRev8入力をarchive、登録uploaderの初回private Drive保存と3filesのダウンロードSHA-256読戻しを完了した（status complete）。必要なGitBook差分は既存CR7の3ページへ追補し、exact markdown/旧suffix保持を読戻し、draftを維持した。配送URI/詳細receipt/原証拠はlocal HANDOFFに保管し、private Task入力やlocal evidence自体はGitへ追跡しない。

本追記は配送実結果を閉じる文書だけの更新で、検証済み製品source47 fed94d3c…・EXE9a89bc48…・harness4b057495…は不変。最終文書配送のOID/最終HANDOFF読戻しは同HANDOFFのdelivery receiptを正本とする。Task Inbox整理は最終Drive保存/readback後だけ実行する。全製品/物理IME/Human受入や稼働中の通常実行体更新を配送成功へ含めない。
