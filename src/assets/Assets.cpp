#include "assets/Assets.h"

#include <windows.h>
#include <d2d1_3.h>
#include <d3d11.h>
#include <dxgi1_2.h>
#include <wincodec.h>
#include <wrl/client.h>
#include <xmllite.h>
#include <webp/demux.h>

#include <algorithm>
#include <array>
#include <cwctype>
#include <fstream>
#include <limits>
#include <vector>

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

std::wstring_view Trim(std::wstring_view value) {
  while (!value.empty() && iswspace(value.front())) value.remove_prefix(1);
  while (!value.empty() && iswspace(value.back())) value.remove_suffix(1);
  return value;
}

bool UnsafeReference(std::wstring_view value) {
  std::wstring lower(value);
  std::ranges::transform(lower, lower.begin(), towlower);
  const auto trimmed = Trim(lower);
  if (trimmed.find(L"://") != std::wstring::npos || trimmed.starts_with(L"//") ||
      trimmed.starts_with(L"data:") ||
      lower.find(L"javascript:") != std::wstring::npos ||
      lower.find(L"vbscript:") != std::wstring::npos ||
      lower.find(L"file:") != std::wstring::npos ||
      lower.find(L"@import") != std::wstring::npos ||
      lower.find(L"expression(") != std::wstring::npos ||
      lower.find(L"behavior:") != std::wstring::npos) return true;
  std::size_t cursor{};
  while ((cursor = lower.find(L"url(", cursor)) != std::wstring::npos) {
    const auto close = lower.find(L')', cursor + 4);
    if (close == std::wstring::npos) return true;
    auto reference = Trim(std::wstring_view(lower).substr(cursor + 4, close - cursor - 4));
    if (reference.size() >= 2 &&
        ((reference.front() == L'\'' && reference.back() == L'\'') ||
         (reference.front() == L'"' && reference.back() == L'"'))) {
      reference.remove_prefix(1);
      reference.remove_suffix(1);
      reference = Trim(reference);
    }
    if (reference.empty() || reference.front() != L'#') return true;
    cursor = close + 1;
  }
  return false;
}

bool SafeHref(std::wstring_view value) {
  value = Trim(value);
  return value.empty() || (value.front() == L'#' && !UnsafeReference(value));
}

void MarkUnsafeSvg(bool& safe, std::wstring& message) {
  safe = false;
  message = L"SVGにscript、event、外部参照、DTD、埋込みHTML、または安全でないCSSがあるため表示を無効化します。"
            L"元ファイルとMarkdownリンクは変更していません。";
}

bool ReadWebpBytes(const std::filesystem::path& path, std::vector<unsigned char>& bytes,
                   std::wstring& error) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    error = L"WebPを開けません: " + path.wstring();
    return false;
  }
  const auto size = input.tellg();
  constexpr std::streamoff maximum_webp_bytes = 256LL * 1024 * 1024;
  if (size <= 0 || size > maximum_webp_bytes) {
    error = L"WebPのサイズが不正、または安全上限を超えています。";
    return false;
  }
  bytes.resize(static_cast<std::size_t>(size));
  input.seekg(0);
  if (!input.read(reinterpret_cast<char*>(bytes.data()), size)) {
    error = L"WebPを最後まで読み込めません。";
    return false;
  }
  return true;
}

