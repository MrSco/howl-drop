#pragma once

// AirDrop on this PC: howldrop-agent.exe (crates/howldrop-agent) driving a
// USB MT7612U stick itself, from user space on WinUSB, with a TCP/IP stack
// of its own as the AWDL link (docs/native-windows.md). Windows only.
//
// One program for the user: the first time it is turned on, howldrop.exe
// downloads the agent from its own GitHub release (its SHA-256 is built into
// the exe; a local build uses the agent beside it), and runs it as the user,
// without a window or an administrator. A stick not yet on WinUSB is the one
// exception: the agent says so, and its setup puts it on, with Windows
// asking once. The agent prints to a named pipe of ours and stops when we
// disconnect it. Nothing listens on the network between the two: the agent
// saves straight into the tray's inbox, says what arrived in event lines
// (radio_text.hpp), and AirDrops what the tray drops into an outbox folder.
// Turning it off gives the stick back to Windows.

#include "node.hpp"
#include "radio_text.hpp"

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

struct ChildProcess;  // radio_win.cpp

// How it is going, for the tray icon's dot: none, amber, green, red.
enum class RadioLevel { Off, Working, Up, Problem };

struct RadioConfig {
    std::filesystem::path inbox;  // where files land (the tray's, Downloads\HowlDrop)
    std::filesystem::path home;   // %LOCALAPPDATA%\HowlDrop: the agent, the outbox, radio.log
    std::string name;             // what the iPhone's share sheet shows
    // Who may send: "everyone", "ask" (contacts at once, anyone else after
    // Accept on the PC) or "contacts"; senders: comma-separated phone
    // numbers and emails (the agent's --receive, --senders).
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
};

class Radio final : public Node::LocalIphone {
public:
    explicit Radio(RadioConfig config);
    ~Radio() override;
    Radio(const Radio&) = delete;
    Radio& operator=(const Radio&) = delete;

    // Sets up what is missing and runs the radio, on a thread of its own.
    void start();
    // Stops the agent, which gives the stick back; blocks until done (up to
    // about 20 s).
    void stop();

    // The user's answer to an ask (the agent waits for it).
    void decide(const std::string& id, bool accept);

    Status queue(const std::filesystem::path& path, const std::string& to) override;
    Node::Nearby nearby() const override;
    Status scan() override;

private:
    void run();
    // The agent, beside us or from the release, and the one-time setup
    // that runs it as administrator without asking.
    bool prepare(std::string& error);
    bool fetch_agent(std::string& error);
    // A release file to dest, checked against sha256; what names it for the user.
    bool download(const std::string& address, const std::string& sha256, const std::string& what,
                  const std::filesystem::path& dest, std::string& error);
    // Runs the agent until it ends; why it ended, or nothing when stopped.
    std::string run_agent();
    void handle_line(const std::string& line);
    void state(const std::string& text, RadioLevel level = RadioLevel::Working);
    void log(const std::string& line);
    bool stopping() const { return stop_.load(); }
    void pause(int ms);
    // The Bluetooth wake (ble_wake_win.hpp), on its own thread: on while a
    // send or a scan wants it, off a little after the last one is done.
    void wake_loop();
    void hold_wake(int seconds);
    void done_waking();

    RadioConfig config_;
    std::filesystem::path outbox_;
    std::filesystem::path scan_request_;
    std::filesystem::path asks_;   // the answers to asks, one file each
    std::filesystem::path agent_;  // howldrop-agent.exe
    std::thread thread_;
    std::atomic<bool> stop_{false};
    std::atomic<bool> up_{false};
    std::shared_ptr<ChildProcess> agent_process_;
    std::mutex process_mu_;
    mutable std::mutex nearby_mu_;
    bool scanning_ = false;
    std::vector<std::string> devices_;
    std::mutex log_mu_;
    std::string stop_reason_;   // why the agent ended, from its events
    bool needs_setup_ = false;  // the agent said the stick isn't on WinUSB
    std::thread wake_thread_;
    std::mutex wake_mu_;
    std::condition_variable wake_cv_;
    std::chrono::steady_clock::time_point wake_until_{};
    int sends_pending_ = 0;  // under wake_mu_
    bool wake_quit_ = false;
};

}  // namespace howldrop
