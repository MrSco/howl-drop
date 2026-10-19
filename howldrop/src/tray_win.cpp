// The Windows tray app (tray.hpp). Plain Win32: the notification is the tray
// icon's own (Shell_NotifyIcon NIF_INFO), which Windows 10 and 11 show as a
// toast, so no WinRT or app registration is needed and MinGW builds it.

#include "tray.hpp"

#include "cards_win.hpp"
#include "clipboard.hpp"
#include "history.hpp"
#include "open_policy.hpp"
#include "purge.hpp"
#include "text.hpp"
#include "transfers.hpp"
#include "ui_win.hpp"
#include "window_win.hpp"
#include "wsl_win.hpp"

#include <windows.h>
#include <shellapi.h>
#include <commdlg.h>
#include <shlobj.h>

#include <chrono>
#include <cmath>
#include <cstdint>
#include <set>
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
constexpr UINT kRadioMessage = WM_APP + 5;
constexpr UINT kTransferMessage = WM_APP + 6;      // a transfer's news from the radio
constexpr UINT kRadioStoppedMessage = WM_APP + 7;  // a radio turned off has given the stick back
// Files older than this go from the inbox while the cleanup setting is on,
// as with the add-on's delete_after_days.
constexpr int kKeepDays = 30;
// Explorer's Send to entry: howldrop.exe iphone FILE... (main.cpp).
const wchar_t* const kSendToName = L"iPhone (HowlDrop).lnk";
// Send to iPhone submenu: one item per nearby device, from this id up.
constexpr UINT kMenuDeviceBase = 300;
constexpr UINT kMenuDeviceMax = 64;
constexpr UINT kTrayId = 1;
constexpr UINT_PTR kStopTimer = 1;
constexpr int kIconResource = 1;
const wchar_t* const kWindowClass = L"HowlDropTray";
const wchar_t* const kRunKey = L"Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const wchar_t* const kRunValue = L"HowlDrop";

enum : UINT {
    kMenuOpenFolder = 100,
    kMenuRadio,
    kMenuCleanup,
    kMenuReceiveEveryone,
    kMenuReceiveAsk,
    kMenuReceiveContacts,
    kMenuOpenWindow,
    kMenuNotify,
    kMenuOpenLinks,
    kMenuOpenMedia,
    kMenuOpenDocuments,
    kMenuShareClipboard,
    kMenuCopyKey,
    kMenuSendIphone,
    kMenuScan,
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
    bool radio = false;            // AirDrop on this PC: the USB stick, in WSL (wsl_win.hpp)
    bool cleanup = true;           // delete files from the inbox after kKeepDays
    // Who may send to AirDrop on this PC (WslRadioConfig::receive): everyone,
    // ask (contacts at once, anyone else after Accept), contacts.
    std::string receive = "ask";
    std::string senders;  // the contacts: phone numbers and emails, comma-separated
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
    bool to_clipboard = false;  // from this PC's radio: the node did not put it there
    bool radio = false;         // from this PC's radio, not Home Assistant
    std::string kind;           // deliver.py's word for it, when the radio said
};

// From this PC's radio (wsl_win.hpp), posted from its thread. generation
// tells a radio turned off since from the one running now.
struct RadioNote {
    unsigned generation = 0;
    std::string state;
    int up = -1;  // 1: on the share sheet, 0: not any more, -1: no change
    RadioLevel level = RadioLevel::Working;
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
    std::vector<std::string> menu_devices;  // the submenu's devices, as shown
    ULONGLONG scan_asked = 0;               // when Look for devices was chosen
    bool scan_seen = false;                 // the relay has said it is scanning
    std::shared_ptr<WslRadio> radio;  // while AirDrop on this PC is on
    unsigned radio_generation = 0;
    std::wstring radio_state;
    RadioLevel radio_level = RadioLevel::Off;
    HICON plain_icon = nullptr;
    HICON badges[4] = {};  // by RadioLevel: the icon with its dot (Off: none)
    std::filesystem::path home;  // %LOCALAPPDATA%\HowlDrop
    std::string name;            // this PC's name, for the share sheet
    ULONGLONG last_purge = 0;
    bool restart_radio = false;  // start it again once the old one has stopped
    int radios_stopping = 0;     // turned off, still giving the stick back
    Transfers transfers;
    std::unique_ptr<Cards> cards;  // the transfer cards, when Direct2D works
    std::unique_ptr<HowlWindow> howl_window;
    std::filesystem::path history_file;  // %LOCALAPPDATA%\HowlDrop\history.tsv
    std::set<std::string> logged;        // transfers already in it
    std::wstring click_target;  // what clicking the last notification opens
    bool click_selects = false;  // show it in its folder instead of opening it
};