bool CreatePngStreamFromBgra(const std::uint8_t* pixels, unsigned width, unsigned height,
                             IStream*& output, std::wstring& error) {
  output = nullptr;
  if (!pixels || width == 0 || height == 0 ||
      width > std::numeric_limits<UINT>::max() / 4U) {
    error = L"画像frameの寸法が不正です。";
    return false;
  }
  const UINT stride = width * 4U;
  if (height > std::numeric_limits<UINT>::max() / stride) {
    error = L"画像frameが描画上限を超えています。";
    return false;
  }
  IWICImagingFactory* factory{};
  IWICBitmapEncoder* encoder{};
  IWICBitmapFrameEncode* frame{};
  IPropertyBag2* properties{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) result = CreateStreamOnHGlobal(nullptr, TRUE, &output);
  if (SUCCEEDED(result)) result = factory->CreateEncoder(GUID_ContainerFormatPng, nullptr, &encoder);
  if (SUCCEEDED(result)) result = encoder->Initialize(output, WICBitmapEncoderNoCache);
  if (SUCCEEDED(result)) result = encoder->CreateNewFrame(&frame, &properties);
  if (SUCCEEDED(result)) result = frame->Initialize(properties);
  if (SUCCEEDED(result)) result = frame->SetSize(width, height);
  WICPixelFormatGUID format = GUID_WICPixelFormat32bppBGRA;
  if (SUCCEEDED(result)) result = frame->SetPixelFormat(&format);
  if (SUCCEEDED(result) && !IsEqualGUID(format, GUID_WICPixelFormat32bppBGRA)) {
    result = WINCODEC_ERR_UNSUPPORTEDPIXELFORMAT;
  }
  if (SUCCEEDED(result)) {
    result = frame->WritePixels(height, stride, stride * height,
                                const_cast<BYTE*>(pixels));
  }
  if (SUCCEEDED(result)) result = frame->Commit();
  if (SUCCEEDED(result)) result = encoder->Commit();
  if (SUCCEEDED(result)) {
    LARGE_INTEGER start{};
    result = output->Seek(start, STREAM_SEEK_SET, nullptr);
  }
  if (properties) properties->Release();
  if (frame) frame->Release();
  if (encoder) encoder->Release();
  if (factory) factory->Release();
  if (FAILED(result)) {
    if (output) output->Release();
    output = nullptr;
    error = L"画像frameを描画用PNGへ変換できません。";
    return false;
  }
  return true;
}

bool InspectSvgBytes(std::string_view bytes, bool& safe, std::wstring& message,
                     std::wstring& error);

