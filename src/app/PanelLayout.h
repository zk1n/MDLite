#pragma once

#include <array>
#include <cstdint>
#include <string>
#include <string_view>

namespace mdlite {

// Panel geometry is stored in logical (DPI-neutral) units.  Native window
// code is responsible for converting these values to device pixels.
enum class PanelId : std::uint8_t {
  Explorer,
  Calendar,
  Outline,
  Git,
};

enum class PanelSlot : std::uint8_t {
  LeftTop,
  LeftBottom,
  RightTop,
  RightBottom,
};

struct PanelState {
  PanelId id{};
  PanelSlot slot{};
  double width{};
  double height{};
  bool collapsed{};
  bool hidden{};
};

class PanelLayout {
 public:
  static constexpr int kSchemaVersion = 1;
  static constexpr double kMinDimension = 1.0;
  static constexpr double kMaxDimension = 4096.0;

  static PanelLayout Default();

  [[nodiscard]] const PanelState* Find(PanelId id) const noexcept;
  [[nodiscard]] PanelState* Find(PanelId id) noexcept;
  [[nodiscard]] const std::array<PanelState, 4>& panels() const noexcept { return panels_; }

  // A move is valid only when both enum values are known. Since the four
  // built-in panels occupy four slots, moving onto another panel swaps slots.
  [[nodiscard]] bool CanMove(PanelId id, PanelSlot destination,
                             std::wstring& error) const;
  bool Move(PanelId id, PanelSlot destination, std::wstring& error);
  bool Resize(PanelId id, double width, double height, std::wstring& error);
  bool SetCollapsed(PanelId id, bool collapsed, std::wstring& error);
  bool SetHidden(PanelId id, bool hidden, std::wstring& error);

  // Clamp both axes so the two panels on each side can fit in a narrow
  // window.  A window below two logical units on either axis is invalid.
  bool ClampForWindow(double window_width, double window_height, std::wstring& error);

  void Reset();
  [[nodiscard]] bool Validate(std::wstring& error) const;

  // Deterministic, UTF-8 TOML-compatible state.  Deserialize accepts only
  // this schema version and leaves output unchanged on failure.
  [[nodiscard]] std::string Serialize() const;
  static bool Deserialize(std::string_view text, PanelLayout& layout, std::wstring& error);

 private:
  std::array<PanelState, 4> panels_{{
      {PanelId::Explorer, PanelSlot::LeftTop, 280.0, 240.0, false, false},
      // Keep the secondary calendar collapsed from the first launch, matching
      // the native editor's established compact initial view. The View menu
      // can reveal it without changing the source document.
      {PanelId::Calendar, PanelSlot::LeftBottom, 280.0, 240.0, false, true},
      {PanelId::Outline, PanelSlot::RightTop, 280.0, 240.0, false, false},
      {PanelId::Git, PanelSlot::RightBottom, 280.0, 240.0, false, false},
  }};
};

}  // namespace mdlite
