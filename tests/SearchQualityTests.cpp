#include "search/Replace.h"
#include "search/Search.h"

#include "core/Document.h"

#include <windows.h>
#include <winioctl.h>

#include <algorithm>
#include <chrono>
#include <cstddef>
#include <filesystem>
#include <fstream>
#include <initializer_list>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

namespace {

int checks{};
int failures{};

void Check(bool condition, const char* message) {
  ++checks;
  if (!condition) {
    ++failures;
    std::cerr << "FAIL: " << message << "\n";
  }
}

class TemporaryDirectory {
 public:
  TemporaryDirectory() {
    std::error_code error;
    temporary_root_ = std::filesystem::canonical(std::filesystem::temp_directory_path(), error);
    if (error) throw std::runtime_error("Cannot resolve the system temporary directory.");
    const auto id = std::to_wstring(GetCurrentProcessId()) + L"-" +
                    std::to_wstring(GetTickCount64());
    path_ = temporary_root_ / (L"MDLite-SearchQuality-" + id);
    if (!std::filesystem::create_directory(path_, error) || error)
      throw std::runtime_error("Cannot claim a fresh search-quality temporary directory.");
    canonical_path_ = std::filesystem::canonical(path_, error);
    const DWORD attributes = GetFileAttributesW(path_.c_str());
    if (error || canonical_path_.parent_path() != temporary_root_ ||
        attributes == INVALID_FILE_ATTRIBUTES ||
        (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 ||
        (attributes & FILE_ATTRIBUTE_DIRECTORY) == 0)
      throw std::runtime_error("The claimed temporary directory failed its boundary check.");
    owned_ = true;
  }
  ~TemporaryDirectory() {
    if (!owned_ || !IsOwnedDirectorySafeToRemove()) return;
    std::error_code ignored;
    std::filesystem::remove_all(canonical_path_, ignored);
  }
  const std::filesystem::path& path() const { return path_; }

 private:
  bool IsOwnedDirectorySafeToRemove() const {
    std::error_code error;
    const auto resolved_parent = std::filesystem::canonical(path_.parent_path(), error);
    if (error || resolved_parent != temporary_root_) return false;
    const auto resolved_path = std::filesystem::canonical(path_, error);
    if (error || resolved_path != canonical_path_) return false;
    const DWORD root_attributes = GetFileAttributesW(canonical_path_.c_str());
    if (root_attributes == INVALID_FILE_ATTRIBUTES ||
        (root_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0 ||
        (root_attributes & FILE_ATTRIBUTE_DIRECTORY) == 0)
      return false;

    std::vector<std::filesystem::path> pending{canonical_path_};
    while (!pending.empty()) {
      const auto directory = std::move(pending.back());
      pending.pop_back();
      std::error_code enumeration_error;
      for (std::filesystem::directory_iterator iterator(directory, enumeration_error), end;
           iterator != end && !enumeration_error; iterator.increment(enumeration_error)) {
        const DWORD attributes = GetFileAttributesW(iterator->path().c_str());
        if (attributes == INVALID_FILE_ATTRIBUTES ||
            (attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0)
          return false;
        if ((attributes & FILE_ATTRIBUTE_DIRECTORY) != 0) pending.push_back(iterator->path());
      }
      if (enumeration_error) return false;
    }
    return true;
  }

  std::filesystem::path temporary_root_;
  std::filesystem::path path_;
  std::filesystem::path canonical_path_;
  bool owned_{};
};

bool WriteBytes(const std::filesystem::path& path, std::string_view bytes) {
  std::filesystem::create_directories(path.parent_path());
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  return output && (bytes.empty() || static_cast<bool>(output.write(
      bytes.data(), static_cast<std::streamsize>(bytes.size()))));
}

bool SetHiddenAttribute(const std::filesystem::path& path, bool hidden) {
  DWORD attributes = GetFileAttributesW(path.c_str());
  if (attributes == INVALID_FILE_ATTRIBUTES) return false;
  if (hidden) {
    attributes &= ~FILE_ATTRIBUTE_NORMAL;
    attributes |= FILE_ATTRIBUTE_HIDDEN;
  } else {
    attributes &= ~FILE_ATTRIBUTE_HIDDEN;
    if (attributes == 0) attributes = FILE_ATTRIBUTE_NORMAL;
  }
  return SetFileAttributesW(path.c_str(), attributes) != 0;
}

bool ReadBytes(const std::filesystem::path& path, std::vector<unsigned char>& bytes) {
  std::ifstream input(path, std::ios::binary | std::ios::ate);
  if (!input) return false;
  const std::streamoff size = input.tellg();
  if (size < 0) return false;
  bytes.resize(static_cast<std::size_t>(size));
  input.seekg(0);
  if (!bytes.empty())
    input.read(reinterpret_cast<char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
  return static_cast<bool>(input);
}

std::string Utf8Bytes(std::wstring_view text) {
  if (text.empty()) return {};
  const int required = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                            static_cast<int>(text.size()), nullptr, 0,
                                            nullptr, nullptr);
  if (required <= 0) return {};
  std::string bytes(static_cast<std::size_t>(required), '\0');
  if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                          static_cast<int>(text.size()), bytes.data(), required,
                          nullptr, nullptr) != required)
    return {};
  return bytes;
}

std::string HexUtf8(std::wstring_view text) {
  static constexpr char digits[] = "0123456789ABCDEF";
  const std::string bytes = Utf8Bytes(text);
  if (bytes.empty()) return {};
  std::string encoded;
  encoded.reserve(bytes.size() * 2);
  for (const unsigned char byte : bytes) {
    encoded.push_back(digits[byte >> 4]);
    encoded.push_back(digits[byte & 0x0F]);
  }
  return encoded;
}

bool WriteV1Journal(const std::filesystem::path& journal,
                    const std::filesystem::path& target,
                    std::wstring_view before, std::wstring_view after) {
  const auto absolute_target = std::filesystem::absolute(target).lexically_normal();
  const auto path_hex = HexUtf8(absolute_target.wstring());
  const auto before_hex = HexUtf8(before);
  const auto after_hex = HexUtf8(after);
  if (path_hex.empty() || before_hex.empty() || after_hex.empty()) return false;
  return WriteBytes(journal, "MDLITE_REPLACE_JOURNAL_V1\n" + path_hex + "\t" +
                                 before_hex + "\t" + after_hex + "\n");
}

struct MountPointReparseBufferForTest {
  DWORD tag{};
  WORD data_length{};
  WORD reserved{};
  WORD substitute_offset{};
  WORD substitute_length{};
  WORD print_offset{};
  WORD print_length{};
  WCHAR path_buffer[1]{};
};

bool CreateDirectoryJunction(const std::filesystem::path& junction,
                             const std::filesystem::path& target) {
  if (!CreateDirectoryW(junction.c_str(), nullptr)) return false;
  const auto target_path = std::filesystem::absolute(target).lexically_normal().wstring();
  const std::wstring substitute = L"\\??\\" + target_path;
  const std::wstring print = target_path;
  const std::size_t path_units = substitute.size() + 1 + print.size() + 1;
  const std::size_t total_size = offsetof(MountPointReparseBufferForTest, path_buffer) +
                                 path_units * sizeof(wchar_t);
  std::vector<std::byte> storage(total_size);
  auto* data = reinterpret_cast<MountPointReparseBufferForTest*>(storage.data());
  data->tag = IO_REPARSE_TAG_MOUNT_POINT;
  data->data_length = static_cast<WORD>(total_size - 8);
  data->reserved = 0;
  data->substitute_offset = 0;
  data->substitute_length = static_cast<WORD>(substitute.size() * sizeof(wchar_t));
  data->print_offset = static_cast<WORD>(data->substitute_length + sizeof(wchar_t));
  data->print_length = static_cast<WORD>(print.size() * sizeof(wchar_t));
  std::copy(substitute.begin(), substitute.end(), data->path_buffer);
  data->path_buffer[substitute.size()] = L'\0';
  std::copy(print.begin(), print.end(), data->path_buffer + data->print_offset / sizeof(wchar_t));
  data->path_buffer[substitute.size() + 1 + print.size()] = L'\0';

  HANDLE handle = CreateFileW(junction.c_str(), GENERIC_WRITE, 0, nullptr, OPEN_EXISTING,
                              FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                              nullptr);
  if (handle == INVALID_HANDLE_VALUE) {
    RemoveDirectoryW(junction.c_str());
    return false;
  }
  DWORD bytes_returned{};
  const BOOL created = DeviceIoControl(handle, FSCTL_SET_REPARSE_POINT, storage.data(),
                                      static_cast<DWORD>(total_size), nullptr, 0,
                                      &bytes_returned, nullptr);
  CloseHandle(handle);
  if (!created) RemoveDirectoryW(junction.c_str());
  return created != 0;
}

bool WriteOversizedFile(const std::filesystem::path& path) {
  std::filesystem::create_directories(path.parent_path());
  std::ofstream output(path, std::ios::binary | std::ios::trunc);
  output.seekp(static_cast<std::streamoff>(mdlite::kSearchMaxFileBytes));
  output.put('x');
  return static_cast<bool>(output);
}

bool ReadDocument(const std::filesystem::path& path, std::wstring& text) {
  mdlite::Document document;
  std::wstring error;
  if (!document.Load(path, error)) return false;
  text = document.text();
  return true;
}

void TestSourcePositionsAndRegex() {
  const std::wstring source = L"漢😀alpha\r\n二行 beta\r\n";
  const std::wstring needle = L"😀alpha\r\n二行";
  mdlite::SearchQuery query;
  query.text = needle;
  query.match_case = true;
  std::vector<mdlite::SearchMatch> matches;
  std::wstring error;
  bool truncated{};
  Check(mdlite::SearchDocumentText(L"unicode.txt", source, query, matches, error, {}, &truncated),
        "multiline literal search succeeds");
  const auto expected_begin = source.find(L"😀");
  Check(matches.size() == 1, "multiline literal produces one match");
  if (!matches.empty()) {
    Check(matches[0].begin == expected_begin && matches[0].end == expected_begin + needle.size(),
          "source offsets preserve UTF-16 code units across emoji and CRLF");
    Check(matches[0].line == 1 && matches[0].column == expected_begin + 1,
          "line and column are one-based and CRLF-aware");
    Check(matches[0].preview == L"漢😀alpha" && matches[0].preview_match_begin == 1 &&
              matches[0].preview_match_end == matches[0].preview.size(),
          "single-line preview highlights the visible part of a multiline match");
    Check(matches[0].snapshot_hash == mdlite::SearchSnapshotHash(source),
          "match carries the exact searched snapshot hash");
  }

  query = {};
  query.text = L"(?<word>日本|語){2}(?<number>\\d+)";
  query.regular_expression = true;
  query.match_case = true;
  std::vector<mdlite::SearchRegexMatch> regex_matches;
  const std::wstring regex_source = L"始日本語123終";
  Check(mdlite::SearchRegexMatches(regex_source, query, regex_matches, error),
        "valid PCRE2 expression with repeated alternatives and captures succeeds");
  Check(regex_matches.size() == 1 && regex_matches[0].captures.size() >= 3,
        "regex results expose capture spans for replacement");
  if (!regex_matches.empty() && regex_matches[0].captures.size() >= 3) {
    const auto& word = regex_matches[0].captures[1];
    const auto& number = regex_matches[0].captures[2];
    Check(word.matched && regex_source.substr(word.begin, word.end - word.begin) == L"語" &&
              number.matched && regex_source.substr(number.begin, number.end - number.begin) == L"123",
          "repeated and trailing capture offsets follow PCRE2 output");
  }
  std::wstring replaced;
  std::size_t replacement_count{};
  Check(mdlite::ReplaceDocumentText(regex_source, query, L"$2-$1-$$-$&", replaced,
                                    replacement_count, error),
        "regex replacement accepts captures and literal dollar syntax");
  Check(replacement_count == 1 && replaced == L"始123-語-$-日本語123終",
        "regex replacement preserves $1, $2, $$, and $& formatting");

  query.text = L"b";
  replaced.clear();
  Check(mdlite::ReplaceDocumentText(L"abc", query, L"$`-$&-$'", replaced,
                                    replacement_count, error),
        "regex replacement accepts prefix and suffix formatting");
  Check(replaced == L"aa-b-cc", "$` and $' preserve the existing whole-source formatting semantics");

  query.text = L"\\b";
  Check(mdlite::SearchRegexMatches(L"a", query, regex_matches, error),
        "zero-width regex completes");
  Check(regex_matches.size() == 2 && regex_matches[0].begin == 0 && regex_matches[0].end == 0 &&
            regex_matches[1].begin == 1 && regex_matches[1].end == 1,
        "zero-width matches advance without looping or dropping the end boundary");

  query.text = L".";
  Check(mdlite::SearchDocumentText(L"emoji.txt", L"😀", query, matches, error) &&
            matches.size() == 1 && matches[0].begin == 0 && matches[0].end == 2,
        "Unicode regex dot returns a complete UTF-16 surrogate pair");
  query.text = L"\\C";
  Check(!mdlite::SearchDocumentText(L"emoji.txt", L"😀", query, matches, error) &&
            matches.empty() && !error.empty(),
        "raw-code-unit regex escape is rejected to prevent split-surrogate source offsets");

  const wchar_t high_surrogate = static_cast<wchar_t>(0xD83D);
  const wchar_t low_surrogate = static_cast<wchar_t>(0xDE00);
  std::wstring long_line(400, L'a');
  long_line[73] = high_surrogate;
  long_line[74] = low_surrogate;
  long_line[200] = L'x';
  long_line[327] = high_surrogate;
  long_line[328] = low_surrogate;
  query = {};
  query.text = L"x";
  query.match_case = true;
  Check(mdlite::SearchDocumentText(L"surrogate-preview.txt", long_line, query, matches, error) &&
            matches.size() == 1,
        "long-line search creates a single bounded-preview fixture");
  if (!matches.empty()) {
    const auto& match = matches[0];
    Check(match.begin == 200 && match.end == 201 && match.column == 201,
          "preview cropping preserves source match offsets and column");
    Check(match.preview.size() <= mdlite::kSearchMaxPreviewCodeUnits &&
              match.preview.front() == L'…' && match.preview.back() == L'…',
          "preview remains within its limit and marks both clipped sides");
    Check(static_cast<std::uint16_t>(match.preview[1]) != static_cast<std::uint16_t>(low_surrogate) &&
              static_cast<std::uint16_t>(match.preview[match.preview.size() - 2]) !=
                  static_cast<std::uint16_t>(high_surrogate),
          "preview crop moves inward instead of splitting supplementary code points");
    Check(match.preview_match_begin < match.preview.size() &&
              match.preview_match_end == match.preview_match_begin + 1 &&
              match.preview[match.preview_match_begin] == L'x',
          "surrogate-safe crop keeps preview highlight offsets aligned");
  }

  query.text.clear();
  Check(!mdlite::SearchDocumentText(L"unicode.txt", source, query, matches, error),
        "empty search text is rejected");
  Check(matches.empty() && !error.empty(), "empty query clears prior matches and reports an error");

  query.text = L"(";
  query.regular_expression = true;
  Check(!mdlite::SearchDocumentText(L"invalid.txt", L"prior match", query, matches, error),
        "invalid regex is rejected");
  Check(matches.empty() && !error.empty(), "invalid regex does not leave earlier results visible");

  query.text = L"(a|aa)+$";
  const std::wstring pathological(20'000, L'a');
  const auto start = std::chrono::steady_clock::now();
  const bool catastrophic_completed = mdlite::SearchDocumentText(
      L"pathological.txt", pathological + L"!", query, matches, error);
  Check(!catastrophic_completed,
        "catastrophic nested repetition is bounded");
  const auto elapsed = std::chrono::steady_clock::now() - start;
  Check(elapsed < std::chrono::seconds(2), "catastrophic regex exits promptly under match limits");

  int cancellation_checks{};
  query.text = L"(a|aa)*b";
  const bool cancelled_result = mdlite::SearchDocumentText(
      L"cancel.txt", pathological + L"!", query, matches, error,
      [&] { return ++cancellation_checks >= 3; });
  Check(!cancelled_result,
        "regex callout observes cancellation during matching");
  Check(error.find(L"中止") != std::wstring::npos,
        "regex cancellation is reported distinctly from a completed no-match search");
}

void TestUnicodeWholeWordBoundaries() {
  const auto check_modes = [](const char* name, std::wstring_view source,
                              std::wstring_view pattern,
                              std::vector<std::size_t> expected_starts) {
    for (const bool regular_expression : {false, true}) {
      mdlite::SearchQuery query{std::wstring(pattern), true, regular_expression};
      query.whole_word = true;
      std::vector<mdlite::SearchMatch> matches;
      std::wstring error;
      const std::string mode = std::string(name) +
          (regular_expression ? " regex whole-word search" : " literal whole-word search");
      Check(mdlite::SearchDocumentText(L"word-boundary.txt", source, query, matches, error),
            mode.c_str());
      Check(matches.size() == expected_starts.size(), mode.c_str());
      if (matches.size() != expected_starts.size()) continue;
      for (std::size_t index = 0; index < expected_starts.size(); ++index) {
        Check(matches[index].begin == expected_starts[index] &&
                  matches[index].end == expected_starts[index] + pattern.size(),
              mode.c_str());
      }
    }
  };

  check_modes("Japanese compound boundary", L"日本 日本語", L"日本", {0});
  check_modes("Greek compound boundary", L"αβ αβγ", L"αβ", {0});
  check_modes("Cyrillic compound boundary", L"тест тестовый", L"тест", {0});
  check_modes("Latin combining acute boundary", L"e\u0301 e", L"e", {3});
  check_modes("Japanese combining dakuten boundary", L"か\u3099 か", L"か", {3});
  check_modes("supplementary letter neighbors", L"\U00010400a a\U00010400 a", L"a", {8});
  check_modes("emoji non-word neighbors", L"\U0001F600cat\U0001F600 cat", L"cat", {2, 8});
}

void TestUnicodeWholeWordReplacementParity() {
  struct ReplacementCase {
    const char* name;
    const wchar_t* directory;
    std::wstring source;
    std::wstring pattern;
    std::wstring expected;
    std::string source_bytes;
    std::string expected_bytes;
    std::size_t match_begin{};
  };
  const std::vector<ReplacementCase> cases{
      {"Latin combining replacement", L"latin", L"e\u0301 e\n", L"e", L"e\u0301 X\n",
       "e\xCC\x81 e\n", "e\xCC\x81 X\n", 3},
      {"Japanese dakuten replacement", L"japanese", L"か\u3099 か\n", L"か", L"か\u3099 X\n",
       "\xE3\x81\x8B\xE3\x82\x99 か\n", "\xE3\x81\x8B\xE3\x82\x99 X\n", 3},
      {"supplementary letter replacement", L"supplementary",
       L"\U00010400a a\U00010400 a\n", L"a", L"\U00010400a a\U00010400 X\n",
       "\xF0\x90\x90\x80" "a a" "\xF0\x90\x90\x80" " a\n",
       "\xF0\x90\x90\x80" "a a" "\xF0\x90\x90\x80" " X\n", 8},
  };

  TemporaryDirectory temporary;
  for (const auto& test : cases) {
    mdlite::SearchQuery query{test.pattern, true, false};
    query.whole_word = true;
    std::vector<mdlite::SearchMatch> matches;
    std::wstring error;
    const std::string search_label = std::string(test.name) + " uses the expected search span";
    Check(mdlite::SearchDocumentText(L"unicode.txt", test.source, query, matches, error),
          search_label.c_str());
    Check(matches.size() == 1 && matches[0].begin == test.match_begin &&
              matches[0].end == test.match_begin + test.pattern.size(),
          search_label.c_str());

    std::wstring direct_output;
    std::size_t direct_count{};
    const std::string replace_label = std::string(test.name) + " direct replacement shares search boundaries";
    Check(mdlite::ReplaceDocumentText(test.source, query, L"X", direct_output,
                                      direct_count, error),
          replace_label.c_str());
    Check(direct_count == matches.size() && direct_output == test.expected,
          replace_label.c_str());

    std::wstring selected_output;
    std::size_t replaced_begin{}, replaced_end{};
    bool replaced{};
    Check(mdlite::ReplaceDocumentMatch(test.source, query, L"X", 0, selected_output,
                                       replaced_begin, replaced_end, replaced, error),
          replace_label.c_str());
    Check(replaced && replaced_begin == test.match_begin &&
              replaced_end == test.match_begin + 1 && selected_output == test.expected,
          replace_label.c_str());

    const auto root = temporary.path() / test.directory;
    const auto path = root / L"note.txt";
    Check(WriteBytes(path, test.source_bytes), "write Unicode whole-word replacement fixture");
    mdlite::ReplacePlan plan;
    Check(mdlite::PreviewWorkspaceReplace(root, query, L"X", {}, plan, error),
          "Unicode whole-word replacement preview succeeds");
    Check(plan.files.size() == 1 && plan.files[0].path == path &&
              plan.files[0].replacement_count == matches.size() &&
              plan.files[0].before == test.source && plan.files[0].after == test.expected &&
              plan.search_issues.empty(),
          "replacement preview count and snapshots match search exactly");

    mdlite::ReplaceApplyResult apply_result;
    Check(mdlite::ApplyWorkspaceReplace(root, plan, apply_result, error),
          "Unicode whole-word replacement applies");
    std::wstring applied_text;
    std::vector<unsigned char> applied_bytes;
    Check(ReadDocument(path, applied_text) && applied_text == test.expected &&
              ReadBytes(path, applied_bytes) &&
              std::string(applied_bytes.begin(), applied_bytes.end()) == test.expected_bytes,
          "apply writes the exact expected Unicode result");

    mdlite::ReplaceApplyResult rollback_result;
    Check(mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error),
          "Unicode whole-word replacement journal rolls back");
    std::wstring restored_text;
    std::vector<unsigned char> restored_bytes;
    Check(ReadDocument(path, restored_text) && restored_text == test.source &&
              ReadBytes(path, restored_bytes) &&
              std::string(restored_bytes.begin(), restored_bytes.end()) == test.source_bytes,
          "rollback restores the exact original Unicode text and bytes");
  }
}

void TestWorkspaceCoverageAndLimits() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"workspace";
  std::filesystem::create_directories(root / L"folder");
  Check(WriteBytes(root / L"notes.md", "needle markdown\n"), "write Markdown fixture");
  Check(WriteBytes(root / L"sheet.csv", "needle disk version\n"), "write CSV fixture");
  Check(WriteBytes(root / L"folder" / L"entry.log", "needle nested log\n"),
        "write nested log fixture");
  Check(WriteBytes(root / L".hidden.txt", "needle hidden\n"), "write hidden fixture");
  Check(WriteBytes(root / L"ignored.txt", "needle ignored\n"), "write ignored fixture");
  Check(WriteBytes(root / L".gitignore", "ignored.txt\n"), "write ignore fixture");
  Check(WriteBytes(root / L"binary.dat", std::string("text\0needle", 11)),
        "write binary fixture");
  Check(WriteBytes(root / L"large.txt",
                   std::string(static_cast<std::size_t>(mdlite::kSearchMaxFileBytes + 1), 'x')),
        "write oversized text fixture");

