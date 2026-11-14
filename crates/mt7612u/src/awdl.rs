//! The stick as owly-awdl's radio: an [`Air`] that injects and captures
//! 802.11 frames on the channel [`Mt7612u::set_channel`] tuned, with the
//! chip ACKing frames to its own address (the EEPROM's, the node's AWDL
//! address).

use std::io;
use std::time::{Duration, Instant};

use nusb::transfer::Buffer;
use owly_awdl::driver::{Air, Netif, Received};
use owly_awdl::{Frame, Ipv6Packet, TxRate};

use crate::rate::AutoRate;
use crate::{Mt7612u, RX_BUF, SendError, regs, rx, tx};

const STATS_EVERY: Duration = Duration::from_secs(10);

/// Unicast data frames the chip may hold at once. The node sends one only
/// while its peer is on our channel (AWDL-16), but a frame waiting in the
/// chip's queue goes on air later: 128 frames at 12 Mbit/s are some 150 ms,
/// past the end of an iPhone's few slots on the channel, and those frames
/// were lost once the chip's retries ran out. Kept to a few milliseconds of
/// air, the rest wait in the driver, which checks the slot again before
/// each. Action and multicast frames still have the chip's whole queue.
const DATA_QUEUE: usize = 4;

/// With [`Radio::deep_queue`], as many unicast data frames as take about
/// what [`DATA_QUEUE`]'s do at 12 Mbit/s (some 4 ms on air), at most this
/// many: at 54 Mbit/s four frames went out in about 1.3 ms, and the chip
/// then sat idle until the driver's next refill.
const DATA_QUEUE_MAX: usize = 32;

/// How many unicast data frames at `rate` the chip may hold.
fn data_queue(rate: tx::Rate, deep: bool) -> usize {
    if !deep {
        return DATA_QUEUE;
    }
    let at_12 = tx::Rate::Legacy(24).kbps100() as usize;
    (DATA_QUEUE * rate.kbps100() as usize / at_12).clamp(DATA_QUEUE, DATA_QUEUE_MAX)
}

/// A started stick ([`Mt7612u::start_rx`] done) as owly-awdl's [`Air`],
/// counting what goes through it.
pub struct Radio {
    stick: Mt7612u,
    stats: Stats,
    logged: Instant,
    /// The rate for unicast data when it's chosen here ([`Radio::auto_rate`]).
    auto: Option<AutoRate>,
    /// Unicast data frames sent since `counted`.
    data_sent: u64,
    /// The chip's room for unicast data follows the rate ([`data_queue`]).
    deep_queue: bool,
    /// Peers that accepted our Block Ack session: their WCID and window.
    tx_ba: Vec<([u8; 6], tx::Ampdu)>,
    counted: Instant,
}

#[derive(Debug, Default)]
struct Stats {
    rx: u64,
    rx_awdl: u64,
    rx_crc: u64,
    rx_unparsed: u64,
    tx: u64,
    /// Frames the chip's queue was too full for, given back to the node.
    tx_full: u64,
}

impl Radio {
    pub fn new(stick: Mt7612u) -> Self {
        Radio {
            stick,
            stats: Stats::default(),
            logged: Instant::now(),
            auto: None,
            data_sent: 0,
            deep_queue: false,
            tx_ba: Vec::new(),
            counted: Instant::now(),
        }
    }

    /// Lets the chip hold unicast data for about the same time on air at
    /// any rate, not the same number of frames.
    pub fn deep_queue(mut self) -> Self {
        self.deep_queue = true;
        self
    }

    /// Unicast data at a rate chosen from the chip's retry counts
    /// ([`crate::rate`]) instead of the node's: 12 to 54 Mbit/s, or with
    /// `ht40` on to HT MCS 15 across 40 MHz.
    pub fn auto_rate(mut self, ht40: bool) -> Self {
        let now = Instant::now();
        let ladder = if ht40 {
            crate::rate::WITH_HT40
        } else {
            crate::rate::LEGACY
        };
        self.auto = Some(AutoRate::new(now, ladder));
        self.counted = now;
        // TX_STA_CNT0 and 1 count from their last read.
        let _ = self.stick.read(crate::Space::Mac, regs::TX_STA_CNT0);
        let _ = self.stick.read(crate::Space::Mac, regs::TX_STA_CNT1);
        self
    }