bool RasterizeSvgToPng(const std::filesystem::path& path, IStream*& output,
                       std::wstring& error) {
  output = nullptr;
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    error = L"SVGを開けません: " + path.wstring();
    return false;
  }
  const auto byte_count = input.tellg();
  if (byte_count <= 0 || byte_count > 16 * 1024 * 1024) {
    error = L"SVGのサイズが不正、または安全上限を超えています。";
    return false;
  }
  std::vector<unsigned char> bytes(static_cast<std::size_t>(byte_count));
  input.seekg(0);
  if (!input.read(reinterpret_cast<char*>(bytes.data()), byte_count)) {
    error = L"SVGを最後まで読み込めません。";
    return false;
  }
  bool safe{};
  std::wstring safety_message;
  const std::string_view inspected_bytes(reinterpret_cast<const char*>(bytes.data()), bytes.size());
  if (!InspectSvgBytes(inspected_bytes, safe, safety_message, error)) return false;
  if (!safe) {
    error = safety_message;
    return false;
  }

  using Microsoft::WRL::ComPtr;
  ComPtr<ID3D11Device> d3d_device;
  ComPtr<ID3D11DeviceContext> d3d_context;
  UINT flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT;
  HRESULT result = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr, flags, nullptr, 0,
                                     D3D11_SDK_VERSION, &d3d_device, nullptr, &d3d_context);
  if (FAILED(result)) {
    result = D3D11CreateDevice(nullptr, D3D_DRIVER_TYPE_WARP, nullptr, flags, nullptr, 0,
                               D3D11_SDK_VERSION, &d3d_device, nullptr, &d3d_context);
  }
  ComPtr<IDXGIDevice> dxgi_device;
  if (SUCCEEDED(result)) result = d3d_device.As(&dxgi_device);
  ComPtr<ID2D1Factory1> factory;
  D2D1_FACTORY_OPTIONS factory_options{};
  if (SUCCEEDED(result)) {
    result = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, __uuidof(ID2D1Factory1),
                               &factory_options,
                               reinterpret_cast<void**>(factory.GetAddressOf()));
  }
  ComPtr<ID2D1Device> d2d_device;
  if (SUCCEEDED(result)) result = factory->CreateDevice(dxgi_device.Get(), &d2d_device);
  ComPtr<ID2D1DeviceContext> base_context;
  if (SUCCEEDED(result)) {
    result = d2d_device->CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE, &base_context);
  }
  ComPtr<ID2D1DeviceContext5> context;
  if (SUCCEEDED(result)) result = base_context.As(&context);

  constexpr UINT width = 320;
  constexpr UINT height = 180;
  D3D11_TEXTURE2D_DESC texture_description{};
  texture_description.Width = width;
  texture_description.Height = height;
  texture_description.MipLevels = 1;
  texture_description.ArraySize = 1;
  texture_description.Format = DXGI_FORMAT_B8G8R8A8_UNORM;
  texture_description.SampleDesc.Count = 1;
  texture_description.Usage = D3D11_USAGE_DEFAULT;
  texture_description.BindFlags = D3D11_BIND_RENDER_TARGET;
  ComPtr<ID3D11Texture2D> target_texture;
  if (SUCCEEDED(result)) {
    result = d3d_device->CreateTexture2D(&texture_description, nullptr, &target_texture);
  }
  ComPtr<IDXGISurface> target_surface;
  if (SUCCEEDED(result)) result = target_texture.As(&target_surface);
  D2D1_BITMAP_PROPERTIES1 bitmap_properties{};
  bitmap_properties.pixelFormat.format = DXGI_FORMAT_B8G8R8A8_UNORM;
  bitmap_properties.pixelFormat.alphaMode = D2D1_ALPHA_MODE_IGNORE;
  bitmap_properties.dpiX = 96.0F;
  bitmap_properties.dpiY = 96.0F;
  bitmap_properties.bitmapOptions = D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW;
  ComPtr<ID2D1Bitmap1> target_bitmap;
  if (SUCCEEDED(result)) {
    result = context->CreateBitmapFromDxgiSurface(target_surface.Get(), &bitmap_properties,
                                                   &target_bitmap);
  }

  ComPtr<IStream> svg_stream;
  if (SUCCEEDED(result)) result = CreateStreamOnHGlobal(nullptr, TRUE, &svg_stream);
  if (SUCCEEDED(result)) {
    ULONG written{};
    result = svg_stream->Write(bytes.data(), static_cast<ULONG>(bytes.size()), &written);
    if (SUCCEEDED(result) && written != bytes.size()) result = STG_E_WRITEFAULT;
  }
  if (SUCCEEDED(result)) {
    LARGE_INTEGER start{};
    result = svg_stream->Seek(start, STREAM_SEEK_SET, nullptr);
  }
  ComPtr<ID2D1SvgDocument> svg_document;
  if (SUCCEEDED(result)) {
    const D2D1_SIZE_F viewport{static_cast<float>(width), static_cast<float>(height)};
    result = context->CreateSvgDocument(svg_stream.Get(), viewport, &svg_document);
  }
  if (SUCCEEDED(result)) {
    context->SetTarget(target_bitmap.Get());
    context->BeginDraw();
    const D2D1_COLOR_F white{1.0F, 1.0F, 1.0F, 1.0F};
    context->Clear(&white);
    context->DrawSvgDocument(svg_document.Get());
    result = context->EndDraw();
    context->SetTarget(nullptr);
  }

  D3D11_TEXTURE2D_DESC staging_description = texture_description;
  staging_description.Usage = D3D11_USAGE_STAGING;
  staging_description.BindFlags = 0;
  staging_description.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
  ComPtr<ID3D11Texture2D> staging_texture;
  if (SUCCEEDED(result)) {
    result = d3d_device->CreateTexture2D(&staging_description, nullptr, &staging_texture);
  }
  if (SUCCEEDED(result)) d3d_context->CopyResource(staging_texture.Get(), target_texture.Get());
  D3D11_MAPPED_SUBRESOURCE mapped{};
  if (SUCCEEDED(result)) {
    result = d3d_context->Map(staging_texture.Get(), 0, D3D11_MAP_READ, 0, &mapped);
  }
  std::vector<std::uint8_t> pixels(static_cast<std::size_t>(width) * height * 4U);
  if (SUCCEEDED(result)) {
    for (UINT row = 0; row < height; ++row) {
      std::copy_n(static_cast<const std::uint8_t*>(mapped.pData) +
                      static_cast<std::size_t>(mapped.RowPitch) * row,
                  width * 4U, pixels.data() + static_cast<std::size_t>(width) * row * 4U);
    }
    d3d_context->Unmap(staging_texture.Get(), 0);
    if (!CreatePngStreamFromBgra(pixels.data(), width, height, output, error)) result = E_FAIL;
  }
  if (FAILED(result)) {
    if (output) output->Release();
    output = nullptr;
    if (error.empty()) error = L"SVGをローカル描画用PNGへ変換できません。";
    return false;
  }
  return true;
}