  std::map<std::filesystem::path, std::wstring> unsaved;
  unsaved.emplace(root / L"sheet.csv", L"dirty buffer has no match\r\n");
  unsaved.emplace(root / L"new.txt", L"needle new unsaved document\r\n");
  unsaved.emplace(temporary.path() / L"outside.txt", L"needle must stay outside\r\n");

  mdlite::SearchQuery query;
  query.text = L"needle";
  query.match_case = true;
  query.include_globs = {L""};
  std::vector<mdlite::SearchMatch> matches;
  std::vector<mdlite::SearchIssue> issues;
  std::wstring error;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues),
        "workspace search completes with explicit partial issues");
  Check(matches.size() == 3, "workspace search includes arbitrary text and unsaved-only files once");
  bool found_markdown{}, found_log{}, found_new{}, found_dirty{}, found_hidden{}, found_ignored{};
  for (const auto& match : matches) {
    const auto name = match.path.filename().wstring();
    found_markdown |= name == L"notes.md";
    found_log |= name == L"entry.log";
    found_new |= name == L"new.txt";
    found_dirty |= name == L"sheet.csv";
    found_hidden |= name == L".hidden.txt";
    found_ignored |= name == L"ignored.txt";
    Check(match.path.is_absolute(), "workspace result path is absolute");
  }
  Check(found_markdown && found_log && found_new,
        "empty include globs search Markdown, non-Markdown, and unsaved new documents");
  Check(!found_dirty && !found_hidden && !found_ignored,
        "dirty text overrides disk, while hidden and gitignored files remain excluded");
  Check(issues.size() >= 2, "binary and oversized files are surfaced as partial-result issues");

  query.include_globs.clear();
  query.include_hidden = true;
  query.respect_gitignore = false;
  issues.clear();
  Check(mdlite::SearchWorkspace(root, query, {}, matches, error, {}, {}, &issues),
        "ignore and hidden exclusions can be disabled");
  found_hidden = false;
  found_ignored = false;
  for (const auto& match : matches) {
    found_hidden |= match.path.filename().wstring() == L".hidden.txt";
    found_ignored |= match.path.filename().wstring() == L"ignored.txt";
  }
  Check(found_hidden && found_ignored, "include-hidden and ignore switches affect results");

  query.include_hidden = false;
  query.respect_gitignore = true;
  query.include_globs = {L"folder\\*.log"};
  issues.clear();
  Check(mdlite::SearchWorkspace(root, query, {}, matches, error, {}, {}, &issues),
        "backslash path separator glob search completes");
  Check(matches.size() == 1 && matches[0].path.filename() == L"entry.log",
        "include glob normalizes separators and selects nested non-Markdown file");

  query.include_globs = {L"**/*.csv"};
  issues.clear();
  Check(mdlite::SearchWorkspace(root, query, {}, matches, error, {}, {}, &issues),
        "recursive glob search completes");
  Check(matches.size() == 1 && matches[0].path.filename() == L"sheet.csv",
        "recursive glob may match a file directly under the workspace root");

  query.include_globs = {std::wstring(257, L'a')};
  Check(!mdlite::SearchWorkspace(root, query, {}, matches, error, {}, {}, &issues),
        "overlong glob is rejected before workspace traversal");
  Check(matches.empty() && !error.empty(), "invalid glob clears matches and reports an error");

  mdlite::SearchQuery result_limit;
  result_limit.text = L"x";
  std::vector<mdlite::SearchMatch> many;
  bool truncated{};
  std::wstring repeated;
  repeated.reserve((mdlite::kSearchMaxMatches + 2) * 2);
  for (std::size_t index = 0; index < mdlite::kSearchMaxMatches + 2; ++index)
    repeated += L"x\n";
  Check(!mdlite::SearchDocumentText(L"many.txt", repeated, result_limit, many, error, {}, &truncated),
        "document search reports a result cap");
  Check(truncated && many.size() == mdlite::kSearchMaxMatches,
        "match-count cap retains a bounded partial list and exposes truncation");
}

