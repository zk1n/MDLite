#include "core/Document.h"
#include "editor/EditorAdapter.h"
#include "git/Conflict.h"
#include "assets/Assets.h"
#include "assets/StorageAdapter.h"
#include "calendar/JapaneseHolidays.h"
#include "markdown/Markdown.h"
#include "profiles/Profiles.h"
#include "process/ProcessRunner.h"
#include "search/Search.h"
#include "search/Replace.h"
#include "settings/Settings.h"
#include "table/Table.h"
#include "workspace/Workspace.h"
#include "workspace/Trust.h"

#include <windows.h>
#include <wincodec.h>
#include <richedit.h>
#include <shlwapi.h>
#include <webp/encode.h>
#include <webp/mux.h>

#include <array>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <map>
#include <string>
#include <thread>
#include <vector>

namespace {

int failures = 0;
int checks = 0;

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    std::cerr << "FAIL: " << message << '\n';
    ++failures;
  }
}

void WriteBytes(const std::filesystem::path& path, const std::vector<unsigned char>& bytes);

bool WriteValidRaster(const std::filesystem::path& path, REFGUID container,
                      REFWICPixelFormatGUID requested_format) {
  IWICImagingFactory* factory{};
  IWICStream* stream{};
  IWICBitmapEncoder* encoder{};
  IWICBitmapFrameEncode* frame{};
  IPropertyBag2* properties{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) result = factory->CreateStream(&stream);
  if (SUCCEEDED(result)) result = stream->InitializeFromFilename(path.c_str(), GENERIC_WRITE);
  if (SUCCEEDED(result)) result = factory->CreateEncoder(container, nullptr, &encoder);
  if (SUCCEEDED(result)) result = encoder->Initialize(stream, WICBitmapEncoderNoCache);
  if (SUCCEEDED(result)) result = encoder->CreateNewFrame(&frame, &properties);
  if (SUCCEEDED(result)) result = frame->Initialize(properties);
  if (SUCCEEDED(result)) result = frame->SetSize(2, 1);
  WICPixelFormatGUID format = requested_format;
  if (SUCCEEDED(result)) result = frame->SetPixelFormat(&format);
  const std::array<unsigned char, 8> bgra{0, 0, 255, 255, 0, 255, 0, 255};
  const std::array<unsigned char, 6> bgr{0, 0, 255, 0, 255, 0};
  if (SUCCEEDED(result) && format == GUID_WICPixelFormat32bppBGRA)
    result = frame->WritePixels(1, 8, 8, const_cast<BYTE*>(bgra.data()));
  else if (SUCCEEDED(result) && format == GUID_WICPixelFormat24bppBGR)
    result = frame->WritePixels(1, 6, 6, const_cast<BYTE*>(bgr.data()));
  else if (SUCCEEDED(result))
    result = WINCODEC_ERR_UNSUPPORTEDPIXELFORMAT;
  if (SUCCEEDED(result)) result = frame->Commit();
  if (SUCCEEDED(result)) result = encoder->Commit();
  if (properties) properties->Release();
  if (frame) frame->Release();
  if (encoder) encoder->Release();
  if (stream) stream->Release();
  if (factory) factory->Release();
  if (FAILED(result)) std::cerr << "WIC raster fixture HRESULT: 0x" << std::hex << static_cast<unsigned long>(result) << std::dec << '\n';
  return SUCCEEDED(result);
}

bool WriteAnimatedWebp(const std::filesystem::path& path) {
  WebPAnimEncoderOptions animation_options;
  WebPConfig config;
  if (!WebPAnimEncoderOptionsInit(&animation_options) || !WebPConfigInit(&config)) return false;
  animation_options.minimize_size = 1;
  config.lossless = 1;
  config.quality = 100.0F;
  WebPAnimEncoder* encoder = WebPAnimEncoderNew(2, 1, &animation_options);
  if (!encoder) return false;
  bool succeeded = true;
  const std::array<std::array<std::uint8_t, 8>, 2> frames{{
      {0x00, 0x00, 0xFF, 0xFF, 0x00, 0xFF, 0x00, 0xFF},
      {0xFF, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF},
  }};
  const std::array<int, 2> timestamps{0, 100};
  for (std::size_t index = 0; index < frames.size() && succeeded; ++index) {
    WebPPicture picture;
    succeeded = WebPPictureInit(&picture) != 0;
    if (!succeeded) break;
    picture.use_argb = 1;
    picture.width = 2;
    picture.height = 1;
    succeeded = WebPPictureImportBGRA(&picture, frames[index].data(), 8) != 0 &&
                WebPAnimEncoderAdd(encoder, &picture, timestamps[index], &config) != 0;
    WebPPictureFree(&picture);
  }
  if (succeeded) succeeded = WebPAnimEncoderAdd(encoder, nullptr, 350, nullptr) != 0;
  WebPData encoded;
  WebPDataInit(&encoded);
  if (succeeded) succeeded = WebPAnimEncoderAssemble(encoder, &encoded) != 0;
  if (succeeded) {
    WriteBytes(path, std::vector<unsigned char>(encoded.bytes, encoded.bytes + encoded.size));
  }
  WebPDataClear(&encoded);
  WebPAnimEncoderDelete(encoder);
  return succeeded;
}

bool InsertNativeRichEditImage(const std::filesystem::path& path, bool convert_to_png) {
  HMODULE rich_edit_module = LoadLibraryW(L"Msftedit.dll");
  if (!rich_edit_module) return false;
  HWND editor = CreateWindowExW(0, MSFTEDIT_CLASS, L"", WS_POPUP | ES_MULTILINE,
                                0, 0, 200, 100, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
  if (!editor) {
    FreeLibrary(rich_edit_module);
    return false;
  }
  IStream* stream{};
  unsigned delay{};
  std::wstring error;
  HRESULT stream_result = S_OK;
  if (convert_to_png) {
    if (!mdlite::CreateRasterFramePngStream(path, 0, stream, delay, error)) stream_result = E_FAIL;
  } else {
    stream_result = SHCreateStreamOnFileEx(path.c_str(), STGM_READ | STGM_SHARE_DENY_WRITE,
                                          FILE_ATTRIBUTE_NORMAL, FALSE, nullptr, &stream);
  }
  HRESULT inserted = stream_result;
  if (SUCCEEDED(stream_result)) {
    RICHEDIT_IMAGE_PARAMETERS parameters{};
    parameters.xWidth = 2540;
    parameters.yHeight = 1270;
    parameters.Ascent = parameters.yHeight;
    parameters.Type = TA_BASELINE;
    parameters.pwszAlternateText = L"native-insertion-test";
    parameters.pIStream = stream;
    inserted = static_cast<HRESULT>(
        SendMessageW(editor, EM_INSERTIMAGE, 0, reinterpret_cast<LPARAM>(&parameters)));
  }
  const int text_length = GetWindowTextLengthW(editor);
  if (stream) stream->Release();
  DestroyWindow(editor);
  FreeLibrary(rich_edit_module);
  if (FAILED(inserted) || text_length != 1) {
    std::cerr << "RichEdit insertion failed for " << path.filename().string()
              << ": hr=0x" << std::hex << static_cast<unsigned long>(inserted)
              << std::dec << ", inline-length=" << text_length << '\n';
  }
  return SUCCEEDED(inserted) && text_length == 1;
}

bool WritePartialDisposalGif(const std::filesystem::path& path) {
  // 3x1 canvas, palette: black, red, green, blue, white. Each pixel is preceded by
  // an LZW clear code so the tiny fixture does not depend on dictionary growth.
  const std::vector<unsigned char> bytes{
      'G','I','F','8','9','a', 0x03,0x00, 0x01,0x00, 0x82,0x00,0x00,
      0x00,0x00,0x00, 0xFF,0x00,0x00, 0x00,0xFF,0x00, 0x00,0x00,0xFF,
      0xFF,0xFF,0xFF, 0x00,0x00,0x00, 0x00,0x00,0x00, 0x00,0x00,0x00,
      // Frame 0: full red canvas, keep.
      0x21,0xF9,0x04,0x04,0x0A,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x03,0x00,0x01,0x00,0x00,
      0x03,0x04,0x18,0x18,0x18,0x09,0x00,
      // Frame 1: transparent at x=0, green at x=1; restore its rect to background.
      0x21,0xF9,0x04,0x09,0x14,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x02,0x00,0x01,0x00,0x00,
      0x03,0x03,0x08,0x28,0x09,0x00,
      // Frame 2: blue at x=2; restore the composed canvas that preceded it.
      0x21,0xF9,0x04,0x0C,0x1E,0x00,0x00,0x00,
      0x2C,0x02,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,
      0x03,0x02,0x38,0x09,0x00,
      // Frame 3: white at x=0, proving disposal 3 restored the pre-frame-2 canvas.
      0x21,0xF9,0x04,0x04,0x28,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,
      0x03,0x02,0x48,0x09,0x00, 0x3B};
  WriteBytes(path, bytes);
  return std::filesystem::file_size(path) == bytes.size();
}

bool DecodePngStream(IStream* stream, unsigned& width, unsigned& height,
                     std::vector<unsigned char>& bgra) {
  width = height = 0;
  bgra.clear();
  if (!stream) return false;
  LARGE_INTEGER start{};
  if (FAILED(stream->Seek(start, STREAM_SEEK_SET, nullptr))) return false;
  IWICImagingFactory* factory{};
  IWICBitmapDecoder* decoder{};
  IWICBitmapFrameDecode* frame{};
  IWICFormatConverter* converter{};
  HRESULT result = CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER,
                                    IID_PPV_ARGS(&factory));
  if (SUCCEEDED(result)) {
    result = factory->CreateDecoderFromStream(stream, nullptr, WICDecodeMetadataCacheOnLoad,
                                              &decoder);
  }
  if (SUCCEEDED(result)) result = decoder->GetFrame(0, &frame);
  if (SUCCEEDED(result)) result = factory->CreateFormatConverter(&converter);
  if (SUCCEEDED(result)) {
    result = converter->Initialize(frame, GUID_WICPixelFormat32bppBGRA,
                                   WICBitmapDitherTypeNone, nullptr, 0.0,
                                   WICBitmapPaletteTypeCustom);
  }
  UINT decoded_width{}, decoded_height{};
  if (SUCCEEDED(result)) result = converter->GetSize(&decoded_width, &decoded_height);
  if (SUCCEEDED(result) && decoded_width != 0 && decoded_height != 0 &&
      decoded_width <= std::numeric_limits<UINT>::max() / 4U &&
      decoded_height <= std::numeric_limits<UINT>::max() / (decoded_width * 4U)) {
    const UINT stride = decoded_width * 4U;
    bgra.resize(static_cast<std::size_t>(stride) * decoded_height);
    result = converter->CopyPixels(nullptr, stride, static_cast<UINT>(bgra.size()), bgra.data());
  } else if (SUCCEEDED(result)) {
    result = E_INVALIDARG;
  }
  if (converter) converter->Release();
  if (frame) frame->Release();
  if (decoder) decoder->Release();
  if (factory) factory->Release();
  if (FAILED(result)) {
    bgra.clear();
    return false;
  }
  width = decoded_width;
  height = decoded_height;
  return true;
}

bool WriteValidPng(const std::filesystem::path& path) {
  return WriteValidRaster(path, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA);
}

bool WriteValidJpeg(const std::filesystem::path& path) {
  return WriteValidRaster(path, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR);
}

void WriteBytes(const std::filesystem::path& path, const std::vector<unsigned char>& bytes) {
  std::ofstream output(path, std::ios::binary);
  output.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
}

void WriteAscii(const std::filesystem::path& path, std::string_view text) {
  WriteBytes(path, std::vector<unsigned char>(text.begin(), text.end()));
}

std::vector<unsigned char> ReadBytes(const std::filesystem::path& path) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  const auto size = input.tellg();
  std::vector<unsigned char> bytes(static_cast<std::size_t>(size));
  input.seekg(0);
  input.read(reinterpret_cast<char*>(bytes.data()), size);
  return bytes;
}

void TestUtf8NoOp(const std::filesystem::path& root) {
  const auto path = root / L"utf8.md";
  const std::vector<unsigned char> original{'#', ' ', 'T', 'i', 't', 'l', 'e', '\n', 0xE6, 0x9C, 0xAC, 0xE6, 0x96, 0x87};
  WriteBytes(path, original);
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "strict UTF-8 document loads");
  Check(document.encoding() == mdlite::TextEncoding::Utf8, "UTF-8 is detected");
  Check(document.line_ending() == mdlite::LineEnding::Lf, "LF is detected");
  Check(document.Save(error), "unmodified save is a no-op");
  Check(ReadBytes(path) == original, "unmodified save preserves exact bytes");
  const auto saved_text = document.text();
  document.MarkEdited(saved_text + L" temporary");
  Check(document.dirty(), "an edited document becomes dirty");
  document.MarkEdited(saved_text);
  Check(!document.dirty(), "returning to the saved checkpoint clears Dirty without a write");
}

void TestUntitledDocument(const std::filesystem::path& root) {
  mdlite::Document document;
  document.CreateUntitled(root / L".mdlite/.state/untitled/test.md");
  Check(document.untitled() && document.dirty() && document.text().empty(),
        "new untitled document is dirty without creating a normal file");
  std::wstring error;
  Check(!document.Save(error), "untitled document requires an explicit save destination");
  document.MarkEdited(L"draft");
  const auto destination = root / L"saved-untitled.md";
  Check(document.SaveAs(destination, error) && !document.untitled() && !document.dirty(),
        "Save As converts an untitled document into a normal saved document");
  Check(ReadBytes(destination) == std::vector<unsigned char>{'d','r','a','f','t'},
        "untitled Save As writes the exact UTF-8 body");
}

void TestSaveAs(const std::filesystem::path& root) {
  const auto source = root / L"save-as-source.md";
  const auto destination = root / L"save-as-copy.md";
  WriteBytes(source, {'o', 'l', 'd'});
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(source, error), "save-as fixture loads");
  document.MarkEdited(L"new content");
  Check(document.SaveAs(destination, error), "save-as creates a distinct new file");
  Check(document.path() == std::filesystem::absolute(destination).lexically_normal(),
        "save-as updates the document path");
  Check(ReadBytes(source) == std::vector<unsigned char>({'o', 'l', 'd'}),
        "save-as leaves the original file unchanged");
  Check(ReadBytes(destination) ==
            std::vector<unsigned char>({'n', 'e', 'w', ' ', 'c', 'o', 'n', 't', 'e', 'n', 't'}),
        "save-as writes the edited bytes");

  mdlite::Document second;
  Check(second.Load(source, error), "overwrite-refusal fixture loads");
  second.MarkEdited(L"blocked");
  Check(!second.SaveAs(destination, error), "save-as refuses an existing destination");
  Check(ReadBytes(destination) ==
            std::vector<unsigned char>({'n', 'e', 'w', ' ', 'c', 'o', 'n', 't', 'e', 'n', 't'}),
        "save-as refusal does not overwrite the destination");
}

void TestCp932RoundTrip(const std::filesystem::path& root) {
  const auto path = root / L"cp932.md";
  const std::string source = "\x83\x65\x83\x58\x83\x67\r\n";
  WriteBytes(path, std::vector<unsigned char>(source.begin(), source.end()));
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "CP932 document loads");
  Check(document.encoding() == mdlite::TextEncoding::Cp932, "CP932 is detected");
  document.MarkEdited(document.text() + L"追記");
  Check(document.Save(error), "CP932 representable edit saves");
  mdlite::Document reloaded;
  Check(reloaded.Load(path, error), "saved CP932 document reloads");
  Check(reloaded.text().ends_with(L"追記"), "CP932 edit round-trips");
  reloaded.MarkEdited(reloaded.text() + L"😀");
  Check(!reloaded.Save(error), "unrepresentable CP932 edit is rejected");
}

