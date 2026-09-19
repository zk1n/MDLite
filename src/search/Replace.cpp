#include "search/Replace.h"

#include "core/Document.h"

#include <windows.h>

#include <algorithm>
#include <chrono>
#include <fstream>
#include <regex>
#include <sstream>

namespace mdlite {
namespace {

bool IsWordCharacter(wchar_t character) { return iswalnum(character) != 0 || character == L'_'; }

bool IsWholeWord(std::wstring_view text, std::size_t begin, std::size_t end) {
  return (begin == 0 || !IsWordCharacter(text[begin - 1])) &&
         (end == text.size() || !IsWordCharacter(text[end]));
}

std::wstring Fold(std::wstring_view value) {
  std::wstring folded(value);
  std::ranges::transform(folded, folded.begin(), towlower);
  return folded;
}

bool ReplaceText(std::wstring_view source, const SearchQuery& query, std::wstring_view replacement,
                 std::wstring& output, std::size_t& count, std::wstring& error) {
  count = 0;
  if (query.regular_expression) {
    try {
      const auto flags = std::regex_constants::ECMAScript |
                         (query.match_case ? std::regex_constants::syntax_option_type{}
                                           : std::regex_constants::icase);
      const std::wregex expression(query.text, flags);
      output.clear();
      std::size_t cursor{};
      const std::wstring owned(source);
      for (std::wsregex_iterator iterator(owned.begin(), owned.end(), expression), end;
           iterator != end; ++iterator) {
        const std::size_t begin = static_cast<std::size_t>(iterator->position());
        const std::size_t finish = begin + static_cast<std::size_t>(iterator->length());
        if (query.whole_word && !IsWholeWord(source, begin, finish)) continue;
        output.append(source.substr(cursor, begin - cursor));
        output += iterator->format(std::wstring(replacement));
        cursor = finish;
        ++count;
      }
      output.append(source.substr(cursor));
      return true;
    } catch (const std::regex_error&) {
      error = L"正規表現または置換式が正しくありません。";
      return false;
    }
  }
  output.clear();
  const std::wstring haystack = query.match_case ? std::wstring(source) : Fold(source);
  const std::wstring needle = query.match_case ? query.text : Fold(query.text);
  if (needle.empty()) {
    error = L"検索文字列を入力してください。";
    return false;
  }
  std::size_t scan{};
  std::size_t emitted{};
  while (scan < source.size()) {
    const std::size_t found = haystack.find(needle, scan);
    if (found == std::wstring::npos) break;
    const std::size_t finish = found + needle.size();
    if (query.whole_word && !IsWholeWord(source, found, finish)) {
      scan = found + 1;
      continue;
    }
    output.append(source.substr(emitted, found - emitted));
    output += replacement;
    scan = finish;
    emitted = finish;
    ++count;
  }
  if (count == 0) output.assign(source);
  else output.append(source.substr(emitted));
  return true;
}

bool EncodeUtf8(std::wstring_view text, std::string& bytes) {
  if (text.empty()) { bytes.clear(); return true; }
  const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                       static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
  if (size <= 0) return false;
  bytes.resize(static_cast<std::size_t>(size));
  return WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                             static_cast<int>(text.size()), bytes.data(), size, nullptr, nullptr) == size;
}

bool DecodeUtf8(const std::string& bytes, std::wstring& text) {
  if (bytes.empty()) { text.clear(); return true; }
  const int size = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                                       static_cast<int>(bytes.size()), nullptr, 0);
  if (size <= 0) return false;
  text.resize(static_cast<std::size_t>(size));
  return MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, bytes.data(),
                             static_cast<int>(bytes.size()), text.data(), size) == size;
}

std::wstring Hex(std::wstring_view value) {
  std::string bytes;
  EncodeUtf8(value, bytes);
  static constexpr char digits[] = "0123456789ABCDEF";
  std::wstring result;
  result.reserve(bytes.size() * 2);
  for (unsigned char byte : bytes) {
    result.push_back(digits[byte >> 4]);
    result.push_back(digits[byte & 15]);
  }
  return result;
}

bool Unhex(std::wstring_view value, std::wstring& output) {
  if (value.size() % 2 != 0) return false;
  auto digit = [](wchar_t c) -> int {
    if (c >= L'0' && c <= L'9') return c - L'0';
    if (c >= L'A' && c <= L'F') return c - L'A' + 10;
    return -1;
  };
  std::string bytes(value.size() / 2, '\0');
  for (std::size_t i = 0; i < bytes.size(); ++i) {
    const int high = digit(value[i * 2]);
    const int low = digit(value[i * 2 + 1]);
    if (high < 0 || low < 0) return false;
    bytes[i] = static_cast<char>((high << 4) | low);
  }
  return DecodeUtf8(bytes, output);
}

