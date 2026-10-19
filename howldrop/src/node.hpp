#pragma once

#include "clipboard.hpp"
#include "config.hpp"
#include "net.hpp"
#include "pairing.hpp"
#include "protocol.hpp"
#include "text.hpp"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <filesystem>
#include <functional>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace howldrop {

// The text a PC sends the relay to ask for a scan of nearby devices.
inline constexpr const char* kScanRequest = "howldrop:scan";

class Node {
public:
    struct Config {
        std::string name = "pc";
        std::string id;
        std::uint16_t discovery_port = kDiscoveryPort;
        std::uint16_t transfer_port = kTransferPort;
        std::uint16_t control_port = kControlPort;
        std::filesystem::path inbox;
        bool lan_broadcast = true;
        std::vector<std::pair<std::string, std::uint16_t>> beacon_targets;
        std::chrono::milliseconds beacon_interval{1000};
        std::chrono::milliseconds peer_ttl{5000};
        std::uint64_t max_file_bytes = kMaxFileBytes;
        bool enable_control = true;
        bool log = true;
        // The pairing key (pairing.hpp): with one, only a sender that knows
        // it can deliver here, and this node signs what it sends. Empty:
        // the unpaired protocol, both ways.
        std::string pair_key;
        // Send what is copied here to the other HowlDrop PCs. Off unless
        // asked for: on, every copy (a password too) goes to any PC that
        // answers on the network. `howldrop clip` sends once either way.
        bool share_clipboard = false;
        // The port beacons go to, when it is not discovery_port: the add-on's
        // iPhone relay listens on its own port, beside the bridge's, and
        // announces itself on the PCs' (kDiscoveryPort).
        std::uint16_t announce_port = 0;
        // Said in the hello. "iphone": the add-on's relay, which AirDrops what
        // it receives to the iPhone. The bridge never takes it for a PC, and
        // send_to_iphone finds it by this.
        std::string role;
        // After send_to_iphone: what was sent, and how it went.
        std::function<void(const std::string& file, const Status& status)> on_sent;
        // More hello fields, read at each beacon: the relay adds the devices
        // its latest scan heard ("near", tab-separated) and "scanning".
        std::function<std::vector<std::pair<std::string, std::string>>()> hello_extra;
        // A file came with a "to" (the device it is for), called before the
        // file takes its final name: the relay notes it for outbox.py.
        std::function<void(const std::filesystem::path& saved, const std::string& to)> on_target;
        // Called on a worker thread once a file is saved or text is on the
        // clipboard, with the sender's name (empty when unknown): the Windows
        // tray app notifies and opens from these. on_log, when set, takes the
        // log lines that otherwise go to stdout.
        std::function<void(const std::filesystem::path& saved, const std::string& from)> on_file;
        std::function<void(const std::string& text, const std::string& from)> on_text;
        std::function<void(const std::string& line)> on_log;
    };

    struct PeerView {
        std::string id;
        std::string name;
        std::string ip;
        std::uint16_t port = 0;
    };

    Node(Config config, Clipboard& clipboard);
    ~Node();
    Node(const Node&) = delete;
    Node& operator=(const Node&) = delete;

    void start();
    void request_stop();
    void stop();
    bool running() const { return running_.load(); }

    Status send_file(const std::string& peer, const std::filesystem::path& path);
    // To the add-on's iPhone relay (role "iphone"), which AirDrops it on: to
    // the device named to, or with to empty the add-on's default.
    Status send_to_iphone(const std::filesystem::path& path, const std::string& to = {});
    // What the relay's latest scan heard, from its hello.
    struct Nearby {
        bool relay = false;     // the relay is on the network, or this PC's radio runs
        bool local = false;     // it is this PC's radio (LocalIphone), not Home Assistant
        bool scanning = false;  // it is scanning now
        std::vector<std::string> devices;
    };
    Nearby iphone_nearby() const;
    // Ask the relay to look for nearby devices (about 30 s).
    Status request_iphone_scan();
    // This PC's own radio (wsl_win.hpp), while it runs: sending to the
    // iPhone, nearby devices and scans go to it instead of the add-on's
    // relay. queue only hands the file over; it reports through on_sent
    // once the phone took it or not.
    class LocalIphone {
    public:
        virtual ~LocalIphone() = default;
        virtual Status queue(const std::filesystem::path& path, const std::string& to) = 0;
        virtual Nearby nearby() const = 0;
        virtual Status scan() = 0;
    };
    void set_local_iphone(std::shared_ptr<LocalIphone> local);
    Status push_clipboard();
    std::vector<PeerView> peers() const;
    void set_share_clipboard(bool on) { share_clipboard_.store(on); }

private:
    using Clock = std::chrono::steady_clock;

