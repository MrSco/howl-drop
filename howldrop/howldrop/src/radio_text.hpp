#pragma once

// What howldrop.exe reads from the radio's agent (radio_win.hpp), kept apart
// from Windows so the tests run anywhere: its event lines, and when to start
// it again.

#include <optional>
#include <array>
#include <cstdint>
#include <string>
#include <vector>

namespace howldrop::radio {

// An event in the agent's output: the fields after "@howldrop", which may
// follow a prefix ("Deliver: @howldrop\treceived\t...").
std::optional<std::vector<std::string>> event_fields(const std::string& line);

struct SenderIdentityAdvert {
    std::uint64_t expires = 0;
    std::vector<std::array<unsigned char, 2>> prefixes;
};
std::optional<SenderIdentityAdvert> sender_identity(const std::vector<std::string>& event);
std::array<unsigned char, 8> advert_hashes(const SenderIdentityAdvert& identity, std::size_t offset, std::uint64_t now);

// After the radio stopped by itself, once it had been on the share sheet
// (the PC slept, the stick went away): how long to wait before starting it
// again, in ms, or -1 to stop and say why. failed: the starts in a row since
// it was last up that didn't get it back there (0: it was up until it
// stopped).
int restart_delay_ms(int failed);

}  // namespace howldrop::radio
