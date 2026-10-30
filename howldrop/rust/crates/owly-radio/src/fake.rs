//! A backend with no radio, for testing what runs on top of one: owlyd's
//! D-Bus API, and the omdrop hooks over it.
//!
//! It keeps a fake `awdl0` under a root directory, the way omdrop-core's
//! conformance tests fake one (`OMDROP_RADIO_TEST_ROOT`):
//! `<root>/sys/class/net/awdl0/` and its line in `<root>/proc/net/if_inet6`.
//! Files in `<root>/owly-fake/` steer it, and it logs to `calls` there:
//!
//! | File | Effect |
//! |---|---|
//! | `up.fail` | `up` fails: `3 <sentence>` (not now) or `4 <sentence>` (no radio) |
//! | `up.sleep` | `up` waits this many seconds first |
//! | `peers` | one MAC a line, the link's peers |
//! | `lose` | when it appears, the link is lost |
//! | `calls` | `up`, `down` and `recover`, one a line |

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::radio::{
    Announce, Backend, Capabilities, Error, ErrorKind, Link, LinkState, MacAddr, PeerInfo,
};
use tokio::sync::watch;

use crate::injection::AWDL_IFACE;
use crate::quirks::Mode;

/// The fake `awdl0`'s MAC.
pub const MAC: MacAddr = MacAddr([0x02, 0, 0, 0, 0, 0x01]);
/// And its index.
pub const INDEX: u32 = 99;

pub struct Fake {
    root: PathBuf,
}

impl Fake {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Fake { root: root.into() }
    }

    fn control(&self, name: &str) -> PathBuf {
        self.root.join("owly-fake").join(name)
    }

    fn log(&self, call: &str) {
        let path = self.control("calls");
        let _ = fs::create_dir_all(path.parent().unwrap());
        if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{call}");
        }
    }

    fn net(&self) -> PathBuf {
        self.root.join("sys/class/net").join(AWDL_IFACE)
    }

    fn if_inet6(&self) -> PathBuf {
        self.root.join("proc/net/if_inet6")
    }

    /// Takes the fake `awdl0` away, as a journal replay would.
    pub fn undo(&self) {
        self.log("recover");
        self.remove();
    }

    fn remove(&self) {
        let _ = fs::remove_dir_all(self.net());
        let _ = fs::write(self.if_inet6(), "");
    }

    fn create(&self) -> std::io::Result<()> {
        let net = self.net();
        fs::create_dir_all(net.join("statistics"))?;
        fs::write(net.join("address"), format!("{MAC}\n"))?;
        fs::write(net.join("ifindex"), format!("{INDEX}\n"))?;
        if !net.join("statistics/rx_packets").exists() {
            fs::write(net.join("statistics/rx_packets"), "0\n")?;
        }
        fs::create_dir_all(self.if_inet6().parent().unwrap())?;
        fs::write(
            self.if_inet6(),
            format!(
                "{:032x} {INDEX:02x} 40 20 80 {AWDL_IFACE:>8}\n",
                u128::from(MAC.link_local())
            ),
        )
    }

    fn peers(&self) -> Vec<PeerInfo> {
        fs::read_to_string(self.control("peers"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.trim().parse().ok())
            .map(|mac| PeerInfo {
                mac,
                name: String::new(),
                devclass: 0,
                rssi: None,
            })
            .collect()
    }

    async fn start(&mut self) -> Result<Link, Error> {
        self.log("up");
        if let Some(secs) = read_number(&self.control("up.sleep")) {
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
        if let Ok(fail) = fs::read_to_string(self.control("up.fail")) {
            let (code, say) = fail
                .trim()
                .split_once(' ')
                .unwrap_or(("4", "Fake radio failed."));
            return Err(Error {
                kind: if code == "3" {
                    ErrorKind::NotNow
                } else {
                    ErrorKind::NoRadio
                },
                message: say.into(),
            });
        }
        self.create()
            .map_err(|e| Error::no_radio(format!("fake awdl0: {e}")))?;
        let (_, peers) = watch::channel(self.peers());
        let (state_tx, state) = watch::channel(LinkState::Up);
        let lose = self.control("lose");
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = state_tx.closed() => return,
                    () = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
                if lose.exists() {
                    let _ = state_tx.send(LinkState::Lost("the fake radio was lost".into()));
                    return;
                }
            }
        });
        Ok(Link {
            name: AWDL_IFACE.into(),
            index: INDEX,
            mac: MAC,
            addr: MAC.link_local(),
            announce: Some(Announce::SHARED),
            peers,
            state,
        })
    }
}

fn read_number(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The fake radio's capabilities: a card in shared mode, ready.
pub fn probe() -> Capabilities {
    Capabilities {
        station: Some("wlan0".into()),
        driver: Some("fake".into()),
        mode: Some(Mode::Shared),
        channel: Some(44),
        hardware: true,
        ..Capabilities::none()
    }
}

impl Backend for Fake {
    async fn up(&mut self) -> Result<Link, Error> {
        self.start().await
    }

    async fn down(&mut self) -> Result<(), Error> {
        self.log("down");
        self.remove();
        Ok(())
    }

    /// Undoes a fake `awdl0` that a killed process left.
    async fn recover(&mut self) -> Result<(), Error> {
        if self.net().exists() {
            self.undo();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::parse_if_inet6;

    #[tokio::test]
    async fn the_fake_awdl0_looks_real() {
        let dir = tempfile::tempdir().unwrap();
        let mut fake = Fake::new(dir.path());
        let link = fake.up().await.unwrap();
        let if_inet6 = fs::read_to_string(dir.path().join("proc/net/if_inet6")).unwrap();
        assert_eq!(parse_if_inet6(&if_inet6, "awdl0"), Some((link.addr, false)));
        fake.down().await.unwrap();
        assert!(!dir.path().join("sys/class/net/awdl0").exists());
        let calls = fs::read_to_string(dir.path().join("owly-fake/calls")).unwrap();
        assert_eq!(calls, "up\ndown\n");
    }
}
