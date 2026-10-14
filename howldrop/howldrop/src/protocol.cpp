#include "protocol.hpp"

#include "config.hpp"

#include <cctype>
#include <cstring>

namespace howldrop {
namespace {

void put_u16(std::uint8_t* p, std::uint16_t v) {
    p[0] = static_cast<std::uint8_t>((v >> 8) & 0xff);
    p[1] = static_cast<std::uint8_t>(v & 0xff);
}

void put_u64(std::uint8_t* p, std::uint64_t v) {
    for (int i = 7; i >= 0; --i) {
        p[i] = static_cast<std::uint8_t>(v & 0xff);
        v >>= 8;
    }
}

std::uint16_t get_u16(const std::uint8_t* p) {
    return static_cast<std::uint16_t>((static_cast<std::uint16_t>(p[0]) << 8) | p[1]);
}

std::uint64_t get_u64(const std::uint8_t* p) {
    std::uint64_t v = 0;
    for (int i = 0; i < 8; ++i) v = (v << 8) | p[i];
    return v;
}

bool valid_key(const std::string& key) {
    if (key.empty() || key.size() > 32) return false;
    for (unsigned char c : key) {
        const bool ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || c == '-';
        if (!ok) return false;
    }
    return true;
}

bool parse_fields(const std::uint8_t* data, std::size_t n, Fields& fields, std::string& error) {
    std::size_t i = 0;
    while (i < n) {
        std::size_t end = i;
        while (end < n && data[end] != '\n') ++end;
        std::string line(reinterpret_cast<const char*>(data + i), end - i);
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (!line.empty()) {
            const auto eq = line.find('=');
            if (eq == std::string::npos || eq == 0) {
                error = "bad header";
                return false;
            }
            std::string key = line.substr(0, eq);
            std::string value = line.substr(eq + 1);
            if (!valid_key(key) || value.size() > 1024 || value.find('\n') != std::string::npos) {
                error = "bad header";
                return false;
            }
            fields.set(std::move(key), std::move(value));
        }
        if (end >= n) break;
        i = end + 1;
    }
    return true;
}

char ascii_lower(char c) {
    if (c >= 'A' && c <= 'Z') return static_cast<char>(c - 'A' + 'a');
    return c;
}

}  // namespace

void Fields::set(std::string key, std::string value) {
    for (auto& item : items) {
        if (item.first == key) {
            item.second = std::move(value);
            return;
        }
    }
    items.emplace_back(std::move(key), std::move(value));
}

const std::string* Fields::find(std::string_view key) const {
    for (const auto& item : items) {
        if (item.first == key) return &item.second;
    }
    return nullptr;
}

std::string Fields::get(std::string_view key) const {
    const std::string* value = find(key);
    return value ? *value : std::string();
}

std::optional<std::vector<std::uint8_t>> encode_head(const Message& message, std::string& error) {
    std::string header;
    for (const auto& item : message.fields.items) {
        if (!valid_key(item.first) || item.second.size() > 1024 || item.second.find('\n') != std::string::npos ||
            item.second.find('\r') != std::string::npos) {
            error = "bad header field";
            return std::nullopt;
        }
        header += item.first;
        header += '=';
        header += item.second;
        header += '\n';
    }
    if (header.size() > kMaxHeaderBytes) {
        error = "header is too large";
        return std::nullopt;
    }
    std::vector<std::uint8_t> out(16 + header.size());
    out[0] = 'O';
    out[1] = 'D';
    out[2] = '1';
    out[3] = '\n';
    out[4] = static_cast<std::uint8_t>(message.type);
    out[5] = 0;
    put_u16(&out[6], static_cast<std::uint16_t>(header.size()));
    put_u64(&out[8], message.body_size);
    if (!header.empty()) std::memcpy(out.data() + 16, header.data(), header.size());
    return out;
}

bool parse_prefix(const std::uint8_t prefix[16], MsgType& type, std::uint16_t& header_len, std::uint64_t& body_size,
                  std::string& error) {
    if (prefix[0] != 'O' || prefix[1] != 'D' || prefix[2] != '1' || prefix[3] != '\n') {
        error = "not an HowlDrop message";
        return false;
    }
    const auto raw = prefix[4];
    if (raw < 1 || raw > 4) {
        error = "unknown message";
        return false;
    }
    type = static_cast<MsgType>(raw);
    header_len = get_u16(prefix + 6);
    body_size = get_u64(prefix + 8);
    if (header_len > kMaxHeaderBytes) {
        error = "header is too large";
        return false;
    }
    return true;
}

std::optional<Message> parse_datagram(const std::uint8_t* data, std::size_t n, std::string& error) {
    if (n < 16) {
        error = "short message";
        return std::nullopt;
    }
    Message message;
    std::uint16_t header_len = 0;
    if (!parse_prefix(data, message.type, header_len, message.body_size, error)) return std::nullopt;
    if (message.body_size != 0 || static_cast<std::size_t>(16 + header_len) != n) {
        error = "bad datagram";
        return std::nullopt;
    }
    if (!parse_fields(data + 16, header_len, message.fields, error)) return std::nullopt;
    return message;
}

bool write_head(Socket& socket, const Message& message, const std::atomic<bool>* stop, std::string& error) {
    auto bytes = encode_head(message, error);
    if (!bytes) return false;
    if (!socket.send_full(bytes->data(), bytes->size(), stop)) {
        error = "send failed";
        return false;
    }
    return true;
}

bool read_head(Socket& socket, Message& message, const std::atomic<bool>* stop, std::string& error) {
    std::uint8_t prefix[16];
    if (!socket.recv_full(prefix, 16, stop)) {
        error = "connection closed";
        return false;
    }
    std::uint16_t header_len = 0;
    if (!parse_prefix(prefix, message.type, header_len, message.body_size, error)) return false;
    std::vector<std::uint8_t> header(header_len);
    if (header_len && !socket.recv_full(header.data(), header_len, stop)) {
        error = "connection closed";
        return false;
    }
    message.fields = Fields{};
    return parse_fields(header.data(), header.size(), message.fields, error);
}

std::string sanitize_filename(std::string name) {
    for (char& c : name) {
        if (c == '\\') c = '/';
    }
    const auto slash = name.find_last_of('/');
    if (slash != std::string::npos) name = name.substr(slash + 1);
    std::string out;
    out.reserve(name.size());
    for (unsigned char c : name) {
        if (c < 0x20 || c == 0x7f) continue;
        if (c == '<' || c == '>' || c == ':' || c == '"' || c == '|' || c == '?' || c == '*' || c == '/') out.push_back('_');
        else out.push_back(static_cast<char>(c));
    }
    while (!out.empty() && (out.back() == ' ' || out.back() == '.')) out.pop_back();
    if (out.empty() || out == "." || out == "..") return "file";
    const auto dot = out.find('.');
    const std::string base = dot == std::string::npos || dot == 0 ? out : out.substr(0, dot);
    std::string upper = base;
    for (char& c : upper) {
        if (c >= 'a' && c <= 'z') c = static_cast<char>(c - 'a' + 'A');
    }
    static const char* reserved[] = {"CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5",
                                     "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
                                     "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"};
    for (const char* word : reserved) {
        if (upper == word) {
            out.insert(out.begin(), '_');
            break;
        }
    }
    if (out.size() > 180) out.resize(180);
    return out;
}

std::string one_line(std::string text) {
    for (char& c : text) {
        if (c == '\n' || c == '\r' || c == '\t') c = ' ';
    }
    if (text.size() > 300) text.resize(300);
    return text;
}

bool is_node_id(const std::string& id) {
    if (id.size() != 32) return false;
    for (unsigned char c : id) {
        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
    }
    return true;
}

bool valid_display_name(const std::string& name) {
    if (name.empty() || name.size() > 64) return false;
    for (unsigned char c : name) {
        if (c < 0x20 || c == 0x7f) return false;
    }
    return true;
}

bool names_equal(const std::string& a, const std::string& b) {
    if (a.size() != b.size()) return false;
    for (std::size_t i = 0; i < a.size(); ++i) {
        const auto ca = static_cast<unsigned char>(ascii_lower(a[i]));
        const auto cb = static_cast<unsigned char>(ascii_lower(b[i]));
        if (ca != cb) return false;
    }
    return true;
}

}  // namespace howldrop
