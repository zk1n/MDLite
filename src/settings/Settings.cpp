#include "settings/Settings.h"

#include <windows.h>
#include <shlobj.h>

#include <algorithm>
#include <cwctype>
#include <fstream>
#include <sstream>

namespace mdlite {
namespace {

std::wstring Trim(std::wstring value) {
  while (!value.empty() && iswspace(value.front())) value.erase(value.begin());
  while (!value.empty() && iswspace(value.back())) value.pop_back();
  return value;
}
std::optional<std::wstring> Unquote(std::wstring value) {
  value = Trim(std::move(value));
  if (value.size() >= 2 && value.front() == L'"' && value.back() == L'"') return value.substr(1, value.size() - 2);
  return std::nullopt;
}
bool DecodeUtf8(const std::string& bytes, std::wstring& value) {
  if (bytes.empty()) { value.clear(); return true; }
  const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), static_cast<int>(bytes.size()), nullptr, 0);
  if (size <= 0) return false;
  value.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(), static_cast<int>(bytes.size()), value.data(), size) == size;
}
bool EncodeUtf8(std::wstring_view text, std::string& bytes) {
  if (text.empty()) { bytes.clear(); return true; }
  const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(), static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
  if (size <= 0) return false;
  bytes.resize(static_cast<std::size_t>(size));
  return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(), static_cast<int>(text.size()), bytes.data(), size, nullptr, nullptr) == size;
}
void Overlay(const SettingsLayer& layer, std::wstring_view origin, EffectiveSettings& target) {
  if (layer.auto_save) { target.auto_save = *layer.auto_save; target.origins[L"auto_save"] = origin; }
  if (layer.auto_save_delay_ms) {
    target.auto_save_delay_ms = *layer.auto_save_delay_ms;
    target.origins[L"auto_save_delay_ms"] = origin;
  }
  if (layer.theme) { target.theme = *layer.theme; target.origins[L"theme"] = origin; }
  if (layer.font_face) { target.font_face = *layer.font_face; target.origins[L"font_face"] = origin; }
  if (layer.font_size_pt) { target.font_size_pt = *layer.font_size_pt; target.origins[L"font_size_pt"] = origin; }
  for (const auto& [command, shortcut] : layer.keybindings) {
    target.keybindings[command] = shortcut;
    target.origins[L"keybinding." + command] = origin;
  }
  for (const auto& [name, color] : layer.colors) {
    target.colors[name] = color;
    target.origins[L"color." + name] = origin;
  }
}
std::wstring NormalizeShortcut(std::wstring value) {
  value = Trim(std::move(value));
  std::ranges::transform(value, value.begin(), towupper);
  return value;
}
bool IsSafeTomlString(std::wstring_view value) {
  return value.find_first_of(L"\"\r\n") == std::wstring_view::npos;
}
bool IsValidCommandName(std::wstring_view command) {
  return !command.empty() && std::ranges::all_of(command, [](wchar_t character) {
    return iswalnum(character) || character == L'.' || character == L'_' || character == L'-';
  });
}
bool IsValidColor(std::wstring_view color) {
  return color.size() == 7 && color.front() == L'#' &&
         std::ranges::all_of(color.substr(1), [](wchar_t value) { return iswxdigit(value) != 0; });
}
bool IsValidShortcut(std::wstring shortcut) {
  shortcut.erase(std::remove_if(shortcut.begin(), shortcut.end(), iswspace), shortcut.end());
  std::ranges::transform(shortcut, shortcut.begin(), towupper);
  if (shortcut.empty() || shortcut == L"NONE") return true;
  std::size_t begin{};
  unsigned modifiers{};
  std::wstring key;
  while (begin <= shortcut.size()) {
    const auto end = shortcut.find(L'+', begin);
    const auto token = shortcut.substr(begin, end == std::wstring::npos ? shortcut.size() - begin : end - begin);
    unsigned flag{};
    if (token == L"CTRL") flag = 1;
    else if (token == L"SHIFT") flag = 2;
    else if (token == L"ALT") flag = 4;
    else {
      if (!key.empty() || token.empty()) return false;
      key = token;
    }
    if (flag != 0) {
      if ((modifiers & flag) != 0 || !key.empty()) return false;
      modifiers |= flag;
    }
    if (end == std::wstring::npos) break;
    begin = end + 1;
  }
  if (key.size() == 1 && ((key[0] >= L'A' && key[0] <= L'Z') ||
                          (key[0] >= L'0' && key[0] <= L'9'))) return true;
  if (key.size() < 2 || key.front() != L'F') return false;
  try {
    std::size_t consumed{};
    const int number = std::stoi(key.substr(1), &consumed);
    return consumed == key.size() - 1 && number >= 1 && number <= 24;
  } catch (const std::exception&) { return false; }
}

}  // namespace

