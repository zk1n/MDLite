#include "app/DiagnosticsView.h"

#include <commctrl.h>

#include <algorithm>
#include <string_view>

namespace mdlite {
namespace {

constexpr int kUpdateButtonId = 1001;
constexpr std::size_t kMaxRecentLines = 20;
constexpr std::size_t kMaxRecentLineLength = 120;
constexpr std::size_t kMaxDependencies = 12;
constexpr std::size_t kMaxFieldLength = 120;
constexpr std::uint64_t kBytesPerMiB = 1024 * 1024;

constexpr wchar_t kProviderFailure[] = L"診断情報を取得できませんでした。";

struct DialogState {
  DiagnosticsSnapshotProvider provider{};
  void* context{};
};

bool IsHighSurrogate(wchar_t value) noexcept {
  return value >= 0xD800 && value <= 0xDBFF;
}

bool IsLineBreakOrControl(wchar_t value) noexcept {
  return value == L'\r' || value == L'\n' || value == L'\0' || value < 0x20 ||
         (value >= 0x7F && value <= 0x9F) || value == 0x2028 || value == 0x2029;
}

std::wstring SingleLine(std::wstring_view value, std::size_t max_length) {
  std::wstring result;
  result.reserve(std::min(value.size(), max_length));
  std::size_t index = 0;
  for (; index < value.size() && result.size() < max_length; ++index) {
    result.push_back(IsLineBreakOrControl(value[index]) ? L' ' : value[index]);
  }

  bool truncated = index < value.size();
  if (!result.empty() && IsHighSurrogate(result.back())) {
    result.pop_back();
    truncated = true;
  }
  if (truncated && max_length > 0) {
    if (result.size() == max_length) result.pop_back();
    result.push_back(L'…');
  }
  return result;
}

std::wstring FormatMiB(std::uint64_t bytes) {
  const auto whole = bytes / kBytesPerMiB;
  const auto decimal = ((bytes % kBytesPerMiB) * 10) / kBytesPerMiB;
  return std::to_wstring(whole) + L"." + std::to_wstring(decimal) + L" MiB";
}

void AppendValue(std::wstring& content, std::wstring_view label,
                 std::wstring_view value) {
  content.append(label);
  content.append(value);
  content.push_back(L'\n');
}

std::wstring RenderSnapshot(const DiagnosticsSnapshot& snapshot) {
  std::wstring content;
  content.reserve(4096);

  content.append(L"プロセスメモリ\n");
  if (snapshot.memory_metrics_available) {
    AppendValue(content, L"  ワーキングセット: ", FormatMiB(snapshot.working_set_bytes));
    AppendValue(content, L"  プライベート使用量: ", FormatMiB(snapshot.private_bytes));
    AppendValue(content, L"  最大ワーキングセット: ",
                FormatMiB(snapshot.peak_working_set_bytes));
  } else {
    AppendValue(content, L"  ワーキングセット: ", L"未取得");
    AppendValue(content, L"  プライベート使用量: ", L"未取得");
    AppendValue(content, L"  最大ワーキングセット: ", L"未取得");
  }
  AppendValue(content, L"開いている文書: ",
              std::to_wstring(snapshot.open_document_count));

  const auto search_state = SingleLine(snapshot.search_state, kMaxFieldLength);
  AppendValue(content, L"検索状態: ", search_state.empty() ? L"未取得" : search_state);
  const auto encoding = SingleLine(snapshot.active_document_encoding, kMaxFieldLength);
  AppendValue(content, L"アクティブ文書の文字コード: ",
              encoding.empty() ? L"なし" : encoding);
  const auto app_version = SingleLine(snapshot.app_version, kMaxFieldLength);
  AppendValue(content, L"アプリ: ", app_version.empty() ? L"未取得" : app_version);

  content.append(L"更新・保守\n");
  const auto application_update =
      SingleLine(snapshot.application_update_status, kMaxFieldLength);
  AppendValue(content, L"  アプリ更新元: ",
              application_update.empty() ? L"未設定" : application_update);
  const auto dependency_update =
      SingleLine(snapshot.dependency_update_status, kMaxFieldLength);
  AppendValue(content, L"  依存更新: ",
              dependency_update.empty() ? L"未設定" : dependency_update);

  content.append(L"依存ライブラリ:\n");
  const auto dependency_count =
      std::min(snapshot.dependency_versions.size(), kMaxDependencies);
  if (dependency_count == 0) {
    content.append(L"  （なし）\n");
  } else {
    for (std::size_t i = 0; i < dependency_count; ++i) {
      const auto name = SingleLine(snapshot.dependency_versions[i].name, 48);
      const auto version = SingleLine(snapshot.dependency_versions[i].version, 48);
      content.append(L"  ・");
      content.append(name.empty() ? L"不明" : name);
      content.append(L": ");
      content.append(version.empty() ? L"不明" : version);
      content.push_back(L'\n');
    }
  }

  content.append(L"最近の診断概要:\n");
  if (snapshot.recent_summaries.empty()) {
    content.append(L"  （なし）");
  } else {
    const auto first = snapshot.recent_summaries.size() > kMaxRecentLines
                           ? snapshot.recent_summaries.size() - kMaxRecentLines
                           : 0;
    for (std::size_t i = first; i < snapshot.recent_summaries.size(); ++i) {
      const auto summary = SingleLine(snapshot.recent_summaries[i], kMaxRecentLineLength);
      content.append(L"  ・");
      content.append(summary);
      if (i + 1 < snapshot.recent_summaries.size()) content.push_back(L'\n');
    }
  }
  return content;
}

void ReadSnapshot(DialogState& state, std::wstring& content) {
  if (!state.provider) {
    content = kProviderFailure;
    return;
  }

  DiagnosticsSnapshot snapshot;
  if (!state.provider(state.context, snapshot)) {
    content = kProviderFailure;
    return;
  }

  content = RenderSnapshot(snapshot);
}

HRESULT CALLBACK TaskDialogCallback(HWND dialog, UINT notification,
                                    WPARAM button_id, LPARAM,
                                    LONG_PTR callback_data) noexcept {
  if (notification != TDN_BUTTON_CLICKED ||
      button_id != static_cast<WPARAM>(kUpdateButtonId)) {
    return S_OK;
  }

  auto* state = reinterpret_cast<DialogState*>(callback_data);
  if (state == nullptr) return S_FALSE;

  try {
    std::wstring refreshed_content;
    ReadSnapshot(*state, refreshed_content);
    // TDM_SET_ELEMENT_TEXT is synchronous. Keep refreshed_content alive until
    // SendMessage returns so the Task Dialog can copy the new content safely.
    SendMessageW(dialog, TDM_SET_ELEMENT_TEXT, TDE_CONTENT,
                 reinterpret_cast<LPARAM>(refreshed_content.c_str()));
  } catch (...) {
    SendMessageW(dialog, TDM_SET_ELEMENT_TEXT, TDE_CONTENT,
                 reinterpret_cast<LPARAM>(kProviderFailure));
  }
  return S_FALSE;
}

}  // namespace

bool ShowDiagnosticsView(HWND owner, DiagnosticsSnapshotProvider provider,
                         void* context) noexcept {
  try {
    DialogState state{provider, context};
    std::wstring initial_content;
    try {
      ReadSnapshot(state, initial_content);
    } catch (...) {
      initial_content = kProviderFailure;
    }

    const TASKDIALOG_BUTTON update_button{kUpdateButtonId, L"更新"};
    TASKDIALOGCONFIG config{};
    config.cbSize = sizeof(config);
    config.hwndParent = owner;
    config.dwFlags = TDF_ALLOW_DIALOG_CANCELLATION;
    config.dwCommonButtons = TDCBF_CLOSE_BUTTON;
    config.pszWindowTitle = L"診断情報";
    config.pszMainInstruction = L"アプリケーションの状態";
    config.pszContent = initial_content.c_str();
    config.cButtons = 1;
    config.pButtons = &update_button;
    config.pfCallback = TaskDialogCallback;
    config.lpCallbackData = reinterpret_cast<LONG_PTR>(&state);

    return SUCCEEDED(TaskDialogIndirect(&config, nullptr, nullptr, nullptr));
  } catch (...) {
    return false;
  }
}

}  // namespace mdlite
