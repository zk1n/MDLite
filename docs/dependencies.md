# 依存関係台帳

## libwebp 1.6.0

- 用途: WebP静止画・animated WebPの復号、frame合成、timing取得。製品にはdecoderとdemuxerを静的リンクする。
- 公式入手先: `https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-1.6.0.tar.gz`
- SHA-256: `e4ab7009bf0629fd11982d4c2aa83964cf244cffba7347ecd39019a9e38c4564`
- ライセンス: BSD-3-Clause相当。追加patent grantを含む。全文はリポジトリ直下の `THIRD_PARTY_NOTICES.md` に収録する。
- ビルド: CMake `FetchContent` が固定URLとhashを照合する。encoder/mux libraryは自動テストfixture生成だけにリンクし、製品runtimeへは含めない。
- 配布境界: 静的リンクのため、実行先にStore版WebP Image Extensionsや任意WIC codecを要求しない。正式package時は `THIRD_PARTY_NOTICES.md` を同梱する。

## Windows SDK / OS components

- WIC: PNG/JPEG/GIFの復号と描画用PNG stream生成。
- XMLLite: SVGのDTD・element・attribute・参照を構文解析して安全判定する。
- Direct2D / Direct3D 11: 検査済みSVG bytesを通信なしでBGRA描画し、RichEdit用PNG streamへ変換する。
- RichEdit / Common Controls: native editor、inline image、標準GUI。
- いずれも対象Windows 11のOS componentであり、MDLiteが別配布する第三者runtimeではない。
