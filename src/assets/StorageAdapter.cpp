#include "assets/StorageAdapter.h"

#include <windows.h>

#include <algorithm>
#include <fstream>
#include <limits>
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

bool IsHex(wchar_t character) {
  return (character >= L'0' && character <= L'9') ||
         (character >= L'a' && character <= L'f') ||
         (character >= L'A' && character <= L'F');
}

bool IsUriUnreserved(wchar_t character) {
  return (character >= L'A' && character <= L'Z') ||
         (character >= L'a' && character <= L'z') ||
         (character >= L'0' && character <= L'9') ||
         character == L'-' || character == L'.' || character == L'_' || character == L'~';
}

bool IsUriSubDelimiter(wchar_t character) {
  return character == L'!' || character == L'$' || character == L'&' ||
         character == L'\'' || character == L'(' || character == L')' ||
         character == L'*' || character == L'+' || character == L',' ||
         character == L';' || character == L'=';
}

bool IsValidPort(std::wstring_view port) {
  if (port.empty()) return false;
  unsigned value{};
  for (const wchar_t character : port) {
    if (character < L'0' || character > L'9') return false;
    const unsigned digit = static_cast<unsigned>(character - L'0');
    if (value > (65535U - digit) / 10U) return false;
    value = value * 10U + digit;
  }
  return value != 0;
}

bool IsValidRegName(std::wstring_view host) {
  if (host.empty()) return false;
  for (std::size_t index{}; index < host.size(); ++index) {
    const wchar_t character = host[index];
    if (character == L'%') {
      if (index + 2 >= host.size() || !IsHex(host[index + 1]) || !IsHex(host[index + 2]))
        return false;
      index += 2;
    } else if (!IsUriUnreserved(character) && !IsUriSubDelimiter(character)) {
      return false;
    }
  }
  return true;
}

bool IsValidIpv4Address(std::wstring_view address) {
  std::size_t cursor{};
  for (unsigned octet_index{}; octet_index < 4; ++octet_index) {
    const auto separator = address.find(L'.', cursor);
    const auto end = separator == std::wstring_view::npos ? address.size() : separator;
    const auto octet = address.substr(cursor, end - cursor);
    if (octet.empty() || octet.size() > 3 || (octet.size() > 1 && octet.front() == L'0'))
      return false;
    unsigned value{};
    for (const wchar_t character : octet) {
      if (character < L'0' || character > L'9') return false;
      value = value * 10U + static_cast<unsigned>(character - L'0');
    }
    if (value > 255U || (octet_index < 3 && separator == std::wstring_view::npos) ||
        (octet_index == 3 && separator != std::wstring_view::npos)) return false;
    cursor = end + 1;
  }
  return cursor == address.size() + 1;
}

bool CountIpv6Groups(std::wstring_view part, bool allow_ipv4_tail, unsigned& groups) {
  if (part.empty()) return true;
  if (part.front() == L':' || part.back() == L':') return false;
  std::size_t cursor{};
  while (cursor < part.size()) {
    const auto separator = part.find(L':', cursor);
    const auto end = separator == std::wstring_view::npos ? part.size() : separator;
    const auto group = part.substr(cursor, end - cursor);
    if (group.empty()) return false;
    if (group.find(L'.') != std::wstring_view::npos) {
      if (!allow_ipv4_tail || separator != std::wstring_view::npos ||
          !IsValidIpv4Address(group)) return false;
      groups += 2;
    } else {
      if (group.size() > 4 ||
          !std::ranges::all_of(group, [](wchar_t character) { return IsHex(character); }))
        return false;
      ++groups;
    }
    if (separator == std::wstring_view::npos) break;
    cursor = separator + 1;
  }
  return true;
}

bool IsValidIpv6Address(std::wstring_view address) {
  const auto compression = address.find(L"::");
  unsigned groups{};
  if (compression == std::wstring_view::npos) {
    if (address.empty() || address.front() == L':' || address.back() == L':' ||
        !CountIpv6Groups(address, true, groups)) return false;
    return groups == 8;
  }
  if (address.find(L"::", compression + 2) != std::wstring_view::npos) return false;
  const auto left = address.substr(0, compression);
  const auto right = address.substr(compression + 2);
  if (!CountIpv6Groups(left, false, groups) ||
      !CountIpv6Groups(right, true, groups)) return false;
  return groups < 8;
}

bool IsValidIpvFuture(std::wstring_view literal) {
  if (literal.size() < 4 || (literal.front() != L'v' && literal.front() != L'V')) return false;
  const auto dot = literal.find(L'.', 1);
  if (dot == std::wstring_view::npos || dot == 1 || dot + 1 == literal.size()) return false;
  for (std::size_t index = 1; index < dot; ++index) {
    if (!IsHex(literal[index])) return false;
  }
  for (std::size_t index = dot + 1; index < literal.size(); ++index) {
    if (!IsUriUnreserved(literal[index]) && !IsUriSubDelimiter(literal[index]) &&
        literal[index] != L':') return false;
  }
  return true;
}