    struct Peer {
        std::string id;
        std::string name;
        std::uint32_t addr_be = 0;
        std::uint16_t tcp_port = 0;
        std::uint16_t discovery_port = 0;
        std::string role;
        std::vector<std::string> heard;  // the relay's: devices its scan heard
        bool scanning = false;
        Clock::time_point seen{};
        std::uint64_t clip_sent_gen = 0;
        std::uint64_t clip_error_gen = 0;
    };

    struct Lookup {
        enum class Kind { Found, Missing, Ambiguous };
        Kind kind = Kind::Missing;
        Peer peer{};
    };

    struct Job {
        std::thread thread;
        std::shared_ptr<std::atomic<bool>> done;
    };

    void run();
    bool prepare(std::string& error);
    void mark_started();
    void fail_start(const std::string& error);
    void loop();
    void close_listeners();
    void send_beacons();
    void send_hello_to(std::uint32_t addr_be, std::uint16_t port_host);
    void handle_datagram(const std::uint8_t* data, std::size_t n, std::uint32_t from_be, std::uint16_t from_port);
    void handle_transfer(Socket socket);
    void handle_control(Socket socket);
    void receive_file(Socket& socket, const Message& message, std::optional<HmacSha256>& mac);
    void receive_clip(Socket& socket, const Message& message, std::optional<HmacSha256>& mac);
    bool trailer_matches(Socket& socket, std::optional<HmacSha256>& mac);
    void write_ack(Socket& socket, bool ok, const std::string& message, const std::string& saved);
    Status send_file_to(const Peer& peer, const std::filesystem::path& path, const std::string& to = {});
    std::optional<Peer> relay_peer() const;
    std::shared_ptr<LocalIphone> local_iphone() const;
    bool send_clip_to(const Peer& peer, const std::string& text, std::string& error);
    Status flush_clips();
    void poll_clipboard();
    void expire_peers();
    void maybe_note_waiting();
    Lookup lookup(const std::string& query) const;
    std::filesystem::path inbox_file(const std::string& safe_name) const;
    Message hello_message() const;
    void reap();
    void join_jobs();
    void log_line(const std::string& line);

    template <class Fn>
    void spawn(Fn&& fn) {
        auto done = std::make_shared<std::atomic<bool>>(false);
        std::thread thread([fn = std::forward<Fn>(fn), done]() mutable {
            try {
                fn();
            } catch (...) {
            }
            done->store(true);
        });
        std::lock_guard<std::mutex> lock(jobs_mu_);
        jobs_.push_back(Job{std::move(thread), std::move(done)});
    }

    Config config_;
    Clipboard& clipboard_;
    Socket udp_;
    Socket tcp_;
    Socket control_;
    mutable std::mutex mu_;
    std::mutex clip_send_mu_;
    std::mutex log_mu_;
    std::mutex jobs_mu_;
    std::mutex start_mu_;
    std::mutex join_mu_;
    std::condition_variable start_cv_;
    std::vector<Peer> peers_;
    std::shared_ptr<LocalIphone> local_iphone_;  // under mu_
    std::vector<Job> jobs_;
    std::thread thread_;
    std::atomic<bool> stop_{false};
    std::atomic<bool> running_{false};
    std::atomic<bool> share_clipboard_{false};
    bool started_ = false;
    std::string start_error_;
    bool baseline_done_ = false;
    bool have_clip_ = false;
    bool waiting_logged_ = false;
    std::uint32_t last_seq_ = 0;
    std::uint64_t clip_gen_ = 0;
    std::uint64_t last_flush_gen_ = 0;
    std::string clip_text_;
    Clock::time_point started_at_{};
    Clock::time_point next_beacon_{};
    Clock::time_point last_flush_{};
};

}  // namespace howldrop
