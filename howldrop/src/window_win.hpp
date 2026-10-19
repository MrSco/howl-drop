#pragma once

// The HowlDrop window, opened by clicking the tray icon. One page to send
// from: drop files on it (or click to choose), pick a nearby device (Look
// for devices) or any nearby iPhone, and see what came and went lately.
// A second page holds the settings, including who may send and the contacts
// accepted without asking. Same look as the transfer cards (ui_win.hpp).
// Windows only.

#include "node.hpp"
#include "transfers.hpp"
#include "wsl_win.hpp"

#include <windows.h>

#include <filesystem>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace howldrop {

struct WindowSettings {
    bool radio = false;
    bool cleanup = true;
    bool notify = true;
    bool open_links = true;
    bool open_media = false;
    bool open_documents = false;
    bool start_with_windows = false;
    std::string receive = "ask";  // everyone, ask, contacts
    std::string senders;          // comma-separated
};

class HowlWindow {
public:
    struct Hooks {
        std::function<WindowSettings()> settings;
        std::function<void(const WindowSettings&)> apply;  // the settings page changed something
        std::function<Node::Nearby()> nearby;
        std::function<void()> scan;
        // to: a device's name, or empty for any nearby iPhone.
        std::function<void(const std::vector<std::filesystem::path>& files, const std::string& to)> send;
        std::function<void(const std::filesystem::path& file)> open;
        std::function<void(const std::filesystem::path& file)> show;
        std::function<void()> open_folder;
        std::function<std::wstring()> status;  // how this PC is reachable, in words
        std::function<RadioLevel()> level;
        // What's happening now: the window shows it instead of the cards.
        std::function<std::vector<Transfers::Transfer>()> transfers;
        std::function<void(const std::string& id, bool accept)> decide;
        // The window came up (true), or closed or was minimized (false).
        std::function<void(bool showing)> on_visibility;
    };

    HowlWindow(HINSTANCE instance, std::wstring name, std::filesystem::path history, Hooks hooks);
    ~HowlWindow();
    HowlWindow(const HowlWindow&) = delete;
    HowlWindow& operator=(const HowlWindow&) = delete;

    void show();     // open it, or bring it to the front
    void refresh();  // nearby devices, status or history may have changed
    bool open() const;
    bool showing() const;  // open, and not minimized
    // Something arrives: an open window comes to the top, so its Accept
    // and progress are seen (Windows may keep the keyboard elsewhere).
    void raise();

    // The main page (or settings) as a PNG, from hooks with made-up data
    // (`howldrop screenshots`).
    static bool render(const std::filesystem::path& file, bool settings, float scale, std::wstring name,
                       std::filesystem::path history, Hooks hooks);

    struct Impl;

private:
    std::unique_ptr<Impl> impl_;
};

}  // namespace howldrop
