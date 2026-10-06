# ローカルReleaseヒューマンプレビュー

この手順は現在のRelease PASS receiptと実ファイルのSHA-256が一致するときだけ、Git対象外の`.local/human-preview/`に新しい架空Workspaceを作成します。既存の本文や`.mdlite`状態を再利用・上書きしません。Workspace内の`PreviewInfo.json`にはGit commit、Release build ID、source／EXE／サンプル内容のSHA-256を記録します。

リポジトリrootで、インストール済みのPowerShell 7から実行します。このVMのWindows PowerShell 5.1は既存のRestricted設定なので、設定を変更せずPowerShell 7を使います。

```powershell
pwsh -File .\tools\Start-HumanPreview.ps1
```

上記は確認済みの`build\release\MDLite.exe`を通常表示の可視ウィンドウで起動し、生成した`HumanPreview.md`を開きます。準備だけを行う場合は`-PrepareOnly`を指定します。

```powershell
pwsh -File .\tools\Start-HumanPreview.ps1 -PrepareOnly
```

準備コマンドはWorkspaceの場所とsource／EXE／内容hashを表示します。実行ポリシーを上書きする引数は使用しません。receiptがない、PASSでない、現在のソースfingerprintと一致しない、またはEXE hashが一致しない場合はWorkspaceを作らず停止します。ソースが更新された場合はRootがRelease build/testを更新してから起動します。

HumanPreview.mdの表を編集・保存し、標準タイトルバー、Activityアイコンと名前、タブ／ステータス、Calendarの日付・件数・詳細・Daily操作を確認できます。Gitパネルでは新規Workspaceの未信頼表示を観察します。Trustの追加・解除、Git操作、設定・IMEの変更はこのプレビューでは行いません。すべてのサンプル文書は架空データで、リンクは同じWorkspace内を指します。

## Root-observed prototype preview — source 0d9f

Root opened the verified Release executable for source `0d9f4cb4444b5bd18d80034f1338ae663809e2c615f70fcd27feceafd98f2aa0`. `PreviewInfo.json` binds it to Release build run `20fc3337df33480abdb7cd3b072fa193` and executable SHA-256 `db8d3fc60ce3c03a2efb66c86a1a77301250d06cfd92eefd094b160791b8e8df`. The run used commit `ab3762702056f8fb8d2e58e1bf343aa49b31f50f` with a dirty worktree; this is prototype evidence, not a post-commit build.

At 1280×800, Root observed the native caption, Activity names/icons, table rendering, readable Git status, and Calendar's five-file count. Calendar Details and creation information were inspected. The owned process exited after a normal `WM_CLOSE`. The preview made no editor, Git Trust, or IME configuration changes.

This is a scoped visual preview, not full GUI, editing, physical-input, IME, or whole-product acceptance. At the historical 0d9f checkpoint, P2 startup failed its 60-second bound and clipboard results were incomplete. Those observations do not describe the current source; overall acceptance remains NOT PASS. A final human preview is pending after the performance fix, final source freeze, commit, and matching Release receipt; use only evidence bound to that final source. See the [acceptance verification record](verification-approved-ui-20261004.md) for source-qualified status.

## 現在の確認用source — c269

現在のRelease sourceは`c2694697ee91768644a40562ff4b5018bd2c8a44a20405f5342e869ea6b27433`、EXE SHA-256は`0800da84c4c93cb72d5a572bb34dc55c1d778d6e510b3fccbda3f52759315794`です。冒頭の起動コマンドはこの版とPASS receiptの一致を検査します。起動時に新しく生成される`PreviewInfo.json`で実際のcommitと版を確認できます。先の0d9fの画面記録は履歴です。

この版ではbuild/CTest、GUI総合236条件、Save各9件、full P0–P5（100反復）、実ATOK4件、実OS Shift+Tab/Ctrl+Z、標準captionボタンの最大化・復元が通過しました。通常UI、表編集・保存、統一アイコン、Git表示、Calendar、保存済みTab復元を架空文書で試せます。

全体受入は未達です。合意したUnicode連続入力10秒条件と矢印条件はFAILを保持し、実clipboardの今回比較とF5構成確認はBLOCKEDです。Microsoft IMEは既存設定で無効のため未実行です。本人による実キーの長押し、主観的な操作感、他DPI/monitor等の未実行条件も残ります。これらを過去版やsynthetic試験のPASSへ読み替えません。詳しい区分は[承認UI検証記録](verification-approved-ui-20261004.md)に対応します。
