#include "profiles/Profiles.h"

#include <windows.h>
#include <shlobj.h>

#include <algorithm>
#include <cwctype>
#include <fstream>
#include <limits>
#include <map>
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
      {L"{{date:yyyyMMddHHmm}}", Number(date.wYear, 4) + Number(date.wMonth, 2) + Number(date.wDay, 2) +
                                  Number(date.wHour, 2) + Number(date.wMinute, 2)},
      {L"{{date:yyyyMMdd}}", Number(date.wYear, 4) + Number(date.wMonth, 2) + Number(date.wDay, 2)},
      {L"{{date:yyyyMM}}", Number(date.wYear, 4) + Number(date.wMonth, 2)},
      {L"{{date:yyyy-MM-dd HH-mm}}", Number(date.wYear, 4) + L"-" + Number(date.wMonth, 2) + L"-" +
                                     Number(date.wDay, 2) + L" " + Number(date.wHour, 2) + L"-" +
                                     Number(date.wMinute, 2)},
      {L"{{date:yyyy-MM-dd}}", Number(date.wYear, 4) + L"-" + Number(date.wMonth, 2) + L"-" + Number(date.wDay, 2)},
      {L"{{date:yyyy}}", Number(date.wYear, 4)},
      {L"{{date:MM}}", Number(date.wMonth, 2)},
      {L"{{date:dd}}", Number(date.wDay, 2)},
      {L"{{date:HH}}", Number(date.wHour, 2)},
      {L"{{date:mm}}", Number(date.wMinute, 2)},
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

std::wstring Trim(std::wstring value) {
  while (!value.empty() && iswspace(value.front())) value.erase(value.begin());
  while (!value.empty() && iswspace(value.back())) value.pop_back();
  return value;
}

std::optional<std::wstring> Unquote(std::wstring value) {
  value = Trim(std::move(value));
  if (value.size() < 2 || value.front() != L'"' || value.back() != L'"') return std::nullopt;
  return value.substr(1, value.size() - 2);
}

bool IsSafeString(std::wstring_view value) {
  return value.find_first_of(L"\"\r\n") == std::wstring_view::npos;
}

bool HasUnsafePathPart(const std::filesystem::path& path) {
  if (path.empty() || path.is_absolute() || path.has_root_path()) return true;
  return std::ranges::any_of(path, [](const auto& part) { return part == L".." || part == L"."; });
}

bool HasInput(const ProfileDefinition& profile, std::wstring_view id) {
  return std::ranges::any_of(profile.inputs, [&](const auto& input) { return input.id == id; });
}

bool HasUnknownToken(std::wstring_view value, const ProfileDefinition& profile, bool allow_cursor = false) {
  std::size_t cursor{};
  while ((cursor = value.find(L"{{", cursor)) != std::wstring_view::npos) {
    const auto end = value.find(L"}}", cursor + 2);
    if (end == std::wstring_view::npos) return true;
    const auto token = value.substr(cursor, end + 2 - cursor);
    const bool known_date = token == L"{{date:yyyy}}" || token == L"{{date:MM}}" ||
                            token == L"{{date:dd}}" || token == L"{{date:HH}}" ||
                            token == L"{{date:mm}}" || token == L"{{date:yyyyMM}}" ||
                            token == L"{{date:yyyyMMdd}}" || token == L"{{date:yyyyMMddHHmm}}" ||
                            token == L"{{date:yyyy-MM-dd}}" || token == L"{{date:yyyy-MM-dd HH-mm}}";
    bool known_input = token == L"{{title}}" && HasInput(profile, L"title");
    constexpr std::wstring_view prefix = L"{{input:";
    if (token.starts_with(prefix) && token.size() > prefix.size() + 2) {
      known_input = HasInput(profile, token.substr(prefix.size(), token.size() - prefix.size() - 2));
    }
    if (!known_date && !known_input && !(allow_cursor && token == L"{{cursor}}")) return true;
    cursor = end + 2;
  }
  return false;
}

bool ExpandInputs(std::wstring& value, const ProfileDefinition& profile, const ProfileValues& values,
                  std::wstring& error) {
  for (const auto& input : profile.inputs) {
    const auto found = values.find(input.id);
    const std::wstring& replacement = found == values.end() ? input.default_value : found->second;
    if (input.required && replacement.empty()) {
      error = L"必須入力が未指定です: " + input.label;
      return false;
    }
    const std::wstring tokens[] = {L"{{input:" + input.id + L"}}",
                                   input.id == L"title" ? L"{{title}}" : L""};
    for (const auto& token : tokens) {
      if (token.empty()) continue;
      std::size_t position{};
      while ((position = value.find(token, position)) != std::wstring::npos) {
        value.replace(position, token.size(), replacement);
        position += replacement.size();
      }
    }
  }
  return true;
}

