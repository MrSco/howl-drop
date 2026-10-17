#include "forward.hpp"
#include "net.hpp"
#include "node.hpp"

#include <chrono>
#include <csignal>
#include <cstdlib>
#include <fstream>
#include <cstdint>
#include <filesystem>
#include <iostream>
#include <map>
#include <set>
#include <string>
#include <thread>
#include <vector>

namespace {

void usage() {
    std::cout
        << "HowlDrop bridge\n"
        << "Forwards an AirDrop file or shared text from this Pi to howldrop.exe on the Windows PC.\n"
        << "The PC is found from its HowlDrop hello on UDP 47631. Files and text go to TCP 47632.\n\n"
        << "  howldrop-bridge name                 Print the Windows PC's name\n"
        << "  howldrop-bridge discover             Print name, address, id, and transfer port\n"
        << "  howldrop-bridge watch --inbox DIR    Forward new files in DIR, and text in DIR/text\n"
        << "  howldrop-bridge relay --inbox DIR    Take files from a paired PC into DIR (the iPhone relay)\n"
        << "  howldrop-bridge file [--host IP] FILE\n"
        << "  howldrop-bridge text [--host IP] TEXT\n\n"
        << "  --host IP       PC address. Without it, wait for a hello beacon\n"
        << "  --port N        PC transfer port (default 47632) when --host is set\n"
        << "  --name NAME     Which PC, when more than one answers\n"
        << "  --inbox DIR     Folder omdrop saves into (watch)\n"
        << "  --timeout SEC   How long to wait for a hello (default 8)\n"
        << "  --keep          (watch) Leave files in DIR after forwarding, and forward only\n"
        << "                  ones that arrive after the bridge starts\n"
        << "  --wait          (watch) No PC yet, or it went away: keep looking for one\n\n"
        << "HOWLDROP_PC_KEY in the environment is the PC's pairing key: files then go only\n"
        << "to a PC that proves it, signed with it.\n";
}

struct Options {
    std::string command;
    std::string host;
    std::uint16_t port = 0;
    std::string name;
    std::filesystem::path inbox;
    std::string text;
    std::filesystem::path file;
    int timeout_s = 8;
    bool text_stdin = false;
    bool keep = false;
    bool wait = false;
    std::string pair_key;  // from HOWLDROP_PC_KEY: not an argument, so not in ps
    std::filesystem::path nearby;        // relay: outbox.py's latest scan, a name per line
    std::filesystem::path scan_request;  // relay: the file that asks outbox.py for a scan
};

bool parse_port(const std::string& text, std::uint16_t& port) {
    if (text.empty()) return false;
    int value = 0;
    for (char c : text) {
        if (c < '0' || c > '9') return false;
        value = value * 10 + (c - '0');
        if (value > 65535) return false;
    }
    if (value <= 0) return false;
    port = static_cast<std::uint16_t>(value);
    return true;
}

bool parse_args(const std::vector<std::string>& args, Options& opt, std::string& error) {
    if (args.size() < 2) {
        opt.command = "help";
        return true;
    }
    opt.command = args[1];
    std::vector<std::string> positionals;
    for (std::size_t i = 2; i < args.size(); ++i) {
        const std::string& arg = args[i];
        auto need = [&](const char* flag) -> const std::string* {
            if (i + 1 >= args.size()) {
                error = std::string(flag) + " needs a value";
                return nullptr;
            }
            return &args[++i];
        };
        if (arg == "--host") {
            const std::string* value = need("--host");
            if (!value) return false;
            opt.host = *value;
        } else if (arg == "--port") {
            const std::string* value = need("--port");
            if (!value || !parse_port(*value, opt.port)) {
                if (error.empty()) error = "--port needs a port number";
                return false;
            }
        } else if (arg == "--name") {
            const std::string* value = need("--name");
            if (!value) return false;
            opt.name = *value;
        } else if (arg == "--inbox") {
            const std::string* value = need("--inbox");
            if (!value) return false;
            opt.inbox = *value;
        } else if (arg == "--nearby") {
            const std::string* value = need("--nearby");
            if (!value) return false;
            opt.nearby = *value;
        } else if (arg == "--scan-request") {
            const std::string* value = need("--scan-request");
            if (!value) return false;
            opt.scan_request = *value;
        } else if (arg == "--timeout") {
            const std::string* value = need("--timeout");
            if (!value) return false;
            int seconds = 0;
            for (char c : *value) {
                if (c < '0' || c > '9') {
                    error = "--timeout needs a number of seconds";
                    return false;
                }
                seconds = seconds * 10 + (c - '0');
                if (seconds > 3600) break;
            }
            if (seconds <= 0) {
                error = "--timeout needs a number of seconds";
                return false;
            }
            opt.timeout_s = seconds;
        } else if (arg == "--keep") {
            opt.keep = true;
        } else if (arg == "--wait") {
            opt.wait = true;
        } else if (arg == "--help" || arg == "-h" || arg == "help") {
            opt.command = "help";
        } else if (!arg.empty() && arg[0] == '-') {
            error = "unknown option " + arg;
            return false;
        } else {
            positionals.push_back(arg);
        }
    }
    if (opt.command == "file") {
        if (positionals.empty()) {
            error = "file needs a path";
            return false;
        }
        std::string path = positionals[0];
        for (std::size_t i = 1; i < positionals.size(); ++i) {
            path.push_back(' ');
            path += positionals[i];
        }
        opt.file = path;
    } else if (opt.command == "text") {
        if (positionals.empty()) {
            error = "text needs the shared text";
            return false;
        }
        if (positionals.size() == 1 && positionals[0] == "-") {
            opt.text_stdin = true;
        } else {
            opt.text = positionals[0];
            for (std::size_t i = 1; i < positionals.size(); ++i) {
                opt.text.push_back(' ');
                opt.text += positionals[i];
            }
        }
    } else if (opt.command != "name" && opt.command != "discover" && opt.command != "watch" && opt.command != "relay" &&
               opt.command != "help") {
        error = "unknown command " + opt.command;
        return false;
    }
    return true;
}

std::optional<howldrop::LanTarget> resolve_target(const Options& opt, std::string& error) {
    std::optional<howldrop::LanTarget> target;
    if (!opt.host.empty()) {
        target.emplace();
        target->host = opt.host;
        target->port = opt.port == 0 ? howldrop::kTransferPort : opt.port;
        target->name = opt.name.empty() ? opt.host : opt.name;
    } else {
        target = howldrop::discover_target(howldrop::kDiscoveryPort, std::chrono::seconds(opt.timeout_s), opt.name,
                                           error);
    }
    if (target) target->pair_key = opt.pair_key;
    return target;
}

int fail(const std::string& message) {
    std::cerr << message << '\n';
    return 1;
}

std::vector<std::filesystem::path> inbox_files(const std::filesystem::path& inbox) {
    std::vector<std::filesystem::path> files;
    std::error_code ec;
    for (const auto& entry : std::filesystem::directory_iterator(inbox, ec)) {
        if (entry.is_regular_file(ec)) files.push_back(entry.path());
    }
    const auto text_dir = inbox / "text";
    if (std::filesystem::is_directory(text_dir, ec)) {
        for (const auto& entry : std::filesystem::directory_iterator(text_dir, ec)) {
            if (entry.is_regular_file(ec)) files.push_back(entry.path());
        }
    }
    return files;
}

// A run of this many retryable failures means the PC has gone (closed,
// asleep, a new address): with --wait, watch returns to looking for one.
constexpr int kGoneAfterRetries = 20;

// Forward what settles in inbox until the PC looks gone (only when it can be
// looked for again) . handled holds what keep mode has already dealt with,
// across returns: forwarded, refused, or there before the bridge started.
void watch_loop(const howldrop::LanTarget& target, const std::filesystem::path& inbox, bool keep, bool can_rediscover,
                howldrop::HandledFiles& handled) {
    std::filesystem::create_directories(inbox / "text");
    std::cout << "forwarding to " << target.name << " at " << target.host << ":" << target.port << '\n';
    std::cout << "inbox " << inbox << '\n';
    // Flushed: the add-on reads this through a pipe, and an unflushed line
    // shows up minutes late or not at all.
    std::cout << "text " << (inbox / "text") << std::endl;

    struct Stamp {
        std::uintmax_t size = 0;
        int seen = 0;
    };
    std::map<std::string, Stamp> stamps;
    howldrop::HandledFiles gave_up;
    int retries = 0;

    while (true) {
        const std::vector<std::filesystem::path> files = inbox_files(inbox);
        std::error_code ec;
        std::set<std::string> live;
        for (const auto& file : files) {
            const std::string key = file.string();
            live.insert(key);
            if (gave_up.has(file) || handled.has(file)) continue;
            const auto size = std::filesystem::file_size(file, ec);
            if (ec) continue;
            Stamp& stamp = stamps[key];
            if (stamp.seen == 0 || stamp.size != size) {
                stamp.size = size;
                stamp.seen = 1;
                continue;
            }
            ++stamp.seen;
            if (stamp.seen < 2) continue;
            const howldrop::DropResult result = howldrop::take_drop(inbox, file, target, keep);
            stamps.erase(key);
            if (result.action == howldrop::DropAction::Retry) {
                std::cerr << file.filename().string() << ": " << result.message << '\n';
                if (can_rediscover && ++retries >= kGoneAfterRetries) {
                    std::cerr << target.name << " stopped answering; looking for a PC again\n";
                    return;
                }
                continue;
            }
            retries = 0;
            if (keep) handled.add(file);
            if (result.action == howldrop::DropAction::Rejected) {
                gave_up.add(file);
                std::cerr << file.filename().string() << ": " << result.message << '\n';
                continue;
            }
            if (result.action == howldrop::DropAction::SentFile) {
                std::cout << "sent file " << result.message << std::endl;
            } else if (result.action == howldrop::DropAction::SentText) {
                std::cout << "sent text" << std::endl;
            }
        }
        for (auto it = stamps.begin(); it != stamps.end();) {
            if (!live.count(it->first)) it = stamps.erase(it);
            else ++it;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(300));
    }
}

volatile std::sig_atomic_t g_stop = 0;
void on_stop(int) { g_stop = 1; }

// The id the relay announces, kept in the outbox so the PCs see the same
// device across restarts.
std::string relay_id(const std::filesystem::path& inbox) {
    const auto path = inbox / ".relay.id";
    {
        std::ifstream in(path);
        std::string line;
        if (in && std::getline(in, line) && howldrop::is_node_id(line)) return line;
    }
    std::uint8_t bytes[16];
    if (!howldrop::secure_random(bytes, sizeof(bytes))) return {};
    const std::string id = howldrop::to_hex(bytes, sizeof(bytes));
    std::ofstream(path, std::ios::trunc) << id << '\n';
    return id;
}

// What the relay says about nearby devices in its hello, from outbox.py's
// scan file: "scanning" while one runs, then "device<tab>NAME" lines. The PC
// lists them (tray menu, howldrop nearby). Kept within a header field's 1024.
std::vector<std::pair<std::string, std::string>> nearby_fields(const std::filesystem::path& file) {
    std::ifstream in(file);
    std::string line;
    std::string heard;
    bool scanning = false;
    while (std::getline(in, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line == "scanning") {
            scanning = true;
        } else if (line.rfind("device\t", 0) == 0) {
            const std::string name = line.substr(7);
            if (name.empty() || !howldrop::valid_display_name(name) || heard.size() + name.size() + 1 > 1000) continue;
            if (!heard.empty()) heard += '\t';
            heard += name;
        }
    }
    std::vector<std::pair<std::string, std::string>> fields;
    if (!heard.empty()) fields.emplace_back("near", heard);
    if (scanning) fields.emplace_back("scanning", "1");
    return fields;
}

// The add-on's iPhone relay: a HowlDrop node a paired PC sends to (howldrop
// iphone FILE, the tray, Explorer's Send to). What it receives lands in
// inbox, the add-on's send folder, which outbox.py AirDrops to the iPhone.
// Paired only: without HOWLDROP_PC_KEY it does not start.
int run_relay(const Options& opt) {
    if (opt.inbox.empty()) return fail("relay needs --inbox, the folder outbox.py sends from");
    if (opt.pair_key.empty()) return fail("relay needs HOWLDROP_PC_KEY, the PC's pairing key");
    std::error_code ec;
    std::filesystem::create_directories(opt.inbox, ec);
    howldrop::Node::Config config;
    config.name = opt.name.empty() ? "iPhone (HowlDrop)" : opt.name;
    config.id = relay_id(opt.inbox);
    config.discovery_port = howldrop::kRelayDiscoveryPort;
    config.transfer_port = howldrop::kRelayTransferPort;
    config.announce_port = howldrop::kDiscoveryPort;
    config.inbox = opt.inbox;
    config.enable_control = false;
    config.pair_key = opt.pair_key;
    config.role = "iphone";
    if (!opt.nearby.empty()) {
        const auto nearby = opt.nearby;
        config.hello_extra = [nearby]() { return nearby_fields(nearby); };
    }
    // The device a file is for, beside it, as outbox.py reads it.
    config.on_target = [](const std::filesystem::path& saved, const std::string& to) {
        auto sidecar = saved;
        sidecar += ".howldrop-to";
        std::ofstream(sidecar, std::ios::trunc) << to;
    };
    // A paired PC asking for a scan (Node::request_iphone_scan).
    if (!opt.scan_request.empty()) {
        const auto request = opt.scan_request;
        config.on_text = [request](const std::string& text, const std::string&) {
            if (text == howldrop::kScanRequest) std::ofstream(request, std::ios::trunc) << "pc\n";
        };
    }
    // Said before the node starts: its own log lines would cut into this one.
    std::cout << "relay \"" << config.name << "\" on TCP " << howldrop::kRelayTransferPort
              << ": files from a paired PC go to " << opt.inbox << std::endl;
    howldrop::MemoryClipboard clipboard;
    howldrop::Node node(config, clipboard);
    try {
        node.start();
    } catch (const std::exception& ex) {
        return fail(ex.what());
    }
    std::signal(SIGINT, on_stop);
    std::signal(SIGTERM, on_stop);
    while (!g_stop && node.running()) std::this_thread::sleep_for(std::chrono::milliseconds(200));
    node.request_stop();
    node.stop();
    return 0;
}

int run_watch(const Options& opt) {
    std::string error;
    std::filesystem::path inbox = opt.inbox;
    if (inbox.empty()) {
        if (const char* home = std::getenv("HOME")) inbox = std::filesystem::path(home) / "HowlDrop" / "incoming";
        else inbox = std::filesystem::path("HowlDrop") / "incoming";
    }
    std::error_code ec;
    std::filesystem::create_directories(inbox / "text", ec);
    if (ec) return fail("could not create " + inbox.string());
    howldrop::HandledFiles handled;
    if (opt.keep) {
        for (const auto& file : inbox_files(inbox)) handled.add(file);
        std::cout << "keeping files in " << inbox << "; " << handled.size()
                  << " already there are not forwarded" << std::endl;
    }
    const bool can_rediscover = opt.wait && opt.host.empty();
    auto target = resolve_target(opt, error);
    bool looking = false;
    while (true) {
        if (!target) {
            if (!opt.wait) return fail(error.empty() ? "no HowlDrop PC answered" : error);
            if (!looking) {
                std::cout << "no HowlDrop PC answering; files stay in " << inbox << ", still looking" << std::endl;
                looking = true;
            }
            std::this_thread::sleep_for(std::chrono::seconds(5));
            error.clear();
            target = resolve_target(opt, error);
            continue;
        }
        looking = false;
        watch_loop(*target, inbox, opt.keep, can_rediscover, handled);
        target.reset();
    }
}

}  // namespace

