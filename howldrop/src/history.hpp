#pragma once

// What came in and went out lately, for the HowlDrop window's Recent list:
// %LOCALAPPDATA%\HowlDrop\history.tsv, one line per file, appended by the
// tray as each transfer finishes, trimmed to the newest kKept.
//   TIME<TAB>in|out<TAB>PEER<TAB>PATH-OR-NAME
// A received file that has since gone (moved, deleted, the 30-day cleanup)
// drops out of the list when it's read.

#include <cstdint>
#include <ctime>
#include <filesystem>
#include <string>
#include <vector>

namespace howldrop::history {

constexpr std::size_t kKept = 200;

struct Entry {
    std::int64_t time = 0;           // seconds since 1970
    bool out = false;                // sent from this PC
    std::string peer;                // who sent it, or who it went to
    std::string item;                // in: the file's path (UTF-8); out: its name
};

void append(const std::filesystem::path& file, const Entry& entry);
// Newest first, at most limit; received files that no longer exist are left out.
std::vector<Entry> load(const std::filesystem::path& file, std::size_t limit);

}  // namespace howldrop::history