void TestNativeHiddenSearchEligibility() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"native-hidden-workspace";
  const auto hidden_file = root / L"native-hidden.txt";
  const auto hidden_directory = root / L"native-hidden-directory";
  const auto visible_child = hidden_directory / L"child.txt";
  const auto dirty_child = hidden_directory / L"dirty.txt";
  const auto unsaved_only_child = hidden_directory / L"new-buffer.txt";
  Check(WriteBytes(hidden_file, "nativehiddenword\n"), "write native hidden-file fixture");
  Check(WriteBytes(visible_child, "nativehiddenword\n"), "write hidden-directory child fixture");
  Check(WriteBytes(dirty_child, "diskonlyword\n"), "write disk version for hidden dirty buffer");
  Check(SetHiddenAttribute(hidden_file, true) && SetHiddenAttribute(hidden_directory, true),
        "set native Windows hidden attributes on a file and ancestor directory");
  const DWORD hidden_file_attributes = GetFileAttributesW(hidden_file.c_str());
  const DWORD hidden_directory_attributes = GetFileAttributesW(hidden_directory.c_str());
  Check(hidden_file_attributes != INVALID_FILE_ATTRIBUTES &&
            (hidden_file_attributes & FILE_ATTRIBUTE_HIDDEN) != 0 &&
            hidden_directory_attributes != INVALID_FILE_ATTRIBUTES &&
            (hidden_directory_attributes & FILE_ATTRIBUTE_HIDDEN) != 0,
        "fixtures carry real FILE_ATTRIBUTE_HIDDEN attributes");

  std::map<std::filesystem::path, std::wstring> unsaved{
      {dirty_child, L"bufferword\n"},
      {unsaved_only_child, L"orphanword\n"},
  };
  mdlite::SearchQuery query{L"nativehiddenword", true, false};
  std::vector<mdlite::SearchMatch> matches;
  std::vector<mdlite::SearchIssue> issues;
  std::wstring error;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.empty(),
        "workspace search skips native hidden files and ancestor directories by default");

  std::vector<mdlite::SearchMatch> explicit_matches;
  Check(mdlite::SearchDocumentText(hidden_file, L"nativehiddenword\n", query,
                                   explicit_matches, error) && explicit_matches.size() == 1,
        "explicit current-document search remains available for a native hidden file");

  query.include_hidden = true;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.size() == 2,
        "include-hidden searches native hidden files and directories");
  bool found_hidden_file{}, found_hidden_directory_child{};
  for (const auto& match : matches) {
    found_hidden_file |= match.path == hidden_file;
    found_hidden_directory_child |= match.path == visible_child;
  }
  Check(found_hidden_file && found_hidden_directory_child,
        "include-hidden results include both native hidden path types");

  query.text = L"bufferword";
  query.include_hidden = false;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.empty(),
        "a dirty buffer below a native hidden ancestor remains excluded by default");
  query.include_hidden = true;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.size() == 1 && matches[0].path == dirty_child &&
            matches[0].snapshot_hash == mdlite::SearchSnapshotHash(L"bufferword\n"),
        "include-hidden keeps dirty-buffer text ahead of its disk snapshot");

  query.text = L"diskonlyword";
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.empty(),
        "included hidden dirty files do not fall back to stale disk text");
  query.text = L"orphanword";
  query.include_hidden = false;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.empty(),
        "unsaved-only buffers below hidden ancestors remain excluded by default");
  query.include_hidden = true;
  Check(mdlite::SearchWorkspace(root, query, unsaved, matches, error, {}, {}, &issues) &&
            matches.size() == 1 && matches[0].path == unsaved_only_child,
        "include-hidden admits unsaved-only buffers below hidden ancestors");

  Check(SetHiddenAttribute(hidden_file, false) && SetHiddenAttribute(hidden_directory, false),
        "clear native hidden attributes from the owned fixtures");
}

