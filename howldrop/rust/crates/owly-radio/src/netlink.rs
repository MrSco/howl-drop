//! Netlink, as much of it as the radio needs: request and response over a
//! socket, attributes in and out, and generic netlink's family lookup.
//! [`nl80211`](crate::nl80211) and [`rtnl`](crate::rtnl) build on it.
//!
//! Requests are synchronous. Each one waits for its acknowledgement, or for
//! the end of its dump, with a receive timeout so a kernel that never
//! answers can't hang the caller.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

use nix::sys::socket::{
    AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, bind, recv, send,
    setsockopt, sockopt,
};
use nix::sys::time::TimeVal;

pub const NLM_F_REQUEST: u16 = 0x1;
pub const NLM_F_ACK: u16 = 0x4;
pub const NLM_F_DUMP: u16 = 0x300;
/// RTM_NEWLINK and RTM_NEWADDR: create, or replace what's there.
pub const NLM_F_CREATE: u16 = 0x400;
pub const NLM_F_EXCL: u16 = 0x200;
pub const NLM_F_REPLACE: u16 = 0x100;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const HEADER: usize = 16;

const NLA_F_NESTED: u16 = 0x8000;
const NLA_TYPE_MASK: u16 = 0x3fff;

/// How long a request waits for the kernel.
const TIMEOUT: Duration = Duration::from_secs(5);

/// A netlink socket of one protocol.
pub struct Socket {
    fd: OwnedFd,
    seq: u32,
    buf: Vec<u8>,
}

/// One message of a response: its type and what follows the header.
#[derive(Debug, Clone)]
pub struct Message {
    pub ty: u16,
    pub payload: Vec<u8>,
}

impl Socket {
    pub fn new(protocol: SockProtocol) -> io::Result<Socket> {
        let fd = nix::sys::socket::socket(
            AddressFamily::Netlink,
            SockType::Raw,
            SockFlag::SOCK_CLOEXEC,
            protocol,
        )?;
        bind(fd.as_raw_fd(), &NetlinkAddr::new(0, 0))?;
        setsockopt(
            &fd,
            sockopt::ReceiveTimeout,
            &TimeVal::from(timeval(TIMEOUT)),
        )?;
        Ok(Socket {
            fd,
            seq: 0,
            buf: vec![0; 64 * 1024],
        })
    }

    /// Sends one request and collects the response: every message of a dump
    /// (`NLM_F_DUMP`), or the replies before the acknowledgement. A negative
    /// errno from the kernel comes back as that error.
    pub fn request(&mut self, ty: u16, flags: u16, payload: &[u8]) -> io::Result<Vec<Message>> {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;
        let dump = flags & NLM_F_DUMP == NLM_F_DUMP;
        let flags = flags | NLM_F_REQUEST | if dump { 0 } else { NLM_F_ACK };
        let mut msg = Vec::with_capacity(HEADER + payload.len());
        msg.extend_from_slice(&((HEADER + payload.len()) as u32).to_ne_bytes());
        msg.extend_from_slice(&ty.to_ne_bytes());
        msg.extend_from_slice(&flags.to_ne_bytes());
        msg.extend_from_slice(&seq.to_ne_bytes());
        msg.extend_from_slice(&0u32.to_ne_bytes());
        msg.extend_from_slice(payload);
        send(self.fd.as_raw_fd(), &msg, MsgFlags::empty())?;

        let mut out = Vec::new();
        loop {
            let n = match recv(self.fd.as_raw_fd(), &mut self.buf, MsgFlags::empty()) {
                Ok(n) => n,
                Err(nix::errno::Errno::EAGAIN) => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the kernel didn't answer a netlink request",
                    ));
                }
                Err(e) => return Err(e.into()),
            };
            let mut rest = &self.buf[..n];
            while rest.len() >= HEADER {
                let len = u32_at(rest, 0) as usize;
                if len < HEADER || len > rest.len() {
                    return Err(io::Error::other("a truncated netlink message"));
                }
                let mty = u16_at(rest, 4);
                let mseq = u32_at(rest, 8);
                let body = &rest[HEADER..len];
                rest = &rest[align(len).min(rest.len())..];
                if mseq != seq {
                    // A late answer to an earlier request that timed out.
                    continue;
                }
                match mty {
                    NLMSG_ERROR => {
                        let errno = body
                            .get(..4)
                            .map(|b| i32::from_ne_bytes(b.try_into().unwrap()))
                            .unwrap_or(-libc_eio());
                        if errno == 0 {
                            return Ok(out);
                        }
                        return Err(io::Error::from_raw_os_error(-errno));
                    }
                    NLMSG_DONE => return Ok(out),
                    // A reply, or one part of a dump; an acknowledgement or
                    // NLMSG_DONE ends the response either way.
                    _ => out.push(Message {
                        ty: mty,
                        payload: body.to_vec(),
                    }),
                }
            }
        }
    }
}

