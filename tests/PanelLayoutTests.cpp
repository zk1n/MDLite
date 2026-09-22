#include "app/PanelLayout.h"
#include "workspace/Workspace.h"

#include <cmath>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <limits>
#include <string>

namespace {

int checks = 0;
int failures = 0;

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    ++failures;
    std::cerr << "FAIL: " << message << "\n";
  }
}

bool Same(const mdlite::PanelLayout& left, const mdlite::PanelLayout& right) {
  const auto& a = left.panels();
  const auto& b = right.panels();
  for (std::size_t index = 0; index < a.size(); ++index) {
    if (a[index].id != b[index].id || a[index].slot != b[index].slot ||
        a[index].width != b[index].width || a[index].height != b[index].height ||
        a[index].collapsed != b[index].collapsed || a[index].hidden != b[index].hidden)
      return false;
  }
  return true;
}

void TestDefaultAndMutation() {
  auto layout = mdlite::PanelLayout::Default();
  std::wstring error;
  Check(layout.Validate(error), "default layout validates");
  Check(layout.panels().size() == 4, "default layout has four panels");
  Check(layout.Find(mdlite::PanelId::Explorer)->slot == mdlite::PanelSlot::LeftTop,
        "Explorer defaults to left top");
  Check(layout.Find(mdlite::PanelId::Calendar)->slot == mdlite::PanelSlot::LeftBottom,
        "Calendar defaults to left bottom");
  Check(layout.Find(mdlite::PanelId::Outline)->slot == mdlite::PanelSlot::RightTop,
        "Outline defaults to right top");
  Check(layout.Find(mdlite::PanelId::Git)->slot == mdlite::PanelSlot::RightBottom,
        "Git defaults to right bottom");
  Check(layout.Find(mdlite::PanelId::Calendar)->hidden,
        "Calendar starts hidden in the compact default layout");

  Check(layout.Move(mdlite::PanelId::Explorer, mdlite::PanelSlot::RightTop, error),
        "moving onto a panel swaps slots");
  Check(layout.Find(mdlite::PanelId::Explorer)->slot == mdlite::PanelSlot::RightTop &&
            layout.Find(mdlite::PanelId::Outline)->slot == mdlite::PanelSlot::LeftTop,
        "slot swap preserves one panel per slot");
  Check(layout.Resize(mdlite::PanelId::Explorer, 320.5, 410.25, error),
        "resize accepts finite logical dimensions");
  Check(layout.SetCollapsed(mdlite::PanelId::Explorer, true, error), "collapse state changes");
  Check(layout.SetHidden(mdlite::PanelId::Git, true, error), "hidden state changes independently");
  Check(layout.ClampForWindow(300.0, 280.0, error), "narrow window clamps layout");
  Check(layout.Find(mdlite::PanelId::Explorer)->width == 150.0 &&
            layout.Find(mdlite::PanelId::Explorer)->height == 140.0,
        "narrow window clamps both dimensions");
  const auto defaults = mdlite::PanelLayout::Default();
  layout.Reset();
  Check(Same(layout, defaults), "reset restores deterministic defaults");
}

