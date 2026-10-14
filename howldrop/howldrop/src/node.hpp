#pragma once

#include "clipboard.hpp"
#include "config.hpp"
#include "net.hpp"
#include "protocol.hpp"
#include "text.hpp"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <filesystem>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <utility>
#include <vector>

namespace howldrop {

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
    Status push_clipboard();
    std::vector<PeerView> peers() const;

private:
    using Clock = std::chrono::steady_clock;

    struct Peer {
        std::string id;
        std::string name;
        std::uint32_t addr_be = 0;
        std::uint16_t tcp_port = 0;
        std::uint16_t discovery_port = 0;
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
    void receive_file(Socket& socket, const Message& message);
    void receive_clip(Socket& socket, const Message& message);
    void write_ack(Socket& socket, bool ok, const std::string& message, const std::string& saved);
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
    std::vector<Job> jobs_;
    std::thread thread_;
    std::atomic<bool> stop_{false};
    std::atomic<bool> running_{false};
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
