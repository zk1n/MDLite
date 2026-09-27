#pragma once

#include "table/Table.h"

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

enum class SpanKind {
  Heading,
  HeadingMarker,
  Strong,
  Emphasis,
  EmphasisMarker,
  Strike,
  Code,
  CodeFence,
  Link,
  ListMarker,
  OrderedListMarker
};

enum class BlockKind {
  Paragraph,
  ThematicBreak,
  BlockQuote,
  BulletListItem,
  OrderedListItem,
  TaskListItem,
  IndentedCode,
  FencedCode,
  Html,
  FrontMatter
};

struct StyleSpan {
  SpanKind kind{};
  std::size_t begin{};
  std::size_t end{};
  int level{};
};

struct Heading {
  int level{};
  std::size_t begin{};
  std::size_t end{};
  std::wstring text;
};

struct ImageReference {
  std::size_t begin{};
  std::size_t end{};
  std::wstring alternate_text;
  std::wstring target;
  unsigned width_dip{};
  std::size_t target_begin{};
  std::size_t target_end{};
  wchar_t target_quote{};
};

struct LinkReference {
  std::size_t begin{};
  std::size_t end{};
  std::size_t text_begin{};
  std::size_t text_end{};
  std::wstring target;
};

struct MarkdownBlock {
  BlockKind kind{};
  std::size_t begin{};
  std::size_t end{};
};

struct MarkdownParseResult {
  std::vector<StyleSpan> spans;
  std::vector<Heading> headings;
  std::vector<ImageReference> images;
  std::vector<LinkReference> links;
  std::vector<GfmTable> tables;
  std::vector<MarkdownBlock> blocks;
};

struct SectionMoveResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

MarkdownParseResult ParseMarkdown(std::wstring_view source);
const ImageReference* FindImageAtSourcePosition(const MarkdownParseResult& parsed,
                                               std::size_t source_position) noexcept;
// Rewrites only the parsed image destination, preserving its alt text and all
// surrounding Markdown/HTML source. HTML attribute values are escaped in place.
std::optional<std::wstring> ReplaceImageReferenceTarget(
    std::wstring_view source, const ImageReference& image,
    std::wstring_view replacement_target);
std::optional<std::wstring> ResizeHtmlImageWidth(std::wstring_view markup,
                                                 unsigned width_dip);
std::vector<ImageReference> ParseMarkdownImages(std::wstring_view source);
SectionMoveResult MoveHeadingSection(std::wstring_view source, std::size_t source_begin,
                                     std::size_t target_begin);

}  // namespace mdlite
