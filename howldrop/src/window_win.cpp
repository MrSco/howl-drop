// The HowlDrop window (window_win.hpp).

#include "window_win.hpp"

#include "cards_win.hpp"
#include "history.hpp"
#include "text.hpp"
#include "transfers.hpp"
#include "ui_win.hpp"

#include <commdlg.h>
#include <shellapi.h>
#include <windowsx.h>

#include <algorithm>
#include <cmath>
#include <cwchar>
#include <map>

namespace howldrop {
namespace {

const wchar_t* const kWindowClass = L"HowlDropWindow";
constexpr float kWidth = 420.0f;
constexpr float kHeight = 700.0f;
constexpr float kSide = 20.0f;
constexpr float kRecentRow = 52.0f;
constexpr float kDeviceRow = 44.0f;
constexpr std::size_t kDevicesShown = 4;
constexpr std::size_t kRecentLoaded = 60;
constexpr int kEditId = 101;

enum Hit {
    kNothing = -1,
    kGear,
    kBack,
    kDrop,
    kClear,
    kScan,
    kDevice,   // index: -1 any nearby iPhone, else the device
    kRecent,   // index into recent
    kFolder,
    kToggle,   // index: Toggle below
    kReceive,  // index: 0 everyone, 1 ask, 2 contacts
    kSave,
    kNowAccept,   // index into now
    kNowDecline,
    kNowRow,
};

constexpr UINT_PTR kAnimate = 7;  // the Now list's moving bars

enum Toggle { kRadio, kNotify, kLinks, kMedia, kDocuments, kCleanup, kStartup, kToggles };

struct Area {
    D2D1_RECT_F rect;
    int hit;
    int index;
};

std::wstring wide(const std::string& s) {
    auto w = wide_from_utf8(s);
    return w ? *w : std::wstring();
}

bool inside(const D2D1_RECT_F& r, float x, float y) { return x >= r.left && x < r.right && y >= r.top && y < r.bottom; }

// "2:31 PM" today, "Yesterday", else the date, in the user's format.
std::wstring when(std::int64_t t) {
    const unsigned long long ticks = (static_cast<unsigned long long>(t) + 11644473600ull) * 10000000ull;
    FILETIME utc{static_cast<DWORD>(ticks & 0xffffffffu), static_cast<DWORD>(ticks >> 32)};
    FILETIME local;
    SYSTEMTIME then, now;
    FileTimeToLocalFileTime(&utc, &local);
    FileTimeToSystemTime(&local, &then);
    GetLocalTime(&now);
    wchar_t buf[64] = L"";
    if (then.wYear == now.wYear && then.wMonth == now.wMonth && then.wDay == now.wDay) {
        GetTimeFormatEx(LOCALE_NAME_USER_DEFAULT, TIME_NOSECONDS, &then, nullptr, buf, 64);
        return buf;
    }
    FILETIME nf;
    SystemTimeToFileTime(&now, &nf);
    const auto big = [](FILETIME f) { return (static_cast<unsigned long long>(f.dwHighDateTime) << 32) | f.dwLowDateTime; };
    if (big(nf) - big(local) < 2ull * 86400 * 10000000ull && now.wDay != then.wDay) {
        // Within two days and not today: yesterday, around enough for a list.
        return L"Yesterday";
    }
    GetDateFormatEx(LOCALE_NAME_USER_DEFAULT, DATE_SHORTDATE, &then, nullptr, buf, 64, nullptr);
    return buf;
}

std::vector<std::filesystem::path> pick_files(HWND owner) {
    std::vector<wchar_t> buffer(64 * 1024, L'\0');
    OPENFILENAMEW ofn{};
    ofn.lStructSize = sizeof(ofn);
    ofn.hwndOwner = owner;
    ofn.lpstrFile = buffer.data();
    ofn.nMaxFile = static_cast<DWORD>(buffer.size());
    ofn.lpstrTitle = L"AirDrop to iPhone";
    ofn.Flags = OFN_ALLOWMULTISELECT | OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST;
    if (!GetOpenFileNameW(&ofn)) return {};
    std::vector<std::filesystem::path> files;
    const std::wstring first(buffer.data());
    const wchar_t* next = buffer.data() + first.size() + 1;
    if (*next == L'\0') {
        files.emplace_back(first);
    } else {
        for (; *next; next += std::wcslen(next) + 1) files.push_back(std::filesystem::path(first) / next);
    }
    return files;
}

}  // namespace

struct HowlWindow::Impl {
    HINSTANCE instance;
    std::wstring name;
    std::filesystem::path history_file;
    Hooks hooks;
    HWND window = nullptr;
    std::unique_ptr<ui::Canvas> canvas;
    bool settings_page = false;
    std::vector<std::filesystem::path> pending;
    std::vector<history::Entry> recent;
    std::filesystem::file_time_type history_stamp{};
    std::map<std::string, IWICBitmapSource*> thumbs;  // nullptr: tried, none
    std::vector<Area> areas;
    int hot = kNothing;
    int hot_index = 0;
    bool tracking = false;
    float scroll = 0;       // the Recent list, in DIPs
    float recent_top = 0;   // where it starts, from the last paint
    float recent_bottom = 0;
    std::vector<Transfers::Transfer> now;  // the Now list, as last drawn
    double phase = 0;
    bool animating = false;
    bool was_showing = false;
    HWND edit = nullptr;
    HFONT edit_font = nullptr;
    HBRUSH edit_brush = nullptr;
    UINT edit_dpi = 0;

