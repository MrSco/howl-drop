#pragma once

#include "config.hpp"

#include <filesystem>
#include <optional>
#include <string>
#include <string_view>

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

namespace howldrop {

inline std::optional<std::wstring> wide_from_utf8(std::string_view utf8) {
#ifdef _WIN32
    if (utf8.empty()) return std::wstring();
    if (utf8.size() > static_cast<std::size_t>(0x7fffffff)) return std::nullopt;
    const int n = MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, utf8.data(),
                                      static_cast<int>(utf8.size()), nullptr, 0);
    if (n <= 0) return std::nullopt;
    std::wstring wide(static_cast<std::size_t>(n), L'\0');
    if (MultiByteToWideChar(CP_UTF8, MB_ERR_INVALID_CHARS, utf8.data(),
                            static_cast<int>(utf8.size()), wide.data(), n) != n) {
        return std::nullopt;
    }
    return wide;
#else
    return std::wstring(utf8.begin(), utf8.end());
#endif
}

inline std::string utf8_from_wide(std::wstring_view wide) {
#ifdef _WIN32
    if (wide.empty()) return {};
    if (wide.size() > static_cast<std::size_t>(0x7fffffff)) return {};
    const int n = WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()),
                                      nullptr, 0, nullptr, nullptr);
    if (n <= 0) return {};
    std::string out(static_cast<std::size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, 0, wide.data(), static_cast<int>(wide.size()), out.data(), n,
                        nullptr, nullptr);
    return out;
#else
    std::string out;
    out.reserve(wide.size());
    for (wchar_t ch : wide) {
        if (ch >= 0 && ch <= 0x7f) out.push_back(static_cast<char>(ch));
        else out.push_back('?');
    }
    return out;
#endif
}

inline std::filesystem::path path_from_utf8(std::string_view utf8) {
#ifdef _WIN32
    auto wide = wide_from_utf8(utf8);
    if (!wide) return {};
    return std::filesystem::path(*wide);
#else
    return std::filesystem::path(std::string(utf8));
#endif
}

inline std::string path_to_utf8(const std::filesystem::path& path) {
#ifdef _WIN32
    return utf8_from_wide(path.wstring());
#else
    return path.string();
#endif
}

inline std::string filename_utf8(const std::filesystem::path& path) {
    return path_to_utf8(path.filename());
}

}  // namespace howldrop
