// AirDrop on this PC (radio_win.hpp): what it reads from the agent, and the
// inbox's 30-day cleanup (purge.hpp).
#include "purge.hpp"
#include "radio_text.hpp"

#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>

namespace radio = howldrop::radio;

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
    const auto identity = radio::sender_identity({"sender-identity", "ready", "2000", "1234", "5678", "9abc", "def0", "aabb"});
    expect(identity.has_value(), "trusted BLE metadata parses");
    if (identity) {
        const auto first = radio::advert_hashes(*identity, 0, 1000);
        expect(first[0] == 0x12 && first[7] == 0xf0, "four contact prefixes fill the BLE slots");
        const auto next = radio::advert_hashes(*identity, 4, 1000);
        expect(next[0] == 0xaa && next[1] == 0xbb && next[2] == 0x12, "additional contacts rotate through the slots");
        expect(radio::advert_hashes(*identity, 0, 2000) == std::array<unsigned char,8>{}, "expired identities stop advertising contact hashes");
    }
    expect(!radio::sender_identity({"sender-identity", "ready", "18446744073709551616", "1234"}), "expiry overflow is refused");
    expect(!radio::sender_identity({"sender-identity", "ready", "2000", "12xx"}), "invalid hashes are refused");
    expect(!radio::sender_identity({"sender-identity", "ready", "2000"}), "missing hashes are refused");
    auto f = radio::event_fields("Deliver: @howldrop\treceived\ttext/a.txt\tRocco's iPhone\ttext");
    expect(f && f->size() == 4 && (*f)[0] == "received" && (*f)[1] == "text/a.txt" && (*f)[3] == "text",
           "an event after a prefix");
    f = radio::event_fields("@howldrop\tnearby\tdone");
    expect(f && f->size() == 2 && (*f)[1] == "done", "an event on its own");
    f = radio::event_fields("@howldrop\tsent\ta.jpg\tfailed\t\r");
    expect(f && f->size() == 4 && (*f)[3].empty(), "an empty last field, and a CR");
    // The radio stopped by itself: again at once after it had been up, then
    // waiting longer, and giving up after four starts that didn't take.
    expect(radio::restart_delay_ms(0) == 3000, "lost while up: again in 3 s");
    expect(radio::restart_delay_ms(1) > radio::restart_delay_ms(0) && radio::restart_delay_ms(3) == 30000,
           "each failed start waits longer");
    expect(radio::restart_delay_ms(4) == -1, "four failed starts in a row: stop and say why");
    f = radio::event_fields("@howldrop\tstopped\tThe Wi-Fi interface wlan0 doesn't exist.");
    expect(f && f->size() == 2 && (*f)[0] == "stopped", "why the agent stopped");
    f = radio::event_fields("@howldrop");
    expect(f && f->empty(), "no fields");
    expect(!radio::event_fields("Deliver: delivered IMG_1.JPG"), "a log line");
    expect(!radio::event_fields("x@howldrop\tup"), "@howldrop inside a word");
    expect(!radio::event_fields("@howldropper\tup"), "a longer word");

    // The cleanup: old files go, new ones and the folders that hold them stay.
    const auto dir = std::filesystem::temp_directory_path() / "howldrop-radio-test";
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

    if (failures == 0) std::printf("radio_text_test passed\n");
    return failures == 0 ? 0 : 1;
}
