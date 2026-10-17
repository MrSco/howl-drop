// The Windows tray app (tray.hpp). Plain Win32: the notification is the tray
// icon's own (Shell_NotifyIcon NIF_INFO), which Windows 10 and 11 show as a
// toast, so no WinRT or app registration is needed and MinGW builds it.

#include "tray.hpp"

#include "clipboard.hpp"
#include "open_policy.hpp"
#include "text.hpp"

#include <windows.h>
#include <shellapi.h>
#include <commdlg.h>
#include <shlobj.h>

#include <cwchar>
#include <fstream>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace howldrop {
namespace {

constexpr UINT kTrayMessage = WM_APP + 1;
constexpr UINT kDropMessage = WM_APP + 2;
constexpr UINT kRefusedMessage = WM_APP + 3;
constexpr UINT kSentMessage = WM_APP + 4;
// Explorer's Send to entry: howldrop.exe iphone FILE... (main.cpp).
const wchar_t* const kSendToName = L"iPhone (HowlDrop).lnk";
constexpr UINT kTrayId = 1;
constexpr UINT_PTR kStopTimer = 1;
constexpr int kIconResource = 1;
const wchar_t* const kWindowClass = L"HowlDropTray";
const wchar_t* const kRunKey = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const wchar_t* const kRunValue = L"HowlDrop";

enum : UINT {
    kMenuOpenFolder = 100,
    kMenuNotify,
    kMenuOpenLinks,
    kMenuOpenMedia,
    kMenuOpenDocuments,
    kMenuShareClipboard,
    kMenuCopyKey,
    kMenuSendIphone,
    kMenuSendToMenu,
    kMenuStartup,
    kMenuQuit,
};

struct Settings {
    bool notify = true;
    bool open_links = true;
    bool open_media = false;
    bool open_documents = false;
    bool share_clipboard = false;  // send what is copied here to other HowlDrop PCs
    bool send_to_menu = true;      // "iPhone (HowlDrop)" in Explorer's Send to
};

// A send to the iPhone relay finished, posted from its worker thread.
struct Sent {
    std::string file;
    bool ok = false;
    std::string message;
};

// What arrived, posted from the node's worker thread to the window.
struct Drop {
    std::filesystem::path path;
    std::string text;
    std::string from;
    bool is_text = false;
};

struct App {
    HWND window = nullptr;
    NOTIFYICONDATAW icon{};
    UINT taskbar_created = 0;
    Settings settings;
    std::filesystem::path settings_file;
    std::filesystem::path log_file;
    std::filesystem::path inbox;
    std::unique_ptr<Clipboard> clipboard;
    std::unique_ptr<Node> node;
    std::string pair_key;
    ULONGLONG last_refusal_note = 0;
    std::vector<std::thread> senders;  // sends from the menu, joined on quit
    std::wstring click_target;  // what clicking the last notification opens
    bool click_selects = false;  // show it in its folder instead of opening it
};

App* g_app = nullptr;

std::wstring wide(const std::string& utf8) {
    auto converted = wide_from_utf8(utf8);
    return converted ? *converted : std::wstring();
}

void copy_into(wchar_t* dest, std::size_t size, const std::wstring& text) {
    std::wcsncpy(dest, text.c_str(), size - 1);
    dest[size - 1] = L'\0';
}

// --- settings -----------------------------------------------------------------

void load_settings(App& app) {
    std::ifstream in(app.settings_file);
    std::string line;
    while (std::getline(in, line)) {
        const auto eq = line.find('=');
        if (eq == std::string::npos) continue;
        const std::string key = line.substr(0, eq);
        const bool on = line.compare(eq + 1, 1, "1") == 0;
        if (key == "notify") app.settings.notify = on;
        else if (key == "open_links") app.settings.open_links = on;
        else if (key == "open_media") app.settings.open_media = on;
        else if (key == "open_documents") app.settings.open_documents = on;
        else if (key == "share_clipboard") app.settings.share_clipboard = on;
        else if (key == "send_to_menu") app.settings.send_to_menu = on;
    }
}

void save_settings(const App& app) {
    std::ofstream out(app.settings_file, std::ios::trunc);
    out << "notify=" << app.settings.notify << '\n'
        << "open_links=" << app.settings.open_links << '\n'
        << "open_media=" << app.settings.open_media << '\n'
        << "open_documents=" << app.settings.open_documents << '\n'
        << "share_clipboard=" << app.settings.share_clipboard << '\n'
        << "send_to_menu=" << app.settings.send_to_menu << '\n';
}

bool starts_with_windows() {
    HKEY key;
    if (RegOpenKeyExW(HKEY_CURRENT_USER, kRunKey, 0, KEY_QUERY_VALUE, &key) != ERROR_SUCCESS) return false;
    const bool present = RegQueryValueExW(key, kRunValue, nullptr, nullptr, nullptr, nullptr) == ERROR_SUCCESS;
    RegCloseKey(key);
    return present;
}

void set_start_with_windows(bool on) {
    HKEY key;
    if (RegCreateKeyExW(HKEY_CURRENT_USER, kRunKey, 0, nullptr, 0, KEY_SET_VALUE, nullptr, &key, nullptr) !=
        ERROR_SUCCESS)
        return;
    if (on) {
        wchar_t exe[MAX_PATH];
        const DWORD n = GetModuleFileNameW(nullptr, exe, MAX_PATH);
        if (n > 0 && n < MAX_PATH) {
            const std::wstring command = L"\"" + std::wstring(exe, n) + L"\"";
            RegSetValueExW(key, kRunValue, 0, REG_SZ, reinterpret_cast<const BYTE*>(command.c_str()),
                           static_cast<DWORD>((command.size() + 1) * sizeof(wchar_t)));
        }
    } else {
        RegDeleteValueW(key, kRunValue);
    }
    RegCloseKey(key);
}

// --- opening and notifying ------------------------------------------------------

void open_target(const std::wstring& target) {
    ShellExecuteW(nullptr, L"open", target.c_str(), nullptr, nullptr, SW_SHOWNORMAL);
}

void show_in_folder(const std::wstring& path) {
    const std::wstring args = L"/select,\"" + path + L"\"";
    ShellExecuteW(nullptr, L"open", L"explorer.exe", args.c_str(), nullptr, SW_SHOWNORMAL);
}

void notify(App& app, const std::wstring& title, const std::wstring& message) {
    if (!app.settings.notify) return;
    NOTIFYICONDATAW data = app.icon;
    data.uFlags = NIF_INFO;
    data.dwInfoFlags = NIIF_USER | NIIF_LARGE_ICON;
    copy_into(data.szInfoTitle, ARRAYSIZE(data.szInfoTitle), title);
    copy_into(data.szInfo, ARRAYSIZE(data.szInfo), message);
    Shell_NotifyIconW(NIM_MODIFY, &data);
}

std::wstring preview(const std::string& text) {
    std::string flat;
    for (char c : text) {
        if (flat.size() >= 180) {
            flat += "...";
            break;
        }
        flat.push_back(c == '\r' || c == '\n' || c == '\t' ? ' ' : c);
    }
    return wide(flat);
}

void handle_drop(App& app, const Drop& drop) {
    const std::wstring from = drop.from.empty() ? L"" : L" from " + wide(drop.from);
    if (drop.is_text) {
        app.click_target.clear();
        notify(app, L"Text" + from + L", on the clipboard", preview(drop.text));
        return;
    }
    const OpenKind kind = classify(drop.path);
    const std::wstring path = drop.path.wstring();
    const std::wstring name = drop.path.filename().wstring();
    std::wstring message = name;
    bool opened = false;

    app.click_target = path;
    app.click_selects = kind == OpenKind::Other;
    if (kind == OpenKind::Link) {
        const std::string url = shortcut_url(drop.path);
        if (!url.empty()) {
            message = wide(url);
            app.click_target = message;
            app.click_selects = false;
            if (app.settings.open_links) {
                open_target(message);
                opened = true;
            }
        } else {
            app.click_selects = true;  // not http(s): never opened
        }
    } else if ((kind == OpenKind::Image || kind == OpenKind::Video) && app.settings.open_media) {
        open_target(path);
        opened = true;
    } else if (kind == OpenKind::Document && app.settings.open_documents) {
        open_target(path);
        opened = true;
    }
    std::wstring title = wide(kind_word(kind)) + from;
    if (opened) title += L", opened";
    notify(app, title, message);
}

// --- tray icon and menu ------------------------------------------------------

void add_icon(App& app) {
    Shell_NotifyIconW(NIM_ADD, &app.icon);
    app.icon.uVersion = NOTIFYICON_VERSION_4;
    Shell_NotifyIconW(NIM_SETVERSION, &app.icon);
}

void show_menu(App& app) {
    HMENU menu = CreatePopupMenu();
    const auto check = [](bool on) { return static_cast<UINT>(MF_STRING | (on ? MF_CHECKED : MF_UNCHECKED)); };
    AppendMenuW(menu, MF_STRING, kMenuOpenFolder, L"Open HowlDrop folder");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, check(app.settings.notify), kMenuNotify, L"Show notifications");
    AppendMenuW(menu, check(app.settings.open_links), kMenuOpenLinks, L"Open links in the browser");
    AppendMenuW(menu, check(app.settings.open_media), kMenuOpenMedia, L"Open photos and videos");
    AppendMenuW(menu, check(app.settings.open_documents), kMenuOpenDocuments, L"Open PDFs and text files");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING, kMenuSendIphone, L"Send a file to iPhone...");
    AppendMenuW(menu, check(app.settings.send_to_menu), kMenuSendToMenu, L"\"iPhone\" in Explorer's Send to menu");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING, kMenuCopyKey, L"Copy pairing key");
    AppendMenuW(menu, check(app.settings.share_clipboard), kMenuShareClipboard,
                L"Share what I copy with my other HowlDrop PCs");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, check(starts_with_windows()), kMenuStartup, L"Start with Windows");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, MF_STRING, kMenuQuit, L"Quit HowlDrop");
    POINT at;
    GetCursorPos(&at);
    SetForegroundWindow(app.window);  // so the menu closes when you click elsewhere
    TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, at.x, at.y, 0, app.window, nullptr);
    PostMessageW(app.window, WM_NULL, 0, 0);
    DestroyMenu(menu);
}

