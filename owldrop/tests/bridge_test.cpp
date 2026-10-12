#include "forward.hpp"
#include "node.hpp"

#include <chrono>
#include <fstream>
#include <iostream>
#include <iterator>
#include <thread>

#ifndef _WIN32
#include <unistd.h>
#endif

namespace {

int g_failed = 0;

void check(bool cond, int line, const char* text) {
    if (!cond) {
        std::cerr << "FAIL " << line << " " << text << '\n';
        ++g_failed;
    }
}

#define CHECK(cond) check(static_cast<bool>(cond), __LINE__, #cond)

std::filesystem::path temp_root() {
#ifdef _WIN32
    const int pid = 0;
#else
    const int pid = static_cast<int>(::getpid());
#endif
    return std::filesystem::temp_directory_path() / ("owldrop-bridge-" + std::to_string(pid));
}

void test_bridge() {
    const auto root = temp_root();
    std::filesystem::remove_all(root);
    const auto pc_inbox = root / "pc";
    const auto pi_inbox = root / "pi";
    std::filesystem::create_directories(pc_inbox);
    std::filesystem::create_directories(pi_inbox / "text");

    const auto pid = static_cast<unsigned>(::getpid());
    const auto discovery = static_cast<std::uint16_t>(25000 + (pid % 5000));
    const auto transfer = static_cast<std::uint16_t>(discovery + 1);
    const auto control = static_cast<std::uint16_t>(discovery + 2);
    const auto listen = static_cast<std::uint16_t>(discovery + 3);

    owldrop::MemoryClipboard clipboard;
    owldrop::Node::Config config;
    config.name = "Office-PC";
    config.id = std::string(32, 'a');
    config.discovery_port = discovery;
    config.transfer_port = transfer;
    config.control_port = control;
    config.inbox = pc_inbox;
    config.lan_broadcast = false;
    config.beacon_interval = std::chrono::milliseconds(50);
    config.peer_ttl = std::chrono::seconds(5);
    config.log = false;
    config.beacon_targets.push_back({"127.0.0.1", listen});

    owldrop::Node node(config, clipboard);
    node.start();

    std::string error;
    const auto learned = owldrop::discover_target(listen, std::chrono::seconds(2), "", error);
    if (!learned) std::cerr << "discover: " << error << '\n';
    CHECK(learned.has_value());
    if (learned) {
        CHECK(learned->name == "Office-PC");
        CHECK(learned->host == "127.0.0.1");
        CHECK(learned->port == transfer);
        CHECK(learned->id == config.id);
    }
    const owldrop::LanTarget target = learned ? *learned : owldrop::LanTarget{"127.0.0.1", transfer, "Office-PC", {}};

    const auto photo = pi_inbox / "photo.bin";
    {
        std::ofstream out(photo, std::ios::binary);
        for (int i = 0; i < 5000; ++i) out.put(static_cast<char>(i % 251));
    }
    const auto sent_file = owldrop::take_drop(pi_inbox, photo, target);
    if (sent_file.action != owldrop::DropAction::SentFile) std::cerr << "file: " << sent_file.message << '\n';
    CHECK(sent_file.action == owldrop::DropAction::SentFile);
    const auto saved = pc_inbox / "photo.bin";
    CHECK(std::filesystem::is_regular_file(saved));
    CHECK(std::filesystem::file_size(saved) == 5000);
    CHECK(std::filesystem::is_regular_file(pi_inbox / ".forwarded" / "photo.bin"));
    CHECK(!std::filesystem::exists(photo));
    {
        std::ifstream in(saved, std::ios::binary);
        bool match = true;
        for (int i = 0; i < 5000; ++i) {
            if (in.get() != (i % 251)) {
                match = false;
                break;
            }
        }
        CHECK(match);
    }

    const auto note = pi_inbox / "text" / "note.txt";
    {
        std::ofstream out(note, std::ios::binary);
        out << "hello from the phone";
    }
    const auto sent_text = owldrop::take_drop(pi_inbox, note, target);
    if (sent_text.action != owldrop::DropAction::SentText) std::cerr << "text: " << sent_text.message << '\n';
    CHECK(sent_text.action == owldrop::DropAction::SentText);
    const auto clip = clipboard.read();
    CHECK(clip.status == owldrop::ClipRead::Status::Text);
    CHECK(clip.text == "hello from the phone");
    CHECK(!std::filesystem::exists(note));

    const auto outside = root / "secret.txt";
    {
        std::ofstream out(outside);
        out << "secret";
    }
    CHECK(owldrop::classify_inbox_file(pi_inbox, outside) == owldrop::DropKind::Skip);
    const auto skipped = owldrop::take_drop(pi_inbox, outside, target);
    CHECK(skipped.action == owldrop::DropAction::Skip);
    CHECK(std::filesystem::exists(outside));
    CHECK(!std::filesystem::exists(pc_inbox / "secret.txt"));

    const auto blob = pi_inbox / "blob.bin";
    {
        std::ofstream out(blob, std::ios::binary);
        out << "hello";
    }
    const auto hostile = owldrop::forward_file(target, blob, "..\\..\\evil.txt");
    if (!hostile.ok) std::cerr << "hostile: " << hostile.message << '\n';
    CHECK(hostile.ok);
    CHECK(hostile.message == "evil.txt");
    const auto evil = pc_inbox / "evil.txt";
    CHECK(std::filesystem::is_regular_file(evil));
    {
        std::ifstream in(evil, std::ios::binary);
        std::string got((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
        CHECK(got == "hello");
    }
    CHECK(!std::filesystem::exists(root / "evil.txt"));
    CHECK(!std::filesystem::exists(pc_inbox.parent_path().parent_path() / "evil.txt"));

    node.stop();
    std::filesystem::remove_all(root);
}

}  // namespace

int main() {
    if (!owldrop::startup_network()) {
        std::cerr << "network startup failed\n";
        return 1;
    }
    try {
        test_bridge();
    } catch (const std::exception& ex) {
        std::cerr << "exception " << ex.what() << '\n';
        return 1;
    }
    owldrop::shutdown_network();
    if (g_failed) {
        std::cerr << g_failed << " checks failed\n";
        return 1;
    }
    std::cout << "owldrop-bridge-test passed\n";
    return 0;
}