bool DecodeWebp(const std::filesystem::path& path, unsigned frame_index,
                 RasterImageInfo& info, IStream** png_stream, unsigned* delay_ms,
                 std::wstring& error) {
  std::vector<unsigned char> bytes;
  if (!ReadWebpBytes(path, bytes, error)) return false;
  WebPData data{bytes.data(), bytes.size()};
  WebPAnimDecoderOptions options;
  if (!WebPAnimDecoderOptionsInit(&options)) {
    error = L"WebP decoderの版を初期化できません。";
    return false;
  }
  options.color_mode = MODE_BGRA;
  WebPAnimDecoder* decoder = WebPAnimDecoderNew(&data, &options);
  if (!decoder) {
    error = L"WebPを復号できません: " + path.wstring();
    return false;
  }
  WebPAnimInfo animation{};
  bool succeeded = WebPAnimDecoderGetInfo(decoder, &animation) != 0;
  constexpr std::uint64_t maximum_pixels = 100ULL * 1024 * 1024;
  if (!succeeded || animation.canvas_width == 0 || animation.canvas_height == 0 ||
      animation.frame_count == 0 || frame_index >= animation.frame_count ||
      static_cast<std::uint64_t>(animation.canvas_width) * animation.canvas_height > maximum_pixels) {
    error = L"WebPの寸法、frame番号、または展開サイズが不正です。";
    succeeded = false;
  } else {
    info = {animation.canvas_width, animation.canvas_height, animation.frame_count,
            animation.frame_count > 1};
  }
  if (succeeded && png_stream) {
    std::uint8_t* pixels{};
    int timestamp{};
    int previous_timestamp{};
    for (unsigned index = 0; index <= frame_index; ++index) {
      previous_timestamp = timestamp;
      if (!WebPAnimDecoderGetNext(decoder, &pixels, &timestamp)) {
        error = L"WebP frameを復号できません。";
        succeeded = false;
        break;
      }
    }
    if (succeeded) {
      succeeded = CreatePngStreamFromBgra(pixels, animation.canvas_width,
                                          animation.canvas_height, *png_stream, error);
      if (succeeded && delay_ms) {
        *delay_ms = std::clamp(static_cast<unsigned>(std::max(1, timestamp - previous_timestamp)),
                               20U, 10000U);
      }
    }
  }
  WebPAnimDecoderDelete(decoder);
  return succeeded;
}

bool ReadUnsignedMetadata(IWICMetadataQueryReader* reader, const wchar_t* name,
                          unsigned& value) {
  if (!reader) return false;
  PROPVARIANT property;
  PropVariantInit(&property);
  const HRESULT result = reader->GetMetadataByName(name, &property);
  bool found = SUCCEEDED(result);
  if (found && property.vt == VT_UI1) value = property.bVal;
  else if (found && property.vt == VT_UI2) value = property.uiVal;
  else if (found && property.vt == VT_UI4) value = property.ulVal;
  else found = false;
  PropVariantClear(&property);
  return found;
}

struct GifFrame {
  unsigned left{};
  unsigned top{};
  unsigned width{};
  unsigned height{};
  unsigned delay_ms{100};
  unsigned disposal{};
  std::vector<std::uint8_t> pixels;
};