int main(int argc, char** argv) {
    std::vector<std::string> args;
    args.reserve(static_cast<std::size_t>(argc));
    for (int i = 0; i < argc; ++i) args.emplace_back(argv[i] ? argv[i] : "");

    Options opt;
    std::string error;
    if (!parse_args(args, opt, error)) return fail(error.empty() ? "bad arguments" : error);
    if (const char* key = std::getenv("HOWLDROP_PC_KEY"); key && *key) {
        opt.pair_key = howldrop::normalize_pair_key(key);
        if (opt.pair_key.empty()) return fail("HOWLDROP_PC_KEY is not a pairing key (32 hex digits)");
    }
    if (opt.command == "help") {
        usage();
        return 0;
    }
    if (!howldrop::startup_network()) return fail("could not start networking");

    if (opt.command == "name" || opt.command == "discover") {
        const auto target = howldrop::discover_target(howldrop::kDiscoveryPort, std::chrono::seconds(opt.timeout_s),
                                                     opt.name, error);
        if (!target) return fail(error.empty() ? "no HowlDrop PC answered" : error);
        if (opt.command == "name") {
            std::cout << target->name << '\n';
        } else {
            std::cout << target->name << '\t' << target->host << '\t' << target->id << '\t' << target->port << '\n';
        }
        return 0;
    }

    if (opt.command == "watch") return run_watch(opt);
    if (opt.command == "relay") return run_relay(opt);

    const auto target = resolve_target(opt, error);
    if (!target) return fail(error.empty() ? "no HowlDrop PC answered" : error);

    if (opt.command == "file") {
        const howldrop::Status status = howldrop::forward_file(*target, opt.file, {});
        if (!status.ok) return fail(status.message);
        std::cout << "sent " << status.message << " to " << target->name << '\n';
        return 0;
    }
    if (opt.command == "text") {
        std::string text = opt.text;
        if (opt.text_stdin) {
            std::string chunk;
            char buf[4096];
            while (std::cin.read(buf, sizeof(buf)) || std::cin.gcount() > 0) {
                chunk.append(buf, static_cast<std::size_t>(std::cin.gcount()));
                if (chunk.size() > howldrop::kMaxClipBytes) return fail("too large");
            }
            text = std::move(chunk);
        }
        const howldrop::Status status = howldrop::forward_text(*target, text);
        if (!status.ok) return fail(status.message);
        std::cout << "sent text to " << target->name << '\n';
        return 0;
    }
    usage();
    return 2;
}
