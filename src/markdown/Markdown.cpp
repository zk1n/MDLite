#include "markdown/Markdown.h"

#include <algorithm>
#include <cwctype>

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
  cursor = 0;
  while ((cursor = line.find(L"<img ", cursor)) != std::wstring_view::npos) {
    const auto element_end = line.find(L'>', cursor + 5);
    if (element_end == std::wstring_view::npos) break;
    const auto element = line.substr(cursor, element_end - cursor + 1);
    const auto attribute = [&](std::wstring_view name) -> std::wstring {
      const auto marker = name.empty() ? std::wstring{} : std::wstring(name) + L"=\"";
      const auto begin = element.find(marker);
      if (begin == std::wstring_view::npos) return {};
      const auto value_begin = begin + marker.size();
      const auto value_end = element.find(L'"', value_begin);
      return value_end == std::wstring_view::npos ? std::wstring{}
                                                   : std::wstring(element.substr(value_begin, value_end - value_begin));
    };
    const auto target = attribute(L"src");
    if (!target.empty()) {
      unsigned width{};
      try {
        const auto width_text = attribute(L"width");
        if (!width_text.empty()) {
          width = std::clamp(static_cast<unsigned>(std::stoul(width_text)), 16U, 8192U);
        }
      } catch (const std::exception&) {
        width = 0;
      }
      result.images.push_back({line_offset + cursor, line_offset + element_end + 1,
                               attribute(L"alt"), target, width});
    }
    cursor = element_end + 1;
  }
}

