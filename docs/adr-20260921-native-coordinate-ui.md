# ADR: source/native 座標境界と Win32 UI 再構築

日付: 2026-09-21

## Decision

RichEdit/Win32 を継続し、Markdown source を正本として、source snapshot と native snapshot を明示的に分離する。
CRLF/LF、画像 object の差分は compact discontinuity のみで写像する。表示書式は presentation-only transaction、
表 grid は RichEdit の実測位置と行高から共有 geometry を描く。WebView、JavaScript/Node、別 preview editor は導入しない。

## Routing / Jev Gate

Initial Decision Packet は Task Inbox の `TASK.md`、`REVIEW.md`、実画面入力、現行 diff/test 状態から作成した。
Luna Max profile の Jev judgment `20260921-mdlite-visual-rebuild-initial-direction-v2` は `sol_xhigh`（confidence 0.81）を
返し、固定 target は `sol_expert_xhigh / gpt-5.6-sol / xhigh` だった。固定ルートの判定は、既存 mapping を土台に UI/table rewrite を
始めず、先に native raw read、座標空間分離、boundary bias、large-file map 制約、DPI 宣言を解消するよう指示した。

実行セッションの child provenance は要求 model と一致せず `gpt-5.6-luna` と記録されたため、dispatch は fail-closed で
`agent_execution_failed` と記録した。判定本文は設計根拠として採用するが、実効 model/effort を推測していない。

## Consequences

- `EditorSnapshot` の従来 export map はテストと source presentation 用に残し、RichEdit の EM/TOM/OLE cp は native snapshot を通る。
- 100 MiB 級文書でも UTF-16 単位の dense map を割り当てない。一方、実 RichEdit native widget、IME、DPI、視覚的な表 hit-test はこの環境では未確認である。
- 休日の自動HTTP更新は安全な既定OFF・user-wide permission・local CSV import の境界を先に実装し、外部通信の実機受入を自動PASSへ昇格しない。
