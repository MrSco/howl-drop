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
    return std::filesystem::temp_directory_path() / ("howldrop-test-" + std::to_string(pid));
}

void test_protocol() {
    howldrop::Message message;
    message.type = howldrop::MsgType::Hello;
    message.fields.set("name", "Office");
    message.fields.set("id", std::string(32, 'a'));
    message.fields.set("port", "47632");
    std::string error;
    auto bytes = howldrop::encode_head(message, error);
    CHECK(bytes.has_value());
    auto parsed = howldrop::parse_datagram(bytes->data(), bytes->size(), error);
    CHECK(parsed.has_value());
    CHECK(parsed->fields.get("name") == "Office");
    CHECK(parsed->fields.get("port") == "47632");
    CHECK(parsed->body_size == 0);

    message.type = howldrop::MsgType::File;
    message.body_size = 100000;
    message.fields.set("file", "blob.bin");
    bytes = howldrop::encode_head(message, error);
    CHECK(bytes.has_value());
    howldrop::MsgType type{};
    std::uint16_t header_len = 0;
    std::uint64_t body_size = 0;
    CHECK(howldrop::parse_prefix(bytes->data(), type, header_len, body_size, error));
    CHECK(type == howldrop::MsgType::File);
    CHECK(body_size == 100000);
}

void test_sanitize() {
    CHECK(howldrop::sanitize_filename("..\\..\\evil.txt") == "evil.txt");
    CHECK(howldrop::sanitize_filename("a/b.txt") == "b.txt");
    CHECK(howldrop::sanitize_filename("CON.txt") == "_CON.txt");
    CHECK(howldrop::sanitize_filename("   ") == "file");
}

