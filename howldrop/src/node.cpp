#include "node.hpp"

#include <array>
#include <fstream>
#include <iostream>
#include <random>
#include <set>
#include <system_error>

namespace howldrop {
namespace {

std::string random_id() {
    std::array<unsigned char, 16> bytes{};
    try {
        std::random_device device;
        for (auto& byte : bytes) byte = static_cast<unsigned char>(device());
    } catch (...) {
        const auto seed = static_cast<std::uint32_t>(std::chrono::steady_clock::now().time_since_epoch().count());
        std::mt19937 gen(seed);
        for (auto& byte : bytes) byte = static_cast<unsigned char>(gen());
    }
    static constexpr char hex[] = "0123456789abcdef";
    std::string id(32, '0');
    for (std::size_t i = 0; i < bytes.size(); ++i) {
        id[i * 2] = hex[bytes[i] >> 4];
        id[i * 2 + 1] = hex[bytes[i] & 0x0f];
    }
    return id;
}

std::string clean_name(std::string name) {
    std::string out;
    out.reserve(name.size());
    for (unsigned char c : name) {
        if (c >= 0x20 && c != 0x7f) out.push_back(static_cast<char>(c));
    }
    if (out.size() > 64) out.resize(64);
    if (out.empty()) out = "pc";
    return out;
}

}  // namespace

Node::Node(Config config, Clipboard& clipboard) : config_(std::move(config)), clipboard_(clipboard) {
    config_.name = clean_name(config_.name);
    if (!is_node_id(config_.id)) config_.id = random_id();
    share_clipboard_.store(config_.share_clipboard);
}

Node::~Node() { stop(); }

void Node::start() {
    stop();
    stop_.store(false);
    {
        std::lock_guard<std::mutex> lock(start_mu_);
        started_ = false;
        start_error_.clear();
    }
    {
        std::lock_guard<std::mutex> lock(join_mu_);
        thread_ = std::thread([this] { run(); });
    }
    std::unique_lock<std::mutex> lock(start_mu_);
    start_cv_.wait(lock, [&] { return started_; });
    if (!start_error_.empty()) {
        const std::string error = start_error_;
        lock.unlock();
        stop();
        throw std::runtime_error(error);
    }
}

void Node::request_stop() { stop_.store(true); }

void Node::stop() {
    request_stop();
    std::lock_guard<std::mutex> lock(join_mu_);
    if (thread_.joinable()) thread_.join();
}

void Node::run() {
    bool announced = false;
    try {
        std::string error;
        if (!prepare(error)) {
            close_listeners();
            fail_start(error);
            running_.store(false);
            return;
        }
        mark_started();
        announced = true;
        loop();
    } catch (const std::exception& ex) {
        if (!announced) fail_start(ex.what());
        else log_line(std::string("stopped: ") + ex.what());
    } catch (...) {
        if (!announced) fail_start("unknown error");
    }
    close_listeners();
    join_jobs();
    running_.store(false);
}

bool Node::prepare(std::string& error) {
    if (!startup_network()) {
        error = "could not start networking";
        return false;
    }
    if (config_.inbox.empty()) {
        error = "no folder is set for received files";
        return false;
    }
    std::filesystem::create_directories(config_.inbox);
    udp_ = Socket::udp_v4();
    if (!udp_.valid() || !udp_.set_reuseaddr() || !udp_.set_broadcast() || !udp_.bind_v4(0, config_.discovery_port) ||
        !udp_.set_nonblocking(true)) {
        error = "UDP port " + std::to_string(config_.discovery_port) + " is not available (" + last_socket_error() +
                "). If HowlDrop is already running, use that window.";
        return false;
    }
    tcp_ = Socket::tcp_v4();
    if (!tcp_.valid() || !tcp_.set_reuseaddr() || !tcp_.bind_v4(0, config_.transfer_port) || !tcp_.listen(16) ||
        !tcp_.set_nonblocking(true)) {
        error = "TCP port " + std::to_string(config_.transfer_port) + " is not available (" + last_socket_error() +
                "). If HowlDrop is already running, use that window.";
        return false;
    }
    if (config_.enable_control) {
        control_ = Socket::tcp_v4();
        std::uint32_t loopback = 0;
        ipv4_from_string("127.0.0.1", loopback);
        if (!control_.valid() || !control_.set_reuseaddr() || !control_.bind_v4(loopback, config_.control_port) ||
            !control_.listen(8) || !control_.set_nonblocking(true)) {
            error = "this PC's HowlDrop control port " + std::to_string(config_.control_port) +
                    " is not available (" + last_socket_error() + ")";
            return false;
        }
    }
    return true;
}

void Node::mark_started() {
    std::lock_guard<std::mutex> lock(start_mu_);
    running_.store(true);
    started_ = true;
    start_cv_.notify_all();
}

void Node::fail_start(const std::string& error) {
    std::lock_guard<std::mutex> lock(start_mu_);
    start_error_ = error.empty() ? "HowlDrop could not start" : error;
    started_ = true;
    start_cv_.notify_all();
}

void Node::close_listeners() {
    udp_.close();
    tcp_.close();
    control_.close();
}

void Node::loop() {
    started_at_ = Clock::now();
    next_beacon_ = started_at_;
    log_line("HowlDrop " + std::string(kVersion) + "  " + config_.name);
    log_line("id " + config_.id);
    log_line("inbox " + path_to_utf8(config_.inbox));
    log_line("listening for peers");
    while (!stop_.load()) {
        const auto now = Clock::now();
        if (now >= next_beacon_) {
            send_beacons();
            next_beacon_ = Clock::now() + config_.beacon_interval;
        }
        int wait_ms = 200;
        const auto until = std::chrono::duration_cast<std::chrono::milliseconds>(next_beacon_ - Clock::now()).count();
        if (until < wait_ms) wait_ms = until < 0 ? 0 : static_cast<int>(until);
        const Ready ready = wait_readable(udp_.valid() ? &udp_ : nullptr, tcp_.valid() ? &tcp_ : nullptr,
                                          control_.valid() ? &control_ : nullptr, wait_ms);
        if (ready.udp) {
            for (;;) {
                std::uint8_t buf[2048];
                std::uint32_t from_be = 0;
                std::uint16_t from_port = 0;
                const int n = udp_.recv_from(buf, sizeof(buf), from_be, from_port);
                if (n <= 0) break;
                handle_datagram(buf, static_cast<std::size_t>(n), from_be, from_port);
            }
        }
        if (ready.tcp) {
            for (;;) {
                Socket client = tcp_.accept_v4();
                if (!client.valid()) break;
                spawn([this, client = std::move(client)]() mutable { handle_transfer(std::move(client)); });
            }
        }
        if (ready.control) {
            for (;;) {
                Socket client = control_.accept_v4();
                if (!client.valid()) break;
                spawn([this, client = std::move(client)]() mutable { handle_control(std::move(client)); });
            }
        }
        expire_peers();
        if (share_clipboard_.load()) {
            poll_clipboard();
        } else {
            // Keep up with the clipboard without reading it, so turning
            // sharing on sends the next copy, not whatever was there.
            const auto seq = clipboard_.sequence();
            std::lock_guard<std::mutex> lock(mu_);
            last_seq_ = seq;
            baseline_done_ = true;
        }
        bool have = false;
        std::uint64_t gen = 0;
        {
            std::lock_guard<std::mutex> lock(mu_);
            have = have_clip_;
            gen = clip_gen_;
        }
        if (have && share_clipboard_.load() &&
            (gen != last_flush_gen_ || Clock::now() - last_flush_ >= std::chrono::seconds(1))) {
            flush_clips();
            last_flush_gen_ = gen;
            last_flush_ = Clock::now();
        }
        maybe_note_waiting();
        reap();
    }
}

Message Node::hello_message() const {
    Message message;
    message.type = MsgType::Hello;
    message.fields.set("name", config_.name);
    message.fields.set("id", config_.id);
    message.fields.set("port", std::to_string(config_.transfer_port));
    if (!config_.role.empty()) message.fields.set("role", config_.role);
    if (config_.hello_extra) {
        for (auto& [key, value] : config_.hello_extra()) message.fields.set(std::move(key), std::move(value));
    }
    return message;
}

void Node::send_hello_to(std::uint32_t addr_be, std::uint16_t port_host) {
    if (!udp_.valid() || addr_be == 0 || port_host == 0) return;
    std::string error;
    auto bytes = encode_head(hello_message(), error);
    if (!bytes) return;
    udp_.send_to(bytes->data(), bytes->size(), addr_be, port_host);
}

void Node::send_beacons() {
    std::string error;
    auto bytes = encode_head(hello_message(), error);
    if (!bytes) return;
    std::set<std::uint64_t> seen;
    auto send_one = [&](std::uint32_t addr, std::uint16_t port) {
        if (addr == 0 || port == 0) return;
        const std::uint64_t key = (static_cast<std::uint64_t>(addr) << 16) | port;
        if (!seen.insert(key).second) return;
        udp_.send_to(bytes->data(), bytes->size(), addr, port);
    };
    if (config_.lan_broadcast) {
        const std::uint16_t port = config_.announce_port ? config_.announce_port : config_.discovery_port;
        send_one(0xffffffffu, port);
        for (std::uint32_t addr : ipv4_broadcasts()) send_one(addr, port);
    }
    for (const auto& target : config_.beacon_targets) {
        std::uint32_t addr = 0;
        if (ipv4_from_string(target.first, addr)) send_one(addr, target.second);
    }
    std::vector<std::pair<std::uint32_t, std::uint16_t>> peers;
    {
        std::lock_guard<std::mutex> lock(mu_);
        for (const auto& peer : peers_) peers.emplace_back(peer.addr_be, peer.discovery_port);
    }
    for (const auto& peer : peers) send_one(peer.first, peer.second);
}

void Node::handle_datagram(const std::uint8_t* data, std::size_t n, std::uint32_t from_be, std::uint16_t from_port) {
    std::string error;
    auto message = parse_datagram(data, n, error);
    if (!message || message->type != MsgType::Hello) return;
    const std::string id = message->fields.get("id");
    const std::string name = message->fields.get("name");
    const std::string port_text = message->fields.get("port");
    const std::string role = message->fields.get("role");
    std::vector<std::string> heard;
    {
        const std::string text = message->fields.get("near");
        std::size_t start = 0;
        while (start < text.size()) {
            const auto tab = text.find('\t', start);
            const std::string item = text.substr(start, tab == std::string::npos ? std::string::npos : tab - start);
            if (!item.empty() && valid_display_name(item)) heard.push_back(item);
            if (tab == std::string::npos) break;
            start = tab + 1;
        }
    }
    const bool scanning = message->fields.get("scanning") == "1";
    if (!is_node_id(id) || !valid_display_name(name) || id == config_.id) return;
    int port = 0;
    try {
        port = std::stoi(port_text);
    } catch (...) {
        return;
    }
    if (port <= 0 || port > 65535 || from_be == 0 || from_port == 0) return;

    std::string note;
    bool reply = false;
    {
        std::lock_guard<std::mutex> lock(mu_);
        Peer* peer = nullptr;
        for (auto& item : peers_) {
            if (item.id == id) peer = &item;
        }
        const auto now = Clock::now();
        if (!peer) {
            Peer created;
            created.id = id;
            created.name = name;
            created.addr_be = from_be;
            created.tcp_port = static_cast<std::uint16_t>(port);
            created.discovery_port = from_port;
            created.role = role;
            created.heard = heard;
            created.scanning = scanning;
            created.seen = now;
            peers_.push_back(std::move(created));
            note = "peer " + name + " " + ipv4_to_string(from_be);
            reply = true;
        } else {
            const bool changed = peer->addr_be != from_be || peer->tcp_port != static_cast<std::uint16_t>(port) ||
                                 peer->discovery_port != from_port || peer->name != name;
            peer->seen = now;
            peer->addr_be = from_be;
            peer->tcp_port = static_cast<std::uint16_t>(port);
            peer->discovery_port = from_port;
            peer->name = name;
            peer->role = role;
            peer->heard = heard;
            peer->scanning = scanning;
            if (changed) {
                note = "peer " + name + " " + ipv4_to_string(from_be);
                reply = true;
            }
        }
    }
    if (!note.empty()) log_line(note);
    if (reply) send_hello_to(from_be, from_port);
}

void Node::expire_peers() {
    std::vector<std::string> notes;
    const auto now = Clock::now();
    {
        std::lock_guard<std::mutex> lock(mu_);
        std::vector<Peer> keep;
        keep.reserve(peers_.size());
        for (auto& peer : peers_) {
            if (now - peer.seen > config_.peer_ttl) notes.push_back("peer " + peer.name + " left");
            else keep.push_back(std::move(peer));
        }
        peers_.swap(keep);
    }
    for (const auto& note : notes) log_line(note);
}

void Node::maybe_note_waiting() {
    if (waiting_logged_) return;
    if (Clock::now() - started_at_ < std::chrono::seconds(3)) return;
    bool any = false;
    {
        std::lock_guard<std::mutex> lock(mu_);
        any = !peers_.empty();
    }
    waiting_logged_ = true;
    if (!any) log_line("no other HowlDrop on this network yet");
}

void Node::handle_transfer(Socket socket) {
    socket.set_recv_timeout_ms(1000);
    socket.set_send_timeout_ms(5000);
    socket.set_nodelay();
    Message message;
    std::string error;
    if (!read_head(socket, message, &stop_, error)) return;
    if (message.type != MsgType::File && message.type != MsgType::Clip) {
        write_ack(socket, false, "unsupported message", "");
        return;
    }
    // Pairing (pairing.hpp): refused before any body is read, and a paired
    // sender gets our proof of the key before it sends the body.
    std::string why;
    const HeadCheck check = check_head(message, config_.pair_key, why);
    if (check == HeadCheck::Refused) {
        write_ack(socket, false, why, "");
        log_line("refused a " + std::string(message.type == MsgType::File ? "file" : "text") + ": " + why);
        return;
    }
    std::optional<HmacSha256> mac;
    if (check == HeadCheck::Paired) {
        std::string ours;
        if (!send_ready(socket, config_.pair_key, message.fields.get("nonce"), ours, &stop_)) return;
        mac.emplace(body_mac(config_.pair_key, message.fields.get("nonce"), ours));
    }
    if (message.type == MsgType::File) receive_file(socket, message, mac);
    else receive_clip(socket, message, mac);
}

bool Node::trailer_matches(Socket& socket, std::optional<HmacSha256>& mac) {
    if (!mac) return true;
    Digest got{};
    if (!socket.recv_full(got.data(), got.size(), &stop_)) return false;
    const Digest want = mac->finish();
    return same_bytes(got.data(), want.data(), want.size());
}

void Node::write_ack(Socket& socket, bool ok, const std::string& message, const std::string& saved) {
    Message ack;
    ack.type = MsgType::Ack;
    ack.fields.set("result", ok ? "ok" : "error");
    const std::string text = one_line(message);
    if (!text.empty()) ack.fields.set("message", text);
    if (!saved.empty()) ack.fields.set("file", one_line(saved));
    std::string error;
    write_head(socket, ack, &stop_, error);
}

void Node::receive_file(Socket& socket, const Message& message, std::optional<HmacSha256>& mac) {
    try {
        if (message.body_size > config_.max_file_bytes) {
            write_ack(socket, false, "file is too large", "");
            return;
        }
        const std::string safe = sanitize_filename(message.fields.get("file"));
        std::filesystem::create_directories(config_.inbox);
        const auto final_path = inbox_file(safe);
        if (final_path.empty()) {
            write_ack(socket, false, "could not choose a file name", "");
            return;
        }
        auto partial = final_path;
        partial += ".partial";
        {
            std::ofstream out(partial, std::ios::binary | std::ios::trunc);
            if (!out) {
                write_ack(socket, false, "could not write the file", "");
                return;
            }
            std::uint64_t left = message.body_size;
            std::vector<std::uint8_t> buf(64 * 1024);
            while (left > 0) {
                const std::size_t chunk = static_cast<std::size_t>(std::min<std::uint64_t>(left, buf.size()));
                if (!socket.recv_full(buf.data(), chunk, &stop_)) {
                    out.close();
                    std::error_code ec;
                    std::filesystem::remove(partial, ec);
                    if (!stop_.load()) {
                        log_line("receiving " + safe + " stopped after " +
                                 std::to_string(message.body_size - left) + " of " +
                                 std::to_string(message.body_size) +
                                 " bytes: the sender closed the connection, or nothing came for 30 s");
                    }
                    return;
                }
                if (mac) mac->update(buf.data(), chunk);
                out.write(reinterpret_cast<const char*>(buf.data()), static_cast<std::streamsize>(chunk));
                if (!out) {
                    out.close();
                    std::error_code ec;
                    std::filesystem::remove(partial, ec);
                    write_ack(socket, false, "could not write the file", "");
                    return;
                }
                left -= chunk;
            }
            out.close();
            if (!out) {
                std::error_code ec;
                std::filesystem::remove(partial, ec);
                write_ack(socket, false, "could not write the file", "");
                return;
            }
        }
        if (!trailer_matches(socket, mac)) {
            std::error_code ec;
            std::filesystem::remove(partial, ec);
            write_ack(socket, false, "the file does not match its pairing signature", "");
            log_line("dropped " + safe + ": it does not match its pairing signature");
            return;
        }
        if (config_.on_target) {
            const std::string to = one_line(message.fields.get("to"));
            if (!to.empty()) config_.on_target(final_path, to.substr(0, 64));
        }
        std::filesystem::rename(partial, final_path);
        const std::string saved = filename_utf8(final_path);
        write_ack(socket, true, "", saved);
        std::string from = message.fields.get("from");
        {
            std::lock_guard<std::mutex> lock(mu_);
            for (const auto& peer : peers_) {
                if (peer.id == from) from = peer.name;
            }
        }
        std::string note = "saved " + path_to_utf8(final_path) + " (" + std::to_string(message.body_size) + " bytes)";
        if (!from.empty()) note += " from " + from;
        log_line(note);
        if (config_.on_file) config_.on_file(final_path, from == message.fields.get("from") ? std::string() : from);
    } catch (const std::exception& ex) {
        write_ack(socket, false, ex.what(), "");
    }
}

void Node::receive_clip(Socket& socket, const Message& message, std::optional<HmacSha256>& mac) {
    if (message.body_size > kMaxClipBytes) {
        write_ack(socket, false, "clipboard text is too large", "");
        return;
    }
    std::string text(static_cast<std::size_t>(message.body_size), '\0');
    if (message.body_size &&
        !socket.recv_full(reinterpret_cast<std::uint8_t*>(&text[0]), text.size(), &stop_)) {
        return;
    }
    if (mac) mac->update(text.data(), text.size());
    if (!trailer_matches(socket, mac)) {
        write_ack(socket, false, "the text does not match its pairing signature", "");
        log_line("dropped a text: it does not match its pairing signature");
        return;
    }
    if (!clipboard_.set_text(text)) {
        write_ack(socket, false, "could not set the clipboard", "");
        return;
    }
    const auto seq = clipboard_.sequence();
    const std::string from = message.fields.get("from");
    std::string name = from;
    {
        std::lock_guard<std::mutex> lock(mu_);
        last_seq_ = seq;
        baseline_done_ = true;
        if (!(have_clip_ && clip_text_ == text)) {
            clip_text_ = text;
            have_clip_ = true;
            ++clip_gen_;
            for (auto& peer : peers_) {
                if (peer.id == from) peer.clip_sent_gen = clip_gen_;
            }
        }
        for (const auto& peer : peers_) {
            if (peer.id == from) name = peer.name;
        }
    }
    write_ack(socket, true, "", "");
    log_line("clipboard from " + (name.empty() ? std::string("a peer") : name));
    if (config_.on_text) config_.on_text(text, name == from ? std::string() : name);
}

std::filesystem::path Node::inbox_file(const std::string& safe_name) const {
    const auto candidate = config_.inbox / path_from_utf8(safe_name);
    if (!std::filesystem::exists(candidate)) return candidate;
    std::string stem = safe_name;
    std::string ext;
    const auto dot = safe_name.find_last_of('.');
    if (dot != std::string::npos && dot != 0) {
        stem = safe_name.substr(0, dot);
        ext = safe_name.substr(dot);
    }
    for (int i = 1; i < 10000; ++i) {
        const auto next = config_.inbox / path_from_utf8(stem + " (" + std::to_string(i) + ")" + ext);
        if (!std::filesystem::exists(next)) return next;
    }
    return {};
}

Node::Lookup Node::lookup(const std::string& query) const {
    std::lock_guard<std::mutex> lock(mu_);
    for (const auto& peer : peers_) {
        if (peer.id == query) return Lookup{Lookup::Kind::Found, peer};
    }
    std::vector<Peer> by_name;
    for (const auto& peer : peers_) {
        if (names_equal(peer.name, query)) by_name.push_back(peer);
    }
    if (by_name.size() == 1) return Lookup{Lookup::Kind::Found, by_name[0]};
    if (by_name.size() > 1) return Lookup{Lookup::Kind::Ambiguous, {}};
    if (query.size() >= 4) {
        std::vector<Peer> by_id;
        for (const auto& peer : peers_) {
            if (peer.id.compare(0, query.size(), query) == 0) by_id.push_back(peer);
        }
        if (by_id.size() == 1) return Lookup{Lookup::Kind::Found, by_id[0]};
        if (by_id.size() > 1) return Lookup{Lookup::Kind::Ambiguous, {}};
    }
    return {};
}

Status Node::send_file(const std::string& peer_query, const std::filesystem::path& path) {
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) return Status::failure("that path is not a file");
    const Lookup found = lookup(peer_query);
    if (found.kind == Lookup::Kind::Ambiguous) {
        return Status::failure("more than one PC matches \"" + peer_query + "\". Use the id from howldrop peers");
    }
    if (found.kind != Lookup::Kind::Found) {
        return Status::failure("no PC named \"" + peer_query + "\" is visible");
    }
    return send_file_to(found.peer, path);
}