SettingsLayer DefaultSettingsLayer() {
  SettingsLayer layer;
  layer.auto_save = true;
  layer.auto_save_delay_ms = 750;
  layer.theme = ThemeMode::System;
  layer.font_face = L"Segoe UI";
  layer.font_size_pt = 11;
  layer.keybindings = {{L"file.open", L"Ctrl+O"}, {L"file.save", L"Ctrl+S"}, {L"file.quickOpen", L"Ctrl+P"},
                       {L"file.close", L"Ctrl+W"}, {L"edit.find", L"Ctrl+F"}, {L"edit.findNext", L"F3"},
                       {L"view.commandPalette", L"Ctrl+Shift+P"}};
  return layer;
}

std::wstring ThemeName(ThemeMode theme) {
  if (theme == ThemeMode::Light) return L"light";
  if (theme == ThemeMode::Dark) return L"dark";
  if (theme == ThemeMode::Custom) return L"custom";
  return L"system";
}
std::optional<ThemeMode> ParseTheme(std::wstring_view value) {
  std::wstring normalized(value);
  std::ranges::transform(normalized, normalized.begin(), towlower);
  if (normalized == L"system") return ThemeMode::System;
  if (normalized == L"light") return ThemeMode::Light;
  if (normalized == L"dark") return ThemeMode::Dark;
  if (normalized == L"custom") return ThemeMode::Custom;
  return std::nullopt;
}

bool ValidateKeybindingConflicts(const std::map<std::wstring, std::wstring>& bindings, std::wstring& error) {
  std::map<std::wstring, std::wstring> owners;
  for (const auto& [command, shortcut] : bindings) {
    const auto normalized = NormalizeShortcut(shortcut);
    if (normalized.empty() || normalized == L"NONE") continue;
    if (!IsValidCommandName(command) || !IsSafeTomlString(shortcut) || !IsValidShortcut(shortcut)) {
      error = L"未対応のキー割当てです: " + command + L" = " + shortcut;
      return false;
    }
    const auto [position, inserted] = owners.emplace(normalized, command);
    if (!inserted) {
      error = L"キー割当てが重複しています: " + shortcut + L" (" + position->second + L", " + command + L")";
      return false;
    }
  }
  return true;
}
bool ValidateSettingsLayer(const SettingsLayer& layer, std::wstring& error) {
  if (layer.auto_save_delay_ms && (*layer.auto_save_delay_ms < 100 || *layer.auto_save_delay_ms > 60000)) {
    error = L"自動保存待機時間は100〜60000msで指定してください。"; return false;
  }
  if (layer.font_face && (layer.font_face->empty() || layer.font_face->size() > LF_FACESIZE - 1 ||
                          !IsSafeTomlString(*layer.font_face))) {
    error = L"フォント名は1〜31文字で指定してください。"; return false;
  }
  if (layer.font_size_pt && (*layer.font_size_pt < 6 || *layer.font_size_pt > 96)) {
    error = L"フォントサイズは6〜96ptで指定してください。"; return false;
  }
  static constexpr std::wstring_view known_colors[]{L"background", L"foreground", L"link", L"heading",
                                                     L"marker", L"code_background", L"table_background"};
  for (const auto& [name, color] : layer.colors) {
    if (std::ranges::find(known_colors, name) == std::end(known_colors) || !IsValidColor(color)) {
      error = L"theme colorは既知のnameと#RRGGBBで指定してください: " + name;
      return false;
    }
  }
  return ValidateKeybindingConflicts(layer.keybindings, error);
}

bool LoadSettingsLayer(const std::filesystem::path& path, SettingsLayer& layer, std::wstring& error) {
  layer = {};
  std::ifstream input(path, std::ios::binary);
  if (!input) return true;
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring text;
  if (!DecodeUtf8(bytes, text)) { error = L"設定ファイルはUTF-8で保存してください: " + path.wstring(); return false; }
  std::wistringstream lines(text);
  std::wstring line;
  bool schema_seen{};
  while (std::getline(lines, line)) {
    line = Trim(std::move(line));
    if (line.empty()) continue;
    if (line.front() == L'#' || line.front() == L'[') {
      layer.preserved_lines.push_back(line);
      continue;
    }
    const auto equals = line.find(L'=');
    if (equals == std::wstring::npos) { error = L"設定行に=がありません。"; return false; }
    const auto key = Trim(line.substr(0, equals));
    const auto raw = Trim(line.substr(equals + 1));
    if (key == L"schema_version") {
      if (raw != L"1") { error = L"未対応のsettings schemaです。"; return false; }
      schema_seen = true;
    } else if (key == L"auto_save") {
      if (raw == L"true") layer.auto_save = true;
      else if (raw == L"false") layer.auto_save = false;
      else { error = L"auto_saveはtrueまたはfalseで指定してください。"; return false; }
    } else if (key == L"auto_save_delay_ms") {
      try {
        std::size_t consumed{};
        const auto value = std::stoul(raw, &consumed);
        if (consumed != raw.size()) throw std::invalid_argument("trailing characters");
        layer.auto_save_delay_ms = static_cast<unsigned>(value);
      } catch (const std::exception&) { error = L"auto_save_delay_msが数値ではありません。"; return false; }
    } else if (key == L"theme") {
      const auto quoted = Unquote(raw);
      const auto parsed = quoted ? ParseTheme(*quoted) : std::nullopt;
      if (!parsed) { error = L"themeはsystem/light/dark/customで指定してください。"; return false; }
      layer.theme = *parsed;
    } else if (key == L"font_face") {
      const auto quoted = Unquote(raw);
      if (!quoted) { error = L"font_faceは引用符で囲んでください。"; return false; }
      layer.font_face = *quoted;
    }
    else if (key == L"font_size_pt") {
      try {
        std::size_t consumed{};
        const auto value = std::stoul(raw, &consumed);
        if (consumed != raw.size()) throw std::invalid_argument("trailing characters");
        layer.font_size_pt = static_cast<unsigned>(value);
      }
      catch (const std::exception&) { error = L"font_size_ptが数値ではありません。"; return false; }
    } else if (key.starts_with(L"bind.")) {
      const auto quoted = Unquote(raw);
      if (!quoted) { error = L"キー割当ては引用符で囲んでください。"; return false; }
      layer.keybindings[key.substr(5)] = *quoted;
    } else if (key.starts_with(L"color.")) {
      const auto quoted = Unquote(raw);
      if (!quoted) { error = L"theme colorは引用符で囲んでください。"; return false; }
      layer.colors[key.substr(6)] = *quoted;
    } else {
      layer.preserved_lines.push_back(line);
    }
  }
  if (!schema_seen) { error = L"settings.tomlにschema_versionがありません。"; return false; }
  return ValidateSettingsLayer(layer, error);
}

