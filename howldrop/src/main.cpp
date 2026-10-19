#include "node.hpp"
#include "pairing.hpp"

#include <chrono>
#include <csignal>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iostream>
#include <random>
#include <thread>
#include <vector>

#ifdef _WIN32
#include <shellapi.h>

#include "tray.hpp"
#else
#include <csignal>
#include <unistd.h>
#endif

namespace {

volatile std::sig_atomic_t g_stop = 0;

#ifdef _WIN32
BOOL WINAPI console_handler(DWORD type) {
    if (type == CTRL_C_EVENT || type == CTRL_BREAK_EVENT || type == CTRL_CLOSE_EVENT) {
        g_stop = 1;
        return TRUE;
    }
    return FALSE;
}

std::wstring env_w(const wchar_t* name) {
    const DWORD needed = GetEnvironmentVariableW(name, nullptr, 0);
    if (needed == 0) return {};
    std::wstring value(needed, L'\0');
    const DWORD got = GetEnvironmentVariableW(name, value.data(), needed);
    if (got == 0 || got >= needed) return {};
    value.resize(got);
    return value;
}
#endif

#ifndef _WIN32
void on_signal(int) { g_stop = 1; }
#endif

std::string computer_name() {
#ifdef _WIN32
    wchar_t buf[256];
    DWORD n = 256;
    if (!GetComputerNameW(buf, &n) || n == 0) return "windows";
    std::string name = howldrop::utf8_from_wide(std::wstring(buf, n));
    if (name.empty()) return "windows";
    if (name.size() > 64) name.resize(64);
    return name;
#else
    char buf[256];
    if (gethostname(buf, sizeof(buf)) != 0) return "pc";
    buf[sizeof(buf) - 1] = '\0';
    std::string name(buf);
    const auto dot = name.find('.');
    if (dot != std::string::npos) name.resize(dot);
    if (name.empty()) name = "pc";
    if (name.size() > 64) name.resize(64);
    return name;
#endif
}

std::filesystem::path state_dir() {
#ifdef _WIN32
    std::wstring local = env_w(L"LOCALAPPDATA");
    if (local.empty()) {
        const std::wstring profile = env_w(L"USERPROFILE");
        if (profile.empty()) return {};
        return std::filesystem::path(profile) / L"AppData" / L"Local" / L"HowlDrop";
    }
    return std::filesystem::path(local) / L"HowlDrop";
#else
    if (const char* home = std::getenv("HOME")) return std::filesystem::path(home) / ".local" / "share" / "howldrop";
    return std::filesystem::temp_directory_path() / "howldrop";
#endif
}

std::filesystem::path inbox_dir() {
#ifdef _WIN32
    const std::wstring profile = env_w(L"USERPROFILE");
    if (profile.empty()) return {};
    return std::filesystem::path(profile) / L"Downloads" / L"HowlDrop";
#else
    if (const char* home = std::getenv("HOME")) return std::filesystem::path(home) / "Downloads" / "HowlDrop";
    return std::filesystem::temp_directory_path() / "HowlDrop";
#endif
}

bool hex_id(const std::string& text) {
    if (text.size() != 32) return false;
    for (unsigned char c : text) {
        if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) return false;
    }
    return true;
}

std::string load_id() {
    const auto dir = state_dir();
    if (dir.empty()) return {};
    std::filesystem::create_directories(dir);
    const auto path = dir / "node.id";
    if (std::filesystem::exists(path)) {
        std::ifstream in(path);
        std::string line;
        std::getline(in, line);
        while (!line.empty() && (line.back() == '\r' || line.back() == ' ')) line.pop_back();
        for (char& c : line) {
            if (c >= 'A' && c <= 'F') c = static_cast<char>(c - 'A' + 'a');
        }
        if (hex_id(line)) return line;
    }
    std::string id(32, '0');
    static constexpr char hex[] = "0123456789abcdef";
    try {
        std::random_device device;
        for (int i = 0; i < 32; ++i) id[static_cast<std::size_t>(i)] = hex[device() & 0x0f];
    } catch (...) {
        const auto seed = static_cast<unsigned>(std::chrono::steady_clock::now().time_since_epoch().count());
        std::mt19937 gen(seed);
        for (int i = 0; i < 32; ++i) id[static_cast<std::size_t>(i)] = hex[gen() & 0x0f];
    }
    std::ofstream out(path, std::ios::trunc);
    if (!out) return id;
    out << id << '\n';
    return id;
}

