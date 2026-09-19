#pragma once

#include <filesystem>
#include <string>

namespace mdlite {

bool IsWorkspaceTrusted(const std::filesystem::path& workspace);
bool SetWorkspaceTrusted(const std::filesystem::path& workspace, bool trusted, std::wstring& error);

}  // namespace mdlite