void TestExternalConflict(const std::filesystem::path& root) {
  const auto path = root / L"conflict.md";
  WriteBytes(path, {'o', 'l', 'd'});
  mdlite::Document document;
  std::wstring error;
  Check(document.Load(path, error), "conflict fixture loads");
  document.MarkEdited(L"editor");
  Sleep(20);
  WriteBytes(path, {'e', 'x', 't', 'e', 'r', 'n', 'a', 'l'});
  Check(!document.Save(error), "external modification blocks overwrite");
  Check(ReadBytes(path) == std::vector<unsigned char>({'e', 'x', 't', 'e', 'r', 'n', 'a', 'l'}),
        "external bytes remain intact");

  const auto stealth_path = root / L"same-size-time.md";
  WriteBytes(stealth_path, {'o', 'l', 'd'});
  mdlite::Document stealth_document;
  Check(stealth_document.Load(stealth_path, error), "same-size conflict fixture loads");
  const auto original_time = std::filesystem::last_write_time(stealth_path);
  stealth_document.MarkEdited(L"app");
  WriteBytes(stealth_path, {'n', 'e', 'w'});
  std::filesystem::last_write_time(stealth_path, original_time);
  Check(!stealth_document.Save(error), "same-size and restored-time external edit is detected");
  Check(ReadBytes(stealth_path) == std::vector<unsigned char>({'n', 'e', 'w'}),
        "same-size external bytes remain intact");

  const auto before_replace_path = root / L"before-replace.md";
  WriteBytes(before_replace_path, {'o', 'l', 'd'});
  mdlite::Document before_replace;
  Check(before_replace.Load(before_replace_path, error), "before-replace fixture loads");
  before_replace.MarkEdited(L"editor");
  Check(!before_replace.Save(error, [&](mdlite::SaveStage stage, const auto& target) {
          if (stage == mdlite::SaveStage::BeforeReplace) WriteBytes(target, {'r', 'a', 'c', 'e'});
        }),
        "external write after final preflight blocks replace");
  Check(ReadBytes(before_replace_path) == std::vector<unsigned char>({'r', 'a', 'c', 'e'}),
        "pre-replace race preserves the external bytes");

  const auto guarded_replace_path = root / L"guarded-replace.md";
  WriteBytes(guarded_replace_path, {'o', 'l', 'd'});
  mdlite::Document guarded_replace;
  Check(guarded_replace.Load(guarded_replace_path, error), "guarded-replace fixture loads");
  guarded_replace.MarkEdited(L"editor");
  bool competing_writer_blocked = false;
  Check(guarded_replace.Save(error, [&](mdlite::SaveStage stage, const auto& target) {
          if (stage != mdlite::SaveStage::BeforeReplaceGuarded) return;
          const HANDLE writer = CreateFileW(target.c_str(), GENERIC_WRITE,
                                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                            nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
          if (writer == INVALID_HANDLE_VALUE) {
            competing_writer_blocked = GetLastError() == ERROR_SHARING_VIOLATION;
          } else {
            CloseHandle(writer);
          }
        }),
        "save succeeds while the guarded target handle is held");
  Check(competing_writer_blocked, "guarded replace excludes a competing writer");
  Check(ReadBytes(guarded_replace_path) == std::vector<unsigned char>({'e', 'd', 'i', 't', 'o', 'r'}),
        "guarded replace commits the editor bytes");

  const auto after_replace_path = root / L"after-replace.md";
  WriteBytes(after_replace_path, {'o', 'l', 'd'});
  mdlite::Document after_replace;
  Check(after_replace.Load(after_replace_path, error), "after-replace fixture loads");
  after_replace.MarkEdited(L"editor");
  Check(!after_replace.Save(error, [&](mdlite::SaveStage stage, const auto& target) {
          if (stage == mdlite::SaveStage::AfterReplace) WriteBytes(target, {'r', 'a', 'c', 'e'});
        }),
        "external write after replace is not promoted to the saved baseline");
  Check(after_replace.dirty(), "post-replace race leaves the editor revision dirty");
  Check(ReadBytes(after_replace_path) == std::vector<unsigned char>({'r', 'a', 'c', 'e'}),
        "post-replace race remains visible for conflict recovery");
}

void TestEmptyEncodingAndInvalidBom(const std::filesystem::path& root) {
  std::wstring error;
  const auto utf8_path = root / L"empty-utf8.md";
  WriteBytes(utf8_path, {'t', 'e', 'x', 't'});
  mdlite::Document utf8;
  Check(utf8.Load(utf8_path, error), "UTF-8 empty-save fixture loads");
  utf8.MarkEdited(L"");
  Check(utf8.Save(error), "UTF-8 document can be saved empty");
  Check(ReadBytes(utf8_path).empty(), "empty UTF-8 has zero bytes");

  const auto bom_path = root / L"empty-bom.md";
  WriteBytes(bom_path, {0xEF, 0xBB, 0xBF, 'x'});
  mdlite::Document bom;
  Check(bom.Load(bom_path, error), "UTF-8 BOM empty-save fixture loads");
  bom.MarkEdited(L"");
  Check(bom.Save(error), "UTF-8 BOM document can be saved empty");
  Check(ReadBytes(bom_path) == std::vector<unsigned char>({0xEF, 0xBB, 0xBF}),
        "empty UTF-8 BOM retains only the BOM");

  const auto cp932_path = root / L"empty-cp932.md";
  WriteBytes(cp932_path, {0x83, 0x65, 0x83, 0x58, 0x83, 0x67});
  mdlite::Document cp932;
  Check(cp932.Load(cp932_path, error) && cp932.encoding() == mdlite::TextEncoding::Cp932,
        "CP932 empty-save fixture loads as CP932");
  cp932.MarkEdited(L"");
  Check(cp932.Save(error), "CP932 document can be saved empty");
  Check(ReadBytes(cp932_path).empty(), "empty CP932 has zero bytes");

  const auto invalid_bom_path = root / L"invalid-bom.md";
  WriteBytes(invalid_bom_path, {0xEF, 0xBB, 0xBF, 0x83, 0x65});
  mdlite::Document invalid_bom;
  Check(!invalid_bom.Load(invalid_bom_path, error),
        "invalid UTF-8 after BOM is not silently reclassified as CP932");
}

void TestEditorLineEndingBoundary(const std::filesystem::path& root) {
  const auto lf_path = root / L"lf-edit.md";
  WriteBytes(lf_path, {'a', '\n', 'b', '\n'});
  mdlite::Document lf_document;
  std::wstring error;
  Check(lf_document.Load(lf_path, error), "LF editor fixture loads");
  lf_document.MarkEditedFromEditor(L"a\r\nb changed\r\n");
  Check(lf_document.Save(error), "LF editor edit saves");
  Check(ReadBytes(lf_path) == std::vector<unsigned char>({'a', '\n', 'b', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\n'}),
        "RichEdit CRLF expansion does not rewrite an LF document");

  const auto crlf_path = root / L"crlf-edit.md";
  WriteBytes(crlf_path, {'a', '\r', '\n', 'b', '\r', '\n'});
  mdlite::Document crlf_document;
  Check(crlf_document.Load(crlf_path, error), "CRLF editor fixture loads");
  crlf_document.MarkEditedFromEditor(L"a\r\nb changed\r\n");
  Check(crlf_document.Save(error), "CRLF editor edit saves");
  Check(ReadBytes(crlf_path) ==
            std::vector<unsigned char>({'a', '\r', '\n', 'b', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\r', '\n'}),
        "CRLF document keeps CRLF after editor edit");

  const auto mixed_path = root / L"mixed-edit.md";
  WriteBytes(mixed_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_document;
  Check(mixed_document.Load(mixed_path, error), "mixed-ending editor fixture loads");
  mixed_document.MarkEditedFromEditor(L"a changed\r\nb\r\nc");
  Check(mixed_document.Save(error), "mixed-ending editor edit saves");
  Check(ReadBytes(mixed_path) ==
            std::vector<unsigned char>({'a', ' ', 'c', 'h', 'a', 'n', 'g', 'e', 'd', '\r', '\n', 'b', '\n', 'c'}),
        "existing mixed line-ending sequence survives editor expansion");

  const auto mixed_delete_path = root / L"mixed-delete.md";
  WriteBytes(mixed_delete_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_delete;
  Check(mixed_delete.Load(mixed_delete_path, error), "mixed delete fixture loads");
  mixed_delete.MarkEditedFromEditor(L"b\r\nc");
  Check(mixed_delete.Save(error), "mixed document saves after deleting first line");
  Check(ReadBytes(mixed_delete_path) == std::vector<unsigned char>({'b', '\n', 'c'}),
        "deleting a CRLF line does not transfer CRLF to the next LF line");

  const auto mixed_insert_path = root / L"mixed-insert.md";
  WriteBytes(mixed_insert_path, {'a', '\r', '\n', 'b', '\n', 'c'});
  mdlite::Document mixed_insert;
  Check(mixed_insert.Load(mixed_insert_path, error), "mixed insert fixture loads");
  mixed_insert.MarkEditedFromEditor(L"a\r\nnew\r\nb\r\nc");
  Check(mixed_insert.Save(error), "mixed document saves after inserting a line");
  Check(ReadBytes(mixed_insert_path) ==
            std::vector<unsigned char>({'a', '\r', '\n', 'n', 'e', 'w', '\r', '\n', 'b', '\n', 'c'}),
        "insertion keeps unchanged mixed endings attached to their source lines");
}

void TestMarkdown() {
  const std::wstring source = L"---\ntitle: '# metadata'\n---\n# Parent\ntext **bold**\n```\n## code\n```\n## Child\n";
  const auto parsed = mdlite::ParseMarkdown(source);
  Check(parsed.headings.size() == 2, "front matter and fenced headings are excluded");
  Check(parsed.headings[0].level == 1 && parsed.headings[0].text == L"Parent", "H1 is parsed");
  Check(parsed.headings[1].level == 2 && parsed.headings[1].text == L"Child", "H2 is parsed");
  Check(!parsed.spans.empty(), "presentation spans are produced");
  const auto visual = mdlite::ParseMarkdown(
      L"![代替](assets/a.png)\n| A | B |\n| :--- | ---: |\n| 1 | 2 |\n");
  Check(visual.images.size() == 1 && visual.images.front().target == L"assets/a.png",
        "image references retain source ranges and targets");
  Check(visual.tables.size() == 1 && visual.tables[0].rows.size() == 3 &&
            visual.tables[0].alignments.size() == 2 &&
            visual.tables[0].alignments[0] == mdlite::TableAlignment::Left &&
            visual.tables[0].alignments[1] == mdlite::TableAlignment::Right,
        "Markdown presentation consumes the shared GFM parser with cell ranges and alignment");
  Check(mdlite::ParseMarkdown(L"h | v\n: | ---\na | b").tables.empty(),
        "Markdown presentation rejects malformed GFM delimiter rows");
  const std::wstring separated_tables =
      L"lead\n| A | B |\n| :--- | ---: |\n| left | right |\nmiddle\n"
      L"| C | D |\n| --- | :---: |\n| x | y |\n"
      L"```md\n| F | G |\n| --- | --- |\n| fenced | content |\n```\n";
  const auto multiple_tables = mdlite::ParseMarkdown(separated_tables);
  const auto first_table_begin = separated_tables.find(L"| A | B |");
  const auto first_table_row_end = separated_tables.find(L"| left | right |") +
                                  std::wstring_view(L"| left | right |").size();
  const auto second_table_begin = separated_tables.find(L"| C | D |");
  const auto second_table_row_end = separated_tables.find(L"| x | y |") +
                                   std::wstring_view(L"| x | y |").size();
  Check(multiple_tables.tables.size() == 2 &&
            multiple_tables.tables[0].begin == first_table_begin &&
            multiple_tables.tables[0].end == first_table_row_end &&
            multiple_tables.tables[0].rows.size() == 3 &&
            multiple_tables.tables[0].alignments[0] == mdlite::TableAlignment::Left &&
            multiple_tables.tables[0].alignments[1] == mdlite::TableAlignment::Right &&
            multiple_tables.tables[1].begin == second_table_begin &&
            multiple_tables.tables[1].end == second_table_row_end &&
            multiple_tables.tables[1].rows.size() == 3 &&
            multiple_tables.tables[1].alignments[1] == mdlite::TableAlignment::Center,
        "Markdown parses separated GFM tables at forward offsets and excludes fenced table-shaped text");
  const auto links = mdlite::ParseMarkdown(L"[local](notes/a.md#heading) ![image](a.png) <https://example.test/>\n");
  Check(links.links.size() == 2 && links.links[0].target == L"notes/a.md#heading" &&
            links.links[1].target == L"https://example.test/",
        "Markdown and autolinks are parsed while image syntax stays separate");
  const auto resized = mdlite::ParseMarkdown(
      L"<img src=\"assets/a.png\" alt=\"sample\" width=\"480\">\n");
  Check(resized.images.size() == 1 && resized.images.front().target == L"assets/a.png" &&
            resized.images.front().width_dip == 480,
        "safe generated img markup retains target and display width");
  const auto escaped_html_image = mdlite::ParseMarkdown(
      L"<img src=\"assets/a&amp;b.png\" alt=\"&quot;A &lt;B&gt;\" width=\"320\">");
  Check(escaped_html_image.images.size() == 1 &&
            escaped_html_image.images.front().target == L"assets/a&b.png" &&
            escaped_html_image.images.front().alternate_text == L"\"A <B>" &&
            mdlite::ImageHtml(escaped_html_image.images.front().alternate_text,
                              escaped_html_image.images.front().target, 640) ==
                L"<img src=\"assets/a&amp;b.png\" alt=\"&quot;A &lt;B&gt;\" width=\"640\">",
        "decoded HTML image fields are escaped once when Markdown images become HTML");
  const std::wstring html_image_with_attributes =
      L"<img TITLE=\"2 > 1\" DATA-SRC=\"fallback.png\" SRC=\"assets/a&#38;b.png\" "
      L"ALT=\"&#65; &#x1F600;\" WIDTH=\"320\">";
  const auto html_image_attributes = mdlite::ParseMarkdown(html_image_with_attributes);
  const auto resized_html_image = mdlite::ResizeHtmlImageWidth(html_image_with_attributes, 640);
  Check(html_image_attributes.images.size() == 1 &&
            html_image_attributes.images.front().target == L"assets/a&b.png" &&
            html_image_attributes.images.front().alternate_text == L"A \U0001F600" &&
            html_image_attributes.images.front().end == html_image_with_attributes.size() &&
            resized_html_image && *resized_html_image ==
                L"<img TITLE=\"2 > 1\" DATA-SRC=\"fallback.png\" SRC=\"assets/a&#38;b.png\" "
                L"ALT=\"&#65; &#x1F600;\" WIDTH=\"640\">",
        "HTML attributes use exact names, quoted greater-than boundaries, numeric references, and source-preserving width resize");
  const std::wstring duplicate_markdown_target = L"![a.png](a.png)";
  const auto duplicate_markdown_parse = mdlite::ParseMarkdown(duplicate_markdown_target);
  std::optional<std::wstring> rewritten_markdown_target;
  if (!duplicate_markdown_parse.images.empty())
    rewritten_markdown_target = mdlite::ReplaceImageReferenceTarget(
        duplicate_markdown_target, duplicate_markdown_parse.images.front(),
        L"https://cdn.example/a.png");
  Check(rewritten_markdown_target &&
            *rewritten_markdown_target == L"![a.png](https://cdn.example/a.png)",
        "image destination replacement leaves matching Markdown alt text unchanged");
  const std::wstring balanced_markdown_destinations =
      L"![nested](https://cdn.example/a(b(c)).png) "
      L"![escaped](https://cdn.example/a\\(b\\).png)";
  const auto balanced_markdown_parse = mdlite::ParseMarkdown(balanced_markdown_destinations);
  const auto balanced_markdown_images = mdlite::ParseMarkdownImages(balanced_markdown_destinations);
  const std::wstring nested_target = L"https://cdn.example/a(b(c)).png";
  const std::wstring escaped_target = L"https://cdn.example/a\\(b\\).png";
  const auto nested_target_begin = balanced_markdown_destinations.find(nested_target);
  const auto escaped_target_begin = balanced_markdown_destinations.find(escaped_target);
  Check(balanced_markdown_parse.images.size() == 2 && balanced_markdown_images.size() == 2 &&
            balanced_markdown_parse.images[0].target == nested_target &&
            balanced_markdown_parse.images[0].target_begin == nested_target_begin &&
            balanced_markdown_parse.images[0].target_end == nested_target_begin + nested_target.size() &&
            balanced_markdown_destinations.substr(
                balanced_markdown_parse.images[0].target_begin,
                balanced_markdown_parse.images[0].target_end -
                    balanced_markdown_parse.images[0].target_begin) == nested_target &&
            balanced_markdown_images[0].target == nested_target &&
            balanced_markdown_images[0].target_begin == nested_target_begin &&
            balanced_markdown_images[0].target_end == nested_target_begin + nested_target.size() &&
            balanced_markdown_parse.images[1].target == escaped_target &&
            balanced_markdown_images[1].target == escaped_target &&
            balanced_markdown_parse.images[1].target_begin == escaped_target_begin &&
            balanced_markdown_parse.images[1].target_end == escaped_target_begin + escaped_target.size() &&
            balanced_markdown_images[1].target_begin == escaped_target_begin &&
            balanced_markdown_images[1].target_end == escaped_target_begin + escaped_target.size(),
        "Markdown scanners preserve exact target spans for nested and escaped parentheses");
  std::optional<std::wstring> rewritten_balanced_target;
  if (!balanced_markdown_parse.images.empty())
    rewritten_balanced_target = mdlite::ReplaceImageReferenceTarget(
        balanced_markdown_destinations, balanced_markdown_parse.images.front(),
        L"https://cdn.example/new(v2).png");
  Check(rewritten_balanced_target &&
            rewritten_balanced_target->starts_with(L"![nested](https://cdn.example/new(v2).png) ") &&
            rewritten_balanced_target->ends_with(L"![escaped](https://cdn.example/a\\(b\\).png)"),
        "Markdown replacement preserves balanced parentheses and escaped neighboring targets");
  const std::wstring angle_markdown_destination =
      L"![angle](<https://cdn.example/a(b).png>)";
  const auto angle_markdown_parse = mdlite::ParseMarkdown(angle_markdown_destination);
  const auto angle_markdown_images = mdlite::ParseMarkdownImages(angle_markdown_destination);
  const std::wstring angle_target = L"https://cdn.example/a(b).png";
  const auto angle_target_begin = angle_markdown_destination.find(angle_target);
  std::optional<std::wstring> rewritten_angle_target;
  if (!angle_markdown_parse.images.empty())
    rewritten_angle_target = mdlite::ReplaceImageReferenceTarget(
        angle_markdown_destination, angle_markdown_parse.images.front(),
        L"https://cdn.example/new(v2).png");
  Check(angle_markdown_parse.images.size() == 1 && angle_markdown_images.size() == 1 &&
            angle_markdown_parse.images.front().target == angle_target &&
            angle_markdown_parse.images.front().target_begin == angle_target_begin &&
            angle_markdown_parse.images.front().target_end == angle_target_begin + angle_target.size() &&
            angle_markdown_images.front().target == angle_target &&
            angle_markdown_images.front().target_begin == angle_target_begin &&
            angle_markdown_images.front().target_end == angle_target_begin + angle_target.size() &&
            rewritten_angle_target &&
            *rewritten_angle_target == L"![angle](<https://cdn.example/new(v2).png>)",
        "angle-bracket destinations expose only their content span and preserve their delimiters on replacement");
  const std::wstring angle_unmatched_paren_source = L"![angle](<old>)";
  const auto angle_unmatched_paren_parse = mdlite::ParseMarkdown(angle_unmatched_paren_source);
  const auto* angle_unmatched_paren_image = angle_unmatched_paren_parse.images.empty()
      ? nullptr
      : &angle_unmatched_paren_parse.images.front();
  std::optional<std::wstring> angle_unmatched_paren_replacement;
  if (angle_unmatched_paren_image)
    angle_unmatched_paren_replacement = mdlite::ReplaceImageReferenceTarget(
        angle_unmatched_paren_source, *angle_unmatched_paren_image,
        L"https://cdn.example/x).png");
  std::optional<std::wstring> angle_space_replacement;
  if (angle_unmatched_paren_image)
    angle_space_replacement = mdlite::ReplaceImageReferenceTarget(
        angle_unmatched_paren_source, *angle_unmatched_paren_image,
        L"https://cdn.example/a b).png");
  Check(angle_unmatched_paren_image && angle_unmatched_paren_replacement &&
            *angle_unmatched_paren_replacement ==
                L"![angle](<https://cdn.example/x).png>)",
        "angle-bracket replacement preserves the wrapper around otherwise-unmatched URL parentheses");
  Check(angle_space_replacement &&
            *angle_space_replacement == L"![angle](<https://cdn.example/a b).png>)" &&
            !mdlite::ReplaceImageReferenceTarget(
                angle_unmatched_paren_source, *angle_unmatched_paren_image,
                L"https://cdn.example/<x).png") &&
            !mdlite::ReplaceImageReferenceTarget(
                angle_unmatched_paren_source, *angle_unmatched_paren_image,
                L"https://cdn.example/x>.png") &&
            !mdlite::ReplaceImageReferenceTarget(
                angle_unmatched_paren_source, *angle_unmatched_paren_image,
                L"https://cdn.example/x\t.png"),
        "angle destinations allow spaces but reject controls and raw angle delimiters in replacement content");
  const std::wstring nested_image_description =
      L"![outer [inner](inner.png)](outer.png)";
  const auto nested_description_parse = mdlite::ParseMarkdown(nested_image_description);
  const auto nested_description_images = mdlite::ParseMarkdownImages(nested_image_description);
  const std::wstring escaped_bracket_description =
      L"![escaped\\](inner.png)](outer.png)";
  const auto escaped_bracket_parse = mdlite::ParseMarkdown(escaped_bracket_description);
  const auto escaped_bracket_images = mdlite::ParseMarkdownImages(escaped_bracket_description);
  Check(nested_description_parse.images.size() == 1 && nested_description_images.size() == 1 &&
            nested_description_parse.images.front().alternate_text == L"outer [inner](inner.png)" &&
            nested_description_parse.images.front().target == L"outer.png" &&
            nested_description_images.front().target == L"outer.png" &&
            escaped_bracket_parse.images.size() == 1 && escaped_bracket_images.size() == 1 &&
            escaped_bracket_parse.images.front().alternate_text == L"escaped\\](inner.png)" &&
            escaped_bracket_parse.images.front().target == L"outer.png" &&
            escaped_bracket_images.front().target == L"outer.png",
        "image destinations follow the outer label bracket across nested links and escaped brackets");
  const std::wstring code_span_brackets_in_image_label =
      L"![outer `](inner.png)` tail](outer.png)";
  const auto code_span_label_parse = mdlite::ParseMarkdown(code_span_brackets_in_image_label);
  const auto code_span_label_images =
      mdlite::ParseMarkdownImages(code_span_brackets_in_image_label);
  const auto outer_target_begin = code_span_brackets_in_image_label.find(L"outer.png");
  Check(code_span_label_parse.images.size() == 1 && code_span_label_images.size() == 1 &&
            code_span_label_parse.images.front().target == L"outer.png" &&
            code_span_label_parse.images.front().target_begin == outer_target_begin &&
            code_span_label_parse.images.front().target_end == outer_target_begin + 9 &&
            code_span_label_parse.images.front().end == code_span_brackets_in_image_label.size() &&
            code_span_label_images.front().target == L"outer.png" &&
            code_span_label_images.front().target_begin == outer_target_begin &&
            code_span_label_images.front().target_end == outer_target_begin + 9,
        "code-span brackets in an image label do not close the label or become its destination");
  const std::wstring bare_title_destination =
      L"![title](https://cdn.example/a(b).png \"image title\")";
  const auto bare_title_parse = mdlite::ParseMarkdown(bare_title_destination);
  const auto bare_title_images = mdlite::ParseMarkdownImages(bare_title_destination);
  const std::wstring bare_title_target = L"https://cdn.example/a(b).png";
  const auto bare_title_target_begin = bare_title_destination.find(bare_title_target);
  Check(bare_title_parse.images.size() == 1 && bare_title_images.size() == 1 &&
            bare_title_parse.images.front().target == bare_title_target &&
            bare_title_parse.images.front().target_begin == bare_title_target_begin &&
            bare_title_parse.images.front().target_end ==
                bare_title_target_begin + bare_title_target.size() &&
            bare_title_images.front().target == bare_title_target &&
            bare_title_images.front().target_begin == bare_title_target_begin &&
            bare_title_images.front().target_end == bare_title_target_begin + bare_title_target.size() &&
            bare_title_parse.images.front().end == bare_title_destination.size(),
        "bare image destinations stop before quoted titles while the source span includes the closing syntax");
  std::optional<std::wstring> rewritten_bare_title;
  if (!bare_title_parse.images.empty())
    rewritten_bare_title = mdlite::ReplaceImageReferenceTarget(
        bare_title_destination, bare_title_parse.images.front(),
        L"https://cdn.example/replacement(v2).png");
  Check(rewritten_bare_title &&
            *rewritten_bare_title ==
                L"![title](https://cdn.example/replacement(v2).png \"image title\")",
        "Markdown target replacement preserves the quoted title");
  const std::wstring angle_space_title_destination =
      L"![title](<https://cdn.example/a b.png> 'image title')";
  const auto angle_space_title_parse = mdlite::ParseMarkdown(angle_space_title_destination);
  const auto angle_space_title_images = mdlite::ParseMarkdownImages(angle_space_title_destination);
  const std::wstring angle_space_target = L"https://cdn.example/a b.png";
  const auto angle_space_target_begin = angle_space_title_destination.find(angle_space_target);
  std::optional<std::wstring> rewritten_angle_space_title;
  if (!angle_space_title_parse.images.empty())
    rewritten_angle_space_title = mdlite::ReplaceImageReferenceTarget(
        angle_space_title_destination, angle_space_title_parse.images.front(),
        L"https://cdn.example/replacement(v2).png");
  Check(angle_space_title_parse.images.size() == 1 && angle_space_title_images.size() == 1 &&
            angle_space_title_parse.images.front().target == angle_space_target &&
            angle_space_title_parse.images.front().target_begin == angle_space_target_begin &&
            angle_space_title_parse.images.front().target_end ==
                angle_space_target_begin + angle_space_target.size() &&
            angle_space_title_images.front().target == angle_space_target &&
            angle_space_title_images.front().target_begin == angle_space_target_begin &&
            angle_space_title_images.front().target_end ==
                angle_space_target_begin + angle_space_target.size() &&
            rewritten_angle_space_title &&
            *rewritten_angle_space_title ==
                L"![title](<https://cdn.example/replacement(v2).png> 'image title')",
        "angle image destinations retain internal spaces and titles outside the replaceable target span");
  const std::wstring parenthesized_title_destination =
      L"![title](https://cdn.example/a.png (outer (nested) title))";
  const auto parenthesized_title_parse = mdlite::ParseMarkdown(parenthesized_title_destination);
  Check(parenthesized_title_parse.images.size() == 1 &&
            parenthesized_title_parse.images.front().target == L"https://cdn.example/a.png" &&
            parenthesized_title_parse.images.front().end == parenthesized_title_destination.size(),
        "parenthesized CommonMark titles allow nested parentheses and retain the closing source span");
  const std::wstring unterminated_title_destination =
      L"![bad](<https://cdn.example/a b.png> \"unterminated)";
  const std::wstring missing_title_wrapper =
      L"![bad](<https://cdn.example/a b.png> \"title\"";
  Check(mdlite::ParseMarkdown(unterminated_title_destination).images.empty() &&
            mdlite::ParseMarkdownImages(unterminated_title_destination).empty() &&
            mdlite::ParseMarkdown(missing_title_wrapper).images.empty() &&
            mdlite::ParseMarkdownImages(missing_title_wrapper).empty(),
        "angle destinations require both a closed title and the enclosing destination delimiter");
  const auto markdown_without_parentheses = mdlite::ParseMarkdown(L"![a.png](a.png)");
  const auto* plain_markdown_image = markdown_without_parentheses.images.empty()
      ? nullptr
      : &markdown_without_parentheses.images.front();
  const std::wstring markdown_destination_control =
      L"https://cdn.example/bad" + std::wstring(1, L'\x01') + L"path.png";
  Check(plain_markdown_image &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/bad).png") &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/bad(a.png") &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/bad path.png") &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/bad\tpath.png") &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, markdown_destination_control) &&
            !mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/dangling\\") &&
            mdlite::ReplaceImageReferenceTarget(
                L"![a.png](a.png)", *plain_markdown_image, L"https://cdn.example/escaped\\).png") ==
                std::optional<std::wstring>(L"![a.png](https://cdn.example/escaped\\).png)"),
        "Markdown replacements reject unrepresentable whitespace or parentheses and accept escaped parentheses");
  const std::wstring duplicate_html_target =
      L"<img alt=\"a&amp;b.png\" src=\"a&amp;b.png\">";
  const auto duplicate_html_parse = mdlite::ParseMarkdown(duplicate_html_target);
  std::optional<std::wstring> rewritten_html_target;
  if (!duplicate_html_parse.images.empty())
    rewritten_html_target = mdlite::ReplaceImageReferenceTarget(
        duplicate_html_target, duplicate_html_parse.images.front(),
        L"https://cdn.example/a.png?x=1&y=2");
  Check(rewritten_html_target &&
            *rewritten_html_target ==
                L"<img alt=\"a&amp;b.png\" src=\"https://cdn.example/a.png?x=1&amp;y=2\">",
        "HTML image upload changes only the parsed src attribute and escapes its new URL");
  const auto unquoted_html_width = mdlite::ResizeHtmlImageWidth(L"<img src=assets/a.png />", 320);
  Check(unquoted_html_width && *unquoted_html_width == L"<img src=assets/a.png width=\"320\" />",
        "unquoted image attributes accept a new width without changing their source");
  const std::wstring duplicate_html_source =
      L"<img src=\"a.png\" src=\"b.png\" alt=\"![nested](x.png)\" width=\"120\">";
  const auto duplicate_html_image = mdlite::ParseMarkdown(duplicate_html_source);
  Check(duplicate_html_image.images.empty() &&
            mdlite::ParseMarkdownImages(duplicate_html_source).empty() &&
            !mdlite::ResizeHtmlImageWidth(duplicate_html_source, 320) &&
            !mdlite::ResizeHtmlImageWidth(L"<img src=\"unterminated.png\"", 320),
        "ambiguous or malformed HTML images are rejected instead of rewritten");
  const auto mixed_image_order = mdlite::ParseMarkdown(
      L"<img src=\"html.png\" alt=\"html\">![markdown](markdown.png)");
  Check(mixed_image_order.images.size() == 2 &&
            mixed_image_order.images[0].begin < mixed_image_order.images[1].begin &&
            mixed_image_order.images[0].target == L"html.png" &&
            mixed_image_order.images[1].target == L"markdown.png",
        "mixed HTML and Markdown image references remain in source order");
  const std::wstring markdown_in_html_attribute =
      L"<img src=\"https://example.invalid/a.png\" alt=\"![guide](hint.png)\">"
      L"![next](next.png)";
  const auto html_attribute_overlap = mdlite::ParseMarkdown(markdown_in_html_attribute);
  const auto html_attribute_images = mdlite::ParseMarkdownImages(markdown_in_html_attribute);
  const auto nested_markdown_begin = markdown_in_html_attribute.find(L"![guide]");
  Check(html_attribute_overlap.images.size() == 2 && html_attribute_images.size() == 2 &&
            html_attribute_overlap.images[0].target == L"https://example.invalid/a.png" &&
            html_attribute_overlap.images[1].target == L"next.png" &&
            html_attribute_overlap.images[0].end <= html_attribute_overlap.images[1].begin &&
            mdlite::FindImageAtSourcePosition(html_attribute_overlap, nested_markdown_begin) ==
                &html_attribute_overlap.images[0],
        "Markdown-looking alt text remains inside its HTML image range while the touching Markdown image stays selectable");
  const std::wstring markdown_alt_html_tag =
      L"![outer <img src=\"inner.png\">](outer.png)";
  const auto markdown_alt_html_image = mdlite::ParseMarkdown(markdown_alt_html_tag);
  const auto markdown_alt_html_images = mdlite::ParseMarkdownImages(markdown_alt_html_tag);
  const auto inner_html_begin = markdown_alt_html_tag.find(L"<img");
  Check(markdown_alt_html_image.images.size() == 1 && markdown_alt_html_images.size() == 1 &&
            markdown_alt_html_image.images.front().target == L"outer.png" &&
            markdown_alt_html_image.images.front().begin == 0 &&
            markdown_alt_html_image.images.front().end == markdown_alt_html_tag.size() &&
            mdlite::FindImageAtSourcePosition(markdown_alt_html_image, inner_html_begin) ==
                &markdown_alt_html_image.images.front(),
        "HTML-looking alt text remains owned by its enclosing Markdown image in both image scanners");
  const std::wstring escaped_markdown_image = L"\\![literal](literal.png)";
  const auto escaped_markdown_parse = mdlite::ParseMarkdown(escaped_markdown_image);
  const auto escaped_markdown_images = mdlite::ParseMarkdownImages(escaped_markdown_image);
  const auto escaped_markdown_snapshot = mdlite::BuildNativeEditorSnapshot(escaped_markdown_image);
  Check(escaped_markdown_parse.images.empty() && escaped_markdown_images.empty() &&
            escaped_markdown_snapshot.collapsed.empty() &&
            escaped_markdown_snapshot.view == escaped_markdown_image,
        "an escaped Markdown image opener remains literal in both parsers and the native editor");
  const std::wstring even_backslashes_before_image = L"\\\\![real](real.png)";
  const auto even_backslashes_image_parse = mdlite::ParseMarkdown(even_backslashes_before_image);
  const auto even_backslashes_image_snapshot =
      mdlite::BuildNativeEditorSnapshot(even_backslashes_before_image);
  Check(even_backslashes_image_parse.images.size() == 1 &&
            even_backslashes_image_parse.images.front().target == L"real.png" &&
            mdlite::ParseMarkdownImages(even_backslashes_before_image).size() == 1 &&
            even_backslashes_image_snapshot.collapsed.size() == 1,
        "an even backslash run leaves a following Markdown image opener active");
  const std::wstring inline_code_image_syntax =
      L"`![single](single.png)` and ``![code](code.png) <img src=\"code-html.png\">`` and "
      L"![real](real.png)";
  const auto inline_code_image_parse = mdlite::ParseMarkdown(inline_code_image_syntax);
  const auto inline_code_image_only = mdlite::ParseMarkdownImages(inline_code_image_syntax);
  Check(inline_code_image_parse.images.size() == 1 && inline_code_image_only.size() == 1 &&
            inline_code_image_parse.images.front().target == L"real.png" &&
            inline_code_image_parse.images.front().begin ==
                inline_code_image_syntax.find(L"![real]"),
        "Markdown and HTML image syntax inside inline code stays literal in both image scanners");
  const std::wstring mixed_backtick_runs =
      L"`open ``![inside](nested.png)` close`` done`` ![real](real.png)";
  const auto mixed_backtick_parse = mdlite::ParseMarkdown(mixed_backtick_runs);
  Check(mixed_backtick_parse.images.size() == 1 &&
            mixed_backtick_parse.images.front().target == L"real.png" &&
            mdlite::ParseMarkdownImages(mixed_backtick_runs).size() == 1,
        "backtick run indexing pairs nearest equal delimiters and skips nested runs without rescanning suffixes");
  const std::wstring escaped_inline_code_image =
      std::wstring(1, L'\\') + L'\x60' + L"![escaped](escaped.png)" +
      std::wstring(1, L'\\') + L'\x60';
  const auto escaped_inline_code_parse = mdlite::ParseMarkdown(escaped_inline_code_image);
  const auto escaped_inline_code_images = mdlite::ParseMarkdownImages(escaped_inline_code_image);
  Check(escaped_inline_code_parse.images.size() == 1 &&
            escaped_inline_code_parse.images.front().target == L"escaped.png" &&
            escaped_inline_code_images.size() == 1 &&
            escaped_inline_code_images.front().target == L"escaped.png",
        "backslash-escaped code punctuation does not hide a Markdown image");
  const std::wstring escaped_prefix_code_span =
      std::wstring(1, L'\\') + L'\x60' + L'\x60' + L"![hidden](hidden.png)" +
      L'\x60' + L" ![visible](visible.png)";
  const auto escaped_prefix_parse = mdlite::ParseMarkdown(escaped_prefix_code_span);
  const auto escaped_prefix_images = mdlite::ParseMarkdownImages(escaped_prefix_code_span);
  Check(escaped_prefix_parse.images.size() == 1 &&
            escaped_prefix_parse.images.front().target == L"visible.png" &&
            escaped_prefix_images.size() == 1 &&
            escaped_prefix_images.front().target == L"visible.png",
        "an escaped first tick leaves a valid suffix run as the code opener");
  const std::wstring even_backslashes_before_code =
      std::wstring(2, L'\\') + L'\x60' + L"![hidden](hidden.png)" + L'\x60';
  Check(mdlite::ParseMarkdown(even_backslashes_before_code).images.empty() &&
            mdlite::ParseMarkdownImages(even_backslashes_before_code).empty(),
        "even backslash parity leaves the following code opener active");
  const std::wstring escaped_backslash_inside_code =
      std::wstring(1, L'\x60') + L"inside \\" + L'\x60' + L" ![after](after.png)";
  const auto escaped_backslash_inside_parse = mdlite::ParseMarkdown(escaped_backslash_inside_code);
  const auto escaped_backslash_inside_images =
      mdlite::ParseMarkdownImages(escaped_backslash_inside_code);
  Check(escaped_backslash_inside_parse.images.size() == 1 &&
            escaped_backslash_inside_parse.images.front().target == L"after.png" &&
            escaped_backslash_inside_images.size() == 1 &&
            escaped_backslash_inside_images.front().target == L"after.png",
        "backslash is literal when an inline code span is already open");
  std::wstring many_inline_codes_and_images;
  many_inline_codes_and_images.reserve(2000 * 40);
  for (int index = 0; index < 2000; ++index) {
    many_inline_codes_and_images += std::wstring(1, L'\x60') + L"code" + L'\x60' +
        L" ![image](image.png) ";
  }
  const auto many_inline_code_parse = mdlite::ParseMarkdown(many_inline_codes_and_images);
  const auto many_inline_code_images = mdlite::ParseMarkdownImages(many_inline_codes_and_images);
  Check(many_inline_code_parse.images.size() == 2000 &&
            many_inline_code_images.size() == 2000,
        "many ordered code spans and image candidates retain linear source-order exclusion");
  const std::wstring html_attribute_backtick_opener =
      L"<img src=\"html.png\" alt=\"title`\">![markdown](markdown.png)`";
  const auto html_attribute_backtick_parse = mdlite::ParseMarkdown(html_attribute_backtick_opener);
  const auto html_attribute_backtick_images = mdlite::ParseMarkdownImages(html_attribute_backtick_opener);
  Check(html_attribute_backtick_parse.images.size() == 2 &&
            html_attribute_backtick_parse.images[0].target == L"html.png" &&
            html_attribute_backtick_parse.images[1].target == L"markdown.png" &&
            html_attribute_backtick_images.size() == 2 &&
            html_attribute_backtick_images[0].target == L"html.png" &&
            html_attribute_backtick_images[1].target == L"markdown.png",
        "a tag-first attribute backtick does not pair with an outside closer or hide the following Markdown image");
  const std::wstring html_attribute_paired_backticks =
      L"<img src=\"html.png\" alt=\"`title`\"> `![hidden](hidden.png)` ![markdown](markdown.png)";
  const auto html_attribute_paired_parse = mdlite::ParseMarkdown(html_attribute_paired_backticks);
  const auto html_attribute_paired_images = mdlite::ParseMarkdownImages(html_attribute_paired_backticks);
  Check(html_attribute_paired_parse.images.size() == 2 &&
            html_attribute_paired_parse.images[0].target == L"html.png" &&
            html_attribute_paired_parse.images[1].target == L"markdown.png" &&
            html_attribute_paired_images.size() == 2 &&
            html_attribute_paired_images[0].target == L"html.png" &&
            html_attribute_paired_images[1].target == L"markdown.png",
        "paired attribute backticks leave a separate following code span and Markdown image intact");
  const std::wstring code_open_before_html_attribute =
      L"`code <img src=\"html.png\" alt=\"title`\">![markdown](markdown.png)";
  const auto code_open_before_html_parse = mdlite::ParseMarkdown(code_open_before_html_attribute);
  const auto code_open_before_html_images = mdlite::ParseMarkdownImages(code_open_before_html_attribute);
  const auto attribute_delimiter = code_open_before_html_attribute.find(L'`', 1);
  bool earlier_code_closed_at_attribute{};
  for (const auto& span : code_open_before_html_parse.spans) {
    if (span.kind == mdlite::SpanKind::Code && span.begin == 1 &&
        span.end == attribute_delimiter) {
      earlier_code_closed_at_attribute = true;
      break;
    }
  }
  Check(code_open_before_html_parse.images.size() == 1 &&
            code_open_before_html_parse.images.front().target == L"markdown.png" &&
            code_open_before_html_images.size() == 1 &&
            code_open_before_html_images.front().target == L"markdown.png" &&
            earlier_code_closed_at_attribute,
        "an earlier code opener can still close on an equal-length backtick inside a later HTML tag");
  const std::wstring multiline_code_image_syntax =
      L"`open\n![inside](code.png)\n<img src=\"code-html.png\">\nclose` ![real](real.png)";
  const auto multiline_code_image_parse = mdlite::ParseMarkdown(multiline_code_image_syntax);
  const auto multiline_code_image_only = mdlite::ParseMarkdownImages(multiline_code_image_syntax);
  Check(multiline_code_image_parse.images.size() == 1 &&
            multiline_code_image_only.size() == 1 &&
            multiline_code_image_parse.images.front().target == L"real.png" &&
            multiline_code_image_parse.images.front().begin ==
                multiline_code_image_syntax.find(L"![real]"),
        "matched multiline code spans keep Markdown and HTML image syntax literal until their closing delimiter");
  const std::wstring unmatched_inline_code = L"`unmatched\n\n![after](after.png)";
  const auto unmatched_inline_code_parse = mdlite::ParseMarkdown(unmatched_inline_code);
  const std::wstring fenced_inline_code_boundary =
      L"`unmatched\n```\n![fenced](fenced.png)\n```\n![after](after.png)";
  const auto fenced_inline_code_parse = mdlite::ParseMarkdown(fenced_inline_code_boundary);
  Check(unmatched_inline_code_parse.images.size() == 1 &&
            unmatched_inline_code_parse.images.front().target == L"after.png" &&
            mdlite::ParseMarkdownImages(unmatched_inline_code).size() == 1 &&
            fenced_inline_code_parse.images.size() == 1 &&
            fenced_inline_code_parse.images.front().target == L"after.png" &&
            mdlite::ParseMarkdownImages(fenced_inline_code_boundary).size() == 1,
        "unmatched inline-code openers reset at blank and fenced block boundaries");
  const std::wstring list_continuation_code =
      L"- `open\n  ![inside](list-code.png)\n  close` and ![real](list-real.png)";
  const auto list_continuation_parse = mdlite::ParseMarkdown(list_continuation_code);
  const auto list_continuation_images = mdlite::ParseMarkdownImages(list_continuation_code);
  const std::wstring list_item_boundary = L"- `open\n- ![new](new-item.png) `close";
  const auto list_item_boundary_parse = mdlite::ParseMarkdown(list_item_boundary);
  Check(list_continuation_parse.images.size() == 1 && list_continuation_images.size() == 1 &&
            list_continuation_parse.images.front().target == L"list-real.png" &&
            list_item_boundary_parse.images.size() == 1 &&
            list_item_boundary_parse.images.front().target == L"new-item.png" &&
            mdlite::ParseMarkdownImages(list_item_boundary).size() == 1,
        "inline-code spans carry through supported list continuations but reset at a new item");
  const auto adjacent_images = mdlite::ParseMarkdown(L"![first](a.png)![second](b.png)");
  Check(adjacent_images.images.size() == 2 &&
            mdlite::FindImageAtSourcePosition(adjacent_images, adjacent_images.images[0].end) ==
                &adjacent_images.images[1] &&
            mdlite::FindImageAtSourcePosition(adjacent_images, adjacent_images.images.back().end) ==
                &adjacent_images.images.back(),
        "caret at an adjacent image boundary selects the image starting there and final-end fallback remains available");
  const auto oversized = mdlite::ParseMarkdown(
      L"<img src=\"assets/a.png\" alt=\"sample\" width=\"4294967295\">\n");
  Check(oversized.images.size() == 1 && oversized.images.front().width_dip == 8192,
        "extreme image display width is clamped before reaching native layout");
  Check(mdlite::BuildMarkdownEditorSnapshot(
            L"<img src=\"assets/a.png\" alt=\"sample\" width=\"480\">").view == L"\uFFFC",
        "resized img markup remains a native derived image object");

  const auto blocks = mdlite::ParseMarkdown(
      L"paragraph *em* __strong__\n---\n> quote\n- bullet\n1. ordered\n- [x] task\n"
      L"    indented code\n<div>kept</div>\n");
  const auto has_block = [&](mdlite::BlockKind kind) {
    return std::ranges::any_of(blocks.blocks, [kind](const auto& block) { return block.kind == kind; });
  };
  Check(has_block(mdlite::BlockKind::Paragraph) && has_block(mdlite::BlockKind::ThematicBreak) &&
            has_block(mdlite::BlockKind::BlockQuote) && has_block(mdlite::BlockKind::BulletListItem) &&
            has_block(mdlite::BlockKind::OrderedListItem) && has_block(mdlite::BlockKind::TaskListItem) &&
            has_block(mdlite::BlockKind::IndentedCode) && has_block(mdlite::BlockKind::Html),
        "initial CommonMark and GFM block set retains source ranges");
  Check(std::ranges::any_of(blocks.spans, [](const auto& span) {
          return span.kind == mdlite::SpanKind::Emphasis;
        }), "single emphasis is recognized separately from strong emphasis");
  const std::wstring indented_code_with_delimiter = L"    a | b\n|---|---|\n";
  const auto indented_code_parse = mdlite::ParseMarkdown(indented_code_with_delimiter);
  const auto indented_code_snapshot =
      mdlite::BuildNativeEditorSnapshot(indented_code_with_delimiter);
  Check(indented_code_parse.tables.empty() && indented_code_snapshot.tables.empty(),
        "an indented-code pipe row cannot become a native GFM table after a delimiter");
  Check(indented_code_snapshot.view ==
            mdlite::CanonicalizeNativeText(indented_code_with_delimiter),
        "indented-code source text remains intact in the native projection");
  const std::wstring indented_code_context =
      L"    ```\n"
      L"    # literal [link](hidden.md) **text**\n"
      L"![visible](visible.png)\n"
      L"# real heading\n";
  const auto indented_code_context_parse = mdlite::ParseMarkdown(indented_code_context);
  const auto indented_code_context_images =
      mdlite::ParseMarkdownImages(indented_code_context);
  const auto indented_heading_move = mdlite::MoveHeadingSection(
      indented_code_context, indented_code_context.find(L"# literal"),
      indented_code_context.size());
  Check(indented_code_context_parse.images.size() == 1 &&
            indented_code_context_parse.images.front().target == L"visible.png" &&
            indented_code_context_images.size() == 1 &&
            indented_code_context_images.front().target == L"visible.png" &&
            indented_code_context_parse.links.empty() &&
            indented_code_context_parse.headings.size() == 1 &&
            indented_code_context_parse.headings.front().text == L"real heading" &&
            !indented_heading_move.changed &&
            indented_heading_move.text == indented_code_context &&
            std::ranges::none_of(indented_code_context_parse.spans, [](const auto& span) {
              return span.kind == mdlite::SpanKind::Strong;
            }),
        "four-space backticks do not open a fence or parse code contents as Markdown");

  const std::wstring list_source = L"  - nested\n1. basic\n12) numbered\n- [x] checked\n"
                                   L"```\n- fenced\n```\n-plain\nordinary - text\n";
  const auto lists = mdlite::ParseMarkdown(list_source);
  std::vector<mdlite::StyleSpan> list_markers;
  std::vector<mdlite::StyleSpan> ordered_markers;
  for (const auto& span : lists.spans) {
    if (span.kind == mdlite::SpanKind::ListMarker) list_markers.push_back(span);
    if (span.kind == mdlite::SpanKind::OrderedListMarker) ordered_markers.push_back(span);
  }
  const std::vector<std::wstring> expected_markers{L"- ", L"- "};
  const std::vector<std::wstring> expected_ordered_markers{L"1. ", L"12) "};
  Check(list_markers.size() == expected_markers.size() &&
            ordered_markers.size() == expected_ordered_markers.size(),
        "bullet/task and ordered prefixes have distinct presentation spans");
  bool marker_offsets_match = list_markers.size() == expected_markers.size();
  for (std::size_t index = 0; marker_offsets_match && index < expected_markers.size(); ++index) {
    const auto& span = list_markers[index];
    marker_offsets_match = span.end - span.begin == expected_markers[index].size() &&
                           list_source.substr(span.begin, span.end - span.begin) == expected_markers[index];
  }
  bool ordered_offsets_match = ordered_markers.size() == expected_ordered_markers.size();
  for (std::size_t index = 0; ordered_offsets_match && index < expected_ordered_markers.size(); ++index) {
    const auto& span = ordered_markers[index];
    ordered_offsets_match = span.end - span.begin == expected_ordered_markers[index].size() &&
                            list_source.substr(span.begin, span.end - span.begin) == expected_ordered_markers[index];
  }
  Check(marker_offsets_match && ordered_offsets_match,
        "list spans point to exact source prefixes, including nested indentation offsets");
  const auto task_marker = std::ranges::find_if(list_markers, [&](const auto& span) {
    return span.begin == list_source.find(L"- [x] checked");
  });
  Check(task_marker != list_markers.end() &&
            list_source.substr(task_marker->end, 3) == L"[x]",
        "task checkbox remains outside the hidden list marker span");

  const std::wstring outline = L"# First\nintro\n## Child\nchild\n# Second\nend\n# Third\nlast\n";
  const auto outline_parse = mdlite::ParseMarkdown(outline);
  const auto moved = mdlite::MoveHeadingSection(outline, outline_parse.headings[0].begin,
                                                outline_parse.headings[3].begin);
  Check(moved.changed && moved.text.starts_with(L"# Second"),
        "outline move changes the source order");
  Check(moved.text.find(L"## Child\nchild") > moved.text.find(L"# First\nintro"),
        "outline move keeps child heading content with its parent");
}

void TestEditorAdapter() {
  const auto snapshot = mdlite::BuildEditorSnapshot(L"a\nb😀\r\nc");
  Check(snapshot.view == L"a\r\nb😀\r\nc", "editor view expands only lone LF to CRLF");
  Check(snapshot.inserted_crs.size() == 1 && snapshot.source_size == 8,
        "editor mapping stores one compact index only for each inserted CR");
  Check(snapshot.SourceToView(2) == 3, "source-to-view mapping accounts for expanded LF");
  Check(snapshot.ViewToSource(2) == 1, "inserted CR maps to the source LF boundary");

  const std::wstring selection_source = L"before\r\nafter😀";
  const auto selection_snapshot = mdlite::BuildNativeTextEditorSnapshot(selection_source);
  const std::size_t selection_begin = selection_source.find(L"after");
  const std::size_t selection_end = selection_source.size();
  const std::size_t native_begin = selection_snapshot.SourceToNative(selection_begin);
  const std::size_t native_end = selection_snapshot.SourceToNative(selection_end);
  const auto forward_selection = mdlite::NativeSelectionToSource(
      selection_snapshot, native_begin, native_end, false);
  const auto reverse_selection = mdlite::NativeSelectionToSource(
      selection_snapshot, native_end, native_begin, true);
  Check(forward_selection == mdlite::SourceSelection{selection_begin, selection_end} &&
            reverse_selection == mdlite::SourceSelection{selection_end, selection_begin},
        "selection mapping preserves anchor, active endpoint, and reversed direction");
  const auto native_reverse = mdlite::SourceSelectionToNative(selection_snapshot, reverse_selection);
  Check(native_reverse == mdlite::SourceSelection{native_end, native_begin},
        "source selection restores native endpoint direction after line-break mapping");

  const auto edited = mdlite::ApplyEditorText(snapshot, L"a\nb😀\r\nc", L"a\r\nnew\r\nb😀\r\nc");
  Check(edited.changed && edited.source == L"a\nnew\nb😀\r\nc",
        "range transaction preserves LF and existing CRLF without whole-document normalization");
  const auto deleted = mdlite::ApplyEditorText(mdlite::BuildEditorSnapshot(L"a\r\nb\nc"), L"a\r\nb\nc", L"b\r\nc");
  Check(deleted.source == L"b\nc", "range transaction keeps the surviving mixed line ending");
  const auto image = mdlite::BuildMarkdownEditorSnapshot(L"before ![alt](img.png) after");
  Check(image.view == L"before \uFFFC after", "derived editor view represents an image without changing source");
  Check(image.SourceToView(22) == 8 && image.ViewToSource(8) == 22,
        "compact mapping resumes immediately after a collapsed image range");
  const auto image_deleted = mdlite::ApplyEditorText(
      image, L"before ![alt](img.png) after", L"before  after",
      mdlite::EditorEditHint{
          mdlite::SourceSelection{image.collapsed.front().source_begin,
                                 image.collapsed.front().source_end},
          mdlite::EditorEditKind::Delete});
  Check(!image_deleted.identity_ambiguous && image_deleted.source == L"before  after",
        "a verified deletion removes the derived image's complete source range");
  const auto native_lines = mdlite::BuildNativeTextEditorSnapshot(L"a\nb\r\nc");
  Check(native_lines.view == L"a\rb\rc", "native editor snapshot uses one CR per paragraph");
  Check(native_lines.SourceToNative(2) == 2 && native_lines.NativeToSource(2) == 2 &&
            native_lines.NativeToSource(1) == 1,
        "native line-ending mapping remains compact and boundary-safe");
  const std::wstring unicode_native_source = L"😀\r\n| A | B |";
  const auto unicode_native = mdlite::BuildNativeTextEditorSnapshot(unicode_native_source);
  Check(unicode_native.SourceToNative(2) == 2 && unicode_native.SourceToNative(3) == 2 &&
            unicode_native.SourceToNative(4) == 3 && unicode_native.NativeToSource(2) == 2 &&
            unicode_native.NativeToSource(3) == 4,
        "native mapping keeps UTF-16 surrogate units distinct at a CRLF boundary");
  const auto unicode_native_edit = mdlite::ApplyEditorText(
      unicode_native, unicode_native_source, L"😀\r\n| AX | B |");
  Check(unicode_native_edit.source == L"😀\r\n| AX | B |",
        "native transaction preserves a CRLF boundary after a UTF-16 edit");
  const auto lone_native_lines = mdlite::BuildNativeTextEditorSnapshot(L"a\nb\nc");
  Check(lone_native_lines.native_discontinuities.empty(),
        "same-width lone LF boundaries do not allocate native discontinuity records");
  Check(mdlite::CanonicalizeNativeText(L"a\r\nb\nc") == L"a\rb\rc",
        "native text canonicalization collapses CRLF and lone LF to one paragraph boundary");
  const auto native_raw_edit = mdlite::ApplyEditorText(
      native_lines, L"a\nb\r\nc", L"a\r\nx\nb\r\nc");
  Check(native_raw_edit.source == L"a\nx\nb\r\nc",
        "native transaction canonicalizes alternate newline export before source mapping");
  const auto native_image = mdlite::BuildNativeEditorSnapshot(L"a![x](p.png)\nb");
  Check(native_image.view == L"a\uFFFC\rb", "native Markdown snapshot collapses images and normalizes LF");
  Check(native_image.SourceToNative(1) == 1 && native_image.NativeToSource(1) == 1 &&
            native_image.NativeToSource(2) == std::wstring_view(L"a![x](p.png)").size(),
        "native image mapping anchors object boundaries without a dense map");
  Check(mdlite::BuildNativeEditorSnapshot(L"![remote](https://example.test/a.png)").view ==
            L"![remote](https://example.test/a.png)" &&
            mdlite::BuildNativeEditorSnapshot(L"![1](1.png)![2](2.png)").view ==
                L"![1](1.png)![2](2.png)",
        "native table projection retains the existing remote and ambiguous image rules");
  const std::wstring adjacent_source = L"A![x](p.png)B";
  const auto adjacent_snapshot = mdlite::BuildMarkdownEditorSnapshot(adjacent_source);
  const auto before_image_edit = mdlite::ApplyEditorText(
      adjacent_snapshot, adjacent_source, L"a\uFFFCB",
      mdlite::EditorEditHint{mdlite::SourceSelection{0, 1},
                            mdlite::EditorEditKind::Replace});
  const std::wstring after_before_image_edit = L"a![x](p.png)B";
  const auto after_before_image_snapshot =
      mdlite::BuildMarkdownEditorSnapshot(after_before_image_edit);
  const std::size_t trailing_text_begin = after_before_image_edit.find(L'B');
  const auto after_image_edit = mdlite::ApplyEditorText(
      after_before_image_snapshot, after_before_image_edit, L"a\uFFFCb",
      mdlite::EditorEditHint{mdlite::SourceSelection{trailing_text_begin,
                                                    trailing_text_begin + 1},
                            mdlite::EditorEditKind::Replace});
  Check(!before_image_edit.identity_ambiguous && !after_image_edit.identity_ambiguous &&
            after_image_edit.source == L"a![x](p.png)b",
        "separate verified edits around an image preserve its Markdown source");
  const std::wstring two_images = L"A![1](1.png)X![2](2.png)B";
  const auto two_image_snapshot = mdlite::BuildMarkdownEditorSnapshot(two_images);
  const auto first_image_deleted = mdlite::ApplyEditorText(
      two_image_snapshot, two_images, L"aX\uFFFCb");
  Check(first_image_deleted.identity_ambiguous && !first_image_deleted.changed &&
            first_image_deleted.source == two_images,
        "coalesced multi-image edits without a verified selection retain the original source");
  const std::wstring ambiguous_image_pair = L"![1](1.png)X![2](2.png)";
  const auto ambiguous_image_snapshot = mdlite::BuildMarkdownEditorSnapshot(ambiguous_image_pair);
  const std::wstring one_native_image(1, static_cast<wchar_t>(0xFFFC));
  const std::size_t image_separator = ambiguous_image_pair.find(L'X');
  const std::size_t second_image_begin = ambiguous_image_pair.find(L"![2]");
  const auto first_image_and_anchor_deleted = mdlite::ApplyEditorText(
      ambiguous_image_snapshot, ambiguous_image_pair, one_native_image,
      mdlite::EditorEditHint{mdlite::SourceSelection{0, second_image_begin},
                            mdlite::EditorEditKind::Delete});
  Check(!first_image_and_anchor_deleted.identity_ambiguous &&
            first_image_and_anchor_deleted.changed &&
            first_image_and_anchor_deleted.source == L"![2](2.png)",
        "a verified selection deleting the first image and its only anchor preserves the second image source");
  const auto anchor_and_second_image_deleted = mdlite::ApplyEditorText(
      ambiguous_image_snapshot, ambiguous_image_pair, one_native_image,
      mdlite::EditorEditHint{mdlite::SourceSelection{image_separator, ambiguous_image_pair.size()},
                            mdlite::EditorEditKind::Delete});
  Check(!anchor_and_second_image_deleted.identity_ambiguous &&
            anchor_and_second_image_deleted.changed &&
            anchor_and_second_image_deleted.source == L"![1](1.png)",
        "a verified selection deleting the second image and its only anchor preserves the first image source");
  const auto unresolved_image_identity = mdlite::ApplyEditorText(
      ambiguous_image_snapshot, ambiguous_image_pair, one_native_image);
  Check(unresolved_image_identity.identity_ambiguous && !unresolved_image_identity.changed &&
            unresolved_image_identity.source == ambiguous_image_pair,
        "image source restoration fails closed when the native edit has no identity hint");
  const std::wstring adjacent_images = L"![1](1.png)![2](2.png)";
  const auto adjacent_images_snapshot = mdlite::BuildMarkdownEditorSnapshot(adjacent_images);
  Check(adjacent_images_snapshot.collapsed.empty() && adjacent_images_snapshot.view == adjacent_images,
        "adjacent images stay as raw Markdown because identical object markers are ambiguous");
  Check(!adjacent_images_snapshot.HasCollapsedSourceRange(0, std::wstring_view(L"![1](1.png)").size()),
        "adjacent raw image ranges are excluded from native rendering");
  const auto adjacent_first_deleted = mdlite::ApplyEditorText(
      adjacent_images_snapshot, adjacent_images, L"![2](2.png)");
  Check(adjacent_first_deleted.source == L"![2](2.png)",
        "deleting the first adjacent image cannot substitute the wrong Markdown source");
  const std::wstring whitespace_images = L"![1](1.png) \t ![2](2.png)";
  const auto whitespace_images_snapshot = mdlite::BuildMarkdownEditorSnapshot(whitespace_images);
  Check(whitespace_images_snapshot.collapsed.empty() &&
            whitespace_images_snapshot.view == whitespace_images,
        "whitespace-only image groups stay raw to preserve source identity");
  const std::wstring line_separated_images = L"![1](1.png)\n\n![2](2.png)";
  const auto line_separated_snapshot = mdlite::BuildMarkdownEditorSnapshot(line_separated_images);
  Check(line_separated_snapshot.collapsed.size() == 2,
        "line-separated images retain native presentation with newline identity anchors");
  Check(line_separated_snapshot.HasCollapsedSourceRange(0, std::wstring_view(L"![1](1.png)").size()) &&
            line_separated_snapshot.HasCollapsedSourceRange(
                line_separated_images.find(L"![2]"), line_separated_images.size()),
        "only exact collapsed image ranges are eligible for native rendering");
  Check(mdlite::ParseMarkdownImages(L"```\n![not-image](code.png)\n```\n![image](real.png)").size() == 1,
        "image-only scan keeps fenced code out of derived image objects");
  const std::wstring image_then_table =
      L"![image](real.png)\n| A | B |\n| --- | --- |\n| 1 | 2 |";
  const auto mixed_snapshot = mdlite::BuildMarkdownEditorSnapshot(image_then_table);
  const auto table_edit = mdlite::InsertTableColumn(
      image_then_table, image_then_table.find(L"1"), true);
  const auto target_snapshot = mdlite::BuildMarkdownEditorSnapshot(table_edit.text);
  const auto mapped_transaction = mdlite::ApplyEditorText(
      mixed_snapshot, image_then_table, target_snapshot.view);
  Check(mapped_transaction.source == table_edit.text,
        "table transaction after a derived image maps back to the exact Markdown source");
  const auto mixed_native = mdlite::BuildNativeEditorSnapshot(image_then_table);
  auto edited_mixed_native_view = mixed_native.view;
  const std::size_t native_value_view = edited_mixed_native_view.find(L"1");
  edited_mixed_native_view.replace(native_value_view, 1, L"2");
  const auto native_transaction = mdlite::ApplyEditorText(
      mixed_native, image_then_table, edited_mixed_native_view);
  std::wstring expected_native_transaction = image_then_table;
  const std::size_t native_value_source = expected_native_transaction.find(L"1");
  expected_native_transaction.replace(native_value_source, 1, L"2");
  Check(native_transaction.source == expected_native_transaction,
        "native cell edit after a derived image preserves the exact Markdown table syntax");

  const std::wstring image_in_table_source =
      L"| Name | Preview |\n| --- | --- |\n| sample | ![pixel](pixel.png) |\n";
  const auto image_in_table = mdlite::BuildNativeEditorSnapshot(image_in_table_source);
  const std::size_t image_marker = image_in_table.view.find(static_cast<wchar_t>(0xFFFC));
  const std::size_t image_source = image_in_table_source.find(L"![pixel]");
  auto edited_image_table_view = image_in_table.view;
  edited_image_table_view.replace(edited_image_table_view.find(L"sample"), 6, L"example");
  const auto image_table_transaction = mdlite::ApplyEditorText(
      image_in_table, image_in_table_source, edited_image_table_view);
  std::wstring expected_image_table_source = image_in_table_source;
  expected_image_table_source.replace(expected_image_table_source.find(L"sample"), 6, L"example");
  Check(image_marker != std::wstring::npos,
        "a local image inside a projected cell uses one native object marker");
  Check(image_marker == std::wstring::npos ||
            image_in_table.NativeToSource(image_marker) == image_source,
        "a projected cell image marker maps back to its original Markdown start");
  Check(image_marker == std::wstring::npos ||
            image_in_table.SourceToNative(image_source) == image_marker,
        "the image Markdown start maps to its projected cell object");
  Check(image_table_transaction.source == expected_image_table_source,
        "editing adjacent projected cell text preserves table image Markdown");

  const std::wstring table_then_image_source =
      L"| Name | Preview |\n| --- | --- |\n| sample | ![inside](inside.png) |\n\n"
      L"![after](after.png) tail";
  const auto table_then_image = mdlite::BuildNativeEditorSnapshot(table_then_image_source);
  const auto table_image_marker = table_then_image.view.find(static_cast<wchar_t>(0xFFFC));
  const auto following_image_marker = table_image_marker == std::wstring::npos
      ? std::wstring::npos
      : table_then_image.view.find(static_cast<wchar_t>(0xFFFC), table_image_marker + 1);
  const auto table_image_source = table_then_image_source.find(L"![inside]");
  const auto following_image_source = table_then_image_source.find(L"![after]");
  const auto following_tail_source = table_then_image_source.find(L"tail");
  Check(table_image_marker != std::wstring::npos &&
            following_image_marker != std::wstring::npos &&
            table_then_image.NativeToSource(table_image_marker) == table_image_source &&
            table_then_image.NativeToSource(following_image_marker) == following_image_source &&
            table_then_image.SourceToNative(table_image_source) == table_image_marker &&
            table_then_image.SourceToNative(following_image_source) == following_image_marker &&
            table_then_image.NativeToSource(
                table_then_image.SourceToNative(following_tail_source)) == following_tail_source,
        "an image consumed inside a projected table leaves following image and text mappings aligned");

  const std::wstring native_table_source =
      L"before\r\n| A | B |\r\n| --- | --- |\r\n| alpha\\|beta | `x|y` |\r\nmiddle\n"
      L"| left | right |\n| --- | --- |\n| a || c |\n| x | y |\nafter";
  const auto native_table = mdlite::BuildNativeEditorSnapshot(native_table_source);
  const std::wstring expected_table_view =
      L"before\r A \t B \r alpha\\|beta \t `x|y` \rmiddle\r left \t right \t"
      L"\r a \t\t c \r x \t y \t\rafter";
  Check(native_table.tables.size() == 2 && native_table.view == expected_table_view,
        "native GFM projection keeps adjacent text and emits visible cells with tab and CR separators");
  Check(native_table.tables[0].cells.size() == 4 && native_table.tables[1].cells.size() == 7 &&
            native_table.tables[1].gaps.size() > native_table.tables[0].gaps.size(),
        "native table mappings cover multiple tables, escaped/code-span pipes, empty and ragged cells");
  const std::size_t escaped_source = native_table_source.find(L"alpha\\|beta");
  const std::size_t escaped_view = native_table.view.find(L"alpha\\|beta");
  const std::size_t middle_source = native_table_source.find(L"middle");
  const std::size_t middle_view = native_table.view.find(L"middle");
  Check(native_table.SourceToView(escaped_source + 3) == escaped_view + 3 &&
            native_table.ViewToSource(escaped_view + 3) == escaped_source + 3 &&
            native_table.SourceToNative(middle_source) == middle_view &&
            native_table.NativeToSource(middle_view) == middle_source,
        "table cell and adjacent-text source/view/native coordinates round-trip across mixed line endings");
  auto edited_table_view = native_table.view;
  edited_table_view.replace(escaped_view, std::wstring_view(L"alpha\\|beta").size(), L"updated");
  const auto edited_table = mdlite::ApplyEditorText(
      native_table, native_table_source, edited_table_view);
  std::wstring expected_table_source = native_table_source;
  expected_table_source.replace(escaped_source, std::wstring_view(L"alpha\\|beta").size(), L"updated");
  Check(edited_table.changed && edited_table.source == expected_table_source &&
            edited_table.begin == escaped_source &&
            edited_table.old_end == escaped_source + std::wstring_view(L"alpha\\|beta").size(),
        "editing one projected cell changes only its source text and preserves Markdown syntax and CRLFs");
  const std::wstring final_cell_source =
      L"|H|B|\r\n|---|---|\r\n|a|b|\r\ntail";
  const auto final_cell_snapshot = mdlite::BuildNativeEditorSnapshot(final_cell_source);
  auto final_cell_view = final_cell_snapshot.view;
  const std::size_t final_cell_view_begin = final_cell_view.find(L"b");
  final_cell_view.replace(final_cell_view_begin, 1, L"z");
  const auto final_cell_edit = mdlite::ApplyEditorText(
      final_cell_snapshot, final_cell_source, final_cell_view);
  std::wstring expected_final_cell_source = final_cell_source;
  expected_final_cell_source.replace(final_cell_source.find(L"b"), 1, L"z");
  Check(final_cell_edit.source == expected_final_cell_source,
        "editing the last unpadded cell preserves its trailing pipe and following CRLF");

  auto native_table_coordinates = native_table;
  mdlite::NativeTableCoordinates first_table_coordinates;
  first_table_coordinates.begin = native_table.tables[0].view_begin;
  std::size_t native_cell_position = first_table_coordinates.begin + 2;
  for (const auto& cell : native_table.tables[0].cells) {
    const auto width = cell.source_end - cell.source_begin;
    first_table_coordinates.cells.push_back({native_cell_position, native_cell_position + width});
    native_cell_position += width + 2;
  }
  first_table_coordinates.end = native_cell_position + 3;
  const auto second_table_view_cell = native_table.tables[1].cells.front().view_begin;
  const auto expected_second_table_native =
      second_table_view_cell + first_table_coordinates.end - native_table.tables[0].view_end;
  Check(mdlite::ReplaceNativeTableCoordinates(native_table_coordinates, 0,
                                               first_table_coordinates) &&
            native_table_coordinates.SourceToNative(escaped_source) ==
                first_table_coordinates.cells[2].begin +
                    (escaped_source - native_table.tables[0].cells[2].source_begin) &&
            native_table_coordinates.NativeToSource(
                first_table_coordinates.cells[2].begin +
                (escaped_source - native_table.tables[0].cells[2].source_begin) + 3) ==
                escaped_source + 3 &&
            native_table_coordinates.SourceToNative(
                native_table.tables[1].cells.front().source_begin) == expected_second_table_native,
        "replacing TOM table/cell coordinates rebuilds native mappings for following tables");
}

void TestEditorTransactionBoundaries() {
  const std::wstring initial = L"ab";
  const auto first_snapshot = mdlite::BuildEditorSnapshot(initial);
  const auto first = mdlite::ApplyEditorText(first_snapshot, initial, L"aXb");
  Check(first.changed && first.source == L"aXb" && first.begin == 1 && first.old_end == 1 &&
            first.new_end == 2,
        "first ASCII character edit is one source transaction with exact insertion bounds");

  const auto second_snapshot = mdlite::BuildEditorSnapshot(first.source);
  const auto second = mdlite::ApplyEditorText(second_snapshot, first.source, L"aXYb");
  Check(second.changed && second.source == L"aXYb" && second.begin == 2 && second.old_end == 2 &&
            second.new_end == 3,
        "second ASCII character edit starts a separate transaction after the first commit");

  const auto replacement_snapshot = mdlite::BuildEditorSnapshot(second.source);
  const auto replacement = mdlite::ApplyEditorText(
      replacement_snapshot, second.source, L"aPASTEb");
  Check(replacement.changed && replacement.source == L"aPASTEb" && replacement.begin == 1 &&
            replacement.old_end == 3 && replacement.new_end == 6,
        "replacement or paste-like input remains one exact source transaction");

  const std::wstring unicode_source = L"😀\r\nA\nB";
  const auto unicode_snapshot = mdlite::BuildEditorSnapshot(unicode_source);
  const auto unicode_edit = mdlite::ApplyEditorText(
      unicode_snapshot, unicode_source, L"😀\r\nAX\r\nB");
  Check(unicode_edit.changed && unicode_edit.source == L"😀\r\nAX\nB" &&
            unicode_edit.begin == 5 && unicode_edit.old_end == 5 && unicode_edit.new_end == 6,
        "a UTF-16 edit beside a surrogate and mixed LF/CRLF preserves source line endings");

  const auto undo = mdlite::ApplyEditorText(
      mdlite::BuildEditorSnapshot(unicode_edit.source), unicode_edit.source, unicode_snapshot.view);
  Check(undo.changed && undo.source == unicode_source && undo.begin == 5 && undo.old_end == 6 &&
            undo.new_end == 5,
        "undo-like reverse transaction restores the exact original surrogate and line endings");
  const auto redo = mdlite::ApplyEditorText(
      mdlite::BuildEditorSnapshot(undo.source), undo.source, L"😀\r\nAX\r\nB");
  Check(redo.changed && redo.source == unicode_edit.source && redo.begin == 5 && redo.old_end == 5 &&
            redo.new_end == 6,
        "redo-like forward transaction restores the exact edited source");

  const std::wstring literal_table =
      L"|  A  | B\\| raw | `C|D` |\r\n| :--- | ---: | :---: |\r\n| left  |  middle  | right |";
  const auto table_edit = mdlite::InsertTableColumn(
      literal_table, literal_table.find(L"middle"), true);
  const auto table_before = mdlite::BuildEditorSnapshot(literal_table);
  const auto table_after = mdlite::BuildEditorSnapshot(table_edit.text);
  const auto table_transaction = mdlite::ApplyEditorText(
      table_before, literal_table, table_after.view);
  Check(table_edit.changed && table_transaction.changed &&
            table_transaction.source == table_edit.text &&
            table_transaction.source.find(L"B\\| raw") != std::wstring::npos &&
            table_transaction.source.find(L"`C|D`") != std::wstring::npos,
        "table edit transaction preserves escaped and code-span pipe literals exactly");
  const auto table_undo = mdlite::ApplyEditorText(
      mdlite::BuildEditorSnapshot(table_transaction.source), table_transaction.source,
      table_before.view);
  Check(table_undo.changed && table_undo.source == literal_table,
        "table undo-like transaction restores literal source without presentation rewriting");
  const auto table_redo = mdlite::ApplyEditorText(
      mdlite::BuildEditorSnapshot(table_undo.source), table_undo.source, table_after.view);
  Check(table_redo.changed && table_redo.source == table_edit.text,
        "table redo-like transaction restores the exact helper result");

  const auto no_op = mdlite::ApplyEditorText(
      mdlite::BuildEditorSnapshot(table_redo.source), table_redo.source, table_after.view);
  Check(!no_op.changed && no_op.source == table_redo.source,
        "reapplying the committed presentation is a source-preserving no-op");
}

void TestWorkspaceState(const std::filesystem::path& root) {
  const auto workspace = root / L"workspace";
  std::filesystem::create_directories(workspace);
  mdlite::WorkspaceStore store(workspace);
  std::wstring error;
  Check(store.Initialize(error), "workspace metadata initializes");
  Check(std::filesystem::exists(workspace / L".mdlite/.gitignore"), "workspace gitignore is created");
  Check(std::filesystem::exists(workspace / L".mdlite/templates/memo.md"), "built-in template is created");
  const auto document = workspace / L"note.md";
  WriteBytes(document, {'o', 'l', 'd'});
  std::filesystem::create_directories(workspace / L"nested");
  WriteBytes(workspace / L"Alpha.md", {'a'});
  WriteBytes(workspace / L"nested/project-note.txt", {'b'});
  const auto recent_candidates = mdlite::FindQuickOpenCandidates(
      workspace, L"", {workspace / L"nested/project-note.txt"}, 3);
  Check(!recent_candidates.empty() && recent_candidates.front().filename() == L"project-note.txt",
        "Quick Open ranks recent documents before the remaining Workspace files");
  const auto filtered_candidates = mdlite::FindQuickOpenCandidates(workspace, L"alpha", {}, 10);
  Check(filtered_candidates.size() == 1 && filtered_candidates.front().filename() == L"Alpha.md",
        "Quick Open filters by case-insensitive file name and relative path");
  Check(store.WriteRecovery(document, L"編集中", error), "recovery content writes inside workspace state");
  Check(store.RecoveryFiles().size() == 1, "recovery file is discoverable");
  mdlite::RecoverySnapshot recovery;
  Check(store.ReadRecoverySnapshot(store.RecoveryFiles().front(), recovery, error) &&
            recovery.source_path == document && recovery.text == L"編集中",
        "recovery header is validated and separated from the editable body");
  Check(store.WriteSession({document, L"C:\\outside.md"}, error), "session file writes");
  std::vector<std::filesystem::path> restored;
  Check(store.ReadSession(restored, error), "session file reads");
  Check(restored.size() == 1 && restored.front() == document,
        "session restore includes only existing documents inside the workspace");
  mdlite::SessionState session;
  session.active_index = 0;
  session.main_x = 40;
  session.main_y = 50;
  session.main_width = 1100;
  session.main_height = 700;
  session.recent_documents = {workspace / L"nested/project-note.txt", workspace / L"Alpha.md"};
  session.documents.push_back({document, 1, 2, 3, true, 100, 120, 640, 480});
  session.documents.back().first_visible_source_offset = 2;
  session.documents.back().horizontal_left_edge_source_offset = 3;
  Check(store.WriteSessionState(session, error), "detailed session state writes");
  mdlite::SessionState detailed;
  Check(store.ReadSessionState(detailed, error), "detailed session state reads");
  Check(detailed.documents.size() == 1 && detailed.documents[0].selection_begin == 1 &&
            detailed.documents[0].selection_end == 2 && detailed.documents[0].first_visible_line == 3 &&
            detailed.documents[0].first_visible_source_offset == 2 &&
            detailed.documents[0].horizontal_left_edge_source_offset == 3 &&
            detailed.documents[0].compact && detailed.documents[0].width == 640 &&
            detailed.main_width == 1100 && detailed.recent_documents.size() == 2 &&
            detailed.recent_documents[0].filename() == L"project-note.txt",
        "session preserves selection, source scroll anchor, legacy scroll, compact placement, and main placement");
  const auto dot_draft = workspace / L"..draft.md";
  WriteBytes(dot_draft, {'d'});
  mdlite::SessionState dot_draft_session;
  dot_draft_session.documents.push_back({dot_draft});
  dot_draft_session.recent_documents.push_back(dot_draft);
  Check(store.WriteSessionState(dot_draft_session, error),
        "session writes a root document whose name begins with two dots");
  mdlite::SessionState restored_dot_draft;
  Check(store.ReadSessionState(restored_dot_draft, error) &&
            restored_dot_draft.documents.size() == 1 &&
            restored_dot_draft.documents[0].path == dot_draft &&
            restored_dot_draft.recent_documents.size() == 1 &&
            restored_dot_draft.recent_documents[0] == dot_draft,
        "session restores ..draft.md as both an open and recent document");

  const auto outside = root / L"outside.md";
  WriteBytes(outside, {'o'});
  mdlite::SessionState outside_session;
  outside_session.documents.push_back({outside});
  outside_session.recent_documents.push_back(outside);
  Check(store.WriteSessionState(outside_session, error),
        "session write skips documents outside the workspace");
  mdlite::SessionState written_outside;
  Check(store.ReadSessionState(written_outside, error) && written_outside.documents.empty() &&
            written_outside.recent_documents.empty(),
        "session serialization omits a real ../outside.md path");
  const std::string outside_session_text =
      "recent = \"../outside.md\"\n\n[[document]]\npath = \"../outside.md\"\n";
  WriteBytes(store.state_root() / L"session.toml",
             std::vector<unsigned char>(outside_session_text.begin(), outside_session_text.end()));
  mdlite::SessionState restored_outside;
  Check(store.ReadSessionState(restored_outside, error) && restored_outside.documents.empty() &&
            restored_outside.recent_documents.empty(),
        "session rejects a real ../outside.md path for open and recent documents");
  const std::string legacy_session =
      "schema_version = 2\nactive_index = 0\n\n[[document]]\npath = \"note.md\"\n"
      "selection_begin = 1\nselection_end = 2\nfirst_visible_line = 7\ncompact = false\n";
  WriteBytes(store.state_root() / L"session.toml",
             std::vector<unsigned char>(legacy_session.begin(), legacy_session.end()));
  mdlite::SessionState legacy;
  Check(store.ReadSessionState(legacy, error) && legacy.documents.size() == 1 &&
            legacy.documents[0].first_visible_line == 7 &&
            !legacy.documents[0].first_visible_source_offset.has_value() &&
            !legacy.documents[0].horizontal_left_edge_source_offset.has_value(),
        "legacy session without source scroll anchor keeps its line and leaves the optional anchor unset");
  Check(store.DiscardRecoverySnapshot(recovery.recovery_path, error),
        "recovery can be explicitly discarded by its validated state path");
  Check(store.RecoveryFiles().empty(), "recovery removal is visible");
}

void TestTrustAndProcess(const std::filesystem::path& root) {
  const auto workspace = root / L"trust";
  const auto trust_store = root / L"user-trust-store";
  mdlite::SetTrustStoreRootForTesting(trust_store);
  std::filesystem::create_directories(workspace / L".mdlite/.state");
  std::wstring error;
  Check(!mdlite::IsWorkspaceTrusted(workspace), "workspace starts untrusted");
  Check(mdlite::SetWorkspaceTrusted(workspace, true, error) && mdlite::IsWorkspaceTrusted(workspace),
        "workspace trust is local and explicit");
  const auto copied = root / L"trust-copy";
  std::filesystem::copy(workspace, copied, std::filesystem::copy_options::recursive);
  Check(!mdlite::IsWorkspaceTrusted(copied),
        "copying a workspace cannot copy the user's path and directory identity grant");
  WriteBytes(copied / L".mdlite/.state/trust.local", {'f','o','r','g','e','d'});
  Check(!mdlite::IsWorkspaceTrusted(copied),
        "a workspace-owned legacy trust token cannot self-declare trust");
  Check(!std::filesystem::is_regular_file(workspace / L".mdlite/.state/trust.local"),
        "trust records are stored outside the workspace tree");
  Check(mdlite::SetWorkspaceTrusted(workspace, false, error) && !mdlite::IsWorkspaceTrusted(workspace),
        "workspace trust can be revoked");
  mdlite::ProcessResult process;
  Check(mdlite::RunProcess(L"C:\\Windows\\System32\\cmd.exe", {L"/d", L"/c", L"echo", L"MDLite process"},
                           workspace, 4096, 5000, process, error),
        "process runner starts an argument-array command");
  Check(process.exit_code == 0 && process.output.find(L"MDLite process") != std::wstring::npos,
        "process runner captures bounded output");
  HANDLE cancellation = CreateEventW(nullptr, TRUE, FALSE, nullptr);
  std::thread cancel_thread([&] {
    Sleep(100);
    SetEvent(cancellation);
  });
  error.clear();
  Check(mdlite::RunProcess(L"C:\\Windows\\System32\\ping.exe", {L"-n", L"10", L"127.0.0.1"},
                           workspace, 4096, 10000, process, error, cancellation),
        "process runner accepts an explicit cancellation handle");
  cancel_thread.join();
  CloseHandle(cancellation);
  Check(process.cancelled, "process cancellation terminates its job and reports cancellation");
}

void TestProfiles(const std::filesystem::path& root) {
  const auto workspace = root / L"profiles";
  std::filesystem::create_directories(workspace);
  mdlite::WorkspaceStore store(workspace);
  std::wstring error;
  Check(store.Initialize(error), "profile workspace initializes");
  SYSTEMTIME date{};
  date.wYear = 2026;
  date.wMonth = 9;
  date.wDay = 19;
  const auto built_in_profiles = mdlite::DefaultProfiles();
  mdlite::NoteCreationResult first{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[0], date, first, error),
        "daily note creates");
  Check(first.path == workspace / L"Dairy/2026/202609/20260919.md", "Dairy spelling and path are exact");
  mdlite::NoteCreationResult reopen{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[0], date, reopen, error),
        "existing daily note opens without overwrite");
  Check(!reopen.created && reopen.path == first.path, "daily collision opens existing note");
  mdlite::NoteCreationResult memo1{};
  mdlite::NoteCreationResult memo2{};
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[2], date, memo1, error),
        "first memo creates");
  Check(mdlite::CreateProfileNote(workspace, built_in_profiles[2], date, memo2, error),
        "second memo creates");
  Check(memo2.path.filename() == L"20260919_01.md", "memo collision uses suffix before extension");

  auto overrides = mdlite::DefaultProfiles();
  overrides[0].directory = L"Journal/{{date:yyyy}}";
  const auto profile_file = workspace / L"profile-overrides.toml";
  Check(mdlite::SaveProfileFile(profile_file, {overrides[0]}, error),
        "profile definitions save atomically");
  std::vector<mdlite::ProfileDefinition> loaded;
  Check(mdlite::LoadProfileFile(profile_file, loaded, error) && loaded.size() == 1,
        "profile definitions round trip through TOML");
  std::vector<mdlite::ProfileDefinition> effective;
  Check(mdlite::ResolveProfiles({}, profile_file, effective, error),
        "profile layer resolves over built-in definitions");
  const auto daily = std::ranges::find_if(effective, [](const auto& item) { return item.id == L"daily"; });
  const auto preview = daily == effective.end() ? std::optional<std::filesystem::path>{} :
      mdlite::PreviewProfilePath(workspace, *daily, date, error);
  Check(preview && *preview == workspace / L"Journal/2026/20260919.md",
        "profile path preview uses the effective editable definition");
  auto unsafe = overrides[0];
  unsafe.directory = L"../outside";
  Check(!mdlite::ValidateProfile(unsafe, error), "profile validation rejects Workspace path escape");
  unsafe = overrides[0];
  unsafe.filename = L"{{unknown}}.md";
  Check(!mdlite::ValidateProfile(unsafe, error), "profile validation rejects undefined variables");
  const auto unsupported_sequence = workspace / L"unsupported-sequence.toml";
  WriteBytes(unsupported_sequence,
             {'s','c','h','e','m','a','_','v','e','r','s','i','o','n',' ','=',' ','1','\n',
              '[','[','p','r','o','f','i','l','e','s',']',']','\n',
              'i','d',' ','=',' ','"','x','"','\n',
              'n','a','m','e',' ','=',' ','"','x','"','\n',
              'd','i','r','e','c','t','o','r','y',' ','=',' ','"','x','"','\n',
              'f','i','l','e','n','a','m','e',' ','=',' ','"','x','.','m','d','"','\n',
              't','e','m','p','l','a','t','e',' ','=',' ','"','t','e','m','p','l','a','t','e','s','/','m','e','m','o','.','m','d','"','\n',
              'c','o','l','l','i','s','i','o','n',' ','=',' ','"','s','e','q','u','e','n','c','e','"','\n',
              's','e','q','u','e','n','c','e','_','f','o','r','m','a','t',' ','=',' ','"','_','%','0','3','d','"','\n'});
  Check(!mdlite::LoadProfileFile(unsupported_sequence, loaded, error),
        "profile parser rejects unsupported sequence format instead of silently changing semantics");

  const std::string input_template = "# {{title}}\n{{input:owner}}\n{{cursor}}end";
  WriteBytes(workspace / L".mdlite/templates/input.md",
             std::vector<unsigned char>(input_template.begin(), input_template.end()));
  mdlite::ProfileDefinition input_profile{
      L"input", L"Input", L"Projects/{{input:owner}}", L"{{title}}.md",
      L"templates/input.md", mdlite::ProfileCollision::OpenExisting,
      {{L"title", L"Title", L"", true}, {L"owner", L"Owner", L"team", false}}};
  const auto input_file = workspace / L"input-profile.toml";
  Check(mdlite::SaveProfileFile(input_file, {input_profile}, error),
        "profile input definitions save");
  Check(mdlite::LoadProfileFile(input_file, loaded, error) && loaded.size() == 1 &&
            loaded.front().inputs.size() == 2 && loaded.front().inputs.front().required,
        "profile input definitions round trip through the shared TOML model");
  mdlite::ProfileValues values{{L"title", L"Roadmap"}, {L"owner", L"alice"}};
  const auto input_preview = mdlite::PreviewProfilePath(workspace, input_profile, date, values, error);
  Check(input_preview && *input_preview == workspace / L"Projects/alice/Roadmap.md",
        "profile path preview expands finite title and input variables");
  mdlite::NoteCreationResult input_note{};
  Check(mdlite::CreateProfileNote(workspace, input_profile, date, values, input_note, error),
        "profile note creates with prompted values");
  const std::vector<unsigned char> expected_input_body{'#',' ','R','o','a','d','m','a','p','\n',
                                                        'a','l','i','c','e','\n','e','n','d'};
  Check(ReadBytes(input_note.path) == expected_input_body && input_note.cursor == 16,
        "profile values expand once and cursor is removed at the expanded UTF-16 position");
  values[L"title"] = L"";
  Check(!mdlite::PreviewProfilePath(workspace, input_profile, date, values, error),
        "profile preview rejects a missing required input");
  values[L"title"] = L"../escape";
  Check(!mdlite::PreviewProfilePath(workspace, input_profile, date, values, error),
        "profile preview rejects path separators introduced by input");
}

