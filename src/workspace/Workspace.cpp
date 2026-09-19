#include "workspace/Workspace.h"

#include <windows.h>

#include <array>
#include <algorithm>
#include <cwctype>
#include <fstream>
#include <iomanip>
#include <sstream>

namespace mdlite {
namespace {

bool WriteNewFile(const std::filesystem::path& path, std::string_view content, std::wstring& error) {
  HANDLE file = CreateFileW(path.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_NEW,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    if (GetLastError() == ERROR_FILE_EXISTS) return true;
    error = L"初期設定ファイルを作成できません: " + path.wstring();
    return false;
  }
  DWORD written{};
  const bool ok = content.size() <= MAXDWORD &&
                  WriteFile(file, content.data(), static_cast<DWORD>(content.size()), &written, nullptr) &&
                  written == content.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok) {
    DeleteFileW(path.c_str());
    error = L"初期設定ファイルを書き込めません: " + path.wstring();
  }
  return ok;
}

bool EncodeUtf8(std::wstring_view text, std::string& result) {
  if (text.empty()) {
    result.clear();
    return true;
  }
  const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                       static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
  if (size <= 0) return false;
  result.resize(size);
  return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                             static_cast<int>(text.size()), result.data(), size, nullptr, nullptr) == size;
}

bool AtomicWriteUtf8(const std::filesystem::path& path, std::wstring_view text, std::wstring& error) {
  std::string bytes;
  if (!EncodeUtf8(text, bytes)) {
    error = L"UTF-8へ変換できない文字があります。";
    return false;
  }
  std::filesystem::path temporary = path;
  temporary += L".new";
  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS,
                            FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) {
    error = L"状態ファイルを作成できません: " + temporary.wstring();
    return false;
  }
  DWORD written{};
  const bool ok = bytes.size() <= MAXDWORD &&
                  WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    DeleteFileW(temporary.c_str());
    error = L"状態ファイルを安全に保存できません: " + path.wstring();
    return false;
  }
  return true;
}

std::uint64_t HashPath(const std::filesystem::path& path) {
  constexpr std::uint64_t offset = 14695981039346656037ULL;
  constexpr std::uint64_t prime = 1099511628211ULL;
  std::uint64_t hash = offset;
  const std::wstring normalized = std::filesystem::absolute(path).lexically_normal().wstring();
  for (wchar_t character : normalized) {
    hash ^= static_cast<std::uint16_t>(towlower(character));
    hash *= prime;
  }
  return hash;
}

}  // namespace

WorkspaceStore::WorkspaceStore(std::filesystem::path root)
    : root_(std::filesystem::absolute(std::move(root)).lexically_normal()) {}

bool WorkspaceStore::Initialize(std::wstring& error) const {
  std::error_code filesystem_error;
  if (!std::filesystem::is_directory(root_, filesystem_error)) {
    error = L"Workspaceフォルダーが存在しません。";
    return false;
  }
  const auto metadata = metadata_root();
  for (const auto& directory : {metadata, metadata / L"templates", metadata / L"themes",
                                metadata / L".cache", state_root(), state_root() / L"recovery",
                                state_root() / L"replace"}) {
    std::filesystem::create_directories(directory, filesystem_error);
    if (filesystem_error) {
      error = L"Workspace状態フォルダーを作成できません: " + directory.wstring();
      return false;
    }
  }

  static constexpr std::array<std::pair<const wchar_t*, std::string_view>, 10> files{{
      {L".gitignore", "/.cache/\n/.state/\n"},
      {L"workspace.toml", "schema_version = 1\n\n[creation]\ndefault_profile = \"memo\"\ndaily_profile = \"daily\"\n\n[calendar]\nweek_start = \"sunday\"\nholiday_region = \"JP\"\n"},
      {L"profiles.toml", "schema_version = 1\n\n[[profiles]]\nid = \"daily\"\nname = \"デイリーノート\"\ndirectory = \"Dairy/{{date:yyyy}}/{{date:yyyyMM}}\"\nfilename = \"{{date:yyyyMMdd}}.md\"\ntemplate = \"templates/daily.md\"\ncollision = \"open-existing\"\n\n[[profiles]]\nid = \"meeting\"\nname = \"Meeting\"\ndirectory = \"Meeting/{{date:yyyy}}/{{date:yyyyMM}}\"\nfilename = \"{{date:yyyyMMdd}}.md\"\ntemplate = \"templates/meeting.md\"\ncollision = \"sequence\"\n\n[[profiles]]\nid = \"memo\"\nname = \"Memo\"\ndirectory = \"Memo/{{date:yyyy}}/{{date:yyyyMM}}/{{date:yyyyMMdd}}\"\nfilename = \"{{date:yyyyMMdd}}.md\"\ntemplate = \"templates/memo.md\"\ncollision = \"sequence\"\nsequence_format = \"_%02d\"\n"},
      {L"keybindings.toml", "schema_version = 1\n"},
      {L"commands.toml", "schema_version = 1\n"},
      {L"settings.toml", "schema_version = 1\n# Workspace overrides: theme, font_face, font_size_pt, bind.<command>\n"},
      {L"storage.toml", "schema_version = 1\n# executable = \"C:/path/to/adapter.exe\"\n# argument = \"upload\"\n# argument = \"{file}\"\n# argument = \"{revision}\"\n# The adapter prints one https URL on stdout and reads credentials from its own protected store.\n"},
      {L"templates\\daily.md", "# {{date:yyyy-MM-dd}}\n\n{{cursor}}\n"},
      {L"templates\\meeting.md", "# Meeting {{date:yyyy-MM-dd}}\n\n## 参加者\n\n## 議題\n\n{{cursor}}\n"},
      {L"templates\\memo.md", "# Memo\n\n{{cursor}}\n"},
  }};
  for (const auto& [relative, content] : files) {
    if (!WriteNewFile(metadata / relative, content, error)) return false;
  }
  return true;
}

