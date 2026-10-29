//! owly-awdl on Linux: its [`driver`] on a
//! monitor interface the injection backend set up.
//!
//! Its [`Air`] is a packet socket on the monitor, which captures and
//! injects radiotap frames. Its [`Netif`] is `awdl0`, which it makes itself:
//! a TAP device carrying Ethernet frames, created down, given the AWDL
//! address as its MAC (AWDL-2) and its EUI-64 link-local address without
//! DAD (NET-1), MTU 1450 with offloads off, then brought up. Each peer that
//! turns valid gets a permanent neighbour entry (NET-2). The monitor asks
//! for MTU 8192 so A-MSDUs fit.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use crate::radio::{MacAddr, PeerInfo};
use owly_awdl::driver::{self, Air, Netif, Received, Running};
use owly_awdl::frame::Frame;
use owly_awdl::{Config, Ipv6Packet};
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;

use crate::radiotap;
use crate::rtnl::Rtnl;

/// `awdl0`'s MTU.
pub const TAP_MTU: u32 = 1450;
/// What the monitor interface asks for, so A-MSDUs fit.
pub const MONITOR_MTU: u32 = 8192;
/// The most a radiotap header of ours takes.
const RADIOTAP_MAX: usize = 16;

/// A running node.
pub struct Native {
    running: Running,
    /// `awdl0`'s index.
    pub index: u32,
}

impl Native {
    /// Makes `tap` and starts a node with `cfg` on `monitor`, publishing
    /// its valid peers on `peers`.
    pub fn start(
        monitor: &str,
        tap: &str,
        cfg: Config,
        peers: watch::Sender<Vec<PeerInfo>>,
    ) -> io::Result<Native> {
        let monitor_index = if_index(monitor)?;
        let mut rtnl = Rtnl::open()?;
        if let Err(e) = rtnl.set_mtu(monitor_index, MONITOR_MTU) {
            tracing::info!("{monitor} keeps its MTU, not {MONITOR_MTU}: {e}");
        }
        let monitor_mtu = std::fs::read_to_string(format!("/sys/class/net/{monitor}/mtu"))
            .ok()
            .and_then(|m| m.trim().parse::<usize>().ok())
            .unwrap_or(2304);
        let mut cfg = cfg;
        cfg.max_frame = monitor_mtu.saturating_sub(RADIOTAP_MAX).max(1500);

        let air = packet_socket(monitor_index, false)?;
        let tap_fd = open_tap(tap)?;
        let index = if_index(tap)?;
        // No address of the kernel's own beside ours.
        let _ = std::fs::write(format!("/proc/sys/net/ipv6/conf/{tap}/addr_gen_mode"), "1");
        let _ = std::fs::write(format!("/proc/sys/net/ipv6/conf/{tap}/accept_dad"), "0");
        let mac = cfg.addr;
        rtnl.set_mac(index, mac)?;
        rtnl.set_mtu(index, TAP_MTU)?;
        rtnl.add_link_local_nodad(index, mac.link_local())?;
        rtnl.set_up(index, true)?;

        let name = tap.to_string();
        let running = driver::spawn(
            cfg,
            move || {
                let monitor = Monitor {
                    fd: register(air)?,
                    buf: vec![0; 16 * 1024],
                };
                let tap = Tap {
                    fd: register(tap_fd)?,
                    name,
                    index,
                    rtnl,
                    buf: vec![0; 16 * 1024],
                };
                Ok((monitor, tap))
            },
            peers,
        )?;
        Ok(Native { running, index })
    }

    /// Why the node stopped by itself, once it has.
    pub fn failed(&self) -> watch::Receiver<Option<String>> {
        self.running.failed()
    }

    /// Stops the node. `awdl0` goes with its file descriptor, and its
    /// neighbour entries with it.
    pub fn stop(self) {
        self.running.stop();
    }
}

/// The monitor interface's packet socket.
struct Monitor {
    fd: AsyncFd<OwnedFd>,
    buf: Vec<u8>,
}

