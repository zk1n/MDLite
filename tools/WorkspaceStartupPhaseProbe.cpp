// Console-only decomposition of Explorer startup costs. No HWNDs or app launch.
#define NOMINMAX
#include <windows.h>
#include <shellapi.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <string>
#include <vector>

using Clock = std::chrono::steady_clock;
double Ms(Clock::time_point start) { return std::chrono::duration<double, std::milli>(Clock::now() - start).count(); }
struct NamedEntry { std::filesystem::path path; std::wstring name; bool directory; };

int wmain(int argc, wchar_t** argv) {
  if (argc != 3) return 2;
  const std::filesystem::path root = argv[1];
  const int count = std::stoi(argv[2]);
  if (count < 1 || count > 10000 || !std::filesystem::exists(root / L".owned-startup-probe")) return 3;
  std::array<char, 8192> header{};
  header.fill('a'); header[0] = '#'; header[1] = ' '; header[8190] = '\r'; header[8191] = '\n';
  auto start = Clock::now();
  for (int i = 0; i < count; ++i) {
    wchar_t name[32]{}; swprintf_s(name, L"note-%05d.md", i);
    const auto path = root / name;
    std::ofstream out(path, std::ios::binary); out.write(header.data(), header.size()); out.close();
    std::filesystem::resize_file(path, 20972); // ~200 MiB logical at 10,000; only first 8 KiB is examined.
  }
  const double generation = Ms(start);
  start = Clock::now();
  std::vector<std::filesystem::directory_entry> entries;
  for (const auto& entry : std::filesystem::directory_iterator(root)) entries.push_back(entry);
  const double enumeration = Ms(start);
  size_t comparisons = 0;
  start = Clock::now();
  auto original = entries;
  std::ranges::sort(original, [&](const auto& a, const auto& b) {
    ++comparisons;
    const bool ad = a.is_directory(), bd = b.is_directory();
    if (ad != bd) return ad;
    const auto an = a.path().filename().wstring(), bn = b.path().filename().wstring();
    const int insensitive = CompareStringOrdinal(an.c_str(), -1, bn.c_str(), -1, TRUE);
    if (insensitive != CSTR_EQUAL && insensitive != 0) return insensitive == CSTR_LESS_THAN;
    return an < bn;
  });
  const double original_sort = Ms(start);
  start = Clock::now();
  std::vector<NamedEntry> named;
  for (const auto& entry : entries) named.push_back({entry.path(), entry.path().filename().wstring(), entry.is_directory()});
  const double name_capture = Ms(start);
  start = Clock::now();
  std::ranges::sort(named, [](const auto& a, const auto& b) {
    if (a.directory != b.directory) return a.directory;
    const int insensitive = CompareStringOrdinal(a.name.c_str(), -1, b.name.c_str(), -1, TRUE);
    if (insensitive != CSTR_EQUAL && insensitive != 0) return insensitive == CSTR_LESS_THAN;
    return a.name < b.name;
  });
  const double captured_sort = Ms(start);
  bool same_order = original.size() == named.size();
  std::vector<std::filesystem::path> files;
  for (size_t i = 0; i < named.size(); ++i) {
    same_order = same_order && original[i].path() == named[i].path;
    if (named[i].path.extension() == L".md") files.push_back(named[i].path);
  }
  // Same read-only API/flags as Application::AddTreeDirectory, sampled once.
  const size_t sample_count = std::min<size_t>(64, files.size());
  if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE))) return 5;
  size_t real_ok = 0, attributes_ok = 0, same_icons = 0;
  std::vector<double> real_times;
  std::vector<int> real_indices;
  start = Clock::now();
  for (size_t i = 0; i < sample_count; ++i) {
    SHFILEINFOW info{};
    const auto one_start = Clock::now();
    real_ok += SHGetFileInfoW(files[i].c_str(), FILE_ATTRIBUTE_NORMAL, &info, sizeof(info), SHGFI_SYSICONINDEX | SHGFI_SMALLICON) != 0;
    real_times.push_back(Ms(one_start));real_indices.push_back(info.iIcon);
  }
  const double real_icons = Ms(start);
  start = Clock::now();
  for (size_t i = 0; i < sample_count; ++i) {
    SHFILEINFOW info{};
    attributes_ok += SHGetFileInfoW(L".md", FILE_ATTRIBUTE_NORMAL, &info, sizeof(info), SHGFI_SYSICONINDEX | SHGFI_SMALLICON | SHGFI_USEFILEATTRIBUTES) != 0;
    same_icons += info.iIcon == real_indices[i];
  }
  const double attribute_icons = Ms(start);
  CoUninitialize();
  // Mirror calendar's LooksBinary + creation-time query; no text/timestamp logging.
  size_t header_ok = 0, creation_ok = 0; unsigned long long header_bytes = 0;
  start = Clock::now();
  for (const auto& path : files) {
    std::ifstream input(path, std::ios::binary); std::array<char, 8192> bytes{};
    input.read(bytes.data(), bytes.size()); header_bytes += input.gcount();
    header_ok += (input.eof() || input.good()) && std::ranges::none_of(bytes.begin(), bytes.begin() + input.gcount(), [](char ch) { return ch == '\0'; });
  }
  const double headers = Ms(start);
  start = Clock::now();
  for (const auto& path : files) {
    HANDLE file = CreateFileW(path.c_str(), FILE_READ_ATTRIBUTES, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (file != INVALID_HANDLE_VALUE) { FILETIME creation{}, access{}, write{}; creation_ok += GetFileTime(file, &creation, &access, &write) != 0; CloseHandle(file); }
  }
  const double creation = Ms(start);
  std::cout << "{\"schema\":\"mdlite-workspace-startup-phases-v1\",\"file_count\":" << files.size()
    << ",\"generation_ms\":" << generation << ",\"enumeration_ms\":" << enumeration
    << ",\"original_sort_ms\":" << original_sort << ",\"sort_comparisons\":" << comparisons
    << ",\"comparator_name_materializations\":" << comparisons * 2
    << ",\"captured_names_ms\":" << name_capture << ",\"captured_sort_ms\":" << captured_sort
    << ",\"sort_order_identical\":" << (same_order ? "true" : "false")
    << ",\"icon_sample_count\":" << sample_count << ",\"real_path_icon_ms\":" << real_icons << ",\"real_path_icon_ok\":" << real_ok
    << ",\"attribute_icon_ms\":" << attribute_icons << ",\"attribute_icon_ok\":" << attributes_ok
    << ",\"first_real_icon_ms\":" << real_times.front()
    << ",\"real_icons_after_first_ms\":" << real_icons-real_times.front()
    << ",\"real_icon_max_ms\":" << *std::ranges::max_element(real_times)
    << ",\"ordinary_md_icon_indices_equal\":" << same_icons
    << ",\"header_scan_ms\":" << headers << ",\"headers_ok\":" << header_ok << ",\"header_bytes\":" << header_bytes
    << ",\"creation_metadata_ms\":" << creation << ",\"creation_metadata_ok\":" << creation_ok
    << ",\"tree_insert_calls_executed\":0,\"explicit_hwnds_created\":0,\"app_launches\":0}" << std::endl;
  return same_order && header_ok == files.size() && creation_ok == files.size() ? 0 : 4;
}
