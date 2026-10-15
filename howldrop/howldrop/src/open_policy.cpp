#include "open_policy.hpp"

#include <algorithm>
#include <cctype>
#include <fstream>

namespace howldrop {
namespace {

std::string lower(std::string text) {
    std::transform(text.begin(), text.end(), text.begin(),
                   [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
    return text;
}

bool one_of(const std::string& ext, std::initializer_list<const char*> list) {
    for (const char* item : list) {
        if (ext == item) return true;
    }
    return false;
}

std::string trim(std::string text) {
    while (!text.empty() && (text.back() == '\r' || text.back() == ' ' || text.back() == '\t')) text.pop_back();
    std::size_t start = 0;
    while (start < text.size() && (text[start] == ' ' || text[start] == '\t')) ++start;
    return text.substr(start);
}

}  // namespace

OpenKind classify(const std::filesystem::path& path) {
    const std::string ext = lower(path.extension().string());
    if (ext == ".url") return OpenKind::Link;
    if (one_of(ext, {".jpg", ".jpeg", ".png", ".gif", ".webp", ".heic", ".heif", ".bmp", ".tif", ".tiff"}))
        return OpenKind::Image;
    if (one_of(ext, {".mov", ".mp4", ".m4v", ".avi", ".mkv", ".webm"})) return OpenKind::Video;
    if (one_of(ext, {".pdf", ".txt"})) return OpenKind::Document;
    return OpenKind::Other;
}

std::string shortcut_url(const std::filesystem::path& path) {
    if (classify(path) != OpenKind::Link) return {};
    std::ifstream in(path, std::ios::binary);
    if (!in) return {};
    std::string line;
    bool in_section = false;
    for (int lines = 0; lines < 64 && std::getline(in, line); ++lines) {
        line = trim(line);
        if (line.size() >= 3 && static_cast<unsigned char>(line[0]) == 0xEF) line.erase(0, 3);  // UTF-8 BOM
        if (!line.empty() && line.front() == '[') {
            in_section = lower(line) == "[internetshortcut]";
            continue;
        }
        if (!in_section || lower(line.substr(0, 4)) != "url=") continue;
        const std::string url = trim(line.substr(4));
        const std::string scheme = lower(url.substr(0, 8));
        if (scheme.rfind("https://", 0) != 0 && scheme.rfind("http://", 0) != 0) return {};
        if (url.size() > 2048) return {};
        for (unsigned char c : url) {
            if (c < 0x20 || c == 0x7f || c == '"') return {};
        }
        return url;
    }
    return {};
}

const char* kind_word(OpenKind kind) {
    switch (kind) {
        case OpenKind::Link: return "Link";
        case OpenKind::Image: return "Photo";
        case OpenKind::Video: return "Video";
        case OpenKind::Document: return "Document";
        default: return "File";
    }
}

}  // namespace howldrop