bool EncodeUtf8(std::wstring_view text, std::string& bytes);

bool WriteUtf8Atomic(const std::filesystem::path& path, std::wstring_view text, std::wstring& error) {
  std::string bytes;
  if (!EncodeUtf8(text, bytes)) { error = L"プロファイル設定をUTF-8へ変換できません。"; return false; }
  std::error_code filesystem_error;
  std::filesystem::create_directories(path.parent_path(), filesystem_error);
  if (filesystem_error) { error = L"プロファイル設定フォルダーを作成できません。"; return false; }
  auto temporary = path;
  temporary += L".new";
  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) { error = L"プロファイル設定一時ファイルを作成できません。"; return false; }
  DWORD written{};
  const bool ok = bytes.size() <= MAXDWORD &&
                  WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    DeleteFileW(temporary.c_str());
    error = L"プロファイル設定を安全に保存できません。";
    return false;
  }
  return true;
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

std::vector<ProfileDefinition> DefaultProfiles() {
  return {
      {L"daily", L"デイリーノート", L"Dairy/{{date:yyyy}}/{{date:yyyyMM}}",
       L"{{date:yyyyMMdd}}.md", L"templates/daily.md", ProfileCollision::OpenExisting},
      {L"meeting", L"Meeting", L"Meeting/{{date:yyyy}}/{{date:yyyyMM}}",
       L"{{date:yyyyMMdd}}.md", L"templates/meeting.md", ProfileCollision::Sequence},
      {L"memo", L"Memo", L"Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}",
       L"{{date:yyyyMMdd}}.md", L"templates/memo.md", ProfileCollision::Sequence},
  };
}

bool ValidateProfile(const ProfileDefinition& profile, std::wstring& error) {
  const bool valid_id = !profile.id.empty() && std::ranges::all_of(profile.id, [](wchar_t character) {
    return iswalnum(character) || character == L'.' || character == L'_' || character == L'-';
  });
  if (!valid_id || profile.name.empty() || !IsSafeString(profile.id) || !IsSafeString(profile.name)) {
    error = L"profile idまたはnameが不正です。";
    return false;
  }
  std::map<std::wstring, bool> input_ids;
  for (const auto& input : profile.inputs) {
    const bool valid_input_id = !input.id.empty() && std::ranges::all_of(input.id, [](wchar_t character) {
      return iswalnum(character) || character == L'_' || character == L'-';
    });
    if (!valid_input_id || input.label.empty() || !IsSafeString(input.id) || !IsSafeString(input.label) ||
        !IsSafeString(input.default_value) || !input_ids.emplace(input.id, true).second) {
      error = L"profile inputのid、label、defaultまたは重複が不正です。";
      return false;
    }
  }
  if (HasUnsafePathPart(profile.directory) || HasUnsafePathPart(profile.template_path) ||
      profile.filename.empty() || profile.filename.filename() != profile.filename ||
      !IsSafeString(profile.directory.generic_wstring()) || !IsSafeString(profile.filename.generic_wstring()) ||
      !IsSafeString(profile.template_path.generic_wstring()) ||
      HasUnknownToken(profile.directory.generic_wstring(), profile) ||
      HasUnknownToken(profile.filename.generic_wstring(), profile)) {
    error = L"profile pathは既知の日付tokenを使うWorkspace内の相対pathで指定してください。";
    return false;
  }
  return true;
}

