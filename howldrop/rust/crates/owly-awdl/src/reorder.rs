//! Receive reordering for the Block Ack sessions a node accepts (AWDL-15).
//!
//! With a session up a phone sends A-MPDUs, and the subframes the radio
//! missed come again in a later one. mac80211 holds frames behind such a
//! gap for a station; a monitor passes each one up as it arrives, so TCP
//! sees most segments out of order, answers with duplicate ACKs, and the
//! phone resends what we already have. Here, per peer and TID, from the
//! ADDBA Request's starting sequence number, a frame's packets wait for the
//! gap before them until it fills, the window moves past it, a Block Ack
//! Request gives it up, or [`TIMEOUT_US`] pass. Old and duplicate frames
//! are dropped, and DELBA ends the session.

use alloc::vec::Vec;

use crate::ip::Ipv6Packet;
use crate::{MacAddr, Time};

/// The most an ADDBA Response of ours offers.
pub const WINDOW: u16 = 64;
/// mac80211's `HT_RX_REORDER_BUF_TIMEOUT`.
pub const TIMEOUT_US: u64 = 100_000;
/// Sessions kept at once; a new one beyond replaces the first.
const SESSIONS: usize = 4;
const SN_MASK: u16 = 0x0fff;
const SN_HALF: u16 = 0x0800;

/// Counters, for diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub held: u64,
    pub delivered_late: u64,
    pub dropped_old: u64,
    pub dropped_duplicate: u64,
    pub released_by_timeout: u64,
    pub released_by_bar: u64,
}

struct Slot {
    packets: Vec<Ipv6Packet>,
    since: Time,
}

struct Session {
    peer: MacAddr,
    tid: u8,
    /// The next sequence number to deliver.
    head: u16,
    /// The window agreed in the ADDBA Response.
    size: u16,
    held: usize,
    slots: Vec<Option<Slot>>,
}

/// How far `sn` is ahead of `base` in the 12-bit sequence space;
/// [`SN_HALF`] or more means behind.
fn ahead(sn: u16, base: u16) -> u16 {
    sn.wrapping_sub(base) & SN_MASK
}

impl Session {
    fn slot(&mut self, sn: u16) -> &mut Option<Slot> {
        &mut self.slots[(sn % WINDOW) as usize]
    }

    /// Delivers from head while the next frame is here.
    fn release_in_order(&mut self, out: &mut Vec<Ipv6Packet>) {
        while let Some(slot) = self.slot(self.head).take() {
            out.extend(slot.packets);
            self.held -= 1;
            self.head = (self.head + 1) & SN_MASK;
        }
    }

    /// Moves head to `to`, delivering what is held on the way and skipping
    /// the gaps, then whatever follows in order.
    fn advance(&mut self, to: u16, out: &mut Vec<Ipv6Packet>) {
        while self.head != to {
            if let Some(slot) = self.slot(self.head).take() {
                out.extend(slot.packets);
                self.held -= 1;
            }
            self.head = (self.head + 1) & SN_MASK;
        }
        self.release_in_order(out);
    }

    fn flush(&mut self, out: &mut Vec<Ipv6Packet>) {
        if self.held > 0 {
            self.advance((self.head + self.size) & SN_MASK, out);
        }
    }
}

/// The sessions of every peer.
#[derive(Default)]
pub struct Reorder {
    sessions: Vec<Session>,
    stats: Stats,
}

impl Reorder {
    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// A session accepted with this window, from `ssn`. What an earlier one
    /// held is delivered.
    pub fn start(
        &mut self,
        peer: MacAddr,
        tid: u8,
        size: u16,
        ssn: u16,
        out: &mut Vec<Ipv6Packet>,
    ) {
        self.stop(peer, tid, out);
        if self.sessions.len() == SESSIONS {
            let mut first = self.sessions.remove(0);
            first.flush(out);
        }
        let size = if size == 0 || size > WINDOW {
            WINDOW
        } else {
            size
        };
        self.sessions.push(Session {
            peer,
            tid,
            head: ssn & SN_MASK,
            size,
            held: 0,
            slots: (0..WINDOW).map(|_| None).collect(),
        });
    }

