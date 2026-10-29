//! The sans-IO protocol core: codecs and state machines that take bytes and
//! return bytes and events. Nothing here touches a socket, a file or a clock.

pub mod archive;
pub mod ble;
pub mod http;
pub mod mdns;
pub mod messages;
pub mod plist;
pub mod receiver;
pub mod sender;