    /// Once a second, the rate for what the chip's counters say.
    fn adapt_rate(&mut self) {
        if self.auto.is_none() || self.counted.elapsed() < Duration::from_secs(1) {
            return;
        }
        let now = Instant::now();
        self.counted = now;
        let frames = std::mem::take(&mut self.data_sent);
        let (Ok(cnt0), Ok(cnt1)) = (
            self.stick.read(crate::Space::Mac, regs::TX_STA_CNT0),
            self.stick.read(crate::Space::Mac, regs::TX_STA_CNT1),
        ) else {
            return;
        };
        let failed = u64::from(cnt0 & 0xffff);
        let retries = u64::from(cnt1 >> 16);
        let auto = self.auto.as_mut().expect("checked above");
        if let Some(rate) = auto.step(now, frames, failed, retries) {
            tracing::info!(
                target: "owly_awdl",
                "unicast at {rate:?}, {}.{} Mbit/s ({retries} retries, {failed} given up, {frames} frames in a second)",
                rate.kbps100() / 10,
                rate.kbps100() % 10
            );
        }
    }

    fn maybe_log(&mut self) {
        if let Err(e) = self.stick.calibrate() {
            tracing::warn!("calibration: {e}");
        }
        self.adapt_rate();
        if self.logged.elapsed() >= STATS_EVERY {
            self.logged = Instant::now();
            tracing::info!(
                "RADIO {:?}, USB errors {}",
                self.stats,
                self.stick.tx_errors()
            );
        }
    }
}

impl Air for Radio {
    async fn recv(&mut self) -> io::Result<Received> {
        loop {
            // nusb's next_complete is cancel-safe: a frame that completes
            // while the driver races this against its timers stays queued.
            let done = self.stick.rx_in.next_complete().await;
            self.stick.rx_in.submit(Buffer::new(RX_BUF));
            done.status.map_err(io::Error::other)?;
            self.maybe_log();
            match rx::parse(&done.buffer[..done.actual_len]) {
                Some(f) if f.crc_error() => self.stats.rx_crc += 1,
                Some(f) => {
                    self.stats.rx += 1;
                    self.stats.rx_awdl += u64::from(rx::is_awdl(&f.frame));
                    // A peer ending its session: the chip stops answering it.
                    if let Some(BlockAck::Delba { tid }) = block_ack(&f.frame) {
                        let peer = f.frame[10..16].try_into().unwrap();
                        self.block_ack(peer, tid, false);
                    }
                    self.notice_tx_ba(&f.frame);
                    return Ok(Received {
                        bytes: f.frame,
                        rssi: Some(f.rssi[0] as i8),
                    });
                }
                None => self.stats.rx_unparsed += 1,
            }
        }
    }

