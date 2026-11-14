// Exercise the real window's native keyboard and accessibility controls,
// with no network, radio or user settings.
#include "window_win.hpp"
#include <oleacc.h>
#include <cstdio>

namespace {
int failures = 0;
const IID kAccessible = {0x618736e0, 0x3c3d, 0x11cf, {0x81, 0x0c, 0x00, 0xaa, 0x00, 0x38, 0x9b, 0x71}};
void expect(bool ok, const char* message) {
    if (!ok) { std::fprintf(stderr, "FAIL: %s\n", message); ++failures; }
}
HWND test_window() {
    HWND result = nullptr;
    EnumThreadWindows(GetCurrentThreadId(), [](HWND w, LPARAM data) -> BOOL {
        wchar_t name[100] = {};
        GetClassNameW(w, name, 100);
        if (std::wstring(name) == L"HowlDropWindow") { *reinterpret_cast<HWND*>(data) = w; return FALSE; }
        return TRUE;
    }, reinterpret_cast<LPARAM>(&result));
    return result;
}
HWND button(HWND parent, const wchar_t* label) {
    for (HWND child = GetWindow(parent, GW_CHILD); child; child = GetWindow(child, GW_HWNDNEXT)) {
        wchar_t name[2048] = {};
        GetWindowTextW(child, name, 2048);
        if (IsWindowVisible(child) && std::wstring(name) == label) return child;
    }
    return nullptr;
}
void paint(HWND window) { RedrawWindow(window, nullptr, nullptr, RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN); }
}

int main() {
    const HRESULT com = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    {
        howldrop::WindowSettings settings;
        bool scan = false, cancelled = false;
        howldrop::HowlWindow::Hooks hooks;
        hooks.settings = [&]() { return settings; };
        hooks.apply = [&](const auto& s) { settings = s; };
        hooks.nearby = []() { howldrop::Node::Nearby nearby; nearby.relay = nearby.local = true; return nearby; };
        hooks.scan = [&]() { scan = true; };
        hooks.control_send = [&](const auto& id, bool retry) { cancelled = id == "job" && !retry; };
        hooks.transfers = []() {
            howldrop::Transfers::Transfer transfer;
            transfer.id = "job"; transfer.controllable = true;
            transfer.direction = howldrop::Transfers::Direction::Out;
            transfer.names = {"photo.jpg"}; transfer.count = 1;
            return std::vector<howldrop::Transfers::Transfer>{transfer};
        };
        howldrop::HowlWindow window(GetModuleHandleW(nullptr), L"UI test", {}, std::move(hooks));
        window.show();
        const HWND hwnd = test_window();
        expect(hwnd != nullptr, "test window opens");
        paint(hwnd);
        HWND cancel = button(hwnd, L"Cancel transfer");
        expect(cancel != nullptr, "transfer has a native Cancel control");
        if (cancel) {
            IAccessible* accessible = nullptr;
            expect(SUCCEEDED(AccessibleObjectFromWindow(cancel, OBJID_CLIENT, kAccessible,
                                                       reinterpret_cast<void**>(&accessible))), "control exposes MSAA");
            if (accessible) {
                VARIANT self{}; self.vt = VT_I4; self.lVal = CHILDID_SELF;
                BSTR name = nullptr;
                expect(SUCCEEDED(accessible->get_accName(self, &name)) && name && std::wstring(name) == L"Cancel transfer",
                       "screen reader gets the action name");
                SysFreeString(name); accessible->Release();
            }
            SetFocus(cancel);
            MSG enter{}; enter.hwnd = cancel; enter.message = WM_KEYDOWN; enter.wParam = VK_RETURN;
            expect(window.handle_message(enter) && cancelled, "Enter activates Cancel");
            MSG tab{}; tab.hwnd = cancel; tab.message = WM_KEYDOWN; tab.wParam = VK_TAB;
            expect(window.handle_message(tab) && GetFocus() != cancel, "Tab advances focus");
        }
        const HWND scan_button = button(hwnd, L"Look for devices");
        if (scan_button) SendMessageW(scan_button, BM_CLICK, 0, 0);
        expect(scan, "native click activates device scan");
        expect(button(hwnd, L"Choose a folder to send") != nullptr, "folder sending is accessible");
        const HWND gear = button(hwnd, L"Settings");
        if (gear) SendMessageW(gear, BM_CLICK, 0, 0);
        paint(hwnd);
        const HWND radio = button(hwnd, L"AirDrop on this PC, off");
        if (radio) SendMessageW(radio, BM_CLICK, 0, 0);
        paint(hwnd);
        expect(settings.radio && button(hwnd, L"AirDrop on this PC, on"), "toggle state updates its accessible name");
        SendMessageW(hwnd, WM_CLOSE, 0, 0);
    }
    if (SUCCEEDED(com)) CoUninitialize();
    if (!failures) std::puts("windows_ui_test passed");
    return failures ? 1 : 0;
}