// --- sending to the iPhone, through the add-on's relay -----------------------------

std::filesystem::path send_to_shortcut() {
    PWSTR folder = nullptr;
    if (FAILED(SHGetKnownFolderPath(FOLDERID_SendTo, 0, nullptr, &folder))) return {};
    std::filesystem::path path = std::filesystem::path(folder) / kSendToName;
    CoTaskMemFree(folder);
    return path;
}

// Explorer's Send to, "iPhone (HowlDrop)": runs this exe with `iphone` and
// the chosen files. Made again each start, so it follows the exe if it moves.
void update_send_to(const App& app) {
    const auto shortcut = send_to_shortcut();
    if (shortcut.empty()) return;
    std::error_code ec;
    if (!app.settings.send_to_menu) {
        std::filesystem::remove(shortcut, ec);
        return;
    }
    wchar_t exe[MAX_PATH];
    const DWORD n = GetModuleFileNameW(nullptr, exe, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) return;
    IShellLinkW* link = nullptr;
    if (FAILED(CoCreateInstance(CLSID_ShellLink, nullptr, CLSCTX_INPROC_SERVER, IID_IShellLinkW,
                                reinterpret_cast<void**>(&link))))
        return;
    link->SetPath(exe);
    link->SetArguments(L"iphone");
    link->SetIconLocation(exe, 0);
    link->SetDescription(L"AirDrop to your iPhone through HowlDrop in Home Assistant");
    IPersistFile* file = nullptr;
    if (SUCCEEDED(link->QueryInterface(IID_IPersistFile, reinterpret_cast<void**>(&file)))) {
        file->Save(shortcut.c_str(), TRUE);
        file->Release();
    }
    link->Release();
}

