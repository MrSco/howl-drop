#include "net.hpp"

#include <algorithm>
#include <cstring>
#include <limits>

#ifdef _WIN32
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#include <iphlpapi.h>
#include <windows.h>
#else
#include <arpa/inet.h>
#include <cerrno>
#include <fcntl.h>
#include <ifaddrs.h>
#include <net/if.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

namespace owldrop {
namespace {

#ifdef _WIN32
using Native = SOCKET;
constexpr Native kInvalid = INVALID_SOCKET;
int native_error() { return WSAGetLastError(); }
bool is_intr_code(int err) { return err == WSAEINTR; }
bool is_timeout_code(int err) { return err == WSAETIMEDOUT || err == WSAEWOULDBLOCK; }
bool is_progress_code(int err) { return err == WSAEWOULDBLOCK || err == WSAEINPROGRESS || err == WSAEINVAL; }
#else
using Native = int;
constexpr Native kInvalid = -1;
int native_error() { return errno; }
bool is_intr_code(int err) { return err == EINTR; }
bool is_timeout_code(int err) { return err == EAGAIN || err == EWOULDBLOCK; }
bool is_progress_code(int err) { return err == EINPROGRESS || err == EAGAIN || err == EWOULDBLOCK; }
#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif
#endif

Native as_native(std::intptr_t fd) { return static_cast<Native>(fd); }

int last_code() { return native_error(); }

}  // namespace

bool startup_network() {
#ifdef _WIN32
    WSADATA data{};
    return WSAStartup(MAKEWORD(2, 2), &data) == 0;
#else
    return true;
#endif
}

void shutdown_network() {
#ifdef _WIN32
    WSACleanup();
#endif
}

std::string last_socket_error() {
#ifdef _WIN32
    const int err = WSAGetLastError();
    char* message = nullptr;
    const DWORD flags = FORMAT_MESSAGE_ALLOCATE_BUFFER | FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    const DWORD n = FormatMessageA(flags, nullptr, static_cast<DWORD>(err), 0, reinterpret_cast<LPSTR>(&message), 0, nullptr);
    std::string text = (n && message) ? std::string(message, n) : ("Windows error " + std::to_string(err));
    if (message) LocalFree(message);
    while (!text.empty() && (text.back() == '\n' || text.back() == '\r' || text.back() == ' ')) text.pop_back();
    return text;
#else
    return std::strerror(errno);
#endif
}

std::string ipv4_to_string(std::uint32_t addr_be) {
    char buf[INET_ADDRSTRLEN] = {};
    in_addr addr{};
    addr.s_addr = addr_be;
    if (!inet_ntop(AF_INET, &addr, buf, sizeof(buf))) return "0.0.0.0";
    return buf;
}

bool ipv4_from_string(const std::string& text, std::uint32_t& addr_be) {
    in_addr addr{};
    if (inet_pton(AF_INET, text.c_str(), &addr) != 1) return false;
    addr_be = addr.s_addr;
    return true;
}

std::vector<std::uint32_t> ipv4_broadcasts() {
    std::vector<std::uint32_t> out;
#ifdef _WIN32
    ULONG size = 15000;
    std::vector<unsigned char> buf(size);
    ULONG rc = 0;
    for (int attempt = 0; attempt < 3; ++attempt) {
        auto* addrs = reinterpret_cast<IP_ADAPTER_ADDRESSES*>(buf.data());
        rc = GetAdaptersAddresses(AF_INET,
                                  GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                                  nullptr, addrs, &size);
        if (rc == ERROR_BUFFER_OVERFLOW) {
            buf.resize(size);
            continue;
        }
        break;
    }
    if (rc != NO_ERROR) return out;
    for (auto* adapter = reinterpret_cast<IP_ADAPTER_ADDRESSES*>(buf.data()); adapter; adapter = adapter->Next) {
        if (adapter->OperStatus != IfOperStatusUp) continue;
        if (adapter->IfType == IF_TYPE_SOFTWARE_LOOPBACK) continue;
        for (auto* unicast = adapter->FirstUnicastAddress; unicast; unicast = unicast->Next) {
            if (!unicast->Address.lpSockaddr || unicast->Address.lpSockaddr->sa_family != AF_INET) continue;
            const auto prefix = unicast->OnLinkPrefixLength;
            if (prefix == 0 || prefix >= 32) continue;
            const auto* sin = reinterpret_cast<sockaddr_in*>(unicast->Address.lpSockaddr);
            const std::uint32_t ip = ntohl(sin->sin_addr.s_addr);
            if (ip == 0 || (ip >> 24) == 127) continue;
            const std::uint32_t mask = 0xffffffffu << (32 - prefix);
            const std::uint32_t bcast = (ip & mask) | ~mask;
            out.push_back(htonl(bcast));
        }
    }
#else
    ifaddrs* list = nullptr;
    if (getifaddrs(&list) != 0) return out;
    for (auto* ifa = list; ifa; ifa = ifa->ifa_next) {
        if (!ifa->ifa_addr || ifa->ifa_addr->sa_family != AF_INET) continue;
        if ((ifa->ifa_flags & IFF_UP) == 0 || (ifa->ifa_flags & IFF_LOOPBACK) != 0) continue;
        std::uint32_t bcast = 0;
        if ((ifa->ifa_flags & IFF_BROADCAST) != 0 && ifa->ifa_broadaddr &&
            ifa->ifa_broadaddr->sa_family == AF_INET) {
            bcast = reinterpret_cast<sockaddr_in*>(ifa->ifa_broadaddr)->sin_addr.s_addr;
        } else if (ifa->ifa_netmask && ifa->ifa_netmask->sa_family == AF_INET) {
            const auto ip = ntohl(reinterpret_cast<sockaddr_in*>(ifa->ifa_addr)->sin_addr.s_addr);
            const auto mask = ntohl(reinterpret_cast<sockaddr_in*>(ifa->ifa_netmask)->sin_addr.s_addr);
            if (mask == 0 || mask == 0xffffffffu || (ip >> 24) == 127) continue;
            bcast = htonl((ip & mask) | ~mask);
        }
        if (bcast != 0) out.push_back(bcast);
    }
    freeifaddrs(list);
#endif
    std::sort(out.begin(), out.end());
    out.erase(std::unique(out.begin(), out.end()), out.end());
    return out;
}

Socket::~Socket() { close(); }

Socket::Socket(Socket&& other) noexcept : fd_(other.fd_) { other.fd_ = -1; }

Socket& Socket::operator=(Socket&& other) noexcept {
    if (this != &other) {
        close();
        fd_ = other.fd_;
        other.fd_ = -1;
    }
    return *this;
}

Socket Socket::udp_v4() {
    const Native fd = ::socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP);
    if (fd == kInvalid) return {};
    return Socket(static_cast<std::intptr_t>(fd));
}