std::optional<Node::Peer> Node::relay_peer() const {
    std::lock_guard<std::mutex> lock(mu_);
    for (const auto& peer : peers_) {
        if (peer.role == "iphone") return peer;
    }
    return std::nullopt;
}

void Node::set_local_iphone(std::shared_ptr<LocalIphone> local) {
    std::lock_guard<std::mutex> lock(mu_);
    local_iphone_ = std::move(local);
}

std::shared_ptr<Node::LocalIphone> Node::local_iphone() const {
    std::lock_guard<std::mutex> lock(mu_);
    return local_iphone_;
}

Node::Nearby Node::iphone_nearby() const {
    if (const auto local = local_iphone()) {
        Nearby out = local->nearby();
        out.relay = true;
        out.local = true;
        return out;
    }
    Nearby out;
    const auto relay = relay_peer();
    if (!relay) return out;
    out.relay = true;
    out.scanning = relay->scanning;
    out.devices = relay->heard;
    return out;
}

Status Node::request_iphone_scan() {
    if (const auto local = local_iphone()) return local->scan();
    const auto relay = relay_peer();
    if (!relay) {
        return Status::failure("Home Assistant's iPhone relay is not on the network: turn on the add-on's "
                               "send_to_iphone, with pc_key set");
    }
    std::string error;
    if (!send_clip_to(*relay, kScanRequest, error)) return Status::failure(error);
    return Status::success("looking for nearby devices");
}

