// AirDrop on this PC (wsl_win.hpp): setting up WSL's side once, then running
// the radio stack in it as a child process.

#include "wsl_win.hpp"

#include "ble_wake_win.hpp"
#include "config.hpp"
#include "pairing.hpp"
#include "text.hpp"
#include "wsl_text.hpp"

#include <windows.h>
#include <roapi.h>
#include <shellapi.h>
#include <winhttp.h>

#include <algorithm>
#include <chrono>
#include <cwchar>
#include <fstream>

// A release build names the Linux part it goes with (CMake: HOWLDROP_LINUX_*,
// set by .github/workflows/release-windows.yml). A local build has none and
// uses a HowlDrop distro already imported, whatever its version.
#ifndef HOWLDROP_LINUX_URL
#define HOWLDROP_LINUX_URL ""
#endif
#ifndef HOWLDROP_LINUX_SHA256
#define HOWLDROP_LINUX_SHA256 ""
#endif
#ifndef HOWLDROP_LINUX_VERSION
#define HOWLDROP_LINUX_VERSION ""
#endif

namespace howldrop {

namespace fs = std::filesystem;

namespace {

const std::string kLinuxUrl = HOWLDROP_LINUX_URL;
const std::string kLinuxSha256 = HOWLDROP_LINUX_SHA256;
const std::string kLinuxVersion = HOWLDROP_LINUX_VERSION;

// One child is created at a time, so the pipe ends a child inherits are its
// own: an inheritable handle goes to every child made while it is open.
std::mutex g_spawn_mu;

std::wstring wide(const std::string& s) {
    auto w = wide_from_utf8(s);
    return w ? *w : std::wstring();
}

std::string trim(std::string s) {
    while (!s.empty() && (s.back() == '\r' || s.back() == '\n' || s.back() == ' ')) s.pop_back();
    std::size_t a = 0;
    while (a < s.size() && s[a] == ' ') ++a;
    return s.substr(a);
}

// One argument on a Windows command line, by CommandLineToArgvW's rules.
std::wstring quote(const std::wstring& arg) {
    if (!arg.empty() && arg.find_first_of(L" \t\"") == std::wstring::npos) return arg;
    std::wstring out = L"\"";
    std::size_t slashes = 0;
    for (wchar_t c : arg) {
        if (c == L'\\') {
            ++slashes;
            continue;
        }
        if (c == L'"') {
            out.append(slashes * 2 + 1, L'\\');
        } else {
            out.append(slashes, L'\\');
        }
        out.push_back(c);
        slashes = 0;
    }
    out.append(slashes * 2, L'\\');
    out.push_back(L'"');
    return out;
}

fs::path wsl_exe() {
    wchar_t dir[MAX_PATH];
    const UINT n = GetSystemDirectoryW(dir, MAX_PATH);
    return (n > 0 && n < MAX_PATH) ? fs::path(dir) / L"wsl.exe" : fs::path(L"wsl.exe");
}

fs::path find_usbipd() {
    wchar_t buf[MAX_PATH];
    const DWORD n = GetEnvironmentVariableW(L"ProgramFiles", buf, MAX_PATH);
    std::error_code ec;
    if (n > 0 && n < MAX_PATH) {
        const fs::path p = fs::path(buf) / L"usbipd-win" / L"usbipd.exe";
        if (fs::exists(p, ec)) return p;
    }
    wchar_t found[MAX_PATH];
    if (SearchPathW(nullptr, L"usbipd.exe", nullptr, MAX_PATH, found, nullptr) > 0) return fs::path(found);
    return {};
}

// wsl.exe arguments to run a command in the distro as root, without a shell.
std::vector<std::wstring> in_distro(const std::vector<std::wstring>& command) {
    std::vector<std::wstring> args = {L"-d", wide(wsl::kDistro), L"-u", L"root", L"-e"};
    args.insert(args.end(), command.begin(), command.end());
    return args;
}

}  // namespace

// A child process with its output (stdout and stderr) on a pipe, or in a
// file, or in a console window of its own.
struct ChildProcess {
    PROCESS_INFORMATION pi{};
    HANDLE out = nullptr;
    std::string pending;

