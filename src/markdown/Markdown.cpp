#include "markdown/Markdown.h"

#include "table/Table.h"

#include <algorithm>
#include <cstdint>
#include <cwctype>
#include <map>
#include <span>
#include <utility>

namespace mdlite {
namespace {

std::wstring_view TrimRight(std::wstring_view value) {
  while (!value.empty() && (value.back() == L'\r' || value.back() == L' ' || value.back() == L'\t')) {
    value.remove_suffix(1);
  }
  return value;
}

struct HtmlAttributeSpan {
  std::wstring_view name;
  std::size_t value_begin{};
  std::size_t value_end{};
  bool has_value{};
  wchar_t quote{};
};

struct HtmlImageTag {
  std::vector<HtmlAttributeSpan> attributes;
  std::size_t closing_marker{};
  std::size_t end{};
  bool self_closing{};
  bool ambiguous_relevant_attributes{};
};

bool IsHtmlSpace(wchar_t character) {
  return character == L' ' || character == L'\t' || character == L'\r' ||
         character == L'\n' || character == L'\f';
}

wchar_t FoldAsciiCase(wchar_t character) {
  return character >= L'A' && character <= L'Z'
      ? static_cast<wchar_t>(character + (L'a' - L'A')) : character;
}

bool HtmlNameEquals(std::wstring_view left, std::wstring_view right) {
  if (left.size() != right.size()) return false;
  for (std::size_t index{}; index < left.size(); ++index) {
    if (FoldAsciiCase(left[index]) != FoldAsciiCase(right[index])) return false;
  }
  return true;
}

std::optional<HtmlImageTag> ScanHtmlImageTag(std::wstring_view source, std::size_t begin) {
  if (begin > source.size() || source.substr(begin, 4) != L"<img") return std::nullopt;
  std::size_t cursor = begin + 4;
  if (cursor < source.size() && !IsHtmlSpace(source[cursor]) && source[cursor] != L'/' &&
      source[cursor] != L'>') return std::nullopt;

  HtmlImageTag tag;
  while (cursor < source.size()) {
    while (cursor < source.size() && IsHtmlSpace(source[cursor])) ++cursor;
    if (cursor >= source.size()) return std::nullopt;
    if (source[cursor] == L'>') {
      tag.closing_marker = cursor;
      tag.end = cursor + 1;
      break;
    }
    if (source[cursor] == L'/') {
      const auto slash = cursor++;
      while (cursor < source.size() && IsHtmlSpace(source[cursor])) ++cursor;
      if (cursor >= source.size() || source[cursor] != L'>') return std::nullopt;
      tag.closing_marker = slash;
      tag.end = cursor + 1;
      tag.self_closing = true;
      break;
    }

    const std::size_t name_begin = cursor;
    while (cursor < source.size() && !IsHtmlSpace(source[cursor]) &&
           source[cursor] != L'=' && source[cursor] != L'>' && source[cursor] != L'/') {
      ++cursor;
    }
    if (cursor == name_begin) return std::nullopt;
    const auto name = source.substr(name_begin, cursor - name_begin);
    while (cursor < source.size() && IsHtmlSpace(source[cursor])) ++cursor;

    HtmlAttributeSpan attribute{name};
    if (cursor < source.size() && source[cursor] == L'=') {
      ++cursor;
      while (cursor < source.size() && IsHtmlSpace(source[cursor])) ++cursor;
      if (cursor >= source.size()) return std::nullopt;
      attribute.has_value = true;
      if (source[cursor] == L'\"' || source[cursor] == L'\'') {
        const wchar_t quote = source[cursor++];
        attribute.quote = quote;
        attribute.value_begin = cursor;
        while (cursor < source.size() && source[cursor] != quote) ++cursor;
        if (cursor >= source.size()) return std::nullopt;
        attribute.value_end = cursor++;
      } else {
        attribute.value_begin = cursor;
        while (cursor < source.size() && !IsHtmlSpace(source[cursor]) &&
               source[cursor] != L'>') ++cursor;
        attribute.value_end = cursor;
        if (attribute.value_begin == attribute.value_end) return std::nullopt;
      }
    }
    tag.attributes.push_back(attribute);
  }
  if (tag.end == 0) return std::nullopt;

  for (const auto name : {std::wstring_view(L"src"), std::wstring_view(L"alt"),
                          std::wstring_view(L"width")}) {
    std::size_t count{};
    for (const auto& attribute : tag.attributes) {
      if (HtmlNameEquals(attribute.name, name) && ++count > 1)
        tag.ambiguous_relevant_attributes = true;
    }
  }
  return tag;
}

const HtmlAttributeSpan* FindHtmlAttribute(const HtmlImageTag& tag, std::wstring_view name) {
  const auto found = std::ranges::find_if(tag.attributes, [name](const auto& attribute) {
    return HtmlNameEquals(attribute.name, name);
  });
  return found == tag.attributes.end() ? nullptr : &*found;
}

bool AppendHtmlCodePoint(std::wstring& output, std::uint32_t code_point) {
  if (code_point == 0 || code_point > 0x10FFFF ||
      (code_point >= 0xD800 && code_point <= 0xDFFF)) return false;
  if constexpr (sizeof(wchar_t) > 2) {
    output.push_back(static_cast<wchar_t>(code_point));
  } else if (code_point <= 0xFFFF) {
    output.push_back(static_cast<wchar_t>(code_point));
  } else {
    code_point -= 0x10000;
    output.push_back(static_cast<wchar_t>(0xD800 + (code_point >> 10)));
    output.push_back(static_cast<wchar_t>(0xDC00 + (code_point & 0x3FF)));
  }
  return true;
}

bool AppendHtmlEntity(std::wstring_view entity, std::wstring& output) {
  if (entity == L"&amp;") { output.push_back(L'&'); return true; }
  if (entity == L"&lt;") { output.push_back(L'<'); return true; }
  if (entity == L"&gt;") { output.push_back(L'>'); return true; }
  if (entity == L"&quot;") { output.push_back(L'\"'); return true; }
  if (!entity.starts_with(L"&#") || entity.size() < 4 || entity.back() != L';') return false;

  std::wstring_view digits = entity.substr(2, entity.size() - 3);
  unsigned base = 10;
  if (!digits.empty() && (digits.front() == L'x' || digits.front() == L'X')) {
    base = 16;
    digits.remove_prefix(1);
  }
  if (digits.empty()) return false;
  std::uint32_t code_point{};
  for (const wchar_t character : digits) {
    unsigned digit{};
    if (character >= L'0' && character <= L'9') digit = static_cast<unsigned>(character - L'0');
    else if (base == 16 && character >= L'a' && character <= L'f')
      digit = 10U + static_cast<unsigned>(character - L'a');
    else if (base == 16 && character >= L'A' && character <= L'F')
      digit = 10U + static_cast<unsigned>(character - L'A');
    else return false;
    if (digit >= base || code_point > (0x10FFFFU - digit) / base) return false;
    code_point = code_point * base + digit;
  }
  return AppendHtmlCodePoint(output, code_point);
}

std::wstring DecodeHtmlAttribute(std::wstring_view value) {
  std::wstring decoded;
  decoded.reserve(value.size());
  for (std::size_t index{}; index < value.size();) {
    if (value[index] != L'&') {
      decoded.push_back(value[index++]);
      continue;
    }
    const auto entity_end = value.find(L';', index + 1);
    if (entity_end != std::wstring_view::npos &&
        AppendHtmlEntity(value.substr(index, entity_end - index + 1), decoded)) {
      index = entity_end + 1;
    } else {
      decoded.push_back(value[index++]);
    }
  }
  return decoded;
}

std::wstring EscapeHtmlImageTarget(std::wstring_view value, wchar_t quote) {
  std::wstring escaped;
  escaped.reserve(value.size());
  for (const wchar_t character : value) {
    if (character == L'&') escaped += L"&amp;";
    else if (character == L'<') escaped += L"&lt;";
    else if (character == L'>') escaped += L"&gt;";
    else if (character == L'\"' && (quote == L'\"' || quote == 0)) escaped += L"&quot;";
    else if (character == L'\'' && (quote == L'\'' || quote == 0)) escaped += L"&#39;";
    else if (quote == 0 && (IsHtmlSpace(character) || character == L'=' || character == L'`')) {
      escaped += L"&#";
      escaped += std::to_wstring(static_cast<unsigned int>(character));
      escaped += L';';
    } else escaped.push_back(character);
  }
  return escaped;
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

bool HasOddBackslashRunBefore(std::wstring_view text, std::size_t position) {
  std::size_t backslash_begin = position;
  while (backslash_begin > 0 && text[backslash_begin - 1] == L'\\') --backslash_begin;
  return (position - backslash_begin) % 2 != 0;
}

void ParseCodeSpans(std::wstring_view line, std::size_t line_offset,
                    MarkdownParseResult& result) {
  struct BacktickRun {
    std::size_t begin{};
    std::size_t end{};
    std::size_t length{};
    bool escaped_opener{};
  };
  std::vector<BacktickRun> runs;
  std::size_t cursor{};
  while (cursor < line.size()) {
    const auto open = line.find(L'`', cursor);
    if (open == std::wstring_view::npos) break;
    std::size_t open_end = open;
    while (open_end < line.size() && line[open_end] == L'`') ++open_end;
    const bool escaped_opener = HasOddBackslashRunBefore(line, open);
    runs.push_back({open, open_end, open_end - open, escaped_opener});
    cursor = open_end;
  }

  std::vector<bool> run_inside_html_image(runs.size());
  std::size_t html_cursor{};
  std::size_t next_run{};
  while (html_cursor < line.size()) {
    const auto html_begin = line.find(L"<img", html_cursor);
    if (html_begin == std::wstring_view::npos) break;
    const auto tag = ScanHtmlImageTag(line, html_begin);
    if (!tag) {
      html_cursor = html_begin + 4;
      continue;
    }
    while (next_run < runs.size() && runs[next_run].end <= html_begin) ++next_run;
    for (std::size_t index = next_run;
         index < runs.size() && runs[index].begin < tag->end; ++index) {
      run_inside_html_image[index] = true;
    }
    html_cursor = tag->end;
  }

  const auto unmatched = std::wstring_view::npos;
  std::vector<std::size_t> next_equal_opener_run(runs.size(), unmatched);
  std::map<std::size_t, std::size_t> nearest_later_run;
  for (std::size_t count = runs.size(); count > 0; --count) {
    const std::size_t index = count - 1;
    const std::size_t opener_length =
        runs[index].length - static_cast<std::size_t>(runs[index].escaped_opener);
    if (opener_length != 0) {
      if (const auto next = nearest_later_run.find(opener_length);
          next != nearest_later_run.end()) {
        next_equal_opener_run[index] = next->second;
      }
    }
    nearest_later_run[runs[index].length] = index;
  }

  for (std::size_t index{}; index < runs.size();) {
    if (run_inside_html_image[index]) {
      ++index;
      continue;
    }
    const std::size_t close_index = next_equal_opener_run[index];
    if (close_index == unmatched) {
      ++index;
      continue;
    }
    const auto& open = runs[index];
    const auto& close = runs[close_index];
    const std::size_t opener_begin = open.begin + static_cast<std::size_t>(open.escaped_opener);
    result.spans.push_back({SpanKind::EmphasisMarker, line_offset + opener_begin,
                            line_offset + open.end, 0});
    result.spans.push_back({SpanKind::Code, line_offset + open.end,
                            line_offset + close.begin, 0});
    result.spans.push_back({SpanKind::EmphasisMarker, line_offset + close.begin,
                            line_offset + close.end, 0});
    index = close_index + 1;
  }
}

struct MarkdownDestinationScan {
  std::size_t target_begin{};
  std::size_t target_end{};
  std::size_t closing_paren{};
};

bool IsMarkdownControl(wchar_t character) {
  const auto code_point = static_cast<std::uint32_t>(character);
  return code_point <= 0x1f || (code_point >= 0x7f && code_point <= 0x9f);
}

bool IsMarkdownWhitespace(wchar_t character) {
  const auto code_point = static_cast<std::uint32_t>(character);
  return IsHtmlSpace(character) || code_point == 0x00a0 || code_point == 0x1680 ||
         (code_point >= 0x2000 && code_point <= 0x200a) ||
         code_point == 0x2028 || code_point == 0x2029 || code_point == 0x202f ||
         code_point == 0x205f || code_point == 0x3000;
}

bool IsMarkdownDestinationSpaceOrControl(wchar_t character) {
  return IsMarkdownWhitespace(character) || IsMarkdownControl(character);
}

std::optional<std::size_t> FindMarkdownImageDestinationOpen(
    std::wstring_view line, std::size_t image_begin, std::size_t line_offset,
    std::span<const StyleSpan> inline_code_spans) {
  if (image_begin + 1 >= line.size() || line[image_begin + 1] != L'[')
    return std::nullopt;

  std::size_t bracket_depth{1};
  std::size_t backslash_run{};
  std::size_t code_span_cursor{};
  for (std::size_t cursor = image_begin + 2; cursor < line.size(); ++cursor) {
    const std::size_t source_position = line_offset + cursor;
    while (code_span_cursor < inline_code_spans.size() &&
           inline_code_spans[code_span_cursor].end <= source_position) {
      ++code_span_cursor;
    }
    if (code_span_cursor < inline_code_spans.size() &&
        inline_code_spans[code_span_cursor].begin <= source_position &&
        source_position < inline_code_spans[code_span_cursor].end) {
      cursor = std::min(line.size(), inline_code_spans[code_span_cursor].end - line_offset) - 1;
      backslash_run = 0;
      continue;
    }

    const wchar_t character = line[cursor];
    if (character == L'\\') {
      ++backslash_run;
      continue;
    }
    const bool escaped = backslash_run % 2 != 0;
    backslash_run = 0;
    if (escaped) continue;
    if (character == L'[') ++bracket_depth;
    else if (character == L']') {
      if (--bracket_depth == 0) {
        if (cursor + 1 < line.size() && line[cursor + 1] == L'(')
          return cursor + 1;
        return std::nullopt;
      }
    }
  }
  return std::nullopt;
}

std::optional<std::size_t> ScanMarkdownTitleClose(std::wstring_view line,
                                                  std::size_t title_begin) {
  if (title_begin >= line.size()) return std::nullopt;
  const wchar_t opener = line[title_begin];
  const bool parenthesized = opener == L'(';
  if (!parenthesized && opener != L'\'' && opener != L'"') return std::nullopt;
  const wchar_t closer = parenthesized ? L')' : opener;
  std::size_t paren_depth{static_cast<std::size_t>(parenthesized)};
  std::size_t backslash_run{};
  for (std::size_t cursor = title_begin + 1; cursor < line.size(); ++cursor) {
    const wchar_t character = line[cursor];
    if (IsMarkdownControl(character)) return std::nullopt;
    if (character == L'\\') {
      ++backslash_run;
      continue;
    }
    const bool escaped = backslash_run % 2 != 0;
    backslash_run = 0;
    if (escaped) continue;
    if (parenthesized) {
      if (character == L'(') ++paren_depth;
      else if (character == L')' && --paren_depth == 0) return cursor;
    } else if (character == closer) {
      return cursor;
    }
  }
  return std::nullopt;
}

std::optional<MarkdownDestinationScan> FinishMarkdownDestination(
    std::wstring_view line, std::size_t target_begin, std::size_t target_end,
    std::size_t suffix_begin) {
  std::size_t cursor = suffix_begin;
  while (cursor < line.size() && IsMarkdownWhitespace(line[cursor])) ++cursor;
  if (cursor >= line.size()) return std::nullopt;
  if (line[cursor] == L')') return MarkdownDestinationScan{target_begin, target_end, cursor};
  if (cursor == suffix_begin) return std::nullopt;

  const auto title_close = ScanMarkdownTitleClose(line, cursor);
  if (!title_close) return std::nullopt;
  cursor = *title_close + 1;
  while (cursor < line.size() && IsMarkdownWhitespace(line[cursor])) ++cursor;
  if (cursor >= line.size() || line[cursor] != L')') return std::nullopt;
  return MarkdownDestinationScan{target_begin, target_end, cursor};
}

std::optional<MarkdownDestinationScan> ScanMarkdownDestination(
    std::wstring_view line, std::size_t open_paren) {
  if (open_paren >= line.size() || line[open_paren] != L'(') return std::nullopt;

  const bool angle_bracketed = open_paren + 1 < line.size() && line[open_paren + 1] == L'<';
  const std::size_t target_begin = open_paren + (angle_bracketed ? 2 : 1);
  std::size_t cursor = target_begin;
  std::size_t paren_depth{};
  std::size_t backslash_run{};
  while (cursor < line.size()) {
    const wchar_t character = line[cursor];
    if (!angle_bracketed && IsMarkdownWhitespace(character)) {
      if (paren_depth != 0 || backslash_run % 2 != 0) return std::nullopt;
      return FinishMarkdownDestination(line, target_begin, cursor, cursor);
    }
    if (IsMarkdownControl(character)) return std::nullopt;
    if (character == L'\\') {
      ++backslash_run;
      ++cursor;
      continue;
    }

    const bool escaped = backslash_run % 2 != 0;
    backslash_run = 0;
    if (angle_bracketed) {
      if (!escaped && character == L'<') return std::nullopt;
      if (!escaped && character == L'>') {
        return FinishMarkdownDestination(line, target_begin, cursor, cursor + 1);
      }
    } else if (!escaped && character == L'(') {
      ++paren_depth;
    } else if (!escaped && character == L')') {
      if (paren_depth == 0)
        return MarkdownDestinationScan{target_begin, cursor, cursor};
      --paren_depth;
    } else if (!escaped && (character == L'<' || character == L'>')) {
      return std::nullopt;
    }
    ++cursor;
  }
  return std::nullopt;
}

bool IsValidMarkdownDestination(std::wstring_view target) {
  if (target.empty()) return false;
  std::size_t paren_depth{};
  std::size_t backslash_run{};
  for (const wchar_t character : target) {
    if (IsMarkdownDestinationSpaceOrControl(character)) return false;
    if (character == L'\\') {
      ++backslash_run;
      continue;
    }
    const bool escaped = backslash_run % 2 != 0;
    backslash_run = 0;
    if (escaped) continue;
    if (character == L'(') ++paren_depth;
    else if (character == L')') {
      if (paren_depth == 0) return false;
      --paren_depth;
    } else if (character == L'<' || character == L'>') {
      return false;
    }
  }
  return paren_depth == 0 && backslash_run % 2 == 0;
}

bool IsValidAngleMarkdownDestination(std::wstring_view target) {
  if (target.empty()) return false;
  std::size_t backslash_run{};
  for (const wchar_t character : target) {
    if (IsMarkdownControl(character)) return false;
    if (character == L'\\') {
      ++backslash_run;
      continue;
    }
    const bool escaped = backslash_run % 2 != 0;
    backslash_run = 0;
    if (!escaped && (character == L'<' || character == L'>')) return false;
  }
  return backslash_run % 2 == 0;
}

void ParseImages(std::wstring_view line, std::size_t line_offset, MarkdownParseResult& result,
                 std::span<const StyleSpan> inline_code_spans) {
  std::size_t cursor = 0;
  std::size_t code_span_cursor{};
  while (cursor < line.size()) {
    auto markdown_begin = line.find(L"![", cursor);
    while (markdown_begin != std::wstring_view::npos &&
           HasOddBackslashRunBefore(line, markdown_begin)) {
      markdown_begin = line.find(L"![", markdown_begin + 2);
    }
    const auto html_begin = line.find(L"<img", cursor);
    if (markdown_begin == std::wstring_view::npos && html_begin == std::wstring_view::npos) break;

    const std::size_t next_token = markdown_begin == std::wstring_view::npos
        ? html_begin
        : html_begin == std::wstring_view::npos ? markdown_begin
                                                : std::min(markdown_begin, html_begin);
    const std::size_t source_token = line_offset + next_token;
    while (code_span_cursor < inline_code_spans.size() &&
           inline_code_spans[code_span_cursor].end <= source_token) {
      ++code_span_cursor;
    }
    if (code_span_cursor < inline_code_spans.size() &&
        inline_code_spans[code_span_cursor].begin <= source_token &&
        source_token < inline_code_spans[code_span_cursor].end) {
      const std::size_t code_end = inline_code_spans[code_span_cursor].end > line_offset
          ? std::min(line.size(), inline_code_spans[code_span_cursor].end - line_offset)
          : next_token + 1;
      cursor = code_end > next_token ? code_end : next_token + 1;
      continue;
    }

    if (markdown_begin != std::wstring_view::npos &&
        (html_begin == std::wstring_view::npos || markdown_begin < html_begin)) {
      const auto destination_open = FindMarkdownImageDestinationOpen(
          line, markdown_begin, line_offset, inline_code_spans);
      if (!destination_open) {
        cursor = markdown_begin + 2;
        continue;
      }
      const auto destination = ScanMarkdownDestination(line, *destination_open);
      if (!destination) {
        cursor = markdown_begin + 2;
        continue;
      }
      result.images.push_back({
          line_offset + markdown_begin, line_offset + destination->closing_paren + 1,
          std::wstring(line.substr(markdown_begin + 2,
                                   *destination_open - markdown_begin - 3)),
          std::wstring(line.substr(destination->target_begin,
                                   destination->target_end - destination->target_begin)),
          0, line_offset + destination->target_begin,
          line_offset + destination->target_end, 0});
      cursor = destination->closing_paren + 1;
      continue;
    }

    const auto tag = ScanHtmlImageTag(line, html_begin);
    if (!tag) {
      cursor = html_begin + 4;
      continue;
    }
    if (!tag->ambiguous_relevant_attributes) {
      const auto* source_attribute = FindHtmlAttribute(*tag, L"src");
      const std::wstring target = source_attribute && source_attribute->has_value
          ? DecodeHtmlAttribute(line.substr(source_attribute->value_begin,
                                            source_attribute->value_end - source_attribute->value_begin))
          : std::wstring{};
      if (!target.empty()) {
        unsigned width{};
        try {
          const auto* width_attribute = FindHtmlAttribute(*tag, L"width");
          const auto width_text = width_attribute && width_attribute->has_value
              ? DecodeHtmlAttribute(line.substr(width_attribute->value_begin,
                                                width_attribute->value_end - width_attribute->value_begin))
              : std::wstring{};
          if (!width_text.empty()) {
            width = std::clamp(static_cast<unsigned>(std::stoul(width_text)), 16U, 8192U);
          }
        } catch (const std::exception&) {
          width = 0;
        }
        const auto* alt_attribute = FindHtmlAttribute(*tag, L"alt");
        const std::wstring alternate_text = alt_attribute && alt_attribute->has_value
            ? DecodeHtmlAttribute(line.substr(alt_attribute->value_begin,
                                              alt_attribute->value_end - alt_attribute->value_begin))
            : std::wstring{};
        result.images.push_back({line_offset + html_begin, line_offset + tag->end,
                                 alternate_text, target, width,
                                 line_offset + source_attribute->value_begin,
                                 line_offset + source_attribute->value_end,
                                 source_attribute->quote});
      }
    }
    cursor = tag->end;
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

std::size_t ListMarkerLength(std::wstring_view rest, BlockKind kind) {
  if (kind == BlockKind::BulletListItem || kind == BlockKind::TaskListItem) return 2;
  return 0;
}

std::size_t OrderedListMarkerLength(std::wstring_view rest, BlockKind kind) {
  if (kind != BlockKind::OrderedListItem) return 0;
  std::size_t marker_end{};
  if (IsOrderedListMarker(rest, marker_end)) return marker_end;
  return 0;
}

struct InlineCodeIndex {
  std::vector<StyleSpan> presentation_spans;
  std::vector<StyleSpan> code_ranges;
};

void AppendCodeSpans(std::wstring_view source, std::size_t begin, std::size_t end,
                     MarkdownParseResult& output) {
  if (end <= begin) return;
  MarkdownParseResult parsed;
  ParseCodeSpans(source.substr(begin, end - begin), begin, parsed);
  output.spans.insert(output.spans.end(), parsed.spans.begin(), parsed.spans.end());
}

InlineCodeIndex ScanInlineCodeSpans(std::wstring_view source) {
  MarkdownParseResult parsed;
  bool in_fence{};
  wchar_t fence_char{};
  bool in_front_matter{};
  bool first_line{true};
  bool previous_has_pipe{};
  bool in_table{};
  bool previous_blockquote{};
  bool active_list_item{};
  std::size_t list_continuation_column{};
  std::size_t inline_run_begin = std::wstring_view::npos;
  std::size_t line_begin{};

  auto flush_inline_run = [&](std::size_t end) {
    if (inline_run_begin == std::wstring_view::npos) return;
    AppendCodeSpans(source, inline_run_begin, end, parsed);
    inline_run_begin = std::wstring_view::npos;
  };
  const auto parse_single_line = [&](std::wstring_view line, std::size_t offset) {
    AppendCodeSpans(source, offset, offset + line.size(), parsed);
  };

  while (line_begin <= source.size()) {
    std::size_t line_end = source.find(L'\n', line_begin);
    if (line_end == std::wstring_view::npos) line_end = source.size();
    const auto line = source.substr(line_begin, line_end - line_begin);
    const auto trimmed = TrimRight(line);

    if (first_line && trimmed == L"---") {
      flush_inline_run(line_begin);
      in_front_matter = true;
      first_line = false;
      previous_has_pipe = false;
      previous_blockquote = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    first_line = false;
    if (in_front_matter) {
      flush_inline_run(line_begin);
      if (trimmed == L"---" || trimmed == L"...") in_front_matter = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    std::size_t indent{};
    while (indent < trimmed.size() && indent < 4 && trimmed[indent] == L' ') ++indent;
    const auto rest = trimmed.substr(indent);
    const bool indented_code = indent == 4 || (!trimmed.empty() && trimmed.front() == L'\t');
    if (!indented_code && (rest.starts_with(L"```") || rest.starts_with(L"~~~"))) {
      flush_inline_run(line_begin);
      const wchar_t current = rest.front();
      if (!in_fence) {
        in_fence = true;
        fence_char = current;
      } else if (current == fence_char) {
        in_fence = false;
      }
      previous_has_pipe = false;
      previous_blockquote = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    if (in_fence) {
      flush_inline_run(line_begin);
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    if (trimmed.empty()) {
      flush_inline_run(line_begin);
      previous_has_pipe = false;
      previous_blockquote = false;
      in_table = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    if (indented_code) {
      flush_inline_run(line_begin);
      previous_has_pipe = false;
      previous_blockquote = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    const bool has_pipe = trimmed.find(L'|') != std::wstring_view::npos;
    if (in_table) {
      if (has_pipe) {
        flush_inline_run(line_begin);
        parse_single_line(line, line_begin);
        previous_has_pipe = true;
        previous_blockquote = false;
        active_list_item = false;
        line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
        continue;
      }
      in_table = false;
      active_list_item = false;
    }
    if (previous_has_pipe && IsGfmTableDelimiter(trimmed)) {
      flush_inline_run(line_begin);
      in_table = true;
      previous_has_pipe = true;
      previous_blockquote = false;
      active_list_item = false;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    const auto kind = ClassifyBlock(rest, false);
    std::size_t hashes{};
    while (hashes < rest.size() && hashes < 6 && rest[hashes] == L'#') ++hashes;
    const bool heading = hashes > 0 && hashes < rest.size() && rest[hashes] == L' ';
    if (kind == BlockKind::BulletListItem || kind == BlockKind::OrderedListItem ||
        kind == BlockKind::TaskListItem) {
      flush_inline_run(line_begin);
      const std::size_t marker_length = kind == BlockKind::OrderedListItem
          ? OrderedListMarkerLength(rest, kind) : ListMarkerLength(rest, kind);
      list_continuation_column = indent + marker_length;
      inline_run_begin = line_begin + list_continuation_column;
      active_list_item = true;
      previous_blockquote = false;
      previous_has_pipe = has_pipe;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    const bool html_image_line = rest.starts_with(L"<img");
    if (active_list_item) {
      const bool paragraph_continuation =
          (kind == BlockKind::Paragraph && !heading) ||
          (kind == BlockKind::Html && html_image_line);
      if (paragraph_continuation &&
          (indent == 0 || indent >= list_continuation_column)) {
        previous_has_pipe = has_pipe;
        previous_blockquote = false;
        line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
        continue;
      }
      flush_inline_run(line_begin);
      active_list_item = false;
    }
    if (kind == BlockKind::BlockQuote) {
      if (!previous_blockquote) {
        flush_inline_run(line_begin);
        inline_run_begin = line_begin;
      }
      previous_blockquote = true;
      previous_has_pipe = has_pipe;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }
    if (previous_blockquote) {
      flush_inline_run(line_begin);
      previous_blockquote = false;
    }

    const bool hard_block = heading || kind == BlockKind::ThematicBreak ||
        kind == BlockKind::BulletListItem || kind == BlockKind::OrderedListItem ||
        kind == BlockKind::TaskListItem || (kind == BlockKind::Html && !html_image_line);
    if (hard_block) {
      flush_inline_run(line_begin);
      if (kind != BlockKind::ThematicBreak && kind != BlockKind::Html)
        parse_single_line(line, line_begin);
      previous_has_pipe = has_pipe;
      line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
      continue;
    }

    if (inline_run_begin == std::wstring_view::npos) inline_run_begin = line_begin;
    previous_has_pipe = has_pipe;
    line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
  }
  flush_inline_run(source.size());

  InlineCodeIndex result;
  for (const auto& span : parsed.spans) {
    result.presentation_spans.push_back(span);
    if (span.kind == SpanKind::Code) result.code_ranges.push_back(span);
  }
  return result;
}

std::span<const StyleSpan> CodeSpansForLine(const std::vector<StyleSpan>& spans,
                                            std::size_t line_begin,
                                            std::size_t line_end) {
  const auto first = std::lower_bound(spans.begin(), spans.end(), line_begin,
      [](const StyleSpan& span, std::size_t position) { return span.end <= position; });
  const auto last = std::lower_bound(first, spans.end(), line_end,
      [](const StyleSpan& span, std::size_t position) { return span.begin < position; });
  const auto all = std::span<const StyleSpan>(spans);
  const auto first_index = static_cast<std::size_t>(first - spans.begin());
  return all.subspan(first_index, static_cast<std::size_t>(last - first));
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
  const auto inline_code = ScanInlineCodeSpans(source);
  result.spans = inline_code.presentation_spans;
  bool in_fence = false;
  wchar_t fence_char = 0;
  bool in_front_matter = false;
  bool first_line = true;
  std::size_t line_begin = 0;
  std::size_t previous_line_begin{};
  bool previous_has_pipe{};

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
    const bool indented_code = indent == 4 || (!trimmed.empty() && trimmed.front() == L'\t');
    if (!indented_code && (rest.starts_with(L"```") || rest.starts_with(L"~~~"))) {
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

    const bool has_pipe = !indented_code &&
        trimmed.find(L'|') != std::wstring_view::npos;
    if (!indented_code && previous_has_pipe && IsGfmTableDelimiter(trimmed)) {
      const auto table = ParseGfmTableAtHeader(source, previous_line_begin);
      if (table && table->begin == previous_line_begin) result.tables.push_back(*table);
    }

    if (!trimmed.empty()) {
      const auto kind = ClassifyBlock(rest, indented_code);
      result.blocks.push_back({kind, line_begin, line_end});
      const auto bullet_marker_length = ListMarkerLength(rest, kind);
      if (bullet_marker_length != 0)
        result.spans.push_back({SpanKind::ListMarker, line_begin + indent,
                                line_begin + indent + bullet_marker_length, 0});
      const auto ordered_marker_length = OrderedListMarkerLength(rest, kind);
      if (ordered_marker_length != 0)
        result.spans.push_back({SpanKind::OrderedListMarker, line_begin + indent,
                                line_begin + indent + ordered_marker_length, 0});
      if (kind == BlockKind::IndentedCode)
        result.spans.push_back({SpanKind::Code, line_begin, line_begin + trimmed.size(), 0});
    }

    std::size_t hashes = 0;
    while (hashes < rest.size() && hashes < 6 && rest[hashes] == L'#') ++hashes;
    if (!indented_code && hashes > 0 && hashes < rest.size() && rest[hashes] == L' ') {
      const std::size_t marker_begin = line_begin + indent;
      const std::size_t text_begin = marker_begin + hashes + 1;
      result.spans.push_back({SpanKind::HeadingMarker, marker_begin, text_begin, static_cast<int>(hashes)});
      result.spans.push_back({SpanKind::Heading, text_begin, line_begin + trimmed.size(),
                              static_cast<int>(hashes)});
      result.headings.push_back({static_cast<int>(hashes), marker_begin,
                                 line_begin + trimmed.size(),
                                 std::wstring(source.substr(text_begin, line_begin + trimmed.size() - text_begin))});
    }

    if (!indented_code) {
      ParseDelimited(line, line_begin, L"**", SpanKind::Strong, result);
      ParseDelimited(line, line_begin, L"__", SpanKind::Strong, result);
      ParseDelimited(line, line_begin, L"~~", SpanKind::Strike, result);
      ParseEmphasis(line, line_begin, result);
      ParseImages(line, line_begin, result,
                  CodeSpansForLine(inline_code.code_ranges, line_begin, line_end));
      ParseLinks(line, line_begin, result);
    }

    previous_line_begin = line_begin;
    previous_has_pipe = has_pipe;
    line_begin = line_end == source.size() ? source.size() + 1 : line_end + 1;
  }
  std::sort(result.spans.begin(), result.spans.end(),
            [](const StyleSpan& a, const StyleSpan& b) { return a.begin < b.begin; });
  return result;
}

const ImageReference* FindImageAtSourcePosition(const MarkdownParseResult& parsed,
                                                std::size_t source_position) noexcept {
  const ImageReference* candidate{};
  for (const auto& image : parsed.images) {
    if (source_position < image.begin || source_position >= image.end) continue;
    if (candidate) return nullptr;
    candidate = &image;
  }
  if (candidate) return candidate;

  for (const auto& image : parsed.images) {
    if (image.end != source_position) continue;
    if (candidate) return nullptr;
    candidate = &image;
  }
  return candidate;
}

std::optional<std::wstring> ReplaceImageReferenceTarget(
    std::wstring_view source, const ImageReference& image,
    std::wstring_view replacement_target) {
  if (replacement_target.empty() || image.begin >= image.end || image.end > source.size() ||
      image.target_begin < image.begin || image.target_begin > image.target_end ||
      image.target_end > image.end) return std::nullopt;

  const auto original_target = source.substr(
      image.target_begin, image.target_end - image.target_begin);
  const bool html_image = source.substr(image.begin, 4) == L"<img";
  if (html_image) {
    if (DecodeHtmlAttribute(original_target) != image.target) return std::nullopt;
  } else if (original_target != image.target) {
    return std::nullopt;
  } else {
    const bool angle_bracketed_destination =
        image.target_begin > image.begin && image.target_end < image.end &&
        source[image.target_begin - 1] == L'<' && source[image.target_end] == L'>';
    const bool valid_destination = angle_bracketed_destination
        ? IsValidAngleMarkdownDestination(replacement_target)
        : IsValidMarkdownDestination(replacement_target);
    if (!valid_destination) return std::nullopt;
  }

  const std::wstring encoded_target = html_image
      ? EscapeHtmlImageTarget(replacement_target, image.target_quote)
      : std::wstring(replacement_target);
  std::wstring updated(source);
  updated.replace(image.target_begin, image.target_end - image.target_begin,
                  encoded_target);
  return updated;
}

std::optional<std::wstring> ResizeHtmlImageWidth(std::wstring_view markup,
                                                 unsigned width_dip) {
  const auto tag = ScanHtmlImageTag(markup, 0);
  if (!tag || tag->end != markup.size() || tag->ambiguous_relevant_attributes)
    return std::nullopt;
  const auto* source = FindHtmlAttribute(*tag, L"src");
  if (!source || !source->has_value ||
      DecodeHtmlAttribute(markup.substr(source->value_begin,
                                        source->value_end - source->value_begin)).empty()) {
    return std::nullopt;
  }
  const auto* width = FindHtmlAttribute(*tag, L"width");
  if (width && !width->has_value) return std::nullopt;
  width_dip = std::clamp(width_dip, 16U, 8192U);
  const auto width_text = std::to_wstring(width_dip);
  std::wstring resized(markup);
  if (width) {
    resized.replace(width->value_begin, width->value_end - width->value_begin, width_text);
  } else {
    std::size_t insertion = tag->closing_marker;
    while (insertion > 0 && IsHtmlSpace(markup[insertion - 1])) --insertion;
    resized.insert(insertion, L" width=\"" + width_text + L"\"");
  }
  return resized;
}

std::vector<ImageReference> ParseMarkdownImages(std::wstring_view source) {
  MarkdownParseResult result;
  const auto inline_code = ScanInlineCodeSpans(source);
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
      const bool indented_code = indent == 4 || (!trimmed.empty() && trimmed.front() == L'\t');
      if (!indented_code && (rest.starts_with(L"```") || rest.starts_with(L"~~~"))) {
        if (!in_fence) { in_fence = true; fence_char = rest.front(); }
        else if (rest.front() == fence_char) in_fence = false;
      } else if (!in_fence && !indented_code) {
        ParseImages(line, line_begin, result,
                    CodeSpansForLine(inline_code.code_ranges, line_begin, line_end));
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