// Pick files and send each to the relay in the background; on_sent reports.
void send_files_to_iphone(App& app) {
    std::vector<wchar_t> buffer(64 * 1024, L'\0');
    OPENFILENAMEW ofn{};
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = app.window;
    ofn.lpstrFile = buffer.data();
    ofn.nMaxFile = static_cast<DWORD>(buffer.size());
    ofn.lpstrTitle = L"AirDrop to iPhone (set it to Everyone for 10 Minutes)";
    ofn.Flags = OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    SetForegroundWindow(app.window);
    if (!GetOpenFileNameW(&ofn)) return;
    // One file: its full path. Several: the folder, then each name; all
    // separated by a NUL, ended by two.
    std::vector<std::filesystem::path> files;
    const std::wstring first(buffer.data());
    const wchar_t* next = buffer.data() + first.size() + 1;
    if (*next == L'\0') {
        files.emplace_back(first);
    } else {
        for (; *next; next += std::wcslen(next) + 1) files.push_back(std::filesystem::path(first) / next);
    }
    Node* node = app.node.get();
    app.senders.emplace_back([node, files]() {
        for (const auto& file : files) node->send_to_iphone(file);
    });
}

void post_sent(const std::string& file, const Status& status) {
    auto* sent = new Sent{file, status.ok, status.message};
    if (!g_app || !g_app->window || !PostMessageW(g_app->window, kSentMessage, 0, reinterpret_cast<LPARAM>(sent)))
        delete sent;
}

