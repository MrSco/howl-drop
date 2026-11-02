//! Fork: where the driver's connections and datagrams come from. The host's
//! network stack by default ([`Os`]); or one the caller runs itself, such as
//! a TCP/IP stack in the same process on the AWDL node's packets, which
//! needs no network interface of the host's (HowlDrop on Windows).

use std::future::Future;
use std::io;
use std::net::{SocketAddr, SocketAddrV6};
use std::pin::Pin;

use tokio::io::{AsyncRead, AsyncWrite};

/// A connection's bytes both ways.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + 'static> Stream for T {}

pub type BoxStream = Box<dyn Stream>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Connections coming in, as a TCP listener's.
pub trait Listen: Send + 'static {
    fn accept(&mut self) -> BoxFuture<'_, io::Result<(BoxStream, SocketAddr)>>;
    fn local_addr(&self) -> io::Result<SocketAddr>;
}

/// Connections going out.
pub trait Connect: Send + Sync + 'static {
    fn connect(&self, addr: SocketAddrV6) -> BoxFuture<'_, io::Result<BoxStream>>;
}

/// Datagrams both ways, as an mDNS socket's: on port 5353, in the mDNS group.
pub trait Datagrams: Send + Sync + 'static {
    fn send_to<'a>(&'a self, buf: &'a [u8], to: SocketAddrV6) -> BoxFuture<'a, io::Result<usize>>;
    fn recv_from<'a>(&'a self, buf: &'a mut [u8])
    -> BoxFuture<'a, io::Result<(usize, SocketAddr)>>;
}

/// The host's network stack.
#[derive(Debug, Clone, Copy, Default)]
pub struct Os;

impl Os {
    pub async fn listen(addr: SocketAddrV6) -> io::Result<tokio::net::TcpListener> {
        tokio::net::TcpListener::bind(addr).await
    }
}

impl Listen for tokio::net::TcpListener {
    fn accept(&mut self) -> BoxFuture<'_, io::Result<(BoxStream, SocketAddr)>> {
        Box::pin(async move {
            let (stream, peer) = tokio::net::TcpListener::accept(self).await?;
            stream.set_nodelay(true)?;
            Ok((Box::new(stream) as BoxStream, peer))
        })
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        tokio::net::TcpListener::local_addr(self)
    }
}

impl Connect for Os {
    fn connect(&self, addr: SocketAddrV6) -> BoxFuture<'_, io::Result<BoxStream>> {
        Box::pin(async move {
            let stream = tokio::net::TcpStream::connect(addr).await?;
            stream.set_nodelay(true)?;
            Ok(Box::new(stream) as BoxStream)
        })
    }
}

impl Datagrams for tokio::net::UdpSocket {
    fn send_to<'a>(&'a self, buf: &'a [u8], to: SocketAddrV6) -> BoxFuture<'a, io::Result<usize>> {
        Box::pin(tokio::net::UdpSocket::send_to(self, buf, to))
    }

    fn recv_from<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> BoxFuture<'a, io::Result<(usize, SocketAddr)>> {
        Box::pin(tokio::net::UdpSocket::recv_from(self, buf))
    }
}
