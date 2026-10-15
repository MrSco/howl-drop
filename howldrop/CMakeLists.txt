cmake_minimum_required(VERSION 3.16)
project(howldrop VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_library(howldrop-core STATIC
    src/net.cpp
    src/protocol.cpp
    src/clipboard.cpp
    src/node.cpp
    src/forward.cpp
    src/open_policy.cpp
    src/pairing.cpp
)
target_include_directories(howldrop-core PUBLIC src)
target_compile_features(howldrop-core PUBLIC cxx_std_17)

if(WIN32)
    target_compile_definitions(howldrop-core PUBLIC
        _WIN32_WINNT=0x0601
        NOMINMAX
        WIN32_LEAN_AND_MEAN
    )
    target_link_libraries(howldrop-core PUBLIC ws2_32 iphlpapi bcrypt)
endif()

find_package(Threads REQUIRED)
target_link_libraries(howldrop-core PUBLIC Threads::Threads)

if(MSVC)
    target_compile_options(howldrop-core PRIVATE /W4)
else()
    target_compile_options(howldrop-core PRIVATE -Wall -Wextra -Wpedantic)
endif()

add_executable(howldrop src/main.cpp)
target_link_libraries(howldrop PRIVATE howldrop-core)
if(NOT MSVC)
    target_compile_options(howldrop PRIVATE -Wall -Wextra -Wpedantic)
endif()
if(WIN32)
    # The tray app, with its icon, and no console window: commands typed in
    # a terminal attach to it (main.cpp).
    enable_language(RC)
    target_sources(howldrop PRIVATE src/tray_win.cpp res/howldrop.rc)
    set_target_properties(howldrop PROPERTIES WIN32_EXECUTABLE TRUE)
    target_link_libraries(howldrop PRIVATE shell32 user32 advapi32)
    if(MINGW)
        target_link_options(howldrop PRIVATE -static -static-libgcc -static-libstdc++)
    endif()
endif()

add_executable(howldrop-bridge src/bridge_main.cpp)
target_link_libraries(howldrop-bridge PRIVATE howldrop-core)
if(NOT MSVC)
    target_compile_options(howldrop-bridge PRIVATE -Wall -Wextra -Wpedantic)
endif()

add_executable(howldrop-open-test tests/open_policy_test.cpp)
target_link_libraries(howldrop-open-test PRIVATE howldrop-core)
add_executable(howldrop-pairing-test tests/pairing_test.cpp)
target_link_libraries(howldrop-pairing-test PRIVATE howldrop-core)
if(NOT MSVC)
    target_compile_options(howldrop-open-test PRIVATE -Wall -Wextra -Wpedantic)
    target_compile_options(howldrop-pairing-test PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(NOT WIN32)
    add_executable(howldrop-test tests/lan_test.cpp)
    target_link_libraries(howldrop-test PRIVATE howldrop-core)
    if(NOT MSVC)
        target_compile_options(howldrop-test PRIVATE -Wall -Wextra -Wpedantic)
    endif()

    add_executable(howldrop-bridge-test tests/bridge_test.cpp)
    target_link_libraries(howldrop-bridge-test PRIVATE howldrop-core)
    if(NOT MSVC)
        target_compile_options(howldrop-bridge-test PRIVATE -Wall -Wextra -Wpedantic)
    endif()
endif()
