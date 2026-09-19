#include "profiles/Profiles.h"

#include <windows.h>

#include <fstream>
#include <limits>
#include <sstream>

namespace mdlite {
namespace {

std::wstring Number(unsigned value, int width) {
  std::wostringstream stream;
  stream.width(width);
  stream.fill(L'0');
  stream << value;
  return stream.str();
}

std::wstring ExpandDate(std::wstring value, const SYSTEMTIME& date) {
  const std::pair<std::wstring_view, std::wstring> replacements[] = {
      {L"{{date:yyyyMMdd}}", Number(date.wYear, 4) + Number(date.wMonth, 2) + Number(date.wDay, 2)},
      {L"{{date:yyyyMM}}", Number(date.wYear, 4) + Number(date.wMonth, 2)},
      {L"{{date:yyyy-MM-dd}}", Number(date.wYear, 4) + L"-" + Number(date.wMonth, 2) + L"-" + Number(date.wDay, 2)},
      {L"{{date:yyyy}}", Number(date.wYear, 4)},
  };
  for (const auto& [token, replacement] : replacements) {
    std::size_t position{};
    while ((position = value.find(token, position)) != std::wstring::npos) {
      value.replace(position, token.size(), replacement);
      position += replacement.size();
    }
  }
  return value;
}

bool ReadUtf8(const std::filesystem::path& path, std::wstring& text, std::wstring& error) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) {
    error = L"テンプレートを開けません: " + path.wstring();
    return false;
  }
  const auto size = input.tellg();
  if (size < 0 || size > std::numeric_limits<int>::max()) return false;
  std::string bytes(static_cast<std::size_t>(size), '\0');
  input.seekg(0);
  if (!bytes.empty() && !input.read(bytes.data(), size)) return false;
  const int required = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                           static_cast<int>(bytes.size()), nullptr, 0);
  if (required < 0 || (!bytes.empty() && required == 0)) {
    error = L"テンプレートはUTF-8で保存してください。";
    return false;
  }
  text.resize(required);
  if (required != 0) MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                         static_cast<int>(bytes.size()), text.data(), required);
  return true;
}

bool EncodeUtf8(std::wstring_view text, std::string& bytes) {
  const int required = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                           static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
  if (required < 0 || (!text.empty() && required == 0)) return false;
  bytes.resize(required);
  return required == 0 || WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                               static_cast<int>(text.size()), bytes.data(), required,
                                               nullptr, nullptr) == required;
}

bool CreateNewUtf8(const std::filesystem::path& path, std::wstring_view text, bool& already_exists,
                   std::wstring& error) {
  std::string bytes;
  if (!EncodeUtf8(text, bytes)) {
    error = L"ノート本文をUTF-8へ変換できません。";
    return false;
  }
  HANDLE file = CreateFileW(path.c_str(), GENERIC_WRITE, FILE_SHARE_READ, nullptr, CREATE_NEW,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    already_exists = GetLastError() == ERROR_FILE_EXISTS || GetLastError() == ERROR_ALREADY_EXISTS;
    if (!already_exists) error = L"ノートを作成できません: " + path.wstring();
    return false;
  }
  DWORD written{};
  const bool ok = bytes.size() <= MAXDWORD &&
                  WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok) {
    DeleteFileW(path.c_str());
    error = L"ノート本文を書き込めません: " + path.wstring();
  }
  already_exists = false;
  return ok;
}

}  // namespace

bool CreateProfileNote(const std::filesystem::path& workspace, BuiltInProfile profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error) {
  std::wstring directory_pattern;
  std::wstring filename_pattern = L"{{date:yyyyMMdd}}.md";
  std::filesystem::path template_path;
  bool sequence = false;
  switch (profile) {
    case BuiltInProfile::Daily:
      directory_pattern = L"Dairy/{{date:yyyy}}/{{date:yyyyMM}}";
      template_path = workspace / L".mdlite/templates/daily.md";
      break;
    case BuiltInProfile::Meeting:
      directory_pattern = L"Meeting/{{date:yyyy}}/{{date:yyyyMM}}";
      template_path = workspace / L".mdlite/templates/meeting.md";
      sequence = true;
      break;
    case BuiltInProfile::Memo:
      directory_pattern = L"Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}";
      template_path = workspace / L".mdlite/templates/memo.md";
      sequence = true;
      break;
  }
  const auto directory = workspace / ExpandDate(directory_pattern, local_date);
  std::error_code filesystem_error;
  std::filesystem::create_directories(directory, filesystem_error);
  if (filesystem_error) {
    error = L"ノート保存先を作成できません: " + directory.wstring();
    return false;
  }
  std::wstring body;
  if (!ReadUtf8(template_path, body, error)) return false;
  body = ExpandDate(std::move(body), local_date);
  const std::wstring marker = L"{{cursor}}";
  result.cursor = body.find(marker);
  if (result.cursor == std::wstring::npos) result.cursor = body.size();
  else body.erase(result.cursor, marker.size());
  if (body.find(marker) != std::wstring::npos) {
    error = L"テンプレートの{{cursor}}は一つだけ指定できます。";
    return false;
  }

  const std::wstring base_filename = ExpandDate(filename_pattern, local_date);
  for (unsigned number = 0; number < 10000; ++number) {
    std::wstring filename = base_filename;
    if (sequence && number != 0) {
      const auto extension = std::filesystem::path(filename).extension().wstring();
      filename.resize(filename.size() - extension.size());
      filename += L"_" + Number(number, 2) + extension;
    }
    result.path = directory / filename;
    bool exists{};
    if (CreateNewUtf8(result.path, body, exists, error)) {
      result.created = true;
      return true;
    }
    if (!exists) return false;
    if (!sequence) {
      result.created = false;
      return true;
    }
  }
  error = L"空いている連番を確保できません。";
  return false;
}

}  // namespace mdlite