Status Node::send_to_iphone(const std::filesystem::path& path, const std::string& to) {
    const std::string name = filename_utf8(path);
    if (const auto local = local_iphone()) {
        // Handed over: the radio reports how it went (on_sent) when it knows.
        Status status = local->queue(path, one_line(to).substr(0, 64));
        if (!status.ok && config_.on_sent) config_.on_sent(name, status);
        return status;
    }
    const auto relay = relay_peer();
    Status status = relay ? send_file_to(*relay, path, one_line(to).substr(0, 64))
                          : Status::failure("Home Assistant's iPhone relay is not on the network: turn on the "
                                            "add-on's send_to_iphone, with pc_key set");
    if (config_.on_sent) config_.on_sent(name, status);
    return status;
}

Status Node::send_file_to(const Peer& peer, const std::filesystem::path& path, const std::string& to) {
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) return Status::failure("that path is not a file");
    std::ifstream in(path, std::ios::binary);
    if (!in) return Status::failure("could not open " + path_to_utf8(path));
    in.seekg(0, std::ios::end);
    const auto end = in.tellg();
    in.seekg(0);
    if (end < 0) return Status::failure("could not read the file size");
    const auto size = static_cast<std::uint64_t>(end);
    if (size > config_.max_file_bytes) return Status::failure("file is too large");
    const std::string safe = sanitize_filename(filename_utf8(path));

    Socket socket = Socket::tcp_v4();
    std::string error;
    if (!socket.valid() || !socket.connect_v4(peer.addr_be, peer.tcp_port, 2000, error)) {
        return Status::failure("could not reach " + peer.name + (error.empty() ? "" : ": " + error));
    }
    socket.set_recv_timeout_ms(1000);
    socket.set_send_timeout_ms(5000);
    socket.set_nodelay();
    Message message;
    message.type = MsgType::File;
    message.body_size = size;
    message.fields.set("file", safe);
    message.fields.set("from", config_.id);
    if (!to.empty()) message.fields.set("to", to);
    std::optional<HmacSha256> mac;
    const OpenResult opened = open_transfer(socket, message, config_.pair_key, mac, &stop_, error);
    if (opened == OpenResult::Lost) return Status::failure("could not reach " + peer.name);
    if (opened == OpenResult::Refused) return Status::failure(peer.name + ": " + error);
    std::vector<char> buf(64 * 1024);
    std::uint64_t left = size;
    while (left > 0) {
        const auto chunk = static_cast<std::streamsize>(std::min<std::uint64_t>(left, buf.size()));
        in.read(buf.data(), chunk);
        if (in.gcount() != chunk) return Status::failure("could not read the file");
        if (mac) mac->update(buf.data(), static_cast<std::size_t>(chunk));
        if (!socket.send_full(reinterpret_cast<std::uint8_t*>(buf.data()), static_cast<std::size_t>(chunk), &stop_)) {
            return Status::failure("connection to " + peer.name + " was lost");
        }
        left -= static_cast<std::uint64_t>(chunk);
    }
    if (!close_transfer(socket, mac, &stop_)) return Status::failure("connection to " + peer.name + " was lost");
    Message ack;
    if (!read_head(socket, ack, &stop_, error) || ack.type != MsgType::Ack || ack.fields.get("result") != "ok") {
        std::string why = ack.fields.get("message");
        if (why.empty()) why = error.empty() ? "the other PC did not accept the file" : error;
        return Status::failure(why);
    }
    std::string saved = ack.fields.get("file");
    if (saved.empty()) saved = safe;
    const std::string note = "sent " + saved + " to " + peer.name;
    log_line(note);
    return Status::success(note);
}

