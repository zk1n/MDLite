#pragma once

#include <cstddef>
#include <optional>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

enum class ConflictChoice { Current, Incoming, Both };

struct ConflictBlock {
  std::size_t begin{};
  std::size_t end{};
  std::size_t current_begin{};
  std::size_t current_end{};
  std::size_t incoming_begin{};
  std::size_t incoming_end{};
};

struct ConflictEdit {
  std::wstring text;
  std::size_t selection{};
  bool changed{};
};

std::vector<ConflictBlock> ParseConflictBlocks(std::wstring_view text);
std::optional<std::size_t> FindConflictBlock(std::wstring_view text, std::size_t position, bool previous);
ConflictEdit ResolveConflictBlock(std::wstring_view text, std::size_t block_index, ConflictChoice choice);

}  // namespace mdlite
