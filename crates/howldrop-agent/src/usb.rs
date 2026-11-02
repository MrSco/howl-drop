//! The radio as a user-space driver: an MT7612U stick over USB
//! (crates/mt7612u), its firmware loaded and channel 149 tuned by this
//! process, with owly-awdl on it. No kernel driver, monitor interface or
//! nl80211 (docs/native-windows.md). The link is `awdl0` made as owly-radio
//! makes it on Linux (`--usb-radio`, experimental there), and a Wintun
//! adapter on Windows ([`crate::wintun`]), where this is the only radio.

use std::path::PathBuf;

use mt7612u::Mt7612u;
use mt7612u::awdl::Radio as UsbAir;
use mt7612u::channel::{Channel, Width};
use owly_awdl::{MacAddr, Presence, RadioConstraints, TxRate};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::events::{emit, say};
use crate::link::{Announce, Link, LinkState};

/// AirDrop's channel the stick stays on (Presence::Fixed), at 40 MHz.
const CHANNEL: u8 = 149;

/// The node, and the link it runs between.
#[cfg(target_os = "linux")]
type Live = owly_radio::native::Native;
#[cfg(windows)]
type Live = (owly_awdl::driver::Running, crate::wintun::Wintun);

pub struct UsbRadio {
    /// Holds mt7662_rom_patch.bin and mt7662.bin.
    firmware: PathBuf,
    tx_rate: Option<TxRate>,
    /// Accept the phone's Block Ack sessions: the chip answers them
    /// (mt7612u's rx_block_ack). On unless --aggregation off.
    aggregation: bool,
    live: Option<(Live, JoinHandle<()>)>,
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
            // On Windows only WinUSB lets a program open it: howldrop.exe's
            // setup puts it on that (setup.rs) when it hears so.
            #[cfg(windows)]
            Ok(s)
                if !s.is_empty()
                    && !s
                        .iter()
                        .any(|d| d.driver().is_some_and(|v| v.eq_ignore_ascii_case("winusb"))) =>
            {
                emit(&["stick", "needs-winusb"]);
                Some(format!(
                    "The MT7612U is on {}, not WinUSB.",
                    s[0].driver().unwrap_or("no driver")
                ))
            }
            Ok(s) if !s.is_empty() => None,
            Ok(_) => Some(if cfg!(windows) {
                "No MT7612U stick on WinUSB is connected.".into()
            } else {
                "No MT7612U stick is connected (or this process may not open it).".into()
            }),
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
            match bring_up(&rom_patch, &firmware) {
                Ok(up) => Ok(up),
                // Nothing resets the stick here (Linux resets it on
                // probe): one that answers nothing, as another driver or a
                // sleep can leave it, is restarted as if replugged, once.
                #[cfg(windows)]
                Err(e) => match crate::setup::cycle_sticks() {
                    Ok(n) if n > 0 => {
                        say(format!("The stick didn't start ({e}); restarting it."));
                        std::thread::sleep(std::time::Duration::from_secs(4));
                        bring_up(&rom_patch, &firmware)
                    }
                    _ => Err(e),
                },
                #[cfg(not(windows))]
                Err(e) => Err(e),
            }
        })
        .await
        .map_err(|e| e.to_string())??;

        let radio = RadioConstraints {
            channel: owly_awdl::Channel(CHANNEL),
            presence: Presence::Fixed,
        };
        let mut cfg = owly_awdl::Config::new(mac, &host_name(), radio);
        if let Some(rate) = self.tx_rate {
            cfg.rate = rate;
        }
        cfg.accept_addba = self.aggregation;
        let (peers_tx, peers) = watch::channel(Vec::new());
        let (live, name, index, mut failed) = start_node(stick, cfg, peers_tx).await?;

        // The link is lost when the node stops by itself.
        let (state_tx, state) = watch::channel(LinkState::Up);
        let supervisor = tokio::spawn(async move {
            while failed.changed().await.is_ok() {
                if let Some(why) = failed.borrow_and_update().clone() {
                    let _ = state_tx.send(LinkState::Lost(why));
                    break;
                }
            }
        });
        let link = Link {
            name,
            index,
            mac,
            addr: mac.link_local(),
            announce: Some(Announce::FIXED),
            peers,
            state,
        };
        self.live = Some((live, supervisor));
        Ok(link)
    }

    pub async fn down(&mut self) {
        if let Some((live, supervisor)) = self.live.take() {
            supervisor.abort();
            let _ = tokio::task::spawn_blocking(move || stop_node(live)).await;
        }
    }
}