bool Node::send_clip_to(const Peer& peer, const std::string& text, std::string& error) {
    Socket socket = Socket::tcp_v4();
    if (!socket.valid() || !socket.connect_v4(peer.addr_be, peer.tcp_port, 2000, error)) {
        if (error.empty()) error = "could not open a connection";
        return false;
    }
    socket.set_recv_timeout_ms(1000);
    socket.set_send_timeout_ms(5000);
    socket.set_nodelay();
    Message message;
    message.type = MsgType::Clip;
    message.body_size = text.size();
    message.fields.set("from", config_.id);
    std::optional<HmacSha256> mac;
    if (open_transfer(socket, message, config_.pair_key, mac, &stop_, error) != OpenResult::Ok) return false;
    if (!text.empty() &&
        !socket.send_full(reinterpret_cast<const std::uint8_t*>(text.data()), text.size(), &stop_)) {
        error = "connection lost";
        return false;
    }
    if (mac) mac->update(text.data(), text.size());
    if (!close_transfer(socket, mac, &stop_)) {
        error = "connection lost";
        return false;
    }
    Message ack;
    if (!read_head(socket, ack, &stop_, error)) return false;
    if (ack.type != MsgType::Ack || ack.fields.get("result") != "ok") {
        error = ack.fields.get("message");
        if (error.empty()) error = "peer did not accept the clipboard";
        return false;
    }
    return true;
}

