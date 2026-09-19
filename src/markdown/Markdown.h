#pragma once

#include <cstddef>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

enum class SpanKind { Heading, HeadingMarker, Strong, EmphasisMarker, Strike, Code, CodeFence };

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

struct MarkdownParseResult {
  std::vector<StyleSpan> spans;
  std::vector<Heading> headings;
};

struct SectionMoveResult {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

MarkdownParseResult ParseMarkdown(std::wstring_view source);
SectionMoveResult MoveHeadingSection(std::wstring_view source, std::size_t source_begin,
                                     std::size_t target_begin);

}  // namespace mdlite