void ParseLinks(std::wstring_view line, std::size_t line_offset, MarkdownParseResult& result) {
  std::size_t cursor{};
  while ((cursor = line.find(L'[', cursor)) != std::wstring_view::npos) {
    if (cursor > 0 && line[cursor - 1] == L'!') { ++cursor; continue; }
    const auto label_end = line.find(L"](", cursor + 1);
    if (label_end == std::wstring_view::npos) break;
    const auto target_end = line.find(L')', label_end + 2);
    if (target_end == std::wstring_view::npos) break;
    const auto text_begin = line_offset + cursor + 1;
    const auto text_end = line_offset + label_end;
    result.links.push_back({line_offset + cursor, line_offset + target_end + 1,
                            text_begin, text_end,
                            std::wstring(line.substr(label_end + 2, target_end - label_end - 2))});
    result.spans.push_back({SpanKind::Link, text_begin, text_end, 0});
    cursor = target_end + 1;
  }
  cursor = 0;
  while ((cursor = line.find(L'<', cursor)) != std::wstring_view::npos) {
    const auto end = line.find(L'>', cursor + 1);
    if (end == std::wstring_view::npos) break;
    const auto target = line.substr(cursor + 1, end - cursor - 1);
    if (target.starts_with(L"http://") || target.starts_with(L"https://") || target.starts_with(L"mailto:")) {
      result.links.push_back({line_offset + cursor, line_offset + end + 1,
                              line_offset + cursor + 1, line_offset + end, std::wstring(target)});
      result.spans.push_back({SpanKind::Link, line_offset + cursor + 1, line_offset + end, 0});
    }
    cursor = end + 1;
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

bool IsThematicBreak(std::wstring_view line) {
  wchar_t marker{};
  unsigned count{};
  for (const wchar_t character : line) {
    if (character == L' ' || character == L'\t' || character == L'\r') continue;
    if (character != L'*' && character != L'-' && character != L'_') return false;
    if (marker == 0) marker = character;
    if (character != marker) return false;
    ++count;
  }
  return count >= 3;
}

bool IsOrderedListMarker(std::wstring_view line, std::size_t& marker_end) {
  std::size_t cursor{};
  while (cursor < line.size() && iswdigit(line[cursor])) ++cursor;
  if (cursor == 0 || cursor > 9 || cursor + 1 >= line.size() ||
      (line[cursor] != L'.' && line[cursor] != L')') ||
      (line[cursor + 1] != L' ' && line[cursor + 1] != L'\t')) return false;
  marker_end = cursor + 2;
  return true;
}

BlockKind ClassifyBlock(std::wstring_view rest, bool indented) {
  if (indented) return BlockKind::IndentedCode;
  if (IsThematicBreak(rest)) return BlockKind::ThematicBreak;
  if (rest.starts_with(L'>') && (rest.size() == 1 || rest[1] == L' ' || rest[1] == L'\t'))
    return BlockKind::BlockQuote;
  if (rest.size() >= 2 && (rest[0] == L'-' || rest[0] == L'+' || rest[0] == L'*') &&
      (rest[1] == L' ' || rest[1] == L'\t')) {
    const auto content = rest.substr(2);
    if (content.size() >= 3 && content[0] == L'[' && content[2] == L']' &&
        (content[1] == L' ' || content[1] == L'x' || content[1] == L'X') &&
        (content.size() == 3 || content[3] == L' ' || content[3] == L'\t'))
      return BlockKind::TaskListItem;
    return BlockKind::BulletListItem;
  }
  std::size_t marker_end{};
  if (IsOrderedListMarker(rest, marker_end)) return BlockKind::OrderedListItem;
  if (rest.starts_with(L'<') && rest.ends_with(L'>')) return BlockKind::Html;
  return BlockKind::Paragraph;
}

void ParseEmphasis(std::wstring_view line, std::size_t line_offset, MarkdownParseResult& result) {
  for (const wchar_t delimiter : {L'*', L'_'}) {
    std::size_t cursor{};
    while (cursor < line.size()) {
      const auto open = line.find(delimiter, cursor);
      if (open == std::wstring_view::npos) break;
      if ((open > 0 && line[open - 1] == L'\\') ||
          (open + 1 < line.size() && line[open + 1] == delimiter)) {
        cursor = open + 1;
        continue;
      }
      const auto close = line.find(delimiter, open + 1);
      if (close == std::wstring_view::npos || close == open + 1 ||
          (close + 1 < line.size() && line[close + 1] == delimiter)) {
        cursor = open + 1;
        continue;
      }
      result.spans.push_back({SpanKind::EmphasisMarker, line_offset + open,
                              line_offset + open + 1, 0});
      result.spans.push_back({SpanKind::Emphasis, line_offset + open + 1,
                              line_offset + close, 0});
      result.spans.push_back({SpanKind::EmphasisMarker, line_offset + close,
                              line_offset + close + 1, 0});
      cursor = close + 1;
    }
  }
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
      result.blocks.push_back({BlockKind::FrontMatter, line_begin, line_end});
      first_line = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    first_line = false;
    if (in_front_matter) {
      result.blocks.back().end = line_end;
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
      if (in_fence) result.blocks.push_back({BlockKind::FencedCode, line_begin, line_end});
      else if (!result.blocks.empty() && result.blocks.back().kind == BlockKind::FencedCode)
        result.blocks.back().end = line_end;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    if (in_fence) {
      result.spans.push_back({SpanKind::Code, line_begin, line_begin + trimmed.size(), 0});
      if (!result.blocks.empty() && result.blocks.back().kind == BlockKind::FencedCode)
        result.blocks.back().end = line_end;
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

    if (!trimmed.empty()) {
      const bool indented_code = indent == 4 || (!trimmed.empty() && trimmed.front() == L'\t');
      const auto kind = ClassifyBlock(rest, indented_code);
      result.blocks.push_back({kind, line_begin, line_end});
      if (kind == BlockKind::IndentedCode)
        result.spans.push_back({SpanKind::Code, line_begin, line_begin + trimmed.size(), 0});
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
    ParseDelimited(line, line_begin, L"__", SpanKind::Strong, result);
    ParseDelimited(line, line_begin, L"~~", SpanKind::Strike, result);
    ParseDelimited(line, line_begin, L"`", SpanKind::Code, result);
    ParseEmphasis(line, line_begin, result);
    ParseImages(line, line_begin, result);
    ParseLinks(line, line_begin, result);

    previous_line_begin = line_begin;
    previous_has_pipe = has_pipe;
    line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
  }
  if (in_table) result.tables.push_back({table_begin, source.size()});
  std::sort(result.spans.begin(), result.spans.end(),
            [](const StyleSpan& a, const StyleSpan& b) { return a.begin < b.begin; });
  return result;
}

std::vector<ImageReference> ParseMarkdownImages(std::wstring_view source) {
  MarkdownParseResult result;
  bool in_fence{};
  wchar_t fence_char{};
  bool in_front_matter{};
  bool first_line{true};
  std::size_t line_begin{};
  while (line_begin <= source.size()) {
    auto line_end = source.find(L'\n', line_begin);
    if (line_end == std::wstring_view::npos) line_end = source.size();
    const auto line = source.substr(line_begin, line_end - line_begin);
    const auto trimmed = TrimRight(line);
    if (first_line && trimmed == L"---") in_front_matter = true;
    else if (in_front_matter && (trimmed == L"---" || trimmed == L"...")) in_front_matter = false;
    else if (!in_front_matter) {
      std::size_t indent{};
      while (indent < trimmed.size() && indent < 4 && trimmed[indent] == L' ') ++indent;
      const auto rest = trimmed.substr(indent);
      if (rest.starts_with(L"```") || rest.starts_with(L"~~~")) {
        if (!in_fence) { in_fence = true; fence_char = rest.front(); }
        else if (rest.front() == fence_char) in_fence = false;
      } else if (!in_fence) {
        ParseImages(line, line_begin, result);
      }
    }
    first_line = false;
    line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
  }
  return std::move(result.images);
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