Status Node::flush_clips() {
    std::lock_guard<std::mutex> send_lock(clip_send_mu_);
    std::uint64_t gen = 0;
    std::string text;
    std::vector<Peer> targets;
    bool any_peers = false;
    {
        std::lock_guard<std::mutex> lock(mu_);
        if (!have_clip_) return Status::success();
        if (clip_text_.size() > kMaxClipBytes) return Status::failure("clipboard text is too large");
        gen = clip_gen_;
        text = clip_text_;
        any_peers = !peers_.empty();
        for (const auto& peer : peers_) {
            // Not the iPhone relay: copied text is for the other PCs.
            if (peer.clip_sent_gen != gen && peer.role.empty()) targets.push_back(peer);
        }
    }
    if (targets.empty()) {
        if (!any_peers) return Status::failure("no other HowlDrop PC is visible yet");
        return Status::success("clipboard sent");
    }
    int sent = 0;
    std::string last_error;
    for (const auto& peer : targets) {
        if (stop_.load()) break;
        std::string error;
        if (send_clip_to(peer, text, error)) {
            ++sent;
            std::lock_guard<std::mutex> lock(mu_);
            for (auto& item : peers_) {
                if (item.id == peer.id && clip_gen_ == gen) item.clip_sent_gen = gen;
            }
            log_line("clipboard to " + peer.name);
        } else {
            last_error = error;
            bool should_log = false;
            {
                std::lock_guard<std::mutex> lock(mu_);
                for (auto& item : peers_) {
                    if (item.id == peer.id && item.clip_error_gen != gen) {
                        item.clip_error_gen = gen;
                        should_log = true;
                    }
                }
            }
            if (should_log) log_line("clipboard to " + peer.name + " failed: " + one_line(error));
        }
    }
    if (sent == 0) return Status::failure(last_error.empty() ? "could not reach a peer" : last_error);
    if (sent != static_cast<int>(targets.size())) {
        return Status::failure("sent the clipboard to " + std::to_string(sent) + " of " + std::to_string(targets.size()) +
                               " PCs");
    }
    return Status::success("clipboard sent");
}