    fn send(&mut self, frame: Frame) -> Result<(), Frame> {
        let Frame {
            bytes,
            to,
            rate,
            no_ack,
        } = frame;
        let data = is_unicast_data(&bytes);
        let chip_rate = match (rate, &self.auto) {
            (_, Some(auto)) if data => auto.rate(),
            (TxRate::Legacy(r), _) => tx::Rate::Legacy(r),
            (TxRate::Ht { mcs, sgi }, _) => tx::Rate::Ht { mcs, sgi },
        };
        self.maybe_log();
        // The node accepting a peer's Block Ack session (AWDL-15): the chip
        // answers its aggregates from now on, before the peer hears yes.
        match block_ack(&bytes) {
            Some(BlockAck::Accepted { tid }) => self.block_ack(to.0, tid, true),
            Some(BlockAck::Delba { tid }) => self.block_ack(to.0, tid, false),
            None => {}
        }
        if data {
            self.stick.reap_tx();
            if self.stick.tx_pending() >= data_queue(chip_rate, self.deep_queue) {
                self.stats.tx_full += 1;
                return Err(Frame {
                    bytes,
                    to,
                    rate,
                    no_ack,
                });
            }
        }
        // QoS data to a peer that took our Block Ack session: the chip may
        // put it in an A-MPDU with what follows it.
        let ampdu = (data && bytes.first() == Some(&0x88))
            .then(|| self.tx_ba.iter().find(|(p, _)| *p == to.0).map(|(_, a)| *a))
            .flatten();
        match self.stick.send_with(bytes, chip_rate, no_ack, ampdu) {
            Ok(()) => {
                self.stats.tx += 1;
                self.data_sent += u64::from(data);
                Ok(())
            }
            Err(SendError::Full(bytes)) => {
                self.stats.tx_full += 1;
                Err(Frame {
                    bytes,
                    to,
                    rate,
                    no_ack,
                })
            }
            Err(SendError::Rate(r)) => {
                tracing::debug!("the MT7612U has no rate {r:?}; frame dropped");
                Ok(())
            }
        }
    }
}

/// A host interface that takes nothing and gives nothing: enough for the
/// node to sync with its peers before there is a network interface
/// (milestone 4 of docs/native-windows.md).
pub struct NoNetif;

impl Netif for NoNetif {
    async fn recv(&mut self) -> io::Result<Ipv6Packet> {
        std::future::pending().await
    }

    fn deliver(&mut self, _packet: Ipv6Packet) {}
}