/// The stick opened, its firmware running and channel 149 tuned.
fn bring_up(rom_patch: &[u8], firmware: &[u8]) -> Result<(Mt7612u, MacAddr), String> {
    let err = |e: mt7612u::Error| e.to_string();
    let mut stick = Mt7612u::open().map_err(err)?;
    let eeprom = stick.eeprom().map_err(err)?;
    let mac = mt7612u::eeprom_mac(&eeprom).ok_or("no MAC address in the EEPROM")?;
    stick.start_mcu(rom_patch, firmware).map_err(err)?;
    stick.init_mac(&eeprom).map_err(err)?;
    stick
        .set_channel(&eeprom, Channel::new(CHANNEL, Width::Ht40Plus))
        .map_err(err)?;
    stick.start_rx().map_err(err)?;
    Ok((stick, MacAddr(mac)))
}

/// The node on the stick, with `awdl0` as owly-radio's Native makes it:
/// the node, the link's name and index, and why the node stopped.
#[cfg(target_os = "linux")]
async fn start_node(
    stick: Mt7612u,
    cfg: owly_awdl::Config,
    peers: watch::Sender<Vec<owly_awdl::PeerInfo>>,
) -> Result<(Live, String, u32, watch::Receiver<Option<String>>), String> {
    use owly_radio::injection::AWDL_IFACE;
    use owly_radio::native::Native;
    let native = tokio::task::spawn_blocking(move || {
        Native::start_air(AWDL_IFACE, cfg, peers, move || Ok(UsbAir::new(stick)))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("AWDL on the stick: {e}"))?;
    let (index, failed) = (native.index, native.failed());
    Ok((native, AWDL_IFACE.into(), index, failed))
}

#[cfg(target_os = "linux")]
fn stop_node(native: Live) {
    native.stop();
}

/// The node on the stick, with a Wintun adapter as its link; wintun.dll
/// beside this program.
#[cfg(windows)]
async fn start_node(
    stick: Mt7612u,
    cfg: owly_awdl::Config,
    peers: watch::Sender<Vec<owly_awdl::PeerInfo>>,
) -> Result<(Live, String, u32, watch::Receiver<Option<String>>), String> {
    let dll = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("wintun.dll")))
        .ok_or("where this program is")?;
    let mac = cfg.addr;
    let (link, running) = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let mut link = crate::wintun::Wintun::create(&dll, mac)?;
        let netif = link.netif().ok_or("the link's end is taken")?;
        let running = owly_awdl::driver::spawn(cfg, move || Ok((UsbAir::new(stick), netif)), peers)
            .map_err(|e| format!("AWDL on the stick: {e}"))?;
        Ok((link, running))
    })
    .await
    .map_err(|e| e.to_string())??;
    let (index, failed) = (link.index, running.failed());
    Ok(((running, link), "HowlDrop AWDL".into(), index, failed))
}

#[cfg(windows)]
fn stop_node((running, link): Live) {
    running.stop();
    link.close();
}

/// The name the node gives itself in its AWDL frames.
fn host_name() -> String {
    let name = if cfg!(windows) {
        std::env::var("COMPUTERNAME").unwrap_or_default()
    } else {
        std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default()
    };
    let name = name.trim();
    if name.is_empty() {
        "howldrop".into()
    } else {
        name.to_string()
    }
}
