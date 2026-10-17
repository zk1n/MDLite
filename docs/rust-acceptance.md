# Rust Editor受入記録

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
