// The transfer cards (cards_win.hpp).

#include "cards_win.hpp"

#include "text.hpp"
#include "ui_win.hpp"

#include <windowsx.h>

#include <algorithm>
#include <cmath>
#include <cwctype>

namespace howldrop {

using Stage = Transfers::Stage;
using Direction = Transfers::Direction;
using Clock = Transfers::Clock;

namespace {

const wchar_t* const kCardClass = L"HowlDropCard";
constexpr float kWidth = 360.0f;
constexpr float kPad = 16.0f;
constexpr float kGap = 10.0f;    // between cards
constexpr float kMargin = 12.0f; // from the screen's edge
constexpr UINT_PTR kTimer = 1;
constexpr std::size_t kMaxCards = 5;

enum Action { kNone = -1, kAccept, kDecline, kOpen, kShow, kClose, kBody };

struct Button {
    D2D1_RECT_F rect;
    std::wstring label;
    bool primary;
    int action;
};

std::wstring wide(const std::string& s) {
    auto w = wide_from_utf8(s);
    return w ? *w : std::wstring();
}

bool inside(const D2D1_RECT_F& r, float x, float y) { return x >= r.left && x < r.right && y >= r.top && y < r.bottom; }

std::wstring plural(std::size_t n, const wchar_t* one, const wchar_t* many) {
    return std::to_wstring(n) + L" " + (n == 1 ? one : many);
}

}  // namespace

struct Cards::Card {
    Cards* owner = nullptr;
    HWND window = nullptr;
    std::unique_ptr<ui::Canvas> canvas;
    Transfers::Transfer t;
    bool was_finished = false;
    Clock::time_point finished_at{};
    bool hover = false;
    int hot = kNone;
    int pressed = kNone;
    std::vector<Button> buttons;
    D2D1_RECT_F close{};
    IWICBitmapSource* thumb = nullptr;
    bool thumb_tried = false;
    float height = 120.0f;  // DIPs, from the last layout
    double phase = 0;       // the indeterminate bar's

    ~Card() {
        if (window) {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);  // its last messages find no card
            DestroyWindow(window);
        }
        canvas.reset();
        if (thumb) thumb->Release();
    }
};

CardText card_text(const Transfers::Transfer& t) {
    const std::wstring peer = t.peer.empty() ? (t.direction == Direction::In ? L"Someone" : L"your iPhone")
                                             : wide(t.peer);
    const std::wstring items = plural(std::max<std::size_t>(t.count, t.names.size()), L"file", L"files");
    if (t.direction == Direction::In) {
        switch (t.stage) {
            case Stage::Asking:
                return {ui::icon::kDownload, ui::theme::accent, L"Receive", peer + L" wants to send " + items};
            case Stage::Waiting:
            case Stage::Moving:
            case Stage::Saving:
                return {ui::icon::kDownload, ui::theme::accent, L"Receiving", L"From " + peer};
            case Stage::Done: {
                std::wstring what = t.kind == "text" ? L"Text" : t.kind == "link" ? L"A link" : items;
                if (t.kind == "image" && t.saved.size() == 1) what = L"A photo";
                if (t.kind == "video" && t.saved.size() == 1) what = L"A video";
                return {ui::icon::kCheck, ui::theme::good, L"Received",
                        what + L" from " + peer + (t.home_assistant ? L", through Home Assistant" : L"")};
            }
            case Stage::Declined:
                return {ui::icon::kClose, ui::theme::dim, L"Declined", peer + L"'s " + items};
            default:
                return {ui::icon::kError, ui::theme::bad, L"Didn't arrive", L"From " + peer};
        }
    }
    switch (t.stage) {
        case Stage::Done:
            return {ui::icon::kCheck, ui::theme::good, L"Sent", L"To " + peer};
        case Stage::Declined:
            return {ui::icon::kClose, ui::theme::dim, L"Declined", L"On " + peer};
        case Stage::Failed:
            return {ui::icon::kError, ui::theme::bad, L"Not sent", L"To " + peer};
        default:
            return {ui::icon::kUpload, ui::theme::accent, L"Sending", L"To " + peer};
    }
}

