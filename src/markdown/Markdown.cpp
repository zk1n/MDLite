#include "markdown/Markdown.h"

#include <algorithm>

namespace mdlite {
namespace {

std::wstring_view TrimRight(std::wstring_view value) {
  while (!value.empty() && (value.back() == L'\r' || value.back() == L' ' || value.back() == L'\t')) {
    value.remove_suffix(1);
  }
  return value;
}

void ParseDelimited(std::wstring_view line, std::size_t line_offset, std::wstring_view delimiter,
                    SpanKind content_kind, MarkdownParseResult& result) {
  std::size_t cursor = 0;
  while (cursor < line.size()) {
    const std::size_t open = line.find(delimiter, cursor);
    if (open == std::wstring_view::npos) break;
    const std::size_t content = open + delimiter.size();
    const std::size_t close = line.find(delimiter, content);
    if (close == std::wstring_view::npos || close == content) break;
    result.spans.push_back({SpanKind::EmphasisMarker, line_offset + open,
                            line_offset + content, 0});
    result.spans.push_back({content_kind, line_offset + content, line_offset + close, 0});
    result.spans.push_back({SpanKind::EmphasisMarker, line_offset + close,
                            line_offset + close + delimiter.size(), 0});
    cursor = close + delimiter.size();
  }
}

void ParseImages(std::wstring_view line, std::size_t line_offset, MarkdownParseResult& result) {
  std::size_t cursor{};
  while ((cursor = line.find(L"![", cursor)) != std::wstring_view::npos) {
    const auto alt_end = line.find(L"](", cursor + 2);
    if (alt_end == std::wstring_view::npos) break;
    const auto target_end = line.find(L')', alt_end + 2);
    if (target_end == std::wstring_view::npos) break;
    result.images.push_back({line_offset + cursor, line_offset + target_end + 1,
                             std::wstring(line.substr(cursor + 2, alt_end - cursor - 2)),
                             std::wstring(line.substr(alt_end + 2, target_end - alt_end - 2)), 0});
    cursor = target_end + 1;
  }
}

bool LooksLikeTableDelimiter(std::wstring_view line) {
  if (line.find(L'|') == std::wstring_view::npos) return false;
  bool dash{};
  for (wchar_t character : line) {
    if (character == L'-') dash = true;
    else if (character != L'|' && character != L':' && character != L' ' && character != L'\t' && character != L'\r')
      return false;
  }
  return dash;
}

}  // namespace

MarkdownParseResult ParseMarkdown(std::wstring_view source) {
  MarkdownParseResult result;
  bool in_fence = false;
  wchar_t fence_char = 0;
  bool in_front_matter = false;
  bool first_line = true;
  std::size_t line_begin = 0;
  std::size_t previous_line_begin{};
  bool previous_has_pipe{};
  bool in_table{};
  std::size_t table_begin{};

  while (line_begin <= source.size()) {
    std::size_t line_end = source.find(L'\n', line_begin);
    if (line_end == std::wstring_view::npos) line_end = source.size();
    std::wstring_view line = source.substr(line_begin, line_end - line_begin);
    const std::wstring_view trimmed = TrimRight(line);

    if (first_line && trimmed == L"---") {
      in_front_matter = true;
      first_line = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    first_line = false;
    if (in_front_matter) {
      if (trimmed == L"---" || trimmed == L"...") in_front_matter = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    std::size_t indent = 0;
    while (indent < trimmed.size() && indent < 4 && trimmed[indent] == L' ') ++indent;
    const auto rest = trimmed.substr(indent);
    if (rest.starts_with(L"```") || rest.starts_with(L"~~~")) {
      const wchar_t current = rest.front();
      if (!in_fence) {
        in_fence = true;
        fence_char = current;
      } else if (current == fence_char) {
        in_fence = false;
      }
      result.spans.push_back({SpanKind::CodeFence, line_begin + indent, line_begin + trimmed.size(), 0});
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    if (in_fence) {
      result.spans.push_back({SpanKind::Code, line_begin, line_begin + trimmed.size(), 0});
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    const bool has_pipe = trimmed.find(L'|') != std::wstring_view::npos;
    if (!in_table && previous_has_pipe && LooksLikeTableDelimiter(trimmed)) {
      in_table = true;
      table_begin = previous_line_begin;
    } else if (in_table && !has_pipe) {
      result.tables.push_back({table_begin, line_begin});
      in_table = false;
    }

    std::size_t hashes = 0;
    while (hashes < rest.size() && hashes < 6 && rest[hashes] == L'#') ++hashes;
    if (hashes > 0 && hashes < rest.size() && rest[hashes] == L' ') {
      const std::size_t marker_begin = line_begin + indent;
      const std::size_t text_begin = marker_begin + hashes + 1;
      result.spans.push_back({SpanKind::HeadingMarker, marker_begin, text_begin, static_cast<int>(hashes)});
      result.spans.push_back({SpanKind::Heading, text_begin, line_begin + trimmed.size(),
                              static_cast<int>(hashes)});
      result.headings.push_back({static_cast<int>(hashes), marker_begin,
                                 line_begin + trimmed.size(),
                                 std::wstring(source.substr(text_begin, line_begin + trimmed.size() - text_begin))});
    }

    ParseDelimited(line, line_begin, L"**", SpanKind::Strong, result);
    ParseDelimited(line, line_begin, L"~~", SpanKind::Strike, result);
    ParseDelimited(line, line_begin, L"`", SpanKind::Code, result);
    ParseImages(line, line_begin, result);

    previous_line_begin = line_begin;
    previous_has_pipe = has_pipe;
    line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
  }
  if (in_table) result.tables.push_back({table_begin, source.size()});
  std::sort(result.spans.begin(), result.spans.end(),
            [](const StyleSpan& a, const StyleSpan& b) { return a.begin < b.begin; });
  return result;
}

SectionMoveResult MoveHeadingSection(std::wstring_view source, std::size_t source_begin,
                                     std::size_t target_begin) {
  if (source_begin == target_begin) return {std::wstring(source), source_begin, false};
  const auto parsed = ParseMarkdown(source);
  const auto source_heading = std::ranges::find_if(
      parsed.headings, [source_begin](const Heading& heading) { return heading.begin == source_begin; });
  if (source_heading == parsed.headings.end()) return {std::wstring(source), source_begin, false};
  std::size_t source_end = source.size();
  for (auto heading = source_heading + 1; heading != parsed.headings.end(); ++heading) {
    if (heading->level <= source_heading->level) {
      source_end = heading->begin;
      break;
    }
  }
  if (target_begin > source_begin && target_begin < source_end)
    return {std::wstring(source), source_begin, false};
  std::wstring section(source.substr(source_begin, source_end - source_begin));
  std::wstring result(source);
  result.erase(source_begin, source_end - source_begin);
  if (target_begin > source_end) target_begin -= source_end - source_begin;
  target_begin = std::min(target_begin, result.size());
  result.insert(target_begin, section);
  return {std::move(result), target_begin, true};
}

}  // namespace mdlite
