# Rust編集基盤の設計

Windows 11 x64用。Rustをdevelopの開発主線とする。
旧実装は保管branchとGit履歴に保持し、現行treeはRust専用とする。
Markdown原文を永続正本とし、表示は派生データとする。

| 責務 | 使用する部品 | MDLiteで実装する部分 |
|---|---|---|
| ウィンドウ、入力、clipboard | Win32 / `windows` | 文書の所有、コマンド、入力優先順位 |
| 文字整形、font fallback | DirectWrite | ソース範囲への対応、表配置 |
| 描画 | Direct2D software HWND target | 一つの編集面、選択、キャレット |
| 日本語入力 | Windows IMM | 未確定状態と確定transaction、候補位置 |
| Markdown | pulldown-cmark | 編集可能範囲、未完成構文、原文保持 |
| Raster/SVG | image、resvg/usvg | ローカルのみ、安全検査、保存後の挿入 |
| 本文・履歴・Unicode境界 | crop 0.4.3 (`utf16-metric`, `graphemes`) | checked byte/UTF-16 API、source/view対応 |
| 派生表示・未確定文字の境界 | unicode-segmentation | Markdownで隠す記号・entity・表示UTF-16との対応 |
| 保存 | Windows CP932変換、ファイル置換 | BOM、改行、外部変更、失敗保護 |

## 選定

Win32とDirectWrite/Direct2Dを選ぶ。OSの文字整形と同じlayoutを描画、
hit test、selection、IME候補位置に使い、独自の文字幅推測を避ける。
`windows` 0.62.2のライセンスはMIT/Apache-2.0、MSRVは1.82。
採用版はCargo.lockを正本とし、Rust 1.99.1の既存mise環境を使う。

このVMでのrenderer-only controlled A/Bは、instrumented diagnostic
binariesで実施し、canonical product EXEは使っていない。Direct2Dの
DEFAULT target作成は約50MBの追加private memoryを確保した。同じ文書・
文字整形・IME設定で比較したSOFTWARE targetは、待機中Working Setが
32,014,336 bytesで、private memoryはDEFAULTより50,065,408 bytes少なかった。
これはrenderer targetの診断値であり、product memory baselineやbudget PASS
ではない。描画方式をsoftwareへ固定し、同じDirectWrite layoutと画像・
選択処理を使う。初回Rust版のcanonical Release EXE focused runでは1／6文書の
Working Setが固定50,000,000-byte目標を超えてFAILしているため、その値は
受入記録に残す。他のGPU／VMで同じ値になるとは仮定せず、GPUメモリと
完成製品の総資源は今回の測定対象外とする。

eguiのTextEditは通常のtext buffer向けで、sourceに対応する可変高の
表・画像を直接扱う編集基盤を肩代わりしない。今回必要な編集面を自作
する点は残り、描画・文字整形の別stackを追加するため選ばなかった。
Rustで書かれた既存editorアプリを、そのまま組込みlibraryとは扱わない。