bool LoadProfileFile(const std::filesystem::path& path, std::vector<ProfileDefinition>& profiles,
                     std::wstring& error) {
  profiles.clear();
  if (path.empty() || !std::filesystem::exists(path)) return true;
  std::wstring text;
  if (!ReadUtf8(path, text, error)) return false;
  std::wistringstream lines(text);
  std::wstring line;
  ProfileDefinition* current{};
  ProfileInputDefinition* current_input{};
  bool schema_seen{};
  while (std::getline(lines, line)) {
    line = Trim(std::move(line));
    if (line.empty() || line.front() == L'#') continue;
    if (line == L"[[profiles]]") {
      profiles.emplace_back();
      current = &profiles.back();
      current_input = nullptr;
      continue;
    }
    if (line == L"[[profiles.inputs]]") {
      if (!current) { error = L"profile inputは[[profiles]]の後に指定してください。"; return false; }
      current->inputs.emplace_back();
      current_input = &current->inputs.back();
      continue;
    }
    const auto equals = line.find(L'=');
    if (equals == std::wstring::npos) { error = L"profiles.tomlの行形式が不正です。"; return false; }
    const auto key = Trim(line.substr(0, equals));
    const auto raw = Trim(line.substr(equals + 1));
    if (!current && key == L"schema_version") {
      if (raw != L"1") { error = L"未対応のprofiles schemaです。"; return false; }
      schema_seen = true;
      continue;
    }
    if (!current) { error = L"profile fieldは[[profiles]]の後に指定してください。"; return false; }
    if (current_input) {
      if (key == L"required") {
        if (raw == L"true") current_input->required = true;
        else if (raw == L"false") current_input->required = false;
        else { error = L"profile input requiredはtrueまたはfalseです。"; return false; }
        continue;
      }
      const auto input_value = Unquote(raw);
      if (!input_value) { error = L"profile input fieldは引用符で囲んでください。"; return false; }
      if (key == L"id") current_input->id = *input_value;
      else if (key == L"label") current_input->label = *input_value;
      else if (key == L"default") current_input->default_value = *input_value;
      else { error = L"未対応のprofile input fieldです: " + key; return false; }
      continue;
    }
    const auto value = Unquote(raw);
    if (!value) { error = L"profile fieldは引用符で囲んでください。"; return false; }
    if (key == L"id") current->id = *value;
    else if (key == L"name") current->name = *value;
    else if (key == L"directory") current->directory = *value;
    else if (key == L"filename") current->filename = *value;
    else if (key == L"template") current->template_path = *value;
    else if (key == L"collision") {
      if (*value == L"open-existing") current->collision = ProfileCollision::OpenExisting;
      else if (*value == L"sequence") current->collision = ProfileCollision::Sequence;
      else { error = L"collisionはopen-existingまたはsequenceです。"; return false; }
    } else if (key == L"sequence_format") {
      if (*value != L"_%02d") {
        error = L"sequence_formatは_%02dのみ対応します。";
        return false;
      }
    } else {
      error = L"未対応のprofile fieldです: " + key;
      return false;
    }
  }
  if (!schema_seen) { error = L"profiles.tomlにschema_versionがありません。"; return false; }
  std::map<std::wstring, bool> ids;
  for (const auto& profile : profiles) {
    if (!ValidateProfile(profile, error)) return false;
    if (!ids.emplace(profile.id, true).second) { error = L"profile idが重複しています: " + profile.id; return false; }
  }
  return true;
}

bool SaveProfileFile(const std::filesystem::path& path, const std::vector<ProfileDefinition>& profiles,
                     std::wstring& error) {
  std::map<std::wstring, bool> ids;
  std::wstring text = L"schema_version = 1\n";
  for (const auto& profile : profiles) {
    if (!ValidateProfile(profile, error)) return false;
    if (!ids.emplace(profile.id, true).second) { error = L"profile idが重複しています: " + profile.id; return false; }
    text += L"\n[[profiles]]\n";
    text += L"id = \"" + profile.id + L"\"\n";
    text += L"name = \"" + profile.name + L"\"\n";
    text += L"directory = \"" + profile.directory.generic_wstring() + L"\"\n";
    text += L"filename = \"" + profile.filename.generic_wstring() + L"\"\n";
    text += L"template = \"" + profile.template_path.generic_wstring() + L"\"\n";
    text += L"collision = \"" + std::wstring(profile.collision == ProfileCollision::Sequence
                                                   ? L"sequence" : L"open-existing") + L"\"\n";
    if (profile.collision == ProfileCollision::Sequence) text += L"sequence_format = \"_%02d\"\n";
    for (const auto& input : profile.inputs) {
      text += L"\n[[profiles.inputs]]\n";
      text += L"id = \"" + input.id + L"\"\n";
      text += L"label = \"" + input.label + L"\"\n";
      text += L"default = \"" + input.default_value + L"\"\n";
      text += L"required = " + std::wstring(input.required ? L"true" : L"false") + L"\n";
    }
  }
  return WriteUtf8Atomic(path, text, error);
}

bool ResolveProfiles(const std::filesystem::path& common_path,
                     const std::filesystem::path& workspace_path,
                     std::vector<ProfileDefinition>& profiles, std::wstring& error) {
  profiles = DefaultProfiles();
  const auto overlay = [&](const std::filesystem::path& path) {
    std::vector<ProfileDefinition> layer;
    if (!LoadProfileFile(path, layer, error)) return false;
    for (auto& profile : layer) {
      const auto existing = std::ranges::find_if(profiles, [&](const auto& item) { return item.id == profile.id; });
      if (existing == profiles.end()) profiles.push_back(std::move(profile));
      else *existing = std::move(profile);
    }
    return true;
  };
  return overlay(common_path) && overlay(workspace_path);
}

std::optional<std::filesystem::path> PreviewProfilePath(const std::filesystem::path& workspace,
                                                        const ProfileDefinition& profile,
                                                        const SYSTEMTIME& local_date,
                                                        std::wstring& error) {
  return PreviewProfilePath(workspace, profile, local_date, {}, error);
}

