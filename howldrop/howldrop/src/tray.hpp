#pragma once

#include "node.hpp"

namespace howldrop {

// The Windows tray app: runs config's node with an icon by the clock, a menu
// of settings, a notification for each file or text that arrives, and the
// opening of the kinds the settings allow (open_policy.hpp). Returns when the
// user quits. Windows only.
int run_tray(Node::Config config);

}  // namespace howldrop