    ChildProcess() = default;
    ChildProcess(const ChildProcess&) = delete;
    ChildProcess& operator=(const ChildProcess&) = delete;
    ~ChildProcess() {
        if (out) CloseHandle(out);
        if (pi.hThread) CloseHandle(pi.hThread);
        if (pi.hProcess) CloseHandle(pi.hProcess);
    }

    // log_file empty: output on the pipe (read_line). visible: a console of
    // its own, for a program that may ask the user something (winget).
    bool start(const fs::path& exe, const std::vector<std::wstring>& args, std::string& error,
               const fs::path& log_file = {}, bool visible = false) {
        std::wstring line = quote(exe.wstring());
        for (const auto& a : args) line += L" " + quote(a);
        std::vector<wchar_t> buffer(line.begin(), line.end());
        buffer.push_back(L'\0');

        std::lock_guard<std::mutex> hold(g_spawn_mu);
        SECURITY_ATTRIBUTES sa{sizeof(sa), nullptr, TRUE};
        HANDLE write = nullptr;
        HANDLE input = nullptr;
        STARTUPINFOW si{};
        si.cb = sizeof(si);
        DWORD flags = CREATE_UNICODE_ENVIRONMENT;
        if (visible) {
            flags |= CREATE_NEW_CONSOLE;
        } else {
            flags |= CREATE_NO_WINDOW;
            if (!log_file.empty()) {
                write = CreateFileW(log_file.c_str(), FILE_APPEND_DATA, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa,
                                    OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, nullptr);
                if (write == INVALID_HANDLE_VALUE) write = nullptr;
            } else {
                HANDLE read = nullptr;
                if (!CreatePipe(&read, &write, &sa, 0)) {
                    error = "could not make a pipe";
                    return false;
                }
                SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0);
                out = read;
            }
            input = CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0,
                                nullptr);
            if (input == INVALID_HANDLE_VALUE) input = nullptr;
            si.dwFlags = STARTF_USESTDHANDLES;
            si.hStdInput = input;
            si.hStdOutput = write;
            si.hStdError = write;
        }
        const BOOL ok = CreateProcessW(nullptr, buffer.data(), nullptr, nullptr, visible ? FALSE : TRUE, flags,
                                       nullptr, nullptr, &si, &pi);
        const DWORD code = GetLastError();
        if (write) CloseHandle(write);
        if (input) CloseHandle(input);
        if (!ok) {
            error = "could not run " + path_to_utf8(exe.filename()) + " (error " + std::to_string(code) + ")";
            return false;
        }
        return true;
    }

    // The next line of output; false once the child has closed it.
    bool read_line(std::string& line) {
        for (;;) {
            const auto nl = pending.find('\n');
            if (nl != std::string::npos) {
                line = pending.substr(0, nl);
                if (!line.empty() && line.back() == '\r') line.pop_back();
                pending.erase(0, nl + 1);
                return true;
            }
            char buf[4096];
            DWORD n = 0;
            if (!out || !ReadFile(out, buf, sizeof(buf), &n, nullptr) || n == 0) {
                if (pending.empty()) return false;
                line.swap(pending);
                pending.clear();
                return true;
            }
            pending.append(buf, n);
        }
    }

    // Everything it prints until it ends, or false after timeout_ms.
    bool read_all(std::string& output, DWORD timeout_ms) {
        const ULONGLONG until = GetTickCount64() + timeout_ms;
        for (;;) {
            DWORD avail = 0;
            if (out && PeekNamedPipe(out, nullptr, 0, nullptr, &avail, nullptr) && avail > 0) {
                char buf[4096];
                DWORD n = 0;
                if (ReadFile(out, buf, sizeof(buf), &n, nullptr) && n > 0) output.append(buf, n);
                continue;
            }
            if (wait(50)) {
                // Ended: what is left in the pipe, then done.
                while (out && PeekNamedPipe(out, nullptr, 0, nullptr, &avail, nullptr) && avail > 0) {
                    char buf[4096];
                    DWORD n = 0;
                    if (!ReadFile(out, buf, sizeof(buf), &n, nullptr) || n == 0) break;
                    output.append(buf, n);
                }
                return true;
            }
            if (GetTickCount64() > until) {
                kill();
                return false;
            }
        }
    }

    bool wait(DWORD ms) const { return pi.hProcess && WaitForSingleObject(pi.hProcess, ms) == WAIT_OBJECT_0; }
    DWORD exit_code() const {
        DWORD code = 1;
        if (pi.hProcess) GetExitCodeProcess(pi.hProcess, &code);
        return code;
    }
    void kill() {
        if (pi.hProcess && !wait(0)) TerminateProcess(pi.hProcess, 1);
    }
};