impl Radio {
    /// An ADDBA response accepting our session (the node asked for it,
    /// [`owly_awdl::Config::tx_ampdu`]), or the peer's DELBA ending it.
    fn notice_tx_ba(&mut self, frame: &[u8]) {
        if frame.len() < 26 || frame[0] != 0xd0 || frame[24] != 3 {
            return;
        }
        let peer: [u8; 6] = frame[10..16].try_into().unwrap();
        let le16 = |i: usize| {
            frame
                .get(i..i + 2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
        };
        match frame[25] {
            // ADDBA response: token, status, parameters.
            1 if le16(27) == Some(0) => {
                let window = le16(29).map_or(64, |p| (p >> 6) & 0x3ff);
                match self.stick.tx_wcid(peer) {
                    Ok(wcid) => {
                        self.tx_ba.retain(|(p, _)| *p != peer);
                        self.tx_ba.push((peer, tx::Ampdu { wcid, window }));
                        tracing::info!(
                            "A-MPDU to {}: window {window}, WCID {wcid}",
                            crate::format_mac(&peer)
                        );
                    }
                    Err(e) => tracing::warn!("A-MPDU to {}: {e}", crate::format_mac(&peer)),
                }
            }
            // DELBA from the recipient (initiator bit clear): ours ends.
            2 if le16(26).is_some_and(|p| p & 0x0800 == 0) => {
                self.tx_ba.retain(|(p, _)| *p != peer);
            }
            _ => {}
        }
    }

    fn block_ack(&mut self, peer: [u8; 6], tid: u8, on: bool) {
        let who = crate::format_mac(&peer);
        match self.stick.rx_block_ack(peer, tid, on) {
            Ok(()) if on => tracing::info!("Block Ack in hardware for {who}, TID {tid}"),
            Ok(()) => tracing::info!("Block Ack in hardware stopped for {who}, TID {tid}"),
            Err(e) => tracing::warn!("Block Ack for {who}, TID {tid}: {e}"),
        }
    }
}

/// A data frame (type 2) to one station: group addresses have the low bit
/// of their first octet set.
fn is_unicast_data(frame: &[u8]) -> bool {
    frame.first().is_some_and(|fc| fc & 0x0c == 0x08) && frame.get(4).is_some_and(|a| a & 1 == 0)
}

/// A Block Ack session starting or ending, as the frames that do it say.
#[derive(Debug, PartialEq, Eq)]
enum BlockAck {
    /// An ADDBA Response with status success.
    Accepted { tid: u8 },
    /// A DELBA.
    Delba { tid: u8 },
}

/// Category 3 (Block Ack) action frames: an ADDBA Response (action 1:
/// token, status, parameter set with the TID in bits 2-5) or a DELBA
/// (action 2: parameter set with the TID in bits 12-15).
fn block_ack(frame: &[u8]) -> Option<BlockAck> {
    let body = frame.get(24..)?;
    if frame.first() != Some(&0xd0) || body.first() != Some(&3) {
        return None;
    }
    let le16 = |at: usize| Some(u16::from_le_bytes([*body.get(at)?, *body.get(at + 1)?]));
    match body.get(1)? {
        1 if le16(3)? == 0 => Some(BlockAck::Accepted {
            tid: ((le16(5)? >> 2) & 0xf) as u8,
        }),
        2 => Some(BlockAck::Delba {
            tid: (le16(2)? >> 12) as u8,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(body: &[u8]) -> Vec<u8> {
        let mut f = vec![0xd0, 0, 0, 0];
        f.extend_from_slice(&[0x4e, 0x4f, 0x7b, 0xa7, 0xf2, 0x70]);
        f.extend_from_slice(&[0x00, 0xc0, 0xca, 0xba, 0xa7, 0x0b]);
        f.extend_from_slice(&[0x00, 0xc0, 0xca, 0xba, 0xa7, 0x0b]);
        f.extend_from_slice(&[0, 0]);
        f.extend_from_slice(body);
        f
    }

    #[test]
    fn a_deep_queue_holds_about_4_ms_of_air_at_any_rate() {
        use tx::Rate;
        assert_eq!(data_queue(Rate::Legacy(108), false), DATA_QUEUE);
        assert_eq!(data_queue(Rate::Legacy(24), true), 4);
        assert_eq!(data_queue(Rate::Legacy(108), true), 18);
        assert_eq!(data_queue(Rate::Ht40 { mcs: 7, sgi: false }, true), 32);
    }

    #[test]
    fn only_unicast_data_is_kept_to_the_short_queue() {
        let mut data = vec![0x08, 0, 0, 0];
        data.extend_from_slice(&[0x16, 0x51, 0xaf, 0x82, 0x26, 0xec]);
        data.extend_from_slice(&[0; 14]);
        assert!(is_unicast_data(&data));
        // QoS data too.
        data[0] = 0x88;
        assert!(is_unicast_data(&data));
        // Multicast data.
        data[4] = 0x33;
        assert!(!is_unicast_data(&data));
        // An action frame to one station.
        let mut action = data.clone();
        action[0] = 0xd0;
        action[4] = 0x16;
        assert!(!is_unicast_data(&action));
        assert!(!is_unicast_data(&[]));
    }

    #[test]
    fn block_ack_sessions_are_seen_starting_and_ending() {
        // ADDBA Response: token 7, success, parameters with TID 5, timeout
        let params: u16 = 0x0002 | (5 << 2) | (64 << 6);
        let mut body = vec![3, 1, 7, 0, 0];
        body.extend_from_slice(&params.to_le_bytes());
        body.extend_from_slice(&[0, 0]);
        assert_eq!(
            block_ack(&action(&body)),
            Some(BlockAck::Accepted { tid: 5 })
        );
        // refused: no session
        body[3] = 37;
        assert_eq!(block_ack(&action(&body)), None);
        // DELBA, TID 6
        let delba = (6u16 << 12).to_le_bytes();
        assert_eq!(
            block_ack(&action(&[3, 2, delba[0], delba[1], 1, 0])),
            Some(BlockAck::Delba { tid: 6 })
        );
        // AWDL's own action frames aren't Block Ack
        assert_eq!(block_ack(&action(&[127, 0x00, 0x17, 0xf2, 8])), None);
    }
}