fn timeval(d: Duration) -> nix::libc::timeval {
    nix::libc::timeval {
        tv_sec: d.as_secs() as _,
        tv_usec: d.subsec_micros() as _,
    }
}

fn libc_eio() -> i32 {
    nix::libc::EIO
}

pub fn align(n: usize) -> usize {
    (n + 3) & !3
}

pub fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_ne_bytes([b[at], b[at + 1]])
}

pub fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_ne_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// Attributes being built: `nlattr` headers and payloads, each padded to
/// four bytes.
#[derive(Debug, Default, Clone)]
pub struct Attrs(pub Vec<u8>);

impl Attrs {
    pub fn new() -> Attrs {
        Attrs::default()
    }

    pub fn bytes(&mut self, ty: u16, value: &[u8]) -> &mut Attrs {
        let len = 4 + value.len();
        self.0.extend_from_slice(&(len as u16).to_ne_bytes());
        self.0.extend_from_slice(&ty.to_ne_bytes());
        self.0.extend_from_slice(value);
        self.0.resize(align(self.0.len()), 0);
        self
    }

    pub fn u8(&mut self, ty: u16, v: u8) -> &mut Attrs {
        self.bytes(ty, &[v])
    }

    pub fn u16(&mut self, ty: u16, v: u16) -> &mut Attrs {
        self.bytes(ty, &v.to_ne_bytes())
    }

    pub fn u32(&mut self, ty: u16, v: u32) -> &mut Attrs {
        self.bytes(ty, &v.to_ne_bytes())
    }

    pub fn flag(&mut self, ty: u16) -> &mut Attrs {
        self.bytes(ty, &[])
    }

    /// A string with its terminating NUL, as the kernel's `NLA_NUL_STRING`
    /// and `NLA_STRING` policies both accept.
    pub fn str(&mut self, ty: u16, s: &str) -> &mut Attrs {
        let mut v = s.as_bytes().to_vec();
        v.push(0);
        self.bytes(ty, &v)
    }

    pub fn nested(&mut self, ty: u16, inner: &Attrs) -> &mut Attrs {
        self.bytes(ty | NLA_F_NESTED, &inner.0)
    }
}

/// The attributes in `buf`, as (type, payload), flags masked off. Stops at
/// the first malformed one.
pub fn attrs(buf: &[u8]) -> impl Iterator<Item = (u16, &[u8])> {
    let mut rest = buf;
    std::iter::from_fn(move || {
        if rest.len() < 4 {
            return None;
        }
        let len = u16_at(rest, 0) as usize;
        let ty = u16_at(rest, 2) & NLA_TYPE_MASK;
        if len < 4 || len > rest.len() {
            return None;
        }
        let value = &rest[4..len];
        rest = &rest[align(len).min(rest.len())..];
        Some((ty, value))
    })
}

/// The first attribute of type `ty`.
pub fn attr(buf: &[u8], ty: u16) -> Option<&[u8]> {
    attrs(buf).find(|(t, _)| *t == ty).map(|(_, v)| v)
}