App* g_app = nullptr;
std::mutex g_log_mu;  // howldrop.log: lines come from the node's threads too

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
        std::string value = line.substr(eq + 1);
        if (!value.empty() && value.back() == '\r') value.pop_back();
        if (key == "receive" && (value == "everyone" || value == "ask" || value == "contacts")) {
            app.settings.receive = value;
        } else if (key == "senders") {
            app.settings.senders = value;
        }
        if (key == "notify") app.settings.notify = on;
        else if (key == "open_links") app.settings.open_links = on;
        else if (key == "open_media") app.settings.open_media = on;
        else if (key == "open_documents") app.settings.open_documents = on;
        else if (key == "share_clipboard") app.settings.share_clipboard = on;
        else if (key == "send_to_menu") app.settings.send_to_menu = on;
        else if (key == "radio") app.settings.radio = on;
        else if (key == "cleanup") app.settings.cleanup = on;
    }
}

void save_settings(const App& app) {
    std::ofstream out(app.settings_file, std::ios::trunc);
    out << "notify=" << app.settings.notify << '\n'
        << "open_links=" << app.settings.open_links << '\n'
        << "open_media=" << app.settings.open_media << '\n'
        << "open_documents=" << app.settings.open_documents << '\n'
        << "share_clipboard=" << app.settings.share_clipboard << '\n'
        << "send_to_menu=" << app.settings.send_to_menu << '\n'
        << "radio=" << app.settings.radio << '\n'
        << "cleanup=" << app.settings.cleanup << '\n'
        << "receive=" << app.settings.receive << '\n'
        << "senders=" << app.settings.senders << '\n';
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
    // The HowlDrop window is open: it says this itself.
    if (app.howl_window && app.howl_window->showing()) return;
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

bool cards_on(const App& app) { return app.cards && app.cards->available(); }

// Finished transfers go into the window's Recent list, once each.
void record_history(App& app) {
    const std::int64_t now = std::chrono::duration_cast<std::chrono::seconds>(
                                 std::chrono::system_clock::now().time_since_epoch())
                                 .count();
    for (const auto& t : app.transfers.all()) {
        if (t.stage != Transfers::Stage::Done || app.logged.count(t.id)) continue;
        app.logged.insert(t.id);
        if (t.direction == Transfers::Direction::Out) {
            for (const auto& name : t.names) history::append(app.history_file, {now, true, t.peer, name});
        } else {
            for (const auto& saved : t.saved) {
                if (saved.is_absolute()) history::append(app.history_file, {now, false, t.peer, path_to_utf8(saved)});
            }
        }
    }
}

// The transfers changed: the cards, the history and the window follow.
void update_cards(App& app) {
    record_history(app);
    if (cards_on(app)) app.cards->update(app.transfers, app.settings.notify);
    if (app.howl_window) app.howl_window->refresh();
}

void handle_drop(App& app, const Drop& drop) {
    const std::wstring from = drop.from.empty() ? L"" : L" from " + wide(drop.from);
    if (drop.is_text) {
        if (drop.to_clipboard && app.clipboard) app.clipboard->set_text(drop.text);
        if (cards_on(app)) {
            // From Home Assistant the text has no file here: a stand-in name.
            const std::filesystem::path where = drop.path.empty() ? std::filesystem::path("text") / "text.txt"
                                                                  : drop.path;
            app.transfers.received(where, drop.from, "text", !drop.radio, Transfers::Clock::now());
            update_cards(app);
            if (!drop.radio && app.howl_window) app.howl_window->raise();
            return;
        }
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
    if (cards_on(app)) {
        app.transfers.received(drop.path, drop.from, drop.kind, !drop.radio, Transfers::Clock::now());
        update_cards(app);
        if (!drop.radio && app.howl_window) app.howl_window->raise();  // from Home Assistant: just arrived
        return;
    }
    std::wstring title = wide(kind_word(kind)) + from;
    if (opened) title += L", opened";
    notify(app, title, message);
}

// A card's Open, by the rules a notification's click had: a link's shortcut
// only as its http(s) address, and nothing that runs (open_policy.hpp): that
// is shown in its folder instead.
void open_from_card(const std::filesystem::path& file) {
    const OpenKind kind = classify(file);
    if (kind == OpenKind::Link) {
        const std::string url = shortcut_url(file);
        if (!url.empty()) open_target(wide(url));
        else show_in_folder(file.wstring());
    } else if (kind == OpenKind::Other) {
        show_in_folder(file.wstring());
    } else {
        open_target(file.wstring());
    }
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
    if (cards_on(app)) {
        AppendMenuW(menu, MF_STRING, kMenuOpenWindow, L"Open HowlDrop");
        SetMenuDefaultItem(menu, kMenuOpenWindow, FALSE);
    }
    AppendMenuW(menu, MF_STRING, kMenuOpenFolder, L"Open HowlDrop folder");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, check(app.settings.radio), kMenuRadio, L"AirDrop on this PC (USB Wi-Fi stick)");
    if (app.settings.radio && !app.radio_state.empty()) {
        AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, (L"      " + app.radio_state).c_str());
    }
    HMENU who = CreatePopupMenu();
    const std::string& receive = app.settings.receive;
    AppendMenuW(who, check(receive == "everyone"), kMenuReceiveEveryone, L"Everyone, without asking");
    AppendMenuW(who, check(receive == "ask"), kMenuReceiveAsk, L"Everyone, but ask me (contacts without asking)");
    AppendMenuW(who, check(receive == "contacts") | (app.settings.senders.empty() ? MF_GRAYED : 0),
                kMenuReceiveContacts,
                app.settings.senders.empty() ? L"Contacts only (add contacts in the HowlDrop window)"
                                             : L"Contacts only");
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(who), L"Who can send to this PC");
    AppendMenuW(menu, check(app.settings.cleanup), kMenuCleanup,
                (L"Delete files here after " + std::to_wstring(kKeepDays) + L" days").c_str());
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    AppendMenuW(menu, check(app.settings.notify), kMenuNotify, L"Show notifications");
    AppendMenuW(menu, check(app.settings.open_links), kMenuOpenLinks, L"Open links in the browser");
    AppendMenuW(menu, check(app.settings.open_media), kMenuOpenMedia, L"Open photos and videos");
    AppendMenuW(menu, check(app.settings.open_documents), kMenuOpenDocuments, L"Open PDFs and text files");
    AppendMenuW(menu, MF_SEPARATOR, 0, nullptr);
    HMENU send = CreatePopupMenu();
    const Node::Nearby nearby = app.node ? app.node->iphone_nearby() : Node::Nearby{};
    app.menu_devices.clear();
    if (!nearby.relay) {
        AppendMenuW(send, MF_STRING | MF_GRAYED, 0,
                    app.settings.radio ? L"Waiting for AirDrop on this PC to start"
                                       : L"Turn on AirDrop on this PC, or Home Assistant's send_to_iphone");
    } else {
        AppendMenuW(send, MF_STRING | MF_GRAYED, 0, nearby.local ? L"Through this PC" : L"Through Home Assistant");
        AppendMenuW(send, MF_STRING, kMenuSendIphone, L"Any nearby iPhone...");
        AppendMenuW(send, MF_SEPARATOR, 0, nullptr);
        for (const auto& name : nearby.devices) {
            if (app.menu_devices.size() >= kMenuDeviceMax) break;
            AppendMenuW(send, MF_STRING, kMenuDeviceBase + static_cast<UINT>(app.menu_devices.size()),
                        (wide(name) + L"...").c_str());
            app.menu_devices.push_back(name);
        }
        if (nearby.devices.empty()) AppendMenuW(send, MF_STRING | MF_GRAYED, 0, L"No devices heard yet");
        AppendMenuW(send, MF_SEPARATOR, 0, nullptr);
        AppendMenuW(send, MF_STRING | (nearby.scanning ? MF_GRAYED : 0), kMenuScan,
                    nearby.scanning ? L"Looking for devices..." : L"Look for devices");
    }
    AppendMenuW(menu, MF_POPUP, reinterpret_cast<UINT_PTR>(send), L"Send to iPhone");
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
    link->SetDescription(L"AirDrop to your iPhone with HowlDrop");
    IPersistFile* file = nullptr;
    if (SUCCEEDED(link->QueryInterface(IID_IPersistFile, reinterpret_cast<void**>(&file)))) {
        file->Save(shortcut.c_str(), TRUE);
        file->Release();
    }
    link->Release();
}