Status Node::push_clipboard() {
    const auto read = clipboard_.read();
    if (read.status != ClipRead::Status::Text || read.text.empty()) return Status::failure("clipboard has no text");
    if (read.text.size() > kMaxClipBytes) return Status::failure("clipboard text is too large");
    const auto seq = clipboard_.sequence();
    {
        std::lock_guard<std::mutex> lock(mu_);
        clip_text_ = read.text;
        have_clip_ = true;
        ++clip_gen_;
        last_seq_ = seq;
        baseline_done_ = true;
    }
    return flush_clips();
}

void Node::poll_clipboard() {
    const auto seq = clipboard_.sequence();
    const auto read = clipboard_.read();
    std::lock_guard<std::mutex> lock(mu_);
    if (!baseline_done_) {
        if (read.status == ClipRead::Status::Busy) return;
        baseline_done_ = true;
        last_seq_ = seq;
        return;
    }
    if (seq == last_seq_) return;
    if (read.status == ClipRead::Status::Busy) return;
    last_seq_ = seq;
    if (read.status != ClipRead::Status::Text) return;
    if (have_clip_ && read.text == clip_text_) return;
    clip_text_ = read.text;
    have_clip_ = true;
    ++clip_gen_;
}

std::vector<Node::PeerView> Node::peers() const {
    std::lock_guard<std::mutex> lock(mu_);
    std::vector<PeerView> out;
    out.reserve(peers_.size());
    for (const auto& peer : peers_) {
        PeerView view;
        view.id = peer.id;
        view.name = peer.name;
        view.ip = ipv4_to_string(peer.addr_be);
        view.port = peer.tcp_port;
        out.push_back(std::move(view));
    }
    return out;
}

