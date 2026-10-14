# Markdown dialect

MDLite uses `pulldown-cmark` 0.13.4 for CommonMark parsing and keeps the
original Markdown bytes as the editable document. The published syntax
baselines are [CommonMark 0.31.2](https://spec.commonmark.org/0.31.2/) and
[GFM 0.29-gfm (2019-04-06)](https://github.github.com/gfm/). MDLite adopts the
published GFM table, task-list, strikethrough, and extended-autolink
extensions. GitHub site behavior outside that specification, including
footnotes and alert blockquotes, is treated as a separate extension; these
markers stay literal until their display and editing preserve the source.
GitHub.com also performs site-specific post-processing and sanitization after
Markdown parsing. MDLite does not use browser rendering or infer that behavior
from the GFM specification. This is a managed subset; passing parser tests does
not claim complete CommonMark or GFM conformance.

`markdown::parser_options()` is shared by the display projection and table
discovery. It explicitly enables tables, strikethrough, task-list markers, and
YAML-style metadata blocks. CommonMark core syntax is provided by the parser
without extension flags. GFM's bare URL and email autolinks are added over
plain parser text events with `linkify` 0.11.0, checked against GFM's boundary,
domain, email, and trailing-punctuation rules. The resulting link runs retain
UTF-8 source ranges and UTF-16 display ranges. The implementation covers the
documented URL, `www.`, email, `mailto:`, and `xmpp:` cases below; it does not
claim complete conformance for every edge case in the GFM examples.

## Editing and display behavior

CommonMark headings, paragraphs, soft and hard breaks, thematic breaks,
emphasis, strong emphasis, inline and fenced code, inline and reference links,
images, lists, block quotes, and raw HTML are parsed into the source-backed
projection. Text, entities, escapes, links, and task markers keep source byte
ranges so the native editor can map display positions back to the exact source
spelling. Formatting and navigation metadata do not rewrite Markdown, line
endings, encoding, dirty state, or history.

Native list bullets and ordered numbers are drawn in a gutter from the
projection's source-ranged `List` and `ListItem` blocks; ordered markers honor
the list start and item order, including nesting. Block quotes get a gutter
rule. These presentation markers stay outside projected/source text. The active
Live editing fragment reveals its original punctuation with source-affine positions;
inactive fragments retain their projected presentation. Tight list items keep
editable parent text before and after inline images; an image-first item keeps
its list marker in the gutter, and nested or loose child blocks retain their
own layout.

In Live view a thematic break displays as one atomic `─` marker mapped to the
original `***`, `---`, or `___` source range. The native view draws a horizontal
rule across the available block width. Selecting and deleting the marker edits
the original delimiter spelling as one operation, and Undo restores it exactly.
Raw view displays the original source line. Thematic breaks are block parser
events, so matching punctuation inside code, front matter, or table cells stays
literal and does not draw a rule.

GFM tables keep their original rows and cell content ranges. Alignment comes
from the delimiter row. Structural table operations replace a bounded source
range, while ordinary cell edits stay inside the selected cell. Table parsing
uses the same explicit options as the projection; code spans and escaped pipes
are covered by the fixture.

Task-list markers display as `☐` and `☑`. Each marker run includes its checked
state, display range, and exact `[ ]` or `[x]` source range. Clicking a marker
uses native hit-testing to replace only that source range as one edit, keeping
the selection and allowing Undo. Bare autolinks display their original visible
spelling and carry a link destination; MDLite does not render them through a
browser or fetch their targets.

YAML front matter is recognized only as a complete parser metadata block at
the document start, terminated by a `---` or `...` line. Its block range
includes the delimiters; the metadata body is displayed literally and is
never parsed or executed as YAML. An unterminated or ambiguous block does not
become front matter. The parser then applies ordinary CommonMark rules, so a
standalone `---` may still be a thematic break.

Raw HTML is retained as literal editor text and source ranges. DirectWrite
displays it as text; MDLite does not run scripts, render it in a browser, or
fetch resources named by HTML. This literal policy is separate from GFM's
HTML-disallow list.

## `pulldown-cmark` 0.13.4 option inventory

The parser extension API defines these 15 flags. The status cells state the
current parse, view, edit, and focused-test behavior; “literal” means the
extension flag is off and source punctuation remains available to edit.

| Flag | Example and purpose | Decision and reason | Parse / view / edit / test |
|---|---|---|---|
| `ENABLE_TABLES` | `| A | B |` with a delimiter row | Adopt: required GFM table editing | Parser table events; aligned source-backed cells; bounded cell/structure edits; projection and table fixtures cover alignment, code, and escaped pipe. |
| `ENABLE_FOOTNOTES` | `Text[^id]` and `[^id]: definition` | Defer as literal: the current projection displays only the parsed reference label and treats the definition as a raw block, hiding the source markers; footnotes are outside the required initial dialect | Flag off; complete spelling stays in the source projection; raw text editing; negative test checks visible reference and definition markers and separately proves why enabling the modern flag is lossy today. |
| `ENABLE_STRIKETHROUGH` | `~~text~~` | Adopt: common GFM formatting | Parser strikethrough events; native strikethrough run; edit the original text and delimiters; fixture checks the run. |
| `ENABLE_TASKLISTS` | `- [ ] todo`, `- [x] done` | Adopt: common GFM task marker | Parser emits checked state; projection emits checkbox glyph and exact marker range; native toggle targets only `[ ]` or `[x]`; fixture checks state and offsets. |
| `ENABLE_SMART_PUNCTUATION` | `...`, `--`, straight quotes | Reject: display must not rewrite punctuation | Flag off; punctuation stays byte-for-byte in the text projection; source editing; negative test checks quotes, ellipses, and dashes unchanged. |
| `ENABLE_HEADING_ATTRIBUTES` | `# Title {#id .class}` | Defer as literal heading text: no attribute layout or editing surface | No heading attributes; braces remain visible; edit raw heading text; negative fixture checks the attribute spelling. |
| `ENABLE_YAML_STYLE_METADATA_BLOCKS` | `---` … `---` or `...` | Adopt: front matter is kept as inert source text | Parser recognizes complete boundaries only; literal metadata body and full block source range; edit source directly; tests cover both closers, CRLF, unclosed, ambiguous, and noninitial input. |
| `ENABLE_PLUSES_DELIMITED_METADATA_BLOCKS` | `+++` … `+++` | Defer as literal: the product only specifies YAML front matter | Flag off; no metadata block; source text editing; negative fixture checks the delimiters remain. |
| `ENABLE_OLD_FOOTNOTES` | Older `[^id]:` continuation rules | Reject: only the newer syntax is a possible future candidate, and it is also deferred | Flag off with modern footnotes; old syntax has no special parse or display; raw editing; options test asserts the old bit is absent. In 0.13.4 this flag includes the `ENABLE_FOOTNOTES` bit, so it must not be set. |
| `ENABLE_MATH` | `$x^2$`, `$$x^2$$` | Defer as literal: MDLite has no math renderer or math editing UI | Flag off; no math events; source text remains visible; negative fixture checks no `Math` run. |
| `ENABLE_GFM` | `> [!NOTE]` alert blockquote kinds | Defer alert semantics: pulldown consumes `[!NOTE]` into `BlockQuoteKind`, but the current projection keeps only generic quotes and would hide the marker; alert layout is outside the required initial dialect | Flag off; `[!NOTE]` remains ordinary quote text; raw text editing; test compares the marker with `ENABLE_GFM` on and off. GFM autolinks are implemented separately. |
| `ENABLE_DEFINITION_LIST` | `Term\n: definition` | Defer as literal paragraphs: no definition-list layout or editing surface | Flag off; punctuation remains visible in source; edit ordinary text; no definition-list-specific parser or view behavior. |
| `ENABLE_SUPERSCRIPT` | `x^2^` | Defer as literal: no superscript syntax in the required dialect | Flag off; carets remain visible; raw text editing; negative fixture checks no `Superscript` run. |
| `ENABLE_SUBSCRIPT` | `H~2O~` | Defer as literal: no subscript syntax in the required dialect | Flag off; tildes remain visible; raw text editing; negative fixture checks no `Subscript` run. |
| `ENABLE_WIKILINKS` | `[[Page]]` | Defer as literal: wiki-link navigation is not part of the current editor | Flag off; brackets and label remain visible; raw text editing; negative fixture checks the spelling. |

The option unit test asserts the four selected flags and the absence of the
other eleven. Modern footnotes are disabled because their current projection
does not preserve the visible `[^id]` and definition markers; a focused test
compares that enabled behavior with the source-preserving literal form. This
is a deliberate deferral, not a bitmask limitation: modern footnotes can be
selected independently. `ENABLE_OLD_FOOTNOTES` changes the modern syntax and
includes the `ENABLE_FOOTNOTES` bit in this parser version, so the old flag is
also explicitly absent.

## Focused references

- [`pulldown-cmark` 0.13.4 `Options`](https://docs.rs/pulldown-cmark/0.13.4/pulldown_cmark/struct.Options.html) documents the 15 flags and the footnote flag relationship.
- [`pulldown-cmark` 0.13.4 `Parser`](https://docs.rs/pulldown-cmark/0.13.4/pulldown_cmark/struct.Parser.html) documents parser construction and source offsets.
- [GFM 0.29 task list items](https://github.github.com/gfm/#task-list-items-extension), [autolinks](https://github.github.com/gfm/#autolinks-extension), and [raw HTML](https://github.github.com/gfm/#raw-html) define the published extension behavior used here.
- [`linkify` 0.11.0](https://docs.rs/linkify/0.11.0/linkify/) provides candidate URL/email spans and punctuation handling; MDLite applies narrower GFM checks before creating a source-mapped link run.

## 旧画像tagの限定互換

旧文書の、引用符付き `src` と任意の `alt` / `title` / 数値 `width` だけを持つ
単独またはinlineの `<img ...>` を、非実行の画像参照として読む。
XMLとして妥当な属性escapeを要求し、namespace・イベント・style・子要素等の属性拡張は受け付けない。
それ以外のraw HTMLは従来どおりliteral表示であり、scriptを実行しない。
幅操作は既存の属性値または `mdlite-width` 注釈だけを変更し、無操作では表記を変換しない。
文書からのSVG取込み・SVG画像解決、remote画像の取得は行わない。
共有UI iconのdecoderは文書の画像取込みとは別に扱う。

生成する画像pathは空白・括弧・`#`・`%`等をpercent escapeし、local解決時に一度だけdecodeする。
画像assetのbytesを幅変更や表示操作で変更しない。
