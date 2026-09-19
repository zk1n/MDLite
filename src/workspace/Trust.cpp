#include "workspace/Trust.h"

#include <windows.h>

#include <fstream>

namespace mdlite {
namespace {

std::string Token(const std::filesystem::path& path) {
  std::uint64_t hash = 14695981039346656037ULL;
  for (wchar_t character : std::filesystem::absolute(path).lexically_normal().wstring()) {
    hash ^= static_cast<std::uint16_t>(towlower(character));
    hash *= 1099511628211ULL;
  }
  char value[32]{};
  sprintf_s(value, "%016llx", static_cast<unsigned long long>(hash));
  return value;
}

std::filesystem::path TrustPath(const std::filesystem::path& workspace) {
  return workspace / L".mdlite/.state/trust.local";
}

}  // namespace

bool IsWorkspaceTrusted(const std::filesystem::path& workspace) {
  std::ifstream input(TrustPath(workspace), std::ios::binary);
  std::string value;
  return input && std::getline(input, value) && value == Token(workspace);
}

bool SetWorkspaceTrusted(const std::filesystem::path& workspace, bool trusted, std::wstring& error) {
  const auto path = TrustPath(workspace);
  if (!trusted) {
    std::error_code filesystem_error;
    std::filesystem::remove(path, filesystem_error);
    if (filesystem_error) { error = L"Trust状態を解除できません。"; return false; }
    return true;
  }
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  const auto token = Token(workspace);
  if (!output || !output.write(token.data(), static_cast<std::streamsize>(token.size()))) {
    error = L"Trust状態を保存できません。";
    return false;
  }
  return true;
}

}  // namespace mdlite
