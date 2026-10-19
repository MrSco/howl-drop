// `howldrop screenshots DIR` (tray.hpp): the README's pictures, drawn by the
// same code as the real window and cards, from made-up data, so they show
// no one's names and stay true as the UI changes.

#include "tray.hpp"

#include "cards_win.hpp"
#include "history.hpp"
#include "text.hpp"
#include "transfers.hpp"
#include "ui_win.hpp"
#include "window_win.hpp"

#include <windows.h>
#include <objbase.h>

#include <chrono>
#include <fstream>
#include <iostream>

namespace howldrop {

namespace fs = std::filesystem;
using Stage = Transfers::Stage;

int make_screenshots(const fs::path& dir) {
    const bool com = SUCCEEDED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED));
    std::error_code ec;
    fs::create_directories(dir, ec);
    if (!ui::init()) {
        std::cerr << "Direct2D isn't available here\n";
        return 1;
    }
    const float scale = 2.0f;  // sharp on a high-DPI screen, and in a README
    const auto now = Transfers::Clock::now();
    const std::wstring inbox = L"Downloads\\HowlDrop";
    bool ok = true;

    // The cards: an ask, its progress, the result, and a send.
    Transfers::Transfer ask;
    ask.id = "demo";
    ask.direction = Transfers::Direction::In;
    ask.peer = "Alex’s iPhone";
    ask.names = {"IMG_4821.MOV"};
    ask.count = 1;
    ask.total = 30040956;
    ask.stage = Stage::Asking;
    ask.deadline = now + std::chrono::seconds(41);
    Transfers::Transfer moving = ask;
    moving.stage = Stage::Moving;
    moving.bytes = 14400000;
    moving.rate = 5100000;
    Transfers::Transfer done = ask;
    done.stage = Stage::Done;
    done.kind = "video";
    done.saved = {fs::path("C:/Users/You/Downloads/HowlDrop/IMG_4821.MOV")};
    Transfers::Transfer sending;
    sending.id = "out";
    sending.direction = Transfers::Direction::Out;
    sending.peer = "Alex’s iPhone";
    sending.names = {"japan-trip.png"};
    sending.count = 1;
    sending.stage = Stage::Accepting;
    sending.detail = "asking: accept it on the iPhone";
    ok &= render_card(ask, inbox, dir / "card-ask.png", scale);
    ok &= render_card(moving, inbox, dir / "card-receiving.png", scale);
    ok &= render_card(done, inbox, dir / "card-received.png", scale);
    ok &= render_card(sending, inbox, dir / "card-sending.png", scale);

    // The window: a Recent list from stand-in files, a transfer under way.
    const fs::path stage = fs::temp_directory_path() / "howldrop-screenshots";
    fs::remove_all(stage, ec);
    fs::create_directories(stage, ec);
    const fs::path history = stage / "history.tsv";
    const std::int64_t t = std::chrono::duration_cast<std::chrono::seconds>(
                               std::chrono::system_clock::now().time_since_epoch())
                               .count();
    const auto file = [&](const char* name) {
        const fs::path p = stage / name;
        std::ofstream(p) << "x";
        return path_to_utf8(p);
    };
    history::append(history, {t - 86400 * 2, false, "Sam’s iPad", file("Quarterly notes.pdf")});
    history::append(history, {t - 90000, false, "Alex’s iPhone", file("www.example.com-recipe.url")});
    history::append(history, {t - 5400, true, "Alex’s iPhone", "japan-trip.png"});
    history::append(history, {t - 600, false, "Alex’s iPhone", file("IMG_4788.HEIC")});

    HowlWindow::Hooks hooks;
    hooks.settings = []() {
        WindowSettings s;
        s.radio = true;
        s.start_with_windows = true;
        s.receive = "ask";
        s.senders = "+1 555 123 4567,alex@example.com";
        return s;
    };
    hooks.nearby = []() {
        Node::Nearby n;
        n.relay = true;
        n.local = true;
        n.devices = {"Alex’s iPhone", "Sam’s iPad"};
        return n;
    };
    hooks.status = []() { return std::wstring(L"On the share sheet as Desk PC · asks first"); };
    hooks.level = []() { return RadioLevel::Up; };
    hooks.transfers = [moving]() { return std::vector<Transfers::Transfer>{moving}; };
    ok &= HowlWindow::render(dir / "window.png", false, scale, L"Desk PC", history, hooks);
    hooks.transfers = []() { return std::vector<Transfers::Transfer>{}; };
    ok &= HowlWindow::render(dir / "settings.png", true, scale, L"Desk PC", history, hooks);
    fs::remove_all(stage, ec);

    std::cout << (ok ? "wrote" : "could not write all of") << " card-ask.png, card-receiving.png, "
              << "card-received.png, card-sending.png, window.png, settings.png in " << path_to_utf8(dir) << '\n';
    if (com) CoUninitialize();
    return ok ? 0 : 1;
}

}  // namespace howldrop