void send_files(App& app, const std::vector<std::filesystem::path>& files, const std::string& to);

// Pick files and send each in the background, to the device named to
// (empty: any nearby iPhone); on_sent reports.
void send_files_to_iphone(App& app, const std::string& to) {
    std::vector<wchar_t> buffer(64 * 1024, L'\0');
    OPENFILENAMEW ofn{};
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = app.window;
    ofn.lpstrFile = buffer.data();
    ofn.nMaxFile = static_cast<DWORD>(buffer.size());
    const std::wstring title = (to.empty() ? std::wstring(L"AirDrop to iPhone") : L"AirDrop to " + wide(to)) +
                               L" (set it to Everyone for 10 Minutes)";
    ofn.lpstrTitle = title.c_str();
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
    send_files(app, files, to);
}

// Send files to the iPhone in the background (this PC's radio, or the
// add-on's relay), to the device named to (empty: any nearby iPhone).
void send_files(App& app, const std::vector<std::filesystem::path>& files, const std::string& to) {
    Node* node = app.node.get();
    if (!node || files.empty()) return;
    // A card for each at once, before the phone is even found.
    const bool local = node->iphone_nearby().local;
    for (const auto& file : files) {
        app.transfers.sending(filename_utf8(file), to, !local, Transfers::Clock::now());
    }
    update_cards(app);
    app.senders.emplace_back([node, files, to]() {
        for (const auto& file : files) node->send_to_iphone(file, to);
    });
}

