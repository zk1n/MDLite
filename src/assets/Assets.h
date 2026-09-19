#pragma once

#include <filesystem>
#include <string>
#include <string_view>

namespace mdlite {

struct AssetImportResult {
  std::filesystem::path stored_path;
  std::wstring relative_reference;
  bool safe_to_render{true};
  std::wstring safety_message;
};

bool IsSupportedImage(const std::filesystem::path& path);
bool InspectImageSafety(const std::filesystem::path& path, bool& safe,
                        std::wstring& message, std::wstring& error);
bool ImportImageAsset(const std::filesystem::path& source, const std::filesystem::path& workspace,
                      const std::filesystem::path& document, AssetImportResult& result,
                      std::wstring& error);
std::wstring ImageMarkdown(std::wstring_view alternate_text, std::wstring_view relative_reference);
std::wstring ImageHtml(std::wstring_view alternate_text, std::wstring_view relative_reference,
                       unsigned width_dip);

}  // namespace mdlite