void Node::handle_control(Socket socket) {
    socket.set_recv_timeout_ms(1000);
    socket.set_send_timeout_ms(2000);
    std::string line;
    char buf[512];
    int idle = 0;
    bool got_newline = false;
    while (line.size() < 32768 && !got_newline) {
        if (stop_.load()) return;
        const int n = socket.recv_some(buf, sizeof(buf));
        if (n > 0) {
            idle = 0;
            for (int i = 0; i < n; ++i) {
                if (buf[i] == '\n') {
                    got_newline = true;
                    break;
                }
                if (buf[i] != '\r') line.push_back(buf[i]);
            }
            continue;
        }
        if (n == 0) break;
        if (n == -2) {
            if (++idle >= 3) break;
            continue;
        }
        return;
    }
    if (!got_newline && line.empty()) return;

    std::string response;
    bool stopping = false;
    if (line == "peers") {
        const auto list = peers();
        response = "ok " + std::to_string(list.size()) + "\n";
        for (const auto& peer : list) {
            response += peer.name + "\t" + peer.id + "\t" + peer.ip + "\t" + std::to_string(peer.port) + "\n";
        }
    } else if (line == "clip") {
        const Status status = push_clipboard();
        response = (status.ok ? "ok " : "error ") + one_line(status.message.empty() ? (status.ok ? "ok" : "failed") : status.message);
    } else if (line == "nearby") {
        const Nearby nearby = iphone_nearby();
        if (!nearby.relay) {
            response = "error neither AirDrop on this PC nor Home Assistant's iPhone relay is running";
        } else {
            response = std::string("ok ") + (nearby.scanning ? "scanning" : "done") + "\n";
            for (const auto& device : nearby.devices) response += device + "\n";
        }
    } else if (line == "scan") {
        const Status status = request_iphone_scan();
        response = (status.ok ? "ok " : "error ") + one_line(status.message);
    } else if (line.rfind("iphone ", 0) == 0) {
        // Sent in the background: a big file outlasts the control reply's
        // timeout. on_sent (the tray's notification) says how it went.
        // "iphone NAME<tab>PATH" sends to that device; "iphone PATH" to the
        // add-on's default.
        std::string rest = line.substr(7);
        std::string to;
        const auto tab = rest.find('\t');
        if (tab != std::string::npos) {
            to = rest.substr(0, tab);
            rest = rest.substr(tab + 1);
        }
        const auto path = path_from_utf8(rest);
        std::error_code ec;
        bool relay = false;
        {
            std::lock_guard<std::mutex> lock(mu_);
            relay = local_iphone_ != nullptr;
            for (const auto& peer : peers_) relay = relay || peer.role == "iphone";
        }
        if (!std::filesystem::is_regular_file(path, ec)) {
            response = "error that path is not a file";
        } else if (!relay) {
            response = "error turn on AirDrop on this PC (the HowlDrop icon's menu), or the Home Assistant "
                       "add-on's send_to_iphone with pc_key set";
        } else {
            spawn([this, path, to]() { send_to_iphone(path, to); });
            response = "ok sending";
        }
    } else if (line == "quit" || line == "stop") {
        stopping = true;  // after the reply: a stop would cut its send short
        response = "ok stopping";
    } else if (line.rfind("send ", 0) == 0) {
        const std::string rest = line.substr(5);
        const auto space = rest.find(' ');
        if (space == std::string::npos || space == 0 || space + 1 >= rest.size()) {
            response = "error usage: send NAME PATH";
        } else {
            const std::string peer = rest.substr(0, space);
            const std::string path_text = rest.substr(space + 1);
            const Status status = send_file(peer, path_from_utf8(path_text));
            response = (status.ok ? "ok " : "error ") +
                       one_line(status.message.empty() ? (status.ok ? "sent" : "failed") : status.message);
        }
    } else if (line.empty()) {
        response = "error empty command";
    } else {
        response = "error unknown command";
    }
    if (response.empty() || response.back() != '\n') response.push_back('\n');
    socket.send_full(reinterpret_cast<const std::uint8_t*>(response.data()), response.size(), &stop_, 3);
    if (stopping) request_stop();
}

