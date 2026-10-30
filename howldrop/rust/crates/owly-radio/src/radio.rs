//! The radio layer: the [`Backend`] trait and the types it trades in.
//! A backend turns a radio into an IPv6 link with AWDL peers on it, and
//! nothing above it can tell how.
//! owlyd runs one: [`injection`](crate::injection), or
//! [`fake`](crate::fake) for testing what runs on top.
//!
//! A backend whose radio injects and captures 802.11 frames leaves AWDL to
//! owly-awdl's [`driver`](owly_awdl::driver), through its `Air` and `Netif`
//! traits, as [`native`](crate::native) does.

use std::fmt;
use std::future::Future;
use std::net::Ipv6Addr;
use std::time::Duration;

pub use owly_awdl::{MacAddr, PeerInfo};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use crate::probe::Card;
use crate::quirks::Mode;

/// A radio backend. What it can do is the probe's to say, which runs apart
/// from it, since a backend may be busy bringing the radio up.
pub trait Backend: Send {
    /// Brings the link up.
    fn up(&mut self) -> impl Future<Output = Result<Link, Error>> + Send;

    /// Takes the link down and gives Wi-Fi back. Safe to call when it's
    /// already down.
    fn down(&mut self) -> impl Future<Output = Result<(), Error>> + Send;

    /// Undoes what a session left behind when the process running it died:
    /// gives Wi-Fi back from the recovery journal. A service calls it once
    /// at start, before taking any session. Does nothing when nothing was
    /// left.
    fn recover(&mut self) -> impl Future<Output = Result<(), Error>> + Send;
}

/// An up link: a kernel network interface with an IPv6 link-local address,
/// and the AWDL peers heard on it.
#[derive(Debug, Clone)]
pub struct Link {
    /// `awdl0`. Only the scope ID matters to sockets (NET-2).
    pub name: String,
    pub index: u32,
    /// The AWDL address, pinned for the session (AWDL-2).
    pub mac: MacAddr,
    /// The link-local address from `mac` (NET-1).
    pub addr: Ipv6Addr,
    /// The burst a receiver repeats its mDNS announcement in, when the radio
    /// doesn't carry the records itself (MDNS-7).
    pub announce: Option<Announce>,
    /// The valid AWDL peers.
    pub peers: watch::Receiver<Vec<PeerInfo>>,
    pub state: watch::Receiver<LinkState>,
}

/// Whether the link is up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Up,
    /// The link went away by itself: the radio or the host's interface
    /// failed.
    Lost(String),
}

/// An mDNS announcement burst (MDNS-7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Announce {
    /// Copies per burst.
    pub count: u32,
    /// Between copies.
    pub interval: Duration,
    /// From the end of one burst to the start of the next.
    pub pause: Duration,
}

impl Announce {
    /// Sharing the AP's channel, where multicast goes in AWDL's multicast
    /// slots and the phone is on our channel for those.
    pub const SHARED: Announce = Announce {
        count: 3,
        interval: Duration::from_millis(50),
        pause: Duration::from_secs(3),
    };

    /// Fixed to one channel (AWDL-12), which the phone visits for a slot or
    /// two in sixteen, and where multicast goes as it comes: a copy every
    /// 16 TU, one availability window, for a whole period.
    pub const FIXED: Announce = Announce {
        count: 64,
        interval: Duration::from_micros(16_400),
        pause: Duration::from_secs(3),
    };
}

/// What the [probe](crate::probe) finds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capabilities {
    /// The Wi-Fi interface the radio would use.
    pub station: Option<String>,
    pub driver: Option<String>,
    /// The card's PCI or USB ID.
    pub device: Option<String>,
    /// How well the radio is known to work.
    pub status: Option<Status>,
    /// How the radio would run.
    pub mode: Option<Mode>,
    /// The channel the session would run on.
    pub channel: Option<u8>,
    /// What nl80211 says the card can do.
    pub card: Option<Card>,
    /// False when this computer has no radio Owly can drive.
    pub hardware: bool,
    /// What stands in the way, most fundamental first.
    pub missing: Vec<Missing>,
    /// What works, but less well than it might: known problems with the
    /// card, and with the computer's setup.
    pub issues: Vec<String>,
}

impl Capabilities {
    /// A report with nothing found yet.
    pub fn none() -> Capabilities {
        Capabilities {
            station: None,
            driver: None,
            device: None,
            status: None,
            mode: None,
            channel: None,
            card: None,
            hardware: false,
            missing: Vec::new(),
            issues: Vec::new(),
        }
    }
}

/// How well a radio is known to work, from what testers reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Works out of the box.
    Tested,
    /// Runs only when allowed to (`allow_untested`).
    Untested,
    /// Known not to work; runs only when allowed to, for retesting.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Missing {
    /// `hardware` (no card it can use), `backend` (the backend's own files
    /// or tools), or `radio` (the card's state, which a person can change).
    pub id: &'static str,
    /// One sentence for the person at the computer.
    pub say: String,
}

/// Why a link didn't come up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub kind: ErrorKind,
    /// One sentence for the person at the computer.
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Not now; trying again later may work (Wi-Fi isn't connected, say).
    NotNow,
    /// No radio to bring up, or it failed.
    NoRadio,
}

impl Error {
    pub fn not_now(message: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::NotNow,
            message: message.into(),
        }
    }

    pub fn no_radio(message: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::NoRadio,
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}
