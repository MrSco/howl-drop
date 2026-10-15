#pragma once

#include "config.hpp"
#include "protocol.hpp"

#include <chrono>
#include <cstdint>
#include <filesystem>
#include <map>
#include <optional>
#include <string>
#include <string_view>

namespace howldrop {

// Identity the Pi bridge puts in the "from" field. The PC does not need to
// have discovered the Pi; it accepts the TCP transfer either way.
inline constexpr const char* kBridgeFromId = "70696272696467650000000000000001";

struct LanTarget {
    std::string host;
    std::uint16_t port = kTransferPort;
    std::string name;
    std::string id;
};

// Hello beacon -> the PC's name, transfer port, and the address it came from.
std::optional<LanTarget> target_from_hello(const std::uint8_t* data, std::size_t n, const std::string& from_ip);

// Bind discovery_port and wait for a Hello. A name_filter limits which PC.
// With no filter, a second distinct PC is an error so a file is not sent to
// the wrong machine.
std::optional<LanTarget> discover_target(std::uint16_t discovery_port, std::chrono::milliseconds timeout,
                                        const std::string& name_filter, std::string& error);

// Send a file. advertised_name is the header the PC sanitizes; an empty name
// uses the path's filename. The PC stores the sanitized base name in its inbox.
Status forward_file(const LanTarget& target, const std::filesystem::path& path, const std::string& advertised_name);

// Send UTF-8 text. The PC sets its clipboard to that text.
Status forward_text(const LanTarget& target, std::string_view text);

enum class DropKind { Skip, File, Text };

// inbox/photo.jpg is a file. inbox/text/note.txt is clipboard text.
// Anything outside the inbox, a symlink, a dotfile, or a *.partial is skipped.
DropKind classify_inbox_file(const std::filesystem::path& inbox, const std::filesystem::path& file);

enum class DropAction { Skip, SentFile, SentText, Retry, Rejected };

struct DropResult {
    DropAction action = DropAction::Skip;
    std::string message;
};

// Classify one inbox entry, forward it, and move it aside when the PC has
// accepted it or has permanently refused it. A PC that is down leaves the
// file in place so a later pass can retry. With keep, the file stays where
// it is either way: the inbox is a shared folder people read, and the
// caller remembers what it has handled.
DropResult take_drop(const std::filesystem::path& inbox, const std::filesystem::path& file, const LanTarget& target,
                     bool keep = false);

// What keep mode has dealt with: each file by path, size and modification
// time. A new file under a name that was used before (deleted from the
// share, then the next drop takes the free name) is not the same file, so it
// is forwarded; the add-on missed a photo that way (0.11.3).
class HandledFiles {
public:
    void add(const std::filesystem::path& file);
    // The file at this path is the one that was handled, unchanged.
    bool has(const std::filesystem::path& file) const;
    std::size_t size() const { return files_.size(); }

private:
    static std::string identity(const std::filesystem::path& file);
    std::map<std::string, std::string> files_;
};

}  // namespace howldrop
