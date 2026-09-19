#pragma once

#include <windows.h>

#include <filesystem>
#include <optional>
#include <string>
#include <vector>

namespace mdlite {

enum class BuiltInProfile { Daily, Meeting, Memo };

enum class ProfileCollision { OpenExisting, Sequence };

struct ProfileDefinition {
  std::wstring id;
  std::wstring name;
  std::filesystem::path directory;
  std::filesystem::path filename;
  std::filesystem::path template_path;
  ProfileCollision collision{ProfileCollision::OpenExisting};
};

struct NoteCreationResult {
  std::filesystem::path path;
  std::size_t cursor{};
  bool created{};
};

bool CreateProfileNote(const std::filesystem::path& workspace, BuiltInProfile profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error);
bool CreateProfileNote(const std::filesystem::path& workspace, const ProfileDefinition& profile,
                       const SYSTEMTIME& local_date, NoteCreationResult& result,
                       std::wstring& error);
std::vector<ProfileDefinition> DefaultProfiles();
bool LoadProfileFile(const std::filesystem::path& path, std::vector<ProfileDefinition>& profiles,
                     std::wstring& error);
bool SaveProfileFile(const std::filesystem::path& path, const std::vector<ProfileDefinition>& profiles,
                     std::wstring& error);
bool ResolveProfiles(const std::filesystem::path& common_path,
                     const std::filesystem::path& workspace_path,
                     std::vector<ProfileDefinition>& profiles, std::wstring& error);
bool ValidateProfile(const ProfileDefinition& profile, std::wstring& error);
std::optional<std::filesystem::path> PreviewProfilePath(const std::filesystem::path& workspace,
                                                        const ProfileDefinition& profile,
                                                        const SYSTEMTIME& local_date,
                                                        std::wstring& error);
std::filesystem::path CommonProfilesPath();

}  // namespace mdlite