// The add-on's pc_key, onto the clipboard. Shown even with notifications off:
// it is the answer to what the user just asked for.
void copy_pair_key(App& app) {
    const std::string shown = display_pair_key(app.pair_key);
    const bool copied = app.clipboard && app.clipboard->set_text(shown);
    const bool notify_was = app.settings.notify;
    app.settings.notify = true;
    app.click_target.clear();
    notify(app, copied ? L"Pairing key copied" : L"Pairing key",
           copied ? L"Paste it into the HowlDrop add-on's pc_key option in Home Assistant, then restart the add-on."
                  : L"Copy failed. Run howldrop key in a terminal to see it: " + wide(shown));
    app.settings.notify = notify_was;
}

void open_folder(App& app) {
    std::error_code ec;
    std::filesystem::create_directories(app.inbox, ec);
    open_target(app.inbox.wstring());
}

LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    App* app = g_app;
    if (!app) return DefWindowProcW(window, message, wparam, lparam);
    if (message == app->taskbar_created && message != 0) {  // Explorer restarted
        add_icon(*app);
        return 0;
    }
    switch (message) {
        case kTrayMessage:
            switch (LOWORD(lparam)) {
                case WM_CONTEXTMENU:
                case WM_RBUTTONUP:
                    show_menu(*app);
                    return 0;
                case NIN_SELECT:  // a click, or Enter on the icon
                case NIN_KEYSELECT:
                    open_folder(*app);
                    return 0;
                case NIN_BALLOONUSERCLICK:
                    if (app->click_target.empty()) open_folder(*app);
                    else if (app->click_selects) show_in_folder(app->click_target);
                    else open_target(app->click_target);
                    return 0;
                default:
                    return 0;
            }
        case kDropMessage: {
            std::unique_ptr<Drop> drop(reinterpret_cast<Drop*>(lparam));
            handle_drop(*app, *drop);
            return 0;
        }
        case kSentMessage: {
            std::unique_ptr<Sent> sent(reinterpret_cast<Sent*>(lparam));
            app->click_target.clear();
            // Said even with notifications off: it answers what was just asked.
            const bool notify_was = app->settings.notify;
            app->settings.notify = true;
            if (sent->ok) {
                notify(*app, L"Sent to your iPhone through Home Assistant",
                       wide(sent->file) + L": accept it on the iPhone (AirDrop set to Everyone for 10 Minutes).");
            } else {
                notify(*app, L"Not sent to the iPhone", wide(sent->file) + L": " + wide(sent->message));
            }
            app->settings.notify = notify_was;
            return 0;
        }
        case kRefusedMessage: {
            // A sender without this PC's pairing key: most likely the add-on
            // before pc_key is set. Said at most every 10 minutes.
            std::unique_ptr<std::string> why(reinterpret_cast<std::string*>(lparam));
            const ULONGLONG now = GetTickCount64();
            if (app->last_refusal_note == 0 || now - app->last_refusal_note > 10 * 60 * 1000) {
                app->last_refusal_note = now;
                app->click_target.clear();
                notify(*app, L"HowlDrop refused a sender that is not paired",
                       L"If that was Home Assistant: right-click this icon, Copy pairing key, and paste it into the "
                       L"add-on's pc_key option.");
            }
            return 0;
        }
        case WM_COMMAND:
            switch (LOWORD(wparam)) {
                case kMenuOpenFolder: open_folder(*app); break;
                case kMenuNotify: app->settings.notify = !app->settings.notify; break;
                case kMenuOpenLinks: app->settings.open_links = !app->settings.open_links; break;
                case kMenuOpenMedia: app->settings.open_media = !app->settings.open_media; break;
                case kMenuOpenDocuments: app->settings.open_documents = !app->settings.open_documents; break;
                case kMenuShareClipboard:
                    app->settings.share_clipboard = !app->settings.share_clipboard;
                    if (app->node) app->node->set_share_clipboard(app->settings.share_clipboard);
                    break;
                case kMenuCopyKey: copy_pair_key(*app); return 0;
                case kMenuSendIphone: send_files_to_iphone(*app); return 0;
                case kMenuSendToMenu:
                    app->settings.send_to_menu = !app->settings.send_to_menu;
                    update_send_to(*app);
                    break;
                case kMenuStartup: set_start_with_windows(!starts_with_windows()); break;
                case kMenuQuit: DestroyWindow(window); return 0;
                default: return 0;
            }
            save_settings(*app);
            return 0;
        case WM_TIMER:  // `howldrop stop` ends the node; the icon goes with it
            if (app->node && !app->node->running()) DestroyWindow(window);
            return 0;
        case WM_DESTROY:
            KillTimer(window, kStopTimer);
            Shell_NotifyIconW(NIM_DELETE, &app->icon);
            PostQuitMessage(0);
            return 0;
        default:
            return DefWindowProcW(window, message, wparam, lparam);
    }
}

