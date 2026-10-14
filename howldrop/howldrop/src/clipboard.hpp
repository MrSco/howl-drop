#pragma once

#include <cstdint>
#include <memory>
#include <mutex>
#include <string>

namespace howldrop {

struct ClipRead {
    enum class Status { Text, NoText, Busy };
    Status status = Status::NoText;
    std::string text;
};

class Clipboard {
public:
    virtual ~Clipboard() = default;
    virtual ClipRead read() = 0;
    virtual bool set_text(const std::string& utf8) = 0;
    virtual std::uint32_t sequence() = 0;
};

// In-memory clipboard used by tests, and by a non-Windows build of the same
// sources. The program you run on a PC uses the Windows clipboard.
class MemoryClipboard final : public Clipboard {
public:
    ClipRead read() override;
    bool set_text(const std::string& utf8) override;
    std::uint32_t sequence() override;

private:
    std::mutex mu_;
    std::string text_;
    std::uint32_t seq_ = 1;
};

std::unique_ptr<Clipboard> make_system_clipboard();

}  // namespace howldrop