void TestSearch(const std::filesystem::path& root) {
  const auto workspace = root / L"search";
  std::filesystem::create_directories(workspace / L"nested");
  WriteBytes(workspace / L"one.md", {'a', 'b', 'c', '\n'});
  WriteBytes(workspace / L"nested/two.md", {'x', 'y', 'z', '\n'});
  std::vector<mdlite::SearchMatch> matches;
  std::wstring error;
  std::map<std::filesystem::path, std::wstring> unsaved{{
      std::filesystem::absolute(workspace / L"one.md").lexically_normal(), L"未保存 abc"}};
  Check(mdlite::SearchWorkspace(workspace, {L"未保存", false, false}, unsaved, matches, error),
        "workspace search includes unsaved text");
  Check(matches.size() == 1 && matches.front().line == 1, "unsaved search result is located");
  Check(mdlite::SearchWorkspace(workspace, {L"x.z", true, true}, {}, matches, error),
        "workspace regex search runs");
  Check(matches.size() == 1, "regex matches disk text");
  const auto collection = workspace / L"collection";
  std::filesystem::create_directories(collection);
  const std::string filler(10 * 1024, 'x');
  for (int index = 0; index < 200; ++index) {
    std::ofstream output(collection / (L"note-" + std::to_wstring(index) + L".md"),
                         std::ios::binary | std::ios::trunc);
    output << "performance token\n" << filler;
  }
  const auto collection_start = GetTickCount64();
  Check(mdlite::SearchWorkspace(collection, {L"performance token", true, false}, {},
                                matches, error),
        "200-file workspace search completes");
  Check(matches.size() == 200, "200-file workspace search publishes every result");
  Check(GetTickCount64() - collection_start < 10000,
        "200-file workspace search avoids a progress-dialog-scale stall");
  const std::wstring source = L"Alpha alpha\nline two 😀\nline three";
  mdlite::SearchQuery document_query{L"alpha", false, false};
  Check(mdlite::SearchDocumentText(workspace / L"open.md", source, document_query, matches, error) &&
            matches.size() == 2,
        "current-document source search shares case-insensitive workspace semantics");
  document_query.match_case = true;
  Check(mdlite::SearchDocumentText(workspace / L"open.md", source, document_query, matches, error) &&
            matches.size() == 1,
        "current-document source search honors match-case");
  mdlite::SearchQuery multiline{L"two 😀\\nline (three)", true, true};
  Check(mdlite::SearchDocumentText(workspace / L"open.md", source, multiline, matches, error) &&
            matches.size() == 1,
        "source regex search supports multiline Unicode matches");
  std::wstring replaced_text;
  std::size_t replaced_count{};
  Check(mdlite::ReplaceDocumentText(source, multiline, L"$1 / $&", replaced_text,
                                    replaced_count, error) && replaced_count == 1 &&
            replaced_text.find(L"three / two 😀\nline three") != std::wstring::npos,
        "source replacement supports ECMAScript capture and whole-match references");
  mdlite::SearchQuery invalid{L"(", true, true};
  error.clear();
  Check(!mdlite::SearchDocumentText(workspace / L"open.md", source, invalid, matches, error),
        "invalid current-document regex fails without changing text");

  WriteBytes(workspace / L"words.md", {'c','a','t',' ','s','c','a','t','t','e','r',' ','c','a','t'});
  mdlite::SearchQuery whole{L"cat", true, false};
  whole.whole_word = true;
  whole.include_globs = {L"*.md"};
  Check(mdlite::SearchWorkspace(workspace, whole, {}, matches, error), "whole-word glob search runs");
  Check(matches.size() == 2, "whole-word search excludes embedded words");

  mdlite::ReplacePlan plan;
  Check(mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, plan, error),
        "replace preview succeeds");
  Check(plan.files.size() == 1 && plan.files.front().replacement_count == 2,
        "replace preview records exact changes");
  mdlite::ReplaceApplyResult applied;
  Check(mdlite::ApplyWorkspaceReplace(workspace, plan, applied, error), "replace apply succeeds");
  mdlite::Document replaced;
  Check(replaced.Load(workspace / L"words.md", error) && replaced.text() == L"dog scatter dog",
        "replace applies only previewed whole words");
  mdlite::ReplaceApplyResult rolled_back;
  Check(mdlite::RollbackWorkspaceReplace(applied.journal, rolled_back, error),
        "replace journal rollback succeeds");
  Check(replaced.Load(workspace / L"words.md", error) && replaced.text() == L"cat scatter cat",
        "rollback restores the preview baseline");

  error.clear();
  mdlite::ReplacePlan cancelled_preview;
  Check(!mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, cancelled_preview, error,
                                         [] { return true; }),
        "replace preview can be cancelled");
  Check(error.find(L"中止") != std::wstring::npos, "cancelled preview reports cancellation");

  WriteBytes(workspace / L"cancel-a.md", {'c','a','t'});
  WriteBytes(workspace / L"cancel-b.md", {'c','a','t'});
  mdlite::ReplacePlan cancellable_plan;
  error.clear();
  Check(mdlite::PreviewWorkspaceReplace(workspace, whole, L"dog", {}, cancellable_plan, error),
        "cancellable replace preview succeeds");
  std::size_t cancellation_checks{};
  mdlite::ReplaceApplyResult cancelled_apply;
  Check(!mdlite::ApplyWorkspaceReplace(workspace, cancellable_plan, cancelled_apply, error,
                                       [&] { return ++cancellation_checks > 1; }),
        "replace apply can be cancelled between files");
  Check(cancelled_apply.applied_files == 1 && std::filesystem::exists(cancelled_apply.journal),
        "cancelled replace records the partial apply in a journal");
  mdlite::Document cancel_a;
  mdlite::Document cancel_b;
  Check(cancel_a.Load(workspace / L"cancel-a.md", error) && cancel_a.text() == L"dog" &&
            cancel_b.Load(workspace / L"cancel-b.md", error) && cancel_b.text() == L"cat",
        "cancelled replace changes only the completed file");
  mdlite::ReplaceApplyResult cancelled_rollback;
  Check(mdlite::RollbackWorkspaceReplace(cancelled_apply.journal, cancelled_rollback, error),
        "cancelled replace journal can roll back the partial apply");

  const auto globs = workspace / L"globs";
  std::filesystem::create_directories(globs / L"nested");
  std::filesystem::create_directories(globs / L".hidden");
  std::filesystem::create_directories(globs / L".mdlite/.state");
  WriteBytes(globs / L"top.md", {'g','l','o','b','-','t','o','k','e','n'});
  WriteBytes(globs / L"nested/deep.md", {'g','l','o','b','-','t','o','k','e','n'});
  WriteBytes(globs / L".hidden/note.md", {'g','l','o','b','-','t','o','k','e','n'});
  WriteBytes(globs / L".mdlite/.state/internal.md", {'g','l','o','b','-','t','o','k','e','n'});
  mdlite::SearchQuery shallow{L"glob-token", true, false};
  shallow.include_globs = {L"*.md"};
  Check(mdlite::SearchWorkspace(globs, shallow, {}, matches, error) && matches.size() == 1 &&
            matches.front().path.filename() == L"top.md",
        "single-star glob does not cross a path separator and hidden/internal files stay excluded");
  shallow.include_globs = {L"**/*.md"};
  Check(mdlite::SearchWorkspace(globs, shallow, {}, matches, error) && matches.size() == 2,
        "double-star glob includes root and nested path components");
  std::size_t progressive_matches{};
  Check(mdlite::SearchWorkspace(globs, shallow, {}, matches, error, {},
                                [&](std::vector<mdlite::SearchMatch> batch) {
                                  progressive_matches += batch.size();
                                }) && progressive_matches == matches.size() &&
            progressive_matches == 2,
        "workspace search publishes progressive batches without losing final results");
  shallow.exclude_globs = {L"nested/**"};
  Check(mdlite::SearchWorkspace(globs, shallow, {}, matches, error) && matches.size() == 1 &&
            matches.front().path.filename() == L"top.md",
        "exclude glob takes precedence over include glob");
  shallow.include_hidden = true;
  shallow.exclude_globs.clear();
  Check(mdlite::SearchWorkspace(globs, shallow, {}, matches, error) && matches.size() == 3,
        "hidden search can be explicitly enabled while internal metadata remains excluded");

  const auto ignored = workspace / L"gitignore-search";
  std::filesystem::create_directories(ignored / L"ignored-dir");
  std::filesystem::create_directories(ignored / L"nested/child");
  std::filesystem::create_directories(ignored / L"generated/a");
  std::filesystem::create_directories(ignored / L"other");
  std::filesystem::create_directories(ignored / L"parent-blocked");
  std::filesystem::create_directories(ignored / L"reopened");
  WriteAscii(ignored / L".gitignore",
             "ignored-dir/\nignored-*.md\n!ignored-keep.md\n*.log\n/root-only.md\n"
             "temp?.md\ngenerated/**/draft*.md\nparent-blocked/\n"
             "!parent-blocked/keep.md\nreopened/\n!reopened/\nreopened/*.md\n"
             "!reopened/keep.md\n");
  WriteAscii(ignored / L"nested/.gitignore", "*.md\n!important.md\n/deep-only.txt\n");
  for (const auto& relative : {L"visible.md", L"ignored-dir/hidden.md", L"ignored-1.md",
                               L"ignored-keep.md", L"trace.log", L"root-only.md",
                               L"temp1.md", L"temp12.md", L"generated/a/draft1.md",
                               L"generated/a/final.md", L"nested/blocked.md",
                               L"nested/important.md", L"nested/deep-only.txt",
                               L"nested/child/important.md", L"nested/child/deep-only.txt",
                               L"other/root-only.md", L"parent-blocked/drop.md",
                               L"parent-blocked/keep.md", L"reopened/drop.md",
                               L"reopened/keep.md"})
    WriteAscii(ignored / relative, "ignore-token");
  WriteBytes(ignored / L"ignored-dir/.gitignore", {0xEF, 0xBB, 0xBF, 0xFF});
  mdlite::SearchQuery ignored_query{L"ignore-token", true, false};
  Check(mdlite::SearchWorkspace(ignored, ignored_query, {}, matches, error) && matches.size() == 9,
        ".gitignore rules honor negation, directory, anchored, star, question, and double-star patterns");
  Check(std::ranges::any_of(matches, [](const auto& match) {
          return match.path.filename() == L"ignored-keep.md";
        }) && std::ranges::any_of(matches, [](const auto& match) {
          return match.path.generic_wstring().ends_with(L"nested/child/deep-only.txt");
        }) && std::ranges::none_of(matches, [](const auto& match) {
          return match.path.generic_wstring().ends_with(L"nested/deep-only.txt");
        }) && std::ranges::none_of(matches, [](const auto& match) {
          return match.path.generic_wstring().ends_with(L"parent-blocked/keep.md");
        }) && std::ranges::any_of(matches, [](const auto& match) {
          return match.path.generic_wstring().ends_with(L"reopened/keep.md");
        }),
        ".gitignore negation respects declaring scope and requires excluded parents to be reopened");
  ignored_query.respect_gitignore = false;
  Check(mdlite::SearchWorkspace(ignored, ignored_query, {}, matches, error) && matches.size() == 20,
        ".gitignore filtering can be explicitly disabled");

  const auto issue_workspace = workspace / L"search-issues";
  std::filesystem::create_directories(issue_workspace);
  WriteAscii(issue_workspace / L"good.md", "issue-token");
  WriteBytes(issue_workspace / L"bad.md", {0xEF, 0xBB, 0xBF, 0xFF});
  mdlite::SearchQuery issue_query{L"issue-token", true, false};
  std::vector<mdlite::SearchIssue> issues;
  std::size_t progressive_issues{};
  error.clear();
  Check(!mdlite::SearchWorkspace(issue_workspace, issue_query, {}, matches, error) &&
            error.find(L"未処理") != std::wstring::npos,
        "workspace search without an issue sink fails closed on an unreadable file");
  error.clear();
  Check(mdlite::SearchWorkspace(issue_workspace, issue_query, {}, matches, error, {}, {}, &issues,
                                [&](std::vector<mdlite::SearchIssue> batch) {
                                  progressive_issues += batch.size();
                                }) && matches.size() == 1 && issues.size() == 1 &&
            progressive_issues == 1 && issues.front().path.filename() == L"bad.md",
        "workspace search reports unreadable files individually and continues with readable files");

  const auto guarded_workspace = workspace / L"search-guarded";
  std::filesystem::create_directories(guarded_workspace / L"uncertain");
  std::filesystem::create_directories(guarded_workspace / L"readable");
  WriteBytes(guarded_workspace / L"uncertain/.gitignore", {0xEF, 0xBB, 0xBF, 0xFF});
  WriteAscii(guarded_workspace / L"uncertain/private.md", "guard-token");
  WriteAscii(guarded_workspace / L"readable/public.md", "guard-token");
  mdlite::SearchQuery guarded_query{L"guard-token", true, false};
  issues.clear();
  error.clear();
  Check(mdlite::SearchWorkspace(guarded_workspace, guarded_query, {}, matches, error, {}, {},
                                &issues) &&
            matches.size() == 1 && matches.front().path.filename() == L"public.md" &&
            issues.size() == 1 && issues.front().path.filename() == L".gitignore",
        "an unreadable .gitignore fails closed for its subtree while readable siblings continue");

  const auto locked_path = issue_workspace / L"locked.md";
  WriteAscii(locked_path, "issue-token");
  HANDLE locked = CreateFileW(locked_path.c_str(), GENERIC_READ | GENERIC_WRITE, 0, nullptr,
                              OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  Check(locked != INVALID_HANDLE_VALUE, "search issue fixture can hold an exclusive file lock");
  if (locked != INVALID_HANDLE_VALUE) {
    issues.clear();
    progressive_issues = 0;
    error.clear();
    Check(mdlite::SearchWorkspace(issue_workspace, issue_query, {}, matches, error, {}, {}, &issues,
                                  [&](std::vector<mdlite::SearchIssue> batch) {
                                    progressive_issues += batch.size();
                                  }) && matches.size() == 1 && issues.size() == 2 &&
              progressive_issues == 2 &&
              std::ranges::any_of(issues, [&](const auto& issue) {
                return issue.path == locked_path;
              }),
          "workspace search reports a locked file and continues without silently omitting it");
    CloseHandle(locked);
  }

  const std::wstring anchored = L"Alpha one\nAlpha two";
  mdlite::SearchQuery anchored_query{L"^Alpha (one)", true, true};
  std::size_t replace_begin{};
  std::size_t replace_end{};
  bool did_replace{};
  Check(mdlite::ReplaceDocumentMatch(anchored, anchored_query, L"$1/$&", 0,
                                     replaced_text, replace_begin, replace_end,
                                     did_replace, error) && did_replace && replace_begin == 0 &&
            replaced_text == L"one/Alpha one\nAlpha two",
        "single document replacement expands captures in full source context");
  anchored_query.text = L"^Alpha two";
  Check(mdlite::ReplaceDocumentMatch(anchored, anchored_query, L"changed",
                                     anchored.find(L"Alpha two"), replaced_text,
                                     replace_begin, replace_end, did_replace, error) &&
            !did_replace && replaced_text == anchored,
        "single replacement does not reinterpret a suffix as the start of the document");
}