namespace {

// Runs a command to its end and returns its exit code: -1 when it could not
// run, or ran longer than timeout_ms. output gets what it printed.
int run_command(const fs::path& exe, const std::vector<std::wstring>& args, std::string& output,
                DWORD timeout_ms = 60000) {
    ChildProcess child;
    std::string error;
    if (!child.start(exe, args, error)) {
        output = error;
        return -1;
    }
    if (!child.read_all(output, timeout_ms)) return -1;
    return static_cast<int>(child.exit_code());
}

// usbipd bind needs an administrator: Windows asks the user (UAC).
int run_elevated(const fs::path& exe, const std::wstring& params) {
    SHELLEXECUTEINFOW info{};
    info.cbSize = sizeof(info);
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = L"runas";
    const std::wstring file = exe.wstring();
    info.lpFile = file.c_str();
    info.lpParameters = params.c_str();
    info.nShow = SW_HIDE;
    if (!ShellExecuteExW(&info) || !info.hProcess) return -1;  // cancelled, most likely
    WaitForSingleObject(info.hProcess, 120000);
    DWORD code = 1;
    GetExitCodeProcess(info.hProcess, &code);
    CloseHandle(info.hProcess);
    return static_cast<int>(code);
}

std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(c >= 'A' && c <= 'Z' ? c - 'A' + 'a' : c);
    return s;
}

}  // namespace

// --- the radio ---------------------------------------------------------------------

WslRadio::WslRadio(WslRadioConfig config) : config_(std::move(config)) {
    outbox_ = config_.home / L"outbox";
    scan_request_ = outbox_ / L".scan-request";
    asks_ = config_.home / L"asks";
    // wsl.exe's own messages (and --list) in UTF-8, not UTF-16.
    SetEnvironmentVariableW(L"WSL_UTF8", L"1");
}

WslRadio::~WslRadio() { stop(); }

void WslRadio::start() {
    if (thread_.joinable()) return;
    stop_.store(false);
    {
        std::lock_guard<std::mutex> hold(wake_mu_);
        wake_quit_ = false;
    }
    wake_thread_ = std::thread([this]() { wake_loop(); });
    thread_ = std::thread([this]() { run(); });
}

void WslRadio::wake_loop() {
    const HRESULT ro = RoInitialize(RO_INIT_MULTITHREADED);
    BleWake wake;
    std::string last_error;
    std::unique_lock<std::mutex> lock(wake_mu_);
    while (!wake_quit_) {
        const bool want = std::chrono::steady_clock::now() < wake_until_;
        lock.unlock();
        if (want && !wake.on()) {
            const std::string error = wake.start();
            if (error.empty()) {
                log("Bluetooth wake on");
                last_error.clear();
            } else if (error != last_error) {
                last_error = error;
                log("Bluetooth wake: " + error);
                state("On the share sheet as " + config_.name + ". Sending to the iPhone needs Bluetooth: " + error,
                      RadioLevel::Up);
            }
        } else if (!want && wake.on()) {
            wake.stop();
            log("Bluetooth wake off");
        }
        lock.lock();
        wake_cv_.wait_for(lock, std::chrono::seconds(want && !wake.on() ? 5 : 1));
    }
    lock.unlock();
    wake.stop();
    if (SUCCEEDED(ro)) RoUninitialize();
}

