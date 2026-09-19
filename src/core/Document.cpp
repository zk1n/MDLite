#include "core/Document.h"

#include <windows.h>

#include <algorithm>
#include <atomic>
#include <fstream>
#include <limits>
#include <span>
#include <system_error>

namespace mdlite {
namespace {

std::wstring WindowsError(const wchar_t* action, DWORD code = GetLastError()) {
  wchar_t* message = nullptr;
  const DWORD length = FormatMessageW(
      FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
      nullptr, code, 0, reinterpret_cast<wchar_t*>(&message), 0, nullptr);
  std::wstring result(action);
  result += L"（";
  if (length != 0 && message != nullptr) {
    result.append(message, length);
    while (!result.empty() && (result.back() == L'\r' || result.back() == L'\n')) result.pop_back();
  } else {
    result += L"Windows error ";
    result += std::to_wstring(code);
  }
  result += L"）";
  if (message != nullptr) LocalFree(message);
  return result;
}

bool ReadBytes(const std::filesystem::path& path, std::vector<std::byte>& bytes, std::wstring& error) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    error = L"ファイルを開けません: " + path.wstring();
    return false;
  }
  const auto end = input.tellg();
  if (end < 0 || static_cast<std::uint64_t>(end) >
                     static_cast<std::uint64_t>(std::numeric_limits<std::size_t>::max())) {
    error = L"ファイルサイズを取得できません。";
    return false;
  }
  bytes.resize(static_cast<std::size_t>(end));
  input.seekg(0, std::ios::beg);
  if (!bytes.empty() && !input.read(reinterpret_cast<char*>(bytes.data()), end)) {
    error = L"ファイルを最後まで読み込めません。";
    return false;
  }
  return true;
}

bool Decode(std::span<const std::byte> bytes, UINT code_page, DWORD flags, std::wstring& text) {
  if (bytes.empty()) {
    text.clear();
    return true;
  }
  if (bytes.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) return false;
  const auto* source = reinterpret_cast<const char*>(bytes.data());
  const int source_size = static_cast<int>(bytes.size());
  const int required = MultiByteToWideChar(code_page, flags, source, source_size, nullptr, 0);
  if (required <= 0) return false;
  text.resize(static_cast<std::size_t>(required));
  return MultiByteToWideChar(code_page, flags, source, source_size, text.data(), required) == required;
}

bool Encode(const std::wstring& text, TextEncoding encoding, std::vector<std::byte>& bytes,
            std::wstring& error) {
  const UINT code_page = encoding == TextEncoding::Cp932 ? 932U : CP_UTF8;
  const DWORD flags = encoding == TextEncoding::Cp932 ? WC_NO_BEST_FIT_CHARS : WC_ERR_INVALID_CHARS;
  BOOL used_default = FALSE;
  BOOL* used_default_ptr = encoding == TextEncoding::Cp932 ? &used_default : nullptr;
  if (text.size() > static_cast<std::size_t>(std::numeric_limits<int>::max())) {
    error = L"文書が文字コード変換の上限を超えています。";
    return false;
  }
  const int source_size = static_cast<int>(text.size());
  const int required = WideCharToMultiByte(code_page, flags, text.data(), source_size, nullptr, 0,
                                           nullptr, used_default_ptr);
  if (required <= 0 || used_default) {
    error = encoding == TextEncoding::Cp932
                ? L"CP932で表現できない文字があります。UTF-8へ変換するか別名保存してください。"
                : L"UTF-8へ変換できない文字があります。";
    return false;
  }
  const std::size_t prefix = encoding == TextEncoding::Utf8Bom ? 3U : 0U;
  bytes.resize(prefix + static_cast<std::size_t>(required));
  if (prefix != 0U) {
    bytes[0] = std::byte{0xEF};
    bytes[1] = std::byte{0xBB};
    bytes[2] = std::byte{0xBF};
  }
  if (required != 0 &&
      WideCharToMultiByte(code_page, flags, text.data(), source_size,
                          reinterpret_cast<char*>(bytes.data() + prefix), required, nullptr,
                          used_default_ptr) != required) {
    error = L"文字コード変換に失敗しました。";
    return false;
  }
  if (used_default) {
    error = L"CP932で表現できない文字があります。";
    return false;
  }
  return true;
}

