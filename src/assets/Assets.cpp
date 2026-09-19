#include "assets/Assets.h"

#include <windows.h>
#include <wincodec.h>

#include <algorithm>
#include <cwctype>
#include <fstream>
#include <limits>

namespace mdlite {
namespace {

std::wstring Lower(std::wstring value) {
  std::ranges::transform(value, value.begin(), towlower);
  return value;
}

std::wstring EscapeMarkdownAlt(std::wstring_view value) {
  std::wstring escaped;
  for (const wchar_t character : value) {
    if (character == L'\\' || character == L'[' || character == L']') escaped.push_back(L'\\');
    escaped.push_back(character);
  }
  return escaped;
}

std::wstring EscapeHtml(std::wstring_view value) {
  std::wstring escaped;
  for (const wchar_t character : value) {
    if (character == L'&') escaped += L"&amp;";
    else if (character == L'<') escaped += L"&lt;";
    else if (character == L'>') escaped += L"&gt;";
    else if (character == L'\"') escaped += L"&quot;";
    else escaped.push_back(character);
  }
  return escaped;
}

bool InspectSvg(const std::filesystem::path& path, bool& safe, std::wstring& message,
                std::wstring& error) {
  safe = true;
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    error = L"SVGを検査できません。";
    return false;
  }
  const auto size = input.tellg();
  if (size < 0 || size > 16 * 1024 * 1024) {
    safe = false;
    message = L"SVGが安全な表示検査のサイズ上限を超えています。元ファイルは変更していません。";
    return true;
  }
  std::string bytes(static_cast<std::size_t>(size), '\0');
  input.seekg(0);
  if (!bytes.empty() && !input.read(bytes.data(), size)) {
    error = L"SVGを最後まで読み込めません。";
    return false;
  }
  std::ranges::transform(bytes, bytes.begin(), [](unsigned char character) {
    return static_cast<char>(std::tolower(character));
  });
  constexpr std::string_view blocked[] = {"<script", "foreignobject", "onload=", "onerror=",
                                           "javascript:", "href=\"http", "href='http",
                                           "xlink:href=\"//", "xlink:href='//", "data:text/html"};
  for (const auto pattern : blocked) {
    if (bytes.find(pattern) != std::string::npos) {
      safe = false;
      message = L"SVGにスクリプト、外部参照、または埋込みHTMLの可能性があるため表示を無効化します。"
                L"元ファイルとリンクは変更していません。";
      break;
    }
  }
  return true;
}

}  // namespace

bool IsSupportedImage(const std::filesystem::path& path) {
  const std::wstring extension = Lower(path.extension().wstring());
  return extension == L".png" || extension == L".jpg" || extension == L".jpeg" ||
         extension == L".gif" || extension == L".webp" || extension == L".svg";
}

bool ReadRasterImageInfo(const std::filesystem::path& path, RasterImageInfo& info,
                         std::wstring& error) {
  info = {};
  IWICImagingFactory* factory{};
  IWICBitmapDecoder* decoder{};
  IWICBitmapFrameDecode* frame{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) {
    result = factory->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ,
                                                WICDecodeMetadataCacheOnLoad, &decoder);
  }
  UINT frames{};
  if (SUCCEEDED(result)) result = decoder->GetFrameCount(&frames);
  if (SUCCEEDED(result) && frames > 0) result = decoder->GetFrame(0, &frame);
  UINT width{};
  UINT height{};
  if (SUCCEEDED(result)) result = frame->GetSize(&width, &height);
  if (frame) frame->Release();
  if (decoder) decoder->Release();
  if (factory) factory->Release();
  if (FAILED(result) || frames == 0 || width == 0 || height == 0) {
    error = L"画像デコーダーで読み込めない、または寸法が不正な画像です: " + path.wstring();
    return false;
  }
  info = {width, height, frames, frames > 1};
  return true;
}