    ~Impl() {
        for (auto& [path, thumb] : thumbs) {
            if (thumb) thumb->Release();
        }
        if (window) DestroyWindow(window);
        if (edit_font) DeleteObject(edit_font);
        if (edit_brush) DeleteObject(edit_brush);
    }

    float scale() const { return static_cast<float>(ui::dpi_of(window)) / 96.0f; }

    bool showing() const { return window && IsWindowVisible(window) && !IsIconic(window); }

    // The tray hides the cards while the window shows the transfers.
    void tell_visibility() {
        const bool now_showing = showing();
        if (now_showing == was_showing) return;
        was_showing = now_showing;
        if (hooks.on_visibility) hooks.on_visibility(now_showing);
    }

    void reload_history() {
        std::error_code ec;
        const auto stamp = std::filesystem::last_write_time(history_file, ec);
        if (!ec && stamp == history_stamp && !recent.empty()) return;
        history_stamp = ec ? std::filesystem::file_time_type{} : stamp;
        recent = history::load(history_file, kRecentLoaded);
    }

    IWICBitmapSource* thumb_for(const history::Entry& e) {
        if (e.out) return nullptr;
        const auto kind = Transfers::kind_of(path_from_utf8(e.item));
        if (kind != "image" && kind != "video") return nullptr;
        auto it = thumbs.find(e.item);
        if (it != thumbs.end()) return it->second;
        IWICBitmapSource* made = ui::thumbnail(path_from_utf8(e.item), 96);
        thumbs[e.item] = made;
        return made;
    }

    void area(D2D1_RECT_F r, int hit, int index = 0) { areas.push_back({r, hit, index}); }
    bool is_hot(int hit, int index = 0) const { return hot == hit && hot_index == index; }

    // --- the main page ---------------------------------------------------------------

