#include "history.hpp"

#include "text.hpp"

#include <fstream>
#include <sstream>

namespace howldrop::history {
namespace {

std::string clean(std::string s) {
    for (char& c : s) {
        if (c == '\t' || c == '\n' || c == '\r') c = ' ';
    }
    return s;
}

std::vector<std::string> read_lines(const std::filesystem::path& file) {
    std::vector<std::string> lines;
    std::ifstream in(file, std::ios::binary);
    std::string line;
    while (std::getline(in, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (!line.empty()) lines.push_back(line);
    }
    return lines;
}

bool parse(const std::string& line, Entry& e) {
    std::istringstream in(line);
    std::string time, dir;
    if (!std::getline(in, time, '\t') || !std::getline(in, dir, '\t') || !std::getline(in, e.peer, '\t') ||
        !std::getline(in, e.item)) {
        return false;
    }
    try {
        e.time = std::stoll(time);
    } catch (...) {
        return false;
    }
    if (dir != "in" && dir != "out") return false;
    e.out = dir == "out";
    return !e.item.empty();
}

}  // namespace

void append(const std::filesystem::path& file, const Entry& entry) {
    std::error_code ec;
    std::filesystem::create_directories(file.parent_path(), ec);
    {
        std::ofstream out(file, std::ios::app | std::ios::binary);
        out << entry.time << '\t' << (entry.out ? "out" : "in") << '\t' << clean(entry.peer) << '\t'
            << clean(entry.item) << '\n';
    }
    // Trimmed now and then: past twice what's kept, back to what's kept.
    auto lines = read_lines(file);
    if (lines.size() <= 2 * kKept) return;
    lines.erase(lines.begin(), lines.end() - static_cast<std::ptrdiff_t>(kKept));
    const auto fresh = file.wstring() + L".new";
    {
        std::ofstream out(std::filesystem::path(fresh), std::ios::trunc | std::ios::binary);
        for (const auto& l : lines) out << l << '\n';
    }
    std::filesystem::rename(fresh, file, ec);
}

std::vector<Entry> load(const std::filesystem::path& file, std::size_t limit) {
    std::vector<Entry> out;
    const auto lines = read_lines(file);
    for (auto it = lines.rbegin(); it != lines.rend() && out.size() < limit; ++it) {
        Entry e;
        if (!parse(*it, e)) continue;
        if (!e.out) {
            std::error_code ec;
            if (!std::filesystem::exists(path_from_utf8(e.item), ec)) continue;
        }
        out.push_back(e);
    }
    return out;
}

}  // namespace howldrop::history