void WslRadio::hold_wake(int seconds) {
    std::lock_guard<std::mutex> hold(wake_mu_);
    const auto until = std::chrono::steady_clock::now() + std::chrono::seconds(seconds);
    if (until > wake_until_) wake_until_ = until;
    wake_cv_.notify_all();
}

// A send or a scan finished: the wake stays a moment for any left.
void WslRadio::done_waking() {
    std::lock_guard<std::mutex> hold(wake_mu_);
    if (sends_pending_ > 0) return;
    const auto soon = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    if (soon < wake_until_) wake_until_ = soon;
    wake_cv_.notify_all();
}

void WslRadio::stop() {
    stop_.store(true);
    std::shared_ptr<ChildProcess> stack;
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        stack = stack_;
    }
    if (stack && !stack->wait(0)) {
        // run.sh's own stop: OWL goes, the card goes back to managed mode.
        std::string out;
        run_command(wsl_exe(), in_distro({L"/wsl-run.sh", L"--stop"}), out, 40000);
        if (!stack->wait(10000)) stack->kill();
    }
    if (thread_.joinable()) thread_.join();
    {
        std::lock_guard<std::mutex> hold(wake_mu_);
        wake_quit_ = true;
        wake_cv_.notify_all();
    }
    if (wake_thread_.joinable()) wake_thread_.join();
}

void WslRadio::pause(int ms) {
    for (int waited = 0; waited < ms && !stopping(); waited += 100) Sleep(100);
}

void WslRadio::log(const std::string& line) {
    std::lock_guard<std::mutex> hold(log_mu_);
    std::ofstream out(config_.home / L"radio.log", std::ios::app | std::ios::binary);
    out << line << '\n';
}

void WslRadio::state(const std::string& text, RadioLevel level) {
    log("state: " + text);
    if (config_.on_state) config_.on_state(text, level);
}

void WslRadio::run() {
    {
        std::lock_guard<std::mutex> hold(log_mu_);
        std::ofstream(config_.home / L"radio.log", std::ios::trunc | std::ios::binary)
            << "HowlDrop " << kVersion << ": AirDrop on this PC\n";
    }
    std::string error;
    std::string busid;
    if (!prepare(error) || !find_stick(busid, error)) {
        state(stopping() ? "Off" : error, stopping() ? RadioLevel::Off : RadioLevel::Problem);
        return;
    }
    run_stack(busid);
    give_back(busid);
    up_.store(false);
    if (config_.on_up) config_.on_up(false);
    std::string out;
    run_command(wsl_exe(), {L"--terminate", wide(wsl::kDistro)}, out, 30000);
    if (stopping()) state("Off", RadioLevel::Off);
}

bool WslRadio::prepare(std::string& error) {
    std::error_code ec;
    fs::create_directories(config_.home, ec);
    fs::create_directories(outbox_, ec);
    fs::create_directories(config_.inbox, ec);
    if (!wsl::wsl_path(path_to_utf8(config_.inbox)) || !wsl::wsl_path(path_to_utf8(outbox_))) {
        error = "The HowlDrop folder must be on a drive of this PC (it is " + path_to_utf8(config_.inbox) + ").";
        return false;
    }

    state("Checking WSL");
    std::string out;
    if (run_command(wsl_exe(), {L"--version"}, out, 30000) != 0) {
        error = "WSL isn't installed. Open a terminal as administrator, run: wsl --install, restart Windows, "
                "then turn AirDrop on this PC on again.";
        return false;
    }

    usbipd_ = find_usbipd();
    if (usbipd_.empty()) {
        const bool yes = config_.ask &&
                         config_.ask(L"AirDrop on this PC hands the USB Wi-Fi stick to WSL with usbipd-win, which "
                                     L"isn't installed.\n\nInstall it now with winget? A window shows the install, "
                                     L"and Windows asks for permission.");
        if (!yes) {
            error = "Needs usbipd-win: install it (winget install usbipd), then turn this on again.";
            return false;
        }
        state("Installing usbipd-win");
        ChildProcess winget;
        if (winget.start(L"winget.exe", {L"install", L"--exact", L"--id", L"dorssel.usbipd-win"}, error, {}, true)) {
            while (!winget.wait(500) && !stopping()) {
            }
        }
        usbipd_ = find_usbipd();
        if (usbipd_.empty()) {
            error = "usbipd-win didn't install. Install it (winget install usbipd), then turn this on again.";
            return false;
        }
    }
    return ensure_distro(error);
}