std::filesystem::path WorkspaceStore::RecoveryPath(const std::filesystem::path& document_path) const {
  std::wostringstream name;
  name << std::hex << std::setw(16) << std::setfill(L'0') << HashPath(document_path) << L".md";
  return state_root() / L"recovery" / name.str();
}

bool WorkspaceStore::WriteRecovery(const std::filesystem::path& document_path, const std::wstring& text,
                                   std::wstring& error) const {
  const auto path = RecoveryPath(document_path);
  std::wstring payload = L"<!-- MDLite recovery\nsource: " +
                         std::filesystem::absolute(document_path).lexically_normal().wstring() +
                         L"\n-->\n" + text;
  return AtomicWriteUtf8(path, payload, error);
}

bool WorkspaceStore::RemoveRecovery(const std::filesystem::path& document_path, std::wstring& error) const {
  std::error_code filesystem_error;
  std::filesystem::remove(RecoveryPath(document_path), filesystem_error);
  if (filesystem_error) {
    error = L"復旧ファイルを削除できません。";
    return false;
  }
  return true;
}

std::vector<std::filesystem::path> WorkspaceStore::RecoveryFiles() const {
  std::vector<std::filesystem::path> result;
  std::error_code error;
  const auto directory = state_root() / L"recovery";
  for (std::filesystem::directory_iterator iterator(directory,
           std::filesystem::directory_options::skip_permission_denied, error), end;
       iterator != end && !error; iterator.increment(error)) {
    if (iterator->is_regular_file() && iterator->path().extension() == L".md") result.push_back(iterator->path());
  }
  return result;
}

bool WorkspaceStore::WriteSession(const std::vector<std::filesystem::path>& open_documents,
                                  std::wstring& error) const {
  SessionState state;
  for (const auto& path : open_documents) state.documents.push_back({path});
  return WriteSessionState(state, error);
}

bool WorkspaceStore::WriteSessionState(const SessionState& state, std::wstring& error) const {
  std::wstring session = L"schema_version = 2\nactive_index = " + std::to_wstring(state.active_index) +
                         L"\nmain_x = " + std::to_wstring(state.main_x) +
                         L"\nmain_y = " + std::to_wstring(state.main_y) +
                         L"\nmain_width = " + std::to_wstring(state.main_width) +
                         L"\nmain_height = " + std::to_wstring(state.main_height) + L"\n";
  for (const auto& document : state.documents) {
    std::error_code relative_error;
    const auto relative = std::filesystem::relative(document.path, root_, relative_error);
    if (relative_error || relative.empty() || relative.native().starts_with(L"..")) continue;
    session += L"\n[[document]]\npath = \"" + relative.generic_wstring() + L"\"\n" +
               L"selection_begin = " + std::to_wstring(document.selection_begin) + L"\n" +
               L"selection_end = " + std::to_wstring(document.selection_end) + L"\n" +
               L"first_visible_line = " + std::to_wstring(document.first_visible_line) + L"\n" +
               L"compact = " + std::wstring(document.compact ? L"true" : L"false") + L"\n" +
               L"x = " + std::to_wstring(document.x) + L"\n" +
               L"y = " + std::to_wstring(document.y) + L"\n" +
               L"width = " + std::to_wstring(document.width) + L"\n" +
               L"height = " + std::to_wstring(document.height) + L"\n";
  }
  return AtomicWriteUtf8(state_root() / L"session.toml", session, error);
}

