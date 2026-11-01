//! The radio as a user-space driver: an MT7612U stick over USB
//! (crates/mt7612u), its firmware loaded and channel 149 tuned by this
//! process, with owly-awdl on it and `awdl0` made as owly-radio makes it.
//! No kernel driver, monitor interface or nl80211: the way to running
//! without WSL (docs/native-windows.md). Experimental: `--usb-radio`.

use std::path::PathBuf;

use mt7612u::Mt7612u;
use mt7612u::awdl::Radio as UsbAir;
use mt7612u::channel::{Channel, Width};
use owly_awdl::{Presence, RadioConstraints, TxRate};
use owly_radio::injection::AWDL_IFACE;
use owly_radio::native::Native;
use owly_radio::radio::{Announce, Link, LinkState, MacAddr};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::events::{emit, say};

/// AirDrop's channel the stick stays on (Presence::Fixed), at 40 MHz.
const CHANNEL: u8 = 149;

pub struct UsbRadio {
    /// Holds mt7662_rom_patch.bin and mt7662.bin.
    firmware: PathBuf,
    tx_rate: Option<TxRate>,
    /// Accept the phone's Block Ack sessions: the chip answers them
    /// (mt7612u's rx_block_ack). On unless --aggregation off.
    aggregation: bool,
    live: Option<(Native, JoinHandle<()>)>,
}

impl UsbRadio {
    pub fn new(
        firmware: PathBuf,
        tx_rate: Option<&str>,
        aggregation: Option<bool>,
    ) -> Result<UsbRadio, String> {
        let tx_rate = tx_rate
            .map(|r| TxRate::parse(r).ok_or_else(|| format!("tx_rate {r:?} isn't a rate")))
            .transpose()?;
        Ok(UsbRadio {
            firmware,
            tx_rate,
            aggregation: aggregation.unwrap_or(true),
            live: None,
        })
    }

    pub fn report(&self) {
        let sticks = mt7612u::sticks().unwrap_or_default();
        let device = sticks
            .first()
            .map(|s| format!("{:04x}:{:04x}", s.vendor_id(), s.product_id()))
            .unwrap_or_default();
        say(format!(
            "Wi-Fi card: MT7612U over USB ({}), driven from user space, channel {CHANNEL}.",
            if device.is_empty() {
                "none found"
            } else {
                &device
            }
        ));
        emit(&["adapter", "mt7612u-usb", "untested", "user-space", &device]);
    }

    pub fn unusable(&self) -> Option<String> {
        match mt7612u::sticks() {
            Ok(s) if !s.is_empty() => None,
            Ok(_) => {
                Some("No MT7612U stick is connected (or this process may not open it).".into())
            }
            Err(e) => Some(format!("Couldn't list USB devices: {e}")),
        }
    }

    pub fn station(&self) -> String {
        "the MT7612U (USB)".into()
    }

    pub async fn up(&mut self) -> Result<Link, String> {
        let dir = self.firmware.clone();
        // Register by register over USB: blocking.
        let (stick, mac) = tokio::task::spawn_blocking(move || -> Result<_, String> {
            let read = |f: &str| {
                std::fs::read(dir.join(f)).map_err(|e| format!("{}: {e}", dir.join(f).display()))
            };
            let (rom_patch, firmware) = (read("mt7662_rom_patch.bin")?, read("mt7662.bin")?);
            let err = |e: mt7612u::Error| e.to_string();
            let mut stick = Mt7612u::open().map_err(err)?;
            let eeprom = stick.eeprom().map_err(err)?;
            let mac = mt7612u::eeprom_mac(&eeprom).ok_or("no MAC address in the EEPROM")?;
            stick.start_mcu(&rom_patch, &firmware).map_err(err)?;
            stick.init_mac(&eeprom).map_err(err)?;
            stick
                .set_channel(&eeprom, Channel::new(CHANNEL, Width::Ht40Plus))
                .map_err(err)?;
            stick.start_rx().map_err(err)?;
            Ok((stick, MacAddr(mac)))
        })
        .await
        .map_err(|e| e.to_string())??;

        let name = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|h| h.trim().to_string())
            .ok()
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "howldrop".into());
        let radio = RadioConstraints {
            channel: owly_awdl::Channel(CHANNEL),
            presence: Presence::Fixed,
        };
        let mut cfg = owly_awdl::Config::new(mac, &name, radio);
        if let Some(rate) = self.tx_rate {
            cfg.rate = rate;
        }
        cfg.accept_addba = self.aggregation;
        let (peers_tx, peers) = watch::channel(Vec::new());
        let native = tokio::task::spawn_blocking(move || {
            Native::start_air(AWDL_IFACE, cfg, peers_tx, move || Ok(UsbAir::new(stick)))
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("AWDL on the stick: {e}"))?;

        // The link is lost when the node stops by itself.
        let (state_tx, state) = watch::channel(LinkState::Up);
        let mut failed = native.failed();
        let supervisor = tokio::spawn(async move {
            while failed.changed().await.is_ok() {
                if let Some(why) = failed.borrow_and_update().clone() {
                    let _ = state_tx.send(LinkState::Lost(why));
                    break;
                }
            }
        });
        let link = Link {
            name: AWDL_IFACE.into(),
            index: native.index,
            mac,
            addr: mac.link_local(),
            announce: Some(Announce::FIXED),
            peers,
            state,
        };
        self.live = Some((native, supervisor));
        Ok(link)
    }

    pub async fn down(&mut self) {
        if let Some((native, supervisor)) = self.live.take() {
            supervisor.abort();
            let _ = tokio::task::spawn_blocking(move || native.stop()).await;
        }
    }
}
