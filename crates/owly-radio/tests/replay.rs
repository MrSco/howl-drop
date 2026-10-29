//! Replay tests: every capture in `tests/captures/` fed through
//! `Node::handle` at its own timestamps, checked against the `.toml` beside
//! it: every AWDL action frame parses whole (T-AWDL-1), the peers it names
//! turn valid, the master is the one it names, and once synced the node's
//! schedule stays within 3 TU of the master's.
//!
//! The corpus is committed, so each capture must also be anonymous, as
//! `owly capture` writes it: every address a stand-in, no wall-clock time,
//! no radiotap beyond channel and signal, no host names.
//!
//! A capture made here from two simulated nodes runs through the same
//! checks, as recorded and as `owly capture` would anonymise it, so the
//! harness is tested while the corpus is still empty.

use std::path::{Path, PathBuf};

use owly_awdl::frame::{self, Received};
use owly_awdl::tlv::Advert;
use owly_awdl::{Action, Channel, Config, Input, MacAddr, Node, Presence, RadioConstraints, Time};
use owly_radio::capture::{Filter, Pseudonyms, addresses};
use owly_radio::{pcap, radiotap};
use serde::Deserialize;

/// What replaying a capture has to find.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Expect {
    channel: u8,
    /// The Linux end, when the capture has one.
    #[serde(default)]
    local: Option<String>,
    /// Nodes that must turn valid.
    peers: Vec<String>,
    /// The master the replaying node must follow at the end.
    #[serde(default)]
    master: Option<String>,
    #[serde(default)]
    notes: Option<String>,
}

fn mac(s: &str) -> MacAddr {
    s.parse().unwrap()
}

struct Outcome {
    action_frames: usize,
    synced_frames: usize,
    off_schedule: usize,
}

/// Replays `records` into a passive node on `channel` that never wins an
/// election, and checks `expect`.
fn replay(name: &str, records: &[pcap::Record], expect: &Expect) -> Outcome {
    let radio = RadioConstraints {
        channel: Channel(expect.channel),
        presence: Presence::Fixed,
    };
    // The lowest address there is: it follows, and wins no tie.
    let mut cfg = Config::new(MacAddr([0x02, 0, 0, 0, 0, 0]), "replay", radio);
    cfg.rssi_threshold = None;
    cfg.neighbour_echo = false;
    let base = records.first().map_or(0, |r| r.micros);
    let at = |micros: u64| Time(micros - base + 1);
    let mut node = Node::new(cfg, at(base));
    let mut timer = at(base);
    // The master whose schedule the node has already taken once.
    let mut synced_to = None;
    let mut out = Outcome {
        action_frames: 0,
        synced_frames: 0,
        off_schedule: 0,
    };
    for (i, r) in records.iter().enumerate() {
        let now = at(r.micros);
        while timer <= now {
            let t = timer;
            for a in node.handle(Input::Timer(t)) {
                if let Action::ArmTimer(next) = a {
                    timer = next.max(t.plus(1));
                }
            }
        }
        let Some(rx) = radiotap::parse(&r.data) else {
            continue;
        };
        if let Ok(Received::Action { header, tlvs, .. }) = frame::parse(rx.frame) {
            out.action_frames += 1;
            // T-AWDL-1: every frame parses whole.
            let advert = Advert::parse(tlvs).unwrap_or_else(|e| {
                panic!("{name}: frame {i} from {} doesn't parse: {e}", header.src)
            });
            // Measured before the node takes the frame in, against the
            // schedule it had: how far the master's is from ours.
            if header.src == node.election().sync
                && !node.election().is_master()
                && let Some(s) = advert.sync
            {
                if synced_to == Some(header.src) {
                    out.synced_frames += 1;
                    if node.clock().error_tu(now, s.next_tu, s.aw_counter).abs() > 3 {
                        out.off_schedule += 1;
                    }
                }
                synced_to = Some(header.src);
            }
        }
        for a in node.handle(Input::Frame {
            rx_time: now,
            rssi: rx.rssi,
            bytes: rx.frame,
        }) {
            if let Action::ArmTimer(next) = a {
                timer = next;
            }
        }
    }
    for p in &expect.peers {
        let m = mac(p);
        assert!(
            node.peer(m).is_some_and(|p| p.valid),
            "{name}: {p} never turned valid"
        );
    }
    if let Some(m) = &expect.master {
        assert_eq!(node.election().master, mac(m), "{name}: another master");
    }
    if out.synced_frames > 0 {
        assert!(
            out.off_schedule * 10 <= out.synced_frames,
            "{name}: {} of {} of the master's frames more than 3 TU off our schedule",
            out.off_schedule,
            out.synced_frames
        );
    }
    out
}

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/captures")
}

