#pragma once

// Pairing: only a sender that knows this PC's key can deliver to it, and a
// sender only sends to a PC that proves it knows the key too. The key is 128
// random bits written as 32 hex digits; howldrop.exe makes one, and the
// add-on's pc_key option holds a copy.
//
// A paired transfer, on the usual TCP connection:
//   1. The sender's head carries `nonce` (fresh, random) and `auth`, an
//      HMAC-SHA256 of the head under the key. A paired PC refuses a head
//      without it, or with a wrong one, before reading any body.
//   2. The PC answers with an Ack whose result is `ready`, carrying its own
//      fresh `nonce` and an `auth` over both nonces. The sender checks it
//      before sending a byte of the file: a machine that only claims to be
//      the PC learns nothing.
//   3. The body follows, then 32 bytes: an HMAC over both nonces and the
//      body. A changed body, or a replayed transfer (the PC's nonce is new
//      each time), fails here and the file is dropped. No clocks involved.
// A sender with no key, to a PC with no key, is the old unpaired protocol.

#include "net.hpp"
#include "protocol.hpp"

#include <array>
#include <atomic>
#include <cstddef>
#include <cstdint>
#include <optional>
#include <string>

namespace howldrop {

using Digest = std::array<std::uint8_t, 32>;

class Sha256 {
public:
    Sha256();
    void update(const void* data, std::size_t n);
    Digest finish();

private:
    void block(const std::uint8_t* p);
    std::uint32_t h_[8];
    std::uint8_t buf_[64];
    std::size_t used_ = 0;
    std::uint64_t total_ = 0;
};

class HmacSha256 {
public:
    explicit HmacSha256(const std::string& key);
    void update(const void* data, std::size_t n);
    void update(const std::string& text) { update(text.data(), text.size()); }
    Digest finish();

private:
    Sha256 inner_;
    std::array<std::uint8_t, 64> outer_key_{};
};

std::string to_hex(const std::uint8_t* data, std::size_t n);
inline std::string to_hex(const Digest& d) { return to_hex(d.data(), d.size()); }
bool same_bytes(const std::uint8_t* a, const std::uint8_t* b, std::size_t n);  // constant time
bool same_text(const std::string& a, const std::string& b);                    // constant time

// Random bytes from the operating system; false if it cannot give them.
bool secure_random(std::uint8_t* out, std::size_t n);

// The key as stored and compared: 32 lowercase hex digits, from text that
// may have dashes, spaces or capitals. Empty when it is not a key.
std::string normalize_pair_key(const std::string& text);
// A new key, normalized; empty if the OS gave no randomness.
std::string new_pair_key();
// For people: 8 groups of 4 digits joined by dashes.
std::string display_pair_key(const std::string& key);

// --- the sender's side ---------------------------------------------------------------
struct PairedSend {
    std::string key;
    std::string nonce;       // ours
    std::string peer_nonce;  // the PC's, from its ready
};
// Adds nonce and auth to a head about to be written. False: no randomness.
bool sign_head(Message& message, const std::string& key, PairedSend& state);
// After writing the head: wait for the PC's ready and check it. On failure,
// error says why (the PC's own refusal, or that it did not prove the key).
bool await_ready(Socket& socket, PairedSend& state, const std::atomic<bool>* stop, std::string& error);
HmacSha256 body_mac(const std::string& key, const std::string& sender_nonce, const std::string& receiver_nonce);

enum class OpenResult { Ok, Lost, Refused };
// Writes message's head; with a key, signs it first and waits for the PC's
// ready, and mac is then set to cover the body. Lost: the connection failed.
// Refused: the PC refused, or did not prove the key; error says which.
OpenResult open_transfer(Socket& socket, Message& message, const std::string& key, std::optional<HmacSha256>& mac,
                         const std::atomic<bool>* stop, std::string& error);
// After the body: the 32-byte trailer, when mac is set.
bool close_transfer(Socket& socket, std::optional<HmacSha256>& mac, const std::atomic<bool>* stop);

// --- the receiver's side -------------------------------------------------------------
enum class HeadCheck { Unpaired, Paired, Refused };
// Unpaired: neither side has a key. Paired: the head is signed with our key.
// Refused: why says what to tell the sender.
HeadCheck check_head(const Message& message, const std::string& key, std::string& why);
// Sends the ready Ack; nonce is set to ours for body_mac.
bool send_ready(Socket& socket, const std::string& key, const std::string& sender_nonce, std::string& nonce,
                const std::atomic<bool>* stop);

}  // namespace howldrop
