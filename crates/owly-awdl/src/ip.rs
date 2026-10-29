//! The IPv6 packets AWDL's data path carries (docs/SPEC.md §3.5, §4), with the
//! link-layer addresses they travel between.

use alloc::vec::Vec;
use core::net::Ipv6Addr;

use crate::MacAddr;

/// An IPv6 packet and the link-layer addresses it travels between. On a
/// host with a TAP device these are its Ethernet frame's; a host without
/// one can derive them from the IPv6 addresses ([`Ipv6Packet::from_ip`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv6Packet {
    pub dst: MacAddr,
    pub src: MacAddr,
    /// From the IPv6 header on.
    pub data: Vec<u8>,
}

const ETHERTYPE_IPV6: [u8; 2] = [0x86, 0xdd];
const HEADER_LEN: usize = 40;
const TCP: u8 = 6;
const ICMPV6: u8 = 58;

impl Ipv6Packet {
    /// The packet in an Ethernet frame, if it carries IPv6 (NET-3).
    pub fn from_ethernet(frame: &[u8]) -> Option<Ipv6Packet> {
        if frame.len() < 14 + HEADER_LEN || frame[12..14] != ETHERTYPE_IPV6 {
            return None;
        }
        Some(Ipv6Packet {
            dst: MacAddr(frame[0..6].try_into().ok()?),
            src: MacAddr(frame[6..12].try_into().ok()?),
            data: frame[14..].to_vec(),
        })
    }

    pub fn to_ethernet(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(14 + self.data.len());
        out.extend_from_slice(&self.dst.0);
        out.extend_from_slice(&self.src.0);
        out.extend_from_slice(&ETHERTYPE_IPV6);
        out.extend_from_slice(&self.data);
        out
    }

    /// A packet whose link-layer addresses follow from its IPv6 ones: a
    /// link-local unicast address gives the MAC it was made from (NET-2),
    /// and a multicast one its `33:33` group.
    pub fn from_ip(data: Vec<u8>) -> Option<Ipv6Packet> {
        let (src, dst) = addrs(&data)?;
        let dst = if dst.is_multicast() {
            let o = dst.octets();
            MacAddr([0x33, 0x33, o[12], o[13], o[14], o[15]])
        } else {
            MacAddr::from_link_local(dst)?
        };
        Some(Ipv6Packet {
            dst,
            src: MacAddr::from_link_local(src)?,
            data,
        })
    }

    /// The source and destination addresses.
    pub fn addrs(&self) -> Option<(Ipv6Addr, Ipv6Addr)> {
        addrs(&self.data)
    }

    /// A TCP SYN to `local` carrying nothing else: the start of a
    /// connection, as a peer opens one.
    pub fn is_bare_syn_to(&self, local: MacAddr) -> bool {
        if self.dst != local {
            return false;
        }
        let Some(tcp) = upper(&self.data, TCP) else {
            return false;
        };
        if tcp.len() < 20 {
            return false;
        }
        let flags = tcp[13] & 0x17; // FIN, SYN, RST, ACK
        let offset = (tcp[12] >> 4) as usize * 4;
        flags == 0x02 && offset >= 20 && tcp.len() == offset
    }