bool DecodeGifFrame(IWICImagingFactory* factory, IWICBitmapDecoder* decoder,
                    unsigned frame_index, unsigned canvas_width, unsigned canvas_height,
                    GifFrame& frame, std::wstring& error) {
  IWICBitmapFrameDecode* source{};
  IWICMetadataQueryReader* metadata{};
  IWICFormatConverter* converter{};
  HRESULT result = decoder->GetFrame(frame_index, &source);
  UINT width{}, height{};
  if (SUCCEEDED(result)) result = source->GetSize(&width, &height);
  if (SUCCEEDED(result)) source->GetMetadataQueryReader(&metadata);
  frame.width = width;
  frame.height = height;
  ReadUnsignedMetadata(metadata, L"/imgdesc/Left", frame.left);
  ReadUnsignedMetadata(metadata, L"/imgdesc/Top", frame.top);
  unsigned metadata_width{};
  unsigned metadata_height{};
  if (ReadUnsignedMetadata(metadata, L"/imgdesc/Width", metadata_width) &&
      metadata_width != frame.width) result = E_INVALIDARG;
  if (ReadUnsignedMetadata(metadata, L"/imgdesc/Height", metadata_height) &&
      metadata_height != frame.height) result = E_INVALIDARG;
  unsigned delay_centiseconds{};
  if (ReadUnsignedMetadata(metadata, L"/grctlext/Delay", delay_centiseconds) &&
      delay_centiseconds != 0) {
    frame.delay_ms = std::clamp(delay_centiseconds * 10U, 20U, 10000U);
  }
  ReadUnsignedMetadata(metadata, L"/grctlext/Disposal", frame.disposal);
  if (SUCCEEDED(result) && (width == 0 || height == 0 || frame.left > canvas_width ||
                           frame.top > canvas_height || width > canvas_width - frame.left ||
                           height > canvas_height - frame.top ||
                           width > std::numeric_limits<UINT>::max() / 4U ||
                           height > std::numeric_limits<UINT>::max() / (width * 4U))) {
    result = E_INVALIDARG;
  }
  if (SUCCEEDED(result)) result = factory->CreateFormatConverter(&converter);
  if (SUCCEEDED(result)) {
    result = converter->Initialize(source, GUID_WICPixelFormat32bppBGRA,
                                   WICBitmapDitherTypeNone, nullptr, 0.0,
                                   WICBitmapPaletteTypeCustom);
  }
  if (SUCCEEDED(result)) {
    const UINT stride = width * 4U;
    frame.pixels.resize(static_cast<std::size_t>(stride) * height);
    result = converter->CopyPixels(nullptr, stride, static_cast<UINT>(frame.pixels.size()),
                                   frame.pixels.data());
  }
  if (converter) converter->Release();
  if (metadata) metadata->Release();
  if (source) source->Release();
  if (FAILED(result)) {
    error = L"GIF frameの寸法、位置、またはpixelを読み取れません。";
    return false;
  }
  return true;
}

std::array<std::uint8_t, 4> GifBackground(IWICImagingFactory* factory,
                                          IWICBitmapDecoder* decoder,
                                          IWICMetadataQueryReader* metadata) {
  std::array<std::uint8_t, 4> background{};
  unsigned background_index{};
  if (!ReadUnsignedMetadata(metadata, L"/logscrdesc/BackgroundColorIndex", background_index)) {
    return background;
  }
  IWICPalette* palette{};
  if (FAILED(factory->CreatePalette(&palette))) return background;
  WICColor colors[256]{};
  UINT color_count{};
  const HRESULT result = decoder->CopyPalette(palette);
  if (SUCCEEDED(result)) palette->GetColors(static_cast<UINT>(std::size(colors)), colors, &color_count);
  if (background_index < color_count) {
    const WICColor color = colors[background_index];
    background = {static_cast<std::uint8_t>(color), static_cast<std::uint8_t>(color >> 8),
                  static_cast<std::uint8_t>(color >> 16), static_cast<std::uint8_t>(color >> 24)};
  }
  palette->Release();
  return background;
}

void ClearGifFrame(std::vector<std::uint8_t>& canvas, unsigned canvas_width,
                   const GifFrame& frame, const std::array<std::uint8_t, 4>& background) {
  for (unsigned y = 0; y < frame.height; ++y) {
    for (unsigned x = 0; x < frame.width; ++x) {
      const std::size_t target =
          (static_cast<std::size_t>(frame.top + y) * canvas_width + frame.left + x) * 4U;
      std::copy(background.begin(), background.end(), canvas.begin() + target);
    }
  }
}

void OverlayGifFrame(std::vector<std::uint8_t>& canvas, unsigned canvas_width,
                     const GifFrame& frame) {
  for (unsigned y = 0; y < frame.height; ++y) {
    for (unsigned x = 0; x < frame.width; ++x) {
      const std::size_t source = (static_cast<std::size_t>(y) * frame.width + x) * 4U;
      if (frame.pixels[source + 3] == 0) continue;
      const std::size_t target =
          (static_cast<std::size_t>(frame.top + y) * canvas_width + frame.left + x) * 4U;
      std::copy_n(frame.pixels.begin() + source, 4, canvas.begin() + target);
    }
  }
}