// This PC's pairing key (pairing.hpp), in the state folder next to node.id.
// create: make one when there is none. Empty when it cannot be read or made.
std::string load_pair_key(bool create) {
    const auto dir = state_dir();
    if (dir.empty()) return {};
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    const auto path = dir / "pair.key";
    {
        std::ifstream in(path);
        std::string line;
        if (in && std::getline(in, line)) {
            const std::string key = howldrop::normalize_pair_key(line);
            if (!key.empty()) return key;
        }
    }
    if (!create) return {};
    const std::string key = howldrop::new_pair_key();
    if (key.empty()) return {};
    std::ofstream out(path, std::ios::trunc);
    if (!out) return {};
    out << key << '\n';
    out.close();
    if (!out) return {};
#ifndef _WIN32
    std::filesystem::permissions(path, std::filesystem::perms::owner_read | std::filesystem::perms::owner_write,
                                 std::filesystem::perm_options::replace, ec);
#endif
    return key;
}

bool save_pair_key(const std::string& key) {
    const auto dir = state_dir();
    if (dir.empty()) return false;
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    std::ofstream out(dir / "pair.key", std::ios::trunc);
    out << key << '\n';
    return static_cast<bool>(out);
}

void usage() {
    std::cout
        << "HowlDrop " << howldrop::kVersion << "\n"
        << "Send a file or clipboard text to another HowlDrop PC on this network.\n"
        << "Ethernet is enough. A Wi-Fi card is not required.\n\n"
        << "  howldrop                 Start (Windows: in the tray, by the clock)\n"
        << "  howldrop serve           Start in this window, with its log\n"
        << "  howldrop serve --share-clipboard   ...and send what you copy to the other PCs\n"
        << "  howldrop peers            List PCs that are currently visible\n"
        << "  howldrop send NAME FILE   Send FILE to the PC named NAME\n"
        << "  howldrop clip             Send the current clipboard text now\n"
        << "  howldrop stop             Stop HowlDrop on this PC\n"
        << "  howldrop iphone [--to NAME] FILE...  AirDrop FILE to the iPhone, through Home Assistant\n"
        << "  howldrop nearby [--scan]  The devices nearby the add-on can send to\n"
        << "  howldrop key              Show this PC's pairing key (the add-on's pc_key)\n"
        << "  howldrop pair KEY         Use another HowlDrop PC's key, so the two can send\n"
        << "                            to each other (then restart HowlDrop)\n\n"
        << "Received files are saved to Downloads\\HowlDrop. Only senders with this PC's\n"
        << "pairing key are accepted. What you copy stays here unless you share it.\n";
}

struct Reply {
    bool ok = false;
    std::string detail;
    std::vector<std::string> lines;
};

std::vector<std::string> split_lines(const std::string& text) {
    std::vector<std::string> lines;
    std::string line;
    for (char c : text) {
        if (c == '\n') {
            if (!line.empty() && line.back() == '\r') line.pop_back();
            lines.push_back(line);
            line.clear();
        } else {
            line.push_back(c);
        }
    }
    if (!line.empty()) lines.push_back(line);
    return lines;
}