Socket Socket::tcp_v4() {
    const Native fd = ::socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (fd == kInvalid) return {};
    return Socket(static_cast<std::intptr_t>(fd));
}

bool Socket::valid() const { return fd_ >= 0; }

void Socket::close() {
    if (fd_ < 0) return;
#ifdef _WIN32
    ::closesocket(as_native(fd_));
#else
    ::close(static_cast<int>(fd_));
#endif
    fd_ = -1;
}

bool Socket::set_reuseaddr() {
    int yes = 1;
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_REUSEADDR, reinterpret_cast<char*>(&yes), sizeof(yes)) == 0;
}

bool Socket::set_broadcast() {
    int yes = 1;
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_BROADCAST, reinterpret_cast<char*>(&yes), sizeof(yes)) == 0;
}

bool Socket::set_nodelay() {
    int yes = 1;
    return ::setsockopt(as_native(fd_), IPPROTO_TCP, TCP_NODELAY, reinterpret_cast<char*>(&yes), sizeof(yes)) == 0;
}

bool Socket::set_nonblocking(bool on) {
#ifdef _WIN32
    u_long mode = on ? 1 : 0;
    return ::ioctlsocket(as_native(fd_), FIONBIO, &mode) == 0;
#else
    int flags = ::fcntl(static_cast<int>(fd_), F_GETFL, 0);
    if (flags < 0) return false;
    if (on) flags |= O_NONBLOCK;
    else flags &= ~O_NONBLOCK;
    return ::fcntl(static_cast<int>(fd_), F_SETFL, flags) == 0;
#endif
}

