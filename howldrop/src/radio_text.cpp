#include "radio_text.hpp"

namespace howldrop::radio {

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

int restart_delay_ms(int failed) {
    // Long enough at the end for a stick that is slow to come back after a wake.
    static constexpr int kDelays[] = {3000, 5000, 15000, 30000};
    return failed >= 0 && failed < 4 ? kDelays[failed] : -1;
}

}  // namespace howldrop::radio
