# 製品要件とUI方針

Rust開発への移行で製品要件を削除しない。ここに記載する未移植機能は
予定された要求であり、現在使える機能とは区別する。
実装状況は[受入記録](rust-acceptance.md)を参照する。

## Workspaceと画面

Activity Barと既存のアイコン体系を維持する。中央は文書tabと一つの
native Live Markdown編集面。左上はExplorer/Searchの排他的な表示先、
左下はCalendar、右側はOutline/Gitとする。補助操作で中央編集面を奪わない。
承認UIを撤回せず、全面レイアウトの移植はEditor改修と別に進める。

paneは折り畳み・復元できる。DIPで初期寸法を定義し、client幅から
editorの最小幅を先に確保する。狭幅でも負の幅や画面外の操作領域を作らない。
font、背景、入力、選択、境界、muted、accentの色を各面に共通適用する。
light/dark/system、DPI、high contrastを含む受入は実施範囲を明記する。

scrollは通常6 DIP程度、hover/dragで8〜10 DIP程度の灰色thumbを基準にする。
ヒット領域と標準wheel/keyboard操作は維持し、表の横scrollは表内に限定する。
native実装を使い、WebViewやJavaScript実行基盤を追加しない。

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

Workspace置換はプレビューと本文の再照合後に適用する。binaryやreparse pointは
対象から除外し、走査/正規表現/結果/previewの上限を扱う。
閉じた文書の置換はWorkspace内の復旧記録へ保存し、後続変更と衝突した場合は戻さない。
Outlineの階層移動は子見出しを含む原文範囲と一つのUndo単位に対応する。
code/Front Matter内の見出し風文字列はOutlineへ含めない。
Quick Openと検索結果の移動に別Editorへの切替を要求しない。

## 保存・復旧・その他の後続要求

UTF-8/BOM、CP932、LF/CRLFと混在改行、外部変更、保存失敗を保護する。
autosaveとWorkspace復旧は後続の要求として保持する。復旧本文は
`.mdlite/.state`、破棄可能cacheは`.mdlite/.cache`に置き、AppDataへ本文を保存しない。

Template、Dairy/Memo/MeetingとCalendar、Git UI、Workspace trust、設定GUI、
変更可能keybinding、compact window、診断と手動更新を後続へ保持する。
祝日は同梱データと明示したlocal取込みを基準にし、通常起動や月移動で通信しない。
壊れた取込みは既知データを維持する。未移植機能を有効なボタンとして表示しない。
公開配布の署名・checksum・timestampとRelease受入は別の作業とする。