Reply control_request(const std::string& command) {
    Reply reply;
    howldrop::Socket socket = howldrop::Socket::tcp_v4();
    std::uint32_t addr = 0;
    std::string error;
    if (!howldrop::ipv4_from_string("127.0.0.1", addr) || !socket.valid() ||
        !socket.connect_v4(addr, howldrop::kControlPort, 1000, error)) {
        reply.detail = "HowlDrop is not running on this PC. Start it first (on Windows, howldrop.exe with no arguments).";
        return reply;
    }
    socket.set_recv_timeout_ms(2000);
    socket.set_send_timeout_ms(2000);
    std::string wire = command;
    wire.push_back('\n');
    if (!socket.send_full(reinterpret_cast<const std::uint8_t*>(wire.data()), wire.size(), nullptr, 3)) {
        reply.detail = "could not talk to the running HowlDrop";
        return reply;
    }
    std::string all;
    int idle = 0;
    char buf[512];
    for (;;) {
        const int n = socket.recv_some(buf, sizeof(buf));
        if (n > 0) {
            all.append(buf, buf + n);
            idle = 0;
            continue;
        }
        if (n == 0) break;
        if (n == -2) {
            if (++idle >= 3) break;
            continue;
        }
        reply.detail = "could not read the reply";
        return reply;
    }
    auto lines = split_lines(all);
    if (lines.empty()) {
        reply.detail = "empty reply";
        return reply;
    }
    const std::string& first = lines[0];
    if (first.rfind("error ", 0) == 0) {
        reply.detail = first.substr(6);
        return reply;
    }
    if (first == "error") {
        reply.detail = "failed";
        return reply;
    }
    if (first.rfind("ok", 0) != 0) {
        reply.detail = first;
        return reply;
    }
    reply.ok = true;
    reply.detail = first.size() > 3 ? first.substr(3) : "";
    if (lines.size() > 1) reply.lines.assign(lines.begin() + 1, lines.end());
    return reply;
}

int cmd_peers() {
    const Reply reply = control_request("peers");
    if (!reply.ok) {
        std::cerr << reply.detail << '\n';
        return 1;
    }
    if (reply.lines.empty()) {
        std::cout << "no other HowlDrop PCs are visible\n";
        return 0;
    }
    for (const auto& line : reply.lines) {
        const auto tab1 = line.find('\t');
        const auto tab2 = tab1 == std::string::npos ? std::string::npos : line.find('\t', tab1 + 1);
        const auto tab3 = tab2 == std::string::npos ? std::string::npos : line.find('\t', tab2 + 1);
        if (tab1 == std::string::npos || tab2 == std::string::npos || tab3 == std::string::npos) {
            std::cout << line << '\n';
            continue;
        }
        const std::string name = line.substr(0, tab1);
        const std::string id = line.substr(tab1 + 1, tab2 - tab1 - 1);
        const std::string ip = line.substr(tab2 + 1, tab3 - tab2 - 1);
        std::cout << name << "  " << ip << "  " << id.substr(0, 8) << '\n';
    }
    return 0;
}

int cmd_send(const std::string& peer, const std::filesystem::path& file) {
    std::error_code ec;
    const auto absolute = std::filesystem::absolute(file, ec);
    if (ec) {
        std::cerr << "could not resolve that path\n";
        return 1;
    }
    const std::string path = howldrop::path_to_utf8(absolute);
    if (path.find('\n') != std::string::npos || path.find('\r') != std::string::npos) {
        std::cerr << "that path cannot be sent\n";
        return 1;
    }
    const Reply reply = control_request("send " + peer + " " + path);
    if (!reply.ok) {
        std::cerr << reply.detail << '\n';
        return 1;
    }
    std::cout << (reply.detail.empty() ? "sent" : reply.detail) << '\n';
    return 0;
}

// howldrop iphone FILE...: the running HowlDrop sends each to the add-on's
// iPhone relay, which AirDrops it. Explorer's Send to runs this with no
// console, so a problem is also said in a message box there.
int cmd_iphone(const std::vector<std::string>& args) {
    std::vector<std::string> problems;
    std::string to;
    std::size_t first = 2;
    if (args.size() >= 4 && args[2] == "--to") {
        to = args[3];
        first = 4;
        if (to.find('\t') != std::string::npos || to.find('\n') != std::string::npos) {
            std::cerr << "that is not a device name\n";
            return 2;
        }
    }
    if (args.size() <= first) problems.push_back("give one or more files to send");
    for (std::size_t i = first; i < args.size(); ++i) {
        std::error_code ec;
        const auto absolute = std::filesystem::absolute(howldrop::path_from_utf8(args[i]), ec);
        const std::string path = ec ? args[i] : howldrop::path_to_utf8(absolute);
        if (path.find('\n') != std::string::npos || path.find('\r') != std::string::npos) {
            problems.push_back(args[i] + ": that path cannot be sent");
            continue;
        }
        const Reply reply = control_request("iphone " + (to.empty() ? std::string() : to + "\t") + path);
        if (reply.ok) {
            std::cout << "sent " << path << " to Home Assistant for the iPhone\n";
        } else {
            problems.push_back(howldrop::path_to_utf8(absolute.filename()) + ": " + reply.detail);
            if (reply.detail.rfind("HowlDrop is not running", 0) == 0) break;
        }
    }
    if (problems.empty()) return 0;
    std::string text;
    for (const auto& p : problems) text += p + "\n";
    std::cerr << text;
#ifdef _WIN32
    if (!GetConsoleWindow()) {
        const auto wide = howldrop::wide_from_utf8(text);
        MessageBoxW(nullptr, wide ? wide->c_str() : L"HowlDrop could not send to the iPhone.",
                    L"HowlDrop: Send to iPhone", MB_OK | MB_ICONWARNING);
    }
#endif
    return 1;
}