bool WslRadio::ensure_distro(std::string& error) {
    std::string out;
    run_command(wsl_exe(), {L"--list", L"--quiet"}, out, 30000);
    const auto names = wsl::parse_distros(out);
    bool have = std::find(names.begin(), names.end(), wsl::kDistro) != names.end();
    if (have && !kLinuxVersion.empty()) {
        std::string version;
        run_command(wsl_exe(), in_distro({L"cat", L"/etc/howldrop-version"}), version, 60000);
        if (trim(version) != kLinuxVersion) {
            state("Updating the Linux part to " + kLinuxVersion);
            log("installed Linux part: " + trim(version) + "; this howldrop.exe goes with " + kLinuxVersion);
            run_command(wsl_exe(), {L"--terminate", wide(wsl::kDistro)}, out, 30000);
            run_command(wsl_exe(), {L"--unregister", wide(wsl::kDistro)}, out, 120000);
            have = false;
        }
    }
    if (have) return true;
    if (kLinuxUrl.empty()) {
        error = "This howldrop.exe has no Linux part to download (a local build). Use a release from GitHub.";
        return false;
    }
    const fs::path tar = config_.home / L"howldrop-linux.tar.gz";
    if (!download_linux(tar, error)) return false;
    state("Installing the Linux part (a minute or two)");
    std::error_code ec;
    fs::create_directories(config_.home / L"wsl", ec);
    const int code = run_command(wsl_exe(),
                                 {L"--import", wide(wsl::kDistro), (config_.home / L"wsl").wstring(), tar.wstring(),
                                  L"--version", L"2"},
                                 out, 15 * 60 * 1000);
    fs::remove(tar, ec);
    if (code != 0) {
        log("wsl --import: " + out);
        error = "Installing the Linux part failed: " + trim(out);
        return false;
    }
    return true;
}