bool IsValidIpLiteral(std::wstring_view literal) {
  const auto zone_marker = literal.find(L"%25");
  if (zone_marker != std::wstring_view::npos) {
    if (literal.front() == L'v' || literal.front() == L'V') return false;
    const auto zone = literal.substr(zone_marker + 3);
    if (zone.empty()) return false;
    for (std::size_t index{}; index < zone.size(); ++index) {
      if (IsUriUnreserved(zone[index])) continue;
      if (zone[index] != L'%' || index + 2 >= zone.size() ||
          !IsHex(zone[index + 1]) || !IsHex(zone[index + 2])) return false;
      index += 2;
    }
    literal = literal.substr(0, zone_marker);
  }
  if (literal.empty()) return false;
  return literal.front() == L'v' || literal.front() == L'V'
      ? IsValidIpvFuture(literal) : IsValidIpv6Address(literal);
}

bool IsValidUriPchar(wchar_t character) {
  return IsUriUnreserved(character) || IsUriSubDelimiter(character) ||
         character == L':' || character == L'@';
}

bool IsValidUriRemainder(std::wstring_view remainder) {
  bool in_fragment{};
  for (std::size_t index{}; index < remainder.size(); ++index) {
    const wchar_t character = remainder[index];
    if (character == L'%') {
      if (index + 2 >= remainder.size() || !IsHex(remainder[index + 1]) ||
          !IsHex(remainder[index + 2])) return false;
      index += 2;
    } else if (character == L'#') {
      if (in_fragment) return false;
      in_fragment = true;
    } else if (character != L'/' && character != L'?' && !IsValidUriPchar(character)) {
      return false;
    }
  }
  return true;
}

bool IsValidHttpsReference(std::wstring_view reference) {
  // The adapter result is inserted into Markdown verbatim, so require a syntactically valid
  // absolute HTTPS URI before accepting it as a successful upload reference.
  constexpr std::wstring_view prefix = L"https://";
  if (reference.size() <= prefix.size()) return false;
  for (std::size_t index{}; index < prefix.size(); ++index) {
    const wchar_t character = reference[index];
    const wchar_t folded = character >= L'A' && character <= L'Z'
        ? static_cast<wchar_t>(character + (L'a' - L'A')) : character;
    if (folded != prefix[index]) return false;
  }
  const auto authority_end = reference.find_first_of(L"/?#", prefix.size());
  const auto authority = reference.substr(
      prefix.size(), authority_end == std::wstring_view::npos
          ? reference.size() - prefix.size() : authority_end - prefix.size());
  if (authority.empty() || authority.front() == L'@' || authority.front() == L':') return false;
  if (authority.find(L'@') != std::wstring_view::npos) return false;
  if (authority.front() == L'[') {
    const auto closing_bracket = authority.find(L']');
    if (closing_bracket == std::wstring_view::npos || closing_bracket == 1 ||
        (closing_bracket + 1 < authority.size() && authority[closing_bracket + 1] != L':'))
      return false;
    if (!IsValidIpLiteral(authority.substr(1, closing_bracket - 1))) return false;
    if (closing_bracket + 1 < authority.size() &&
        !IsValidPort(authority.substr(closing_bracket + 2))) return false;
  } else {
    const auto port_separator = authority.find(L':');
    if (port_separator != std::wstring_view::npos) {
      if (authority.find(L':', port_separator + 1) != std::wstring_view::npos ||
          !IsValidPort(authority.substr(port_separator + 1))) return false;
    }
    const auto host = authority.substr(0, port_separator);
    if (!IsValidRegName(host)) return false;
  }

  const auto remainder = authority_end == std::wstring_view::npos
      ? std::wstring_view{} : reference.substr(authority_end);
  return IsValidUriRemainder(remainder);
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
      if (equals == std::wstring::npos) {
        error = L"storage adapterのtimeout_msが不正です。";
        return false;
      }
      try {
        std::size_t parsed{};
        const auto value = std::stoull(line.substr(equals + 1), &parsed);
        const auto suffix = line.substr(equals + 1 + parsed);
        if (suffix.find_first_not_of(L" \t\r") != std::wstring::npos || value == 0 ||
            value > std::numeric_limits<DWORD>::max()) {
          error = L"storage adapterのtimeout_msは1以上のミリ秒で指定してください。";
          return false;
        }
        adapter.timeout_ms = static_cast<DWORD>(value);
      } catch (const std::exception&) {
        error = L"storage adapterのtimeout_msは1以上のミリ秒で指定してください。";
        return false;
      }
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
  if (!IsValidHttpsReference(result.reference)) {
    error = L"adapterは空白や不正な% escapeを含まない絶対https URIを1行目へ返す必要があります。";
    return false;
  }
  return true;
}

}  // namespace mdlite
