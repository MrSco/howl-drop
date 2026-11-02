//! The link on Windows: a Wintun adapter, as `awdl0` is on Linux. Up with
//! MTU 1450, no DAD or router discovery, and only the AWDL address's
//! link-local address (NET-1): Windows' own goes, since a packet from it has
//! no MAC the node can send it from. Wintun carries IP packets, which
//! owly-awdl takes as they are ([`Ipv6Packet::from_ip`]). Making the adapter
//! and its session takes an administrator.

use std::io;
use std::net::{IpAddr, Ipv6Addr};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use owly_awdl::driver::Netif;
use owly_awdl::{Ipv6Packet, MacAddr};
use tokio::sync::mpsc;
use wintun::{Adapter, Session};

/// The adapter's name, as Windows' network settings show it.
const NAME: &str = "HowlDrop AWDL";
/// As awdl0's (owly-radio's TAP_MTU).
const MTU: u32 = 1450;
/// Wintun's ring: 4 MiB each way.
const RING: u32 = 0x40_0000;

pub struct Wintun {
    pub index: u32,
    _adapter: Arc<Adapter>,
    session: Arc<Session>,
    rx: Option<mpsc::Receiver<Vec<u8>>>,
}

impl Wintun {
    /// Wintun's adapter with `wintun.dll` from `dll`, set up for `mac`.
    pub fn create(dll: &Path, mac: MacAddr) -> Result<Wintun, String> {
        // SAFETY: wintun.dll is WireGuard's, loaded once for the process.
        #[allow(unsafe_code)]
        let wintun = unsafe { wintun::load_from_path(dll) }
            .map_err(|e| format!("loading {}: {e}", dll.display()))?;
        let adapter = Adapter::open(&wintun, NAME)
            .or_else(|_| Adapter::create(&wintun, NAME, "HowlDrop", None))
            .map_err(|e| format!("the Wintun adapter (run as administrator?): {e}"))?;
        let index = adapter.get_adapter_index().map_err(|e| e.to_string())?;
        let addr = mac.link_local();

        netsh(&format!(
            "interface ipv6 set interface {index} mtu={MTU} dadtransmits=0 routerdiscovery=disabled"
        ))?;
        prune(&adapter, index, addr);
        netsh(&format!(
            "interface ipv6 add address interface={index} address={addr}"
        ))?;

        let session = Arc::new(adapter.start_session(RING).map_err(|e| e.to_string())?);
        // Windows gives the link a link-local address of its own once the
        // session brings it up, and may again later: it goes each time.
        let watched = Arc::downgrade(&adapter);
        std::thread::Builder::new()
            .name("wintun-addresses".into())
            .spawn(move || {
                while let Some(adapter) = watched.upgrade() {
                    prune(&adapter, index, addr);
                    drop(adapter);
                    std::thread::sleep(Duration::from_millis(500));
                }
            })
            .map_err(|e| e.to_string())?;
        // What the host sends on the link, read on a thread of its own.
        let (tx, rx) = mpsc::channel(1024);
        let reader = session.clone();
        std::thread::Builder::new()
            .name("wintun".into())
            .spawn(move || {
                while let Ok(packet) = reader.receive_blocking() {
                    let bytes = packet.bytes();
                    if bytes.first().map(|b| b >> 4) == Some(6)
                        && tx.blocking_send(bytes.to_vec()).is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Wintun {
            index,
            _adapter: adapter,
            session,
            rx: Some(rx),
        })
    }

    /// The node's end of the link; once.
    pub fn netif(&mut self) -> Option<WintunNetif> {
        Some(WintunNetif {
            session: self.session.clone(),
            rx: self.rx.take()?,
            unsendable: 0,
        })
    }

    /// Ends the session; the adapter goes with the last reference to it.
    pub fn close(self) {
        let _ = self.session.shutdown();
    }
}

pub struct WintunNetif {
    session: Arc<Session>,
    rx: mpsc::Receiver<Vec<u8>>,
    /// Host packets with no MAC to send them from (an address not ours).
    unsendable: u64,
}

impl Netif for WintunNetif {
    async fn recv(&mut self) -> io::Result<Ipv6Packet> {
        loop {
            let data = self
                .rx
                .recv()
                .await
                .ok_or_else(|| io::Error::other("the Wintun session ended"))?;
            match Ipv6Packet::from_ip(data) {
                Some(packet) => return Ok(packet),
                None => {
                    self.unsendable += 1;
                    if self.unsendable.is_power_of_two() {
                        tracing::warn!(
                            "{} host packets dropped: not from or to a link-local address with a MAC",
                            self.unsendable
                        );
                    }
                }
            }
        }
    }

    fn deliver(&mut self, packet: Ipv6Packet) {
        let Ok(len) = u16::try_from(packet.data.len()) else {
            return;
        };
        // A full ring drops it, as a full queue would.
        if let Ok(mut out) = self.session.allocate_send_packet(len) {
            out.bytes_mut().copy_from_slice(&packet.data);
            self.session.send_packet(out);
        }
    }
}

/// Every IPv6 address on the link but `keep` deleted.
fn prune(adapter: &Adapter, index: u32, keep: Ipv6Addr) {
    for old in adapter.get_addresses().unwrap_or_default() {
        if let IpAddr::V6(a) = old
            && a != keep
        {
            match netsh(&format!(
                "interface ipv6 delete address interface={index} address={a}"
            )) {
                Ok(()) => tracing::debug!("{a} deleted from the link"),
                Err(e) => tracing::warn!("{e}"),
            }
        }
    }
}

fn netsh(args: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    /// No console window flashing up from a program without one.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("netsh")
        .args(args.split(' '))
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("netsh: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "netsh {args}: {}",
            String::from_utf8_lossy(&out.stdout).trim()
        ))
    }
}

/// Timers to the millisecond for this process. Windows wakes them on its
/// 15.6 ms tick otherwise, most of an AWDL slot (16 TU) late, and the node's
/// frames, the peer's TCP ACKs among them, with them. Windows 11 also lets
/// a program without a window in sight fall back to the coarse tick unless
/// it asks not to.
#[allow(unsafe_code)]
pub fn fine_timers() {
    use windows_sys::Win32::Media::timeBeginPeriod;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcess, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
        ProcessPowerThrottling, SetProcessInformation,
    };
    let state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
        StateMask: 0,
    };
    // SAFETY: plain Win32 calls on this process; `state` outlives the call
    // and its size is given. The period lasts until the process ends.
    unsafe {
        SetProcessInformation(
            GetCurrentProcess(),
            ProcessPowerThrottling,
            (&raw const state).cast(),
            size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        );
        timeBeginPeriod(1);
    }
}