// howldrop nearby [--scan]: the devices the add-on's last scan heard (only
// those listening: an iPhone set to Everyone, a Mac with AirDrop open).
int cmd_nearby(const std::vector<std::string>& args) {
    if (args.size() >= 3 && args[2] == "--scan") {
        const Reply scan = control_request("scan");
        if (!scan.ok) {
            std::cerr << scan.detail << '\n';
            return 1;
        }
        std::cout << "looking (about 30 seconds)...\n";
        std::this_thread::sleep_for(std::chrono::seconds(3));
        for (int i = 0; i < 60; ++i) {
            const Reply now = control_request("nearby");
            if (now.ok && now.detail != "scanning") break;
            std::this_thread::sleep_for(std::chrono::seconds(1));
        }
    }
    const Reply reply = control_request("nearby");
    if (!reply.ok) {
        std::cerr << reply.detail << '\n';
        return 1;
    }
    if (reply.lines.empty()) {
        std::cout << "nothing heard yet: set the iPhone's AirDrop to Everyone for 10 Minutes, then howldrop nearby --scan\n";
    }
    for (const auto& name : reply.lines) std::cout << name << '\n';
    if (reply.detail == "scanning") std::cout << "(still looking)\n";
    return 0;
}

int cmd_simple(const std::string& command) {
    const Reply reply = control_request(command);
    if (!reply.ok) {
        std::cerr << reply.detail << '\n';
        return 1;
    }
    if (!reply.detail.empty()) std::cout << reply.detail << '\n';
    return 0;
}

int cmd_serve(bool share_clipboard) {
    if (!howldrop::startup_network()) {
        std::cerr << "could not start networking\n";
        return 1;
    }
    const auto inbox = inbox_dir();
    if (inbox.empty()) {
        std::cerr << "could not find Downloads\n";
        return 1;
    }
    auto clipboard = howldrop::make_system_clipboard();
    howldrop::Node::Config config;
    config.name = computer_name();
    config.id = load_id();
    config.inbox = inbox;
    config.pair_key = load_pair_key(true);
    config.share_clipboard = share_clipboard;
    if (config.pair_key.empty()) {
        std::cerr << "could not read or make the pairing key in " << howldrop::path_to_utf8(state_dir()) << '\n';
        return 1;
    }
    howldrop::Node node(std::move(config), *clipboard);
    try {
        node.start();
    } catch (const std::exception& ex) {
        std::cerr << ex.what() << '\n';
        return 1;
    }
#ifdef _WIN32
    SetConsoleCtrlHandler(console_handler, TRUE);
#else
    std::signal(SIGINT, on_signal);
    std::signal(SIGTERM, on_signal);
    std::signal(SIGPIPE, SIG_IGN);
#endif
    while (!g_stop && node.running()) std::this_thread::sleep_for(std::chrono::milliseconds(200));
    node.request_stop();
    node.stop();
    std::cout << "stopped\n";
    return 0;
}

#ifdef _WIN32
// The tray app: the same node as `serve`, without a console window.
int cmd_tray() {
    const auto inbox = inbox_dir();
    if (inbox.empty()) {
        MessageBoxW(nullptr, L"HowlDrop could not find your Downloads folder.", L"HowlDrop", MB_OK | MB_ICONERROR);
        return 1;
    }
    howldrop::Node::Config config;
    config.name = computer_name();
    config.id = load_id();
    config.inbox = inbox;
    config.pair_key = load_pair_key(true);
    if (config.pair_key.empty()) {
        MessageBoxW(nullptr, L"HowlDrop could not read or make its pairing key.", L"HowlDrop", MB_OK | MB_ICONERROR);
        return 1;
    }
    return howldrop::run_tray(std::move(config));
}

