cmake_minimum_required(VERSION 3.16)
project(owldrop VERSION 0.1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

add_library(owldrop-core STATIC
    src/net.cpp
    src/protocol.cpp
    src/clipboard.cpp
    src/node.cpp
    src/forward.cpp
)
target_include_directories(owldrop-core PUBLIC src)
target_compile_features(owldrop-core PUBLIC cxx_std_17)

if(WIN32)
    target_compile_definitions(owldrop-core PUBLIC
        _WIN32_WINNT=0x0601
        NOMINMAX
        WIN32_LEAN_AND_MEAN
    )
    target_link_libraries(owldrop-core PUBLIC ws2_32 iphlpapi)
endif()

find_package(Threads REQUIRED)
target_link_libraries(owldrop-core PUBLIC Threads::Threads)

if(MSVC)
    target_compile_options(owldrop-core PRIVATE /W4)
else()
    target_compile_options(owldrop-core PRIVATE -Wall -Wextra -Wpedantic)
endif()

add_executable(owldrop src/main.cpp)
target_link_libraries(owldrop PRIVATE owldrop-core)
if(NOT MSVC)
    target_compile_options(owldrop PRIVATE -Wall -Wextra -Wpedantic)
endif()
if(WIN32)
    target_link_libraries(owldrop PRIVATE shell32)
    if(MINGW)
        target_link_options(owldrop PRIVATE -static -static-libgcc -static-libstdc++)
    endif()
endif()

add_executable(owldrop-bridge src/bridge_main.cpp)
target_link_libraries(owldrop-bridge PRIVATE owldrop-core)
if(NOT MSVC)
    target_compile_options(owldrop-bridge PRIVATE -Wall -Wextra -Wpedantic)
endif()

if(NOT WIN32)
    add_executable(owldrop-test tests/lan_test.cpp)
    target_link_libraries(owldrop-test PRIVATE owldrop-core)
    if(NOT MSVC)
        target_compile_options(owldrop-test PRIVATE -Wall -Wextra -Wpedantic)
    endif()

    add_executable(owldrop-bridge-test tests/bridge_test.cpp)
    target_link_libraries(owldrop-bridge-test PRIVATE owldrop-core)
    if(NOT MSVC)
        target_compile_options(owldrop-bridge-test PRIVATE -Wall -Wextra -Wpedantic)
    endif()
endif()
