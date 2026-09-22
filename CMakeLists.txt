cmake_minimum_required(VERSION 3.28)

project(MDLite VERSION 0.1.0 LANGUAGES CXX)

include(FetchContent)

if(NOT WIN32)
  message(FATAL_ERROR "MDLite v0.1 targets Windows 11 x64.")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
# MDLite does not use C++ modules. Keep ordinary MSVC header dependency
# tracking enabled; the module scanner can emit an empty dependency set for
# headers and leave a launcher object with an obsolete Application layout.
set(CMAKE_CXX_SCAN_FOR_MODULES OFF)
if(MSVC)
  # Local/F5 and unsigned verification builds must run on a clean Windows 11
  # machine without requiring an administrator-installed VC redistributable.
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

# WebP is linked statically so the required five image formats work on a clean
# Windows 11 installation without depending on the optional Store codec.
set(WEBP_BUILD_ANIM_UTILS OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_CWEBP OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_DWEBP OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_GIF2WEBP OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_IMG2WEBP OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_VWEBP OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_WEBPINFO OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_WEBPMUX OFF CACHE BOOL "" FORCE)
set(WEBP_BUILD_EXTRAS OFF CACHE BOOL "" FORCE)
FetchContent_Declare(libwebp
  URL https://storage.googleapis.com/downloads.webmproject.org/releases/webp/libwebp-1.6.0.tar.gz
  URL_HASH SHA256=e4ab7009bf0629fd11982d4c2aa83964cf244cffba7347ecd39019a9e38c4564
  DOWNLOAD_EXTRACT_TIMESTAMP TRUE
)
FetchContent_MakeAvailable(libwebp)

add_library(mdlite_core
  src/assets/Assets.cpp
  src/assets/Assets.h
  src/assets/StorageAdapter.cpp
  src/assets/StorageAdapter.h
  src/calendar/JapaneseHolidays.cpp
  src/calendar/JapaneseHolidays.h
  src/core/Document.cpp
  src/core/Document.h
  src/editor/EditorAdapter.cpp
  src/editor/EditorAdapter.h
  src/git/Conflict.cpp
  src/git/Conflict.h
  src/markdown/Markdown.cpp
  src/markdown/Markdown.h
  src/profiles/Profiles.cpp
  src/profiles/Profiles.h
  src/process/ProcessRunner.cpp
  src/process/ProcessRunner.h
  src/search/Search.cpp
  src/search/Search.h
  src/search/Replace.cpp
  src/search/Replace.h
  src/settings/Settings.cpp
  src/settings/Settings.h
  src/table/Table.cpp
  src/table/Table.h
  src/workspace/Workspace.cpp
  src/workspace/Workspace.h
  src/workspace/Trust.cpp
  src/workspace/Trust.h
)
target_include_directories(mdlite_core PUBLIC "${CMAKE_CURRENT_SOURCE_DIR}/src")
target_compile_definitions(mdlite_core PUBLIC UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_compile_options(mdlite_core PRIVATE /W4 /permissive- /utf-8 /EHsc)
target_link_libraries(mdlite_core PUBLIC shell32 ole32 windowscodecs xmllite winhttp webp webpdemux
  d2d1 d3d11 dxgi)

add_executable(MDLite WIN32
  src/app/MDLite.manifest
  src/main.cpp
  src/app/Application.cpp
  src/app/Application.h
)
# Application is constructed by value in wWinMain; its stack layout must be
# rebuilt whenever the class or EffectiveSettings layout changes.
set_source_files_properties(src/main.cpp PROPERTIES
  OBJECT_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/src/app/Application.h;${CMAKE_CURRENT_SOURCE_DIR}/src/settings/Settings.h")
target_compile_definitions(MDLite PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_compile_options(MDLite PRIVATE /W4 /permissive- /utf-8 /EHsc)
target_link_libraries(MDLite PRIVATE mdlite_core comctl32 comdlg32 shell32 shlwapi ole32 uuid windowscodecs dwmapi wer)

include(CTest)
if(BUILD_TESTING)
  add_executable(mdlite_core_tests tests/CoreTests.cpp)
  target_compile_definitions(mdlite_core_tests PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
  target_compile_options(mdlite_core_tests PRIVATE /W4 /permissive- /utf-8 /EHsc)
  target_link_libraries(mdlite_core_tests PRIVATE mdlite_core libwebpmux shlwapi)
  add_test(NAME mdlite_core_tests COMMAND mdlite_core_tests)
endif()

install(TARGETS MDLite RUNTIME DESTINATION .)
