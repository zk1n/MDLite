#pragma once

#include <cstdint>
#include <filesystem>
#include <functional>
#include <string>
#include <vector>

namespace mdlite {

enum class TextEncoding { Utf8, Utf8Bom, Cp932 };
enum class LineEnding { None, Lf, CrLf, Mixed };
enum class SaveStage { BeforeReplace, BeforeReplaceGuarded, AfterReplace };
using SaveStageHook = std::function<void(SaveStage, const std::filesystem::path&)>;

struct FileFingerprint {
  std::uint64_t size{};
  std::uint64_t write_time{};
  std::uint64_t file_id{};
  std::uint64_t content_hash{};
  std::uint32_t volume_id{};
  bool valid{};

  friend bool operator==(const FileFingerprint&, const FileFingerprint&) = default;
};

class Document {
 public:
  bool Load(const std::filesystem::path& path, std::wstring& error);
  void CreateUntitled(const std::filesystem::path& recovery_identity);
  bool Save(std::wstring& error, const SaveStageHook& stage_hook = {});
  bool SaveAs(const std::filesystem::path& path, std::wstring& error);

  void SetText(std::wstring text);
  void MarkEdited(std::wstring text);
  void MarkEditedFromEditor(std::wstring text);

  [[nodiscard]] const std::filesystem::path& path() const noexcept { return path_; }
  [[nodiscard]] const std::wstring& text() const noexcept { return text_; }
  [[nodiscard]] TextEncoding encoding() const noexcept { return encoding_; }
  [[nodiscard]] LineEnding line_ending() const noexcept { return line_ending_; }
  [[nodiscard]] bool dirty() const noexcept {
    return !saved_checkpoint_valid_ || current_content_size_ != saved_content_size_ ||
           current_content_hash_ != saved_content_hash_;
  }
  [[nodiscard]] bool untitled() const noexcept { return untitled_; }
  [[nodiscard]] std::uint64_t revision() const noexcept { return revision_; }
  [[nodiscard]] std::uint64_t saved_revision() const noexcept { return saved_revision_; }
  [[nodiscard]] bool HasExternalChange() const;

 private:
  std::filesystem::path path_;
  std::wstring text_;
  TextEncoding encoding_{TextEncoding::Utf8};
  LineEnding line_ending_{LineEnding::None};
  FileFingerprint disk_fingerprint_{};
  bool untitled_{};
  std::uint64_t revision_{};
  std::uint64_t saved_revision_{};
  std::uint64_t current_content_hash_{};
  std::uint64_t saved_content_hash_{};
  std::size_t current_content_size_{};
  std::size_t saved_content_size_{};
  bool saved_checkpoint_valid_{};
};

std::wstring EncodingLabel(TextEncoding encoding);
std::wstring LineEndingLabel(LineEnding ending);
std::wstring NormalizeEditorLineEndings(std::wstring text, LineEnding ending);

}  // namespace mdlite
