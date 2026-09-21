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

同梱データは通常起動時に通信しない。内閣府 CSV のローカル取込みは検証後に全置換し、空、重複、壊れた
encoding、HTML相当の入力は拒否する。月次の公式取得許可は user-wide settings の `holiday_auto_update`
（既定 `false`）で、Workspace 設定だけでは有効化できない。許可時の実装は固定host/URLのWinHTTPを
非同期で一度だけ確認し、TLS、timeout、サイズ、redirect、304、件数減少、原子的last-known-good cacheを
適用する。実サイトの正常系取得とHuman GUIは別欄で未実施として扱い、取込み失敗時も同梱データを保持する。