void TestTableEditing() {
  const std::wstring table = L"| A | B |\n| --- | --- |\n| 1 | 2 |";
  const auto moved = mdlite::MoveToAdjacentTableCell(table, table.find(L"1"), false);
  Check(!moved.changed && moved.selection == table.find(L"2"), "Tab moves to the next table cell");
  const auto appended = mdlite::MoveToAdjacentTableCell(table, table.find(L"2"), false);
  Check(appended.changed && appended.text.ends_with(L"|  |  |"), "Tab at the last cell appends a row");
  const auto inserted = mdlite::InsertTableColumn(table, table.find(L"1"), true);
  Check(inserted.changed && inserted.text.find(L"| --- | --- | --- |") != std::wstring::npos,
        "column insertion extends the delimiter row");
  const std::wstring literal_table =
      L"|  A  | B\\| raw | `C|D` |\r\n| :--- | ---: | :---: |\n| left  |  middle  | right |";
  const auto literal_insert = mdlite::InsertTableColumn(
      literal_table, literal_table.find(L"middle"), true);
  Check(literal_insert.changed &&
            literal_insert.text ==
                L"|  A  | B\\| raw |  | `C|D` |\r\n| :--- | ---: | --- | :---: |\n| left  |  middle  |  | right |" &&
            literal_insert.text[literal_insert.selection] == L'|',
        "column insertion preserves literal cell text, escaped pipes, and mixed line endings");
  const auto literal_delete = mdlite::DeleteTableColumn(literal_table, literal_table.find(L"middle"));
  Check(literal_delete.changed &&
            literal_delete.text == L"|  A  | `C|D` |\r\n| :--- | :---: |\n| left  | right |" &&
            literal_delete.selection == literal_delete.text.find(L"left"),
        "column deletion changes only the selected separator range and keeps caret in the row");
  const auto deleted = mdlite::DeleteTableRow(table, table.find(L"1"));
  Check(deleted.changed && deleted.text.find(L"| 1 | 2 |") == std::wstring::npos,
        "table row deletion removes only the selected row");
  const auto right_inside = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"1"), mdlite::TableCaretDirection::Right);
  const auto right_boundary = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"1") + 1, mdlite::TableCaretDirection::Right);
  Check(!right_inside && right_boundary && *right_boundary == table.find(L"2"),
        "Right stays in a cell until its boundary and then moves to the next cell");
  const auto down = mdlite::MoveTableCaretAtBoundary(
      table, table.find(L"A"), mdlite::TableCaretDirection::Down);
  Check(down && *down == table.find(L"1"),
        "Down skips the GFM delimiter row and keeps the table column");
  const auto backwards_from_eof_row = mdlite::MoveToAdjacentTableCell(table, table.find(L"1"), true);
  Check(!backwards_from_eof_row.changed &&
            backwards_from_eof_row.selection == table.find(L"B"),
        "Shift+Tab from the first body cell skips the delimiter and reaches the header");
  const std::wstring crlf_table = L"| A | B |\r\n| --- | --- |\r\n| 1 | 2 |";
  const auto crlf_insert = mdlite::InsertTableRow(crlf_table, crlf_table.find(L"1"), false);
  Check(crlf_insert.changed && crlf_insert.text.find(L"|  |  |\r\n| 1 | 2 |") != std::wstring::npos,
        "table row insertion preserves the surrounding CRLF convention");
  const auto crlf_after = mdlite::InsertTableRow(crlf_table, crlf_table.find(L"A"), true);
  Check(crlf_after.changed && crlf_after.text.find(L"| A | B |\r\n|  |  |\r\n| ---") != std::wstring::npos,
        "table row insertion after a CRLF row does not split or duplicate the line ending");
  const auto crlf_delete = mdlite::DeleteTableRow(crlf_table, crlf_table.find(L"1"));
  Check(crlf_delete.changed && crlf_delete.text.ends_with(L"| --- | --- |") &&
            crlf_delete.text.find(L"\n\n") == std::wstring::npos,
        "table row deletion consumes the complete CRLF line ending");
  const std::wstring escaped = L"| `a|b` | c\\|d | e |\n| --- | --- | --- |";
  const auto escaped_move = mdlite::MoveToAdjacentTableCell(escaped, escaped.find(L"a|b"), false);
  Check(!escaped_move.changed && escaped_move.selection == escaped.find(L"c\\|d"),
        "escaped and code-span pipes do not split visual table cells");
  const auto visual_rows = mdlite::ParseTableVisualRows(escaped, 0, escaped.size());
  Check(visual_rows.size() == 2 && visual_rows[0].cells.size() == 3,
        "visual table grid uses the same escaped/code-span separator semantics");
}