    // Transfers under way (and just finished), newest first: what the cards
    // would show, while the window is open. Returns where the page goes on.
    float paint_now(ui::Canvas& g, float y) {
        now = hooks.transfers ? hooks.transfers() : std::vector<Transfers::Transfer>{};
        std::reverse(now.begin(), now.end());
        if (now.size() > 3) now.resize(3);
        bool moving = false;
        const float right = g.width() - kSide;
        for (std::size_t i = 0; i < now.size(); ++i) {
            const auto& t = now[i];
            const int index = static_cast<int>(i);
            const CardText say = card_text(t);
            const bool asking = t.stage == Transfers::Stage::Asking;
            const bool active = !t.finished() && !asking;
            moving = moving || active || asking;
            float h = 56;
            if (active) h += 30;
            if (asking) h += 46;
            const D2D1_RECT_F row = D2D1::RectF(kSide, y, right, y + h);
            g.fill(row, 12, is_hot(kNowRow, index) && t.stage == Transfers::Stage::Done ? ui::theme::raised
                                                                                        : ui::theme::card);
            g.text(say.glyph, D2D1::RectF(row.left + 14, row.top + 12, row.left + 34, row.top + 34), ui::Font::Icon,
                   say.tint);
            g.text(say.title, D2D1::RectF(row.left + 42, row.top + 10, row.right - 14, row.top + 32), ui::Font::Heading,
                   ui::theme::text);
            std::wstring line = say.line;
            if (!t.names.empty() && t.kind != "text") {
                line += L" · " + wide(t.names.front());
                const std::size_t count = std::max(t.count, t.names.size());
                if (count > 1) line += L" and " + std::to_wstring(count - 1) + L" more";
            }
            g.text(line, D2D1::RectF(row.left + 42, row.top + 32, row.right - 14, row.top + 50), ui::Font::Small,
                   ui::theme::dim);
            float in = row.top + 56;
            if (active) {
                g.progress(D2D1::RectF(row.left + 14, in, row.right - 14, in + 6), card_fraction(t), phase);
                g.text(card_status(t), D2D1::RectF(row.left + 14, in + 9, row.right - 14, in + 27), ui::Font::Small,
                       ui::theme::dim);
            }
            if (asking) {
                const double left_s = std::chrono::duration<double>(t.deadline - Transfers::Clock::now()).count();
                g.progress(D2D1::RectF(row.left + 14, in, row.right - 14, in + 3),
                           std::clamp(left_s / Transfers::kAskTimeout.count(), 0.0, 1.0), 0, ui::theme::dim);
                const float half = (row.right - row.left - 28 - 10) / 2;
                const D2D1_RECT_F no = D2D1::RectF(row.left + 14, in + 10, row.left + 14 + half, in + 42);
                const D2D1_RECT_F yes = D2D1::RectF(row.right - 14 - half, in + 10, row.right - 14, in + 42);
                g.fill(no, 8, is_hot(kNowDecline, index) ? ui::theme::raised_hover : ui::theme::raised);
                g.text(L"Decline", no, ui::Font::Button, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
                g.fill(yes, 8, is_hot(kNowAccept, index) ? ui::theme::accent_hover : ui::theme::accent);
                g.text(L"Accept", yes, ui::Font::Button, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
                area(row, kNowRow, index);
                area(no, kNowDecline, index);
                area(yes, kNowAccept, index);
            } else {
                area(row, kNowRow, index);
            }
            y = row.bottom + 8;
        }
        // The bars move between the tray's refreshes.
        if (window && moving != animating) {
            animating = moving;
            if (moving) SetTimer(window, kAnimate, 40, nullptr);
            else KillTimer(window, kAnimate);
        }
        return now.empty() ? y : y + 6;
    }

    void paint_main(ui::Canvas& g) {
        const float w = g.width();
        const float right = w - kSide;
        // Header: status dot, name, how it's reachable; settings at the right.
        const RadioLevel level = hooks.level ? hooks.level() : RadioLevel::Off;
        const ui::Color dot = level == RadioLevel::Up        ? ui::theme::good
                              : level == RadioLevel::Working ? ui::theme::warn
                              : level == RadioLevel::Problem ? ui::theme::bad
                                                             : ui::theme::dim;
        g.dot(D2D1::Point2F(kSide + 5, 30), 5, dot);
        g.text(name, D2D1::RectF(kSide + 18, 16, right - 44, 44), ui::Font::Title, ui::theme::text);
        g.text(hooks.status ? hooks.status() : L"", D2D1::RectF(kSide, 44, right - 44, 62), ui::Font::Small,
               ui::theme::dim);
        const D2D1_RECT_F gear = D2D1::RectF(right - 34, 18, right, 52);
        if (is_hot(kGear)) g.fill(gear, 8, ui::theme::raised);
        g.text(ui::icon::kSettings, gear, ui::Font::Icon, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
        area(gear, kGear);

        // What's happening now, then the drop zone.
        float y = paint_now(g, 76);
        const D2D1_RECT_F zone = D2D1::RectF(kSide, y, right, y + 112);
        g.fill(zone, 12, is_hot(kDrop) ? ui::theme::raised : ui::theme::card);
        g.outline(zone, 12, ui::theme::faint);
        area(zone, kDrop);
        if (pending.empty()) {
            g.text(ui::icon::kUpload, D2D1::RectF(zone.left, y + 14, zone.right, y + 46), ui::Font::IconLarge,
                   ui::theme::accent, DWRITE_TEXT_ALIGNMENT_CENTER);
            g.text(L"Drop files here to AirDrop them", D2D1::RectF(zone.left, y + 52, zone.right, y + 74),
                   ui::Font::Body, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
            g.text(L"or click to choose", D2D1::RectF(zone.left, y + 76, zone.right, y + 94), ui::Font::Small,
                   ui::theme::dim, DWRITE_TEXT_ALIGNMENT_CENTER);
        } else {
            std::wstring names;
            for (std::size_t i = 0; i < pending.size() && i < 3; ++i) {
                names += (i ? L", " : L"") + pending[i].filename().wstring();
            }
            if (pending.size() > 3) names += L" and " + std::to_wstring(pending.size() - 3) + L" more";
            const std::wstring count = std::to_wstring(pending.size()) + (pending.size() == 1 ? L" file" : L" files") +
                                       L" ready";
            g.text(count, D2D1::RectF(zone.left + 16, y + 18, zone.right - 44, y + 40), ui::Font::Heading,
                   ui::theme::text);
            g.text(names, D2D1::RectF(zone.left + 16, y + 42, zone.right - 16, y + 80), ui::Font::Small,
                   ui::theme::dim, DWRITE_TEXT_ALIGNMENT_LEADING, true);
            g.text(L"Choose where to send them below", D2D1::RectF(zone.left + 16, y + 84, zone.right - 16, y + 102),
                   ui::Font::Small, ui::theme::accent);
            const D2D1_RECT_F clear = D2D1::RectF(zone.right - 36, y + 10, zone.right - 10, y + 36);
            if (is_hot(kClear)) g.fill(clear, 6, ui::theme::raised_hover);
            g.text(ui::icon::kClose, clear, ui::Font::Icon, ui::theme::dim, DWRITE_TEXT_ALIGNMENT_CENTER);
            area(clear, kClear);
        }
        y = zone.bottom + 22;

        // Send to: any nearby iPhone, and what the last scan heard.
        const Node::Nearby around = hooks.nearby ? hooks.nearby() : Node::Nearby{};
        g.text(L"Send to", D2D1::RectF(kSide, y, right - 150, y + 22), ui::Font::Heading, ui::theme::text);
        if (around.relay) {
            const D2D1_RECT_F scan = D2D1::RectF(right - 150, y, right, y + 22);
            g.text(around.scanning ? L"Looking…" : L"Look for devices", scan, ui::Font::Small,
                   around.scanning ? ui::theme::dim : (is_hot(kScan) ? ui::theme::accent_hover : ui::theme::accent),
                   DWRITE_TEXT_ALIGNMENT_TRAILING);
            if (!around.scanning) area(scan, kScan);
        }
        y += 30;
        if (!around.relay) {
            const D2D1_RECT_F note = D2D1::RectF(kSide, y, right, y + 54);
            g.fill(note, 10, ui::theme::card);
            g.text(L"Turn on AirDrop on this PC in Settings, or the Home Assistant add-on's send_to_iphone.",
                   D2D1::RectF(note.left + 14, note.top, note.right - 14, note.bottom), ui::Font::Small, ui::theme::dim,
                   DWRITE_TEXT_ALIGNMENT_LEADING, true);
            y = note.bottom + 22;
        } else {
            std::vector<std::wstring> rows = {L"Any nearby iPhone"};
            for (std::size_t i = 0; i < around.devices.size() && i < kDevicesShown; ++i) rows.push_back(wide(around.devices[i]));
            for (std::size_t i = 0; i < rows.size(); ++i) {
                const int index = static_cast<int>(i) - 1;
                const D2D1_RECT_F row = D2D1::RectF(kSide, y, right, y + kDeviceRow - 6);
                g.fill(row, 10, is_hot(kDevice, index) ? ui::theme::raised_hover : ui::theme::card);
                g.text(ui::icon::kPhone, D2D1::RectF(row.left + 12, row.top, row.left + 36, row.bottom), ui::Font::Icon,
                       ui::theme::accent);
                g.text(rows[i], D2D1::RectF(row.left + 44, row.top, row.right - 110, row.bottom), ui::Font::Body,
                       ui::theme::text);
                g.text(pending.empty() ? L"Choose files…" : L"Send", D2D1::RectF(row.right - 110, row.top, row.right - 14,
                                                                                 row.bottom),
                       ui::Font::Small, pending.empty() ? ui::theme::dim : ui::theme::accent,
                       DWRITE_TEXT_ALIGNMENT_TRAILING);
                area(row, kDevice, index);
                y += kDeviceRow;
            }
            if (around.devices.empty() && !around.scanning) {
                g.text(L"Set the iPhone's AirDrop to Everyone for 10 Minutes, then Look for devices.",
                       D2D1::RectF(kSide + 4, y - 2, right, y + 16), ui::Font::Small, ui::theme::dim);
                y += 18;
            }
            y += 16;
        }

        // Recent, scrolling in what's left.
        g.text(L"Recent", D2D1::RectF(kSide, y, right - 120, y + 22), ui::Font::Heading, ui::theme::text);
        const D2D1_RECT_F folder = D2D1::RectF(right - 120, y, right, y + 22);
        g.text(L"Open folder", folder, ui::Font::Small, is_hot(kFolder) ? ui::theme::accent_hover : ui::theme::accent,
               DWRITE_TEXT_ALIGNMENT_TRAILING);
        area(folder, kFolder);
        y += 30;
        recent_top = y;
        recent_bottom = g.height() - 12;
        const float max_scroll = std::max(0.0f, static_cast<float>(recent.size()) * kRecentRow - (recent_bottom - recent_top));
        scroll = std::clamp(scroll, 0.0f, max_scroll);
        if (recent.empty()) {
            g.text(L"Nothing yet. What you receive and send shows here.", D2D1::RectF(kSide, y, right, y + 20),
                   ui::Font::Small, ui::theme::dim);
            return;
        }
        // Rows are drawn whole or not at all inside the list's band.
        for (std::size_t i = 0; i < recent.size(); ++i) {
            const float top = recent_top + static_cast<float>(i) * kRecentRow - scroll;
            if (top + kRecentRow - 6 <= recent_top || top >= recent_bottom) continue;
            if (top < recent_top - 0.5f || top + kRecentRow - 6 > recent_bottom) continue;
            const auto& e = recent[i];
            const D2D1_RECT_F row = D2D1::RectF(kSide, top, right, top + kRecentRow - 6);
            if (is_hot(kRecent, static_cast<int>(i))) g.fill(row, 10, ui::theme::card);
            const D2D1_RECT_F pic = D2D1::RectF(row.left + 6, row.top + 3, row.left + 46, row.top + 43);
            if (IWICBitmapSource* t = thumb_for(e)) {
                g.image(t, pic, 6);
            } else {
                g.fill(pic, 6, ui::theme::raised);
                const auto kind = e.out ? std::string("out") : Transfers::kind_of(path_from_utf8(e.item));
                const wchar_t* glyph = e.out ? ui::icon::kUpload
                                       : kind == "link" ? ui::icon::kLink
                                       : kind == "text" ? ui::icon::kText
                                                        : ui::icon::kFile;
                g.text(glyph, pic, ui::Font::Icon, ui::theme::dim, DWRITE_TEXT_ALIGNMENT_CENTER);
            }
            const std::wstring title = e.out ? wide(e.item) : path_from_utf8(e.item).filename().wstring();
            const std::wstring peer = e.peer.empty() ? (e.out ? L"your iPhone" : L"someone") : wide(e.peer);
            const std::wstring sub = (e.out ? L"Sent to " : L"From ") + peer + L" · " + when(e.time);
            g.text(title, D2D1::RectF(row.left + 56, row.top + 3, row.right - 8, row.top + 23), ui::Font::Body,
                   ui::theme::text);
            g.text(sub, D2D1::RectF(row.left + 56, row.top + 23, row.right - 8, row.top + 41), ui::Font::Small,
                   ui::theme::dim);
            if (!e.out) area(row, kRecent, static_cast<int>(i));
        }
    }

    // --- the settings page --------------------------------------------------------------

    void paint_settings(ui::Canvas& g) {
        const float w = g.width();
        const float right = w - kSide;
        const WindowSettings s = hooks.settings ? hooks.settings() : WindowSettings{};
        const D2D1_RECT_F back = D2D1::RectF(kSide - 6, 16, kSide + 28, 50);
        if (is_hot(kBack)) g.fill(back, 8, ui::theme::raised);
        g.text(ui::icon::kBack, back, ui::Font::Icon, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
        area(back, kBack);
        g.text(L"Settings", D2D1::RectF(kSide + 36, 16, right, 50), ui::Font::Title, ui::theme::text);
        float y = 64;

        const auto section = [&](const wchar_t* title) {
            g.text(title, D2D1::RectF(kSide, y, right, y + 20), ui::Font::Small, ui::theme::dim);
            y += 24;
        };
        const auto toggle = [&](const wchar_t* label, bool on, int index) {
            const D2D1_RECT_F row = D2D1::RectF(kSide, y, right, y + 32);
            if (is_hot(kToggle, index)) g.fill(row, 8, ui::theme::card);
            g.text(label, D2D1::RectF(row.left + 8, row.top, row.right - 56, row.bottom), ui::Font::Body,
                   ui::theme::text);
            const D2D1_RECT_F track = D2D1::RectF(row.right - 48, row.top + 7, row.right - 10, row.top + 25);
            g.fill(track, 9, on ? ui::theme::accent : ui::theme::raised_hover);
            g.dot(D2D1::Point2F(on ? track.right - 9 : track.left + 9, track.top + 9), 6.5f, ui::theme::text);
            area(row, kToggle, index);
            y += 34;
        };

        section(L"AIRDROP ON THIS PC");
        toggle(L"AirDrop on this PC (USB Wi-Fi stick)", s.radio, kRadio);
        y += 8;
        section(L"WHO CAN SEND TO THIS PC");
        const wchar_t* modes[3] = {L"Everyone, without asking", L"Everyone, but ask me (contacts without asking)",
                                   L"Contacts only"};
        const char* keys[3] = {"everyone", "ask", "contacts"};
        for (int i = 0; i < 3; ++i) {
            const bool enabled = i != 2 || !s.senders.empty();
            const bool chosen = s.receive == keys[i];
            const D2D1_RECT_F row = D2D1::RectF(kSide, y, right, y + 30);
            if (enabled && is_hot(kReceive, i)) g.fill(row, 8, ui::theme::card);
            const D2D1_POINT_2F c = D2D1::Point2F(row.left + 16, row.top + 15);
            g.outline(D2D1::RectF(c.x - 8, c.y - 8, c.x + 8, c.y + 8), 8, chosen ? ui::theme::accent : ui::theme::dim,
                      1.5f);
            if (chosen) g.dot(c, 4.5f, ui::theme::accent);
            std::wstring label = modes[i];
            if (!enabled) label += L" (add contacts below)";
            g.text(label, D2D1::RectF(row.left + 34, row.top, row.right - 8, row.bottom), ui::Font::Body,
                   enabled ? ui::theme::text : ui::theme::dim);
            if (enabled) area(row, kReceive, i);
            y += 32;
        }
        y += 8;
        section(L"CONTACTS");
        g.fill(D2D1::RectF(kSide, y, right, y + 76), 8, ui::theme::card);
        if (window) {
            place_edit(D2D1::RectF(kSide, y, right, y + 76));
        } else {
            // A picture of the page: the contacts as the box would show them.
            std::string lines = s.senders;
            std::replace(lines.begin(), lines.end(), ',', '\n');
            g.text(wide(lines), D2D1::RectF(kSide + 10, y + 6, right - 10, y + 70), ui::Font::Body, ui::theme::text,
                   DWRITE_TEXT_ALIGNMENT_LEADING, true);
        }
        y += 82;
        g.text(L"Phone numbers with country code (+1 555 123 4567) or emails, one per line.",
               D2D1::RectF(kSide, y, right - 90, y + 34), ui::Font::Small, ui::theme::dim, DWRITE_TEXT_ALIGNMENT_LEADING,
               true);
        const D2D1_RECT_F save = D2D1::RectF(right - 80, y + 2, right, y + 32);
        g.fill(save, 8, is_hot(kSave) ? ui::theme::accent_hover : ui::theme::accent);
        g.text(L"Save", save, ui::Font::Button, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
        area(save, kSave);
        y += 44;
        section(L"WHEN SOMETHING ARRIVES");
        toggle(L"Show finished transfers", s.notify, kNotify);
        toggle(L"Open links in the browser", s.open_links, kLinks);
        toggle(L"Open photos and videos", s.open_media, kMedia);
        toggle(L"Open PDFs and text files", s.open_documents, kDocuments);
        y += 8;
        section(L"THIS PC");
        toggle(L"Delete received files after 30 days", s.cleanup, kCleanup);
        toggle(L"Start with Windows", s.start_with_windows, kStartup);
    }

    // The contacts box: a native edit control over the page, dark.
    void place_edit(D2D1_RECT_F r) {
        if (!edit) {
            edit = CreateWindowExW(0, L"EDIT", L"", WS_CHILD | ES_MULTILINE | ES_AUTOVSCROLL | ES_WANTRETURN | WS_VSCROLL,
                                   0, 0, 10, 10, window, reinterpret_cast<HMENU>(static_cast<INT_PTR>(kEditId)),
                                   instance, nullptr);
            if (!edit_brush) edit_brush = CreateSolidBrush(RGB(30, 30, 32));
            load_contacts();
        }
        const UINT dpi = ui::dpi_of(window);
        if (dpi != edit_dpi) {
            if (edit_font) DeleteObject(edit_font);
            edit_font = CreateFontW(-static_cast<int>(std::lround(14.0 * dpi / 96.0)), 0, 0, 0, FW_NORMAL, FALSE, FALSE,
                                    FALSE, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY,
                                    DEFAULT_PITCH, L"Segoe UI");
            SendMessageW(edit, WM_SETFONT, reinterpret_cast<WPARAM>(edit_font), TRUE);
            edit_dpi = dpi;
        }
        const float s = scale();
        const int pad = static_cast<int>(std::lround(8 * s));
        SetWindowPos(edit, nullptr, static_cast<int>(r.left * s) + pad, static_cast<int>(r.top * s) + pad / 2,
                     static_cast<int>((r.right - r.left) * s) - 2 * pad, static_cast<int>((r.bottom - r.top) * s) - pad,
                     SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    }

    void load_contacts() {
        const WindowSettings s = hooks.settings ? hooks.settings() : WindowSettings{};
        std::wstring text;
        std::size_t start = 0;
        while (start <= s.senders.size()) {
            const auto comma = s.senders.find(',', start);
            std::string one = s.senders.substr(start, comma == std::string::npos ? std::string::npos : comma - start);
            while (!one.empty() && one.front() == ' ') one.erase(0, 1);
            while (!one.empty() && one.back() == ' ') one.pop_back();
            if (!one.empty()) text += (text.empty() ? L"" : L"\r\n") + wide(one);
            if (comma == std::string::npos) break;
            start = comma + 1;
        }
        SetWindowTextW(edit, text.c_str());
    }

    void save_contacts() {
        const int n = GetWindowTextLengthW(edit);
        std::wstring text(static_cast<std::size_t>(n) + 1, L'\0');
        GetWindowTextW(edit, text.data(), n + 1);
        text.resize(static_cast<std::size_t>(n));
        std::string joined;
        std::string one;
        const auto flush = [&]() {
            while (!one.empty() && (one.front() == ' ' || one.front() == '\t')) one.erase(0, 1);
            while (!one.empty() && (one.back() == ' ' || one.back() == '\t')) one.pop_back();
            if (!one.empty()) joined += (joined.empty() ? "" : ",") + one;
            one.clear();
        };
        const std::string utf8 = utf8_from_wide(text);
        for (char ch : utf8) {
            if (ch == '\n' || ch == '\r' || ch == ',' || ch == ';') flush();
            else one.push_back(ch);
        }
        flush();
        WindowSettings s = hooks.settings();
        s.senders = joined;
        if (s.senders.empty() && s.receive == "contacts") s.receive = "ask";
        hooks.apply(s);
    }

    // --- input -------------------------------------------------------------------------

    void act(int hit, int index) {
        switch (hit) {
            case kGear:
                settings_page = true;
                scroll = 0;
                break;
            case kBack:
                settings_page = false;
                if (edit) ShowWindow(edit, SW_HIDE);
                break;
            case kDrop: {
                auto files = pick_files(window);
                if (!files.empty()) pending = std::move(files);
                break;
            }
            case kClear:
                pending.clear();
                break;
            case kScan:
                if (hooks.scan) hooks.scan();
                break;
            case kDevice: {
                const Node::Nearby around = hooks.nearby ? hooks.nearby() : Node::Nearby{};
                const std::string to = index >= 0 && static_cast<std::size_t>(index) < around.devices.size()
                                           ? around.devices[static_cast<std::size_t>(index)]
                                           : std::string();
                std::vector<std::filesystem::path> files = pending;
                if (files.empty()) files = pick_files(window);
                if (!files.empty() && hooks.send) hooks.send(files, to);
                if (!files.empty()) pending.clear();
                break;
            }
            case kRecent:
                if (index >= 0 && static_cast<std::size_t>(index) < recent.size() && hooks.open) {
                    hooks.open(path_from_utf8(recent[static_cast<std::size_t>(index)].item));
                }
                break;
            case kFolder:
                if (hooks.open_folder) hooks.open_folder();
                break;
            case kToggle: {
                WindowSettings s = hooks.settings();
                switch (index) {
                    case kRadio: s.radio = !s.radio; break;
                    case kNotify: s.notify = !s.notify; break;
                    case kLinks: s.open_links = !s.open_links; break;
                    case kMedia: s.open_media = !s.open_media; break;
                    case kDocuments: s.open_documents = !s.open_documents; break;
                    case kCleanup: s.cleanup = !s.cleanup; break;
                    case kStartup: s.start_with_windows = !s.start_with_windows; break;
                    default: break;
                }
                hooks.apply(s);
                break;
            }
            case kReceive: {
                WindowSettings s = hooks.settings();
                const char* keys[3] = {"everyone", "ask", "contacts"};
                if (index >= 0 && index < 3) s.receive = keys[index];
                hooks.apply(s);
                break;
            }
            case kSave:
                save_contacts();
                break;
            case kNowAccept:
            case kNowDecline:
                if (index >= 0 && static_cast<std::size_t>(index) < now.size() && hooks.decide) {
                    hooks.decide(now[static_cast<std::size_t>(index)].id, hit == kNowAccept);
                }
                break;
            case kNowRow:
                if (index >= 0 && static_cast<std::size_t>(index) < now.size()) {
                    const auto& t = now[static_cast<std::size_t>(index)];
                    if (t.stage == Transfers::Stage::Done && !t.saved.empty() && t.kind != "text" && hooks.open) {
                        hooks.open(t.saved.front());
                    }
                }
                break;
            default:
                break;
        }
        InvalidateRect(window, nullptr, FALSE);
    }

    std::pair<int, int> hit_at(float x, float y) const {
        for (auto it = areas.rbegin(); it != areas.rend(); ++it) {  // the last drawn is on top
            if (inside(it->rect, x, y)) return {it->hit, it->index};
        }
        return {kNothing, 0};
    }

    LRESULT proc(UINT message, WPARAM wparam, LPARAM lparam) {
        const float s = scale();
        switch (message) {
            case WM_PAINT: {
                PAINTSTRUCT ps;
                BeginPaint(window, &ps);
                if (canvas->begin()) {
                    areas.clear();
                    canvas->clear(ui::theme::window);
                    if (settings_page) {
                        paint_settings(*canvas);
                    } else {
                        if (edit) ShowWindow(edit, SW_HIDE);
                        paint_main(*canvas);
                    }
                    canvas->end();
                }
                EndPaint(window, &ps);
                return 0;
            }
            case WM_SIZE:
                canvas->resize();
                InvalidateRect(window, nullptr, FALSE);
                tell_visibility();
                return 0;
            case WM_TIMER:
                if (wparam == kAnimate) {
                    phase = std::fmod(phase + 0.025, 1.0);
                    InvalidateRect(window, nullptr, FALSE);
                }
                return 0;
            case WM_DPICHANGED: {
                const RECT* r = reinterpret_cast<const RECT*>(lparam);
                SetWindowPos(window, nullptr, r->left, r->top, r->right - r->left, r->bottom - r->top,
                             SWP_NOZORDER | SWP_NOACTIVATE);
                return 0;
            }
            case WM_MOUSEMOVE: {
                if (!tracking) {
                    TRACKMOUSEEVENT track{sizeof(track), TME_LEAVE, window, 0};
                    TrackMouseEvent(&track);
                    tracking = true;
                }
                const auto [h, i] = hit_at(static_cast<float>(GET_X_LPARAM(lparam)) / s,
                                           static_cast<float>(GET_Y_LPARAM(lparam)) / s);
                if (h != hot || i != hot_index) {
                    hot = h;
                    hot_index = i;
                    InvalidateRect(window, nullptr, FALSE);
                }
                return 0;
            }
            case WM_MOUSELEAVE:
                tracking = false;
                hot = kNothing;
                InvalidateRect(window, nullptr, FALSE);
                return 0;
            case WM_SETCURSOR:
                if (LOWORD(lparam) == HTCLIENT && hot != kNothing) {
                    SetCursor(LoadCursorW(nullptr, MAKEINTRESOURCEW(32649)));  // IDC_HAND
                    return TRUE;
                }
                break;
            case WM_LBUTTONUP: {
                const auto [h, i] = hit_at(static_cast<float>(GET_X_LPARAM(lparam)) / s,
                                           static_cast<float>(GET_Y_LPARAM(lparam)) / s);
                if (h != kNothing) act(h, i);
                return 0;
            }
            case WM_RBUTTONUP: {
                const auto [h, i] = hit_at(static_cast<float>(GET_X_LPARAM(lparam)) / s,
                                           static_cast<float>(GET_Y_LPARAM(lparam)) / s);
                if (h == kRecent && i >= 0 && static_cast<std::size_t>(i) < recent.size() && hooks.show) {
                    hooks.show(path_from_utf8(recent[static_cast<std::size_t>(i)].item));
                }
                return 0;
            }
            case WM_MOUSEWHEEL:
                if (!settings_page) {
                    scroll -= static_cast<float>(GET_WHEEL_DELTA_WPARAM(wparam)) / WHEEL_DELTA * kRecentRow;
                    InvalidateRect(window, nullptr, FALSE);
                }
                return 0;
            case WM_DROPFILES: {
                HDROP drop = reinterpret_cast<HDROP>(wparam);
                const UINT n = DragQueryFileW(drop, 0xFFFFFFFF, nullptr, 0);
                std::vector<std::filesystem::path> files;
                for (UINT i = 0; i < n; ++i) {
                    const UINT len = DragQueryFileW(drop, i, nullptr, 0);
                    std::wstring path(len + 1, L'\0');
                    DragQueryFileW(drop, i, path.data(), len + 1);
                    path.resize(len);
                    std::error_code ec;
                    if (std::filesystem::is_regular_file(path, ec)) files.emplace_back(path);
                }
                DragFinish(drop);
                if (!files.empty()) {
                    pending = std::move(files);
                    settings_page = false;
                    InvalidateRect(window, nullptr, FALSE);
                }
                return 0;
            }
            case WM_CTLCOLOREDIT: {
                HDC dc = reinterpret_cast<HDC>(wparam);
                SetTextColor(dc, RGB(244, 244, 246));
                SetBkColor(dc, RGB(30, 30, 32));
                return reinterpret_cast<LRESULT>(edit_brush);
            }
            case WM_CLOSE:
                DestroyWindow(window);
                return 0;
            case WM_DESTROY:
                edit = nullptr;  // destroyed with its parent
                window = nullptr;
                canvas.reset();
                animating = false;
                tell_visibility();
                return 0;
            default:
                break;
        }
        return DefWindowProcW(window, message, wparam, lparam);
    }
};

namespace {

LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* impl = reinterpret_cast<HowlWindow::Impl*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (!impl || !impl->window) return DefWindowProcW(window, message, wparam, lparam);
    return impl->proc(message, wparam, lparam);
}

}  // namespace

HowlWindow::HowlWindow(HINSTANCE instance, std::wstring name, std::filesystem::path history, Hooks hooks)
    : impl_(std::make_unique<Impl>()) {
    impl_->instance = instance;
    impl_->name = std::move(name);
    impl_->history_file = std::move(history);
    impl_->hooks = std::move(hooks);
    WNDCLASSEXW cls{};
    cls.cbSize = sizeof(cls);
    cls.lpfnWndProc = window_proc;
    cls.hInstance = instance;
    cls.hCursor = LoadCursorW(nullptr, MAKEINTRESOURCEW(32512));
    cls.hIcon = LoadIconW(instance, MAKEINTRESOURCEW(1));
    cls.hbrBackground = CreateSolidBrush(RGB(23, 23, 26));
    cls.lpszClassName = kWindowClass;
    RegisterClassExW(&cls);
}

HowlWindow::~HowlWindow() = default;

bool HowlWindow::open() const { return impl_->window != nullptr; }

bool HowlWindow::showing() const { return impl_->showing(); }

void HowlWindow::raise() {
    HWND w = impl_->window;
    if (!impl_->showing()) return;
    // Topmost and back: to the top of the others without taking the
    // keyboard; then the foreground if Windows allows it, and a flash in
    // the taskbar if it doesn't.
    SetWindowPos(w, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    SetWindowPos(w, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    if (!SetForegroundWindow(w)) {
        FLASHWINFO flash{sizeof(flash), w, FLASHW_TRAY | FLASHW_TIMERNOFG, 0, 0};
        FlashWindowEx(&flash);
    }
    refresh();
}

bool HowlWindow::render(const std::filesystem::path& file, bool settings, float scale, std::wstring name,
                        std::filesystem::path history, Hooks hooks) {
    if (!ui::init()) return false;
    Impl d;
    d.name = std::move(name);
    d.history_file = std::move(history);
    d.hooks = std::move(hooks);
    d.phase = 0.35;
    d.reload_history();
    return ui::render_png(file, kWidth, kHeight, scale, 12.0f, [&](ui::Canvas& g) {
        g.clear(ui::theme::window);
        if (settings) d.paint_settings(g);
        else d.paint_main(g);
    });
}

void HowlWindow::show() {
    if (!ui::init()) return;
    Impl& d = *impl_;
    if (d.window) {
        ShowWindow(d.window, SW_RESTORE);
        SetForegroundWindow(d.window);
        refresh();
        d.tell_visibility();
        return;
    }
    const DWORD style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    d.window = CreateWindowExW(WS_EX_ACCEPTFILES, kWindowClass, L"HowlDrop", style, CW_USEDEFAULT, CW_USEDEFAULT, 100,
                               100, nullptr, nullptr, d.instance, nullptr);
    if (!d.window) return;
    SetWindowLongPtrW(d.window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(&d));
    ui::dark_frame(d.window, true);
    d.canvas = std::make_unique<ui::Canvas>(d.window);
    // The client area kWidth x kHeight DIPs, at this monitor's scale, near
    // the tray.
    const float s = d.scale();
    RECT r{0, 0, static_cast<LONG>(std::lround(kWidth * s)), static_cast<LONG>(std::lround(kHeight * s))};
    AdjustWindowRectEx(&r, style, FALSE, WS_EX_ACCEPTFILES);
    RECT work{};
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
    const int w = r.right - r.left;
    const int h = r.bottom - r.top;
    const int margin = static_cast<int>(std::lround(12 * s));
    SetWindowPos(d.window, nullptr, std::max<int>(work.left, work.right - w - margin),
                 std::max<int>(work.top, work.bottom - h - margin), w, h, SWP_NOZORDER);
    d.reload_history();
    ShowWindow(d.window, SW_SHOW);
    SetForegroundWindow(d.window);
    d.tell_visibility();
}

void HowlWindow::refresh() {
    Impl& d = *impl_;
    if (!d.window) return;
    d.reload_history();
    InvalidateRect(d.window, nullptr, FALSE);
}

}  // namespace howldrop