bool Socket::set_recv_timeout_ms(int ms) {
#ifdef _WIN32
    DWORD timeout = static_cast<DWORD>(ms);
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_RCVTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout)) == 0;
#else
    timeval tv{};
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)) == 0;
#endif
}

bool Socket::set_send_timeout_ms(int ms) {
#ifdef _WIN32
    DWORD timeout = static_cast<DWORD>(ms);
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_SNDTIMEO, reinterpret_cast<char*>(&timeout), sizeof(timeout)) == 0;
#else
    timeval tv{};
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    return ::setsockopt(as_native(fd_), SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) == 0;
#endif
}

bool Socket::bind_v4(std::uint32_t addr_be, std::uint16_t port_host) {
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port_host);
    addr.sin_addr.s_addr = addr_be;
    return ::bind(as_native(fd_), reinterpret_cast<sockaddr*>(&addr), sizeof(addr)) == 0;
}

bool Socket::listen(int backlog) { return ::listen(as_native(fd_), backlog) == 0; }

Socket Socket::accept_v4() {
    sockaddr_in from{};
#ifdef _WIN32
    int len = sizeof(from);
#else
    socklen_t len = sizeof(from);
#endif
    const Native fd = ::accept(as_native(fd_), reinterpret_cast<sockaddr*>(&from), &len);
    if (fd == kInvalid) return {};
    return Socket(static_cast<std::intptr_t>(fd));
}

bool Socket::connect_v4(std::uint32_t addr_be, std::uint16_t port_host, int timeout_ms, std::string& error) {
    if (!set_nonblocking(true)) {
        error = last_socket_error();
        return false;
    }
    sockaddr_in addr{};
    addr.sin_family = AF_INET;
    addr.sin_port = htons(port_host);
    addr.sin_addr.s_addr = addr_be;
    const int rc = ::connect(as_native(fd_), reinterpret_cast<sockaddr*>(&addr), sizeof(addr));
    if (rc != 0) {
        const int err = last_code();
        if (!is_progress_code(err) && err != 0) {
            error = last_socket_error();
            return false;
        }
#ifdef _WIN32
        WSAPOLLFD pfd{};
        pfd.fd = as_native(fd_);
        pfd.events = POLLOUT;
        const int waited = WSAPoll(&pfd, 1, timeout_ms);
#else
        pollfd pfd{};
        pfd.fd = static_cast<int>(fd_);
        pfd.events = POLLOUT;
        const int waited = ::poll(&pfd, 1, timeout_ms);
#endif
        if (waited == 0) {
            error = "timed out";
            return false;
        }
        if (waited < 0) {
            error = last_socket_error();
            return false;
        }
        int soerr = 0;
#ifdef _WIN32
        int slen = sizeof(soerr);
#else
        socklen_t slen = sizeof(soerr);
#endif
        if (::getsockopt(as_native(fd_), SOL_SOCKET, SO_ERROR, reinterpret_cast<char*>(&soerr), &slen) != 0 || soerr != 0) {
#ifdef _WIN32
            if (soerr != 0) WSASetLastError(soerr);
#endif
#ifndef _WIN32
            if (soerr != 0) errno = soerr;
#endif
            error = soerr ? last_socket_error() : "connect failed";
            return false;
        }
    }
    if (!set_nonblocking(false)) {
        error = last_socket_error();
        return false;
    }
    return true;
}

bool Socket::send_full(const std::uint8_t* data, std::size_t n, const std::atomic<bool>* stop, int max_idle) {
    std::size_t sent = 0;
    int idle = 0;
    while (sent < n) {
        if (stop && stop->load()) return false;
        const std::size_t chunk_size = std::min<std::size_t>(n - sent, static_cast<std::size_t>(1 << 20));
#ifdef _WIN32
        const int flags = 0;
#else
        const int flags = MSG_NOSIGNAL;
#endif
        const int wrote = ::send(as_native(fd_), reinterpret_cast<const char*>(data + sent), static_cast<int>(chunk_size), flags);
        if (wrote > 0) {
            sent += static_cast<std::size_t>(wrote);
            idle = 0;
            continue;
        }
        const int err = last_code();
        if (is_intr_code(err)) continue;
        if (is_timeout_code(err)) {
            if (++idle >= max_idle) return false;
            continue;
        }
        return false;
    }
    return true;
}

