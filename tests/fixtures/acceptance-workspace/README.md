# MDLite Human acceptance kit

架空の「星見図書室」だけを使う最終操作確認用kitです。初期状態はすべて `NOT RUN` です。実ノート、認証情報、実remoteを使わないでください。

## 1. 一時Workspaceを生成する

repo rootのPowerShellから次を実行し、表示された一時pathを控えます。PNG/JPEG/GIF/WebPも同時に生成され、repo内は変更しません。

```powershell
$acceptanceRoot = & .\tests\fixtures\acceptance-workspace\New-AcceptanceWorkspace.ps1
$acceptanceRoot
```

F5でDebug版をビルド・起動し、表示された一時pathをWorkspaceとして開きます。Releaseを確認する場合は同じ一時Workspaceを使い、build種別を記録してください。異常終了試験もこの一時copyだけで行います。

## 2. 環境記録

| 項目 | 記入欄 |
|---|---|
| Build SHA (`git rev-parse HEAD`) |  |
| exe SHA-256 (`Get-FileHash build\debug\MDLite.exe`) |  |
| Build種別 | Debug / Release |
| Windows edition/build |  |
| 表示倍率・monitor構成 |  |
| 入力方式 | Microsoft IME / ATOK / その他 |
| 実施者・日時 |  |

## 3. 手順と期待結果

### H-01 基本表示・表・リンク・アウトライン

1. `notes/観測日誌.md`、`notes/機材メモ.md`、`notes/画像確認.md` を開く。
2. 見出しと本文の高さ、表cell、5形式の画像、Markdown linkが同じLive Editor内で自然に共存することを確認する。
3. 表をTab／Shift+Tab／矢印で移動し、行列を追加・削除してUndo/Redoする。
4. linkを開き、outlineで「画像」sectionを子見出しごと移動してUndo/Redoする。

期待: Markdown sourceとDirty/Undoを壊さず、目的位置へ移動できる。SVGは外部参照のない `safe-gradient.svg` だけを表示する。

### H-02 主window・複数compact・保存・復旧

1. 主windowと2つ以上のcompact windowへ別文書を置き、交互に日本語を入力して保存する。
2. 再度編集し、5秒以上待ってからTask ManagerでMDLiteを終了する。
3. 同じ一時Workspaceを再起動し、復旧promptで復元を選ぶ。
4. 復旧文と保存済み本文を比較し、正常終了後の再起動でもtab/compactと対象文書が一致することを確認する。

期待: compactごとの文書、IME状態、保存対象が混線せず、diskの保存済み本文を上書きせずに未保存編集を復旧できる。

### H-03 source検索・置換・Undo

1. `彗星` を大小文字区別なし／whole-wordでWorkspace検索し、2件目以降の結果へ移動する。
2. `(?m)^観測` などのregexを検索し、正規表現方言と置換参照の表示を確認する。
3. `notes/観測日誌.md` を未保存のままWorkspace置換previewへ含め、適用後にUndoする。
4. 長い検索をCancelし、editorが応答したまま中止結果を示すことを確認する。

期待: 表示文字列ではなくMarkdown sourceが対象で、case/regex/whole-word、未保存buffer、2件目以降の移動、取消、Undoが成立する。

### H-04 IME・DPI・画像操作

1. Microsoft IMEと、利用可能ならATOKで変換候補の表示・確定・削除・選択・Undoを行う。
2. Windows表示倍率100/125/150%で表grid、outline drag、compact配置を確認する。
3. `notes/画像確認.md` でPNG/JPEG/GIF/WebP/SVGを表示し、画像resize後も縦横比が保たれることを確認する。
4. 異なる周期のGIF／WebP animationがそれぞれ表示中に進み、非表示tab/viewport外では不要な更新が止まることを確認する。

期待: source/Dirty/Undoを変えない表示処理として成立し、外部通信や任意codec導入を要求しない。利用できないIME/DPI条件は `NOT RUN` のまま理由を書く。

## 4. 結果記録

`PASS` は実際に全手順を確認した場合だけ記入します。失敗時は再現手順と、可能なら画面・source hashを残します。

| Test ID | 期待 | 実際 | 結果 | 補足／障害ID |
|---|---|---|---|---|
| H-01 | 基本表示・表・link・outline・Undo/Redo成立 | 未実施 | NOT RUN |  |
| H-02 | 複数compact、保存、異常終了復旧、再起動一致 | 未実施 | NOT RUN |  |
| H-03 | case/regex/whole-word、2件目、未保存置換、Cancel、Undo成立 | 未実施 | NOT RUN |  |
| H-04 | IME、DPI、5画像形式、resize、animation成立 | 未実施 | NOT RUN |  |

確認後、一時Workspaceは通常の一時データとして削除できます。repo内の `tests/fixtures/acceptance-workspace` は入力kitなので編集不要です。
