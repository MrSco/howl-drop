// AirDrop on this PC (radio_win.hpp): howldrop-agent.exe, run as the user
// without a window, read through a named pipe.

#include "radio_win.hpp"

#include "ble_wake_win.hpp"
#include "config.hpp"
#include "pairing.hpp"
#include "radio_text.hpp"
#include "text.hpp"

#include <windows.h>
#include <roapi.h>
#include <shellapi.h>
#include <winhttp.h>

#include <algorithm>
#include <chrono>
#include <cwchar>
#include <fstream>
#include <thread>

// A release build names the agent it goes with, howldrop-native.zip (CMake:
// HOWLDROP_NATIVE_*, set by .github/workflows/release-windows.yml). A local
// build has none and uses howldrop-agent.exe beside it.
#ifndef HOWLDROP_NATIVE_URL
#define HOWLDROP_NATIVE_URL ""
#endif
#ifndef HOWLDROP_NATIVE_SHA256
#define HOWLDROP_NATIVE_SHA256 ""
#endif

namespace howldrop {

namespace fs = std::filesystem;

namespace {

const std::string kNativeUrl = HOWLDROP_NATIVE_URL;
const std::string kNativeSha256 = HOWLDROP_NATIVE_SHA256;

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

}  // namespace

// A child process with its output (stdout and stderr) on a pipe, or in a
// file, or in a console window of its own; or, run as administrator, on a
// named pipe it connects to (start_on_pipe).
struct ChildProcess {
    PROCESS_INFORMATION pi{};
    HANDLE out = nullptr;
    HANDLE event = nullptr;  // for overlapped reads of a named pipe
    std::string pending;

    ChildProcess() = default;
    ChildProcess(const ChildProcess&) = delete;
    ChildProcess& operator=(const ChildProcess&) = delete;
    ~ChildProcess() {
        if (out) CloseHandle(out);
        if (event) CloseHandle(event);
        if (pi.hThread) CloseHandle(pi.hThread);
        if (pi.hProcess) CloseHandle(pi.hProcess);
    }

    // A named pipe of ours that a program run elsewhere (start_on_pipe)
    // connects to and prints to: its name.
    bool make_pipe(std::wstring& name, std::string& error) {
        name = L"\\\\.\\pipe\\howldrop-agent-" + std::to_wstring(GetCurrentProcessId()) + L"-" +
               std::to_wstring(GetTickCount64());
        out = CreateNamedPipeW(name.c_str(), PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                               PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS, 1, 65536,
                               65536, 0, nullptr);
        event = CreateEventW(nullptr, TRUE, FALSE, nullptr);
        if (out == INVALID_HANDLE_VALUE || !event) {
            out = nullptr;
            error = "could not make a pipe";
            return false;
        }
        return true;
    }

    // exe with args and "--pipe NAME", without a window: it connects to our
    // pipe and prints to it, and stops when we disconnect it (disconnect).
    bool start_on_pipe(const fs::path& exe, const std::vector<std::wstring>& args, std::string& error) {
        std::wstring name;
        if (!make_pipe(name, error)) return false;
        std::wstring line = quote(exe.wstring());
        for (const auto& a : args) line += L" " + quote(a);
        line += L" --pipe " + quote(name);
        std::vector<wchar_t> buffer(line.begin(), line.end());
        buffer.push_back(L'\0');
        STARTUPINFOW si{};
        si.cb = sizeof(si);
        if (!CreateProcessW(nullptr, buffer.data(), nullptr, nullptr, FALSE,
                            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT, nullptr, nullptr, &si, &pi)) {
            error = "could not run " + path_to_utf8(exe.filename()) + " (error " + std::to_string(GetLastError()) +
                    ")";
            return false;
        }
        return connect(error);
    }

    // Waits for the child to connect to the pipe, or for it to end first.
    bool connect(std::string& error) {
        OVERLAPPED ov{};
        ov.hEvent = event;
        if (!ConnectNamedPipe(out, &ov)) {
            const DWORD code = GetLastError();
            if (code == ERROR_IO_PENDING) {
                const HANDLE both[] = {event, pi.hProcess};
                if (WaitForMultipleObjects(pi.hProcess ? 2 : 1, both, FALSE, 60000) != WAIT_OBJECT_0) {
                    CancelIoEx(out, &ov);
                    DWORD n = 0;
                    GetOverlappedResult(out, &ov, &n, TRUE);
                    error = wait(0) ? "it ended as it started (exit " + std::to_string(exit_code()) + ")"
                                    : "it didn't connect";
                    return false;
                }
            } else if (code != ERROR_PIPE_CONNECTED) {
                error = "the pipe (error " + std::to_string(code) + ")";
                return false;
            }
        }
        return true;
    }

