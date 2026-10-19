#include "transfers.hpp"

#include <algorithm>
#include <cctype>

namespace howldrop {
namespace {

using Clock = Transfers::Clock;

constexpr std::chrono::seconds kOutStall{240};  // outbox.py tries a file for 180 s

std::uint64_t number(const std::string& s) {
    std::uint64_t n = 0;
    for (char c : s) {
        if (c < '0' || c > '9') return n;
        n = n * 10 + static_cast<std::uint64_t>(c - '0');
    }
    return n;
}

std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return s;
}

// deliver.py keeps a name, or makes it NAME-1.EXT beside one already there.
bool same_file(const std::string& asked, const std::string& landed) {
    if (asked == landed) return true;
    const auto dot = asked.rfind('.');
    const std::string stem = asked.substr(0, dot);
    const std::string ext = dot == std::string::npos ? std::string() : asked.substr(dot);
    if (landed.size() <= stem.size() + ext.size() + 1 || landed.compare(0, stem.size(), stem) != 0) return false;
    if (landed[stem.size()] != '-') return false;
    return landed.compare(landed.size() - ext.size(), ext.size(), ext) == 0;
}

bool is_link(const std::string& name) { return name.rfind("http://", 0) == 0 || name.rfind("https://", 0) == 0; }

}  // namespace

double Transfers::Transfer::fraction() const {
    if (stage == Stage::Done || stage == Stage::Saving) return 1.0;
    if (stage != Stage::Moving || total == 0) return -1.0;
    return std::min(1.0, static_cast<double>(bytes) / static_cast<double>(total));
}

std::string Transfers::kind_of(const std::filesystem::path& path) {
    static const char* const images[] = {".jpg", ".jpeg", ".png", ".heic", ".heif", ".gif", ".webp", ".tif",
                                         ".tiff", ".bmp", ".dng"};
    static const char* const videos[] = {".mov", ".mp4", ".m4v", ".avi", ".mkv", ".webm", ".hevc"};
    const std::string ext = lower(path.extension().string());
    if (ext == ".url") return "link";
    if (path.parent_path().filename() == "text") return "text";
    for (const char* e : images) {
        if (ext == e) return "image";
    }
    for (const char* e : videos) {
        if (ext == e) return "video";
    }
    return "file";
}

Transfers::Transfer* Transfers::get(const std::string& id) {
    for (auto& t : list_) {
        if (t.id == id) return &t;
    }
    return nullptr;
}

const Transfers::Transfer* Transfers::find(const std::string& id) const {
    for (const auto& t : list_) {
        if (t.id == id) return &t;
    }
    return nullptr;
}

void Transfers::forget(const std::string& id) {
    list_.erase(std::remove_if(list_.begin(), list_.end(), [&](const Transfer& t) { return t.id == id; }),
                list_.end());
}

Transfers::Transfer& Transfers::add(const std::string& id, Direction direction, Clock::time_point now) {
    Transfer t;
    t.id = id;
    t.direction = direction;
    t.started = now;
    t.updated = now;
    list_.push_back(t);
    return list_.back();
}

std::string Transfers::apply(const std::vector<std::string>& e, Clock::time_point now) {
    if (e.size() < 2) return {};
    const std::string& what = e[0];
    const std::string& id = e[1];
    const auto field = [&](std::size_t i) { return i < e.size() ? e[i] : std::string(); };

    if (what == "ask" || what == "incoming") {
        Transfer* t = get(id);
        if (!t) t = &add(id, Direction::In, now);
        t->peer = field(2);
        t->count = static_cast<std::size_t>(number(field(3)));
        t->total = number(field(4));
        t->names.assign(e.begin() + std::min<std::size_t>(5, e.size()), e.end());
        if (t->count < t->names.size()) t->count = t->names.size();
        t->updated = now;
        if (what == "ask") {
            t->stage = Stage::Asking;
            t->deadline = now + kAskTimeout;
        } else if (t->stage == Stage::Asking || t->stage == Stage::Waiting) {
            t->stage = Stage::Waiting;  // accepted: the upload comes next
        }
        return id;
    }
    // outbox.py names the file, not an id.
    if (what == "sending") {
        sending(id, {}, false, now);
        return apply_stage(id, field(2), now);
    }
    if (what == "sent") return sent(id, field(2) == "sent", field(3), now);
    Transfer* t = get(id);
    if (!t) return {};
    t->updated = now;
    if (what == "asked") {
        const std::string outcome = field(2);
        if (outcome == "accepted") {
            t->stage = Stage::Waiting;
        } else {
            t->stage = Stage::Declined;
            t->detail = outcome == "timeout" ? "no answer in time" : "declined";
        }
    } else if (what == "receiving") {
        const std::uint64_t bytes = number(field(2));
        const std::uint64_t total = number(field(3));
        if (t->stage != Stage::Moving) t->started = now;  // the rate counts from the first byte
        const double elapsed = std::chrono::duration<double>(now - t->started).count();
        if (elapsed > 0.2) t->rate = static_cast<double>(bytes) / elapsed;
        t->bytes = bytes;
        if (total) t->total = total;
        t->stage = Stage::Moving;
    } else if (what == "stored") {
        t->stage = Stage::Saving;
        if (t->names.empty()) t->names.assign(e.begin() + 2, e.end());
        t->count = std::max(t->count, e.size() - 2);
        if (t->total) t->bytes = t->total;
    } else if (what == "failed") {
        t->stage = Stage::Failed;
        t->detail = field(2);
    } else {
        return {};
    }
    return id;
}