void TestBoundedSearchResultPayload() {
  mdlite::SearchQuery query{L"x", true, false};
  std::wstring error;
  const auto make_source = [](std::size_t matches) {
    std::string source;
    source.reserve(matches * 2);
    for (std::size_t index = 0; index < matches; ++index) source += "x ";
    return source;
  };
  const auto as_wide = [](const std::string& source) {
    return std::wstring(source.begin(), source.end());
  };
  const std::filesystem::path direct_path = L"direct-preview.txt";
  const std::size_t per_match_estimate = sizeof(mdlite::SearchMatch) +
      direct_path.native().size() * sizeof(wchar_t) +
      mdlite::kSearchMaxPreviewCodeUnits * sizeof(wchar_t);
  const std::size_t direct_matches = std::min(
      mdlite::kSearchMaxMatches,
      mdlite::kSearchMaxResultPayloadBytes / per_match_estimate + 2);
  const auto direct_text = as_wide(make_source(direct_matches));
  std::vector<mdlite::SearchMatch> matches;
  bool truncated{};
  Check(!mdlite::SearchDocumentText(direct_path, direct_text, query, matches, error, {}, &truncated),
        "single-document search reports the result-payload cap");
  Check(truncated && !matches.empty() && matches.size() < direct_matches &&
            error.find(L"8 MiB") != std::wstring::npos,
        "payload cap preserves partial results and reports its byte limit");
  std::size_t direct_payload_bytes{};
  bool previews_bounded = true;
  for (const auto& match : matches) {
    direct_payload_bytes += sizeof(mdlite::SearchMatch) +
        match.path.native().size() * sizeof(wchar_t) + match.preview.size() * sizeof(wchar_t);
    previews_bounded &= match.preview.size() <= mdlite::kSearchMaxPreviewCodeUnits &&
        match.preview_match_begin < match.preview.size() &&
        match.preview_match_end == match.preview_match_begin + 1 &&
        match.preview[match.preview_match_begin] == L'x';
  }
  Check(previews_bounded, "long-line previews stay bounded and retain exact highlight offsets");
  Check(direct_payload_bytes <= mdlite::kSearchMaxResultPayloadBytes,
        "single-document result object, path, and preview payload stays within 8 MiB");

  const std::filesystem::path long_display_path(std::wstring(1'024, L'p'));
  const std::size_t long_path_per_match = sizeof(mdlite::SearchMatch) +
      long_display_path.native().size() * sizeof(wchar_t) + sizeof(wchar_t);
  const std::size_t expected_long_path_matches =
      mdlite::kSearchMaxResultPayloadBytes / long_path_per_match;
  std::wstring short_line_source;
  short_line_source.reserve((mdlite::kSearchMaxMatches + 2) * 2);
  for (std::size_t index = 0; index < mdlite::kSearchMaxMatches + 2; ++index)
    short_line_source += L"x\n";
  Check(!mdlite::SearchDocumentText(long_display_path, short_line_source, query,
                                    matches, error, {}, &truncated),
        "single-document result accounting includes the owned path size");
  std::size_t long_path_payload_bytes{};
  for (const auto& match : matches)
    long_path_payload_bytes += sizeof(mdlite::SearchMatch) +
        match.path.native().size() * sizeof(wchar_t) + match.preview.size() * sizeof(wchar_t);
  Check(truncated && matches.size() == expected_long_path_matches &&
            long_path_payload_bytes <= mdlite::kSearchMaxResultPayloadBytes &&
            error.find(L"8 MiB") != std::wstring::npos,
        "large display paths consume the same explicit result-payload budget");

  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"result-payload-workspace";
  const auto first_path = root / L"a.txt";
  const auto second_path = root / L"b.txt";
  const auto path_bytes = std::max(first_path.native().size(), second_path.native().size()) *
                          sizeof(wchar_t);
  const std::size_t workspace_per_match_estimate = sizeof(mdlite::SearchMatch) + path_bytes +
      mdlite::kSearchMaxPreviewCodeUnits * sizeof(wchar_t);
  const std::size_t per_file_matches = std::min(
      (mdlite::kSearchMaxMatches - 1) / 2,
      (mdlite::kSearchMaxResultPayloadBytes / workspace_per_match_estimate) * 2 / 3);
  Check(per_file_matches > 0, "workspace payload fixture has a bounded match count");
  Check(WriteBytes(first_path, make_source(per_file_matches)), "write first payload-limit fixture");
  Check(WriteBytes(second_path, make_source(per_file_matches)), "write second payload-limit fixture");
  std::size_t progress_matches{};
  bool progress_previews_bounded = true;
  const auto progress = [&](std::vector<mdlite::SearchMatch> batch) {
    progress_matches += batch.size();
    for (const auto& match : batch)
      progress_previews_bounded &= match.preview.size() <= mdlite::kSearchMaxPreviewCodeUnits;
  };
  std::vector<mdlite::SearchIssue> issues;
  Check(mdlite::SearchWorkspace(root, query, {}, matches, error, {}, progress, &issues),
        "workspace search returns explicit partial results when payload is full");
  std::size_t workspace_payload_bytes{};
  for (const auto& match : matches)
    workspace_payload_bytes += sizeof(mdlite::SearchMatch) +
        match.path.native().size() * sizeof(wchar_t) + match.preview.size() * sizeof(wchar_t);
  Check(!matches.empty() && progress_matches == matches.size() && progress_previews_bounded,
        "progress batches match bounded workspace results");
  Check(workspace_payload_bytes <= mdlite::kSearchMaxResultPayloadBytes,
        "workspace result object, path, and preview payload stays within 8 MiB across files");
  Check(issues.size() == 1 && issues[0].kind == mdlite::SearchIssueKind::Error &&
            issues[0].message.find(L"8 MiB") != std::wstring::npos,
        "workspace payload cap stops scanning and reports an explicit partial-result issue");
}

void TestReplaceSnapshotsAndJournalSafety() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"replace-workspace";
  const auto path = root / L"notes.txt";
  Check(WriteBytes(path, "alpha alpha\n"), "write replacement source");
  mdlite::SearchQuery query;
  query.text = L"alpha";
  query.match_case = true;
  mdlite::ReplacePlan plan;
  std::wstring error;
  Check(mdlite::PreviewWorkspaceReplace(root, query, L"beta", {}, plan, error),
        "workspace replace preview succeeds");
  Check(plan.files.size() == 1 && plan.files[0].replacement_count == 2 &&
            plan.files[0].before == L"alpha alpha\n" && plan.files[0].after == L"beta beta\n",
        "replacement preview stores exact before/after source snapshots");

  Check(WriteBytes(path, "external alpha\n"), "edit source after preview");
  mdlite::ReplaceApplyResult apply_result;
  Check(!mdlite::ApplyWorkspaceReplace(root, plan, apply_result, error),
        "apply refuses a stale disk snapshot");
  Check(apply_result.conflicts.size() == 1 && !apply_result.journal.empty(),
        "stale apply reports its conflict and keeps a local recovery journal");
  std::wstring after_conflict;
  Check(ReadDocument(path, after_conflict) && after_conflict == L"external alpha\n",
        "stale apply leaves the external edit untouched");
  mdlite::ReplaceRollbackPreview stale_preview;
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, apply_result.journal,
                                                  stale_preview, error) &&
            stale_preview.files.empty(),
        "read-only rollback preview rejects externally changed content");
  mdlite::ReplaceApplyResult rollback_result;
  Check(!mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error) &&
            !rollback_result.conflicts.empty(),
        "rollback refuses to overwrite content changed since the preview");
  Check(ReadDocument(path, after_conflict) && after_conflict == L"external alpha\n",
        "conflicted rollback leaves current file content intact");

  Check(mdlite::PreviewWorkspaceReplace(root, query, L"beta", {}, plan, error),
        "fresh preview accepts the new source version");
  Check(mdlite::ApplyWorkspaceReplace(root, plan, apply_result, error),
        "apply writes a fresh preview plan");
  std::wstring applied;
  Check(ReadDocument(path, applied) && applied == L"external beta\n",
        "apply changes only the matching current text");
  mdlite::ReplaceRollbackPreview v2_preview;
  Check(mdlite::PreviewWorkspaceReplaceRollback(root, apply_result.journal,
                                                 v2_preview, error),
        "read-only preview accepts a current V2 rollback journal");
  Check(v2_preview.owning_workspace == std::filesystem::absolute(root).lexically_normal() &&
            v2_preview.files.size() == 1 && v2_preview.files[0].path == path &&
            v2_preview.files[0].expected_current == L"external beta\n" &&
            v2_preview.files[0].restore_text == L"external alpha\n",
        "V2 preview exposes its owner and exact after/before snapshots");
  mdlite::ReplaceRollbackPreview wrong_workspace_preview;
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root / L"wrong-workspace",
                                                  apply_result.journal,
                                                  wrong_workspace_preview, error) &&
            wrong_workspace_preview.files.empty(),
        "rollback preview rejects a journal owned by another workspace");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, temporary.path() / L"outside.journal",
                                                  wrong_workspace_preview, error) &&
            wrong_workspace_preview.files.empty(),
        "rollback preview rejects a journal path outside the workspace state area");
  Check(mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error),
        "V2 local journal restores an unchanged after-snapshot");
  std::wstring rolled_back;
  Check(ReadDocument(path, rolled_back) && rolled_back == L"external alpha\n",
        "rollback restores exact pre-replacement source text");

  const auto first_cancelled = root / L"cancel-a.txt";
  const auto second_cancelled = root / L"cancel-b.txt";
  Check(WriteBytes(first_cancelled, "cat"), "write first cancellable replacement source");
  Check(WriteBytes(second_cancelled, "cat"), "write second cancellable replacement source");
  query.text = L"cat";
  Check(mdlite::PreviewWorkspaceReplace(root, query, L"dog", {}, plan, error),
        "cancellable replacement preview succeeds");
  std::size_t cancellation_checks{};
  Check(!mdlite::ApplyWorkspaceReplace(root, plan, apply_result, error,
                                       [&] { return ++cancellation_checks > 1; }),
        "apply cancellation is observed between file writes");
  Check(apply_result.applied_files == 1 && !apply_result.journal.empty(),
        "partial apply records its completed file in the recovery journal");
  mdlite::ReplaceRollbackPreview partial_preview;
  Check(mdlite::PreviewWorkspaceReplaceRollback(root, apply_result.journal,
                                                 partial_preview, error),
        "read-only preview accepts a partially applied V2 journal");
  Check(partial_preview.files.size() == 1 &&
            partial_preview.files[0].path == first_cancelled &&
            partial_preview.files[0].expected_current == L"dog" &&
            partial_preview.files[0].restore_text == L"cat",
        "partial journal preview omits the file that was never applied");
  std::wstring first_text, second_text;
  Check(ReadDocument(first_cancelled, first_text) && first_text == L"dog" &&
            ReadDocument(second_cancelled, second_text) && second_text == L"cat",
        "cancellation changes only the file completed before the stop");
  Check(mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error),
        "partial cancellation journal restores only its completed file");
  Check(ReadDocument(first_cancelled, first_text) && first_text == L"cat" &&
            ReadDocument(second_cancelled, second_text) && second_text == L"cat",
        "partial cancellation rollback leaves the untouched file unchanged");

  mdlite::ReplacePlan unsafe;
  unsafe.query = query;
  unsafe.replacement = L"outside";
  unsafe.files.push_back({temporary.path() / L"outside.txt", L"alpha", L"outside", 1});
  Check(!mdlite::ApplyWorkspaceReplace(root, unsafe, apply_result, error),
        "apply rejects a crafted target outside the workspace before journal creation");
  Check(!mdlite::RollbackWorkspaceReplace(temporary.path() / L"outside.journal",
                                          rollback_result, error),
        "rollback rejects a journal outside the workspace state directory");
}

