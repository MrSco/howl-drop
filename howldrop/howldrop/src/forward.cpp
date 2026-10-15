#include "forward.hpp"

#include "config.hpp"
#include "net.hpp"
#include "text.hpp"

#include <fstream>
#include <utility>
#include <vector>

namespace howldrop {
namespace {

bool retryable(const Status& status) {
    return !status.ok && status.message.rfind("rejected:", 0) != 0 && status.message.rfind("too large", 0) != 0 &&
           status.message.rfind("that path", 0) != 0 && status.message.rfind("could not read", 0) != 0 &&
           status.message.rfind("empty", 0) != 0;
}

Status connect_transfer(const LanTarget& target, Socket& socket) {
    if (target.host.empty() || target.port == 0) return Status::failure("could not reach the PC");
    std::uint32_t addr = 0;
    if (!ipv4_from_string(target.host, addr)) return Status::failure("could not reach the PC");
    socket = Socket::tcp_v4();
    std::string error;
    if (!socket.valid()) return Status::failure("could not reach the PC: no socket");
    if (!socket.connect_v4(addr, target.port, 2000, error)) {
        return Status::failure("could not reach the PC: connecting to " + target.host + ":" +
                               std::to_string(target.port) + " " + (error.empty() ? "failed" : error));
    }
    socket.set_recv_timeout_ms(1000);
    socket.set_send_timeout_ms(5000);
    socket.set_nodelay();
    return Status::success();
}

Status read_ack(Socket& socket) {
    Message ack;
    std::string error;
    if (!read_head(socket, ack, nullptr, error)) {
        return Status::failure("could not reach the PC: no answer after sending (" +
                               (error.empty() ? std::string("connection closed") : error) + ")");
    }
    if (ack.type != MsgType::Ack || ack.fields.get("result") != "ok") {
        std::string why = ack.fields.get("message");
        if (why.empty()) why = "the PC did not accept it";
        return Status::failure("rejected: " + why);
    }
    std::string saved = ack.fields.get("file");
    return Status::success(saved);
}

bool move_aside(const std::filesystem::path& directory, const std::filesystem::path& file) {
    std::error_code ec;
    std::filesystem::create_directories(directory, ec);
    if (ec) return false;
    const std::string name = filename_utf8(file);
    auto dest = directory / path_from_utf8(name);
    if (std::filesystem::exists(dest)) {
        std::string stem = name;
        std::string ext;
        const auto dot = name.find_last_of('.');
        if (dot != std::string::npos && dot != 0) {
            stem = name.substr(0, dot);
            ext = name.substr(dot);
        }
        bool placed = false;
        for (int i = 1; i < 10000; ++i) {
            dest = directory / path_from_utf8(stem + " (" + std::to_string(i) + ")" + ext);
            if (!std::filesystem::exists(dest)) {
                placed = true;
                break;
            }
        }
        if (!placed) return false;
    }
    std::filesystem::rename(file, dest, ec);
    if (!ec) return true;
    std::filesystem::copy_file(file, dest, std::filesystem::copy_options::overwrite_existing, ec);
    if (ec) return false;
    std::filesystem::remove(file, ec);
    return !ec;
}

}  // namespace

std::optional<LanTarget> target_from_hello(const std::uint8_t* data, std::size_t n, const std::string& from_ip) {
    std::string error;
    const auto message = parse_datagram(data, n, error);
    if (!message || message->type != MsgType::Hello) return std::nullopt;
    const std::string id = message->fields.get("id");
    const std::string name = message->fields.get("name");
    const std::string port_text = message->fields.get("port");
    if (!is_node_id(id) || !valid_display_name(name) || from_ip.empty() || from_ip == "0.0.0.0") return std::nullopt;
    int port = 0;
    try {
        port = std::stoi(port_text);
    } catch (...) {
        return std::nullopt;
    }
    if (port <= 0 || port > 65535) return std::nullopt;
    LanTarget target;
    target.host = from_ip;
    target.port = static_cast<std::uint16_t>(port);
    target.name = name;
    target.id = id;
    return target;
}

std::optional<LanTarget> discover_target(std::uint16_t discovery_port, std::chrono::milliseconds timeout,
                                        const std::string& name_filter, std::string& error) {
    if (discovery_port == 0) {
        error = "discovery port is not set";
        return std::nullopt;
    }
    Socket udp = Socket::udp_v4();
    if (!udp.valid() || !udp.set_reuseaddr() || !udp.bind_v4(0, discovery_port)) {
        error = "UDP port " + std::to_string(discovery_port) + " is not available";
        return std::nullopt;
    }
    udp.set_recv_timeout_ms(200);

    using Clock = std::chrono::steady_clock;
    const auto deadline = Clock::now() + timeout;
    std::optional<LanTarget> found;
    std::optional<Clock::time_point> settle;

    while (Clock::now() < deadline) {
        if (found && settle && Clock::now() >= *settle) return found;
        std::uint8_t buf[2048];
        std::uint32_t from_be = 0;
        std::uint16_t from_port = 0;
        const int n = udp.recv_from(buf, sizeof(buf), from_be, from_port);
        if (n <= 0) continue;
        auto target = target_from_hello(buf, static_cast<std::size_t>(n), ipv4_to_string(from_be));
        if (!target) continue;
        if (!name_filter.empty() && !names_equal(target->name, name_filter)) continue;
        if (!found) {
            found = std::move(target);
            if (!name_filter.empty()) return found;
            const auto extra = Clock::now() + std::chrono::milliseconds(400);
            settle = extra < deadline ? extra : deadline;
            continue;
        }
        if (found->id != target->id) {
            error = "more than one HowlDrop PC is visible (" + found->name + ", " + target->name + "). Pass --name";
            return std::nullopt;
        }
        found->host = target->host;
        found->port = target->port;
        found->name = target->name;
    }
    if (!found) {
        error = name_filter.empty() ? "no HowlDrop PC answered"
                                    : "no HowlDrop PC named \"" + name_filter + "\" answered";
        return std::nullopt;
    }
    return found;
}

Status forward_file(const LanTarget& target, const std::filesystem::path& path, const std::string& advertised_name) {
    std::error_code ec;
    if (!std::filesystem::is_regular_file(path, ec)) return Status::failure("that path is not a file");
    std::ifstream in(path, std::ios::binary);
    if (!in) return Status::failure("could not read the file");
    in.seekg(0, std::ios::end);
    const auto end = in.tellg();
    in.seekg(0);
    if (end < 0) return Status::failure("could not read the file");
    const auto size = static_cast<std::uint64_t>(end);
    if (size > kMaxFileBytes) return Status::failure("too large");

    std::string name = advertised_name.empty() ? filename_utf8(path) : advertised_name;
    if (name.empty()) name = "file";

    Socket socket;
    const Status connected = connect_transfer(target, socket);
    if (!connected.ok) return connected;

    Message message;
    message.type = MsgType::File;
    message.body_size = size;
    message.fields.set("file", name);
    message.fields.set("from", kBridgeFromId);
    std::string error;
    if (!write_head(socket, message, nullptr, error)) return Status::failure("rejected: " + error);

    std::vector<char> buf(64 * 1024);
    std::uint64_t left = size;
    while (left > 0) {
        const auto chunk = static_cast<std::streamsize>(std::min<std::uint64_t>(left, buf.size()));
        in.read(buf.data(), chunk);
        if (in.gcount() != chunk) return Status::failure("could not read the file");
        if (!socket.send_full(reinterpret_cast<std::uint8_t*>(buf.data()), static_cast<std::size_t>(chunk), nullptr)) {
            const std::string why = last_socket_error();
            return Status::failure("could not reach the PC: the connection broke after " + std::to_string(size - left) +
                                   " of " + std::to_string(size) + " bytes (" + why + ")");
        }
        left -= static_cast<std::uint64_t>(chunk);
    }
    Status ack = read_ack(socket);
    if (!ack.ok) return ack;
    if (ack.message.empty()) ack.message = name;
    return ack;
}

Status forward_text(const LanTarget& target, std::string_view text) {
    if (text.size() > kMaxClipBytes) return Status::failure("too large");
    Socket socket;
    const Status connected = connect_transfer(target, socket);
    if (!connected.ok) return connected;

    Message message;
    message.type = MsgType::Clip;
    message.body_size = text.size();
    message.fields.set("from", kBridgeFromId);
    std::string error;
    if (!write_head(socket, message, nullptr, error)) return Status::failure("rejected: " + error);
    if (!text.empty() &&
        !socket.send_full(reinterpret_cast<const std::uint8_t*>(text.data()), text.size(), nullptr)) {
        return Status::failure("could not reach the PC: the connection broke while sending (" + last_socket_error() +
                               ")");
    }
    return read_ack(socket);
}

DropKind classify_inbox_file(const std::filesystem::path& inbox, const std::filesystem::path& file) {
    try {
        std::error_code ec;
        if (std::filesystem::is_symlink(file, ec) || ec) return DropKind::Skip;
        const auto base = std::filesystem::weakly_canonical(inbox, ec);
        if (ec) return DropKind::Skip;
        const auto canon = std::filesystem::weakly_canonical(file, ec);
        if (ec) return DropKind::Skip;
        if (!std::filesystem::is_regular_file(canon, ec) || ec) return DropKind::Skip;
        const auto rel = std::filesystem::relative(canon, base, ec);
        if (ec) return DropKind::Skip;
        const std::string generic = rel.generic_string();
        if (generic.empty() || generic == "." || generic.rfind("..", 0) == 0) return DropKind::Skip;

        int depth = 0;
        bool text_dir = false;
        std::string leaf;
        for (const auto& part : rel) {
            const std::string name = part.generic_string();
            if (name.empty() || name == "." || name == ".." || name[0] == '.') return DropKind::Skip;
            leaf = name;
            if (depth == 0 && name == "text") text_dir = true;
            ++depth;
        }
        if (leaf.size() >= 8 && leaf.compare(leaf.size() - 8, 8, ".partial") == 0) return DropKind::Skip;
        if (depth == 1) return DropKind::File;
        if (depth == 2 && text_dir) return DropKind::Text;
        return DropKind::Skip;
    } catch (...) {
        return DropKind::Skip;
    }
}

DropResult take_drop(const std::filesystem::path& inbox, const std::filesystem::path& file, const LanTarget& target,
                     bool keep) {
    const DropKind kind = classify_inbox_file(inbox, file);
    if (kind == DropKind::Skip) return DropResult{DropAction::Skip, {}};

    Status status = kind == DropKind::Text ? Status::failure("could not read the file") : Status::success();
    if (kind == DropKind::Text) {
        std::error_code ec;
        const auto bytes = std::filesystem::file_size(file, ec);
        if (ec) return DropResult{DropAction::Rejected, "could not read the file"};
        if (bytes > kMaxClipBytes) {
            if (!keep) move_aside(inbox / ".failed", file);
            return DropResult{DropAction::Rejected, "too large"};
        }
        std::ifstream in(file, std::ios::binary);
        std::string text(static_cast<std::size_t>(bytes), '\0');
        if (bytes && !in.read(text.data(), static_cast<std::streamsize>(bytes))) {
            return DropResult{DropAction::Rejected, "could not read the file"};
        }
        status = forward_text(target, text);
    } else {
        status = forward_file(target, file, filename_utf8(file));
    }

    if (!status.ok) {
        if (retryable(status)) return DropResult{DropAction::Retry, status.message};
        if (!keep) move_aside(inbox / ".failed", file);
        return DropResult{DropAction::Rejected, status.message};
    }
    if (!keep) move_aside(inbox / ".forwarded", file);
    const DropAction action = kind == DropKind::Text ? DropAction::SentText : DropAction::SentFile;
    return DropResult{action, status.message};
}

std::string HandledFiles::identity(const std::filesystem::path& file) {
    std::error_code ec;
    const auto size = std::filesystem::file_size(file, ec);
    if (ec) return {};
    const auto time = std::filesystem::last_write_time(file, ec);
    if (ec) return {};
    return std::to_string(size) + ":" + std::to_string(time.time_since_epoch().count());
}

void HandledFiles::add(const std::filesystem::path& file) { files_[file.string()] = identity(file); }

bool HandledFiles::has(const std::filesystem::path& file) const {
    const auto it = files_.find(file.string());
    if (it == files_.end()) return false;
    const std::string now = identity(file);
    return !now.empty() && now == it->second;
}

}  // namespace howldrop