bool CreateRasterFramePngStream(const std::filesystem::path& path, unsigned frame_index,
                                IStream*& stream, unsigned& delay_ms, std::wstring& error) {
  stream = nullptr;
  delay_ms = 100;
  IWICImagingFactory* factory{};
  IWICBitmapDecoder* decoder{};
  IWICBitmapFrameDecode* source_frame{};
  IWICMetadataQueryReader* metadata{};
  IWICBitmapEncoder* encoder{};
  IWICBitmapFrameEncode* target_frame{};
  IPropertyBag2* properties{};
  IStream* output{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) {
    result = factory->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ,
                                                WICDecodeMetadataCacheOnLoad, &decoder);
  }
  UINT frame_count{};
  if (SUCCEEDED(result)) result = decoder->GetFrameCount(&frame_count);
  if (SUCCEEDED(result) && frame_index >= frame_count) result = E_INVALIDARG;
  if (SUCCEEDED(result)) result = decoder->GetFrame(frame_index, &source_frame);

  if (SUCCEEDED(result) && SUCCEEDED(source_frame->GetMetadataQueryReader(&metadata))) {
    PROPVARIANT value;
    PropVariantInit(&value);
    if (SUCCEEDED(metadata->GetMetadataByName(L"/grctlext/Delay", &value))) {
      unsigned centiseconds{};
      if (value.vt == VT_UI2) centiseconds = value.uiVal;
      else if (value.vt == VT_UI4) centiseconds = value.ulVal;
      if (centiseconds != 0) delay_ms = std::clamp(centiseconds * 10U, 20U, 10000U);
    }
    PropVariantClear(&value);
  }

  UINT width{};
  UINT height{};
  if (SUCCEEDED(result)) result = source_frame->GetSize(&width, &height);
  if (SUCCEEDED(result) && (width == 0 || height == 0)) result = E_INVALIDARG;
  if (SUCCEEDED(result)) result = CreateStreamOnHGlobal(nullptr, TRUE, &output);
  if (SUCCEEDED(result)) result = factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder);
  if (SUCCEEDED(result)) result = encoder->Initialize(output, WICBitmapEncoderNoCache);
  if (SUCCEEDED(result)) result = encoder->CreateNewFrame(&target_frame, &properties);
  if (SUCCEEDED(result)) result = target_frame->Initialize(properties);
  if (SUCCEEDED(result)) result = target_frame->SetSize(width, height);
  WICPixelFormatGUID pixel_format = GUID_WICPixelFormat32bppBGRA;
  if (SUCCEEDED(result)) result = target_frame->SetPixelFormat(&pixel_format);
  if (SUCCEEDED(result)) result = target_frame->WriteSource(source_frame, nullptr);
  if (SUCCEEDED(result)) result = target_frame->Commit();
  if (SUCCEEDED(result)) result = encoder->Commit();
  if (SUCCEEDED(result)) {
    LARGE_INTEGER start{};
    result = output->Seek(start, STREAM_SEEK_SET, nullptr);
  }

  if (properties) properties->Release();
  if (target_frame) target_frame->Release();
  if (encoder) encoder->Release();
  if (metadata) metadata->Release();
  if (source_frame) source_frame->Release();
  if (decoder) decoder->Release();
  if (factory) factory->Release();
  if (FAILED(result)) {
    if (output) output->Release();
    error = L"アニメーション画像のframeを描画用に変換できません: " + path.wstring();
    return false;
  }
  stream = output;
  return true;
}

bool InspectImageSafety(const std::filesystem::path& path, bool& safe,
                        std::wstring& message, std::wstring& error) {
  safe = true;
  message.clear();
  if (Lower(path.extension().wstring()) == L".svg") return InspectSvg(path, safe, message, error);
  RasterImageInfo info;
  if (!ReadRasterImageInfo(path, info, error)) {
    safe = false;
    message = error;
    return true;
  }
  return true;
}

bool ImportImageAsset(const std::filesystem::path& source, const std::filesystem::path& workspace,
                      const std::filesystem::path& document, AssetImportResult& result,
                      std::wstring& error) {
  if (!IsSupportedImage(source) || !std::filesystem::is_regular_file(source)) {
    error = L"PNG、JPEG、GIF、WebP、SVGの画像を選択してください。";
    return false;
  }
  bool source_safe{};
  std::wstring source_safety;
  if (!InspectImageSafety(source, source_safe, source_safety, error)) return false;
  if (!source_safe && Lower(source.extension().wstring()) != L".svg") {
    error = source_safety;
    return false;
  }
  std::error_code filesystem_error;
  const auto asset_directory = workspace / L"assets";
  std::filesystem::create_directories(asset_directory, filesystem_error);
  if (filesystem_error) {
    error = L"assetsフォルダーを作成できません。";
    return false;
  }
  const auto canonical_source = std::filesystem::weakly_canonical(source, filesystem_error);
  if (filesystem_error) {
    error = L"画像パスを確認できません。";
    return false;
  }
  result.stored_path = asset_directory / source.filename();
  for (unsigned suffix = 0; std::filesystem::exists(result.stored_path); ++suffix) {
    if (std::filesystem::equivalent(canonical_source, result.stored_path, filesystem_error) &&
        !filesystem_error) break;
    const std::wstring extension = source.extension().wstring();
    const std::wstring stem = source.stem().wstring();
    result.stored_path = asset_directory /
        (stem + L"_" + std::to_wstring(suffix + 1) + extension);
  }
  if (!std::filesystem::exists(result.stored_path)) {
    if (!CopyFileW(canonical_source.c_str(), result.stored_path.c_str(), TRUE)) {
      error = L"画像をassetsへ安全にコピーできません。本文にはリンクを挿入していません。";
      return false;
    }
  }
  const auto relative = std::filesystem::relative(result.stored_path, document.parent_path(), filesystem_error);
  if (filesystem_error) {
    error = L"文書から画像への相対パスを作成できません。";
    return false;
  }
  result.relative_reference = relative.generic_wstring();
  if (!InspectImageSafety(result.stored_path, result.safe_to_render, result.safety_message, error)) return false;
  return true;
}

std::wstring ImageMarkdown(std::wstring_view alternate_text, std::wstring_view relative_reference) {
  return L"![" + EscapeMarkdownAlt(alternate_text) + L"](" + std::wstring(relative_reference) + L")";
}

std::wstring ImageHtml(std::wstring_view alternate_text, std::wstring_view relative_reference,
                       unsigned width_dip) {
  width_dip = std::clamp(width_dip, 16U, 8192U);
  return L"<img src=\"" + EscapeHtml(relative_reference) + L"\" alt=\"" +
         EscapeHtml(alternate_text) + L"\" width=\"" + std::to_wstring(width_dip) + L"\">";
}

}  // namespace mdlite
