//! The stick as owly-awdl's radio: an [`Air`] that injects and captures
//! 802.11 frames on the channel [`Mt7612u::set_channel`] tuned, with the
//! chip ACKing frames to its own address (the EEPROM's, the node's AWDL
//! address).

use std::io;
use std::time::{Duration, Instant};

use nusb::transfer::Buffer;
use owly_awdl::driver::{Air, Netif, Received};
use owly_awdl::{Frame, Ipv6Packet, TxRate};

use crate::{Mt7612u, RX_BUF, SendError, rx, tx};

const STATS_EVERY: Duration = Duration::from_secs(10);

/// A started stick ([`Mt7612u::start_rx`] done) as owly-awdl's [`Air`],
/// counting what goes through it.
pub struct Radio {
    stick: Mt7612u,
    stats: Stats,
    /// The last frames sent (diagnostics).
    recent: Vec<String>,
    logged: Instant,
}

#[derive(Debug, Default)]
struct Stats {
    rx: u64,
    rx_awdl: u64,
    rx_crc: u64,
    rx_unparsed: u64,
    tx: u64,
    tx_full: u64,
    /// Frames still with the chip 50 ms after a full queue, the last time.
    tx_stuck: usize,
}

impl Radio {
    pub fn new(stick: Mt7612u) -> Self {
        Radio {
            stick,
            stats: Stats::default(),
            recent: Vec::new(),
            logged: Instant::now(),
        }
    }

    fn maybe_log(&mut self) {
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
        let chip_rate = match rate {
            TxRate::Legacy(r) => tx::Rate::Legacy(r),
            TxRate::Ht { mcs, sgi } => tx::Rate::Ht { mcs, sgi },
        };
        self.maybe_log();
        // What was sent, for the first full queue (diagnostics only).
        let what = format!(
            "{} bytes fc {:02x}{:02x} to {} {rate:?}{}",
            bytes.len(),
            bytes.first().copied().unwrap_or(0),
            bytes.get(1).copied().unwrap_or(0),
            crate::format_mac(&to.0),
            if no_ack { " no-ack" } else { "" }
        );
        match self.stick.send(bytes, chip_rate, no_ack) {
            Ok(()) => {
                self.stats.tx += 1;
                if self.recent.len() == 40 {
                    self.recent.remove(0);
                }
                self.recent
                    .push(format!("{:.3} {what}", self.logged.elapsed().as_secs_f64()));
                Ok(())
            }
            Err(SendError::Full(bytes)) => {
                self.stats.tx_full += 1;
                if self.stats.tx_full == 1 {
                    tracing::info!(
                        "first full queue; the last frames sent:\n{}",
                        self.recent.join("\n")
                    );
                }
                if self.stats.tx_full % 50 == 1 {
                    // Slow or stuck? A short wait says (diagnostics only).
                    self.stats.tx_stuck = self.stick.flush_tx(Duration::from_millis(50));
                }
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