impl Air for Monitor {
    async fn recv(&mut self) -> io::Result<Received> {
        loop {
            let mut guard = self.fd.readable().await?;
            let read = guard.try_io(|fd| Ok(nix::unistd::read(fd.get_ref(), &mut self.buf)?));
            match read {
                Ok(Ok(n)) => {
                    if let Some(rx) = radiotap::parse(&self.buf[..n]) {
                        return Ok(Received {
                            bytes: rx.frame.to_vec(),
                            rssi: rx.rssi,
                        });
                    }
                }
                Ok(Err(e)) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("the monitor interface failed: {e}"),
                    ));
                }
                Err(_would_block) => {}
            }
        }
    }

    fn send(&mut self, frame: Frame) -> Result<(), Frame> {
        let mut buf = radiotap::tx_header(frame.rate, frame.no_ack);
        buf.extend_from_slice(&frame.bytes);
        match nix::unistd::write(self.fd.get_ref(), &buf) {
            Ok(_) => Ok(()),
            Err(nix::errno::Errno::EAGAIN | nix::errno::Errno::ENOBUFS) => Err(frame),
            Err(e) => {
                tracing::debug!("couldn't inject a frame: {e}");
                Ok(())
            }
        }
    }
}

/// `awdl0`.
struct Tap {
    fd: AsyncFd<OwnedFd>,
    name: String,
    index: u32,
    rtnl: Rtnl,
    buf: Vec<u8>,
}

impl Netif for Tap {
    async fn recv(&mut self) -> io::Result<Ipv6Packet> {
        loop {
            let mut guard = self.fd.readable().await?;
            let read = guard.try_io(|fd| Ok(nix::unistd::read(fd.get_ref(), &mut self.buf)?));
            match read {
                Ok(Ok(n)) => {
                    if let Some(packet) = Ipv6Packet::from_ethernet(&self.buf[..n]) {
                        return Ok(packet);
                    }
                }
                Ok(Err(e)) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("{} failed: {e}", self.name),
                    ));
                }
                Err(_would_block) => {}
            }
        }
    }

    fn deliver(&mut self, packet: Ipv6Packet) {
        let frame = packet.to_ethernet();
        if let Err(e) = nix::unistd::write(self.fd.get_ref(), &frame)
            && e != nix::errno::Errno::EAGAIN
        {
            tracing::debug!("{} refused a packet: {e}", self.name);
        }
    }

    fn peer_up(&mut self, mac: MacAddr) {
        if let Err(e) = self.rtnl.add_neighbour(self.index, mac.link_local(), mac) {
            tracing::warn!("no neighbour entry for {mac}: {e}");
        }
    }

    fn peer_down(&mut self, mac: MacAddr) {
        let _ = self.rtnl.del_neighbour(self.index, mac.link_local());
    }
}

/// Registers a descriptor with the current runtime.
fn register(fd: OwnedFd) -> io::Result<AsyncFd<OwnedFd>> {
    #[allow(unsafe_code)]
    // SAFETY: an OwnedFd is open and stays the same descriptor until the
    // AsyncFd, which owns it, is dropped.
    unsafe { AsyncFd::register(fd) }.map_err(|e| io::Error::other(e.to_string()))
}

fn if_index(name: &str) -> io::Result<u32> {
    nix::net::if_::if_nametoindex(name).map_err(|e| io::Error::other(format!("{name}: {e}")))
}

