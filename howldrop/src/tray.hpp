#pragma once

#include "node.hpp"

namespace howldrop {

// The Windows tray app: runs config's node with an icon by the clock, a menu
// of settings, a notification for each file or text that arrives, and the
// opening of the kinds the settings allow (open_policy.hpp). Returns when the
// user quits. Windows only.
int run_tray(Node::Config config);

// `howldrop screenshots DIR`: the window, its settings and the transfer
// cards as PNGs, drawn from made-up data, for the README
// (screenshots_win.cpp). Windows only.
int make_screenshots(const std::filesystem::path& dir);

}  // namespace howldrop