bool SaveSettingsLayer(const std::filesystem::path& path, const SettingsLayer& layer, std::wstring& error) {
  if (!ValidateSettingsLayer(layer, error)) return false;
  std::wstring text = L"schema_version = 1\n";
  if (layer.auto_save) text += L"auto_save = " + std::wstring(*layer.auto_save ? L"true" : L"false") + L"\n";
  if (layer.auto_save_delay_ms) text += L"auto_save_delay_ms = " + std::to_wstring(*layer.auto_save_delay_ms) + L"\n";
  if (layer.theme) text += L"theme = \"" + ThemeName(*layer.theme) + L"\"\n";
  if (layer.font_face) text += L"font_face = \"" + *layer.font_face + L"\"\n";
  if (layer.font_size_pt) text += L"font_size_pt = " + std::to_wstring(*layer.font_size_pt) + L"\n";
  for (const auto& [command, shortcut] : layer.keybindings) text += L"bind." + command + L" = \"" + shortcut + L"\"\n";
  for (const auto& [name, color] : layer.colors) text += L"color." + name + L" = \"" + color + L"\"\n";
  if (!layer.preserved_lines.empty()) {
    text += L"\n# Preserved settings not managed by this MDLite build\n";
    for (const auto& line : layer.preserved_lines) text += line + L"\n";
  }
  std::string bytes;
  if (!EncodeUtf8(text, bytes)) { error = L"設定をUTF-8へ変換できません。"; return false; }
  std::error_code filesystem_error;
  std::filesystem::create_directories(path.parent_path(), filesystem_error);
  if (filesystem_error) { error = L"設定フォルダーを作成できません。"; return false; }
  auto temporary = path; temporary += L".new";
  HANDLE file = CreateFileW(temporary.c_str(), GENERIC_WRITE, 0, nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
  if (file == INVALID_HANDLE_VALUE) { error = L"設定一時ファイルを作成できません。"; return false; }
  DWORD written{};
  const bool ok = bytes.size() <= MAXDWORD && WriteFile(file, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                  written == bytes.size() && FlushFileBuffers(file);
  CloseHandle(file);
  if (!ok || !MoveFileExW(temporary.c_str(), path.c_str(), MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)) {
    DeleteFileW(temporary.c_str()); error = L"設定を安全に保存できません。"; return false;
  }
  return true;
}

bool ResolveSettings(const std::filesystem::path& common_path, const std::filesystem::path& workspace_path,
                     EffectiveSettings& settings, std::wstring& error) {
  settings = {};
  Overlay(DefaultSettingsLayer(), L"標準値", settings);
  SettingsLayer common;
  if (!LoadSettingsLayer(common_path, common, error)) return false;
  Overlay(common, L"共通設定", settings);
  SettingsLayer workspace;
  if (!workspace_path.empty() && !LoadSettingsLayer(workspace_path, workspace, error)) return false;
  Overlay(workspace, L"Workspace上書き", settings);
  return ValidateKeybindingConflicts(settings.keybindings, error);
}

std::filesystem::path CommonSettingsPath() {
  PWSTR path{};
  if (FAILED(SHGetKnownFolderPath(FOLDERID_LocalAppData, KF_FLAG_CREATE, nullptr, &path))) return {};
  std::filesystem::path result = std::filesystem::path(path) / L"MDLite" / L"settings.toml";
  CoTaskMemFree(path);
  return result;
}

}  // namespace mdlite
