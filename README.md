# MDLite

軽量なWindows向けWorkspace Markdown Editor。

## 状態

`develop`をWindows 11 x64向けのRust開発主線とします。
製品全体の受入と公開バイナリのリリースは別の判断です。
実装と試験の範囲は[受入記録](docs/rust-acceptance.md)を参照してください。

## 開発方針

通常のテキスト編集時の省メモリと操作性を重視します。Live Markdown、Workspace操作、
検索・置換、ノート作成支援にはcurrent sourceのnative routeがあります。Workspaceのlease、
duplicate notification、別Workspace起動の契約は[Workspace操作](docs/workspace-operations.md)、
要件ごとの接続と未確認範囲は[実装・検証表](docs/ui-feature-parity.md)を参照してください。

移行前のC++版は`cpp-archive/pre-rust-20261013`とGit履歴に保全します。
現行treeはRust専用です。今後の作業branchは`develop`から切ります。
`main`、tag、公開Releaseは今回の移行対象に含めません。

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
Ctrl+Shift+ZもRedoです。本文と履歴はcropの共有Ropeを使い、parser向けの
連続文字列は同じrevisionで共有します。[Markdown方言](docs/markdown-dialect.md)
に採用構文、原文の扱い、対応範囲を記載します。
表操作と画像の比率固定resizeは対象のコンテキスト操作から行います。

Explorer、Workspace検索、Calendar、Git、autosave/recoveryのnative source routeは接続されています。
Workspaceのphysical-identity lease、重複起動通知、別Workspaceのprocess launchと保存境界は
[Workspace操作](docs/workspace-operations.md)に記録します。実装接続とvisible受入は分け、
今回のbuildで未実施のGUI操作・独立review・配送条件は[受入記録](docs/rust-acceptance.md)に残します。
[実装・検証表](docs/ui-feature-parity.md)はslice単位の接続状態と未確認範囲を示し、
[製品要件とUI方針](docs/product-requirements.md)は受入待ちの要求を保持します。

[部品選定と責務](docs/rust-architecture.md)、[依存ライセンス](docs/dependencies.md)。

Editorのキー操作とLiveでの原文境界は[操作仕様](docs/editor-operations.md)に記録します。

起動中のアプリを保全する必要がある場合は、buildの`-TargetDirectory`で
別の出力先、`-EvidenceDirectory`で別の検証記録先を指定できます。
通常のVS Code起動設定は既定の`target`を使用します。

## ライセンス

MIT License。Copyright (c) 2026 zk1n。
