# Editor操作仕様

Markdownの原文と向き付きselection（anchor/head）は文書に属する。
Live/Sourceの切替、描画、scrollだけでは本文・dirty・Undoを変更しない。
IME合成中は未確定文字を本文や履歴へ入れず、変換キーをIMEへ渡す。

## 移動と選択

| 操作 | 規則 |
|---|---|
| 左右 | grapheme境界で移動。選択があるときの通常左右はその端へ選択を解除する |
| 上下 | 表示行で移動し、連続上下では意図した横位置を保持する |
| Shift+矢印 | anchorを保持してheadを移動する。逆向きの選択も保持する |
| Ctrl+左右 | Unicodeのword-boundary分割による単語・句読点の境界へ移動。空白を飛ばす |
| Ctrl+上下 | 前/次の原文の物理行頭へ移動（空行も含む）。折返し表示行とは区別する |
| Ctrl+Shift+矢印 | 上記の移動規則でanchorを保持し選択する |
| Home/End | 現在の表示行の先頭/末尾。表セルではセル全体の先頭/末尾。Shiftで選択を伸縮する |
| Ctrl+Home/End | 文書先頭/末尾。Shiftで選択を伸縮する |
| PageUp/Down | viewportの高さに応じてcaretとscrollを更新。Shiftで選択する |
| click/drag/Shift+click | keyboardと同じ原文selectionを使う |

UTF-8 byte位置をselectionに使用し、DirectWrite/IMMのUTF-16位置とは
検証した変換を介して対応する。CRLF、結合文字、emojiの途中へ移動しない。
Liveの非表示構文を移動・削除の境界と同一視しない。
空行、EOF、コードの閉じfence外側にも原文の挿入位置を保持する。
折返しの境界で同じ原文位置を共有する場合は、前の表示行末尾か次の
表示行先頭かのaffinityも保持する。End/clickとUndo/Redoはその側を復元する。

## 編集と履歴

| 操作 | 規則 |
|---|---|
| Backspace/Delete | graphemeまたは選択を削除。Ctrl併用は単語境界まで削除する |
| Ctrl+A | active文書全体を選択する |
| Ctrl+Z | Undo。本文・向き付きselection・保存状態を復元する |
| Ctrl+Y/Ctrl+Shift+Z | Redo。Undo後の新規編集はRedo分岐を破棄する |
| Ctrl+N/O/S/Shift+S | 新規/開く/保存/名前を付けて保存 |
| Ctrl+Tab/Ctrl+Shift+Tab | 次/前の文書タブへ移動する |
| Ctrl+E | 原文を変更せずLive/Sourceを切り替える |
| Ctrl+W | active文書を閉じる。未保存本文の確認を省略しない |
| Enter（段落） | 文書の改行形式で一度改行する |
| Enter（list/quote） | 記号・番号・引用prefixを継続。番号は増やし、task項目は未チェックにする。空項目では現在行のprefixを除いて抜ける |
| Enter（code） | 現在の先頭indentを保持。fenceを自動追加しない |
| Tab（本文/code） | caretには4 spaces、範囲選択には対象行のindentを挿入する |
| Shift+Tab（本文/code） | 先頭の最大4 spacesまたは1 tabを除く。選択の向きを保つ |

連続通常入力は同じcaretでの連続挿入をまとめる。移動、選択変更、
command、Enter、IME、保存で区切る。入力間隔が1秒を超える場合も区切る。
IME確定と選択置換は一つの編集単位。取消はdirtyやUndoを増やさない。

Liveでは編集対象の本文/見出し/リスト等の構文記号を原文位置に対応して
表示し、成立前後の記法をその場で入力・修正する。コード内の`##`等は
literalのまま保持し、表示のためにescape/fenceを本文へ追加しない。
表の構造用パイプと区切り行は保護し、セル本文の意味のあるパイプは表示する。

表ではTab/Shift+TabとEnterの明示したセル移動、末尾での行追加、
Ctrl+Enter/Escの退出を使う。矢印移動は本文を変更しない。
通常の本文commandを表に誤って吸収しない。
Ctrl+上下は同じ列の前/次行、Ctrl+単語移動はセル端で前/次セルへ移動する。
Ctrl+削除はセル本文に限定し、構造記号を壊さない。
新しい表の区切り行を書いている間は原文入力を続け、Enterで最初のdata cellへ移る。
表の前の挿入位置で文章を書いた場合は、表を保つための空行を含む段落を
一つの構造編集として作る。既存の表の記法は保持し、Undoで元の境界へ戻る。

## 入力・終了の確認境界

preeditの背景・下線・caret・候補queryは同じ一時layoutの実測geometryを使う。
本文の前半、未確定文字、後半を一体で配置し、blockのfont/styleを継承する。
合成終了後は通常入力へ復帰する。終了時は未保存確認と保存失敗保護を維持する。
window終了、文書close、IDE Stopの証拠を区別する。

OS clipboard統合と物理入力・実IMEの検証状態は[受入記録](rust-acceptance.md)
に別々に記録する。機械試験のPASSだけでHuman受入を主張しない。