FileFingerprint Fingerprint(const std::filesystem::path& path) {
  FileFingerprint result{};
  HANDLE file = CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES,
                            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                            OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) return result;
  BY_HANDLE_FILE_INFORMATION info{};
  if (GetFileInformationByHandle(file, &info)) {
    result.size = (static_cast<std::uint64_t>(info.nFileSizeHigh) << 32U) | info.nFileSizeLow;
    result.write_time = (static_cast<std::uint64_t>(info.ftLastWriteTime.dwHighDateTime) << 32U) |
                        info.ftLastWriteTime.dwLowDateTime;
    result.file_id = (static_cast<std::uint64_t>(info.nFileIndexHigh) << 32U) | info.nFileIndexLow;
    result.volume_id = info.dwVolumeSerialNumber;
    result.valid = true;
  }
  CloseHandle(file);
  return result;
}

LineEnding DetectLineEnding(const std::wstring& text) {
  bool lf = false;
  bool crlf = false;
  for (std::size_t i = 0; i < text.size(); ++i) {
    if (text[i] != L'\n') continue;
    if (i != 0 && text[i - 1] == L'\r') crlf = true;
    else lf = true;
  }
  if (lf && crlf) return LineEnding::Mixed;
  if (crlf) return LineEnding::CrLf;
  if (lf) return LineEnding::Lf;
  return LineEnding::None;
}

bool WriteAll(HANDLE file, std::span<const std::byte> bytes, std::wstring& error) {
  std::size_t offset = 0;
  while (offset < bytes.size()) {
    const auto remaining = std::min<std::size_t>(bytes.size() - offset, 1U << 20U);
    DWORD written = 0;
    if (!WriteFile(file, bytes.data() + offset, static_cast<DWORD>(remaining), &written, nullptr) ||
        written == 0) {
      error = WindowsError(L"一時ファイルへ書き込めません");
      return false;
    }
    offset += written;
  }
  return true;
}

bool SafeReplace(const std::filesystem::path& target, std::span<const std::byte> bytes,
                 std::wstring& error) {
  static std::atomic_uint64_t counter{};
  std::filesystem::path temporary = target;
  temporary += L".mdlite-save-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
               std::to_wstring(++counter) + L".tmp";

  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                            FILE_ATTRIBUTE_TEMPORARY, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    error = WindowsError(L"保存用一時ファイルを作成できません");
    return false;
  }
  bool ok = WriteAll(file, bytes, error);
  if (ok && !FlushFileBuffers(file)) {
    error = WindowsError(L"保存内容をディスクへ反映できません");
    ok = false;
  }
  CloseHandle(file);
  if (!ok) {
    DeleteFileW(temporary.c_str());
    return false;
  }

  const DWORD attributes = GetFileAttributesW(target.c_str());
  if (attributes != INVALID_FILE_ATTRIBUTES) {
    if (!ReplaceFileW(target.c_str(), temporary.c_str(), nullptr, REPLACEFILE_IGNORE_MERGE_ERRORS,
                      nullptr, nullptr)) {
      error = WindowsError(L"元ファイルを安全に置換できません");
      DeleteFileW(temporary.c_str());
      return false;
    }
  } else if (!MoveFileExW(temporary.c_str(), target.c_str(), MOVEFILE_WRITE_THROUGH)) {
    error = WindowsError(L"新しいファイルを保存できません");
    DeleteFileW(temporary.c_str());
    return false;
  }
  return true;
}

}  // namespace

