//! The link on Windows, inside the agent: a TCP/IP stack of its own
//! (smoltcp, through tokio-smoltcp) on the AWDL node's IPv6 packets, so no
//! network adapter of Windows' is needed, nor the administrator one would
//! take. The receiver listens on it, the sender connects through it, and
//! mDNS (UDP to and from port 5353, which smoltcp's multicast doesn't
//! reach) is answered beside it (owly-airdrop's transport traits).

use std::io;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use futures_util::{Sink, Stream};
use owly_airdrop::driver::transport::{BoxFuture, BoxStream, Connect, Datagrams, Listen};
use owly_awdl::Ipv6Packet;
use owly_awdl::driver::Netif;
use smoltcp::iface::Config;
use smoltcp::phy::{DeviceCapabilities, Medium};
use smoltcp::wire::{HardwareAddress, IpCidr};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;
use tokio_smoltcp::device::AsyncDevice;
use tokio_smoltcp::{BufferSize, Net, NetConfig};
use tokio_util::sync::PollSender;

/// As awdl0's (owly-radio's TAP_MTU).
const MTU: usize = 1450;
/// A TCP connection's receive buffer: enough window for an iPhone's
/// upload at full speed.
const TCP_RX_BUFFER: usize = 1 << 20;
/// A TCP connection's send buffer, which bounds the data in flight. smoltcp
/// has no SACK and recovers a lost segment by resending everything after it,
/// while the iPhone listens on our channel only in some AWDL slots. A
/// megabyte in flight turned each loss into a megabyte resent behind the
/// hole and stalled uploads; a classic 64 KiB window keeps recovery cheap.
const TCP_TX_BUFFER: usize = 64 << 10;
const MDNS_PORT: u16 = 5353;
const UDP: u8 = 17;
/// Packets queued between the node and the stack, each way.
const QUEUE: usize = 1024;

/// The stack, for the receiver, the sender and mDNS.
pub struct Stack {
    net: Net,
    addr: Ipv6Addr,
    mdns: Arc<Mdns>,
}

impl Stack {
    /// The stack at `addr`, and the node's end of it. Inside a tokio
    /// runtime, which runs it.
    pub fn new(addr: Ipv6Addr) -> (Arc<Stack>, StackNetif) {
        let (to_stack, from_node) = mpsc::channel(QUEUE);
        let (to_node, from_stack) = mpsc::channel(QUEUE);
        let (mdns_in, mdns_rx) = mpsc::channel(64);
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ip;
        caps.max_transmission_unit = MTU;
        let device = Device {
            rx: from_node,
            tx: PollSender::new(to_node.clone()),
            caps,
        };
        let mut iface = Config::new(HardwareAddress::Ip);
        let mut seed = [0u8; 8];
        let _ = getrandom::fill(&mut seed);
        iface.random_seed = u64::from_le_bytes(seed);
        let mut config = NetConfig::new(iface, IpCidr::new(addr.into(), 64), Vec::new());
        config.buffer_size = BufferSize {
            tcp_rx_size: TCP_RX_BUFFER,
            tcp_tx_size: tcp_tx_buffer(),
            ..BufferSize::default()
        };
        let net = Net::new(device, config);
        let mdns = Arc::new(Mdns {
            addr,
            out: to_node,
            rx: tokio::sync::Mutex::new(mdns_rx),
        });
        let netif = StackNetif {
            to_stack,
            from_stack,
            mdns_in,
            addr,
            packets: 0,
            bytes: 0,
            dropped: 0,
            mdns_dropped: 0,
            logged: std::time::Instant::now(),
        };
        (Arc::new(Stack { net, addr, mdns }), netif)
    }

    /// Connections to `port` on the stack's address.
    pub async fn listen(&self, port: u16) -> io::Result<Listener> {
        let listener = self
            .net
            .tcp_bind(SocketAddr::new(self.addr.into(), port))
            .await?;
        Ok(Listener(listener))
    }

    /// mDNS's socket: one at a time, as the agent registers one service.
    pub fn mdns(&self) -> Arc<dyn Datagrams> {
        self.mdns.clone()
    }
}

/// The receiver's connections.
pub struct Listener(tokio_smoltcp::TcpListener);

impl Listen for Listener {
    fn accept(&mut self) -> BoxFuture<'_, io::Result<(BoxStream, SocketAddr)>> {
        Box::pin(async move {
            let (stream, peer) = self.0.accept().await?;
            Ok((Box::new(Tcp(Some(stream))) as BoxStream, peer))
        })
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.0.local_addr()
    }
}

/// The sender's connections.
#[derive(Clone)]
pub struct Connector(pub Arc<Stack>);

