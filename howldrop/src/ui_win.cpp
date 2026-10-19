// HowlDrop's drawing kit (ui_win.hpp).

#include "ui_win.hpp"

#include <dwmapi.h>
#include <shlobj.h>
#include <shobjidl.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

namespace howldrop::ui {
namespace {

ID2D1Factory* g_d2d = nullptr;
IDWriteFactory* g_dwrite = nullptr;
IWICImagingFactory* g_wic = nullptr;
IDWriteTextFormat* g_formats[7] = {};
IDWriteInlineObject* g_ellipsis[7] = {};

template <class T>
void release(T*& p) {
    if (p) p->Release();
    p = nullptr;
}

D2D1_COLOR_F d2d(Color c) { return D2D1::ColorF(c.r, c.g, c.b, c.a); }

bool has_family(const wchar_t* name) {
    IDWriteFontCollection* fonts = nullptr;
    if (FAILED(g_dwrite->GetSystemFontCollection(&fonts, FALSE))) return false;
    UINT32 index = 0;
    BOOL exists = FALSE;
    fonts->FindFamilyName(name, &index, &exists);
    fonts->Release();
    return exists != FALSE;
}

bool make_formats() {
    const wchar_t* text = has_family(L"Segoe UI Variable Text") ? L"Segoe UI Variable Text" : L"Segoe UI";
    const wchar_t* display = has_family(L"Segoe UI Variable Display") ? L"Segoe UI Variable Display" : L"Segoe UI";
    const wchar_t* icons = has_family(L"Segoe Fluent Icons") ? L"Segoe Fluent Icons" : L"Segoe MDL2 Assets";
    struct Spec {
        const wchar_t* family;
        DWRITE_FONT_WEIGHT weight;
        float size;
    };
    const Spec specs[7] = {
        {display, DWRITE_FONT_WEIGHT_SEMI_BOLD, 17.0f},  // Title
        {text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0f},     // Heading
        {text, DWRITE_FONT_WEIGHT_NORMAL, 14.0f},        // Body
        {text, DWRITE_FONT_WEIGHT_NORMAL, 12.0f},        // Small
        {text, DWRITE_FONT_WEIGHT_SEMI_BOLD, 13.5f},     // Button
        {icons, DWRITE_FONT_WEIGHT_NORMAL, 14.0f},       // Icon
        {icons, DWRITE_FONT_WEIGHT_NORMAL, 22.0f},       // IconLarge
    };
    for (int i = 0; i < 7; ++i) {
        if (FAILED(g_dwrite->CreateTextFormat(specs[i].family, nullptr, specs[i].weight, DWRITE_FONT_STYLE_NORMAL,
                                              DWRITE_FONT_STRETCH_NORMAL, specs[i].size, L"", &g_formats[i]))) {
            return false;
        }
        g_dwrite->CreateEllipsisTrimmingSign(g_formats[i], &g_ellipsis[i]);
    }
    return true;
}

IDWriteTextLayout* layout(const std::wstring& s, float width, float height, Font f, bool wrap,
                          DWRITE_TEXT_ALIGNMENT align = DWRITE_TEXT_ALIGNMENT_LEADING) {
    const int i = static_cast<int>(f);
    IDWriteTextLayout* out = nullptr;
    if (FAILED(g_dwrite->CreateTextLayout(s.c_str(), static_cast<UINT32>(s.size()), g_formats[i],
                                          std::max(1.0f, width), std::max(1.0f, height), &out))) {
        return nullptr;
    }
    out->SetWordWrapping(wrap ? DWRITE_WORD_WRAPPING_WRAP : DWRITE_WORD_WRAPPING_NO_WRAP);
    out->SetTextAlignment(align);
    DWRITE_TRIMMING trim{DWRITE_TRIMMING_GRANULARITY_CHARACTER, 0, 0};
    out->SetTrimming(&trim, g_ellipsis[i]);
    return out;
}

}  // namespace

bool init() {
    if (g_d2d) return true;
    if (FAILED(D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, &g_d2d))) return false;
    if (FAILED(DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED, __uuidof(IDWriteFactory),
                                   reinterpret_cast<IUnknown**>(&g_dwrite)))) {
        release(g_d2d);
        return false;
    }
    CoCreateInstance(CLSID_WICImagingFactory, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&g_wic));
    if (!make_formats()) {
        release(g_dwrite);
        release(g_d2d);
        return false;
    }
    return true;
}

void enable_dpi_awareness() {
    using SetContext = BOOL(WINAPI*)(HANDLE);
    const auto set = reinterpret_cast<SetContext>(
        reinterpret_cast<void*>(GetProcAddress(GetModuleHandleW(L"user32.dll"), "SetProcessDpiAwarenessContext")));
    if (set) set(reinterpret_cast<HANDLE>(static_cast<LONG_PTR>(-4)));  // PER_MONITOR_AWARE_V2
}

