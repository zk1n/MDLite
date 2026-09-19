#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace mdlite {

struct SessionDocument {
  std::filesystem::path path;
  std::size_t selection_begin{};
  std::size_t selection_end{};
  int first_visible_line{};
  bool compact{};
  int x{};
  int y{};
  int width{720};
  int height{520};
};

struct SessionState {
  std::vector<SessionDocument> documents;
  std::size_t active_index{};
  int main_x{};
  int main_y{};
  int main_width{1280};
  int main_height{800};
};

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
  bool ReadSession(std::vector<std::filesystem::path>& open_documents,
                   std::wstring& error) const;
  bool WriteSessionState(const SessionState& session, std::wstring& error) const;
  bool ReadSessionState(SessionState& session, std::wstring& error) const;

  [[nodiscard]] const std::filesystem::path& root() const noexcept { return root_; }
  [[nodiscard]] std::filesystem::path metadata_root() const { return root_ / L".mdlite"; }
  [[nodiscard]] std::filesystem::path state_root() const { return metadata_root() / L".state"; }

 private:
  std::filesystem::path RecoveryPath(const std::filesystem::path& document_path) const;
  std::filesystem::path root_;
};

}  // namespace mdlite
