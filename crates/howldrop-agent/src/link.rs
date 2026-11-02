//! The link the receiver and the sender use: `awdl0` on Linux, as
//! owly-radio makes it, and on Windows a Wintun adapter ([`crate::wintun`])
//! described the same way.

#[cfg(target_os = "linux")]
pub use owly_radio::radio::{Announce, Link, LinkState};

#[cfg(windows)]
pub use self::windows::{Announce, Link, LinkState};

#[cfg(windows)]
mod windows {
    use std::net::Ipv6Addr;
    use std::time::Duration;

    use owly_awdl::{MacAddr, PeerInfo};
    use tokio::sync::watch;

    /// As owly-radio's: the host's interface for the AWDL link.
    pub struct Link {
        pub name: String,
        /// Only the scope ID matters to sockets (NET-2).
        pub index: u32,
        /// The AWDL address, pinned for the session (AWDL-2).
        #[allow(dead_code)]
        pub mac: MacAddr,
        /// The link-local address from `mac` (NET-1).
        pub addr: Ipv6Addr,
        /// The burst a receiver repeats its mDNS announcement in (MDNS-7).
        pub announce: Option<Announce>,
        /// The valid AWDL peers.
        pub peers: watch::Receiver<Vec<PeerInfo>>,
        pub state: watch::Receiver<LinkState>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum LinkState {
        Up,
        /// The radio or the adapter failed.
        Lost(String),
    }

    /// An mDNS announcement burst (MDNS-7).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Announce {
        pub count: u32,
        pub interval: Duration,
        pub pause: Duration,
    }

    impl Announce {
        /// For a radio that stays on one channel: a burst across a peer's
        /// whole availability window, every 3 s.
        pub const FIXED: Announce = Announce {
            count: 64,
            interval: Duration::from_micros(16_400),
            pause: Duration::from_secs(3),
        };
    }
}