std::optional<std::filesystem::path> PreviewProfilePath(const std::filesystem::path& workspace,
                                                        const ProfileDefinition& profile,
                                                        const SYSTEMTIME& local_date,
                                                        const ProfileValues& values,
                                                        std::wstring& error) {
  if (!ValidateProfile(profile, error)) return std::nullopt;
  auto directory = ExpandDate(profile.directory.generic_wstring(), local_date);
  auto filename = ExpandDate(profile.filename.generic_wstring(), local_date);
  if (!ExpandInputs(directory, profile, values, error) || !ExpandInputs(filename, profile, values, error)) {
    return std::nullopt;
  }
  if (std::filesystem::path(filename).filename() != std::filesystem::path(filename)) {
    error = L"展開後のfilenameにdirectory区切りを含められません。";
    return std::nullopt;
  }
  const auto relative = (std::filesystem::path(directory) / filename).lexically_normal();
  if (HasUnsafePathPart(relative) || relative.generic_wstring().find_first_of(L"<>:\"|?*") != std::wstring::npos) {
    error = L"展開後のprofile pathがWorkspace外または禁止文字を含みます。";
    return std::nullopt;
  }
  return (workspace / relative).lexically_normal();
}

std::filesystem::path CommonProfilesPath() {
  PWSTR path{};
  if (FAILED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_CREATE, nullptr, &path))) return {};
  std::filesystem::path result = std::filesystem::path(path) / L"MDLite" / L"profiles.toml";
  CoTaskMemFree(path);
  return result;
}

bool CreateProfileNote(const std::filesystem::path& workspace, BuiltInProfile profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error) {
  const std::wstring id = profile == BuiltInProfile::Daily ? L"daily" :
                          profile == BuiltInProfile::Meeting ? L"meeting" : L"memo";
  std::vector<ProfileDefinition> profiles;
  if (!ResolveProfiles(CommonProfilesPath(), workspace / L".mdlite/profiles.toml", profiles, error)) return false;
  const auto found = std::ranges::find_if(profiles, [&](const auto& item) { return item.id == id; });
  if (found == profiles.end()) { error = L"組込みprofileが見つかりません: " + id; return false; }
  return CreateProfileNote(workspace, *found, local_date, result, error);
}

bool CreateProfileNote(const std::filesystem::path& workspace, const ProfileDefinition& profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error) {
  return CreateProfileNote(workspace, profile, local_date, {}, result, error);
}

bool CreateProfileNote(const std::filesystem::path& workspace, const ProfileDefinition& profile,
                       const SYSTEMTIME& local_date, const ProfileValues& values,
                       NoteCreationResult& result, std::wstring& error) {
  const auto preview = PreviewProfilePath(workspace, profile, local_date, values, error);
  if (!preview) return false;
  const auto directory = preview->parent_path();
  std::error_code filesystem_error;
  std::filesystem::create_directories(directory, filesystem_error);
  if (filesystem_error) {
    error = L"ノート保存先を作成できません: " + directory.wstring();
    return false;
  }
  std::wstring body;
  const auto template_path = workspace / L".mdlite" / profile.template_path;
  if (!ReadUtf8(template_path, body, error)) return false;
  const std::wstring marker = L"{{cursor}}";
  if (HasUnknownToken(body, profile, true)) {
    error = L"テンプレートに未定義の変数があります。";
    return false;
  }
  const auto marker_position = body.find(marker);
  if (marker_position != std::wstring::npos && body.find(marker, marker_position + marker.size()) != std::wstring::npos) {
    error = L"テンプレートの{{cursor}}は一つだけ指定できます。";
    return false;
  }
  std::wstring prefix = marker_position == std::wstring::npos ? body : body.substr(0, marker_position);
  std::wstring suffix = marker_position == std::wstring::npos ? L"" : body.substr(marker_position + marker.size());
  prefix = ExpandDate(std::move(prefix), local_date);
  suffix = ExpandDate(std::move(suffix), local_date);
  if (!ExpandInputs(prefix, profile, values, error) || !ExpandInputs(suffix, profile, values, error)) return false;
  result.cursor = prefix.size();
  body = std::move(prefix) + std::move(suffix);

  const std::wstring base_filename = preview->filename().wstring();
  for (unsigned number = 0; number < 10000; ++number) {
    std::wstring filename = base_filename;
    if (profile.collision == ProfileCollision::Sequence && number != 0) {
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
    if (profile.collision == ProfileCollision::OpenExisting) {
      result.created = false;
      return true;
    }
  }
  error = L"空いている連番を確保できません。";
  return false;
}

}  // namespace mdlite
