#pragma once

#include "core/Document.h"

#include <filesystem>
#include <map>
#include <optional>
#include <string>
#include <vector>

namespace mdlite {

enum class ThemeMode { System, Light, Dark, Custom };

struct SettingsLayer {
  std::optional<bool> auto_save;
  std::optional<unsigned> auto_save_delay_ms;
  std::optional<ThemeMode> theme;
  std::optional<std::wstring> font_face;
  std::optional<unsigned> font_size_pt;
  std::optional<std::filesystem::path> default_memo_workspace;
  std::map<std::wstring, std::wstring> keybindings;
  std::map<std::wstring, std::wstring> colors;
  std::vector<std::wstring> preserved_lines;
};

struct EffectiveSettings {
  bool auto_save{true};
  unsigned auto_save_delay_ms{750};
  ThemeMode theme{ThemeMode::Dark};
  std::wstring font_face{L"Segoe UI"};
  unsigned font_size_pt{11};
  std::filesystem::path default_memo_workspace;
  std::map<std::wstring, std::wstring> keybindings;
  std::map<std::wstring, std::wstring> colors;
  std::map<std::wstring, std::wstring> origins;
};

struct SettingsFileSnapshot {
  Document document;
  bool existed{};
};

SettingsLayer DefaultSettingsLayer();
bool LoadSettingsLayer(const std::filesystem::path& path, SettingsLayer& layer, std::wstring& error);
bool SaveSettingsLayer(const std::filesystem::path& path, const SettingsLayer& layer, std::wstring& error);
bool CaptureSettingsFile(const std::filesystem::path& path, SettingsFileSnapshot& snapshot, std::wstring& error);
bool ParseSettingsSnapshot(const SettingsFileSnapshot& snapshot, SettingsLayer& layer, std::wstring& error);
bool CheckSettingsSnapshot(const SettingsFileSnapshot& snapshot, std::wstring& error);
bool SaveSettingsSnapshot(SettingsFileSnapshot& snapshot, const SettingsLayer& layer, std::wstring& error);
bool ResolveSettings(const std::filesystem::path& common_path, const std::filesystem::path& workspace_path,
                     EffectiveSettings& settings, std::wstring& error);
bool ValidateSettingsLayer(const SettingsLayer& layer, std::wstring& error);
bool ValidateKeybindingConflicts(const std::map<std::wstring, std::wstring>& bindings, std::wstring& error);
std::filesystem::path CommonSettingsPath();
std::wstring ThemeName(ThemeMode theme);
std::optional<ThemeMode> ParseTheme(std::wstring_view value);

}  // namespace mdlite