    /// An ICMPv6 echo request from `src` to `dst`, link-local to
    /// link-local.
    pub fn echo_request(
        src: MacAddr,
        dst: MacAddr,
        id: u16,
        seq: u16,
        payload: &[u8],
    ) -> Ipv6Packet {
        let (s, d) = (src.link_local(), dst.link_local());
        let mut icmp = Vec::with_capacity(8 + payload.len());
        icmp.extend_from_slice(&[128, 0, 0, 0]);
        icmp.extend_from_slice(&id.to_be_bytes());
        icmp.extend_from_slice(&seq.to_be_bytes());
        icmp.extend_from_slice(payload);
        let sum = checksum(s, d, ICMPV6, &icmp);
        icmp[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut data = Vec::with_capacity(HEADER_LEN + icmp.len());
        data.extend_from_slice(&[0x60, 0, 0, 0]);
        data.extend_from_slice(&(icmp.len() as u16).to_be_bytes());
        data.push(ICMPV6);
        data.push(255);
        data.extend_from_slice(&s.octets());
        data.extend_from_slice(&d.octets());
        data.extend_from_slice(&icmp);
        Ipv6Packet { dst, src, data }
    }

    /// Whether this is an ICMPv6 echo reply with identifier `id`.
    pub fn is_echo_reply(&self, id: u16) -> bool {
        upper(&self.data, ICMPV6)
            .is_some_and(|m| m.len() >= 8 && m[0] == 129 && m[4..6] == id.to_be_bytes())
    }
}

fn addrs(data: &[u8]) -> Option<(Ipv6Addr, Ipv6Addr)> {
    if data.len() < HEADER_LEN || data[0] >> 4 != 6 {
        return None;
    }
    let src: [u8; 16] = data[8..24].try_into().ok()?;
    let dst: [u8; 16] = data[24..40].try_into().ok()?;
    Some((Ipv6Addr::from(src), Ipv6Addr::from(dst)))
}

/// The upper-layer message, if the packet's next header is `protocol`
/// with no extension headers between.
fn upper(data: &[u8], protocol: u8) -> Option<&[u8]> {
    addrs(data)?;
    if data[6] != protocol {
        return None;
    }
    let len = u16::from_be_bytes([data[4], data[5]]) as usize;
    data.get(HEADER_LEN..HEADER_LEN + len)
}

/// The Internet checksum over the IPv6 pseudo-header and `message`.
fn checksum(src: Ipv6Addr, dst: Ipv6Addr, next: u8, message: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut add = |b: &[u8]| {
        for pair in b.chunks(2) {
            let hi = pair[0] as u32;
            let lo = pair.get(1).copied().unwrap_or(0) as u32;
            sum += (hi << 8) | lo;
        }
    };
    add(&src.octets());
    add(&dst.octets());
    add(&(message.len() as u32).to_be_bytes());
    add(&[0, 0, 0, next]);
    add(message);
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    const US: MacAddr = MacAddr([0x02, 0, 0, 0, 0, 0x01]);
    const PHONE: MacAddr = MacAddr([0x86, 0x08, 0x04, 0x8a, 0x2b, 0xf4]);

    fn tcp(flags: u8, payload: &[u8]) -> Ipv6Packet {
        let mut seg = alloc::vec![0u8; 20];
        seg[12] = 5 << 4;
        seg[13] = flags;
        seg.extend_from_slice(payload);
        let mut data = alloc::vec![0x60, 0, 0, 0];
        data.extend_from_slice(&(seg.len() as u16).to_be_bytes());
        data.extend_from_slice(&[TCP, 64]);
        data.extend_from_slice(&PHONE.link_local().octets());
        data.extend_from_slice(&US.link_local().octets());
        data.extend_from_slice(&seg);
        Ipv6Packet {
            dst: US,
            src: PHONE,
            data,
        }
    }

    #[test]
    fn bare_syns_are_told_apart() {
        assert!(tcp(0x02, &[]).is_bare_syn_to(US));
        assert!(!tcp(0x02, &[]).is_bare_syn_to(PHONE), "not to us");
        assert!(!tcp(0x12, &[]).is_bare_syn_to(US), "SYN-ACK");
        assert!(!tcp(0x02, b"x").is_bare_syn_to(US), "with data");
        assert!(!tcp(0x10, &[]).is_bare_syn_to(US), "ACK");
    }

    #[test]
    fn echo_requests_are_valid_icmpv6() {
        let p = Ipv6Packet::echo_request(US, PHONE, 0x4f77, 1, b"owly");
        let (s, d) = p.addrs().unwrap();
        assert_eq!((s, d), (US.link_local(), PHONE.link_local()));
        // A correct checksum sums, with itself, to zero.
        let icmp = &p.data[HEADER_LEN..];
        assert_eq!(checksum(s, d, ICMPV6, icmp), 0);
        assert_eq!(p.data[7], 255, "hop limit");
        // The reply: the same message, type 129, addresses swapped.
        let mut reply = p.data.clone();
        reply[8..24].copy_from_slice(&d.octets());
        reply[24..40].copy_from_slice(&s.octets());
        reply[HEADER_LEN] = 129;
        let reply = Ipv6Packet {
            dst: US,
            src: PHONE,
            data: reply,
        };
        assert!(reply.is_echo_reply(0x4f77));
        assert!(!reply.is_echo_reply(1));
        assert!(!p.is_echo_reply(0x4f77));
    }

    #[test]
    fn ethernet_and_ip_addressing() {
        let p = tcp(0x02, &[]);
        let eth = p.to_ethernet();
        assert_eq!(Ipv6Packet::from_ethernet(&eth), Some(p.clone()));
        let mut v4 = eth.clone();
        v4[12..14].copy_from_slice(&[0x08, 0x00]);
        assert_eq!(Ipv6Packet::from_ethernet(&v4), None);
        assert_eq!(Ipv6Packet::from_ip(p.data.clone()), Some(p.clone()));
        let mut mdns = p.data;
        mdns[24..40].copy_from_slice(&"ff02::fb".parse::<Ipv6Addr>().unwrap().octets());
        let m = Ipv6Packet::from_ip(mdns).unwrap();
        assert_eq!(m.dst, MacAddr([0x33, 0x33, 0, 0, 0, 0xfb]));
    }
}