bool wait_for_peer(howldrop::Node& node, const std::string& name) {
    for (int i = 0; i < 100; ++i) {
        for (const auto& peer : node.peers()) {
            if (peer.name == name) return true;
        }
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    return false;
}

bool wait_for_text(howldrop::MemoryClipboard& clipboard, const std::string& text) {
    for (int i = 0; i < 150; ++i) {
        const auto read = clipboard.read();
        if (read.status == howldrop::ClipRead::Status::Text && read.text == text) return true;
        std::this_thread::sleep_for(std::chrono::milliseconds(20));
    }
    return false;
}

void test_lan() {
    const auto root = temp_root();
    std::filesystem::remove_all(root);
    std::filesystem::create_directories(root / "a");
    std::filesystem::create_directories(root / "b");

    howldrop::MemoryClipboard clip_a;
    howldrop::MemoryClipboard clip_b;
    howldrop::Node::Config a_cfg;
    a_cfg.name = "pc-a";
    a_cfg.id = std::string(32, 'a');
    a_cfg.discovery_port = 41231;
    a_cfg.transfer_port = 41232;
    a_cfg.control_port = 41233;
    a_cfg.inbox = root / "a";
    a_cfg.lan_broadcast = false;
    a_cfg.beacon_interval = std::chrono::milliseconds(50);
    a_cfg.peer_ttl = std::chrono::seconds(2);
    a_cfg.log = false;
    a_cfg.beacon_targets.push_back({"127.0.0.1", 41241});
    a_cfg.pair_key = howldrop::new_pair_key();  // both PCs share it (howldrop pair)
    a_cfg.share_clipboard = true;

    howldrop::Node::Config b_cfg = a_cfg;
    b_cfg.name = "pc-b";
    b_cfg.id = std::string(32, 'b');
    b_cfg.discovery_port = 41241;
    b_cfg.transfer_port = 41242;
    b_cfg.control_port = 41243;
    b_cfg.inbox = root / "b";
    b_cfg.beacon_targets = {{"127.0.0.1", 41231}};

    howldrop::Node node_a(a_cfg, clip_a);
    howldrop::Node node_b(b_cfg, clip_b);
    node_a.start();
    node_b.start();
    CHECK(wait_for_peer(node_a, "pc-b"));
    CHECK(wait_for_peer(node_b, "pc-a"));

    const auto source = root / "blob.bin";
    {
        std::ofstream out(source, std::ios::binary);
        for (int i = 0; i < 100000; ++i) out.put(static_cast<char>(i % 251));
    }
    const auto missing = node_a.send_file("nobody", source);
    CHECK(!missing.ok);
    const auto sent = node_a.send_file("pc-b", source);
    if (!sent.ok) std::cerr << "send: " << sent.message << '\n';
    CHECK(sent.ok);
    const auto saved = root / "b" / "blob.bin";
    CHECK(std::filesystem::is_regular_file(saved));
    CHECK(std::filesystem::file_size(saved) == 100000);
    {
        std::ifstream in(saved, std::ios::binary);
        bool match = true;
        for (int i = 0; i < 100000; ++i) {
            const int byte = in.get();
            if (byte != (i % 251)) {
                match = false;
                break;
            }
        }
        CHECK(match);
    }

    // A hand-made transfer to pc-b: key "" sends it unsigned, tamper sends a
    // body other than the one the trailer covers. Returns the ack's result.
    const auto raw_send = [&](const std::string& name, const std::string& key, bool tamper) {
        howldrop::Socket socket = howldrop::Socket::tcp_v4();
        std::uint32_t addr = 0;
        std::string error;
        CHECK(howldrop::ipv4_from_string("127.0.0.1", addr));
        CHECK(socket.connect_v4(addr, 41242, 2000, error));
        howldrop::Message message;
        message.type = howldrop::MsgType::File;
        const std::string body = "hello";
        message.body_size = body.size();
        message.fields.set("file", name);
        message.fields.set("from", std::string(32, 'a'));
        std::optional<howldrop::HmacSha256> mac;
        const auto opened = howldrop::open_transfer(socket, message, key, mac, nullptr, error);
        if (opened != howldrop::OpenResult::Ok) return std::string("refused before the body");
        const std::string sent = tamper ? "jello" : body;
        if (mac) mac->update(body.data(), body.size());
        socket.send_full(reinterpret_cast<const std::uint8_t*>(sent.data()), sent.size(), nullptr);
        howldrop::close_transfer(socket, mac, nullptr);
        howldrop::Message ack;
        if (!howldrop::read_head(socket, ack, nullptr, error)) return std::string("no answer");
        return ack.fields.get("result") + " " + ack.fields.get("file");
    };

    // Names are made safe: no way out of the inbox.
    CHECK(raw_send("..\\..\\evil.txt", a_cfg.pair_key, false) == "ok evil.txt");
    {
        const auto evil = root / "b" / "evil.txt";
        CHECK(std::filesystem::is_regular_file(evil));
        std::ifstream in(evil, std::ios::binary);
        std::string got((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
        CHECK(got == "hello");
        CHECK(!std::filesystem::exists(root / "evil.txt"));
    }
    // Pairing: an unpaired sender is refused before its body, a wrong key
    // too, and a body that is not the signed one is dropped.
    {
        // The refusal may beat the body, or the reset may beat the refusal.
        const std::string unsigned_result = raw_send("unsigned.txt", "", false);
        CHECK(unsigned_result == "error " || unsigned_result == "no answer");
        CHECK(!std::filesystem::exists(root / "b" / "unsigned.txt"));
    }
    CHECK(raw_send("wrong-key.txt", howldrop::new_pair_key(), false) == "refused before the body");
    CHECK(!std::filesystem::exists(root / "b" / "wrong-key.txt"));
    CHECK(raw_send("tampered.txt", a_cfg.pair_key, true) == "error ");
    CHECK(!std::filesystem::exists(root / "b" / "tampered.txt"));
    CHECK(!std::filesystem::exists(root / "b" / "tampered.txt.partial"));

    CHECK(clip_a.set_text("hello from A"));
    CHECK(wait_for_text(clip_b, "hello from A"));

    // Sharing off: what is copied on A stays on A.
    node_a.set_share_clipboard(false);
    CHECK(clip_a.set_text("a password on A"));
    std::this_thread::sleep_for(std::chrono::milliseconds(1500));
    CHECK(clip_b.read().text == "hello from A");

    node_a.stop();
    node_b.stop();
    std::filesystem::remove_all(root);
}

}  // namespace

int main() {
    if (!howldrop::startup_network()) {
        std::cerr << "network startup failed\n";
        return 1;
    }
    try {
        test_protocol();
        test_sanitize();
        test_lan();
    } catch (const std::exception& ex) {
        std::cerr << "exception " << ex.what() << '\n';
        return 1;
    }
    howldrop::shutdown_network();
    if (g_failed) {
        std::cerr << g_failed << " checks failed\n";
        return 1;
    }
    std::cout << "howldrop-test passed\n";
    return 0;
}