bool Socket::recv_full(std::uint8_t* data, std::size_t n, const std::atomic<bool>* stop, int max_idle) {
    std::size_t got = 0;
    int idle = 0;
    while (got < n) {
        if (stop && stop->load()) return false;
        const std::size_t chunk_size = std::min<std::size_t>(n - got, static_cast<std::size_t>(1 << 20));
        const int read = ::recv(as_native(fd_), reinterpret_cast<char*>(data + got), static_cast<int>(chunk_size), 0);
        if (read > 0) {
            got += static_cast<std::size_t>(read);
            idle = 0;
            continue;
        }
        if (read == 0) return false;
        const int err = last_code();
        if (is_intr_code(err)) continue;
        if (is_timeout_code(err)) {
            if (++idle >= max_idle) return false;
            continue;
        }
        return false;
    }
    return true;
}

int Socket::recv_some(void* data, std::size_t n) {
    if (n == 0) return 0;
    if (n > static_cast<std::size_t>(std::numeric_limits<int>::max())) n = static_cast<std::size_t>(1 << 20);
    for (;;) {
        const int read = ::recv(as_native(fd_), reinterpret_cast<char*>(data), static_cast<int>(n), 0);
        if (read >= 0) return read;
        const int err = last_code();
        if (is_intr_code(err)) continue;
        if (is_timeout_code(err)) return -2;
        return -1;
    }
}

int Socket::recv_from(std::uint8_t* data, std::size_t n, std::uint32_t& from_be, std::uint16_t& from_port) {
    sockaddr_in from{};
#ifdef _WIN32
    int len = sizeof(from);
#else
    socklen_t len = sizeof(from);
#endif
    if (n > static_cast<std::size_t>(std::numeric_limits<int>::max())) n = static_cast<std::size_t>(1 << 20);
    const int read = ::recvfrom(as_native(fd_), reinterpret_cast<char*>(data), static_cast<int>(n), 0,
                                reinterpret_cast<sockaddr*>(&from), &len);
    if (read >= 0) {
        from_be = from.sin_addr.s_addr;
        from_port = ntohs(from.sin_port);
        return read;
    }
    const int err = last_code();
    if (is_timeout_code(err) || is_intr_code(err)) return -2;
    return -1;
}

bool Socket::send_to(const std::uint8_t* data, std::size_t n, std::uint32_t addr_be, std::uint16_t port_host) {
    sockaddr_in to{};
    to.sin_family = AF_INET;
    to.sin_port = htons(port_host);
    to.sin_addr.s_addr = addr_be;
    if (n > static_cast<std::size_t>(std::numeric_limits<int>::max())) return false;
    const int wrote = ::sendto(as_native(fd_), reinterpret_cast<const char*>(data), static_cast<int>(n), 0,
                               reinterpret_cast<sockaddr*>(&to), sizeof(to));
    return wrote == static_cast<int>(n);
}

Ready wait_readable(Socket* udp, Socket* tcp, Socket* control, int timeout_ms) {
    Ready ready;
#ifdef _WIN32
    WSAPOLLFD pfds[3]{};
#else
    pollfd pfds[3]{};
#endif
    int count = 0;
    int udp_i = -1, tcp_i = -1, control_i = -1;
    auto add = [&](Socket* socket, int& index) {
        if (!socket || !socket->valid()) return;
        index = count;
#ifdef _WIN32
        pfds[count].fd = as_native(socket->raw());
#else
        pfds[count].fd = static_cast<int>(socket->raw());
#endif
        pfds[count].events = POLLIN;
        ++count;
    };
    add(udp, udp_i);
    add(tcp, tcp_i);
    add(control, control_i);
    if (count == 0) return ready;
#ifdef _WIN32
    const int rc = WSAPoll(pfds, static_cast<ULONG>(count), timeout_ms);
#else
    const int rc = ::poll(pfds, static_cast<nfds_t>(count), timeout_ms);
#endif
    if (rc <= 0) return ready;
    auto hit = [&](int index) {
        return index >= 0 && (pfds[index].revents & (POLLIN | POLLERR | POLLHUP)) != 0;
    };
    ready.udp = hit(udp_i);
    ready.tcp = hit(tcp_i);
    ready.control = hit(control_i);
    return ready;
}

}  // namespace owldrop
