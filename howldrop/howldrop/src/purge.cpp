#include "purge.hpp"

#include <system_error>
#include <vector>

#ifdef _WIN32
#include <windows.h>
#else
#include <sys/stat.h>
#endif

namespace howldrop {
namespace {

using SysClock = std::chrono::system_clock;

// When the file arrived, or nothing when it cannot be read.
bool arrived(const std::filesystem::path& path, SysClock::time_point& when) {
#ifdef _WIN32
    WIN32_FILE_ATTRIBUTE_DATA data{};
    if (!GetFileAttributesExW(path.c_str(), GetFileExInfoStandard, &data)) return false;
    const auto ticks = [](const FILETIME& t) {
        return (static_cast<unsigned long long>(t.dwHighDateTime) << 32) | t.dwLowDateTime;
    };
    unsigned long long latest = ticks(data.ftLastWriteTime);
    if (ticks(data.ftCreationTime) > latest) latest = ticks(data.ftCreationTime);
    // FILETIME: 100 ns ticks since 1601; the Unix epoch is 11644473600 s later.
    const long long unix_100ns = static_cast<long long>(latest) - 116444736000000000LL;
    when = SysClock::time_point(std::chrono::duration_cast<SysClock::duration>(
        std::chrono::duration<long long, std::ratio<1, 10000000>>(unix_100ns)));
    return true;
#else
    struct stat st {};
    if (lstat(path.c_str(), &st) != 0) return false;
    const auto latest = st.st_mtime > st.st_ctime ? st.st_mtime : st.st_ctime;
    when = SysClock::from_time_t(latest);
    return true;
#endif
}

}  // namespace

int purge_old_files(const std::filesystem::path& dir, int days, SysClock::time_point now) {
    if (days <= 0) return 0;
    const auto cutoff = now - std::chrono::hours(24) * days;
    std::error_code ec;
    if (!std::filesystem::is_directory(dir, ec)) return 0;
    int removed = 0;
    std::vector<std::filesystem::path> folders;
    for (auto it = std::filesystem::recursive_directory_iterator(
             dir, std::filesystem::directory_options::skip_permission_denied, ec);
         !ec && it != std::filesystem::recursive_directory_iterator(); it.increment(ec)) {
        const auto& entry = *it;
        std::error_code e;
        if (entry.is_directory(e) && !entry.is_symlink(e)) {
            folders.push_back(entry.path());
            continue;
        }
        SysClock::time_point when;
        if (!arrived(entry.path(), when) || when >= cutoff) continue;
        if (std::filesystem::remove(entry.path(), e)) ++removed;
    }
    // Deepest first, so a folder emptied by its children goes too.
    for (auto f = folders.rbegin(); f != folders.rend(); ++f) {
        if (*f == dir / "text") continue;
        std::error_code e;
        if (std::filesystem::is_empty(*f, e) && !e) std::filesystem::remove(*f, e);
    }
    return removed;
}

}  // namespace howldrop
