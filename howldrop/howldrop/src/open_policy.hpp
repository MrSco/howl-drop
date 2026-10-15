#pragma once

#include <filesystem>
#include <string>

namespace howldrop {

// What a received file is, for what the Windows app may open by itself. Only
// these kinds are ever opened automatically; anything else (programs,
// scripts, shortcuts to files, installers, archives) is Other and is only
// ever shown in its folder.
enum class OpenKind { Link, Image, Video, Document, Other };

OpenKind classify(const std::filesystem::path& path);

// The http(s) address in a .url Internet shortcut ([InternetShortcut] URL=),
// or "" for anything else: another scheme (file:, javascript:, a UNC path),
// a malformed file, or one that isn't a .url.
std::string shortcut_url(const std::filesystem::path& path);

// "Photo", "Video", "Link", "Document" or "File".
const char* kind_word(OpenKind kind);

}  // namespace howldrop