void TestJapaneseHolidays() {
  const auto name = mdlite::JapaneseHolidayName(2026, 9, 22);
  Check(name && *name == L"休日", "Cabinet Office holiday data includes 2026-09-22");
  Check(mdlite::JapaneseHolidayName(2026, 5, 6) &&
            *mdlite::JapaneseHolidayName(2026, 5, 6) == L"休日" &&
            mdlite::JapaneseHolidayName(2027, 3, 22) &&
            *mdlite::JapaneseHolidayName(2027, 3, 22) == L"休日",
        "bundled holiday fixtures include 2026-05-06 and 2027-03-22");
  Check(mdlite::JapaneseHolidayYearSupported(2027), "last bundled holiday year is supported");
  Check(!mdlite::JapaneseHolidayYearSupported(2028), "out-of-range holiday year remains unknown");
  mdlite::JapaneseHolidayImportInfo info;
  std::wstring error;
  Check(mdlite::ImportJapaneseHolidayCsv(L"date,name\n2028/01/01,元日\n2028/02/11,建国記念の日\n",
                                         info, error) && info.records == 2 &&
            mdlite::JapaneseHolidayName(2028, 1, 1) &&
            *mdlite::JapaneseHolidayName(2028, 1, 1) == L"元日",
        "local holiday CSV import atomically accepts a validated fixture");
  Check(mdlite::JapaneseHolidayYearSupported(2028) && mdlite::JapaneseHolidayLastYear() >= 2028,
        "imported holiday years become known without network access");
  error.clear();
  Check(!mdlite::ImportJapaneseHolidayCsv(L"date,name\n2028/01/01,元日\n2028/01/01,重複\n",
                                          info, error) && !error.empty(),
        "holiday CSV duplicate dates are rejected without replacing data");
  error.clear();
  Check(!mdlite::ValidateJapaneseHolidayCsv(L"<html><body>error</body></html>", info, error) &&
            !error.empty(),
        "HTML error pages are rejected as holiday data");
  error.clear();
  Check(mdlite::ValidateJapaneseHolidayCsv(L"date,name\n2028-05-01,祝日\n", info, error) &&
            info.first_year == 2028 && info.last_year == 2028,
        "holiday validation accepts strict ISO-like local CSV date fixtures");
  mdlite::ClearImportedJapaneseHolidays();
}

