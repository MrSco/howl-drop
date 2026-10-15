// The Windows tray app (tray.hpp). Plain Win32: the notification is the tray
// icon's own (Shell_NotifyIcon NIF_INFO), which Windows 10 and 11 show as a
// toast, so no WinRT or app registration is needed and MinGW builds it.

#include "tray.hpp"

#include "clipboard.hpp"
#include "open_policy.hpp"
#include "text.hpp"

#include <windows.h>
#include <shellapi.h>

#include <cwchar>
#include <fstream>
#include <memory>
#include <mutex>
#include <string>

namespace howldrop {
namespace {

constexpr UINT kTrayMessage = WM_APP + 1;
constexpr UINT kDropMessage = WM_APP + 2;
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
    kMenuStartup,
    kMenuQuit,
};

struct Settings {
    bool notify = true;
    bool open_links = true;
    bool open_media = false;
    bool open_documents = false;
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
    }
}

void save_settings(const App& app) {
    std::ofstream out(app.settings_file, std::ios::trunc);
    out << "notify=" << app.settings.notify << '\n'
        << "open_links=" << app.settings.open_links << '\n'
        << "open_media=" << app.settings.open_media << '\n'
        << "open_documents=" << app.settings.open_documents << '\n';
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
        case WM_COMMAND:
            switch (LOWORD(wparam)) {
                case kMenuOpenFolder: open_folder(*app); break;
                case kMenuNotify: app->settings.notify = !app->settings.notify; break;
                case kMenuOpenLinks: app->settings.open_links = !app->settings.open_links; break;
                case kMenuOpenMedia: app->settings.open_media = !app->settings.open_media; break;
                case kMenuOpenDocuments: app->settings.open_documents = !app->settings.open_documents; break;
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
    if (std::filesystem::exists(app.settings_file, ec)) load_settings(app);
    else save_settings(app);
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
    const std::filesystem::path log_file = app.log_file;
    config.on_log = [log_file](const std::string& line) {
        static std::mutex lock;  // lines come from the node's threads
        const std::lock_guard<std::mutex> hold(lock);
        std::ofstream out(log_file, std::ios::app);
        out << line << '\n';
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

    MSG msg;
    while (GetMessageW(&msg, nullptr, 0, 0) > 0) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
    app.node->request_stop();
    app.node->stop();
    g_app = nullptr;
    if (single) CloseHandle(single);
    return 0;
}

}  // namespace howldrop