void TestInvalidInputAndVersions() {
  auto layout = mdlite::PanelLayout::Default();
  const auto original = layout;
  std::wstring error;
  const auto bad_panel = static_cast<mdlite::PanelId>(99);
  const auto bad_slot = static_cast<mdlite::PanelSlot>(99);
  Check(!layout.Find(bad_panel), "unknown panel id is rejected");
  Check(!layout.Move(bad_panel, mdlite::PanelSlot::LeftTop, error), "move rejects unknown panel id");
  Check(!layout.Move(mdlite::PanelId::Explorer, bad_slot, error), "move rejects unknown slot");
  Check(!layout.Resize(mdlite::PanelId::Explorer, -1.0, 10.0, error),
        "resize rejects negative dimensions");
  Check(!layout.Resize(mdlite::PanelId::Explorer, std::numeric_limits<double>::quiet_NaN(), 10.0, error),
        "resize rejects NaN dimensions");
  Check(!layout.Resize(mdlite::PanelId::Explorer, mdlite::PanelLayout::kMaxDimension + 1.0, 10.0, error),
        "resize rejects out-of-range dimensions");
  Check(!layout.ClampForWindow(std::numeric_limits<double>::infinity(), 100.0, error),
        "clamp rejects non-finite window dimensions");
  Check(!layout.ClampForWindow(1.0, 100.0, error), "clamp rejects an unusably narrow window");

  const std::string serialized = layout.Serialize();
  std::string duplicate = serialized;
  const auto calendar = duplicate.find("id = \"calendar\"");
  duplicate.replace(calendar, std::string("id = \"calendar\"").size(), "id = \"explorer\"");
  mdlite::PanelLayout parsed = original;
  Check(!mdlite::PanelLayout::Deserialize(duplicate, parsed, error),
        "deserializer rejects duplicate panel identities");
  Check(Same(parsed, original), "failed deserialization leaves output unchanged");
  std::string stale = serialized;
  const auto version = stale.find("schema_version = 1");
  stale.replace(version, std::string("schema_version = 1").size(), "schema_version = 0");
  Check(!mdlite::PanelLayout::Deserialize(stale, parsed, error),
        "deserializer rejects stale schema versions");
  const auto last_record = serialized.rfind("\n[[panel]]");
  Check(!mdlite::PanelLayout::Deserialize(serialized.substr(0, last_record), parsed, error),
        "deserializer rejects incomplete layouts");
  Check(!mdlite::PanelLayout::Deserialize(serialized + "unknown = true\n", parsed, error),
        "deserializer rejects unknown state keys safely");
}

void TestSerializationAndWorkspace(const std::filesystem::path& root) {
  auto expected = mdlite::PanelLayout::Default();
  std::wstring error;
  Check(expected.Move(mdlite::PanelId::Git, mdlite::PanelSlot::LeftTop, error),
        "custom layout move succeeds before persistence");
  Check(expected.Resize(mdlite::PanelId::Calendar, 333.125, 177.5, error),
        "custom layout resize succeeds before persistence");
  Check(expected.SetCollapsed(mdlite::PanelId::Calendar, true, error),
        "custom collapsed state persists");
  Check(expected.SetHidden(mdlite::PanelId::Outline, true, error),
        "custom hidden state persists");
  const auto bytes = expected.Serialize();
  mdlite::PanelLayout direct;
  const bool direct_ok = mdlite::PanelLayout::Deserialize(bytes, direct, error);
  Check(direct_ok && Same(expected, direct),
        "layout serializes and round trips deterministically");
  Check(bytes == direct.Serialize(), "round-trip serialization is byte deterministic");

  const auto workspace_root = root / "workspace";
  std::filesystem::create_directories(workspace_root);
  mdlite::WorkspaceStore store(workspace_root);
  Check(store.Initialize(error), "workspace initializes for panel state");
  Check(store.WritePanelLayout(expected, error), "panel layout writes inside workspace state");
  Check(std::filesystem::is_regular_file(store.panel_layout_path()),
        "panel layout uses the local .mdlite state directory");
  mdlite::PanelLayout loaded;
  Check(store.ReadPanelLayout(loaded, error) && Same(expected, loaded),
        "panel layout reads from workspace state");
  std::ifstream input(store.panel_layout_path(), std::ios::binary);
  const std::string persisted((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  Check(persisted == bytes, "workspace persistence keeps deterministic bytes");
  input.close();
  std::filesystem::remove(store.panel_layout_path());
  loaded = expected;
  Check(store.ReadPanelLayout(loaded, error) && Same(loaded, mdlite::PanelLayout::Default()),
        "missing panel state safely falls back to defaults");
}

}  // namespace

int main() {
  const auto root = std::filesystem::temp_directory_path() / "mdlite-panel-layout-tests";
  std::error_code filesystem_error;
  std::filesystem::remove_all(root, filesystem_error);
  std::filesystem::create_directories(root, filesystem_error);
  TestDefaultAndMutation();
  TestInvalidInputAndVersions();
  TestSerializationAndWorkspace(root);
  std::filesystem::remove_all(root, filesystem_error);
  if (failures == 0) std::cout << "All " << checks << " panel layout checks passed.\n";
  return failures == 0 ? 0 : 1;
}