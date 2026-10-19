#pragma once

// What howldrop.exe reads and writes when it runs the radio in WSL
// (wsl_win.hpp), kept apart from Windows so the tests run anywhere:
// `usbipd list` and `wsl --list` output, the stack's event lines, and
// Windows paths as WSL sees them.

#include <optional>
#include <string>
#include <vector>

namespace howldrop::wsl {

// The USB Wi-Fi stick the stack drives: MediaTek MT7612U.
inline constexpr const char* kStickId = "0e8d:7612";
// The distro howldrop.exe imports and runs.
inline constexpr const char* kDistro = "HowlDrop";

struct UsbDevice {
    std::string busid;   // 2-1
    std::string id;      // 0e8d:7612, lower case
    std::string name;    // 802.11ac Wireless LAN Card
    std::string state;   // Not shared, Shared, Shared (forced), Attached
    bool shared() const { return state.rfind("Shared", 0) == 0 || attached(); }
    bool attached() const { return state.rfind("Attached", 0) == 0; }
};

// The devices under Connected: in `usbipd list` (columns apart by two or
// more spaces).
std::vector<UsbDevice> parse_usbipd_list(const std::string& text);
std::optional<UsbDevice> find_stick(const std::vector<UsbDevice>& devices);

// `wsl --list --quiet` (WSL_UTF8=1): one distro per line.
std::vector<std::string> parse_distros(const std::string& text);

// A Windows path on a drive (C:\Users\x\Downloads\HowlDrop) as WSL mounts
// it (/mnt/c/Users/x/Downloads/HowlDrop). Nothing for a network path.
std::optional<std::string> wsl_path(const std::string& windows_path);

// An event in the stack's output: the fields after "@howldrop", which may
// follow a prefix ("Deliver: @howldrop\treceived\t...").
std::optional<std::vector<std::string>> event_fields(const std::string& line);

}  // namespace howldrop::wsl