bool WriteJournal(const std::filesystem::path& path, const ReplacePlan& plan,
                  const std::vector<bool>& applied, std::wstring& error) {
  std::wstring body = L"MDLITE_REPLACE_JOURNAL_V1\n";
  for (std::size_t index = 0; index < plan.files.size(); ++index) {
    if (!applied[index]) continue;
    body += Hex(plan.files[index].path.wstring()) + L"\t" + Hex(plan.files[index].before) + L"\t" +
            Hex(plan.files[index].after) + L"\n";
  }
  std::string bytes;
  if (!EncodeUtf8(body, bytes)) return false;
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  if (!output || (!bytes.empty() && !output.write(bytes.data(), static_cast<std::streamsize>(bytes.size())))) {
    error = L"置換journalを書き込めません。";
    return false;
  }
  output.flush();
  return static_cast<bool>(output);
}

}  // namespace

bool PreviewWorkspaceReplace(const std::filesystem::path& root, const SearchQuery& query,
                             std::wstring_view replacement,
                             const std::map<std::filesystem::path, std::wstring>& unsaved,
                             ReplacePlan& plan, std::wstring& error) {
  plan = {query, std::wstring(replacement), {}};
  std::vector<SearchMatch> matches;
  if (!SearchWorkspace(root, query, unsaved, matches, error)) return false;
  std::map<std::filesystem::path, std::size_t> counts;
  for (const auto& match : matches) ++counts[match.path];
  for (const auto& [path, expected_count] : counts) {
    std::wstring before;
    const auto unsaved_it = unsaved.find(path);
    if (unsaved_it != unsaved.end()) before = unsaved_it->second;
    else {
      Document document;
      if (!document.Load(path, error)) return false;
      before = document.text();
    }
    std::wstring after;
    std::size_t actual_count{};
    if (!ReplaceText(before, query, replacement, after, actual_count, error)) return false;
    if (actual_count != expected_count) {
      error = L"置換preview中に検索結果が変化しました。";
      return false;
    }
    plan.files.push_back({path, std::move(before), std::move(after), actual_count});
  }
  return true;
}

bool ApplyWorkspaceReplace(const std::filesystem::path& workspace, const ReplacePlan& plan,
                           ReplaceApplyResult& result, std::wstring& error) {
  result = {};
  const auto journal_directory = workspace / L".mdlite/.state/replace";
  std::error_code filesystem_error;
  std::filesystem::create_directories(journal_directory, filesystem_error);
  if (filesystem_error) { error = L"置換journal領域を作成できません。"; return false; }
  const auto stamp = std::chrono::duration_cast<std::chrono::milliseconds>(
      std::chrono::system_clock::now().time_since_epoch()).count();
  result.journal = journal_directory / (L"replace-" + std::to_wstring(stamp) + L".journal");
  std::vector<bool> applied(plan.files.size(), false);
  for (std::size_t index = 0; index < plan.files.size(); ++index) {
    const auto& item = plan.files[index];
    Document document;
    std::wstring item_error;
    if (!document.Load(item.path, item_error) || document.text() != item.before) {
      result.conflicts.push_back(item.path);
      continue;
    }
    document.MarkEdited(item.after);
    if (!document.Save(item_error)) {
      result.conflicts.push_back(item.path);
      continue;
    }
    applied[index] = true;
    ++result.applied_files;
    result.applied_replacements += item.replacement_count;
  }
  if (!WriteJournal(result.journal, plan, applied, error)) return false;
  return result.conflicts.empty();
}

bool RollbackWorkspaceReplace(const std::filesystem::path& journal,
                              ReplaceApplyResult& result, std::wstring& error) {
  result = {};
  std::ifstream input(journal, std::ios::binary);
  if (!input) { error = L"置換journalを開けません。"; return false; }
  const std::string bytes((std::istreambuf_iterator<char>(input)), std::istreambuf_iterator<char>());
  std::wstring text;
  if (!DecodeUtf8(bytes, text)) { error = L"置換journalが壊れています。"; return false; }
  std::wistringstream lines(text);
  std::wstring line;
  if (!std::getline(lines, line) || line != L"MDLITE_REPLACE_JOURNAL_V1") {
    error = L"置換journal形式が一致しません。";
    return false;
  }
  while (std::getline(lines, line)) {
    const auto first = line.find(L'\t');
    const auto second = first == std::wstring::npos ? first : line.find(L'\t', first + 1);
    if (first == std::wstring::npos || second == std::wstring::npos) continue;
    std::wstring path_text, before, after;
    if (!Unhex(line.substr(0, first), path_text) ||
        !Unhex(line.substr(first + 1, second - first - 1), before) ||
        !Unhex(line.substr(second + 1), after)) continue;
    Document document;
    std::wstring item_error;
    const std::filesystem::path path(path_text);
    if (!document.Load(path, item_error) || document.text() != after) {
      result.conflicts.push_back(path);
      continue;
    }
    document.MarkEdited(before);
    if (!document.Save(item_error)) { result.conflicts.push_back(path); continue; }
    ++result.applied_files;
  }
  return result.conflicts.empty();
}

}  // namespace mdlite
