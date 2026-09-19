#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace mdlite {

class WorkspaceStore {
 public:
  explicit WorkspaceStore(std::filesystem::path root);

  bool Initialize(std::wstring& error) const;
  bool WriteRecovery(const std::filesystem::path& document_path, const std::wstring& text,
                     std::wstring& error) const;
  bool RemoveRecovery(const std::filesystem::path& document_path, std::wstring& error) const;
  std::vector<std::filesystem::path> RecoveryFiles() const;
  bool WriteSession(const std::vector<std::filesystem::path>& open_documents,
                    std::wstring& error) const;

  [[nodiscard]] const std::filesystem::path& root() const noexcept { return root_; }
  [[nodiscard]] std::filesystem::path metadata_root() const { return root_ / L".mdlite"; }
  [[nodiscard]] std::filesystem::path state_root() const { return metadata_root() / L".state"; }

 private:
  std::filesystem::path RecoveryPath(const std::filesystem::path& document_path) const;
  std::filesystem::path root_;
};

}  // namespace mdlite
