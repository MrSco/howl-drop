//! rtnetlink, for what the radio does to links and addresses: bring an
//! interface up or down, give it the station's MAC (AWDL-2), give `awdl0`
//! its link-local address without DAD (NET-1), and give each AWDL peer a
//! neighbour entry (NET-2).

use std::io;
use std::net::Ipv6Addr;

use crate::radio::MacAddr;
use nix::sys::socket::SockProtocol;

use crate::netlink::{Attrs, NLM_F_CREATE, NLM_F_REPLACE, Socket};

const RTM_NEWLINK: u16 = 16;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;
const RTM_NEWNEIGH: u16 = 28;
const RTM_DELNEIGH: u16 = 29;

const IFLA_ADDRESS: u16 = 1;
const IFLA_MTU: u16 = 4;
const IFF_UP: u32 = 0x1;

const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_FLAGS: u16 = 8;
const IFA_F_NODAD: u32 = 0x02;
const RT_SCOPE_LINK: u8 = 253;
const NDA_DST: u16 = 1;
const NDA_LLADDR: u16 = 2;
const NUD_PERMANENT: u16 = 0x80;
const AF_UNSPEC: u8 = 0;
const AF_INET6: u8 = 10;

pub struct Rtnl {
    socket: Socket,
}

impl Rtnl {
    pub fn open() -> io::Result<Rtnl> {
        Ok(Rtnl {
            socket: Socket::new(SockProtocol::NetlinkRoute)?,
        })
    }

    fn set_link(&mut self, index: u32, flags: u32, change: u32, a: &Attrs) -> io::Result<()> {
        let mut msg = ifinfomsg(index, flags, change);
        msg.extend_from_slice(&a.0);
        self.socket.request(RTM_NEWLINK, 0, &msg).map(drop)
    }

    pub fn set_up(&mut self, index: u32, up: bool) -> io::Result<()> {
        self.set_link(index, if up { IFF_UP } else { 0 }, IFF_UP, &Attrs::new())
    }

    pub fn set_mac(&mut self, index: u32, mac: MacAddr) -> io::Result<()> {
        let mut a = Attrs::new();
        a.bytes(IFLA_ADDRESS, &mac.0);
        self.set_link(index, 0, 0, &a)
    }

    pub fn set_mtu(&mut self, index: u32, mtu: u32) -> io::Result<()> {
        let mut a = Attrs::new();
        a.u32(IFLA_MTU, mtu);
        self.set_link(index, 0, 0, &a)
    }

    /// Adds a /64 link-scope address with no duplicate address detection,
    /// replacing it if it's there (and tentative) already.
    pub fn add_link_local_nodad(&mut self, index: u32, addr: Ipv6Addr) -> io::Result<()> {
        let mut msg = ifaddrmsg(index, IFA_F_NODAD as u8);
        let mut a = Attrs::new();
        a.bytes(IFA_LOCAL, &addr.octets())
            .bytes(IFA_ADDRESS, &addr.octets())
            .u32(IFA_FLAGS, IFA_F_NODAD);
        msg.extend_from_slice(&a.0);
        self.socket
            .request(RTM_NEWADDR, NLM_F_CREATE | NLM_F_REPLACE, &msg)
            .map(drop)
    }

    pub fn del_addr(&mut self, index: u32, addr: Ipv6Addr) -> io::Result<()> {
        let mut msg = ifaddrmsg(index, 0);
        let mut a = Attrs::new();
        a.bytes(IFA_LOCAL, &addr.octets())
            .bytes(IFA_ADDRESS, &addr.octets());
        msg.extend_from_slice(&a.0);
        self.socket.request(RTM_DELADDR, 0, &msg).map(drop)
    }

    /// NET-2: a permanent neighbour entry for an AWDL peer, `addr` at
    /// `mac`, so nothing waits on neighbour discovery over the air.
    pub fn add_neighbour(&mut self, index: u32, addr: Ipv6Addr, mac: MacAddr) -> io::Result<()> {
        let mut msg = ndmsg(index, NUD_PERMANENT);
        let mut a = Attrs::new();
        a.bytes(NDA_DST, &addr.octets()).bytes(NDA_LLADDR, &mac.0);
        msg.extend_from_slice(&a.0);
        self.socket
            .request(RTM_NEWNEIGH, NLM_F_CREATE | NLM_F_REPLACE, &msg)
            .map(drop)
    }

    pub fn del_neighbour(&mut self, index: u32, addr: Ipv6Addr) -> io::Result<()> {
        let mut msg = ndmsg(index, 0);
        let mut a = Attrs::new();
        a.bytes(NDA_DST, &addr.octets());
        msg.extend_from_slice(&a.0);
        self.socket.request(RTM_DELNEIGH, 0, &msg).map(drop)
    }
}

fn ndmsg(index: u32, state: u16) -> Vec<u8> {
    let mut v = vec![AF_INET6, 0, 0, 0];
    v.extend_from_slice(&(index as i32).to_ne_bytes());
    v.extend_from_slice(&state.to_ne_bytes());
    v.extend_from_slice(&[0, 0]); // flags, type
    v
}

fn ifinfomsg(index: u32, flags: u32, change: u32) -> Vec<u8> {
    let mut v = vec![AF_UNSPEC, 0, 0, 0];
    v.extend_from_slice(&(index as i32).to_ne_bytes());
    v.extend_from_slice(&flags.to_ne_bytes());
    v.extend_from_slice(&change.to_ne_bytes());
    v
}

fn ifaddrmsg(index: u32, flags: u8) -> Vec<u8> {
    let mut v = vec![AF_INET6, 64, flags, RT_SCOPE_LINK];
    v.extend_from_slice(&index.to_ne_bytes());
    v
}
