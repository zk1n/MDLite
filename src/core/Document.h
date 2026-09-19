#pragma once

#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace mdlite {

enum class TextEncoding { Utf8, Utf8Bom, Cp932 };
enum class LineEnding { None, Lf, CrLf, Mixed };

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
  bool Save(std::wstring& error);
  bool SaveAs(const std::filesystem::path& path, std::wstring& error);

  void SetText(std::wstring text);
  void MarkEdited(std::wstring text);
  void MarkEditedFromEditor(std::wstring text);

  [[nodiscard]] const std::filesystem::path& path() const noexcept { return path_; }
  [[nodiscard]] const std::wstring& text() const noexcept { return text_; }
  [[nodiscard]] TextEncoding encoding() const noexcept { return encoding_; }
  [[nodiscard]] LineEnding line_ending() const noexcept { return line_ending_; }
  [[nodiscard]] bool dirty() const noexcept { return revision_ != saved_revision_; }
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
};

std::wstring EncodingLabel(TextEncoding encoding);
std::wstring LineEndingLabel(LineEnding ending);
std::wstring NormalizeEditorLineEndings(std::wstring text, LineEnding ending);

}  // namespace mdlite
