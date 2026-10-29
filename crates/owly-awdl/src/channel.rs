//! Channels and channel sequences (docs/SPEC.md §3.4).
//!
//! A sequence names one channel for each of 16 slots, each slot one
//! extended availability window (64 TU). Peers exchange data only in slots
//! where both are on the same channel (AWDL-11).

use alloc::vec::Vec;

use crate::Error;
use crate::wire::{Bytes, Put};

/// Slots in a channel sequence (AWDL-10).
pub const SLOTS: usize = 16;

/// An 802.11 channel number, on 2.4 GHz up to 14 and on 5 GHz from 32.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Channel(pub u8);

impl Channel {
    /// AWDL's social channels (AWDL-11).
    pub const SOCIAL: [Channel; 3] = [Channel(6), Channel(44), Channel(149)];

    /// The centre frequency of its 20 MHz channel.
    pub const fn freq_mhz(self) -> Option<u32> {
        let n = self.0 as u32;
        match n {
            1..=13 => Some(2407 + 5 * n),
            14 => Some(2484),
            32..=196 => Some(5000 + 5 * n),
            _ => None,
        }
    }

    pub const fn from_freq_mhz(mhz: u32) -> Option<Channel> {
        match mhz {
            2484 => Some(Channel(14)),
            2412..=2472 if (mhz - 2407).is_multiple_of(5) => {
                Some(Channel(((mhz - 2407) / 5) as u8))
            }
            5160..=5980 if mhz.is_multiple_of(5) => Some(Channel(((mhz - 5000) / 5) as u8)),
            _ => None,
        }
    }

    /// Its operating class in the opclass encoding (AWDL-10): 81 on 2.4 GHz,
    /// 128 on 5 GHz.
    pub const fn opclass(self) -> u8 {
        if self.0 <= 14 { 0x51 } else { 0x80 }
    }

    /// One slot naming this channel, in the opclass encoding.
    pub const fn slot(self) -> [u8; 2] {
        [self.0, self.opclass()]
    }
}

/// How a sequence writes each slot's channel (AWDL-10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// One byte: the channel number.
    Simple,
    /// Two bytes: flags, then the channel number.
    Legacy,
    /// Two bytes: the channel number, then its operating class.
    OpClass,
}

impl Encoding {
    pub const fn from_u8(v: u8) -> Option<Encoding> {
        match v {
            0 => Some(Encoding::Simple),
            1 => Some(Encoding::Legacy),
            3 => Some(Encoding::OpClass),
            _ => None,
        }
    }

    pub const fn to_u8(self) -> u8 {
        match self {
            Encoding::Simple => 0,
            Encoding::Legacy => 1,
            Encoding::OpClass => 3,
        }
    }

    /// Bytes per slot.
    pub const fn size(self) -> usize {
        match self {
            Encoding::Simple => 1,
            Encoding::Legacy | Encoding::OpClass => 2,
        }
    }
}

/// A channel sequence. Each slot keeps the bytes it was written with, so a
/// sequence copied from a peer goes out byte for byte as the peer sent it;
/// it can only be decoded with the encoding it came in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sequence {
    pub encoding: Encoding,
    pub slots: [[u8; 2]; SLOTS],
}

/// The availability windows per slot the sequence header announces, less
/// one: the presence mode of 4 (§3.2).
const STEP_COUNT: u8 = 3;

impl Sequence {
    /// No channel in any slot.
    pub const fn empty(encoding: Encoding) -> Sequence {
        Sequence {
            encoding,
            slots: [[0; 2]; SLOTS],
        }
    }

    /// `channel` in every slot, in the opclass encoding.
    pub const fn fixed(channel: Channel) -> Sequence {
        Sequence {
            encoding: Encoding::OpClass,
            slots: [channel.slot(); SLOTS],
        }
    }

    /// The channel number in slot `i` (taken modulo 16), 0 for none.
    pub const fn channel(&self, i: usize) -> u8 {
        let s = self.slots[i % SLOTS];
        match self.encoding {
            Encoding::Simple | Encoding::OpClass => s[0],
            Encoding::Legacy => s[1],
        }
    }

    pub fn channels(&self) -> [u8; SLOTS] {
        core::array::from_fn(|i| self.channel(i))
    }

    /// How many slots name `channel`.
    pub fn count(&self, channel: u8) -> usize {
        (0..SLOTS).filter(|&i| self.channel(i) == channel).count()
    }

    /// AWDL-12: this sequence with every slot blanked that doesn't name
    /// `keep`, the one channel a radio that can't hop is on. What's kept is
    /// copied from the peer's bytes, not made up. Returns how many were
    /// kept.
    pub fn intersect(&self, keep: u8) -> (Sequence, usize) {
        let mut out = Sequence::empty(self.encoding);
        let mut kept = 0;
        for i in 0..SLOTS {
            if keep != 0 && self.channel(i) == keep {
                out.slots[i] = self.slots[i];
                kept += 1;
            }
        }
        (out, kept)
    }

    /// The channel most slots name, with `prefer` winning exact ties; 0
    /// when no slot names one.
    pub fn dominant(&self, prefer: u8) -> u8 {
        let mut counts = [0u8; 256];
        for i in 0..SLOTS {
            let c = self.channel(i);
            if c != 0 {
                counts[c as usize] += 1;
            }
        }
        let mut best = 0usize;
        for (c, &n) in counts.iter().enumerate().skip(1) {
            if n > counts[best] {
                best = c;
            }
        }
        if counts[best] == 0 {
            return 0;
        }
        if prefer != 0 && counts[prefer as usize] == counts[best] {
            return prefer;
        }
        best as u8
    }

