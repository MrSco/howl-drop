#include "wsl_text.hpp"

#include <algorithm>
#include <cctype>
#include <sstream>

namespace howldrop::wsl {
namespace {

std::string trim(const std::string& s) {
    std::size_t a = 0;
    std::size_t b = s.size();
    while (a < b && std::isspace(static_cast<unsigned char>(s[a]))) ++a;
    while (b > a && std::isspace(static_cast<unsigned char>(s[b - 1]))) --b;
    return s.substr(a, b - a);
}

std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return s;
}

// Columns: runs of text apart by two or more spaces.
std::vector<std::string> columns(const std::string& line) {
    std::vector<std::string> out;
    std::size_t i = 0;
    while (i < line.size()) {
        while (i < line.size() && line[i] == ' ') ++i;
        if (i >= line.size()) break;
        std::size_t end = i;
        while (end < line.size() && !(line[end] == ' ' && (end + 1 >= line.size() || line[end + 1] == ' '))) ++end;
        out.push_back(trim(line.substr(i, end - i)));
        i = end;
    }
    return out;
}

std::vector<std::string> lines_of(const std::string& text) {
    std::vector<std::string> out;
    std::istringstream in(text);
    std::string line;
    while (std::getline(in, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        out.push_back(line);
    }
    return out;
}

}  // namespace

std::vector<UsbDevice> parse_usbipd_list(const std::string& text) {
    std::vector<UsbDevice> out;
    bool connected = false;
    for (const auto& line : lines_of(text)) {
        const std::string t = trim(line);
        if (t == "Connected:") {
            connected = true;
            continue;
        }
        if (!t.empty() && t.back() == ':' && t.find(' ') == std::string::npos) {
            connected = false;  // Persisted:, or a section to come
            continue;
        }
        if (!connected || t.empty() || t.rfind("BUSID", 0) == 0) continue;
        const auto cols = columns(t);
        if (cols.size() < 4 || cols[1].size() != 9 || cols[1][4] != ':') continue;
        UsbDevice device;
        device.busid = cols[0];
        device.id = lower(cols[1]);
        device.state = cols.back();
        for (std::size_t i = 2; i + 1 < cols.size(); ++i) device.name += (device.name.empty() ? "" : "  ") + cols[i];
        out.push_back(device);
    }
    return out;
}

const StickKind* stick_kind(const std::string& id) {
    for (const auto& kind : kSticks) {
        if (id == kind.id) return &kind;
    }
    return nullptr;
}

std::optional<UsbDevice> find_stick(const std::vector<UsbDevice>& devices) {
    std::optional<UsbDevice> found;
    for (const auto& device : devices) {
        const StickKind* kind = stick_kind(device.id);
        if (!kind || (found && (found->kind->tested || !kind->tested))) continue;
        found = device;
        found->kind = kind;
    }
    return found;
}

std::vector<std::string> parse_distros(const std::string& text) {
    std::vector<std::string> out;
    for (auto line : lines_of(text)) {
        // Without WSL_UTF8, wsl.exe writes UTF-16: drop the NULs and the BOM.
        line.erase(std::remove(line.begin(), line.end(), '\0'), line.end());
        if (line.rfind("\xEF\xBB\xBF", 0) == 0) line.erase(0, 3);
        line = trim(line);
        if (!line.empty()) out.push_back(line);
    }
    return out;
}

std::optional<std::string> wsl_path(const std::string& windows_path) {
    const std::string& p = windows_path;
    if (p.size() < 3 || !std::isalpha(static_cast<unsigned char>(p[0])) || p[1] != ':' ||
        (p[2] != '\\' && p[2] != '/')) {
        return std::nullopt;
    }
    std::string out = "/mnt/";
    out.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(p[0]))));
    for (std::size_t i = 2; i < p.size(); ++i) out.push_back(p[i] == '\\' ? '/' : p[i]);
    while (out.size() > 6 && out.back() == '/') out.pop_back();  // past "/mnt/c"
    return out;
}

std::optional<std::vector<std::string>> event_fields(const std::string& line) {
    static const std::string kMark = "@howldrop";
    const auto at = line.find(kMark);
    if (at == std::string::npos) return std::nullopt;
    const std::size_t after = at + kMark.size();
    if (after < line.size() && line[after] != '\t' && line[after] != '\r') return std::nullopt;
    if (at > 0 && line[at - 1] != ' ') return std::nullopt;  // only at the start or after a "Prefix: "
    std::vector<std::string> fields;
    std::string rest = after < line.size() ? line.substr(after) : std::string();
    if (!rest.empty() && rest.back() == '\r') rest.pop_back();
    std::size_t i = 0;
    while (i < rest.size()) {
        if (rest[i] != '\t') break;
        const std::size_t next = rest.find('\t', i + 1);
        fields.push_back(rest.substr(i + 1, next == std::string::npos ? std::string::npos : next - i - 1));
        if (next == std::string::npos) break;
        i = next;
    }
    return fields;
}

}  // namespace howldrop::wsl