/// A raw packet socket on interface `index`, non-blocking: what a monitor
/// interface captures, radiotap first, and injects. Frames sent through
/// the interface, by anyone, are read back only when `outgoing`.
pub fn packet_socket(index: u32, outgoing: bool) -> io::Result<OwnedFd> {
    use nix::libc;
    let proto = (libc::ETH_P_ALL as u16).to_be();
    #[allow(unsafe_code)]
    // SAFETY: socket(2) takes no pointers; the descriptor it returns is
    // checked and then owned.
    let fd = unsafe {
        let fd = libc::socket(
            libc::AF_PACKET,
            libc::SOCK_RAW | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            proto as i32,
        );
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        OwnedFd::from_raw_fd(fd)
    };
    let mut addr: libc::sockaddr_ll = zeroed_sockaddr_ll();
    addr.sll_family = libc::AF_PACKET as u16;
    addr.sll_protocol = proto;
    addr.sll_ifindex = index as i32;
    #[allow(unsafe_code)]
    // SAFETY: `addr` is a valid sockaddr_ll for the length given, and lives
    // through the call.
    let r = unsafe {
        libc::bind(
            fd.as_raw_fd(),
            (&addr as *const libc::sockaddr_ll).cast(),
            size_of::<libc::sockaddr_ll>() as u32,
        )
    };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    // Our own frames aren't wanted back (Linux 4.20 and later), and a
    // burst of an iPhone's shouldn't overflow the buffer.
    let ignore: i32 = (!outgoing).into();
    let size: i32 = 4 << 20;
    for (level, name, value) in [
        (libc::SOL_PACKET, libc::PACKET_IGNORE_OUTGOING, &ignore),
        (libc::SOL_SOCKET, libc::SO_RCVBUF, &size),
        (libc::SOL_SOCKET, libc::SO_SNDBUF, &size),
    ] {
        #[allow(unsafe_code)]
        // SAFETY: `value` points to an i32 that lives through the call.
        unsafe {
            libc::setsockopt(
                fd.as_raw_fd(),
                level,
                name,
                (value as *const i32).cast(),
                size_of::<i32>() as u32,
            );
        }
    }
    Ok(fd)
}

fn zeroed_sockaddr_ll() -> nix::libc::sockaddr_ll {
    nix::libc::sockaddr_ll {
        sll_family: 0,
        sll_protocol: 0,
        sll_ifindex: 0,
        sll_hatype: 0,
        sll_pkttype: 0,
        sll_halen: 0,
        sll_addr: [0; 8],
    }
}

/// A TAP device named `name`, carrying Ethernet frames with no packet
/// information header and no offloads, left down. It lasts as long as the
/// descriptor.
fn open_tap(name: &str) -> io::Result<OwnedFd> {
    use nix::fcntl::{OFlag, open};
    use nix::libc;
    use nix::sys::stat::Mode;
    const TUNSETIFF: libc::c_ulong = 0x4004_54ca;
    const TUNSETOFFLOAD: libc::c_ulong = 0x4004_54d0;
    const IFF_TAP: libc::c_short = 0x0002;
    const IFF_NO_PI: libc::c_short = 0x1000;

    let fd = open(
        "/dev/net/tun",
        OFlag::O_RDWR | OFlag::O_NONBLOCK | OFlag::O_CLOEXEC,
        Mode::empty(),
    )?;
    let cname = CString::new(name).map_err(|_| io::Error::other("a NUL in the name"))?;
    let bytes = cname.as_bytes_with_nul();
    if bytes.len() > libc::IFNAMSIZ {
        return Err(io::Error::other(format!(
            "{name}: too long for an interface"
        )));
    }
    // struct ifreq: the name, then the flags.
    let mut ifreq = [0u8; 40];
    ifreq[..bytes.len()].copy_from_slice(bytes);
    ifreq[16..18].copy_from_slice(&(IFF_TAP | IFF_NO_PI).to_ne_bytes());
    #[allow(unsafe_code)]
    // SAFETY: TUNSETIFF reads and writes a struct ifreq, which `ifreq` is
    // big enough for and outlives the call.
    let r = unsafe { libc::ioctl(fd.as_raw_fd(), TUNSETIFF as _, ifreq.as_mut_ptr()) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    #[allow(unsafe_code)]
    // SAFETY: TUNSETOFFLOAD takes its argument by value.
    let r = unsafe { libc::ioctl(fd.as_raw_fd(), TUNSETOFFLOAD as _, 0 as libc::c_ulong) };
    if r < 0 {
        tracing::info!("{name} keeps its offloads: {}", io::Error::last_os_error());
    }
    Ok(fd)
}