// Look for devices: the relay's scan takes about 30 s; the timer notifies
// what it found (check_scan).
void look_for_devices(App& app) {
    Node* node = app.node.get();
    const Status status = node->request_iphone_scan();
    app.click_target.clear();
    const bool notify_was = app.settings.notify;
    app.settings.notify = true;
    if (status.ok) {
        app.scan_asked = GetTickCount64();
        app.scan_seen = false;
        notify(app, L"Looking for nearby devices",
               L"About 30 seconds. iPhones show up while their AirDrop is set to Everyone for 10 Minutes.");
    } else {
        notify(app, L"Can't look for devices", wide(status.message));
    }
    app.settings.notify = notify_was;
}

void check_scan(App& app) {
    if (!app.scan_asked || !app.node) return;
    const Node::Nearby nearby = app.node->iphone_nearby();
    if (nearby.scanning) {
        app.scan_seen = true;
        return;
    }
    const bool late = GetTickCount64() - app.scan_asked > 120 * 1000;
    if (!app.scan_seen && !late) return;
    app.scan_asked = 0;
    std::wstring found;
    for (const auto& name : nearby.devices) found += (found.empty() ? L"" : L", ") + wide(name);
    app.click_target.clear();
    const bool notify_was = app.settings.notify;
    app.settings.notify = true;
    notify(app, found.empty() ? L"No devices found" : L"Nearby: right-click HowlDrop, Send to iPhone",
           found.empty() ? L"Set the iPhone's AirDrop to Everyone for 10 Minutes, keep it unlocked, and look again."
                         : found);
    app.settings.notify = notify_was;
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

// --- AirDrop on this PC, and the inbox's cleanup ---------------------------------

void post_drop(Drop* drop);

void log_line(const App& app, const std::string& line) {
    const std::lock_guard<std::mutex> hold(g_log_mu);
    std::ofstream out(app.log_file, std::ios::app);
    out << line << '\n';
}

// The tray icon with a dot in its lower right corner: amber while AirDrop on
// this PC starts, green on the share sheet, red when it needs the user.
// Drawn into the icon's own 32-bit pixels, alpha and all.
HICON badged(HICON base, COLORREF color) {
    ICONINFO info{};
    if (!GetIconInfo(base, &info)) return nullptr;
    BITMAP bm{};
    GetObjectW(info.hbmColor ? info.hbmColor : info.hbmMask, sizeof(bm), &bm);
    const int w = bm.bmWidth;
    const int h = info.hbmColor ? bm.bmHeight : bm.bmHeight / 2;
    BITMAPINFO bi{};
    bi.bmiHeader.biSize = sizeof(bi.bmiHeader);
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h;  // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi.bmiHeader.biCompression = BI_RGB;
    std::vector<std::uint32_t> px(static_cast<std::size_t>(w) * h, 0);
    HDC dc = GetDC(nullptr);
    bool alpha = false;
    if (info.hbmColor && GetDIBits(dc, info.hbmColor, 0, h, px.data(), &bi, DIB_RGB_COLORS)) {
        for (auto p : px) alpha = alpha || (p >> 24) != 0;
    }
    if (!alpha) {  // an icon without alpha: opaque wherever its mask says so
        std::vector<std::uint32_t> mask(px.size(), 0);
        GetDIBits(dc, info.hbmMask, 0, h, mask.data(), &bi, DIB_RGB_COLORS);
        for (std::size_t i = 0; i < px.size(); ++i) px[i] = (mask[i] & 0xffffff) ? 0 : (px[i] | 0xff000000u);
    }
    const double r = w * 0.22;
    const double cx = w - r - 0.5;
    const double cy = h - r - 0.5;
    const std::uint32_t fill =
        0xff000000u | (GetRValue(color) << 16) | (GetGValue(color) << 8) | GetBValue(color);
    for (int y = 0; y < h; ++y) {
        for (int x = 0; x < w; ++x) {
            const double d = std::sqrt((x - cx) * (x - cx) + (y - cy) * (y - cy));
            if (d <= r) px[static_cast<std::size_t>(y) * w + x] = fill;
            else if (d <= r + 1.2) px[static_cast<std::size_t>(y) * w + x] = 0xff202020u;  // a rim
        }
    }
    HBITMAP color_bm = CreateCompatibleBitmap(dc, w, h);
    SetDIBits(dc, color_bm, 0, h, px.data(), &bi, DIB_RGB_COLORS);
    ReleaseDC(nullptr, dc);
    HBITMAP mask_bm = CreateBitmap(w, h, 1, 1, nullptr);  // all zero: alpha decides
    ICONINFO out{};
    out.fIcon = TRUE;
    out.hbmColor = color_bm;
    out.hbmMask = mask_bm;
    HICON icon = CreateIconIndirect(&out);
    DeleteObject(color_bm);
    DeleteObject(mask_bm);
    if (info.hbmColor) DeleteObject(info.hbmColor);
    if (info.hbmMask) DeleteObject(info.hbmMask);
    return icon;
}

void set_tip(App& app) {
    std::wstring tip = L"HowlDrop: files go to " + app.inbox.wstring();
    if (app.settings.radio && !app.radio_state.empty()) tip = L"HowlDrop: " + app.radio_state;
    copy_into(app.icon.szTip, ARRAYSIZE(app.icon.szTip), tip);
    const auto level = app.radio ? app.radio_level : RadioLevel::Off;
    HICON badge = app.badges[static_cast<int>(level)];
    app.icon.hIcon = badge ? badge : app.plain_icon;
    NOTIFYICONDATAW data = app.icon;
    data.uFlags = NIF_TIP | NIF_SHOWTIP | NIF_ICON;
    Shell_NotifyIconW(NIM_MODIFY, &data);
}

void post_radio(unsigned generation, std::string state, int up, RadioLevel level = RadioLevel::Working) {
    auto* note = new RadioNote{generation, std::move(state), up, level};
    if (!g_app || !g_app->window || !PostMessageW(g_app->window, kRadioMessage, 0, reinterpret_cast<LPARAM>(note)))
        delete note;
}

void start_radio(App& app) {
    if (app.radio) return;
    if (app.radios_stopping > 0) {
        // The stick is still on its way back from the last one.
        app.restart_radio = true;
        return;
    }
    const unsigned generation = ++app.radio_generation;
    WslRadioConfig config;
    config.inbox = app.inbox;
    config.home = app.home;
    config.name = app.name;
    config.receive = app.settings.receive;
    config.senders = app.settings.senders;
    config.on_event = [](const std::vector<std::string>& event) {
        auto* copy = new std::vector<std::string>(event);
        if (!g_app || !g_app->window || !PostMessageW(g_app->window, kTransferMessage, 0, reinterpret_cast<LPARAM>(copy)))
            delete copy;
    };
    config.on_state = [generation](const std::string& state, RadioLevel level) {
        post_radio(generation, state, -1, level);
    };
    config.on_up = [generation](bool up) { post_radio(generation, {}, up ? 1 : 0); };
    config.on_received = [](const std::filesystem::path& saved, const std::string& from, const std::string& kind) {
        if (kind != "text") {
            post_drop(new Drop{saved, {}, from, false, false, true, kind});
            return;
        }
        // Text: onto the clipboard, as from Home Assistant.
        std::ifstream in(saved, std::ios::binary);
        std::string text(kMaxClipBytes, '\0');
        in.read(text.data(), static_cast<std::streamsize>(text.size()));
        text.resize(static_cast<std::size_t>(in.gcount()));
        post_drop(new Drop{saved, text, from, true, true, true, kind});
    };
    config.on_sent = [](const std::string& file, const Status& status) { post_sent(file, status); };
    config.ask = [](const std::wstring& question) {
        return MessageBoxW(nullptr, question.c_str(), L"HowlDrop",
                           MB_YESNO | MB_ICONQUESTION | MB_SETFOREGROUND | MB_TOPMOST) == IDYES;
    };
    app.radio = std::make_shared<WslRadio>(std::move(config));
    app.radio_state = L"Starting";
    app.radio_level = RadioLevel::Working;
    set_tip(app);
    app.radio->start();
}

// wait: on quit. Otherwise it stops in the background (up to a minute).
void stop_radio(App& app, bool wait) {
    if (!app.radio) return;
    if (app.node) app.node->set_local_iphone(nullptr);
    std::shared_ptr<WslRadio> radio = std::move(app.radio);
    ++app.radio_generation;  // what the old one still says is not shown
    app.radio_state.clear();
    app.radio_level = RadioLevel::Off;
    set_tip(app);
    if (wait) {
        radio->stop();
    } else {
        ++app.radios_stopping;
        HWND window = app.window;
        app.senders.emplace_back([radio, window]() {
            radio->stop();
            PostMessageW(window, kRadioStoppedMessage, 0, 0);
        });
    }
}

// Who can send changed: the receiver reads it as it starts.
void restart_radio(App& app) {
    if (!app.radio) return;
    stop_radio(app, false);
    start_radio(app);  // after the old one has stopped (restart_radio)
}

void handle_radio(App& app, const RadioNote& note) {
    if (note.generation != app.radio_generation || !app.radio) return;
    if (note.up == 1) {
        if (app.node) app.node->set_local_iphone(app.radio);
        notify(app, L"AirDrop on this PC is on", L"This PC is on the iPhone's share sheet as " + wide(app.name) + L".");
    } else if (note.up == 0) {
        if (app.node) app.node->set_local_iphone(nullptr);
    }
    if (!note.state.empty()) {
        app.radio_state = wide(note.state);
        app.radio_level = note.level;
        set_tip(app);
        log_line(app, "AirDrop on this PC: " + note.state);
    }
}

// --- the HowlDrop window ----------------------------------------------------------

WindowSettings window_settings(const App& app) {
    WindowSettings s;
    s.radio = app.settings.radio;
    s.cleanup = app.settings.cleanup;
    s.notify = app.settings.notify;
    s.open_links = app.settings.open_links;
    s.open_media = app.settings.open_media;
    s.open_documents = app.settings.open_documents;
    s.start_with_windows = starts_with_windows();
    s.receive = app.settings.receive;
    s.senders = app.settings.senders;
    return s;
}

void purge_inbox(App& app);

// The window's settings page changed something: as the menu would.
void apply_window_settings(App& app, const WindowSettings& s) {
    const bool radio_changed = s.radio != app.settings.radio;
    const bool who_changed = s.receive != app.settings.receive || s.senders != app.settings.senders;
    const bool cleanup_on = s.cleanup && !app.settings.cleanup;
    app.settings.radio = s.radio;
    app.settings.cleanup = s.cleanup;
    app.settings.notify = s.notify;
    app.settings.open_links = s.open_links;
    app.settings.open_media = s.open_media;
    app.settings.open_documents = s.open_documents;
    app.settings.receive = s.receive;
    app.settings.senders = s.senders;
    if (s.start_with_windows != starts_with_windows()) set_start_with_windows(s.start_with_windows);
    save_settings(app);
    if (radio_changed) {
        if (s.radio) start_radio(app);
        else stop_radio(app, false);
    } else if (who_changed) {
        restart_radio(app);
    }
    if (cleanup_on) {
        app.last_purge = 0;
        purge_inbox(app);
    }
    if (app.howl_window) app.howl_window->refresh();
}

std::wstring status_text(const App& app) {
    if (app.settings.radio) {
        const std::string& r = app.settings.receive;
        const std::wstring who = r == "everyone" ? L"everyone can send" : r == "ask" ? L"asks first" : L"contacts only";
        return (app.radio_state.empty() ? std::wstring(L"Starting") : app.radio_state) + L" · " + who;
    }
    const Node::Nearby around = app.node ? app.node->iphone_nearby() : Node::Nearby{};
    if (around.relay) return L"Through Home Assistant";
    return L"AirDrop on this PC is off: turn it on in Settings";
}

// Once a day while the cleanup setting is on.
void purge_inbox(App& app) {
    if (!app.settings.cleanup) return;
    const ULONGLONG now = GetTickCount64();
    if (app.last_purge && now - app.last_purge < 24ull * 3600 * 1000) return;
    app.last_purge = now;
    const int gone = purge_old_files(app.inbox, kKeepDays);
    if (gone > 0) {
        log_line(app, "deleted " + std::to_string(gone) + " file(s) older than " + std::to_string(kKeepDays) +
                          " days from " + path_to_utf8(app.inbox));
    }
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
                    if (app->howl_window && cards_on(*app)) app->howl_window->show();
                    else open_folder(*app);
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
            if (cards_on(*app)) {
                app->transfers.sent(sent->file, sent->ok, sent->message, Transfers::Clock::now());
                update_cards(*app);
                return 0;
            }
            app->click_target.clear();
            // Said even with notifications off: it answers what was just asked.
            const bool notify_was = app->settings.notify;
            app->settings.notify = true;
            if (sent->ok) {
                notify(*app, L"Sent to your iPhone",
                       wide(sent->file) + L": accept it on the iPhone (AirDrop set to Everyone for 10 Minutes).");
            } else {
                notify(*app, L"Not sent to the iPhone", wide(sent->file) + L": " + wide(sent->message));
            }
            app->settings.notify = notify_was;
            return 0;
        }
        case kRadioMessage: {
            std::unique_ptr<RadioNote> note(reinterpret_cast<RadioNote*>(lparam));
            handle_radio(*app, *note);
            return 0;
        }
        case kTransferMessage: {
            std::unique_ptr<std::vector<std::string>> event(reinterpret_cast<std::vector<std::string>*>(lparam));
            if (!app->transfers.apply(*event, Transfers::Clock::now()).empty()) update_cards(*app);
            // An AirDrop coming in: an open window comes to the top to show it.
            const std::string& what = event->empty() ? std::string() : event->front();
            if ((what == "ask" || what == "incoming") && app->howl_window) app->howl_window->raise();
            return 0;
        }
        case kRadioStoppedMessage:
            if (app->radios_stopping > 0) --app->radios_stopping;
            if (app->radios_stopping == 0 && app->restart_radio) {
                app->restart_radio = false;
                if (app->settings.radio) start_radio(*app);
            }
            return 0;
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
                case kMenuOpenWindow:
                    if (app->howl_window) app->howl_window->show();
                    return 0;
                case kMenuRadio:
                    app->settings.radio = !app->settings.radio;
                    if (app->settings.radio) start_radio(*app);
                    else stop_radio(*app, false);
                    break;
                case kMenuCleanup:
                    app->settings.cleanup = !app->settings.cleanup;
                    app->last_purge = 0;
                    purge_inbox(*app);
                    break;
                case kMenuReceiveEveryone:
                case kMenuReceiveAsk:
                case kMenuReceiveContacts: {
                    const UINT id = LOWORD(wparam);
                    const std::string mode = id == kMenuReceiveEveryone ? "everyone"
                                             : id == kMenuReceiveAsk    ? "ask"
                                                                        : "contacts";
                    if (mode != app->settings.receive) {
                        app->settings.receive = mode;
                        restart_radio(*app);
                    }
                    break;
                }
                case kMenuNotify: app->settings.notify = !app->settings.notify; break;
                case kMenuOpenLinks: app->settings.open_links = !app->settings.open_links; break;
                case kMenuOpenMedia: app->settings.open_media = !app->settings.open_media; break;
                case kMenuOpenDocuments: app->settings.open_documents = !app->settings.open_documents; break;
                case kMenuShareClipboard:
                    app->settings.share_clipboard = !app->settings.share_clipboard;
                    if (app->node) app->node->set_share_clipboard(app->settings.share_clipboard);
                    break;
                case kMenuCopyKey: copy_pair_key(*app); return 0;
                case kMenuSendIphone: send_files_to_iphone(*app, {}); return 0;
                case kMenuScan: look_for_devices(*app); return 0;
                case kMenuSendToMenu:
                    app->settings.send_to_menu = !app->settings.send_to_menu;
                    update_send_to(*app);
                    break;
                case kMenuStartup: set_start_with_windows(!starts_with_windows()); break;
                case kMenuQuit: DestroyWindow(window); return 0;
                default: {
                    const UINT id = LOWORD(wparam);
                    if (id >= kMenuDeviceBase && id < kMenuDeviceBase + app->menu_devices.size()) {
                        send_files_to_iphone(*app, app->menu_devices[id - kMenuDeviceBase]);
                    }
                    return 0;
                }
            }
            save_settings(*app);
            return 0;
        case WM_TIMER:  // `howldrop stop` ends the node; the icon goes with it
            if (app->node && !app->node->running()) {
                DestroyWindow(window);
                return 0;
            }
            check_scan(*app);
            purge_inbox(*app);
            if (!app->transfers.expire(Transfers::Clock::now()).empty()) update_cards(*app);
            if (app->howl_window) app->howl_window->refresh();  // status, nearby devices
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
    // Sharp text in the cards and the window on any monitor's scale.
    ui::enable_dpi_awareness();
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
    app.history_file = dir / L"history.tsv";
    app.log_file = dir / L"howldrop.log";
    app.home = dir;
    app.name = config.name;
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
    app.plain_icon = app.icon.hIcon;
    app.badges[static_cast<int>(RadioLevel::Working)] = badged(app.plain_icon, RGB(245, 166, 35));
    app.badges[static_cast<int>(RadioLevel::Up)] = badged(app.plain_icon, RGB(52, 199, 89));
    app.badges[static_cast<int>(RadioLevel::Problem)] = badged(app.plain_icon, RGB(255, 59, 48));
    app.icon.hBalloonIcon = static_cast<HICON>(LoadImageW(instance, MAKEINTRESOURCEW(kIconResource), IMAGE_ICON,
                                                          GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON), 0));
    copy_into(app.icon.szTip, ARRAYSIZE(app.icon.szTip), L"HowlDrop: files go to " + app.inbox.wstring());
    add_icon(app);

    // From Home Assistant (or another HowlDrop PC), through the node.
    config.on_file = [](const std::filesystem::path& saved, const std::string& from) {
        post_drop(new Drop{saved, {}, from, false, false, false, {}});
    };
    config.on_text = [](const std::string& text, const std::string& from) {
        post_drop(new Drop{{}, text, from, true, false, false, {}});
    };
    config.on_sent = [](const std::string& file, const Status& status) { post_sent(file, status); };
    const std::filesystem::path log_file = app.log_file;
    config.on_log = [log_file](const std::string& line) {
        {
            const std::lock_guard<std::mutex> hold(g_log_mu);
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
    // The transfer cards (COM first: thumbnails come from the shell).
    {
        const auto downloads = app.inbox.parent_path().filename().wstring();
        const std::wstring label = (downloads.empty() ? std::wstring() : downloads + L"\\") +
                                   app.inbox.filename().wstring();
        Cards::Actions actions;
        actions.decide = [](const std::string& id, bool accept) {
            if (g_app && g_app->radio) g_app->radio->decide(id, accept);
        };
        actions.open = [](const std::filesystem::path& file) { open_from_card(file); };
        actions.show = [](const std::filesystem::path& file) { show_in_folder(file.wstring()); };
        actions.retired = [](const std::string& id) {
            if (g_app) g_app->transfers.forget(id);
        };
        app.cards = std::make_unique<Cards>(instance, label, std::move(actions));
    }
    // The HowlDrop window, opened from the icon.
    {
        HowlWindow::Hooks hooks;
        hooks.settings = []() { return g_app ? window_settings(*g_app) : WindowSettings{}; };
        hooks.apply = [](const WindowSettings& s) {
            if (g_app) apply_window_settings(*g_app, s);
        };
        hooks.nearby = []() { return g_app && g_app->node ? g_app->node->iphone_nearby() : Node::Nearby{}; };
        hooks.scan = []() {
            if (g_app) look_for_devices(*g_app);
        };
        hooks.send = [](const std::vector<std::filesystem::path>& files, const std::string& to) {
            if (g_app) send_files(*g_app, files, to);
        };
        hooks.open = [](const std::filesystem::path& file) { open_from_card(file); };
        hooks.show = [](const std::filesystem::path& file) { show_in_folder(file.wstring()); };
        hooks.open_folder = []() {
            if (g_app) open_folder(*g_app);
        };
        hooks.status = []() { return g_app ? status_text(*g_app) : std::wstring(); };
        hooks.level = []() { return g_app && g_app->radio ? g_app->radio_level : RadioLevel::Off; };
        hooks.transfers = []() {
            std::vector<Transfers::Transfer> out;
            if (!g_app) return out;
            const auto now = Transfers::Clock::now();
            for (const auto& t : g_app->transfers.all()) {
                // Under way, or just over (a card would still show it).
                if (!t.finished() || now - t.updated < std::chrono::seconds(10)) out.push_back(t);
            }
            return out;
        };
        hooks.decide = [](const std::string& id, bool accept) {
            if (g_app && g_app->radio) g_app->radio->decide(id, accept);
        };
        hooks.on_visibility = [](bool showing) {
            if (g_app && g_app->cards) g_app->cards->set_hidden(showing);
        };
        app.howl_window = std::make_unique<HowlWindow>(instance, wide(app.name), app.history_file, std::move(hooks));
    }
    update_send_to(app);
    purge_inbox(app);
    if (app.settings.radio) start_radio(app);
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
    stop_radio(app, true);     // the stick goes back to Windows
    app.node->request_stop();  // a send in progress stops at its next chunk
    for (auto& sender : app.senders) {
        if (sender.joinable()) sender.join();
    }
    app.node->stop();
    app.howl_window.reset();  // thumbnails are COM objects
    app.cards.reset();
    g_app = nullptr;
    if (com) CoUninitialize();
    if (single) CloseHandle(single);
    return 0;
}

}  // namespace howldrop
