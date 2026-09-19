#include "assets/StorageAdapter.h"

#include <windows.h>

#include <fstream>
#include <sstream>

namespace mdlite {
namespace {

bool DecodeUtf8(const std::string& bytes, std::wstring& value) {
  if (bytes.empty()) { value.clear(); return true; }
  const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                       static_cast<int>(bytes.size()), nullptr, 0);
  if (size <= 0) return false;
  value.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                             static_cast<int>(bytes.size()), value.data(), size) == size;
}

std::wstring Unquote(std::wstring value) {
  const auto first = value.find(L'"');
  const auto last = value.rfind(L'"');
  return first != std::wstring::npos && last > first ? value.substr(first + 1, last - first - 1) : L"";
}

std::uint64_t HashFile(const std::filesystem::path& path, bool& ok) {
  std::ifstream input(path, std::ios::binary);
  std::uint64_t hash = 14695981039346656037ULL;
  char buffer[8192];
  while (input.read(buffer, sizeof(buffer)) || input.gcount() != 0)
    for (std::streamsize i = 0; i < input.gcount(); ++i) { hash ^= static_cast<unsigned char>(buffer[i]); hash *= 1099511628211ULL; }
  ok = input.eof();
  return hash;
}

void Replace(std::wstring& value, std::wstring_view token, std::wstring_view replacement) {
  std::size_t position{};
  while ((position = value.find(token, position)) != std::wstring::npos) {
    value.replace(position, token.size(), replacement);
    position += replacement.size();
  }
}

}  // namespace

bool LoadStorageAdapter(const std::filesystem::path& config, StorageAdapter& adapter,
                        std::wstring& error) {
  adapter = {};
  std::ifstream input(config, std::ios::binary);
  if (!input) { error = L"storage adapter設定を開けません。"; return false; }
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring text;
  if (!DecodeUtf8(bytes, text)) { error = L"storage adapter設定はUTF-8で保存してください。"; return false; }
  std::wistringstream lines(text);
  std::wstring line;
  while (std::getline(lines, line)) {
    if (line.starts_with(L"executable")) {
      adapter.executable = Unquote(line);
      adapter.executable.make_preferred();
    }
    else if (line.starts_with(L"argument")) adapter.arguments.push_back(Unquote(line));
    else if (line.starts_with(L"timeout_ms")) {
      const auto equals = line.find(L'=');
      if (equals != std::wstring::npos) adapter.timeout_ms = std::stoul(line.substr(equals + 1));
    }
  }
  if (adapter.executable.empty()) { error = L"storage adapterのexecutableが未設定です。"; return false; }
  return true;
}

bool UploadWithStorageAdapter(const StorageAdapter& adapter, const std::filesystem::path& asset,
                              std::wstring_view revision, void* cancellation_event,
                              StorageUploadResult& result, std::wstring& error) {
  bool before_ok{};
  const auto before = HashFile(asset, before_ok);
  if (!before_ok) { error = L"upload前のlocal assetを読み取れません。"; return false; }
  auto arguments = adapter.arguments;
  for (auto& argument : arguments) {
    Replace(argument, L"{file}", asset.wstring());
    Replace(argument, L"{revision}", revision);
  }
  if (!RunProcess(adapter.executable, arguments, asset.parent_path(), 64 * 1024,
                  adapter.timeout_ms, result.process, error, cancellation_event)) return false;
  bool after_ok{};
  const auto after = HashFile(asset, after_ok);
  if (!after_ok || before != after) { error = L"adapter実行中にlocal assetが変更されました。本文リンクは変更しません。"; return false; }
  if (result.process.cancelled) { error = L"uploadを中止しました。local assetは保持しています。"; return false; }
  if (result.process.timed_out || result.process.exit_code != 0) { error = L"upload adapterが失敗しました。local assetは保持しています。"; return false; }
  std::wistringstream lines(result.process.output);
  std::getline(lines, result.reference);
  while (!result.reference.empty() && (result.reference.back() == L'\r' || result.reference.back() == L'\n')) result.reference.pop_back();
  if (!result.reference.starts_with(L"https://")) { error = L"adapterはhttps参照を1行目へ返す必要があります。"; return false; }
  return true;
}

}  // namespace mdlite
