// The transfer cards' model (transfers.hpp): event sequences from the radio
// in WSL and the Home Assistant route's callbacks, and what each card shows.
#include "history.hpp"
#include "text.hpp"
#include "transfers.hpp"

#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <vector>

using howldrop::Transfers;
using Stage = Transfers::Stage;

namespace {

int failures = 0;

void expect(bool ok, const std::string& what) {
    if (!ok) {
        std::fprintf(stderr, "FAIL: %s\n", what.c_str());
        ++failures;
    }
}

Transfers::Clock::time_point at(int ms) { return Transfers::Clock::time_point(std::chrono::milliseconds(ms)); }

}  // namespace

int main() {
    {
        // Everyone, ask me: asked, accepted, the upload, stored, delivered.
        Transfers t;
        expect(t.apply({"ask", "a1", "Rocco’s iPhone", "2", "31500000", "IMG_1.HEIC", "IMG_2.MOV"}, at(0)) == "a1",
               "an ask makes a transfer");
        const auto* x = t.find("a1");
        expect(x && x->stage == Stage::Asking && x->count == 2 && x->total == 31500000 && x->peer == "Rocco’s iPhone",
               "it asks, with who, how many and how big");
        t.apply({"asked", "a1", "accepted"}, at(2000));
        t.apply({"incoming", "a1", "Rocco’s iPhone", "2", "31500000", "IMG_1.HEIC", "IMG_2.MOV"}, at(2000));
        expect(t.find("a1")->stage == Stage::Waiting, "accepted: waiting for the upload");
        t.apply({"receiving", "a1", "0", "31500000"}, at(2100));
        t.apply({"receiving", "a1", "15750000", "31500000"}, at(5100));
        x = t.find("a1");
        expect(x->stage == Stage::Moving && x->fraction() > 0.49 && x->fraction() < 0.51, "halfway");
        expect(x->rate > 5.0e6 && x->rate < 5.5e6, "about 5.25 MB/s");
        t.apply({"stored", "a1", "IMG_1.HEIC", "IMG_2.MOV"}, at(7000));
        expect(t.find("a1")->stage == Stage::Saving, "stored: being moved in");
        t.received("C:/in/IMG_1.HEIC", "Rocco’s iPhone", "image", false, at(7100));
        expect(t.find("a1")->stage == Stage::Saving, "one of two delivered");
        // deliver.py renamed the second beside an older one.
        expect(t.received("C:/in/IMG_2-1.MOV", "Rocco’s iPhone", "video", false, at(7200)) == "a1",
               "a renamed file still belongs to its transfer");
        x = t.find("a1");
        expect(x->stage == Stage::Done && x->saved.size() == 2 && x->kind == "image", "both in: done");
        expect(t.all().size() == 1, "no stray transfers");
        // From the first byte (2.1 s) to stored (7 s).
        expect(Transfers::took_text(*x) == "31.5 MB in 4.9 s (6.4 MB/s)",
               "how long it took: " + Transfers::took_text(*x));
    }
    {
        // Declined, and an ask nobody answers.
        Transfers t;
        t.apply({"ask", "d", "Pat", "1", "10", "x.jpg"}, at(0));
        t.apply({"asked", "d", "declined"}, at(1000));
        expect(t.find("d")->stage == Stage::Declined && t.find("d")->detail == "declined", "declined");
        t.apply({"ask", "q", "Pat", "1", "10", "x.jpg"}, at(0));
        expect(t.expire(at(59000)).empty(), "still asking at 59 s");
        const auto changed = t.expire(at(60000));
        expect(changed.size() == 1 && t.find("q")->stage == Stage::Declined, "unanswered at 60 s: declined");
    }
    {
        // Everyone (no ask), a stall, and a failure the receiver reports.
        Transfers t;
        t.apply({"incoming", "s", "Pat", "1", "100", "a.mov"}, at(0));
        t.apply({"receiving", "s", "10", "100"}, at(1000));
        expect(t.expire(at(60000)).empty(), "quiet for 59 s is fine");
        t.expire(at(92000));
        expect(t.find("s")->stage == Stage::Failed, "quiet for 91 s: stopped");
        t.apply({"incoming", "f", "Pat", "1", "100", "b.mov"}, at(0));
        t.apply({"failed", "f", "the transfer stalled"}, at(5000));
        expect(t.find("f")->stage == Stage::Failed && t.find("f")->detail == "the transfer stalled", "failed");
        expect(t.apply({"receiving", "nope", "1", "2"}, at(0)).empty(), "an unknown id touches nothing");
    }
    {
        // A link, and shared text, from the radio.
        Transfers t;
        t.apply({"incoming", "l", "Pat", "1", "0", "https://example.com/"}, at(0));
        expect(t.received("C:/in/example.com.url", "Pat", "link", false, at(500)) == "l", "the link's shortcut");
        expect(t.find("l")->stage == Stage::Done && t.find("l")->kind == "link", "link done");
        t.apply({"incoming", "x", "Pat", "1", "12", "text-1A.txt"}, at(0));
        t.apply({"stored", "x", "text-1A.txt"}, at(100));
        expect(t.received("C:/in/text/text-1A.txt", "Pat", "text", false, at(200)) == "x" &&
                   t.find("x")->kind == "text",
               "text");
    }
    {
        // Home Assistant: files arrive whole, each a card of its own.
        Transfers t;
        const auto a = t.received("C:/in/IMG_9.JPG", "Rocco’s iPhone", "", true, at(0));
        const auto* x = t.find(a);
        expect(x && x->stage == Stage::Done && x->home_assistant && x->kind == "image", "a done card from HA");
        t.apply({"incoming", "r", "Pat", "1", "10", "IMG_9.JPG"}, at(0));
        const auto b = t.received("C:/in/IMG_9.JPG", "Pat", "", true, at(100));
        expect(b != "r", "a file from HA never completes the radio's transfer");
    }
    {
        // Sending: queued here, the sender's stages, the result.
        Transfers t;
        const auto id = t.sending("japan-trip.png", "Rocco’s iPhone", false, at(0));
        expect(t.find(id)->stage == Stage::Waiting && t.find(id)->peer == "Rocco’s iPhone", "queued");
        t.apply({"sending", "japan-trip.png", "looking for the iPhone"}, at(100));
        t.apply({"sending", "japan-trip.png", "asking: accept it on the iPhone"}, at(1000));
        expect(t.find(id)->stage == Stage::Accepting, "the phone shows Accept");
        t.apply({"sending", "japan-trip.png", "accepted; uploading"}, at(4000));
        expect(t.find(id)->stage == Stage::Moving && t.find(id)->fraction() < 0, "uploading, size unknown");
        t.apply({"sent", "japan-trip.png", "sent", "on the iPhone"}, at(7000));
        expect(t.find(id)->stage == Stage::Done, "sent");
        expect(t.all().size() == 1, "the events found the queued transfer");
        const auto no = t.sending("b.jpg", "", true, at(0));
        t.sent("b.jpg", false, "not accepted on the iPhone in time: send it again and tap Accept", at(30000));
        expect(t.find(no)->stage == Stage::Failed, "not sent");
        t.sent("c.jpg", false, "declined on the iPhone", at(0));
        expect(t.all().back().stage == Stage::Declined, "declined, even unannounced");
        t.forget(no);
        expect(!t.find(no), "forgotten");
    }
    {
        // The window's Recent list.
        namespace history = howldrop::history;
        const auto dir = std::filesystem::temp_directory_path() / "howldrop-history-test";
        std::filesystem::remove_all(dir);
        std::filesystem::create_directories(dir);
        const auto file = dir / "history.tsv";
        const auto photo = dir / "IMG_1.HEIC";
        std::ofstream(photo) << "x";
        history::append(file, {100, false, "Rocco’s\tiPhone", howldrop::path_to_utf8(photo)});
        history::append(file, {200, true, "Rocco’s iPhone", "japan-trip.png"});
        history::append(file, {300, false, "Pat", howldrop::path_to_utf8(dir / "gone.jpg")});
        auto recent = history::load(file, 10);
        expect(recent.size() == 2, "a received file that's gone is left out");
        expect(recent.size() == 2 && recent[0].out && recent[0].item == "japan-trip.png" && recent[0].time == 200,
               "newest first");
        expect(recent.size() == 2 && recent[1].peer == "Rocco’s iPhone", "a tab in a name can't split a line");
        expect(history::load(file, 1).size() == 1, "at most the limit");
        for (int i = 0; i < 2 * static_cast<int>(history::kKept) + 1; ++i) {
            history::append(file, {1000 + i, true, "Pat", "f" + std::to_string(i)});
        }
        recent = history::load(file, 1000);
        expect(recent.size() <= 2 * history::kKept && recent.front().item == "f400", "trimmed, newest kept");
        expect(history::load(dir / "none.tsv", 10).empty(), "no file, no history");
        std::filesystem::remove_all(dir);
    }
    expect(Transfers::kind_of("x/IMG.HEIC") == "image" && Transfers::kind_of("x/a.MOV") == "video" &&
               Transfers::kind_of("x/text/t.txt") == "text" && Transfers::kind_of("x/a.url") == "link" &&
               Transfers::kind_of("x/a.pdf") == "file",
           "kinds");

    if (failures == 0) std::printf("transfers_test passed\n");
    return failures == 0 ? 0 : 1;
}
