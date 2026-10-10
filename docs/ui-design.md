# MDLite 表示・UI設計（Task 20260920-mdlite-visual-rebuild-localfirst）

## 構造

- 左側は Workspace tree、中央は tab と同一 RichEdit の live Markdown 編集面、右側は outline。
- 検索、カレンダー、Git、設定は中央の作業面を奪わない補助導線として必要時だけ表示する。
- Markdown source が正本であり、RichEdit の書式、画像 object、表 grid は表示派生物である。

## 座標と編集境界

source UTF-16 index、RichEdit native character position、表示 geometry を別の単位として扱う。
改行（CRLF/LF）と画像 object（Markdown range/U+FFFC）は compact な discontinuity 表だけで写像し、
全文の UTF-16 map を作らない。編集・選択・リンク・outline・検索・表・画像の RichEdit API は native
snapshot を使用し、保存・Undo・再読込みは source snapshot を使用する。写像が曖昧な画像 object は
sourceを推測せず raw Markdown に留める。

## 共通 style とレイアウト

テーマ色、フォント、選択状態は native controls と editor に同じ設定を適用する。通常のサイズは DIP
基準で、`WM_DPICHANGED` 時に font と layout を再生成する。左/右 pane、tab、検索 bar、結果、editor の
順に余白を取り、狭い幅でも editor が負の幅や画面外にならないようにする。

Workspace／Outline pane は View メニューから個別に折り畳み・復元できる。既定幅はDIPの初期値に留め、
利用可能なclient幅からeditorの最小幅を先に確保し、残りへpaneを比例配分する。狭幅で両paneを維持できない
場合はnavigationを自動的に隠し、editorへ負幅・clippingを渡さない。Find/Replaceもclient幅に応じて
advanced option、glob、Workspace actionを折り畳み、検索と置換の基本入力を残す。

配色は `background`（window）、`surface`（pane/tab/list）、`surface_alt`（補助面）、
`editor_background`（RichEdit）、`input_background`（検索入力）、`border`、`muted`、`accent` の
tokenを `ThemeColor` から解決し、light/dark/system と DPI変更の再適用で同じ palette を native control、
editor、表gridへ伝播する。表示用 table grid は WM_PAINT の update clip HDC のみへ描画し、保留中の
source/presentation 世代では描画・hit-testを止めて古い geometry を混在させない。

Markdown presentation は全体を通常書式へ戻してから必要な span だけを再適用する。underline、hidden、
link、background、paragraph spacing/border を明示的に解除し、source変更や公開Undoを発生させない。

## 表

表の原文は GFM Markdown のまま保持する。表示 grid は同じ RichEdit 面で、行ごとの文字位置と font metrics
から共有する上下境界・列境界を測って描画する。cell ごとに固定高さの矩形を重ねず、描画、caret、Tab、
矢印、選択、source transaction は同じ source/native mapping を通る。

## 休日データ

Rev2では休日データを小さな同梱データから読み、更新は新しい同梱版または利用者によるローカルCSV取込みに
限る。通常起動、月移動、カレンダー表示では通信しない。ローカル取込みは全件を検証してから置換し、空、重複、
壊れたencoding、HTML相当の入力を拒否する。自動取得設定やアプリ内HTTP取得経路は設けず、失敗時は既知の
同梱データを維持する。

## 2026-10 編集・検索品質改善の設計差分

上記は2026-09の設計記録です。現在の検索は左サイドバー内でWorkspace treeと切り替え、中央の編集面上部に検索帯を追加しません。結果はファイル別のTreeViewで表示します。

表は単一Documentのソースを正本として、各表の派生表示を子RichEditに投影します。親編集面には測定した表の占有範囲を残し、子側の横スクロールで本文の横位置を動かしません。入力・選択・IME・履歴操作だけを正本の文脈へ写像し、geometry／paint／scrollの問い合わせでは親への再入同期を行いません。列幅は内容とフォントに基づき、ウィンドウへの等分割は行いません。

通常のnative scrollbarは範囲、ヒット領域、標準操作を保ち、描画のみを細い灰色へ変更します。通常6 DIP、hover／drag時10 DIPのthumbを使い、high contrastでは標準描画に戻します。詳細な操作は[編集・検索ガイド](editing-search.md)を参照してください。