void TestSelectedMatchWorkspaceApplyValidation() {
  struct SelectedCase {
    const char* name;
    const wchar_t* directory;
    std::string source_bytes;
    std::wstring source;
    mdlite::SearchQuery query;
    std::wstring replacement;
    std::size_t match_index{};
    std::wstring expected_after;
  };
  const std::vector<SelectedCase> cases{
      {"literal", L"literal", "cat cat cat\n", L"cat cat cat\n",
       mdlite::SearchQuery{L"cat", true, false}, L"kitten", 1, L"cat kitten cat\n"},
      {"capture", L"captures", "cat cat\n", L"cat cat\n",
       mdlite::SearchQuery{L"(cat)", true, true}, L"[$1]", 1, L"cat [cat]\n"},
      {"zero-width", L"zero-width", "cat", L"cat",
       mdlite::SearchQuery{L"\\b", true, true}, L"X", 1, L"catX"},
      {"same-begin-zero", L"same-begin-zero", "a", L"a",
       mdlite::SearchQuery{L"(?:|a)", true, true}, L"X", 0, L"Xa"},
      {"same-begin-nonempty", L"same-begin-nonempty", "a", L"a",
       mdlite::SearchQuery{L"(?:|a)", true, true}, L"X", 1, L"X"},
  };

  TemporaryDirectory temporary;
  std::wstring error;
  const auto make_selected_plan = [&](const std::filesystem::path& root,
                                      const std::filesystem::path& path,
                                      const SelectedCase& test,
                                      mdlite::ReplacePlan& selected_plan) {
    mdlite::ReplacePlan full_plan;
    if (!mdlite::PreviewWorkspaceReplace(root, test.query, test.replacement, {}, full_plan, error) ||
        full_plan.files.size() != 1)
      return false;
    std::vector<mdlite::SearchMatch> matches;
    if (!mdlite::SearchDocumentText(path, full_plan.files[0].before, test.query, matches, error) ||
        test.match_index >= matches.size())
      return false;
    const auto& match = matches[test.match_index];
    std::wstring after;
    std::size_t replaced_begin{}, replaced_end{};
    bool replaced{};
    if (!mdlite::ReplaceDocumentMatch(full_plan.files[0].before, test.query, test.replacement,
                                      match.begin, match.end, after, replaced_begin,
                                      replaced_end, replaced, error) ||
        !replaced || replaced_begin != match.begin)
      return false;
    mdlite::ReplaceFilePlan file = full_plan.files.front();
    file.after = std::move(after);
    file.replacement_count = 1;
    file.selected_match = mdlite::ReplaceMatchSelection{
        match.begin, match.end, match.snapshot_hash};
    selected_plan = {test.query, test.replacement, {std::move(file)},
                     std::move(full_plan.search_issues)};
    return true;
  };

  for (const auto& test : cases) {
    const auto root = temporary.path() / test.directory;
    const auto path = root / L"note.txt";
    const std::string label = std::string("selected ") + test.name;
    Check(WriteBytes(path, test.source_bytes), "write selected-match source");
    mdlite::ReplacePlan selected_plan;
    Check(make_selected_plan(root, path, test, selected_plan),
          label.c_str());
    Check(selected_plan.files.size() == 1 && selected_plan.files[0].replacement_count == 1 &&
              selected_plan.files[0].selected_match.has_value() &&
              selected_plan.files[0].after == test.expected_after,
          label.c_str());

    mdlite::ReplaceApplyResult apply_result;
    Check(mdlite::ApplyWorkspaceReplace(root, selected_plan, apply_result, error),
          label.c_str());
    std::wstring after_apply;
    Check(ReadDocument(path, after_apply) && after_apply == test.expected_after &&
              apply_result.applied_files == 1 && apply_result.applied_replacements == 1,
          label.c_str());
    mdlite::ReplaceApplyResult rollback_result;
    Check(mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error),
          label.c_str());
    std::wstring after_rollback;
    Check(ReadDocument(path, after_rollback) && after_rollback == test.source,
          label.c_str());
  }

  const auto tamper_root = temporary.path() / L"tampered-selected";
  const auto tamper_path = tamper_root / L"note.txt";
  const SelectedCase& literal = cases.front();
  Check(WriteBytes(tamper_path, literal.source_bytes), "write selected-plan validation source");
  mdlite::ReplacePlan valid_selected;
  Check(make_selected_plan(tamper_root, tamper_path, literal, valid_selected),
        "create a valid selected plan for tampering checks");

  const auto rejects_without_write = [&](mdlite::ReplacePlan altered, const char* message) {
    mdlite::ReplaceApplyResult result;
    error.clear();
    Check(!mdlite::ApplyWorkspaceReplace(tamper_root, altered, result, error) &&
              result.journal.empty() && result.applied_files == 0,
          message);
    std::wstring current;
    Check(ReadDocument(tamper_path, current) && current == literal.source,
          "invalid selected plan leaves the current file unchanged");
  };

  auto bad_begin = valid_selected;
  ++bad_begin.files[0].selected_match->begin;
  rejects_without_write(std::move(bad_begin), "apply rejects a tampered selected begin offset");
  auto bad_end = valid_selected;
  ++bad_end.files[0].selected_match->end;
  rejects_without_write(std::move(bad_end), "apply rejects a tampered selected end offset");
  auto bad_hash = valid_selected;
  bad_hash.files[0].selected_match->snapshot_hash ^= 1;
  rejects_without_write(std::move(bad_hash), "apply rejects a tampered selected snapshot hash");
  auto bad_after = valid_selected;
  bad_after.files[0].after = L"kitten kitten kitten\n";
  rejects_without_write(std::move(bad_after), "apply rejects a selected after-snapshot with extra replacements");

  Check(WriteBytes(tamper_path, "external cat cat cat\n"),
        "edit selected-plan source after its snapshot");
  mdlite::ReplaceApplyResult stale_result;
  Check(!mdlite::ApplyWorkspaceReplace(tamper_root, valid_selected, stale_result, error) &&
            stale_result.conflicts.size() == 1 && stale_result.applied_files == 0,
        "selected-plan apply still rejects a stale disk source");
  std::wstring external_text;
  Check(ReadDocument(tamper_path, external_text) && external_text == L"external cat cat cat\n",
        "stale selected-plan rejection leaves external edits unchanged");

  const auto& same_begin_query = cases.back().query;
  std::wstring mismatched_output = L"must be cleared";
  std::size_t mismatched_begin{}, mismatched_end{};
  bool mismatched_replaced{true};
  Check(!mdlite::ReplaceDocumentMatch(L"aa", same_begin_query, L"X", 0, 2,
                                      mismatched_output, mismatched_begin, mismatched_end,
                                      mismatched_replaced, error),
        "exact-span replacement rejects a span that search did not produce");
  Check(!mismatched_replaced && mismatched_output.empty() && !error.empty(),
        "rejected exact span leaves no replacement output");
}

