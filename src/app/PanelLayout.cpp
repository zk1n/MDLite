#include "app/PanelLayout.h"

#include <algorithm>
#include <charconv>
#include <cmath>
#include <iomanip>
#include <locale>
#include <sstream>
#include <string>
#include <utility>

namespace mdlite {
namespace {

constexpr std::array<PanelId, 4> kPanelIds{
    PanelId::Explorer, PanelId::Calendar, PanelId::Outline, PanelId::Git};
constexpr std::array<PanelSlot, 4> kPanelSlots{
    PanelSlot::LeftTop, PanelSlot::LeftBottom, PanelSlot::RightTop, PanelSlot::RightBottom};

template <typename Enum>
constexpr bool IsEnumValue(Enum value, const auto& values) noexcept {
  return std::ranges::find(values, value) != values.end();
}

constexpr std::size_t Index(PanelId id) noexcept {
  return static_cast<std::size_t>(id);
}

constexpr std::size_t Index(PanelSlot slot) noexcept {
  return static_cast<std::size_t>(slot);
}



std::string PanelNameUtf8(PanelId id) {
  switch (id) {
    case PanelId::Explorer: return "explorer";
    case PanelId::Calendar: return "calendar";
    case PanelId::Outline: return "outline";
    case PanelId::Git: return "git";
  }
  return {};
}

std::string SlotNameUtf8(PanelSlot slot) {
  switch (slot) {
    case PanelSlot::LeftTop: return "left_top";
    case PanelSlot::LeftBottom: return "left_bottom";
    case PanelSlot::RightTop: return "right_top";
    case PanelSlot::RightBottom: return "right_bottom";
  }
  return {};
}

bool ParsePanel(std::string_view value, PanelId& result) {
  if (value == "explorer") result = PanelId::Explorer;
  else if (value == "calendar") result = PanelId::Calendar;
  else if (value == "outline") result = PanelId::Outline;
  else if (value == "git") result = PanelId::Git;
  else return false;
  return true;
}

bool ParseSlot(std::string_view value, PanelSlot& result) {
  if (value == "left_top") result = PanelSlot::LeftTop;
  else if (value == "left_bottom") result = PanelSlot::LeftBottom;
  else if (value == "right_top") result = PanelSlot::RightTop;
  else if (value == "right_bottom") result = PanelSlot::RightBottom;
  else return false;
  return true;
}

std::string Trim(std::string_view value) {
  std::size_t begin = 0;
  while (begin < value.size() && (value[begin] == ' ' || value[begin] == '\t' ||
                                  value[begin] == '\r')) ++begin;
  std::size_t end = value.size();
  while (end > begin && (value[end - 1] == ' ' || value[end - 1] == '\t' ||
                         value[end - 1] == '\r')) --end;
  return std::string(value.substr(begin, end - begin));
}

bool ParseQuoted(std::string_view value, std::string& result) {
  const auto trimmed = Trim(value);
  if (trimmed.size() < 2 || trimmed.front() != '"' || trimmed.back() != '"') return false;
  const auto body = std::string_view(trimmed).substr(1, trimmed.size() - 2);
  if (body.find('"') != std::string_view::npos || body.find('\\') != std::string_view::npos)
    return false;
  result = std::string(body);
  return true;
}

bool ParseDouble(std::string_view value, double& result) {
  const auto trimmed = Trim(value);
  const auto parsed = std::from_chars(trimmed.data(), trimmed.data() + trimmed.size(), result);
  return parsed.ec == std::errc{} && parsed.ptr == trimmed.data() + trimmed.size() &&
         std::isfinite(result);
}

bool ParseBool(std::string_view value, bool& result) {
  const auto trimmed = Trim(value);
  if (trimmed == "true") result = true;
  else if (trimmed == "false") result = false;
  else return false;
  return true;
}

std::wstring InvalidPanelError() { return L"不明なパネルIDです。"; }
std::wstring InvalidSlotError() { return L"不明なパネルスロットです。"; }
std::wstring InvalidDimensionError() { return L"パネル寸法が範囲外です。"; }

struct ParsedPanel {
  PanelState value{};
  bool id{};
  bool slot{};
  bool width{};
  bool height{};
  bool collapsed{};
  bool hidden{};
};

}  // namespace

PanelLayout PanelLayout::Default() { return PanelLayout{}; }

PanelState* PanelLayout::Find(PanelId id) noexcept {
  if (!IsEnumValue(id, kPanelIds)) return nullptr;
  return &panels_[Index(id)];
}

const PanelState* PanelLayout::Find(PanelId id) const noexcept {
  if (!IsEnumValue(id, kPanelIds)) return nullptr;
  return &panels_[Index(id)];
}

bool PanelLayout::CanMove(PanelId id, PanelSlot destination, std::wstring& error) const {
  error.clear();
  const auto* panel = Find(id);
  if (!panel) {
    error = InvalidPanelError();
    return false;
  }
  if (!IsEnumValue(destination, kPanelSlots)) {
    error = InvalidSlotError();
    return false;
  }

  return true;
}

bool PanelLayout::Move(PanelId id, PanelSlot destination, std::wstring& error) {
  if (!CanMove(id, destination, error)) return false;
  auto& source = panels_[Index(id)];
  if (source.slot == destination) return true;
  for (auto& candidate : panels_) {
    if (candidate.id != id && candidate.slot == destination) {
      candidate.slot = source.slot;
      source.slot = destination;
      return true;
    }
  }
  error = L"パネルスロットが見つかりません。";
  return false;
}

bool PanelLayout::Resize(PanelId id, double width, double height, std::wstring& error) {
  error.clear();
  if (!Find(id)) {
    error = InvalidPanelError();
    return false;
  }
  if (!std::isfinite(width) || !std::isfinite(height) ||
      width < kMinDimension || height < kMinDimension ||
      width > kMaxDimension || height > kMaxDimension) {
    error = InvalidDimensionError();
    return false;
  }
  auto& panel = panels_[Index(id)];
  panel.width = width;
  panel.height = height;
  return true;
}

bool PanelLayout::SetCollapsed(PanelId id, bool collapsed, std::wstring& error) {
  error.clear();
  if (!Find(id)) {
    error = InvalidPanelError();
    return false;
  }
  panels_[Index(id)].collapsed = collapsed;
  return true;
}

bool PanelLayout::SetHidden(PanelId id, bool hidden, std::wstring& error) {
  error.clear();
  if (!Find(id)) {
    error = InvalidPanelError();
    return false;
  }
  panels_[Index(id)].hidden = hidden;
  return true;
}

bool PanelLayout::ClampForWindow(double window_width, double window_height, std::wstring& error) {
  error.clear();
  if (!std::isfinite(window_width) || !std::isfinite(window_height) ||
      window_width < 2.0 || window_height < 2.0) {
    error = L"ウィンドウ寸法が小さすぎるか不正です。";
    return false;
  }
  const double max_width = std::min(kMaxDimension, window_width / 2.0);
  const double max_height = std::min(kMaxDimension, window_height / 2.0);
  for (auto& panel : panels_) {
    panel.width = std::clamp(panel.width, kMinDimension, max_width);
    panel.height = std::clamp(panel.height, kMinDimension, max_height);
  }
  return true;
}

void PanelLayout::Reset() { *this = Default(); }

bool PanelLayout::Validate(std::wstring& error) const {
  error.clear();
  std::array<bool, 4> seen_panels{};
  std::array<bool, 4> seen_slots{};
  for (const auto& panel : panels_) {
    if (!IsEnumValue(panel.id, kPanelIds)) {
      error = InvalidPanelError();
      return false;
    }
    if (!IsEnumValue(panel.slot, kPanelSlots)) {
      error = InvalidSlotError();
      return false;
    }
    if (seen_panels[Index(panel.id)]) {
      error = L"同じパネルが重複しています。";
      return false;
    }
    if (seen_slots[Index(panel.slot)]) {
      error = L"同じパネルスロットが重複しています。";
      return false;
    }
    seen_panels[Index(panel.id)] = true;
    seen_slots[Index(panel.slot)] = true;
    if (!std::isfinite(panel.width) || !std::isfinite(panel.height) ||
        panel.width < kMinDimension || panel.height < kMinDimension ||
        panel.width > kMaxDimension || panel.height > kMaxDimension) {
      error = InvalidDimensionError();
      return false;
    }
  }
  return std::ranges::all_of(seen_panels, [](bool seen) { return seen; }) &&
         std::ranges::all_of(seen_slots, [](bool seen) { return seen; });
}

std::string PanelLayout::Serialize() const {
  std::ostringstream output;
  output.imbue(std::locale::classic());
  output << "schema_version = " << kSchemaVersion << "\n";
  output << std::setprecision(17);
  for (const auto& panel : panels_) {
    output << "\n[[panel]]\n"
           << "id = \"" << PanelNameUtf8(panel.id) << "\"\n"
           << "slot = \"" << SlotNameUtf8(panel.slot) << "\"\n"
           << "width = " << panel.width << "\n"
           << "height = " << panel.height << "\n"
           << "collapsed = " << (panel.collapsed ? "true" : "false") << "\n"
           << "hidden = " << (panel.hidden ? "true" : "false") << "\n";
  }
  return output.str();
}

bool PanelLayout::Deserialize(std::string_view text, PanelLayout& layout, std::wstring& error) {
  error.clear();
  int schema_version = 0;
  bool schema_seen = false;
  std::array<ParsedPanel, 4> parsed{};
  std::size_t parsed_count = 0;
  ParsedPanel* current = nullptr;
  std::size_t position = 0;
  while (position <= text.size()) {
    const auto end = text.find('\n', position);
    const auto line = text.substr(position, end == std::string_view::npos ? text.size() - position
                                                                            : end - position);
    position = end == std::string_view::npos ? text.size() + 1 : end + 1;
    const auto trimmed = Trim(line);
    if (trimmed.empty()) continue;
    if (trimmed == "[[panel]]") {
      if (parsed_count >= parsed.size()) {
        error = L"パネルの数が多すぎます。";
        return false;
      }
      current = &parsed[parsed_count++];
      continue;
    }
    const auto equals = trimmed.find('=');
    if (equals == std::string::npos || trimmed.find('=', equals + 1) != std::string::npos) {
      error = L"パネル状態の行を解釈できません。";
      return false;
    }
    const auto key = Trim(std::string_view(trimmed).substr(0, equals));
    const auto value = Trim(std::string_view(trimmed).substr(equals + 1));
    if (key == "schema_version") {
      if (schema_seen || current || value.empty()) {
        error = L"schema_versionの位置または重複が不正です。";
        return false;
      }
      const auto schema_parse = std::from_chars(value.data(), value.data() + value.size(), schema_version);
      if (schema_parse.ec != std::errc{} || schema_parse.ptr != value.data() + value.size()) {
        error = L"schema_versionが不正です。";
        return false;
      }
      schema_seen = true;
      continue;
    }
    if (!current) {
      error = L"パネルレコード外のキーです。";
      return false;
    }
    if (key == "id") {
      if (current->id) { error = L"パネルIDが重複しています。"; return false; }
      std::string value_text;
      if (!ParseQuoted(value, value_text) || !ParsePanel(value_text, current->value.id)) {
        error = L"パネルIDが不正です。";
        return false;
      }
      current->id = true;
    } else if (key == "slot") {
      if (current->slot) { error = L"パネルスロットが重複しています。"; return false; }
      std::string value_text;
      if (!ParseQuoted(value, value_text) || !ParseSlot(value_text, current->value.slot)) {
        error = L"パネルスロットが不正です。";
        return false;
      }
      current->slot = true;
    } else if (key == "width" || key == "height") {
      bool& seen = key == "width" ? current->width : current->height;
      if (seen) { error = L"パネル寸法が重複しています。"; return false; }
      double dimension{};
      if (!ParseDouble(value, dimension) || dimension < kMinDimension ||
          dimension > kMaxDimension) {
        error = InvalidDimensionError();
        return false;
      }
      if (key == "width") current->value.width = dimension;
      else current->value.height = dimension;
      seen = true;
    } else if (key == "collapsed" || key == "hidden") {
      bool& seen = key == "collapsed" ? current->collapsed : current->hidden;
      if (seen) { error = L"パネル表示状態が重複しています。"; return false; }
      bool value_bool{};
      if (!ParseBool(value, value_bool)) {
        error = L"パネル表示状態が不正です。";
        return false;
      }
      if (key == "collapsed") current->value.collapsed = value_bool;
      else current->value.hidden = value_bool;
      seen = true;
    } else {
      error = L"未知のパネル状態キーです。";
      return false;
    }
  }
  if (!schema_seen || schema_version != kSchemaVersion) {
    error = L"未対応または古いパネル状態schemaです。";
    return false;
  }
  if (parsed_count != parsed.size()) {
    error = L"パネル状態の数が不足しています。";
    return false;
  }
  PanelLayout candidate;
  std::array<bool, 4> seen_ids{};
  std::array<bool, 4> seen_slots{};
  for (const auto& record : parsed) {
    if (!record.id || !record.slot || !record.width || !record.height ||
        !record.collapsed || !record.hidden) {
      error = L"パネル状態の項目が不足しています。";
      return false;
    }
    if (seen_ids[Index(record.value.id)] || seen_slots[Index(record.value.slot)]) {
      error = L"パネルまたはスロットが重複しています。";
      return false;
    }
    seen_ids[Index(record.value.id)] = true;
    seen_slots[Index(record.value.slot)] = true;
    candidate.panels_[Index(record.value.id)] = record.value;
  }
  if (!candidate.Validate(error)) return false;
  layout = candidate;
  return true;
}

}  // namespace mdlite