bool WorkspaceStore::ReadSession(std::vector<std::filesystem::path>& open_documents,
                                 std::wstring& error) const {
  SessionState state;
  if (!ReadSessionState(state, error)) return false;
  open_documents.clear();
  for (const auto& document : state.documents) open_documents.push_back(document.path);
  return true;
}

bool WorkspaceStore::ReadSessionState(SessionState& state, std::wstring& error) const {
  state = {};
  const auto path = state_root() / L"session.toml";
  std::ifstream input(path, std::ios::binary);
  if (!input) return true;
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring text;
  if (!bytes.empty()) {
    const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                         static_cast<int>(bytes.size()), nullptr, 0);
    if (size <= 0) {
      error = L"セッション状態が正しいUTF-8ではありません。";
      return false;
    }
    text.resize(static_cast<std::size_t>(size));
    MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), static_cast<int>(bytes.size()),
                        text.data(), size);
  }
  std::wistringstream lines(text);
  std::wstring line;
  SessionDocument* current{};
  const auto number = [](std::wstring_view value, long long fallback = 0) {
    try { return std::stoll(std::wstring(value)); } catch (const std::exception&) { return fallback; }
  };
  while (std::getline(lines, line)) {
    if (line == L"[[document]]") { state.documents.emplace_back(); current = &state.documents.back(); continue; }
    const auto equals = line.find(L'=');
    if (equals == std::wstring::npos) continue;
    std::wstring key = line.substr(0, equals);
    std::wstring value = line.substr(equals + 1);
    while (!key.empty() && iswspace(key.back())) key.pop_back();
    while (!value.empty() && iswspace(value.front())) value.erase(value.begin());
    if (key == L"open") {
      state.documents.emplace_back();
      current = &state.documents.back();
      key = L"path";
    }
    if (!current) {
      if (key == L"active_index") state.active_index = static_cast<std::size_t>(std::max(0LL, number(value)));
      else if (key == L"main_x") state.main_x = static_cast<int>(number(value));
      else if (key == L"main_y") state.main_y = static_cast<int>(number(value));
      else if (key == L"main_width") state.main_width = static_cast<int>(number(value, 1280));
      else if (key == L"main_height") state.main_height = static_cast<int>(number(value, 800));
      else continue;
    }
    if (key == L"path" || key == L"open") {
      if (value.size() < 2 || value.front() != L'\"' || value.back() != L'\"') continue;
      const std::filesystem::path relative(value.substr(1, value.size() - 2));
      if (relative.is_absolute()) continue;
      const auto normalized = relative.lexically_normal();
      if (normalized.empty() || normalized.native().starts_with(L"..")) continue;
      const auto candidate = (root_ / normalized).lexically_normal();
      std::error_code filesystem_error;
      if (std::filesystem::is_regular_file(candidate, filesystem_error)) current->path = candidate;
    } else if (key == L"selection_begin") current->selection_begin = static_cast<std::size_t>(std::max(0LL, number(value)));
    else if (key == L"selection_end") current->selection_end = static_cast<std::size_t>(std::max(0LL, number(value)));
    else if (key == L"first_visible_line") current->first_visible_line = static_cast<int>(std::max(0LL, number(value)));
    else if (key == L"compact") current->compact = value == L"true";
    else if (key == L"x") current->x = static_cast<int>(number(value));
    else if (key == L"y") current->y = static_cast<int>(number(value));
    else if (key == L"width") current->width = static_cast<int>(number(value, 720));
    else if (key == L"height") current->height = static_cast<int>(number(value, 520));
  }
  std::erase_if(state.documents, [](const SessionDocument& item) { return item.path.empty(); });
  state.active_index = state.documents.empty() ? 0 : std::min(state.active_index, state.documents.size() - 1);
  return true;
}

}  // namespace mdlite