    /// DELBA: what the session held is delivered.
    pub fn stop(&mut self, peer: MacAddr, tid: u8, out: &mut Vec<Ipv6Packet>) {
        if let Some(i) = self
            .sessions
            .iter()
            .position(|s| s.peer == peer && s.tid == tid)
        {
            self.sessions.remove(i).flush(out);
        }
    }

    /// A frame's packets, `sn` its sequence number: into `out` when they
    /// may go now (at once, outside a session), else held. A frame with no
    /// packets for us still fills its place.
    pub fn rx(
        &mut self,
        peer: MacAddr,
        tid: u8,
        sn: u16,
        packets: Vec<Ipv6Packet>,
        now: Time,
        out: &mut Vec<Ipv6Packet>,
    ) {
        let stats = &mut self.stats;
        let Some(s) = self
            .sessions
            .iter_mut()
            .find(|s| s.peer == peer && s.tid == tid)
        else {
            out.extend(packets);
            return;
        };
        let sn = sn & SN_MASK;
        let a = ahead(sn, s.head);
        if a >= SN_HALF {
            // Behind the window: delivered already, or given up on.
            stats.dropped_old += 1;
            return;
        }
        if a >= s.size {
            // Past the window: it moves up so sn is its last.
            s.advance(sn.wrapping_sub(s.size - 1) & SN_MASK, out);
        }
        if s.slot(sn).is_some() {
            stats.dropped_duplicate += 1;
            return;
        }
        if sn != s.head {
            stats.held += 1;
        } else if s.held > 0 {
            stats.delivered_late += 1;
        }
        *s.slot(sn) = Some(Slot {
            packets,
            since: now,
        });
        s.held += 1;
        s.release_in_order(out);
    }

    /// A Block Ack Request: the peer gave up on what is before `ssn`.
    pub fn bar(&mut self, peer: MacAddr, tid: u8, ssn: u16, out: &mut Vec<Ipv6Packet>) {
        let stats = &mut self.stats;
        let Some(s) = self
            .sessions
            .iter_mut()
            .find(|s| s.peer == peer && s.tid == tid)
        else {
            return;
        };
        let a = ahead(ssn & SN_MASK, s.head);
        if a == 0 || a >= SN_HALF {
            return;
        }
        if s.held > 0 {
            stats.released_by_bar += 1;
        }
        s.advance(ssn & SN_MASK, out);
    }

    /// Gives up the gaps before frames held [`TIMEOUT_US`] or longer.
    pub fn expire(&mut self, now: Time, out: &mut Vec<Ipv6Packet>) {
        for s in self.sessions.iter_mut().filter(|s| s.held > 0) {
            let mut last = None;
            for k in 0..s.size {
                let sn = (s.head + k) & SN_MASK;
                if let Some(slot) = s.slot(sn)
                    && now.since(slot.since) >= TIMEOUT_US
                {
                    last = Some(sn);
                }
            }
            if let Some(sn) = last {
                self.stats.released_by_timeout += 1;
                s.advance((sn + 1) & SN_MASK, out);
            }
        }
    }

    /// When the oldest frame held times out.
    pub fn deadline(&self) -> Option<Time> {
        self.sessions
            .iter()
            .flat_map(|s| s.slots.iter().flatten())
            .map(|slot| slot.since.plus(TIMEOUT_US))
            .min()
    }

