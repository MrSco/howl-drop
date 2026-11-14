#include "radio_text.hpp"
#include <algorithm>
#include <limits>

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

std::optional<SenderIdentityAdvert> sender_identity(const std::vector<std::string>& e) {
    if (e.size() < 4 || e.size() > 131 || e[0] != "sender-identity" || e[1] != "ready" || e[2].empty()) return std::nullopt;
    SenderIdentityAdvert result;
    for (char c : e[2]) {
        if (c < '0' || c > '9' || result.expires > (std::numeric_limits<std::uint64_t>::max() - (c - '0')) / 10) return std::nullopt;
        result.expires = result.expires * 10 + static_cast<std::uint64_t>(c - '0');
    }
    const auto nibble = [](char c) { return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : -1; };
    for (std::size_t i = 3; i < e.size(); ++i) {
        if (e[i].size() != 4) return std::nullopt;
        std::array<unsigned char, 2> prefix{};
        for (std::size_t n = 0; n < 2; ++n) {
            const int a = nibble(e[i][2*n]), b = nibble(e[i][2*n+1]);
            if (a < 0 || b < 0) return std::nullopt;
            prefix[n] = static_cast<unsigned char>(a * 16 + b);
        }
        result.prefixes.push_back(prefix);
    }
    return result;
}

std::array<unsigned char, 8> advert_hashes(const SenderIdentityAdvert& identity, std::size_t offset, std::uint64_t now) {
    std::array<unsigned char, 8> out{};
    if (identity.expires <= now || identity.prefixes.empty()) return out;
    for (std::size_t i = 0; i < std::min<std::size_t>(4, identity.prefixes.size()); ++i) {
        const auto& prefix = identity.prefixes[(offset + i) % identity.prefixes.size()];
        out[2*i] = prefix[0]; out[2*i+1] = prefix[1];
    }
    return out;
}

int restart_delay_ms(int failed) {
    // Long enough at the end for a stick that is slow to come back after a wake.
    static constexpr int kDelays[] = {3000, 5000, 15000, 30000};
    return failed >= 0 && failed < 4 ? kDelays[failed] : -1;
}

}  // namespace howldrop::radio