// spec: T-AWDL-1, T-AWDL-3
#[test]
fn every_capture_in_the_corpus_replays() {
    let Ok(dir) = std::fs::read_dir(corpus()) else {
        return;
    };
    let mut seen = 0;
    for entry in dir {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "pcap") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let expect_path = path.with_extension("toml");
        let text = std::fs::read_to_string(&expect_path)
            .unwrap_or_else(|e| panic!("{name}: no {}: {e}", expect_path.display()));
        let expect: Expect = toml::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let _ = &expect.notes;
        let records = pcap::read(std::fs::File::open(&path).unwrap()).unwrap();
        assert_anonymous(&name, &records, &expect);
        let out = replay(&name, &records, &expect);
        assert!(out.action_frames > 0, "{name}: no AWDL action frames");
        seen += 1;
    }
    eprintln!("{seen} captures replayed");
}

/// Fails unless a capture is as `owly capture` anonymises it.
fn assert_anonymous(name: &str, records: &[pcap::Record], expect: &Expect) {
    let stand_in = |m: &str| {
        assert!(
            Pseudonyms::is_pseudonym(mac(m)),
            "{name}: {m} in its .toml isn't a stand-in"
        )
    };
    expect.peers.iter().for_each(|p| stand_in(p));
    expect.local.iter().for_each(|p| stand_in(p));
    expect.master.iter().for_each(|p| stand_in(p));
    if let Some(first) = records.first() {
        assert!(
            first.micros < 86_400_000_000,
            "{name}: its time is the wall clock's, not counted from its first frame"
        );
    }
    for (i, r) in records.iter().enumerate() {
        let present = u32::from_le_bytes(r.data[4..8].try_into().unwrap());
        assert_eq!(
            present & !(1 << 3 | 1 << 5),
            0,
            "{name}: frame {i}'s radiotap has more than channel and signal"
        );
        let frame = radiotap::parse(&r.data).unwrap().frame;
        for m in addresses(frame) {
            assert!(Pseudonyms::is_pseudonym(m), "{name}: frame {i} has {m}");
        }
        if let Ok(Received::Action { tlvs, .. }) = frame::parse(frame)
            && let Ok(Advert {
                name: Some(host), ..
            }) = Advert::parse(tlvs)
        {
            assert!(
                host.bytes().all(|b| b == b'x'),
                "{name}: frame {i} names {host:?}"
            );
        }
    }
}

/// `records` as `owly capture` writes them.
fn anonymise(records: &[pcap::Record], keep: Vec<MacAddr>) -> (Vec<pcap::Record>, Pseudonyms) {
    let filter = Filter { keep };
    let kept: Vec<(u64, Option<i8>, Vec<u8>)> = records
        .iter()
        .filter_map(|r| {
            let rx = radiotap::parse(&r.data)?;
            Some((r.micros, rx.rssi, filter.apply(rx.frame)?))
        })
        .collect();
    let names = Pseudonyms::new(kept.iter().flat_map(|(_, _, f)| addresses(f)));
    let base = kept.first().map_or(0, |k| k.0);
    let out = kept
        .into_iter()
        .map(|(micros, rssi, mut frame)| {
            names.apply(&mut frame);
            let mut data = radiotap::rx_header(Some(5220), rssi);
            data.extend_from_slice(&frame);
            pcap::Record {
                micros: micros - base,
                data,
            }
        })
        .collect();
    (out, names)
}

