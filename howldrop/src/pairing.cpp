#include "pairing.hpp"

#include <cstring>

#ifdef _WIN32
#include <windows.h>
#include <bcrypt.h>
#else
#include <sys/random.h>
#endif

namespace howldrop {
namespace {

constexpr std::uint32_t kRound[64] = {
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

std::uint32_t rotr(std::uint32_t x, int n) { return (x >> n) | (x << (32 - n)); }

const char* const kHead = "howldrop-pair-v1 head\n";
const char* const kReady = "howldrop-pair-v1 ready\n";
const char* const kBody = "howldrop-pair-v1 body\n";

std::string head_text(const Message& message, const std::string& nonce) {
    return kHead + std::to_string(static_cast<int>(message.type)) + "\n" + message.fields.get("from") + "\n" +
           message.fields.get("file") + "\n" + std::to_string(message.body_size) + "\n" + nonce;
}

std::string mac_hex(const std::string& key, const std::string& text) {
    HmacSha256 mac(key);
    mac.update(text);
    return to_hex(mac.finish());
}

bool new_nonce(std::string& out) {
    std::uint8_t bytes[16];
    if (!secure_random(bytes, sizeof(bytes))) return false;
    out = to_hex(bytes, sizeof(bytes));
    return true;
}

bool is_nonce(const std::string& text) { return text.size() == 32 && normalize_pair_key(text) == text; }

}  // namespace

Sha256::Sha256()
    : h_{0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19} {}

void Sha256::block(const std::uint8_t* p) {
    std::uint32_t w[64];
    for (int i = 0; i < 16; ++i) {
        w[i] = (std::uint32_t(p[4 * i]) << 24) | (std::uint32_t(p[4 * i + 1]) << 16) |
               (std::uint32_t(p[4 * i + 2]) << 8) | std::uint32_t(p[4 * i + 3]);
    }
    for (int i = 16; i < 64; ++i) {
        const std::uint32_t s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        const std::uint32_t s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    std::uint32_t a = h_[0], b = h_[1], c = h_[2], d = h_[3], e = h_[4], f = h_[5], g = h_[6], h = h_[7];
    for (int i = 0; i < 64; ++i) {
        const std::uint32_t t1 = h + (rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)) + ((e & f) ^ (~e & g)) + kRound[i] + w[i];
        const std::uint32_t t2 = (rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
        h = g;
        g = f;
        f = e;
        e = d + t1;
        d = c;
        c = b;
        b = a;
        a = t1 + t2;
    }
    h_[0] += a;
    h_[1] += b;
    h_[2] += c;
    h_[3] += d;
    h_[4] += e;
    h_[5] += f;
    h_[6] += g;
    h_[7] += h;
}

void Sha256::update(const void* data, std::size_t n) {
    const auto* p = static_cast<const std::uint8_t*>(data);
    total_ += n;
    while (n > 0) {
        const std::size_t take = std::min<std::size_t>(n, 64 - used_);
        std::memcpy(buf_ + used_, p, take);
        used_ += take;
        p += take;
        n -= take;
        if (used_ == 64) {
            block(buf_);
            used_ = 0;
        }
    }
}

Digest Sha256::finish() {
    const std::uint64_t bits = total_ * 8;
    const std::uint8_t one = 0x80;
    update(&one, 1);
    const std::uint8_t zero = 0;
    while (used_ != 56) update(&zero, 1);
    std::uint8_t length[8];
    for (int i = 0; i < 8; ++i) length[i] = static_cast<std::uint8_t>(bits >> (56 - 8 * i));
    update(length, 8);
    Digest out{};
    for (int i = 0; i < 8; ++i) {
        out[4 * i] = static_cast<std::uint8_t>(h_[i] >> 24);
        out[4 * i + 1] = static_cast<std::uint8_t>(h_[i] >> 16);
        out[4 * i + 2] = static_cast<std::uint8_t>(h_[i] >> 8);
        out[4 * i + 3] = static_cast<std::uint8_t>(h_[i]);
    }
    return out;
}

HmacSha256::HmacSha256(const std::string& key) {
    std::array<std::uint8_t, 64> block{};
    if (key.size() > 64) {
        Sha256 hashed;
        hashed.update(key.data(), key.size());
        const Digest d = hashed.finish();
        std::memcpy(block.data(), d.data(), d.size());
    } else {
        std::memcpy(block.data(), key.data(), key.size());
    }
    std::array<std::uint8_t, 64> inner_key{};
    for (std::size_t i = 0; i < 64; ++i) {
        inner_key[i] = block[i] ^ 0x36;
        outer_key_[i] = block[i] ^ 0x5c;
    }
    inner_.update(inner_key.data(), inner_key.size());
}

void HmacSha256::update(const void* data, std::size_t n) { inner_.update(data, n); }

Digest HmacSha256::finish() {
    const Digest inner = inner_.finish();
    Sha256 outer;
    outer.update(outer_key_.data(), outer_key_.size());
    outer.update(inner.data(), inner.size());
    return outer.finish();
}

std::string to_hex(const std::uint8_t* data, std::size_t n) {
    static constexpr char kDigits[] = "0123456789abcdef";
    std::string out;
    out.reserve(n * 2);
    for (std::size_t i = 0; i < n; ++i) {
        out.push_back(kDigits[data[i] >> 4]);
        out.push_back(kDigits[data[i] & 0x0f]);
    }
    return out;
}

bool same_bytes(const std::uint8_t* a, const std::uint8_t* b, std::size_t n) {
    std::uint8_t diff = 0;
    for (std::size_t i = 0; i < n; ++i) diff |= static_cast<std::uint8_t>(a[i] ^ b[i]);
    return diff == 0;
}

bool same_text(const std::string& a, const std::string& b) {
    if (a.size() != b.size()) return false;
    return same_bytes(reinterpret_cast<const std::uint8_t*>(a.data()), reinterpret_cast<const std::uint8_t*>(b.data()),
                      a.size());
}

bool secure_random(std::uint8_t* out, std::size_t n) {
#ifdef _WIN32
    return BCryptGenRandom(nullptr, out, static_cast<ULONG>(n), BCRYPT_USE_SYSTEM_PREFERRED_RNG) == 0;
#else
    std::size_t got = 0;
    while (got < n) {
        const ssize_t r = ::getrandom(out + got, n - got, 0);
        if (r <= 0) return false;
        got += static_cast<std::size_t>(r);
    }
    return true;
#endif
}

std::string normalize_pair_key(const std::string& text) {
    std::string out;
    for (char c : text) {
        if (c >= '0' && c <= '9') out.push_back(c);
        else if (c >= 'a' && c <= 'f') out.push_back(c);
        else if (c >= 'A' && c <= 'F') out.push_back(static_cast<char>(c - 'A' + 'a'));
        else if (c == '-' || c == ' ' || c == '\t' || c == '\r' || c == '\n') continue;
        else return {};
    }
    return out.size() == 32 ? out : std::string();
}

std::string new_pair_key() {
    std::uint8_t bytes[16];
    if (!secure_random(bytes, sizeof(bytes))) return {};
    return to_hex(bytes, sizeof(bytes));
}

std::string display_pair_key(const std::string& key) {
    std::string out;
    for (std::size_t i = 0; i < key.size(); ++i) {
        if (i > 0 && i % 4 == 0) out.push_back('-');
        out.push_back(key[i]);
    }
    return out;
}

bool sign_head(Message& message, const std::string& key, PairedSend& state) {
    state.key = key;
    if (!new_nonce(state.nonce)) return false;
    message.fields.set("nonce", state.nonce);
    message.fields.set("auth", mac_hex(key, head_text(message, state.nonce)));
    return true;
}

bool await_ready(Socket& socket, PairedSend& state, const std::atomic<bool>* stop, std::string& error) {
    Message ready;
    if (!read_head(socket, ready, stop, error)) {
        error = "the PC did not answer the pairing (an older howldrop.exe?)";
        return false;
    }
    if (ready.type != MsgType::Ack) {
        error = "the PC did not answer the pairing";
        return false;
    }
    if (ready.fields.get("result") != "ready") {
        error = ready.fields.get("message");
        if (error.empty()) error = "the PC refused the pairing";
        return false;
    }
    const std::string nonce = ready.fields.get("nonce");
    if (!is_nonce(nonce) ||
        !same_text(ready.fields.get("auth"), mac_hex(state.key, kReady + state.nonce + "\n" + nonce))) {
        error = "the PC did not prove the pairing key: it is not this PC, or its key differs";
        return false;
    }
    state.peer_nonce = nonce;
    return true;
}

HmacSha256 body_mac(const std::string& key, const std::string& sender_nonce, const std::string& receiver_nonce) {
    HmacSha256 mac(key);
    mac.update(kBody + sender_nonce + "\n" + receiver_nonce + "\n");
    return mac;
}

OpenResult open_transfer(Socket& socket, Message& message, const std::string& key, std::optional<HmacSha256>& mac,
                         const std::atomic<bool>* stop, std::string& error) {
    mac.reset();
    PairedSend state;
    if (!key.empty() && !sign_head(message, key, state)) {
        error = "no randomness for the pairing nonce";
        return OpenResult::Lost;
    }
    if (!write_head(socket, message, stop, error)) return OpenResult::Lost;
    if (key.empty()) return OpenResult::Ok;
    if (!await_ready(socket, state, stop, error)) return OpenResult::Refused;
    mac.emplace(body_mac(key, state.nonce, state.peer_nonce));
    return OpenResult::Ok;
}

bool close_transfer(Socket& socket, std::optional<HmacSha256>& mac, const std::atomic<bool>* stop) {
    if (!mac) return true;
    const Digest trailer = mac->finish();
    return socket.send_full(trailer.data(), trailer.size(), stop);
}

HeadCheck check_head(const Message& message, const std::string& key, std::string& why) {
    const std::string* auth = message.fields.find("auth");
    if (!auth) {
        if (key.empty()) return HeadCheck::Unpaired;
        why = "this PC is paired: set pc_key to its pairing key (tray menu, Copy pairing key)";
        return HeadCheck::Refused;
    }
    if (key.empty()) {
        why = "this PC has no pairing key";
        return HeadCheck::Refused;
    }
    const std::string nonce = message.fields.get("nonce");
    if (!is_nonce(nonce) || !same_text(*auth, mac_hex(key, head_text(message, nonce)))) {
        why = "the pairing key does not match this PC's (tray menu, Copy pairing key)";
        return HeadCheck::Refused;
    }
    return HeadCheck::Paired;
}

bool send_ready(Socket& socket, const std::string& key, const std::string& sender_nonce, std::string& nonce,
                const std::atomic<bool>* stop) {
    if (!new_nonce(nonce)) return false;
    Message ready;
    ready.type = MsgType::Ack;
    ready.fields.set("result", "ready");
    ready.fields.set("nonce", nonce);
    ready.fields.set("auth", mac_hex(key, kReady + sender_nonce + "\n" + nonce));
    std::string error;
    return write_head(socket, ready, stop, error);
}

}  // namespace howldrop