bool WslRadio::download_linux(const fs::path& tar, std::string& error) {
    state("Downloading the Linux part");
    const std::wstring url = wide(kLinuxUrl);
    URL_COMPONENTSW parts{};
    parts.dwStructSize = sizeof(parts);
    wchar_t host[256];
    wchar_t path[2048];
    parts.lpszHostName = host;
    parts.dwHostNameLength = 256;
    parts.lpszUrlPath = path;
    parts.dwUrlPathLength = 2048;
    if (!WinHttpCrackUrl(url.c_str(), 0, 0, &parts) || parts.nScheme != INTERNET_SCHEME_HTTPS) {
        error = "The Linux part's address is not https: " + kLinuxUrl;
        return false;
    }
    const std::wstring agent = L"HowlDrop/" + wide(kVersion);
    HINTERNET session = WinHttpOpen(agent.c_str(), WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_NO_PROXY_NAME,
                                    WINHTTP_NO_PROXY_BYPASS, 0);
    HINTERNET connection = session ? WinHttpConnect(session, host, parts.nPort, 0) : nullptr;
    HINTERNET request = connection ? WinHttpOpenRequest(connection, L"GET", path, nullptr, WINHTTP_NO_REFERER,
                                                        WINHTTP_DEFAULT_ACCEPT_TYPES, WINHTTP_FLAG_SECURE)
                                   : nullptr;
    const auto close_all = [&]() {
        if (request) WinHttpCloseHandle(request);
        if (connection) WinHttpCloseHandle(connection);
        if (session) WinHttpCloseHandle(session);
    };
    DWORD status = 0;
    DWORD size = sizeof(status);
    if (!request || !WinHttpSendRequest(request, WINHTTP_NO_ADDITIONAL_HEADERS, 0, WINHTTP_NO_REQUEST_DATA, 0, 0, 0) ||
        !WinHttpReceiveResponse(request, nullptr) ||
        !WinHttpQueryHeaders(request, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_HEADER_NAME_BY_INDEX,
                             &status, &size, WINHTTP_NO_HEADER_INDEX) ||
        status != 200) {
        error = "Couldn't download the Linux part (" +
                (status ? "HTTP " + std::to_string(status) : "error " + std::to_string(GetLastError())) +
                "). Check the connection and turn this on again.";
        close_all();
        return false;
    }
    unsigned long long total = 0;
    wchar_t length[32];
    DWORD length_size = sizeof(length);
    if (WinHttpQueryHeaders(request, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_HEADER_NAME_BY_INDEX, length, &length_size,
                            WINHTTP_NO_HEADER_INDEX)) {
        total = std::wcstoull(length, nullptr, 10);
    }

    const fs::path part = tar.wstring() + L".part";
    std::ofstream file(part, std::ios::binary | std::ios::trunc);
    Sha256 sha;
    unsigned long long got = 0;
    int shown = -1;
    std::vector<char> buf(256 * 1024);
    bool ok = static_cast<bool>(file);
    while (ok && !stopping()) {
        DWORD n = 0;
        if (!WinHttpReadData(request, buf.data(), static_cast<DWORD>(buf.size()), &n)) {
            ok = false;
            break;
        }
        if (n == 0) break;
        file.write(buf.data(), n);
        sha.update(buf.data(), n);
        got += n;
        const int percent = total ? static_cast<int>(got * 100 / total) : -1;
        if (percent != shown && percent >= 0) {
            shown = percent;
            state("Downloading the Linux part: " + std::to_string(percent) + "% of " +
                  std::to_string(total / (1024 * 1024)) + " MB");
        }
    }
    close_all();
    file.close();
    std::error_code ec;
    if (!ok || stopping() || !file) {
        fs::remove(part, ec);
        if (!stopping()) error = "The download of the Linux part broke off. Turn this on again to retry.";
        return false;
    }
    const std::string digest = to_hex(sha.finish());
    if (lower(digest) != lower(kLinuxSha256)) {
        fs::remove(part, ec);
        log("download hash " + digest + ", expected " + kLinuxSha256);
        error = "The downloaded Linux part isn't the one this howldrop.exe expects (its SHA-256 differs), so it "
                "was deleted. Turn this on again to retry.";
        return false;
    }
    fs::rename(part, tar, ec);
    if (ec) {
        error = "Couldn't keep the download: " + ec.message();
        return false;
    }
    return true;
}

bool WslRadio::find_stick(std::string& busid, std::string& error) {
    bool asked = false;
    while (!stopping()) {
        std::string out;
        run_command(usbipd_, {L"list"}, out, 30000);
        const auto stick = wsl::find_stick(wsl::parse_usbipd_list(out));
        if (!stick) {
            state("Plug in the MT7612U Wi-Fi stick", RadioLevel::Problem);
            pause(3000);
            continue;
        }
        busid = stick->busid;
        if (stick->shared()) return true;
        if (asked) {
            error = "The stick isn't shared with WSL (usbipd bind was cancelled). Turn this on again to retry.";
            return false;
        }
        // Once per stick and port: usbipd remembers.
        state("Sharing the stick with WSL (Windows asks for permission)");
        const int code = run_elevated(usbipd_, L"bind --busid " + wide(busid));
        log("usbipd bind --busid " + busid + ": exit " + std::to_string(code));
        asked = true;
    }
    return false;
}