bool Document::Load(const std::filesystem::path& path, std::wstring& error) {
  std::vector<std::byte> bytes;
  if (!ReadBytes(path, bytes, error)) return false;

  std::span<const std::byte> payload(bytes);
  TextEncoding detected = TextEncoding::Utf8;
  if (bytes.size() >= 3 && bytes[0] == std::byte{0xEF} && bytes[1] == std::byte{0xBB} &&
      bytes[2] == std::byte{0xBF}) {
    detected = TextEncoding::Utf8Bom;
    payload = payload.subspan(3);
  }

  std::wstring decoded;
  if (!Decode(payload, CP_UTF8, MB_ERR_INVALID_CHARS, decoded)) {
    detected = TextEncoding::Cp932;
    if (!Decode(payload, 932U, MB_ERR_INVALID_CHARS, decoded)) {
      error = L"UTF-8またはCP932として安全に読み込めません。文字コードを確認してください。";
      return false;
    }
  }

  const FileFingerprint fingerprint = Fingerprint(path);
  if (!fingerprint.valid) {
    error = L"読み込み後のファイル状態を確認できません。";
    return false;
  }
  path_ = std::filesystem::absolute(path).lexically_normal();
  text_ = std::move(decoded);
  encoding_ = detected;
  line_ending_ = DetectLineEnding(text_);
  disk_fingerprint_ = fingerprint;
  revision_ = 1;
  saved_revision_ = 1;
  return true;
}

bool Document::Save(std::wstring& error) {
  if (!dirty()) return true;
  const FileFingerprint current = Fingerprint(path_);
  if (!current.valid || !(current == disk_fingerprint_)) {
    error = L"ファイルが外部で変更または削除されています。編集内容は上書きしていません。";
    return false;
  }
  std::vector<std::byte> bytes;
  if (!Encode(text_, encoding_, bytes, error)) return false;
  if (!SafeReplace(path_, bytes, error)) return false;
  disk_fingerprint_ = Fingerprint(path_);
  if (!disk_fingerprint_.valid) {
    error = L"保存後のファイル状態を確認できません。";
    return false;
  }
  saved_revision_ = revision_;
  return true;
}

void Document::SetText(std::wstring text) { text_ = std::move(text); }

void Document::MarkEdited(std::wstring text) {
  if (text == text_) return;
  text_ = std::move(text);
  ++revision_;
}

void Document::MarkEditedFromEditor(std::wstring text) {
  if (line_ending_ != LineEnding::Mixed) {
    MarkEdited(NormalizeEditorLineEndings(std::move(text), line_ending_));
    return;
  }

  std::vector<bool> original_endings;
  for (std::size_t index = 0; index < text_.size(); ++index) {
    if (text_[index] != L'\n') continue;
    original_endings.push_back(index != 0 && text_[index - 1] == L'\r');
  }
  std::wstring restored;
  restored.reserve(text.size());
  std::size_t line = 0;
  for (std::size_t index = 0; index < text.size(); ++index) {
    if (text[index] == L'\r' && index + 1 < text.size() && text[index + 1] == L'\n') {
      const bool use_crlf = line < original_endings.size() ? original_endings[line] : true;
      if (use_crlf) restored.push_back(L'\r');
      restored.push_back(L'\n');
      ++index;
      ++line;
    } else {
      restored.push_back(text[index]);
    }
  }
  MarkEdited(std::move(restored));
}

bool Document::HasExternalChange() const {
  const FileFingerprint current = Fingerprint(path_);
  return !current.valid || !(current == disk_fingerprint_);
}

std::wstring EncodingLabel(TextEncoding encoding) {
  switch (encoding) {
    case TextEncoding::Utf8: return L"UTF-8";
    case TextEncoding::Utf8Bom: return L"UTF-8 BOM";
    case TextEncoding::Cp932: return L"CP932";
  }
  return L"不明";
}

std::wstring LineEndingLabel(LineEnding ending) {
  switch (ending) {
    case LineEnding::None: return L"改行なし";
    case LineEnding::Lf: return L"LF";
    case LineEnding::CrLf: return L"CRLF";
    case LineEnding::Mixed: return L"混在";
  }
  return L"不明";
}

std::wstring NormalizeEditorLineEndings(std::wstring text, LineEnding ending) {
  if (ending == LineEnding::CrLf || ending == LineEnding::None || ending == LineEnding::Mixed) {
    return text;
  }
  std::wstring normalized;
  normalized.reserve(text.size());
  for (std::size_t index = 0; index < text.size(); ++index) {
    if (text[index] == L'\r' && index + 1 < text.size() && text[index + 1] == L'\n') continue;
    normalized.push_back(text[index]);
  }
  return normalized;
}

}  // namespace mdlite