pub fn as_u8(v: &[u8]) -> Option<u8> {
    v.first().copied()
}

pub fn as_u16(v: &[u8]) -> Option<u16> {
    Some(u16::from_ne_bytes(v.get(..2)?.try_into().ok()?))
}

pub fn as_u32(v: &[u8]) -> Option<u32> {
    Some(u32::from_ne_bytes(v.get(..4)?.try_into().ok()?))
}

/// A string attribute, without its NUL.
pub fn as_str(v: &[u8]) -> Option<String> {
    let end = v.iter().position(|&b| b == 0).unwrap_or(v.len());
    String::from_utf8(v[..end].to_vec()).ok()
}

const GENL_ID_CTRL: u16 = 0x10;
const CTRL_CMD_GETFAMILY: u8 = 3;
const CTRL_ATTR_FAMILY_ID: u16 = 1;
const CTRL_ATTR_FAMILY_NAME: u16 = 2;

/// A generic netlink family, such as `nl80211`.
pub struct Genl {
    socket: Socket,
    family: u16,
}

impl Genl {
    pub fn open(name: &str) -> io::Result<Genl> {
        let mut socket = Socket::new(SockProtocol::NetlinkGeneric)?;
        let mut a = Attrs::new();
        a.str(CTRL_ATTR_FAMILY_NAME, name);
        let replies = socket.request(GENL_ID_CTRL, 0, &genl_payload(CTRL_CMD_GETFAMILY, &a))?;
        let family = replies
            .iter()
            .find_map(|m| attr(m.payload.get(4..)?, CTRL_ATTR_FAMILY_ID).and_then(as_u16))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("no generic netlink family {name}"),
                )
            })?;
        Ok(Genl { socket, family })
    }

    /// A command and its replies' attributes.
    pub fn request(&mut self, cmd: u8, flags: u16, a: &Attrs) -> io::Result<Vec<Vec<u8>>> {
        let replies = self
            .socket
            .request(self.family, flags, &genl_payload(cmd, a))?;
        Ok(replies
            .into_iter()
            .filter(|m| m.ty == self.family)
            .map(|m| m.payload.get(4..).unwrap_or_default().to_vec())
            .collect())
    }
}

fn genl_payload(cmd: u8, a: &Attrs) -> Vec<u8> {
    let mut v = vec![cmd, 0, 0, 0];
    v.extend_from_slice(&a.0);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributes_round_trip() {
        let mut inner = Attrs::new();
        inner.flag(6);
        let mut a = Attrs::new();
        a.u32(3, 7).str(4, "mon0").u8(9, 1).nested(23, &inner);
        // Every attribute starts on four bytes.
        assert_eq!(a.0.len() % 4, 0);
        let got: Vec<_> = attrs(&a.0).collect();
        assert_eq!(got.len(), 4);
        assert_eq!(as_u32(got[0].1), Some(7));
        assert_eq!(as_str(got[1].1).as_deref(), Some("mon0"));
        assert_eq!(got[1].1.len(), 5, "with its NUL");
        assert_eq!(as_u8(got[2].1), Some(1));
        assert_eq!(got[3].0, 23, "the nested flag is masked off");
        assert_eq!(attrs(got[3].1).map(|(t, _)| t).collect::<Vec<_>>(), [6]);
    }

    #[test]
    fn malformed_attributes_end_the_walk() {
        let mut a = Attrs::new();
        a.u32(1, 1);
        let mut bytes = a.0.clone();
        // A header claiming more than there is.
        bytes.extend_from_slice(&[40, 0, 2, 0, 0, 0]);
        assert_eq!(attrs(&bytes).count(), 1);
    }

    #[test]
    fn nl80211_is_found_without_root() {
        // Every Linux kernel with cfg80211 has it; a container may not.
        match Genl::open("nl80211") {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => panic!("{e}"),
        }
        assert!(Genl::open("owly-no-such-family").is_err());
    }
}