UINT dpi_of(HWND window) {
    using GetDpi = UINT(WINAPI*)(HWND);
    static const auto get = reinterpret_cast<GetDpi>(
        reinterpret_cast<void*>(GetProcAddress(GetModuleHandleW(L"user32.dll"), "GetDpiForWindow")));
    const UINT dpi = get && window ? get(window) : 96;
    return dpi ? dpi : 96;
}

void dark_frame(HWND window, bool rounded) {
    const BOOL dark = TRUE;
    DwmSetWindowAttribute(window, 20, &dark, sizeof(dark));  // DWMWA_USE_IMMERSIVE_DARK_MODE
    const int corners = rounded ? 2 : 0;                      // DWMWCP_ROUND
    DwmSetWindowAttribute(window, 33, &corners, sizeof(corners));
    const COLORREF border = RGB(52, 52, 56);
    DwmSetWindowAttribute(window, 34, &border, sizeof(border));  // DWMWA_BORDER_COLOR
}

std::wstring size_text(unsigned long long bytes) {
    wchar_t buf[32];
    if (bytes < 1000) {
        std::swprintf(buf, 32, L"%llu bytes", bytes);
    } else if (bytes < 1000ull * 1000) {
        std::swprintf(buf, 32, L"%.0f KB", static_cast<double>(bytes) / 1000.0);
    } else if (bytes < 1000ull * 1000 * 1000) {
        std::swprintf(buf, 32, L"%.1f MB", static_cast<double>(bytes) / 1e6);
    } else {
        std::swprintf(buf, 32, L"%.2f GB", static_cast<double>(bytes) / 1e9);
    }
    return buf;
}

// --- Canvas ------------------------------------------------------------------------

Canvas::Canvas(HWND window) : window_(window) {}

Canvas::~Canvas() { drop_target(); }

void Canvas::drop_target() {
    for (auto& [source, bitmap] : images_) {
        source->Release();
        bitmap->Release();
    }
    images_.clear();
    release(brush_);
    release(target_);
}

bool Canvas::make_target() {
    if (target_) return true;
    if (!g_d2d) return false;
    RECT rc;
    GetClientRect(window_, &rc);
    const UINT dpi = dpi_of(window_);
    D2D1_RENDER_TARGET_PROPERTIES props = D2D1::RenderTargetProperties();
    props.dpiX = props.dpiY = static_cast<float>(dpi);
    if (FAILED(g_d2d->CreateHwndRenderTarget(
            props,
            D2D1::HwndRenderTargetProperties(window_, D2D1::SizeU(static_cast<UINT32>(rc.right - rc.left),
                                                                  static_cast<UINT32>(rc.bottom - rc.top))),
            &target_))) {
        return false;
    }
    target_->SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
    return SUCCEEDED(target_->CreateSolidColorBrush(D2D1::ColorF(0, 0, 0), &brush_));
}

bool Canvas::begin() {
    if (!make_target()) return false;
    const float dpi = static_cast<float>(dpi_of(window_));
    target_->SetDpi(dpi, dpi);
    target_->BeginDraw();
    return true;
}

void Canvas::end() {
    if (target_ && target_->EndDraw() == static_cast<HRESULT>(D2DERR_RECREATE_TARGET)) drop_target();
}

void Canvas::resize() {
    if (!target_) return;
    RECT rc;
    GetClientRect(window_, &rc);
    target_->Resize(D2D1::SizeU(static_cast<UINT32>(rc.right - rc.left), static_cast<UINT32>(rc.bottom - rc.top)));
}

float Canvas::width() const {
    RECT rc;
    GetClientRect(window_, &rc);
    return static_cast<float>(rc.right - rc.left) * 96.0f / static_cast<float>(dpi_of(window_));
}

float Canvas::height() const {
    RECT rc;
    GetClientRect(window_, &rc);
    return static_cast<float>(rc.bottom - rc.top) * 96.0f / static_cast<float>(dpi_of(window_));
}

ID2D1SolidColorBrush* Canvas::brush(Color c) {
    brush_->SetColor(d2d(c));
    return brush_;
}

void Canvas::clear(Color c) { target_->Clear(d2d(c)); }

void Canvas::fill(D2D1_RECT_F r, float radius, Color c) {
    if (radius <= 0) {
        target_->FillRectangle(r, brush(c));
    } else {
        target_->FillRoundedRectangle(D2D1::RoundedRect(r, radius, radius), brush(c));
    }
}

void Canvas::outline(D2D1_RECT_F r, float radius, Color c, float width) {
    const float h = width / 2;
    const D2D1_RECT_F in{r.left + h, r.top + h, r.right - h, r.bottom - h};
    target_->DrawRoundedRectangle(D2D1::RoundedRect(in, radius, radius), brush(c), width);
}

void Canvas::line(D2D1_POINT_2F a, D2D1_POINT_2F b, Color c, float width) {
    target_->DrawLine(a, b, brush(c), width);
}