std::wstring card_status(const Transfers::Transfer& t) {
    if (t.direction == Direction::In) {
        if (t.stage == Stage::Moving && t.total) {
            std::wstring status = ui::size_text(t.bytes) + L" of " + ui::size_text(t.total);
            if (t.rate > 0) status += L" · " + ui::size_text(static_cast<unsigned long long>(t.rate)) + L"/s";
            return status;
        }
        return t.stage == Stage::Saving ? L"Saving…" : L"Starting…";
    }
    std::wstring status = t.detail.empty() ? L"Starting…" : wide(t.detail);
    if (!status.empty()) status[0] = static_cast<wchar_t>(towupper(status[0]));
    return status;
}

double card_fraction(const Transfers::Transfer& t) {
    if (t.stage == Stage::Saving) return 1.0;
    return t.direction == Direction::In && t.stage == Stage::Moving ? t.fraction() : -1.0;
}

namespace {

// Lays the card out and, with paint, draws it. Returns its height.
float render(Cards::Card& c, const std::wstring& inbox_label, bool paint) {
    ui::Canvas& g = *c.canvas;
    const Transfers::Transfer& t = c.t;
    const float w = kWidth;
    const float inner = w - 2 * kPad;
    float y = kPad;
    c.buttons.clear();
    if (paint) g.clear(ui::theme::card);

    const CardText say = card_text(t);
    // Header: icon, title, close.
    if (paint) {
        g.text(say.glyph, D2D1::RectF(kPad, y, kPad + 22, y + 24), ui::Font::Icon, say.tint);
        g.text(say.title, D2D1::RectF(kPad + 30, y, w - kPad - 30, y + 24), ui::Font::Title, ui::theme::text);
    }
    c.close = D2D1::RectF(w - kPad - 24, y, w - kPad, y + 24);
    if (paint) {
        if (c.hot == kClose) g.fill(c.close, 6, ui::theme::raised);
        g.text(ui::icon::kClose, c.close, ui::Font::Icon, c.hot == kClose ? ui::theme::text : ui::theme::dim,
               DWRITE_TEXT_ALIGNMENT_CENTER);
    }
    y += 32;

    // Who and what.
    const float line_h = g.text_height(say.line, inner, ui::Font::Body, true);
    if (paint) g.text(say.line, D2D1::RectF(kPad, y, w - kPad, y + line_h), ui::Font::Body, ui::theme::text,
                      DWRITE_TEXT_ALIGNMENT_LEADING, true);
    y += line_h + 6;

    // A picture beside the names, once it's here.
    const bool picture = t.stage == Stage::Done && (t.kind == "image" || t.kind == "video") && !t.saved.empty();
    if (picture && !c.thumb_tried) {
        c.thumb_tried = true;
        c.thumb = ui::thumbnail(t.saved.front(), 128);
    }
    float left = kPad;
    const float names_top = y;
    if (picture && c.thumb) {
        if (paint) g.image(c.thumb, D2D1::RectF(kPad, y, kPad + 64, y + 64), 8);
        left = kPad + 76;
    }

    // Names: up to three, then how many more. Text shows its words instead.
    std::vector<std::wstring> lines;
    if (t.stage == Stage::Done && t.kind == "text") {
        lines.push_back(L"On the clipboard");
    } else {
        const std::size_t shown = std::min<std::size_t>(t.names.size(), 3);
        for (std::size_t i = 0; i < shown; ++i) lines.push_back(wide(t.names[i]));
        const std::size_t count = std::max(t.count, t.names.size());
        if (count > shown) lines.push_back(L"and " + std::to_wstring(count - shown) + L" more");
    }
    for (const auto& l : lines) {
        if (paint) g.text(l, D2D1::RectF(left, y, w - kPad, y + 18), ui::Font::Small, ui::theme::dim);
        y += 18;
    }
    if (picture && c.thumb) y = std::max(y, names_top + 64);
    y += 6;

    // Size and where it goes, while it's coming.
    if (t.direction == Direction::In && (t.stage == Stage::Asking || t.stage == Stage::Waiting)) {
        std::wstring meta = t.total ? ui::size_text(t.total) + L" · " : L"";
        meta += L"Saves to " + inbox_label;
        if (paint) g.text(meta, D2D1::RectF(kPad, y, w - kPad, y + 18), ui::Font::Small, ui::theme::dim);
        y += 24;
    }

    // Progress, with the numbers or the sender's stage under it.
    const bool moving = !t.finished() && t.stage != Stage::Asking;
    if (moving) {
        if (paint) g.progress(D2D1::RectF(kPad, y, w - kPad, y + 6), card_fraction(t), c.phase);
        y += 14;
        if (paint) g.text(card_status(t), D2D1::RectF(kPad, y, w - kPad, y + 18), ui::Font::Small, ui::theme::dim);
        y += 22;
    } else if ((t.stage == Stage::Failed || t.stage == Stage::Declined) && !t.detail.empty()) {
        std::wstring why = wide(t.detail);
        why[0] = static_cast<wchar_t>(towupper(why[0]));
        const float h = g.text_height(why, inner, ui::Font::Small, true);
        if (paint) g.text(why, D2D1::RectF(kPad, y, w - kPad, y + h), ui::Font::Small, ui::theme::dim,
                          DWRITE_TEXT_ALIGNMENT_LEADING, true);
        y += h + 8;
    }

    // An ask: how long it waits, and the answer.
    if (t.stage == Stage::Asking) {
        const double left_s = std::chrono::duration<double>(t.deadline - Clock::now()).count();
        const double frac = std::clamp(left_s / Transfers::kAskTimeout.count(), 0.0, 1.0);
        if (paint) g.progress(D2D1::RectF(kPad, y, w - kPad, y + 3), frac, 0, ui::theme::dim);
        y += 12;
        const float half = (inner - 10) / 2;
        c.buttons.push_back({D2D1::RectF(kPad, y, kPad + half, y + 38), L"Decline", false, kDecline});
        c.buttons.push_back({D2D1::RectF(w - kPad - half, y, w - kPad, y + 38), L"Accept", true, kAccept});
        y += 38;
    } else if (t.direction == Direction::In && t.stage == Stage::Done && !t.saved.empty() && t.kind != "text") {
        const float half = (inner - 10) / 2;
        if (t.kind == "link") {
            c.buttons.push_back({D2D1::RectF(kPad, y, w - kPad, y + 34), L"Open link", true, kOpen});
        } else {
            c.buttons.push_back({D2D1::RectF(kPad, y, kPad + half, y + 34), L"Show in folder", false, kShow});
            c.buttons.push_back({D2D1::RectF(w - kPad - half, y, w - kPad, y + 34), L"Open", true, kOpen});
        }
        y += 34;
    }
    if (paint) {
        for (const auto& b : c.buttons) {
            const bool hot = c.hot == b.action;
            const ui::Color fill = b.primary ? (hot ? ui::theme::accent_hover : ui::theme::accent)
                                             : (hot ? ui::theme::raised_hover : ui::theme::raised);
            g.fill(b.rect, 9, fill);
            g.text(b.label, b.rect, ui::Font::Button, ui::theme::text, DWRITE_TEXT_ALIGNMENT_CENTER);
        }
    }
    return y + kPad;
}

int hit(const Cards::Card& c, float x, float y) {
    if (inside(c.close, x, y)) return kClose;
    for (const auto& b : c.buttons) {
        if (inside(b.rect, x, y)) return b.action;
    }
    return kBody;
}

}  // namespace

