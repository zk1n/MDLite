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
  const auto visual = mdlite::ParseMarkdown(L"![代替](assets/a.png)\n| A | B |\n|---|---|\n| 1 | 2 |\n");
  Check(visual.images.size() == 1 && visual.images.front().target == L"assets/a.png",
        "image references retain source ranges and targets");
  Check(visual.tables.size() == 1, "GFM table blocks are identified for native presentation");
  const auto links = mdlite::ParseMarkdown(L"[local](notes/a.md#heading) ![image](a.png) <https://example.test/>\n");
  Check(links.links.size() == 2 && links.links[0].target == L"notes/a.md#heading" &&
            links.links[1].target == L"https://example.test/",
        "Markdown and autolinks are parsed while image syntax stays separate");
  const auto resized = mdlite::ParseMarkdown(
      L"<img src=\"assets/a.png\" alt=\"sample\" width=\"480\">\n");
  Check(resized.images.size() == 1 && resized.images.front().target == L"assets/a.png" &&
            resized.images.front().width_dip == 480,
        "safe generated img markup retains target and display width");
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

  const auto edited = mdlite::ApplyEditorText(snapshot, L"a\nb😀\r\nc", L"a\r\nnew\r\nb😀\r\nc");
  Check(edited.changed && edited.source == L"a\nnew\nb😀\r\nc",
        "range transaction preserves LF and existing CRLF without whole-document normalization");
  const auto deleted = mdlite::ApplyEditorText(mdlite::BuildEditorSnapshot(L"a\r\nb\nc"), L"a\r\nb\nc", L"b\r\nc");
  Check(deleted.source == L"b\nc", "range transaction keeps the surviving mixed line ending");
  const auto image = mdlite::BuildMarkdownEditorSnapshot(L"before ![alt](img.png) after");
  Check(image.view == L"before \uFFFC after", "derived editor view represents an image without changing source");
  Check(image.SourceToView(22) == 8 && image.ViewToSource(8) == 22,
        "compact mapping resumes immediately after a collapsed image range");
  const auto image_deleted = mdlite::ApplyEditorText(image, L"before ![alt](img.png) after", L"before  after");
  Check(image_deleted.source == L"before  after", "deleting the derived image removes its complete source range");
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
  const std::wstring adjacent_source = L"A![x](p.png)B";
  const auto adjacent_snapshot = mdlite::BuildMarkdownEditorSnapshot(adjacent_source);
  const auto adjacent_edit = mdlite::ApplyEditorText(adjacent_snapshot, adjacent_source, L"a\uFFFCb");
  Check(adjacent_edit.source == L"a![x](p.png)b",
        "one coalesced edit on both sides preserves the unchanged image Markdown source");
  const std::wstring two_images = L"A![1](1.png)X![2](2.png)B";
  const auto two_image_snapshot = mdlite::BuildMarkdownEditorSnapshot(two_images);
  const auto first_image_deleted = mdlite::ApplyEditorText(
      two_image_snapshot, two_images, L"aX\uFFFCb");
  Check(first_image_deleted.source == L"aX![2](2.png)b",
        "deleting the first of two derived images preserves the surviving image identity");
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
  const auto target_native = mdlite::BuildNativeEditorSnapshot(table_edit.text);
  const auto native_transaction = mdlite::ApplyEditorText(
      mixed_native, image_then_table, target_native.view);
  Check(native_transaction.source == table_edit.text,
        "native transaction after a derived image maps back to the exact Markdown source");
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
  Check(store.WriteSessionState(session, error), "detailed session state writes");
  mdlite::SessionState detailed;
  Check(store.ReadSessionState(detailed, error), "detailed session state reads");
  Check(detailed.documents.size() == 1 && detailed.documents[0].selection_begin == 1 &&
            detailed.documents[0].selection_end == 2 && detailed.documents[0].first_visible_line == 3 &&
            detailed.documents[0].compact && detailed.documents[0].width == 640 &&
            detailed.main_width == 1100 && detailed.recent_documents.size() == 2 &&
            detailed.recent_documents[0].filename() == L"project-note.txt",
        "session preserves selection, scroll, compact placement, and main placement");
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
            backwards_from_eof_row.selection == table.rfind(L"---"),
        "Shift+Tab from the first cell of an EOF row reaches the previous row");
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
        "online holiday validation accepts strict ISO-like date fixtures");
  Check(mdlite::JapaneseHolidayUpdateDue(0, 0, 1),
        "fake clock treats an empty holiday update state as due");
  Check(!mdlite::JapaneseHolidayUpdateDue(1'000, 0, 1'000 + 27 * 24 * 60 * 60),
        "fake clock suppresses a holiday update before the 28-day interval");
  Check(mdlite::JapaneseHolidayUpdateDue(1'000, 0, 1'000 + 28 * 24 * 60 * 60),
        "fake clock schedules a holiday update at the 28-day interval");
  Check(!mdlite::JapaneseHolidayUpdateDue(1'000, 0, 900),
        "clock rollback does not turn every startup into a network retry");
  const std::wstring online_fixture =
      L"date,name\n2028/01/01,元日\n2028/02/11,建国記念の日\n"
      L"2028/02/23,天皇誕生日\n2028/03/20,春分の日\n2028/04/29,昭和の日\n"
      L"2028/05/03,憲法記念日\n2028/05/04,みどりの日\n2028/05/05,こどもの日\n"
      L"2028/07/17,海の日\n2028/08/11,山の日\n2028/09/18,敬老の日\n";
  const auto accepted = mdlite::AssessJapaneseHolidayResponse(
      200, false, false, 10, online_fixture);
  Check(accepted.accepted && accepted.replace_cache && accepted.info.records == 11,
        "mock HTTP 200 accepts a validated holiday payload for atomic cache replacement");
  const auto unchanged = mdlite::AssessJapaneseHolidayResponse(304, true, true, 11, {});
  Check(unchanged.accepted && !unchanged.replace_cache,
        "mock HTTP 304 accepts only when a verified last-known-good cache exists");
  const auto missing_cache = mdlite::AssessJapaneseHolidayResponse(304, true, false, 11, {});
  Check(!missing_cache.accepted && !missing_cache.error.empty(),
        "mock HTTP 304 without a verified cache fails closed");
  const auto not_found = mdlite::AssessJapaneseHolidayResponse(404, false, false, 11, {});
  Check(!not_found.accepted && not_found.error.find(L"404") != std::wstring::npos,
        "mock HTTP 404 preserves the existing holiday data");
  const auto server_error = mdlite::AssessJapaneseHolidayResponse(500, false, false, 11, {});
  Check(!server_error.accepted && server_error.error.find(L"500") != std::wstring::npos,
        "mock HTTP 500 preserves the existing holiday data");
  const auto timeout = mdlite::AssessJapaneseHolidayResponse(0, false, false, 11, {});
  Check(!timeout.accepted && !timeout.error.empty(),
        "mock timeout/network failure keeps the last-known-good holiday data");
  const auto empty_body = mdlite::AssessJapaneseHolidayResponse(200, false, false, 11, {});
  Check(!empty_body.accepted && !empty_body.replace_cache && !empty_body.error.empty(),
        "mock HTTP empty bodies cannot replace the holiday cache");
  const auto html = mdlite::AssessJapaneseHolidayResponse(
      200, false, false, 11, L"<html><body>error</body></html>");
  Check(!html.accepted && !html.replace_cache,
        "mock HTTP HTML error pages cannot replace the holiday cache");
  const auto shrink = mdlite::AssessJapaneseHolidayResponse(200, false, false, 30,
                                                             online_fixture);
  Check(!shrink.accepted && shrink.error.find(L"減少") != std::wstring::npos,
        "mock HTTP large record reductions are rejected");
  wchar_t real_http[2]{};
  if (GetEnvironmentVariableW(L"MDLITE_TEST_REAL_HOLIDAY_HTTP", real_http, 2) == 1 &&
      real_http[0] == L'1') {
    mdlite::JapaneseHolidayOnlineResult result;
    const bool fetched = mdlite::FetchJapaneseHolidayCsv(std::stop_token{}, {}, {}, result);
    if (!fetched) {
      // Keep the diagnostic ASCII-safe even when the test process has the
      // default "C" locale and cannot render the Japanese product text.
      std::string error_ascii;
      for (const wchar_t character : result.error) {
        error_ascii += character < 0x80 ? static_cast<char>(character) : '?';
      }
      std::cerr << "real holiday HTTP status=" << result.status
                << " error_length=" << result.error.size()
                << " error_ascii=" << error_ascii << "\n";
    }
    Check(fetched && result.status == 200 && !result.csv.empty() && result.error.empty(),
          "opt-in real WinHTTP holiday probe receives the official CSV");
  }
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
  Check(first.relative_reference == L"assets/image.png", "image reference is document-relative");
  Check(WriteValidPng(png), "replacement PNG fixture is valid");
  mdlite::AssetImportResult second{};
  Check(mdlite::ImportImageAsset(png, workspace, workspace / L"note.md", second, error),
        "second image import succeeds without overwrite");
  Check(second.stored_path.filename() == L"image_1.png", "asset collision gets a new name");
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

  const auto invalid_config = directory / L"storage-invalid.toml";
  WriteBytes(invalid_config, std::vector<unsigned char>{
      'e','x','e','c','u','t','a','b','l','e',' ','=',' ','"','c','m','d','.','e','x','e','"','\n',
      't','i','m','e','o','u','t','_','m','s',' ','=',' ','n','o','t','-','a','-','n','u','m','b','e','r','\n'});
  error.clear();
  Check(!mdlite::LoadStorageAdapter(invalid_config, adapter, error) && !error.empty(),
        "invalid storage timeout is reported without terminating the app");
}

void TestSettings(const std::filesystem::path& root) {
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
            !effective.holiday_auto_update &&
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