void WslRadio::run_stack(const std::string& busid) {
    const auto save = wsl::wsl_path(path_to_utf8(config_.inbox));
    const auto outbox = wsl::wsl_path(path_to_utf8(outbox_));
    const auto scan = wsl::wsl_path(path_to_utf8(scan_request_));
    const auto asks = wsl::wsl_path(path_to_utf8(asks_));
    std::error_code ec;
    fs::remove(scan_request_, ec);
    fs::create_directories(asks_, ec);
    if (stopping()) return;
    state("Starting");
    auto stack = std::make_shared<ChildProcess>();
    std::string error;
    if (!stack->start(wsl_exe(),
                      in_distro({L"/wsl-run.sh", L"--name", wide(config_.name), L"--save", wide(*save), L"--outbox",
                                 wide(*outbox), L"--scan-request", wide(*scan), L"--receive", wide(config_.receive),
                                 L"--senders", wide(config_.senders), L"--ask-dir", wide(asks.value_or(""))}),
                      error)) {
        state("Couldn't start the Linux part: " + error, RadioLevel::Problem);
        return;
    }
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        stack_ = stack;
    }
    if (stopping()) {  // stop() came while it started, and did not see it
        std::string out;
        run_command(wsl_exe(), in_distro({L"/wsl-run.sh", L"--stop"}), out, 40000);
    }
    std::string line;
    while (stack->read_line(line)) handle_line(line, busid);
    stack->wait(INFINITE);
    log("the Linux part ended (exit " + std::to_string(stack->exit_code()) + ")");
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        stack_.reset();
    }
    if (!stopping()) {
        const bool failed_stick = last_line_.rfind("stick: ", 0) == 0;
        state(failed_stick ? last_line_.substr(7)
                           : "Stopped" + (last_line_.empty() ? std::string() : ": " + last_line_) +
                                 ". Turn it off and on to retry.",
              RadioLevel::Problem);
    }
}

void WslRadio::handle_line(const std::string& line, const std::string& busid) {
    log(line);
    const auto event = wsl::event_fields(line);
    // outbox.py starts on a file: its window (180 s) is when the phone must hear the wake.
    if (line.rfind("Send: sending ", 0) == 0) hold_wake(200);
    if (!event) {
        const std::string t = trim(line);
        if (!t.empty() && t.rfind("OWL:", 0) != 0 && t.rfind("Traceback", 0) != 0 && line[0] != ' ') {
            last_line_ = t;
        }
        return;
    }
    const auto& f = *event;
    if (f.empty()) return;
    const std::string& what = f[0];
    const auto field = [&](std::size_t i) { return i < f.size() ? f[i] : std::string(); };
    if (what == "stick") {
        if (field(1) == "waiting") {
            state("Handing the stick to WSL");
            attach(busid);
        } else if (field(1) == "ready") {
            state("Starting the radio");
        } else if (field(1) == "failed") {
            last_line_ = "stick: " + field(2);
            state(field(2), RadioLevel::Problem);
        }
    } else if (what == "up") {
        up_.store(true);
        state("On the share sheet as " + config_.name, RadioLevel::Up);
        if (config_.on_up) config_.on_up(true);
    } else if (what == "received") {
        const std::string rel = field(1);
        if (rel.empty() || rel.find("..") != std::string::npos || rel[0] == '/') return;
        if (config_.on_received) config_.on_received(config_.inbox / path_from_utf8(rel), field(2), field(3));
    } else if (what == "sent") {
        {
            std::lock_guard<std::mutex> hold(wake_mu_);
            if (sends_pending_ > 0) --sends_pending_;
        }
        done_waking();
        if (config_.on_sent) {
            config_.on_sent(field(1), field(2) == "sent" ? Status::success(field(3)) : Status::failure(field(3)));
        }
    } else if (what == "ask" || what == "asked" || what == "incoming" || what == "receiving" || what == "stored" ||
               what == "failed" || what == "sending") {
        if (config_.on_event) config_.on_event(f);
    } else if (what == "nearby") {
        bool done = false;
        {
            std::lock_guard<std::mutex> hold(nearby_mu_);
            scanning_ = field(1) == "scanning";
            done = !scanning_;
            if (!scanning_ || f.size() > 2) devices_.assign(f.begin() + 2, f.end());
        }
        if (done) done_waking();
    }
}

