#pragma once

// AirDrop on this PC: the add-on's radio stack (OWL, the AirDrop receiver
// and sender) in a WSL 2 distro of its own, driving a USB MT7612U stick
// that usbipd-win hands to WSL. Windows only.
//
// One program for the user: the first time it is turned on, howldrop.exe
// downloads the Linux part from its own GitHub release (its SHA-256 is built
// into the exe), imports it as the distro HowlDrop, and from then on starts
// it as a hidden child, wsl-run.sh, and reads its output. Nothing listens on
// the network between the two: the stack saves straight into the tray's
// inbox (the same folder, as /mnt/c/...), says what arrived in event lines
// on its output (wsl_text.hpp), and AirDrops what the tray drops into an
// outbox folder. Turning it off gives the stick back to Windows.

#include "node.hpp"
#include "wsl_text.hpp"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace howldrop {

struct ChildProcess;  // wsl_win.cpp

// How it is going, for the tray icon's dot: none, amber, green, red.
enum class RadioLevel { Off, Working, Up, Problem };

struct WslRadioConfig {
    std::filesystem::path inbox;  // where files land (the tray's, Downloads\HowlDrop)
    std::filesystem::path home;   // %LOCALAPPDATA%\HowlDrop: the distro, the outbox, radio.log
    std::string name;             // what the iPhone's share sheet shows
    // Who may send: "everyone", "ask" (contacts at once, anyone else after
    // Accept on the PC) or "contacts"; senders: comma-separated phone
    // numbers and emails (wsl-run.sh --receive, --senders).
    std::string receive = "ask";
    std::string senders;
    // All called on the radio's own thread.
    std::function<void(const std::string& state, RadioLevel level)> on_state;  // a line for the menu
    std::function<void(bool up)> on_up;                      // on the share sheet, or not any more
    std::function<void(const std::filesystem::path& saved, const std::string& from, const std::string& kind)>
        on_received;
    std::function<void(const std::string& file, const Status& status)> on_sent;
    // A transfer's news for its card (transfers.hpp): ask, asked, incoming,
    // receiving, stored, failed, sending; the fields after @howldrop.
    std::function<void(const std::vector<std::string>& event)> on_event;
    // A yes/no question for the user (installing usbipd-win).
    std::function<bool(const std::wstring& question)> ask;
};

class WslRadio final : public Node::LocalIphone {
public:
    explicit WslRadio(WslRadioConfig config);
    ~WslRadio() override;
    WslRadio(const WslRadio&) = delete;
    WslRadio& operator=(const WslRadio&) = delete;

    // Sets up what is missing and runs the radio, on a thread of its own.
    void start();
    // Stops the stack, detaches the stick and stops the distro; blocks
    // until done (up to about a minute).
    void stop();

    // The user's answer to an ask (howl_ask.py waits for it).
    void decide(const std::string& id, bool accept);

    Status queue(const std::filesystem::path& path, const std::string& to) override;
    Node::Nearby nearby() const override;
    Status scan() override;

private:
    void run();
    bool prepare(std::string& error);
    bool ensure_distro(std::string& error);
    bool download_linux(const std::filesystem::path& tar, std::string& error);
    bool find_stick(wsl::UsbDevice& stick, std::string& error);
    void run_stack(const wsl::UsbDevice& stick);
    void handle_line(const std::string& line, const std::string& busid);
    void attach(const std::string& busid);
    void give_back(const std::string& busid);
    void state(const std::string& text, RadioLevel level = RadioLevel::Working);
    void log(const std::string& line);
    bool stopping() const { return stop_.load(); }
    void pause(int ms);
    // The Bluetooth wake (ble_wake_win.hpp), on its own thread: on while a
    // send or a scan wants it, off a little after the last one is done.
    void wake_loop();
    void hold_wake(int seconds);
    void done_waking();

    WslRadioConfig config_;
    std::filesystem::path outbox_;
    std::filesystem::path scan_request_;
    std::filesystem::path asks_;  // the answers to asks, one file each
    std::filesystem::path usbipd_;
    std::thread thread_;
    std::atomic<bool> stop_{false};
    std::atomic<bool> up_{false};
    std::shared_ptr<ChildProcess> stack_;
    std::mutex process_mu_;
    mutable std::mutex nearby_mu_;
    bool scanning_ = false;
    std::vector<std::string> devices_;
    std::mutex log_mu_;
    std::string last_line_;
    std::string stick_note_;  // ", RTL8852BU, untested" for an untested stick
    std::thread wake_thread_;
    std::mutex wake_mu_;
    std::condition_variable wake_cv_;
    std::chrono::steady_clock::time_point wake_until_{};
    int sends_pending_ = 0;  // under wake_mu_
    bool wake_quit_ = false;
};

}  // namespace howldrop
