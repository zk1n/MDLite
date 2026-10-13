# Rust編集基盤の試験設計

Windows 11 x64用の試験版。Rust正式採用とC++版への統合は未決定。
Markdown原文を永続正本とし、表示は派生データとする。

| 責務 | 使用する部品 | MDLiteで実装する部分 |
|---|---|---|
| ウィンドウ、入力、clipboard | Win32 / `windows` | 文書の所有、コマンド、入力優先順位 |
| 文字整形、font fallback | DirectWrite | ソース範囲への対応、表配置 |
| 描画 | Direct2D software HWND target | 一つの編集面、選択、キャレット |
| 日本語入力 | Windows IMM | 未確定状態と確定transaction、候補位置 |
| Markdown | pulldown-cmark | 編集可能範囲、未完成構文、原文保持 |
| Raster/SVG | image、resvg/usvg | ローカルのみ、安全検査、保存後の挿入 |
| Unicode境界 | unicode-segmentation | UTF-8/UTF-16/source/view対応 |
| 保存 | Windows CP932変換、ファイル置換 | BOM、改行、外部変更、失敗保護 |

## 選定

Win32とDirectWrite/Direct2Dを選ぶ。OSの文字整形と同じlayoutを描画、
hit test、selection、IME候補位置に使い、独自の文字幅推測を避ける。
`windows` 0.62.2のライセンスはMIT/Apache-2.0、MSRVは1.82。
採用版はCargo.lockを正本とし、Rust 1.99.0の既存mise環境を使う。

このVMでのrenderer-only controlled A/Bは、instrumented diagnostic
binariesで実施し、canonical product EXEは使っていない。Direct2Dの
DEFAULT target作成は約50MBの追加private memoryを確保した。同じ文書・
文字整形・IME設定で比較したSOFTWARE targetは、待機中Working Setが
32,014,336 bytesで、private memoryはDEFAULTより50,065,408 bytes少なかった。
これはrenderer targetの診断値であり、product memory baselineやbudget PASS
ではない。描画方式をsoftwareへ固定し、同じDirectWrite layoutと画像・
選択処理を使う。現在のcanonical Release EXE focused runでは1／6文書の
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

50,000,000 bytes目標は実測で判断する。薄いshellの値を完成製品の保証に
しない。採用可否、C++比較、GUI証拠と未検証範囲は受入記録に残す。
