cmake_minimum_required(VERSION 3.28)

project(MDLite VERSION 0.1.0 LANGUAGES CXX)

if(NOT WIN32)
  message(FATAL_ERROR "MDLite v0.1 targets Windows 11 x64.")
endif()

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
if(MSVC)
  # Local/F5 and unsigned verification builds must run on a clean Windows 11
  # machine without requiring an administrator-installed VC redistributable.
  set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()

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
target_link_libraries(mdlite_core PUBLIC shell32 ole32 windowscodecs)

add_executable(MDLite WIN32
  src/app/MDLite.manifest
  src/main.cpp
  src/app/Application.cpp
  src/app/Application.h
)
target_compile_definitions(MDLite PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
target_compile_options(MDLite PRIVATE /W4 /permissive- /utf-8 /EHsc)
target_link_libraries(MDLite PRIVATE mdlite_core comctl32 comdlg32 shell32 shlwapi ole32 uuid windowscodecs)

include(CTest)
if(BUILD_TESTING)
  add_executable(mdlite_core_tests tests/CoreTests.cpp)
  target_compile_definitions(mdlite_core_tests PRIVATE UNICODE _UNICODE NOMINMAX WIN32_LEAN_AND_MEAN)
  target_compile_options(mdlite_core_tests PRIVATE /W4 /permissive- /utf-8 /EHsc)
  target_link_libraries(mdlite_core_tests PRIVATE mdlite_core)
  add_test(NAME mdlite_core_tests COMMAND mdlite_core_tests)
endif()

install(TARGETS MDLite RUNTIME DESTINATION .)
