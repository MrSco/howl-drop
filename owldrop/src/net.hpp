#pragma once

#include <atomic>
#include <cstdint>
#include <string>
#include <vector>

namespace owldrop {

bool startup_network();
void shutdown_network();
std::string last_socket_error();
std::string ipv4_to_string(std::uint32_t addr_be);
bool ipv4_from_string(const std::string& text, std::uint32_t& addr_be);

// Subnet broadcast addresses for every IPv4 interface that is up and is not
// loopback. This is the IP stack (Ethernet included). It is not a Wi-Fi API.
std::vector<std::uint32_t> ipv4_broadcasts();

class Socket {
public:
    Socket() = default;
    ~Socket();
    Socket(Socket&& other) noexcept;
    Socket& operator=(Socket&& other) noexcept;
    Socket(const Socket&) = delete;
    Socket& operator=(const Socket&) = delete;

    static Socket udp_v4();
    static Socket tcp_v4();

    bool valid() const;
    void close();

    bool set_reuseaddr();
    bool set_broadcast();
    bool set_nodelay();
    bool set_nonblocking(bool on);
    bool set_recv_timeout_ms(int ms);
    bool set_send_timeout_ms(int ms);

    // addr_be is already in network byte order. port_host is the numeric port.
    bool bind_v4(std::uint32_t addr_be, std::uint16_t port_host);
    bool listen(int backlog);
    Socket accept_v4();
    bool connect_v4(std::uint32_t addr_be, std::uint16_t port_host, int timeout_ms, std::string& error);

    bool send_full(const std::uint8_t* data, std::size_t n, const std::atomic<bool>* stop, int max_idle = 30);
    bool recv_full(std::uint8_t* data, std::size_t n, const std::atomic<bool>* stop, int max_idle = 30);

    // >0 bytes, 0 closed, -2 timeout or would-block, -1 error.
    int recv_some(void* data, std::size_t n);
    // >0 bytes, -2 would-block, -1 error. from_port is host order.
    int recv_from(std::uint8_t* data, std::size_t n, std::uint32_t& from_be, std::uint16_t& from_port);
    bool send_to(const std::uint8_t* data, std::size_t n, std::uint32_t addr_be, std::uint16_t port_host);

    std::intptr_t raw() const { return fd_; }

private:
    explicit Socket(std::intptr_t fd) : fd_(fd) {}
    std::intptr_t fd_ = -1;
};

struct Ready {
    bool udp = false;
    bool tcp = false;
    bool control = false;
};

Ready wait_readable(Socket* udp, Socket* tcp, Socket* control, int timeout_ms);

}  // namespace owldrop