    // Disconnects start_on_pipe's pipe: the child sees it go and stops.
    void disconnect() {
        if (out && event) DisconnectNamedPipe(out);
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
            if (!out || !read_some(buf, sizeof(buf), n) || n == 0) {
                if (pending.empty()) return false;
                line.swap(pending);
                pending.clear();
                return true;
            }
            pending.append(buf, n);
        }
    }

    // One read of the output; overlapped on start_on_pipe's pipe, so that
    // disconnect() can end it from another thread.
    bool read_some(char* buf, DWORD size, DWORD& n) {
        if (!event) return ReadFile(out, buf, size, &n, nullptr);
        OVERLAPPED ov{};
        ov.hEvent = event;
        ResetEvent(event);
        if (!ReadFile(out, buf, size, nullptr, &ov) && GetLastError() != ERROR_IO_PENDING) return false;
        return GetOverlappedResult(out, &ov, &n, TRUE);
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

// The agent's setup needs an administrator: Windows asks the user (UAC).
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

Radio::Radio(RadioConfig config) : config_(std::move(config)) {
    outbox_ = config_.home / L"outbox";
    scan_request_ = outbox_ / L".scan-request";
    asks_ = config_.home / L"asks";
}

Radio::~Radio() { stop(); }

void Radio::start() {
    if (thread_.joinable()) return;
    stop_.store(false);
    {
        std::lock_guard<std::mutex> hold(wake_mu_);
        wake_quit_ = false;
    }
    wake_thread_ = std::thread([this]() { wake_loop(); });
    thread_ = std::thread([this]() { run(); });
}

void Radio::wake_loop() {
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

void Radio::hold_wake(int seconds) {
    std::lock_guard<std::mutex> hold(wake_mu_);
    const auto until = std::chrono::steady_clock::now() + std::chrono::seconds(seconds);
    if (until > wake_until_) wake_until_ = until;
    wake_cv_.notify_all();
}

// A send or a scan finished: the wake stays a moment for any left.
void Radio::done_waking() {
    std::lock_guard<std::mutex> hold(wake_mu_);
    if (sends_pending_ > 0) return;
    const auto soon = std::chrono::steady_clock::now() + std::chrono::seconds(3);
    if (soon < wake_until_) wake_until_ = soon;
    wake_cv_.notify_all();
}

void Radio::stop() {
    stop_.store(true);
    std::shared_ptr<ChildProcess> agent;
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        agent = agent_process_;
    }
    if (agent) {
        // The agent sees its pipe go, and gives the stick back.
        agent->disconnect();
        if (!agent->wait(20000)) agent->kill();
    }
    if (thread_.joinable()) thread_.join();
    {
        std::lock_guard<std::mutex> hold(wake_mu_);
        wake_quit_ = true;
        wake_cv_.notify_all();
    }
    if (wake_thread_.joinable()) wake_thread_.join();
}

void Radio::pause(int ms) {
    for (int waited = 0; waited < ms && !stopping(); waited += 100) Sleep(100);
}

void Radio::log(const std::string& line) {
    std::lock_guard<std::mutex> hold(log_mu_);
    std::ofstream out(config_.home / L"radio.log", std::ios::app | std::ios::binary);
    out << line << '\n';
}

void Radio::state(const std::string& text, RadioLevel level) {
    log("state: " + text);
    if (config_.on_state) config_.on_state(text, level);
}

void Radio::run() {
    {
        std::lock_guard<std::mutex> hold(log_mu_);
        // The last run's log stays beside it, so a start that failed can
        // still be read after the one that worked.
        std::error_code ec;
        fs::rename(config_.home / L"radio.log", config_.home / L"radio.log.1", ec);
        std::ofstream(config_.home / L"radio.log", std::ios::trunc | std::ios::binary)
            << "HowlDrop " << kVersion << ": AirDrop on this PC\n";
    }
    std::string error;
    if (!prepare(error)) {
        state(stopping() ? "Off" : error, stopping() ? RadioLevel::Off : RadioLevel::Problem);
        return;
    }
    // Once it has been on the share sheet, it starts again by itself when it
    // stops (radio::restart_delay_ms), as turning it off and on would.
    bool was_up = false;
    bool set_up_again = false;
    for (int failed = 0;;) {
        std::string why = run_agent();
        const bool came_up = up_.exchange(false);
        if (config_.on_up) config_.on_up(false);
        if (stopping()) break;
        // The stick isn't on WinUSB (a new one, or put back on its own
        // driver): the setup puts it on, once, and it starts again.
        if (needs_setup_ && !set_up_again) {
            set_up_again = true;
            if (!prepare(error)) {
                state(stopping() ? "Off" : error, stopping() ? RadioLevel::Off : RadioLevel::Problem);
                return;
            }
            continue;
        }
        was_up = was_up || came_up;
        failed = came_up ? 0 : failed + 1;
        const int delay = was_up ? radio::restart_delay_ms(failed) : -1;
        if (delay < 0) {
            state("Stopped: " + why + ". Turn it off and on to retry.", RadioLevel::Problem);
            return;
        }
        log("starting again in " + std::to_string(delay / 1000) + " s: " + why);
        state("Starting again: " + why);
        pause(delay);
        if (stopping()) break;
    }
    state("Off", RadioLevel::Off);
}

bool Radio::download(const std::string& address, const std::string& sha256, const std::string& what,
                        const fs::path& tar, std::string& error) {
    state("Downloading " + what);
    const std::wstring url = wide(address);
    URL_COMPONENTSW parts{};
    parts.dwStructSize = sizeof(parts);
    wchar_t host[256];
    wchar_t path[2048];
    parts.lpszHostName = host;
    parts.dwHostNameLength = 256;
    parts.lpszUrlPath = path;
    parts.dwUrlPathLength = 2048;
    if (!WinHttpCrackUrl(url.c_str(), 0, 0, &parts) || parts.nScheme != INTERNET_SCHEME_HTTPS) {
        error = "The address of " + what + " is not https: " + address;
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
        error = "Couldn't download " + what + " (" +
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
            state("Downloading " + what + ": " + std::to_string(percent) + "% of " +
                  std::to_string(total / (1024 * 1024)) + " MB");
        }
    }
    close_all();
    file.close();
    std::error_code ec;
    if (!ok || stopping() || !file) {
        fs::remove(part, ec);
        if (!stopping()) error = "The download of " + what + " broke off. Turn this on again to retry.";
        return false;
    }
    const std::string digest = to_hex(sha.finish());
    if (lower(digest) != lower(sha256)) {
        fs::remove(part, ec);
        log("download hash " + digest + ", expected " + sha256);
        error = "The download of " + what + " isn't the one this howldrop.exe expects (its SHA-256 differs), so it "
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

bool Radio::fetch_agent(std::string& error) {
    const fs::path bin = config_.home / L"native" / L"bin";
    agent_ = bin / L"howldrop-agent.exe";
    std::string have;
    {
        std::ifstream in(bin / L".sha256", std::ios::binary);
        std::getline(in, have);
    }
    std::error_code ec;
    if (fs::exists(agent_, ec) && !kNativeSha256.empty() && lower(trim(have)) == lower(kNativeSha256)) return true;
    if (kNativeUrl.empty()) {
        error = "This local build has no howldrop-agent.exe beside it.";
        return false;
    }
    const fs::path zip = config_.home / L"howldrop-native.zip";
    if (!download(kNativeUrl, kNativeSha256, "the AirDrop part", zip, error)) return false;
    fs::remove_all(bin, ec);
    fs::create_directories(bin, ec);
    // Windows' own tar (10 1803 and later) reads zip.
    wchar_t system[MAX_PATH];
    const UINT sn = GetSystemDirectoryW(system, MAX_PATH);
    const fs::path tar = (sn > 0 && sn < MAX_PATH ? fs::path(system) : fs::path(L"C:\\Windows\\System32")) / L"tar.exe";
    std::string out;
    const int code = run_command(tar, {L"-xf", zip.wstring(), L"-C", bin.wstring()}, out, 120000);
    fs::remove(zip, ec);
    if (code != 0 || !fs::exists(agent_, ec)) {
        log("tar -xf howldrop-native.zip: exit " + std::to_string(code) + ": " + trim(out));
        error = "Unpacking the AirDrop part failed. Turn this on again to retry.";
        return false;
    }
    std::ofstream(bin / L".sha256", std::ios::binary) << kNativeSha256 << '\n';
    return true;
}

bool Radio::prepare(std::string& error) {
    std::error_code ec;
    fs::create_directories(config_.home, ec);
    fs::create_directories(outbox_, ec);
    fs::create_directories(config_.inbox, ec);
    wchar_t self[MAX_PATH];
    const DWORD n = GetModuleFileNameW(nullptr, self, MAX_PATH);
    agent_ = (n > 0 && n < MAX_PATH ? fs::path(self).parent_path() : fs::path()) / L"howldrop-agent.exe";
    if (!fs::exists(agent_, ec) && !fetch_agent(error)) return false;
    // The agent runs as the user. Only a stick not yet on WinUSB needs an
    // administrator, once (setup.rs), when the agent says so.
    if (!needs_setup_) return true;
    needs_setup_ = false;
    state("Getting the stick ready (Windows asks for permission, this once)");
    const fs::path setup_log = config_.home / L"native-setup.log";
    const int code = run_elevated(agent_, L"--setup " + quote(setup_log.wstring()));
    std::string said;
    {
        std::ifstream in(setup_log, std::ios::binary);
        said.assign(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
    }
    log("howldrop-agent --setup: exit " + std::to_string(code) + "\n" + trim(said));
    if (code != 0) {
        const auto last = trim(said).rfind('\n');
        const std::string why = trim(last == std::string::npos ? trim(said) : trim(said).substr(last + 1));
        error = code == -1 ? "Getting the stick ready needs Windows' permission. Turn it on again to retry."
                           : "Getting the stick ready failed: " + (why.empty() ? "exit " + std::to_string(code) : why);
        return false;
    }
    return true;
}

std::string Radio::run_agent() {
    stop_reason_.clear();
    std::error_code ec;
    fs::remove(scan_request_, ec);
    fs::create_directories(asks_, ec);
    if (stopping()) return {};
    state("Starting");
    const fs::path data = config_.home / L"native";
    auto agent = std::make_shared<ChildProcess>();
    std::string error;
    if (!agent->start_on_pipe(agent_,
                              {L"--name", wide(config_.name), L"--staging", (data / L"staging").wstring(),
                               L"--deliver", config_.inbox.wstring(), L"--identity", (data / L"identity").wstring(),
                               L"--receive", wide(config_.receive), L"--senders", wide(config_.senders), L"--ask-dir",
                               asks_.wstring(), L"--outbox", outbox_.wstring(), L"--discard", L"--scan-request",
                               scan_request_.wstring(), L"--events", L"--local-jobs", L"--tx-rate", wide(config_.tx_rate)},
                              error)) {
        log("howldrop-agent: " + error);
        return error;
    }
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        agent_process_ = agent;
    }
    if (stopping()) agent->disconnect();  // stop() came while it started
    std::string line;
    while (agent->read_line(line)) handle_line(line);
    log("howldrop-agent ended");
    {
        std::lock_guard<std::mutex> hold(process_mu_);
        agent_process_.reset();
    }
    if (!stop_reason_.empty()) return stop_reason_;
    return "howldrop-agent ended";
}

void Radio::handle_line(const std::string& line) {
    log(line);
    const auto event = radio::event_fields(line);
    // The agent starts on a file: its window (180 s) is when the phone must hear the wake.
    if (line.rfind("Send: sending ", 0) == 0) hold_wake(200);
    if (!event) return;
    const auto& f = *event;
    if (f.empty()) return;
    const std::string& what = f[0];
    const auto field = [&](std::size_t i) { return i < f.size() ? f[i] : std::string(); };
    if (what == "stick") {
        if (field(1) == "needs-winusb") needs_setup_ = true;
    } else if (what == "stopped") {
        stop_reason_ = field(1);
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
    } else if (what.rfind("outgoing", 0) == 0) {
        if (what == "outgoing-done") {
            { std::lock_guard<std::mutex> hold(wake_mu_); if (sends_pending_ > 0) --sends_pending_; }
            done_waking();
        }
        if (config_.on_event) config_.on_event(f);
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

void Radio::decide(const std::string& id, bool accept) {
    // An id is the agent's hex (receive.rs random_id); anything else names no ask.
    if (id.empty() || id.size() > 32 || id.find_first_not_of("0123456789abcdef") != std::string::npos) return;
    std::error_code ec;
    fs::create_directories(asks_, ec);
    std::ofstream(asks_ / (id + (accept ? ".accept" : ".decline")), std::ios::binary) << '\n';
    log(std::string("ask ") + id + (accept ? ": accepted on the PC" : ": declined on the PC"));
}

// --- sending to the iPhone (Node::LocalIphone) ----------------------------------------

Status Radio::queue(const fs::path& path, const std::string& to) {
    return queue_selection({path}, to);
}

Status Radio::queue_selection(const std::vector<fs::path>& paths, const std::string& to) {
    if (!up_.load()) return Status::failure("AirDrop on this PC isn't on yet");
    if (paths.empty() || paths.size() > 256) return Status::failure("choose 1 to 256 files or folders");
    if (to.find_first_of("\t\r\n") != std::string::npos) return Status::failure("invalid recipient");
    const std::string id = new_pair_key();
    if (id.empty()) return Status::failure("could not create a send job");
    std::string text = "HowlDrop job 1\n" + to + "\n";
    std::vector<std::string> fields = {"outgoing", id, to, std::to_string(paths.size()), "0"};
    std::error_code ec;
    for (const auto& input : paths) {
        const auto path = fs::absolute(input, ec);
        if (ec || (!fs::is_regular_file(path, ec) && !fs::is_directory(path, ec)))
            return Status::failure("choose existing files or folders");
        const auto utf8 = path_to_utf8(path);
        if (utf8.find_first_of("\t\r\n") != std::string::npos) return Status::failure("invalid file path");
        text += utf8 + "\n";
        fields.push_back(filename_utf8(path));
    }
    if (text.size() > (1u << 20)) return Status::failure("selection is too large");
    fs::create_directories(outbox_, ec);
    const auto dest = outbox_ / (id + ".howldrop-job");
    const auto partial = outbox_ / (id + ".partial");
    {
        std::ofstream out(partial, std::ios::binary);
        out << text;
        out.close();
        if (!out) { fs::remove(partial, ec); return Status::failure("could not queue the selection"); }
    }
    fs::rename(partial, dest, ec);
    if (ec) { fs::remove(partial, ec); return Status::failure("could not queue the selection"); }
    { std::lock_guard<std::mutex> hold(wake_mu_); ++sends_pending_; }
    if (config_.on_event) config_.on_event(fields);
    hold_wake(200);
    return Status::success("queued");
}

Status Radio::control_send(const std::string& id, bool retry) {
    if (id.size() != 32 || id.find_first_not_of("0123456789abcdef") != std::string::npos)
        return Status::failure("invalid transfer");
    std::error_code ec;
    const auto job = outbox_ / (id + ".howldrop-job");
    const auto cancel = outbox_ / (id + ".howldrop-cancel");
    if (retry) {
        if (!up_.load()) return Status::failure("turn on AirDrop on this PC first");
        fs::remove(cancel, ec);
        fs::rename(outbox_ / "failed" / job.filename(), job, ec);
        if (ec) return Status::failure("this selection is no longer available for Retry");
        std::ifstream input(job, std::ios::binary);
        std::string header, to, path;
        std::getline(input, header);
        std::getline(input, to);
        std::vector<std::string> fields = {"outgoing", id, to, "0", "0"};
        while (std::getline(input, path)) fields.push_back(filename_utf8(path_from_utf8(path)));
        fields[3] = std::to_string(fields.size() - 5);
        if (config_.on_event) config_.on_event(fields);
        { std::lock_guard<std::mutex> hold(wake_mu_); ++sends_pending_; }
        hold_wake(200);
    } else {
        if (!fs::exists(job, ec)) return Status::failure("this transfer has already finished");
        std::ofstream out(cancel, std::ios::binary);
        out << '\n';
        if (!out) return Status::failure("could not cancel this transfer");
        if (config_.on_event) config_.on_event({"outgoing-stage", id, "cancelling"});
    }
    return Status::success(retry ? "queued again" : "cancelling");
}

Node::Nearby Radio::nearby() const {
    std::lock_guard<std::mutex> hold(nearby_mu_);
    Node::Nearby out;
    out.relay = up_.load();
    out.local = true;
    out.scanning = scanning_;
    out.devices = devices_;
    return out;
}

Status Radio::scan() {
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
