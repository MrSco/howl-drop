//! The driver: runs the [`proto`](crate::proto) core on tokio, over rustls,
//! with uploads extracted into a sandboxed staging directory.

#[cfg(feature = "mdns")]
pub mod mdns;
pub mod receiver;
pub mod sender;
pub mod staging;
pub mod tls;
