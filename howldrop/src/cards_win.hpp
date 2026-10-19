#pragma once

// The transfer cards: one small dark window per AirDrop coming in or going
// out, stacked above the tray (bottom right of the screen), after
// omdrop-windows' design. An incoming ask has Accept and Decline; a transfer
// on its way has a progress bar; a finished one says how it went, with Open
// and Show in folder, then goes by itself unless the pointer is on it. They
// replace the tray's notifications for transfers. Windows only.

#include "transfers.hpp"
#include "ui_win.hpp"

#include <windows.h>

#include <filesystem>
#include <functional>
#include <memory>
#include <set>
#include <string>
#include <vector>

namespace howldrop {

// What a transfer says, on a card or in the HowlDrop window's Now list.
struct CardText {
    const wchar_t* glyph;
    ui::Color tint;
    std::wstring title;  // Receive, Receiving, Received, Sending, Sent, ...
    std::wstring line;   // who, and what
};
CardText card_text(const Transfers::Transfer& t);
std::wstring card_status(const Transfers::Transfer& t);  // under the progress bar
double card_fraction(const Transfers::Transfer& t);      // the bar: 0..1, or -1 moving
// A card for t, as a PNG (`howldrop screenshots`).
bool render_card(const Transfers::Transfer& t, const std::wstring& inbox_label, const std::filesystem::path& file,
                 float scale);

class Cards {
public:
    struct Actions {
        std::function<void(const std::string& id, bool accept)> decide;  // an ask's answer
        std::function<void(const std::filesystem::path& file)> open;     // a file, or a link's .url
        std::function<void(const std::filesystem::path& file)> show;     // in its folder
        std::function<void(const std::string& id)> retired;              // the card is gone
    };

    Cards(HINSTANCE instance, std::wstring inbox_label, Actions actions);
    ~Cards();
    Cards(const Cards&) = delete;
    Cards& operator=(const Cards&) = delete;

    // False when Direct2D couldn't start: use notifications instead.
    bool available() const { return available_; }
    // The model changed. finished_too: show transfers that just ended (the
    // notifications setting); an ask is always shown.
    void update(const Transfers& model, bool finished_too);
    // While the HowlDrop window is open it shows the transfers: the cards
    // keep their state, out of sight.
    void set_hidden(bool hidden);

    struct Card;

private:
    friend LRESULT CALLBACK card_proc(HWND, UINT, WPARAM, LPARAM);
    void place();
    void retire(Card* card);
    void act(Card& card, int action);

    HINSTANCE instance_;
    std::wstring inbox_label_;
    Actions actions_;
    bool available_ = false;
    bool hidden_ = false;
    std::vector<std::unique_ptr<Card>> cards_;  // oldest first, lowest on screen
    std::set<std::string> dismissed_;           // closed by the user: not shown again
};

}  // namespace howldrop
