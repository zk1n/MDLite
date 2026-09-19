#include "git/Conflict.h"

#include <algorithm>

namespace mdlite {
namespace {

struct Line {
  std::size_t begin{};
  std::size_t content_end{};
  std::size_t end{};
};

std::vector<Line> Lines(std::wstring_view text) {
  std::vector<Line> lines;
  for (std::size_t begin{}; begin < text.size();) {
    const auto newline = text.find(L'\n', begin);
    const auto end = newline == std::wstring_view::npos ? text.size() : newline + 1;
    auto content_end = newline == std::wstring_view::npos ? text.size() : newline;
    if (content_end > begin && text[content_end - 1] == L'\r') --content_end;
    lines.push_back({begin, content_end, end});
    begin = end;
  }
  return lines;
}

bool StartsWith(std::wstring_view text, const Line& line, std::wstring_view marker) {
  return text.substr(line.begin, line.content_end - line.begin).starts_with(marker);
}

}  // namespace

std::vector<ConflictBlock> ParseConflictBlocks(std::wstring_view text) {
  const auto lines = Lines(text);
  std::vector<ConflictBlock> blocks;
  for (std::size_t index{}; index < lines.size(); ++index) {
    if (!StartsWith(text, lines[index], L"<<<<<<<")) continue;
    const auto begin_index = index;
    std::optional<std::size_t> base_index;
    std::optional<std::size_t> separator_index;
    std::optional<std::size_t> end_index;
    for (++index; index < lines.size(); ++index) {
      if (!base_index && !separator_index && StartsWith(text, lines[index], L"|||||||"))
        base_index = index;
      else if (!separator_index && StartsWith(text, lines[index], L"======="))
        separator_index = index;
      else if (separator_index && StartsWith(text, lines[index], L">>>>>>>")) {
        end_index = index;
        break;
      } else if (StartsWith(text, lines[index], L"<<<<<<<")) {
        --index;
        break;
      }
    }
    if (!separator_index || !end_index) {
      index = begin_index;
      continue;
    }
    blocks.push_back({lines[begin_index].begin, lines[*end_index].end,
                      lines[begin_index].end,
                      lines[base_index.value_or(*separator_index)].begin,
                      lines[*separator_index].end, lines[*end_index].begin});
  }
  return blocks;
}

std::optional<std::size_t> FindConflictBlock(std::wstring_view text, std::size_t position, bool previous) {
  const auto blocks = ParseConflictBlocks(text);
  if (blocks.empty()) return std::nullopt;
  const auto containing = std::ranges::find_if(blocks, [&](const auto& block) {
    return position >= block.begin && position < block.end;
  });
  if (containing != blocks.end()) return static_cast<std::size_t>(containing - blocks.begin());
  if (previous) {
    for (std::size_t index = blocks.size(); index-- > 0;)
      if (blocks[index].begin < position) return index;
    return blocks.size() - 1;
  }
  const auto found = std::ranges::find_if(blocks, [&](const auto& block) { return block.begin >= position; });
  return found == blocks.end() ? std::optional<std::size_t>(0)
                               : std::optional<std::size_t>(static_cast<std::size_t>(found - blocks.begin()));
}

ConflictEdit ResolveConflictBlock(std::wstring_view text, std::size_t block_index, ConflictChoice choice) {
  const auto blocks = ParseConflictBlocks(text);
  if (block_index >= blocks.size()) return {std::wstring(text), 0, false};
  const auto& block = blocks[block_index];
  std::wstring replacement;
  if (choice == ConflictChoice::Current || choice == ConflictChoice::Both)
    replacement.append(text.substr(block.current_begin, block.current_end - block.current_begin));
  if (choice == ConflictChoice::Incoming || choice == ConflictChoice::Both)
    replacement.append(text.substr(block.incoming_begin, block.incoming_end - block.incoming_begin));
  std::wstring updated(text);
  updated.replace(block.begin, block.end - block.begin, replacement);
  return {std::move(updated), block.begin + replacement.size(), true};
}

}  // namespace mdlite
