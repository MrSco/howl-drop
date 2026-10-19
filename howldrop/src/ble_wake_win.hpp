#pragma once

// The AirDrop wake from Windows' own Bluetooth. An iPhone set to Everyone
// opens its AirDrop port only after a sender's Bluetooth LE advert: Apple
// Continuity type 0x05 in manufacturer data (company 0x004C), the one
// omdrop-awdl's ble-airdrop-adv.py sends (arm A, zero contact hashes). In
// WSL there is no Bluetooth, so howldrop.exe sends it while AirDrop on this
// PC sends or looks for devices (wsl_win.hpp). Receiving needs no wake.
//
// WinRT's BluetoothLEAdvertisementPublisher, through its ABI (MinGW has no
// headers for it; the interface ids were checked against what Windows
// reports, 2026-10-18). The PC's Bluetooth stays Windows': nothing connected
// to it drops. Windows only.

#include <string>

namespace howldrop {

class BleWake {
public:
    BleWake() = default;
    ~BleWake();
    BleWake(const BleWake&) = delete;
    BleWake& operator=(const BleWake&) = delete;

    // On the thread that calls them, which must have called RoInitialize.
    // start: empty, or why it could not (Bluetooth off, say).
    std::string start();
    void stop();
    bool on() const { return publisher_ != nullptr; }

private:
    void* publisher_ = nullptr;  // the publisher's interface, while advertising
};

}  // namespace howldrop