LRESULT CALLBACK card_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam) {
    auto* c = reinterpret_cast<Cards::Card*>(GetWindowLongPtrW(window, GWLP_USERDATA));
    if (!c) return DefWindowProcW(window, message, wparam, lparam);
    const float scale = static_cast<float>(ui::dpi_of(window)) / 96.0f;
    switch (message) {
        case WM_MOUSEACTIVATE:
            return MA_NOACTIVATE;  // never take focus from what the user is doing
        case WM_PAINT: {
            PAINTSTRUCT ps;
            BeginPaint(window, &ps);
            if (c->canvas->begin()) {
                render(*c, c->owner->inbox_label_, true);
                c->canvas->end();
            }
            EndPaint(window, &ps);
            return 0;
        }
        case WM_SIZE:
            c->canvas->resize();
            return 0;
        case WM_MOUSEMOVE: {
            if (!c->hover) {
                c->hover = true;
                TRACKMOUSEEVENT track{sizeof(track), TME_LEAVE, window, 0};
                TrackMouseEvent(&track);
            }
            const int now = hit(*c, static_cast<float>(GET_X_LPARAM(lparam)) / scale,
                                static_cast<float>(GET_Y_LPARAM(lparam)) / scale);
            if (now != c->hot) {
                c->hot = now;
                InvalidateRect(window, nullptr, FALSE);
            }
            return 0;
        }
        case WM_MOUSELEAVE:
            c->hover = false;
            c->hot = kNone;
            InvalidateRect(window, nullptr, FALSE);
            return 0;
        case WM_SETCURSOR:
            if (c->hot != kNone && c->hot != kBody) {
                SetCursor(LoadCursorW(nullptr, MAKEINTRESOURCEW(32649)));  // IDC_HAND
                return TRUE;
            }
            break;
        case WM_LBUTTONDOWN:
            c->pressed = hit(*c, static_cast<float>(GET_X_LPARAM(lparam)) / scale,
                             static_cast<float>(GET_Y_LPARAM(lparam)) / scale);
            return 0;
        case WM_LBUTTONUP: {
            const int up = hit(*c, static_cast<float>(GET_X_LPARAM(lparam)) / scale,
                               static_cast<float>(GET_Y_LPARAM(lparam)) / scale);
            const int down = c->pressed;
            c->pressed = kNone;
            if (up == down) c->owner->act(*c, up);  // may destroy c
            return 0;
        }
        case WM_TIMER: {
            c->phase = std::fmod(c->phase + 0.025, 1.0);
            const auto now = Clock::now();
            if (c->t.finished() && !c->hover) {
                const auto linger = c->t.stage == Stage::Done ? std::chrono::seconds(8) : std::chrono::seconds(10);
                if (now - c->finished_at > linger) {
                    c->owner->retire(c);  // destroys c
                    return 0;
                }
            }
            const bool animated = !c->t.finished();
            if (animated) InvalidateRect(window, nullptr, FALSE);
            return 0;
        }
        default:
            break;
    }
    return DefWindowProcW(window, message, wparam, lparam);
}