bool DecodeGif(const std::filesystem::path& path, unsigned frame_index,
               RasterImageInfo& info, IStream** png_stream, unsigned* delay_ms,
               std::wstring& error) {
  IWICImagingFactory* factory{};
  IWICBitmapDecoder* decoder{};
  IWICMetadataQueryReader* metadata{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) {
    result = factory->CreateDecoderFromFilename(path.c_str(), nullptr, GENERIC_READ,
                                                WICDecodeMetadataCacheOnLoad, &decoder);
  }
  UINT frame_count{};
  if (SUCCEEDED(result)) result = decoder->GetFrameCount(&frame_count);
  if (SUCCEEDED(result)) result = decoder->GetMetadataQueryReader(&metadata);
  unsigned width{};
  unsigned height{};
  if (SUCCEEDED(result) &&
      (!ReadUnsignedMetadata(metadata, L"/logscrdesc/Width", width) ||
       !ReadUnsignedMetadata(metadata, L"/logscrdesc/Height", height))) {
    result = E_INVALIDARG;
  }
  constexpr std::uint64_t maximum_pixels = 100ULL * 1024 * 1024;
  if (SUCCEEDED(result) && (frame_count == 0 || frame_index >= frame_count || width == 0 ||
                           height == 0 ||
                           static_cast<std::uint64_t>(width) * height > maximum_pixels)) {
    result = E_INVALIDARG;
  }
  if (SUCCEEDED(result)) info = {width, height, frame_count, frame_count > 1};
  if (SUCCEEDED(result) && png_stream) {
    const auto background = GifBackground(factory, decoder, metadata);
    std::vector<std::uint8_t> canvas(static_cast<std::size_t>(width) * height * 4U);
    for (std::size_t offset = 0; offset < canvas.size(); offset += 4U) {
      std::copy(background.begin(), background.end(), canvas.begin() + offset);
    }
    std::vector<std::uint8_t> saved_canvas;
    GifFrame previous;
    for (unsigned index = 0; index <= frame_index; ++index) {
      if (index != 0) {
        if (previous.disposal == 2) ClearGifFrame(canvas, width, previous, background);
        else if (previous.disposal == 3 && !saved_canvas.empty()) canvas = saved_canvas;
      }
      GifFrame current;
      if (!DecodeGifFrame(factory, decoder, index, width, height, current, error)) {
        result = E_FAIL;
        break;
      }
      if (current.disposal == 3) saved_canvas = canvas;
      else saved_canvas.clear();
      OverlayGifFrame(canvas, width, current);
      if (index == frame_index) {
        if (delay_ms) *delay_ms = current.delay_ms;
        if (!CreatePngStreamFromBgra(canvas.data(), width, height, *png_stream, error)) {
          result = E_FAIL;
        }
      }
      previous = std::move(current);
    }
  }
  if (metadata) metadata->Release();
  if (decoder) decoder->Release();
  if (factory) factory->Release();
  if (FAILED(result)) {
    if (error.empty()) error = L"GIFのframeを安全に合成できません: " + path.wstring();
    return false;
  }
  return true;
}

