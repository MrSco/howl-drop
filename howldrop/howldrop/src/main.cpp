#include "node.hpp"

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

void usage() {
    std::cout
        << "HowlDrop " << howldrop::kVersion << "\n"
        << "Send a file or clipboard text to another HowlDrop PC on this network.\n"
        << "Ethernet is enough. A Wi-Fi card is not required.\n\n"
        << "  howldrop                 Start (Windows: in the tray, by the clock)\n"
        << "  howldrop serve           Start in this window, with its log\n"
        << "  howldrop peers            List PCs that are currently visible\n"
        << "  howldrop send NAME FILE   Send FILE to the PC named NAME\n"
        << "  howldrop clip             Send the current clipboard text now\n"
        << "  howldrop stop             Stop HowlDrop on this PC\n\n"
        << "Received files are saved to Downloads\\HowlDrop.\n"
        << "Text you copy while HowlDrop is running is sent to those PCs.\n";
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

int cmd_simple(const std::string& command) {
    const Reply reply = control_request(command);
    if (!reply.ok) {
        std::cerr << reply.detail << '\n';
        return 1;
    }
    if (!reply.detail.empty()) std::cout << reply.detail << '\n';
    return 0;
}

int cmd_serve() {
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
        if (cmd == "serve") return cmd_serve();
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
