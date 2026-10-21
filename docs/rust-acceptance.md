# Rust Editor受入記録

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