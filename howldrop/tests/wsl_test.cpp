// AirDrop on this PC (wsl_win.hpp): what it reads from usbipd, wsl.exe and the
// stack in WSL, and the inbox's 30-day cleanup (purge.hpp).
#include "purge.hpp"
#include "wsl_text.hpp"

#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>

namespace wsl = howldrop::wsl;

namespace {

int failures = 0;

void expect(bool ok, const std::string& what) {
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
        ++failures;
    }
}

void touch(const std::filesystem::path& path) {
    std::filesystem::create_directories(path.parent_path());
    std::ofstream(path, std::ios::binary) << "x";
}

}  // namespace

int main() {
    // usbipd-win 5.3, the stick shared and a mouse not.
    const std::string list =
        "Connected:\r\n"
        "BUSID  VID:PID    DEVICE                                                        STATE\r\n"
        "1-4    046d:c52b  Logitech USB Input Device, USB Input Device                   Not shared\r\n"
        "2-1    0E8D:7612  802.11ac Wireless LAN Card                                    Shared (forced)\r\n"
        "\r\n"
        "Persisted:\r\n"
        "GUID                                  DEVICE\r\n"
        "6a6b8c1e-0d7f-4e3a-9a8b-1c2d3e4f5a6b  1-9 old stick\r\n";
    const auto devices = wsl::parse_usbipd_list(list);
    expect(devices.size() == 2, "two connected devices, nothing from Persisted");
    const auto stick = wsl::find_stick(devices);
    expect(stick.has_value(), "the MT7612U is found, whatever the case of its id");
    if (stick) {
        expect(stick->busid == "2-1", "its bus id");
        expect(stick->state == "Shared (forced)" && stick->shared() && !stick->attached(), "its state");
        expect(stick->name == "802.11ac Wireless LAN Card", "its name");
    }
    expect(devices.size() == 2 && devices[0].state == "Not shared" && !devices[0].shared(), "not shared");
    expect(devices.size() == 2 && devices[0].name == "Logitech USB Input Device, USB Input Device",
           "a name with commas and single spaces");
    const auto attached = wsl::parse_usbipd_list("Connected:\nBUSID  VID:PID    DEVICE  STATE\n"
                                                 "2-1    0e8d:7612  802.11ac Wireless LAN Card  Attached\n");
    expect(attached.size() == 1 && attached[0].attached() && attached[0].shared(), "attached counts as shared");
    expect(!wsl::find_stick(wsl::parse_usbipd_list("Connected:\nBUSID  VID:PID    DEVICE  STATE\n")),
           "no stick, nothing found");

    const auto distros = wsl::parse_distros("Ubuntu\r\nHowlDrop\r\n\r\n");
    expect(distros.size() == 2 && distros[1] == wsl::kDistro, "distros, one per line");
    const std::string utf16("U\0b\0u\0n\0t\0u\0\r\0\n\0", 16);
    const auto wide = wsl::parse_distros(utf16);
    expect(wide.size() == 1 && wide[0] == "Ubuntu", "UTF-16 output without WSL_UTF8");

    expect(wsl::wsl_path("C:\\Users\\Occor\\Downloads\\HowlDrop") == std::string("/mnt/c/Users/Occor/Downloads/HowlDrop"),
           "a drive path");
    expect(wsl::wsl_path("D:/x/") == std::string("/mnt/d/x"), "forward slashes, trailing slash dropped");
    expect(wsl::wsl_path("C:\\") == std::string("/mnt/c"), "a drive's root");
    expect(!wsl::wsl_path("\\\\server\\share\\x"), "a network path is not on a drive WSL mounts");
    expect(!wsl::wsl_path("relative\\x"), "a relative path");

    auto f = wsl::event_fields("Deliver: @howldrop\treceived\ttext/a.txt\tRocco's iPhone\ttext");
    expect(f && f->size() == 4 && (*f)[0] == "received" && (*f)[1] == "text/a.txt" && (*f)[3] == "text",
           "an event after a prefix");
    f = wsl::event_fields("@howldrop\tnearby\tdone");
    expect(f && f->size() == 2 && (*f)[1] == "done", "an event on its own");
    f = wsl::event_fields("@howldrop\tsent\ta.jpg\tfailed\t\r");
    expect(f && f->size() == 4 && (*f)[3].empty(), "an empty last field, and a CR");
    f = wsl::event_fields("@howldrop");
    expect(f && f->empty(), "no fields");
    expect(!wsl::event_fields("Deliver: delivered IMG_1.JPG"), "a log line");
    expect(!wsl::event_fields("x@howldrop\tup"), "@howldrop inside a word");
    expect(!wsl::event_fields("@howldropper\tup"), "a longer word");

    // The cleanup: old files go, new ones and the folders that hold them stay.
    const auto dir = std::filesystem::temp_directory_path() / "howldrop-wsl-test";
    std::filesystem::remove_all(dir);
    touch(dir / "IMG_1.JPG");
    touch(dir / "text" / "text-1.txt");
    touch(dir / "send" / "sent" / "a.jpg");
    const auto now = std::chrono::system_clock::now();
    expect(howldrop::purge_old_files(dir, 30, now) == 0, "nothing goes before its time");
    expect(howldrop::purge_old_files(dir, 0, now + std::chrono::hours(24 * 400)) == 0, "0 days keeps everything");
    expect(howldrop::purge_old_files(dir, 30, now + std::chrono::hours(24 * 31)) == 3, "after 30 days all three go");
    expect(std::filesystem::exists(dir) && std::filesystem::exists(dir / "text"), "the inbox and text/ stay");
    expect(!std::filesystem::exists(dir / "send"), "emptied folders go");
    expect(howldrop::purge_old_files(dir / "missing", 30, now) == 0, "a missing folder is fine");
    std::filesystem::remove_all(dir);

    if (failures == 0) std::printf("wsl_test passed\n");
    return failures == 0 ? 0 : 1;
}
