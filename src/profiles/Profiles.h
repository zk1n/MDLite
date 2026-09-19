#pragma once

#include <windows.h>

#include <filesystem>
#include <string>

namespace mdlite {

enum class BuiltInProfile { Daily, Meeting, Memo };

struct NoteCreationResult {
  std::filesystem::path path;
  std::size_t cursor{};
  bool created{};
};

bool CreateProfileNote(const std::filesystem::path& workspace, BuiltInProfile profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error);

}  // namespace mdlite