void TestAssets(const std::filesystem::path& root) {
  const auto workspace = root / L"assets";
  const auto source_directory = root / L"asset-source";
  std::filesystem::create_directories(workspace);
  std::filesystem::create_directories(source_directory);
  const auto png = source_directory / L"image.png";
  Check(WriteValidPng(png), "valid PNG fixture is encoded by WIC");
  mdlite::RasterImageInfo image_info;
  std::wstring error;
  Check(mdlite::ReadRasterImageInfo(png, image_info, error) && image_info.width == 2 &&
            image_info.height == 1 && image_info.frame_count == 1,
        "real PNG is decoded with dimensions and frame count");
  const auto jpeg = source_directory / L"image.jpg";
  Check(WriteValidJpeg(jpeg) &&
            mdlite::ReadRasterImageInfo(jpeg, image_info, error) && image_info.width == 2 &&
            image_info.height == 1,
        "real JPEG is encoded and decoded through the required native path");
  const auto webp = source_directory / L"image.webp";
  IStream* frame_stream{};
  unsigned frame_delay{};
  WriteBytes(webp, {0x52,0x49,0x46,0x46,0x1A,0x00,0x00,0x00,0x57,0x45,0x42,0x50,
                    0x56,0x50,0x38,0x4C,0x0E,0x00,0x00,0x00,0x2F,0x00,0x00,0x00,
                    0x10,0x07,0x10,0x11,0x11,0x88,0x88,0xFE,0x07,0x00});
  error.clear();
  const bool webp_decoded = mdlite::ReadRasterImageInfo(webp, image_info, error);
  Check(webp_decoded && image_info.width == 1 && image_info.height == 1,
        "real WebP is decoded through the bundled libwebp path");
  frame_stream = nullptr;
  frame_delay = 0;
  Check(mdlite::CreateRasterFramePngStream(webp, 0, frame_stream, frame_delay, error) &&
            frame_stream != nullptr && frame_delay >= 20,
        "a real WebP frame is decoded and converted to a native PNG stream");
  if (frame_stream) frame_stream->Release();
  const auto animated_webp = source_directory / L"animated.webp";
  Check(WriteAnimatedWebp(animated_webp), "animated WebP fixture is encoded by libwebp");
  error.clear();
  Check(mdlite::ReadRasterImageInfo(animated_webp, image_info, error) &&
            image_info.width == 2 && image_info.height == 1 && image_info.frame_count == 2 &&
            image_info.animated,
        "animated WebP dimensions and frame count are decoded");
  frame_stream = nullptr;
  frame_delay = 0;
  Check(mdlite::CreateRasterFramePngStream(animated_webp, 0, frame_stream, frame_delay, error) &&
            frame_stream != nullptr && frame_delay == 100,
        "animated WebP first frame is composited with its 100 ms timing");
  if (frame_stream) frame_stream->Release();
  frame_stream = nullptr;
  frame_delay = 0;
  const bool second_webp_frame = mdlite::CreateRasterFramePngStream(
      animated_webp, 1, frame_stream, frame_delay, error);
  if (second_webp_frame && frame_delay != 250) {
    std::cerr << "animated WebP second frame delay: " << frame_delay << " ms\n";
  }
  Check(second_webp_frame && frame_stream != nullptr && frame_delay == 250,
        "animated WebP second frame is composited with its distinct 250 ms timing");
  if (frame_stream) frame_stream->Release();
  Check(mdlite::CreateRasterFramePngStream(png, 0, frame_stream, frame_delay, error) &&
            frame_stream != nullptr && frame_delay >= 20,
        "a WIC raster frame is converted to an in-memory PNG for native image refresh");
  if (frame_stream) frame_stream->Release();
  const auto gif = source_directory / L"animated.gif";
  WriteBytes(gif, {
      0x47,0x49,0x46,0x38,0x39,0x61,0x01,0x00,0x01,0x00,0x80,0x00,0x00,
      0x00,0x00,0x00,0xFF,0xFF,0xFF,
      0x21,0xFF,0x0B,0x4E,0x45,0x54,0x53,0x43,0x41,0x50,0x45,0x32,0x2E,0x30,
      0x03,0x01,0x00,0x00,0x00,
      0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x44,0x01,0x00,
      0x21,0xF9,0x04,0x00,0x0A,0x00,0x00,0x00,
      0x2C,0x00,0x00,0x00,0x00,0x01,0x00,0x01,0x00,0x00,0x02,0x02,0x4C,0x01,0x00,
      0x3B});
  error.clear();
  Check(mdlite::ReadRasterImageInfo(gif, image_info, error) && image_info.animated &&
            image_info.frame_count == 2,
        "animated GIF is decoded as a multi-frame raster image");
  frame_stream = nullptr;
  Check(mdlite::CreateRasterFramePngStream(gif, 1, frame_stream, frame_delay, error) &&
            frame_stream != nullptr,
        "a later animated GIF frame is converted for native playback");
  if (frame_stream) frame_stream->Release();
  const auto partial_gif = source_directory / L"partial-disposal.gif";
  Check(WritePartialDisposalGif(partial_gif), "partial-frame GIF fixture is written");
  error.clear();
  Check(mdlite::ReadRasterImageInfo(partial_gif, image_info, error) && image_info.animated &&
            image_info.width == 3 && image_info.height == 1 && image_info.frame_count == 4,
        "partial-frame GIF reports its logical canvas and frame count");
  const auto check_gif_frame = [&](unsigned index, unsigned expected_delay,
                                   const std::vector<unsigned char>& expected,
                                   const char* message) {
    IStream* composed{};
    unsigned delay{};
    unsigned width{}, height{};
    std::vector<unsigned char> pixels;
    error.clear();
    const bool created = mdlite::CreateRasterFramePngStream(
        partial_gif, index, composed, delay, error);
    const bool decoded = created && DecodePngStream(composed, width, height, pixels);
    Check(decoded && width == 3 && height == 1 && delay == expected_delay && pixels == expected,
          message);
    if (composed) composed->Release();
  };
  check_gif_frame(0, 100,
                  {0,0,255,255, 0,0,255,255, 0,0,255,255},
                  "GIF frame 0 fills the logical canvas");
  check_gif_frame(1, 200,
                  {0,0,255,255, 0,255,0,255, 0,0,255,255},
                  "GIF partial frame honors offset and transparent pixels");
  check_gif_frame(2, 300,
                  {0,0,0,255, 0,0,0,255, 255,0,0,255},
                  "GIF disposal 2 restores the previous frame rectangle to background");
  check_gif_frame(3, 400,
                  {255,255,255,255, 0,0,0,255, 0,0,255,255},
                  "GIF disposal 3 restores the canvas saved before the previous frame");
  mdlite::AssetImportResult first{};
  Check(mdlite::ImportImageAsset(png, workspace, workspace / L"note.md", first, error),
        "supported image copies into workspace assets");
  Check(first.relative_reference == L"assets/image.png" && first.created_new_asset,
        "image reference is document-relative and reports its newly copied asset");
  mdlite::AssetImportResult existing_asset{};
  Check(mdlite::ImportImageAsset(first.stored_path, workspace, workspace / L"note.md",
                                 existing_asset, error) && !existing_asset.created_new_asset &&
            existing_asset.stored_path == first.stored_path,
        "reusing an existing workspace asset does not claim ownership of or replace it");
  Check(WriteValidPng(png), "replacement PNG fixture is valid");
  mdlite::AssetImportResult second{};
  Check(mdlite::ImportImageAsset(png, workspace, workspace / L"note.md", second, error),
        "second image import succeeds without overwrite");
  Check(second.stored_path.filename() == L"image_1.png" && second.created_new_asset,
        "asset collision gets a new name and reports the newly copied asset");
  const auto corrupt = source_directory / L"corrupt.png";
  WriteBytes(corrupt, {0x89, 'P', 'N', 'G'});
  mdlite::AssetImportResult corrupt_result{};
  error.clear();
  Check(!mdlite::ImportImageAsset(corrupt, workspace, workspace / L"note.md", corrupt_result, error),
        "corrupt raster image is rejected before a Markdown link is created");
  const auto svg = source_directory / L"unsafe.svg";
  const std::string unsafe = "<svg><script>alert(1)</script></svg>";
  WriteBytes(svg, std::vector<unsigned char>(unsafe.begin(), unsafe.end()));
  mdlite::AssetImportResult svg_result{};
  Check(mdlite::ImportImageAsset(svg, workspace, workspace / L"note.md", svg_result, error),
        "unsafe SVG is preserved as a local asset");
  Check(!svg_result.safe_to_render, "unsafe SVG is disabled for rendering");
  IStream* unsafe_svg_stream{};
  unsigned unsafe_svg_delay{};
  error.clear();
  Check(!mdlite::CreateRasterFramePngStream(svg, 0, unsafe_svg_stream, unsafe_svg_delay, error) &&
            unsafe_svg_stream == nullptr,
        "the SVG rasterization entry point independently rejects unsafe bytes");
  const auto inspect_svg = [&](std::wstring_view name, std::string_view body) {
    const auto path = source_directory / std::filesystem::path(name);
    WriteBytes(path, std::vector<unsigned char>(body.begin(), body.end()));
    bool safe{};
    std::wstring message;
    error.clear();
    Check(mdlite::InspectImageSafety(path, safe, message, error),
          "SVG safety inspection completes without fetching references");
    return safe;
  };
  Check(inspect_svg(L"safe.svg",
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"10\">"
                    "<defs><linearGradient id=\"g\"><stop offset=\"0\"/></linearGradient></defs>"
                    "<rect width=\"20\" height=\"10\" fill=\"url(#g)\"/></svg>"),
        "ordinary SVG with an internal paint reference is allowed");
  Check(!inspect_svg(L"spaced-href.svg", "<svg><use href = \"https://example.invalid/a.svg#x\"/></svg>"),
        "whitespace around external href is rejected structurally");
  Check(!inspect_svg(L"spaced-event.svg", "<svg onload = \"alert(1)\"><path d=\"M0 0\"/></svg>"),
        "all event attributes are rejected regardless of spacing");
  Check(!inspect_svg(L"css-url.svg", "<svg><rect style=\"fill:url(https://example.invalid/a)\"/></svg>"),
        "external CSS url references are rejected");
  Check(!inspect_svg(L"entity.svg",
                     "<!DOCTYPE svg [<!ENTITY xxe SYSTEM 'file:///Windows/win.ini'>]><svg>&xxe;</svg>"),
        "DTD and external entities are rejected by the XML parser");
  Check(!inspect_svg(L"xml-base.svg",
                     "<svg xmlns=\"http://www.w3.org/2000/svg\" xml:base=\"https://example.invalid/\">"
                     "<use href=\"#shape\"/></svg>"),
        "external XML base cannot turn an internal fragment into a remote reference");
  Check(!inspect_svg(L"xml-stylesheet.svg",
                     "<?xml-stylesheet type=\"text/css\" href=\"https://example.invalid/x.css\"?>"
                     "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect width=\"1\" height=\"1\"/></svg>"),
        "external xml-stylesheet processing instructions are rejected");
  Check(InsertNativeRichEditImage(png, false),
        "PNG stream inserts one native RichEdit inline image");
  Check(InsertNativeRichEditImage(jpeg, false),
        "JPEG stream inserts one native RichEdit inline image");
  Check(InsertNativeRichEditImage(gif, true),
        "decoded GIF frame inserts one native RichEdit inline image");
  Check(InsertNativeRichEditImage(webp, true),
        "bundled WebP frame inserts one native RichEdit inline image");
  Check(InsertNativeRichEditImage(source_directory / L"safe.svg", true),
        "validated and rasterized SVG inserts one native RichEdit inline image");
  Check(mdlite::ImageHtml(L"a\"b", L"assets/x.png", 480).find(L"width=\"480\"") != std::wstring::npos,
        "image resize markup stores width without height");
}