Cards::Cards(HINSTANCE instance, std::wstring inbox_label, Actions actions)
    : instance_(instance), inbox_label_(std::move(inbox_label)), actions_(std::move(actions)) {
    available_ = ui::init();
    if (!available_) return;
    WNDCLASSEXW cls{};
    cls.cbSize = sizeof(cls);
    cls.style = CS_DROPSHADOW;
    cls.lpfnWndProc = card_proc;
    cls.hInstance = instance_;
    cls.hCursor = LoadCursorW(nullptr, MAKEINTRESOURCEW(32512));  // IDC_ARROW
    cls.lpszClassName = kCardClass;
    RegisterClassExW(&cls);
}

Cards::~Cards() { cards_.clear(); }

void Cards::update(const Transfers& model, bool finished_too) {
    if (!available_) return;
    const auto now = Clock::now();
    for (const auto& t : model.all()) {
        if (dismissed_.count(t.id)) continue;
        Card* card = nullptr;
        for (auto& c : cards_) {
            if (c->t.id == t.id) card = c.get();
        }
        if (!card) {
            // A finished transfer gets a card only when it just ended.
            const bool fresh = now - t.updated < std::chrono::seconds(5);
            if (t.finished() && (!finished_too || !fresh)) continue;
            auto made = std::make_unique<Card>();
            made->owner = this;
            made->window = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE, kCardClass, L"HowlDrop",
                                           WS_POPUP, 0, 0, 10, 10, nullptr, nullptr, instance_, nullptr);
            if (!made->window) continue;
            made->canvas = std::make_unique<ui::Canvas>(made->window);
            SetWindowLongPtrW(made->window, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(made.get()));
            ui::dark_frame(made->window, true);
            SetTimer(made->window, kTimer, 40, nullptr);
            card = made.get();
            cards_.push_back(std::move(made));
        }
        const bool finished = t.finished();
        if (finished && !card->was_finished) card->finished_at = now;
        card->was_finished = finished;
        card->t = t;
        card->height = render(*card, inbox_label_, false);
        InvalidateRect(card->window, nullptr, FALSE);
    }
    // Too many: the oldest finished ones go first.
    while (cards_.size() > kMaxCards) {
        auto it = std::find_if(cards_.begin(), cards_.end(), [](const auto& c) { return c->t.finished(); });
        if (it == cards_.end()) it = cards_.begin();
        const std::string id = (*it)->t.id;
        cards_.erase(it);
        if (actions_.retired) actions_.retired(id);
    }
    place();
}