    /// Parses the sequence structure that the Channel Sequence TLV (18)
    /// carries: count, encoding, duplicate count, step count, fill channel,
    /// then the slots. Only the shape legacy accepted is accepted: 16 slots, no
    /// duplicates, four AWs a slot and fill `0xffff`.
    pub fn parse(value: &[u8]) -> Result<Sequence, Error> {
        let b = Bytes(value);
        if b.u8(0)? as usize + 1 != SLOTS {
            return Err(Error::Malformed("channel sequence length"));
        }
        let encoding =
            Encoding::from_u8(b.u8(1)?).ok_or(Error::Malformed("channel sequence encoding"))?;
        if b.u8(2)? != 0 {
            return Err(Error::Malformed("channel sequence duplicate count"));
        }
        if b.u8(3)? != STEP_COUNT {
            return Err(Error::Malformed("channel sequence step count"));
        }
        if b.le16(4)? != 0xffff {
            return Err(Error::Malformed("channel sequence fill channel"));
        }
        let size = encoding.size();
        let mut seq = Sequence::empty(encoding);
        for (i, slot) in seq.slots.iter_mut().enumerate() {
            let raw = b.slice(6 + i * size, size)?;
            slot[..size].copy_from_slice(raw);
        }
        Ok(seq)
    }

    /// Bytes [`Sequence::parse`] reads.
    pub const fn wire_len(&self) -> usize {
        6 + SLOTS * self.encoding.size()
    }

    /// Writes the structure [`Sequence::parse`] reads.
    pub fn put(&self, out: &mut Vec<u8>) {
        out.u8(SLOTS as u8 - 1);
        out.u8(self.encoding.to_u8());
        out.u8(0);
        out.u8(STEP_COUNT);
        out.le16(0xffff);
        let size = self.encoding.size();
        for slot in &self.slots {
            out.extend_from_slice(&slot[..size]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(channels: [u8; SLOTS]) -> Sequence {
        let mut s = Sequence::empty(Encoding::OpClass);
        for (slot, c) in s.slots.iter_mut().zip(channels) {
            if c != 0 {
                *slot = Channel(c).slot();
            }
        }
        s
    }

    // spec: AWDL-10
    #[test]
    fn sequences_round_trip_in_each_encoding() {
        let s = seq([149, 149, 149, 0, 0, 0, 0, 0, 6, 149, 149, 0, 0, 0, 0, 0]);
        let mut out = Vec::new();
        s.put(&mut out);
        assert_eq!(out.len(), s.wire_len());
        assert_eq!(Sequence::parse(&out), Ok(s));

        // Legacy: flags, then the channel.
        let mut legacy = Vec::from([15, 1, 0, 3, 0xff, 0xff]);
        for i in 0..SLOTS {
            legacy.extend_from_slice(&[0x01, if i < 8 { 44 } else { 6 }]);
        }
        let l = Sequence::parse(&legacy).unwrap();
        assert_eq!(l.channel(0), 44);
        assert_eq!(l.channel(15), 6);

        let mut simple = Vec::from([15, 0, 0, 3, 0xff, 0xff]);
        simple.extend_from_slice(&[6; SLOTS]);
        let s = Sequence::parse(&simple).unwrap();
        assert_eq!(s.channels(), [6; SLOTS]);
        let mut out = Vec::new();
        s.put(&mut out);
        assert_eq!(out, simple);
    }

    #[test]
    fn only_owls_shape_parses() {
        let mut good = Vec::new();
        Sequence::fixed(Channel(6)).put(&mut good);
        for (at, bad) in [(0, 7), (1, 2), (2, 1), (3, 0), (4, 0)] {
            let mut b = good.clone();
            b[at] = bad;
            assert!(Sequence::parse(&b).is_err(), "byte {at} = {bad}");
        }
        assert_eq!(
            Sequence::parse(&good[..good.len() - 1]),
            Err(Error::Truncated)
        );
    }

    // spec: AWDL-12
    #[test]
    fn intersecting_keeps_only_our_channel() {
        let phone = seq([36, 36, 149, 0, 0, 0, 0, 36, 6, 36, 149, 36, 0, 0, 0, 36]);
        let (ours, kept) = phone.intersect(149);
        assert_eq!(kept, 2);
        assert_eq!(
            ours.channels(),
            [0, 0, 149, 0, 0, 0, 0, 0, 0, 0, 149, 0, 0, 0, 0, 0]
        );
        assert_eq!(phone.intersect(44).1, 0);
    }

    #[test]
    fn the_dominant_channel_is_the_peers_best() {
        // As legacy picked it: the operator's channel breaks exact ties only.
        let phone = seq([36, 36, 149, 0, 0, 0, 0, 36, 6, 36, 149, 36, 0, 0, 0, 36]);
        assert_eq!(phone.dominant(6), 36);
        let tie = seq([6, 149, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(tie.dominant(149), 149);
        assert_eq!(Sequence::empty(Encoding::OpClass).dominant(6), 0);
    }

    #[test]
    fn channels_have_frequencies() {
        assert_eq!(Channel(6).freq_mhz(), Some(2437));
        assert_eq!(Channel(44).freq_mhz(), Some(5220));
        assert_eq!(Channel(149).freq_mhz(), Some(5745));
        assert_eq!(Channel::from_freq_mhz(5745), Some(Channel(149)));
        assert_eq!(Channel::from_freq_mhz(2437), Some(Channel(6)));
        assert_eq!(Channel::from_freq_mhz(2436), None);
        assert_eq!(Channel(0).freq_mhz(), None);
    }
}
