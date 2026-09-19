#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

enum class SpanKind { Heading, HeadingMarker, Strong, EmphasisMarker, Strike, Code, CodeFence, Link };

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
};

struct LinkReference {
  std::size_t begin{};
  std::size_t end{};
  std::size_t text_begin{};
  std::size_t text_end{};
  std::wstring target;
};

struct TableBlock {
  std::size_t begin{};
  std::size_t end{};
};

struct MarkdownParseResult {
  std::vector<StyleSpan> spans;
  std::vector<Heading> headings;
  std::vector<ImageReference> images;
  std::vector<LinkReference> links;
  std::vector<TableBlock> tables;
};

struct SectionMoveResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

MarkdownParseResult ParseMarkdown(std::wstring_view source);
std::vector<ImageReference> ParseMarkdownImages(std::wstring_view source);
SectionMoveResult MoveHeadingSection(std::wstring_view source, std::size_t source_begin,
                                     std::size_t target_begin);

}  // namespace mdlite