void post_drop(Drop* drop) {
    if (!g_app || !g_app->window || !PostMessageW(g_app->window, kDropMessage, 0, reinterpret_cast<LPARAM>(drop)))
        delete drop;
}

}  // namespace

int run_tray(Node::Config config) {
    // One tray app per user: a second start would only fail to bind the ports.
    HANDLE single = CreateMutexW(nullptr, TRUE, L"Local\\HowlDropTray");
    if (GetLastError() == ERROR_ALREADY_EXISTS) {
        MessageBoxW(nullptr, L"HowlDrop is already running. Its icon is by the clock.", L"HowlDrop",
                    MB_OK | MB_ICONINFORMATION);
        return 0;
    }

    App app;
    g_app = &app;
    app.inbox = config.inbox;
    wchar_t local[MAX_PATH];
    const DWORD n = GetEnvironmentVariableW(L"LOCALAPPDATA", local, MAX_PATH);
    const std::filesystem::path dir = (n > 0 && n < MAX_PATH)
                                          ? std::filesystem::path(local) / L"HowlDrop"
                                          : config.inbox.parent_path() / L".howldrop";
    std::error_code ec;
    std::filesystem::create_directories(dir, ec);
    app.settings_file = dir / L"settings.ini";
    app.log_file = dir / L"howldrop.log";
    const bool first_run = !std::filesystem::exists(app.settings_file, ec);
    if (first_run) save_settings(app);
    else load_settings(app);
    app.pair_key = config.pair_key;
    config.share_clipboard = app.settings.share_clipboard;
    if (std::filesystem::file_size(app.log_file, ec) > (1u << 20)) std::filesystem::remove(app.log_file, ec);

    HINSTANCE instance = GetModuleHandleW(nullptr);
    WNDCLASSEXW cls{};
    cls.cbSize = sizeof(cls);
    cls.lpfnWndProc = window_proc;
    cls.hInstance = instance;
    cls.lpszClassName = kWindowClass;
    RegisterClassExW(&cls);
    app.window = CreateWindowExW(0, kWindowClass, L"HowlDrop", 0, 0, 0, 0, 0, nullptr, nullptr, instance, nullptr);
    if (!app.window) return 1;
    app.taskbar_created = RegisterWindowMessageW(L"TaskbarCreated");

    app.icon.cbSize = sizeof(app.icon);
    app.icon.hWnd = app.window;
    app.icon.uID = kTrayId;
    app.icon.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP;
    app.icon.uCallbackMessage = kTrayMessage;
    app.icon.hIcon = static_cast<HICON>(LoadImageW(instance, MAKEINTRESOURCEW(kIconResource), IMAGE_ICON,
                                                   GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON), 0));
    if (!app.icon.hIcon) app.icon.hIcon = LoadIconW(nullptr, MAKEINTRESOURCEW(32512));  // IDI_APPLICATION
    app.icon.hBalloonIcon = static_cast<HICON>(LoadImageW(instance, MAKEINTRESOURCEW(kIconResource), IMAGE_ICON,
                                                          GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON), 0));
    copy_into(app.icon.szTip, ARRAYSIZE(app.icon.szTip), L"HowlDrop: files go to " + app.inbox.wstring());
    add_icon(app);

    config.on_file = [](const std::filesystem::path& saved, const std::string& from) {
        post_drop(new Drop{saved, {}, from, false});
    };
    config.on_text = [](const std::string& text, const std::string& from) {
        post_drop(new Drop{{}, text, from, true});
    };
    config.on_sent = [](const std::string& file, const Status& status) { post_sent(file, status); };
    const std::filesystem::path log_file = app.log_file;
    config.on_log = [log_file](const std::string& line) {
        {
            static std::mutex lock;  // lines come from the node's threads
            const std::lock_guard<std::mutex> hold(lock);
            std::ofstream out(log_file, std::ios::app);
            out << line << '\n';
        }
        if (line.rfind("refused a ", 0) == 0 && g_app && g_app->window) {
            auto* why = new std::string(line);
            if (!PostMessageW(g_app->window, kRefusedMessage, 0, reinterpret_cast<LPARAM>(why))) delete why;
        }
    };

    app.clipboard = make_system_clipboard();
    try {
        app.node = std::make_unique<Node>(std::move(config), *app.clipboard);
        app.node->start();
    } catch (const std::exception& ex) {
        Shell_NotifyIconW(NIM_DELETE, &app.icon);
        MessageBoxW(nullptr, (L"HowlDrop could not start: " + wide(ex.what())).c_str(), L"HowlDrop",
                    MB_OK | MB_ICONERROR);
        return 1;
    }
    SetTimer(app.window, kStopTimer, 1000, nullptr);
    const bool com = SUCCEEDED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED));
    update_send_to(app);
    if (first_run) {
        notify(app, L"HowlDrop is running",
               L"To get AirDrops from Home Assistant: right-click this icon, Copy pairing key, and paste it into "
               L"the add-on's pc_key option.");
    }

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    app.node->request_stop();  // a send in progress stops at its next chunk
    for (auto& sender : app.senders) {
        if (sender.joinable()) sender.join();
    }
    app.node->stop();
    g_app = nullptr;
    if (com) CoUninitialize();
    if (single) CloseHandle(single);
    return 0;
}

}  // namespace howldrop