    /// Whether a session with `peer` on `tid` is up.
    pub fn active(&self, peer: MacAddr, tid: u8) -> bool {
        self.sessions.iter().any(|s| s.peer == peer && s.tid == tid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    const P: MacAddr = MacAddr([2, 0, 0, 0, 0, 1]);

    fn pkt(n: u8) -> Vec<Ipv6Packet> {
        vec![Ipv6Packet {
            dst: MacAddr([2, 0, 0, 0, 0, 9]),
            src: P,
            data: vec![n],
        }]
    }

    fn got(out: &mut Vec<Ipv6Packet>) -> Vec<u8> {
        out.drain(..).map(|p| p.data[0]).collect()
    }

    #[test]
    fn without_a_session_frames_go_at_once() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.rx(P, 0, 5, pkt(5), Time(0), &mut out);
        r.rx(P, 0, 3, pkt(3), Time(0), &mut out);
        assert_eq!(got(&mut out), [5, 3]);
    }

    #[test]
    fn a_gap_holds_what_follows_until_it_fills() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 64, 10, &mut out);
        r.rx(P, 0, 10, pkt(10), Time(0), &mut out);
        r.rx(P, 0, 12, pkt(12), Time(0), &mut out);
        r.rx(P, 0, 13, pkt(13), Time(0), &mut out);
        assert_eq!(got(&mut out), [10]);
        r.rx(P, 0, 11, pkt(11), Time(1), &mut out);
        assert_eq!(got(&mut out), [11, 12, 13]);
        assert_eq!(r.deadline(), None);
        // Again, and older: dropped.
        r.rx(P, 0, 12, pkt(12), Time(2), &mut out);
        r.rx(P, 0, 10, pkt(10), Time(2), &mut out);
        assert!(out.is_empty());
        assert_eq!(r.stats().dropped_old, 2);
    }

    #[test]
    fn a_duplicate_is_dropped() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 64, 0, &mut out);
        r.rx(P, 0, 2, pkt(2), Time(0), &mut out);
        r.rx(P, 0, 2, pkt(2), Time(0), &mut out);
        assert_eq!(r.stats().dropped_duplicate, 1);
        r.rx(P, 0, 0, pkt(0), Time(0), &mut out);
        r.rx(P, 0, 1, pkt(1), Time(0), &mut out);
        assert_eq!(got(&mut out), [0, 1, 2]);
    }

    #[test]
    fn a_gap_is_given_up_after_the_timeout() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 64, 0, &mut out);
        r.rx(P, 0, 1, pkt(1), Time(1_000), &mut out);
        r.rx(P, 0, 3, pkt(3), Time(50_000), &mut out);
        assert_eq!(r.deadline(), Some(Time(101_000)));
        r.expire(Time(100_999), &mut out);
        assert!(out.is_empty());
        r.expire(Time(101_000), &mut out);
        assert_eq!(got(&mut out), [1]);
        r.expire(Time(150_000), &mut out);
        assert_eq!(got(&mut out), [3]);
        assert_eq!(r.deadline(), None);
        r.rx(P, 0, 4, pkt(4), Time(150_000), &mut out);
        assert_eq!(got(&mut out), [4]);
    }

    #[test]
    fn a_frame_past_the_window_moves_it() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 8, 0, &mut out);
        r.rx(P, 0, 2, pkt(2), Time(0), &mut out);
        r.rx(P, 0, 9, pkt(9), Time(0), &mut out);
        // The window is now 2..=9: 2 goes, 9 waits for 3..=8.
        assert_eq!(got(&mut out), [2]);
        r.bar(P, 0, 9, &mut out);
        assert_eq!(got(&mut out), [9]);
        assert_eq!(r.stats().released_by_bar, 1);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 64, 4094, &mut out);
        r.rx(P, 0, 0, pkt(0), Time(0), &mut out);
        r.rx(P, 0, 4095, pkt(255), Time(0), &mut out);
        assert!(out.is_empty());
        r.rx(P, 0, 4094, pkt(254), Time(0), &mut out);
        assert_eq!(got(&mut out), [254, 255, 0]);
    }

    #[test]
    fn delba_and_a_new_session_deliver_what_was_held() {
        let mut r = Reorder::default();
        let mut out = Vec::new();
        r.start(P, 0, 64, 0, &mut out);
        r.rx(P, 0, 3, pkt(3), Time(0), &mut out);
        r.start(P, 0, 64, 100, &mut out);
        assert_eq!(got(&mut out), [3]);
        r.rx(P, 0, 101, pkt(101), Time(0), &mut out);
        r.stop(P, 0, &mut out);
        assert_eq!(got(&mut out), [101]);
        assert!(!r.active(P, 0));
    }
}