bool InspectSvgBytes(std::string_view bytes, bool& safe, std::wstring& message,
                     std::wstring& error) {
  safe = true;
  if (bytes.size() > 16 * 1024 * 1024) {
    safe = false;
    message = L"SVGが安全な表示検査のサイズ上限を超えています。元ファイルは変更していません。";
    return true;
  }
  IStream* stream{};
  IXmlReader* reader{};
  HRESULT result = CreateStreamOnHGlobal(nullptr, TRUE, &stream);
  if (SUCCEEDED(result) && !bytes.empty()) {
    ULONG written{};
    result = stream->Write(bytes.data(), static_cast<ULONG>(bytes.size()), &written);
    if (SUCCEEDED(result) && written != bytes.size()) result = STG_E_WRITEFAULT;
  }
  if (SUCCEEDED(result)) {
    LARGE_INTEGER start{};
    result = stream->Seek(start, STREAM_SEEK_SET, nullptr);
  }
  if (SUCCEEDED(result)) result = CreateXmlReader(__uuidof(IXmlReader), reinterpret_cast<void**>(&reader), nullptr);
  if (SUCCEEDED(result)) result = reader->SetProperty(XmlReaderProperty_DtdProcessing, DtdProcessing_Prohibit);
  if (SUCCEEDED(result)) result = reader->SetInput(stream);

  bool saw_svg{};
  XmlNodeType node{};
  while (reader && SUCCEEDED(result) &&
         SUCCEEDED(result = reader->Read(&node)) && node != XmlNodeType_None && safe) {
    // External stylesheets can be requested through a processing instruction
    // before the root element.  SVG rendering is deliberately local-only, so
    // reject every processing instruction (the XML declaration has its own
    // node type and remains allowed).
    if (node == XmlNodeType_ProcessingInstruction) {
      safe = false;
      break;
    }
    if (node != XmlNodeType_Element) continue;
    const wchar_t* raw_name{};
    UINT name_length{};
    if (FAILED(reader->GetLocalName(&raw_name, &name_length))) { safe = false; break; }
    std::wstring name(raw_name, name_length);
    std::ranges::transform(name, name.begin(), towlower);
    if (!saw_svg) {
      saw_svg = name == L"svg";
      if (!saw_svg) break;
    }
    static constexpr std::wstring_view blocked_elements[] = {
        L"script", L"foreignobject", L"iframe", L"object", L"embed", L"audio", L"video",
        L"image", L"style", L"handler", L"listener"};
    if (std::ranges::find(blocked_elements, name) != std::end(blocked_elements)) {
      safe = false;
      break;
    }
    HRESULT attribute = reader->MoveToFirstAttribute();
    while (attribute == S_OK && safe) {
      const wchar_t* raw_attribute{};
      const wchar_t* raw_prefix{};
      const wchar_t* raw_value{};
      UINT attribute_length{}, prefix_length{}, value_length{};
      if (FAILED(reader->GetLocalName(&raw_attribute, &attribute_length)) ||
          FAILED(reader->GetPrefix(&raw_prefix, &prefix_length)) ||
          FAILED(reader->GetValue(&raw_value, &value_length))) {
        safe = false;
        break;
      }
      std::wstring attribute_name(raw_attribute, attribute_length);
      std::wstring prefix(raw_prefix, prefix_length);
      std::wstring_view value(raw_value, value_length);
      std::ranges::transform(attribute_name, attribute_name.begin(), towlower);
      std::ranges::transform(prefix, prefix.begin(), towlower);
      const bool namespace_declaration = attribute_name == L"xmlns" || prefix == L"xmlns";
      if (namespace_declaration) {
        const auto namespace_value = Trim(value);
        if (namespace_value != L"http://www.w3.org/2000/svg" &&
            namespace_value != L"http://www.w3.org/1999/xlink") safe = false;
      } else if (attribute_name.starts_with(L"on") ||
                 ((attribute_name == L"href" || attribute_name == L"src") && !SafeHref(value)) ||
                 UnsafeReference(value)) {
        safe = false;
      }
      attribute = reader->MoveToNextAttribute();
    }
    reader->MoveToElement();
  }
  if (reader) reader->Release();
  if (stream) stream->Release();
  if (!saw_svg || FAILED(result)) safe = false;
  if (!safe) MarkUnsafeSvg(safe, message);
  return true;
}

bool InspectSvg(const std::filesystem::path& path, bool& safe, std::wstring& message,
                std::wstring& error) {
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
  return InspectSvgBytes(bytes, safe, message, error);
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
  const auto extension = Lower(path.extension().wstring());
  if (extension == L".webp") {
    return DecodeWebp(path, 0, info, nullptr, nullptr, error);
  }
  if (extension == L".gif") return DecodeGif(path, 0, info, nullptr, nullptr, error);
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
  const auto extension = Lower(path.extension().wstring());
  if (extension == L".webp") {
    RasterImageInfo info;
    return DecodeWebp(path, frame_index, info, &stream, &delay_ms, error);
  }
  if (extension == L".gif") {
    RasterImageInfo info;
    return DecodeGif(path, frame_index, info, &stream, &delay_ms, error);
  }
  if (extension == L".svg") {
    if (frame_index != 0) {
      error = L"SVGに複数frameはありません。";
      return false;
    }
    return RasterizeSvgToPng(path, stream, error);
  }
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
    if (error.empty()) {
      error = L"アニメーション画像のframeを描画用に変換できません: " + path.wstring();
    }
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
