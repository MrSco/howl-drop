#pragma once

// What the transfer cards show (cards_win.hpp): each AirDrop coming in or
// going out, from the radio's events in WSL (howl_ask.py, outbox.py,
// deliver.py; wsl_text.hpp parses them) and from the Home Assistant route's
// callbacks (Node::Config on_file, on_text, on_sent). Kept apart from the
// drawing so the tests run anywhere.

#include <chrono>
#include <cstdint>
#include <filesystem>
#include <string>
#include <vector>

namespace howldrop {

class Transfers {
public:
    using Clock = std::chrono::steady_clock;

    enum class Direction { In, Out };
    enum class Stage {
        Asking,    // in: waiting for Accept or Decline on the PC
        Waiting,   // in: accepted, nothing yet; out: looking for the phone
        Accepting, // out: the phone shows Accept
        Moving,    // bytes on their way (in: counted; out: not)
        Saving,    // in: stored, being moved into the inbox
        Done,
        Declined,  // in: declined or unanswered; out: declined on the phone
        Failed,
    };

    struct Transfer {
        std::string id;
        Direction direction = Direction::In;
        std::string peer;                  // the phone's (or PC's) name; empty when unknown
        std::vector<std::string> names;    // file names, or a link's address
        std::size_t count = 0;             // how many items (names may be capped)
        std::uint64_t total = 0;           // bytes, 0 when unknown
        std::uint64_t bytes = 0;
        double rate = 0;                   // bytes per second, smoothed
        Stage stage = Stage::Waiting;
        std::string detail;                // a reason, or the sender's stage in words
        std::vector<std::filesystem::path> saved;  // in: where the files landed
        std::string kind;                  // in, done: image, video, text, link, file
        bool home_assistant = false;       // arrived through the add-on
        Clock::time_point started{};
        Clock::time_point updated{};
        Clock::time_point deadline{};      // asking: when no answer means Decline

        bool finished() const { return stage == Stage::Done || stage == Stage::Declined || stage == Stage::Failed; }
        // 0..1, or -1 when the size isn't known (an indeterminate bar)
        double fraction() const;
    };

    // How long an ask waits for the user (the receiver's own limit, howl_ask.py).
    static constexpr std::chrono::seconds kAskTimeout{60};
    // A transfer with no news for this long has stopped.
    static constexpr std::chrono::seconds kStall{90};

    // A radio event (the fields after @howldrop). Returns the id of the
    // transfer it touched, or empty when it was about none.
    std::string apply(const std::vector<std::string>& event, Clock::time_point now);
    // A file delivered into the inbox (`received` from WSL, or the add-on's
    // forwarder): kind as deliver.py names it, or empty to guess.
    std::string received(const std::filesystem::path& saved, const std::string& from, const std::string& kind,
                         bool home_assistant, Clock::time_point now);
    // This PC starts sending a file (either route), and how that ended.
    std::string sending(const std::string& name, const std::string& to, bool home_assistant, Clock::time_point now);
    std::string sent(const std::string& name, bool ok, const std::string& detail, Clock::time_point now);
    // Mark transfers that went quiet as failed; returns the ids changed.
    std::vector<std::string> expire(Clock::time_point now);

    const Transfer* find(const std::string& id) const;
    const std::vector<Transfer>& all() const { return list_; }
    void forget(const std::string& id);

    static std::string kind_of(const std::filesystem::path& path);

private:
    Transfer* get(const std::string& id);
    Transfer& add(const std::string& id, Direction direction, Clock::time_point now);
    // outbox.py's stage for the file it is sending, in words.
    std::string apply_stage(const std::string& name, const std::string& detail, Clock::time_point now);

    std::vector<Transfer> list_;
    unsigned serial_ = 0;
};

}  // namespace howldrop