impl Connect for Connector {
    fn connect(&self, addr: SocketAddrV6) -> BoxFuture<'_, io::Result<BoxStream>> {
        Box::pin(async move {
            // A connection closed while connecting was reset: refused, as
            // the host's stack says.
            let stream = self
                .0
                .net
                .tcp_connect(SocketAddr::V6(addr))
                .await
                .map_err(|e| match e.kind() {
                    io::ErrorKind::ConnectionAborted => {
                        io::Error::new(io::ErrorKind::ConnectionRefused, e)
                    }
                    _ => e,
                })?;
            Ok(Box::new(Tcp(Some(stream))) as BoxStream)
        })
    }
}

/// A connection that closes as the host's do. tokio-smoltcp's shutdown
/// waits for the peer to close too, which an iPhone does seconds later, and
/// dropping one forgets it at once, unsent data and FIN with it. Here
/// shutdown sends the FIN and waits only for what was written to be taken,
/// and a dropped connection is closed in the background.
struct Tcp(Option<tokio_smoltcp::TcpStream>);

/// How long a dropped connection may take to close.
const LINGER: std::time::Duration = std::time::Duration::from_secs(10);

impl Tcp {
    fn inner(&mut self) -> Pin<&mut tokio_smoltcp::TcpStream> {
        Pin::new(self.0.as_mut().expect("open until dropped"))
    }
}

impl AsyncRead for Tcp {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.inner().poll_read(cx, buf)
    }
}

impl AsyncWrite for Tcp {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.inner().poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Like an OS TCP socket: writes already notify the reactor. Waiting
        // for every ACK here makes each TLS record a stop-and-wait exchange
        // across the phone's intermittent AWDL slots. Shutdown below still
        // drains the queue before the connection is released.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Its first poll queues the FIN; Closed, or not yet, is the peer's.
        if let Poll::Ready(Err(e)) = self.inner().poll_shutdown(cx) {
            return Poll::Ready(Err(e));
        }
        self.inner().poll_flush(cx)
    }
}

impl Drop for Tcp {
    fn drop(&mut self) {
        let Some(mut stream) = self.0.take() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        runtime.spawn(async move {
            let close = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_shutdown(cx));
            let _ = tokio::time::timeout(LINGER, close).await;
        });
    }
}

/// mDNS: UDP on port 5353, read off the node's packets before the stack
/// sees them, and written as packets of its own.
struct Mdns {
    addr: Ipv6Addr,
    out: mpsc::Sender<Vec<u8>>,
    rx: tokio::sync::Mutex<mpsc::Receiver<(Vec<u8>, SocketAddrV6)>>,
}

impl Datagrams for Mdns {
    fn send_to<'a>(&'a self, buf: &'a [u8], to: SocketAddrV6) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(async move {
            let packet = udp_packet(self.addr, MDNS_PORT, *to.ip(), to.port(), buf);
            self.out
                .send(packet)
                .await
                .map_err(|_| io::Error::other("the link is down"))?;
            Ok(buf.len())
        })
    }

    fn recv_from<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> BoxFuture<'a, io::Result<(usize, SocketAddr)>> {
        Box::pin(async move {
            let (data, from) = self
                .rx
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| io::Error::other("the link is down"))?;
            let n = data.len().min(buf.len());
            buf[..n].copy_from_slice(&data[..n]);
            Ok((n, SocketAddr::V6(from)))
        })
    }
}

/// The stack's device: the node's packets in, the stack's out.
struct Device {
    rx: mpsc::Receiver<Vec<u8>>,
    tx: PollSender<Vec<u8>>,
    caps: DeviceCapabilities,
}

impl Stream for Device {
    type Item = io::Result<Vec<u8>>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx).map(|p| p.map(Ok))
    }
}

impl Sink<Vec<u8>> for Device {
    type Error = io::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tx.poll_reserve(cx).map_err(io::Error::other)
    }

    fn start_send(mut self: Pin<&mut Self>, item: Vec<u8>) -> io::Result<()> {
        trace_tcp("out", &item);
        self.tx.send_item(item).map_err(io::Error::other)
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_close(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tx.close();
        Poll::Ready(Ok(()))
    }
}

impl AsyncDevice for Device {
    fn capabilities(&self) -> &DeviceCapabilities {
        &self.caps
    }
}

/// The AWDL node's end of the stack.
pub struct StackNetif {
    to_stack: mpsc::Sender<Vec<u8>>,
    from_stack: mpsc::Receiver<Vec<u8>>,
    mdns_in: mpsc::Sender<(Vec<u8>, SocketAddrV6)>,
    addr: Ipv6Addr,
    packets: u64,
    bytes: u64,
    dropped: u64,
    mdns_dropped: u64,
    logged: std::time::Instant,
}