std::string Transfers::received(const std::filesystem::path& saved, const std::string& from, const std::string& kind,
                                bool home_assistant, Clock::time_point now) {
    const std::string name = saved.filename().u8string();
    const std::string k = kind.empty() ? kind_of(saved) : kind;
    // The newest transfer from the radio still waiting for files it named.
    Transfer* match = nullptr;
    if (!home_assistant) {
        for (auto it = list_.rbegin(); it != list_.rend() && !match; ++it) {
            Transfer& t = *it;
            if (t.direction != Direction::In || t.home_assistant || t.finished()) continue;
            if (t.stage == Stage::Asking) continue;
            if (t.saved.size() >= std::max<std::size_t>(t.count, 1)) continue;
            for (const auto& asked : t.names) {
                if (same_file(asked, name) || (k == "link" && is_link(asked)) ||
                    (k == "text" && name.rfind("text-", 0) == 0)) {
                    match = &t;
                    break;
                }
            }
        }
    }
    if (!match) {
        match = &add("in-" + std::to_string(++serial_), Direction::In, now);
        match->peer = from;
        match->names.push_back(name);
        match->count = 1;
        match->home_assistant = home_assistant;
    }
    match->saved.push_back(saved);
    if (match->kind.empty()) match->kind = k;
    if (match->peer.empty()) match->peer = from;
    match->updated = now;
    if (match->saved.size() >= std::max<std::size_t>(match->count, 1)) match->stage = Stage::Done;
    return match->id;
}

std::string Transfers::sending(const std::string& name, const std::string& to, bool home_assistant,
                               Clock::time_point now) {
    for (auto it = list_.rbegin(); it != list_.rend(); ++it) {
        if (it->direction == Direction::Out && !it->finished() && !it->names.empty() && it->names[0] == name) {
            if (!to.empty()) it->peer = to;
            it->updated = now;
            return it->id;
        }
    }
    Transfer& t = add("out-" + std::to_string(++serial_), Direction::Out, now);
    t.names.push_back(name);
    t.count = 1;
    t.peer = to;
    t.home_assistant = home_assistant;
    t.stage = Stage::Waiting;
    t.detail = home_assistant ? "handing it to Home Assistant" : "looking for the iPhone";
    return t.id;
}

std::string Transfers::apply_stage(const std::string& name, const std::string& detail, Clock::time_point now) {
    for (auto it = list_.rbegin(); it != list_.rend(); ++it) {
        Transfer& t = *it;
        if (t.direction != Direction::Out || t.finished() || t.names.empty() || t.names[0] != name) continue;
        t.updated = now;
        if (!detail.empty()) t.detail = detail;
        // "accepted; uploading" says both: the upload is the news.
        if (detail.find("upload") != std::string::npos) {
            t.stage = Stage::Moving;
        } else if (detail.find("accept") != std::string::npos) {
            t.stage = Stage::Accepting;
        } else {
            t.stage = Stage::Waiting;
        }
        return t.id;
    }
    return {};
}

std::string Transfers::sent(const std::string& name, bool ok, const std::string& detail, Clock::time_point now) {
    Transfer* match = nullptr;
    for (auto it = list_.rbegin(); it != list_.rend() && !match; ++it) {
        if (it->direction == Direction::Out && !it->finished() && !it->names.empty() && it->names[0] == name) {
            match = &*it;
        }
    }
    if (!match) {
        match = &add("out-" + std::to_string(++serial_), Direction::Out, now);
        match->names.push_back(name);
        match->count = 1;
    }
    match->updated = now;
    match->detail = detail;
    if (ok) {
        match->stage = Stage::Done;
    } else {
        match->stage = detail.find("declined") != std::string::npos ? Stage::Declined : Stage::Failed;
    }
    return match->id;
}

std::vector<std::string> Transfers::expire(Clock::time_point now) {
    std::vector<std::string> changed;
    for (auto& t : list_) {
        if (t.finished()) continue;
        if (t.stage == Stage::Asking) {
            if (now >= t.deadline) {
                t.stage = Stage::Declined;
                t.detail = "no answer in time";
                changed.push_back(t.id);
            }
            continue;
        }
        const auto quiet = now - t.updated;
        if (t.stage == Stage::Saving && quiet > std::chrono::seconds(15)) {
            t.stage = Stage::Done;  // stored; the inbox move went unreported
            changed.push_back(t.id);
        } else if (quiet > (t.direction == Direction::Out ? kOutStall : kStall)) {
            t.stage = Stage::Failed;
            t.detail = "the transfer stopped";
            changed.push_back(t.id);
        }
    }
    return changed;
}

}  // namespace howldrop
