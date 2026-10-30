//! AirDrop: a sans-IO protocol core, and a driver for it on tokio and rustls.
//!
//! [`proto`] implements layers 3–7 of docs/SPEC.md as codecs and state machines
//! with no I/O. The `tokio` feature adds [`driver`], which runs them over TLS
//! sockets and a staging directory; the `mdns` feature adds a DNS-SD responder
//! and browser to it.

#[cfg(feature = "tokio")]
pub mod driver;
pub mod proto;