/// Two nodes in step, every frame each sends written down as the other
/// heard it.
fn simulated_capture() -> Vec<pcap::Record> {
    let radio = || RadioConstraints {
        channel: Channel(44),
        presence: Presence::Fixed,
    };
    let phone = MacAddr([0x86, 8, 4, 0x8a, 0x2b, 0xf4]);
    let linux = MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 1]);
    let mut nodes = [
        Node::new(Config::new(phone, "iPhone", radio()), Time(1)),
        Node::new(Config::new(linux, "linux", radio()), Time(23_456)),
    ];
    let mut timers = [Time(1), Time(23_456)];
    let mut records = Vec::new();
    let epoch = 1_760_000_000_000_000u64;
    let end = Time(6_000_000);
    loop {
        let i = if timers[0] <= timers[1] { 0 } else { 1 };
        let now = timers[i];
        if now > end {
            break;
        }
        let actions: Vec<Action> = nodes[i].handle(Input::Timer(now)).collect();
        let mut queue = vec![(i, actions)];
        while let Some((from, actions)) = queue.pop() {
            for a in actions {
                match a {
                    Action::ArmTimer(t) => timers[from] = t.max(now.plus(1)),
                    Action::Transmit { frame, .. } => {
                        let mut data = radiotap::tx_header(frame.rate, frame.no_ack);
                        data.extend_from_slice(&frame.bytes);
                        records.push(pcap::Record {
                            micros: epoch + now.0,
                            data,
                        });
                        let to = 1 - from;
                        let heard: Vec<Action> = nodes[to]
                            .handle(Input::Frame {
                                rx_time: now,
                                rssi: Some(-50),
                                bytes: &frame.bytes,
                            })
                            .collect();
                        queue.push((to, heard));
                    }
                    _ => {}
                }
            }
        }
    }
    records
}

#[test]
fn the_replay_harness_checks_a_capture_of_its_own() {
    let records = simulated_capture();
    // Through a file and back.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sim.pcap");
    let mut w = pcap::Writer::new(std::fs::File::create(&path).unwrap()).unwrap();
    for r in &records {
        w.write(r.micros, &r.data).unwrap();
    }
    w.flush().unwrap();
    drop(w);
    let read = pcap::read(std::fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(read, records);
    let expect: Expect = toml::from_str(
        r#"
        channel = 44
        local = "aa:bb:cc:dd:ee:01"
        peers = ["86:08:04:8a:2b:f4", "aa:bb:cc:dd:ee:01"]
        master = "aa:bb:cc:dd:ee:01"
        "#,
    )
    .unwrap();
    let out = replay("sim", &read, &expect);
    assert!(out.action_frames > 100);
    assert!(out.synced_frames > 10, "the master's frames were checked");
    assert_eq!(out.off_schedule, 0);
}

#[test]
fn an_anonymised_capture_replays_the_same() {
    let phone = MacAddr([0x86, 8, 4, 0x8a, 0x2b, 0xf4]);
    let linux = MacAddr([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 1]);
    let raw = simulated_capture();
    let (records, names) = anonymise(&raw, vec![phone, linux]);
    assert_eq!(records.len(), raw.len());
    let fake = |m: MacAddr| names.get(m).unwrap().to_string();
    let expect = Expect {
        channel: 44,
        local: Some(fake(linux)),
        peers: vec![fake(phone), fake(linux)],
        master: Some(fake(linux)),
        notes: None,
    };
    assert_anonymous("anonymised", &records, &expect);
    let out = replay("anonymised", &records, &expect);
    assert!(out.synced_frames > 10);
    assert_eq!(out.off_schedule, 0);
}

#[test]
#[should_panic(expected = "isn't a stand-in")]
fn a_capture_with_real_addresses_is_refused() {
    let expect: Expect = toml::from_str("channel = 44\npeers = [\"86:08:04:8a:2b:f4\"]").unwrap();
    assert_anonymous("raw", &simulated_capture(), &expect);
}

#[test]
#[should_panic(expected = "wall clock")]
fn a_capture_on_the_wall_clock_is_refused() {
    let expect: Expect = toml::from_str("channel = 44\npeers = []").unwrap();
    assert_anonymous("raw", &simulated_capture(), &expect);
}

#[test]
#[should_panic(expected = "never turned valid")]
fn a_missing_peer_fails_the_replay() {
    let expect: Expect = toml::from_str("channel = 44\npeers = [\"02:99:99:99:99:99\"]").unwrap();
    replay("sim", &simulated_capture(), &expect);
}
