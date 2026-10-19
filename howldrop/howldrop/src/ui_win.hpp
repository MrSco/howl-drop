#pragma once

// Drawing for HowlDrop's own windows: the transfer cards (cards_win.hpp) and
// the HowlDrop window (window_win.hpp). Direct2D and DirectWrite, so text
// and rounded shapes are smooth at any DPI, in a dark theme after
// omdrop-windows' look. Everything is measured in DIPs (1/96 inch). Windows
// only; MinGW ships the headers and libraries.

#include <windows.h>
#include <d2d1.h>
#include <dwrite.h>
#include <wincodec.h>

#include <filesystem>
#include <functional>
#include <string>
#include <utility>
#include <vector>

namespace howldrop::ui {

struct Color {
    float r, g, b, a = 1.0f;
};

namespace theme {
inline constexpr Color window{0.090f, 0.090f, 0.098f};    // #17171a
inline constexpr Color card{0.118f, 0.118f, 0.125f};      // #1e1e20
inline constexpr Color raised{0.176f, 0.176f, 0.188f};    // #2d2d30: secondary buttons, rows
inline constexpr Color raised_hover{0.220f, 0.220f, 0.235f};
inline constexpr Color text{0.957f, 0.957f, 0.965f};
inline constexpr Color dim{0.600f, 0.600f, 0.635f};
inline constexpr Color faint{1.0f, 1.0f, 1.0f, 0.10f};    // tracks, rules
inline constexpr Color accent{0.039f, 0.518f, 1.0f};      // #0a84ff
inline constexpr Color accent_hover{0.200f, 0.612f, 1.0f};
inline constexpr Color good{0.204f, 0.780f, 0.349f};      // #34c759
inline constexpr Color warn{1.0f, 0.624f, 0.039f};        // #ff9f0a
inline constexpr Color bad{1.0f, 0.271f, 0.227f};         // #ff453a
}  // namespace theme

enum class Font { Title, Heading, Body, Small, Button, Icon, IconLarge };

// Segoe Fluent Icons (Windows 11) or Segoe MDL2 Assets (10): same code points.
namespace icon {
inline constexpr const wchar_t* kDownload = L"";
inline constexpr const wchar_t* kUpload = L"";
inline constexpr const wchar_t* kClose = L"";
inline constexpr const wchar_t* kCheck = L"";
inline constexpr const wchar_t* kError = L"";
inline constexpr const wchar_t* kPhone = L"";
inline constexpr const wchar_t* kSettings = L"";
inline constexpr const wchar_t* kFolder = L"";
inline constexpr const wchar_t* kLink = L"";
inline constexpr const wchar_t* kText = L"";  // copy
inline constexpr const wchar_t* kFile = L"";
inline constexpr const wchar_t* kBack = L"";
inline constexpr const wchar_t* kRefresh = L"";
}  // namespace icon

// The factories, made once. False when Direct2D is unavailable; callers then
// fall back to plain notifications.
bool init();
// Per-monitor DPI awareness (Windows 10 1703+), before any window exists.
void enable_dpi_awareness();
UINT dpi_of(HWND window);
// A window's frame: Windows 11's rounded corners and a dark border.
void dark_frame(HWND window, bool rounded);

// "780 KB", "12.3 MB"
std::wstring size_text(unsigned long long bytes);

class Canvas {
public:
    explicit Canvas(HWND window);
    // Off screen, on target, width x height DIPs (render_png).
    Canvas(ID2D1RenderTarget* target, float width, float height);
    ~Canvas();
    Canvas(const Canvas&) = delete;
    Canvas& operator=(const Canvas&) = delete;

    bool begin();  // false: nothing can be drawn now
    void end();
    void resize();
    float width() const;   // the client area, in DIPs
    float height() const;

    void clear(Color c);
    void fill(D2D1_RECT_F r, float radius, Color c);
    void outline(D2D1_RECT_F r, float radius, Color c, float width = 1.0f);
    void line(D2D1_POINT_2F a, D2D1_POINT_2F b, Color c, float width = 1.0f);
    void dot(D2D1_POINT_2F centre, float radius, Color c);
    // One line, cut with "…", unless wrap (then as many lines as fit).
    void text(const std::wstring& s, D2D1_RECT_F r, Font f, Color c,
              DWRITE_TEXT_ALIGNMENT align = DWRITE_TEXT_ALIGNMENT_LEADING, bool wrap = false);
    float text_height(const std::wstring& s, float width, Font f, bool wrap) const;
    // fraction 0..1, or < 0 for an indeterminate bar (phase 0..1 animates it).
    void progress(D2D1_RECT_F r, double fraction, double phase, Color c = theme::accent);
    // A picture, cropped to fill r, with rounded corners.
    void image(IWICBitmapSource* source, D2D1_RECT_F r, float radius);

private:
    ID2D1SolidColorBrush* brush(Color c);
    bool make_target();
    void drop_target();

    HWND window_ = nullptr;
    ID2D1RenderTarget* target_ = nullptr;
    ID2D1HwndRenderTarget* hwnd_target_ = nullptr;  // target_, when it is a window's
    float width_ = 0, height_ = 0;                   // off screen
    ID2D1SolidColorBrush* brush_ = nullptr;
    // Pictures drawn lately, as bitmaps of this target (each source held).
    std::vector<std::pair<IWICBitmapSource*, ID2D1Bitmap*>> images_;
};

// A file's thumbnail as Explorer makes it (HEIC too, with the codec), or
// nullptr. The caller releases it. COM must be initialised on this thread.
IWICBitmapSource* thumbnail(const std::filesystem::path& file, int pixels);

// A picture of what draw draws on width x height DIPs at scale, with
// rounded, transparent corners, saved as PNG (`howldrop screenshots`).
bool render_png(const std::filesystem::path& file, float width, float height, float scale, float radius,
                const std::function<void(Canvas&)>& draw);

}  // namespace howldrop::ui