// Once, through the HowlDrop distro (it has modprobe for vhci-hcd). Not
// --auto-attach: its re-attach loop runs inside WSL and outlives usbipd.exe,
// so it would take the stick back after give_back detached it.
void WslRadio::attach(const std::string& busid) {
    std::string out;
    const int code =
        run_command(usbipd_, {L"attach", L"--wsl", wide(wsl::kDistro), L"--busid", wide(busid)}, out, 60000);
    log("usbipd attach: exit " + std::to_string(code) + ": " + trim(out));
}

void WslRadio::give_back(const std::string& busid) {
    std::string out;
    run_command(usbipd_, {L"detach", L"--busid", wide(busid)}, out, 30000);
    log("usbipd detach: " + trim(out));
}

void WslRadio::decide(const std::string& id, bool accept) {
    // An id is howl_ask.new_id()'s hex; anything else names no ask.
    if (id.empty() || id.size() > 32 || id.find_first_not_of("0123456789abcdef") != std::string::npos) return;
    std::error_code ec;
    fs::create_directories(asks_, ec);
    std::ofstream(asks_ / (id + (accept ? ".accept" : ".decline")), std::ios::binary) << '\n';
    log(std::string("ask ") + id + (accept ? ": accepted on the PC" : ": declined on the PC"));
}

// --- sending to the iPhone (Node::LocalIphone) ----------------------------------------

Status WslRadio::queue(const fs::path& path, const std::string& to) {
    if (!up_.load()) return Status::failure("AirDrop on this PC isn't on yet");
    std::error_code ec;
    if (!fs::is_regular_file(path, ec)) return Status::failure("that path is not a file");
    fs::create_directories(outbox_, ec);
    // outbox.py takes a file once it stops growing, and skips *.partial; the
    // device it is for goes beside it first (NAME.howldrop-to).
    const fs::path name = path.filename();
    fs::path dest = outbox_ / name;
    for (int n = 1; fs::exists(dest, ec) || fs::exists(dest.wstring() + L".partial", ec); ++n) {
        dest = outbox_ / (name.stem().wstring() + L"-" + std::to_wstring(n) + name.extension().wstring());
    }
    if (!to.empty()) {
        std::ofstream(fs::path(dest.wstring() + L".howldrop-to"), std::ios::binary) << to << '\n';
    }
    const fs::path partial = dest.wstring() + L".partial";
    if (!fs::copy_file(path, partial, fs::copy_options::overwrite_existing, ec)) {
        fs::remove(dest.wstring() + L".howldrop-to", ec);
        return Status::failure("couldn't copy it for sending: " + ec.message());
    }
    fs::rename(partial, dest, ec);
    if (ec) return Status::failure("couldn't hand it over: " + ec.message());
    {
        std::lock_guard<std::mutex> hold(wake_mu_);
        ++sends_pending_;
    }
    hold_wake(200);
    return Status::success("handed to AirDrop on this PC");
}

Node::Nearby WslRadio::nearby() const {
    std::lock_guard<std::mutex> hold(nearby_mu_);
    Node::Nearby out;
    out.relay = up_.load();
    out.local = true;
    out.scanning = scanning_;
    out.devices = devices_;
    return out;
}

Status WslRadio::scan() {
    if (!up_.load()) return Status::failure("AirDrop on this PC isn't on yet");
    hold_wake(150);  // up to three tries (outbox.py scan); the phone only answers once woken
    std::ofstream(scan_request_, std::ios::binary) << "scan\n";
    {
        std::lock_guard<std::mutex> hold(nearby_mu_);
        scanning_ = true;
    }
    return Status::success("looking for nearby devices");
}

}  // namespace howldrop