// howldrop.exe is a windowed program, so a command typed in a terminal
// writes to that terminal by attaching to it; `serve` opens one if needed.
// Output already sent to a pipe or a file (howldrop peers > out.txt) stays
// there.
void attach_console(bool create) {
    const auto redirected = [](DWORD which) {
        const HANDLE handle = GetStdHandle(which);
        return handle != nullptr && handle != INVALID_HANDLE_VALUE && GetFileType(handle) != FILE_TYPE_UNKNOWN;
    };
    const bool out = redirected(STD_OUTPUT_HANDLE);
    const bool err = redirected(STD_ERROR_HANDLE);
    if (out && err) return;
    if (!AttachConsole(ATTACH_PARENT_PROCESS) && !(create && AllocConsole())) return;
    FILE* ignored = nullptr;
    if (!out) freopen_s(&ignored, "CONOUT$", "w", stdout);
    if (!err) freopen_s(&ignored, "CONOUT$", "w", stderr);
    std::ios::sync_with_stdio(true);
}
#endif

}  // namespace

#ifdef _WIN32
int main() {
    int argc = 0;
    LPWSTR* wargv = CommandLineToArgvW(GetCommandLineW(), &argc);
    if (!wargv) return 1;
    std::vector<std::string> args;
    args.reserve(static_cast<std::size_t>(argc));
    for (int i = 0; i < argc; ++i) args.push_back(howldrop::utf8_from_wide(wargv[i] ? wargv[i] : L""));
    LocalFree(wargv);
#else
int main(int argc, char** argv) {
    std::vector<std::string> args;
    args.reserve(static_cast<std::size_t>(argc));
    for (int i = 0; i < argc; ++i) args.emplace_back(argv[i] ? argv[i] : "");
#endif
    if (!howldrop::startup_network()) {
        std::cerr << "could not start networking\n";
        return 1;
    }
#ifdef _WIN32
    if (args.size() < 2) return cmd_tray();
    attach_console(args[1] == "serve");
#endif
    const std::string cmd = args.size() >= 2 ? args[1] : "serve";
    if (cmd == "help" || cmd == "--help" || cmd == "-h") {
        usage();
        return 0;
    }
    try {
        if (cmd == "serve") return cmd_serve(args.size() >= 3 && args[2] == "--share-clipboard");
        if (cmd == "key") {
            const std::string key = load_pair_key(true);
            if (key.empty()) {
                std::cerr << "could not read or make the pairing key\n";
                return 1;
            }
            std::cout << howldrop::display_pair_key(key) << '\n';
            return 0;
        }
        if (cmd == "pair") {
            const std::string key = args.size() >= 3 ? howldrop::normalize_pair_key(args[2]) : std::string();
            if (key.empty()) {
                std::cerr << "that is not a pairing key: give the 32 digits `howldrop key` shows on the other PC\n";
                return 2;
            }
            if (!save_pair_key(key)) {
                std::cerr << "could not save the pairing key\n";
                return 1;
            }
            std::cout << "paired: restart HowlDrop to use it. The add-on's pc_key must be this key too.\n";
            return 0;
        }
        if (cmd == "iphone") return cmd_iphone(args);
        if (cmd == "nearby") return cmd_nearby(args);
#ifdef _WIN32
        // The README's pictures of the window and the cards, made-up data.
        if (cmd == "screenshots") return howldrop::make_screenshots(howldrop::path_from_utf8(args.size() >= 3 ? args[2] : "."));
#endif
        if (cmd == "peers") return cmd_peers();
        if (cmd == "clip") return cmd_simple("clip");
        if (cmd == "stop") return cmd_simple("quit");
        if (cmd == "send") {
            if (args.size() < 4) {
                usage();
                return 2;
            }
            std::string path = args[3];
            for (std::size_t i = 4; i < args.size(); ++i) {
                path.push_back(' ');
                path += args[i];
            }
            return cmd_send(args[2], howldrop::path_from_utf8(path));
        }
    } catch (const std::exception& ex) {
        std::cerr << ex.what() << '\n';
        return 1;
    }
    usage();
    return 2;
}
