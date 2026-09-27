#pragma once

#include <windows.h>

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace mdlite {

struct DiagnosticsDependencyVersion {
  std::wstring name;
  std::wstring version;
};

struct DiagnosticsSnapshot {
  bool memory_metrics_available{};
  std::uint64_t working_set_bytes{};
  std::uint64_t private_bytes{};
  std::uint64_t peak_working_set_bytes{};
  std::size_t open_document_count{};
  std::wstring search_state;
  std::wstring active_document_encoding;
  std::wstring app_version;
  std::wstring application_update_status;
  std::wstring dependency_update_status;
  std::vector<DiagnosticsDependencyVersion> dependency_versions;

  // Summaries must already be redacted: never include note text, credentials,
  // or full private paths. Supply lines oldest-to-newest; the view keeps the
  // latest 20 and truncates each to 120 UTF-16 code units.
  std::vector<std::wstring> recent_summaries;
};

using DiagnosticsSnapshotProvider =
    bool (*)(void* context, DiagnosticsSnapshot& snapshot);

// Opens the native diagnostics dialog. The provider is called once before the
// dialog opens and again only when the user presses 更新. Returns whether the
// Task Dialog opened successfully; provider failures are shown in the dialog
// without exposing exception details.
[[nodiscard]] bool ShowDiagnosticsView(HWND owner,
                                       DiagnosticsSnapshotProvider provider,
                                       void* context) noexcept;

}  // namespace mdlite
