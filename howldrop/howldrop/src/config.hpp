#pragma once

#include <cstddef>
#include <cstdint>

namespace howldrop {

// First slice of HowlDrop for Windows. Peers share an ordinary IP network.
// Nothing here talks to a Wi-Fi driver, Bluetooth, or AWDL.
constexpr const char* kVersion = "0.12.0";
constexpr std::uint16_t kDiscoveryPort = 47631;
constexpr std::uint16_t kTransferPort = 47632;
constexpr std::uint16_t kControlPort = 47633;
constexpr std::uint64_t kMaxClipBytes = 1u << 20;
constexpr std::uint64_t kMaxFileBytes = 8ull << 30;
constexpr std::size_t kMaxHeaderBytes = 8192;

}  // namespace howldrop
