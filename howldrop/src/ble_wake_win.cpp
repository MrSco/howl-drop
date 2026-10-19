// The AirDrop wake (ble_wake_win.hpp): WinRT's BLE advertisement publisher,
// called through its ABI.

#include "ble_wake_win.hpp"

#include <windows.h>
#include <inspectable.h>
#include <roapi.h>
#include <windows.storage.streams.h>
#include <winstring.h>

#include <cstdio>
#include <cstring>
#include <cwchar>
#include <random>

namespace howldrop {

// Windows.Devices.Bluetooth.Advertisement, as much as the wake uses. The
// methods are declared in their ABI order up to the last one called. Not in
// an anonymous namespace: there GCC sees that nothing in the program
// implements them and, at -O2, turns the calls into jumps to nowhere.
namespace winrt_abi {

struct IVectorAbi : public IInspectable {
    virtual HRESULT STDMETHODCALLTYPE GetAt(UINT32 index, void** item) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_Size(UINT32* size) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetView(void** view) = 0;
    virtual HRESULT STDMETHODCALLTYPE IndexOf(void* item, UINT32* index, boolean* found) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetAt(UINT32 index, void* item) = 0;
    virtual HRESULT STDMETHODCALLTYPE InsertAt(UINT32 index, void* item) = 0;
    virtual HRESULT STDMETHODCALLTYPE RemoveAt(UINT32 index) = 0;
    virtual HRESULT STDMETHODCALLTYPE Append(void* item) = 0;
};

struct IManufacturerData : public IInspectable {
    virtual HRESULT STDMETHODCALLTYPE get_CompanyId(UINT16* value) = 0;
    virtual HRESULT STDMETHODCALLTYPE put_CompanyId(UINT16 value) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_Data(ABI::Windows::Storage::Streams::IBuffer** value) = 0;
    virtual HRESULT STDMETHODCALLTYPE put_Data(ABI::Windows::Storage::Streams::IBuffer* value) = 0;
};

struct IAdvertisement : public IInspectable {
    virtual HRESULT STDMETHODCALLTYPE get_Flags(void** value) = 0;
    virtual HRESULT STDMETHODCALLTYPE put_Flags(void* value) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_LocalName(HSTRING* value) = 0;
    virtual HRESULT STDMETHODCALLTYPE put_LocalName(HSTRING value) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_ServiceUuids(void** value) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_ManufacturerData(IVectorAbi** value) = 0;
};

// The bytes behind a WinRT buffer (robuffer.h's, which MinGW only declares).
struct IBufferBytes : public IUnknown {
    virtual HRESULT STDMETHODCALLTYPE Buffer(BYTE** value) = 0;
};

struct IPublisher : public IInspectable {
    virtual HRESULT STDMETHODCALLTYPE get_Status(INT32* value) = 0;
    virtual HRESULT STDMETHODCALLTYPE get_Advertisement(IAdvertisement** value) = 0;
    virtual HRESULT STDMETHODCALLTYPE Start() = 0;
    virtual HRESULT STDMETHODCALLTYPE Stop() = 0;
};

}  // namespace winrt_abi

namespace {

using namespace winrt_abi;

const IID kPublisherIid = {0xCDE820F9, 0xD9FA, 0x43D6, {0xA2, 0x64, 0xDD, 0xD8, 0xB7, 0xDA, 0x8B, 0x78}};
const IID kAdvertisementIid = {0x066FB2B7, 0x33D1, 0x4E7D, {0x83, 0x67, 0xCF, 0x81, 0xD0, 0xF7, 0x96, 0x53}};
const IID kManufacturerDataIid = {0x912DBA18, 0x6963, 0x4533, {0xB0, 0x61, 0x46, 0x94, 0xDA, 0xFB, 0x34, 0xE5}};
const IID kBufferFactoryIid = {0x71AF914D, 0xC10F, 0x484B, {0xBC, 0x50, 0x14, 0xBC, 0x62, 0x3B, 0x3A, 0x27}};
const IID kBufferByteAccessIid = {0x905A0FEF, 0xBC53, 0x11DF, {0x8C, 0x49, 0x00, 0x1E, 0x4F, 0xC6, 0x86, 0xDA}};

constexpr INT32 kStarted = 2;  // BluetoothLEAdvertisementPublisherStatus
constexpr INT32 kAborted = 5;
constexpr UINT16 kApple = 0x004C;

template <class T>
void release(T*& p) {
    if (p) p->Release();
    p = nullptr;
}

HRESULT activate(const wchar_t* cls, REFIID iid, void** out) {
    HSTRING name = nullptr;
    HRESULT hr = WindowsCreateString(cls, static_cast<UINT32>(std::wcslen(cls)), &name);
    if (FAILED(hr)) return hr;
    IInspectable* object = nullptr;
    hr = RoActivateInstance(name, &object);
    WindowsDeleteString(name);
    if (FAILED(hr)) return hr;
    hr = object->QueryInterface(iid, out);
    object->Release();
    return hr;
}

// A WinRT buffer holding bytes.
HRESULT make_buffer(const unsigned char* bytes, UINT32 n, ABI::Windows::Storage::Streams::IBuffer** out) {
    const wchar_t* cls = L"Windows.Storage.Streams.Buffer";
    HSTRING name = nullptr;
    HRESULT hr = WindowsCreateString(cls, static_cast<UINT32>(std::wcslen(cls)), &name);
    if (FAILED(hr)) return hr;
    ABI::Windows::Storage::Streams::IBufferFactory* factory = nullptr;
    hr = RoGetActivationFactory(name, kBufferFactoryIid, reinterpret_cast<void**>(&factory));
    WindowsDeleteString(name);
    if (FAILED(hr)) return hr;
    ABI::Windows::Storage::Streams::IBuffer* buffer = nullptr;
    hr = factory->Create(n, &buffer);
    factory->Release();
    if (FAILED(hr)) return hr;
    IBufferBytes* access = nullptr;
    hr = buffer->QueryInterface(kBufferByteAccessIid, reinterpret_cast<void**>(&access));
    if (SUCCEEDED(hr)) {
        BYTE* data = nullptr;
        hr = access->Buffer(&data);
        if (SUCCEEDED(hr)) {
            std::memcpy(data, bytes, n);
            hr = buffer->put_Length(n);
        }
        access->Release();
    }
    if (FAILED(hr)) {
        buffer->Release();
        return hr;
    }
    *out = buffer;
    return S_OK;
}

std::string hex(HRESULT hr) {
    char buf[16];
    std::snprintf(buf, sizeof(buf), "0x%08lX", static_cast<unsigned long>(hr));
    return buf;
}

}  // namespace

BleWake::~BleWake() { stop(); }

std::string BleWake::start() {
    if (publisher_) return {};
    // 05 12, then 18 bytes: 40, a 3-byte tag (random, as a Mac's rotates),
    // 00000000, 03, four zero 2-byte contact hashes, 00.
    unsigned char value[20] = {0x05, 0x12, 0x40, 0, 0, 0, 0, 0, 0, 0, 0x03};
    std::random_device random;
    for (int i = 3; i < 6; ++i) value[i] = static_cast<unsigned char>(random() & 0xff);

    IPublisher* publisher = nullptr;
    IAdvertisement* advertisement = nullptr;
    IVectorAbi* sections = nullptr;
    IManufacturerData* data = nullptr;
    ABI::Windows::Storage::Streams::IBuffer* buffer = nullptr;
    std::string error;
    HRESULT hr = activate(L"Windows.Devices.Bluetooth.Advertisement.BluetoothLEAdvertisementPublisher", kPublisherIid,
                          reinterpret_cast<void**>(&publisher));
    if (SUCCEEDED(hr)) hr = publisher->get_Advertisement(&advertisement);
    if (SUCCEEDED(hr)) {
        IAdvertisement* checked = nullptr;  // the interface it says it is
        hr = advertisement->QueryInterface(kAdvertisementIid, reinterpret_cast<void**>(&checked));
        release(checked);
    }
    if (SUCCEEDED(hr)) hr = advertisement->get_ManufacturerData(&sections);
    if (SUCCEEDED(hr)) {
        hr = activate(L"Windows.Devices.Bluetooth.Advertisement.BluetoothLEManufacturerData", kManufacturerDataIid,
                      reinterpret_cast<void**>(&data));
    }
    if (SUCCEEDED(hr)) hr = data->put_CompanyId(kApple);
    if (SUCCEEDED(hr)) hr = make_buffer(value, sizeof(value), &buffer);
    if (SUCCEEDED(hr)) hr = data->put_Data(buffer);
    if (SUCCEEDED(hr)) hr = sections->Append(data);
    UINT32 count = 0;
    if (SUCCEEDED(hr)) hr = sections->get_Size(&count);
    if (SUCCEEDED(hr) && count != 1) error = "the advert did not take its data";
    if (SUCCEEDED(hr) && error.empty()) hr = publisher->Start();
    INT32 status = 0;
    for (int i = 0; SUCCEEDED(hr) && error.empty() && i < 30; ++i) {
        if (FAILED(publisher->get_Status(&status)) || status == kStarted || status == kAborted) break;
        Sleep(100);
    }
    if (FAILED(hr)) {
        error = "Bluetooth advertising failed (" + hex(hr) + ")";
    } else if (error.empty() && status != kStarted) {
        error = status == kAborted ? "Bluetooth is off or busy (advertising aborted)"
                                   : "Bluetooth advertising did not start (status " + std::to_string(status) + ")";
    }
    release(buffer);
    release(data);
    release(sections);
    release(advertisement);
    if (!error.empty()) {
        if (publisher) publisher->Stop();
        release(publisher);
        return error;
    }
    publisher_ = publisher;
    return {};
}

void BleWake::stop() {
    auto* publisher = static_cast<IPublisher*>(publisher_);
    if (!publisher) return;
    publisher->Stop();
    publisher->Release();
    publisher_ = nullptr;
}

}  // namespace howldrop
