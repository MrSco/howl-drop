#pragma once

// What howldrop.exe reads and writes when it runs the radio in WSL
// (wsl_win.hpp), kept apart from Windows so the tests run anywhere:
// `usbipd list` and `wsl --list` output, the stack's event lines, and
// Windows paths as WSL sees them.

#include <optional>
#include <string>
#include <vector>

namespace howldrop::wsl {

// A USB Wi-Fi stick the stack can drive in WSL: the driver WSL's kernel
// binds to it (wsl-run.sh --driver), its chip, and whether it has been tested
// with an iPhone (else it runs as untested, and says so).
struct StickKind {
    const char* id;  // 0e8d:7612
    const char* driver;
    const char* chip;
    bool tested;
};

// Every USB ID the WSL kernel's mt76x2u and rtw89_8852bu bind (its
// modules.alias), less 7392:b711, which mt76x0u claims too. The MT7612U's is
// the tested one; the RTL8852BU runs in Owly's exclusive mode, untested here.
// wsl-run.sh and the image's firmware (addon/Dockerfile) follow this table.
inline constexpr StickKind kSticks[] = {
    {"045e:02e6", "mt76x2u", "MT7612U", true},
    {"045e:02fe", "mt76x2u", "MT7612U", true},
    {"0471:2126", "mt76x2u", "MT7612U", true},
    {"0471:7600", "mt76x2u", "MT7612U", true},
    {"056e:400a", "mt76x2u", "MT7612U", true},
    {"057c:8503", "mt76x2u", "MT7612U", true},
    {"0846:9014", "mt76x2u", "MT7612U", true},
    {"0846:9053", "mt76x2u", "MT7612U", true},
    {"0b05:17eb", "mt76x2u", "MT7612U", true},
    {"0b05:180b", "mt76x2u", "MT7612U", true},
    {"0b05:1833", "mt76x2u", "MT7612U", true},
    {"0e8d:7612", "mt76x2u", "MT7612U", true},
    {"0e8d:7632", "mt76x2u", "MT7612U", true},
    {"2357:0137", "mt76x2u", "MT7612U", true},
    {"2c4e:0103", "mt76x2u", "MT7612U", true},
    {"0586:3428", "rtw89_8852bu", "RTL8852BU", false},
    {"0b05:1a62", "rtw89_8852bu", "RTL8852BU", false},
    {"0bda:a85b", "rtw89_8852bu", "RTL8852BU", false},
    {"0bda:b832", "rtw89_8852bu", "RTL8852BU", false},
    {"0bda:b83a", "rtw89_8852bu", "RTL8852BU", false},
    {"0bda:b852", "rtw89_8852bu", "RTL8852BU", false},
    {"0bda:b85a", "rtw89_8852bu", "RTL8852BU", false},
    {"0db0:6931", "rtw89_8852bu", "RTL8852BU", false},
    {"0db0:f0c8", "rtw89_8852bu", "RTL8852BU", false},
    {"2001:3327", "rtw89_8852bu", "RTL8852BU", false},
    {"3574:6121", "rtw89_8852bu", "RTL8852BU", false},
    {"35bc:0100", "rtw89_8852bu", "RTL8852BU", false},
    {"35bc:0108", "rtw89_8852bu", "RTL8852BU", false},
    {"7392:6822", "rtw89_8852bu", "RTL8852BU", false},
};

// The stick kind for a USB ID, lower case; null for none.
const StickKind* stick_kind(const std::string& id);
// The distro howldrop.exe imports and runs.
inline constexpr const char* kDistro = "HowlDrop";

struct UsbDevice {
    std::string busid;   // 2-1
    std::string id;      // 0e8d:7612, lower case
    std::string name;    // 802.11ac Wireless LAN Card
    std::string state;   // Not shared, Shared, Shared (forced), Attached
    const StickKind* kind = nullptr;  // find_stick: what it is
    bool shared() const { return state.rfind("Shared", 0) == 0 || attached(); }
    bool attached() const { return state.rfind("Attached", 0) == 0; }
};

// The devices under Connected: in `usbipd list` (columns apart by two or
// more spaces).
std::vector<UsbDevice> parse_usbipd_list(const std::string& text);
// The stick to use of those connected: a tested one first.
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
