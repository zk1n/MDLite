# MDLite

軽量なWindows向けWorkspace Markdown Editor。

## 状態

初版の必須機能を統合した、Windows 11 x64・日本語UI向けネイティブWin32アプリです。
Debug／Releaseでビルドできます。Humanによる実操作受入、正式Release、署名済み配布物、`main`統合はまだ行いません。

## 開発方針

Markdownソースを正本とし、表示書式だけで本文を変更しません。現在は複数タブ、
source位置対応付きの見出しLive表示・native画像・表presentation、アウトライン、
本文／Workspace検索・preview付き横断置換、安全保存、UTF-8／BOM／CP932、
復旧スナップショット、Dairy／Meeting／Memo、カレンダー、Markdown表操作、
Workspaceファイル操作、画像asset取込み、明示Trust付きGit／storage adapterを実装しています。

対象CommonMark/GFM構文、設定GUI、変更を伴うGit補助、compact window、画像resize／可視animationまで接続済みです。
IME／ATOK、DPI、主観的な画像・animation品質、各GUI操作、CommonMark/GFM公式conformance corpusなど、
Human実操作が必要な受入条件があります。P0〜P5と100回反復の自動測定結果・大容量時の制約は
[実装・受入状況](docs/implementation-status.md)を参照してください。
固定版の第三者部品と配布条件は[依存関係台帳](docs/dependencies.md)に記録しています。

開発は`develop`を起点とした作業ブランチで行います。
`main`は初期化用ファイルと、確認済みリリースの履歴を保持します。

## ビルドと実行

Visual Studio Build ToolsのMSVC x64、Windows SDK、CMake、Ninjaを使用します。
インストール位置は`vswhere.exe`から実行時に検出します。

```powershell
.\tools\Invoke-Build.ps1 -Preset debug -Test
.\tools\Invoke-Build.ps1 -Preset release -Test
.\tools\Measure-Performance.ps1 -Preset release
```

VS Codeでは「CMake: debug build」を実行後、F5で`build/debug/MDLite.exe`を起動します。
Workspaceをコマンドライン引数へ渡すこともできます。

## ライセンス

MIT License。Copyright (c) 2026 zk1n。