void TestStorageAdapter(const std::filesystem::path& root) {
  const auto directory = root / L"storage";
  std::filesystem::create_directories(directory);
  const auto asset = directory / L"asset.png";
  WriteBytes(asset, {'P','N','G'});
  const auto config = directory / L"storage.toml";
  WriteBytes(config, std::vector<unsigned char>{
      'e','x','e','c','u','t','a','b','l','e',' ','=',' ','"','C',':','/','W','i','n','d','o','w','s','/','S','y','s','t','e','m','3','2','/','c','m','d','.','e','x','e','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','/','d','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','/','c','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','e','c','h','o','"','\n',
      'a','r','g','u','m','e','n','t',' ','=',' ','"','h','t','t','p','s',':','/','/','e','x','a','m','p','l','e','.','i','n','v','a','l','i','d','/','a','s','s','e','t','"','\n'});
  mdlite::StorageAdapter adapter;
  std::wstring error;
  Check(mdlite::LoadStorageAdapter(config, adapter, error), "storage adapter config loads without credentials");
  mdlite::StorageUploadResult result;
  const bool uploaded = mdlite::UploadWithStorageAdapter(adapter, asset, L"revision-1", nullptr, result, error);
  Check(uploaded,
        "mock storage adapter success is supported");
  Check(result.reference == L"https://example.invalid/asset" && ReadBytes(asset) == std::vector<unsigned char>({'P','N','G'}),
        "storage adapter keeps local asset and returns a reference");
  auto reference_adapter = adapter;
  reference_adapter.arguments.back() = L"https://example.invalid/a b.png";
  mdlite::StorageUploadResult spaced_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          spaced_reference, error) && !error.empty(),
        "storage adapter rejects whitespace in an HTTPS URI before it can enter Markdown");
  reference_adapter.arguments.back() = L"https:///missing-host.png";
  mdlite::StorageUploadResult missing_authority_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          missing_authority_reference, error) && !error.empty(),
        "storage adapter rejects HTTPS references without an authority");
  reference_adapter.arguments.back() = L"https://example.invalid/a%2G.png";
  mdlite::StorageUploadResult malformed_escape_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          malformed_escape_reference, error) && !error.empty(),
        "storage adapter rejects malformed percent escapes in an HTTPS URI");
  reference_adapter.arguments.back() = L"https://example.invalid:abc/asset.png";
  mdlite::StorageUploadResult malformed_port_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          malformed_port_reference, error) && !error.empty(),
        "storage adapter rejects nonnumeric HTTPS authority ports");
  reference_adapter.arguments.back() = L"https://[garbage]/asset.png";
  mdlite::StorageUploadResult malformed_ip_literal_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          malformed_ip_literal_reference, error) && !error.empty(),
        "storage adapter rejects malformed HTTPS IP literals");
  reference_adapter.arguments.back() = L"https://host[name/asset.png";
  mdlite::StorageUploadResult malformed_reg_name_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          malformed_reg_name_reference, error) && !error.empty(),
        "storage adapter rejects URI-reserved characters in an unbracketed host");
  reference_adapter.arguments.back() = L"https://example.invalid/a{b}.png";
  mdlite::StorageUploadResult malformed_uri_character_reference;
  error.clear();
  Check(!mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                          malformed_uri_character_reference, error) && !error.empty(),
        "storage adapter rejects characters outside the RFC 3986 URI character set");
  reference_adapter.arguments.back() = L"https://[2001:db8::1]:443/a%20b.png?x=1";
  mdlite::StorageUploadResult encoded_reference;
  error.clear();
  Check(mdlite::UploadWithStorageAdapter(reference_adapter, asset, L"revision-1", nullptr,
                                         encoded_reference, error) &&
            encoded_reference.reference == reference_adapter.arguments.back(),
        "storage adapter accepts a properly encoded URL destination");

  const auto invalid_config = directory / L"storage-invalid.toml";
  WriteBytes(invalid_config, std::vector<unsigned char>{
      'e','x','e','c','u','t','a','b','l','e',' ','=',' ','"','c','m','d','.','e','x','e','"','\n',
      't','i','m','e','o','u','t','_','m','s',' ','=',' ','n','o','t','-','a','-','n','u','m','b','e','r','\n'});
  error.clear();
  Check(!mdlite::LoadStorageAdapter(invalid_config, adapter, error) && !error.empty(),
        "invalid storage timeout is reported without terminating the app");
}