void Cards::place() {
    RECT work{};
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &work, 0);
    int bottom = work.bottom;
    for (auto& c : cards_) {
        const float scale = static_cast<float>(ui::dpi_of(c->window)) / 96.0f;
        const int w = static_cast<int>(std::lround(kWidth * scale));
        const int h = static_cast<int>(std::lround(c->height * scale));
        const int margin = static_cast<int>(std::lround(kMargin * scale));
        const int x = work.right - margin - w;
        const int y = bottom - margin - h;
        SetWindowPos(c->window, HWND_TOPMOST, x, y, w, h,
                     SWP_NOACTIVATE | (hidden_ ? SWP_HIDEWINDOW : SWP_SHOWWINDOW));
        bottom = y - static_cast<int>(std::lround((kGap - kMargin) * scale));
    }
}

void Cards::set_hidden(bool hidden) {
    if (hidden == hidden_) return;
    hidden_ = hidden;
    place();
}

void Cards::retire(Card* card) {
    auto it = std::find_if(cards_.begin(), cards_.end(), [&](const auto& c) { return c.get() == card; });
    if (it == cards_.end()) return;
    const std::string id = card->t.id;
    cards_.erase(it);  // destroys the window
    place();
    if (actions_.retired) actions_.retired(id);
}

void Cards::act(Card& card, int action) {
    const Transfers::Transfer& t = card.t;
    switch (action) {
        case kAccept:
        case kDecline:
            if (actions_.decide) actions_.decide(t.id, action == kAccept);
            card.t.stage = action == kAccept ? Stage::Waiting : Stage::Declined;
            if (action == kDecline) {
                card.t.detail = "declined";
                card.finished_at = Clock::now();
                card.was_finished = true;
            }
            card.height = render(card, inbox_label_, false);
            place();
            InvalidateRect(card.window, nullptr, FALSE);
            return;
        case kClose:
            if (t.stage == Stage::Asking && actions_.decide) actions_.decide(t.id, false);
            dismissed_.insert(t.id);
            retire(&card);
            return;
        case kOpen:
            if (!t.saved.empty() && actions_.open) actions_.open(t.saved.front());
            return;
        case kShow:
            if (!t.saved.empty() && actions_.show) actions_.show(t.saved.front());
            return;
        case kBody:
            // A finished file: as the notification's click did.
            if (t.stage == Stage::Done && !t.saved.empty()) {
                const bool opens = t.kind == "image" || t.kind == "video" || t.kind == "link";
                if (opens && actions_.open) actions_.open(t.saved.front());
                if (!opens && actions_.show) actions_.show(t.saved.front());
            }
            return;
        default:
            return;
    }
}

}  // namespace howldrop
