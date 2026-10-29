#![no_main]

use libfuzzer_sys::fuzz_target;
use owly_awdl::tlv::Advert;
use owly_awdl::{
    Channel, Config, Input, Ipv6Packet, MacAddr, Node, Presence, RadioConstraints, Time, frame,
};

// Frames off the air, each after a 2-byte length, fed to a node in turn a
// quarter of a second apart, with its timers run after each, so peers time
// out and elections run; and each one through the parsers on their own.
fuzz_target!(|data: &[u8]| {
    let radio = RadioConstraints {
        channel: Channel(6),
        presence: if data.first().is_some_and(|b| b & 1 == 1) {
            Presence::Fixed
        } else {
            Presence::Share
        },
    };
    let us = MacAddr([2, 0, 0, 0, 0, 1]);
    let mut cfg = Config::new(us, "fuzz", radio);
    cfg.accept_addba = true;
    cfg.amsdu = true;
    cfg.rssi_threshold = None;
    // A table the fuzzer's small inputs can fill.
    cfg.max_peers = 4;
    let mut node = Node::new(cfg, Time(1));
    let mut now = Time(1);
    let mut rest = data.get(1..).unwrap_or_default();
    while rest.len() >= 2 {
        let len = (u16::from_le_bytes([rest[0], rest[1]]) as usize).min(rest.len() - 2);
        let bytes = &rest[2..2 + len];
        rest = &rest[2 + len..];
        if let Ok(frame::Received::Action { tlvs, .. }) = frame::parse(bytes) {
            let _ = Advert::parse(tlvs);
        }
        if let Ok(frame::Received::Data { header, amsdu, body }) = frame::parse(bytes) {
            let _ = frame::msdus(&header, amsdu, body);
        }
        now = now.plus(250_000);
        node.handle(Input::Frame { rx_time: now, rssi: Some(-40), bytes }).for_each(drop);
        node.handle(Input::Timer(now)).for_each(drop);
        // And what the stack sends back to whoever is there.
        for p in node.peers().map(|p| p.mac).collect::<Vec<_>>() {
            let packet = Ipv6Packet::echo_request(us, p, 1, 1, bytes);
            node.handle(Input::Outbound { now, packet }).for_each(drop);
        }
    }
    node.handle(Input::Timer(now.plus(10_000_000))).for_each(drop);
});
