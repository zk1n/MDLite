#pragma once

#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace mdlite {

struct ProcessResult {
  unsigned long exit_code{static_cast<unsigned long>(-1)};
  std::wstring output;
  bool timed_out{};
  bool cancelled{};
  bool truncated{};
};

bool RunProcess(const std::filesystem::path& executable, const std::vector<std::wstring>& arguments,
                const std::filesystem::path& working_directory, std::size_t output_limit,
                unsigned long timeout_ms, ProcessResult& result, std::wstring& error,
                void* cancellation_event = nullptr);

}  // namespace mdlite