impl Netif for StackNetif {
    async fn recv(&mut self) -> io::Result<Ipv6Packet> {
        loop {
            let data = self
                .from_stack
                .recv()
                .await
                .ok_or_else(|| io::Error::other("the stack stopped"))?;
            // One from an address with no MAC (none of ours) can't go.
            if let Some(packet) = Ipv6Packet::from_ip(data) {
                return Ok(packet);
            }
        }
    }

    fn deliver(&mut self, packet: Ipv6Packet) {
        trace_tcp("in", &packet.data);
        self.packets += 1;
        self.bytes += packet.data.len() as u64;
        if self.logged.elapsed() >= std::time::Duration::from_secs(10) {
            self.logged = std::time::Instant::now();
            tracing::info!(
                packets = self.packets,
                bytes = self.bytes,
                queue_drops = self.dropped,
                mdns_queue_drops = self.mdns_dropped,
                queued = QUEUE - self.to_stack.capacity(),
                "network stack receive totals"
            );
        }
        if let Some((from, payload)) = mdns_payload(&packet.data, self.addr) {
            if self.mdns_in.try_send((payload.to_vec(), from)).is_err() {
                self.mdns_dropped += 1;
            }
            return;
        }
        // A full queue drops it, as a full ring would.
        if self.to_stack.try_send(packet.data).is_err() {
            self.dropped += 1;
        }
    }
}

// Header-only diagnostics: never log TLS payload or shared content.
fn trace_tcp(direction: &str, packet: &[u8]) {
    if !tracing::enabled!(tracing::Level::TRACE) || packet.len() < 60 || packet[6] != 6 {
        return;
    }
    let tcp = &packet[40..];
    let header = usize::from(tcp[12] >> 4) * 4;
    if header < 20 || header > tcp.len() {
        return;
    }
    tracing::trace!(
        direction,
        source = u16::from_be_bytes([tcp[0], tcp[1]]),
        destination = u16::from_be_bytes([tcp[2], tcp[3]]),
        seq = u32::from_be_bytes(tcp[4..8].try_into().unwrap()),
        ack = u32::from_be_bytes(tcp[8..12].try_into().unwrap()),
        flags = tcp[13],
        window = u16::from_be_bytes([tcp[14], tcp[15]]),
        payload = tcp.len() - header,
        "TCP"
    );
}

