#pragma once

#include "process/ProcessRunner.h"

#include <filesystem>
#include <string>
#include <vector>

namespace mdlite {

struct StorageAdapter {
  std::filesystem::path executable;
  std::vector<std::wstring> arguments;
  unsigned long timeout_ms{120000};
};

struct StorageUploadResult {
  std::wstring reference;
  ProcessResult process;
};

bool LoadStorageAdapter(const std::filesystem::path& config, StorageAdapter& adapter,
                        std::wstring& error);
bool UploadWithStorageAdapter(const StorageAdapter& adapter, const std::filesystem::path& asset,
                              std::wstring_view revision, void* cancellation_event,
                              StorageUploadResult& result, std::wstring& error);

}  // namespace mdlite
