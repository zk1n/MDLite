#include "workspace/Trust.h"

#include <windows.h>
#include <shlobj.h>

#include <algorithm>
#include <atomic>
#include <cstring>
#include <cwctype>
#include <cstdint>
#include <fstream>
#include <iterator>

namespace mdlite {
namespace {

std::filesystem::path test_store_root;

std::wstring WorkspaceIdentity(const std::filesystem::path& path) {
  std::error_code error;
  auto normalized = std::filesystem::weakly_canonical(path, error);
  if (error) normalized = std::filesystem::absolute(path).lexically_normal();
  std::wstring identity = normalized.wstring();
  std::ranges::transform(identity, identity.begin(), towlower);
  HANDLE directory = CreateFileW(normalized.c_str(), 0,
      FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
      FILE_FLAG_BACKUP_SEMANTICS, nullptr);
  if (directory == INVALID_HANDLE_VALUE) return {};
  BY_HANDLE_FILE_INFORMATION info{};
  if (!GetFileInformationByHandle(directory, &info)) {
    CloseHandle(directory);
    return {};
  }
  CloseHandle(directory);
  identity += L"\n" + std::to_wstring(info.dwVolumeSerialNumber) + L":" +
      std::to_wstring((static_cast<std::uint64_t>(info.nFileIndexHigh) << 32U) |
                      info.nFileIndexLow);
  return identity;
}

std::string Token(std::wstring_view identity) {
  std::uint64_t hash = 14695981039346656037ULL;
  for (wchar_t character : identity) {
    hash ^= static_cast<std::uint16_t>(character);
    hash *= 1099511628211ULL;
  }
  char value[32]{};
  sprintf_s(value, "%016llx", static_cast<unsigned long long>(hash));
  return value;
}

std::filesystem::path TrustStoreRoot() {
  if (!test_store_root.empty()) return test_store_root;
  PWSTR local_app_data{};
  if (FAILED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_CREATE, nullptr,
                                  &local_app_data))) return {};
  const std::filesystem::path root = std::filesystem::path(local_app_data) / L"MDLite/trust/v1";
  CoTaskMemFree(local_app_data);
  return root;
}

std::filesystem::path TrustPath(std::wstring_view identity) {
  const auto token = Token(identity);
  return TrustStoreRoot() / (std::wstring(token.begin(), token.end()) + L".trust");
}

bool WriteIdentity(const std::filesystem::path& path, std::wstring_view identity,
                   std::wstring& error) {
  std::error_code filesystem_error;
  std::filesystem::create_directories(path.parent_path(), filesystem_error);
  if (filesystem_error) {
    error = L"利用者のTrust保存領域を作成できません。";
    return false;
  }
  static std::atomic_uint64_t counter{};
  auto temporary = path;
  temporary += L".tmp-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
               std::to_wstring(++counter);
  std::ofstream output(temporary, std::ios::binary | std::ios::trunc);
  constexpr char magic[] = "MDLITE_TRUST_V1\n";
  output.write(magic, sizeof(magic) - 1);
  output.write(reinterpret_cast<const char*>(identity.data()),
               static_cast<std::streamsize>(identity.size() * sizeof(wchar_t)));
  output.flush();
  output.close();
  if (!output || !MoveFileExW(temporary.c_str(), path.c_str(),
                              MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    std::filesystem::remove(temporary, filesystem_error);
    error = L"Trust状態を原子的に保存できません。";
    return false;
  }
  return true;
}

bool ReadIdentity(const std::filesystem::path& path, std::wstring& identity) {
  std::ifstream input(path, std::ios::binary);
  if (!input) return false;
  constexpr std::string_view magic = "MDLITE_TRUST_V1\n";
  std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  if (!bytes.starts_with(magic) || (bytes.size() - magic.size()) % sizeof(wchar_t) != 0) return false;
  identity.resize((bytes.size() - magic.size()) / sizeof(wchar_t));
  if (!identity.empty())
    memcpy(identity.data(), bytes.data() + magic.size(), identity.size() * sizeof(wchar_t));
  return true;
}

}  // namespace

bool IsWorkspaceTrusted(const std::filesystem::path& workspace) {
  const auto identity = WorkspaceIdentity(workspace);
  if (identity.empty()) return false;
  std::wstring stored;
  return ReadIdentity(TrustPath(identity), stored) && stored == identity;
}

bool SetWorkspaceTrusted(const std::filesystem::path& workspace, bool trusted, std::wstring& error) {
  const auto identity = WorkspaceIdentity(workspace);
  if (identity.empty()) {
    error = L"Workspaceのpathとdirectory identityを確認できません。";
    return false;
  }
  const auto path = TrustPath(identity);
  if (!trusted) {
    std::error_code filesystem_error;
    std::filesystem::remove(path, filesystem_error);
    if (filesystem_error) { error = L"Trust状態を解除できません。"; return false; }
    return true;
  }
  return WriteIdentity(path, identity, error);
}

void SetTrustStoreRootForTesting(const std::filesystem::path& root) { test_store_root = root; }

}  // namespace mdlite