/// The send buffer, overridable with `HOWLDROP_TCP_TX_KB` for radio benchmarks.
fn tcp_tx_buffer() -> usize {
    std::env::var("HOWLDROP_TCP_TX_KB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|kb| (16..=4096).contains(kb))
        .map_or(TCP_TX_BUFFER, |kb| kb << 10)
}

/// An mDNS datagram in `packet` (UDP to port 5353, not from `ours`): who
/// sent it, and its payload.
fn mdns_payload(packet: &[u8], ours: Ipv6Addr) -> Option<(SocketAddrV6, &[u8])> {
    if packet.len() < 48 || packet[0] >> 4 != 6 || packet[6] != UDP {
        return None;
    }
    let src = Ipv6Addr::from(<[u8; 16]>::try_from(&packet[8..24]).ok()?);
    let udp = &packet[40..];
    let (sport, dport) = (
        u16::from_be_bytes([udp[0], udp[1]]),
        u16::from_be_bytes([udp[2], udp[3]]),
    );
    let len = usize::from(u16::from_be_bytes([udp[4], udp[5]]));
    if dport != MDNS_PORT || src == ours || len < 8 || len > udp.len() {
        return None;
    }
    Some((SocketAddrV6::new(src, sport, 0, 0), &udp[8..len]))
}

/// An IPv6 packet carrying `payload` as UDP, with its checksum.
fn udp_packet(src: Ipv6Addr, sport: u16, dst: Ipv6Addr, dport: u16, payload: &[u8]) -> Vec<u8> {
    let udp_len = 8 + payload.len();
    let mut p = Vec::with_capacity(40 + udp_len);
    p.extend_from_slice(&[0x60, 0, 0, 0]);
    p.extend_from_slice(&(udp_len as u16).to_be_bytes());
    // UDP, and a hop limit of 255, as mDNS on a link asks.
    p.extend_from_slice(&[UDP, 255]);
    p.extend_from_slice(&src.octets());
    p.extend_from_slice(&dst.octets());
    p.extend_from_slice(&sport.to_be_bytes());
    p.extend_from_slice(&dport.to_be_bytes());
    p.extend_from_slice(&(udp_len as u16).to_be_bytes());
    p.extend_from_slice(&[0, 0]);
    p.extend_from_slice(payload);
    let sum = checksum(src, dst, &p[40..]);
    p[46..48].copy_from_slice(&sum.to_be_bytes());
    p
}

/// UDP's checksum over IPv6 (RFC 8200 8.1): the pseudo-header and `udp`.
fn checksum(src: Ipv6Addr, dst: Ipv6Addr, udp: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut add = |bytes: &[u8]| {
        for pair in bytes.chunks(2) {
            let word = u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]);
            sum += u32::from(word);
        }
    };
    add(&src.octets());
    add(&dst.octets());
    add(&(udp.len() as u32).to_be_bytes());
    add(&[0, 0, 0, UDP]);
    add(udp);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    match !(sum as u16) {
        0 => 0xffff,
        s => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn socket_flush_does_not_wait_for_acks_and_large_writes_are_delivered() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let addr_a = owly_awdl::MacAddr([2, 0, 0, 0, 0, 1]).link_local();
        let addr_b = owly_awdl::MacAddr([2, 0, 0, 0, 0, 2]).link_local();
        let (a, mut link_a) = Stack::new(addr_a);
        let (b, mut link_b) = Stack::new(addr_b);
        let hold = Arc::new(AtomicBool::new(false));
        let filter = hold.clone();
        let dropped = Arc::new(AtomicBool::new(false));
        let loss = dropped.clone();
        let bridge = tokio::spawn(async move {
            loop {
                tokio::select! {
                p = link_a.recv() => {
                    let p = p.unwrap();
                    if p.data.len() <= 1000 || loss.swap(true, Ordering::Relaxed) { link_b.deliver(p); }
                },
                    p = link_b.recv() => { if !filter.load(Ordering::Relaxed) { link_a.deliver(p.unwrap()); } }
                }
            }
        });
        let exercise = async {
            let mut listener = b.listen(8771).await.unwrap();
            let connector = Connector(a);
            let (client, accepted) = tokio::join!(
                connector.connect(SocketAddrV6::new(addr_b, 8771, 0, 0)),
                listener.accept()
            );
            let mut client = client.unwrap();
            let (mut server, _) = accepted.unwrap();
            hold.store(true, Ordering::Relaxed);
            client.write_all(b"ping").await.unwrap();
            tokio::time::timeout(Duration::from_millis(100), client.flush())
                .await
                .expect("flush waited for ACKs")
                .unwrap();
            let mut ping = [0; 4];
            server.read_exact(&mut ping).await.unwrap();
            assert_eq!(&ping, b"ping");
            hold.store(false, Ordering::Relaxed);
            // More than either TCP buffer, exercising wakeups and backpressure.
            let data = vec![0x5a; 3 << 20];
            let send = async {
                client.write_all(&data).await.unwrap();
                client.flush().await.unwrap();
            };
            let receive = async {
                let mut received = vec![0; data.len()];
                server.read_exact(&mut received).await.unwrap();
                assert_eq!(received, data);
            };
            tokio::join!(send, receive);
        };
        let result = tokio::time::timeout(Duration::from_secs(10), exercise).await;
        bridge.abort();
        result.unwrap();
        assert!(
            dropped.load(Ordering::Relaxed),
            "the test must exercise packet loss"
        );
    }

    #[test]
    fn tcp_has_congestion_control_for_the_radio() {
        use smoltcp::socket::tcp;
        let socket = tcp::Socket::new(
            tcp::SocketBuffer::new(vec![0; 4096]),
            tcp::SocketBuffer::new(vec![0; 4096]),
        );
        assert_eq!(socket.congestion_control(), tcp::CongestionControl::Reno);
    }

    #[test]
    fn mdns_goes_out_and_comes_back() {
        let ours: Ipv6Addr = "fe80::2c0:caff:feba:a70b".parse().unwrap();
        let phone: Ipv6Addr = "fe80::4c4f:7bff:fea7:f270".parse().unwrap();
        let group: Ipv6Addr = "ff02::fb".parse().unwrap();
        let packet = udp_packet(phone, MDNS_PORT, group, MDNS_PORT, b"query");
        assert_eq!(packet.len(), 40 + 8 + 5);
        // A receiver's own sum over the whole of it comes to zero.
        let mut whole = udp_packet(phone, MDNS_PORT, group, MDNS_PORT, b"query");
        let sum = u16::from_be_bytes([whole[46], whole[47]]);
        whole[46..48].copy_from_slice(&[0, 0]);
        assert_eq!(checksum(phone, group, &whole[40..]), sum);
        let (from, payload) = mdns_payload(&packet, ours).unwrap();
        assert_eq!(
            (*from.ip(), from.port(), payload),
            (phone, MDNS_PORT, &b"query"[..])
        );
        // Our own, and other ports, aren't mDNS for us.
        assert!(mdns_payload(&udp_packet(ours, MDNS_PORT, group, MDNS_PORT, b"x"), ours).is_none());
        assert!(mdns_payload(&udp_packet(phone, 8771, ours, 8771, b"x"), ours).is_none());
    }
}
