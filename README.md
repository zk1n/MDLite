# MDLite

軽量なWindows向けWorkspace Markdown Editor。

## 状態

`experiment/rust-rewrite`はWindows 11 x64向けのRust編集基盤試験版です。
正式採用、C++版への統合、公開バイナリのリリースは行っていません。
実装と試験の範囲は[受入記録](docs/rust-acceptance.md)を参照してください。

## 開発方針

通常のテキスト編集時の省メモリと操作性を重視し、Live Markdown、
Workspace操作、検索・置換、ノート作成支援を提供する予定です。

C++版は`develop`に保全し、Rust試験版は確認した初期`main`から分岐しています。
この試験では`main`と`develop`を更新せず、実験branchだけを配送します。

## 開発・起動

既存のmise管理Rust 1.99.0、MSVC x64、Windows SDKを使用します。
Cargo.lockを追跡し、環境やglobal設定を自動更新しません。

```powershell
./tools/Invoke-RustBuild.ps1 -Configuration Debug -Verify
mise exec -- cargo run --locked -- tests/fixtures/editor.md
./tools/Invoke-RustBuild.ps1 -Configuration Release
./target/release/mdlite.exe tests/fixtures/editor.md
```

VS Codeで`MDLite Rust Debug`を選びF5でbuildとCodeLLDB起動を行います。
確認用WorkspaceはGit対象外の`samples/rust-editor`へ作成し、既存の
手入力や画像は上書きしません。正本fixtureは`tests/fixtures`です。
`MDLite Rust Release`は性能確認用の別入口です。拡張は既存の
rust-analyzerとCodeLLDBを使います。実際に確認した操作範囲は受入記録に
示し、設定があるだけで物理F5の検証済みとは扱いません。

## 編集基盤

Markdown原文を正本とし、Live表示の見出し・表・画像を同じ面で扱います。
Ctrl+Eで明示的ソース表示へ切り替えます。Ctrl+N／O／S／Shift+Sで
新規／開く／保存／名前を付けて保存、Ctrl+Z／YでUndo／Redoを行います。
表操作と画像の比率固定resizeは対象のコンテキスト操作から行います。

未実装のExplorer、Workspace検索、Calendar、Git UI、autosaveや復旧を
有効な機能として表示しません。Workspace保存境界と製品全体の要求は
[受入記録](docs/rust-acceptance.md)に保持しています。

[部品選定と責務](docs/rust-architecture.md)、[依存ライセンス](docs/dependencies.md)。

## ライセンス

MIT License。Copyright (c) 2026 zk1n。
