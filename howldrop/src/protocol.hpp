#pragma once

#include "net.hpp"

#include <atomic>
#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace howldrop {

enum class MsgType : std::uint8_t { Hello = 1, File = 2, Clip = 3, Ack = 4 };

struct Fields {
    std::vector<std::pair<std::string, std::string>> items;
    void set(std::string key, std::string value);
    const std::string* find(std::string_view key) const;
    std::string get(std::string_view key) const;
};

struct Message {
    MsgType type{};
    Fields fields;
    std::uint64_t body_size = 0;
};

struct Status {
    bool ok = true;
    std::string message;
    static Status success(std::string message = {}) { return Status{true, std::move(message)}; }
    static Status failure(std::string message) { return Status{false, std::move(message)}; }
};

std::optional<std::vector<std::uint8_t>> encode_head(const Message& message, std::string& error);
bool parse_prefix(const std::uint8_t prefix[16], MsgType& type, std::uint16_t& header_len,
                  std::uint64_t& body_size, std::string& error);
std::optional<Message> parse_datagram(const std::uint8_t* data, std::size_t n, std::string& error);

bool write_head(Socket& socket, const Message& message, const std::atomic<bool>* stop, std::string& error);
bool read_head(Socket& socket, Message& message, const std::atomic<bool>* stop, std::string& error);

std::string sanitize_filename(std::string name);
std::string one_line(std::string text);
bool is_node_id(const std::string& id);
bool valid_display_name(const std::string& name);
bool names_equal(const std::string& a, const std::string& b);

}  // namespace howldrop