void TestReplaceIgnoresOnlyKnownSafeExclusions() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"image-workspace";
  const auto note = root / L"cat.md";
  const auto image = root / L"image.png";
  const std::vector<unsigned char> png_bytes{
      0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
      0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
      0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
      0x08, 0x04, 0x00, 0x00, 0x00, 0xB5, 0x1C, 0x0C,
      0x02, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41,
      0x54, 0x78, 0xDA, 0x63, 0xFC, 0xFF, 0x1F, 0x00,
      0x03, 0x03, 0x02, 0x00, 0xEF, 0xA3, 0xE5, 0x76,
      0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
      0xAE, 0x42, 0x60, 0x82};
  std::string png_payload;
  for (const auto byte : png_bytes) png_payload.push_back(static_cast<char>(byte));
  Check(WriteBytes(note, "cat\n"), "write Markdown source beside a binary image");
  Check(WriteBytes(image, png_payload), "write valid 1x1 PNG binary fixture");
  std::vector<unsigned char> image_before;
  Check(ReadBytes(image, image_before) && image_before == png_bytes,
        "read the original image bytes before replacement");

  mdlite::SearchQuery query{L"cat", true, false};
  mdlite::ReplacePlan plan;
  std::wstring error;
  Check(mdlite::PreviewWorkspaceReplace(root, query, L"dog", {}, plan, error),
        "binary image exclusion does not block safe text replacement preview");
  Check(plan.files.size() == 1 && plan.files[0].path == note &&
            plan.files[0].replacement_count == 1,
        "preview contains only the matching Markdown file");
  Check(plan.search_issues.size() == 1 &&
            plan.search_issues[0].kind == mdlite::SearchIssueKind::Excluded &&
            plan.search_issues[0].path == image && !plan.search_issues[0].message.empty(),
        "preview retains the excluded image path and reason");

  mdlite::ReplaceApplyResult apply_result;
  Check(mdlite::ApplyWorkspaceReplace(root, plan, apply_result, error),
        "safe text replacement applies beside the excluded image");
  std::wstring text_after_apply;
  Check(ReadDocument(note, text_after_apply) && text_after_apply == L"dog\n",
        "apply changes the Markdown file");
  std::vector<unsigned char> image_after_apply;
  Check(ReadBytes(image, image_after_apply) && image_after_apply == image_before,
        "apply leaves the binary image byte-for-byte unchanged");

  mdlite::ReplaceApplyResult rollback_result;
  Check(mdlite::RollbackWorkspaceReplace(apply_result.journal, rollback_result, error),
        "replacement journal rolls back with the excluded image present");
  std::wstring text_after_rollback;
  Check(ReadDocument(note, text_after_rollback) && text_after_rollback == L"cat\n",
        "rollback restores the Markdown file");
  std::vector<unsigned char> image_after_rollback;
  Check(ReadBytes(image, image_after_rollback) && image_after_rollback == image_before,
        "rollback leaves the binary image byte-for-byte unchanged");
}