void Canvas::dot(D2D1_POINT_2F centre, float radius, Color c) {
    target_->FillEllipse(D2D1::Ellipse(centre, radius, radius), brush(c));
}

void Canvas::text(const std::wstring& s, D2D1_RECT_F r, Font f, Color c, DWRITE_TEXT_ALIGNMENT align, bool wrap) {
    IDWriteTextLayout* l = layout(s, r.right - r.left, r.bottom - r.top, f, wrap, align);
    if (!l) return;
    l->SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
    target_->PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_ALIASED);
    target_->DrawTextLayout(D2D1::Point2F(r.left, r.top), l, brush(c), D2D1_DRAW_TEXT_OPTIONS_CLIP);
    target_->PopAxisAlignedClip();
    l->Release();
}

float Canvas::text_height(const std::wstring& s, float width, Font f, bool wrap) const {
    IDWriteTextLayout* l = layout(s.empty() ? L" " : s, width, 10000.0f, f, wrap);
    if (!l) return 16.0f;
    DWRITE_TEXT_METRICS m{};
    l->GetMetrics(&m);
    l->Release();
    return std::ceil(m.height);
}

void Canvas::progress(D2D1_RECT_F r, double fraction, double phase, Color c) {
    const float h = r.bottom - r.top;
    fill(r, h / 2, theme::faint);
    const float w = r.right - r.left;
    if (fraction >= 0) {
        const float fw = static_cast<float>(std::min(1.0, fraction)) * w;
        if (fw > 0.5f) fill(D2D1::RectF(r.left, r.top, r.left + std::max(fw, h), r.bottom), h / 2, c);
        return;
    }
    // Indeterminate: a third of the track sliding across, clipped to it.
    const float seg = w * 0.32f;
    const float x = r.left - seg + static_cast<float>(phase) * (w + seg);
    target_->PushAxisAlignedClip(r, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
    fill(D2D1::RectF(x, r.top, x + seg, r.bottom), h / 2, c);
    target_->PopAxisAlignedClip();
}

void Canvas::image(IWICBitmapSource* source, D2D1_RECT_F r, float radius) {
    if (!source) return;
    ID2D1Bitmap* bitmap = nullptr;
    for (const auto& [s, b] : images_) {
        if (s == source) bitmap = b;
    }
    if (!bitmap) {
        if (FAILED(target_->CreateBitmapFromWicBitmap(source, nullptr, &bitmap))) return;
        source->AddRef();
        images_.emplace_back(source, bitmap);
        if (images_.size() > 64) {  // the oldest goes
            images_.front().first->Release();
            images_.front().second->Release();
            images_.erase(images_.begin());
        }
    }
    const D2D1_SIZE_F size = bitmap->GetSize();
    if (size.width <= 0 || size.height <= 0) return;
    // Cover r: scale to the larger ratio, centred.
    const float w = r.right - r.left;
    const float h = r.bottom - r.top;
    const float s = std::max(w / size.width, h / size.height);
    const float ox = r.left + (w - size.width * s) / 2;
    const float oy = r.top + (h - size.height * s) / 2;
    ID2D1BitmapBrush* b = nullptr;
    if (FAILED(target_->CreateBitmapBrush(bitmap, &b))) return;
    b->SetInterpolationMode(D2D1_BITMAP_INTERPOLATION_MODE_LINEAR);
    b->SetTransform(D2D1::Matrix3x2F::Scale(s, s) * D2D1::Matrix3x2F::Translation(ox, oy));
    target_->FillRoundedRectangle(D2D1::RoundedRect(r, radius, radius), b);
    b->Release();
}

IWICBitmapSource* thumbnail(const std::filesystem::path& file, int pixels) {
    if (!g_wic) return nullptr;
    IShellItemImageFactory* factory = nullptr;
    if (FAILED(SHCreateItemFromParsingName(file.c_str(), nullptr, IID_PPV_ARGS(&factory)))) return nullptr;
    HBITMAP bitmap = nullptr;
    const HRESULT hr = factory->GetImage(SIZE{pixels, pixels}, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK, &bitmap);
    factory->Release();
    if (FAILED(hr) || !bitmap) return nullptr;
    IWICBitmap* wic = nullptr;
    // Photos' thumbnails carry no meaningful alpha: ignore it.
    const HRESULT made = g_wic->CreateBitmapFromHBITMAP(bitmap, nullptr, WICBitmapIgnoreAlpha, &wic);
    DeleteObject(bitmap);
    if (FAILED(made)) return nullptr;
    IWICFormatConverter* converter = nullptr;
    if (FAILED(g_wic->CreateFormatConverter(&converter)) ||
        FAILED(converter->Initialize(wic, GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, nullptr, 0,
                                     WICBitmapPaletteTypeCustom))) {
        if (converter) converter->Release();
        wic->Release();
        return nullptr;
    }
    wic->Release();
    return converter;
}

}  // namespace howldrop::ui