void TestSettings(const std::filesystem::path& root) {
  const auto defaults = mdlite::DefaultSettingsLayer();
  Check(defaults.theme == mdlite::ThemeMode::Dark,
        "new settings default to the approved dark theme");
  const auto directory = root / L"settings";
  const auto common_path = directory / L"common.toml";
  const auto workspace_path = directory / L"workspace.toml";
  mdlite::SettingsLayer common;
  common.theme = mdlite::ThemeMode::Dark;
  common.auto_save = false;
  common.auto_save_delay_ms = 1500;
  common.colors[L"link"] = L"#80A0FF";
  common.font_face = L"Yu Gothic UI";
  common.default_memo_workspace = std::filesystem::absolute(directory / L"memos");
  common.keybindings[L"file.save"] = L"Ctrl+Shift+S";
  std::wstring error;
  Check(mdlite::SaveSettingsLayer(common_path, common, error), "common settings save atomically");
  mdlite::SettingsLayer workspace;
  workspace.theme = mdlite::ThemeMode::Light;
  workspace.font_size_pt = 14;
  Check(mdlite::SaveSettingsLayer(workspace_path, workspace, error), "workspace settings save atomically");
  mdlite::EffectiveSettings effective;
  Check(mdlite::ResolveSettings(common_path, workspace_path, effective, error), "settings hierarchy resolves");
  Check(effective.theme == mdlite::ThemeMode::Light && effective.font_face == L"Yu Gothic UI" &&
            effective.font_size_pt == 14 && !effective.auto_save && effective.auto_save_delay_ms == 1500 &&
            effective.colors[L"link"] == L"#80A0FF" &&
            effective.default_memo_workspace == *common.default_memo_workspace,
        "workspace overrides common while inherited values remain");
  Check(effective.origins[L"theme"] == L"Workspace上書き" &&
            effective.origins[L"font_face"] == L"共通設定",
        "effective settings expose their origins");
  workspace.keybindings[L"file.save"] = L"Ctrl+O";
  Check(mdlite::SaveSettingsLayer(workspace_path, workspace, error), "individual layer accepts a shortcut used only in another layer");
  error.clear();
  Check(!mdlite::ResolveSettings(common_path, workspace_path, effective, error) && !error.empty(),
        "effective keybinding conflicts are rejected");
  mdlite::SettingsLayer invalid;
  invalid.font_face = L"Broken\"Font";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid.toml", invalid, error),
        "settings serializer rejects unsafe quoted strings");
  invalid = {};
  invalid.colors[L"unknown"] = L"#FFFFFF";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid-color.toml", invalid, error),
        "settings reject unknown custom color keys");
  invalid = {};
  invalid.keybindings[L"file.open"] = L"Ctrl+Shift+NotAKey";
  error.clear();
  Check(!mdlite::SaveSettingsLayer(directory / L"invalid-shortcut.toml", invalid, error),
        "settings reject shortcuts the accelerator parser cannot apply");
  const std::string malformed = "font_size_pt = 12junk\n";
  WriteBytes(directory / L"malformed.toml",
             std::vector<unsigned char>(malformed.begin(), malformed.end()));
  error.clear();
  Check(!mdlite::LoadSettingsLayer(directory / L"malformed.toml", invalid, error),
        "settings reject numeric values with trailing characters");
  const std::string future = "schema_version = 1\n[future]\nnew_option = \"keep-me\"\n";
  const auto future_path = directory / L"future.toml";
  WriteBytes(future_path, std::vector<unsigned char>(future.begin(), future.end()));
  mdlite::SettingsLayer future_layer;
  Check(mdlite::LoadSettingsLayer(future_path, future_layer, error) &&
            mdlite::SaveSettingsLayer(future_path, future_layer, error),
        "settings GUI model can round trip a file with future fields");
  const auto preserved = ReadBytes(future_path);
  const std::string preserved_text(preserved.begin(), preserved.end());
  Check(preserved_text.find("new_option = \"keep-me\"") != std::string::npos,
        "settings save preserves fields unknown to this build");
  const std::string legacy_holiday = "schema_version = 1\nholiday_auto_update = true\n";
  const auto legacy_path = directory / L"legacy-holiday-update.toml";
  WriteBytes(legacy_path, std::vector<unsigned char>(legacy_holiday.begin(), legacy_holiday.end()));
  mdlite::SettingsLayer legacy_layer;
  Check(mdlite::LoadSettingsLayer(legacy_path, legacy_layer, error) && legacy_layer.preserved_lines.empty(),
        "removed holiday network setting is consumed instead of preserved");
  Check(mdlite::SaveSettingsLayer(legacy_path, legacy_layer, error),
        "legacy holiday network setting can be saved away");
  const auto legacy_saved = ReadBytes(legacy_path);
  const std::string legacy_saved_text(legacy_saved.begin(), legacy_saved.end());
  Check(legacy_saved_text.find("holiday_auto_update") == std::string::npos,
        "settings save does not restore the removed holiday network setting");

  const auto guarded_path = directory / L"guarded-settings.toml";
  mdlite::SettingsLayer baseline;
  baseline.auto_save = false;
  baseline.auto_save_delay_ms = 1250;
  baseline.font_size_pt = 11;
  Check(mdlite::SaveSettingsLayer(guarded_path, baseline, error), "guarded settings fixture saved");
  mdlite::SettingsFileSnapshot snapshot;
  mdlite::SettingsLayer captured;
  Check(mdlite::CaptureSettingsFile(guarded_path, snapshot, error) &&
        mdlite::ParseSettingsSnapshot(snapshot, captured, error),
        "settings values parse from the same captured file snapshot");
  auto external = baseline;
  external.auto_save = true;
  external.auto_save_delay_ms = 1900;
  external.font_size_pt = 14;
  Check(mdlite::SaveSettingsLayer(guarded_path, external, error), "peer changes settings after form capture");
  const auto peer_bytes = ReadBytes(guarded_path);
  mdlite::SettingsLayer captured_again;
  Check(mdlite::ParseSettingsSnapshot(snapshot, captured_again, error) &&
        captured_again.auto_save_delay_ms == 1250,
        "parsing retained snapshot does not silently reload peer values");
  Check(!mdlite::SaveSettingsSnapshot(snapshot, captured, error) &&
        ReadBytes(guarded_path) == peer_bytes,
        "unchanged serialization still rejects a stale settings snapshot without overwriting peer bytes");

  Check(mdlite::CaptureSettingsFile(guarded_path, snapshot, error), "recapture current settings snapshot");
  std::filesystem::remove(guarded_path);
  Check(!mdlite::SaveSettingsSnapshot(snapshot, baseline, error) &&
        !std::filesystem::exists(guarded_path),
        "deleted settings are not recreated from a stale existing-file snapshot");

  const auto missing_path = directory / L"new-settings-directory" / L"settings.toml";
  Check(mdlite::CaptureSettingsFile(missing_path, snapshot, error) && !snapshot.existed,
        "missing settings have an explicit absent-file snapshot");
  Check(mdlite::SaveSettingsLayer(missing_path, external, error), "peer creates missing settings after capture");
  const auto created_peer_bytes = ReadBytes(missing_path);
  Check(!mdlite::SaveSettingsSnapshot(snapshot, baseline, error) &&
        ReadBytes(missing_path) == created_peer_bytes,
        "absent-file snapshot rejects peer-created settings");
  std::filesystem::remove(missing_path);
  Check(mdlite::CaptureSettingsFile(missing_path, snapshot, error) &&
        mdlite::SaveSettingsSnapshot(snapshot, baseline, error) && snapshot.existed &&
        !snapshot.document.HasExternalChange(),
        "guarded first settings save creates a file and advances its snapshot");
  const auto unchanged_bytes = ReadBytes(missing_path);
  const auto unchanged_time = std::filesystem::last_write_time(missing_path);
  Check(mdlite::SaveSettingsSnapshot(snapshot, baseline, error) &&
        ReadBytes(missing_path) == unchanged_bytes &&
        std::filesystem::last_write_time(missing_path) == unchanged_time,
        "unchanged settings apply succeeds without rewriting the file");
  auto same_size = unchanged_bytes;
  const std::string unchanged_text(unchanged_bytes.begin(), unchanged_bytes.end());
  const auto delay_position = unchanged_text.find("1250");
  same_size[delay_position] = '2';
  WriteBytes(missing_path, same_size);
  std::filesystem::last_write_time(missing_path, unchanged_time);
  Check(!mdlite::SaveSettingsSnapshot(snapshot, baseline, error) && ReadBytes(missing_path) == same_size,
        "settings content fingerprint rejects same-size same-timestamp peer changes");
}

void TestGitConflicts() {
  const std::wstring source =
      L"before\r\n<<<<<<< HEAD\r\ncurrent\r\n||||||| base\r\nold\r\n=======\r\nincoming\r\n>>>>>>> topic\r\nafter\r\n"
      L"<<<<<<< HEAD\nleft\n=======\nright\n>>>>>>> topic\n";
  const auto blocks = mdlite::ParseConflictBlocks(source);
  Check(blocks.size() == 2, "normal and diff3 conflict blocks parse");
  const auto next = mdlite::FindConflictBlock(source, 1, false);
  const auto previous = mdlite::FindConflictBlock(source, 1, true);
  Check(next && *next == 0 && previous && *previous == 1, "conflict navigation wraps in both directions");
  const auto current = mdlite::ResolveConflictBlock(source, 0, mdlite::ConflictChoice::Current);
  Check(current.changed && current.text.find(L"current\r\n") != std::wstring::npos &&
            current.text.find(L"old\r\n") == std::wstring::npos &&
            current.text.find(L"incoming\r\n") == std::wstring::npos,
        "current-side resolution excludes diff3 base and incoming text");
  const auto both = mdlite::ResolveConflictBlock(source, 1, mdlite::ConflictChoice::Both);
  Check(both.changed && both.text.find(L"left\nright\n") != std::wstring::npos,
        "both-side resolution preserves side order and line endings");
  Check(mdlite::ParseConflictBlocks(L"literal <<<<<<< text\n").empty(),
        "inline marker-like text is not a conflict block");
}

}  // namespace

int wmain() {
  const HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
  const auto root = std::filesystem::temp_directory_path() /
                    (L"mdlite-core-tests-" + std::to_wstring(GetCurrentProcessId()));
  std::error_code error;
  std::filesystem::remove_all(root, error);
  std::filesystem::create_directories(root);
  TestUtf8NoOp(root);
  TestUntitledDocument(root);
  TestSaveAs(root);
  TestCp932RoundTrip(root);
  TestExternalConflict(root);
  TestEmptyEncodingAndInvalidBom(root);
  TestEditorLineEndingBoundary(root);
  TestMarkdown();
  TestEditorAdapter();
  TestEditorTransactionBoundaries();
  TestWorkspaceState(root);
  TestTrustAndProcess(root);
  TestProfiles(root);
  TestSearch(root);
  TestTableEditing();
  TestJapaneseHolidays();
  TestAssets(root);
  TestStorageAdapter(root);
  TestSettings(root);
  TestGitConflicts();
  std::filesystem::remove_all(root, error);
  if (SUCCEEDED(com)) CoUninitialize();
  if (failures == 0) std::cout << "All " << checks << " MDLite core checks passed.\n";
  return failures == 0 ? 0 : 1;
}