void TestReplaceStillRejectsSearchErrorsAndLimits() {
  const mdlite::SearchIssue legacy_issue{L"legacy.txt", L"legacy issue"};
  Check(legacy_issue.kind == mdlite::SearchIssueKind::Error,
        "existing two-field SearchIssue aggregate defaults to Error");

  TemporaryDirectory temporary;
  mdlite::SearchQuery query{L"cat", true, false};
  std::wstring error;
  mdlite::ReplacePlan plan;

  const auto oversized_root = temporary.path() / L"oversized-workspace";
  const auto oversized_file = oversized_root / L"large.txt";
  Check(WriteBytes(oversized_root / L"cat.md", "cat\n"),
        "write Markdown source beside an over-limit file");
  Check(WriteOversizedFile(oversized_file), "write an over-limit workspace fixture");
  Check(!mdlite::PreviewWorkspaceReplace(oversized_root, query, L"dog", {}, plan, error),
        "over-limit search issue still blocks replacement preview");
  Check(plan.files.empty() && plan.search_issues.size() == 1 &&
            plan.search_issues[0].kind == mdlite::SearchIssueKind::Error &&
            plan.search_issues[0].path == oversized_file,
        "over-limit error remains visible in the failed replacement plan");

  const auto unreadable_root = temporary.path() / L"unreadable-workspace";
  const auto unreadable_file = unreadable_root / L"locked.txt";
  Check(WriteBytes(unreadable_root / L"cat.md", "cat\n"),
        "write Markdown source beside an unreadable file");
  Check(WriteBytes(unreadable_file, "locked content\n"), "write an unreadable file fixture");
  HANDLE locked = CreateFileW(unreadable_file.c_str(), GENERIC_READ, 0, nullptr, OPEN_EXISTING,
                              FILE_ATTRIBUTE_NORMAL, nullptr);
  Check(locked != INVALID_HANDLE_VALUE, "open a no-sharing handle for the unreadable fixture");
  error.clear();
  const bool unreadable_preview = locked != INVALID_HANDLE_VALUE &&
      mdlite::PreviewWorkspaceReplace(unreadable_root, query, L"dog", {}, plan, error);
  if (locked != INVALID_HANDLE_VALUE) CloseHandle(locked);
  Check(!unreadable_preview, "unreadable file still blocks replacement preview");
  Check(plan.files.empty() && plan.search_issues.size() == 1 &&
            plan.search_issues[0].kind == mdlite::SearchIssueKind::Error &&
            plan.search_issues[0].path == unreadable_file,
        "unreadable error remains visible in the failed replacement plan");
}

void TestV1RollbackJournalPreview() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"legacy-rollback-workspace";
  const auto target = root / L"note.txt";
  const auto journal = root / L".mdlite/.state/replace/legacy.journal";
  Check(WriteBytes(target, "after text\n"), "write the current V1 rollback target");
  Check(WriteV1Journal(journal, target, L"before text\n", L"after text\n"),
        "write a bounded V1 rollback journal fixture");

  mdlite::ReplaceRollbackPreview preview;
  std::wstring error;
  Check(mdlite::PreviewWorkspaceReplaceRollback(root, journal, preview, error),
        "read-only preview accepts a valid V1 journal");
  Check(preview.owning_workspace == std::filesystem::absolute(root).lexically_normal() &&
            preview.journal == std::filesystem::absolute(journal).lexically_normal() &&
            preview.files.size() == 1 && preview.files[0].path == target &&
            preview.files[0].expected_current == L"after text\n" &&
            preview.files[0].restore_text == L"before text\n",
        "V1 preview returns its owner and exact rollback snapshots");
  std::wstring unchanged;
  Check(ReadDocument(target, unchanged) && unchanged == L"after text\n" &&
            std::filesystem::exists(journal),
        "read-only preview leaves the target and journal untouched");

  mdlite::ReplaceApplyResult rollback_result;
  Check(mdlite::RollbackWorkspaceReplace(journal, rollback_result, error),
        "existing rollback applies the V1 preview");
  std::wstring restored;
  Check(ReadDocument(target, restored) && restored == L"before text\n",
        "V1 rollback restores the original content");
  Check(mdlite::PreviewWorkspaceReplaceRollback(root, journal, preview, error) &&
            preview.files.empty(),
        "already restored and unapplied V1 entries are omitted from preview");
}

