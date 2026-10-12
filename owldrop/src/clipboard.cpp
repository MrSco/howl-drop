#include "clipboard.hpp"

#include "text.hpp"

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>
#endif

#include <cstring>

namespace owldrop {

ClipRead MemoryClipboard::read() {
    std::lock_guard<std::mutex> lock(mu_);
    return ClipRead{ClipRead::Status::Text, text_};
}

bool MemoryClipboard::set_text(const std::string& utf8) {
    std::lock_guard<std::mutex> lock(mu_);
    text_ = utf8;
    ++seq_;
    return true;
}

std::uint32_t MemoryClipboard::sequence() {
    std::lock_guard<std::mutex> lock(mu_);
    return seq_;
}

#ifdef _WIN32

class WinClipboard final : public Clipboard {
public:
    ClipRead read() override {
        std::lock_guard<std::mutex> lock(mu_);
        if (!OpenClipboard(nullptr)) return ClipRead{ClipRead::Status::Busy, {}};
        struct CloseClipboardGuard {
            ~CloseClipboardGuard() { CloseClipboard(); }
        } guard;
        HANDLE data = GetClipboardData(CF_UNICODETEXT);
        if (!data) return ClipRead{ClipRead::Status::NoText, {}};
        const auto* wide = static_cast<const wchar_t*>(GlobalLock(data));
        if (!wide) return ClipRead{ClipRead::Status::Busy, {}};
        std::wstring text(wide);
        GlobalUnlock(data);
        return ClipRead{ClipRead::Status::Text, utf8_from_wide(text)};
    }

    bool set_text(const std::string& utf8) override {
        std::lock_guard<std::mutex> lock(mu_);
        auto wide = wide_from_utf8(utf8);
        if (!wide) return false;
        const std::size_t bytes = (wide->size() + 1) * sizeof(wchar_t);
        HGLOBAL memory = GlobalAlloc(GMEM_MOVEABLE, bytes);
        if (!memory) return false;
        auto* dest = static_cast<wchar_t*>(GlobalLock(memory));
        if (!dest) {
            GlobalFree(memory);
            return false;
        }
        std::memcpy(dest, wide->c_str(), bytes);
        GlobalUnlock(memory);
        if (!OpenClipboard(nullptr)) {
            GlobalFree(memory);
            return false;
        }
        struct CloseClipboardGuard {
            ~CloseClipboardGuard() { CloseClipboard(); }
        } guard;
        if (!EmptyClipboard()) {
            GlobalFree(memory);
            return false;
        }
        if (!SetClipboardData(CF_UNICODETEXT, memory)) {
            GlobalFree(memory);
            return false;
        }
        return true;
    }

    std::uint32_t sequence() override { return GetClipboardSequenceNumber(); }

private:
    std::mutex mu_;
};

std::unique_ptr<Clipboard> make_system_clipboard() { return std::make_unique<WinClipboard>(); }

#else

std::unique_ptr<Clipboard> make_system_clipboard() { return std::make_unique<MemoryClipboard>(); }

#endif

}  // namespace owldrop