[windows公開版manifest](https://docs.rs/crate/windows/0.62.2/source/Cargo.toml)、
[DirectWrite hit test](https://microsoft.github.io/windows-docs-rs/doc/windows/Win32/Graphics/DirectWrite/struct.IDWriteTextLayout.html)、
[egui TextEdit](https://docs.rs/egui/latest/egui/widgets/text_edit/struct.TextEdit.html)。

## Workspace ownership and external execution

`WorkspaceIdentity` combines the canonical root with physical directory identity. `instance::claim`
holds a local named-mutex lease in `Persistence`; a duplicate activation targets only a window whose
canonical executable matches `current_exe`. Opening a different Workspace starts that same executable
with the selected root. The lease itself writes no Workspace files. See [Workspace operations](workspace-operations.md)
for state ownership and [implementation status](ui-feature-parity.md) for source/build slices.

Git and storage adapters use `ProcessRequest`/`process::run` for explicit trusted operations. Windows
children enter an owned Job Object before resume; request timeout, cancellation, and per-stream retained
output are bounded. This execution API serves those operations without adding an arbitrary-process UI.
GUI, real storage service, and physical-input acceptance remain in the acceptance record.

Gitの明示的なTrust付与／再試行は、通常のrepository discoveryで親repo・worktreeを
優先し、repoなしを直前に再確認できた場合だけWorkspace rootをlocal initする。
Trust登録とinitは別の結果であり、init失敗でもTrustと編集状態を保持する。
通常のWorkspace切替は世代を更新し、信頼済みの場合だけ読取りのStatusを予約する。
表示pathだけprefix／区切りを整え、process引数のcanonical pathは変更しない。

## 適合を実アプリで確認する境界

IMMは未確定文字を原文・履歴から分離し、確定文字だけを一操作で適用する。
実MS IMEの候補位置、取消、再変換はsynthetic composition試験だけでは
証明できない。必要ならTSF text storeの導入を検討するが、二つの入力
writerを同時に有効化しない。custom viewのNarrator対応は別途実証が必要。

[IMM処理](https://github.com/MicrosoftDocs/win32/blob/docs/desktop-src/Intl/processing-the-wm-ime-composition-message.md)、
[TSF text store](https://learn.microsoft.com/en-us/windows/win32/api/textstor/nn-textstor-itextstoreacp)。

parserのsource offsetsだけではescapeやentity、隠れた記号の完全な編集
mappingにならない。表示と原文の対応を明示的に保持し、不正構文は原文を
壊さずliteral表示へ戻す。全ファイルの再serializeは行わない。

表操作は読み取り専用で確認した`zk1n/vscode-markdown-live-editor`の
ローカルrevision `198d09df867d1cdf6283f168100d0540f42b026a`を参照した。
`tableSource.ts`と`tableEditing.ts`の操作契約をRustへ実装し、TypeScriptや
JavaScriptのランタイム／ソースを製品へ取り込んでいない。Tab／Enterは
移動先内容を選択し、末尾では一操作で行を追加する。Ctrl+Enter／Escの
明示退出と、原文を変えない矢印移動を区別する。

SVGはscript、外部参照、埋込みHTMLを拒否する。usvgの外部画像resolverを
無効にし、remote imageやfontを取得しない。画像のデコード予算と閉じた
viewの解放を管理する。CP932はWindowsの変換を使いbest-fitを拒否し、
表示変更だけで原bytesを変えない。

[pulldown-cmark](https://docs.rs/pulldown-cmark/latest/pulldown_cmark/)、
[usvg resolver](https://docs.rs/usvg/0.45.1/usvg/struct.ImageHrefResolver.html)、
[Windows文字変換](https://learn.microsoft.com/en-us/windows/win32/api/stringapiset/nf-stringapiset-widechartomultibyte)。

旧測定の50,000,000 bytes目標は履歴として保持する。今回の望ましい目安を薄いshellの値で完成製品の保証に
しない。GUI証拠と未検証範囲は受入記録に残す。


## crop本文と派生データの所有

`Document::source`の編集正本は`crop::Rope`一つである。挿入・削除・置換は
checked byte rangeを検証し、COW candidateへ適用してselectionも検証した後だけ
公開する。不正境界・範囲・selectionやrevision枯渇では本文、dirty、履歴は変わらない。
履歴はbefore/after Rope snapshot、selectionと境界affinity、saved-state identityを保持し、Undo/Redoは
共有treeの切替を行う。全文Stringによる第二編集engineやdelta文字列の再適用はない。
Undo後の編集でRedoを破棄し、saved-stateへ戻るとcleanになる。履歴は文書closeで解放し、
現時点では履歴件数に上限を設けない。cropの共有nodeの正確な保持bytesは計測していない。

`rope()`とchecked `slice()`は参照寿命で文書をborrowし、編集との同時使用を防ぐ。
`snapshot()`はDocument ID/revisionとCOW Ropeを所有するread-only snapshotである。
source byteはUTF-8 scalar境界を検証し、UTF-16変換はcrop metricを利用して
サロゲート途中を拒否する。raw移動・削除、native hit位置と表外へのsnapはcropの
chunkをまたぐgrapheme APIを使う。表示で隠すMarkdownやentityの対応はMDLiteの
source-affine tokenに残す。普通の表示graphemeはsource rangeからspellingを借り、
一つのrangeはsmallvecのinline領域に保持する。変換されたentity、escape、分割された
style間graphemeのみ短い表示Stringを持つ。

crop 0.4.3の`Rope::graphemes()`にはregional-indicatorのcontextを失う既知の
再現がある。`a🇦🇧🇨🇩🇪z`で`a / 🇦🇧 / 🇨 / 🇩🇪 / z`を返すが、Unicode標準の
境界は`a / 🇦🇧 / 🇨🇩 / 🇪 / z`である。このiteratorはsource navigationに使用しない。
同版の`Rope::is_grapheme_boundary`はoriginal chunks/contextから判定し、RI列、
chunk跨ぎ、ZWJ、異体字セレクター、結合文字、CRLFのfixtureで正しい境界を確認する。
MDLiteの小さなprevious/next wrapperは、このpredicateが真になる最寄りscalar境界を
探すだけで、Unicode分割規則は実装しない。本文をStringへ戻すfallbackやlibrary forkは
持たない。raw native移動・削除にもこのcontext保持経路を使う。


pulldown-cmarkと保存APIには連続`&str`が必要なため、Documentが
`OnceCell<Arc<str>>`を一revisionにつき最大一つ生成する。`source_shared()`は同じ
bufferのArcを共有し、編集・Undo・RedoでDocumentのcacheを破棄する。
選択移動、no-op、失敗はcacheを維持する。保存はこの読み取り境界を使い、既存の
BOM/CP932、混在改行、外部変更拒否、best-fit拒否、元bytes snapshotを維持する。
元bytesは保存保護のため別途保持する必要データである。

UIのAppはactive Document ID/revisionで一つのprojectionとparsed tableの共有cacheを
所有する。幅、表scroll等のlayout-only更新は再parseも全文copyも行わない。
通常の選択移動・再paintはlayout keyが同じならDirectWrite layoutも再構築しない。
編集後はcache keyで破棄・再解析し、tab切替・closeではactive解析cacheを解放する。
全体Markdown解析はrevision変更時に残る。完全な差分parser/全文書仮想化は導入しない。
DirectWrite用UTF-16や短い描画ラベルは外部API向けの派生入力で、編集本文ではない。

箇条書きと番号付きリストは既存projection block treeからnative gutterの短いlabelを
一itemにつき一つ作り、DirectWriteの実測幅に合わせてtext viewのindentを確保する。
番号はlist startと同depthのsibling順に従い、tight/loose・複数段落・複数行・nested itemで
同itemの番号を重複しない。tight itemがnested listを含む場合、その前の親本文も描画する。
引用はindentとnative gutterの縦線を使う。gutterはsource/view mapに入れず、表示だけで
source、dirty、Undo、selectionを変更しない。Liveの編集対象では原文の記法を
source位置に対応して表示し、その場で入力・修正する。表構造の明示Source操作は別契約とする。
code blockは既存InlineKind::CodeのDirectWrite stylingでCascadia Monoを指定し、OS font
fallbackを維持する。所有hidden windowでlabel layout、code family、source/history不変性を
確認するが、実際のfontの外観やphysical GUI受入とは区別する。


crop 0.4.3はMIT、MSRV 1.85で、既存Rust 1.99と互換である。manifestをexact pinし、
Cargo.lockにはcropとstr_indicesが追加される。pulldown-cmark 0.13.4もexact pinし、
HTML生成/CLIを使わないためdefault featuresを無効にしてgetopts、unicode-width、
pulldown-cmark-escapeを除く。GFM bare link用linkify 0.11.0はparser不足を補う。
smallvecは一source spanをallocationなしで保持するための直接依存である。

## native入力と画像保持

compositionがUndo/Redoや表keyに先行する。composition外ではCtrl+ZをUndo、
Ctrl+Shift+Z/Ctrl+YをRedoへ振り分け、後続control文字を抑制する。
`IMR_QUERYCHARPOSITION`は構造体size/alignment、active owner、revision-owned layout、
requested composition UTF-16位置とscalar境界を検証する。描画と同じ幅で作成した
DirectWrite layoutのHitTestTextPositionを用い、表viewの実座標、scroll、DPIを反映し、
clientからscreen pixelへ変換する。解決できないqueryには0を返す。固定文字幅やcaret
への代替を成功応答にしない。先頭/途中/末尾、改行、emoji、表offset、scroll、DPI、
不正sizeとstale ownerの試験は純粋計算とowned hidden Win32 windowで行う。
実MS IME候補UIやphysical keyboardの受入とは分ける。

同期IMM/focus通知はAppのmutable borrow中にも所有snapshotとして順序を保ち、
外側callbackの処理後に適用する。二重mutable参照や通知の破棄で再入を解決しない。
preeditは対象viewの前半・未確定文字・後半を含む一時DirectWrite layoutで整形し、
本文のfont/styleとbaselineを共有する。背景・下線・caret・query位置はその実測範囲を使う。
原文selectionはlayout更新で書き換えず、空行・EOF・fence境界の原文位置も保持する。
通常の連続挿入は狭いtyping groupとし、移動・command・IME・改行・保存・idleで区切る。

画像layoutはbounded metadata probeで寸法だけを取得し、decode/uploadは可視範囲と
前後一viewportのlookaheadに限定する。512 MiBのaggregate cache保持予算を設け、
追加時にoffscreen entryをevictする前にImageViewのCOM cloneも解放する。
同じviewportの再paintではcacheを再利用する。静止画は一frameのchecked RGBA bytes、
animationはcurrent CPU frame、canvas、Direct2D bitmapの三frameと圧縮inputを
保守的に見積もる。decode後の実寸でも入場を検証する。decoder内部・一時decode資源や
OS COM allocationはこの見積り外であり、process memoryの厳密な上限ではない。
過大入力や予算不足はplaceholderと診断を残して本文を保持する。tab切替、close、raw
切替ではdecoder/bitmapとtimerを解放する。50 MBは望ましい目安で、今回の必達条件ではない。