void TestRollbackPreviewRejectsUnsafeAndMalformedJournals() {
  TemporaryDirectory temporary;
  const auto root = temporary.path() / L"rollback-validation-workspace";
  const auto target = root / L"note.txt";
  const auto journal_directory = root / L".mdlite/.state/replace";
  const auto good_journal = journal_directory / L"good.journal";
  Check(WriteBytes(target, "after\n"), "write current safe rollback target");
  Check(WriteV1Journal(good_journal, target, L"before\n", L"after\n"),
        "write safe journal for validation cases");

  mdlite::ReplaceRollbackPreview preview;
  std::wstring error;
  Check(!mdlite::PreviewWorkspaceReplaceRollback(temporary.path() / L"wrong-workspace",
                                                  good_journal, preview, error) &&
            preview.files.empty(),
        "preview rejects a caller Workspace that does not own the journal");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, temporary.path() / L"outside.journal",
                                                  preview, error) && preview.files.empty(),
        "preview rejects a journal path outside the safe state directory");

  const auto tampered_journal = journal_directory / L"tampered.journal";
  std::string tampered_bytes = "MDLITE_REPLACE_JOURNAL_V2\r\n";
  const std::uint64_t empty_entry_count{};
  tampered_bytes.append(reinterpret_cast<const char*>(&empty_entry_count),
                        sizeof(empty_entry_count));
  tampered_bytes += "unexpected trailing data";
  Check(WriteBytes(tampered_journal, tampered_bytes), "write a tampered V2 journal fixture");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, tampered_journal, preview, error) &&
            preview.files.empty() && !error.empty(),
        "preview rejects tampered V2 journals without publishing a partial result");

  const auto count_limited_journal = journal_directory / L"count-limit.journal";
  std::string count_limited_bytes = "MDLITE_REPLACE_JOURNAL_V2\r\n";
  const std::uint64_t excessive_entry_count = mdlite::kSearchMaxMatches + 1;
  count_limited_bytes.append(reinterpret_cast<const char*>(&excessive_entry_count),
                             sizeof(excessive_entry_count));
  Check(WriteBytes(count_limited_journal, count_limited_bytes),
        "write a bounded V2 entry-count limit fixture");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, count_limited_journal, preview, error) &&
            preview.files.empty(),
        "preview rejects entry counts over the existing memory limit");

  const auto text_limited_journal = journal_directory / L"text-limit.journal";
  std::string text_limited_bytes = "MDLITE_REPLACE_JOURNAL_V2\r\n";
  const std::uint64_t one_entry = 1;
  const std::uint64_t path_length = 1;
  const std::uint64_t excessive_text_length =
      mdlite::kSearchMaxReplacePlanBytes / sizeof(wchar_t) + 1;
  const std::uint64_t no_after_text{};
  for (const auto value : {one_entry, path_length, excessive_text_length, no_after_text})
    text_limited_bytes.append(reinterpret_cast<const char*>(&value), sizeof(value));
  Check(WriteBytes(text_limited_journal, text_limited_bytes),
        "write a V2 text-length limit fixture");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, text_limited_journal, preview, error) &&
            preview.files.empty(),
        "preview rejects journal text lengths over the existing memory limit before allocation");

  const auto outside_target = temporary.path() / L"outside.txt";
  Check(WriteBytes(outside_target, "after\n"), "write a path-outside target fixture");
  const auto outside_target_journal = journal_directory / L"outside-target.journal";
  Check(WriteV1Journal(outside_target_journal, outside_target, L"before\n", L"after\n"),
        "write a journal referring outside its Workspace");
  Check(!mdlite::PreviewWorkspaceReplaceRollback(root, outside_target_journal, preview, error) &&
            preview.files.empty(),
        "preview rejects journal targets outside the Workspace");

  const auto actual_directory = root / L"actual-targets";
  const auto actual_target = actual_directory / L"note.txt";
  const auto reparse_directory = root / L"linked-targets";
  Check(WriteBytes(actual_target, "after\n"), "write a file behind a directory reparse point");
  const bool target_junction_created = CreateDirectoryJunction(reparse_directory, actual_directory);
  Check(target_junction_created,
        "create an unprivileged directory reparse fixture");
  const DWORD target_junction_attributes = GetFileAttributesW(reparse_directory.c_str());
  if (target_junction_created && target_junction_attributes != INVALID_FILE_ATTRIBUTES &&
      (target_junction_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
    const auto reparse_target_journal = journal_directory / L"reparse-target.journal";
    Check(WriteV1Journal(reparse_target_journal, reparse_directory / L"note.txt",
                         L"before\n", L"after\n"),
          "write a journal with a reparse target");
    Check(!mdlite::PreviewWorkspaceReplaceRollback(root, reparse_target_journal, preview, error) &&
              preview.files.empty(),
          "preview rejects targets behind a directory reparse point");
    Check(RemoveDirectoryW(reparse_directory.c_str()) != 0,
          "remove the owned target reparse fixture");
  }

  const auto journal_reparse_root = temporary.path() / L"reparse-journal-workspace";
  const auto journal_reparse_target = journal_reparse_root / L"note.txt";
  const auto real_journal_directory = journal_reparse_root / L"real-journals";
  const auto real_journal = real_journal_directory / L"reparse.journal";
  Check(WriteBytes(journal_reparse_target, "after\n"),
        "write the target for the reparse journal path test");
  Check(WriteV1Journal(real_journal, journal_reparse_target, L"before\n", L"after\n"),
        "write a journal behind a directory reparse point");
  const auto reparse_journal_directory = journal_reparse_root / L".mdlite/.state/replace";
  std::filesystem::create_directories(reparse_journal_directory.parent_path());
  const bool journal_junction_created =
      CreateDirectoryJunction(reparse_journal_directory, real_journal_directory);
  Check(journal_junction_created,
        "create a reparse point in the journal path");
  const DWORD journal_junction_attributes = GetFileAttributesW(reparse_journal_directory.c_str());
  if (journal_junction_created && journal_junction_attributes != INVALID_FILE_ATTRIBUTES &&
      (journal_junction_attributes & FILE_ATTRIBUTE_REPARSE_POINT) != 0) {
    Check(!mdlite::PreviewWorkspaceReplaceRollback(journal_reparse_root,
                                                    reparse_journal_directory / L"reparse.journal",
                                                    preview, error) && preview.files.empty(),
          "preview rejects a journal below a reparse point");
    Check(RemoveDirectoryW(reparse_journal_directory.c_str()) != 0,
          "remove the owned journal reparse fixture");
  }
}

}  // namespace

int main() {
  TestSourcePositionsAndRegex();
  TestUnicodeWholeWordBoundaries();
  TestUnicodeWholeWordReplacementParity();
  TestWorkspaceCoverageAndLimits();
  TestNativeHiddenSearchEligibility();
  TestBoundedSearchResultPayload();
  TestReplaceSnapshotsAndJournalSafety();
  TestSelectedMatchWorkspaceApplyValidation();
  TestReplaceIgnoresOnlyKnownSafeExclusions();
  TestReplaceStillRejectsSearchErrorsAndLimits();
  TestV1RollbackJournalPreview();
  TestRollbackPreviewRejectsUnsafeAndMalformedJournals();
  std::cout << checks << " checks, " << failures << " failures\n";
  return failures == 0 ? 0 : 1;
}