void Node::reap() {
    std::vector<Job> current;
    {
        std::lock_guard<std::mutex> lock(jobs_mu_);
        current.swap(jobs_);
    }
    std::vector<Job> keep;
    for (auto& job : current) {
        if (job.done && job.done->load()) {
            if (job.thread.joinable()) job.thread.join();
        } else {
            keep.push_back(std::move(job));
        }
    }
    if (!keep.empty()) {
        std::lock_guard<std::mutex> lock(jobs_mu_);
        for (auto& job : jobs_) keep.push_back(std::move(job));
        jobs_.swap(keep);
    }
}

void Node::join_jobs() {
    for (;;) {
        std::vector<Job> current;
        {
            std::lock_guard<std::mutex> lock(jobs_mu_);
            current.swap(jobs_);
        }
        if (current.empty()) {
            std::lock_guard<std::mutex> lock(jobs_mu_);
            if (jobs_.empty()) return;
            continue;
        }
        for (auto& job : current) {
            if (job.thread.joinable()) job.thread.join();
        }
    }
}

void Node::log_line(const std::string& line) {
    if (!config_.log) return;
    std::lock_guard<std::mutex> lock(log_mu_);
    if (config_.on_log) {
        config_.on_log(line);
        return;
    }
    std::cout << line << '\n' << std::flush;
}

}  // namespace howldrop
